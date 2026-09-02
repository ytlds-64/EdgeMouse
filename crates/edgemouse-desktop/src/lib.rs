use edgemouse_agent::config::{LoadedConfig, PeerAddress};
use edgemouse_agent::{control, platform};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Manager;

struct AppState {
    config_path: Option<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSnapshot {
    running: bool,
    process_id: Option<u32>,
    version: String,
    error: Option<String>,
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    desktop_version: String,
    agent: AgentSnapshot,
    platform: PlatformSnapshot,
    config: ConfigSnapshot,
}

#[tauri::command]
fn get_app_snapshot(state: tauri::State<'_, AppState>) -> AppSnapshot {
    AppSnapshot {
        desktop_version: env!("CARGO_PKG_VERSION").to_owned(),
        agent: agent_snapshot(),
        platform: platform_snapshot(),
        config: config_snapshot(state.config_path.as_deref()),
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

fn agent_snapshot() -> AgentSnapshot {
    match control::query_status() {
        Ok(Some(status)) => AgentSnapshot {
            running: true,
            process_id: Some(status.process_id),
            version: status.version,
            error: None,
        },
        Ok(None) => AgentSnapshot {
            running: false,
            process_id: None,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            error: None,
        },
        Err(error) => AgentSnapshot {
            running: false,
            process_id: None,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            error: Some(error.to_string()),
        },
    }
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
    }
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
        .manage(AppState { config_path })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                window.set_title(&format!("EdgeMouse {}", env!("CARGO_PKG_VERSION")))?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_app_snapshot, window_action])
        .run(tauri::generate_context!())
        .expect("failed to run EdgeMouse desktop application");
}
