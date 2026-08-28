use crate::{KeyboardEvent, PhysicalMouseEvent, Point, RemoteMouseEvent};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
    NotRequired,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaptureMode {
    Local { restore: Option<Point> },
    Remote { anchor: Point },
    ReceivingRemote { position: Point },
}

/// OS input capture. Implementations must keep callbacks non-blocking.
pub trait MouseCaptureBackend {
    fn permission_state(&self) -> PermissionState;
    fn set_mode(&mut self, mode: CaptureMode) -> Result<(), PlatformError>;
    fn try_next_event(&mut self) -> Result<Option<PhysicalMouseEvent>, PlatformError>;
}

/// OS input injection. Implementations must ignore events marked as self-generated.
pub trait MouseInjectionBackend {
    fn permission_state(&self) -> PermissionState;
    fn inject(&mut self, event: RemoteMouseEvent) -> Result<(), PlatformError>;

    /// Advances any platform-specific, low-latency movement pacing.
    ///
    /// Most backends inject movement immediately and need no polling. A backend
    /// may override this to render a buffered absolute position on a stable
    /// cadence without delaying buttons, wheels, or keyboard input.
    fn poll(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), PlatformError>;
}

/// OS keyboard capture. Implementations fail open if their non-blocking queue
/// cannot keep up, so EdgeMouse can never permanently trap local input.
pub trait KeyboardCaptureBackend {
    fn permission_state(&self) -> PermissionState;
    fn set_remote(&mut self, remote: bool) -> Result<(), PlatformError>;
    fn try_next_event(&mut self) -> Result<Option<KeyboardEvent>, PlatformError>;

    fn take_emergency_release(&self) -> bool {
        false
    }
}

pub trait KeyboardInjectionBackend {
    fn permission_state(&self) -> PermissionState;
    fn inject(&mut self, event: KeyboardEvent) -> Result<(), PlatformError>;
    fn release_all(&mut self) -> Result<(), PlatformError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformError {
    message: String,
}

impl PlatformError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for PlatformError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PlatformError {}
