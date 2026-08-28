use crate::config::{LoadedConfig, PeerAddress, ResolvedScreen};
use crate::control::ControlServer;
use crate::discovery::{
    DISCOVERY_PORT, DiscoveryRequest, discover_trusted_peer, respond_to_trusted_peer,
};
use crate::network::{Network, NetworkEvent};
use crate::platform;
use edgemouse_core::{
    CaptureMode, ControlState, Edge, Effect, KeyboardCaptureBackend, KeyboardInjectionBackend,
    MouseCaptureBackend, MouseInjectionBackend, PhysicalMouseEvent, Point, Rect, RemoteMouseEvent,
    RoutedEvent, RoutedKeyboardEvent, ScreenId, Session, Vector,
};
use edgemouse_protocol::ScreenInfo;
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
const POINTER_BOUNDARY_TOLERANCE: f64 = 1.0;
const POINTER_INTERIOR_INSET: f64 = 1.0;
const LOCAL_TAKEOVER_DISTANCE: f64 = 180.0;
const LOCAL_TAKEOVER_MOTION_GAP_MS: u64 = 300;
const LOCAL_TAKEOVER_ACK_TIMEOUT_MS: u64 = 1_500;

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

#[derive(Debug)]
struct LocalTakeoverGesture {
    edge: Edge,
    progress: f64,
    last_motion_ms: Option<u64>,
    pending: Option<(u64, u64)>,
}

impl LocalTakeoverGesture {
    const fn new(edge: Edge) -> Self {
        Self {
            edge,
            progress: 0.0,
            last_motion_ms: None,
            pending: None,
        }
    }

    fn observe(&mut self, event: PhysicalMouseEvent, now_ms: u64) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let PhysicalMouseEvent::Move { movement } = event else {
            self.progress = 0.0;
            self.last_motion_ms = None;
            return false;
        };
        if self
            .last_motion_ms
            .is_some_and(|last| now_ms.saturating_sub(last) > LOCAL_TAKEOVER_MOTION_GAP_MS)
        {
            self.progress = 0.0;
        }
        self.last_motion_ms = Some(now_ms);

        let (toward, perpendicular) = match self.edge {
            Edge::Left => (-movement.dx, movement.dy),
            Edge::Right => (movement.dx, movement.dy),
            Edge::Top => (-movement.dy, movement.dx),
            Edge::Bottom => (movement.dy, movement.dx),
        };
        if toward <= 0.0 {
            self.progress = (self.progress + toward * 2.0).max(0.0);
            return false;
        }
        self.progress = (self.progress + toward - perpendicular.abs().min(toward) * 0.1).max(0.0);
        if self.progress < LOCAL_TAKEOVER_DISTANCE {
            return false;
        }
        self.progress = 0.0;
        true
    }

    fn mark_requested(&mut self, owner_session_id: u64, now_ms: u64) {
        self.pending = Some((owner_session_id, now_ms));
        self.last_motion_ms = None;
    }

    fn accept_ack(&mut self, owner_session_id: u64) -> bool {
        if !matches!(self.pending, Some((pending, _)) if pending == owner_session_id) {
            return false;
        }
        self.reset();
        true
    }

    fn timed_out(&self, now_ms: u64) -> Option<u64> {
        let (owner_session_id, requested_at) = self.pending?;
        (now_ms.saturating_sub(requested_at) >= LOCAL_TAKEOVER_ACK_TIMEOUT_MS)
            .then_some(owner_session_id)
    }

    fn reset(&mut self) {
        self.progress = 0.0;
        self.last_motion_ms = None;
        self.pending = None;
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
    let _control_server = ControlServer::start(Arc::clone(&stopping))?;
    let mut reconnecting = false;
    let mut backoff = ReconnectBackoff::default();

    loop {
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        let detected = if config.local_screen.automatic {
            let desktop = platform::desktop_geometry()?;
            println!(
                "Detected desktop: {:.0}x{:.0} at ({:.0}, {:.0}), {} display(s), scale {:.2}",
                desktop.bounds.width,
                desktop.bounds.height,
                desktop.bounds.origin.x,
                desktop.bounds.origin.y,
                desktop.display_count,
                desktop.scale_factor
            );
            Some((desktop.bounds, desktop.scale_factor))
        } else {
            None
        };
        let local = config.resolve_local_screen(detected)?;
        let initial_pointer =
            normalize_initial_pointer(local.screen.bounds, platform::current_pointer()?)?;
        let session_id = new_session_id(config.local_node.0);
        if reconnecting {
            println!("Reconnecting to the trusted peer…");
        } else {
            println!("Connecting to the trusted peer…");
        }
        let mut transport = config.transport.clone();
        let mut discovery_responder = None;
        if config.peer_address == PeerAddress::Auto {
            let request = DiscoveryRequest {
                local_node: config.local_node,
                expected_peer: config.peer_node,
                local_name: transport.local_name.clone(),
                quic_port: transport.bind_address.port(),
                timeout: transport.connect_timeout,
            };
            if config.local_node < config.peer_node {
                println!("Discovering the trusted peer on UDP {DISCOVERY_PORT}…");
                match discover_trusted_peer(&request, &stopping) {
                    Ok(discovered) => {
                        transport.peer_address = discovered.address;
                        println!(
                            "Discovered trusted peer {} at {}",
                            discovered.name, discovered.address
                        );
                    }
                    Err(_) if stopping.load(Ordering::Acquire) => return Ok(()),
                    Err(error) => {
                        reconnecting = true;
                        let delay = backoff.next_delay();
                        eprintln!(
                            "Peer discovery failed: {error}; local mouse control remains available; retrying in {} second(s)",
                            delay.as_secs()
                        );
                        if wait_until_retry_or_stop(&stopping, delay) {
                            return Ok(());
                        }
                        continue;
                    }
                }
            } else {
                println!(
                    "Waiting for the trusted peer and answering UDP {DISCOVERY_PORT} discovery…"
                );
                discovery_responder = Some(start_discovery_responder(request)?);
            }
        }
        let local_screen_info = ScreenInfo {
            id: local.screen.id,
            name: local.screen.name.clone(),
            bounds: local.screen.bounds,
            scale_factor: local.screen.scale_factor,
        };
        let network_result = Network::connect(
            transport,
            local_screen_info,
            session_id,
            Arc::clone(&stopping),
        );
        finish_discovery_responder(discovery_responder)?;
        let network = match network_result {
            Ok(network) => network,
            Err(_) if stopping.load(Ordering::Acquire) => return Ok(()),
            Err(error) => {
                reconnecting = true;
                let delay = backoff.next_delay();
                eprintln!(
                    "Connection attempt failed: {error}; local mouse control remains available; retrying in {} second(s)",
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

        let topology = config.topology(local.screen.clone(), &network.peer_screen)?;
        println!(
            "Peer desktop : {:.0}x{:.0} at ({:.0}, {:.0}), scale {:.2}",
            network.peer_screen.bounds.width,
            network.peer_screen.bounds.height,
            network.peer_screen.bounds.origin.x,
            network.peer_screen.bounds.origin.y,
            network.peer_screen.scale_factor
        );

        match run_connected(
            &config,
            &local,
            topology,
            network,
            initial_pointer,
            session_id,
            &stopping,
        )? {
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

type DiscoveryResponder = (Arc<AtomicBool>, std::thread::JoinHandle<Result<(), String>>);

fn start_discovery_responder(
    request: DiscoveryRequest,
) -> Result<DiscoveryResponder, Box<dyn Error>> {
    let stopping = Arc::new(AtomicBool::new(false));
    let thread_stopping = Arc::clone(&stopping);
    let thread = std::thread::Builder::new()
        .name("edgemouse-discovery".to_owned())
        .spawn(move || {
            respond_to_trusted_peer(&request, &thread_stopping).map_err(|error| error.to_string())
        })?;
    Ok((stopping, thread))
}

fn finish_discovery_responder(responder: Option<DiscoveryResponder>) -> Result<(), Box<dyn Error>> {
    let Some((stopping, thread)) = responder else {
        return Ok(());
    };
    stopping.store(true, Ordering::Release);
    match thread.join() {
        Ok(_) => Ok(()),
        Err(_) => Err("UDP discovery responder thread panicked".into()),
    }
}

fn run_connected(
    config: &LoadedConfig,
    local: &ResolvedScreen,
    topology: edgemouse_core::Topology,
    network: Network,
    initial_pointer: Point,
    session_id: u64,
    stopping: &AtomicBool,
) -> Result<ConnectionEnd, Box<dyn Error>> {
    let mut capture = platform::start_capture(local.screen.bounds, local.coordinate_scale)?;
    let mut injector = platform::injector(initial_pointer);
    let mut keyboard_capture = platform::start_keyboard_capture()?;
    let mut keyboard_injector = platform::keyboard_injector();
    let mut session = Session::new(
        config.local_node,
        topology,
        local.screen.id,
        initial_pointer,
        config.session,
    )?;
    let mut remote = RemoteReceiver::new(
        local.screen.id,
        local.screen.bounds,
        config.session.peer_timeout_ms,
    );
    let clock = Instant::now();

    println!(
        "Edge switching active; press Ctrl+C while input is local, or run `edgemouse stop`, to stop"
    );
    let result = run_loop(
        &network,
        &mut capture,
        &mut injector,
        &mut keyboard_capture,
        &mut keyboard_injector,
        &mut session,
        &mut remote,
        config.peer_on,
        session_id,
        stopping,
        clock,
    );

    let effects = session.disconnect_peer(config.peer_node);
    drop(apply_effects(
        effects,
        &network,
        &mut capture,
        &mut keyboard_capture,
        &mut session,
        session_id,
    ));
    if matches!(session.state(), ControlState::Recovering { .. }) {
        drop(session.complete_recovery());
    }
    drop(capture.set_mode(CaptureMode::Local { restore: None }));
    drop(keyboard_capture.set_remote(false));
    drop(injector.release_all());
    drop(keyboard_injector.release_all());
    let _ = remote.reset(&mut injector);

    result
}

fn normalize_initial_pointer(
    local_bounds: Rect,
    initial_pointer: Point,
) -> Result<Point, Box<dyn Error>> {
    let Some(normalized) = normalize_pointer_to_bounds(local_bounds, initial_pointer) else {
        return Err(format!(
            "current pointer ({:.1}, {:.1}) is outside configured local screen bounds; check origin_x, origin_y, width, and height",
            initial_pointer.x, initial_pointer.y
        )
        .into());
    };
    if normalized != initial_pointer {
        eprintln!(
            "current pointer ({:.1}, {:.1}) was on the configured screen boundary; clamped to ({:.1}, {:.1}) for safe reconnect",
            initial_pointer.x, initial_pointer.y, normalized.x, normalized.y
        );
    }
    Ok(normalized)
}

fn normalize_pointer_to_bounds(bounds: Rect, pointer: Point) -> Option<Point> {
    if bounds.contains(pointer) {
        return Some(pointer);
    }
    let is_near_closed_bounds = pointer.is_finite()
        && pointer.x >= bounds.left() - POINTER_BOUNDARY_TOLERANCE
        && pointer.x <= bounds.right() + POINTER_BOUNDARY_TOLERANCE
        && pointer.y >= bounds.top() - POINTER_BOUNDARY_TOLERANCE
        && pointer.y <= bounds.bottom() + POINTER_BOUNDARY_TOLERANCE;
    is_near_closed_bounds.then(|| bounds.clamp_inside(pointer, POINTER_INTERIOR_INSET))
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
    keyboard_capture: &mut platform::NativeKeyboardCapture,
    keyboard_injector: &mut platform::NativeKeyboardInjector,
    session: &mut Session,
    remote: &mut RemoteReceiver,
    takeover_edge: Edge,
    session_id: u64,
    stopping: &AtomicBool,
    clock: Instant,
) -> Result<ConnectionEnd, Box<dyn Error>> {
    let mut takeover = LocalTakeoverGesture::new(takeover_edge);
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
                    if matches!(
                        event.event,
                        RemoteMouseEvent::Enter { .. }
                            | RemoteMouseEvent::Leave
                            | RemoteMouseEvent::ReleaseAll
                    ) {
                        keyboard_injector.release_all()?;
                    }
                    let transition = remote.handle(remote_session, event, injector, now_ms)?;
                    apply_remote_transition(transition, remote.local_screen, capture, session)?;
                    let transition = remote.take_ready_datagram(injector, now_ms)?;
                    apply_remote_transition(transition, remote.local_screen, capture, session)?;
                }
                NetworkEvent::Message(WireMessage::Keyboard {
                    session_id: remote_session,
                    event,
                }) => {
                    remote.handle_keyboard(remote_session, event, keyboard_injector, now_ms)?;
                    let transition = remote.take_ready_datagram(injector, now_ms)?;
                    apply_remote_transition(transition, remote.local_screen, capture, session)?;
                }
                NetworkEvent::Message(WireMessage::ControlReclaim { owner_session_id }) => {
                    if owner_session_id != session_id {
                        return Err("peer requested control from a stale local session".into());
                    }
                    if remote.is_active() {
                        return Err(
                            "peer requested control during simultaneous incoming control".into(),
                        );
                    }
                    if !matches!(
                        session.state(),
                        ControlState::Remote { peer } if peer == network.peer_node
                    ) {
                        return Err(
                            "peer requested control when no outgoing session was active".into()
                        );
                    }
                    let effects = session.yield_remote_control(network.peer_node);
                    apply_effects(
                        effects,
                        network,
                        capture,
                        keyboard_capture,
                        session,
                        session_id,
                    )?;
                    network.send(WireMessage::ControlReclaimAck { owner_session_id })?;
                    println!("Peer physical mouse reclaimed control");
                }
                NetworkEvent::Message(WireMessage::ControlReclaimAck { owner_session_id }) => {
                    if !takeover.accept_ack(owner_session_id) {
                        continue;
                    }
                    if remote.active_session() != Some(owner_session_id) {
                        return Err("peer acknowledged a control session that is not active".into());
                    }
                    keyboard_injector.release_all()?;
                    let position = remote
                        .reset(injector)
                        .ok_or("incoming control had no pointer position to reclaim")?;
                    restore_incoming_control(remote.local_screen, position, capture, session)?;
                    let result = session.handle_input(
                        PhysicalMouseEvent::Move {
                            movement: movement_across_edge(
                                remote.local_bounds,
                                position,
                                takeover_edge,
                            ),
                        },
                        now_ms,
                    )?;
                    apply_effects(
                        result.effects,
                        network,
                        capture,
                        keyboard_capture,
                        session,
                        session_id,
                    )?;
                    println!("Local physical mouse crossed {takeover_edge:?} and took control");
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
                    drop(keyboard_injector.release_all());
                    if let Some(position) = remote.reset(injector) {
                        restore_incoming_control(remote.local_screen, position, capture, session)?;
                    }
                    return Ok(ConnectionEnd::Disconnected(reason));
                }
            }
        }

        if let Some(position) = remote.poll_timeout(injector, now_ms)? {
            keyboard_injector.release_all()?;
            restore_incoming_control(remote.local_screen, position, capture, session)?;
            return Ok(ConnectionEnd::Disconnected(
                "incoming remote control timed out".to_owned(),
            ));
        }

        if let Some(owner_session_id) = takeover.timed_out(now_ms) {
            if remote.active_session() == Some(owner_session_id) {
                keyboard_injector.release_all()?;
                if let Some(position) = remote.reset(injector) {
                    restore_incoming_control(remote.local_screen, position, capture, session)?;
                }
                return Ok(ConnectionEnd::Disconnected(
                    "local physical mouse reclaim was not acknowledged".to_owned(),
                ));
            }
            takeover.reset();
        }

        let timeout_effects = session.poll_timeout(now_ms);
        let peer_timed_out = timeout_effects
            .iter()
            .any(|effect| matches!(effect, Effect::PeerTimedOut { .. }));
        apply_effects(
            timeout_effects,
            network,
            capture,
            keyboard_capture,
            session,
            session_id,
        )?;
        if peer_timed_out {
            return Ok(ConnectionEnd::Disconnected(
                "trusted peer heartbeat timed out".to_owned(),
            ));
        }

        let mut handled_input = false;
        if keyboard_capture.take_emergency_release() {
            let effects = session.disconnect_peer(network.peer_node);
            apply_effects(
                effects,
                network,
                capture,
                keyboard_capture,
                session,
                session_id,
            )?;
            return Ok(ConnectionEnd::Disconnected(
                "emergency keyboard release requested".to_owned(),
            ));
        }
        while let Some(event) = capture.try_next_event()? {
            handled_input = true;
            if remote.is_active() {
                if takeover.observe(event, now_ms) {
                    let owner_session_id = remote
                        .active_session()
                        .expect("remote control was checked as active");
                    network.send(WireMessage::ControlReclaim { owner_session_id })?;
                    takeover.mark_requested(owner_session_id, now_ms);
                    println!(
                        "Local physical mouse pushed toward {takeover_edge:?}; requesting control"
                    );
                }
                continue;
            }
            takeover.reset();
            let result = session.handle_input(event, now_ms)?;
            apply_effects(
                result.effects,
                network,
                capture,
                keyboard_capture,
                session,
                session_id,
            )?;
        }
        while let Some(event) = keyboard_capture.try_next_event()? {
            handled_input = true;
            let result = session.handle_keyboard(event);
            apply_effects(
                result.effects,
                network,
                capture,
                keyboard_capture,
                session,
                session_id,
            )?;
        }
        if !handled_input {
            std::thread::sleep(IDLE_POLL_INTERVAL);
        }
    }
    Ok(ConnectionEnd::Stopped)
}

fn movement_across_edge(bounds: Rect, position: Point, edge: Edge) -> Vector {
    let margin = 2.0;
    match edge {
        Edge::Left => Vector::new(-(position.x - bounds.left() + margin), 0.0),
        Edge::Right => Vector::new(bounds.right() - position.x + margin, 0.0),
        Edge::Top => Vector::new(0.0, -(position.y - bounds.top() + margin)),
        Edge::Bottom => Vector::new(0.0, bounds.bottom() - position.y + margin),
    }
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
    keyboard_capture: &mut platform::NativeKeyboardCapture,
    session: &mut Session,
    session_id: u64,
) -> Result<(), Box<dyn Error>> {
    for effect in effects {
        match effect {
            Effect::CapturePointer { anchor } => {
                keyboard_capture.set_remote(true)?;
                capture.set_mode(CaptureMode::Remote { anchor })?;
                println!("Mouse and keyboard control handed to peer");
            }
            Effect::ReleasePointer {
                restore_position, ..
            } => {
                keyboard_capture.set_remote(false)?;
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
            Effect::SendKeyboard { peer, event } => {
                if peer != network.peer_node {
                    return Err("keyboard routing requested an unknown peer".into());
                }
                network.send(WireMessage::Keyboard { session_id, event })?;
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

    const fn active_session(&self) -> Option<u64> {
        self.active_session
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

    fn handle_keyboard(
        &mut self,
        session_id: u64,
        routed: RoutedKeyboardEvent,
        injector: &mut impl KeyboardInjectionBackend,
        now_ms: u64,
    ) -> Result<(), Box<dyn Error>> {
        if session_id == 0 || routed.sequence == 0 {
            return Err("peer sent a zero keyboard session or sequence number".into());
        }
        if self.active_session != Some(session_id) {
            return Err("peer sent keyboard input before entering the local screen".into());
        }
        if routed.sequence <= self.last_sequence {
            return Ok(());
        }
        injector.inject(routed.event)?;
        self.last_sequence = routed.sequence;
        self.last_reliable_sequence = routed.sequence;
        self.last_activity_ms = Some(now_ms);
        Ok(())
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
    #[cfg(target_os = "windows")]
    platform::install_shutdown_handler(Arc::clone(&stopping))?;
    #[cfg(not(target_os = "windows"))]
    {
        let handler_state = Arc::clone(&stopping);
        ctrlc::set_handler(move || handler_state.store(true, Ordering::Release))?;
    }
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
    use edgemouse_core::{
        ButtonState, KeyCode, KeyState, KeyboardEvent, MouseButton, PermissionState, PlatformError,
        Point,
    };

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

    #[derive(Default)]
    struct FakeKeyboardInjector {
        events: Vec<KeyboardEvent>,
        releases: usize,
    }

    impl KeyboardInjectionBackend for FakeKeyboardInjector {
        fn permission_state(&self) -> PermissionState {
            PermissionState::NotRequired
        }

        fn inject(&mut self, event: KeyboardEvent) -> Result<(), PlatformError> {
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
    fn reconnect_clamps_a_windows_pointer_on_the_closed_right_edge() {
        let bounds = Rect::new(Point::new(0.0, 0.0), 1_920.0, 1_080.0).unwrap();
        let pointer = Point::new(1_920.0, 601.0);

        assert_eq!(
            normalize_pointer_to_bounds(bounds, pointer),
            Some(Point::new(1_919.0, 601.0))
        );
    }

    #[test]
    fn reconnect_rejects_a_pointer_far_outside_the_configured_screen() {
        let bounds = Rect::new(Point::new(0.0, 0.0), 1_920.0, 1_080.0).unwrap();

        assert_eq!(
            normalize_pointer_to_bounds(bounds, Point::new(2_500.0, 601.0)),
            None
        );
    }

    #[test]
    fn local_takeover_requires_a_deliberate_push_toward_the_peer() {
        let mut gesture = LocalTakeoverGesture::new(Edge::Left);
        for (index, dx) in [-50.0, -50.0, -50.0].into_iter().enumerate() {
            assert!(!gesture.observe(
                PhysicalMouseEvent::Move {
                    movement: Vector::new(dx, 0.0),
                },
                index as u64 * 50,
            ));
        }
        assert!(gesture.observe(
            PhysicalMouseEvent::Move {
                movement: Vector::new(-40.0, 0.0),
            },
            150,
        ));
    }

    #[test]
    fn local_takeover_wrong_direction_and_long_pause_reset_progress() {
        let mut gesture = LocalTakeoverGesture::new(Edge::Left);
        assert!(!gesture.observe(
            PhysicalMouseEvent::Move {
                movement: Vector::new(-100.0, 0.0),
            },
            0,
        ));
        assert!(!gesture.observe(
            PhysicalMouseEvent::Move {
                movement: Vector::new(60.0, 0.0),
            },
            50,
        ));
        assert!(!gesture.observe(
            PhysicalMouseEvent::Move {
                movement: Vector::new(-150.0, 0.0),
            },
            500,
        ));
    }

    #[test]
    fn local_takeover_ack_is_session_bound_and_times_out_safely() {
        let mut gesture = LocalTakeoverGesture::new(Edge::Left);
        gesture.mark_requested(77, 100);

        assert!(!gesture.accept_ack(78));
        assert_eq!(gesture.timed_out(1_599), None);
        assert_eq!(gesture.timed_out(1_600), Some(77));
        assert!(gesture.accept_ack(77));
        assert_eq!(gesture.timed_out(u64::MAX), None);
    }

    #[test]
    fn reclaim_crossing_vector_exits_the_configured_peer_edge() {
        let bounds = Rect::new(Point::new(-100.0, 20.0), 300.0, 200.0).unwrap();
        let position = Point::new(75.0, 80.0);

        let left = movement_across_edge(bounds, position, Edge::Left);
        let right = movement_across_edge(bounds, position, Edge::Right);

        assert!(position.x + left.dx < bounds.left());
        assert!(position.x + right.dx > bounds.right());
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
    fn keyboard_requires_an_active_mouse_handoff_and_shares_ordering() {
        let mut receiver = remote_receiver(1_500);
        let mut mouse = FakeInjector::default();
        let mut keyboard = FakeKeyboardInjector::default();
        let key = RoutedKeyboardEvent {
            sequence: 2,
            event: KeyboardEvent {
                key: KeyCode::A,
                state: KeyState::Pressed,
                repeat: false,
            },
        };
        assert!(receiver.handle_keyboard(9, key, &mut keyboard, 5).is_err());
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
                &mut mouse,
                10,
            )
            .unwrap();
        receiver.handle_keyboard(9, key, &mut keyboard, 20).unwrap();
        receiver.handle_keyboard(9, key, &mut keyboard, 21).unwrap();
        assert_eq!(keyboard.events, vec![key.event]);
        assert_eq!(receiver.last_sequence, 2);
        assert_eq!(receiver.last_reliable_sequence, 2);
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
