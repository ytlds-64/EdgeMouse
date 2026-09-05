use edgemouse_agent::config::{
    LoadedConfig, PairingConfig, PeerAddress, SessionPreferences, edge_name, import_paired_config,
    initialize_unpaired_config, persist_peer_on, persist_session_preferences,
};
use edgemouse_agent::{control, pairing, platform};
use edgemouse_core::{DisplayGeometry, Edge, Rect};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

struct AppState {
    config_path: Option<PathBuf>,
    agent_status: Arc<Mutex<AgentStatusCache>>,
    preferences_path: PathBuf,
    preferences: Arc<Mutex<DesktopPreferences>>,
    pairing_session: Arc<Mutex<Option<PairingSession>>>,
}

struct PairingSession {
    code: String,
    stopping: Arc<AtomicBool>,
    receiver: Receiver<Result<pairing::PairingResult, String>>,
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
    peer_desktop: Option<DesktopSnapshot>,
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
            peer_desktop: telemetry.peer_desktop.map(DesktopSnapshot::from),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSnapshot {
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    scale_factor: f64,
    displays: Vec<DisplaySnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DisplaySnapshot {
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    pixel_width: u32,
    pixel_height: u32,
    scale_factor: f64,
    primary: bool,
}

impl From<DisplayGeometry> for DisplaySnapshot {
    fn from(display: DisplayGeometry) -> Self {
        Self {
            origin_x: display.bounds.origin.x,
            origin_y: display.bounds.origin.y,
            width: display.bounds.width,
            height: display.bounds.height,
            pixel_width: display.pixel_width,
            pixel_height: display.pixel_height,
            scale_factor: display.scale_factor,
            primary: display.primary,
        }
    }
}

impl DesktopSnapshot {
    fn new(bounds: Rect, scale_factor: f64, displays: Vec<DisplayGeometry>) -> Self {
        Self {
            origin_x: bounds.origin.x,
            origin_y: bounds.origin.y,
            width: bounds.width,
            height: bounds.height,
            scale_factor,
            displays: displays.into_iter().map(DisplaySnapshot::from).collect(),
        }
    }
}

impl From<control::DesktopTelemetry> for DesktopSnapshot {
    fn from(desktop: control::DesktopTelemetry) -> Self {
        Self::new(desktop.bounds, desktop.scale_factor, desktop.displays)
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
    desktop: Option<DesktopSnapshot>,
    geometry_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigSnapshot {
    path: Option<String>,
    valid: bool,
    pairing_required: bool,
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
    pointer_smoothing: Option<u8>,
    keyboard_enabled: Option<bool>,
    reclaim_enabled: Option<bool>,
    block_switch_while_dragging: Option<bool>,
    auto_reconnect: Option<bool>,
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
struct SavedInputSettings {
    restarted: bool,
    warning: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedLayout {
    applied_live: bool,
    warning: Option<String>,
    local_peer_on: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentServiceResult {
    running: bool,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticCheck {
    key: String,
    passed: bool,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticReport {
    checks: Vec<DiagnosticCheck>,
    summary: String,
    log_lines: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileActionResult {
    path: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DesktopPreferences {
    autostart: bool,
    background: bool,
    notifications: bool,
    theme: String,
    language: String,
    update_channel: String,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            autostart: false,
            background: true,
            notifications: true,
            theme: "system".to_owned(),
            language: "zh-CN".to_owned(),
            update_channel: "stable".to_owned(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedDesktopPreferences {
    preferences: DesktopPreferences,
    notification_ready: bool,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingStatus {
    phase: String,
    code: Option<String>,
    peer_name: Option<String>,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateResult {
    available: bool,
    current_version: String,
    version: Option<String>,
    installed: bool,
    message: String,
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
async fn check_for_updates(
    app: tauri::AppHandle,
    install: bool,
) -> Result<AppUpdateResult, String> {
    use tauri_plugin_updater::UpdaterExt;

    let current_version = env!("CARGO_PKG_VERSION").to_owned();
    let updater = app
        .updater()
        .map_err(|error| format!("无法初始化更新服务：{error}"))?;
    let Some(update) = updater
        .check()
        .await
        .map_err(|error| format!("无法连接更新服务器：{error}"))?
    else {
        return Ok(AppUpdateResult {
            available: false,
            current_version,
            version: None,
            installed: false,
            message: "当前已是最新版".to_owned(),
        });
    };
    let version = update.version.to_string();
    if !install {
        return Ok(AppUpdateResult {
            available: true,
            current_version,
            version: Some(version.clone()),
            installed: false,
            message: format!("发现 EdgeMouse {version}，可以下载并安装"),
        });
    }
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| format!("下载或安装 EdgeMouse {version} 失败：{error}"))?;
    app.restart();
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

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn save_input_settings(
    state: tauri::State<'_, AppState>,
    profile: String,
    reverse_horizontal: bool,
    reverse_vertical: bool,
    pointer_smoothing: u8,
    keyboard_enabled: bool,
    reclaim_enabled: bool,
    drag_lock: bool,
) -> Result<SavedInputSettings, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法保存输入设置".to_owned())?;
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        let config =
            LoadedConfig::load(&path).map_err(|error| format!("读取输入配置失败：{error}"))?;
        let mut preferences = session_preferences(&config);
        let outgoing_profile = if cfg!(target_os = "windows") {
            "windows-to-mac"
        } else {
            "mac-to-windows"
        };
        let incoming_profile = if cfg!(target_os = "windows") {
            "mac-to-windows"
        } else {
            "windows-to-mac"
        };
        if profile == outgoing_profile {
            preferences.reverse_scroll_horizontal = reverse_horizontal;
            preferences.reverse_scroll_vertical = reverse_vertical;
            preferences.keyboard_enabled = keyboard_enabled;
            preferences.block_switch_while_dragging = drag_lock;
        } else if profile == incoming_profile {
            preferences.pointer_smoothing = pointer_smoothing;
            preferences.reclaim_enabled = reclaim_enabled;
        } else {
            return Err(format!("不支持的输入方向：{profile}"));
        }
        persist_session_preferences(&path, preferences)
            .map_err(|error| format!("保存输入设置失败：{error}"))?;
        let (restarted, warning) = restart_agent_if_running(&path);
        let mut cache = agent_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = AgentStatusCache::default();
        Ok(SavedInputSettings { restarted, warning })
    })
    .await
    .map_err(|error| format!("保存输入设置的后台任务失败：{error}"))?
}

#[tauri::command]
async fn save_connection_settings(
    state: tauri::State<'_, AppState>,
    auto_reconnect: bool,
) -> Result<SavedInputSettings, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法保存连接设置".to_owned())?;
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        let config =
            LoadedConfig::load(&path).map_err(|error| format!("读取连接配置失败：{error}"))?;
        let mut preferences = session_preferences(&config);
        preferences.auto_reconnect = auto_reconnect;
        persist_session_preferences(&path, preferences)
            .map_err(|error| format!("保存连接设置失败：{error}"))?;
        let (restarted, warning) = restart_agent_if_running(&path);
        let mut cache = agent_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = AgentStatusCache::default();
        Ok(SavedInputSettings { restarted, warning })
    })
    .await
    .map_err(|error| format!("保存连接设置的后台任务失败：{error}"))?
}

#[tauri::command]
async fn reconnect_agent(state: tauri::State<'_, AppState>) -> Result<AgentServiceResult, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法重新连接".to_owned())?;
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        stop_agent()?;
        let result = start_agent(&path)?;
        let mut cache = agent_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = AgentStatusCache::default();
        Ok(result)
    })
    .await
    .map_err(|error| format!("重新连接的后台任务失败：{error}"))?
}

#[tauri::command]
fn get_desktop_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> DesktopPreferences {
    use tauri_plugin_autostart::ManagerExt;

    let mut preferences = state
        .preferences
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        preferences.autostart = enabled;
    }
    preferences
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn save_desktop_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    autostart: bool,
    background: bool,
    notifications: bool,
    theme: String,
    language: String,
    update_channel: String,
) -> Result<SavedDesktopPreferences, String> {
    use tauri_plugin_autostart::ManagerExt;
    use tauri_plugin_notification::{NotificationExt, PermissionState};

    if !matches!(theme.as_str(), "system" | "light" | "dark") {
        return Err("不支持的界面主题".to_owned());
    }
    if !matches!(language.as_str(), "zh-CN" | "en") {
        return Err("不支持的界面语言".to_owned());
    }
    if !matches!(update_channel.as_str(), "stable" | "preview") {
        return Err("不支持的更新通道".to_owned());
    }

    if autostart {
        app.autolaunch()
            .enable()
            .map_err(|error| format!("无法启用登录时启动：{error}"))?;
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| format!("无法关闭登录时启动：{error}"))?;
    }

    let notification_ready = if notifications {
        app.notification()
            .request_permission()
            .map(|permission| permission == PermissionState::Granted)
            .unwrap_or(false)
    } else {
        false
    };
    let preferences = DesktopPreferences {
        autostart,
        background,
        notifications: notifications && notification_ready,
        theme,
        language,
        update_channel,
    };
    persist_desktop_preferences(&state.preferences_path, &preferences)?;
    *state
        .preferences
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = preferences.clone();

    if notifications && notification_ready {
        let _ = app
            .notification()
            .builder()
            .title("EdgeMouse")
            .body("系统通知已启用")
            .show();
    }
    Ok(SavedDesktopPreferences {
        preferences,
        notification_ready,
        message: if notifications && !notification_ready {
            "其他设置已保存，但系统没有授予通知权限".to_owned()
        } else {
            "桌面设置已保存并生效".to_owned()
        },
    })
}

#[tauri::command]
fn show_system_notification(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    title: String,
    body: String,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    let enabled = state
        .preferences
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .notifications;
    if !enabled {
        return Ok(());
    }
    if title.chars().count() > 80 || body.chars().count() > 240 {
        return Err("通知内容过长".to_owned());
    }
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| format!("无法显示系统通知：{error}"))
}

#[tauri::command]
fn reset_preferences(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<SavedDesktopPreferences, String> {
    use tauri_plugin_autostart::ManagerExt;

    if let Some(path) = state.config_path.as_deref() {
        persist_session_preferences(
            path,
            SessionPreferences {
                entry_hysteresis: 8.0,
                reverse_scroll_horizontal: false,
                reverse_scroll_vertical: false,
                pointer_smoothing: 52,
                keyboard_enabled: true,
                reclaim_enabled: true,
                block_switch_while_dragging: true,
                auto_reconnect: true,
            },
        )
        .map_err(|error| format!("恢复输入与连接设置失败：{error}"))?;
        let _ = restart_agent_if_running(path);
    }
    let _ = app.autolaunch().disable();
    let preferences = DesktopPreferences::default();
    persist_desktop_preferences(&state.preferences_path, &preferences)?;
    *state
        .preferences
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = preferences.clone();
    Ok(SavedDesktopPreferences {
        preferences,
        notification_ready: true,
        message: "已恢复推荐设置；设备身份和可信证书均已保留".to_owned(),
    })
}

#[tauri::command]
async fn run_diagnostics(state: tauri::State<'_, AppState>) -> Result<DiagnosticReport, String> {
    let config_path = state.config_path.clone();
    tauri::async_runtime::spawn_blocking(move || build_diagnostic_report(config_path.as_deref()))
        .await
        .map_err(|error| format!("运行诊断的后台任务失败：{error}"))
}

#[tauri::command]
async fn open_logs_folder(state: tauri::State<'_, AppState>) -> Result<FileActionResult, String> {
    let config_path = state.config_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let directory = log_directory(config_path.as_deref())?;
        fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建日志目录 {}：{error}", directory.display()))?;
        open_path(&directory)?;
        Ok(FileActionResult {
            path: directory.display().to_string(),
            message: "已打开日志文件夹".to_owned(),
        })
    })
    .await
    .map_err(|error| format!("打开日志文件夹的后台任务失败：{error}"))?
}

#[tauri::command]
async fn export_diagnostics(
    state: tauri::State<'_, AppState>,
    include_logs: bool,
    include_config: bool,
    include_system: bool,
) -> Result<FileActionResult, String> {
    let config_path = state.config_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        export_diagnostic_bundle(
            config_path.as_deref(),
            include_logs,
            include_config,
            include_system,
        )
    })
    .await
    .map_err(|error| format!("导出诊断包的后台任务失败：{error}"))?
}

#[tauri::command]
async fn verify_trusted_device(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let config =
            LoadedConfig::load(&path).map_err(|error| format!("可信设备验证失败：{error}"))?;
        Ok(format!(
            "可信证书验证通过 · {}",
            format_node(config.peer_node.0)
        ))
    })
    .await
    .map_err(|error| format!("验证可信设备的后台任务失败：{error}"))?
}

#[tauri::command]
async fn start_pairing_host(state: tauri::State<'_, AppState>) -> Result<PairingStatus, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法开始配对".to_owned())?;
    let sessions = Arc::clone(&state.pairing_session);
    tauri::async_runtime::spawn_blocking(move || {
        cancel_pairing_session(&sessions);
        let _ = stop_agent();
        let config =
            PairingConfig::load(&path).map_err(|error| format!("读取配对身份失败：{error}"))?;
        let host = pairing::PairingHost::start(config)
            .map_err(|error| format!("无法创建配对会话：{error}"))?;
        let code = host.formatted_code();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = host
                .run(&worker_stopping)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        *sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(PairingSession {
            code: code.clone(),
            stopping,
            receiver,
        });
        Ok(PairingStatus {
            phase: "hosting".to_owned(),
            code: Some(code),
            peer_name: None,
            message: "配对码已生成，正在等待另一台电脑加入".to_owned(),
        })
    })
    .await
    .map_err(|error| format!("创建配对会话的后台任务失败：{error}"))?
}

#[tauri::command]
fn get_pairing_status(state: tauri::State<'_, AppState>) -> PairingStatus {
    let mut sessions = state
        .pairing_session
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(session) = sessions.as_ref() else {
        return PairingStatus {
            phase: "idle".to_owned(),
            code: None,
            peer_name: None,
            message: "没有正在进行的配对".to_owned(),
        };
    };
    match session.receiver.try_recv() {
        Ok(Ok(result)) => {
            let peer_name = result.peer_name;
            *sessions = None;
            if let Some(path) = state.config_path.as_deref() {
                let _ = start_agent(path);
            }
            PairingStatus {
                phase: "complete".to_owned(),
                code: None,
                peer_name: Some(peer_name.clone()),
                message: format!("已与 {peer_name} 完成安全配对"),
            }
        }
        Ok(Err(error)) => {
            *sessions = None;
            if let Some(path) = state.config_path.as_deref() {
                let _ = start_agent(path);
            }
            PairingStatus {
                phase: "failed".to_owned(),
                code: None,
                peer_name: None,
                message: format!("配对失败：{error}"),
            }
        }
        Err(TryRecvError::Empty) => PairingStatus {
            phase: "hosting".to_owned(),
            code: Some(session.code.clone()),
            peer_name: None,
            message: "正在等待另一台电脑输入配对码".to_owned(),
        },
        Err(TryRecvError::Disconnected) => {
            *sessions = None;
            PairingStatus {
                phase: "failed".to_owned(),
                code: None,
                peer_name: None,
                message: "配对任务意外结束".to_owned(),
            }
        }
    }
}

#[tauri::command]
async fn join_pairing(
    state: tauri::State<'_, AppState>,
    code: String,
    host: Option<String>,
) -> Result<PairingStatus, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法加入配对".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _ = stop_agent();
        let config =
            PairingConfig::load(&path).map_err(|error| format!("读取配对身份失败：{error}"))?;
        let stopping = AtomicBool::new(false);
        let direct_host = host
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let result = pairing::join(config, &code, direct_host, &stopping)
            .map_err(|error| format!("加入配对失败：{error}"));
        let restart = start_agent(&path);
        let result = result?;
        if let Err(error) = restart {
            return Err(format!("证书已保存，但后台服务恢复失败：{error}"));
        }
        Ok(PairingStatus {
            phase: "complete".to_owned(),
            code: None,
            peer_name: Some(result.peer_name.clone()),
            message: format!("已与 {} 完成安全配对", result.peer_name),
        })
    })
    .await
    .map_err(|error| format!("加入配对的后台任务失败：{error}"))?
}

#[tauri::command]
fn cancel_pairing(state: tauri::State<'_, AppState>) -> Result<(), String> {
    cancel_pairing_session(&state.pairing_session);
    if let Some(path) = state.config_path.as_deref() {
        let _ = start_agent(path);
    }
    Ok(())
}

#[tauri::command]
async fn forget_trusted_device(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml".to_owned())?;
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        let _ = stop_agent();
        let config =
            PairingConfig::load(&path).map_err(|error| format!("读取可信证书位置失败：{error}"))?;
        if !config.peer_certificate_path.is_file() {
            return Ok("当前没有已保存的可信设备证书".to_owned());
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("系统时间无效：{error}"))?
            .as_secs();
        let backup = config
            .peer_certificate_path
            .with_extension(format!("unpaired-{stamp}.bak"));
        fs::rename(&config.peer_certificate_path, &backup).map_err(|error| {
            format!(
                "无法备份可信证书 {}：{error}",
                config.peer_certificate_path.display()
            )
        })?;
        *agent_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = AgentStatusCache::default();
        Ok(format!("已解除配对；原证书已备份到 {}", backup.display()))
    })
    .await
    .map_err(|error| format!("解除配对的后台任务失败：{error}"))?
}

fn cancel_pairing_session(sessions: &Mutex<Option<PairingSession>>) {
    if let Some(session) = sessions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        session.stopping.store(true, Ordering::Release);
    }
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    const ALLOWED: [&str; 3] = [
        "https://github.com/ytlds-64/EdgeMouse",
        "https://github.com/ytlds-64/EdgeMouse/issues",
        "https://github.com/ytlds-64/EdgeMouse/blob/main/LICENSE",
    ];
    if !ALLOWED.iter().any(|allowed| url == *allowed) {
        return Err("不允许打开这个外部地址".to_owned());
    }
    open_url(&url)
}

#[tauri::command]
async fn save_layout(
    state: tauri::State<'_, AppState>,
    peer_on: String,
    edge_protection: bool,
) -> Result<SavedLayout, String> {
    let path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法保存屏幕布局".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let windows_to_mac = parse_layout_edge(&peer_on)?;
        let local_peer_on = local_layout_edge(windows_to_mac, cfg!(target_os = "macos"));
        persist_peer_on(&path, local_peer_on)?;
        let config =
            LoadedConfig::load(&path).map_err(|error| format!("读取边缘设置失败：{error}"))?;
        let mut preferences = session_preferences(&config);
        preferences.entry_hysteresis = if edge_protection { 8.0 } else { 0.0 };
        persist_session_preferences(&path, preferences)
            .map_err(|error| format!("保存边缘设置失败：{error}"))?;
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

#[tauri::command]
async fn set_agent_running(
    state: tauri::State<'_, AppState>,
    running: bool,
) -> Result<AgentServiceResult, String> {
    let config_path = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 edgemouse.toml；无法启动后台服务".to_owned())?;
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        let result = if running {
            start_agent(&config_path)
        } else {
            stop_agent()
        }?;
        let mut cache = agent_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = AgentStatusCache::default();
        Ok(result)
    })
    .await
    .map_err(|error| format!("后台服务操作失败：{error}"))?
}

#[tauri::command]
async fn import_existing_pairing(
    state: tauri::State<'_, AppState>,
) -> Result<AgentServiceResult, String> {
    let destination = state
        .config_path
        .clone()
        .ok_or_else(|| "未找到 EdgeMouse 用户配置目录".to_owned())?;
    let selected = tauri::async_runtime::spawn_blocking(select_existing_config)
        .await
        .map_err(|error| format!("打开配置选择窗口失败：{error}"))??;
    let Some(selected) = selected else {
        return Ok(AgentServiceResult {
            running: false,
            message: "已取消导入旧配对配置".to_owned(),
        });
    };
    let agent_status = Arc::clone(&state.agent_status);
    tauri::async_runtime::spawn_blocking(move || {
        let _ = stop_agent();
        let backup = import_paired_config(&selected, &destination)
            .map_err(|error| format!("导入旧配对配置失败：{error}"))?;
        let result = start_agent(&destination)?;
        *agent_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = AgentStatusCache::default();
        Ok(AgentServiceResult {
            running: result.running,
            message: format!(
                "旧配对配置已导入，EdgeMouse 已启动；原配置备份在 {}",
                backup.display()
            ),
        })
    })
    .await
    .map_err(|error| format!("导入配对配置的后台任务失败：{error}"))?
}

#[cfg(target_os = "macos")]
fn select_existing_config() -> Result<Option<PathBuf>, String> {
    let output = Command::new("osascript")
        .args([
            "-e",
            "POSIX path of (choose file with prompt \"选择以前使用的 edgemouse.toml\")",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("无法打开 macOS 文件选择窗口：{error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        if error.contains("User canceled") || error.contains("-128") {
            return Ok(None);
        }
        return Err(format!("macOS 文件选择窗口失败：{}", error.trim()));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

#[cfg(target_os = "windows")]
fn select_existing_config() -> Result<Option<PathBuf>, String> {
    let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = '选择以前使用的 edgemouse.toml'
$dialog.Filter = 'EdgeMouse 配置 (edgemouse.toml)|edgemouse.toml|TOML 文件 (*.toml)|*.toml|所有文件 (*.*)|*.*'
$dialog.Multiselect = $false
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
  [Console]::Write($dialog.FileName)
}
"#;
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-Sta", "-Command", script])
        .stdin(Stdio::null());
    suppress_windows_console(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("无法打开 Windows 文件选择窗口：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Windows 文件选择窗口失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn select_existing_config() -> Result<Option<PathBuf>, String> {
    Err("当前平台暂不支持选择旧配置".to_owned())
}

fn start_agent(config_path: &Path) -> Result<AgentServiceResult, String> {
    if let Some(status) = control::query_status().map_err(|error| error.to_string())? {
        return Ok(AgentServiceResult {
            running: true,
            message: format!("EdgeMouse 已在运行 · PID {}", status.process_id),
        });
    }
    LoadedConfig::load(config_path).map_err(|error| {
        if config_requires_pairing(config_path) {
            "尚未完成安全配对。请打开「连接」页面配对新设备，或导入以前的 edgemouse.toml".to_owned()
        } else {
            format!("启动前配置检查失败：{error}")
        }
    })?;
    let agent_path = resolve_agent_executable(config_path)?;
    let project_root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let log_directory = project_root.join("logs");
    fs::create_dir_all(&log_directory)
        .map_err(|error| format!("无法创建日志目录 {}：{error}", log_directory.display()))?;
    let output_log = log_directory.join("desktop-agent.out.log");
    let error_log = log_directory.join("desktop-agent.err.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_log)
        .map_err(|error| format!("无法打开日志 {}：{error}", output_log.display()))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&error_log)
        .map_err(|error| format!("无法打开日志 {}：{error}", error_log.display()))?;

    let mut command = Command::new(&agent_path);
    command
        .arg("run")
        .arg(config_path)
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    suppress_windows_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 {}：{error}", agent_path.display()))?;

    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(125));
        if let Some(status) = control::query_status().map_err(|error| error.to_string())? {
            return Ok(AgentServiceResult {
                running: true,
                message: format!("EdgeMouse 已启动 · PID {}", status.process_id),
            });
        }
        if let Some(exit_status) = child
            .try_wait()
            .map_err(|error| format!("无法读取后台服务状态：{error}"))?
        {
            return Err(format!(
                "后台服务启动后立即退出（{exit_status}）；请查看 {}",
                error_log.display()
            ));
        }
    }
    Err(format!("后台服务启动超时；请查看 {}", error_log.display()))
}

fn config_requires_pairing(path: &Path) -> bool {
    PairingConfig::load(path).is_ok_and(|config| !config.peer_certificate_path.is_file())
}

fn stop_agent() -> Result<AgentServiceResult, String> {
    let Some(status) = control::request_stop().map_err(|error| error.to_string())? else {
        return Ok(AgentServiceResult {
            running: false,
            message: "EdgeMouse 已停止".to_owned(),
        });
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
        if control::query_status()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(AgentServiceResult {
                running: false,
                message: format!("EdgeMouse 已安全停止 · PID {}", status.process_id),
            });
        }
    }
    Err("后台服务没有在 5 秒内停止；本地控制权已请求恢复".to_owned())
}

fn session_preferences(config: &LoadedConfig) -> SessionPreferences {
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

fn restart_agent_if_running(path: &Path) -> (bool, Option<String>) {
    match control::query_status() {
        Ok(Some(_)) => match stop_agent().and_then(|_| start_agent(path)) {
            Ok(_) => (true, None),
            Err(error) => (
                false,
                Some(format!("设置已保存，但后台服务重启失败：{error}")),
            ),
        },
        Ok(None) => (false, Some("设置已保存；后台服务下次启动时生效".to_owned())),
        Err(error) => (
            false,
            Some(format!("设置已保存；无法确认后台服务状态：{error}")),
        ),
    }
}

fn resolve_agent_executable(config_path: &Path) -> Result<PathBuf, String> {
    let executable_name = if cfg!(target_os = "windows") {
        "edgemouse.exe"
    } else {
        "edgemouse"
    };
    let mut candidates = Vec::new();
    if let Ok(current_executable) = std::env::current_exe()
        && let Some(directory) = current_executable.parent()
    {
        candidates.push(directory.join(executable_name));
    }
    if let Some(project_root) = config_path.parent() {
        candidates.push(
            project_root
                .join("target")
                .join("release")
                .join(executable_name),
        );
    }
    candidates.dedup();

    let expected_version = format!("edgemouse {}", env!("CARGO_PKG_VERSION"));
    let mut found_versions = Vec::new();
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let mut command = Command::new(&candidate);
        command.arg("version").stdin(Stdio::null());
        suppress_windows_console(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("无法检查 {}：{error}", candidate.display()))?;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success() && version == expected_version {
            return Ok(candidate);
        }
        found_versions.push(format!("{} ({version})", candidate.display()));
    }
    let detail = if found_versions.is_empty() {
        "没有找到后台程序".to_owned()
    } else {
        format!("找到的版本不匹配：{}", found_versions.join("；"))
    };
    Err(format!(
        "{detail}。请先同时构建 edgemouse-agent 与 edgemouse-desktop {}",
        env!("CARGO_PKG_VERSION")
    ))
}

#[cfg(target_os = "windows")]
fn suppress_windows_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn suppress_windows_console(_command: &mut Command) {}

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
            desktop: Some(DesktopSnapshot::new(
                desktop.bounds,
                desktop.scale_factor,
                desktop.displays,
            )),
            geometry_error: None,
        },
        Err(error) => PlatformSnapshot {
            operating_system: status.operating_system.to_owned(),
            permission_granted: status.permission_granted,
            desktop_width: None,
            desktop_height: None,
            scale_factor: None,
            display_count: None,
            desktop: None,
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
            pairing_required: false,
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
            pointer_smoothing: Some(config.pointer_smoothing),
            keyboard_enabled: Some(config.keyboard_enabled),
            reclaim_enabled: Some(config.reclaim_enabled),
            block_switch_while_dragging: Some(config.session.block_switch_while_dragging),
            auto_reconnect: Some(config.auto_reconnect),
        },
        Err(error) => {
            if let Ok(pairing) = PairingConfig::load(path)
                && !pairing.peer_certificate_path.is_file()
            {
                return ConfigSnapshot {
                    path: Some(path.display().to_string()),
                    valid: false,
                    pairing_required: true,
                    error: Some("尚未完成安全配对".to_owned()),
                    local_name: Some(pairing.local_name),
                    local_node: Some(format_node(pairing.local_node.0)),
                    peer_node: None,
                    peer_address: Some("auto (UDP 43892)".to_owned()),
                    listen_address: Some("0.0.0.0:43891".to_owned()),
                    local_screen_name: None,
                    local_screen_id: None,
                    local_screen_automatic: Some(true),
                    peer_screen_name: None,
                    peer_screen_id: None,
                    peer_on: None,
                    entry_hysteresis: None,
                    peer_timeout_ms: None,
                    reverse_scroll_horizontal: None,
                    reverse_scroll_vertical: None,
                    pointer_smoothing: None,
                    keyboard_enabled: None,
                    reclaim_enabled: None,
                    block_switch_while_dragging: None,
                    auto_reconnect: None,
                };
            }
            empty_config(Some(path), &error.to_string())
        }
    }
}

fn empty_config(path: Option<&Path>, error: &str) -> ConfigSnapshot {
    ConfigSnapshot {
        path: path.map(|value| value.display().to_string()),
        valid: false,
        pairing_required: false,
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
        pointer_smoothing: None,
        keyboard_enabled: None,
        reclaim_enabled: None,
        block_switch_while_dragging: None,
        auto_reconnect: None,
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

fn build_diagnostic_report(config_path: Option<&Path>) -> DiagnosticReport {
    let config_result = config_path
        .ok_or_else(|| "未找到 edgemouse.toml".to_owned())
        .and_then(|path| LoadedConfig::load(path).map_err(|error| error.to_string()));
    let platform = platform::current_status();
    let running = control::query_status().ok().flatten();

    let certificate = match &config_result {
        Ok(config) => DiagnosticCheck {
            key: "certificate".to_owned(),
            passed: true,
            detail: format!(
                "可信设备指纹 {}…{} 已加载",
                &format_node(config.peer_node.0)[..4],
                &format_node(config.peer_node.0)[28..]
            ),
        },
        Err(error) => DiagnosticCheck {
            key: "certificate".to_owned(),
            passed: false,
            detail: format!("配置或可信证书不可用：{error}"),
        },
    };
    let discovery = match &config_result {
        Ok(config) => {
            let automatic = matches!(config.peer_address, PeerAddress::Auto);
            DiagnosticCheck {
                key: "discovery".to_owned(),
                passed: automatic,
                detail: if automatic {
                    "局域网自动发现已启用 · UDP 43892".to_owned()
                } else {
                    "当前使用固定地址，IP 变化后不会自动发现".to_owned()
                },
            }
        }
        Err(_) => DiagnosticCheck {
            key: "discovery".to_owned(),
            passed: false,
            detail: "无法从配置确认发现模式".to_owned(),
        },
    };
    let permission_ok = platform.permission_granted != Some(false);
    let permissions = DiagnosticCheck {
        key: "permissions".to_owned(),
        passed: permission_ok,
        detail: if platform.operating_system.eq_ignore_ascii_case("windows") {
            "Windows 输入捕获与注入接口可用".to_owned()
        } else if permission_ok {
            "macOS 输入权限可用".to_owned()
        } else {
            "macOS 辅助功能或输入监控权限尚未授予".to_owned()
        },
    };
    let recovery_ok = running.is_some()
        && config_result
            .as_ref()
            .is_ok_and(|config| config.auto_reconnect);
    let recovery = DiagnosticCheck {
        key: "recovery".to_owned(),
        passed: recovery_ok,
        detail: match running {
            Some(_) if recovery_ok => "后台服务在线，心跳恢复与自动重连已启用".to_owned(),
            Some(_) => "后台服务在线，但自动重连已关闭".to_owned(),
            None => "后台服务未运行，无法验证自动恢复链路".to_owned(),
        },
    };
    let checks = vec![certificate, discovery, permissions, recovery];
    let passed = checks.iter().filter(|check| check.passed).count();
    let summary = format!(
        "EdgeMouse {} 诊断：{passed}/{} 项通过\n{}",
        env!("CARGO_PKG_VERSION"),
        checks.len(),
        checks
            .iter()
            .map(|check| format!(
                "{} {}：{}",
                if check.passed { "✓" } else { "×" },
                check.key,
                check.detail
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
    DiagnosticReport {
        checks,
        summary,
        log_lines: recent_log_lines(config_path, 8),
    }
}

fn log_directory(config_path: Option<&Path>) -> Result<PathBuf, String> {
    let base = config_path
        .and_then(Path::parent)
        .ok_or_else(|| "未找到配置目录，无法定位日志".to_owned())?;
    Ok(base.join("logs"))
}

fn recent_log_lines(config_path: Option<&Path>, maximum: usize) -> Vec<String> {
    let Some(base) = config_path.and_then(Path::parent) else {
        return Vec::new();
    };
    let candidates = [
        base.join("logs").join("desktop-agent.err.log"),
        base.join("logs").join("desktop-agent.out.log"),
        base.join(if cfg!(target_os = "windows") {
            "windows-current.log"
        } else {
            "mac-current.log"
        }),
    ];
    let mut lines = Vec::new();
    for path in candidates {
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        lines.extend(
            source
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(redact_ipv4_addresses),
        );
    }
    if lines.len() > maximum {
        lines.drain(..lines.len() - maximum);
    }
    lines
}

fn redact_ipv4_addresses(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
                index += 1;
            }
            let candidate = &source[start..index];
            let parts = candidate.split('.').collect::<Vec<_>>();
            if parts.len() == 4
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
            {
                output.push_str(parts[0]);
                output.push('.');
                output.push_str(parts[1]);
                output.push('.');
                output.push_str(parts[2]);
                output.push_str(".x");
            } else {
                output.push_str(candidate);
            }
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn export_diagnostic_bundle(
    config_path: Option<&Path>,
    include_logs: bool,
    include_config: bool,
    include_system: bool,
) -> Result<FileActionResult, String> {
    use zip::write::SimpleFileOptions;

    let report = build_diagnostic_report(config_path);
    let downloads = std::env::var_os(if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .map(PathBuf::from)
    .map(|path| path.join("Downloads"))
    .filter(|path| path.is_dir())
    .or_else(|| config_path.and_then(Path::parent).map(Path::to_path_buf))
    .ok_or_else(|| "无法定位诊断包保存目录".to_owned())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("系统时间无效：{error}"))?
        .as_secs();
    let destination = downloads.join(format!("edgemouse-diagnostics-{stamp}.zip"));
    let file = File::create(&destination)
        .map_err(|error| format!("无法创建 {}：{error}", destination.display()))?;
    let mut archive = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    archive
        .start_file("summary.txt", options)
        .map_err(|error| format!("无法创建诊断摘要：{error}"))?;
    archive
        .write_all(report.summary.as_bytes())
        .map_err(|error| format!("无法写入诊断摘要：{error}"))?;

    if include_logs {
        archive
            .start_file("recent.log", options)
            .map_err(|error| format!("无法创建日志摘要：{error}"))?;
        archive
            .write_all(report.log_lines.join("\n").as_bytes())
            .map_err(|error| format!("无法写入日志摘要：{error}"))?;
    }
    if include_config {
        let config = config_snapshot(config_path);
        let safe_config = format!(
            "valid={}\nlocal_name={}\npeer_address={}\nlisten_address={}\npeer_on={}\nauto_reconnect={}\nkeyboard_enabled={}\npointer_smoothing={}\n",
            config.valid,
            config.local_name.as_deref().unwrap_or("unknown"),
            config
                .peer_address
                .as_deref()
                .map(redact_ipv4_addresses)
                .unwrap_or_else(|| "unknown".to_owned()),
            config
                .listen_address
                .as_deref()
                .map(redact_ipv4_addresses)
                .unwrap_or_else(|| "unknown".to_owned()),
            config.peer_on.as_deref().unwrap_or("unknown"),
            config.auto_reconnect.unwrap_or(false),
            config.keyboard_enabled.unwrap_or(false),
            config.pointer_smoothing.unwrap_or(0),
        );
        archive
            .start_file("config-summary.txt", options)
            .map_err(|error| format!("无法创建配置摘要：{error}"))?;
        archive
            .write_all(safe_config.as_bytes())
            .map_err(|error| format!("无法写入配置摘要：{error}"))?;
    }
    if include_system {
        let platform = platform_snapshot();
        let system = format!(
            "desktop_version={}\noperating_system={}\npermission_granted={:?}\ndisplay_count={:?}\ndesktop={}x{}\n",
            env!("CARGO_PKG_VERSION"),
            platform.operating_system,
            platform.permission_granted,
            platform.display_count,
            platform.desktop_width.unwrap_or(0.0),
            platform.desktop_height.unwrap_or(0.0),
        );
        archive
            .start_file("system.txt", options)
            .map_err(|error| format!("无法创建系统摘要：{error}"))?;
        archive
            .write_all(system.as_bytes())
            .map_err(|error| format!("无法写入系统摘要：{error}"))?;
    }
    archive
        .finish()
        .map_err(|error| format!("无法完成诊断包：{error}"))?;
    Ok(FileActionResult {
        path: destination.display().to_string(),
        message: "诊断包已保存到下载目录".to_owned(),
    })
}

fn open_path(path: &Path) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(path);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    suppress_windows_console(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开 {}：{error}", path.display()))
}

fn open_url(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler").arg(url);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    suppress_windows_console(&mut command);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开项目地址：{error}"))
}

fn desktop_preferences_path(config_path: Option<&Path>) -> PathBuf {
    if let Some(parent) = config_path.and_then(Path::parent) {
        return parent.join("edgemouse-desktop.toml");
    }
    if cfg!(target_os = "windows")
        && let Some(base) = std::env::var_os("APPDATA")
    {
        return PathBuf::from(base)
            .join("EdgeMouse")
            .join("desktop-settings.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("EdgeMouse")
            .join("desktop-settings.toml");
    }
    PathBuf::from("edgemouse-desktop.toml")
}

fn load_desktop_preferences(path: &Path) -> DesktopPreferences {
    fs::read_to_string(path)
        .ok()
        .and_then(|source| toml::from_str(&source).ok())
        .unwrap_or_default()
}

fn persist_desktop_preferences(
    path: &Path,
    preferences: &DesktopPreferences,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建设置目录 {}：{error}", parent.display()))?;
    }
    let source = toml::to_string_pretty(preferences)
        .map_err(|error| format!("无法生成桌面设置：{error}"))?;
    fs::write(path, source).map_err(|error| format!("无法保存桌面设置 {}：{error}", path.display()))
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
    if working_config.exists() {
        return Some(working_config);
    }
    default_config_path()
}

fn default_config_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("EdgeMouse").join("edgemouse.toml"));
    }
    std::env::var_os("HOME").map(PathBuf::from).map(|home| {
        home.join("Library")
            .join("Application Support")
            .join("EdgeMouse")
            .join("edgemouse.toml")
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_path = resolve_config_path();
    if let Some(path) = config_path.as_deref()
        && let Err(error) = initialize_unpaired_config(path)
    {
        eprintln!("EdgeMouse could not prepare its per-user configuration: {error}");
    }
    let preferences_path = desktop_preferences_path(config_path.as_deref());
    let preferences = load_desktop_preferences(&preferences_path);
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            config_path,
            agent_status: Arc::new(Mutex::new(AgentStatusCache::default())),
            preferences_path,
            preferences: Arc::new(Mutex::new(preferences)),
            pairing_session: Arc::new(Mutex::new(None)),
        })
        .setup(|app| {
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

            if let Some(window) = app.get_webview_window("main") {
                window.set_title(&format!("EdgeMouse {}", env!("CARGO_PKG_VERSION")))?;
                #[cfg(target_os = "macos")]
                window.set_decorations(true)?;
            }
            #[cfg(target_os = "macos")]
            install_macos_menu(app.handle(), "zh-CN")?;

            let show_item =
                MenuItem::with_id(app, "tray_show", "打开 EdgeMouse", true, None::<&str>)?;
            let quit_item =
                MenuItem::with_id(app, "tray_quit", "退出 EdgeMouse", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("EdgeMouse")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "tray_show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "tray_quit" => {
                        let _ = stop_agent();
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                        && let Some(window) = tray.app_handle().get_webview_window("main")
                    {
                        let _ = window.unminimize();
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // A packaged EdgeMouse app owns the background agent. Once the two
            // devices have been paired successfully, launching the desktop app
            // should restore the connection without requiring a separate script
            // or a second manual click.
            let config_path = app.state::<AppState>().config_path.clone();
            if let Some(config_path) = config_path
                && LoadedConfig::load(&config_path).is_ok()
            {
                thread::spawn(move || {
                    if let Err(error) = start_agent(&config_path) {
                        eprintln!("EdgeMouse could not start its background service: {error}");
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let background = window
                    .state::<AppState>()
                    .preferences
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .background;
                api.prevent_close();
                if background {
                    let _ = window.hide();
                } else {
                    let app = window.app_handle().clone();
                    thread::spawn(move || {
                        let _ = stop_agent();
                        app.exit(0);
                    });
                }
            }
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
            check_for_updates,
            save_scroll_settings,
            save_input_settings,
            save_connection_settings,
            reconnect_agent,
            get_desktop_preferences,
            save_desktop_preferences,
            show_system_notification,
            reset_preferences,
            run_diagnostics,
            open_logs_folder,
            export_diagnostics,
            verify_trusted_device,
            start_pairing_host,
            get_pairing_status,
            join_pairing,
            cancel_pairing,
            forget_trusted_device,
            open_external_url,
            save_layout,
            set_agent_running,
            import_existing_pairing,
            set_menu_language,
            window_action
        ])
        .run(tauri::generate_context!())
        .expect("failed to run EdgeMouse desktop application");
}

#[cfg(test)]
mod tests {
    use super::{
        AgentStatusCache, config_requires_pairing, local_layout_edge, missed_agent_snapshot,
        parse_layout_edge, scroll_settings_source,
    };
    use edgemouse_agent::config::initialize_unpaired_config;
    use edgemouse_agent::control::{ConnectionTelemetry, RunningStatus};
    use edgemouse_core::Edge;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn fresh_packaged_config_is_reported_as_requiring_pairing() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "edgemouse-desktop-unpaired-{}-{unique}",
            std::process::id()
        ));
        let path = directory.join("edgemouse.toml");
        initialize_unpaired_config(&path).unwrap();

        assert!(config_requires_pairing(&path));

        std::fs::remove_dir_all(directory).unwrap();
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
