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

    /// Allow a plain `http://` server that is not this computer. Off by
    /// default: the password, tokens, screenshots and window titles would
    /// cross the network unencrypted. Only an admin who knows the network is
    /// trusted turns it on, in `config.toml`; the window then warns that the
    /// connection is not encrypted.
    pub allow_insecure_http: bool,

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalConsent {
    /// No affirmative consent has been granted by the local user yet.
    /// The default: absent consent fails closed.
    #[default]
    Absent,
    /// Explicit affirmative consent granted per capability by the user.
    /// Invariant: ConsentedPermissions uses strict default-deny schema (omitted fields default to false).
    Granted(ConsentedPermissions),
    /// Explicitly revoked by the user.
    Revoked,
}

/// Granular permissions consented to by the interactive user on this device.
///
/// Invariant: Default is ALWAYS strictly all-false (default-deny).
/// When deserializing partial JSON (e.g. `{"Granted": {"keyboard": true}}`),
/// any omitted fields default to `false`. Missing or partial consent NEVER
/// broadens permissions or falls back to permissive config defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsentedPermissions {
    pub screenshot: bool,
    pub webcam: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
    pub blur_screenshots: bool,
}

impl Default for ConsentedPermissions {
    fn default() -> Self {
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

impl ConsentedPermissions {
    pub fn all_disabled() -> Self {
        Self::default()
    }

    /// Calculates the strict intersection of two permission grants.
    /// A capability is only true if it is true in BOTH self and other.
    /// For blur_screenshots, true (blurred) is preserved as defense-in-depth if either requests it.
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            screenshot: self.screenshot && other.screenshot,
            webcam: self.webcam && other.webcam,
            keyboard: self.keyboard && other.keyboard,
            mouse: self.mouse && other.mouse,
            active_window: self.active_window && other.active_window,
            running_programs: self.running_programs && other.running_programs,
            blur_screenshots: self.blur_screenshots || other.blur_screenshots,
        }
    }
}

impl From<CapturePermissions> for ConsentedPermissions {
    fn from(p: CapturePermissions) -> Self {
        Self {
            screenshot: p.screenshot,
            webcam: p.webcam,
            keyboard: p.keyboard,
            mouse: p.mouse,
            active_window: p.active_window,
            running_programs: p.running_programs,
            blur_screenshots: p.blur_screenshots,
        }
    }
}

impl From<ConsentedPermissions> for CapturePermissions {
    fn from(c: ConsentedPermissions) -> Self {
        Self {
            screenshot: c.screenshot,
            webcam: c.webcam,
            keyboard: c.keyboard,
            mouse: c.mouse,
            active_window: c.active_window,
            running_programs: c.running_programs,
            blur_screenshots: c.blur_screenshots,
        }
    }
}

impl LocalConsent {
    pub const FILE_NAME: &str = "consent.json";

    /// Computes the monotonic narrowed consent between the current state and target.
    /// - If both are Granted, returns Granted(old.intersect(new)).
    ///   Any withdrawn permission is immediately false; no expansion is granted yet.
    /// - If self was Absent, returns Absent (fails closed, ungranted).
    /// - If target is Revoked or Absent, returns Revoked (all permissions withdrawn).
    /// - If self was Revoked, returns Revoked (no permissions were active).
    pub fn narrow_to(&self, target: &Self) -> Self {
        match (self, target) {
            (LocalConsent::Granted(old), LocalConsent::Granted(new)) => {
                LocalConsent::Granted(old.intersect(new))
            }
            (LocalConsent::Absent, _) => LocalConsent::Absent,
            _ => LocalConsent::Revoked,
        }
    }

    pub fn data_path() -> Option<PathBuf> {
        crate::shell::data_dir().map(|d| d.join(Self::FILE_NAME))
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::data_path().ok_or_else(|| anyhow::anyhow!("could not resolve user data directory"))?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        let data = serde_json::to_vec_pretty(self)?;
        atomic_write_file(path, &data)
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

/// Write data to an ephemeral temporary sibling file and atomically replace `path`.
///
/// On Windows, `std::fs::rename` uses `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)`,
/// ensuring atomic replacement on the same volume without truncating or leaving
/// partial bytes in `path` if writing or syncing fails.
///
/// If an error occurs during write, sync, or rename, the temporary file is deleted
/// and the original file at `path` is preserved without modification.
pub fn atomic_write_file(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp_name = format!(".tmp-consent-{}.tmp", uuid::Uuid::new_v4());
    let temp_path = parent.join(temp_name);

    let write_res = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(data)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!("failed writing temp consent file: {e}"));
    }

    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!("failed atomically replacing consent file: {e}"));
    }

    Ok(())
}

/// Abstract storage boundary for LocalConsent persistence.
/// Ensures tests never touch the real user's consent file and enables fault injection.
pub trait ConsentStore: Send + Sync {
    fn load(&self) -> LocalConsent;
    fn save(&self, consent: &LocalConsent) -> anyhow::Result<()>;
}

/// Standard production file-based consent store with atomic replacement.
pub struct FileConsentStore {
    path: PathBuf,
}

impl FileConsentStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn production() -> Option<Self> {
        LocalConsent::data_path().map(Self::new)
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConsentStore for FileConsentStore {
    fn load(&self) -> LocalConsent {
        LocalConsent::load_from(&self.path)
    }

    fn save(&self, consent: &LocalConsent) -> anyhow::Result<()> {
        consent.save_to(&self.path)
    }
}

pub type SaveHook = Box<dyn Fn(&LocalConsent) -> anyhow::Result<()> + Send + Sync>;

/// In-memory fake consent store with fault injection for deterministic testing.
#[allow(dead_code)]
pub struct MemoryConsentStore {
    consent: std::sync::Mutex<LocalConsent>,
    fail_saves: std::sync::atomic::AtomicBool,
    fail_loads: std::sync::atomic::AtomicBool,
    save_count: std::sync::atomic::AtomicUsize,
    load_count: std::sync::atomic::AtomicUsize,
    save_hook: std::sync::Mutex<Option<SaveHook>>,
}

impl Default for MemoryConsentStore {
    fn default() -> Self {
        Self::new(LocalConsent::Absent)
    }
}

impl std::fmt::Debug for MemoryConsentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryConsentStore")
            .field("save_count", &self.save_count())
            .field("load_count", &self.load_count())
            .finish()
    }
}

#[allow(dead_code)]
impl MemoryConsentStore {
    pub fn new(initial: LocalConsent) -> Self {
        Self {
            consent: std::sync::Mutex::new(initial),
            fail_saves: std::sync::atomic::AtomicBool::new(false),
            fail_loads: std::sync::atomic::AtomicBool::new(false),
            save_count: std::sync::atomic::AtomicUsize::new(0),
            load_count: std::sync::atomic::AtomicUsize::new(0),
            save_hook: std::sync::Mutex::new(None),
        }
    }

    pub fn set_save_hook<F>(&self, hook: F)
    where
        F: Fn(&LocalConsent) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        *self.save_hook.lock().unwrap() = Some(Box::new(hook));
    }

    pub fn clear_save_hook(&self) {
        *self.save_hook.lock().unwrap() = None;
    }

    pub fn set_fail_saves(&self, fail: bool) {
        self.fail_saves.store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_fail_loads(&self, fail: bool) {
        self.fail_loads.store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn save_count(&self) -> usize {
        self.save_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn load_count(&self) -> usize {
        self.load_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl ConsentStore for MemoryConsentStore {
    fn load(&self) -> LocalConsent {
        self.load_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail_loads.load(std::sync::atomic::Ordering::SeqCst) {
            return LocalConsent::Absent;
        }
        *self.consent.lock().unwrap()
    }

    fn save(&self, consent: &LocalConsent) -> anyhow::Result<()> {
        self.save_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(ref hook) = *self.save_hook.lock().unwrap() {
            hook(consent)?;
        }
        if self.fail_saves.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("simulated consent disk persistence failure (fault injection)");
        }
        *self.consent.lock().unwrap() = *consent;
        Ok(())
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
    #[allow(dead_code)]
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
    /// `http://` to another computer, without the admin's opt-in.
    Insecure,
}

impl std::fmt::Display for ServerUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerUrlError::Empty => write!(f, "Enter the server address."),
            ServerUrlError::UnsupportedScheme(scheme) => {
                write!(f, "The server address must start with https:// or http:// (not {scheme}://).")
            }
            ServerUrlError::Invalid => write!(f, "That is not a valid server address."),
            ServerUrlError::Insecure => write!(
                f,
                "This server address is not encrypted (http://). Use its https:// address — your password and screenshots must not cross the network in the clear."
            ),
        }
    }
}

impl std::error::Error for ServerUrlError {}

/// Is `host` this computer (`localhost`, `127.0.0.0/8`, `::1`)? Traffic to it
/// never leaves the machine, so plain `http://` is fine there.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// May the agent talk to `url` at all? `https://` always; `http://` only to
/// this computer, or anywhere once the admin set `allow_insecure_http`.
pub fn check_transport(url: &str, allow_insecure_http: bool) -> Result<(), ServerUrlError> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| ServerUrlError::Invalid)?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if allow_insecure_http || parsed.host_str().is_some_and(is_loopback_host) => Ok(()),
        "http" => Err(ServerUrlError::Insecure),
        other => Err(ServerUrlError::UnsupportedScheme(other.to_owned())),
    }
}

/// Does talking to `url` send data over the network unencrypted (plain
/// `http://` to another computer)? The window warns while it does.
pub fn is_unencrypted_remote(url: &str) -> bool {
    check_transport(url, false) == Err(ServerUrlError::Insecure)
}

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
            allow_insecure_http: false,
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
        let app_config = Self::from_config(builder.build()?);
        // Local consent persistence is managed independently via `ConsentStore` in `Worker`
        // to guarantee strict test isolation and failure-aware atomic transactions.
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
        let allow_insecure_http = self.allow_insecure_http;
        let usable = |url: &str| {
            let url = normalize_server_url(url)?;
            check_transport(&url, allow_insecure_http).map(|()| url)
        };
        self.api_base_url = match usable(&self.api_base_url) {
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
    /// long as it is a valid address the agent may use.
    pub fn effective_api_base_url(&self, saved_server: Option<&str>) -> String {
        match saved_server.map(|server| self.server_url(server)) {
            Some(Ok(url)) => url,
            Some(Err(e)) => {
                tracing::warn!(%e, "the saved server address is invalid; using the configured one");
                self.api_base_url.clone()
            }
            None => self.api_base_url.clone(),
        }
    }

    /// A server address typed at sign-in, normalized — or why it cannot be
    /// used (not an address, or `http://` to another computer without
    /// `allow_insecure_http`).
    pub fn server_url(&self, input: &str) -> Result<String, ServerUrlError> {
        let url = normalize_server_url(input)?;
        check_transport(&url, self.allow_insecure_http)?;
        Ok(url)
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
    fn plain_http_is_only_for_this_computer_unless_the_admin_opts_in() {
        for local in ["http://localhost:6006", "http://127.0.0.1:44300", "http://127.9.9.9", "http://[::1]:6006", "http://dev.localhost"] {
            assert_eq!(check_transport(local, false), Ok(()), "{local}");
            assert!(!is_unencrypted_remote(local), "{local}");
        }
        for remote in ["http://10.0.0.5:6006", "http://tracker.example.com", "http://localhost.example.com"] {
            assert_eq!(check_transport(remote, false), Err(ServerUrlError::Insecure), "{remote}");
            assert_eq!(check_transport(remote, true), Ok(()), "opted in: {remote}");
            assert!(is_unencrypted_remote(remote), "{remote}");
        }
        assert_eq!(check_transport("https://tracker.example.com", false), Ok(()));
        assert_eq!(check_transport("ftp://tracker.example.com", true), Err(ServerUrlError::UnsupportedScheme("ftp".into())));
        assert!(ServerUrlError::Insecure.to_string().contains("https://"));
    }

    #[test]
    fn an_insecure_server_is_refused_everywhere_without_the_opt_in() {
        // Configured: the default is used instead (loudly).
        let cfg = from_toml("api_base_url = \"http://10.0.0.5:6006\"");
        assert_eq!(cfg.api_base_url, AppConfig::default().api_base_url);
        assert!(!cfg.allow_insecure_http);
        // Typed at sign-in, or saved with an older sign-in.
        assert_eq!(cfg.server_url("http://10.0.0.5:6006"), Err(ServerUrlError::Insecure));
        assert_eq!(cfg.effective_api_base_url(Some("http://10.0.0.5:6006")), AppConfig::default().api_base_url);
        assert_eq!(cfg.server_url("localhost:6006/"), Ok("https://localhost:6006".into()));

        let opted_in = from_toml("allow_insecure_http = true\napi_base_url = \"http://10.0.0.5:6006\"");
        assert_eq!(opted_in.api_base_url, "http://10.0.0.5:6006");
        assert_eq!(opted_in.server_url("http://10.0.0.6"), Ok("http://10.0.0.6".into()));
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

    #[test]
    fn consent_partial_json_deserialization_default_deny() {
        // Serde deserialization of partial JSON must default ALL omitted capabilities to false (default-deny).
        // It must NOT broaden permissions or reuse permissive config defaults.
        let json_partial = r#"{"Granted": {"keyboard": true}}"#;
        let parsed: LocalConsent = serde_json::from_str(json_partial).expect("partial consent should parse");
        match parsed {
            LocalConsent::Granted(c) => {
                assert!(c.keyboard, "keyboard was explicitly granted");
                assert!(!c.mouse, "mouse omitted in json must default to false");
                assert!(!c.screenshot, "screenshot omitted in json must default to false");
                assert!(!c.webcam, "webcam omitted in json must default to false");
                assert!(!c.active_window, "active_window omitted in json must default to false");
                assert!(!c.running_programs, "running_programs omitted in json must default to false");
                assert!(c.blur_screenshots, "blur_screenshots defaults to true for defense-in-depth");
            }
            other => panic!("expected Granted, got {other:?}"),
        }

        // Empty Granted object defaults everything to false
        let json_empty_grant = r#"{"Granted": {}}"#;
        let parsed: LocalConsent = serde_json::from_str(json_empty_grant).expect("empty grant should parse");
        match parsed {
            LocalConsent::Granted(c) => {
                assert!(!c.keyboard);
                assert!(!c.mouse);
                assert!(!c.screenshot);
                assert!(!c.webcam);
                assert!(!c.active_window);
                assert!(!c.running_programs);
                assert!(c.blur_screenshots);
            }
            other => panic!("expected Granted, got {other:?}"),
        }

        // Explicit Absent
        let json_absent = r#""Absent""#;
        assert_eq!(serde_json::from_str::<LocalConsent>(json_absent).unwrap(), LocalConsent::Absent);

        // Explicit Revoked
        let json_revoked = r#""Revoked""#;
        assert_eq!(serde_json::from_str::<LocalConsent>(json_revoked).unwrap(), LocalConsent::Revoked);

        // Invalid JSON fails closed to error on parse
        assert!(serde_json::from_str::<LocalConsent>(r#"{"Granted": 12345}"#).is_err());
    }

    #[test]
    fn consent_atomic_file_replacement_and_durability() {
        let dir = std::env::temp_dir().join(format!("dosi-test-atomic-{}", uuid::Uuid::new_v4()));
        let path = dir.join("consent.json");

        // Write initial data
        let initial_consent = LocalConsent::Granted(ConsentedPermissions {
            keyboard: true,
            mouse: false,
            screenshot: false,
            webcam: false,
            active_window: false,
            running_programs: false,
            blur_screenshots: true,
        });
        initial_consent.save_to(&path).expect("initial atomic write must succeed");
        assert_eq!(LocalConsent::load_from(&path), initial_consent);

        // Atomic replacement with updated data
        let updated_consent = LocalConsent::Revoked;
        updated_consent.save_to(&path).expect("atomic replacement must succeed");
        assert_eq!(LocalConsent::load_from(&path), updated_consent);

        // Ensure no sibling temp files were left behind
        let read_dir = std::fs::read_dir(&dir).expect("directory must be readable");
        let entries: Vec<_> = read_dir.filter_map(Result::ok).collect();
        assert_eq!(entries.len(), 1, "only the target file must exist; temp file removed");
        assert_eq!(entries[0].file_name(), "consent.json");

        // Attempting to write to an invalid path fails gracefully and leaves original untouched
        let bad_path = path.join("subfile.json"); // path is a file, cannot be parent
        let fail_res = LocalConsent::Absent.save_to(&bad_path);
        assert!(fail_res.is_err(), "write to bad path must fail");
        assert_eq!(LocalConsent::load_from(&path), updated_consent, "prior data preserved on failure");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_consent_store_fault_injection() {
        let store = MemoryConsentStore::new(LocalConsent::Absent);
        assert_eq!(store.load(), LocalConsent::Absent);
        assert_eq!(store.load_count(), 1);
        assert_eq!(store.save_count(), 0);

        // Save succeeds
        let grant = LocalConsent::Granted(ConsentedPermissions::default());
        store.save(&grant).expect("save should succeed");
        assert_eq!(store.save_count(), 1);
        assert_eq!(store.load(), grant);

        // Fault injection: simulate disk write failure
        store.set_fail_saves(true);
        let fail_res = store.save(&LocalConsent::Revoked);
        assert!(fail_res.is_err(), "simulated failure must return Err");
        assert_eq!(store.save_count(), 2);
        // Original grant in store is preserved
        assert_eq!(store.load(), grant);
    }

    #[test]
    fn consent_monotonic_narrowing_and_hook_execution() {
        let broad = ConsentedPermissions {
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
            blur_screenshots: true,
        };
        let narrow = ConsentedPermissions {
            screenshot: false,
            webcam: false,
            keyboard: false,
            mouse: true,
            active_window: false,
            running_programs: false,
            blur_screenshots: false,
        };

        // Intersect proves strict default-deny narrowing
        let inter = broad.intersect(&narrow);
        assert!(inter.mouse);
        assert!(!inter.keyboard);
        assert!(!inter.active_window);
        assert!(!inter.screenshot);
        assert!(!inter.webcam);
        assert!(inter.blur_screenshots, "blur preserved as defense-in-depth");

        // Mixed request: old had mouse+keyboard, new wants mouse+webcam
        let old_grant = LocalConsent::Granted(ConsentedPermissions {
            mouse: true,
            keyboard: true,
            ..ConsentedPermissions::default()
        });
        let new_grant = LocalConsent::Granted(ConsentedPermissions {
            mouse: true,
            webcam: true,
            ..ConsentedPermissions::default()
        });
        let staged = old_grant.narrow_to(&new_grant);
        match staged {
            LocalConsent::Granted(p) => {
                assert!(p.mouse, "retained capability remains");
                assert!(!p.keyboard, "withdrawn capability is false");
                assert!(!p.webcam, "unpersisted expansion capability is NOT granted yet");
            }
            _ => panic!("expected Granted"),
        }

        // Narrowing to Revoked yields Revoked
        assert_eq!(old_grant.narrow_to(&LocalConsent::Revoked), LocalConsent::Revoked);
        assert_eq!(LocalConsent::Absent.narrow_to(&new_grant), LocalConsent::Absent);

        // Store save_hook execution
        let store = MemoryConsentStore::new(LocalConsent::Absent);
        let hook_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hook_ran_clone = hook_ran.clone();
        store.set_save_hook(move |consent| {
            assert_eq!(*consent, old_grant);
            hook_ran_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        store.save(&old_grant).expect("save should succeed");
        assert!(hook_ran.load(std::sync::atomic::Ordering::SeqCst));
    }
}

