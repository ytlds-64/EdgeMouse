use edgemouse_core::{NodeId, RemoteMouseEvent, RoutedEvent};
use edgemouse_protocol::WireMessage;
use edgemouse_transport::{PeerConfig, PeerLink};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::mpsc;

const COMMAND_CAPACITY: usize = 1_024;
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const MOUSE_FLUSH_INTERVAL_FAST: Duration = Duration::from_millis(4);
const METRICS_INTERVAL: Duration = Duration::from_secs(5);

pub enum NetworkEvent {
    Message(WireMessage),
    Datagram {
        session_id: u64,
        after_sequence: u64,
        event: RoutedEvent,
    },
    Metrics {
        rtt_ms: f64,
        send_interval_ms: u64,
        sent_moves: u64,
        coalesced_moves: u64,
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
    incoming_move: Arc<Mutex<Option<WireMessage>>>,
    thread: Option<JoinHandle<()>>,
    pub peer_node: NodeId,
    pub peer_name: String,
}

#[derive(Default)]
struct MoveCoalescer {
    pending: Option<WireMessage>,
    coalesced: u64,
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

impl Network {
    pub fn connect(config: PeerConfig, session_id: u64) -> Result<Self, String> {
        let (commands_sender, commands_receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (event_sender, event_receiver) = std_mpsc::channel();
        let (startup_sender, startup_receiver) = std_mpsc::sync_channel(1);
        let pending_move = Arc::new(Mutex::new(MoveCoalescer::default()));
        let network_pending_move = Arc::clone(&pending_move);
        let incoming_move = Arc::new(Mutex::new(None));
        let network_incoming_move = Arc::clone(&incoming_move);
        let thread = std::thread::Builder::new()
            .name("edgemouse-network".to_owned())
            .spawn(move || {
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
                    let link = match PeerLink::connect(config).await {
                        Ok(link) => link,
                        Err(error) => {
                            drop(startup_sender.send(Err(error.to_string())));
                            return;
                        }
                    };
                    let peer_node = link.peer_node();
                    let peer_name = link.peer_name().to_owned();
                    if startup_sender.send(Ok((peer_node, peer_name))).is_err() {
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
            Ok(Ok((peer_node, peer_name))) => Ok(Self {
                commands: commands_sender,
                events: event_receiver,
                pending_move,
                incoming_move,
                thread: Some(thread),
                peer_node,
                peer_name,
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
                let message = self
                    .incoming_move
                    .lock()
                    .map_err(|_| "incoming mouse buffer lock was poisoned".to_owned())?
                    .take();
                match message {
                    Some(WireMessage::MouseDatagram {
                        session_id,
                        after_sequence,
                        sequence,
                        screen,
                        position,
                    }) => Ok(Some(NetworkEvent::Datagram {
                        session_id,
                        after_sequence,
                        event: RoutedEvent {
                            sequence,
                            event: RemoteMouseEvent::MoveAbsolute { screen, position },
                        },
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
    incoming_move: Arc<Mutex<Option<WireMessage>>>,
    session_id: u64,
) {
    let (mut sender, mut receiver, datagrams) = link.split();
    let started = tokio::time::Instant::now();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mouse_flush = tokio::time::sleep(MOUSE_FLUSH_INTERVAL_FAST);
    tokio::pin!(mouse_flush);
    let mut metrics = tokio::time::interval(METRICS_INTERVAL);
    metrics.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sent_moves = 0_u64;
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
                    if let Err(error) = datagrams.send(&message) {
                        drop(events.send(NetworkEvent::Disconnected(error.to_string())));
                        break;
                    }
                    sent_moves = sent_moves.saturating_add(1);
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
                if sent_moves > 0 || coalesced_moves > 0 {
                    let rtt_ms = sender.smoothed_rtt().as_secs_f64() * 1_000.0;
                    let send_interval_ms = u64::try_from(
                        adaptive_mouse_interval(sender.smoothed_rtt()).as_millis(),
                    ).unwrap_or(u64::MAX);
                    if events.send(NetworkEvent::Metrics {
                        rtt_ms,
                        send_interval_ms,
                        sent_moves,
                        coalesced_moves,
                    }).is_err() {
                        break;
                    }
                    sent_moves = 0;
                }
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
                        Ok(mut pending) => *pending = Some(message),
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
