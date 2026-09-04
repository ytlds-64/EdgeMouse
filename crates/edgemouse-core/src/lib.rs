//! Platform-neutral domain model for EdgeMouse.
//!
//! This crate deliberately contains no OS or networking calls. Platform
//! adapters feed physical mouse events into [`Session`], then apply the
//! returned [`Effect`] values on their own threads.

#![forbid(unsafe_code)]

mod geometry;
mod input;
mod platform;
mod session;
mod topology;

pub use geometry::{DisplayGeometry, Edge, GeometryError, Point, Rect, Vector};
pub use input::{
    ButtonState, Effect, InputDisposition, InputResult, KeyCode, KeyState, KeyboardEvent,
    MouseButton, PhysicalMouseEvent, RemoteMouseEvent, RoutedEvent, RoutedKeyboardEvent,
};
pub use platform::{
    CaptureMode, KeyboardCaptureBackend, KeyboardInjectionBackend, MouseCaptureBackend,
    MouseInjectionBackend, PermissionState, PlatformError,
};
pub use session::{ControlState, Session, SessionConfig, SessionError};
pub use topology::{
    Advance, NodeId, Portal, Screen, ScreenId, Topology, TopologyError, Transition,
};
