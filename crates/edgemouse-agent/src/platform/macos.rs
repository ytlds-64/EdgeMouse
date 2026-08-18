use super::PlatformStatus;

pub fn status() -> PlatformStatus {
    let trusted = edgemouse_platform_macos::accessibility_trusted();
    PlatformStatus {
        operating_system: "macOS",
        capture_api: "CGEventTap (implemented)",
        injection_api: "CGEventPost (implemented)",
        permission_granted: Some(trusted),
    }
}
