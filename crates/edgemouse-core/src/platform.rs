use crate::{PhysicalMouseEvent, Point, RemoteMouseEvent};
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
