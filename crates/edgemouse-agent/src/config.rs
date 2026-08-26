use edgemouse_core::{Edge, NodeId, Point, Rect, Screen, ScreenId, SessionConfig, Topology};
use edgemouse_transport::{Identity, PeerConfig, TrustedPeer};
use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAddress {
    Static(SocketAddr),
    Auto,
}

#[derive(Clone)]
pub struct LoadedConfig {
    pub transport: PeerConfig,
    pub peer_address: PeerAddress,
    pub topology: Topology,
    pub local_node: NodeId,
    pub peer_node: NodeId,
    pub local_screen: ScreenId,
    pub local_bounds: Rect,
    pub local_scale: f64,
    pub session: SessionConfig,
}

impl LoadedConfig {
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let raw: RawConfig = toml::from_str(&source)
            .map_err(|error| format!("invalid config {}: {error}", path.display()))?;
        raw.finish(path.parent().unwrap_or_else(|| Path::new(".")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    local: RawLocal,
    peer: RawPeer,
    layout: RawLayout,
    #[serde(default)]
    session: RawSession,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLocal {
    name: String,
    listen: String,
    certificate: PathBuf,
    private_key: PathBuf,
    screen: RawScreen,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPeer {
    address: String,
    certificate: PathBuf,
    screen: RawScreen,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScreen {
    id: u64,
    name: String,
    #[serde(default)]
    origin_x: f64,
    #[serde(default)]
    origin_y: f64,
    width: f64,
    height: f64,
    #[serde(default = "default_scale")]
    scale: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayout {
    peer_on: String,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawSession {
    hysteresis: f64,
    timeout_ms: u64,
    connect_timeout_seconds: u64,
}

impl Default for RawSession {
    fn default() -> Self {
        Self {
            hysteresis: 8.0,
            timeout_ms: 1_500,
            connect_timeout_seconds: 30,
        }
    }
}

impl RawConfig {
    fn finish(self, base: &Path) -> Result<LoadedConfig, Box<dyn Error>> {
        let certificate = read_relative(base, &self.local.certificate, "local certificate")?;
        let private_key = read_relative(base, &self.local.private_key, "local private key")?;
        let peer_certificate = read_relative(base, &self.peer.certificate, "peer certificate")?;
        let identity = Identity::from_der(certificate, private_key)?;
        let peer = TrustedPeer::from_der(peer_certificate)?;
        let local_node = identity.node_id();
        let peer_node = peer.node_id();
        let local_screen = ScreenId(self.local.screen.id);
        let peer_screen = ScreenId(self.peer.screen.id);
        let local_bounds = self.local.screen.bounds()?;
        let local_scale = self.local.screen.scale;

        let mut topology = Topology::default();
        topology.add_screen(self.local.screen.build(local_node)?)?;
        topology.add_screen(self.peer.screen.build(peer_node)?)?;
        topology.connect_bidirectional(
            local_screen,
            parse_edge(&self.layout.peer_on)?,
            peer_screen,
        )?;

        let bind_address = parse_address(&self.local.listen, "local listen address")?;
        let peer_address = parse_peer_address(&self.peer.address)?;
        let transport = PeerConfig {
            bind_address,
            // Auto discovery replaces this sentinel before the transport starts.
            peer_address: match peer_address {
                PeerAddress::Static(address) => address,
                PeerAddress::Auto => "0.0.0.0:0".parse()?,
            },
            local_name: self.local.name,
            identity,
            peer,
            connect_timeout: Duration::from_secs(self.session.connect_timeout_seconds),
        };
        transport.validate()?;

        let session = SessionConfig {
            entry_hysteresis: self.session.hysteresis,
            peer_timeout_ms: self.session.timeout_ms,
            block_switch_while_dragging: true,
        };

        Ok(LoadedConfig {
            transport,
            peer_address,
            topology,
            local_node,
            peer_node,
            local_screen,
            local_bounds,
            local_scale,
            session,
        })
    }
}

impl RawScreen {
    fn bounds(&self) -> Result<Rect, Box<dyn Error>> {
        Ok(Rect::new(
            Point::new(self.origin_x, self.origin_y),
            self.width,
            self.height,
        )?)
    }

    fn build(&self, node: NodeId) -> Result<Screen, Box<dyn Error>> {
        Ok(Screen::new(
            ScreenId(self.id),
            node,
            &self.name,
            self.bounds()?,
            self.scale,
        )?)
    }
}

fn read_relative(base: &Path, path: &Path, label: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let resolved = if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    };
    fs::read(&resolved)
        .map_err(|error| format!("failed to read {label} {}: {error}", resolved.display()).into())
}

fn parse_address(value: &str, label: &str) -> Result<SocketAddr, Box<dyn Error>> {
    value
        .parse()
        .map_err(|error| format!("invalid {label} `{value}`: {error}").into())
}

fn parse_peer_address(value: &str) -> Result<PeerAddress, Box<dyn Error>> {
    if value.eq_ignore_ascii_case("auto") {
        Ok(PeerAddress::Auto)
    } else {
        Ok(PeerAddress::Static(parse_address(value, "peer address")?))
    }
}

fn parse_edge(value: &str) -> Result<Edge, Box<dyn Error>> {
    match value.to_ascii_lowercase().as_str() {
        "left" => Ok(Edge::Left),
        "right" => Ok(Edge::Right),
        "top" => Ok(Edge::Top),
        "bottom" => Ok(Edge::Bottom),
        _ => {
            Err(format!("layout.peer_on must be left, right, top, or bottom; got `{value}`").into())
        }
    }
}

const fn default_scale() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_edge_names_case_insensitively() {
        assert_eq!(parse_edge("LEFT").unwrap(), Edge::Left);
        assert_eq!(parse_edge("right").unwrap(), Edge::Right);
        assert_eq!(parse_edge("Top").unwrap(), Edge::Top);
        assert_eq!(parse_edge("bottom").unwrap(), Edge::Bottom);
        assert!(parse_edge("diagonal").is_err());
    }

    #[test]
    fn session_defaults_are_safety_oriented() {
        let session = RawSession::default();
        assert_eq!(session.hysteresis, 8.0);
        assert_eq!(session.timeout_ms, 1_500);
        assert_eq!(session.connect_timeout_seconds, 30);
    }

    #[test]
    fn peer_address_accepts_auto_case_insensitively() {
        assert_eq!(parse_peer_address("auto").unwrap(), PeerAddress::Auto);
        assert_eq!(parse_peer_address("AUTO").unwrap(), PeerAddress::Auto);
    }

    #[test]
    fn peer_address_preserves_static_socket_addresses() {
        assert_eq!(
            parse_peer_address("192.168.8.202:43891").unwrap(),
            PeerAddress::Static("192.168.8.202:43891".parse().unwrap())
        );
        assert!(parse_peer_address("automatic").is_err());
    }
}
