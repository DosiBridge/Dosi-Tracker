//! zbus fakes of the desktop services the agent talks to, served on private
//! buses from [`super::private_bus`]. The zbus-backed ones are Linux-only,
//! like zbus itself.

pub mod login1;
#[cfg(target_os = "linux")]
pub mod mutter_idle;
pub mod notifications;
#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(target_os = "linux")]
pub mod screensaver;
pub mod sni_watcher;
pub mod systemd_shim;
