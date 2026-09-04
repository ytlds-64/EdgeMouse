#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug, Clone, Copy)]
pub struct PlatformStatus {
    pub operating_system: &'static str,
    pub capture_api: &'static str,
    pub injection_api: &'static str,
    pub permission_granted: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct DesktopGeometry {
    pub bounds: edgemouse_core::Rect,
    pub scale_factor: f64,
    pub display_count: u32,
    pub displays: Vec<edgemouse_core::DisplayGeometry>,
}

pub fn desktop_geometry() -> Result<DesktopGeometry, edgemouse_core::PlatformError> {
    #[cfg(target_os = "macos")]
    let (bounds, scale_factor, displays) = edgemouse_platform_macos::desktop_geometry()?;
    #[cfg(target_os = "windows")]
    let (bounds, scale_factor, displays) = edgemouse_platform_windows::desktop_geometry()?;

    let display_count = u32::try_from(displays.len())
        .map_err(|_| edgemouse_core::PlatformError::new("display count exceeds supported range"))?;

    Ok(DesktopGeometry {
        bounds,
        scale_factor,
        display_count,
        displays,
    })
}

#[must_use]
pub fn current_status() -> PlatformStatus {
    #[cfg(target_os = "macos")]
    {
        macos::status()
    }

    #[cfg(target_os = "windows")]
    {
        PlatformStatus {
            operating_system: "Windows",
            capture_api: "Raw Input movement with WH_MOUSE_LL safety suppression",
            injection_api: "SendInput (implemented)",
            permission_granted: None,
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PlatformStatus {
            operating_system: std::env::consts::OS,
            capture_api: "unsupported",
            injection_api: "unsupported",
            permission_granted: None,
        }
    }
}

#[cfg(target_os = "macos")]
pub type NativeMouseCapture = edgemouse_platform_macos::MacMouseCapture;
#[cfg(target_os = "macos")]
pub type NativeMouseInjector = edgemouse_platform_macos::MacMouseInjector;
#[cfg(target_os = "macos")]
pub type NativeKeyboardCapture = edgemouse_platform_macos::MacKeyboardCapture;
#[cfg(target_os = "macos")]
pub type NativeKeyboardInjector = edgemouse_platform_macos::MacKeyboardInjector;

#[cfg(target_os = "windows")]
pub type NativeMouseCapture = edgemouse_platform_windows::WindowsMouseCapture;
#[cfg(target_os = "windows")]
pub type NativeMouseInjector = edgemouse_platform_windows::WindowsMouseInjector;
#[cfg(target_os = "windows")]
pub type NativeKeyboardCapture = edgemouse_platform_windows::WindowsKeyboardCapture;
#[cfg(target_os = "windows")]
pub type NativeKeyboardInjector = edgemouse_platform_windows::WindowsKeyboardInjector;

#[cfg(target_os = "macos")]
pub fn current_pointer() -> Result<edgemouse_core::Point, edgemouse_core::PlatformError> {
    edgemouse_platform_macos::current_pointer()
}

#[cfg(target_os = "windows")]
pub fn current_pointer() -> Result<edgemouse_core::Point, edgemouse_core::PlatformError> {
    edgemouse_platform_windows::current_pointer()
}

#[cfg(target_os = "macos")]
pub fn start_capture(
    _local_bounds: edgemouse_core::Rect,
    _coordinate_scale: f64,
    _initial_pointer: edgemouse_core::Point,
    _windows_raw_input: bool,
) -> Result<NativeMouseCapture, edgemouse_core::PlatformError> {
    NativeMouseCapture::start()
}

#[cfg(target_os = "windows")]
pub fn start_capture(
    local_bounds: edgemouse_core::Rect,
    coordinate_scale: f64,
    initial_pointer: edgemouse_core::Point,
    windows_raw_input: bool,
) -> Result<NativeMouseCapture, edgemouse_core::PlatformError> {
    let capture_anchor = edgemouse_core::Point::new(
        local_bounds.origin.x + local_bounds.width / 2.0,
        local_bounds.origin.y + local_bounds.height / 2.0,
    );
    NativeMouseCapture::start(
        coordinate_scale,
        capture_anchor,
        initial_pointer,
        windows_raw_input,
    )
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn injector(initial: edgemouse_core::Point, pointer_smoothing: u8) -> NativeMouseInjector {
    NativeMouseInjector::new_with_smoothing(initial, pointer_smoothing)
}

#[cfg(target_os = "macos")]
pub fn start_keyboard_capture() -> Result<NativeKeyboardCapture, edgemouse_core::PlatformError> {
    NativeKeyboardCapture::start()
}

#[cfg(target_os = "windows")]
pub fn start_keyboard_capture() -> Result<NativeKeyboardCapture, edgemouse_core::PlatformError> {
    NativeKeyboardCapture::start()
}

#[cfg(target_os = "windows")]
pub fn install_shutdown_handler(
    stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), edgemouse_core::PlatformError> {
    edgemouse_platform_windows::install_shutdown_handler(stopping)
}

#[cfg(target_os = "macos")]
#[must_use]
pub const fn keyboard_injector() -> NativeKeyboardInjector {
    NativeKeyboardInjector::new()
}

#[cfg(target_os = "windows")]
#[must_use]
pub const fn keyboard_injector() -> NativeKeyboardInjector {
    NativeKeyboardInjector::new()
}

#[cfg(target_os = "windows")]
#[must_use]
pub fn injector(initial: edgemouse_core::Point, _pointer_smoothing: u8) -> NativeMouseInjector {
    NativeMouseInjector::new(initial)
}
