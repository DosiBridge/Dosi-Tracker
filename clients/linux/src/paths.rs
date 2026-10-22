//! Safe XDG storage and configuration path resolution for Dosi-Tracker Linux.
//!
//! Enforces strict non-mutating inspection:
//! - Resolves standard XDG Base Directory Specification paths.
//! - Inspects directory and file existence without creating directories or files.
//! - Purely side-effect-free during `doctor`, `--help`, and default invocations.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Resolved XDG paths for the Linux tracker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxPaths {
    /// Configuration directory (`$XDG_CONFIG_HOME/dosi-tracker` or `~/.config/dosi-tracker`).
    pub config_dir: PathBuf,
    /// Data directory (`$XDG_DATA_HOME/dosi-tracker` or `~/.local/share/dosi-tracker`).
    pub data_dir: PathBuf,
    /// State directory (`$XDG_STATE_HOME/dosi-tracker` or `~/.local/state/dosi-tracker`).
    pub state_dir: PathBuf,
    /// Local user consent persistence file (`<config_dir>/consent.json`).
    pub consent_file: PathBuf,
}

impl LinuxPaths {
    /// Resolve paths using an explicitly provided home directory and optional XDG environment variables.
    /// This enables deterministic, side-effect-free testing without touching the active user profile.
    pub fn resolve(
        home_dir: Option<&Path>,
        xdg_config_home: Option<&Path>,
        xdg_data_home: Option<&Path>,
        xdg_state_home: Option<&Path>,
    ) -> Self {
        let fallback_home = home_dir.unwrap_or_else(|| Path::new("/home/unknown"));

        let config_dir = xdg_config_home
            .map(|p| p.join("dosi-tracker"))
            .unwrap_or_else(|| fallback_home.join(".config").join("dosi-tracker"));

        let data_dir = xdg_data_home
            .map(|p| p.join("dosi-tracker"))
            .unwrap_or_else(|| {
                fallback_home
                    .join(".local")
                    .join("share")
                    .join("dosi-tracker")
            });

        let state_dir = xdg_state_home
            .map(|p| p.join("dosi-tracker"))
            .unwrap_or_else(|| {
                fallback_home
                    .join(".local")
                    .join("state")
                    .join("dosi-tracker")
            });

        let consent_file = config_dir.join("consent.json");

        Self {
            config_dir,
            data_dir,
            state_dir,
            consent_file,
        }
    }

    /// Resolve paths from current process environment variables.
    /// Pure inspection: does NOT create any directories or files.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let xdg_config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let xdg_data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        let xdg_state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);

        Self::resolve(
            home.as_deref(),
            xdg_config.as_deref(),
            xdg_data.as_deref(),
            xdg_state.as_deref(),
        )
    }

    /// Check if configuration directory exists on disk without creating it.
    pub fn config_dir_exists(&self) -> bool {
        self.config_dir.is_dir()
    }

    /// Check if data directory exists on disk without creating it.
    pub fn data_dir_exists(&self) -> bool {
        self.data_dir.is_dir()
    }

    /// Check if consent file exists on disk without creating it.
    pub fn consent_file_exists(&self) -> bool {
        self.consent_file.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_fallback_paths_from_home() {
        let home = Path::new("/home/worker");
        let paths = LinuxPaths::resolve(Some(home), None, None, None);

        assert_eq!(
            paths.config_dir,
            PathBuf::from("/home/worker/.config/dosi-tracker")
        );
        assert_eq!(
            paths.data_dir,
            PathBuf::from("/home/worker/.local/share/dosi-tracker")
        );
        assert_eq!(
            paths.state_dir,
            PathBuf::from("/home/worker/.local/state/dosi-tracker")
        );
        assert_eq!(
            paths.consent_file,
            PathBuf::from("/home/worker/.config/dosi-tracker/consent.json")
        );
    }

    #[test]
    fn respects_explicit_xdg_overrides() {
        let home = Path::new("/home/worker");
        let custom_config = Path::new("/custom/config");
        let custom_data = Path::new("/custom/data");
        let custom_state = Path::new("/custom/state");

        let paths = LinuxPaths::resolve(
            Some(home),
            Some(custom_config),
            Some(custom_data),
            Some(custom_state),
        );

        assert_eq!(
            paths.config_dir,
            PathBuf::from("/custom/config/dosi-tracker")
        );
        assert_eq!(paths.data_dir, PathBuf::from("/custom/data/dosi-tracker"));
        assert_eq!(paths.state_dir, PathBuf::from("/custom/state/dosi-tracker"));
        assert_eq!(
            paths.consent_file,
            PathBuf::from("/custom/config/dosi-tracker/consent.json")
        );
    }

    #[test]
    fn resolution_creates_zero_directories_or_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home_dummy");
        // Home does not exist yet
        assert!(!home.exists());

        let paths = LinuxPaths::resolve(Some(&home), None, None, None);

        // Path resolution must NOT create home or subdirectories
        assert!(!home.exists());
        assert!(!paths.config_dir.exists());
        assert!(!paths.data_dir.exists());
        assert!(!paths.state_dir.exists());
        assert!(!paths.consent_file.exists());
        assert!(!paths.config_dir_exists());
        assert!(!paths.consent_file_exists());
    }
}
