//! Agent configuration (design 9.5), ported from the Windows `AppConfig`.
//!
//! Layers, lowest first: built-in defaults < `/etc/dosi-tracker/config.toml`
//! (admin) < `$XDG_CONFIG_HOME/dosi-tracker/config.toml` (user) < `DOSI__*`
//! environment variables (`DOSI__API_BASE_URL`, `DOSI__CAPTURE__SCREENSHOT=false`).
//! A higher layer overrides a lower one, except for the `[capture]` keys:
//! those are local restrictions only, ANDed across layers (a lower layer's
//! `false` cannot be re-enabled above it), and `blur_screenshots` is ORed.
//!
//! Every key is validated. A bad value is reported as a [`ConfigIssue`] and
//! never silently widens anything: an unreadable capture switch counts as
//! `false`, an unreadable blur switch as `true`, any other bad value keeps
//! the lower layer's value. There is no built-in server: `api_base_url` is
//! `None` until a layer or the sign-in provides one. URLs go through the
//! ported [`normalize_server_url`]; plain `http://` is accepted only for a
//! loopback host unless `allow_insecure_http = true`.
//!
//! `config set` / `config unset` rewrite the user file whole (comments are not
//! preserved) through [`crate::paths::write_private_atomic`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::credentials::StoreSetting;
use crate::paths::LinuxPaths;

/// Prefix of the environment layer; `__` separates table and key.
pub const ENV_PREFIX: &str = "DOSI__";

/// Interval bounds (minutes), as on Windows.
pub const MIN_INTERVAL_MINUTES: u64 = 5;
pub const MAX_INTERVAL_MINUTES: u64 = 60;

/// Local restrictions on what may be captured. Defaults allow everything the
/// project and the user's consent allow (they decide; this only narrows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRestrictions {
    pub screenshot: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
    /// Blur screen captures before upload. The effective value is the
    /// project's policy OR the consent's OR this, so a local config can add
    /// blur but never remove it.
    pub blur_screenshots: bool,
}

impl Default for CaptureRestrictions {
    fn default() -> Self {
        Self {
            screenshot: true,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
            blur_screenshots: false,
        }
    }
}

/// `[credentials]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialsConfig {
    pub store: StoreSetting,
    /// Keep the password in the Secret Service (never in a file).
    pub remember_password: bool,
}

impl Default for CredentialsConfig {
    fn default() -> Self {
        Self {
            store: StoreSetting::Auto,
            remember_password: true,
        }
    }
}

/// `[tracking]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackingConfig {
    /// Resume tracking after a login autostart (design 5.2 conditions apply).
    pub resume_at_login: bool,
    /// Manual mode (no idle source): auto-pause after this many hours.
    pub manual_max_hours: u32,
}

impl Default for TrackingConfig {
    fn default() -> Self {
        Self {
            resume_at_login: true,
            manual_max_hours: 10,
        }
    }
}

/// `[notifications]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationsConfig {
    pub enabled: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// `gui.backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuiBackend {
    #[default]
    Auto,
    X11,
    Wayland,
}

impl GuiBackend {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(GuiBackend::Auto),
            "x11" => Some(GuiBackend::X11),
            "wayland" => Some(GuiBackend::Wayland),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GuiBackend::Auto => "auto",
            GuiBackend::X11 => "x11",
            GuiBackend::Wayland => "wayland",
        }
    }
}

/// `[gui]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiConfig {
    pub backend: GuiBackend,
    pub open_window_at_login: bool,
    pub daily_goal_hours: u32,
    pub work_days_per_week: u32,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            backend: GuiBackend::Auto,
            open_window_at_login: false,
            daily_goal_hours: 8,
            work_days_per_week: 5,
        }
    }
}

/// `log.level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Some(LogLevel::Error),
            "warn" => Some(LogLevel::Warn),
            "info" => Some(LogLevel::Info),
            "debug" => Some(LogLevel::Debug),
            "trace" => Some(LogLevel::Trace),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// `[log]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LogConfig {
    pub level: LogLevel,
}

/// Runtime configuration of the agent (design 9.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Base URL of the Dosi-Tracker backend API. No built-in default.
    pub api_base_url: Option<String>,
    /// The web dashboard, opened by "Open web dashboard".
    pub dashboard_url: Option<String>,
    /// The planned length of a block, in minutes (5..=60). Projects set their
    /// own; this is used only when a project does not say.
    pub interval_minutes: u64,
    /// Accept plain `http://` to a non-loopback server (admin opt-in).
    pub allow_insecure_http: bool,
    pub capture: CaptureRestrictions,
    pub credentials: CredentialsConfig,
    pub tracking: TrackingConfig,
    pub notifications: NotificationsConfig,
    pub gui: GuiConfig,
    pub log: LogConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_base_url: None,
            dashboard_url: None,
            interval_minutes: 10,
            allow_insecure_http: false,
            capture: CaptureRestrictions::default(),
            credentials: CredentialsConfig::default(),
            tracking: TrackingConfig::default(),
            notifications: NotificationsConfig::default(),
            gui: GuiConfig::default(),
            log: LogConfig::default(),
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
    /// Plain `http://` to a non-loopback host without `allow_insecure_http`.
    InsecureHttp(String),
}

impl std::fmt::Display for ServerUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerUrlError::Empty => write!(f, "Enter the server address."),
            ServerUrlError::UnsupportedScheme(scheme) => {
                write!(
                    f,
                    "The server address must start with https:// or http:// (not {scheme}://)."
                )
            }
            ServerUrlError::Invalid => write!(f, "That is not a valid server address."),
            ServerUrlError::InsecureHttp(message) => f.write_str(message),
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
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or(ServerUrlError::Invalid)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ServerUrlError::Invalid);
    }
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    Ok(format!("{scheme}://{host}{port}{path}"))
}

/// [`normalize_server_url`], then the transport rule: `http://` only to a
/// loopback host unless `allow_insecure_http`.
pub fn validated_server_url(
    input: &str,
    allow_insecure_http: bool,
) -> Result<String, ServerUrlError> {
    let url = normalize_server_url(input)?;
    crate::api::check_server_url(&url, allow_insecure_http)
        .map_err(ServerUrlError::InsecureHttp)?;
    Ok(url)
}

/// Which layer a value or an issue came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerOrigin {
    System(PathBuf),
    User(PathBuf),
    Environment,
}

impl std::fmt::Display for LayerOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerOrigin::System(path) | LayerOrigin::User(path) => write!(f, "{}", path.display()),
            LayerOrigin::Environment => f.write_str("environment"),
        }
    }
}

/// Something wrong with the configuration; the agent still runs (on the
/// safe value) and `doctor` / `config show` report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub origin: LayerOrigin,
    /// The dotted key, when the issue is about one key.
    pub key: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.key {
            Some(key) => write!(f, "{}: {key}: {}", self.origin, self.message),
            None => write!(f, "{}: {}", self.origin, self.message),
        }
    }
}

/// The type and rules of one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Url,
    Interval,
    Bool,
    /// A `[capture]` switch: ANDed across layers.
    CaptureBool,
    /// `capture.blur_screenshots`: ORed across layers.
    Blur,
    Store,
    /// Whole hours, 1..=24.
    Hours,
    /// Days per week, 1..=7.
    Days,
    Backend,
    Level,
}

/// Every key of design 9.5, dotted.
const KEYS: &[(&str, Kind)] = &[
    ("api_base_url", Kind::Url),
    ("dashboard_url", Kind::Url),
    ("interval_minutes", Kind::Interval),
    ("allow_insecure_http", Kind::Bool),
    ("capture.screenshot", Kind::CaptureBool),
    ("capture.keyboard", Kind::CaptureBool),
    ("capture.mouse", Kind::CaptureBool),
    ("capture.active_window", Kind::CaptureBool),
    ("capture.running_programs", Kind::CaptureBool),
    ("capture.blur_screenshots", Kind::Blur),
    ("credentials.store", Kind::Store),
    ("credentials.remember_password", Kind::Bool),
    ("tracking.resume_at_login", Kind::Bool),
    ("tracking.manual_max_hours", Kind::Hours),
    ("notifications.enabled", Kind::Bool),
    ("gui.backend", Kind::Backend),
    ("gui.open_window_at_login", Kind::Bool),
    ("gui.daily_goal_hours", Kind::Hours),
    ("gui.work_days_per_week", Kind::Days),
    ("log.level", Kind::Level),
];

/// The dotted names of every configuration key.
pub fn keys() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|(key, _)| *key)
}

fn kind_of(key: &str) -> Option<Kind> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, kind)| *kind)
}

/// A validated value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Typed {
    Url(String),
    Int(u64),
    Bool(bool),
    Store(StoreSetting),
    Backend(GuiBackend),
    Level(LogLevel),
}

impl Typed {
    fn to_toml(&self) -> toml::Value {
        match self {
            Typed::Url(url) => toml::Value::String(url.clone()),
            Typed::Int(n) => toml::Value::Integer(i64::try_from(*n).unwrap_or(i64::MAX)),
            Typed::Bool(b) => toml::Value::Boolean(*b),
            Typed::Store(store) => toml::Value::String(store.as_str().into()),
            Typed::Backend(backend) => toml::Value::String(backend.as_str().into()),
            Typed::Level(level) => toml::Value::String(level.as_str().into()),
        }
    }
}

/// A raw value from a file (TOML-typed) or the environment (a string).
#[derive(Debug, Clone)]
enum Raw {
    Toml(toml::Value),
    Env(String),
}

impl Raw {
    fn as_bool(&self) -> Option<bool> {
        match self {
            Raw::Toml(toml::Value::Boolean(b)) => Some(*b),
            Raw::Env(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            Raw::Toml(_) => None,
        }
    }

    fn as_int(&self) -> Option<i64> {
        match self {
            Raw::Toml(toml::Value::Integer(n)) => Some(*n),
            Raw::Env(s) => s.trim().parse().ok(),
            Raw::Toml(_) => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Raw::Toml(toml::Value::String(s)) => Some(s),
            Raw::Env(s) => Some(s),
            Raw::Toml(_) => None,
        }
    }
}

/// How out-of-range numbers are handled: loading clamps (as Windows does),
/// `config set` refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    Clamp,
    Strict,
}

fn parse_typed(kind: Kind, raw: &Raw, range: Range) -> Result<Typed, String> {
    let bounded = |n: i64, min: u64, max: u64, what: &str| -> Result<Typed, String> {
        let clamped = n.clamp(min as i64, max as i64) as u64;
        if range == Range::Strict && clamped as i64 != n {
            return Err(format!("{what} must be between {min} and {max}"));
        }
        Ok(Typed::Int(clamped))
    };
    match kind {
        Kind::Url => {
            let text = raw.as_str().ok_or("expected a URL string")?;
            normalize_server_url(text)
                .map(Typed::Url)
                .map_err(|e| e.to_string())
        }
        Kind::Interval => {
            let n = raw.as_int().ok_or("expected a whole number of minutes")?;
            bounded(
                n,
                MIN_INTERVAL_MINUTES,
                MAX_INTERVAL_MINUTES,
                "the interval",
            )
        }
        Kind::Hours => {
            let n = raw.as_int().ok_or("expected a whole number of hours")?;
            bounded(n, 1, 24, "hours")
        }
        Kind::Days => {
            let n = raw.as_int().ok_or("expected a whole number of days")?;
            bounded(n, 1, 7, "days per week")
        }
        Kind::Bool | Kind::CaptureBool | Kind::Blur => raw
            .as_bool()
            .map(Typed::Bool)
            .ok_or_else(|| "expected true or false".to_owned()),
        Kind::Store => raw
            .as_str()
            .and_then(StoreSetting::parse)
            .map(Typed::Store)
            .ok_or_else(|| "expected auto, secret-service or file".to_owned()),
        Kind::Backend => raw
            .as_str()
            .and_then(GuiBackend::parse)
            .map(Typed::Backend)
            .ok_or_else(|| "expected auto, x11 or wayland".to_owned()),
        Kind::Level => raw
            .as_str()
            .and_then(LogLevel::parse)
            .map(Typed::Level)
            .ok_or_else(|| "expected error, warn, info, debug or trace".to_owned()),
    }
}

/// Where the layers come from. [`ConfigSources::for_paths`] is the real set;
/// tests point the system file elsewhere.
#[derive(Debug, Clone)]
pub struct ConfigSources {
    pub system_file: PathBuf,
    pub user_file: PathBuf,
    /// The environment variables to consider (only `DOSI__*` ones are used).
    pub env: Vec<(String, String)>,
}

impl ConfigSources {
    /// `/etc/dosi-tracker/config.toml`, the user's file and `env` (pass
    /// `std::env::vars()` from the binary's entry point).
    pub fn for_paths(paths: &LinuxPaths, env: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            system_file: paths.system_config_file(),
            user_file: paths.config_file(),
            env: env.into_iter().collect(),
        }
    }
}

/// The effective configuration and what was wrong on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub issues: Vec<ConfigIssue>,
}

impl AppConfig {
    /// Load every layer (missing files are fine) and validate each key.
    pub fn load(sources: &ConfigSources) -> LoadedConfig {
        let mut issues = Vec::new();
        let mut layers: Vec<(LayerOrigin, Vec<(String, Raw)>)> = Vec::new();
        for origin in [
            LayerOrigin::System(sources.system_file.clone()),
            LayerOrigin::User(sources.user_file.clone()),
        ] {
            let path = match &origin {
                LayerOrigin::System(p) | LayerOrigin::User(p) => p.clone(),
                LayerOrigin::Environment => unreachable!(),
            };
            let user = matches!(origin, LayerOrigin::User(_));
            match read_layer_file(&path, user) {
                Ok(Some(table)) => {
                    let values = flatten(&table);
                    layers.push((origin, values));
                }
                Ok(None) => {}
                Err(message) => issues.push(ConfigIssue {
                    origin,
                    key: None,
                    message,
                }),
            }
        }
        layers.push((LayerOrigin::Environment, env_layer(&sources.env)));

        let mut config = AppConfig::default();
        let mut server_origin = None;
        for (origin, values) in &layers {
            for (key, raw) in values {
                if config.apply(origin, key, raw, &mut issues) && key == "api_base_url" {
                    server_origin = Some(origin.clone());
                }
            }
        }
        if let Some(origin) = server_origin {
            config.check_transport(origin, &mut issues);
        }
        LoadedConfig { config, issues }
    }

    /// Apply one raw value; returns whether it changed the configuration.
    fn apply(
        &mut self,
        origin: &LayerOrigin,
        key: &str,
        raw: &Raw,
        issues: &mut Vec<ConfigIssue>,
    ) -> bool {
        let Some(kind) = kind_of(key) else {
            issues.push(ConfigIssue {
                origin: origin.clone(),
                key: Some(key.to_owned()),
                message: "unknown key (ignored)".into(),
            });
            return false;
        };
        let typed = match parse_typed(kind, raw, Range::Clamp) {
            Ok(typed) => typed,
            Err(message) => {
                // Fail closed: a capture switch nobody can read is off, a blur
                // switch nobody can read is on; anything else keeps the value
                // below it.
                let fallback = match kind {
                    Kind::CaptureBool => Some(Typed::Bool(false)),
                    Kind::Blur => Some(Typed::Bool(true)),
                    _ => None,
                };
                let note = match kind {
                    Kind::CaptureBool => " (treated as false)",
                    Kind::Blur => " (treated as true)",
                    _ => " (ignored)",
                };
                issues.push(ConfigIssue {
                    origin: origin.clone(),
                    key: Some(key.to_owned()),
                    message: format!("{message}{note}"),
                });
                match fallback {
                    Some(typed) => typed,
                    None => return false,
                }
            }
        };
        self.set_typed(key, typed);
        true
    }

    /// Apply one validated value with the layering rules.
    fn set_typed(&mut self, key: &str, typed: Typed) {
        let capture = &mut self.capture;
        match (key, typed) {
            ("api_base_url", Typed::Url(url)) => self.api_base_url = Some(url),
            ("dashboard_url", Typed::Url(url)) => self.dashboard_url = Some(url),
            ("interval_minutes", Typed::Int(n)) => self.interval_minutes = n,
            ("allow_insecure_http", Typed::Bool(b)) => self.allow_insecure_http = b,
            ("capture.screenshot", Typed::Bool(b)) => capture.screenshot &= b,
            ("capture.keyboard", Typed::Bool(b)) => capture.keyboard &= b,
            ("capture.mouse", Typed::Bool(b)) => capture.mouse &= b,
            ("capture.active_window", Typed::Bool(b)) => capture.active_window &= b,
            ("capture.running_programs", Typed::Bool(b)) => capture.running_programs &= b,
            ("capture.blur_screenshots", Typed::Bool(b)) => capture.blur_screenshots |= b,
            ("credentials.store", Typed::Store(store)) => self.credentials.store = store,
            ("credentials.remember_password", Typed::Bool(b)) => {
                self.credentials.remember_password = b
            }
            ("tracking.resume_at_login", Typed::Bool(b)) => self.tracking.resume_at_login = b,
            ("tracking.manual_max_hours", Typed::Int(n)) => {
                self.tracking.manual_max_hours = n as u32
            }
            ("notifications.enabled", Typed::Bool(b)) => self.notifications.enabled = b,
            ("gui.backend", Typed::Backend(backend)) => self.gui.backend = backend,
            ("gui.open_window_at_login", Typed::Bool(b)) => self.gui.open_window_at_login = b,
            ("gui.daily_goal_hours", Typed::Int(n)) => self.gui.daily_goal_hours = n as u32,
            ("gui.work_days_per_week", Typed::Int(n)) => self.gui.work_days_per_week = n as u32,
            ("log.level", Typed::Level(level)) => self.log.level = level,
            (key, typed) => unreachable!("{key} cannot hold {typed:?}"),
        }
    }

    /// A configured plain-`http://` server that is not loopback is dropped
    /// unless `allow_insecure_http` (from any layer) opts in.
    fn check_transport(&mut self, origin: LayerOrigin, issues: &mut Vec<ConfigIssue>) {
        if let Some(url) = &self.api_base_url {
            if let Err(message) = crate::api::check_server_url(url, self.allow_insecure_http) {
                issues.push(ConfigIssue {
                    origin,
                    key: Some("api_base_url".into()),
                    message: format!("{message} (ignored)"),
                });
                self.api_base_url = None;
            }
        }
    }

    pub fn interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.interval_minutes * 60)
    }

    /// The server to talk to: the address saved with the sign-in wins over
    /// the configured one, as long as it is a valid address the transport
    /// rules accept. `None` when neither exists (the user must give one).
    pub fn effective_api_base_url(&self, saved_server: Option<&str>) -> Option<String> {
        match saved_server.map(|s| validated_server_url(s, self.allow_insecure_http)) {
            Some(Ok(url)) => Some(url),
            Some(Err(e)) => {
                tracing::warn!(%e, "the saved server address is invalid; using the configured one");
                self.api_base_url.clone()
            }
            None => self.api_base_url.clone(),
        }
    }

    /// The effective value of `key` as TOML, or `None` for an unknown key or
    /// an unset optional one.
    pub fn value(&self, key: &str) -> Option<toml::Value> {
        let b = |b: bool| Some(toml::Value::Boolean(b));
        let n = |n: u64| Some(toml::Value::Integer(n as i64));
        let s = |s: &str| Some(toml::Value::String(s.to_owned()));
        match key {
            "api_base_url" => self.api_base_url.as_deref().and_then(s),
            "dashboard_url" => self.dashboard_url.as_deref().and_then(s),
            "interval_minutes" => n(self.interval_minutes),
            "allow_insecure_http" => b(self.allow_insecure_http),
            "capture.screenshot" => b(self.capture.screenshot),
            "capture.keyboard" => b(self.capture.keyboard),
            "capture.mouse" => b(self.capture.mouse),
            "capture.active_window" => b(self.capture.active_window),
            "capture.running_programs" => b(self.capture.running_programs),
            "capture.blur_screenshots" => b(self.capture.blur_screenshots),
            "credentials.store" => s(self.credentials.store.as_str()),
            "credentials.remember_password" => b(self.credentials.remember_password),
            "tracking.resume_at_login" => b(self.tracking.resume_at_login),
            "tracking.manual_max_hours" => n(u64::from(self.tracking.manual_max_hours)),
            "notifications.enabled" => b(self.notifications.enabled),
            "gui.backend" => s(self.gui.backend.as_str()),
            "gui.open_window_at_login" => b(self.gui.open_window_at_login),
            "gui.daily_goal_hours" => n(u64::from(self.gui.daily_goal_hours)),
            "gui.work_days_per_week" => n(u64::from(self.gui.work_days_per_week)),
            "log.level" => s(self.log.level.as_str()),
            _ => None,
        }
    }

    /// The effective configuration as TOML (`config show`).
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }
}

/// Read one layer file. `Ok(None)` when it does not exist. The user file
/// must be a regular file of the current user that nobody else can write
/// (someone else's file could point the agent at another server).
fn read_layer_file(path: &Path, user_file: bool) -> Result<Option<toml::Table>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot inspect the file: {e} (ignored)")),
    };
    if user_file {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err("not a regular file (ignored)".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: geteuid has no preconditions and cannot fail.
            if meta.uid() != unsafe { libc::geteuid() } {
                return Err("owned by another user (ignored)".into());
            }
            if meta.mode() & 0o022 != 0 {
                return Err("writable by other users (ignored)".into());
            }
        }
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read: {e} (ignored)"))?;
    text.parse::<toml::Table>()
        .map(Some)
        .map_err(|e| format!("not valid TOML: {} (ignored)", e.message()))
}

/// The keys of a layer file, dotted (`capture.screenshot`). Unknown keys,
/// including tables nested deeper than one level, are reported by `apply`.
fn flatten(table: &toml::Table) -> Vec<(String, Raw)> {
    let mut out = Vec::new();
    for (key, value) in table {
        match value {
            toml::Value::Table(inner) => {
                for (inner_key, inner_value) in inner {
                    out.push((format!("{key}.{inner_key}"), Raw::Toml(inner_value.clone())));
                }
            }
            other => out.push((key.clone(), Raw::Toml(other.clone()))),
        }
    }
    out
}

/// `DOSI__CAPTURE__SCREENSHOT=false` -> (`capture.screenshot`, "false").
fn env_layer(env: &[(String, String)]) -> Vec<(String, Raw)> {
    let mut out: Vec<(String, Raw)> = env
        .iter()
        .filter_map(|(name, value)| {
            let rest = name.strip_prefix(ENV_PREFIX)?;
            Some((
                rest.to_ascii_lowercase().replace("__", "."),
                Raw::Env(value.clone()),
            ))
        })
        .collect();
    // A stable order whatever the process environment's order was.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Why `config set` / `config unset` refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    UnknownKey(String),
    InvalidValue {
        key: String,
        message: String,
    },
    /// The user file exists but cannot be parsed or trusted; it is not
    /// overwritten.
    UserFileUnusable(String),
    Io(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::UnknownKey(key) => write!(f, "unknown configuration key `{key}`"),
            ConfigError::InvalidValue { key, message } => write!(f, "{key}: {message}"),
            ConfigError::UserFileUnusable(why) => {
                write!(f, "the configuration file cannot be updated: {why}")
            }
            ConfigError::Io(why) => write!(f, "could not save the configuration: {why}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Validate `value` for `key` as `config set` would, returning the TOML value
/// to store. `effective` is the configuration in force (for the transport rule
/// on `api_base_url`).
pub fn validate_setting(
    key: &str,
    value: &str,
    effective: &AppConfig,
) -> Result<toml::Value, ConfigError> {
    let kind = kind_of(key).ok_or_else(|| ConfigError::UnknownKey(key.to_owned()))?;
    let invalid = |message: String| ConfigError::InvalidValue {
        key: key.to_owned(),
        message,
    };
    let typed = parse_typed(kind, &Raw::Env(value.to_owned()), Range::Strict).map_err(invalid)?;
    if let (Kind::Url, "api_base_url", Typed::Url(url)) = (kind, key, &typed) {
        crate::api::check_server_url(url, effective.allow_insecure_http).map_err(invalid)?;
    }
    Ok(typed.to_toml())
}

/// `config set KEY VALUE`: validate, then rewrite the user file (0600, atomic).
#[cfg(unix)]
pub fn set_user_value(
    user_file: &Path,
    key: &str,
    value: &str,
    effective: &AppConfig,
) -> Result<(), ConfigError> {
    let value = validate_setting(key, value, effective)?;
    let mut table = read_user_table(user_file)?;
    match key.split_once('.') {
        Some((section, name)) => {
            let entry = table
                .entry(section.to_owned())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if !entry.is_table() {
                *entry = toml::Value::Table(toml::Table::new());
            }
            entry
                .as_table_mut()
                .expect("just made a table")
                .insert(name.to_owned(), value);
        }
        None => {
            table.insert(key.to_owned(), value);
        }
    }
    write_user_table(user_file, &table)
}

/// `config unset KEY`: remove the key from the user file. Returns whether it
/// was there; the file is only rewritten when something changed.
#[cfg(unix)]
pub fn unset_user_value(user_file: &Path, key: &str) -> Result<bool, ConfigError> {
    kind_of(key).ok_or_else(|| ConfigError::UnknownKey(key.to_owned()))?;
    let mut table = read_user_table(user_file)?;
    let removed = match key.split_once('.') {
        Some((section, name)) => {
            let removed = table
                .get_mut(section)
                .and_then(toml::Value::as_table_mut)
                .and_then(|inner| inner.remove(name))
                .is_some();
            if table
                .get(section)
                .and_then(toml::Value::as_table)
                .is_some_and(toml::Table::is_empty)
            {
                table.remove(section);
            }
            removed
        }
        None => table.remove(key).is_some(),
    };
    if removed {
        write_user_table(user_file, &table)?;
    }
    Ok(removed)
}

#[cfg(unix)]
fn read_user_table(user_file: &Path) -> Result<toml::Table, ConfigError> {
    match read_layer_file(user_file, true) {
        Ok(Some(table)) => Ok(table),
        Ok(None) => Ok(toml::Table::new()),
        Err(why) => Err(ConfigError::UserFileUnusable(
            why.trim_end_matches(" (ignored)").to_owned(),
        )),
    }
}

#[cfg(unix)]
fn write_user_table(user_file: &Path, table: &toml::Table) -> Result<(), ConfigError> {
    let dir = user_file
        .parent()
        .ok_or_else(|| ConfigError::Io("the configuration file has no directory".into()))?;
    crate::paths::ensure_private_dir(dir).map_err(|e| ConfigError::Io(e.to_string()))?;
    let text = toml::to_string(table).map_err(|e| ConfigError::Io(e.to_string()))?;
    crate::paths::write_private_atomic(user_file, text.as_bytes())
        .map_err(|e| ConfigError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary layer set: system and user files plus environment.
    struct Layers {
        dir: tempfile::TempDir,
        env: Vec<(String, String)>,
    }

    impl Layers {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                env: Vec::new(),
            }
        }

        fn system_file(&self) -> PathBuf {
            self.dir.path().join("etc").join("config.toml")
        }

        fn user_file(&self) -> PathBuf {
            self.dir.path().join("user").join("config.toml")
        }

        fn system(self, toml: &str) -> Self {
            std::fs::create_dir_all(self.system_file().parent().unwrap()).unwrap();
            std::fs::write(self.system_file(), toml).unwrap();
            self
        }

        fn user(self, toml: &str) -> Self {
            std::fs::create_dir_all(self.user_file().parent().unwrap()).unwrap();
            std::fs::write(self.user_file(), toml).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(self.user_file(), std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            self
        }

        fn env(mut self, name: &str, value: &str) -> Self {
            self.env.push((name.into(), value.into()));
            self
        }

        fn sources(&self) -> ConfigSources {
            ConfigSources {
                system_file: self.system_file(),
                user_file: self.user_file(),
                env: self.env.clone(),
            }
        }

        fn load(&self) -> LoadedConfig {
            AppConfig::load(&self.sources())
        }
    }

    fn from_toml(toml: &str) -> AppConfig {
        Layers::new().user(toml).load().config
    }

    #[test]
    fn empty_config_is_all_defaults() {
        let loaded = Layers::new().user("").load();
        let cfg = loaded.config;
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(cfg.api_base_url, None, "no built-in server");
        assert_eq!(cfg.interval_minutes, 10);
        assert!(cfg.capture.screenshot && cfg.capture.keyboard && !cfg.capture.blur_screenshots);
        assert_eq!(cfg, AppConfig::default());
        assert_eq!(
            Layers::new().load().config,
            AppConfig::default(),
            "no files at all"
        );
    }

    #[test]
    fn partial_config_only_overrides_what_it_sets() {
        let cfg = from_toml("api_base_url = \"https://tracker.example.com\"");
        assert_eq!(
            cfg.api_base_url.as_deref(),
            Some("https://tracker.example.com")
        );
        assert_eq!(cfg.interval_minutes, 10);
        assert!(cfg.capture.keyboard);
    }

    #[test]
    fn partial_capture_table_keeps_other_permissions() {
        let cfg = from_toml("interval_minutes = 15\n[capture]\nscreenshot = false\n");
        assert_eq!(cfg.interval_minutes, 15);
        assert!(!cfg.capture.screenshot);
        // Untouched keys keep their defaults.
        assert!(cfg.capture.keyboard && cfg.capture.mouse);
        assert!(cfg.capture.active_window && cfg.capture.running_programs);
        assert_eq!(cfg.api_base_url, None);
    }

    #[test]
    fn dashboard_url_defaults_and_trims_a_trailing_slash() {
        assert_eq!(from_toml("").dashboard_url, None);
        let cfg = from_toml("dashboard_url = \"https://app.example.com/ \"");
        assert_eq!(
            cfg.dashboard_url.as_deref(),
            Some("https://app.example.com")
        );
    }

    #[test]
    fn api_base_url_is_normalized_and_validated() {
        let cfg = from_toml("api_base_url = \" https://tracker.example.com// \"");
        assert_eq!(
            cfg.api_base_url.as_deref(),
            Some("https://tracker.example.com")
        );
        // Not a URL the agent can use: no server, never a broken one.
        let loaded = Layers::new()
            .user("api_base_url = \"ftp://tracker.example.com\"")
            .load();
        assert_eq!(loaded.config.api_base_url, None);
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].key.as_deref(), Some("api_base_url"));
        assert!(!loaded.config.capture.blur_screenshots);
    }

    #[test]
    fn server_urls_are_normalized() {
        let ok = |input: &str| normalize_server_url(input).unwrap();
        assert_eq!(
            ok("https://tracker.example.com/"),
            "https://tracker.example.com"
        );
        assert_eq!(
            ok("  HTTPS://Tracker.Example.com:8443/// "),
            "https://tracker.example.com:8443"
        );
        assert_eq!(ok("http://10.0.0.5:6006"), "http://10.0.0.5:6006");
        // A bare host (what people type) is https.
        assert_eq!(ok("tracker.example.com"), "https://tracker.example.com");
        assert_eq!(ok("localhost:6006"), "https://localhost:6006");
        // A reverse-proxy path prefix is kept; the default port is dropped.
        assert_eq!(
            ok("https://example.com:443/tracker/"),
            "https://example.com/tracker"
        );
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
        for bad in [
            "https://",
            "https://exa mple.com",
            "https://user:pw@example.com",
            "https://example.com/?a=1",
            "https://example.com/#x",
        ] {
            assert_eq!(
                normalize_server_url(bad),
                Err(ServerUrlError::Invalid),
                "{bad}"
            );
        }
        // The messages are meant for the sign-in form.
        assert!(ServerUrlError::UnsupportedScheme("ftp".into())
            .to_string()
            .contains("https://"));
    }

    #[test]
    fn a_server_saved_at_sign_in_overrides_the_config() {
        let cfg = from_toml("api_base_url = \"https://configured.example.com\"");
        assert_eq!(
            cfg.effective_api_base_url(None).as_deref(),
            Some("https://configured.example.com")
        );
        assert_eq!(
            cfg.effective_api_base_url(Some("saved.example.com/"))
                .as_deref(),
            Some("https://saved.example.com")
        );
        // A damaged saved value never breaks the agent.
        assert_eq!(
            cfg.effective_api_base_url(Some("ftp://saved")).as_deref(),
            Some("https://configured.example.com")
        );
        // Nor does a saved plain-http one the transport rules refuse.
        assert_eq!(
            cfg.effective_api_base_url(Some("http://tracker.example.com"))
                .as_deref(),
            Some("https://configured.example.com")
        );
        assert_eq!(AppConfig::default().effective_api_base_url(None), None);
    }

    #[test]
    fn interval_is_clamped() {
        assert_eq!(from_toml("interval_minutes = 1").interval_minutes, 5);
        assert_eq!(from_toml("interval_minutes = 600").interval_minutes, 60);
        assert_eq!(
            from_toml("interval_minutes = 15").interval(),
            std::time::Duration::from_secs(900)
        );
    }

    #[test]
    fn wrong_types_fall_back_to_defaults() {
        let loaded = Layers::new().user("interval_minutes = \"ten\"").load();
        assert_eq!(loaded.config.interval_minutes, 10);
        assert_eq!(loaded.issues.len(), 1, "reported, not silent");
        assert!(loaded.issues[0].to_string().contains("interval_minutes"));
        // A file that is not TOML at all is ignored as a whole and reported.
        let loaded = Layers::new().user("this is = = not toml").load();
        assert_eq!(loaded.config, AppConfig::default());
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].key, None);
    }

    #[test]
    fn layers_apply_in_precedence_order() {
        // Windows looked next to the executable, then in the working
        // directory; Linux layers /etc < user file < DOSI__ environment.
        let layers = Layers::new()
            .system("api_base_url = \"https://admin.example.com\"\ninterval_minutes = 20\n[log]\nlevel = \"warn\"\n")
            .user("api_base_url = \"https://user.example.com\"\n[log]\nlevel = \"debug\"\n");
        let cfg = layers.load().config;
        assert_eq!(
            cfg.api_base_url.as_deref(),
            Some("https://user.example.com")
        );
        assert_eq!(
            cfg.interval_minutes, 20,
            "the admin value stays where the user set none"
        );
        assert_eq!(cfg.log.level, LogLevel::Debug);

        let layers = layers
            .env("DOSI__API_BASE_URL", "env.example.com")
            .env("DOSI__LOG__LEVEL", "trace")
            .env("DOSI_SOCKET", "/not/config");
        let loaded = layers.load();
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(
            loaded.config.api_base_url.as_deref(),
            Some("https://env.example.com")
        );
        assert_eq!(loaded.config.log.level, LogLevel::Trace);
    }

    #[test]
    fn capture_switches_only_narrow_across_layers_and_blur_only_adds() {
        // The admin turns screenshots off and blur on; neither the user file
        // nor the environment can undo that.
        let layers = Layers::new()
            .system("[capture]\nscreenshot = false\nblur_screenshots = true\n")
            .user("[capture]\nscreenshot = true\nblur_screenshots = false\nkeyboard = false\n")
            .env("DOSI__CAPTURE__SCREENSHOT", "true")
            .env("DOSI__CAPTURE__KEYBOARD", "true")
            .env("DOSI__CAPTURE__MOUSE", "false")
            .env("DOSI__CAPTURE__BLUR_SCREENSHOTS", "false");
        let capture = layers.load().config.capture;
        assert!(!capture.screenshot, "admin false stays false");
        assert!(capture.blur_screenshots, "admin blur stays on");
        assert!(
            !capture.keyboard,
            "user false cannot be re-enabled by the environment"
        );
        assert!(!capture.mouse, "the environment can narrow too");
        assert!(capture.active_window && capture.running_programs);
    }

    #[test]
    fn unreadable_capture_values_fail_closed() {
        let loaded = Layers::new()
            .user("[capture]\nscreenshot = \"maybe\"\nblur_screenshots = 3\n")
            .load();
        assert!(
            !loaded.config.capture.screenshot,
            "unreadable switch counts as off"
        );
        assert!(
            loaded.config.capture.blur_screenshots,
            "unreadable blur counts as on"
        );
        assert_eq!(loaded.issues.len(), 2);
        let loaded = Layers::new().env("DOSI__CAPTURE__MOUSE", "perhaps").load();
        assert!(!loaded.config.capture.mouse);
    }

    #[test]
    fn plain_http_is_loopback_only_unless_allowed() {
        let loaded = Layers::new()
            .user("api_base_url = \"http://tracker.example.com\"")
            .load();
        assert_eq!(loaded.config.api_base_url, None);
        assert!(loaded.issues[0].message.contains("allow_insecure_http"));

        let cfg = from_toml("api_base_url = \"http://127.0.0.1:6006\"");
        assert_eq!(cfg.api_base_url.as_deref(), Some("http://127.0.0.1:6006"));

        // The opt-in may come from any layer, even a later one.
        let loaded = Layers::new()
            .user("api_base_url = \"http://tracker.example.com\"")
            .env("DOSI__ALLOW_INSECURE_HTTP", "true")
            .load();
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(
            loaded.config.api_base_url.as_deref(),
            Some("http://tracker.example.com")
        );
    }

    #[test]
    fn every_design_key_is_known_and_validated() {
        let all = "api_base_url = \"https://a.example\"\ndashboard_url = \"https://d.example\"\n\
            interval_minutes = 30\nallow_insecure_http = false\n\
            [capture]\nscreenshot = true\nkeyboard = true\nmouse = true\nactive_window = false\nrunning_programs = true\nblur_screenshots = true\n\
            [credentials]\nstore = \"secret-service\"\nremember_password = false\n\
            [tracking]\nresume_at_login = false\nmanual_max_hours = 6\n\
            [notifications]\nenabled = false\n\
            [gui]\nbackend = \"x11\"\nopen_window_at_login = true\ndaily_goal_hours = 7\nwork_days_per_week = 4\n\
            [log]\nlevel = \"error\"\n";
        let loaded = Layers::new().user(all).load();
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        let cfg = loaded.config;
        assert_eq!(cfg.credentials.store, StoreSetting::SecretService);
        assert!(!cfg.credentials.remember_password && !cfg.tracking.resume_at_login);
        assert_eq!(cfg.tracking.manual_max_hours, 6);
        assert!(!cfg.notifications.enabled);
        assert_eq!(cfg.gui.backend, GuiBackend::X11);
        assert_eq!(
            (cfg.gui.daily_goal_hours, cfg.gui.work_days_per_week),
            (7, 4)
        );
        assert_eq!(cfg.log.level, LogLevel::Error);
        assert!(!cfg.capture.active_window && cfg.capture.blur_screenshots);
        for key in keys() {
            assert!(cfg.value(key).is_some(), "{key} has a value");
        }
        assert_eq!(keys().count(), 20);

        let loaded = Layers::new()
            .user("webcam = true\n[capture]\nwebcam = true\n[gui]\nbackend = \"mir\"\n")
            .load();
        assert_eq!(loaded.config.gui.backend, GuiBackend::Auto);
        assert_eq!(loaded.issues.len(), 3, "{:?}", loaded.issues);
        assert!(loaded
            .issues
            .iter()
            .any(|i| i.message.starts_with("unknown key")));
    }

    #[test]
    fn config_set_validates_strictly() {
        let cfg = AppConfig::default();
        assert_eq!(
            validate_setting("interval_minutes", "15", &cfg),
            Ok(toml::Value::Integer(15))
        );
        assert!(matches!(
            validate_setting("interval_minutes", "1", &cfg),
            Err(ConfigError::InvalidValue { .. })
        ));
        assert!(matches!(
            validate_setting("capture.webcam", "true", &cfg),
            Err(ConfigError::UnknownKey(_))
        ));
        assert_eq!(
            validate_setting("api_base_url", "tracker.example.com/", &cfg),
            Ok(toml::Value::String("https://tracker.example.com".into()))
        );
        assert!(validate_setting("api_base_url", "http://tracker.example.com", &cfg).is_err());
        let insecure = AppConfig {
            allow_insecure_http: true,
            ..AppConfig::default()
        };
        assert!(validate_setting("api_base_url", "http://tracker.example.com", &insecure).is_ok());
        assert_eq!(
            validate_setting("credentials.store", "file", &cfg),
            Ok(toml::Value::String("file".into()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_set_and_unset_rewrite_the_user_file_privately() {
        use std::os::unix::fs::PermissionsExt;
        let layers = Layers::new();
        let user = layers.user_file();
        let cfg = AppConfig::default();
        set_user_value(&user, "capture.screenshot", "false", &cfg).unwrap();
        set_user_value(&user, "api_base_url", "tracker.example.com", &cfg).unwrap();
        let mode = std::fs::metadata(&user).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(user.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        let loaded = layers.load();
        assert!(!loaded.config.capture.screenshot);
        assert_eq!(
            loaded.config.api_base_url.as_deref(),
            Some("https://tracker.example.com")
        );

        assert!(unset_user_value(&user, "capture.screenshot").unwrap());
        assert!(
            !unset_user_value(&user, "capture.screenshot").unwrap(),
            "already gone"
        );
        let text = std::fs::read_to_string(&user).unwrap();
        assert!(
            !text.contains("[capture]"),
            "an emptied table is removed: {text}"
        );
        assert!(layers.load().config.capture.screenshot);

        // Invalid values never reach the file.
        assert!(set_user_value(&user, "gui.backend", "mir", &cfg).is_err());
        assert!(!std::fs::read_to_string(&user).unwrap().contains("mir"));

        // A user file that is not TOML is never overwritten.
        std::fs::write(&user, "not = = toml").unwrap();
        assert!(matches!(
            set_user_value(&user, "log.level", "debug", &cfg),
            Err(ConfigError::UserFileUnusable(_))
        ));
        assert_eq!(std::fs::read_to_string(&user).unwrap(), "not = = toml");
    }

    #[cfg(unix)]
    #[test]
    fn a_user_file_others_can_write_is_ignored() {
        use std::os::unix::fs::PermissionsExt;
        let layers = Layers::new().user("api_base_url = \"https://evil.example.com\"");
        std::fs::set_permissions(layers.user_file(), std::fs::Permissions::from_mode(0o666))
            .unwrap();
        let loaded = layers.load();
        assert_eq!(loaded.config.api_base_url, None);
        assert!(loaded.issues[0].message.contains("writable by other users"));
    }
}
