//! CLI integration tests for dosi-tracker-linux.
//!
//! Enforces:
//! - Real binary invocation via Command::new.
//! - Successful parsing of doctor JSON report.
//! - Non-mutating verification: zero files or directories created in isolated temp HOME.
//! - Truthful capability statuses (Wayland unprivileged input = UNSUPPORTED, all adapters = NOT_IMPLEMENTED).

use dosi_tracker_linux::diagnostics::{
    ArchitecturalSupport, DiagnosticStatus, DoctorReport, SessionType,
};
use std::fs;
use std::process::Command;

#[test]
fn cli_help_exits_zero_with_no_filesystem_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
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
fn cli_default_invocation_exits_zero_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
        .env("HOME", &home)
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dosi-tracker-linux"));
    assert!(
        !home.exists(),
        "Default invocation must not create directories"
    );
}

#[test]
fn cli_doctor_plain_text_succeeds_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
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

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
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
    assert_eq!(
        report.capabilities.screen_capture.adapter_status,
        DiagnosticStatus::NotImplemented
    );
    assert_eq!(
        report.capabilities.screen_capture.architectural_support,
        ArchitecturalSupport::SupportedViaPortal
    );

    // All native adapters unimplemented
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

    // Privacy invariants
    assert!(report.privacy_invariants.unprivileged_process);
    assert!(report.privacy_invariants.anti_keylogger_policy);
    assert!(report.privacy_invariants.zero_synthetic_telemetry);
    assert!(report.privacy_invariants.default_deny_consent);

    // No files created
    assert!(!home.exists());
}

#[test]
fn cli_doctor_json_injected_x11_reports_architectural_support_with_unimplemented_adapters() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
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
    assert_eq!(
        report.capabilities.keyboard_counting.adapter_status,
        DiagnosticStatus::NotImplemented
    );
}

#[test]
fn cli_unknown_subcommand_exits_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
        .arg("start-tracking")
        .output()
        .expect("failed to execute binary");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown command or option: start-tracking"));
}

#[test]
fn cli_doctor_reports_truthful_credential_store_fallback_and_warning() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home_dummy");

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
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

    let output = Command::new(env!("CARGO_BIN_EXE_dosi-tracker-linux"))
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
