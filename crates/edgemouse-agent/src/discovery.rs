use edgemouse_core::NodeId;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const DISCOVERY_PORT: u16 = 43_892;
const DISCOVERY_MAGIC: &[u8; 8] = b"EDGMOUSE";
const DISCOVERY_VERSION: u8 = 1;
const DISCOVERY_NAME_MAX_LEN: usize = 63;
const DISCOVERY_HEADER_LEN: usize = 28;
const DISCOVERY_PACKET_MAX_LEN: usize = DISCOVERY_HEADER_LEN + DISCOVERY_NAME_MAX_LEN;
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(5);
const RECEIVE_POLL_INTERVAL: Duration = Duration::from_millis(200);
#[cfg(any(target_os = "macos", test))]
const MAX_SUBNET_DISCOVERY_TARGETS: u64 = 2_048;

#[derive(Debug, Clone)]
pub struct DiscoveryRequest {
    pub local_node: NodeId,
    pub expected_peer: NodeId,
    pub local_name: String,
    pub quic_port: u16,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub node: NodeId,
    pub name: String,
    pub address: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Announcement {
    node: NodeId,
    quic_port: u16,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryError(String);

impl DiscoveryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn io(context: &str, error: io::Error) -> Self {
        Self::new(format!("{context}: {error}"))
    }
}

impl Display for DiscoveryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DiscoveryError {}

pub fn discover_trusted_peer(
    request: &DiscoveryRequest,
    stopping: &AtomicBool,
) -> Result<DiscoveredPeer, DiscoveryError> {
    let bind_address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT));
    let destinations = discovery_destinations();
    discover_on(request, stopping, bind_address, &destinations, true)
}

pub fn respond_to_trusted_peer(
    request: &DiscoveryRequest,
    stopping: &AtomicBool,
) -> Result<DiscoveredPeer, DiscoveryError> {
    let bind_address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT));
    discover_on(request, stopping, bind_address, &[], false)
}

fn discover_on(
    request: &DiscoveryRequest,
    stopping: &AtomicBool,
    bind_address: SocketAddr,
    destinations: &[SocketAddr],
    broadcast: bool,
) -> Result<DiscoveredPeer, DiscoveryError> {
    validate_request(request)?;
    let announcement = Announcement {
        node: request.local_node,
        quic_port: request.quic_port,
        name: request.local_name.clone(),
    };
    let packet = encode_announcement(&announcement)?;
    let socket = UdpSocket::bind(bind_address)
        .map_err(|error| DiscoveryError::io("failed to bind UDP discovery socket", error))?;
    socket
        .set_broadcast(broadcast)
        .map_err(|error| DiscoveryError::io("failed to configure UDP broadcast", error))?;
    socket
        .set_read_timeout(Some(RECEIVE_POLL_INTERVAL))
        .map_err(|error| DiscoveryError::io("failed to configure discovery timeout", error))?;

    if !destinations.is_empty() {
        send_announcements(&socket, &packet, destinations)?;
    }
    let started = Instant::now();
    let mut next_announcement = Instant::now() + ANNOUNCE_INTERVAL;
    // The extra byte lets us detect and reject an oversized datagram instead of
    // accidentally accepting its valid-looking prefix after truncation.
    let mut buffer = [0_u8; DISCOVERY_PACKET_MAX_LEN + 1];

    while started.elapsed() < request.timeout {
        if stopping.load(Ordering::Acquire) {
            return Err(DiscoveryError::new("peer discovery cancelled"));
        }
        if !destinations.is_empty() && Instant::now() >= next_announcement {
            send_announcements(&socket, &packet, destinations)?;
            next_announcement = Instant::now() + ANNOUNCE_INTERVAL;
        }

        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                if length > DISCOVERY_PACKET_MAX_LEN {
                    continue;
                }
                let Ok(candidate) = decode_announcement(&buffer[..length]) else {
                    continue;
                };
                if candidate.node == request.local_node || candidate.node != request.expected_peer {
                    continue;
                }

                // Reply directly before returning. This closes the race where one peer
                // discovers the other and stops broadcasting before its own announcement
                // reaches the second peer.
                let _ = socket.send_to(&packet, source);
                return Ok(DiscoveredPeer {
                    node: candidate.node,
                    name: candidate.name,
                    address: SocketAddr::new(source.ip(), candidate.quic_port),
                });
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        // Windows reports an ICMP "port unreachable" response to a
                        // previously sent UDP probe as WSAECONNRESET on recv_from.
                        // The peer may simply not have bound its discovery socket yet.
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionRefused
                ) => {}
            Err(error) => {
                return Err(DiscoveryError::io(
                    "failed to receive a UDP discovery announcement",
                    error,
                ));
            }
        }
    }

    Err(DiscoveryError::new(format!(
        "trusted peer {} was not discovered within {} second(s); ensure both agents use address = \"auto\" and allow inbound UDP {DISCOVERY_PORT}",
        edgemouse_transport::format_node_id(request.expected_peer),
        request.timeout.as_secs()
    )))
}

fn discovery_destinations() -> Vec<SocketAddr> {
    let mut addresses = Vec::new();

    // macOS can route the limited broadcast through a VPN interface, while some
    // routers and Windows firewall profiles drop directed broadcasts. Add a
    // bounded unicast sweep of each small local subnet so login agents can still
    // discover a DHCP peer without storing yesterday's IP address.
    #[cfg(target_os = "macos")]
    if let Ok(local_addresses) = macos_discovery_addresses() {
        addresses.extend(
            local_addresses
                .into_iter()
                .map(|address| SocketAddr::V4(SocketAddrV4::new(address, DISCOVERY_PORT))),
        );
    }

    addresses.push(SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::BROADCAST,
        DISCOVERY_PORT,
    )));
    addresses.sort_unstable();
    addresses.dedup();
    addresses
}

#[cfg(target_os = "macos")]
fn macos_discovery_addresses() -> io::Result<Vec<Ipv4Addr>> {
    struct InterfaceAddresses(*mut libc::ifaddrs);

    impl Drop for InterfaceAddresses {
        fn drop(&mut self) {
            // SAFETY: getifaddrs allocated this list, and this guard owns it.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }

    let mut head = std::ptr::null_mut();
    // SAFETY: head is a valid output pointer and is freed exactly once below.
    if unsafe { libc::getifaddrs(&raw mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if head.is_null() {
        return Ok(Vec::new());
    }
    let _addresses = InterfaceAddresses(head);
    let mut current = head;
    let mut addresses = Vec::new();

    while !current.is_null() {
        // SAFETY: current belongs to the live getifaddrs list and remains valid
        // until the InterfaceAddresses guard is dropped after this loop.
        let interface = unsafe { &*current };
        let flags = interface.ifa_flags;
        let is_usable = flags & (libc::IFF_UP as u32) != 0
            && flags & (libc::IFF_BROADCAST as u32) != 0
            && flags & (libc::IFF_LOOPBACK as u32) == 0;
        if is_usable {
            let local = ipv4_from_sockaddr(interface.ifa_addr);
            let netmask = ipv4_from_sockaddr(interface.ifa_netmask);
            let broadcast = ipv4_from_sockaddr(interface.ifa_dstaddr);
            if let Some(broadcast) = broadcast
                && !broadcast.is_unspecified()
                && broadcast != Ipv4Addr::BROADCAST
            {
                addresses.push(broadcast);
            }
            if let (Some(local), Some(netmask)) = (local, netmask) {
                addresses.extend(subnet_discovery_targets(local, netmask));
            }
        }
        current = interface.ifa_next;
    }

    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

#[cfg(target_os = "macos")]
fn ipv4_from_sockaddr(address: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if address.is_null() {
        return None;
    }
    // SAFETY: address points into the live getifaddrs list. The family check
    // happens before interpreting the OS-provided structure as sockaddr_in.
    if i32::from(unsafe { (*address).sa_family }) != libc::AF_INET {
        return None;
    }
    // SAFETY: AF_INET guarantees sockaddr_in layout and alignment here.
    let address = unsafe { &*address.cast::<libc::sockaddr_in>() };
    Some(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()))
}

#[cfg(any(target_os = "macos", test))]
fn subnet_discovery_targets(local: Ipv4Addr, netmask: Ipv4Addr) -> Vec<Ipv4Addr> {
    let local = u32::from(local);
    let netmask = u32::from(netmask);
    let prefix_length = netmask.leading_ones();
    let expected_netmask = if prefix_length == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_length)
    };
    if netmask != expected_netmask {
        return Vec::new();
    }

    let address_count = 1_u64 << (32 - prefix_length);
    if address_count <= 2 || address_count - 2 > MAX_SUBNET_DISCOVERY_TARGETS {
        return Vec::new();
    }

    let network = local & netmask;
    let broadcast = network | !netmask;
    ((network + 1)..broadcast)
        .filter(|candidate| *candidate != local)
        .map(Ipv4Addr::from)
        .collect()
}

fn validate_request(request: &DiscoveryRequest) -> Result<(), DiscoveryError> {
    if request.local_node.0 == 0 || request.expected_peer.0 == 0 {
        return Err(DiscoveryError::new("discovery node IDs must be non-zero"));
    }
    if request.local_node == request.expected_peer {
        return Err(DiscoveryError::new(
            "discovery local and peer node IDs must differ",
        ));
    }
    if request.quic_port == 0 {
        return Err(DiscoveryError::new("discovery QUIC port must be non-zero"));
    }
    if request.timeout.is_zero() {
        return Err(DiscoveryError::new(
            "discovery timeout must be greater than zero",
        ));
    }
    validate_name(&request.local_name)
}

fn validate_name(name: &str) -> Result<(), DiscoveryError> {
    if name.is_empty() {
        return Err(DiscoveryError::new("discovery name cannot be empty"));
    }
    if name.len() > DISCOVERY_NAME_MAX_LEN {
        return Err(DiscoveryError::new(format!(
            "discovery name cannot exceed {DISCOVERY_NAME_MAX_LEN} UTF-8 bytes"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(DiscoveryError::new(
            "discovery name cannot contain control characters",
        ));
    }
    Ok(())
}

fn send_announcements(
    socket: &UdpSocket,
    packet: &[u8],
    destinations: &[SocketAddr],
) -> Result<(), DiscoveryError> {
    let mut successful = 0_usize;
    let mut failures = Vec::new();
    for destination in destinations {
        match socket.send_to(packet, destination) {
            Ok(sent) if sent == packet.len() => successful += 1,
            Ok(_) => failures.push(format!("{destination}: datagram was only partially sent")),
            Err(error) => failures.push(format!("{destination}: {error}")),
        }
    }
    if successful == 0 {
        return Err(DiscoveryError::new(format!(
            "failed to send UDP discovery announcement to every destination ({})",
            failures.join("; ")
        )));
    }
    Ok(())
}

fn encode_announcement(announcement: &Announcement) -> Result<Vec<u8>, DiscoveryError> {
    if announcement.node.0 == 0 {
        return Err(DiscoveryError::new(
            "discovery announcement node ID must be non-zero",
        ));
    }
    if announcement.quic_port == 0 {
        return Err(DiscoveryError::new(
            "discovery announcement QUIC port must be non-zero",
        ));
    }
    validate_name(&announcement.name)?;
    let name_length = u8::try_from(announcement.name.len())
        .map_err(|_| DiscoveryError::new("discovery name length does not fit in one byte"))?;
    let mut packet = Vec::with_capacity(DISCOVERY_HEADER_LEN + announcement.name.len());
    packet.extend_from_slice(DISCOVERY_MAGIC);
    packet.push(DISCOVERY_VERSION);
    packet.extend_from_slice(&announcement.node.0.to_be_bytes());
    packet.extend_from_slice(&announcement.quic_port.to_be_bytes());
    packet.push(name_length);
    packet.extend_from_slice(announcement.name.as_bytes());
    Ok(packet)
}

fn decode_announcement(packet: &[u8]) -> Result<Announcement, DiscoveryError> {
    if packet.len() < DISCOVERY_HEADER_LEN {
        return Err(DiscoveryError::new("discovery announcement is truncated"));
    }
    if &packet[..DISCOVERY_MAGIC.len()] != DISCOVERY_MAGIC {
        return Err(DiscoveryError::new(
            "discovery announcement has an invalid magic value",
        ));
    }
    if packet[8] != DISCOVERY_VERSION {
        return Err(DiscoveryError::new(format!(
            "unsupported discovery version {}",
            packet[8]
        )));
    }
    let node_bytes: [u8; 16] = packet[9..25]
        .try_into()
        .map_err(|_| DiscoveryError::new("discovery node ID is truncated"))?;
    let node = NodeId(u128::from_be_bytes(node_bytes));
    if node.0 == 0 {
        return Err(DiscoveryError::new(
            "discovery announcement node ID must be non-zero",
        ));
    }
    let port_bytes: [u8; 2] = packet[25..27]
        .try_into()
        .map_err(|_| DiscoveryError::new("discovery port is truncated"))?;
    let quic_port = u16::from_be_bytes(port_bytes);
    if quic_port == 0 {
        return Err(DiscoveryError::new(
            "discovery announcement QUIC port must be non-zero",
        ));
    }
    let name_length = usize::from(packet[27]);
    if name_length > DISCOVERY_NAME_MAX_LEN || packet.len() != DISCOVERY_HEADER_LEN + name_length {
        return Err(DiscoveryError::new(
            "discovery announcement has an invalid name length",
        ));
    }
    let name = std::str::from_utf8(&packet[DISCOVERY_HEADER_LEN..])
        .map_err(|_| DiscoveryError::new("discovery name is not valid UTF-8"))?
        .to_owned();
    validate_name(&name)?;
    Ok(Announcement {
        node,
        quic_port,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn request(local: u128, peer: u128, name: &str, quic_port: u16) -> DiscoveryRequest {
        DiscoveryRequest {
            local_node: NodeId(local),
            expected_peer: NodeId(peer),
            local_name: name.to_owned(),
            quic_port,
            timeout: Duration::from_secs(2),
        }
    }

    #[test]
    fn announcement_round_trips_and_rejects_malformed_packets() {
        let announcement = Announcement {
            node: NodeId(42),
            quic_port: 43_891,
            name: "Windows 电脑".to_owned(),
        };
        let packet = encode_announcement(&announcement).unwrap();
        assert_eq!(decode_announcement(&packet).unwrap(), announcement);

        assert!(decode_announcement(&packet[..10]).is_err());
        let mut wrong_magic = packet.clone();
        wrong_magic[0] ^= 0xff;
        assert!(decode_announcement(&wrong_magic).is_err());
        let mut trailing = packet.clone();
        trailing.push(0);
        assert!(decode_announcement(&trailing).is_err());
    }

    #[test]
    fn trusted_peers_discover_each_other_over_udp() {
        let first_probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let first_address = first_probe.local_addr().unwrap();
        let second_probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let second_address = second_probe.local_addr().unwrap();
        drop(first_probe);
        drop(second_probe);

        let stopping = Arc::new(AtomicBool::new(false));
        let first_stopping = Arc::clone(&stopping);
        let second_stopping = Arc::clone(&stopping);
        let first = std::thread::spawn(move || {
            discover_on(
                &request(1, 2, "first", 50_001),
                &first_stopping,
                first_address,
                &[second_address],
                false,
            )
        });
        let second = std::thread::spawn(move || {
            discover_on(
                &request(2, 1, "second", 50_002),
                &second_stopping,
                second_address,
                &[first_address],
                false,
            )
        });

        let discovered_by_first = first.join().unwrap().unwrap();
        let discovered_by_second = second.join().unwrap().unwrap();
        assert_eq!(discovered_by_first.node, NodeId(2));
        assert_eq!(discovered_by_first.name, "second");
        assert_eq!(
            discovered_by_first.address,
            "127.0.0.1:50002".parse().unwrap()
        );
        assert_eq!(discovered_by_second.node, NodeId(1));
        assert_eq!(discovered_by_second.name, "first");
        assert_eq!(
            discovered_by_second.address,
            "127.0.0.1:50001".parse().unwrap()
        );
    }

    #[test]
    fn discovery_ignores_an_untrusted_node() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        let listener_address = listener.local_addr().unwrap();
        drop(listener);
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender_address = sender.local_addr().unwrap();
        drop(sender);

        let stopping = Arc::new(AtomicBool::new(false));
        let discovery_stopping = Arc::clone(&stopping);
        let discoverer = std::thread::spawn(move || {
            let mut request = request(1, 2, "first", 50_001);
            request.timeout = Duration::from_millis(500);
            discover_on(
                &request,
                &discovery_stopping,
                listener_address,
                &[sender_address],
                false,
            )
        });

        std::thread::sleep(Duration::from_millis(50));
        let sender = UdpSocket::bind(sender_address).unwrap();
        let packet = encode_announcement(&Announcement {
            node: NodeId(99),
            quic_port: 50_099,
            name: "untrusted".to_owned(),
        })
        .unwrap();
        sender.send_to(&packet, listener_address).unwrap();
        assert!(discoverer.join().unwrap().is_err());
    }

    #[test]
    fn cancelled_discovery_returns_promptly() {
        let bind_probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let bind_address = bind_probe.local_addr().unwrap();
        drop(bind_probe);
        let destination_probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let destination = destination_probe.local_addr().unwrap();
        drop(destination_probe);
        let stopping = AtomicBool::new(true);

        let result = discover_on(
            &request(1, 2, "first", 50_001),
            &stopping,
            bind_address,
            &[destination],
            false,
        );
        assert_eq!(result.unwrap_err().to_string(), "peer discovery cancelled");
    }

    #[test]
    fn a_listener_replies_without_needing_to_announce() {
        let listener_probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let listener_address = listener_probe.local_addr().unwrap();
        drop(listener_probe);
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let stopping = AtomicBool::new(false);

        let listener = std::thread::spawn(move || {
            discover_on(
                &request(2, 1, "listener", 50_002),
                &stopping,
                listener_address,
                &[],
                false,
            )
        });
        std::thread::sleep(Duration::from_millis(50));
        let packet = encode_announcement(&Announcement {
            node: NodeId(1),
            quic_port: 50_001,
            name: "sender".to_owned(),
        })
        .unwrap();
        sender.send_to(&packet, listener_address).unwrap();
        let mut reply = [0_u8; DISCOVERY_PACKET_MAX_LEN];
        let (reply_length, _) = sender.recv_from(&mut reply).unwrap();

        assert_eq!(
            decode_announcement(&reply[..reply_length]).unwrap().node,
            NodeId(2)
        );
        assert_eq!(listener.join().unwrap().unwrap().node, NodeId(1));
    }

    #[test]
    fn subnet_targets_cover_a_small_dhcp_network_without_self_or_broadcast() {
        let targets = subnet_discovery_targets(
            "192.168.8.189".parse().unwrap(),
            "255.255.254.0".parse().unwrap(),
        );
        assert_eq!(targets.len(), 509);
        assert!(targets.contains(&"192.168.8.201".parse().unwrap()));
        assert!(!targets.contains(&"192.168.8.189".parse().unwrap()));
        assert!(!targets.contains(&"192.168.8.0".parse().unwrap()));
        assert!(!targets.contains(&"192.168.9.255".parse().unwrap()));
    }

    #[test]
    fn subnet_targets_refuse_an_excessive_or_invalid_sweep() {
        assert!(
            subnet_discovery_targets(
                "192.168.8.189".parse().unwrap(),
                "255.255.0.0".parse().unwrap(),
            )
            .is_empty()
        );
        assert!(
            subnet_discovery_targets(
                "192.168.8.189".parse().unwrap(),
                "255.0.255.0".parse().unwrap(),
            )
            .is_empty()
        );
    }
}
