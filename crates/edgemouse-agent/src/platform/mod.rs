#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug, Clone, Copy)]
pub struct PlatformStatus {
    pub operating_system: &'static str,
    pub capture_api: &'static str,
    pub injection_api: &'static str,
    pub permission_granted: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub struct DesktopGeometry {
    pub bounds: edgemouse_core::Rect,
    pub scale_factor: f64,
    pub display_count: u32,
}

pub fn desktop_geometry() -> Result<DesktopGeometry, edgemouse_core::PlatformError> {
    #[cfg(target_os = "macos")]
    let (bounds, scale_factor, display_count) = edgemouse_platform_macos::desktop_geometry()?;
    #[cfg(target_os = "windows")]
    let (bounds, scale_factor, display_count) = edgemouse_platform_windows::desktop_geometry()?;

    Ok(DesktopGeometry {
        bounds,
        scale_factor,
        display_count,
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
            capture_api: "WH_MOUSE_LL (implemented; Raw Input optimization pending)",
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
pub type NativeKeyboardInjector = edgemouse_platform_macos::MacKeyboardInjector;

#[cfg(target_os = "windows")]
pub type NativeMouseCapture = edgemouse_platform_windows::WindowsMouseCapture;
#[cfg(target_os = "windows")]
pub type NativeMouseInjector = edgemouse_platform_windows::WindowsMouseInjector;
#[cfg(target_os = "windows")]
pub type NativeKeyboardCapture = edgemouse_platform_windows::WindowsKeyboardCapture;
#[cfg(target_os = "windows")]
pub type NativeKeyboardInjector = edgemouse_platform_windows::WindowsKeyboardInjector;

/// Keyboard capture on macOS is intentionally inactive in the first keyboard
/// MVP. Windows-to-Mac control is enabled without changing the proven macOS
/// mouse event tap; the reverse direction can be added independently.
#[cfg(target_os = "macos")]
pub struct NativeKeyboardCapture;

#[cfg(target_os = "macos")]
impl edgemouse_core::KeyboardCaptureBackend for NativeKeyboardCapture {
    fn permission_state(&self) -> edgemouse_core::PermissionState {
        edgemouse_core::PermissionState::Granted
    }

    fn set_remote(&mut self, _remote: bool) -> Result<(), edgemouse_core::PlatformError> {
        Ok(())
    }

    fn try_next_event(
        &mut self,
    ) -> Result<Option<edgemouse_core::KeyboardEvent>, edgemouse_core::PlatformError> {
        Ok(None)
    }
}

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
) -> Result<NativeMouseCapture, edgemouse_core::PlatformError> {
    NativeMouseCapture::start()
}

#[cfg(target_os = "windows")]
pub fn start_capture(
    local_bounds: edgemouse_core::Rect,
    coordinate_scale: f64,
    initial_pointer: edgemouse_core::Point,
) -> Result<NativeMouseCapture, edgemouse_core::PlatformError> {
    let capture_anchor = edgemouse_core::Point::new(
        local_bounds.origin.x + local_bounds.width / 2.0,
        local_bounds.origin.y + local_bounds.height / 2.0,
    );
    NativeMouseCapture::start(coordinate_scale, capture_anchor, initial_pointer)
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn injector(initial: edgemouse_core::Point) -> NativeMouseInjector {
    NativeMouseInjector::new(initial)
}

#[cfg(target_os = "macos")]
pub fn start_keyboard_capture() -> Result<NativeKeyboardCapture, edgemouse_core::PlatformError> {
    Ok(NativeKeyboardCapture)
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
pub fn injector(initial: edgemouse_core::Point) -> NativeMouseInjector {
    NativeMouseInjector::new(initial)
}
