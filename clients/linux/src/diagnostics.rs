//! Capability diagnostics and environment detection for Dosi-Tracker Linux
//! (`dosi-tracker doctor`, report v2).
//!
//! Enforces:
//! - Non-mutating: the report never creates a file or directory, never
//!   starts the agent, never activates a D-Bus service (only names that
//!   already have an owner are asked anything) and never prompts.
//! - Truthful distinction between architectural OS limits (Wayland input
//!   isolation) and what this build implements; unmeasured is never zero.
//! - Every v1 JSON field is kept. Report v2 adds `schema`, the live probes
//!   (`probes`: logind session, idle source, X11 extensions, Screenshot
//!   portal, Secret Service owner and lock state, StatusNotifierWatcher,
//!   notification server, GUI libraries, unit and autostart state, the
//!   agent's `hello`, the legacy plaintext credentials file, the opt-in
//!   file store) and `problems`, of which the blocking ones make
//!   `doctor --strict` exit 1.

use crate::paths::LinuxPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The v2 report's schema name.
pub const DOCTOR_SCHEMA: &str = "dosi-tracker.doctor/2";

/// Session type detected from environment inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    Wayland,
    X11,
    Headless,
    Unknown(String),
}

impl SessionType {
    pub fn as_str(&self) -> &str {
        match self {
            SessionType::Wayland => "wayland",
            SessionType::X11 => "x11",
            SessionType::Headless => "headless",
            SessionType::Unknown(s) => s.as_str(),
        }
    }
}

/// Explicitly injectable session environment for side-effect-free diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEnvironment {
    pub xdg_session_type: Option<String>,
    pub wayland_display: Option<String>,
    pub display: Option<String>,
    pub xdg_current_desktop: Option<String>,
    pub user: Option<String>,
    pub home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_runtime_dir: Option<PathBuf>,
    pub dbus_session_bus_address: Option<String>,
}

impl SessionEnvironment {
    /// Collect environment variables from the current process.
    pub fn from_env() -> Self {
        let text = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let path = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self {
            xdg_session_type: text("XDG_SESSION_TYPE"),
            wayland_display: text("WAYLAND_DISPLAY"),
            display: text("DISPLAY"),
            xdg_current_desktop: text("XDG_CURRENT_DESKTOP"),
            user: text("USER"),
            home: path("HOME"),
            xdg_config_home: path("XDG_CONFIG_HOME"),
            xdg_data_home: path("XDG_DATA_HOME"),
            xdg_state_home: path("XDG_STATE_HOME"),
            xdg_runtime_dir: path("XDG_RUNTIME_DIR"),
            dbus_session_bus_address: text("DBUS_SESSION_BUS_ADDRESS"),
        }
    }

    /// Detect session type based purely on environment observations.
    pub fn detect_session(&self) -> SessionType {
        if let Some(ref st) = self.xdg_session_type {
            let lower = st.to_ascii_lowercase();
            if lower == "wayland" {
                return SessionType::Wayland;
            }
            if lower == "x11" {
                return SessionType::X11;
            }
            if lower == "tty" {
                return SessionType::Headless;
            }
            if !lower.is_empty() {
                return SessionType::Unknown(lower);
            }
        }

        if self.wayland_display.is_some() {
            return SessionType::Wayland;
        }

        if self.display.is_some() {
            return SessionType::X11;
        }

        SessionType::Headless
    }

    /// The XDG paths of this environment (resolution only).
    pub fn paths(&self) -> LinuxPaths {
        LinuxPaths::resolve(
            self.home.as_deref(),
            self.xdg_config_home.as_deref(),
            self.xdg_data_home.as_deref(),
            self.xdg_state_home.as_deref(),
        )
        .with_runtime_dir(self.xdg_runtime_dir.as_deref())
    }
}

/// Status of an individual telemetry capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticStatus {
    /// Not built in this version.
    NotImplemented,
    /// Operating system or display server architecture fundamentally prohibits
    /// unprivileged background access without security compromise.
    Unsupported,
    /// Built, and the session type allows it.
    Supported,
    /// Not checked: no graphical session to check it in.
    NotChecked,
}

/// Architectural OS support tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArchitecturalSupport {
    SupportedUnconfined,
    SupportedViaPortal,
    UnsupportedOnWayland,
    NoDisplayServer,
    SupportedProcFs,
    SupportedV4l2,
}

/// Detailed diagnostics for a single capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDiagnosis {
    pub adapter_status: DiagnosticStatus,
    pub architectural_support: ArchitecturalSupport,
    pub architectural_notes: String,
    pub adapter_notes: String,
}

/// Static client metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientMetadata {
    pub name: String,
    pub version: String,
    pub release_tier: String,
}

/// Resolved path diagnostics with non-mutating existence indicators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathDiagnosis {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub consent_file: PathBuf,
    pub config_dir_exists: bool,
    pub data_dir_exists: bool,
    pub state_dir_exists: bool,
    pub consent_file_exists: bool,
}

/// Inventory of native collection and sync adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInventory {
    pub screen_capture: DiagnosticStatus,
    pub input_counter: DiagnosticStatus,
    pub window_sampler: DiagnosticStatus,
    pub webcam_capture: DiagnosticStatus,
    pub offline_queue: DiagnosticStatus,
    pub backend_sync: DiagnosticStatus,
}

/// Diagnosis of active credential storage backend and fallback status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStoreDiagnosis {
    /// `SecretService`, `FileFallback` (the opt-in file store) or
    /// `Unavailable` (`auto` without a session bus).
    pub backend: String,
    pub path: Option<PathBuf>,
    pub fallback_active: bool,
    pub warning: Option<String>,
}

/// Complete diagnostic report emitted by `dosi-tracker doctor [--json]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// `dosi-tracker.doctor/2` (absent in v1 reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub client: ClientMetadata,
    pub session: SessionType,
    pub desktop_environment: Option<String>,
    pub user: Option<String>,
    pub paths: PathDiagnosis,
    pub capabilities: CapabilityMatrix,
    pub adapters: AdapterInventory,
    pub privacy_invariants: PrivacyInvariants,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_store: Option<CredentialStoreDiagnosis>,
    /// v2: what the live checks found (absent when they did not run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probes: Option<LiveProbes>,
    /// v2: what needs attention; `blocking` ones fail `doctor --strict`.
    #[serde(default)]
    pub problems: Vec<Problem>,
}

/// Matrix of all telemetry capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityMatrix {
    pub screen_capture: CapabilityDiagnosis,
    pub keyboard_counting: CapabilityDiagnosis,
    pub mouse_counting: CapabilityDiagnosis,
    pub active_window: CapabilityDiagnosis,
    pub running_programs: CapabilityDiagnosis,
    pub webcam_photo: CapabilityDiagnosis,
}

/// Truthful verification invariants declared by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyInvariants {
    pub unprivileged_process: bool,
    pub anti_keylogger_policy: bool,
    pub zero_synthetic_telemetry: bool,
    pub default_deny_consent: bool,
}

// ---- v2: live probes and problems -----------------------------------------------------------

/// How serious a [`Problem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Tracking cannot work as configured: `doctor --strict` exits 1.
    Blocking,
    /// Works, with a limit worth knowing.
    Warning,
}

/// Something that needs attention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Problem {
    pub fn blocking(code: &str, message: impl Into<String>, hint: Option<&str>) -> Self {
        Self {
            severity: Severity::Blocking,
            code: code.into(),
            message: message.into(),
            hint: hint.map(str::to_owned),
        }
    }

    pub fn warning(code: &str, message: impl Into<String>, hint: Option<&str>) -> Self {
        Self {
            severity: Severity::Warning,
            code: code.into(),
            message: message.into(),
            hint: hint.map(str::to_owned),
        }
    }
}

/// logind's view of this login (design 4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogindProbe {
    pub reachable: bool,
    /// `x11`, `wayland`, `tty` or `none`, as the agent would decide.
    pub session_kind: String,
    pub source: String,
    pub session_id: Option<String>,
    pub desktop: Option<String>,
    pub reason: Option<String>,
}

/// The X server and the extensions the X11 sensors need.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct X11Probe {
    pub display: String,
    pub connected: bool,
    /// The XInput version (`2.2` or later is needed), when present.
    pub xinput: Option<String>,
    pub screensaver: bool,
    pub randr: bool,
    pub error: Option<String>,
}

/// A well-known D-Bus name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusNameProbe {
    /// The name has an owner now.
    pub owner: bool,
    /// The bus can start a service for it.
    pub activatable: bool,
}

/// The Screenshot portal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalProbe {
    pub present: bool,
    /// `org.freedesktop.portal.Screenshot` version, when the portal runs.
    pub screenshot_version: Option<u32>,
}

/// The Secret Service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretServiceProbe {
    pub owner: bool,
    pub activatable: bool,
    /// The default collection is locked (only asked when a service runs).
    pub locked: Option<bool>,
}

/// The notification server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationsProbe {
    pub owner: bool,
    pub server: Option<String>,
}

/// One GUI library the window needs at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryProbe {
    pub name: String,
    pub found: bool,
}

/// The systemd user unit and the login autostart entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceProbe {
    pub user_manager: bool,
    pub unit_loaded: bool,
    pub unit_active: bool,
    pub autostart_enabled: bool,
}

/// The running agent, as its `hello` says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonProbe {
    pub running: bool,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub protocol: Option<u32>,
    pub error: Option<String>,
}

/// The signed-in account's queue, read with the read-only inspector.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueProbe {
    pub workspace_id: String,
    /// Blocks waiting to upload.
    pub pending: u64,
    /// Blocks the server refused, kept 14 days for diagnostics.
    pub parked: u64,
    pub error: Option<String>,
}

/// Everything the live checks found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveProbes {
    pub running_as_root: bool,
    pub runtime_dir: Option<PathBuf>,
    pub session_bus: bool,
    pub logind: LogindProbe,
    /// `xi2`, `x11-screensaver`, `mutter`, `screensaver`, `logind`; `None`
    /// means manual mode (no away detection).
    pub idle_source: Option<String>,
    pub x11: Option<X11Probe>,
    pub portal: PortalProbe,
    pub secret_service: SecretServiceProbe,
    pub sni_watcher: BusNameProbe,
    pub notifications: NotificationsProbe,
    pub gui_libraries: Vec<LibraryProbe>,
    pub service: ServiceProbe,
    pub daemon: DaemonProbe,
    /// The queue of the account in `session.json` (none when signed out).
    #[serde(default)]
    pub queue: Option<QueueProbe>,
    /// The foundation login's plaintext credentials file, if it is still there.
    pub legacy_credentials_file: Option<PathBuf>,
    /// Set when the opt-in file credential store is configured.
    pub file_store_warning: Option<String>,
    pub config_issues: Vec<String>,
}

/// What [`DoctorReport::generate_with`] knows besides the environment.
#[derive(Debug, Clone, Default)]
pub struct DoctorInputs {
    /// `credentials.store` (default `auto`).
    pub credentials_store: Option<String>,
    /// The live checks' findings (Linux; `None` skips them).
    pub probes: Option<LiveProbes>,
}

fn diagnosis(
    status: DiagnosticStatus,
    support: ArchitecturalSupport,
    architectural: &str,
    adapter: &str,
) -> CapabilityDiagnosis {
    CapabilityDiagnosis {
        adapter_status: status,
        architectural_support: support,
        architectural_notes: architectural.to_string(),
        adapter_notes: adapter.to_string(),
    }
}

const NO_DISPLAY: &str = "No graphical display server detected.";
const NOT_CHECKED: &str = "Not checked: no graphical session is attached.";

impl DoctorReport {
    /// The report for `env` without live checks (the capability matrix,
    /// paths and invariants only).
    pub fn generate(env: &SessionEnvironment) -> Self {
        Self::generate_with(env, &DoctorInputs::default())
    }

    /// The report for `env`, with the live checks in `inputs`.
    pub fn generate_with(env: &SessionEnvironment, inputs: &DoctorInputs) -> Self {
        use ArchitecturalSupport as A;
        use DiagnosticStatus as S;
        let session = env.detect_session();
        let paths = env.paths();

        let path_diag = PathDiagnosis {
            config_dir: paths.config_dir.clone(),
            data_dir: paths.data_dir.clone(),
            state_dir: paths.state_dir.clone(),
            consent_file: paths.consent_file.clone(),
            config_dir_exists: paths.config_dir_exists(),
            data_dir_exists: paths.data_dir_exists(),
            state_dir_exists: paths.state_dir.is_dir(),
            consent_file_exists: paths.consent_file_exists(),
        };

        let screen_capture = match session {
            SessionType::Wayland => diagnosis(
                S::Supported,
                A::SupportedViaPortal,
                "Wayland screenshots go through org.freedesktop.portal.Screenshot; the desktop may ask for permission.",
                "Screenshot portal adapter (non-interactive request; the returned file is deleted after decoding).",
            ),
            SessionType::X11 => diagnosis(
                S::Supported,
                A::SupportedUnconfined,
                "X11 root window capture is permitted through GetImage.",
                "X11 GetImage of the monitor showing the active window, JPEG ladder and blur.",
            ),
            _ => diagnosis(S::NotChecked, A::NoDisplayServer, NO_DISPLAY, NOT_CHECKED),
        };
        let input = |what: &str, event: &str| {
            match session {
            SessionType::Wayland => diagnosis(
                S::Unsupported,
                A::UnsupportedOnWayland,
                &format!("Wayland isolates unprivileged {what} input to the focused surface. Anti-keylogger invariant prohibits root or kernel input-device access."),
                &format!("Global {what} counting is architecturally UNSUPPORTED on Wayland. Reported as null, never zero."),
            ),
            SessionType::X11 => diagnosis(
                S::Supported,
                A::SupportedUnconfined,
                &format!("X11 raw {what} events are available through XInput2 ({event})."),
                &format!("XInput2 listener counting {what} events per second; synthetic (XTEST) events are counted as injected only."),
            ),
            _ => diagnosis(S::NotChecked, A::NoDisplayServer, NO_DISPLAY, NOT_CHECKED),
        }
        };
        let keyboard_counting = input("keyboard", "XI_RawKeyPress");
        let mouse_counting = input("mouse", "XI_RawButtonPress");
        let active_window = match session {
            SessionType::Wayland => diagnosis(
                S::Unsupported,
                A::UnsupportedOnWayland,
                "Wayland window hierarchy is private to the compositor; no standard unprivileged active window API exists on GNOME/KDE.",
                "Active window tracking is architecturally UNSUPPORTED on Wayland. Reported as empty.",
            ),
            SessionType::X11 => diagnosis(
                S::Supported,
                A::SupportedUnconfined,
                "EWMH _NET_ACTIVE_WINDOW and _NET_WM_NAME architecturally permitted under X11.",
                "Native X11 active window adapter wired via EWMH and WM_CLASS.",
            ),
            _ => diagnosis(S::NotChecked, A::NoDisplayServer, NO_DISPLAY, NOT_CHECKED),
        };
        let running_programs = diagnosis(
            S::Supported,
            A::SupportedProcFs,
            "User-owned process enumeration architecturally permitted via /proc scanning.",
            "Own-user programs that own a window (X11) or run in an app scope (Wayland), names only.",
        );
        let webcam_photo = diagnosis(
            S::NotImplemented,
            A::SupportedV4l2,
            "Video capture architecturally permitted via V4L2 (/dev/video*) or PipeWire camera portal.",
            "Webcam capture is not available on Linux in this version and cannot be consented to.",
        );

        let window_sampler_status = active_window.adapter_status.clone();
        let input_counter = keyboard_counting.adapter_status.clone();
        let screen_status = screen_capture.adapter_status.clone();

        let credential_store = credential_store_diagnosis(env, &paths, inputs);
        let mut report = Self {
            schema: Some(DOCTOR_SCHEMA.to_string()),
            client: ClientMetadata {
                name: "dosi-tracker-linux".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                release_tier: "v1".to_string(),
            },
            session,
            desktop_environment: env.xdg_current_desktop.clone(),
            user: env.user.clone(),
            paths: path_diag,
            capabilities: CapabilityMatrix {
                screen_capture,
                keyboard_counting,
                mouse_counting,
                active_window,
                running_programs,
                webcam_photo,
            },
            adapters: AdapterInventory {
                screen_capture: screen_status,
                input_counter,
                window_sampler: window_sampler_status,
                webcam_capture: DiagnosticStatus::NotImplemented,
                offline_queue: DiagnosticStatus::Supported,
                backend_sync: DiagnosticStatus::Supported,
            },
            privacy_invariants: PrivacyInvariants {
                unprivileged_process: true,
                anti_keylogger_policy: true,
                zero_synthetic_telemetry: true,
                default_deny_consent: true,
            },
            credential_store: Some(credential_store),
            probes: inputs.probes.clone(),
            problems: Vec::new(),
        };
        report.problems = report.find_problems();
        report
    }

    /// The problems the report shows, blocking ones first.
    fn find_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if let Some(store) = &self.credential_store {
            if store.backend == "Unavailable" {
                problems.push(Problem::blocking(
                    "no_secret_service",
                    "No Secret Service is reachable, so signing in cannot store its tokens.",
                    Some(crate::credentials::NO_SECRET_SERVICE_HINT),
                ));
            }
        }
        let Some(probes) = &self.probes else {
            return problems;
        };
        if probes.running_as_root {
            problems.push(Problem::blocking(
                "root",
                "Running as root: the agent refuses to run as root.",
                Some("run dosi-tracker as the signed-in user"),
            ));
        }
        if probes.runtime_dir.is_none() {
            problems.push(Problem::blocking(
                "no_runtime_dir",
                "XDG_RUNTIME_DIR is not set: the agent needs a login session's runtime directory.",
                None,
            ));
        }
        if let Some(path) = &probes.legacy_credentials_file {
            problems.push(Problem::blocking(
                "legacy_credentials_file",
                format!(
                    "{} holds a sign-in from an early version in plain text.",
                    path.display()
                ),
                Some("sign in again with dosi-tracker login (the file is then deleted)"),
            ));
        }
        if probes.daemon.error.is_some() {
            problems.push(Problem::blocking(
                "daemon_unreachable",
                format!(
                    "The running agent cannot be used: {}",
                    probes.daemon.error.as_deref().unwrap_or_default()
                ),
                Some("dosi-tracker service restart"),
            ));
        }
        if let Some(queue) = probes.queue.as_ref().filter(|q| q.parked > 0) {
            problems.push(Problem::warning(
                "blocks_not_accepted",
                format!(
                    "{} queued blocks were not accepted by the server; they are kept for 14 days.",
                    queue.parked
                ),
                Some("dosi-tracker queue list --parked"),
            ));
        }
        if probes.secret_service.locked == Some(true) {
            problems.push(Problem::warning(
                "keyring_locked",
                "The keyring is locked: uploads wait until it is unlocked.",
                Some("dosi-tracker unlock"),
            ));
        }
        if let Some(warning) = &probes.file_store_warning {
            problems.push(Problem::warning(
                "file_credential_store",
                warning.clone(),
                None,
            ));
        }
        match self.session {
            SessionType::Headless | SessionType::Unknown(_) => problems.push(Problem::warning(
                "no_graphical_session",
                "No graphical session: sign-in, queue and sync work, tracking does not.",
                None,
            )),
            SessionType::Wayland => {
                problems.push(Problem::warning(
                    "wayland_limits",
                    "Wayland: keyboard and mouse counts, activity and the active window are not available (an X11 session records them).",
                    None,
                ));
                if !probes.portal.present {
                    problems.push(Problem::warning(
                        "no_screenshot_portal",
                        "No xdg-desktop-portal: screenshots are not available.",
                        Some("install xdg-desktop-portal and a backend for your desktop"),
                    ));
                }
            }
            SessionType::X11 => {
                if let Some(x11) = &probes.x11 {
                    if !x11.connected {
                        problems.push(Problem::warning(
                            "x11_unreachable",
                            format!(
                                "The X server {} cannot be reached{}.",
                                x11.display,
                                x11.error
                                    .as_deref()
                                    .map(|e| format!(": {e}"))
                                    .unwrap_or_default()
                            ),
                            None,
                        ));
                    } else if x11.xinput.is_none() {
                        problems.push(Problem::warning(
                            "no_xinput2",
                            "The X server has no XInput 2.2: keyboard and mouse counts are not available.",
                            None,
                        ));
                    }
                }
            }
        }
        if matches!(self.session, SessionType::X11 | SessionType::Wayland)
            && probes.idle_source.is_none()
        {
            problems.push(Problem::warning(
                "manual_mode",
                "No idle source: away time cannot be detected, so tracking never starts by itself (manual mode).",
                None,
            ));
        }
        if !probes.sni_watcher.owner {
            problems.push(Problem::warning(
                "no_tray",
                "No tray host (StatusNotifierWatcher): a notification shows that tracking is on instead.",
                Some("on GNOME, enable the AppIndicator extension"),
            ));
        }
        if !probes.notifications.owner && probes.session_bus {
            problems.push(Problem::warning(
                "no_notifications",
                "No notification server: Dosi Tracker cannot tell you about away time or problems.",
                None,
            ));
        }
        let missing: Vec<&str> = probes
            .gui_libraries
            .iter()
            .filter(|l| !l.found)
            .map(|l| l.name.as_str())
            .collect();
        if !missing.is_empty() {
            problems.push(Problem::warning(
                "gui_libraries_missing",
                format!(
                    "The window needs libraries that are missing: {}.",
                    missing.join(", ")
                ),
                None,
            ));
        }
        for issue in &probes.config_issues {
            problems.push(Problem::warning("config_issue", issue.clone(), None));
        }
        problems.sort_by_key(|p| p.severity != Severity::Blocking);
        problems
    }

    /// Does anything block tracking?
    pub fn has_blocking_problems(&self) -> bool {
        self.problems
            .iter()
            .any(|p| p.severity == Severity::Blocking)
    }

    /// Serialize report to JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Render human-readable text output for terminal display.
    pub fn to_pretty_text(&self) -> String {
        let mut out = String::new();
        out.push_str("=======================================================\n");
        out.push_str(&format!(
            "Dosi-Tracker Linux Client Diagnostics (v{} - {})\n",
            self.client.version, self.client.release_tier
        ));
        out.push_str("=======================================================\n\n");

        out.push_str("[Session Environment]\n");
        out.push_str(&format!("  Session Type: {}\n", self.session.as_str()));
        out.push_str(&format!(
            "  Desktop Environment: {}\n",
            self.desktop_environment
                .as_deref()
                .unwrap_or("none / unknown")
        ));
        out.push_str(&format!(
            "  User: {}\n\n",
            self.user.as_deref().unwrap_or("unknown")
        ));

        out.push_str("[Storage Paths (Non-Mutating Inspection)]\n");
        out.push_str(&format!(
            "  Config Directory: {} (exists: {})\n",
            self.paths.config_dir.display(),
            self.paths.config_dir_exists
        ));
        out.push_str(&format!(
            "  Data Directory:   {} (exists: {})\n",
            self.paths.data_dir.display(),
            self.paths.data_dir_exists
        ));
        out.push_str(&format!(
            "  Consent File:     {} (exists: {})\n\n",
            self.paths.consent_file.display(),
            self.paths.consent_file_exists
        ));

        out.push_str("[Telemetry Capability Matrix]\n");
        for (name, cap) in [
            ("Screen Capture:   ", &self.capabilities.screen_capture),
            ("Keyboard Counting:", &self.capabilities.keyboard_counting),
            ("Mouse Counting:   ", &self.capabilities.mouse_counting),
            ("Active Window:    ", &self.capabilities.active_window),
            ("Running Programs: ", &self.capabilities.running_programs),
            ("Webcam Photo:     ", &self.capabilities.webcam_photo),
        ] {
            out.push_str(&format!(
                "  {name} {:?} ({:?})\n",
                cap.adapter_status, cap.architectural_support
            ));
        }
        out.push('\n');

        out.push_str("[Native Adapters Inventory]\n");
        out.push_str(&format!(
            "  Screen Capture: {:?}\n",
            self.adapters.screen_capture
        ));
        out.push_str(&format!(
            "  Input Counter:  {:?}\n",
            self.adapters.input_counter
        ));
        out.push_str(&format!(
            "  Window Sampler: {:?}\n",
            self.adapters.window_sampler
        ));
        out.push_str(&format!(
            "  Webcam Capture: {:?}\n",
            self.adapters.webcam_capture
        ));
        out.push_str(&format!(
            "  Offline Queue:  {:?}\n",
            self.adapters.offline_queue
        ));
        out.push_str(&format!(
            "  Backend Sync:   {:?}\n\n",
            self.adapters.backend_sync
        ));

        out.push_str("[Privacy Invariants]\n");
        out.push_str(&format!(
            "  Unprivileged Process:      {}\n",
            self.privacy_invariants.unprivileged_process
        ));
        out.push_str(&format!(
            "  Anti-Keylogger Policy:     {}\n",
            self.privacy_invariants.anti_keylogger_policy
        ));
        out.push_str(&format!(
            "  Zero Synthetic Telemetry:  {}\n",
            self.privacy_invariants.zero_synthetic_telemetry
        ));
        out.push_str(&format!(
            "  Default-Deny Consent Core: {}\n",
            self.privacy_invariants.default_deny_consent
        ));

        if let Some(ref cred) = self.credential_store {
            out.push_str("\n[Credential Storage]\n");
            out.push_str(&format!("  Backend: {}\n", cred.backend));
            if let Some(ref p) = cred.path {
                out.push_str(&format!("  Path:    {}\n", p.display()));
            }
            if let Some(ref warn) = cred.warning {
                out.push_str(&format!("  {}\n", warn));
            }
        }

        if let Some(probes) = &self.probes {
            let yes = |b: bool| if b { "yes" } else { "no" };
            out.push_str("\n[Live Checks]\n");
            out.push_str(&format!(
                "  Session (logind):   {} ({}){}\n",
                probes.logind.session_kind,
                probes.logind.source,
                probes
                    .logind
                    .reason
                    .as_deref()
                    .map(|r| format!(" - {r}"))
                    .unwrap_or_default()
            ));
            out.push_str(&format!(
                "  Idle source:        {}\n",
                probes
                    .idle_source
                    .as_deref()
                    .unwrap_or("none (manual mode)")
            ));
            if let Some(x11) = &probes.x11 {
                out.push_str(&format!(
                    "  X11 {}:            connected {}, XInput {}, MIT-SCREEN-SAVER {}, RANDR {}\n",
                    x11.display,
                    yes(x11.connected),
                    x11.xinput.as_deref().unwrap_or("no"),
                    yes(x11.screensaver),
                    yes(x11.randr)
                ));
            }
            out.push_str(&format!(
                "  Screenshot portal:  {}\n",
                match probes.portal.screenshot_version {
                    Some(v) => format!("yes (version {v})"),
                    None => yes(probes.portal.present).to_string(),
                }
            ));
            out.push_str(&format!(
                "  Secret Service:     {}{}\n",
                yes(probes.secret_service.owner || probes.secret_service.activatable),
                match probes.secret_service.locked {
                    Some(true) => " (locked)",
                    Some(false) => " (unlocked)",
                    None => "",
                }
            ));
            out.push_str(&format!(
                "  Tray host:          {}\n",
                yes(probes.sni_watcher.owner)
            ));
            out.push_str(&format!(
                "  Notifications:      {}\n",
                probes
                    .notifications
                    .server
                    .clone()
                    .unwrap_or_else(|| yes(probes.notifications.owner).to_string())
            ));
            out.push_str(&format!(
                "  Service:            user manager {}, unit {}, running {}, starts at login {}\n",
                yes(probes.service.user_manager),
                yes(probes.service.unit_loaded),
                yes(probes.service.unit_active),
                yes(probes.service.autostart_enabled)
            ));
            out.push_str(&format!(
                "  Agent:              {}\n",
                match (&probes.daemon.version, probes.daemon.pid) {
                    (Some(v), Some(pid)) => format!("running (version {v}, pid {pid})"),
                    _ => "not running".to_string(),
                }
            ));
        }
        if !self.problems.is_empty() {
            out.push_str("\n[Problems]\n");
            for problem in &self.problems {
                out.push_str(&format!(
                    "  [{}] {}\n",
                    match problem.severity {
                        Severity::Blocking => "blocking",
                        Severity::Warning => "warning",
                    },
                    problem.message
                ));
                if let Some(hint) = &problem.hint {
                    out.push_str(&format!("      hint: {hint}\n"));
                }
            }
        }

        out
    }
}

/// The credential store the agent would use, from the configuration and
/// whether a session bus exists (the live Secret Service check is a probe).
fn credential_store_diagnosis(
    env: &SessionEnvironment,
    paths: &LinuxPaths,
    inputs: &DoctorInputs,
) -> CredentialStoreDiagnosis {
    let setting = inputs
        .credentials_store
        .as_deref()
        .unwrap_or("auto")
        .to_ascii_lowercase();
    if setting == "file" {
        return CredentialStoreDiagnosis {
            backend: "FileFallback".to_string(),
            path: Some(paths.credentials_file()),
            fallback_active: true,
            warning: Some(
                "WARNING: File credential store fallback active (credentials.store = \"file\"): tokens are kept in a 0600 file, never the password. Freedesktop SecretService is recommended for production security."
                    .to_string(),
            ),
        };
    }
    let bus = cfg!(target_os = "linux") && env.dbus_session_bus_address.is_some();
    if bus || setting == "secret-service" {
        CredentialStoreDiagnosis {
            backend: "SecretService".to_string(),
            path: None,
            fallback_active: false,
            warning: None,
        }
    } else {
        CredentialStoreDiagnosis {
            backend: "Unavailable".to_string(),
            path: None,
            fallback_active: false,
            warning: Some(
                "WARNING: no session bus, so no Secret Service: signing in will fail. Unlock a keyring in a desktop session, or opt in to the file store with credentials.store = \"file\"."
                    .to_string(),
            ),
        }
    }
}

/// The live checks (Linux): read-only questions with short timeouts.
#[cfg(target_os = "linux")]
pub mod live {
    use super::*;
    use std::time::Duration;
    use zbus::blocking::Connection;

    /// How long one D-Bus call may take.
    const CALL: Duration = Duration::from_secs(2);

    /// The libraries the window dlopens.
    pub const GUI_LIBRARIES: [&str; 6] = [
        "libEGL.so.1",
        "libGL.so.1",
        "libxkbcommon-x11.so.0",
        "libX11-xcb.so.1",
        "libXcursor.so.1",
        "libXi.so.6",
    ];

    fn connect(address: &str) -> Option<Connection> {
        zbus::blocking::connection::Builder::address(address)
            .ok()?
            .method_timeout(CALL)
            .build()
            .ok()
    }

    /// Does `name` have an owner (never activating it)? And can it be activated?
    pub fn bus_name(conn: &Connection, name: &str) -> BusNameProbe {
        let owner = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(name,),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize::<bool>().ok())
            .unwrap_or(false);
        let activatable = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListActivatableNames",
                &(),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize::<Vec<String>>().ok())
            .is_some_and(|names| names.iter().any(|n| n == name));
        BusNameProbe { owner, activatable }
    }

    fn property<T>(
        conn: &Connection,
        dest: &str,
        path: &str,
        interface: &str,
        name: &str,
    ) -> Option<T>
    where
        T: TryFrom<zbus::zvariant::OwnedValue>,
    {
        let reply = conn
            .call_method(
                Some(dest),
                path,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(interface, name),
            )
            .ok()?;
        let value: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
        T::try_from(value).ok()
    }

    fn secret_service(conn: &Connection) -> SecretServiceProbe {
        let name = bus_name(conn, crate::credentials::SECRET_SERVICE_NAME);
        let locked = name.owner.then(|| {
            let path: zbus::zvariant::OwnedObjectPath = conn
                .call_method(
                    Some(crate::credentials::SECRET_SERVICE_NAME),
                    "/org/freedesktop/secrets",
                    Some("org.freedesktop.Secret.Service"),
                    "ReadAlias",
                    &("default",),
                )
                .ok()?
                .body()
                .deserialize()
                .ok()?;
            if path.as_str() == "/" {
                return None;
            }
            property::<bool>(
                conn,
                crate::credentials::SECRET_SERVICE_NAME,
                path.as_str(),
                "org.freedesktop.Secret.Collection",
                "Locked",
            )
        });
        SecretServiceProbe {
            owner: name.owner,
            activatable: name.activatable,
            locked: locked.flatten(),
        }
    }

    fn portal(conn: &Connection) -> PortalProbe {
        let name = bus_name(conn, "org.freedesktop.portal.Desktop");
        let version = name
            .owner
            .then(|| {
                property::<u32>(
                    conn,
                    "org.freedesktop.portal.Desktop",
                    "/org/freedesktop/portal/desktop",
                    "org.freedesktop.portal.Screenshot",
                    "version",
                )
            })
            .flatten();
        PortalProbe {
            present: name.owner,
            screenshot_version: version,
        }
    }

    fn notifications(conn: &Connection) -> NotificationsProbe {
        let name = bus_name(conn, crate::agent::notify::NOTIFICATIONS_BUS_NAME);
        let server = name
            .owner
            .then(|| {
                let reply = conn
                    .call_method(
                        Some(crate::agent::notify::NOTIFICATIONS_BUS_NAME),
                        crate::agent::notify::NOTIFICATIONS_PATH,
                        Some(crate::agent::notify::NOTIFICATIONS_INTERFACE),
                        "GetServerInformation",
                        &(),
                    )
                    .ok()?;
                let (name, vendor, version, _spec): (String, String, String, String) =
                    reply.body().deserialize().ok()?;
                Some(format!("{name} {version} ({vendor})"))
            })
            .flatten();
        NotificationsProbe {
            owner: name.owner,
            server,
        }
    }

    /// Connect to `display` and ask for the extensions the sensors use.
    pub fn x11(display: &str) -> X11Probe {
        use x11rb::connection::RequestConnection;
        use x11rb::protocol::xinput::ConnectionExt as _;
        let mut probe = X11Probe {
            display: display.to_string(),
            ..X11Probe::default()
        };
        let conn = match x11rb::rust_connection::RustConnection::connect(Some(display)) {
            Ok((conn, _)) => conn,
            Err(e) => {
                probe.error = Some(e.to_string());
                return probe;
            }
        };
        probe.connected = true;
        let present =
            |name: &'static str| conn.extension_information(name).ok().flatten().is_some();
        probe.screensaver = present(x11rb::protocol::screensaver::X11_EXTENSION_NAME);
        probe.randr = present(x11rb::protocol::randr::X11_EXTENSION_NAME);
        if present(x11rb::protocol::xinput::X11_EXTENSION_NAME) {
            probe.xinput = conn
                .xinput_xi_query_version(2, 2)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .filter(|v| (v.major_version, v.minor_version) >= (2, 2))
                .map(|v| format!("{}.{}", v.major_version, v.minor_version));
        }
        probe
    }

    /// Can the window's libraries be loaded?
    pub fn gui_libraries() -> Vec<LibraryProbe> {
        GUI_LIBRARIES
            .iter()
            .map(|name| {
                let c_name = std::ffi::CString::new(*name).expect("no NUL in library names");
                // SAFETY: dlopen with a valid C string; the handle is closed at once.
                let found = unsafe {
                    let handle = libc::dlopen(c_name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
                    if handle.is_null() {
                        false
                    } else {
                        libc::dlclose(handle);
                        true
                    }
                };
                LibraryProbe {
                    name: (*name).to_string(),
                    found,
                }
            })
            .collect()
    }

    /// The queue of the account `session.json` names, read-only (nothing is
    /// created, migrated or checkpointed).
    pub fn queue(paths: &LinuxPaths) -> Option<QueueProbe> {
        let text = std::fs::read_to_string(paths.session_file()).ok()?;
        let session: crate::agent::session_store::SessionData = serde_json::from_str(&text).ok()?;
        let workspace = session.workspace?.id;
        let scope = crate::outbox::OutboxScope::new(session.account?, workspace.clone()).ok()?;
        let db = paths.outbox_db();
        if !db.is_file() {
            return Some(QueueProbe {
                workspace_id: workspace,
                ..QueueProbe::default()
            });
        }
        Some(match crate::outbox::inspect_outbox_status(&db, &scope) {
            Ok(report) => QueueProbe {
                workspace_id: workspace,
                pending: report.durable_queue.queued_count,
                parked: report.durable_queue.parked_count,
                error: None,
            },
            Err(e) => QueueProbe {
                workspace_id: workspace,
                error: Some(e.to_string()),
                ..QueueProbe::default()
            },
        })
    }

    /// Everything the live checks need.
    pub struct ProbeContext<'a> {
        pub env: &'a SessionEnvironment,
        pub vars: &'a [(String, String)],
        pub paths: &'a LinuxPaths,
        /// The agent's socket.
        pub socket: Option<PathBuf>,
        pub credentials_store: String,
        pub config_issues: Vec<String>,
    }

    /// Run every live check.
    pub fn run(ctx: &ProbeContext<'_>) -> LiveProbes {
        let mut probes = LiveProbes {
            // SAFETY: geteuid has no preconditions and cannot fail.
            running_as_root: unsafe { libc::geteuid() } == 0,
            runtime_dir: ctx.env.xdg_runtime_dir.clone(),
            config_issues: ctx.config_issues.clone(),
            ..LiveProbes::default()
        };

        let session_env =
            crate::tracking::session_probe::SessionEnv::from_vars(ctx.vars.iter().cloned());
        let system = connect(session_env.system_bus_address());
        let info = crate::tracking::session_probe::probe(
            &session_env,
            system.as_ref(),
            // SAFETY: as above.
            unsafe { libc::geteuid() },
        );
        probes.logind = LogindProbe {
            reachable: system.is_some(),
            session_kind: serde_json::to_value(info.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            source: format!("{:?}", info.source).to_ascii_lowercase(),
            session_id: info.session_id.clone(),
            desktop: info.desktop.clone(),
            reason: info.reason.clone(),
        };

        let session = ctx
            .env
            .dbus_session_bus_address
            .as_deref()
            .and_then(connect);
        probes.session_bus = session.is_some();
        if let Some(conn) = &session {
            probes.secret_service = secret_service(conn);
            probes.portal = portal(conn);
            probes.sni_watcher = bus_name(conn, "org.kde.StatusNotifierWatcher");
            probes.notifications = notifications(conn);
        }

        probes.x11 = match (info.kind, info.display.as_deref()) {
            (crate::agent::state::SessionKind::X11, Some(display)) => Some(x11(display)),
            _ => None,
        };
        probes.idle_source = match info.kind {
            crate::agent::state::SessionKind::X11 => probes.x11.as_ref().and_then(|x| {
                if x.xinput.is_some() {
                    Some("xi2".to_string())
                } else if x.screensaver {
                    Some("x11-screensaver".to_string())
                } else {
                    None
                }
            }),
            crate::agent::state::SessionKind::Wayland => {
                let owned = |name: &str| session.as_ref().is_some_and(|c| bus_name(c, name).owner);
                if crate::tracking::dbus_idle::is_gnome(info.desktop.as_deref())
                    && owned("org.gnome.Mutter.IdleMonitor")
                {
                    Some("mutter".to_string())
                } else if owned("org.freedesktop.ScreenSaver") {
                    Some("screensaver".to_string())
                } else if system.is_some() && info.session_path.is_some() {
                    Some("logind".to_string())
                } else {
                    None
                }
            }
            _ => None,
        };
        probes.gui_libraries = gui_libraries();

        let systemctl = crate::service::Systemctl::new(crate::service::ServiceEnv::from_vars(
            ctx.vars.iter().cloned(),
        ))
        .with_timeout(Duration::from_secs(3));
        let service = crate::service::status(ctx.paths, &systemctl);
        probes.service = ServiceProbe {
            user_manager: service.user_manager,
            unit_loaded: service.installed,
            unit_active: service.active,
            autostart_enabled: service.enabled_at_login,
        };

        if let Some(socket) = &ctx.socket {
            match crate::ipc::client::Client::connect(socket, "cli") {
                Ok(client) => {
                    let hello = client.hello();
                    probes.daemon = DaemonProbe {
                        running: true,
                        version: Some(hello.daemon_version.clone()),
                        pid: Some(hello.pid),
                        protocol: Some(hello.protocol),
                        error: None,
                    };
                }
                Err(crate::ipc::client::ClientError::NotRunning(_)) => {}
                Err(e) => {
                    probes.daemon.error = Some(e.to_string());
                }
            }
        }

        probes.queue = queue(ctx.paths);
        probes.legacy_credentials_file = crate::credentials::legacy_credentials_file(ctx.paths);
        if ctx.credentials_store == "file" {
            probes.file_store_warning = Some(
                "The opt-in file credential store keeps the refresh token in a 0600 file (never the password)."
                    .to_string(),
            );
        }
        probes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wayland_session_from_session_type() {
        let env = SessionEnvironment {
            xdg_session_type: Some("wayland".into()),
            wayland_display: None,
            display: None,
            xdg_current_desktop: Some("GNOME".into()),
            user: Some("dev".into()),
            ..Default::default()
        };
        let report = DoctorReport::generate(&env);

        assert_eq!(report.session, SessionType::Wayland);
        assert_eq!(
            report.capabilities.keyboard_counting.adapter_status,
            DiagnosticStatus::Unsupported
        );
        assert_eq!(
            report.capabilities.keyboard_counting.architectural_support,
            ArchitecturalSupport::UnsupportedOnWayland
        );
        assert_eq!(
            report.capabilities.mouse_counting.adapter_status,
            DiagnosticStatus::Unsupported
        );
        assert_eq!(
            report.capabilities.active_window.adapter_status,
            DiagnosticStatus::Unsupported
        );
        assert_eq!(
            report.capabilities.screen_capture.architectural_support,
            ArchitecturalSupport::SupportedViaPortal
        );
        assert_eq!(
            report.capabilities.screen_capture.adapter_status,
            DiagnosticStatus::Supported,
            "the Screenshot portal adapter exists"
        );
    }

    #[test]
    fn detects_x11_session_from_display_var() {
        let env = SessionEnvironment {
            xdg_session_type: Some("x11".into()),
            wayland_display: None,
            display: Some(":0".into()),
            xdg_current_desktop: Some("XFCE".into()),
            user: Some("dev".into()),
            ..Default::default()
        };
        let report = DoctorReport::generate(&env);

        assert_eq!(report.session, SessionType::X11);
        assert_eq!(
            report.capabilities.keyboard_counting.adapter_status,
            DiagnosticStatus::Supported,
            "the XInput2 listener exists"
        );
        assert_eq!(
            report.capabilities.keyboard_counting.architectural_support,
            ArchitecturalSupport::SupportedUnconfined
        );
    }

    #[test]
    fn detects_headless_when_no_display_vars_exist() {
        let env = SessionEnvironment::default();
        let report = DoctorReport::generate(&env);

        assert_eq!(report.session, SessionType::Headless);
        assert_eq!(
            report.capabilities.screen_capture.architectural_support,
            ArchitecturalSupport::NoDisplayServer
        );
    }

    #[test]
    fn adapters_report_what_this_build_has() {
        let headless = DoctorReport::generate(&SessionEnvironment::default());
        assert_eq!(
            headless.adapters.screen_capture,
            DiagnosticStatus::NotChecked
        );
        assert_eq!(
            headless.adapters.input_counter,
            DiagnosticStatus::NotChecked
        );
        assert_eq!(
            headless.adapters.window_sampler,
            DiagnosticStatus::NotChecked
        );
        assert_eq!(
            headless.adapters.webcam_capture,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(headless.adapters.offline_queue, DiagnosticStatus::Supported);
        assert_eq!(headless.adapters.backend_sync, DiagnosticStatus::Supported);
        assert_eq!(
            headless.capabilities.webcam_photo.adapter_status,
            DiagnosticStatus::NotImplemented,
            "no webcam in v1"
        );
        let x11 = DoctorReport::generate(&SessionEnvironment {
            xdg_session_type: Some("x11".into()),
            display: Some(":0".into()),
            ..Default::default()
        });
        assert_eq!(x11.adapters.screen_capture, DiagnosticStatus::Supported);
        assert_eq!(x11.adapters.input_counter, DiagnosticStatus::Supported);
        let wayland = DoctorReport::generate(&SessionEnvironment {
            xdg_session_type: Some("wayland".into()),
            ..Default::default()
        });
        assert_eq!(
            wayland.adapters.input_counter,
            DiagnosticStatus::Unsupported
        );
        assert_eq!(wayland.adapters.screen_capture, DiagnosticStatus::Supported);
    }

    #[test]
    fn json_serialization_is_valid_and_parseable() {
        let env = SessionEnvironment {
            xdg_session_type: Some("wayland".into()),
            wayland_display: Some("wayland-0".into()),
            ..Default::default()
        };
        let report = DoctorReport::generate(&env);
        let json = report.to_json().expect("must serialize");

        assert!(json.contains("\"client\""));
        assert!(json.contains("\"dosi-tracker-linux\""));
        assert!(json.contains("\"wayland\""));
        assert!(json.contains("\"UNSUPPORTED\""));
        assert!(json.contains(DOCTOR_SCHEMA));

        let deserialized: DoctorReport = serde_json::from_str(&json).expect("must deserialize");
        assert_eq!(deserialized.session, SessionType::Wayland);
    }

    #[test]
    fn x11_session_reports_window_sampler_supported_in_json_and_text() {
        let env = SessionEnvironment {
            xdg_session_type: Some("x11".into()),
            display: Some(":0".into()),
            ..Default::default()
        };
        let report = DoctorReport::generate(&env);
        assert_eq!(
            report.capabilities.active_window.adapter_status,
            DiagnosticStatus::Supported
        );
        assert_eq!(report.adapters.window_sampler, DiagnosticStatus::Supported);

        let text = report.to_pretty_text();
        assert!(text.contains("Window Sampler: Supported"));

        let json = report.to_json().expect("serialize");
        assert!(json.contains("\"window_sampler\": \"SUPPORTED\""));
    }

    #[test]
    fn wayland_session_reports_window_sampler_unsupported_in_json_and_text() {
        let env = SessionEnvironment {
            xdg_session_type: Some("wayland".into()),
            wayland_display: Some("wayland-0".into()),
            ..Default::default()
        };
        let report = DoctorReport::generate(&env);
        assert_eq!(
            report.capabilities.active_window.adapter_status,
            DiagnosticStatus::Unsupported
        );
        assert_eq!(
            report.adapters.window_sampler,
            DiagnosticStatus::Unsupported
        );

        let text = report.to_pretty_text();
        assert!(text.contains("Window Sampler: Unsupported"));

        let json = report.to_json().expect("serialize");
        assert!(json.contains("\"window_sampler\": \"UNSUPPORTED\""));
    }

    #[test]
    fn headless_session_reports_window_sampler_not_checked() {
        let env = SessionEnvironment::default();
        let report = DoctorReport::generate(&env);
        assert_eq!(
            report.capabilities.active_window.adapter_status,
            DiagnosticStatus::NotChecked
        );
        assert_eq!(report.adapters.window_sampler, DiagnosticStatus::NotChecked);

        let text = report.to_pretty_text();
        assert!(text.contains("Window Sampler: NotChecked"));

        let json = report.to_json().expect("serialize");
        assert!(json.contains("\"window_sampler\": \"NOT_CHECKED\""));
    }

    #[test]
    fn v1_reports_still_parse() {
        let v1 = r#"{"client":{"name":"dosi-tracker-linux","version":"0.1.0","release_tier":"headless-foundation"},
            "session":"headless","desktop_environment":null,"user":null,
            "paths":{"config_dir":"/c","data_dir":"/d","state_dir":"/s","consent_file":"/c/consent.json",
              "config_dir_exists":false,"data_dir_exists":false,"state_dir_exists":false,"consent_file_exists":false},
            "capabilities":{
              "screen_capture":{"adapter_status":"NOT_IMPLEMENTED","architectural_support":"NO_DISPLAY_SERVER","architectural_notes":"","adapter_notes":""},
              "keyboard_counting":{"adapter_status":"NOT_IMPLEMENTED","architectural_support":"NO_DISPLAY_SERVER","architectural_notes":"","adapter_notes":""},
              "mouse_counting":{"adapter_status":"NOT_IMPLEMENTED","architectural_support":"NO_DISPLAY_SERVER","architectural_notes":"","adapter_notes":""},
              "active_window":{"adapter_status":"NOT_IMPLEMENTED","architectural_support":"NO_DISPLAY_SERVER","architectural_notes":"","adapter_notes":""},
              "running_programs":{"adapter_status":"SUPPORTED","architectural_support":"SUPPORTED_PROC_FS","architectural_notes":"","adapter_notes":""},
              "webcam_photo":{"adapter_status":"NOT_IMPLEMENTED","architectural_support":"SUPPORTED_V4L2","architectural_notes":"","adapter_notes":""}},
            "adapters":{"screen_capture":"NOT_IMPLEMENTED","input_counter":"NOT_IMPLEMENTED","window_sampler":"NOT_IMPLEMENTED",
              "webcam_capture":"NOT_IMPLEMENTED","offline_queue":"NOT_IMPLEMENTED","backend_sync":"NOT_IMPLEMENTED"},
            "privacy_invariants":{"unprivileged_process":true,"anti_keylogger_policy":true,"zero_synthetic_telemetry":true,"default_deny_consent":true}}"#;
        let report: DoctorReport = serde_json::from_str(v1).unwrap();
        assert!(report.schema.is_none() && report.probes.is_none() && report.problems.is_empty());
    }

    #[test]
    fn credential_store_follows_the_configuration() {
        let no_bus = SessionEnvironment::default();
        let auto = DoctorReport::generate(&no_bus);
        let store = auto.credential_store.clone().unwrap();
        assert_eq!(
            store.backend, "Unavailable",
            "auto never falls back to a file"
        );
        assert!(!store.fallback_active);
        assert!(auto.has_blocking_problems());

        let file = DoctorReport::generate_with(
            &no_bus,
            &DoctorInputs {
                credentials_store: Some("file".into()),
                probes: None,
            },
        );
        let store = file.credential_store.unwrap();
        assert_eq!(store.backend, "FileFallback");
        assert!(store.fallback_active);
        assert!(store.warning.unwrap().contains("never the password"));

        let bus = SessionEnvironment {
            dbus_session_bus_address: Some("unix:path=/run/user/1000/bus".into()),
            ..Default::default()
        };
        let report = DoctorReport::generate(&bus);
        #[cfg(target_os = "linux")]
        assert_eq!(report.credential_store.unwrap().backend, "SecretService");
        #[cfg(not(target_os = "linux"))]
        assert_eq!(report.credential_store.unwrap().backend, "Unavailable");
    }

    #[test]
    fn problems_are_found_from_the_probes_blocking_first() {
        let env = SessionEnvironment {
            xdg_session_type: Some("wayland".into()),
            dbus_session_bus_address: Some("unix:path=/x".into()),
            ..Default::default()
        };
        let probes = LiveProbes {
            running_as_root: false,
            runtime_dir: None,
            session_bus: true,
            legacy_credentials_file: Some(PathBuf::from(
                "/home/u/.config/dosi-tracker/credentials.json",
            )),
            ..LiveProbes::default()
        };
        let report = DoctorReport::generate_with(
            &env,
            &DoctorInputs {
                credentials_store: None,
                probes: Some(probes),
            },
        );
        let codes: Vec<&str> = report.problems.iter().map(|p| p.code.as_str()).collect();
        for code in [
            "no_runtime_dir",
            "legacy_credentials_file",
            "wayland_limits",
            "no_screenshot_portal",
            "manual_mode",
            "no_tray",
            "no_notifications",
        ] {
            assert!(codes.contains(&code), "{code} in {codes:?}");
        }
        assert_eq!(report.problems[0].severity, Severity::Blocking);
        assert!(report
            .problems
            .windows(2)
            .all(|w| !(w[0].severity == Severity::Warning && w[1].severity == Severity::Blocking)));
        assert!(report.has_blocking_problems());
        assert!(report.to_pretty_text().contains("[Problems]"));
    }
}
