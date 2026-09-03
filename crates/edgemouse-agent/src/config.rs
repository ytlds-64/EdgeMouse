use edgemouse_core::{Edge, NodeId, Point, Rect, Screen, ScreenId, SessionConfig, Topology};
use edgemouse_protocol::ScreenInfo;
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
    pub local_node: NodeId,
    pub peer_node: NodeId,
    pub local_screen: ScreenConfig,
    pub peer_screen: ScreenId,
    pub peer_screen_name: String,
    pub peer_on: Edge,
    pub session: SessionConfig,
    pub windows_raw_input: bool,
    pub reverse_scroll_horizontal: bool,
    pub reverse_scroll_vertical: bool,
}

#[derive(Debug, Clone)]
pub struct ScreenConfig {
    pub id: ScreenId,
    pub name: String,
    pub automatic: bool,
    manual_bounds: Option<Rect>,
    manual_scale: f64,
}

#[derive(Debug, Clone)]
pub struct ResolvedScreen {
    pub screen: Screen,
    /// Converts per-monitor-aware Windows hook coordinates into topology coordinates.
    /// Automatic desktop detection uses the OS global coordinate space directly.
    pub coordinate_scale: f64,
}

#[derive(Debug, Clone)]
pub struct PairingConfig {
    pub local_name: String,
    pub local_certificate: Vec<u8>,
    pub local_node: NodeId,
    pub peer_certificate_path: PathBuf,
    pub timeout: Duration,
}

impl LoadedConfig {
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let raw = read_raw(path)?;
        raw.finish(path.parent().unwrap_or_else(|| Path::new(".")))
    }
}

pub fn persist_peer_on(path: &Path, peer_on: Edge) -> Result<(), String> {
    let original = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let updated = replace_table_value(
        &original,
        "layout",
        "peer_on",
        &format!("\"{}\"", edge_name(peer_on)),
    );
    fs::write(path, updated)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    if let Err(error) = LoadedConfig::load(path) {
        let restore_error = fs::write(path, original).err();
        return Err(match restore_error {
            Some(restore_error) => format!(
                "saved configuration is invalid: {error}; restoring the original also failed: {restore_error}"
            ),
            None => format!("saved configuration is invalid; restored the original: {error}"),
        });
    }
    Ok(())
}

pub const fn edge_name(edge: Edge) -> &'static str {
    match edge {
        Edge::Left => "left",
        Edge::Right => "right",
        Edge::Top => "top",
        Edge::Bottom => "bottom",
    }
}

fn replace_table_value(source: &str, table: &str, key: &str, value: &str) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let had_trailing_newline = source.ends_with('\n');
    let mut lines = source.lines().map(str::to_owned).collect::<Vec<_>>();
    let header = format!("[{table}]");
    let start = match lines.iter().position(|line| line.trim() == header) {
        Some(index) => index,
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(header);
            lines.len() - 1
        }
    };
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, line)| {
            let line = line.trim();
            (line.starts_with('[') && line.ends_with(']')).then_some(index)
        })
        .unwrap_or(lines.len());
    let existing = lines[start + 1..end].iter().position(|line| {
        line.split_once('=')
            .is_some_and(|(candidate, _)| candidate.trim() == key)
    });
    let replacement = format!("{key} = {value}");
    if let Some(offset) = existing {
        lines[start + 1 + offset] = replacement;
    } else {
        let mut insertion_index = end;
        while insertion_index > start + 1 && lines[insertion_index - 1].trim().is_empty() {
            insertion_index -= 1;
        }
        lines.insert(insertion_index, replacement);
    }

    let mut updated = lines.join(newline);
    if had_trailing_newline || source.is_empty() {
        updated.push_str(newline);
    }
    updated
}

impl PairingConfig {
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let raw = read_raw(path)?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let local_certificate = read_relative(base, &raw.local.certificate, "local certificate")?;
        let private_key = read_relative(base, &raw.local.private_key, "local private key")?;
        let identity = Identity::from_der(local_certificate.clone(), private_key)?;
        let peer_certificate_path = resolve_relative(base, &raw.peer.certificate);
        if raw.local.name.is_empty() || raw.local.name.chars().any(char::is_control) {
            return Err("local.name must be non-empty and contain no control characters".into());
        }
        if raw.local.name.len() > 63 {
            return Err("local.name cannot exceed 63 UTF-8 bytes".into());
        }
        if raw.session.connect_timeout_seconds == 0 {
            return Err("session.connect_timeout_seconds must be greater than zero".into());
        }
        Ok(Self {
            local_name: raw.local.name,
            local_certificate,
            local_node: identity.node_id(),
            peer_certificate_path,
            timeout: Duration::from_secs(raw.session.connect_timeout_seconds),
        })
    }
}

fn read_raw(path: &Path) -> Result<RawConfig, Box<dyn Error>> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    toml::from_str(&source)
        .map_err(|error| format!("invalid config {}: {error}", path.display()).into())
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
    #[serde(default = "default_auto")]
    auto: bool,
    #[serde(default)]
    origin_x: f64,
    #[serde(default)]
    origin_y: f64,
    width: Option<f64>,
    height: Option<f64>,
    scale: Option<f64>,
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
    windows_raw_input: bool,
    reverse_scroll_horizontal: bool,
    reverse_scroll_vertical: bool,
}

impl Default for RawSession {
    fn default() -> Self {
        Self {
            hysteresis: 8.0,
            timeout_ms: 1_500,
            connect_timeout_seconds: 30,
            windows_raw_input: true,
            reverse_scroll_horizontal: false,
            reverse_scroll_vertical: false,
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
        let local_screen = self.local.screen.into_config()?;
        let peer_screen = ScreenId(self.peer.screen.id);
        if local_screen.id == peer_screen {
            return Err("local and peer screen ids must be different".into());
        }
        validate_screen_name(&self.peer.screen.name, "peer.screen.name")?;
        let peer_screen_name = self.peer.screen.name;
        let peer_on = parse_edge(&self.layout.peer_on)?;

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
            local_node,
            peer_node,
            local_screen,
            peer_screen,
            peer_screen_name,
            peer_on,
            session,
            windows_raw_input: self.session.windows_raw_input,
            reverse_scroll_horizontal: self.session.reverse_scroll_horizontal,
            reverse_scroll_vertical: self.session.reverse_scroll_vertical,
        })
    }
}

impl RawScreen {
    fn into_config(self) -> Result<ScreenConfig, Box<dyn Error>> {
        validate_screen_name(&self.name, "local.screen.name")?;
        let manual_bounds = if self.auto {
            None
        } else {
            let width = self
                .width
                .ok_or("local.screen.width is required unless local.screen.auto = true")?;
            let height = self
                .height
                .ok_or("local.screen.height is required unless local.screen.auto = true")?;
            Some(Rect::new(
                Point::new(self.origin_x, self.origin_y),
                width,
                height,
            )?)
        };
        let manual_scale = self.scale.unwrap_or_else(default_scale);
        if !manual_scale.is_finite() || manual_scale <= 0.0 {
            return Err("local.screen.scale must be finite and greater than zero".into());
        }
        Ok(ScreenConfig {
            id: ScreenId(self.id),
            name: self.name,
            automatic: self.auto,
            manual_bounds,
            manual_scale,
        })
    }
}

impl LoadedConfig {
    pub fn resolve_local_screen(
        &self,
        detected: Option<(Rect, f64)>,
    ) -> Result<ResolvedScreen, Box<dyn Error>> {
        let (bounds, scale_factor, coordinate_scale) = if self.local_screen.automatic {
            let (bounds, scale_factor) = detected.ok_or(
                "automatic screen detection was requested, but no desktop geometry was supplied",
            )?;
            (bounds, scale_factor, 1.0)
        } else {
            (
                self.local_screen
                    .manual_bounds
                    .ok_or("manual local screen bounds are missing")?,
                self.local_screen.manual_scale,
                self.local_screen.manual_scale,
            )
        };
        Ok(ResolvedScreen {
            screen: Screen::new(
                self.local_screen.id,
                self.local_node,
                &self.local_screen.name,
                bounds,
                scale_factor,
            )?,
            coordinate_scale,
        })
    }

    pub fn topology(&self, local: Screen, peer: &ScreenInfo) -> Result<Topology, Box<dyn Error>> {
        if peer.id != self.peer_screen {
            return Err(format!(
                "trusted peer announced screen {}, but configuration expects {}",
                peer.id.0, self.peer_screen.0
            )
            .into());
        }
        let remote = Screen::new(
            peer.id,
            self.peer_node,
            &peer.name,
            peer.bounds,
            peer.scale_factor,
        )?;
        let mut topology = Topology::default();
        topology.add_screen(local)?;
        topology.add_screen(remote)?;
        topology.connect_bidirectional(self.local_screen.id, self.peer_on, self.peer_screen)?;
        Ok(topology)
    }
}

fn validate_screen_name(name: &str, label: &str) -> Result<(), Box<dyn Error>> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(format!("{label} must be non-empty and contain no control characters").into());
    }
    if name.len() > 63 {
        return Err(format!("{label} cannot exceed 63 UTF-8 bytes").into());
    }
    Ok(())
}

fn read_relative(base: &Path, path: &Path, label: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let resolved = resolve_relative(base, path);
    fs::read(&resolved)
        .map_err(|error| format!("failed to read {label} {}: {error}", resolved.display()).into())
}

fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
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

const fn default_auto() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_all_edge_names_case_insensitively() {
        assert_eq!(parse_edge("LEFT").unwrap(), Edge::Left);
        assert_eq!(parse_edge("right").unwrap(), Edge::Right);
        assert_eq!(parse_edge("Top").unwrap(), Edge::Top);
        assert_eq!(parse_edge("bottom").unwrap(), Edge::Bottom);
        assert!(parse_edge("diagonal").is_err());
    }

    #[test]
    fn layout_edge_replacement_preserves_other_tables_and_line_endings() {
        let source = "[local]\r\nname = \"Windows\"\r\n\r\n[layout]\r\npeer_on = \"right\"\r\n\r\n[session]\r\ntimeout_ms = 1500\r\n";
        let updated = replace_table_value(source, "layout", "peer_on", "\"top\"");
        assert!(updated.contains("[layout]\r\npeer_on = \"top\"\r\n"));
        assert!(updated.contains("[session]\r\ntimeout_ms = 1500\r\n"));
    }

    #[test]
    fn missing_table_and_value_are_inserted() {
        let updated =
            replace_table_value("[local]\nname = \"Mac\"\n", "layout", "peer_on", "\"left\"");
        assert!(updated.ends_with("\n[layout]\npeer_on = \"left\"\n"));
    }

    #[test]
    fn session_defaults_are_safety_oriented() {
        let session = RawSession::default();
        assert_eq!(session.hysteresis, 8.0);
        assert_eq!(session.timeout_ms, 1_500);
        assert_eq!(session.connect_timeout_seconds, 30);
        assert!(session.windows_raw_input);
        assert!(!session.reverse_scroll_horizontal);
        assert!(!session.reverse_scroll_vertical);
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

    #[test]
    fn automatic_screen_does_not_require_fixed_geometry() {
        let screen = RawScreen {
            id: 7,
            name: "Automatic desktop".to_owned(),
            auto: true,
            origin_x: 0.0,
            origin_y: 0.0,
            width: None,
            height: None,
            scale: None,
        }
        .into_config()
        .unwrap();
        assert!(screen.automatic);
        assert_eq!(screen.id, ScreenId(7));
        assert!(screen.manual_bounds.is_none());
    }

    #[test]
    fn existing_screen_config_defaults_to_automatic_detection() {
        let screen: RawScreen = toml::from_str(
            r#"
id = 1
name = "Existing Windows display"
width = 1920
height = 1080
scale = 2.0
"#,
        )
        .unwrap();
        assert!(screen.auto);
        assert!(screen.into_config().unwrap().automatic);
    }

    #[test]
    fn manual_screen_still_requires_width_and_height() {
        let screen = RawScreen {
            id: 7,
            name: "Manual display".to_owned(),
            auto: false,
            origin_x: 0.0,
            origin_y: 0.0,
            width: None,
            height: Some(1080.0),
            scale: Some(2.0),
        };
        assert!(screen.into_config().is_err());
    }

    #[test]
    fn pairing_config_loads_before_the_peer_certificate_exists() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "edgemouse-pairing-config-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(directory.join("identity")).unwrap();
        let identity = Identity::generate().unwrap();
        fs::write(
            directory.join("identity/certificate.der"),
            &identity.certificate,
        )
        .unwrap();
        fs::write(
            directory.join("identity/private-key.der"),
            &identity.private_key,
        )
        .unwrap();
        let config_path = directory.join("edgemouse.toml");
        fs::write(
            &config_path,
            r#"
[local]
name = "test-machine"
listen = "0.0.0.0:43891"
certificate = "identity/certificate.der"
private_key = "identity/private-key.der"
[local.screen]
id = 1
name = "Local"
width = 1920
height = 1080
[peer]
address = "auto"
certificate = "not-created-yet.der"
[peer.screen]
id = 2
name = "Peer"
width = 1512
height = 982
[layout]
peer_on = "right"
"#,
        )
        .unwrap();

        let pairing = PairingConfig::load(&config_path).unwrap();
        assert_eq!(pairing.local_node, identity.node_id);
        assert_eq!(
            pairing.peer_certificate_path,
            directory.join("not-created-yet.der")
        );
        assert!(LoadedConfig::load(&config_path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persist_peer_on_updates_and_revalidates_a_complete_config() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "edgemouse-layout-config-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(directory.join("identity")).unwrap();
        let local = Identity::generate().unwrap();
        let peer = Identity::generate().unwrap();
        fs::write(
            directory.join("identity/certificate.der"),
            &local.certificate,
        )
        .unwrap();
        fs::write(
            directory.join("identity/private-key.der"),
            &local.private_key,
        )
        .unwrap();
        fs::write(directory.join("peer.der"), &peer.certificate).unwrap();
        let config_path = directory.join("edgemouse.toml");
        fs::write(
            &config_path,
            r#"
[local]
name = "test-machine"
listen = "0.0.0.0:43891"
certificate = "identity/certificate.der"
private_key = "identity/private-key.der"
[local.screen]
id = 1
name = "Local"
auto = true
[peer]
address = "auto"
certificate = "peer.der"
[peer.screen]
id = 2
name = "Peer"
auto = true
[layout]
peer_on = "right"
"#,
        )
        .unwrap();

        persist_peer_on(&config_path, Edge::Bottom).unwrap();
        assert_eq!(
            LoadedConfig::load(&config_path).unwrap().peer_on,
            Edge::Bottom
        );
        assert!(
            fs::read_to_string(&config_path)
                .unwrap()
                .contains("peer_on = \"bottom\"")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
