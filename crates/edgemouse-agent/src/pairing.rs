use crate::config::PairingConfig;
use edgemouse_core::NodeId;
use edgemouse_transport::TrustedPeer;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{
    IpAddr, Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub const PAIRING_PORT: u16 = 43_893;
const PAIRING_DISCOVERY_MAGIC: &[u8; 8] = b"EDGPAIR1";
const PAIRING_EXCHANGE_MAGIC: &[u8; 8] = b"EDGPAKE1";
const PAIRING_RECORD_MAGIC: &[u8; 8] = b"EDGCERT1";
const PAIRING_AUTH_LABEL: &[u8] = b"edgemouse-pairing-v1-record";
const PAIRING_CONFIRM_LABEL: &[u8] = b"edgemouse-pairing-v1-confirm";
const PAIRING_ID_A: &[u8] = b"edgemouse-pairing-v1-joiner";
const PAIRING_ID_B: &[u8] = b"edgemouse-pairing-v1-host";
const PAIRING_NAME_MAX_LEN: usize = 63;
const PAIRING_CERTIFICATE_MAX_LEN: usize = 16 * 1024;
const PAIRING_DISCOVERY_HEADER_LEN: usize = 27;
const PAIRING_DISCOVERY_MAX_LEN: usize = PAIRING_DISCOVERY_HEADER_LEN + PAIRING_NAME_MAX_LEN;
const PAIRING_RECORD_HEADER_LEN: usize = 30;
const PAIRING_RECORD_MAX_LEN: usize =
    PAIRING_RECORD_HEADER_LEN + PAIRING_NAME_MAX_LEN + PAIRING_CERTIFICATE_MAX_LEN;
const SPAKE_MESSAGE_LEN: usize = 33;
const EXCHANGE_MESSAGE_LEN: usize = 8 + 16 + SPAKE_MESSAGE_LEN;
const HMAC_LEN: usize = 32;
const HOST_LIFETIME: Duration = Duration::from_secs(5 * 60);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const ANNOUNCE_INTERVAL: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub struct PairingResult {
    pub peer_name: String,
    pub peer_node: NodeId,
    pub certificate_path: PathBuf,
    pub certificate_installed: bool,
}

#[derive(Debug)]
pub struct PairingHost {
    config: PairingConfig,
    listener: TcpListener,
    announcer: UdpSocket,
    offer: PairingOffer,
    offer_packet: Vec<u8>,
    code: String,
}

impl PairingHost {
    pub fn start(config: PairingConfig) -> Result<Self, PairingError> {
        validate_config(&config)?;
        let listener_address =
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PAIRING_PORT));
        let listener = TcpListener::bind(listener_address).map_err(|error| {
            PairingError::io(
                &format!(
                    "failed to listen for pairing on TCP {PAIRING_PORT}; stop any other pairing session and check the firewall"
                ),
                error,
            )
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| PairingError::io("failed to configure the pairing listener", error))?;

        let announcer =
            UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))).map_err(
                |error| PairingError::io("failed to create pairing discovery socket", error),
            )?;
        announcer
            .set_broadcast(true)
            .map_err(|error| PairingError::io("failed to enable pairing broadcast", error))?;

        let offer = PairingOffer {
            id: random_array()?,
            tcp_port: PAIRING_PORT,
            name: config.local_name.clone(),
        };
        let offer_packet = encode_offer(&offer)?;
        let code = generate_code()?;
        Ok(Self {
            config,
            listener,
            announcer,
            offer,
            offer_packet,
            code,
        })
    }

    #[must_use]
    pub fn formatted_code(&self) -> String {
        format!("{}-{}", &self.code[..4], &self.code[4..])
    }

    pub fn run(self, stopping: &AtomicBool) -> Result<PairingResult, PairingError> {
        let destination = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::BROADCAST,
            crate::discovery::DISCOVERY_PORT,
        ));
        let started = Instant::now();
        let mut next_announcement = Instant::now();
        let mut attempts = 0_usize;

        while started.elapsed() < HOST_LIFETIME {
            if stopping.load(Ordering::Acquire) {
                return Err(PairingError::new("pairing cancelled"));
            }
            if Instant::now() >= next_announcement {
                send_offer(&self.announcer, &self.offer_packet, destination)?;
                next_announcement = Instant::now() + ANNOUNCE_INTERVAL;
            }

            match self.listener.accept() {
                Ok((stream, _source)) => {
                    attempts += 1;
                    match host_exchange(stream, &self.config, &self.offer, &self.code) {
                        Ok(peer) => return finish_pairing(&self.config, peer),
                        Err(error) if attempts < MAX_ATTEMPTS => {
                            eprintln!(
                                "Pairing attempt {attempts}/{MAX_ATTEMPTS} was rejected: {error}"
                            );
                        }
                        Err(error) => {
                            return Err(PairingError::new(format!(
                                "pairing stopped after {MAX_ATTEMPTS} rejected attempts; restart it to get a new code (last error: {error})"
                            )));
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => {
                    return Err(PairingError::io(
                        "failed to accept a pairing connection",
                        error,
                    ));
                }
            }
        }

        Err(PairingError::new(
            "pairing code expired after 5 minutes; restart pairing to get a new code",
        ))
    }
}

pub fn join(
    config: PairingConfig,
    code: &str,
    direct_host: Option<&str>,
    stopping: &AtomicBool,
) -> Result<PairingResult, PairingError> {
    validate_config(&config)?;
    let code = normalize_code(code)?;
    let (address, expected_offer_id) = if let Some(host) = direct_host {
        (parse_direct_host(host)?, None)
    } else {
        let discovered = discover_offer(config.timeout, stopping)?;
        (
            SocketAddr::new(discovered.source.ip(), discovered.offer.tcp_port),
            Some(discovered.offer.id),
        )
    };
    let peer = join_exchange(
        address,
        config.timeout,
        &config,
        expected_offer_id.as_ref(),
        &code,
    )?;
    finish_pairing(&config, peer)
}

fn parse_direct_host(value: &str) -> Result<SocketAddr, PairingError> {
    if let Ok(address) = value.parse::<SocketAddr>() {
        if address.port() == 0 {
            return Err(PairingError::new("direct pairing port cannot be zero"));
        }
        return Ok(address);
    }
    value
        .parse::<IpAddr>()
        .map(|address| SocketAddr::new(address, PAIRING_PORT))
        .map_err(|error| {
            PairingError::new(format!(
                "invalid direct pairing host `{value}`; use an IP address such as 192.168.8.202: {error}"
            ))
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairingOffer {
    id: [u8; 16],
    tcp_port: u16,
    name: String,
}

#[derive(Debug, Clone)]
struct DiscoveredOffer {
    offer: PairingOffer,
    source: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CertificateRecord {
    role: u8,
    node: NodeId,
    name: String,
    certificate: Vec<u8>,
}

fn validate_config(config: &PairingConfig) -> Result<(), PairingError> {
    validate_name(&config.local_name)?;
    if config.local_certificate.is_empty()
        || config.local_certificate.len() > PAIRING_CERTIFICATE_MAX_LEN
    {
        return Err(PairingError::new(format!(
            "local certificate must contain 1 to {PAIRING_CERTIFICATE_MAX_LEN} bytes"
        )));
    }
    if config.local_node.0 == 0 {
        return Err(PairingError::new("local node ID cannot be zero"));
    }
    if config.timeout.is_zero() {
        return Err(PairingError::new(
            "pairing discovery timeout must be greater than zero",
        ));
    }
    Ok(())
}

fn generate_code() -> Result<String, PairingError> {
    let rng = SystemRandom::new();
    let mut digits = String::with_capacity(8);
    while digits.len() < 8 {
        let mut random = [0_u8; 8];
        rng.fill(&mut random)
            .map_err(|_| PairingError::new("operating system random generator failed"))?;
        for byte in random {
            // Rejection sampling keeps each decimal digit equally likely.
            if byte < 250 {
                digits.push(char::from(b'0' + byte % 10));
                if digits.len() == 8 {
                    break;
                }
            }
        }
    }
    Ok(digits)
}

fn random_array<const N: usize>() -> Result<[u8; N], PairingError> {
    let mut bytes = [0_u8; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| PairingError::new("operating system random generator failed"))?;
    Ok(bytes)
}

fn normalize_code(value: &str) -> Result<String, PairingError> {
    let digits: String = value
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .collect();
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PairingError::new(
            "pairing code must contain exactly 8 digits (for example 1234-5678)",
        ));
    }
    Ok(digits)
}

fn encode_offer(offer: &PairingOffer) -> Result<Vec<u8>, PairingError> {
    validate_name(&offer.name)?;
    if offer.tcp_port == 0 {
        return Err(PairingError::new("pairing TCP port cannot be zero"));
    }
    let name_length = u8::try_from(offer.name.len())
        .map_err(|_| PairingError::new("pairing device name is too long"))?;
    let mut packet = Vec::with_capacity(PAIRING_DISCOVERY_HEADER_LEN + offer.name.len());
    packet.extend_from_slice(PAIRING_DISCOVERY_MAGIC);
    packet.extend_from_slice(&offer.id);
    packet.extend_from_slice(&offer.tcp_port.to_be_bytes());
    packet.push(name_length);
    packet.extend_from_slice(offer.name.as_bytes());
    Ok(packet)
}

fn decode_offer(packet: &[u8]) -> Result<PairingOffer, PairingError> {
    if packet.len() < PAIRING_DISCOVERY_HEADER_LEN {
        return Err(PairingError::new("pairing offer is truncated"));
    }
    if &packet[..8] != PAIRING_DISCOVERY_MAGIC {
        return Err(PairingError::new("pairing offer has invalid magic"));
    }
    let id = packet[8..24]
        .try_into()
        .map_err(|_| PairingError::new("pairing offer ID is truncated"))?;
    let tcp_port = u16::from_be_bytes(
        packet[24..26]
            .try_into()
            .map_err(|_| PairingError::new("pairing offer port is truncated"))?,
    );
    if tcp_port == 0 {
        return Err(PairingError::new("pairing offer port cannot be zero"));
    }
    let name_length = usize::from(packet[26]);
    if name_length > PAIRING_NAME_MAX_LEN
        || packet.len() != PAIRING_DISCOVERY_HEADER_LEN + name_length
    {
        return Err(PairingError::new(
            "pairing offer has an invalid device-name length",
        ));
    }
    let name = std::str::from_utf8(&packet[PAIRING_DISCOVERY_HEADER_LEN..])
        .map_err(|_| PairingError::new("pairing device name is not valid UTF-8"))?
        .to_owned();
    validate_name(&name)?;
    Ok(PairingOffer { id, tcp_port, name })
}

fn send_offer(
    socket: &UdpSocket,
    packet: &[u8],
    destination: SocketAddr,
) -> Result<(), PairingError> {
    let sent = socket
        .send_to(packet, destination)
        .map_err(|error| PairingError::io("failed to broadcast pairing offer", error))?;
    if sent != packet.len() {
        return Err(PairingError::new(
            "pairing offer broadcast was only partially sent",
        ));
    }
    Ok(())
}

fn discover_offer(
    timeout: Duration,
    stopping: &AtomicBool,
) -> Result<DiscoveredOffer, PairingError> {
    let bind_address = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::UNSPECIFIED,
        crate::discovery::DISCOVERY_PORT,
    ));
    let socket = UdpSocket::bind(bind_address).map_err(|error| {
        PairingError::io(
            &format!(
                "failed to listen for pairing offers on UDP {}; stop the normal EdgeMouse agent before pairing",
                crate::discovery::DISCOVERY_PORT
            ),
            error,
        )
    })?;
    socket
        .set_read_timeout(Some(DISCOVERY_POLL_INTERVAL))
        .map_err(|error| {
            PairingError::io("failed to configure pairing discovery timeout", error)
        })?;
    let started = Instant::now();
    let mut buffer = [0_u8; PAIRING_DISCOVERY_MAX_LEN + 1];
    while started.elapsed() < timeout {
        if stopping.load(Ordering::Acquire) {
            return Err(PairingError::new("pairing cancelled"));
        }
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                if length > PAIRING_DISCOVERY_MAX_LEN {
                    continue;
                }
                if let Ok(offer) = decode_offer(&buffer[..length]) {
                    return Ok(DiscoveredOffer { offer, source });
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionRefused
                ) => {}
            Err(error) => {
                return Err(PairingError::io("failed to receive a pairing offer", error));
            }
        }
    }
    Err(PairingError::new(format!(
        "no pairing host was found within {} second(s); start `edgemouse pair host` on Windows and allow inbound TCP {PAIRING_PORT}",
        timeout.as_secs()
    )))
}

fn host_exchange(
    mut stream: TcpStream,
    config: &PairingConfig,
    offer: &PairingOffer,
    code: &str,
) -> Result<CertificateRecord, PairingError> {
    configure_stream(&stream)?;
    write_tcp_offer(&mut stream, offer)?;
    let mut client_hello = [0_u8; EXCHANGE_MESSAGE_LEN];
    stream
        .read_exact(&mut client_hello)
        .map_err(|error| PairingError::io("failed to read pairing key exchange", error))?;
    if &client_hello[..8] != PAIRING_EXCHANGE_MAGIC || client_hello[8..24] != offer.id {
        return Err(PairingError::new(
            "pairing key exchange does not match this one-time offer",
        ));
    }
    let offer_id = &offer.id;
    let client_spake = &client_hello[24..];
    let (id_a, id_b) = spake_identities(offer_id);
    let (state, host_spake) =
        Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &id_a, &id_b);
    if host_spake.len() != SPAKE_MESSAGE_LEN {
        return Err(PairingError::new(
            "SPAKE2 produced an unexpected host message length",
        ));
    }
    write_exchange(&mut stream, offer_id, &host_spake)?;
    let shared_key = state
        .finish(client_spake)
        .map_err(|error| PairingError::new(format!("invalid SPAKE2 client message: {error}")))?;

    let host_payload = encode_record(&local_record(config, b'B'))?;
    write_authenticated_record(
        &mut stream,
        &shared_key,
        offer_id,
        client_spake,
        &host_spake,
        &host_payload,
    )?;
    let (client, client_payload) = read_authenticated_record(
        &mut stream,
        &shared_key,
        offer_id,
        client_spake,
        &host_spake,
        b'A',
        config.local_node,
    )?;
    ensure_certificate_installable(&config.peer_certificate_path, &client.certificate)?;
    let host_confirmation = confirmation_tag(
        &shared_key,
        offer_id,
        client_spake,
        &host_spake,
        &host_payload,
        &client_payload,
        b"host",
    );
    stream
        .write_all(host_confirmation.as_ref())
        .map_err(|error| PairingError::io("failed to send pairing confirmation", error))?;
    let mut client_confirmation = [0_u8; HMAC_LEN];
    stream
        .read_exact(&mut client_confirmation)
        .map_err(|error| PairingError::io("failed to read pairing confirmation", error))?;
    verify_confirmation(
        &shared_key,
        offer_id,
        client_spake,
        &host_spake,
        &host_payload,
        &client_payload,
        b"joiner",
        &client_confirmation,
    )?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(client)
}

fn join_exchange(
    address: SocketAddr,
    timeout: Duration,
    config: &PairingConfig,
    expected_offer_id: Option<&[u8; 16]>,
    code: &str,
) -> Result<CertificateRecord, PairingError> {
    let mut stream =
        TcpStream::connect_timeout(&address, timeout.min(HANDSHAKE_TIMEOUT)).map_err(|error| {
            PairingError::io(
                &format!("failed to connect to pairing host at {address}"),
                error,
            )
        })?;
    configure_stream(&stream)?;
    let tcp_offer = read_tcp_offer(&mut stream)?;
    if expected_offer_id.is_some_and(|expected| *expected != tcp_offer.id) {
        return Err(PairingError::new(
            "TCP pairing host does not match the discovered one-time offer",
        ));
    }
    let offer_id = &tcp_offer.id;
    let (id_a, id_b) = spake_identities(offer_id);
    let (state, client_spake) =
        Spake2::<Ed25519Group>::start_a(&Password::new(code.as_bytes()), &id_a, &id_b);
    if client_spake.len() != SPAKE_MESSAGE_LEN {
        return Err(PairingError::new(
            "SPAKE2 produced an unexpected client message length",
        ));
    }
    write_exchange(&mut stream, offer_id, &client_spake)?;
    let mut host_hello = [0_u8; EXCHANGE_MESSAGE_LEN];
    stream
        .read_exact(&mut host_hello)
        .map_err(|error| PairingError::io("failed to read pairing key exchange", error))?;
    if &host_hello[..8] != PAIRING_EXCHANGE_MAGIC || &host_hello[8..24] != offer_id {
        return Err(PairingError::new(
            "pairing host replied for a different one-time offer",
        ));
    }
    let host_spake = &host_hello[24..];
    let shared_key = state
        .finish(host_spake)
        .map_err(|error| PairingError::new(format!("invalid SPAKE2 host message: {error}")))?;

    let (host, host_payload) = read_authenticated_record(
        &mut stream,
        &shared_key,
        offer_id,
        &client_spake,
        host_spake,
        b'B',
        config.local_node,
    )?;
    ensure_certificate_installable(&config.peer_certificate_path, &host.certificate)?;
    let client_payload = encode_record(&local_record(config, b'A'))?;
    write_authenticated_record(
        &mut stream,
        &shared_key,
        offer_id,
        &client_spake,
        host_spake,
        &client_payload,
    )?;
    let mut host_confirmation = [0_u8; HMAC_LEN];
    stream
        .read_exact(&mut host_confirmation)
        .map_err(|error| PairingError::io("failed to read pairing confirmation", error))?;
    verify_confirmation(
        &shared_key,
        offer_id,
        &client_spake,
        host_spake,
        &host_payload,
        &client_payload,
        b"host",
        &host_confirmation,
    )?;
    let client_confirmation = confirmation_tag(
        &shared_key,
        offer_id,
        &client_spake,
        host_spake,
        &host_payload,
        &client_payload,
        b"joiner",
    );
    stream
        .write_all(client_confirmation.as_ref())
        .map_err(|error| PairingError::io("failed to send pairing confirmation", error))?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(host)
}

fn configure_stream(stream: &TcpStream) -> Result<(), PairingError> {
    stream
        .set_nonblocking(false)
        .map_err(|error| PairingError::io("failed to configure pairing connection", error))?;
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|error| PairingError::io("failed to set pairing read timeout", error))?;
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|error| PairingError::io("failed to set pairing write timeout", error))?;
    stream
        .set_nodelay(true)
        .map_err(|error| PairingError::io("failed to configure pairing latency", error))?;
    Ok(())
}

fn write_tcp_offer(stream: &mut TcpStream, offer: &PairingOffer) -> Result<(), PairingError> {
    let packet = encode_offer(offer)?;
    let length = u16::try_from(packet.len())
        .map_err(|_| PairingError::new("TCP pairing offer is too large"))?;
    stream
        .write_all(&length.to_be_bytes())
        .and_then(|()| stream.write_all(&packet))
        .map_err(|error| PairingError::io("failed to send TCP pairing offer", error))
}

fn read_tcp_offer(stream: &mut TcpStream) -> Result<PairingOffer, PairingError> {
    let mut length_bytes = [0_u8; 2];
    stream
        .read_exact(&mut length_bytes)
        .map_err(|error| PairingError::io("failed to read TCP pairing offer", error))?;
    let length = usize::from(u16::from_be_bytes(length_bytes));
    if !(PAIRING_DISCOVERY_HEADER_LEN..=PAIRING_DISCOVERY_MAX_LEN).contains(&length) {
        return Err(PairingError::new(
            "TCP pairing offer has an invalid bounded length",
        ));
    }
    let mut packet = vec![0_u8; length];
    stream
        .read_exact(&mut packet)
        .map_err(|error| PairingError::io("failed to read TCP pairing offer", error))?;
    decode_offer(&packet)
}

fn spake_identities(offer_id: &[u8; 16]) -> (SpakeIdentity, SpakeIdentity) {
    let mut id_a = Vec::with_capacity(PAIRING_ID_A.len() + offer_id.len());
    id_a.extend_from_slice(PAIRING_ID_A);
    id_a.extend_from_slice(offer_id);
    let mut id_b = Vec::with_capacity(PAIRING_ID_B.len() + offer_id.len());
    id_b.extend_from_slice(PAIRING_ID_B);
    id_b.extend_from_slice(offer_id);
    (SpakeIdentity::new(&id_a), SpakeIdentity::new(&id_b))
}

fn write_exchange(
    stream: &mut TcpStream,
    offer_id: &[u8; 16],
    spake_message: &[u8],
) -> Result<(), PairingError> {
    if spake_message.len() != SPAKE_MESSAGE_LEN {
        return Err(PairingError::new("invalid SPAKE2 message length"));
    }
    let mut message = Vec::with_capacity(EXCHANGE_MESSAGE_LEN);
    message.extend_from_slice(PAIRING_EXCHANGE_MAGIC);
    message.extend_from_slice(offer_id);
    message.extend_from_slice(spake_message);
    stream
        .write_all(&message)
        .map_err(|error| PairingError::io("failed to send pairing key exchange", error))
}

fn local_record(config: &PairingConfig, role: u8) -> CertificateRecord {
    CertificateRecord {
        role,
        node: config.local_node,
        name: config.local_name.clone(),
        certificate: config.local_certificate.clone(),
    }
}

fn encode_record(record: &CertificateRecord) -> Result<Vec<u8>, PairingError> {
    validate_name(&record.name)?;
    if !matches!(record.role, b'A' | b'B') {
        return Err(PairingError::new("invalid pairing role"));
    }
    if record.node.0 == 0 {
        return Err(PairingError::new(
            "certificate record node ID cannot be zero",
        ));
    }
    if record.certificate.is_empty() || record.certificate.len() > PAIRING_CERTIFICATE_MAX_LEN {
        return Err(PairingError::new(format!(
            "certificate record must contain 1 to {PAIRING_CERTIFICATE_MAX_LEN} bytes"
        )));
    }
    let name_length = u8::try_from(record.name.len())
        .map_err(|_| PairingError::new("pairing device name is too long"))?;
    let certificate_length = u32::try_from(record.certificate.len())
        .map_err(|_| PairingError::new("pairing certificate is too large"))?;
    let mut payload = Vec::with_capacity(
        PAIRING_RECORD_HEADER_LEN + record.name.len() + record.certificate.len(),
    );
    payload.extend_from_slice(PAIRING_RECORD_MAGIC);
    payload.push(record.role);
    payload.extend_from_slice(&record.node.0.to_be_bytes());
    payload.push(name_length);
    payload.extend_from_slice(&certificate_length.to_be_bytes());
    payload.extend_from_slice(record.name.as_bytes());
    payload.extend_from_slice(&record.certificate);
    Ok(payload)
}

fn decode_record(
    payload: &[u8],
    expected_role: u8,
    local_node: NodeId,
) -> Result<CertificateRecord, PairingError> {
    if payload.len() < PAIRING_RECORD_HEADER_LEN {
        return Err(PairingError::new("certificate record is truncated"));
    }
    if &payload[..8] != PAIRING_RECORD_MAGIC {
        return Err(PairingError::new("certificate record has invalid magic"));
    }
    let role = payload[8];
    if role != expected_role {
        return Err(PairingError::new("certificate record has the wrong role"));
    }
    let node = NodeId(u128::from_be_bytes(payload[9..25].try_into().map_err(
        |_| PairingError::new("certificate record node ID is truncated"),
    )?));
    let name_length = usize::from(payload[25]);
    let certificate_length =
        usize::try_from(u32::from_be_bytes(payload[26..30].try_into().map_err(
            |_| PairingError::new("certificate record length is truncated"),
        )?))
        .map_err(|_| PairingError::new("certificate record length is unsupported"))?;
    if name_length > PAIRING_NAME_MAX_LEN
        || certificate_length == 0
        || certificate_length > PAIRING_CERTIFICATE_MAX_LEN
        || payload.len() != PAIRING_RECORD_HEADER_LEN + name_length + certificate_length
    {
        return Err(PairingError::new(
            "certificate record has invalid field lengths",
        ));
    }
    let name_end = PAIRING_RECORD_HEADER_LEN + name_length;
    let name = std::str::from_utf8(&payload[PAIRING_RECORD_HEADER_LEN..name_end])
        .map_err(|_| PairingError::new("paired device name is not valid UTF-8"))?
        .to_owned();
    validate_name(&name)?;
    let certificate = payload[name_end..].to_vec();
    let derived = TrustedPeer::from_der(certificate.clone())
        .map_err(|error| PairingError::new(format!("paired certificate is invalid: {error}")))?;
    if node != derived.node_id() {
        return Err(PairingError::new(
            "paired certificate does not match its announced node ID",
        ));
    }
    if node == local_node {
        return Err(PairingError::new(
            "the other machine presented this machine's own certificate",
        ));
    }
    Ok(CertificateRecord {
        role,
        node,
        name,
        certificate,
    })
}

fn write_authenticated_record(
    stream: &mut TcpStream,
    shared_key: &[u8],
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    payload: &[u8],
) -> Result<(), PairingError> {
    if payload.len() > PAIRING_RECORD_MAX_LEN {
        return Err(PairingError::new("certificate record is too large"));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| PairingError::new("certificate record is too large"))?;
    let tag = record_tag(shared_key, offer_id, client_spake, host_spake, payload);
    stream
        .write_all(&length.to_be_bytes())
        .and_then(|()| stream.write_all(payload))
        .and_then(|()| stream.write_all(tag.as_ref()))
        .map_err(|error| PairingError::io("failed to send authenticated certificate", error))
}

fn read_authenticated_record(
    stream: &mut TcpStream,
    shared_key: &[u8],
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    expected_role: u8,
    local_node: NodeId,
) -> Result<(CertificateRecord, Vec<u8>), PairingError> {
    let mut length_bytes = [0_u8; 4];
    stream
        .read_exact(&mut length_bytes)
        .map_err(|error| PairingError::io("failed to read authenticated certificate", error))?;
    let length = usize::try_from(u32::from_be_bytes(length_bytes))
        .map_err(|_| PairingError::new("certificate record length is unsupported"))?;
    if !(PAIRING_RECORD_HEADER_LEN..=PAIRING_RECORD_MAX_LEN).contains(&length) {
        return Err(PairingError::new(
            "authenticated certificate has an invalid bounded length",
        ));
    }
    let mut payload = vec![0_u8; length];
    let mut tag = [0_u8; HMAC_LEN];
    stream
        .read_exact(&mut payload)
        .and_then(|()| stream.read_exact(&mut tag))
        .map_err(|error| PairingError::io("failed to read authenticated certificate", error))?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, shared_key);
    hmac::verify(
        &key,
        &record_authentication_data(offer_id, client_spake, host_spake, &payload),
        &tag,
    )
    .map_err(|_| {
        PairingError::new(
            "pairing authentication failed; verify the short code and restart both pairing commands",
        )
    })?;
    let record = decode_record(&payload, expected_role, local_node)?;
    Ok((record, payload))
}

fn record_tag(
    shared_key: &[u8],
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    payload: &[u8],
) -> hmac::Tag {
    let key = hmac::Key::new(hmac::HMAC_SHA256, shared_key);
    hmac::sign(
        &key,
        &record_authentication_data(offer_id, client_spake, host_spake, payload),
    )
}

fn record_authentication_data(
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    payload: &[u8],
) -> Vec<u8> {
    let mut data = Vec::with_capacity(
        PAIRING_AUTH_LABEL.len()
            + offer_id.len()
            + client_spake.len()
            + host_spake.len()
            + payload.len(),
    );
    data.extend_from_slice(PAIRING_AUTH_LABEL);
    data.extend_from_slice(offer_id);
    data.extend_from_slice(client_spake);
    data.extend_from_slice(host_spake);
    data.extend_from_slice(payload);
    data
}

#[allow(clippy::too_many_arguments)]
fn confirmation_tag(
    shared_key: &[u8],
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    host_payload: &[u8],
    client_payload: &[u8],
    confirmer: &[u8],
) -> hmac::Tag {
    let key = hmac::Key::new(hmac::HMAC_SHA256, shared_key);
    hmac::sign(
        &key,
        &confirmation_authentication_data(
            offer_id,
            client_spake,
            host_spake,
            host_payload,
            client_payload,
            confirmer,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_confirmation(
    shared_key: &[u8],
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    host_payload: &[u8],
    client_payload: &[u8],
    confirmer: &[u8],
    received: &[u8],
) -> Result<(), PairingError> {
    if received.len() != HMAC_LEN {
        return Err(PairingError::new("pairing confirmation has invalid length"));
    }
    let transcript = confirmation_authentication_data(
        offer_id,
        client_spake,
        host_spake,
        host_payload,
        client_payload,
        confirmer,
    );
    let session_key = hmac::Key::new(hmac::HMAC_SHA256, shared_key);
    hmac::verify(&session_key, &transcript, received)
        .map_err(|_| PairingError::new("pairing key confirmation failed; no certificate was saved"))
}

#[allow(clippy::too_many_arguments)]
fn confirmation_authentication_data(
    offer_id: &[u8; 16],
    client_spake: &[u8],
    host_spake: &[u8],
    host_payload: &[u8],
    client_payload: &[u8],
    confirmer: &[u8],
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(PAIRING_CONFIRM_LABEL);
    data.extend_from_slice(offer_id);
    data.extend_from_slice(client_spake);
    data.extend_from_slice(host_spake);
    data.extend_from_slice(&(host_payload.len() as u32).to_be_bytes());
    data.extend_from_slice(host_payload);
    data.extend_from_slice(&(client_payload.len() as u32).to_be_bytes());
    data.extend_from_slice(client_payload);
    data.extend_from_slice(confirmer);
    data
}

fn finish_pairing(
    config: &PairingConfig,
    peer: CertificateRecord,
) -> Result<PairingResult, PairingError> {
    let certificate_installed =
        install_certificate(&config.peer_certificate_path, &peer.certificate)?;
    Ok(PairingResult {
        peer_name: peer.name,
        peer_node: peer.node,
        certificate_path: config.peer_certificate_path.clone(),
        certificate_installed,
    })
}

fn install_certificate(path: &Path, certificate: &[u8]) -> Result<bool, PairingError> {
    if ensure_certificate_installable(path, certificate)? {
        return Ok(false);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        PairingError::io(
            &format!(
                "failed to create certificate directory {}",
                parent.display()
            ),
            error,
        )
    })?;

    for _ in 0..10 {
        let suffix = random_array::<8>()?;
        let temporary = parent.join(format!(
            ".edgemouse-pairing-{}-{}.tmp",
            std::process::id(),
            u64::from_be_bytes(suffix)
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(PairingError::io(
                    &format!(
                        "failed to create temporary certificate {}",
                        temporary.display()
                    ),
                    error,
                ));
            }
        };
        let write_result = file.write_all(certificate).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            drop(fs::remove_file(&temporary));
            return Err(PairingError::io(
                &format!(
                    "failed to write temporary certificate {}",
                    temporary.display()
                ),
                error,
            ));
        }
        match publish_without_overwrite(&temporary, path) {
            Ok(()) => {
                drop(fs::remove_file(&temporary));
                return Ok(true);
            }
            Err(error) => {
                drop(fs::remove_file(&temporary));
                if path.exists() {
                    let existing = fs::read(path).map_err(|read_error| {
                        PairingError::io(
                            &format!(
                                "failed to inspect concurrently created certificate {}",
                                path.display()
                            ),
                            read_error,
                        )
                    })?;
                    if existing == certificate {
                        return Ok(false);
                    }
                    return Err(PairingError::new(format!(
                        "refusing to replace a different trusted certificate at {}",
                        path.display()
                    )));
                }
                return Err(PairingError::io(
                    &format!("failed to install peer certificate {}", path.display()),
                    error,
                ));
            }
        }
    }
    Err(PairingError::new(
        "failed to allocate a unique temporary certificate file",
    ))
}

fn ensure_certificate_installable(path: &Path, certificate: &[u8]) -> Result<bool, PairingError> {
    if !path.exists() {
        return Ok(false);
    }
    let existing = fs::read(path).map_err(|error| {
        PairingError::io(
            &format!(
                "failed to read existing peer certificate {}",
                path.display()
            ),
            error,
        )
    })?;
    if existing == certificate {
        Ok(true)
    } else {
        Err(PairingError::new(format!(
            "refusing to replace a different trusted certificate at {}; move it aside only if you intentionally want to pair a different machine",
            path.display()
        )))
    }
}

#[cfg(unix)]
fn publish_without_overwrite(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(temporary, destination)
}

#[cfg(windows)]
fn publish_without_overwrite(temporary: &Path, destination: &Path) -> io::Result<()> {
    // Windows rename is no-clobber when the destination already exists.
    fs::rename(temporary, destination)
}

#[cfg(not(any(unix, windows)))]
fn publish_without_overwrite(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(temporary, destination)
}

fn validate_name(name: &str) -> Result<(), PairingError> {
    if name.is_empty() {
        return Err(PairingError::new("pairing device name cannot be empty"));
    }
    if name.len() > PAIRING_NAME_MAX_LEN {
        return Err(PairingError::new(format!(
            "pairing device name cannot exceed {PAIRING_NAME_MAX_LEN} UTF-8 bytes"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(PairingError::new(
            "pairing device name cannot contain control characters",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingError(String);

impl PairingError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn io(context: &str, error: io::Error) -> Self {
        Self::new(format!("{context}: {error}"))
    }
}

impl Display for PairingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PairingError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairing_config(
        name: &str,
        generated: edgemouse_transport::GeneratedIdentity,
        peer_path: PathBuf,
    ) -> PairingConfig {
        PairingConfig {
            local_name: name.to_owned(),
            local_certificate: generated.certificate,
            local_node: generated.node_id,
            peer_certificate_path: peer_path,
            timeout: Duration::from_secs(2),
        }
    }

    #[test]
    fn codes_are_normalized_and_strictly_validated() {
        assert_eq!(normalize_code("1234-5678").unwrap(), "12345678");
        assert_eq!(normalize_code("1234 5678").unwrap(), "12345678");
        assert!(normalize_code("1234567").is_err());
        assert!(normalize_code("1234-abcd").is_err());
        let generated = generate_code().unwrap();
        assert_eq!(generated.len(), 8);
        assert!(generated.bytes().all(|byte| byte.is_ascii_digit()));
    }

    #[test]
    fn direct_hosts_accept_an_ip_or_an_explicit_port() {
        assert_eq!(
            parse_direct_host("192.168.8.202").unwrap(),
            "192.168.8.202:43893".parse().unwrap()
        );
        assert_eq!(
            parse_direct_host("192.168.8.202:50000").unwrap(),
            "192.168.8.202:50000".parse().unwrap()
        );
        assert!(parse_direct_host("windows-pc").is_err());
        assert!(parse_direct_host("192.168.8.202:0").is_err());
    }

    #[test]
    fn offers_round_trip_and_reject_malformed_lengths() {
        let offer = PairingOffer {
            id: [7; 16],
            tcp_port: PAIRING_PORT,
            name: "Windows 电脑".to_owned(),
        };
        let packet = encode_offer(&offer).unwrap();
        assert_eq!(decode_offer(&packet).unwrap(), offer);
        assert!(decode_offer(&packet[..20]).is_err());
        let mut trailing = packet;
        trailing.push(0);
        assert!(decode_offer(&trailing).is_err());
    }

    #[test]
    fn certificate_records_round_trip_and_bind_node_id() {
        let generated = edgemouse_transport::Identity::generate().unwrap();
        let record = CertificateRecord {
            role: b'A',
            node: generated.node_id,
            name: "macbook".to_owned(),
            certificate: generated.certificate,
        };
        let payload = encode_record(&record).unwrap();
        assert_eq!(
            decode_record(&payload, b'A', NodeId(record.node.0 ^ 1)).unwrap(),
            record
        );
        let mut forged = payload;
        forged[24] ^= 1;
        assert!(decode_record(&forged, b'A', NodeId(999)).is_err());
    }

    #[test]
    fn different_codes_do_not_authenticate_the_same_record() {
        let offer_id = [9; 16];
        let (id_a, id_b) = spake_identities(&offer_id);
        let (state_a, message_a) =
            Spake2::<Ed25519Group>::start_a(&Password::new(b"12345678"), &id_a, &id_b);
        let (state_b, message_b) =
            Spake2::<Ed25519Group>::start_b(&Password::new(b"87654321"), &id_a, &id_b);
        let key_a = state_a.finish(&message_b).unwrap();
        let key_b = state_b.finish(&message_a).unwrap();
        let payload = b"certificate";
        let tag = record_tag(&key_a, &offer_id, &message_a, &message_b, payload);
        let key = hmac::Key::new(hmac::HMAC_SHA256, &key_b);
        assert!(
            hmac::verify(
                &key,
                &record_authentication_data(&offer_id, &message_a, &message_b, payload),
                tag.as_ref(),
            )
            .is_err()
        );
    }

    #[test]
    fn loopback_exchange_authenticates_both_certificates() {
        let directory = std::env::temp_dir().join(format!(
            "edgemouse-pairing-loopback-{}-{}",
            std::process::id(),
            u64::from_be_bytes(random_array().unwrap())
        ));
        let host = pairing_config(
            "windows-pc",
            edgemouse_transport::Identity::generate().unwrap(),
            directory.join("mac.der"),
        );
        let joiner = pairing_config(
            "macbook",
            edgemouse_transport::Identity::generate().unwrap(),
            directory.join("windows.der"),
        );
        let host_certificate = host.local_certificate.clone();
        let joiner_certificate = joiner.local_certificate.clone();
        let offer = PairingOffer {
            id: [42_u8; 16],
            tcp_port: PAIRING_PORT,
            name: "windows-pc".to_owned(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            host_exchange(stream, &host, &offer, "12345678")
        });
        let host_record =
            join_exchange(address, Duration::from_secs(2), &joiner, None, "12345678").unwrap();
        let joiner_record = host_thread.join().unwrap().unwrap();

        assert_eq!(host_record.name, "windows-pc");
        assert_eq!(host_record.certificate, host_certificate);
        assert_eq!(joiner_record.name, "macbook");
        assert_eq!(joiner_record.certificate, joiner_certificate);
    }

    #[test]
    fn certificate_install_is_no_clobber_and_idempotent() {
        let directory = std::env::temp_dir().join(format!(
            "edgemouse-pairing-test-{}-{}",
            std::process::id(),
            u64::from_be_bytes(random_array().unwrap())
        ));
        let path = directory.join("peer.der");
        assert!(install_certificate(&path, b"first").unwrap());
        assert!(!install_certificate(&path, b"first").unwrap());
        assert!(install_certificate(&path, b"different").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"first");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn confirmation_is_bound_to_both_records_and_role() {
        let key = [1_u8; 32];
        let offer = [2_u8; 16];
        let client_spake = [3_u8; SPAKE_MESSAGE_LEN];
        let host_spake = [4_u8; SPAKE_MESSAGE_LEN];
        let host = b"host record";
        let client = b"client record";
        let tag = confirmation_tag(
            &key,
            &offer,
            &client_spake,
            &host_spake,
            host,
            client,
            b"host",
        );
        verify_confirmation(
            &key,
            &offer,
            &client_spake,
            &host_spake,
            host,
            client,
            b"host",
            tag.as_ref(),
        )
        .unwrap();
        assert!(
            verify_confirmation(
                &key,
                &offer,
                &client_spake,
                &host_spake,
                host,
                client,
                b"joiner",
                tag.as_ref(),
            )
            .is_err()
        );
    }

    #[test]
    fn node_ids_are_shown_in_errors_and_results_as_fixed_hex() {
        assert_eq!(edgemouse_transport::format_node_id(NodeId(1)).len(), 32);
    }
}
