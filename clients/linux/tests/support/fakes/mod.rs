//! zbus fakes of the desktop services the agent talks to, served on private
//! buses from [`super::private_bus`], and a recording `systemctl`. The
//! zbus-backed and Unix-only ones are Linux-only, like zbus itself.

#[cfg(target_os = "linux")]
pub mod login1;
#[cfg(target_os = "linux")]
pub mod mutter_idle;
#[cfg(target_os = "linux")]
pub mod notifications;
#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(target_os = "linux")]
pub mod screensaver;
#[cfg(target_os = "linux")]
pub mod sni_watcher;
#[cfg(target_os = "linux")]
pub mod systemd_shim;
