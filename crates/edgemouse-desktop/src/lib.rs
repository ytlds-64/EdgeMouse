use edgemouse_agent::config::{LoadedConfig, PeerAddress, edge_name, persist_peer_on};
use edgemouse_agent::{control, platform};
use edgemouse_core::Edge;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;

struct AppState {
    config_path: Option<PathBuf>,
    agent_status: Arc<Mutex<AgentStatusCache>>,
}

#[derive(Default)]
struct AgentStatusCache {
    last_status: Option<control::RunningStatus>,
    consecutive_misses: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSnapshot {
    running: bool,
    status_fresh: bool,
    process_id: Option<u32>,
    version: String,
    connection: Option<ConnectionSnapshot>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionSnapshot {
    state: String,
    peer_name: Option<String>,
    connected_since_unix_ms: Option<u64>,
    metrics_updated_unix_ms: Option<u64>,
    reconnect_count: u32,
    rtt_ms: Option<f32>,
    jitter_ms: Option<f32>,
    send_interval_ms: Option<u32>,
    sent_moves: u64,
    skipped_moves: u64,
    coalesced_moves: u64,
    received_moves: u64,
    stale_moves: u64,
    superseded_moves: u64,
}

impl From<control::ConnectionTelemetry> for ConnectionSnapshot {
    fn from(telemetry: control::ConnectionTelemetry) -> Self {
        Self {
            state: telemetry.phase.as_str().to_owned(),
            peer_name: telemetry.peer_name,
            connected_since_unix_ms: telemetry.connected_since_unix_ms,
            metrics_updated_unix_ms: telemetry.metrics_updated_unix_ms,
            reconnect_count: telemetry.reconnect_count,
            rtt_ms: telemetry.rtt_ms,
            jitter_ms: telemetry.jitter_ms,
            send_interval_ms: telemetry.send_interval_ms,
            sent_moves: telemetry.sent_moves,
            skipped_moves: telemetry.skipped_moves,
            coalesced_moves: telemetry.coalesced_moves,
            received_moves: telemetry.received_moves,
            stale_moves: telemetry.stale_moves,
            superseded_moves: telemetry.superseded_moves,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformSnapshot {
    operating_system: String,
    permission_granted: Option<bool>,
    desktop_width: Option<f64>,
    desktop_height: Option<f64>,
    scale_factor: Option<f64>,
    display_count: Option<u32>,
    geometry_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigSnapshot {
    path: Option<String>,
    valid: bool,
    error: Option<String>,
    local_name: Option<String>,
    local_node: Option<String>,
    peer_node: Option<String>,
    peer_address: Option<String>,
    listen_address: Option<String>,
    local_screen_name: Option<String>,
    local_screen_id: Option<u64>,
    local_screen_automatic: Option<bool>,
    peer_screen_name: Option<String>,
    peer_screen_id: Option<u64>,
    peer_on: Option<String>,
    entry_hysteresis: Option<f64>,
    peer_timeout_ms: Option<u64>,
    reverse_scroll_horizontal: Option<bool>,
    reverse_scroll_vertical: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    desktop_version: String,
    agent: AgentSnapshot,
    platform: PlatformSnapshot,
    config: ConfigSnapshot,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedScrollSettings {
    applied_live: bool,
    warning: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedLayout {
    applied_live: bool,
    warning: Option<String>,
    local_peer_on: String,
}

#[tauri::command]
async fn get_app_snapshot(state: tauri::State<'_, AppState>) -> Result<AppSnapshot, String> {
    let config_path = state.config_path.clone();
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        build_app_snapshot(config_path.as_deref(), &agent_status)
    })
    .await
    .map_err(|error| format!("failed to read EdgeMouse desktop status: {error}"))
}

#[tauri::command]
async fn save_scroll_settings(
    state: tauri::State<'_, AppState>,
    reverse_horizontal: bool,
    reverse_vertical: bool,
) -> Result<SavedScrollSettings, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法保存滚动方向".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        persist_scroll_settings(&path, reverse_horizontal, reverse_vertical)?;
        let (applied_live, warning) =
            match control::update_scroll_settings(reverse_horizontal, reverse_vertical) {
                Ok(Some(_)) => (true, None),
                Ok(None) => (false, Some("设置已保存；后台服务下次启动时生效".to_owned())),
                Err(error) => (false, Some(format!("设置已保存；实时更新失败：{error}"))),
            };
        Ok(SavedScrollSettings {
            applied_live,
            warning,
        })
    })
    .await
    .map_err(|error| format!("保存滚动方向的后台任务失败：{error}"))?
}

#[tauri::command]
async fn save_layout(
    state: tauri::State<'_, AppState>,
    peer_on: String,
) -> Result<SavedLayout, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法保存屏幕布局".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let windows_to_mac = parse_layout_edge(&peer_on)?;
        let local_peer_on = local_layout_edge(windows_to_mac, cfg!(target_os = "macos"));
        persist_peer_on(&path, local_peer_on)?;
        let (applied_live, warning) = match control::update_layout(local_peer_on) {
            Ok(Some(_)) => (true, None),
            Ok(None) => (
                false,
                Some("布局已保存；后台服务未运行，启动服务后请再点一次保存以同步对端".to_owned()),
            ),
            Err(error) => (false, Some(format!("布局已保存；实时同步失败：{error}"))),
        };
        Ok(SavedLayout {
            applied_live,
            warning,
            local_peer_on: edge_name(local_peer_on).to_owned(),
        })
    })
    .await
    .map_err(|error| format!("保存屏幕布局的后台任务失败：{error}"))?
}

fn parse_layout_edge(value: &str) -> Result<Edge, String> {
    match value {
        "left" => Ok(Edge::Left),
        "right" => Ok(Edge::Right),
        "top" => Ok(Edge::Top),
        "bottom" => Ok(Edge::Bottom),
        _ => Err(format!("不支持的屏幕方向：{value}")),
    }
}

const fn local_layout_edge(windows_to_mac: Edge, local_is_macos: bool) -> Edge {
    if local_is_macos {
        windows_to_mac.opposite()
    } else {
        windows_to_mac
    }
}

fn build_app_snapshot(
    config_path: Option<&Path>,
    agent_status: &Mutex<AgentStatusCache>,
) -> AppSnapshot {
    AppSnapshot {
        desktop_version: env!("CARGO_PKG_VERSION").to_owned(),
        agent: agent_snapshot(agent_status),
        platform: platform_snapshot(),
        config: config_snapshot(config_path),
    }
}

#[tauri::command]
fn window_action(window: tauri::WebviewWindow, action: String) -> Result<(), String> {
    let result = match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => window.is_maximized().and_then(|maximized| {
            if maximized {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }),
        "close" => window.close(),
        "drag" => window.start_dragging(),
        _ => return Err(format!("unsupported window action `{action}`")),
    };
    result.map_err(|error| error.to_string())
}

const AGENT_STATUS_MISS_TOLERANCE: u8 = 3;

fn agent_snapshot(cache: &Mutex<AgentStatusCache>) -> AgentSnapshot {
    match control::query_status() {
        Ok(Some(status)) => {
            let snapshot = running_agent_snapshot(status.clone(), true, None);
            let mut cache = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.last_status = Some(status);
            cache.consecutive_misses = 0;
            snapshot
        }
        Ok(None) => missed_agent_snapshot(cache, None),
        Err(error) => missed_agent_snapshot(cache, Some(error.to_string())),
    }
}

fn running_agent_snapshot(
    status: control::RunningStatus,
    status_fresh: bool,
    error: Option<String>,
) -> AgentSnapshot {
    AgentSnapshot {
        running: true,
        status_fresh,
        process_id: Some(status.process_id),
        version: status.version,
        connection: Some(status.connection.into()),
        error,
    }
}

fn missed_agent_snapshot(
    cache: &Mutex<AgentStatusCache>,
    query_error: Option<String>,
) -> AgentSnapshot {
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.consecutive_misses = cache.consecutive_misses.saturating_add(1);
    if cache.consecutive_misses < AGENT_STATUS_MISS_TOLERANCE
        && let Some(status) = cache.last_status.clone()
    {
        let detail = query_error.unwrap_or_else(|| "本机状态通道暂时没有响应".to_owned());
        return running_agent_snapshot(
            status,
            false,
            Some(format!("{detail}；正在自动确认后台服务状态")),
        );
    }

    AgentSnapshot {
        running: false,
        status_fresh: false,
        process_id: None,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        connection: None,
        error: query_error,
    }
}

#[tauri::command]
fn set_menu_language(app: tauri::AppHandle, language: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    install_macos_menu(&app, &language).map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    let _ = (app, language);
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_macos_menu(app: &tauri::AppHandle, language: &str) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadataBuilder, MenuBuilder, SubmenuBuilder};

    let chinese = language != "en";
    let about = AboutMetadataBuilder::new()
        .name(Some("EdgeMouse"))
        .version(Some(env!("CARGO_PKG_VERSION")))
        .copyright(Some("Copyright © 2026 EdgeMouse contributors"))
        .credits(Some(if chinese {
            "让 Windows 与 macOS 像一张连续的桌面一样自然协作。"
        } else {
            "Make Windows and macOS feel like one continuous desktop."
        }))
        .build();

    let app_menu = SubmenuBuilder::new(app, "EdgeMouse")
        .about_with_text(
            if chinese {
                "关于 EdgeMouse"
            } else {
                "About EdgeMouse"
            },
            Some(about),
        )
        .separator()
        .services_with_text(if chinese { "服务" } else { "Services" })
        .separator()
        .hide_with_text(if chinese {
            "隐藏 EdgeMouse"
        } else {
            "Hide EdgeMouse"
        })
        .hide_others_with_text(if chinese {
            "隐藏其他"
        } else {
            "Hide Others"
        })
        .show_all_with_text(if chinese { "全部显示" } else { "Show All" })
        .separator()
        .quit_with_text(if chinese {
            "退出 EdgeMouse"
        } else {
            "Quit EdgeMouse"
        })
        .build()?;
    let file_menu = SubmenuBuilder::new(app, if chinese { "文件" } else { "File" })
        .close_window_with_text(if chinese {
            "关闭窗口"
        } else {
            "Close Window"
        })
        .build()?;
    let edit_menu = SubmenuBuilder::new(app, if chinese { "编辑" } else { "Edit" })
        .undo_with_text(if chinese { "撤销" } else { "Undo" })
        .redo_with_text(if chinese { "重做" } else { "Redo" })
        .separator()
        .cut_with_text(if chinese { "剪切" } else { "Cut" })
        .copy_with_text(if chinese { "拷贝" } else { "Copy" })
        .paste_with_text(if chinese { "粘贴" } else { "Paste" })
        .select_all_with_text(if chinese { "全选" } else { "Select All" })
        .build()?;
    let view_menu = SubmenuBuilder::new(app, if chinese { "显示" } else { "View" })
        .fullscreen_with_text(if chinese {
            "进入全屏幕"
        } else {
            "Enter Full Screen"
        })
        .build()?;
    let window_menu = SubmenuBuilder::new(app, if chinese { "窗口" } else { "Window" })
        .minimize_with_text(if chinese { "最小化" } else { "Minimize" })
        .maximize_with_text(if chinese { "缩放" } else { "Zoom" })
        .separator()
        .bring_all_to_front_with_text(if chinese {
            "前置全部窗口"
        } else {
            "Bring All to Front"
        })
        .build()?;
    let help_menu = SubmenuBuilder::new(app, if chinese { "帮助" } else { "Help" })
        .text(
            "show_help",
            if chinese {
                "EdgeMouse 帮助"
            } else {
                "EdgeMouse Help"
            },
        )
        .build()?;
    let menu = MenuBuilder::new(app)
        .items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ])
        .build()?;
    app.set_menu(menu)?;
    Ok(())
}

fn platform_snapshot() -> PlatformSnapshot {
    let status = platform::current_status();
    match platform::desktop_geometry() {
        Ok(desktop) => PlatformSnapshot {
            operating_system: status.operating_system.to_owned(),
            permission_granted: status.permission_granted,
            desktop_width: Some(desktop.bounds.width),
            desktop_height: Some(desktop.bounds.height),
            scale_factor: Some(desktop.scale_factor),
            display_count: Some(desktop.display_count),
            geometry_error: None,
        },
        Err(error) => PlatformSnapshot {
            operating_system: status.operating_system.to_owned(),
            permission_granted: status.permission_granted,
            desktop_width: None,
            desktop_height: None,
            scale_factor: None,
            display_count: None,
            geometry_error: Some(error.to_string()),
        },
    }
}

fn config_snapshot(path: Option<&Path>) -> ConfigSnapshot {
    let Some(path) = path else {
        return empty_config(None, "未找到 edgemouse.toml；可用 --config 指定配置文件");
    };
    match LoadedConfig::load(path) {
        Ok(config) => ConfigSnapshot {
            path: Some(path.display().to_string()),
            valid: true,
            error: None,
            local_name: Some(config.transport.local_name.clone()),
            local_node: Some(format_node(config.local_node.0)),
            peer_node: Some(format_node(config.peer_node.0)),
            peer_address: Some(match config.peer_address {
                PeerAddress::Auto => "auto (UDP 43892)".to_owned(),
                PeerAddress::Static(address) => address.to_string(),
            }),
            listen_address: Some(config.transport.bind_address.to_string()),
            local_screen_name: Some(config.local_screen.name.clone()),
            local_screen_id: Some(config.local_screen.id.0),
            local_screen_automatic: Some(config.local_screen.automatic),
            peer_screen_name: Some(config.peer_screen_name.clone()),
            peer_screen_id: Some(config.peer_screen.0),
            peer_on: Some(format!("{:?}", config.peer_on).to_ascii_lowercase()),
            entry_hysteresis: Some(config.session.entry_hysteresis),
            peer_timeout_ms: Some(config.session.peer_timeout_ms),
            reverse_scroll_horizontal: Some(config.reverse_scroll_horizontal),
            reverse_scroll_vertical: Some(config.reverse_scroll_vertical),
        },
        Err(error) => empty_config(Some(path), &error.to_string()),
    }
}

fn empty_config(path: Option<&Path>, error: &str) -> ConfigSnapshot {
    ConfigSnapshot {
        path: path.map(|value| value.display().to_string()),
        valid: false,
        error: Some(error.to_owned()),
        local_name: None,
        local_node: None,
        peer_node: None,
        peer_address: None,
        listen_address: None,
        local_screen_name: None,
        local_screen_id: None,
        local_screen_automatic: None,
        peer_screen_name: None,
        peer_screen_id: None,
        peer_on: None,
        entry_hysteresis: None,
        peer_timeout_ms: None,
        reverse_scroll_horizontal: None,
        reverse_scroll_vertical: None,
    }
}

fn persist_scroll_settings(
    path: &Path,
    reverse_horizontal: bool,
    reverse_vertical: bool,
) -> Result<(), String> {
    let original = fs::read_to_string(path)
        .map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let updated = scroll_settings_source(&original, reverse_horizontal, reverse_vertical);
    fs::write(path, updated).map_err(|error| format!("写入 {} 失败：{error}", path.display()))?;
    if let Err(error) = LoadedConfig::load(path) {
        let restore_error = fs::write(path, original).err();
        return Err(match restore_error {
            Some(restore_error) => {
                format!("保存后的配置无效：{error}；恢复原配置也失败：{restore_error}")
            }
            None => format!("保存后的配置无效，已恢复原配置：{error}"),
        });
    }
    Ok(())
}

fn scroll_settings_source(
    source: &str,
    reverse_horizontal: bool,
    reverse_vertical: bool,
) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let had_trailing_newline = source.ends_with('\n');
    let mut lines = source.lines().map(str::to_owned).collect::<Vec<_>>();
    let session_start = lines.iter().position(|line| line.trim() == "[session]");
    let start = match session_start {
        Some(index) => index,
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[session]".to_owned());
            lines.len() - 1
        }
    };
    let mut end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, line)| {
            let line = line.trim();
            (line.starts_with('[') && line.ends_with(']')).then_some(index)
        })
        .unwrap_or(lines.len());
    let mut insertion_index = end;
    while insertion_index > start + 1 && lines[insertion_index - 1].trim().is_empty() {
        insertion_index -= 1;
    }

    for (key, enabled) in [
        ("reverse_scroll_horizontal", reverse_horizontal),
        ("reverse_scroll_vertical", reverse_vertical),
    ] {
        let existing = lines[start + 1..end].iter().position(|line| {
            line.split_once('=')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
        });
        let replacement = format!("{key} = {enabled}");
        if let Some(offset) = existing {
            lines[start + 1 + offset] = replacement;
        } else {
            lines.insert(insertion_index, replacement);
            insertion_index += 1;
            end += 1;
        }
    }

    let mut updated = lines.join(newline);
    if had_trailing_newline || source.is_empty() {
        updated.push_str(newline);
    }
    updated
}

fn format_node(value: u128) -> String {
    format!("{value:032x}")
}

fn resolve_config_path() -> Option<PathBuf> {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--config" {
            return arguments.next().map(PathBuf::from);
        }
        if argument.ends_with(".toml") {
            return Some(PathBuf::from(argument));
        }
    }
    if let Some(path) = std::env::var_os("EDGEMOUSE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    let working_config = std::env::current_dir().ok()?.join("edgemouse.toml");
    working_config.exists().then_some(working_config)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_path = resolve_config_path();
    tauri::Builder::default()
        .manage(AppState {
            config_path,
            agent_status: Arc::new(Mutex::new(AgentStatusCache::default())),
        })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                window.set_title(&format!("EdgeMouse {}", env!("CARGO_PKG_VERSION")))?;
                #[cfg(target_os = "macos")]
                window.set_decorations(true)?;
            }
            #[cfg(target_os = "macos")]
            install_macos_menu(app.handle(), "zh-CN")?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "show_help"
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.eval("document.querySelector('[data-page=\"about\"]')?.click();");
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_snapshot,
            save_scroll_settings,
            save_layout,
            set_menu_language,
            window_action
        ])
        .run(tauri::generate_context!())
        .expect("failed to run EdgeMouse desktop application");
}

#[cfg(test)]
mod tests {
    use super::{
        AgentStatusCache, local_layout_edge, missed_agent_snapshot, parse_layout_edge,
        scroll_settings_source,
    };
    use edgemouse_agent::control::{ConnectionTelemetry, RunningStatus};
    use edgemouse_core::Edge;
    use std::sync::Mutex;

    #[test]
    fn parses_the_four_layout_edges() {
        assert_eq!(parse_layout_edge("left"), Ok(Edge::Left));
        assert_eq!(parse_layout_edge("right"), Ok(Edge::Right));
        assert_eq!(parse_layout_edge("top"), Ok(Edge::Top));
        assert_eq!(parse_layout_edge("bottom"), Ok(Edge::Bottom));
        assert!(parse_layout_edge("diagonal").is_err());
    }

    #[test]
    fn canonical_layout_is_converted_for_each_computer() {
        assert_eq!(local_layout_edge(Edge::Right, false), Edge::Right);
        assert_eq!(local_layout_edge(Edge::Right, true), Edge::Left);
        assert_eq!(local_layout_edge(Edge::Top, false), Edge::Top);
        assert_eq!(local_layout_edge(Edge::Top, true), Edge::Bottom);
    }

    #[test]
    fn scroll_settings_are_inserted_without_reformatting_other_tables() {
        let source = "[local]\r\nname = \"Windows\"\r\n\r\n[session]\r\ntimeout_ms = 1500\r\n\r\n[layout]\r\npeer_on = \"right\"\r\n";
        let updated = scroll_settings_source(source, true, false);
        assert!(updated.contains("timeout_ms = 1500\r\nreverse_scroll_horizontal = true\r\nreverse_scroll_vertical = false\r\n\r\n[layout]"));
        assert!(updated.contains("[local]\r\nname = \"Windows\""));
    }

    #[test]
    fn scroll_settings_replace_existing_values() {
        let source =
            "[session]\nreverse_scroll_horizontal = false\nreverse_scroll_vertical = true\n";
        let updated = scroll_settings_source(source, true, false);
        assert_eq!(
            updated,
            "[session]\nreverse_scroll_horizontal = true\nreverse_scroll_vertical = false\n"
        );
    }

    #[test]
    fn missing_session_table_is_added() {
        let source = "[layout]\npeer_on = \"right\"\n";
        let updated = scroll_settings_source(source, false, true);
        assert!(updated.ends_with(
            "\n[session]\nreverse_scroll_horizontal = false\nreverse_scroll_vertical = true\n"
        ));
    }

    #[test]
    fn transient_agent_status_misses_keep_the_last_known_running_state() {
        let cache = Mutex::new(AgentStatusCache {
            last_status: Some(RunningStatus {
                process_id: 42,
                version: "test-version".to_owned(),
                connection: ConnectionTelemetry::default(),
            }),
            consecutive_misses: 0,
        });

        let first = missed_agent_snapshot(&cache, None);
        let second = missed_agent_snapshot(&cache, Some("temporary failure".to_owned()));

        assert!(first.running);
        assert!(!first.status_fresh);
        assert_eq!(first.process_id, Some(42));
        assert!(second.running);
        assert!(!second.status_fresh);
        assert!(
            second
                .error
                .is_some_and(|error| error.contains("temporary failure"))
        );
    }

    #[test]
    fn repeated_agent_status_misses_eventually_report_the_service_stopped() {
        let cache = Mutex::new(AgentStatusCache {
            last_status: Some(RunningStatus {
                process_id: 42,
                version: "test-version".to_owned(),
                connection: ConnectionTelemetry::default(),
            }),
            consecutive_misses: 2,
        });

        let snapshot = missed_agent_snapshot(&cache, None);

        assert!(!snapshot.running);
        assert!(!snapshot.status_fresh);
        assert_eq!(snapshot.process_id, None);
    }
}
