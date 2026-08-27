use std::error::Error;
use std::fmt;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const CONTROL_ADDRESS: &str = "127.0.0.1:43894";
const REQUEST_MAGIC: &[u8; 8] = b"EDGMCTL1";
const RESPONSE_MAGIC: &[u8; 8] = b"EDGMACK1";
const REQUEST_LENGTH: usize = REQUEST_MAGIC.len() + 1;
const RESPONSE_HEADER_LENGTH: usize = RESPONSE_MAGIC.len() + 1 + 4 + 1;
const MAX_VERSION_LENGTH: usize = 63;
const CLIENT_TIMEOUT: Duration = Duration::from_millis(500);
const SERVER_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Command {
    Status = 1,
    Stop = 2,
}

impl Command {
    fn from_byte(value: u8) -> Option<Self> {
        match value {
            value if value == Self::Status as u8 => Some(Self::Status),
            value if value == Self::Stop as u8 => Some(Self::Stop),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningStatus {
    pub process_id: u32,
    pub version: String,
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
    pub fn start(stopping: Arc<AtomicBool>) -> Result<Self, ControlError> {
        let address = CONTROL_ADDRESS
            .parse()
            .map_err(|error| ControlError::new(format!("invalid control address: {error}")))?;
        Self::start_on(address, stopping)
    }

    fn start_on(address: SocketAddr, stopping: Arc<AtomicBool>) -> Result<Self, ControlError> {
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
            .spawn(move || serve(socket, &stopping, &worker_closing))
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

fn request(address: &str, command: Command) -> Result<Option<RunningStatus>, ControlError> {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| ControlError::new(format!("failed to open control client: {error}")))?;
    socket
        .set_read_timeout(Some(CLIENT_TIMEOUT))
        .map_err(|error| {
            ControlError::new(format!("failed to configure control client: {error}"))
        })?;
    socket
        .send_to(&encode_request(command), address)
        .map_err(|error| ControlError::new(format!("failed to contact EdgeMouse: {error}")))?;

    let mut response = [0_u8; RESPONSE_HEADER_LENGTH + MAX_VERSION_LENGTH];
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

fn serve(socket: UdpSocket, stopping: &AtomicBool, closing: &AtomicBool) {
    let status = RunningStatus {
        process_id: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let mut request = [0_u8; 64];

    while !closing.load(Ordering::Acquire) {
        match socket.recv_from(&mut request) {
            Ok((length, source)) => {
                if !source.ip().is_loopback() {
                    continue;
                }
                let Some(command) = decode_request(&request[..length]) else {
                    continue;
                };
                let response = encode_response(command, &status);
                drop(socket.send_to(&response, source));
                if command == Command::Stop {
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

fn decode_request(request: &[u8]) -> Option<Command> {
    if request.len() != REQUEST_LENGTH || &request[..REQUEST_MAGIC.len()] != REQUEST_MAGIC {
        return None;
    }
    Command::from_byte(request[REQUEST_MAGIC.len()])
}

fn encode_response(command: Command, status: &RunningStatus) -> Vec<u8> {
    let version = status.version.as_bytes();
    let version_length = version.len().min(MAX_VERSION_LENGTH);
    let mut response = Vec::with_capacity(RESPONSE_HEADER_LENGTH + version_length);
    response.extend_from_slice(RESPONSE_MAGIC);
    response.push(command as u8);
    response.extend_from_slice(&status.process_id.to_be_bytes());
    response.push(u8::try_from(version_length).expect("maximum version length fits in a byte"));
    response.extend_from_slice(&version[..version_length]);
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
    if version_length > MAX_VERSION_LENGTH
        || response.len() != RESPONSE_HEADER_LENGTH + version_length
    {
        return Err(ControlError::new(
            "EdgeMouse sent an invalid control response length",
        ));
    }
    let version = std::str::from_utf8(&response[RESPONSE_HEADER_LENGTH..])
        .map_err(|_| ControlError::new("EdgeMouse sent an invalid version string"))?
        .to_owned();
    if version.is_empty() {
        return Err(ControlError::new("EdgeMouse sent an empty version string"));
    }
    Ok(RunningStatus {
        process_id,
        version,
    })
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

        let status = RunningStatus {
            process_id: 42,
            version: "test-version".to_owned(),
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
        let server = ControlServer::start_on(loopback_ephemeral(), Arc::clone(&stopping)).unwrap();
        let address = server.address.to_string();

        let status = request(&address, Command::Status).unwrap().unwrap();
        assert_eq!(status.process_id, std::process::id());
        assert_eq!(status.version, env!("CARGO_PKG_VERSION"));
        assert!(!stopping.load(Ordering::Acquire));

        assert!(request(&address, Command::Stop).unwrap().is_some());
        let deadline = Instant::now() + Duration::from_secs(1);
        while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(stopping.load(Ordering::Acquire));
    }

    #[test]
    fn a_second_server_cannot_take_the_same_address() {
        let first = ControlServer::start_on(loopback_ephemeral(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        let second = ControlServer::start_on(first.address, Arc::new(AtomicBool::new(false)));
        assert!(second.is_err());
    }
}
