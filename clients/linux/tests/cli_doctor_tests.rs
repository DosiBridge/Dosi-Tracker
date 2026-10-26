//! CLI integration tests for the `dosi-tracker` binary.
//!
//! Enforces:
//! - Real binary invocation via Command::new.
//! - Successful parsing of doctor JSON report.
//! - Non-mutating verification: zero files or directories created in isolated temp HOME.
//! - Truthful capability statuses (Wayland unprivileged input = UNSUPPORTED;
//!   what this build implements = SUPPORTED; the webcam = NOT_IMPLEMENTED).
//! - Report v2: the v1 fields plus `schema`, `probes` and `problems`,
//!   validated against `tests/schemas/doctor.v2.json`; `--strict` exits 1 on
//!   blocking problems; doctor never starts the agent.

mod support;

use dosi_tracker_linux::diagnostics::{
    ArchitecturalSupport, DiagnosticStatus, DoctorReport, SessionType,
};
use std::fs;
use std::process::Command;

#[test]
fn cli_help_exits_zero_with_no_filesystem_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .arg("--help")
        .env("HOME", &home)
        .output()
        .expect("failed to execute binary");

    assert!(
        output.status.success(),
        "Process exited with {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("INVARIANTS:"));

    // Ensure no files or directories were created in dummy home or tempdir
    assert!(
        !home.exists(),
        "Dummy home must NOT have been created by --help"
    );
    let entries = fs::read_dir(temp.path()).unwrap().count();
    assert_eq!(entries, 0, "No files should be created in temp dir");
}

#[test]
fn cli_help_command_prints_the_same_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .arg("help")
        .env_clear()
        .output()
        .expect("failed to execute binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("USAGE:") && stdout.contains("INVARIANTS:"));
}

#[test]
fn cli_default_invocation_exits_zero_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .env("HOME", &home)
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(concat!("dosi-tracker ", env!("CARGO_PKG_VERSION"))));
    assert!(
        !home.exists(),
        "Default invocation must not create directories"
    );
}

#[test]
fn cli_doctor_plain_text_succeeds_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .arg("doctor")
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Dosi-Tracker Linux Client Diagnostics"));
    assert!(stdout.contains("[Storage Paths (Non-Mutating Inspection)]"));
    assert!(stdout.contains("[Telemetry Capability Matrix]"));
    assert!(stdout.contains("[Native Adapters Inventory]"));
    assert!(stdout.contains("[Privacy Invariants]"));

    assert!(
        !home.exists(),
        "Doctor command must not create files or directories"
    );
}

#[test]
fn cli_doctor_json_injected_wayland_reports_truthful_statuses() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env("XDG_SESSION_TYPE", "wayland")
        .env("WAYLAND_DISPLAY", "wayland-0")
        .env_remove("DISPLAY")
        .env("XDG_CURRENT_DESKTOP", "GNOME")
        .env("USER", "testworker")
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: DoctorReport = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Failed to parse doctor JSON: {}\nOutput was:\n{}",
            e, stdout
        )
    });

    // Client metadata
    assert_eq!(report.client.name, "dosi-tracker-linux");
    assert_eq!(report.session, SessionType::Wayland);
    assert_eq!(report.desktop_environment.as_deref(), Some("GNOME"));
    assert_eq!(report.user.as_deref(), Some("testworker"));

    // Wayland architectural limits vs adapter statuses
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
    // The Screenshot portal adapter exists (v1); the v1 foundation said NOT_IMPLEMENTED.
    assert_eq!(
        report.capabilities.screen_capture.adapter_status,
        DiagnosticStatus::Supported
    );
    assert_eq!(
        report.capabilities.screen_capture.architectural_support,
        ArchitecturalSupport::SupportedViaPortal
    );

    // Adapter statuses: input and the window sampler are Unsupported on
    // Wayland, the portal capture is built, the webcam is not.
    assert_eq!(report.adapters.screen_capture, DiagnosticStatus::Supported);
    assert_eq!(report.adapters.input_counter, DiagnosticStatus::Unsupported);
    assert_eq!(
        report.adapters.window_sampler,
        DiagnosticStatus::Unsupported
    );
    assert_eq!(
        report.adapters.webcam_capture,
        DiagnosticStatus::NotImplemented
    );

    // Privacy invariants
    assert!(report.privacy_invariants.unprivileged_process);
    assert!(report.privacy_invariants.anti_keylogger_policy);
    assert!(report.privacy_invariants.zero_synthetic_telemetry);
    assert!(report.privacy_invariants.default_deny_consent);

    // No files created
    assert!(!home.exists());
}

#[test]
fn cli_doctor_json_injected_x11_reports_architectural_support_and_built_adapters() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env("XDG_SESSION_TYPE", "x11")
        .env("DISPLAY", ":0")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: DoctorReport = serde_json::from_str(&stdout).expect("Valid JSON");

    assert_eq!(report.session, SessionType::X11);
    assert_eq!(
        report.capabilities.keyboard_counting.architectural_support,
        ArchitecturalSupport::SupportedUnconfined
    );
    // The XInput2 listener exists (v1); the v1 foundation said NOT_IMPLEMENTED.
    assert_eq!(
        report.capabilities.keyboard_counting.adapter_status,
        DiagnosticStatus::Supported
    );
    assert_eq!(
        report.capabilities.active_window.adapter_status,
        DiagnosticStatus::Supported
    );
    assert_eq!(report.adapters.window_sampler, DiagnosticStatus::Supported);
}

#[test]
fn cli_unknown_subcommand_exits_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .arg("start-tracking")
        .output()
        .expect("failed to execute binary");

    assert!(!output.status.success());
    // A usage error: exit 2 in the v1 exit-code table (the foundation used 1).
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown command or option: start-tracking"));
}

#[test]
fn cli_doctor_reports_truthful_credential_store_fallback_and_warning() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    // The file store is an explicit opt-in now (credentials.store = "file").
    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env("DOSI__CREDENTIALS__STORE", "file")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: DoctorReport = serde_json::from_str(&stdout).expect("Valid JSON");

    let cred_diag = report
        .credential_store
        .expect("Credential store diagnosis must be present");
    assert_eq!(cred_diag.backend, "FileFallback");
    assert!(cred_diag.fallback_active);
    let warning = cred_diag
        .warning
        .expect("FileFallback must emit an explicit warning");
    assert!(warning.contains("WARNING: File credential store fallback active"));
    assert!(warning.contains("Freedesktop SecretService is recommended"));
}

#[test]
fn cli_doctor_reports_secret_service_when_dbus_present_on_linux() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/tmp/mock-test-session-bus",
        )
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: DoctorReport = serde_json::from_str(&stdout).expect("Valid JSON");

    let cred_diag = report
        .credential_store
        .expect("Credential store diagnosis must be present");

    #[cfg(target_os = "linux")]
    {
        assert_eq!(cred_diag.backend, "SecretService");
        assert!(!cred_diag.fallback_active);
        assert_eq!(cred_diag.warning, None);
    }

    #[cfg(not(target_os = "linux"))]
    {
        assert_eq!(cred_diag.backend, "FileFallback");
        assert!(cred_diag.fallback_active);
    }
}

#[test]
fn cli_version_names_the_dosi_tracker_binary() {
    for flag in ["--version", "-V", "version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
            .arg(flag)
            .env_clear()
            .output()
            .expect("failed to execute binary");
        assert!(output.status.success(), "{flag}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            concat!("dosi-tracker ", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
    }
}

#[cfg(all(feature = "gui", target_os = "linux"))]
#[test]
fn gui_prints_its_version_and_needs_a_display() {
    let gui = env!("CARGO_BIN_EXE_dosi-tracker-gui");
    let output = Command::new(gui)
        .arg("--version")
        .env_clear()
        .output()
        .expect("failed to execute GUI binary");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("dosi-tracker-gui ", env!("CARGO_PKG_VERSION"))
    );

    // No display in the environment: the window explains it and exits 9.
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(gui)
        .env_clear()
        .env("HOME", temp.path())
        .env("XDG_RUNTIME_DIR", temp.path())
        .output()
        .expect("failed to execute GUI binary");
    assert_eq!(output.status.code(), Some(9));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no graphical display"));

    // Unknown arguments are usage errors, and their values are not echoed.
    let output = Command::new(gui)
        .args(["--password", "gui-canary-pw"])
        .env_clear()
        .output()
        .expect("failed to execute GUI binary");
    assert_eq!(output.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("gui-canary-pw"));
}

#[test]
fn cli_doctor_never_falls_back_to_the_file_store_silently() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker"))
        .args(["doctor", "--json"])
        .env_clear()
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("failed to execute binary");
    assert!(output.status.success(), "doctor without --strict exits 0");
    let report: DoctorReport = serde_json::from_slice(&output.stdout).expect("Valid JSON");
    let store = report.credential_store.clone().unwrap();
    assert_eq!(store.backend, "Unavailable");
    assert!(!store.fallback_active);
    assert!(store.warning.unwrap().contains("credentials.store"));
    assert!(report
        .problems
        .iter()
        .any(|p| p.code == "no_secret_service"));
}

#[cfg(target_os = "linux")]
#[test]
fn cli_doctor_json_matches_report_v2_and_creates_nothing() {
    let xdg = support::xdg::TestXdg::builder().create_dirs(false).build();
    let output = xdg
        .command(env!("CARGO_BIN_EXE_dosi-tracker"))
        .args(["doctor", "--json"])
        .env("XDG_SESSION_TYPE", "x11")
        .env("DISPLAY", ":97")
        .output()
        .expect("failed to execute binary");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("Valid JSON");
    support::schema::assert_valid(&support::schema::load("doctor.v2.json"), &value);
    assert_eq!(value["schema"], "dosi-tracker.doctor/2");
    // Live probes ran, read-only: no display behind :97, no agent, no files.
    assert_eq!(value["probes"]["daemon"]["running"], false);
    assert!(value["probes"]["gui_libraries"]
        .as_array()
        .is_some_and(|l| !l.is_empty()));
    assert!(!xdg.home.exists(), "doctor created HOME");
    let runtime = xdg.runtime_dir.clone().unwrap();
    assert_eq!(
        std::fs::read_dir(runtime).unwrap().count(),
        0,
        "nothing in the runtime dir"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn cli_doctor_strict_fails_only_on_blocking_problems() {
    let xdg = support::xdg::TestXdg::new();
    let run = |store: &str| {
        xdg.command(env!("CARGO_BIN_EXE_dosi-tracker"))
            .args(["doctor", "--strict", "--json"])
            .env("DOSI__CREDENTIALS__STORE", store)
            .output()
            .expect("failed to execute binary")
    };
    // auto without a Secret Service: blocking.
    let blocked = run("auto");
    assert_eq!(blocked.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert!(value["problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["severity"] == "blocking" && p["code"] == "no_secret_service"));
    // The file store opt-in: warnings only.
    let fine = run("file");
    let value: serde_json::Value = serde_json::from_slice(&fine.stdout).unwrap();
    assert_eq!(fine.status.code(), Some(0), "{value:#}");
    assert!(value["problems"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["severity"] == "warning"));

    // The plaintext credentials file of the foundation login blocks until
    // the next sign-in deletes it.
    let legacy = xdg.config_dir().join("credentials.json");
    std::fs::create_dir_all(xdg.config_dir()).unwrap();
    std::fs::write(&legacy, "{}").unwrap();
    let flagged = run("file");
    assert_eq!(flagged.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&flagged.stdout).unwrap();
    assert_eq!(
        value["probes"]["legacy_credentials_file"],
        legacy.to_string_lossy().as_ref()
    );
}
