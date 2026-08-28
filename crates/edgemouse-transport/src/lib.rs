//! Mutually authenticated QUIC transport for one trusted EdgeMouse peer.

#![forbid(unsafe_code)]

use edgemouse_core::NodeId;
use edgemouse_protocol::{
    HEADER_LEN, MOUSE_DATAGRAM_FRAME_LEN, ScreenInfo, WireMessage, decode_frame, encode_frame,
    expected_frame_len,
};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{
    ClientConfig, Connection, Endpoint, RecvStream, SendStream, ServerConfig, TransportConfig,
};
use ring::digest::{SHA256, digest};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, client, server};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const TLS_SERVER_NAME: &str = "edgemouse.local";
const ALPN: &[u8] = b"edgemouse/4";
const RETRY_INTERVAL: Duration = Duration::from_millis(500);
const DATAGRAM_RECEIVE_BUFFER_BYTES: usize = 4 * 1024;
// One fixed-size movement frame: congestion drops positions at the application latest-value slot.
const DATAGRAM_SEND_BUFFER_BYTES: usize = MOUSE_DATAGRAM_FRAME_LEN;
const CAPABILITY_MOUSE: u32 = 1 << 0;
const CAPABILITY_MOUSE_DATAGRAM: u32 = 1 << 1;
const REQUIRED_CAPABILITIES: u32 = CAPABILITY_MOUSE | CAPABILITY_MOUSE_DATAGRAM;

#[derive(Clone)]
pub struct Identity {
    certificate: CertificateDer<'static>,
    private_key: Arc<PrivateKeyDer<'static>>,
    node_id: NodeId,
}

impl Identity {
    pub fn from_der(certificate: Vec<u8>, private_key: Vec<u8>) -> Result<Self, TransportError> {
        if certificate.is_empty() || private_key.is_empty() {
            return Err(TransportError::new(
                "identity certificate and key cannot be empty",
            ));
        }
        let certificate = CertificateDer::from(certificate);
        let node_id = node_id_for_certificate(certificate.as_ref());
        Ok(Self {
            certificate,
            private_key: Arc::new(PrivatePkcs8KeyDer::from(private_key).into()),
            node_id,
        })
    }

    pub fn generate() -> Result<GeneratedIdentity, TransportError> {
        let generated = rcgen::generate_simple_self_signed(vec![TLS_SERVER_NAME.to_owned()])
            .map_err(|error| TransportError::with_source("failed to generate identity", error))?;
        let certificate = generated.cert.der().to_vec();
        let private_key = generated.signing_key.serialize_der();
        let node_id = node_id_for_certificate(&certificate);
        Ok(GeneratedIdentity {
            certificate,
            private_key,
            node_id,
        })
    }

    #[must_use]
    pub const fn node_id(&self) -> NodeId {
        self.node_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedIdentity {
    pub certificate: Vec<u8>,
    pub private_key: Vec<u8>,
    pub node_id: NodeId,
}

#[derive(Clone)]
pub struct TrustedPeer {
    certificate: CertificateDer<'static>,
    node_id: NodeId,
}

impl TrustedPeer {
    pub fn from_der(certificate: Vec<u8>) -> Result<Self, TransportError> {
        if certificate.is_empty() {
            return Err(TransportError::new("peer certificate cannot be empty"));
        }
        let certificate = CertificateDer::from(certificate);
        let mut roots = RootCertStore::empty();
        roots.add(certificate.clone()).map_err(|error| {
            TransportError::with_source("peer certificate is not valid DER", error)
        })?;
        let node_id = node_id_for_certificate(certificate.as_ref());
        Ok(Self {
            certificate,
            node_id,
        })
    }

    #[must_use]
    pub const fn node_id(&self) -> NodeId {
        self.node_id
    }
}

#[derive(Clone)]
pub struct PeerConfig {
    pub bind_address: SocketAddr,
    pub peer_address: SocketAddr,
    pub local_name: String,
    pub identity: Identity,
    pub peer: TrustedPeer,
    pub connect_timeout: Duration,
}

impl PeerConfig {
    pub fn validate(&self) -> Result<(), TransportError> {
        if self.local_name.is_empty() {
            return Err(TransportError::new("local peer name cannot be empty"));
        }
        if self.identity.node_id() == self.peer.node_id() {
            return Err(TransportError::new(
                "local and peer certificates identify the same node",
            ));
        }
        if self.connect_timeout.is_zero() {
            return Err(TransportError::new(
                "connect timeout must be greater than zero",
            ));
        }
        Ok(())
    }
}

pub struct PeerLink {
    guard: Arc<LinkGuard>,
    sender: SendStream,
    receiver: RecvStream,
    read_state: FrameReadState,
    peer_node: NodeId,
    peer_name: String,
    peer_screen: ScreenInfo,
}

impl PeerLink {
    pub async fn connect(
        config: PeerConfig,
        local_screen: ScreenInfo,
    ) -> Result<Self, TransportError> {
        config.validate()?;
        let server_config = make_server_config(&config.identity, &config.peer)?;
        let client_config = make_client_config(&config.identity, &config.peer)?;
        let mut endpoint = Endpoint::server(server_config, config.bind_address)
            .map_err(|error| TransportError::with_source("failed to bind QUIC endpoint", error))?;
        endpoint.set_default_client_config(client_config);

        let local_node = config.identity.node_id();
        let peer_node = config.peer.node_id();
        let (connection, sender, receiver) = if local_node < peer_node {
            let connection =
                connect_with_retry(&endpoint, config.peer_address, config.connect_timeout).await?;
            let (sender, receiver) = connection.open_bi().await.map_err(|error| {
                TransportError::with_source("failed to open control stream", error)
            })?;
            (connection, sender, receiver)
        } else {
            let incoming = tokio::time::timeout(config.connect_timeout, endpoint.accept())
                .await
                .map_err(|_| TransportError::new("timed out waiting for trusted peer"))?
                .ok_or_else(|| {
                    TransportError::new("QUIC endpoint closed while waiting for peer")
                })?;
            let connection = incoming.await.map_err(|error| {
                TransportError::with_source("failed to accept peer connection", error)
            })?;
            let (sender, receiver) = connection.accept_bi().await.map_err(|error| {
                TransportError::with_source("failed to accept control stream", error)
            })?;
            (connection, sender, receiver)
        };

        let mut link = Self {
            guard: Arc::new(LinkGuard {
                endpoint,
                connection,
            }),
            sender,
            receiver,
            read_state: FrameReadState::default(),
            peer_node,
            peer_name: String::new(),
            peer_screen: local_screen.clone(),
        };
        if link.guard.connection.max_datagram_size().is_none() {
            return Err(TransportError::new(
                "trusted peer did not negotiate QUIC Datagram support",
            ));
        }
        link.exchange_hello(local_node, &config.local_name, local_screen)
            .await?;
        Ok(link)
    }

    #[must_use]
    pub const fn peer_node(&self) -> NodeId {
        self.peer_node
    }

    #[must_use]
    pub fn peer_name(&self) -> &str {
        &self.peer_name
    }

    #[must_use]
    pub fn peer_screen(&self) -> &ScreenInfo {
        &self.peer_screen
    }

    pub fn local_address(&self) -> Result<SocketAddr, TransportError> {
        self.guard
            .endpoint
            .local_addr()
            .map_err(|error| TransportError::with_source("failed to read local address", error))
    }

    pub async fn send(&mut self, message: &WireMessage) -> Result<(), TransportError> {
        let frame = encode_frame(message)
            .map_err(|error| TransportError::with_source("failed to encode message", error))?;
        self.sender
            .write_all(&frame)
            .await
            .map_err(|error| TransportError::with_source("failed to write QUIC frame", error))
    }

    pub async fn receive(&mut self) -> Result<WireMessage, TransportError> {
        self.read_state.receive(&mut self.receiver).await
    }

    pub fn close(&self, reason: &'static [u8]) {
        self.guard.connection.close(0_u8.into(), reason);
    }

    #[must_use]
    pub fn split(self) -> (PeerSender, PeerReceiver, PeerDatagrams) {
        let guard = self.guard;
        (
            PeerSender {
                guard: Arc::clone(&guard),
                sender: self.sender,
            },
            PeerReceiver {
                guard: Arc::clone(&guard),
                receiver: self.receiver,
                read_state: self.read_state,
            },
            PeerDatagrams { guard },
        )
    }

    async fn exchange_hello(
        &mut self,
        local_node: NodeId,
        local_name: &str,
        local_screen: ScreenInfo,
    ) -> Result<(), TransportError> {
        self.send(&WireMessage::Hello {
            node: local_node,
            name: local_name.to_owned(),
            capabilities: REQUIRED_CAPABILITIES,
            screen: local_screen,
        })
        .await?;
        match self.receive().await? {
            WireMessage::Hello {
                node,
                name,
                capabilities,
                screen,
            } if node == self.peer_node
                && capabilities & REQUIRED_CAPABILITIES == REQUIRED_CAPABILITIES =>
            {
                self.peer_name = name;
                self.peer_screen = screen;
                Ok(())
            }
            WireMessage::Hello {
                node, capabilities, ..
            } if node == self.peer_node => Err(TransportError::new(format!(
                "peer is missing required capabilities (announced {capabilities:#010x})"
            ))),
            WireMessage::Hello { node, .. } => Err(TransportError::new(format!(
                "peer certificate identifies {}, but Hello announced {}",
                format_node_id(self.peer_node),
                format_node_id(node)
            ))),
            _ => Err(TransportError::new(
                "peer did not begin with a Hello message",
            )),
        }
    }
}

struct LinkGuard {
    endpoint: Endpoint,
    connection: Connection,
}

impl Drop for LinkGuard {
    fn drop(&mut self) {
        self.connection.close(0_u8.into(), b"edgemouse shutdown");
    }
}

pub struct PeerSender {
    guard: Arc<LinkGuard>,
    sender: SendStream,
}

impl PeerSender {
    pub async fn send(&mut self, message: &WireMessage) -> Result<(), TransportError> {
        let frame = encode_frame(message)
            .map_err(|error| TransportError::with_source("failed to encode message", error))?;
        self.sender
            .write_all(&frame)
            .await
            .map_err(|error| TransportError::with_source("failed to write QUIC frame", error))
    }

    pub fn close(&self, reason: &'static [u8]) {
        self.guard.connection.close(0_u8.into(), reason);
    }

    #[must_use]
    pub fn smoothed_rtt(&self) -> Duration {
        self.guard.connection.rtt()
    }
}

pub struct PeerReceiver {
    guard: Arc<LinkGuard>,
    receiver: RecvStream,
    read_state: FrameReadState,
}

pub struct PeerDatagrams {
    guard: Arc<LinkGuard>,
}

impl PeerDatagrams {
    /// Queues the newest movement only when Quinn has room without evicting an older datagram.
    ///
    /// `Ok(false)` means the caller should discard this stale position and try again with the
    /// next absolute position instead of allowing a network backlog to form.
    pub fn send(&self, message: &WireMessage) -> Result<bool, TransportError> {
        let frame = encode_frame(message)
            .map_err(|error| TransportError::with_source("failed to encode datagram", error))?;
        if self.guard.connection.datagram_send_buffer_space() < frame.len() {
            return Ok(false);
        }
        self.guard
            .connection
            .send_datagram(frame.into())
            .map_err(|error| TransportError::with_source("failed to send QUIC datagram", error))?;
        Ok(true)
    }

    pub async fn receive(&self) -> Result<WireMessage, TransportError> {
        let frame = self
            .guard
            .connection
            .read_datagram()
            .await
            .map_err(|error| TransportError::with_source("failed to read QUIC datagram", error))?;
        decode_frame(&frame)
            .map_err(|error| TransportError::with_source("invalid datagram from peer", error))
    }
}

impl PeerReceiver {
    pub async fn receive(&mut self) -> Result<WireMessage, TransportError> {
        self.read_state.receive(&mut self.receiver).await
    }

    #[must_use]
    pub fn remote_address(&self) -> SocketAddr {
        self.guard.connection.remote_address()
    }
}

async fn connect_with_retry(
    endpoint: &Endpoint,
    peer_address: SocketAddr,
    timeout: Duration,
) -> Result<Connection, TransportError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let connecting = endpoint
            .connect(peer_address, TLS_SERVER_NAME)
            .map_err(|error| {
                TransportError::with_source("invalid peer connection parameters", error)
            })?;
        let last_error = match connecting.await {
            Ok(connection) => return Ok(connection),
            Err(error) => error,
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(TransportError::with_source(
                "timed out connecting to trusted peer",
                last_error,
            ));
        }
        tokio::time::sleep(RETRY_INTERVAL).await;
    }
}

fn make_client_config(
    identity: &Identity,
    peer: &TrustedPeer,
) -> Result<ClientConfig, TransportError> {
    let roots = roots_for(peer)?;
    let mut crypto = client::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![identity.certificate.clone()],
            identity.private_key.as_ref().clone_key(),
        )
        .map_err(|error| TransportError::with_source("invalid client identity", error))?;
    crypto.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicClientConfig::try_from(crypto)
        .map_err(|error| TransportError::with_source("invalid QUIC client TLS config", error))?;
    let mut config = ClientConfig::new(Arc::new(crypto));
    config.transport_config(mouse_transport_config());
    Ok(config)
}

fn make_server_config(
    identity: &Identity,
    peer: &TrustedPeer,
) -> Result<ServerConfig, TransportError> {
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots_for(peer)?))
        .build()
        .map_err(|error| TransportError::with_source("invalid peer trust certificate", error))?;
    let mut crypto = server::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![identity.certificate.clone()],
            identity.private_key.as_ref().clone_key(),
        )
        .map_err(|error| TransportError::with_source("invalid server identity", error))?;
    crypto.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicServerConfig::try_from(crypto)
        .map_err(|error| TransportError::with_source("invalid QUIC server TLS config", error))?;
    let mut config = ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(mouse_transport_config());
    Ok(config)
}

fn mouse_transport_config() -> Arc<TransportConfig> {
    let mut transport = TransportConfig::default();
    transport.max_concurrent_uni_streams(0_u8.into());
    transport.max_concurrent_bidi_streams(1_u8.into());
    transport.datagram_receive_buffer_size(Some(DATAGRAM_RECEIVE_BUFFER_BYTES));
    transport.datagram_send_buffer_size(DATAGRAM_SEND_BUFFER_BYTES);
    Arc::new(transport)
}

fn roots_for(peer: &TrustedPeer) -> Result<RootCertStore, TransportError> {
    let mut roots = RootCertStore::empty();
    roots.add(peer.certificate.clone()).map_err(|error| {
        TransportError::with_source("peer certificate is not a trust anchor", error)
    })?;
    Ok(roots)
}

#[derive(Default)]
struct FrameReadState {
    bytes: Vec<u8>,
    expected_len: Option<usize>,
}

impl FrameReadState {
    /// Reads one frame while preserving partial progress if this future is cancelled.
    async fn receive(&mut self, receiver: &mut RecvStream) -> Result<WireMessage, TransportError> {
        let mut chunk = [0_u8; 4_096];
        loop {
            let target_len = self.expected_len.unwrap_or(HEADER_LEN);
            if self.bytes.len() == target_len {
                if self.expected_len.is_none() {
                    let frame_len = expected_frame_len(&self.bytes)
                        .map_err(|error| {
                            TransportError::with_source("invalid QUIC frame header", error)
                        })?
                        .ok_or_else(|| TransportError::new("incomplete QUIC frame header"))?;
                    self.expected_len = Some(frame_len);
                    continue;
                }

                let message = decode_frame(&self.bytes).map_err(|error| {
                    TransportError::with_source("invalid message from peer", error)
                });
                self.bytes.clear();
                self.expected_len = None;
                return message;
            }

            let remaining = target_len.saturating_sub(self.bytes.len());
            let read_len = remaining.min(chunk.len());
            let read = receiver
                .read(&mut chunk[..read_len])
                .await
                .map_err(|error| TransportError::with_source("failed to read QUIC frame", error))?
                .ok_or_else(|| {
                    TransportError::new(format!(
                        "QUIC control stream ended with {} bytes of an incomplete frame",
                        self.bytes.len()
                    ))
                })?;
            if read == 0 {
                return Err(TransportError::new(
                    "QUIC control stream returned an empty read",
                ));
            }
            self.bytes.extend_from_slice(&chunk[..read]);
        }
    }
}

#[must_use]
pub fn node_id_for_certificate(certificate: &[u8]) -> NodeId {
    let hash = digest(&SHA256, certificate);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash.as_ref()[..16]);
    NodeId(u128::from_be_bytes(bytes))
}

#[must_use]
pub fn format_node_id(node: NodeId) -> String {
    format!("{:032x}", node.0)
}

#[derive(Debug)]
pub struct TransportError {
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl TransportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    fn with_source(message: impl Into<String>, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl Display for TransportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl Error for TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;

    #[test]
    fn generated_identity_round_trips_and_has_stable_node_id() {
        let generated = Identity::generate().unwrap();
        let identity =
            Identity::from_der(generated.certificate.clone(), generated.private_key.clone())
                .unwrap();
        let peer = TrustedPeer::from_der(generated.certificate.clone()).unwrap();

        assert_eq!(generated.node_id, identity.node_id());
        assert_eq!(generated.node_id, peer.node_id());
        assert_eq!(format_node_id(generated.node_id).len(), 32);
    }

    #[test]
    fn refuses_to_connect_an_identity_to_itself() {
        let generated = Identity::generate().unwrap();
        let config = PeerConfig {
            bind_address: "127.0.0.1:0".parse().unwrap(),
            peer_address: "127.0.0.1:43891".parse().unwrap(),
            local_name: "same".to_owned(),
            identity: Identity::from_der(generated.certificate.clone(), generated.private_key)
                .unwrap(),
            peer: TrustedPeer::from_der(generated.certificate).unwrap(),
            connect_timeout: Duration::from_secs(1),
        };

        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn mutually_authenticated_peers_exchange_framed_messages() {
        let first = Identity::generate().unwrap();
        let second = Identity::generate().unwrap();
        let first_address = unused_udp_address();
        let second_address = unused_udp_address();
        let first_config = PeerConfig {
            bind_address: first_address,
            peer_address: second_address,
            local_name: "first".to_owned(),
            identity: Identity::from_der(first.certificate.clone(), first.private_key).unwrap(),
            peer: TrustedPeer::from_der(second.certificate.clone()).unwrap(),
            connect_timeout: Duration::from_secs(5),
        };
        let second_config = PeerConfig {
            bind_address: second_address,
            peer_address: first_address,
            local_name: "second".to_owned(),
            identity: Identity::from_der(second.certificate, second.private_key).unwrap(),
            peer: TrustedPeer::from_der(first.certificate).unwrap(),
            connect_timeout: Duration::from_secs(5),
        };

        let (first_link, second_link) = tokio::join!(
            PeerLink::connect(first_config, test_screen(1)),
            PeerLink::connect(second_config, test_screen(2))
        );
        let mut first_link = first_link.unwrap();
        let mut second_link = second_link.unwrap();
        assert_eq!(first_link.peer_name(), "second");
        assert_eq!(second_link.peer_name(), "first");
        assert_eq!(first_link.peer_screen(), &test_screen(2));
        assert_eq!(second_link.peer_screen(), &test_screen(1));

        let heartbeat = WireMessage::Heartbeat {
            session_id: 42,
            monotonic_ms: 123,
        };
        first_link.send(&heartbeat).await.unwrap();
        assert_eq!(second_link.receive().await.unwrap(), heartbeat);

        let (first_sender, _, first_datagrams) = first_link.split();
        let (second_sender, _, second_datagrams) = second_link.split();
        let movement = WireMessage::MouseDatagram {
            session_id: 42,
            after_sequence: 7,
            sequence: 8,
            screen: edgemouse_core::ScreenId(3),
            position: edgemouse_core::Point::new(10.5, 20.25),
        };
        assert!(first_datagrams.send(&movement).unwrap());
        assert_eq!(second_datagrams.receive().await.unwrap(), movement);

        // Keep the endpoint driver from draining the queue while this tight loop runs. The
        // application must skip excess latest-value movement instead of entering Quinn's
        // drop-oldest path or accumulating delayed cursor positions.
        let mut skipped = 0;
        for sequence in 9..10_000 {
            let movement = WireMessage::MouseDatagram {
                session_id: 42,
                after_sequence: 7,
                sequence,
                screen: edgemouse_core::ScreenId(3),
                position: edgemouse_core::Point::new(sequence as f64, 20.25),
            };
            if !first_datagrams.send(&movement).unwrap() {
                skipped += 1;
            }
        }
        assert!(skipped > 0);

        first_sender.close(b"test complete");
        second_sender.close(b"test complete");
    }

    #[tokio::test]
    async fn cancelled_receive_preserves_a_partial_frame() {
        let first = Identity::generate().unwrap();
        let second = Identity::generate().unwrap();
        let first_address = unused_udp_address();
        let second_address = unused_udp_address();
        let first_config = PeerConfig {
            bind_address: first_address,
            peer_address: second_address,
            local_name: "first".to_owned(),
            identity: Identity::from_der(first.certificate.clone(), first.private_key).unwrap(),
            peer: TrustedPeer::from_der(second.certificate.clone()).unwrap(),
            connect_timeout: Duration::from_secs(5),
        };
        let second_config = PeerConfig {
            bind_address: second_address,
            peer_address: first_address,
            local_name: "second".to_owned(),
            identity: Identity::from_der(second.certificate, second.private_key).unwrap(),
            peer: TrustedPeer::from_der(first.certificate).unwrap(),
            connect_timeout: Duration::from_secs(5),
        };

        let (first_link, second_link) = tokio::join!(
            PeerLink::connect(first_config, test_screen(1)),
            PeerLink::connect(second_config, test_screen(2))
        );
        let mut first_link = first_link.unwrap();
        let mut second_link = second_link.unwrap();
        let heartbeat = WireMessage::Heartbeat {
            session_id: 42,
            monotonic_ms: 123,
        };
        let frame = encode_frame(&heartbeat).unwrap();

        first_link.sender.write_all(&frame[..5]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), second_link.receive())
                .await
                .is_err()
        );
        first_link.sender.write_all(&frame[5..]).await.unwrap();
        assert_eq!(second_link.receive().await.unwrap(), heartbeat);

        first_link.close(b"test complete");
        second_link.close(b"test complete");
    }

    fn unused_udp_address() -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.local_addr().unwrap()
    }

    fn test_screen(id: u64) -> ScreenInfo {
        ScreenInfo {
            id: edgemouse_core::ScreenId(id),
            name: format!("screen-{id}"),
            bounds: edgemouse_core::Rect::new(edgemouse_core::Point::new(0.0, 0.0), 1920.0, 1080.0)
                .unwrap(),
            scale_factor: 1.0,
        }
    }
}
