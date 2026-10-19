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

    /// The web dashboard, opened by "Open web dashboard" in the window and tray.
    pub dashboard_url: String,

    /// How often a tracking snapshot is taken, in minutes (5..=60).
    /// A longer interval means the agent sleeps more -> lower CPU/battery use.
    pub interval_minutes: u64,

    /// Per-project capture permissions (defaults; server config wins).
    pub capture: CapturePermissions,

    /// Explicit affirmative local consent from the interactive user on this device.
    /// Invariant: Defaults to `LocalConsent::Absent` to fail closed until an explicit
    /// user consent dialog slice is completed. Defaults/server policy never fabricate consent.
    pub consent: LocalConsent,
}

/// Explicit affirmative local consent from the interactive user on this device.
///
/// Invariant: Configuration defaults or server-side project policies do NOT
/// constitute local user consent. Captures and sensing occur only if:
///   Effective = ProjectPolicy ∩ LocalConsent
///
/// Until a reviewed consent UI slice is completed, production runtime defaults
/// to `LocalConsent::Absent`, causing all sensor collection to fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalConsent {
    /// No affirmative consent has been granted by the local user yet.
    Absent,
    /// Explicit affirmative consent granted per capability by the user.
    Granted(CapturePermissions),
    /// Explicitly revoked by the user.
    Revoked,
}

impl Default for LocalConsent {
    fn default() -> Self {
        LocalConsent::Absent
    }
}

impl LocalConsent {
    pub const FILE_NAME: &str = "consent.json";

    pub fn data_path() -> Option<PathBuf> {
        crate::shell::data_dir().map(|d| d.join(Self::FILE_NAME))
    }

    pub fn load() -> Self {
        Self::data_path().map(|p| Self::load_from(&p)).unwrap_or(LocalConsent::Absent)
    }

    pub fn load_from(path: &Path) -> Self {
        if !path.is_file() {
            return LocalConsent::Absent;
        }
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<LocalConsent>(&bytes).unwrap_or_else(|e| {
                tracing::warn!(?e, path = %path.display(), "corrupted or invalid consent file; failing closed to Absent");
                LocalConsent::Absent
            }),
            Err(e) => {
                tracing::warn!(?e, path = %path.display(), "could not read consent file; failing closed to Absent");
                LocalConsent::Absent
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::data_path().ok_or_else(|| anyhow::anyhow!("could not resolve user data directory"))?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn is_granted(&self) -> bool {
        matches!(self, LocalConsent::Granted(_))
    }

    /// Compute effective permissions given server policy, local configuration, and this consent.
    /// Fails closed: if consent is Absent or Revoked, all permissions are false.
    pub fn effective_permissions(&self, project: &crate::model::Project, local_cfg: &CapturePermissions) -> CapturePermissions {
        match self {
            LocalConsent::Granted(consent) => CapturePermissions {
                screenshot: project.allow_screenshot && local_cfg.screenshot && consent.screenshot,
                webcam: project.allow_webcam && local_cfg.webcam && consent.webcam,
                keyboard: project.allow_keyboard && local_cfg.keyboard && consent.keyboard,
                mouse: project.allow_mouse && local_cfg.mouse && consent.mouse,
                active_window: project.allow_active_window && local_cfg.active_window && consent.active_window,
                running_programs: project.allow_running_programs && local_cfg.running_programs && consent.running_programs,
                blur_screenshots: project.blur_screenshots || local_cfg.blur_screenshots || consent.blur_screenshots,
            },
            LocalConsent::Absent | LocalConsent::Revoked => CapturePermissions {
                screenshot: false,
                webcam: false,
                keyboard: false,
                mouse: false,
                active_window: false,
                running_programs: false,
                blur_screenshots: true,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CapturePermissions {
    pub screenshot: bool,
    pub webcam: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
    /// Blur screen captures before upload. The effective value is the
    /// project's policy OR this, so a local config can add blur but never
    /// remove it.
    pub blur_screenshots: bool,
}

impl CapturePermissions {
    /// Default-off configuration for initial explicit consent presentation.
    pub fn all_disabled() -> Self {
        Self {
            screenshot: false,
            webcam: false,
            keyboard: false,
            mouse: false,
            active_window: false,
            running_programs: false,
            blur_screenshots: true,
        }
    }
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
            blur_screenshots: false,
        }
    }
}

/// Why a server address cannot be used, worded for the sign-in form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerUrlError {
    Empty,
    /// Only `http://` and `https://` are supported.
    UnsupportedScheme(String),
    Invalid,
}

impl std::fmt::Display for ServerUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerUrlError::Empty => write!(f, "Enter the server address."),
            ServerUrlError::UnsupportedScheme(scheme) => {
                write!(f, "The server address must start with https:// or http:// (not {scheme}://).")
            }
            ServerUrlError::Invalid => write!(f, "That is not a valid server address."),
        }
    }
}

impl std::error::Error for ServerUrlError {}

/// Normalize a server (API) address typed at sign-in or written in the
/// config: surrounding whitespace and trailing slashes go (pages are appended
/// as "/path"), a bare host gets `https://`, and only `http(s)` URLs with a
/// host and no credentials, query or fragment are accepted. A path prefix
/// (a server behind a reverse proxy at `/tracker`) is kept.
pub fn normalize_server_url(input: &str) -> Result<String, ServerUrlError> {
    let trimmed = input.trim();
    if trimmed.trim_end_matches('/').is_empty() {
        return Err(ServerUrlError::Empty);
    }
    // "localhost:6006" would otherwise parse as scheme "localhost".
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = reqwest::Url::parse(&with_scheme).map_err(|_| ServerUrlError::Invalid)?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(ServerUrlError::UnsupportedScheme(scheme.to_string()));
    }
    let host = url.host_str().filter(|h| !h.is_empty()).ok_or(ServerUrlError::Invalid)?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(ServerUrlError::Invalid);
    }
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    Ok(format!("{scheme}://{host}{port}{path}"))
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: "https://localhost:6006".to_string(), // matches HttpApi.Host App:SelfUrl
            dashboard_url: "http://localhost:6005".to_string(), // the Next.js frontend (`npm run dev`)
            interval_minutes: 10,
            capture: CapturePermissions::default(),
            consent: LocalConsent::Absent,
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
        let mut app_config = Self::from_config(builder.build()?);
        let persisted_consent = LocalConsent::load();
        if persisted_consent != LocalConsent::Absent {
            app_config.consent = persisted_consent;
        }
        Ok(app_config)
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
        // Pages are appended as "/path", so no trailing slash.
        let trimmed = self.dashboard_url.trim().trim_end_matches('/');
        self.dashboard_url = trimmed.to_string();
        // A trailing slash would make every endpoint "//connect/token" (404,
        // reported as "cannot reach the server"); a typo must not go unnoticed.
        self.api_base_url = match normalize_server_url(&self.api_base_url) {
            Ok(url) => url,
            Err(e) => {
                let fallback = AppConfig::default().api_base_url;
                tracing::warn!(url = %self.api_base_url, %e, %fallback, "invalid api_base_url; using the default");
                fallback
            }
        };
        self
    }

    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.interval_minutes * 60)
    }

    /// The server to talk to: the address saved with the sign-in (typed on
    /// the sign-in screen) wins over `config.toml` / `DOSI__API_BASE_URL`, as
    /// long as it is a valid address.
    #[cfg_attr(not(test), expect(dead_code, reason = "the server address saved at sign-in"))]
    pub fn effective_api_base_url(&self, saved_server: Option<&str>) -> String {
        match saved_server.map(normalize_server_url) {
            Some(Ok(url)) => url,
            Some(Err(e)) => {
                tracing::warn!(%e, "the saved server address is invalid; using the configured one");
                self.api_base_url.clone()
            }
            None => self.api_base_url.clone(),
        }
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
    fn dashboard_url_defaults_and_trims_a_trailing_slash() {
        assert_eq!(from_toml("").dashboard_url, "http://localhost:6005");
        let cfg = from_toml("dashboard_url = \"https://app.example.com/ \"");
        assert_eq!(cfg.dashboard_url, "https://app.example.com");
    }

    #[test]
    fn api_base_url_is_normalized_and_validated() {
        let cfg = from_toml("api_base_url = \" https://tracker.example.com// \"");
        assert_eq!(cfg.api_base_url, "https://tracker.example.com");
        // Not a URL the agent can use: the default, not a broken one.
        let cfg = from_toml("api_base_url = \"ftp://tracker.example.com\"");
        assert_eq!(cfg.api_base_url, AppConfig::default().api_base_url);
        assert!(!cfg.capture.blur_screenshots);
    }

    #[test]
    fn server_urls_are_normalized() {
        let ok = |input: &str| normalize_server_url(input).unwrap();
        assert_eq!(ok("https://tracker.example.com/"), "https://tracker.example.com");
        assert_eq!(ok("  HTTPS://Tracker.Example.com:8443/// "), "https://tracker.example.com:8443");
        assert_eq!(ok("http://10.0.0.5:6006"), "http://10.0.0.5:6006");
        // A bare host (what people type) is https.
        assert_eq!(ok("tracker.example.com"), "https://tracker.example.com");
        assert_eq!(ok("localhost:6006"), "https://localhost:6006");
        // A reverse-proxy path prefix is kept; the default port is dropped.
        assert_eq!(ok("https://example.com:443/tracker/"), "https://example.com/tracker");
        assert_eq!(ok("http://[::1]:6006/"), "http://[::1]:6006");
    }

    #[test]
    fn unusable_server_urls_are_refused() {
        assert_eq!(normalize_server_url(""), Err(ServerUrlError::Empty));
        assert_eq!(normalize_server_url(" / "), Err(ServerUrlError::Empty));
        assert_eq!(
            normalize_server_url("ftp://example.com"),
            Err(ServerUrlError::UnsupportedScheme("ftp".into()))
        );
        for bad in ["https://", "https://exa mple.com", "https://user:pw@example.com", "https://example.com/?a=1", "https://example.com/#x"] {
            assert_eq!(normalize_server_url(bad), Err(ServerUrlError::Invalid), "{bad}");
        }
        // The messages are meant for the sign-in form.
        assert!(ServerUrlError::UnsupportedScheme("ftp".into()).to_string().contains("https://"));
    }

    #[test]
    fn a_server_saved_at_sign_in_overrides_the_config() {
        let cfg = from_toml("api_base_url = \"https://configured.example.com\"");
        assert_eq!(cfg.effective_api_base_url(None), "https://configured.example.com");
        assert_eq!(cfg.effective_api_base_url(Some("saved.example.com/")), "https://saved.example.com");
        // A damaged saved value never breaks the agent.
        assert_eq!(cfg.effective_api_base_url(Some("ftp://saved")), "https://configured.example.com");
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
