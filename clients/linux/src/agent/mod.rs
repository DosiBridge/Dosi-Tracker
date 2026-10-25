//! The per-user agent: the daemon's worker, control, uploader, session and
//! notification parts, ported from the Windows agent with Linux deltas.
//!
//! Modules that talk D-Bus or use libc are Linux-only; the tray also needs the
//! `tray` feature.

pub mod blocks;
pub mod config;
pub mod control;
#[cfg(target_os = "linux")]
pub mod daemon;
pub mod model;
#[cfg(target_os = "linux")]
pub mod notify;
pub mod power;
pub mod session;
#[cfg(target_os = "linux")]
pub mod session_events;
#[cfg(target_os = "linux")]
pub mod session_store;
pub mod state;
#[cfg(all(feature = "tray", target_os = "linux"))]
pub mod tray;
pub mod uploader;
pub mod worker;
