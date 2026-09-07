//! Shared preferences only: identities, paths, permissions and desktop UI
//! preferences never enter this document. Each field has its own logical clock
//! so offline edits to different directions do not overwrite one another.
use crate::config::{
    LoadedConfig, SessionPreferences, persist_peer_on, persist_session_preferences,
};
use edgemouse_core::{Edge, NodeId};
use edgemouse_protocol::{SETTINGS_COUNT, SettingEntry, valid_setting};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

pub const LAYOUT: u8 = 0;
pub const HYSTERESIS: u8 = 1;
pub const AUTO_RECONNECT: u8 = 2;
pub const MAC_PROFILE: u8 = 3;
pub const WINDOWS_PROFILE: u8 = 9;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredEntry {
    key: u8,
    revision: u64,
    author: String,
    value: f64,
}

#[derive(Default, Serialize, Deserialize)]
struct Document {
    local: String,
    peer: String,
    entries: Vec<StoredEntry>,
    #[serde(default)]
    acknowledged: Vec<StoredEntry>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub entries: Vec<SettingEntry>,
    pub pending: bool,
}

fn node_name(node: NodeId) -> String {
    format!("{:032x}", node.0)
}
fn stored(entry: &SettingEntry) -> StoredEntry {
    StoredEntry {
        key: entry.key,
        revision: entry.revision,
        author: node_name(entry.author),
        value: entry.value,
    }
}
fn decode(
    entries: &[StoredEntry],
    local: NodeId,
    peer: NodeId,
) -> Result<Vec<SettingEntry>, String> {
    if entries.len() > SETTINGS_COUNT {
        return Err("too many shared settings".into());
    }
    let mut seen = [false; SETTINGS_COUNT];
    let mut result = Vec::new();
    for item in entries {
        let author =
            NodeId(u128::from_str_radix(&item.author, 16).map_err(|_| "invalid settings author")?);
        if !valid_setting(item.key, item.value)
            || (author != local && author != peer)
            || seen[usize::from(item.key)]
        {
            return Err("invalid shared setting".into());
        }
        seen[usize::from(item.key)] = true;
        result.push(SettingEntry {
            key: item.key,
            revision: item.revision,
            author,
            value: item.value,
        });
    }
    result.sort_by_key(|entry| entry.key);
    Ok(result)
}

fn lock(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("settings-lock"))
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if started.elapsed() < Duration::from_secs(3) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(format!("shared settings are busy: {error}")),
        }
    }
}

fn load(path: &Path, config: &LoadedConfig, windows: bool) -> Result<Document, String> {
    let mut document: Document = match fs::read_to_string(path.with_extension("settings-sync")) {
        Ok(source) => {
            toml::from_str(&source).map_err(|e| format!("invalid shared settings file: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Document::default(),
        Err(e) => return Err(e.to_string()),
    };
    let local = node_name(config.local_node);
    let peer = node_name(config.peer_node);
    // Re-pairing must not copy the previous computer's shared settings/identity.
    if document.local != local || document.peer != peer {
        document = Document {
            local,
            peer,
            ..Document::default()
        };
    }
    decode(&document.entries, config.local_node, config.peer_node)?;
    let outgoing = if windows {
        WINDOWS_PROFILE
    } else {
        MAC_PROFILE
    };
    let incoming = if windows {
        MAC_PROFILE
    } else {
        WINDOWS_PROFILE
    };
    let edge = if windows {
        config.peer_on
    } else {
        config.peer_on.opposite()
    };
    let seeds = [
        (LAYOUT, edge_value(edge)),
        (HYSTERESIS, config.session.entry_hysteresis),
        (AUTO_RECONNECT, f64::from(config.auto_reconnect)),
        (outgoing, f64::from(config.reverse_scroll_horizontal)),
        (outgoing + 1, f64::from(config.reverse_scroll_vertical)),
        (outgoing + 3, f64::from(config.keyboard_enabled)),
        (
            outgoing + 5,
            f64::from(config.session.block_switch_while_dragging),
        ),
        (incoming + 2, f64::from(config.pointer_smoothing)),
        (incoming + 4, f64::from(config.reclaim_enabled)),
    ];
    for (key, value) in seeds {
        if !document.entries.iter().any(|entry| entry.key == key) {
            document.entries.push(StoredEntry {
                key,
                value,
                revision: 0,
                author: node_name(config.local_node),
            });
        }
    }
    document.entries.sort_by_key(|entry| entry.key);
    Ok(document)
}

fn save(path: &Path, document: &Document) -> Result<(), String> {
    let source = toml::to_string(document).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("settings-sync.tmp");
    fs::write(&temporary, source).map_err(|e| e.to_string())?;
    // The advisory lock serializes writers; rename prevents partial snapshots.
    fs::rename(temporary, path.with_extension("settings-sync")).map_err(|e| e.to_string())
}

pub fn snapshot(path: &Path, windows: bool) -> Result<Snapshot, String> {
    let _lock = lock(path)?;
    let config = LoadedConfig::load(path).map_err(|e| e.to_string())?;
    let document = load(path, &config, windows)?;
    let entries = decode(&document.entries, config.local_node, config.peer_node)?;
    let acknowledged = decode(&document.acknowledged, config.local_node, config.peer_node)?;
    let pending = entries != acknowledged;
    Ok(Snapshot { entries, pending })
}

/// Save only fields the user actually edited, never stale UI defaults belonging
/// to the other computer. A logical clock avoids depending on synchronized time.
pub fn edit(path: &Path, windows: bool, changes: &[(u8, f64)]) -> Result<(), String> {
    if changes
        .iter()
        .any(|&(key, value)| !valid_setting(key, value))
    {
        return Err("invalid setting value".into());
    }
    let _lock = lock(path)?;
    let config = LoadedConfig::load(path).map_err(|e| e.to_string())?;
    let mut document = load(path, &config, windows)?;
    let revision = document
        .entries
        .iter()
        .map(|entry| entry.revision)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("shared settings revision overflow")?;
    for &(key, value) in changes {
        document.entries.retain(|entry| entry.key != key);
        document.entries.push(StoredEntry {
            key,
            value,
            revision,
            author: node_name(config.local_node),
        });
    }
    document.entries.sort_by_key(|entry| entry.key);
    // Persist the intent first, so a crash/restart can reapply it before connecting.
    save(path, &document)?;
    apply(
        path,
        &config,
        windows,
        &decode(&document.entries, config.local_node, config.peer_node)?,
    )?;
    Ok(())
}

fn merge_entries(local: &mut Vec<SettingEntry>, remote: &[SettingEntry]) {
    for entry in remote {
        if let Some(current) = local.iter_mut().find(|current| current.key == entry.key) {
            // Equal logical time resolves deterministically; baseline common
            // fields use the same rule. Different keys always merge independently.
            if entry.revision > current.revision
                || (entry.revision == current.revision && entry.author < current.author)
            {
                *current = entry.clone();
            }
        } else {
            local.push(entry.clone());
        }
    }
    local.sort_by_key(|entry| entry.key);
}

pub fn receive(path: &Path, windows: bool, remote: &[SettingEntry]) -> Result<bool, String> {
    let _lock = lock(path)?;
    let config = LoadedConfig::load(path).map_err(|e| e.to_string())?;
    let mut document = load(path, &config, windows)?;
    let remote = decode(
        &remote.iter().map(stored).collect::<Vec<_>>(),
        config.local_node,
        config.peer_node,
    )?;
    let mut entries = decode(&document.entries, config.local_node, config.peer_node)?;
    merge_entries(&mut entries, &remote);
    document.entries = entries.iter().map(stored).collect();
    save(path, &document)?;
    apply(path, &config, windows, &entries)
}

pub fn acknowledge(path: &Path, windows: bool, sent: &[SettingEntry]) -> Result<(), String> {
    let _lock = lock(path)?;
    let config = LoadedConfig::load(path).map_err(|e| e.to_string())?;
    let mut document = load(path, &config, windows)?;
    document.acknowledged = sent.iter().map(stored).collect();
    save(path, &document)
}

pub fn edge_value(edge: Edge) -> f64 {
    match edge {
        Edge::Left => 0.0,
        Edge::Right => 1.0,
        Edge::Top => 2.0,
        Edge::Bottom => 3.0,
    }
}
fn value_edge(value: f64) -> Edge {
    match value as u8 {
        0 => Edge::Left,
        1 => Edge::Right,
        2 => Edge::Top,
        _ => Edge::Bottom,
    }
}

pub fn preferences(config: &LoadedConfig) -> SessionPreferences {
    SessionPreferences {
        entry_hysteresis: config.session.entry_hysteresis,
        reverse_scroll_horizontal: config.reverse_scroll_horizontal,
        reverse_scroll_vertical: config.reverse_scroll_vertical,
        pointer_smoothing: config.pointer_smoothing,
        keyboard_enabled: config.keyboard_enabled,
        reclaim_enabled: config.reclaim_enabled,
        block_switch_while_dragging: config.session.block_switch_while_dragging,
        auto_reconnect: config.auto_reconnect,
    }
}

fn desired(
    config: &LoadedConfig,
    windows: bool,
    entries: &[SettingEntry],
) -> (Edge, SessionPreferences) {
    let mut settings = preferences(config);
    let mut edge = config.peer_on;
    let outgoing = if windows {
        WINDOWS_PROFILE
    } else {
        MAC_PROFILE
    };
    let incoming = if windows {
        MAC_PROFILE
    } else {
        WINDOWS_PROFILE
    };
    for entry in entries {
        let (key, value) = (entry.key, entry.value);
        match key {
            LAYOUT => {
                let shared = value_edge(value);
                edge = if windows { shared } else { shared.opposite() };
            }
            HYSTERESIS => settings.entry_hysteresis = value,
            AUTO_RECONNECT => settings.auto_reconnect = value != 0.0,
            _ if key == outgoing => settings.reverse_scroll_horizontal = value != 0.0,
            _ if key == outgoing + 1 => settings.reverse_scroll_vertical = value != 0.0,
            _ if key == outgoing + 3 => settings.keyboard_enabled = value != 0.0,
            _ if key == outgoing + 5 => settings.block_switch_while_dragging = value != 0.0,
            _ if key == incoming + 2 => settings.pointer_smoothing = value as u8,
            _ if key == incoming + 4 => settings.reclaim_enabled = value != 0.0,
            _ => {}
        }
    }
    (edge, settings)
}

fn apply(
    path: &Path,
    config: &LoadedConfig,
    windows: bool,
    entries: &[SettingEntry],
) -> Result<bool, String> {
    let (edge, settings) = desired(config, windows, entries);
    let changed = edge != config.peer_on || settings != preferences(config);
    if edge != config.peer_on {
        persist_peer_on(path, edge)?;
    }
    if settings != preferences(config) {
        persist_session_preferences(path, settings)?;
    }
    Ok(changed)
}

/// Reapply a committed journal after an interrupted save, before input starts.
pub fn reconcile(path: &Path, windows: bool) -> Result<(), String> {
    receive(path, windows, &[]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgemouse_transport::Identity;

    struct Pair {
        directory: std::path::PathBuf,
        mac: std::path::PathBuf,
        windows: std::path::PathBuf,
    }
    impl Pair {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir()
                .join(format!("edgemouse-settings-{}-{stamp}", std::process::id()));
            let mac = directory.join("mac.toml");
            let windows = directory.join("windows.toml");
            fs::create_dir_all(&directory).unwrap();
            let identities = [Identity::generate().unwrap(), Identity::generate().unwrap()];
            for (index, (path, edge)) in
                [(&mac, "top"), (&windows, "right")].into_iter().enumerate()
            {
                let own = &identities[index];
                let peer = &identities[1 - index];
                fs::write(
                    directory.join(format!("cert-{index}.der")),
                    &own.certificate,
                )
                .unwrap();
                fs::write(directory.join(format!("key-{index}.der")), &own.private_key).unwrap();
                fs::write(
                    directory.join(format!("peer-{index}.der")),
                    &peer.certificate,
                )
                .unwrap();
                fs::write(path, format!("[local]\nname = 'test'\nlisten = '127.0.0.1:43891'\ncertificate = 'cert-{index}.der'\nprivate_key = 'key-{index}.der'\n[local.screen]\nid = {}\nname = 'Local'\nauto = true\n[peer]\naddress = 'auto'\ncertificate = 'peer-{index}.der'\n[peer.screen]\nid = {}\nname = 'Peer'\nauto = true\n[layout]\npeer_on = '{edge}'\n", index + 1, 2 - index)).unwrap();
            }
            Self {
                directory,
                mac,
                windows,
            }
        }
        fn exchange(&self) {
            for _ in 0..3 {
                let mac = snapshot(&self.mac, false).unwrap().entries;
                let windows = snapshot(&self.windows, true).unwrap().entries;
                receive(&self.windows, true, &mac).unwrap();
                receive(&self.mac, false, &windows).unwrap();
                acknowledge(&self.mac, false, &mac).unwrap();
                acknowledge(&self.windows, true, &windows).unwrap();
            }
        }
    }
    impl Drop for Pair {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn saved_profiles_layout_and_connection_preferences_apply_on_the_correct_computer() {
        let pair = Pair::new();
        // Editing the Windows -> Mac profile from the Mac changes outgoing
        // scroll/keyboard on Windows and incoming smoothing/reclaim on Mac.
        edit(
            &pair.mac,
            false,
            &[
                (WINDOWS_PROFILE, 1.0),
                (WINDOWS_PROFILE + 1, 1.0),
                (WINDOWS_PROFILE + 2, 87.0),
                (WINDOWS_PROFILE + 3, 0.0),
                (WINDOWS_PROFILE + 4, 0.0),
                (WINDOWS_PROFILE + 5, 0.0),
                (LAYOUT, 3.0),
                (HYSTERESIS, 0.0),
                (AUTO_RECONNECT, 0.0),
            ],
        )
        .unwrap();
        // Independent offline changes in the other control direction survive.
        edit(
            &pair.windows,
            true,
            &[(MAC_PROFILE + 1, 1.0), (MAC_PROFILE + 2, 23.0)],
        )
        .unwrap();
        reconcile(&pair.mac, false).unwrap(); // restart before the peer was reachable
        pair.exchange();
        let mac = LoadedConfig::load(&pair.mac).unwrap();
        let win = LoadedConfig::load(&pair.windows).unwrap();
        assert_eq!(mac.peer_on, Edge::Top);
        assert_eq!(win.peer_on, Edge::Bottom);
        assert!(
            win.reverse_scroll_horizontal
                && win.reverse_scroll_vertical
                && mac.reverse_scroll_vertical
        );
        assert!(!mac.reverse_scroll_horizontal);
        assert!(!win.keyboard_enabled && mac.keyboard_enabled);
        assert!(!win.session.block_switch_while_dragging);
        assert_eq!(mac.pointer_smoothing, 87);
        assert_eq!(win.pointer_smoothing, 23);
        assert!(!mac.reclaim_enabled && win.reclaim_enabled);
        assert_eq!(mac.session.entry_hysteresis, 0.0);
        assert_eq!(win.session.entry_hysteresis, 0.0);
        assert!(!mac.auto_reconnect && !win.auto_reconnect);
        assert_eq!(
            snapshot(&pair.mac, false).unwrap().entries,
            snapshot(&pair.windows, true).unwrap().entries
        );
        assert!(!snapshot(&pair.mac, false).unwrap().pending);
        assert!(!snapshot(&pair.windows, true).unwrap().pending);
        let saved = fs::read_to_string(pair.mac.with_extension("settings-sync")).unwrap();
        assert!(!saved.contains("private_key") && !saved.contains("certificate"));
    }

    #[test]
    fn delayed_acknowledgement_cannot_mark_a_newer_edit_synced() {
        let pair = Pair::new();
        pair.exchange();
        let sent = snapshot(&pair.mac, false).unwrap().entries;
        edit(&pair.mac, false, &[(MAC_PROFILE, 1.0)]).unwrap();
        acknowledge(&pair.mac, false, &sent).unwrap();
        assert!(snapshot(&pair.mac, false).unwrap().pending);
        pair.exchange();
        assert!(!snapshot(&pair.mac, false).unwrap().pending);
    }

    #[test]
    fn serialized_writers_preserve_concurrent_local_changes() {
        let pair = Pair::new();
        std::thread::scope(|scope| {
            scope.spawn(|| edit(&pair.mac, false, &[(MAC_PROFILE, 1.0)]).unwrap());
            scope.spawn(|| edit(&pair.mac, false, &[(WINDOWS_PROFILE, 1.0)]).unwrap());
        });
        let entries = snapshot(&pair.mac, false).unwrap().entries;
        assert!(
            entries
                .iter()
                .filter(|entry| [MAC_PROFILE, WINDOWS_PROFILE].contains(&entry.key))
                .all(|entry| entry.value == 1.0)
        );
        pair.exchange();
        assert_eq!(
            snapshot(&pair.mac, false).unwrap().entries,
            snapshot(&pair.windows, true).unwrap().entries
        );
    }
    fn entry(key: u8, revision: u64, author: u128, value: f64) -> SettingEntry {
        SettingEntry {
            key,
            revision,
            author: NodeId(author),
            value,
        }
    }
    #[test]
    fn different_offline_fields_merge_without_losing_either_edit() {
        let a = vec![entry(3, 4, 1, 1.0), entry(9, 0, 2, 0.0)];
        let b = vec![entry(3, 0, 1, 0.0), entry(9, 5, 2, 1.0)];
        let mut first = a.clone();
        let mut second = b.clone();
        merge_entries(&mut first, &b);
        merge_entries(&mut second, &a);
        assert_eq!(first, second);
        assert_eq!(
            first.iter().map(|entry| entry.value).collect::<Vec<_>>(),
            vec![1.0, 1.0]
        );
    }
    #[test]
    fn simultaneous_edits_converge_and_replays_do_not_undo_them() {
        let a = vec![entry(0, 1, 1, 3.0)];
        let b = vec![entry(0, 1, 2, 1.0)];
        let mut first = a.clone();
        let mut second = b.clone();
        merge_entries(&mut first, &b);
        merge_entries(&mut second, &a);
        assert_eq!(first, second);
        merge_entries(&mut first, &[entry(0, 0, 1, 0.0)]);
        assert_eq!(first, a);
    }
    #[test]
    fn remote_settings_are_bounded_and_cannot_claim_unrelated_identities() {
        assert!(decode(&[stored(&entry(3, 1, 3, 1.0))], NodeId(1), NodeId(2)).is_err());
        assert!(decode(&[stored(&entry(15, 1, 1, 1.0))], NodeId(1), NodeId(2)).is_err());
        assert!(decode(&[stored(&entry(5, 1, 1, 101.0))], NodeId(1), NodeId(2)).is_err());
    }
}
