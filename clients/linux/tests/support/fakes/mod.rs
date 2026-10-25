//! zbus fakes of the desktop services the agent talks to, served on private
//! buses from [`super::private_bus`].

pub mod login1;
pub mod mutter_idle;
pub mod notifications;
pub mod portal;
pub mod screensaver;
pub mod sni_watcher;
pub mod systemd_shim;
