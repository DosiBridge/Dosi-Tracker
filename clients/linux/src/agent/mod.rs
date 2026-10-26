//! The per-user agent: the daemon's worker, control, uploader, session and
//! notification parts, ported from the Windows agent with Linux deltas.
//!
//! Modules that talk D-Bus or use libc are Linux-only. The tray's view, icon
//! and actions are always built on Linux (the GUI shares them); its ksni
//! service also needs the `tray` feature. `session_store` is portable; its
//! file I/O is Unix-only.
//! The worker, its control task and the uploader drive the Linux-only
//! session events and device context, so they are Linux-only too.

pub mod blocks;
pub mod config;
#[cfg(target_os = "linux")]
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
pub mod session_store;
pub mod state;
#[cfg(target_os = "linux")]
pub mod tray;
#[cfg(target_os = "linux")]
pub mod uploader;
#[cfg(target_os = "linux")]
pub mod worker;
