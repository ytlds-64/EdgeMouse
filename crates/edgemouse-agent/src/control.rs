use edgemouse_core::Edge;
use std::error::Error;
use std::fmt;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CONTROL_ADDRESS: &str = "127.0.0.1:43894";
const REQUEST_MAGIC: &[u8; 8] = b"EDGMCTL1";
const RESPONSE_MAGIC: &[u8; 8] = b"EDGMACK1";
const REQUEST_LENGTH: usize = REQUEST_MAGIC.len() + 1;
const SCROLL_REQUEST_LENGTH: usize = REQUEST_LENGTH + 1;
const LAYOUT_REQUEST_LENGTH: usize = REQUEST_LENGTH + 1;
const RESPONSE_HEADER_LENGTH: usize = RESPONSE_MAGIC.len() + 1 + 4 + 1;
const MAX_VERSION_LENGTH: usize = 63;
const MAX_PEER_NAME_LENGTH: usize = 63;
const TELEMETRY_VERSION: u8 = 1;
const TELEMETRY_FIXED_LENGTH: usize = 1 + 1 + 1 + 4 + 8 + 8 + 4 + 4 + 4 + (8 * 6) + 1;
const MAX_RESPONSE_LENGTH: usize =
    RESPONSE_HEADER_LENGTH + MAX_VERSION_LENGTH + TELEMETRY_FIXED_LENGTH + MAX_PEER_NAME_LENGTH;
const CLIENT_TIMEOUT: Duration = Duration::from_millis(500);
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(200);
const FLAG_CONNECTED_SINCE: u8 = 1 << 0;
const FLAG_LINK_METRICS: u8 = 1 << 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Command {
    Status = 1,
    Stop = 2,
    SetScroll = 3,
    SetLayout = 4,
}

impl Command {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            value if value == Self::Status as u8 => Some(Self::Status),
            value if value == Self::Stop as u8 => Some(Self::Stop),
            value if value == Self::SetScroll as u8 => Some(Self::SetScroll),
            value if value == Self::SetLayout as u8 => Some(Self::SetLayout),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollSettings {
    pub reverse_horizontal: bool,
    pub reverse_vertical: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControlRequest {
    command: Command,
    scroll: Option<ScrollSettings>,
    peer_on: Option<Edge>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectionPhase {
    #[default]
    Starting = 0,
    Connecting = 1,
    Connected = 2,
    Reconnecting = 3,
}

impl ConnectionPhase {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            value if value == Self::Starting as u8 => Some(Self::Starting),
            value if value == Self::Connecting as u8 => Some(Self::Connecting),
            value if value == Self::Connected as u8 => Some(Self::Connected),
            value if value == Self::Reconnecting as u8 => Some(Self::Reconnecting),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectionTelemetry {
    pub phase: ConnectionPhase,
    pub peer_name: Option<String>,
    pub connected_since_unix_ms: Option<u64>,
    pub metrics_updated_unix_ms: Option<u64>,
    pub reconnect_count: u32,
    pub rtt_ms: Option<f32>,
    pub jitter_ms: Option<f32>,
    pub send_interval_ms: Option<u32>,
    pub sent_moves: u64,
    pub skipped_moves: u64,
    pub coalesced_moves: u64,
    pub received_moves: u64,
    pub stale_moves: u64,
    pub superseded_moves: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkMetrics {
    pub rtt_ms: f64,
    pub jitter_ms: f64,
    pub send_interval_ms: u64,
    pub sent_moves: u64,
    pub skipped_moves: u64,
    pub coalesced_moves: u64,
    pub received_moves: u64,
    pub stale_moves: u64,
    pub superseded_moves: u64,
}

#[derive(Clone)]
pub struct RuntimeTelemetry {
    inner: Arc<Mutex<ConnectionTelemetry>>,
    reverse_scroll_horizontal: Arc<AtomicBool>,
    reverse_scroll_vertical: Arc<AtomicBool>,
    pending_layout: Arc<Mutex<Option<Edge>>>,
}

impl Default for RuntimeTelemetry {
    fn default() -> Self {
        Self::with_scroll_settings(false, false)
    }
}

impl RuntimeTelemetry {
    pub fn with_scroll_settings(reverse_horizontal: bool, reverse_vertical: bool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ConnectionTelemetry::default())),
            reverse_scroll_horizontal: Arc::new(AtomicBool::new(reverse_horizontal)),
            reverse_scroll_vertical: Arc::new(AtomicBool::new(reverse_vertical)),
            pending_layout: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_scroll_settings(&self, settings: ScrollSettings) {
        self.reverse_scroll_horizontal
            .store(settings.reverse_horizontal, Ordering::Release);
        self.reverse_scroll_vertical
            .store(settings.reverse_vertical, Ordering::Release);
    }

    pub fn scroll_settings(&self) -> ScrollSettings {
        ScrollSettings {
            reverse_horizontal: self.reverse_scroll_horizontal.load(Ordering::Acquire),
            reverse_vertical: self.reverse_scroll_vertical.load(Ordering::Acquire),
        }
    }

    pub fn request_layout_update(&self, peer_on: Edge) {
        let mut pending = self
            .pending_layout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *pending = Some(peer_on);
    }

    pub fn layout_update(&self) -> Option<Edge> {
        *self
            .pending_layout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn complete_layout_update(&self, peer_on: Edge) {
        let mut pending = self
            .pending_layout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *pending == Some(peer_on) {
            *pending = None;
        }
    }

    pub fn discard_layout_update(&self) {
        let mut pending = self
            .pending_layout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *pending = None;
    }

    pub fn begin_connecting(&self, reconnecting: bool) {
        let mut status = self.lock();
        status.phase = if reconnecting {
            ConnectionPhase::Reconnecting
        } else {
            ConnectionPhase::Connecting
        };
    }

    pub fn connected(&self, peer_name: &str) {
        let mut status = self.lock();
        status.phase = ConnectionPhase::Connected;
        status.peer_name = Some(peer_name.to_owned());
        status.connected_since_unix_ms = Some(unix_time_ms());
        clear_link_metrics(&mut status);
    }

    pub fn disconnected(&self) {
        let mut status = self.lock();
        if status.phase == ConnectionPhase::Connected {
            status.reconnect_count = status.reconnect_count.saturating_add(1);
        }
        status.phase = ConnectionPhase::Reconnecting;
        status.connected_since_unix_ms = None;
        clear_link_metrics(&mut status);
    }

    pub fn update_link(&self, metrics: LinkMetrics) {
        let mut status = self.lock();
        status.metrics_updated_unix_ms = Some(unix_time_ms());
        status.rtt_ms = finite_f32(metrics.rtt_ms);
        status.jitter_ms = finite_f32(metrics.jitter_ms);
        status.send_interval_ms = Some(u32::try_from(metrics.send_interval_ms).unwrap_or(u32::MAX));
        status.sent_moves = metrics.sent_moves;
        status.skipped_moves = metrics.skipped_moves;
        status.coalesced_moves = metrics.coalesced_moves;
        status.received_moves = metrics.received_moves;
        status.stale_moves = metrics.stale_moves;
        status.superseded_moves = metrics.superseded_moves;
    }

    fn snapshot(&self) -> ConnectionTelemetry {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConnectionTelemetry> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn clear_link_metrics(status: &mut ConnectionTelemetry) {
    status.metrics_updated_unix_ms = None;
    status.rtt_ms = None;
    status.jitter_ms = None;
    status.send_interval_ms = None;
    status.sent_moves = 0;
    status.skipped_moves = 0;
    status.coalesced_moves = 0;
    status.received_moves = 0;
    status.stale_moves = 0;
    status.superseded_moves = 0;
}

fn finite_f32(value: f64) -> Option<f32> {
    value
        .is_finite()
        .then(|| value.clamp(0.0, f64::from(f32::MAX)) as f32)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunningStatus {
    pub process_id: u32,
    pub version: String,
    pub connection: ConnectionTelemetry,
}

#[derive(Debug)]
pub struct ControlError(String);

impl ControlError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ControlError {}

pub struct ControlServer {
    address: SocketAddr,
    closing: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    pub fn start(
        stopping: Arc<AtomicBool>,
        telemetry: RuntimeTelemetry,
    ) -> Result<Self, ControlError> {
        let address = CONTROL_ADDRESS
            .parse()
            .map_err(|error| ControlError::new(format!("invalid control address: {error}")))?;
        Self::start_on(address, stopping, telemetry)
    }

    fn start_on(
        address: SocketAddr,
        stopping: Arc<AtomicBool>,
        telemetry: RuntimeTelemetry,
    ) -> Result<Self, ControlError> {
        let socket = UdpSocket::bind(address).map_err(|error| {
            if error.kind() == io::ErrorKind::AddrInUse {
                ControlError::new(
                    "another EdgeMouse instance is already running (the local control port is in use)",
                )
            } else {
                ControlError::new(format!(
                    "failed to open local control channel on {address}: {error}"
                ))
            }
        })?;
        let address = socket.local_addr().map_err(|error| {
            ControlError::new(format!("failed to inspect local control channel: {error}"))
        })?;
        socket
            .set_read_timeout(Some(SERVER_POLL_INTERVAL))
            .map_err(|error| {
                ControlError::new(format!(
                    "failed to configure local control channel: {error}"
                ))
            })?;

        let closing = Arc::new(AtomicBool::new(false));
        let worker_closing = Arc::clone(&closing);
        let thread = thread::Builder::new()
            .name("edgemouse-control".to_owned())
            .spawn(move || serve(socket, &stopping, &worker_closing, &telemetry))
            .map_err(|error| {
                ControlError::new(format!("failed to start local control channel: {error}"))
            })?;

        Ok(Self {
            address,
            closing,
            thread: Some(thread),
        })
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::Release);
        if let Ok(socket) = UdpSocket::bind("127.0.0.1:0") {
            drop(socket.send_to(&encode_request(Command::Status), self.address));
        }
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

pub fn query_status() -> Result<Option<RunningStatus>, ControlError> {
    request(CONTROL_ADDRESS, Command::Status)
}

pub fn request_stop() -> Result<Option<RunningStatus>, ControlError> {
    request(CONTROL_ADDRESS, Command::Stop)
}

pub fn update_scroll_settings(
    reverse_horizontal: bool,
    reverse_vertical: bool,
) -> Result<Option<RunningStatus>, ControlError> {
    let settings = ScrollSettings {
        reverse_horizontal,
        reverse_vertical,
    };
    let packet = encode_scroll_request(settings);
    request_packet(CONTROL_ADDRESS, Command::SetScroll, &packet)
}

pub fn update_layout(peer_on: Edge) -> Result<Option<RunningStatus>, ControlError> {
    let packet = encode_layout_request(peer_on);
    request_packet(CONTROL_ADDRESS, Command::SetLayout, &packet)
}

fn request(address: &str, command: Command) -> Result<Option<RunningStatus>, ControlError> {
    request_packet(address, command, &encode_request(command))
}

fn request_packet(
    address: &str,
    command: Command,
    packet: &[u8],
) -> Result<Option<RunningStatus>, ControlError> {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| ControlError::new(format!("failed to open control client: {error}")))?;
    socket
        .set_read_timeout(Some(CLIENT_TIMEOUT))
        .map_err(|error| {
            ControlError::new(format!("failed to configure control client: {error}"))
        })?;
    socket
        .send_to(packet, address)
        .map_err(|error| ControlError::new(format!("failed to contact EdgeMouse: {error}")))?;

    let mut response = [0_u8; MAX_RESPONSE_LENGTH];
    match socket.recv_from(&mut response) {
        Ok((length, source)) => {
            if !source.ip().is_loopback() {
                return Err(ControlError::new(
                    "ignored a control response from outside this computer",
                ));
            }
            decode_response(&response[..length], command).map(Some)
        }
        Err(error) if is_no_response(&error) => Ok(None),
        Err(error) => Err(ControlError::new(format!(
            "failed to read EdgeMouse status: {error}"
        ))),
    }
}

fn serve(
    socket: UdpSocket,
    stopping: &AtomicBool,
    closing: &AtomicBool,
    telemetry: &RuntimeTelemetry,
) {
    let mut request = [0_u8; 64];

    while !closing.load(Ordering::Acquire) {
        match socket.recv_from(&mut request) {
            Ok((length, source)) => {
                if !source.ip().is_loopback() {
                    continue;
                }
                let Some(request) = decode_request(&request[..length]) else {
                    continue;
                };
                if let Some(settings) = request.scroll {
                    telemetry.set_scroll_settings(settings);
                }
                if let Some(peer_on) = request.peer_on {
                    telemetry.request_layout_update(peer_on);
                }
                let status = RunningStatus {
                    process_id: std::process::id(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    connection: telemetry.snapshot(),
                };
                let response = encode_response(request.command, &status);
                drop(socket.send_to(&response, source));
                if request.command == Command::Stop {
                    stopping.store(true, Ordering::Release);
                }
            }
            Err(error) if is_no_response(&error) => {}
            Err(_) => break,
        }
    }
}

fn encode_request(command: Command) -> [u8; REQUEST_LENGTH] {
    let mut request = [0_u8; REQUEST_LENGTH];
    request[..REQUEST_MAGIC.len()].copy_from_slice(REQUEST_MAGIC);
    request[REQUEST_MAGIC.len()] = command as u8;
    request
}

fn encode_scroll_request(settings: ScrollSettings) -> [u8; SCROLL_REQUEST_LENGTH] {
    let mut request = [0_u8; SCROLL_REQUEST_LENGTH];
    request[..REQUEST_MAGIC.len()].copy_from_slice(REQUEST_MAGIC);
    request[REQUEST_MAGIC.len()] = Command::SetScroll as u8;
    request[REQUEST_LENGTH] =
        u8::from(settings.reverse_horizontal) | (u8::from(settings.reverse_vertical) << 1);
    request
}

fn encode_layout_request(peer_on: Edge) -> [u8; LAYOUT_REQUEST_LENGTH] {
    let mut request = [0_u8; LAYOUT_REQUEST_LENGTH];
    request[..REQUEST_MAGIC.len()].copy_from_slice(REQUEST_MAGIC);
    request[REQUEST_MAGIC.len()] = Command::SetLayout as u8;
    request[REQUEST_LENGTH] = encode_edge(peer_on);
    request
}

fn decode_request(request: &[u8]) -> Option<ControlRequest> {
    if request.len() < REQUEST_LENGTH || &request[..REQUEST_MAGIC.len()] != REQUEST_MAGIC {
        return None;
    }
    let command = Command::from_byte(request[REQUEST_MAGIC.len()])?;
    let (scroll, peer_on) = match command {
        Command::Status | Command::Stop if request.len() == REQUEST_LENGTH => (None, None),
        Command::SetScroll if request.len() == SCROLL_REQUEST_LENGTH => {
            let flags = request[REQUEST_LENGTH];
            if flags & !0b11 != 0 {
                return None;
            }
            (
                Some(ScrollSettings {
                    reverse_horizontal: flags & 0b01 != 0,
                    reverse_vertical: flags & 0b10 != 0,
                }),
                None,
            )
        }
        Command::SetLayout if request.len() == LAYOUT_REQUEST_LENGTH => {
            (None, Some(decode_edge(request[REQUEST_LENGTH])?))
        }
        _ => return None,
    };
    Some(ControlRequest {
        command,
        scroll,
        peer_on,
    })
}

const fn encode_edge(edge: Edge) -> u8 {
    match edge {
        Edge::Left => 0,
        Edge::Right => 1,
        Edge::Top => 2,
        Edge::Bottom => 3,
    }
}

const fn decode_edge(value: u8) -> Option<Edge> {
    match value {
        0 => Some(Edge::Left),
        1 => Some(Edge::Right),
        2 => Some(Edge::Top),
        3 => Some(Edge::Bottom),
        _ => None,
    }
}

fn encode_response(command: Command, status: &RunningStatus) -> Vec<u8> {
    let version = status.version.as_bytes();
    let version_length = version.len().min(MAX_VERSION_LENGTH);
    let peer_name = status
        .connection
        .peer_name
        .as_deref()
        .unwrap_or("")
        .as_bytes();
    let peer_name_length = peer_name.len().min(MAX_PEER_NAME_LENGTH);
    let mut response = Vec::with_capacity(
        RESPONSE_HEADER_LENGTH + version_length + TELEMETRY_FIXED_LENGTH + peer_name_length,
    );
    response.extend_from_slice(RESPONSE_MAGIC);
    response.push(command as u8);
    response.extend_from_slice(&status.process_id.to_be_bytes());
    response.push(u8::try_from(version_length).expect("maximum version length fits in a byte"));
    response.extend_from_slice(&version[..version_length]);
    response.push(TELEMETRY_VERSION);
    response.push(status.connection.phase as u8);
    let mut flags = 0_u8;
    if status.connection.connected_since_unix_ms.is_some() {
        flags |= FLAG_CONNECTED_SINCE;
    }
    if status.connection.metrics_updated_unix_ms.is_some() {
        flags |= FLAG_LINK_METRICS;
    }
    response.push(flags);
    response.extend_from_slice(&status.connection.reconnect_count.to_be_bytes());
    response.extend_from_slice(
        &status
            .connection
            .connected_since_unix_ms
            .unwrap_or_default()
            .to_be_bytes(),
    );
    response.extend_from_slice(
        &status
            .connection
            .metrics_updated_unix_ms
            .unwrap_or_default()
            .to_be_bytes(),
    );
    response.extend_from_slice(
        &status
            .connection
            .rtt_ms
            .unwrap_or_default()
            .to_bits()
            .to_be_bytes(),
    );
    response.extend_from_slice(
        &status
            .connection
            .jitter_ms
            .unwrap_or_default()
            .to_bits()
            .to_be_bytes(),
    );
    response.extend_from_slice(
        &status
            .connection
            .send_interval_ms
            .unwrap_or_default()
            .to_be_bytes(),
    );
    response.extend_from_slice(&status.connection.sent_moves.to_be_bytes());
    response.extend_from_slice(&status.connection.skipped_moves.to_be_bytes());
    response.extend_from_slice(&status.connection.coalesced_moves.to_be_bytes());
    response.extend_from_slice(&status.connection.received_moves.to_be_bytes());
    response.extend_from_slice(&status.connection.stale_moves.to_be_bytes());
    response.extend_from_slice(&status.connection.superseded_moves.to_be_bytes());
    response.push(u8::try_from(peer_name_length).expect("maximum peer name length fits in a byte"));
    response.extend_from_slice(&peer_name[..peer_name_length]);
    response
}

fn decode_response(
    response: &[u8],
    expected_command: Command,
) -> Result<RunningStatus, ControlError> {
    if response.len() < RESPONSE_HEADER_LENGTH
        || &response[..RESPONSE_MAGIC.len()] != RESPONSE_MAGIC
    {
        return Err(ControlError::new(
            "EdgeMouse sent an invalid control response",
        ));
    }
    let command = Command::from_byte(response[RESPONSE_MAGIC.len()])
        .ok_or_else(|| ControlError::new("EdgeMouse sent an unknown control response"))?;
    if command != expected_command {
        return Err(ControlError::new(
            "EdgeMouse sent a response for the wrong control command",
        ));
    }
    let process_offset = RESPONSE_MAGIC.len() + 1;
    let process_id = u32::from_be_bytes(
        response[process_offset..process_offset + 4]
            .try_into()
            .expect("checked response header length"),
    );
    let version_length = usize::from(response[process_offset + 4]);
    let version_end = RESPONSE_HEADER_LENGTH + version_length;
    if version_length > MAX_VERSION_LENGTH || response.len() < version_end {
        return Err(ControlError::new(
            "EdgeMouse sent an invalid control response length",
        ));
    }
    let version = std::str::from_utf8(&response[RESPONSE_HEADER_LENGTH..version_end])
        .map_err(|_| ControlError::new("EdgeMouse sent an invalid version string"))?
        .to_owned();
    if version.is_empty() {
        return Err(ControlError::new("EdgeMouse sent an empty version string"));
    }

    let connection = if response.len() == version_end {
        ConnectionTelemetry::default()
    } else {
        decode_telemetry(&response[version_end..])?
    };
    Ok(RunningStatus {
        process_id,
        version,
        connection,
    })
}

fn decode_telemetry(data: &[u8]) -> Result<ConnectionTelemetry, ControlError> {
    let mut decoder = Decoder::new(data);
    let telemetry_version = decoder.u8()?;
    if telemetry_version != TELEMETRY_VERSION {
        return Err(ControlError::new(format!(
            "EdgeMouse sent unsupported telemetry version {telemetry_version}"
        )));
    }
    let phase = ConnectionPhase::from_byte(decoder.u8()?)
        .ok_or_else(|| ControlError::new("EdgeMouse sent an invalid connection phase"))?;
    let flags = decoder.u8()?;
    let reconnect_count = decoder.u32()?;
    let connected_since = decoder.u64()?;
    let metrics_updated = decoder.u64()?;
    let rtt_ms = f32::from_bits(decoder.u32()?);
    let jitter_ms = f32::from_bits(decoder.u32()?);
    let send_interval_ms = decoder.u32()?;
    let sent_moves = decoder.u64()?;
    let skipped_moves = decoder.u64()?;
    let coalesced_moves = decoder.u64()?;
    let received_moves = decoder.u64()?;
    let stale_moves = decoder.u64()?;
    let superseded_moves = decoder.u64()?;
    let peer_name_length = usize::from(decoder.u8()?);
    if peer_name_length > MAX_PEER_NAME_LENGTH {
        return Err(ControlError::new(
            "EdgeMouse sent an invalid peer name length",
        ));
    }
    let peer_name_bytes = decoder.bytes(peer_name_length)?;
    if !decoder.is_finished() {
        return Err(ControlError::new(
            "EdgeMouse sent trailing control response data",
        ));
    }
    let peer_name = std::str::from_utf8(peer_name_bytes)
        .map_err(|_| ControlError::new("EdgeMouse sent an invalid peer name"))?;
    let metrics_present = flags & FLAG_LINK_METRICS != 0;
    if metrics_present && (!rtt_ms.is_finite() || !jitter_ms.is_finite()) {
        return Err(ControlError::new(
            "EdgeMouse sent invalid connection metrics",
        ));
    }
    Ok(ConnectionTelemetry {
        phase,
        peer_name: (!peer_name.is_empty()).then(|| peer_name.to_owned()),
        connected_since_unix_ms: (flags & FLAG_CONNECTED_SINCE != 0).then_some(connected_since),
        metrics_updated_unix_ms: metrics_present.then_some(metrics_updated),
        reconnect_count,
        rtt_ms: metrics_present.then_some(rtt_ms),
        jitter_ms: metrics_present.then_some(jitter_ms),
        send_interval_ms: metrics_present.then_some(send_interval_ms),
        sent_moves,
        skipped_moves,
        coalesced_moves,
        received_moves,
        stale_moves,
        superseded_moves,
    })
}

struct Decoder<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn bytes(&mut self, length: usize) -> Result<&'a [u8], ControlError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| ControlError::new("EdgeMouse sent truncated telemetry data"))?;
        let bytes = &self.data[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn array<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], ControlError> {
        self.bytes(LENGTH)?
            .try_into()
            .map_err(|_| ControlError::new("EdgeMouse sent malformed telemetry data"))
    }

    fn u8(&mut self) -> Result<u8, ControlError> {
        Ok(self.array::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, ControlError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ControlError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    const fn is_finished(&self) -> bool {
        self.offset == self.data.len()
    }
}

fn is_no_response(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::time::Instant;

    fn loopback_ephemeral() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 0)
    }

    #[test]
    fn request_and_response_reject_malformed_messages() {
        assert_eq!(decode_request(b"short"), None);
        let mut request = encode_request(Command::Status);
        request[0] ^= 0xff;
        assert_eq!(decode_request(&request), None);

        let status_request = decode_request(&encode_request(Command::Status)).unwrap();
        assert_eq!(status_request.command, Command::Status);
        assert_eq!(status_request.scroll, None);
        assert_eq!(status_request.peer_on, None);
        let scroll = ScrollSettings {
            reverse_horizontal: true,
            reverse_vertical: false,
        };
        let scroll_request = decode_request(&encode_scroll_request(scroll)).unwrap();
        assert_eq!(scroll_request.command, Command::SetScroll);
        assert_eq!(scroll_request.scroll, Some(scroll));
        assert_eq!(scroll_request.peer_on, None);
        let mut invalid_scroll = encode_scroll_request(scroll);
        invalid_scroll[REQUEST_LENGTH] |= 0b100;
        assert_eq!(decode_request(&invalid_scroll), None);
        let layout_request = decode_request(&encode_layout_request(Edge::Top)).unwrap();
        assert_eq!(layout_request.command, Command::SetLayout);
        assert_eq!(layout_request.scroll, None);
        assert_eq!(layout_request.peer_on, Some(Edge::Top));
        let mut invalid_layout = encode_layout_request(Edge::Left);
        invalid_layout[REQUEST_LENGTH] = 4;
        assert_eq!(decode_request(&invalid_layout), None);

        let status = RunningStatus {
            process_id: 42,
            version: "test-version".to_owned(),
            connection: ConnectionTelemetry {
                phase: ConnectionPhase::Connected,
                peer_name: Some("test-peer".to_owned()),
                connected_since_unix_ms: Some(1_000),
                metrics_updated_unix_ms: Some(2_000),
                reconnect_count: 3,
                rtt_ms: Some(18.5),
                jitter_ms: Some(2.25),
                send_interval_ms: Some(4),
                sent_moves: 10,
                skipped_moves: 1,
                coalesced_moves: 2,
                received_moves: 20,
                stale_moves: 3,
                superseded_moves: 4,
            },
        };
        let response = encode_response(Command::Status, &status);
        assert_eq!(decode_response(&response, Command::Status).unwrap(), status);
        assert!(decode_response(&response, Command::Stop).is_err());
        assert!(decode_response(&response[..response.len() - 1], Command::Status).is_err());
    }

    #[test]
    fn a_closed_local_udp_port_means_the_agent_is_not_running() {
        for kind in [
            io::ErrorKind::WouldBlock,
            io::ErrorKind::TimedOut,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
        ] {
            assert!(is_no_response(&io::Error::from(kind)));
        }
        assert!(!is_no_response(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
    }

    #[test]
    fn status_and_stop_use_the_loopback_control_channel() {
        let stopping = Arc::new(AtomicBool::new(false));
        let telemetry = RuntimeTelemetry::default();
        telemetry.connected("test-peer");
        let server = ControlServer::start_on(
            loopback_ephemeral(),
            Arc::clone(&stopping),
            telemetry.clone(),
        )
        .unwrap();
        let address = server.address.to_string();

        let status = request(&address, Command::Status).unwrap().unwrap();
        assert_eq!(status.process_id, std::process::id());
        assert_eq!(status.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(status.connection.phase, ConnectionPhase::Connected);
        assert_eq!(status.connection.peer_name.as_deref(), Some("test-peer"));
        assert!(!stopping.load(Ordering::Acquire));

        let scroll = ScrollSettings {
            reverse_horizontal: true,
            reverse_vertical: true,
        };
        assert!(
            request_packet(&address, Command::SetScroll, &encode_scroll_request(scroll),)
                .unwrap()
                .is_some()
        );
        assert_eq!(telemetry.scroll_settings(), scroll);

        assert!(
            request_packet(
                &address,
                Command::SetLayout,
                &encode_layout_request(Edge::Bottom),
            )
            .unwrap()
            .is_some()
        );
        assert_eq!(telemetry.layout_update(), Some(Edge::Bottom));
        assert_eq!(telemetry.layout_update(), Some(Edge::Bottom));
        telemetry.complete_layout_update(Edge::Right);
        assert_eq!(telemetry.layout_update(), Some(Edge::Bottom));
        telemetry.complete_layout_update(Edge::Bottom);
        assert_eq!(telemetry.layout_update(), None);

        assert!(request(&address, Command::Stop).unwrap().is_some());
        let deadline = Instant::now() + Duration::from_secs(1);
        while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(stopping.load(Ordering::Acquire));
    }

    #[test]
    fn a_second_server_cannot_take_the_same_address() {
        let first = ControlServer::start_on(
            loopback_ephemeral(),
            Arc::new(AtomicBool::new(false)),
            RuntimeTelemetry::default(),
        )
        .unwrap();
        let second = ControlServer::start_on(
            first.address,
            Arc::new(AtomicBool::new(false)),
            RuntimeTelemetry::default(),
        );
        assert!(second.is_err());
    }

    #[test]
    fn legacy_status_responses_remain_readable() {
        let mut response = Vec::new();
        response.extend_from_slice(RESPONSE_MAGIC);
        response.push(Command::Status as u8);
        response.extend_from_slice(&42_u32.to_be_bytes());
        response.push(5);
        response.extend_from_slice(b"0.1.0");

        let status = decode_response(&response, Command::Status).unwrap();
        assert_eq!(status.process_id, 42);
        assert_eq!(status.version, "0.1.0");
        assert_eq!(status.connection, ConnectionTelemetry::default());
    }

    #[test]
    fn runtime_telemetry_tracks_connection_and_link_quality() {
        let telemetry = RuntimeTelemetry::default();
        assert_eq!(telemetry.scroll_settings(), ScrollSettings::default());
        let scroll = ScrollSettings {
            reverse_horizontal: true,
            reverse_vertical: false,
        };
        telemetry.set_scroll_settings(scroll);
        assert_eq!(telemetry.scroll_settings(), scroll);
        telemetry.begin_connecting(false);
        assert_eq!(telemetry.snapshot().phase, ConnectionPhase::Connecting);
        telemetry.connected("macbook");
        telemetry.update_link(LinkMetrics {
            rtt_ms: 19.25,
            jitter_ms: 2.5,
            send_interval_ms: 4,
            sent_moves: 10,
            skipped_moves: 1,
            coalesced_moves: 2,
            received_moves: 20,
            stale_moves: 3,
            superseded_moves: 4,
        });
        let connected = telemetry.snapshot();
        assert_eq!(connected.phase, ConnectionPhase::Connected);
        assert_eq!(connected.peer_name.as_deref(), Some("macbook"));
        assert_eq!(connected.rtt_ms, Some(19.25));
        assert_eq!(connected.jitter_ms, Some(2.5));
        assert!(connected.metrics_updated_unix_ms.is_some());

        telemetry.disconnected();
        let disconnected = telemetry.snapshot();
        assert_eq!(disconnected.phase, ConnectionPhase::Reconnecting);
        assert_eq!(disconnected.reconnect_count, 1);
        assert_eq!(disconnected.rtt_ms, None);
        assert_eq!(disconnected.sent_moves, 0);
    }
}
