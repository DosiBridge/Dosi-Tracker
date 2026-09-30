use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Name of the optional config file.
const CONFIG_FILE: &str = "config.toml";

/// Runtime configuration for the tracker agent.
///
/// Loaded from `config.toml` next to the executable (falling back to the
/// working directory) and/or environment variables prefixed with `DOSI__`.
/// Any key left out keeps its default, so a config file only needs the values
/// it changes. Tracking permissions are normally overridden by the
/// per-project settings returned from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Base URL of the Dosi-Tracker backend API.
    pub api_base_url: String,

    /// How often a tracking snapshot is taken, in minutes (5..=60).
    /// A longer interval means the agent sleeps more -> lower CPU/battery use.
    pub interval_minutes: u64,

    /// Per-project capture permissions (defaults; server config wins).
    pub capture: CapturePermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CapturePermissions {
    pub screenshot: bool,
    pub webcam: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
}

impl Default for CapturePermissions {
    fn default() -> Self {
        Self {
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: "https://localhost:6006".to_string(), // matches HttpApi.Host App:SelfUrl
            interval_minutes: 10,
            capture: CapturePermissions::default(),
        }
    }
}

impl AppConfig {
    /// Load config from `config.toml` (optional) and `DOSI__*` env vars.
    ///
    /// The file is looked up next to the executable first — the agent
    /// autostarts with an arbitrary working directory, so a CWD-relative lookup
    /// would silently miss the installed config — and only then in the working
    /// directory (handy for `cargo run`).
    pub fn load() -> anyhow::Result<Self> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let cwd = std::env::current_dir().ok();
        let candidates: Vec<PathBuf> = [exe_dir, cwd]
            .into_iter()
            .flatten()
            .map(|dir| dir.join(CONFIG_FILE))
            .collect();

        let mut builder = config::Config::builder();
        match find_config_file(&candidates) {
            Some(path) => {
                tracing::info!(path = %path.display(), "loading config file");
                builder = builder.add_source(config::File::from(path));
            }
            None => tracing::info!("no config.toml found; using defaults"),
        }
        let builder =
            builder.add_source(config::Environment::with_prefix("DOSI").separator("__"));

        Ok(Self::from_config(builder.build()?))
    }

    /// Deserialize over the defaults. A value of the wrong type is reported
    /// loudly instead of silently discarding the whole file.
    fn from_config(cfg: config::Config) -> Self {
        let app = cfg.try_deserialize().unwrap_or_else(|e| {
            tracing::warn!(%e, "invalid configuration; using defaults");
            AppConfig::default()
        });
        Self::normalized(app)
    }

    fn normalized(mut self) -> Self {
        self.interval_minutes = self.interval_minutes.clamp(5, 60);
        self
    }

    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.interval_minutes * 60)
    }
}

/// The first candidate that exists, in priority order.
fn find_config_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_toml(toml: &str) -> AppConfig {
        let cfg = config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .unwrap();
        AppConfig::from_config(cfg)
    }

    #[test]
    fn empty_config_is_all_defaults() {
        let cfg = from_toml("");
        assert_eq!(cfg.api_base_url, AppConfig::default().api_base_url);
        assert_eq!(cfg.interval_minutes, 10);
        assert!(cfg.capture.screenshot && !cfg.capture.webcam);
    }

    #[test]
    fn partial_config_only_overrides_what_it_sets() {
        // Previously a single missing key made the whole file fall back to defaults.
        let cfg = from_toml("api_base_url = \"https://tracker.example.com\"");
        assert_eq!(cfg.api_base_url, "https://tracker.example.com");
        assert_eq!(cfg.interval_minutes, 10);
        assert!(cfg.capture.keyboard);
    }

    #[test]
    fn partial_capture_table_keeps_other_permissions() {
        let cfg = from_toml("interval_minutes = 15\n[capture]\nwebcam = true\nscreenshot = false\n");
        assert_eq!(cfg.interval_minutes, 15);
        assert!(cfg.capture.webcam);
        assert!(!cfg.capture.screenshot);
        // Untouched keys keep their defaults.
        assert!(cfg.capture.keyboard && cfg.capture.mouse);
        assert!(cfg.capture.active_window && cfg.capture.running_programs);
        assert_eq!(cfg.api_base_url, AppConfig::default().api_base_url);
    }

    #[test]
    fn interval_is_clamped() {
        assert_eq!(from_toml("interval_minutes = 1").interval_minutes, 5);
        assert_eq!(from_toml("interval_minutes = 600").interval_minutes, 60);
    }

    #[test]
    fn wrong_types_fall_back_to_defaults() {
        let cfg = from_toml("interval_minutes = \"ten\"");
        assert_eq!(cfg.interval_minutes, 10);
    }

    #[test]
    fn config_next_to_the_executable_wins_over_the_working_directory() {
        let root = std::env::temp_dir().join(format!("dosi-config-{}", uuid::Uuid::new_v4()));
        let exe_dir = root.join("exe");
        let cwd = root.join("cwd");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let candidates = [exe_dir.join(CONFIG_FILE), cwd.join(CONFIG_FILE)];

        assert_eq!(find_config_file(&candidates), None);

        std::fs::write(cwd.join(CONFIG_FILE), "").unwrap();
        assert_eq!(find_config_file(&candidates), Some(cwd.join(CONFIG_FILE)));

        std::fs::write(exe_dir.join(CONFIG_FILE), "").unwrap();
        assert_eq!(find_config_file(&candidates), Some(exe_dir.join(CONFIG_FILE)));

        let _ = std::fs::remove_dir_all(&root);
    }
}
