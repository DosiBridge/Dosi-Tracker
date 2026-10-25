//! Capability diagnostics and environment detection for Dosi-Tracker Linux.
//!
//! Enforces:
//! - Diagnostic execution without DISPLAY, D-Bus, credentials, or sensor access.
//! - Truthful distinction between architectural OS limits (e.g. Wayland input isolation)
//!   and unimplemented native adapters (NOT_IMPLEMENTED).
//! - Zero synthetic telemetry; zero falsified support.

use crate::paths::LinuxPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

impl SessionEnvironment {
    /// Collect environment variables from the current process.
    pub fn from_env() -> Self {
        Self {
            xdg_session_type: std::env::var("XDG_SESSION_TYPE").ok(),
            wayland_display: std::env::var("WAYLAND_DISPLAY").ok(),
            display: std::env::var("DISPLAY").ok(),
            xdg_current_desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
            user: std::env::var("USER").ok(),
            home: std::env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            xdg_data_home: std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            xdg_state_home: std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
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
}

/// Status of an individual telemetry capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticStatus {
    /// Native adapter is not yet implemented in this source slice.
    NotImplemented,
    /// Operating system or display server architecture fundamentally prohibits
    /// unprivileged background access without security compromise.
    Unsupported,
    /// Native adapter is implemented and supported in this environment.
    Supported,
    /// Permission not checked in headless diagnostic mode.
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
    pub backend: String,
    pub path: Option<PathBuf>,
    pub fallback_active: bool,
    pub warning: Option<String>,
}

/// Complete diagnostic report emitted by `dosi-tracker-linux doctor [--json]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
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

impl DoctorReport {
    /// Generate a truthful diagnostic report for the given environment.
    pub fn generate(env: &SessionEnvironment) -> Self {
        let session = env.detect_session();
        let paths = LinuxPaths::resolve(
            env.home.as_deref(),
            env.xdg_config_home.as_deref(),
            env.xdg_data_home.as_deref(),
            env.xdg_state_home.as_deref(),
        );

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
            SessionType::Wayland => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::SupportedViaPortal,
                architectural_notes: "Wayland requires org.freedesktop.portal.ScreenCast via PipeWire with explicit interactive user dialog."
                    .to_string(),
                adapter_notes: "Native portal ScreenCast adapter not implemented in this foundation slice."
                    .to_string(),
            },
            SessionType::X11 => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::SupportedUnconfined,
                architectural_notes: "X11 root window capture architecturally permitted via XShmGetImage."
                    .to_string(),
                adapter_notes: "Native X11 capture adapter not implemented in this foundation slice."
                    .to_string(),
            },
            SessionType::Headless | SessionType::Unknown(_) => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::NoDisplayServer,
                architectural_notes: "No graphical display server detected."
                    .to_string(),
                adapter_notes: "Native screen capture adapter not implemented in this foundation slice."
                    .to_string(),
            },
        };

        let keyboard_counting = match session {
            SessionType::Wayland => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::Unsupported,
                architectural_support: ArchitecturalSupport::UnsupportedOnWayland,
                architectural_notes: "Wayland protocol strictly isolates unprivileged input to focused surface. Anti-keylogger invariant prohibits root or kernel input-device access."
                    .to_string(),
                adapter_notes: "Global keyboard counting is architecturally UNSUPPORTED on Wayland. Must report null."
                    .to_string(),
            },
            SessionType::X11 => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::SupportedUnconfined,
                architectural_notes: "X11 unconfined raw key events architecturally permitted via XInput2 (XI_RawKeyPress)."
                    .to_string(),
                adapter_notes: "Native XInput2 listener not implemented in this foundation slice."
                    .to_string(),
            },
            SessionType::Headless | SessionType::Unknown(_) => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::NoDisplayServer,
                architectural_notes: "No display server detected in environment."
                    .to_string(),
                adapter_notes: "Native input listener not implemented in this foundation slice."
                    .to_string(),
            },
        };

        let mouse_counting = match session {
            SessionType::Wayland => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::Unsupported,
                architectural_support: ArchitecturalSupport::UnsupportedOnWayland,
                architectural_notes: "Wayland protocol isolates unprivileged mouse pointer events. Anti-keylogger invariant prohibits root or kernel input-device access."
                    .to_string(),
                adapter_notes: "Global mouse counting is architecturally UNSUPPORTED on Wayland. Must report null."
                    .to_string(),
            },
            SessionType::X11 => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::SupportedUnconfined,
                architectural_notes: "X11 unconfined raw click events architecturally permitted via XInput2 (XI_RawButtonPress)."
                    .to_string(),
                adapter_notes: "Native XInput2 listener not implemented in this foundation slice."
                    .to_string(),
            },
            SessionType::Headless | SessionType::Unknown(_) => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::NoDisplayServer,
                architectural_notes: "No display server detected in environment."
                    .to_string(),
                adapter_notes: "Native input listener not implemented in this foundation slice."
                    .to_string(),
            },
        };

        let active_window = match session {
            SessionType::Wayland => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::Unsupported,
                architectural_support: ArchitecturalSupport::UnsupportedOnWayland,
                architectural_notes: "Wayland window hierarchy is private to the compositor; no standard unprivileged active window API exists on GNOME/KDE."
                    .to_string(),
                adapter_notes: "Active window tracking is architecturally UNSUPPORTED on Wayland. Must report null."
                    .to_string(),
            },
            SessionType::X11 => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::Supported,
                architectural_support: ArchitecturalSupport::SupportedUnconfined,
                architectural_notes: "EWMH _NET_ACTIVE_WINDOW and _NET_WM_NAME architecturally permitted under X11."
                    .to_string(),
                adapter_notes: "Native X11 active window adapter wired via EWMH and WM_CLASS."
                    .to_string(),
            },
            SessionType::Headless | SessionType::Unknown(_) => CapabilityDiagnosis {
                adapter_status: DiagnosticStatus::NotImplemented,
                architectural_support: ArchitecturalSupport::NoDisplayServer,
                architectural_notes: "No display server detected in environment."
                    .to_string(),
                adapter_notes: "Native window sampler requires a graphical display server."
                    .to_string(),
            },
        };

        let running_programs = CapabilityDiagnosis {
            adapter_status: DiagnosticStatus::Supported,
            architectural_support: ArchitecturalSupport::SupportedProcFs,
            architectural_notes:
                "User-owned process enumeration architecturally permitted via /proc scanning."
                    .to_string(),
            adapter_notes: "Native /proc running-apps inventory adapter wired.".to_string(),
        };

        let webcam_photo = CapabilityDiagnosis {
            adapter_status: DiagnosticStatus::NotImplemented,
            architectural_support: ArchitecturalSupport::SupportedV4l2,
            architectural_notes: "Video capture architecturally permitted via V4L2 (/dev/video*) or PipeWire camera portal."
                .to_string(),
            adapter_notes: "Native webcam capture adapter not implemented in this foundation slice."
                .to_string(),
        };

        let window_sampler_status = active_window.adapter_status.clone();

        Self {
            client: ClientMetadata {
                name: "dosi-tracker-linux".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                release_tier: "headless-foundation".to_string(),
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
                screen_capture: DiagnosticStatus::NotImplemented,
                input_counter: DiagnosticStatus::NotImplemented,
                window_sampler: window_sampler_status,
                webcam_capture: DiagnosticStatus::NotImplemented,
                offline_queue: DiagnosticStatus::NotImplemented,
                backend_sync: DiagnosticStatus::NotImplemented,
            },
            privacy_invariants: PrivacyInvariants {
                unprivileged_process: true,
                anti_keylogger_policy: true,
                zero_synthetic_telemetry: true,
                default_deny_consent: true,
            },
            credential_store: {
                let default_cred_path = paths.config_dir.join("credentials.json");
                let (backend, fallback_active, warning) = if cfg!(target_os = "linux")
                    && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
                {
                    ("SecretService".to_string(), false, None)
                } else {
                    (
                        "FileFallback".to_string(),
                        true,
                        Some(
                            "WARNING: File credential store fallback active. Freedesktop SecretService is recommended for production security."
                                .to_string(),
                        ),
                    )
                };
                Some(CredentialStoreDiagnosis {
                    backend,
                    path: Some(default_cred_path),
                    fallback_active,
                    warning,
                })
            },
        }
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
        out.push_str(&format!(
            "  Screen Capture:    {:?} ({:?})\n",
            self.capabilities.screen_capture.adapter_status,
            self.capabilities.screen_capture.architectural_support
        ));
        out.push_str(&format!(
            "  Keyboard Counting: {:?} ({:?})\n",
            self.capabilities.keyboard_counting.adapter_status,
            self.capabilities.keyboard_counting.architectural_support
        ));
        out.push_str(&format!(
            "  Mouse Counting:    {:?} ({:?})\n",
            self.capabilities.mouse_counting.adapter_status,
            self.capabilities.mouse_counting.architectural_support
        ));
        out.push_str(&format!(
            "  Active Window:     {:?} ({:?})\n",
            self.capabilities.active_window.adapter_status,
            self.capabilities.active_window.architectural_support
        ));
        out.push_str(&format!(
            "  Running Programs:  {:?} ({:?})\n",
            self.capabilities.running_programs.adapter_status,
            self.capabilities.running_programs.architectural_support
        ));
        out.push_str(&format!(
            "  Webcam Photo:      {:?} ({:?})\n\n",
            self.capabilities.webcam_photo.adapter_status,
            self.capabilities.webcam_photo.architectural_support
        ));

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

        out
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
            DiagnosticStatus::NotImplemented
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
    fn all_adapters_are_truthfully_reported_not_implemented() {
        let env = SessionEnvironment::default();
        let report = DoctorReport::generate(&env);

        assert_eq!(
            report.adapters.screen_capture,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(
            report.adapters.input_counter,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(
            report.adapters.window_sampler,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(
            report.adapters.webcam_capture,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(
            report.adapters.offline_queue,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(
            report.adapters.backend_sync,
            DiagnosticStatus::NotImplemented
        );
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
    fn headless_session_reports_window_sampler_not_implemented() {
        let env = SessionEnvironment::default();
        let report = DoctorReport::generate(&env);
        assert_eq!(
            report.capabilities.active_window.adapter_status,
            DiagnosticStatus::NotImplemented
        );
        assert_eq!(
            report.adapters.window_sampler,
            DiagnosticStatus::NotImplemented
        );

        let text = report.to_pretty_text();
        assert!(text.contains("Window Sampler: NotImplemented"));

        let json = report.to_json().expect("serialize");
        assert!(json.contains("\"window_sampler\": \"NOT_IMPLEMENTED\""));
    }
}
