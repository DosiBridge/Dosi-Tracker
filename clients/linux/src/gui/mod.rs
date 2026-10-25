//! `dosi-tracker-gui` (feature `gui`): eframe/egui thin client of the daemon,
//! ported from the Windows agent's UI.

pub mod activity;
pub mod alerts;
pub mod consent;
pub mod format;
pub mod link;
pub mod login;
#[cfg(feature = "test-support")]
pub mod preview;
pub mod settings;
pub mod theme;
pub mod timer;
pub mod widgets;
