use edgemouse_core::{NodeId, RemoteMouseEvent, RoutedEvent};
use edgemouse_protocol::{ScreenInfo, WireMessage};
use edgemouse_transport::{PeerConfig, PeerLink};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const COMMAND_CAPACITY: usize = 1_024;
const CONNECT_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const MOUSE_FLUSH_INTERVAL_FAST: Duration = Duration::from_millis(4);
const METRICS_INTERVAL: Duration = Duration::from_secs(5);
const ACTIVE_MOVEMENT_GAP_MAX: Duration = Duration::from_millis(100);
const ARRIVAL_JITTER_WEIGHT: f64 = 1.0 / 16.0;
#[cfg(target_os = "macos")]
const INCOMING_MOVE_CAPACITY: usize = 32;
#[cfg(not(target_os = "macos"))]
const INCOMING_MOVE_CAPACITY: usize = 1;
#[cfg(target_os = "windows")]
const WINDOWS_TIMER_PERIOD_MS: u32 = 1;

#[cfg(target_os = "windows")]
#[link(name = "Winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

struct TimerResolution;

impl TimerResolution {
    #[cfg(target_os = "windows")]
    fn request() -> Result<Self, String> {
        // SAFETY: timeBeginPeriod takes one integer value and retains no Rust memory.
        if unsafe { timeBeginPeriod(WINDOWS_TIMER_PERIOD_MS) } == 0 {
            Ok(Self)
        } else {
            Err("Windows refused the 1 ms network timer resolution".to_owned())
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn request() -> Result<Self, String> {
        Ok(Self)
    }
}

#[cfg(target_os = "windows")]
impl Drop for TimerResolution {
    fn drop(&mut self) {
        // SAFETY: This balances the one successful timeBeginPeriod call in request().
        unsafe { timeEndPeriod(WINDOWS_TIMER_PERIOD_MS) };
    }
}

pub enum NetworkEvent {
    Message(WireMessage),
    Datagram {
        session_id: u64,
        after_sequence: u64,
        event: RoutedEvent,
        received_at: Instant,
    },
    Metrics {
        rtt_ms: f64,
        send_interval_ms: u64,
        sent_moves: u64,
        skipped_moves: u64,
        coalesced_moves: u64,
        received_moves: u64,
        stale_moves: u64,
        arrival_jitter_ms: f64,
        max_arrival_gap_ms: f64,
        superseded_moves: u64,
    },
    Disconnected(String),
}

enum NetworkCommand {
    Message(WireMessage),
    Shutdown,
}

pub struct Network {
    commands: mpsc::Sender<NetworkCommand>,
    events: std_mpsc::Receiver<NetworkEvent>,
    pending_move: Arc<Mutex<MoveCoalescer>>,
    incoming_move: Arc<Mutex<IncomingMoveCoalescer>>,
    thread: Option<JoinHandle<()>>,
    pub peer_node: NodeId,
    pub peer_name: String,
    pub peer_screen: ScreenInfo,
}

#[derive(Default)]
struct MoveCoalescer {
    pending: Option<WireMessage>,
    coalesced: u64,
}

#[derive(Default)]
struct IncomingMoveCoalescer {
    pending: VecDeque<BufferedIncomingMove>,
    last_forwarded_sequence: u64,
    received: u64,
    stale: u64,
    last_arrival: Option<Instant>,
    previous_gap: Option<Duration>,
    arrival_jitter_ms: f64,
    max_arrival_gap_ms: f64,
    superseded: u64,
}

#[derive(Debug)]
struct BufferedIncomingMove {
    message: WireMessage,
    received_at: Instant,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct IncomingMoveMetrics {
    received: u64,
    stale: u64,
    arrival_jitter_ms: f64,
    max_arrival_gap_ms: f64,
    superseded: u64,
}

impl MoveCoalescer {
    fn push(&mut self, message: WireMessage) {
        if self.pending.replace(message).is_some() {
            self.coalesced = self.coalesced.saturating_add(1);
        }
    }

    fn take(&mut self) -> Option<WireMessage> {
        self.pending.take()
    }

    fn take_coalesced(&mut self) -> u64 {
        let count = self.coalesced;
        self.coalesced = 0;
        count
    }
}

impl IncomingMoveCoalescer {
    fn push(&mut self, message: WireMessage) {
        self.push_at(message, Instant::now());
    }

    fn push_at(&mut self, message: WireMessage, now: Instant) {
        let sequence = mouse_datagram_sequence(&message)
            .expect("incoming coalescer only accepts movement datagrams");
        self.received = self.received.saturating_add(1);
        self.note_arrival(now);
        let pending_sequence = self
            .pending
            .back()
            .and_then(|pending| mouse_datagram_sequence(&pending.message))
            .unwrap_or(0);
        if sequence <= self.last_forwarded_sequence || sequence <= pending_sequence {
            self.stale = self.stale.saturating_add(1);
            return;
        }
        if self.pending.len() == INCOMING_MOVE_CAPACITY {
            drop(self.pending.pop_front());
            self.superseded = self.superseded.saturating_add(1);
        }
        self.pending.push_back(BufferedIncomingMove {
            message,
            received_at: now,
        });
    }

    fn note_arrival(&mut self, now: Instant) {
        if let Some(last_arrival) = self.last_arrival {
            let gap = now.saturating_duration_since(last_arrival);
            if gap <= ACTIVE_MOVEMENT_GAP_MAX {
                let gap_ms = gap.as_secs_f64() * 1_000.0;
                self.max_arrival_gap_ms = self.max_arrival_gap_ms.max(gap_ms);
                if let Some(previous_gap) = self.previous_gap {
                    let previous_ms = previous_gap.as_secs_f64() * 1_000.0;
                    let variation = (gap_ms - previous_ms).abs();
                    self.arrival_jitter_ms +=
                        (variation - self.arrival_jitter_ms) * ARRIVAL_JITTER_WEIGHT;
                }
                self.previous_gap = Some(gap);
            } else {
                self.previous_gap = None;
            }
        }
        self.last_arrival = Some(now);
    }

    fn take(&mut self) -> Option<BufferedIncomingMove> {
        let buffered = self.pending.pop_front()?;
        self.last_forwarded_sequence = mouse_datagram_sequence(&buffered.message)
            .expect("incoming coalescer only contains movement datagrams");
        Some(buffered)
    }

    fn take_metrics(&mut self) -> IncomingMoveMetrics {
        let metrics = IncomingMoveMetrics {
            received: self.received,
            stale: self.stale,
            arrival_jitter_ms: self.arrival_jitter_ms,
            max_arrival_gap_ms: self.max_arrival_gap_ms,
            superseded: self.superseded,
        };
        self.received = 0;
        self.stale = 0;
        self.arrival_jitter_ms = 0.0;
        self.max_arrival_gap_ms = 0.0;
        self.superseded = 0;
        metrics
    }
}

impl Network {
    pub fn connect(
        config: PeerConfig,
        local_screen: ScreenInfo,
        session_id: u64,
        stopping: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let (commands_sender, commands_receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (event_sender, event_receiver) = std_mpsc::channel();
        let (startup_sender, startup_receiver) = std_mpsc::sync_channel(1);
        let pending_move = Arc::new(Mutex::new(MoveCoalescer::default()));
        let network_pending_move = Arc::clone(&pending_move);
        let incoming_move = Arc::new(Mutex::new(IncomingMoveCoalescer::default()));
        let network_incoming_move = Arc::clone(&incoming_move);
        let thread = std::thread::Builder::new()
            .name("edgemouse-network".to_owned())
            .spawn(move || {
                let _timer_resolution = match TimerResolution::request() {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        drop(startup_sender.send(Err(error)));
                        return;
                    }
                };
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        drop(
                            startup_sender
                                .send(Err(format!("failed to start network runtime: {error}"))),
                        );
                        return;
                    }
                };
                runtime.block_on(async move {
                    let link = tokio::select! {
                        result = PeerLink::connect(config, local_screen) => match result {
                            Ok(link) => link,
                            Err(error) => {
                                drop(startup_sender.send(Err(error.to_string())));
                                return;
                            }
                        },
                        () = wait_for_cancellation(&stopping) => {
                            drop(startup_sender.send(Err("connection cancelled".to_owned())));
                            return;
                        }
                    };
                    let peer_node = link.peer_node();
                    let peer_name = link.peer_name().to_owned();
                    let peer_screen = link.peer_screen().clone();
                    if startup_sender
                        .send(Ok((peer_node, peer_name, peer_screen)))
                        .is_err()
                    {
                        return;
                    }
                    run_network(
                        link,
                        commands_receiver,
                        event_sender,
                        network_pending_move,
                        network_incoming_move,
                        session_id,
                    )
                    .await;
                });
            })
            .map_err(|error| format!("failed to start network thread: {error}"))?;

        match startup_receiver.recv() {
            Ok(Ok((peer_node, peer_name, peer_screen))) => Ok(Self {
                commands: commands_sender,
                events: event_receiver,
                pending_move,
                incoming_move,
                thread: Some(thread),
                peer_node,
                peer_name,
                peer_screen,
            }),
            Ok(Err(error)) => {
                drop(thread.join());
                Err(error)
            }
            Err(_) => {
                drop(thread.join());
                Err("network thread exited during startup".to_owned())
            }
        }
    }

    pub fn send(&self, message: WireMessage) -> Result<(), String> {
        if is_coalescible_move(&message) {
            self.pending_move
                .lock()
                .map_err(|_| "mouse movement buffer lock was poisoned".to_owned())?
                .push(message);
            return Ok(());
        }

        let pending = self
            .pending_move
            .lock()
            .map_err(|_| "mouse movement buffer lock was poisoned".to_owned())?
            .take();
        if let Some(pending) = pending {
            self.send_immediate(pending)?;
        }
        self.send_immediate(message)
    }

    fn send_immediate(&self, message: WireMessage) -> Result<(), String> {
        self.commands
            .try_send(NetworkCommand::Message(message))
            .map_err(|error| format!("network queue unavailable: {error}"))
    }

    pub fn try_receive(&self) -> Result<Option<NetworkEvent>, String> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(std_mpsc::TryRecvError::Empty) => {
                let buffered = self
                    .incoming_move
                    .lock()
                    .map_err(|_| "incoming mouse buffer lock was poisoned".to_owned())?
                    .take();
                match buffered {
                    Some(BufferedIncomingMove {
                        message:
                            WireMessage::MouseDatagram {
                                session_id,
                                after_sequence,
                                sequence,
                                screen,
                                position,
                            },
                        received_at,
                    }) => Ok(Some(NetworkEvent::Datagram {
                        session_id,
                        after_sequence,
                        event: RoutedEvent {
                            sequence,
                            event: RemoteMouseEvent::MoveAbsolute { screen, position },
                        },
                        received_at,
                    })),
                    Some(_) => {
                        Err("incoming mouse buffer contained a non-datagram message".to_owned())
                    }
                    None => Ok(None),
                }
            }
            Err(std_mpsc::TryRecvError::Disconnected) => {
                Err("network event channel disconnected".to_owned())
            }
        }
    }
}

async fn wait_for_cancellation(stopping: &AtomicBool) {
    while !stopping.load(Ordering::Acquire) {
        tokio::time::sleep(CONNECT_CANCELLATION_POLL_INTERVAL).await;
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        drop(self.commands.try_send(NetworkCommand::Shutdown));
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

async fn run_network(
    link: PeerLink,
    mut commands: mpsc::Receiver<NetworkCommand>,
    events: std_mpsc::Sender<NetworkEvent>,
    pending_move: Arc<Mutex<MoveCoalescer>>,
    incoming_move: Arc<Mutex<IncomingMoveCoalescer>>,
    session_id: u64,
) {
    let (mut sender, mut receiver, datagrams) = link.split();
    let started = tokio::time::Instant::now();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mouse_flush = tokio::time::sleep(MOUSE_FLUSH_INTERVAL_FAST);
    tokio::pin!(mouse_flush);
    let mut metrics = tokio::time::interval_at(
        tokio::time::Instant::now() + METRICS_INTERVAL,
        METRICS_INTERVAL,
    );
    metrics.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sent_moves = 0_u64;
    let mut skipped_moves = 0_u64;
    let mut last_reliable_sequence = 0_u64;

    loop {
        tokio::select! {
            biased;
            _ = heartbeat.tick() => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                if let Err(error) = sender.send(&WireMessage::Heartbeat {
                    session_id,
                    monotonic_ms: elapsed,
                }).await {
                    drop(events.send(NetworkEvent::Disconnected(error.to_string())));
                    break;
                }
            }
            command = commands.recv() => match command {
                Some(NetworkCommand::Message(message)) => {
                    let is_move = is_coalescible_move(&message);
                    if let Err(error) = sender.send(&message).await {
                        drop(events.send(NetworkEvent::Disconnected(error.to_string())));
                        break;
                    }
                    if let Some(sequence) = reliable_mouse_sequence(&message) {
                        last_reliable_sequence = sequence;
                    }
                    if is_move {
                        sent_moves = sent_moves.saturating_add(1);
                    }
                }
                Some(NetworkCommand::Shutdown) | None => {
                    drop(sender.send(&WireMessage::Goodbye { session_id }).await);
                    sender.close(b"normal shutdown");
                    break;
                }
            },
            _ = &mut mouse_flush => {
                let message = match pending_move.lock() {
                    Ok(mut pending) => pending.take(),
                    Err(_) => {
                        drop(events.send(NetworkEvent::Disconnected(
                            "mouse movement buffer lock was poisoned".to_owned(),
                        )));
                        break;
                    }
                };
                if let Some(message) = message {
                    let message = match into_mouse_datagram(message, last_reliable_sequence) {
                        Ok(message) => message,
                        Err(error) => {
                            drop(events.send(NetworkEvent::Disconnected(error)));
                            break;
                        }
                    };
                    match datagrams.send(&message) {
                        Ok(true) => sent_moves = sent_moves.saturating_add(1),
                        Ok(false) => skipped_moves = skipped_moves.saturating_add(1),
                        Err(error) => {
                            drop(events.send(NetworkEvent::Disconnected(error.to_string())));
                            break;
                        }
                    }
                }
                mouse_flush.as_mut().reset(
                    tokio::time::Instant::now()
                        + adaptive_mouse_interval(sender.smoothed_rtt()),
                );
            },
            _ = metrics.tick() => {
                let coalesced_moves = match pending_move.lock() {
                    Ok(mut pending) => pending.take_coalesced(),
                    Err(_) => {
                        drop(events.send(NetworkEvent::Disconnected(
                            "mouse movement buffer lock was poisoned".to_owned(),
                        )));
                        break;
                    }
                };
                let incoming_metrics = match incoming_move.lock() {
                    Ok(mut pending) => pending.take_metrics(),
                    Err(_) => {
                        drop(events.send(NetworkEvent::Disconnected(
                            "incoming mouse buffer lock was poisoned".to_owned(),
                        )));
                        break;
                    }
                };
                let rtt_ms = sender.smoothed_rtt().as_secs_f64() * 1_000.0;
                let send_interval_ms = u64::try_from(
                    adaptive_mouse_interval(sender.smoothed_rtt()).as_millis(),
                ).unwrap_or(u64::MAX);
                if events.send(NetworkEvent::Metrics {
                    rtt_ms,
                    send_interval_ms,
                    sent_moves,
                    skipped_moves,
                    coalesced_moves,
                    received_moves: incoming_metrics.received,
                    stale_moves: incoming_metrics.stale,
                    arrival_jitter_ms: incoming_metrics.arrival_jitter_ms,
                    max_arrival_gap_ms: incoming_metrics.max_arrival_gap_ms,
                    superseded_moves: incoming_metrics.superseded,
                }).is_err() {
                    break;
                }
                sent_moves = 0;
                skipped_moves = 0;
            },
            message = receiver.receive() => match message {
                Ok(WireMessage::Goodbye { .. }) => {
                    drop(events.send(NetworkEvent::Disconnected("peer shut down".to_owned())));
                    break;
                }
                Ok(WireMessage::MouseDatagram { .. }) => {
                    drop(events.send(NetworkEvent::Disconnected(
                        "peer sent a movement datagram on the reliable stream".to_owned(),
                    )));
                    break;
                }
                Ok(message) => {
                    if events.send(NetworkEvent::Message(message)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    drop(events.send(NetworkEvent::Disconnected(error.to_string())));
                    break;
                }
            },
            message = datagrams.receive() => match message {
                Ok(message @ WireMessage::MouseDatagram { .. }) => {
                    match incoming_move.lock() {
                        Ok(mut pending) => pending.push(message),
                        Err(_) => {
                            drop(events.send(NetworkEvent::Disconnected(
                                "incoming mouse buffer lock was poisoned".to_owned(),
                            )));
                            break;
                        }
                    }
                }
                Ok(_) => {
                    drop(events.send(NetworkEvent::Disconnected(
                        "peer sent a non-movement QUIC datagram".to_owned(),
                    )));
                    break;
                }
                Err(error) => {
                    drop(events.send(NetworkEvent::Disconnected(error.to_string())));
                    break;
                }
            }
        }
    }
}

fn into_mouse_datagram(message: WireMessage, after_sequence: u64) -> Result<WireMessage, String> {
    match message {
        WireMessage::Mouse {
            session_id,
            event:
                RoutedEvent {
                    sequence,
                    event: RemoteMouseEvent::MoveAbsolute { screen, position },
                },
        } => Ok(WireMessage::MouseDatagram {
            session_id,
            after_sequence,
            sequence,
            screen,
            position,
        }),
        _ => Err("mouse datagram buffer contained a non-movement message".to_owned()),
    }
}

fn reliable_mouse_sequence(message: &WireMessage) -> Option<u64> {
    match message {
        WireMessage::Mouse { event, .. } => Some(event.sequence),
        WireMessage::Keyboard { event, .. } => Some(event.sequence),
        _ => None,
    }
}

fn mouse_datagram_sequence(message: &WireMessage) -> Option<u64> {
    match message {
        WireMessage::MouseDatagram { sequence, .. } => Some(*sequence),
        _ => None,
    }
}

fn adaptive_mouse_interval(rtt: Duration) -> Duration {
    if rtt <= Duration::from_millis(25) {
        Duration::from_millis(4)
    } else if rtt <= Duration::from_millis(75) {
        Duration::from_millis(6)
    } else if rtt <= Duration::from_millis(150) {
        Duration::from_millis(8)
    } else {
        Duration::from_millis(12)
    }
}

fn is_coalescible_move(message: &WireMessage) -> bool {
    matches!(
        message,
        WireMessage::Mouse {
            event: edgemouse_core::RoutedEvent {
                event: RemoteMouseEvent::MoveAbsolute { .. },
                ..
            },
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgemouse_core::{Point, RoutedEvent, ScreenId};

    fn movement(sequence: u64, x: f64) -> WireMessage {
        WireMessage::Mouse {
            session_id: 7,
            event: RoutedEvent {
                sequence,
                event: RemoteMouseEvent::MoveAbsolute {
                    screen: ScreenId(3),
                    position: Point::new(x, 20.0),
                },
            },
        }
    }

    fn movement_datagram(sequence: u64, x: f64) -> WireMessage {
        WireMessage::MouseDatagram {
            session_id: 7,
            after_sequence: 1,
            sequence,
            screen: ScreenId(3),
            position: Point::new(x, 20.0),
        }
    }

    #[test]
    fn coalescer_keeps_only_the_latest_absolute_move() {
        let mut coalescer = MoveCoalescer::default();
        coalescer.push(movement(1, 10.0));
        coalescer.push(movement(2, 20.0));

        assert_eq!(coalescer.take(), Some(movement(2, 20.0)));
        assert_eq!(coalescer.take_coalesced(), 1);
        assert_eq!(coalescer.take_coalesced(), 0);
    }

    #[test]
    fn only_absolute_moves_are_coalescible() {
        assert!(is_coalescible_move(&movement(1, 10.0)));
        assert!(!is_coalescible_move(&WireMessage::Heartbeat {
            session_id: 7,
            monotonic_ms: 10,
        }));
    }

    #[test]
    fn movement_datagram_carries_the_reliable_ordering_watermark() {
        assert_eq!(
            into_mouse_datagram(movement(9, 12.5), 7).unwrap(),
            WireMessage::MouseDatagram {
                session_id: 7,
                after_sequence: 7,
                sequence: 9,
                screen: ScreenId(3),
                position: Point::new(12.5, 20.0),
            }
        );
    }

    #[test]
    fn incoming_coalescer_never_lets_a_late_packet_reverse_newer_motion() {
        let mut coalescer = IncomingMoveCoalescer::default();

        coalescer.push(movement_datagram(10, 120.0));
        coalescer.push(movement_datagram(11, 110.0));
        coalescer.push(movement_datagram(9, 130.0));
        if INCOMING_MOVE_CAPACITY > 1 {
            assert_eq!(
                coalescer.take().map(|buffered| buffered.message),
                Some(movement_datagram(10, 120.0))
            );
        }
        assert_eq!(
            coalescer.take().map(|buffered| buffered.message),
            Some(movement_datagram(11, 110.0))
        );

        coalescer.push(movement_datagram(10, 120.0));
        assert!(coalescer.take().is_none());
        let metrics = coalescer.take_metrics();
        assert_eq!(metrics.received, 4);
        assert_eq!(metrics.stale, 2);
        assert_eq!(metrics.superseded, u64::from(INCOMING_MOVE_CAPACITY == 1));
        let reset = coalescer.take_metrics();
        assert_eq!(reset.received, 0);
        assert_eq!(reset.stale, 0);
        assert_eq!(reset.arrival_jitter_ms, 0.0);
        assert_eq!(reset.max_arrival_gap_ms, 0.0);
        assert_eq!(reset.superseded, 0);
    }

    #[test]
    fn incoming_coalescer_reports_active_movement_jitter() {
        let start = Instant::now();
        let mut coalescer = IncomingMoveCoalescer::default();
        coalescer.push_at(movement_datagram(1, 1.0), start);
        coalescer.push_at(movement_datagram(2, 2.0), start + Duration::from_millis(4));
        coalescer.push_at(movement_datagram(3, 3.0), start + Duration::from_millis(8));
        coalescer.push_at(movement_datagram(4, 4.0), start + Duration::from_millis(20));

        let metrics = coalescer.take_metrics();
        assert_eq!(metrics.received, 4);
        assert_eq!(metrics.stale, 0);
        assert!((metrics.arrival_jitter_ms - 0.5).abs() < f64::EPSILON);
        assert_eq!(metrics.max_arrival_gap_ms, 12.0);
    }

    #[test]
    fn incoming_buffer_keeps_a_short_ordered_history() {
        let start = Instant::now();
        let mut coalescer = IncomingMoveCoalescer::default();
        for sequence in 1..=u64::try_from(INCOMING_MOVE_CAPACITY + 2).unwrap() {
            coalescer.push_at(
                movement_datagram(sequence, sequence as f64),
                start + Duration::from_millis(sequence),
            );
        }

        let first = coalescer
            .take()
            .expect("the buffer should contain movement");
        assert_eq!(mouse_datagram_sequence(&first.message), Some(3));
        assert_eq!(first.received_at, start + Duration::from_millis(3));
        assert_eq!(coalescer.take_metrics().superseded, 2);
    }

    #[test]
    fn mouse_send_interval_adapts_to_rtt() {
        assert_eq!(
            adaptive_mouse_interval(Duration::from_millis(10)),
            Duration::from_millis(4)
        );
        assert_eq!(
            adaptive_mouse_interval(Duration::from_millis(50)),
            Duration::from_millis(6)
        );
        assert_eq!(
            adaptive_mouse_interval(Duration::from_millis(100)),
            Duration::from_millis(8)
        );
        assert_eq!(
            adaptive_mouse_interval(Duration::from_millis(200)),
            Duration::from_millis(12)
        );
    }
}
