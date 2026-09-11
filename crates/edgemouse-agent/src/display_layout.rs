//! Persistent, atomic layout register scoped to the current pairing. No local
//! paths or arbitrary configuration text are accepted from the peer.
use crate::config::LoadedConfig;
use edgemouse_core::{Edge, NodeId, Point, Rect};
use edgemouse_protocol::WireMessage;
use edgemouse_protocol::display_layout::{DisplayLayout, DisplayLayoutState, DisplayPositions};
use edgemouse_transport::PeerLink;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutChoice {
    pub mac_on: String,
    pub windows: Vec<[f64; 4]>,
    pub mac: Vec<[f64; 4]>,
    #[serde(default)]
    pub positions: Option<LayoutPositions>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPositions {
    pub windows: Vec<[f64; 4]>,
    pub mac: Vec<[f64; 4]>,
}

impl LayoutChoice {
    pub fn decode(&self) -> Result<DisplayLayout, String> {
        let mac_on = match self.mac_on.as_str() {
            "left" => Edge::Left,
            "right" => Edge::Right,
            "top" => Edge::Top,
            "bottom" => Edge::Bottom,
            _ => return Err("invalid monitor layout direction".into()),
        };
        let rectangles = |values: &[[f64; 4]],
                          frames: Option<&Vec<[f64; 4]>>|
         -> Result<(Vec<Rect>, Vec<Rect>), String> {
            if values.len() > edgemouse_protocol::MAX_DISPLAY_COUNT
                || frames.is_some_and(|f| f.len() != values.len())
            {
                return Err("invalid display placement count".into());
            }
            let convert = |v: &[f64; 4]| {
                Rect::new(Point::new(v[0], v[1]), v[2], v[3]).map_err(|e| e.to_string())
            };
            let mut pairs = values
                .iter()
                .enumerate()
                .map(|(i, v)| Ok((convert(v)?, frames.map(|f| convert(&f[i])).transpose()?)))
                .collect::<Result<Vec<_>, String>>()?;
            pairs.sort_by(|(a, _), (b, _)| {
                a.top()
                    .total_cmp(&b.top())
                    .then(a.left().total_cmp(&b.left()))
                    .then(a.width.total_cmp(&b.width))
                    .then(a.height.total_cmp(&b.height))
            });
            Ok((
                pairs.iter().map(|(a, _)| *a).collect(),
                pairs.iter().filter_map(|(_, b)| *b).collect(),
            ))
        };
        let (windows, win_positions) =
            rectangles(&self.windows, self.positions.as_ref().map(|p| &p.windows))?;
        let (mac, mac_positions) = rectangles(&self.mac, self.positions.as_ref().map(|p| &p.mac))?;
        let layout = DisplayLayout {
            mac_on,
            windows,
            mac,
            positions: self.positions.as_ref().map(|_| DisplayPositions {
                windows: win_positions,
                mac: mac_positions,
            }),
        };
        if !layout.valid() {
            return Err("invalid selected display geometry".into());
        }
        Ok(layout)
    }
}

impl From<&DisplayLayout> for LayoutChoice {
    fn from(layout: &DisplayLayout) -> Self {
        let rectangles = |values: &[Rect]| {
            values
                .iter()
                .map(|r| [r.left(), r.top(), r.width, r.height])
                .collect()
        };
        Self {
            mac_on: crate::config::edge_name(layout.mac_on).into(),
            windows: rectangles(&layout.windows),
            mac: rectangles(&layout.mac),
            positions: layout.positions.as_ref().map(|p| LayoutPositions {
                windows: rectangles(&p.windows),
                mac: rectangles(&p.mac),
            }),
        }
    }
}

pub struct Snapshot {
    pub state: DisplayLayoutState,
    pub pending: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    local: String,
    peer: String,
    revision: u64,
    author: String,
    layout: Option<LayoutChoice>,
    acknowledged_revision: Option<u64>,
    acknowledged_author: Option<String>,
}

fn node_name(node: NodeId) -> String {
    format!("{:032x}", node.0)
}

struct Store<'a> {
    path: &'a Path,
    local: NodeId,
    peer: NodeId,
}

impl<'a> Store<'a> {
    fn for_config(path: &'a Path) -> Result<Self, String> {
        let config = LoadedConfig::load(path).map_err(|e| e.to_string())?;
        Ok(Self {
            path,
            local: config.local_node,
            peer: config.peer_node,
        })
    }

    fn lock(&self) -> Result<File, String> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("display-layout-lock"))
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) if started.elapsed() < Duration::from_secs(3) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(format!("display layout is busy: {error}")),
            }
        }
    }

    fn default_document(&self) -> Document {
        Document {
            local: node_name(self.local),
            peer: node_name(self.peer),
            revision: 0,
            author: node_name(self.local),
            layout: None,
            acknowledged_revision: Some(0),
            acknowledged_author: Some(node_name(self.local)),
        }
    }

    fn load(&self) -> Result<Document, String> {
        let path = self.path.with_extension("display-layout");
        let document: Document = match fs::metadata(&path) {
            Ok(metadata) if metadata.len() <= 32 * 1024 => {
                toml::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("invalid display layout: {e}"))?
            }
            Ok(_) => return Err("display layout file is too large".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.default_document(),
            Err(e) => return Err(e.to_string()),
        };
        if document.local != node_name(self.local) || document.peer != node_name(self.peer) {
            return Ok(self.default_document());
        }
        self.state(&document)?;
        Ok(document)
    }

    fn state(&self, document: &Document) -> Result<DisplayLayoutState, String> {
        let author = NodeId(
            u128::from_str_radix(&document.author, 16).map_err(|_| "invalid layout author")?,
        );
        let state = DisplayLayoutState {
            revision: document.revision,
            author,
            layout: document
                .layout
                .as_ref()
                .map(LayoutChoice::decode)
                .transpose()?,
        };
        self.validate(&state)?;
        Ok(state)
    }

    fn validate(&self, state: &DisplayLayoutState) -> Result<(), String> {
        if (state.author != self.local && state.author != self.peer)
            || state.layout.as_ref().is_some_and(|layout| {
                !layout.valid() || LayoutChoice::from(layout).decode().as_ref() != Ok(layout)
            })
        {
            return Err("invalid paired display layout".into());
        }
        Ok(())
    }

    fn save(&self, document: &Document) -> Result<(), String> {
        let source = toml::to_string(document).map_err(|e| e.to_string())?;
        let temporary = self.path.with_extension("display-layout.tmp");
        fs::write(&temporary, source).map_err(|e| e.to_string())?;
        fs::rename(temporary, self.path.with_extension("display-layout")).map_err(|e| e.to_string())
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let _lock = self.lock()?;
        let document = self.load()?;
        let pending = document.acknowledged_revision != Some(document.revision)
            || document.acknowledged_author.as_ref() != Some(&document.author);
        Ok(Snapshot {
            state: self.state(&document)?,
            pending,
        })
    }

    fn edit(&self, layout: Option<LayoutChoice>) -> Result<(), String> {
        let layout = layout.as_ref().map(LayoutChoice::decode).transpose()?;
        let _lock = self.lock()?;
        let mut document = self.load()?;
        if self.state(&document)?.layout == layout {
            return Ok(());
        }
        document.revision = document
            .revision
            .checked_add(1)
            .ok_or("display layout revision exhausted")?;
        document.author = node_name(self.local);
        document.layout = layout.as_ref().map(LayoutChoice::from);
        self.save(&document)
    }

    fn receive(&self, incoming: &DisplayLayoutState) -> Result<DisplayLayoutState, String> {
        self.validate(incoming)?;
        let _lock = self.lock()?;
        let mut document = self.load()?;
        let current = self.state(&document)?;
        let winner = winning(&current, incoming)?;
        if winner == *incoming {
            document.revision = incoming.revision;
            document.author = node_name(incoming.author);
            document.layout = incoming.layout.as_ref().map(LayoutChoice::from);
            document.acknowledged_revision = Some(incoming.revision);
            document.acknowledged_author = Some(node_name(incoming.author));
            self.save(&document)?;
        }
        Ok(winner)
    }

    fn acknowledge(&self, expected: &DisplayLayoutState) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut document = self.load()?;
        if self.state(&document)? == *expected {
            document.acknowledged_revision = Some(expected.revision);
            document.acknowledged_author = Some(node_name(expected.author));
            self.save(&document)?;
        }
        Ok(())
    }
}

fn winning(a: &DisplayLayoutState, b: &DisplayLayoutState) -> Result<DisplayLayoutState, String> {
    if a.stamp() == b.stamp() && a != b {
        return Err("conflicting display layout revision".into());
    }
    Ok(if a.stamp() >= b.stamp() {
        a.clone()
    } else {
        b.clone()
    })
}

pub fn snapshot(path: &Path) -> Result<Snapshot, String> {
    Store::for_config(path)?.snapshot()
}
pub fn edit(path: &Path, layout: Option<LayoutChoice>) -> Result<(), String> {
    Store::for_config(path)?.edit(layout)
}
pub fn receive(path: &Path, incoming: &DisplayLayoutState) -> Result<DisplayLayoutState, String> {
    Store::for_config(path)?.receive(incoming)
}
pub fn acknowledge(path: &Path, expected: &DisplayLayoutState) -> Result<(), String> {
    Store::for_config(path)?.acknowledge(expected)
}

/// Both peers build the same topology before enabling input. Concurrent newer
/// UI edits remain pending and are synchronized by the normal runtime loop.
pub async fn exchange(link: &mut PeerLink, path: &Path) -> Result<DisplayLayoutState, String> {
    exchange_store(link, &Store::for_config(path)?).await
}

async fn exchange_store(
    link: &mut PeerLink,
    store: &Store<'_>,
) -> Result<DisplayLayoutState, String> {
    let local = store.snapshot()?.state;
    link.send(&WireMessage::DisplayLayoutUpdate {
        request_id: 0,
        state: local.clone(),
    })
    .await
    .map_err(|e| e.to_string())?;
    let WireMessage::DisplayLayoutUpdate {
        request_id: 0,
        state: remote,
    } = link.receive().await.map_err(|e| e.to_string())?
    else {
        return Err("expected initial display layout".into());
    };
    store.receive(&remote)?;
    let agreed = winning(&local, &remote)?;
    store.acknowledge(&agreed)?;
    Ok(agreed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_offline_edits_converge_survive_restart_and_ignore_stale_acknowledgements() {
        let root = std::env::temp_dir().join(format!(
            "edgemouse-display-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let ap = root.join("a.toml");
        let bp = root.join("b.toml");
        let a = Store {
            path: &ap,
            local: NodeId(1),
            peer: NodeId(2),
        };
        let b = Store {
            path: &bp,
            local: NodeId(2),
            peer: NodeId(1),
        };
        let first = LayoutChoice {
            mac_on: "left".into(),
            windows: vec![[0.0, 0.0, 100.0, 200.0]],
            mac: vec![[0.0, -100.0, 100.0, 100.0]],
            positions: Some(LayoutPositions {
                windows: vec![[0.0, 0.0, 500.0, 1000.0]],
                mac: vec![[-1000.0, 0.0, 1000.0, 1000.0]],
            }),
        };
        let mut second = first.clone();
        second.mac = vec![[0.0, 0.0, 80.0, 100.0]];
        a.edit(Some(first.clone())).unwrap();
        b.edit(Some(second.clone())).unwrap();
        let av = a.snapshot().unwrap().state;
        let bv = b.snapshot().unwrap().state;
        a.receive(&bv).unwrap();
        b.receive(&av).unwrap();
        assert_eq!(a.snapshot().unwrap().state, b.snapshot().unwrap().state);
        assert_eq!(
            a.snapshot().unwrap().state.layout,
            Some(second.decode().unwrap())
        );
        b.acknowledge(&bv).unwrap();
        assert!(!b.snapshot().unwrap().pending);
        a.edit(None).unwrap();
        a.acknowledge(&av).unwrap();
        assert!(a.snapshot().unwrap().pending);
        let reset = a.snapshot().unwrap().state;
        b.receive(&reset).unwrap();
        assert!(b.snapshot().unwrap().state.layout.is_none());
        assert_eq!(a.snapshot().unwrap().state, reset);
        let repaired = Store {
            path: &ap,
            local: NodeId(1),
            peer: NodeId(3),
        };
        assert!(repaired.snapshot().unwrap().state.layout.is_none());
        let mut forged = reset;
        forged.author = NodeId(9);
        assert!(a.receive(&forged).is_err());
        let mut duplicate = first;
        duplicate.mac.push(duplicate.mac[0]);
        assert!(a.edit(Some(duplicate)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn authenticated_exchange_converges_and_preserves_edits_during_handshake() {
        use edgemouse_core::ScreenId;
        use edgemouse_protocol::ScreenInfo;
        use edgemouse_transport::{Identity, PeerConfig, TrustedPeer};
        use std::net::UdpSocket;
        let first = Identity::generate().unwrap();
        let second = Identity::generate().unwrap();
        let address = || {
            UdpSocket::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
        };
        let first_address = address();
        let second_address = address();
        let root =
            std::env::temp_dir().join(format!("edgemouse-display-exchange-{}", first.node_id.0));
        fs::create_dir_all(&root).unwrap();
        let a_path = root.join("a.toml");
        let b_path = root.join("b.toml");
        let a = Store {
            path: &a_path,
            local: first.node_id,
            peer: second.node_id,
        };
        let b = Store {
            path: &b_path,
            local: second.node_id,
            peer: first.node_id,
        };
        let config_a = PeerConfig {
            bind_address: first_address,
            peer_address: second_address,
            local_name: "Mac test".into(),
            identity: Identity::from_der(first.certificate.clone(), first.private_key).unwrap(),
            peer: TrustedPeer::from_der(second.certificate.clone()).unwrap(),
            connect_timeout: Duration::from_secs(5),
        };
        let config_b = PeerConfig {
            bind_address: second_address,
            peer_address: first_address,
            local_name: "Windows test".into(),
            identity: Identity::from_der(second.certificate, second.private_key).unwrap(),
            peer: TrustedPeer::from_der(first.certificate).unwrap(),
            connect_timeout: Duration::from_secs(5),
        };
        let screen = |id| ScreenInfo {
            id: ScreenId(id),
            name: "test".into(),
            bounds: Rect::new(Point::new(0.0, 0.0), 1920.0, 1080.0).unwrap(),
            scale_factor: 1.0,
            displays: vec![],
        };
        let (link_a, link_b) = tokio::join!(
            PeerLink::connect(config_a, screen(1)),
            PeerLink::connect(config_b, screen(2))
        );
        let mut link_a = link_a.unwrap();
        let mut link_b = link_b.unwrap();
        let upper = LayoutChoice {
            mac_on: "left".into(),
            windows: vec![[0.0, 0.0, 2160.0, 3840.0]],
            mac: vec![[0.0, -1080.0, 1920.0, 1080.0], [240.0, 0.0, 1470.0, 956.0]],
            positions: Some(LayoutPositions {
                windows: vec![[0.0, 0.0, 562.5, 1000.0]],
                mac: vec![[-943.0, 0.0, 943.0, 530.0], [-722.0, 530.0, 722.0, 470.0]],
            }),
        };
        let mut lower = upper.clone();
        lower.positions.as_mut().unwrap().windows[0][1] = 600.0;
        a.edit(Some(upper.clone())).unwrap();
        let local_initial = a.snapshot().unwrap().state;
        let remote_initial = b.snapshot().unwrap().state;
        let expected = winning(&local_initial, &remote_initial).unwrap();
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(exchange_store(&mut link_a, &a), async {
                let WireMessage::DisplayLayoutUpdate {
                    request_id: 0,
                    state,
                } = link_b.receive().await.unwrap()
                else {
                    panic!("wrong handshake message")
                };
                assert_eq!(state, local_initial);
                a.edit(Some(lower.clone())).unwrap(); // UI edit while the initial exchange is in flight.
                b.receive(&state).unwrap();
                link_b
                    .send(&WireMessage::DisplayLayoutUpdate {
                        request_id: 0,
                        state: remote_initial,
                    })
                    .await
                    .unwrap();
            })
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap(), expected);
        assert_eq!(
            a.snapshot().unwrap().state.layout,
            Some(lower.decode().unwrap())
        );
        assert!(
            a.snapshot().unwrap().pending,
            "handshake cannot acknowledge an unseen newer UI edit"
        );
        let (result_a, result_b) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                exchange_store(&mut link_a, &a),
                exchange_store(&mut link_b, &b)
            )
        })
        .await
        .unwrap();
        assert_eq!(result_a.unwrap(), result_b.unwrap());
        assert_eq!(a.snapshot().unwrap().state, b.snapshot().unwrap().state);
        assert!(!a.snapshot().unwrap().pending && !b.snapshot().unwrap().pending);
        fs::remove_dir_all(root).unwrap();
    }
}
