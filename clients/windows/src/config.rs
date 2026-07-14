use serde::{Deserialize, Serialize};

/// Runtime configuration for the tracker agent.
///
/// Loaded from `config.toml` next to the executable and/or environment
/// variables prefixed with `DOSI_`. Tracking permissions are normally
/// overridden by the per-project settings returned from the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct CapturePermissions {
    pub screenshot: bool,
    pub webcam: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: "https://localhost:44300".to_string(),
            interval_minutes: 10,
            capture: CapturePermissions {
                screenshot: true,
                webcam: false,
                keyboard: true,
                mouse: true,
                active_window: true,
                running_programs: true,
            },
        }
    }
}

impl AppConfig {
    /// Load config from `config.toml` (optional) and `DOSI_*` env vars.
    pub fn load() -> anyhow::Result<Self> {
        let builder = config::Config::builder()
            .add_source(config::File::with_name("config").required(false))
            .add_source(config::Environment::with_prefix("DOSI").separator("__"));

        let cfg = builder.build()?;
        // Fall back to defaults for any missing key.
        let app: AppConfig = cfg.try_deserialize().unwrap_or_default();
        Ok(app.normalized())
    }

    fn normalized(mut self) -> Self {
        self.interval_minutes = self.interval_minutes.clamp(5, 60);
        self
    }

    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.interval_minutes * 60)
    }
}
