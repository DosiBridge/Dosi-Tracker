//! The user's own preferences for the desktop app (how the window behaves,
//! notifications, their daily goal), kept in
//! `%APPDATA%\DosiTracker\preferences.json`.
//!
//! These are personal and local. What is *captured* is never set here — that
//! is the workspace admin's per-project policy, enforced by the server.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "preferences.json";

/// Bounds of the daily goal, in hours.
pub const GOAL_MIN_HOURS: f32 = 1.0;
pub const GOAL_MAX_HOURS: f32 = 16.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// When Windows starts the app at sign-in, keep it in the tray instead of
    /// opening the window (only once a sign-in is saved).
    pub start_minimized: bool,
    /// Minimizing hides the window to the tray rather than the taskbar.
    pub minimize_to_tray: bool,
    /// Tell the user from the tray, while the window is not in front: the
    /// time-away question, tracking that started by itself, a sign-in that
    /// expired. (The window shows them either way.)
    pub show_notifications: bool,
    /// Hours a day the user aims to track, for the progress bars.
    pub daily_goal_hours: f32,
    /// Working days a week; the weekly goal is the daily goal times this.
    pub work_days_per_week: u8,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            start_minimized: true,
            minimize_to_tray: true,
            show_notifications: true,
            daily_goal_hours: 8.0,
            work_days_per_week: 5,
        }
    }
}

impl Preferences {
    /// The saved preferences, or the defaults when there are none (or the file
    /// is unreadable — preferences must never stop the tracker from starting).
    pub fn load() -> Self {
        path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = path().ok_or_else(|| anyhow::anyhow!("could not resolve the user config directory"))?;
        self.save_to(&path)
    }

    fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
            .normalized()
    }

    /// Write-then-rename: a crash mid-save never leaves a truncated file
    /// (which would silently reset every preference).
    fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        crate::fsutil::write_atomically(path, &serde_json::to_vec_pretty(&self.clone().normalized())?)
    }

    /// Clamp values a hand-edited file could put out of range: the goal to
    /// half-hour steps within bounds, the week to 1–7 days.
    pub fn normalized(mut self) -> Self {
        let goal = if self.daily_goal_hours.is_finite() { self.daily_goal_hours } else { 8.0 };
        self.daily_goal_hours = ((goal * 2.0).round() / 2.0).clamp(GOAL_MIN_HOURS, GOAL_MAX_HOURS);
        self.work_days_per_week = self.work_days_per_week.clamp(1, 7);
        self
    }

    pub fn daily_goal_secs(&self) -> f64 {
        f64::from(self.daily_goal_hours) * 3600.0
    }

    pub fn weekly_goal_secs(&self) -> f64 {
        self.daily_goal_secs() * f64::from(self.work_days_per_week)
    }
}

fn path() -> Option<PathBuf> {
    crate::shell::data_dir().map(|d| d.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!("dosi-prefs-{}", uuid::Uuid::new_v4()))
            .join(FILE_NAME)
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        assert_eq!(Preferences::load_from(&temp_file()), Preferences::default());
    }

    #[test]
    fn saved_preferences_round_trip() {
        let path = temp_file();
        let prefs = Preferences {
            start_minimized: false,
            minimize_to_tray: false,
            show_notifications: false,
            daily_goal_hours: 6.5,
            work_days_per_week: 4,
        };
        prefs.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path), prefs);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_partial_or_corrupt_file_keeps_the_other_defaults() {
        let path = temp_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"daily_goal_hours": 7}"#).unwrap();
        let prefs = Preferences::load_from(&path);
        assert_eq!(prefs.daily_goal_hours, 7.0);
        assert!(prefs.start_minimized && prefs.minimize_to_tray);
        // A file saved before notifications were a choice keeps them on.
        assert!(prefs.show_notifications);

        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let prefs = Preferences { daily_goal_hours: 40.3, work_days_per_week: 9, ..Default::default() }.normalized();
        assert_eq!(prefs.daily_goal_hours, GOAL_MAX_HOURS);
        assert_eq!(prefs.work_days_per_week, 7);

        let prefs = Preferences { daily_goal_hours: 7.3, work_days_per_week: 0, ..Default::default() }.normalized();
        assert_eq!(prefs.daily_goal_hours, 7.5);
        assert_eq!(prefs.work_days_per_week, 1);

        let prefs = Preferences { daily_goal_hours: f32::NAN, ..Default::default() }.normalized();
        assert_eq!(prefs.daily_goal_hours, 8.0);
        assert_eq!(prefs.weekly_goal_secs(), 8.0 * 3600.0 * 5.0);
    }
}
