//! IPC protocol v1 between the daemon and its clients (CLI, GUI): newline-
//! delimited JSON over a same-uid Unix socket; envelope, methods, error codes
//! and the state snapshot.

#[cfg(target_os = "linux")]
pub mod client;
#[cfg(target_os = "linux")]
pub mod server;
