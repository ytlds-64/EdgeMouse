use crate::{
    Advance, ButtonState, Edge, Effect, InputResult, KeyboardEvent, MouseButton, NodeId,
    PhysicalMouseEvent, Point, RemoteMouseEvent, RoutedEvent, RoutedKeyboardEvent, ScreenId,
    Topology, TopologyError,
};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    Local,
    Remote { peer: NodeId },
    Recovering { peer: NodeId },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionConfig {
    pub entry_hysteresis: f64,
    pub peer_timeout_ms: u64,
    pub block_switch_while_dragging: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            entry_hysteresis: 8.0,
            peer_timeout_ms: 1_500,
            block_switch_while_dragging: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    local_node: NodeId,
    topology: Topology,
    config: SessionConfig,
    state: ControlState,
    current_screen: ScreenId,
    pointer: Point,
    entry_guard: Option<(ScreenId, Edge)>,
    pressed_buttons: BTreeSet<MouseButton>,
    next_sequence: u64,
    last_peer_activity_ms: Option<u64>,
    local_restore: Option<(ScreenId, Point)>,
}

impl Session {
    pub fn new(
        local_node: NodeId,
        topology: Topology,
        initial_screen: ScreenId,
        initial_pointer: Point,
        config: SessionConfig,
    ) -> Result<Self, SessionError> {
        if !config.entry_hysteresis.is_finite() || config.entry_hysteresis < 0.0 {
            return Err(SessionError::InvalidHysteresis);
        }
        if config.peer_timeout_ms == 0 {
            return Err(SessionError::InvalidTimeout);
        }
        let screen = topology
            .screen(initial_screen)
            .ok_or(SessionError::Topology(TopologyError::MissingScreen(
                initial_screen,
            )))?;
        if screen.node != local_node {
            return Err(SessionError::InitialScreenNotLocal(initial_screen));
        }
        if !screen.bounds.contains(initial_pointer) {
            return Err(SessionError::InitialPointerOutside(initial_screen));
        }

        Ok(Self {
            local_node,
            topology,
            config,
            state: ControlState::Local,
            current_screen: initial_screen,
            pointer: initial_pointer,
            entry_guard: None,
            pressed_buttons: BTreeSet::new(),
            next_sequence: 1,
            last_peer_activity_ms: None,
            local_restore: None,
        })
    }

    #[must_use]
    pub const fn state(&self) -> ControlState {
        self.state
    }

    #[must_use]
    pub const fn current_screen(&self) -> ScreenId {
        self.current_screen
    }

    #[must_use]
    pub const fn pointer(&self) -> Point {
        self.pointer
    }

    /// Aligns the local routing state with a pointer position last controlled by
    /// the peer. This is used when an incoming remote-control session ends.
    pub fn synchronize_local_pointer(
        &mut self,
        screen: ScreenId,
        pointer: Point,
    ) -> Result<(), SessionError> {
        if self.state != ControlState::Local {
            return Err(SessionError::CannotSynchronizeWhileRemote);
        }
        let target = self
            .topology
            .screen(screen)
            .ok_or(SessionError::Topology(TopologyError::MissingScreen(screen)))?;
        if target.node != self.local_node {
            return Err(SessionError::SynchronizedScreenNotLocal(screen));
        }
        if !target.bounds.contains(pointer) {
            return Err(SessionError::SynchronizedPointerOutside(screen));
        }
        self.current_screen = screen;
        self.pointer = pointer;
        self.entry_guard = None;
        self.pressed_buttons.clear();
        self.last_peer_activity_ms = None;
        self.local_restore = None;
        Ok(())
    }

    pub fn handle_input(
        &mut self,
        event: PhysicalMouseEvent,
        now_ms: u64,
    ) -> Result<InputResult, SessionError> {
        if matches!(self.state, ControlState::Recovering { .. }) {
            return Ok(InputResult::pass_through());
        }

        match event {
            PhysicalMouseEvent::LocalMove { position, movement } => {
                self.handle_local_motion(position, movement, now_ms)
            }
            PhysicalMouseEvent::Move { movement } => self.handle_motion(movement, now_ms),
            PhysicalMouseEvent::Button { button, state } => Ok(self.handle_button(button, state)),
            PhysicalMouseEvent::Wheel {
                horizontal,
                vertical,
            } => self.handle_wheel(horizontal, vertical),
        }
    }

    #[must_use]
    pub fn handle_keyboard(&mut self, event: KeyboardEvent) -> InputResult {
        match self.state {
            ControlState::Local | ControlState::Recovering { .. } => InputResult::pass_through(),
            ControlState::Remote { peer } => {
                let sequence = self.next_sequence();
                InputResult::suppress(vec![Effect::SendKeyboard {
                    peer,
                    event: RoutedKeyboardEvent { sequence, event },
                }])
            }
        }
    }

    pub fn note_peer_activity(&mut self, peer: NodeId, now_ms: u64) {
        if matches!(self.state, ControlState::Remote { peer: active } if active == peer) {
            self.last_peer_activity_ms = Some(now_ms);
        }
    }

    pub fn poll_timeout(&mut self, now_ms: u64) -> Vec<Effect> {
        let ControlState::Remote { peer } = self.state else {
            return Vec::new();
        };
        let Some(last_activity) = self.last_peer_activity_ms else {
            return Vec::new();
        };
        if now_ms.saturating_sub(last_activity) < self.config.peer_timeout_ms {
            return Vec::new();
        }
        self.begin_recovery(peer, true)
    }

    pub fn disconnect_peer(&mut self, peer: NodeId) -> Vec<Effect> {
        if !matches!(self.state, ControlState::Remote { peer: active } if active == peer) {
            return Vec::new();
        }
        self.begin_recovery(peer, true)
    }

    /// Yields an outgoing remote-control session at the trusted peer's
    /// request. Unlike timeout recovery, this is an orderly handoff and does
    /// not report the peer as failed.
    pub fn yield_remote_control(&mut self, peer: NodeId) -> Vec<Effect> {
        if !matches!(self.state, ControlState::Remote { peer: active } if active == peer) {
            return Vec::new();
        }
        self.begin_recovery(peer, false)
    }

    /// Confirms that the platform adapter restored and revealed the local pointer.
    pub fn complete_recovery(&mut self) -> Result<(), SessionError> {
        if !matches!(self.state, ControlState::Recovering { .. }) {
            return Err(SessionError::NotRecovering);
        }
        let (screen, pointer) = self
            .local_restore
            .take()
            .ok_or(SessionError::MissingRestorePoint)?;
        self.current_screen = screen;
        self.pointer = pointer;
        self.state = ControlState::Local;
        self.entry_guard = None;
        self.last_peer_activity_ms = None;
        self.pressed_buttons.clear();
        Ok(())
    }

    fn handle_local_motion(
        &mut self,
        position: Point,
        movement: crate::Vector,
        now_ms: u64,
    ) -> Result<InputResult, SessionError> {
        // An OS warp, clipping at a display boundary, or acceleration can make
        // accumulated deltas disagree with the visible cursor. Re-anchor every
        // local event, without clearing held buttons or the handback guard.
        if self.state != ControlState::Local {
            return Ok(InputResult::suppress(Vec::new()));
        }
        if !position.is_finite() || !movement.is_finite() {
            return Err(SessionError::Topology(TopologyError::NonFiniteMovement));
        }
        let bounds = self.topology.require_bounds(self.current_screen)?;
        self.pointer = bounds.clamp_inside(position, 1.0);
        self.update_entry_guard();

        // Native cursors stop on the last pixel. Only outward motion at an
        // actual desktop edge may attempt a portal; a large delta or an old
        // routing position must never cause a crossing halfway down a portrait
        // screen. Remote motion continues to use unbounded relative deltas.
        let at_edge = self
            .topology
            .can_cross_from(self.current_screen, position, movement);
        if at_edge {
            self.handle_motion(movement, now_ms)
        } else {
            Ok(InputResult::pass_through())
        }
    }

    fn handle_motion(
        &mut self,
        movement: crate::Vector,
        now_ms: u64,
    ) -> Result<InputResult, SessionError> {
        self.update_entry_guard();
        let blocked_edge = self
            .entry_guard
            .and_then(|(screen, edge)| (screen == self.current_screen).then_some(edge));
        let advance =
            self.topology
                .advance(self.current_screen, self.pointer, movement, blocked_edge)?;

        if self.config.block_switch_while_dragging
            && !self.pressed_buttons.is_empty()
            && matches!(advance, Advance::Crossed(_))
        {
            let screen =
                self.topology
                    .screen(self.current_screen)
                    .ok_or(SessionError::Topology(TopologyError::MissingScreen(
                        self.current_screen,
                    )))?;
            self.pointer = self
                .topology
                .clamp_pointer(screen.id, self.pointer + movement)?;
            return Ok(self.route_motion_without_transition());
        }

        match advance {
            Advance::Stayed { screen, position } => {
                self.current_screen = screen;
                self.pointer = position;
                Ok(self.route_motion_without_transition())
            }
            Advance::Crossed(transition) => {
                let from_node = self.node_for(transition.from)?;
                let to_node = self.node_for(transition.to)?;
                let previous_pointer = self.pointer;
                self.current_screen = transition.to;
                self.pointer = transition.position;
                self.entry_guard = Some((transition.to, transition.to_edge));

                match (from_node == self.local_node, to_node == self.local_node) {
                    (true, true) => Ok(InputResult::pass_through()),
                    (true, false) => {
                        let restore_position = self
                            .topology
                            .screen(transition.from)
                            .ok_or(SessionError::Topology(TopologyError::MissingScreen(
                                transition.from,
                            )))?
                            .bounds
                            .clamp_inside(previous_pointer, 1.0);
                        self.state = ControlState::Remote { peer: to_node };
                        self.last_peer_activity_ms = Some(now_ms);
                        self.local_restore = Some((transition.from, restore_position));
                        let enter = self.next_remote_event(RemoteMouseEvent::Enter {
                            screen: transition.to,
                            position: transition.position,
                        });
                        Ok(InputResult::suppress(vec![
                            Effect::CapturePointer {
                                anchor: previous_pointer,
                            },
                            Effect::Send {
                                peer: to_node,
                                event: enter,
                            },
                        ]))
                    }
                    (false, true) => {
                        let peer = from_node;
                        let release_all = self.next_remote_event(RemoteMouseEvent::ReleaseAll);
                        let leave = self.next_remote_event(RemoteMouseEvent::Leave);
                        self.state = ControlState::Local;
                        self.last_peer_activity_ms = None;
                        self.local_restore = None;
                        self.pressed_buttons.clear();
                        Ok(InputResult::suppress(vec![
                            Effect::Send {
                                peer,
                                event: release_all,
                            },
                            Effect::Send { peer, event: leave },
                            Effect::ReleasePointer {
                                screen: transition.to,
                                restore_position: transition.position,
                            },
                        ]))
                    }
                    (false, false) if from_node == to_node => {
                        let enter = self.next_remote_event(RemoteMouseEvent::Enter {
                            screen: transition.to,
                            position: transition.position,
                        });
                        Ok(InputResult::suppress(vec![Effect::Send {
                            peer: to_node,
                            event: enter,
                        }]))
                    }
                    (false, false) => {
                        let release_all = self.next_remote_event(RemoteMouseEvent::ReleaseAll);
                        let leave = self.next_remote_event(RemoteMouseEvent::Leave);
                        let enter = self.next_remote_event(RemoteMouseEvent::Enter {
                            screen: transition.to,
                            position: transition.position,
                        });
                        self.state = ControlState::Remote { peer: to_node };
                        self.last_peer_activity_ms = Some(now_ms);
                        Ok(InputResult::suppress(vec![
                            Effect::Send {
                                peer: from_node,
                                event: release_all,
                            },
                            Effect::Send {
                                peer: from_node,
                                event: leave,
                            },
                            Effect::Send {
                                peer: to_node,
                                event: enter,
                            },
                        ]))
                    }
                }
            }
        }
    }

    fn route_motion_without_transition(&mut self) -> InputResult {
        match self.state {
            ControlState::Local | ControlState::Recovering { .. } => InputResult::pass_through(),
            ControlState::Remote { peer } => {
                let event = self.next_remote_event(RemoteMouseEvent::MoveAbsolute {
                    screen: self.current_screen,
                    position: self.pointer,
                });
                InputResult::suppress(vec![Effect::Send { peer, event }])
            }
        }
    }

    fn handle_button(&mut self, button: MouseButton, state: ButtonState) -> InputResult {
        match state {
            ButtonState::Pressed => {
                self.pressed_buttons.insert(button);
            }
            ButtonState::Released => {
                self.pressed_buttons.remove(&button);
            }
        }

        match self.state {
            ControlState::Local | ControlState::Recovering { .. } => InputResult::pass_through(),
            ControlState::Remote { peer } => {
                let event = self.next_remote_event(RemoteMouseEvent::Button { button, state });
                InputResult::suppress(vec![Effect::Send { peer, event }])
            }
        }
    }

    fn handle_wheel(
        &mut self,
        horizontal: f64,
        vertical: f64,
    ) -> Result<InputResult, SessionError> {
        if !horizontal.is_finite() || !vertical.is_finite() {
            return Err(SessionError::NonFiniteWheel);
        }
        match self.state {
            ControlState::Local | ControlState::Recovering { .. } => {
                Ok(InputResult::pass_through())
            }
            ControlState::Remote { peer } => {
                let event = self.next_remote_event(RemoteMouseEvent::Wheel {
                    horizontal,
                    vertical,
                });
                Ok(InputResult::suppress(vec![Effect::Send { peer, event }]))
            }
        }
    }

    fn update_entry_guard(&mut self) {
        let Some((screen_id, edge)) = self.entry_guard else {
            return;
        };
        if screen_id != self.current_screen {
            self.entry_guard = None;
            return;
        }
        if self
            .topology
            .distance_from_portal(screen_id, self.pointer, edge)
            >= self.config.entry_hysteresis
        {
            self.entry_guard = None;
        }
    }

    fn node_for(&self, screen: ScreenId) -> Result<NodeId, SessionError> {
        self.topology
            .screen(screen)
            .map(|screen| screen.node)
            .ok_or(SessionError::Topology(TopologyError::MissingScreen(screen)))
    }

    fn next_remote_event(&mut self, event: RemoteMouseEvent) -> RoutedEvent {
        let sequence = self.next_sequence();
        RoutedEvent { sequence, event }
    }

    fn next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        sequence
    }

    fn begin_recovery(&mut self, peer: NodeId, report_timeout: bool) -> Vec<Effect> {
        let mut effects = Vec::with_capacity(2);
        if report_timeout {
            effects.push(Effect::PeerTimedOut { peer });
        }
        let Some((screen, restore_position)) = self.local_restore else {
            self.state = ControlState::Recovering { peer };
            return effects;
        };
        self.state = ControlState::Recovering { peer };
        self.last_peer_activity_ms = None;
        effects.push(Effect::ReleasePointer {
            screen,
            restore_position,
        });
        effects
    }
}

#[derive(Debug)]
pub enum SessionError {
    Topology(TopologyError),
    InvalidHysteresis,
    InvalidTimeout,
    InitialScreenNotLocal(ScreenId),
    InitialPointerOutside(ScreenId),
    NonFiniteWheel,
    NotRecovering,
    MissingRestorePoint,
    CannotSynchronizeWhileRemote,
    SynchronizedScreenNotLocal(ScreenId),
    SynchronizedPointerOutside(ScreenId),
}

impl Display for SessionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Topology(error) => Display::fmt(error, formatter),
            Self::InvalidHysteresis => {
                formatter.write_str("entry hysteresis must be finite and non-negative")
            }
            Self::InvalidTimeout => formatter.write_str("peer timeout must be greater than zero"),
            Self::InitialScreenNotLocal(screen) => {
                write!(formatter, "initial screen {} is not local", screen.0)
            }
            Self::InitialPointerOutside(screen) => {
                write!(formatter, "initial pointer is outside screen {}", screen.0)
            }
            Self::NonFiniteWheel => formatter.write_str("wheel deltas must be finite"),
            Self::NotRecovering => formatter.write_str("session is not recovering"),
            Self::MissingRestorePoint => formatter.write_str("session has no local restore point"),
            Self::CannotSynchronizeWhileRemote => {
                formatter.write_str("cannot synchronize the local pointer while routing remotely")
            }
            Self::SynchronizedScreenNotLocal(screen) => {
                write!(formatter, "synchronized screen {} is not local", screen.0)
            }
            Self::SynchronizedPointerOutside(screen) => {
                write!(
                    formatter,
                    "synchronized pointer is outside screen {}",
                    screen.0
                )
            }
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Topology(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TopologyError> for SessionError {
    fn from(value: TopologyError) -> Self {
        Self::Topology(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InputDisposition, KeyCode, KeyState, KeyboardEvent, Rect, Screen, Vector};

    const LOCAL: NodeId = NodeId(10);
    const REMOTE: NodeId = NodeId(20);
    const LOCAL_SCREEN: ScreenId = ScreenId(1);
    const REMOTE_SCREEN: ScreenId = ScreenId(2);

    fn topology() -> Topology {
        let mut topology = Topology::default();
        topology
            .add_screen(
                Screen::new(
                    LOCAL_SCREEN,
                    LOCAL,
                    "Windows",
                    Rect::new(Point::new(0.0, 0.0), 100.0, 100.0).unwrap(),
                    1.0,
                )
                .unwrap(),
            )
            .unwrap();
        topology
            .add_screen(
                Screen::new(
                    REMOTE_SCREEN,
                    REMOTE,
                    "Mac",
                    Rect::new(Point::new(0.0, 0.0), 100.0, 100.0).unwrap(),
                    2.0,
                )
                .unwrap(),
            )
            .unwrap();
        topology
            .connect_bidirectional(LOCAL_SCREEN, Edge::Right, REMOTE_SCREEN)
            .unwrap();
        topology
    }

    fn session() -> Session {
        Session::new(
            LOCAL,
            topology(),
            LOCAL_SCREEN,
            Point::new(99.0, 50.0),
            SessionConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn crossing_the_edge_captures_and_enters_remote() {
        let mut session = session();
        let result = session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();

        assert_eq!(result.disposition, InputDisposition::Suppress);
        assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
        assert!(matches!(result.effects[0], Effect::CapturePointer { .. }));
        assert!(matches!(
            result.effects[1],
            Effect::Send {
                peer: REMOTE,
                event: RoutedEvent {
                    event: RemoteMouseEvent::Enter {
                        screen: REMOTE_SCREEN,
                        ..
                    },
                    ..
                }
            }
        ));
    }

    #[test]
    fn keyboard_follows_remote_mouse_control() {
        let mut session = session();
        let local = session.handle_keyboard(KeyboardEvent {
            key: KeyCode::A,
            state: KeyState::Pressed,
            repeat: false,
        });
        assert_eq!(local.disposition, InputDisposition::PassThrough);

        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();
        let remote = session.handle_keyboard(KeyboardEvent {
            key: KeyCode::A,
            state: KeyState::Pressed,
            repeat: true,
        });
        assert_eq!(remote.disposition, InputDisposition::Suppress);
        assert!(matches!(
            remote.effects.as_slice(),
            [Effect::SendKeyboard {
                peer: REMOTE,
                event: RoutedKeyboardEvent {
                    event: KeyboardEvent {
                        key: KeyCode::A,
                        state: KeyState::Pressed,
                        repeat: true,
                    },
                    ..
                }
            }]
        ));
    }

    #[test]
    fn entry_hysteresis_prevents_an_immediate_bounce() {
        let mut session = session();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();
        let still_remote = session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(-20.0, 0.0),
                },
                110,
            )
            .unwrap();

        assert_eq!(still_remote.disposition, InputDisposition::Suppress);
        assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
    }

    #[test]
    fn returns_local_after_moving_inward_and_back() {
        let mut session = session();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(10.0, 0.0),
                },
                110,
            )
            .unwrap();
        let returned = session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(-30.0, 0.0),
                },
                120,
            )
            .unwrap();

        assert_eq!(session.state(), ControlState::Local);
        assert!(matches!(
            returned.effects.last(),
            Some(Effect::ReleasePointer { .. })
        ));
    }

    #[test]
    fn dragging_blocks_a_screen_transition() {
        let mut session = session();
        session
            .handle_input(
                PhysicalMouseEvent::Button {
                    button: MouseButton::Primary,
                    state: ButtonState::Pressed,
                },
                0,
            )
            .unwrap();
        let result = session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(20.0, 0.0),
                },
                10,
            )
            .unwrap();

        assert_eq!(result.disposition, InputDisposition::PassThrough);
        assert_eq!(session.state(), ControlState::Local);
    }

    #[test]
    fn timeout_restores_local_control() {
        let mut session = session();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();
        let effects = session.poll_timeout(1_600);

        assert_eq!(session.state(), ControlState::Recovering { peer: REMOTE });
        assert!(matches!(effects[0], Effect::PeerTimedOut { peer: REMOTE }));
        assert!(matches!(effects[1], Effect::ReleasePointer { .. }));

        session.complete_recovery().unwrap();
        assert_eq!(session.state(), ControlState::Local);
        assert_eq!(session.current_screen(), LOCAL_SCREEN);
    }

    #[test]
    fn timeout_restore_point_is_kept_inside_the_local_screen() {
        let mut session = Session::new(
            LOCAL,
            topology(),
            LOCAL_SCREEN,
            Point::new(99.75, 50.0),
            SessionConfig::default(),
        )
        .unwrap();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();

        let effects = session.poll_timeout(1_600);

        assert!(matches!(
            effects[1],
            Effect::ReleasePointer {
                restore_position: Point { x: 99.0, y: 50.0 },
                ..
            }
        ));
    }

    #[test]
    fn peer_requested_yield_restores_without_reporting_a_timeout() {
        let mut session = session();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();

        let effects = session.yield_remote_control(REMOTE);

        assert_eq!(session.state(), ControlState::Recovering { peer: REMOTE });
        assert!(matches!(
            effects.as_slice(),
            [Effect::ReleasePointer { .. }]
        ));
        session.complete_recovery().unwrap();
        assert_eq!(session.state(), ControlState::Local);
    }

    #[test]
    fn synchronizes_pointer_after_incoming_remote_control() {
        let mut session = session();
        let position = Point::new(25.0, 75.0);

        session
            .synchronize_local_pointer(LOCAL_SCREEN, position)
            .unwrap();

        assert_eq!(session.state(), ControlState::Local);
        assert_eq!(session.current_screen(), LOCAL_SCREEN);
        assert_eq!(session.pointer(), position);
    }

    #[test]
    fn refuses_to_synchronize_an_outgoing_remote_session() {
        let mut session = session();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(5.0, 0.0),
                },
                100,
            )
            .unwrap();

        assert!(matches!(
            session.synchronize_local_pointer(LOCAL_SCREEN, Point::new(25.0, 75.0)),
            Err(SessionError::CannotSynchronizeWhileRemote)
        ));
    }

    fn portrait_session(edge: Edge) -> Session {
        let mut topology = Topology::default();
        for (id, node, bounds, scale) in [
            (
                LOCAL_SCREEN,
                LOCAL,
                Rect::new(Point::new(-3840.0, 0.0), 6000.0, 3840.0).unwrap(),
                2.0,
            ),
            (
                REMOTE_SCREEN,
                REMOTE,
                Rect::new(Point::new(-253.0, -1080.0), 1920.0, 2036.0).unwrap(),
                1.0,
            ),
        ] {
            topology
                .add_screen(Screen::new(id, node, "desktop", bounds, scale).unwrap())
                .unwrap();
        }
        topology
            .connect_bidirectional(LOCAL_SCREEN, edge, REMOTE_SCREEN)
            .unwrap();
        Session::new(
            LOCAL,
            topology,
            LOCAL_SCREEN,
            Point::new(1000.0, 3830.0),
            SessionConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn portrait_lower_half_remains_local_despite_stale_routing_position() {
        let mut session = portrait_session(Edge::Bottom);
        // The visible cursor was moved/clipped independently of the old routing
        // position. Even a large physical delta cannot cross at 1/3 height.
        for y in [1280.0, 1920.0, 2160.0, 2800.0, 3600.0, 3838.0] {
            let result = session
                .handle_input(
                    PhysicalMouseEvent::LocalMove {
                        position: Point::new(1000.0, y),
                        movement: Vector::new(0.0, 1500.0),
                    },
                    10,
                )
                .unwrap();
            assert_eq!(session.state(), ControlState::Local);
            assert_eq!(session.pointer(), Point::new(1000.0, y));
            assert!(result.effects.is_empty());
        }
        let result = session
            .handle_input(
                PhysicalMouseEvent::LocalMove {
                    position: Point::new(1000.0, 3839.0),
                    movement: Vector::new(0.0, 3.0),
                },
                20,
            )
            .unwrap();
        assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
        assert!(
            result
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::CapturePointer { .. }))
        );
    }

    #[test]
    fn mac_controls_both_halves_of_scaled_windows_portrait_before_right_handback() {
        // Exact display arrangement from the 0.6.11 diagnostic report: the
        // Windows landscape is left of a 200%-scaled portrait, vertically offset.
        let windows = Rect::new(Point::new(-3840.0, 0.0), 6000.0, 3840.0).unwrap();
        let mac = Rect::new(Point::new(-253.0, -1080.0), 1920.0, 2036.0).unwrap();
        let mut topology = Topology::default();
        for (id, node, bounds, scale) in [
            (LOCAL_SCREEN, LOCAL, mac, 1.0),
            (REMOTE_SCREEN, REMOTE, windows, 2.0),
        ] {
            topology
                .add_screen(Screen::new(id, node, "desktop", bounds, scale).unwrap())
                .unwrap();
        }
        topology
            .set_displays(
                REMOTE_SCREEN,
                vec![
                    Rect::new(Point::new(-3840.0, 952.0), 3840.0, 2160.0).unwrap(),
                    Rect::new(Point::new(0.0, 0.0), 2160.0, 3840.0).unwrap(),
                ],
            )
            .unwrap();
        topology
            .set_displays(
                LOCAL_SCREEN,
                vec![
                    Rect::new(Point::new(-253.0, -1080.0), 1920.0, 1080.0).unwrap(),
                    Rect::new(Point::new(0.0, 0.0), 1470.0, 956.0).unwrap(),
                ],
            )
            .unwrap();
        topology
            .connect_bidirectional(LOCAL_SCREEN, Edge::Left, REMOTE_SCREEN)
            .unwrap();

        for y in [400.0, 1496.0, 2292.5, 3400.0] {
            let mut session = Session::new(
                LOCAL,
                topology.clone(),
                LOCAL_SCREEN,
                Point::new(-252.0, -540.0),
                SessionConfig::default(),
            )
            .unwrap();
            let move_by = |session: &mut Session, movement| {
                session
                    .handle_input(PhysicalMouseEvent::Move { movement }, 1)
                    .unwrap()
            };
            move_by(&mut session, Vector::new(-100.0, 0.0));
            let pointer = session.pointer();
            move_by(&mut session, Vector::new(100.0 - pointer.x, y - pointer.y));
            // Traverse the full portrait, including x=1080 (the midpoint), and
            // prove every sent coordinate reaches the right half unchanged.
            for x in [500.0, 1079.0, 1080.0, 1081.0, 1600.0, 2100.0, 2159.0] {
                let dx = x - session.pointer().x;
                let result = move_by(&mut session, Vector::new(dx, 0.0));
                assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
                assert_eq!(session.pointer(), Point::new(x, y));
                assert!(matches!(
                    result.effects.as_slice(),
                    [Effect::Send {
                        event: RoutedEvent {
                            event: RemoteMouseEvent::MoveAbsolute { position, .. },
                            ..
                        },
                        ..
                    }] if *position == Point::new(x, y)
                ));
            }
            move_by(&mut session, Vector::new(3.0, 0.0));
            assert_eq!(session.state(), ControlState::Local);
        }
    }

    #[test]
    fn real_local_edges_work_in_all_directions_with_negative_origins() {
        for (edge, position, movement) in [
            (
                Edge::Left,
                Point::new(-3840.0, 1200.0),
                Vector::new(-3.0, 0.0),
            ),
            (
                Edge::Right,
                Point::new(2159.0, 1200.0),
                Vector::new(3.0, 0.0),
            ),
            (Edge::Top, Point::new(1000.0, 0.0), Vector::new(0.0, -3.0)),
            (
                Edge::Bottom,
                Point::new(1000.0, 3839.0),
                Vector::new(0.0, 3.0),
            ),
        ] {
            let mut session = portrait_session(edge);
            // Tangential/inward motion at the boundary must not cross.
            session
                .handle_input(
                    PhysicalMouseEvent::LocalMove {
                        position,
                        movement: movement.scaled(-1.0),
                    },
                    1,
                )
                .unwrap();
            assert_eq!(session.state(), ControlState::Local);
            session
                .handle_input(PhysicalMouseEvent::LocalMove { position, movement }, 2)
                .unwrap();
            assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
            let remote_pointer = session.pointer();
            session
                .handle_input(PhysicalMouseEvent::LocalMove { position, movement }, 3)
                .unwrap();
            assert_eq!(
                session.pointer(),
                remote_pointer,
                "queued local motion must not move the remote cursor"
            );
        }
    }

    #[test]
    fn real_local_coordinates_preserve_drag_lock() {
        let mut session = portrait_session(Edge::Bottom);
        session
            .handle_input(
                PhysicalMouseEvent::Button {
                    button: MouseButton::Primary,
                    state: ButtonState::Pressed,
                },
                1,
            )
            .unwrap();
        let motion = PhysicalMouseEvent::LocalMove {
            position: Point::new(1000.0, 3839.0),
            movement: Vector::new(0.0, 3.0),
        };
        session.handle_input(motion, 2).unwrap();
        assert_eq!(session.state(), ControlState::Local);
        session
            .handle_input(
                PhysicalMouseEvent::Button {
                    button: MouseButton::Primary,
                    state: ButtonState::Released,
                },
                3,
            )
            .unwrap();
        session.handle_input(motion, 4).unwrap();
        assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
    }

    #[test]
    fn real_local_coordinates_preserve_handback_hysteresis() {
        let mut session = portrait_session(Edge::Bottom);
        let edge_motion = PhysicalMouseEvent::LocalMove {
            position: Point::new(1000.0, 3839.0),
            movement: Vector::new(0.0, 3.0),
        };
        session.handle_input(edge_motion, 1).unwrap();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(0.0, 30.0),
                },
                2,
            )
            .unwrap();
        session
            .handle_input(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(0.0, -50.0),
                },
                3,
            )
            .unwrap();
        assert_eq!(session.state(), ControlState::Local);
        session.handle_input(edge_motion, 4).unwrap();
        assert_eq!(session.state(), ControlState::Local);
        session
            .handle_input(
                PhysicalMouseEvent::LocalMove {
                    position: Point::new(1000.0, 3750.0),
                    movement: Vector::new(0.0, -80.0),
                },
                5,
            )
            .unwrap();
        session.handle_input(edge_motion, 6).unwrap();
        assert_eq!(session.state(), ControlState::Remote { peer: REMOTE });
    }
}
