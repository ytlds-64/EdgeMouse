use crate::config::{LoadedConfig, PeerAddress};
use crate::discovery::{DISCOVERY_PORT, DiscoveryRequest, discover_trusted_peer};
use crate::network::{Network, NetworkEvent};
use crate::platform;
use edgemouse_core::{
    CaptureMode, ControlState, Effect, MouseCaptureBackend, MouseInjectionBackend, Point, Rect,
    RemoteMouseEvent, RoutedEvent, ScreenId, Session,
};
use edgemouse_protocol::WireMessage;
use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(1);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);
const RECONNECT_STOP_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionEnd {
    Stopped,
    Disconnected(String),
}

#[derive(Debug)]
struct ReconnectBackoff {
    next: Duration,
}

impl Default for ReconnectBackoff {
    fn default() -> Self {
        Self {
            next: RECONNECT_INITIAL_DELAY,
        }
    }
}

impl ReconnectBackoff {
    fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = self.next.saturating_mul(2).min(RECONNECT_MAX_DELAY);
        delay
    }

    fn reset(&mut self) {
        self.next = RECONNECT_INITIAL_DELAY;
    }
}

pub fn run(config_path: &Path) -> Result<(), Box<dyn Error>> {
    let config = LoadedConfig::load(config_path)?;
    println!(
        "Local node : {}",
        edgemouse_transport::format_node_id(config.local_node)
    );
    println!(
        "Peer node  : {}",
        edgemouse_transport::format_node_id(config.peer_node)
    );
    let stopping = install_shutdown_handler()?;
    let mut reconnecting = false;
    let mut backoff = ReconnectBackoff::default();

    loop {
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        let initial_pointer = platform::current_pointer()?;
        validate_initial_pointer(&config, initial_pointer)?;
        let session_id = new_session_id(config.local_node.0);
        if reconnecting {
            println!("Reconnecting to the trusted peer…");
        } else {
            println!("Connecting to the trusted peer…");
        }
        let mut transport = config.transport.clone();
        if config.peer_address == PeerAddress::Auto {
            println!("Discovering the trusted peer on UDP {DISCOVERY_PORT}…");
            let request = DiscoveryRequest {
                local_node: config.local_node,
                expected_peer: config.peer_node,
                local_name: transport.local_name.clone(),
                quic_port: transport.bind_address.port(),
                timeout: transport.connect_timeout,
            };
            match discover_trusted_peer(&request, &stopping) {
                Ok(discovered) => {
                    transport.peer_address = discovered.address;
                    println!(
                        "Discovered trusted peer {} at {}",
                        discovered.name, discovered.address
                    );
                }
                Err(_) if stopping.load(Ordering::Acquire) => return Ok(()),
                Err(error) if !reconnecting => return Err(error.into()),
                Err(error) => {
                    let delay = backoff.next_delay();
                    eprintln!(
                        "Peer discovery failed: {error}; retrying in {} second(s)",
                        delay.as_secs()
                    );
                    if wait_until_retry_or_stop(&stopping, delay) {
                        return Ok(());
                    }
                    continue;
                }
            }
        }
        let network = match Network::connect(transport, session_id, Arc::clone(&stopping)) {
            Ok(network) => network,
            Err(_) if stopping.load(Ordering::Acquire) => return Ok(()),
            Err(error) if !reconnecting => return Err(error.into()),
            Err(error) => {
                let delay = backoff.next_delay();
                eprintln!(
                    "Reconnect attempt failed: {error}; retrying in {} second(s)",
                    delay.as_secs()
                );
                if wait_until_retry_or_stop(&stopping, delay) {
                    return Ok(());
                }
                continue;
            }
        };
        if network.peer_node != config.peer_node {
            return Err("connected peer identity does not match configuration".into());
        }
        if reconnecting {
            println!("Reconnected to {} with mutual TLS", network.peer_name);
        } else {
            println!("Connected to {} with mutual TLS", network.peer_name);
        }
        backoff.reset();

        match run_connected(&config, network, initial_pointer, session_id, &stopping)? {
            ConnectionEnd::Stopped => return Ok(()),
            ConnectionEnd::Disconnected(reason) => {
                reconnecting = true;
                let delay = backoff.next_delay();
                eprintln!(
                    "Connection lost: {reason}; local mouse control restored; retrying in {} second(s)",
                    delay.as_secs()
                );
                if wait_until_retry_or_stop(&stopping, delay) {
                    return Ok(());
                }
            }
        }
    }
}

fn run_connected(
    config: &LoadedConfig,
    network: Network,
    initial_pointer: Point,
    session_id: u64,
    stopping: &AtomicBool,
) -> Result<ConnectionEnd, Box<dyn Error>> {
    let mut capture = platform::start_capture(config.local_bounds, config.local_scale)?;
    let mut injector = platform::injector(initial_pointer);
    let mut session = Session::new(
        config.local_node,
        config.topology.clone(),
        config.local_screen,
        initial_pointer,
        config.session,
    )?;
    let mut remote = RemoteReceiver::new(
        config.local_screen,
        config.local_bounds,
        config.session.peer_timeout_ms,
    );
    let clock = Instant::now();

    println!("Edge switching active; press Ctrl+C to stop");
    let result = run_loop(
        &network,
        &mut capture,
        &mut injector,
        &mut session,
        &mut remote,
        session_id,
        stopping,
        clock,
    );

    let effects = session.disconnect_peer(config.peer_node);
    drop(apply_effects(
        effects,
        &network,
        &mut capture,
        &mut session,
        session_id,
    ));
    if matches!(session.state(), ControlState::Recovering { .. }) {
        drop(session.complete_recovery());
    }
    drop(capture.set_mode(CaptureMode::Local { restore: None }));
    drop(injector.release_all());
    let _ = remote.reset(&mut injector);

    result
}

fn validate_initial_pointer(
    config: &LoadedConfig,
    initial_pointer: Point,
) -> Result<(), Box<dyn Error>> {
    if config.local_bounds.contains(initial_pointer) {
        return Ok(());
    }
    Err(format!(
        "current pointer ({:.1}, {:.1}) is outside configured local screen bounds; check origin_x, origin_y, width, and height",
        initial_pointer.x, initial_pointer.y
    )
    .into())
}

fn wait_until_retry_or_stop(stopping: &AtomicBool, delay: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < delay {
        if stopping.load(Ordering::Acquire) {
            return true;
        }
        std::thread::sleep(RECONNECT_STOP_POLL_INTERVAL);
    }
    stopping.load(Ordering::Acquire)
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    network: &Network,
    capture: &mut platform::NativeMouseCapture,
    injector: &mut platform::NativeMouseInjector,
    session: &mut Session,
    remote: &mut RemoteReceiver,
    session_id: u64,
    stopping: &AtomicBool,
    clock: Instant,
) -> Result<ConnectionEnd, Box<dyn Error>> {
    while !stopping.load(Ordering::Acquire) {
        let now_ms = elapsed_ms(clock);
        while let Some(event) = network.try_receive()? {
            match event {
                NetworkEvent::Message(WireMessage::Heartbeat {
                    session_id: remote_session,
                    ..
                }) => {
                    session.note_peer_activity(network.peer_node, now_ms);
                    remote.note_heartbeat(remote_session, now_ms);
                }
                NetworkEvent::Message(WireMessage::Mouse {
                    session_id: remote_session,
                    event,
                }) => {
                    let transition = remote.handle(remote_session, event, injector, now_ms)?;
                    apply_remote_transition(transition, remote.local_screen, capture, session)?;
                    let transition = remote.take_ready_datagram(injector, now_ms)?;
                    apply_remote_transition(transition, remote.local_screen, capture, session)?;
                }
                NetworkEvent::Datagram {
                    session_id: remote_session,
                    after_sequence,
                    event,
                } => {
                    let transition = remote.handle_datagram(
                        remote_session,
                        after_sequence,
                        event,
                        injector,
                        now_ms,
                    )?;
                    apply_remote_transition(transition, remote.local_screen, capture, session)?;
                }
                NetworkEvent::Message(WireMessage::Hello { .. }) => {
                    return Err("peer sent a duplicate Hello message".into());
                }
                NetworkEvent::Message(WireMessage::Goodbye { .. }) => {
                    return Ok(ConnectionEnd::Disconnected("peer shut down".to_owned()));
                }
                NetworkEvent::Message(WireMessage::MouseDatagram { .. }) => {
                    return Err("peer sent a movement datagram on the reliable stream".into());
                }
                NetworkEvent::Metrics {
                    rtt_ms,
                    send_interval_ms,
                    sent_moves,
                    skipped_moves,
                    coalesced_moves,
                    received_moves,
                    stale_moves,
                } => {
                    println!(
                        "Mouse link: RTT {rtt_ms:.1} ms; interval {send_interval_ms} ms; sent {sent_moves}; skipped {skipped_moves}; merged {coalesced_moves}; received {received_moves}; stale {stale_moves}"
                    );
                }
                NetworkEvent::Disconnected(reason) => {
                    if let Some(position) = remote.reset(injector) {
                        restore_incoming_control(remote.local_screen, position, capture, session)?;
                    }
                    return Ok(ConnectionEnd::Disconnected(reason));
                }
            }
        }

        if let Some(position) = remote.poll_timeout(injector, now_ms)? {
            restore_incoming_control(remote.local_screen, position, capture, session)?;
            eprintln!("incoming remote control timed out; restored local mouse control");
        }

        let timeout_effects = session.poll_timeout(now_ms);
        apply_effects(timeout_effects, network, capture, session, session_id)?;

        let mut handled_input = false;
        while let Some(event) = capture.try_next_event()? {
            handled_input = true;
            if remote.is_active() {
                continue;
            }
            let result = session.handle_input(event, now_ms)?;
            apply_effects(result.effects, network, capture, session, session_id)?;
        }
        if !handled_input {
            std::thread::sleep(IDLE_POLL_INTERVAL);
        }
    }
    Ok(ConnectionEnd::Stopped)
}

fn apply_remote_transition(
    transition: RemoteTransition,
    local_screen: ScreenId,
    capture: &mut platform::NativeMouseCapture,
    session: &mut Session,
) -> Result<(), Box<dyn Error>> {
    match transition {
        RemoteTransition::None => Ok(()),
        RemoteTransition::Started { position } => {
            if session.state() != ControlState::Local {
                return Err(
                    "simultaneous mouse handoff detected; local and peer both tried to take control"
                        .into(),
                );
            }
            capture.set_mode(CaptureMode::ReceivingRemote { position })?;
            println!("Peer took mouse control on this screen");
            Ok(())
        }
        RemoteTransition::FirstMotion => {
            println!("Receiving mouse movement from peer");
            Ok(())
        }
        RemoteTransition::Ended { position } => {
            restore_incoming_control(local_screen, position, capture, session)?;
            println!("Peer returned mouse control to this computer");
            Ok(())
        }
    }
}

fn restore_incoming_control(
    local_screen: ScreenId,
    position: Point,
    capture: &mut platform::NativeMouseCapture,
    session: &mut Session,
) -> Result<(), Box<dyn Error>> {
    session.synchronize_local_pointer(local_screen, position)?;
    capture.set_mode(CaptureMode::Local {
        restore: Some(position),
    })?;
    Ok(())
}

fn apply_effects(
    effects: Vec<Effect>,
    network: &Network,
    capture: &mut platform::NativeMouseCapture,
    session: &mut Session,
    session_id: u64,
) -> Result<(), Box<dyn Error>> {
    for effect in effects {
        match effect {
            Effect::CapturePointer { anchor } => {
                capture.set_mode(CaptureMode::Remote { anchor })?;
                println!("Mouse control handed to peer");
            }
            Effect::ReleasePointer {
                restore_position, ..
            } => {
                capture.set_mode(CaptureMode::Local {
                    restore: Some(restore_position),
                })?;
                if matches!(session.state(), ControlState::Recovering { .. }) {
                    session.complete_recovery()?;
                }
                println!("Local mouse control restored");
            }
            Effect::Send { peer, event } => {
                if peer != network.peer_node {
                    return Err("routing requested an unknown peer".into());
                }
                network.send(WireMessage::Mouse { session_id, event })?;
            }
            Effect::PeerTimedOut { peer } => {
                eprintln!(
                    "peer {} timed out; restored local pointer",
                    edgemouse_transport::format_node_id(peer)
                );
            }
        }
    }
    Ok(())
}

struct RemoteReceiver {
    local_screen: ScreenId,
    local_bounds: Rect,
    timeout_ms: u64,
    active_session: Option<u64>,
    last_sequence: u64,
    last_reliable_sequence: u64,
    last_activity_ms: Option<u64>,
    last_position: Option<Point>,
    motion_reported: bool,
    out_of_bounds_reported: bool,
    pending_datagram: Option<PendingDatagram>,
}

#[derive(Debug, Clone, Copy)]
struct PendingDatagram {
    session_id: u64,
    after_sequence: u64,
    event: RoutedEvent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RemoteTransition {
    None,
    Started { position: Point },
    FirstMotion,
    Ended { position: Point },
}

impl RemoteReceiver {
    const fn new(local_screen: ScreenId, local_bounds: Rect, timeout_ms: u64) -> Self {
        Self {
            local_screen,
            local_bounds,
            timeout_ms,
            active_session: None,
            last_sequence: 0,
            last_reliable_sequence: 0,
            last_activity_ms: None,
            last_position: None,
            motion_reported: false,
            out_of_bounds_reported: false,
            pending_datagram: None,
        }
    }

    const fn is_active(&self) -> bool {
        self.active_session.is_some()
    }

    fn handle(
        &mut self,
        session_id: u64,
        routed: RoutedEvent,
        injector: &mut impl MouseInjectionBackend,
        now_ms: u64,
    ) -> Result<RemoteTransition, Box<dyn Error>> {
        if session_id == 0 || routed.sequence == 0 {
            return Err("peer sent a zero session or sequence number".into());
        }
        if matches!(routed.event, RemoteMouseEvent::Enter { .. })
            && self.active_session != Some(session_id)
        {
            self.out_of_bounds_reported = false;
        }
        let routed = self.normalize_target_event(routed)?;
        let started = if matches!(routed.event, RemoteMouseEvent::Enter { .. }) {
            if self.active_session != Some(session_id) {
                injector.release_all()?;
                self.active_session = Some(session_id);
                self.last_sequence = 0;
                self.last_reliable_sequence = 0;
                self.last_position = None;
                self.motion_reported = false;
                self.pending_datagram = None;
                true
            } else {
                false
            }
        } else if self.active_session != Some(session_id) {
            return Err("peer sent mouse input before entering the local screen".into());
        } else {
            false
        };
        if routed.sequence <= self.last_sequence {
            return Ok(RemoteTransition::None);
        }
        injector.inject(routed.event)?;
        self.last_sequence = routed.sequence;
        self.last_reliable_sequence = routed.sequence;
        self.last_activity_ms = Some(now_ms);
        match routed.event {
            RemoteMouseEvent::Enter { position, .. } => {
                self.last_position = Some(position);
                if started {
                    Ok(RemoteTransition::Started { position })
                } else {
                    Ok(RemoteTransition::None)
                }
            }
            RemoteMouseEvent::MoveAbsolute { position, .. } => {
                self.last_position = Some(position);
                if self.motion_reported {
                    Ok(RemoteTransition::None)
                } else {
                    self.motion_reported = true;
                    Ok(RemoteTransition::FirstMotion)
                }
            }
            RemoteMouseEvent::Leave => {
                let position = self
                    .last_position
                    .take()
                    .ok_or("peer left before announcing a pointer position")?;
                self.active_session = None;
                self.last_activity_ms = None;
                self.motion_reported = false;
                self.out_of_bounds_reported = false;
                self.pending_datagram = None;
                Ok(RemoteTransition::Ended { position })
            }
            _ => Ok(RemoteTransition::None),
        }
    }

    fn handle_datagram(
        &mut self,
        session_id: u64,
        after_sequence: u64,
        routed: RoutedEvent,
        injector: &mut impl MouseInjectionBackend,
        now_ms: u64,
    ) -> Result<RemoteTransition, Box<dyn Error>> {
        if session_id == 0 || routed.sequence == 0 {
            return Err("peer sent a zero session or sequence number".into());
        }
        if after_sequence >= routed.sequence {
            return Err("peer sent an invalid movement ordering watermark".into());
        }
        if !matches!(routed.event, RemoteMouseEvent::MoveAbsolute { .. }) {
            return Err("peer sent a non-movement event as a datagram".into());
        }
        let routed = self.normalize_target_event(routed)?;
        if self.active_session != Some(session_id) || routed.sequence <= self.last_sequence {
            return Ok(RemoteTransition::None);
        }
        if after_sequence > self.last_reliable_sequence {
            if self
                .pending_datagram
                .is_none_or(|pending| routed.sequence > pending.event.sequence)
            {
                self.pending_datagram = Some(PendingDatagram {
                    session_id,
                    after_sequence,
                    event: routed,
                });
            }
            return Ok(RemoteTransition::None);
        }
        self.inject_datagram(routed, injector, now_ms)
    }

    fn take_ready_datagram(
        &mut self,
        injector: &mut impl MouseInjectionBackend,
        now_ms: u64,
    ) -> Result<RemoteTransition, Box<dyn Error>> {
        let Some(pending) = self.pending_datagram.take() else {
            return Ok(RemoteTransition::None);
        };
        if self.active_session != Some(pending.session_id)
            || pending.event.sequence <= self.last_sequence
        {
            return Ok(RemoteTransition::None);
        }
        if pending.after_sequence > self.last_reliable_sequence {
            self.pending_datagram = Some(pending);
            return Ok(RemoteTransition::None);
        }
        self.inject_datagram(pending.event, injector, now_ms)
    }

    fn inject_datagram(
        &mut self,
        routed: RoutedEvent,
        injector: &mut impl MouseInjectionBackend,
        now_ms: u64,
    ) -> Result<RemoteTransition, Box<dyn Error>> {
        let RemoteMouseEvent::MoveAbsolute { position, .. } = routed.event else {
            return Err("pending datagram did not contain absolute movement".into());
        };
        injector.inject(routed.event)?;
        self.last_sequence = routed.sequence;
        self.last_activity_ms = Some(now_ms);
        self.last_position = Some(position);
        if self.motion_reported {
            Ok(RemoteTransition::None)
        } else {
            self.motion_reported = true;
            Ok(RemoteTransition::FirstMotion)
        }
    }

    fn note_heartbeat(&mut self, session_id: u64, now_ms: u64) {
        if self.active_session == Some(session_id) {
            self.last_activity_ms = Some(now_ms);
        }
    }

    fn poll_timeout(
        &mut self,
        injector: &mut impl MouseInjectionBackend,
        now_ms: u64,
    ) -> Result<Option<Point>, Box<dyn Error>> {
        let Some(last_activity) = self.last_activity_ms else {
            return Ok(None);
        };
        if now_ms.saturating_sub(last_activity) < self.timeout_ms {
            return Ok(None);
        }
        injector.release_all()?;
        self.active_session = None;
        self.last_activity_ms = None;
        self.motion_reported = false;
        self.out_of_bounds_reported = false;
        self.pending_datagram = None;
        Ok(self.last_position.take())
    }

    fn reset(&mut self, injector: &mut impl MouseInjectionBackend) -> Option<Point> {
        drop(injector.release_all());
        self.active_session = None;
        self.last_activity_ms = None;
        self.last_sequence = 0;
        self.last_reliable_sequence = 0;
        self.motion_reported = false;
        self.out_of_bounds_reported = false;
        self.pending_datagram = None;
        self.last_position.take()
    }

    fn normalize_target_event(
        &mut self,
        routed: RoutedEvent,
    ) -> Result<RoutedEvent, Box<dyn Error>> {
        let normalize = |screen: ScreenId, position: Point| -> Result<Point, Box<dyn Error>> {
            if screen != self.local_screen {
                return Err(format!(
                    "peer targeted screen {}, but this node owns screen {}",
                    screen.0, self.local_screen.0
                )
                .into());
            }
            if !position.is_finite() {
                return Err("peer sent a non-finite pointer position".into());
            }
            Ok(if self.local_bounds.contains(position) {
                position
            } else {
                self.local_bounds.clamp_inside(position, 1.0)
            })
        };

        let (event, original, normalized) = match routed.event {
            RemoteMouseEvent::Enter { screen, position } => {
                let normalized = normalize(screen, position)?;
                (
                    RemoteMouseEvent::Enter {
                        screen,
                        position: normalized,
                    },
                    Some(position),
                    Some(normalized),
                )
            }
            RemoteMouseEvent::MoveAbsolute { screen, position } => {
                let normalized = normalize(screen, position)?;
                (
                    RemoteMouseEvent::MoveAbsolute {
                        screen,
                        position: normalized,
                    },
                    Some(position),
                    Some(normalized),
                )
            }
            event => (event, None, None),
        };
        if let (Some(original), Some(normalized)) = (original, normalized)
            && original != normalized
            && !self.out_of_bounds_reported
        {
            eprintln!(
                "peer pointer ({:.1}, {:.1}) was outside screen {}; clamped to ({:.1}, {:.1})",
                original.x, original.y, self.local_screen.0, normalized.x, normalized.y
            );
            self.out_of_bounds_reported = true;
        }
        Ok(RoutedEvent {
            sequence: routed.sequence,
            event,
        })
    }
}

fn install_shutdown_handler() -> Result<Arc<AtomicBool>, Box<dyn Error>> {
    let stopping = Arc::new(AtomicBool::new(false));
    let handler_state = Arc::clone(&stopping);
    ctrlc::set_handler(move || handler_state.store(true, Ordering::Release))?;
    Ok(stopping)
}

fn elapsed_ms(clock: Instant) -> u64 {
    u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn new_session_id(node: u128) -> u64 {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let mixed = timestamp ^ node ^ (node >> 64);
    u64::try_from(mixed & u128::from(u64::MAX))
        .unwrap_or(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgemouse_core::{ButtonState, MouseButton, PermissionState, PlatformError, Point};

    #[derive(Default)]
    struct FakeInjector {
        events: Vec<RemoteMouseEvent>,
        releases: usize,
    }

    impl MouseInjectionBackend for FakeInjector {
        fn permission_state(&self) -> PermissionState {
            PermissionState::NotRequired
        }

        fn inject(&mut self, event: RemoteMouseEvent) -> Result<(), PlatformError> {
            self.events.push(event);
            Ok(())
        }

        fn release_all(&mut self) -> Result<(), PlatformError> {
            self.releases += 1;
            Ok(())
        }
    }

    fn remote_receiver(timeout_ms: u64) -> RemoteReceiver {
        RemoteReceiver::new(
            ScreenId(7),
            Rect::new(Point::new(0.0, 0.0), 100.0, 100.0).unwrap(),
            timeout_ms,
        )
    }

    #[test]
    fn reconnect_backoff_is_bounded_and_resettable() {
        let mut backoff = ReconnectBackoff::default();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
        assert_eq!(backoff.next_delay(), Duration::from_secs(2));
        assert_eq!(backoff.next_delay(), Duration::from_secs(4));
        assert_eq!(backoff.next_delay(), Duration::from_secs(5));
        assert_eq!(backoff.next_delay(), Duration::from_secs(5));
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn remote_receiver_requires_enter_and_deduplicates_sequences() {
        let mut receiver = remote_receiver(1_500);
        let mut injector = FakeInjector::default();
        let button = RoutedEvent {
            sequence: 2,
            event: RemoteMouseEvent::Button {
                button: MouseButton::Primary,
                state: ButtonState::Pressed,
            },
        };
        assert!(receiver.handle(9, button, &mut injector, 10).is_err());

        let enter = RoutedEvent {
            sequence: 1,
            event: RemoteMouseEvent::Enter {
                screen: ScreenId(7),
                position: Point::new(5.0, 5.0),
            },
        };
        assert_eq!(
            receiver.handle(9, enter, &mut injector, 20).unwrap(),
            RemoteTransition::Started {
                position: Point::new(5.0, 5.0)
            }
        );
        receiver.handle(9, enter, &mut injector, 21).unwrap();
        assert_eq!(injector.events.len(), 1);
    }

    #[test]
    fn remote_timeout_releases_buttons() {
        let mut receiver = remote_receiver(100);
        let mut injector = FakeInjector::default();
        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 1,
                    event: RemoteMouseEvent::Enter {
                        screen: ScreenId(7),
                        position: Point::new(5.0, 5.0),
                    },
                },
                &mut injector,
                10,
            )
            .unwrap();
        assert_eq!(
            receiver.poll_timeout(&mut injector, 111).unwrap(),
            Some(Point::new(5.0, 5.0))
        );
        assert!(injector.releases >= 2);
    }

    #[test]
    fn datagram_waits_for_its_reliable_control_watermark() {
        let mut receiver = remote_receiver(1_500);
        let mut injector = FakeInjector::default();
        let movement = RoutedEvent {
            sequence: 4,
            event: RemoteMouseEvent::MoveAbsolute {
                screen: ScreenId(7),
                position: Point::new(30.0, 40.0),
            },
        };

        assert_eq!(
            receiver
                .handle_datagram(9, 3, movement, &mut injector, 5)
                .unwrap(),
            RemoteTransition::None
        );
        assert!(injector.events.is_empty());

        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 1,
                    event: RemoteMouseEvent::Enter {
                        screen: ScreenId(7),
                        position: Point::new(5.0, 5.0),
                    },
                },
                &mut injector,
                10,
            )
            .unwrap();
        receiver
            .handle_datagram(9, 3, movement, &mut injector, 15)
            .unwrap();
        assert_eq!(injector.events.len(), 1);

        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 3,
                    event: RemoteMouseEvent::Button {
                        button: MouseButton::Primary,
                        state: ButtonState::Pressed,
                    },
                },
                &mut injector,
                20,
            )
            .unwrap();
        assert_eq!(injector.events.len(), 2);
        assert_eq!(
            receiver.take_ready_datagram(&mut injector, 21).unwrap(),
            RemoteTransition::FirstMotion
        );
        assert_eq!(
            injector.events,
            vec![
                RemoteMouseEvent::Enter {
                    screen: ScreenId(7),
                    position: Point::new(5.0, 5.0),
                },
                RemoteMouseEvent::Button {
                    button: MouseButton::Primary,
                    state: ButtonState::Pressed,
                },
                movement.event,
            ]
        );
    }

    #[test]
    fn remote_leave_returns_the_last_injected_position() {
        let mut receiver = remote_receiver(1_500);
        let mut injector = FakeInjector::default();
        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 1,
                    event: RemoteMouseEvent::Enter {
                        screen: ScreenId(7),
                        position: Point::new(1.0, 5.0),
                    },
                },
                &mut injector,
                10,
            )
            .unwrap();
        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 2,
                    event: RemoteMouseEvent::MoveAbsolute {
                        screen: ScreenId(7),
                        position: Point::new(30.0, 40.0),
                    },
                },
                &mut injector,
                20,
            )
            .unwrap();

        let transition = receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 3,
                    event: RemoteMouseEvent::Leave,
                },
                &mut injector,
                30,
            )
            .unwrap();

        assert_eq!(
            transition,
            RemoteTransition::Ended {
                position: Point::new(30.0, 40.0)
            }
        );
        assert!(!receiver.is_active());
    }

    #[test]
    fn remote_receiver_clamps_out_of_bounds_position_before_leave() {
        let mut receiver = remote_receiver(1_500);
        let mut injector = FakeInjector::default();
        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 1,
                    event: RemoteMouseEvent::Enter {
                        screen: ScreenId(7),
                        position: Point::new(1.0, 5.0),
                    },
                },
                &mut injector,
                10,
            )
            .unwrap();
        receiver
            .handle(
                9,
                RoutedEvent {
                    sequence: 2,
                    event: RemoteMouseEvent::MoveAbsolute {
                        screen: ScreenId(7),
                        position: Point::new(-20.0, 120.0),
                    },
                },
                &mut injector,
                20,
            )
            .unwrap();

        assert_eq!(
            receiver
                .handle(
                    9,
                    RoutedEvent {
                        sequence: 3,
                        event: RemoteMouseEvent::Leave,
                    },
                    &mut injector,
                    30,
                )
                .unwrap(),
            RemoteTransition::Ended {
                position: Point::new(1.0, 99.0)
            }
        );
        assert!(matches!(
            injector.events[1],
            RemoteMouseEvent::MoveAbsolute {
                position: Point { x: 1.0, y: 99.0 },
                ..
            }
        ));
    }
}
