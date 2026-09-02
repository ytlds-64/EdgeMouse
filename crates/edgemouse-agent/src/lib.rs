//! Reusable EdgeMouse agent services.
//!
//! The command-line binary and the desktop application both consume this
//! library so configuration parsing and local status reporting have a single
//! implementation.

pub mod config;
pub mod control;
pub mod discovery;
pub mod network;
pub mod pairing;
pub mod platform;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod runtime;
