#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug, Clone, Copy)]
pub struct PlatformStatus {
    pub operating_system: &'static str,
    pub capture_api: &'static str,
    pub injection_api: &'static str,
    pub permission_granted: Option<bool>,
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

#[cfg(target_os = "windows")]
pub type NativeMouseCapture = edgemouse_platform_windows::WindowsMouseCapture;
#[cfg(target_os = "windows")]
pub type NativeMouseInjector = edgemouse_platform_windows::WindowsMouseInjector;

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
    _coordinate_scale: f64,
) -> Result<NativeMouseCapture, edgemouse_core::PlatformError> {
    NativeMouseCapture::start()
}

#[cfg(target_os = "windows")]
pub fn start_capture(
    coordinate_scale: f64,
) -> Result<NativeMouseCapture, edgemouse_core::PlatformError> {
    NativeMouseCapture::start(coordinate_scale)
}

#[cfg(target_os = "macos")]
#[must_use]
pub fn injector(initial: edgemouse_core::Point) -> NativeMouseInjector {
    NativeMouseInjector::new(initial)
}

#[cfg(target_os = "windows")]
#[must_use]
pub fn injector(initial: edgemouse_core::Point) -> NativeMouseInjector {
    NativeMouseInjector::new(initial)
}
