//! Integration and invariant tests for the Linux rootless installer lifecycle.
//!
//! Enforces:
//! - Static safety: no `sudo`, no `curl | sh`, no network downloads, no autostart.
//! - Execution of rootless installer script in isolated temporary prefixes.
//! - Manifest generation and ownership tracking.
//! - Preservation of foreign/unrelated files and user configuration.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_bash_interpreter() -> Option<PathBuf> {
    // Check known Git Bash path on Windows first (which has cargo in PATH)
    let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    if git_bash.is_file() {
        return Some(git_bash);
    }

    // Try finding "bash" in system PATH
    if let Ok(output) = Command::new("bash").arg("--version").output() {
        if output.status.success() {
            return Some(PathBuf::from("bash"));
        }
    }

    None
}

#[test]
fn installer_static_safety_and_hygiene_invariants() {
    let script_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("install.sh");
    assert!(
        script_path.is_file(),
        "install.sh must exist at clients/linux/scripts/install.sh"
    );

    let content = fs::read_to_string(&script_path).expect("failed reading install.sh");

    // Invariant: No sudo or root escalation commands invoked
    assert!(
        !content.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("sudo ") || trimmed == "sudo"
        }),
        "install.sh must not invoke sudo for command execution"
    );

    // Invariant: No curl-pipe-shell or remote execution
    assert!(
        !content.contains("curl "),
        "install.sh must not use curl for downloads"
    );
    assert!(
        !content.contains("wget "),
        "install.sh must not use wget for downloads"
    );
    assert!(
        !content.contains("| bash") && !content.contains("| sh"),
        "install.sh must not contain curl-pipe-shell patterns"
    );

    // Invariant: No autostart or systemd service installation
    assert!(
        !content.contains("systemctl"),
        "install.sh must not register systemd services in this slice"
    );
    assert!(
        !content.contains("/etc/systemd"),
        "install.sh must not write to systemd directories"
    );

    // Invariant: Uninstall script exists and delegates
    let uninstall_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("uninstall.sh");
    assert!(
        uninstall_path.is_file(),
        "uninstall.sh must exist at clients/linux/scripts/uninstall.sh"
    );
    let uninstall_content =
        fs::read_to_string(&uninstall_path).expect("failed reading uninstall.sh");
    assert!(uninstall_content.contains("install.sh"));
    assert!(uninstall_content.contains("uninstall"));
}

#[test]
fn installer_help_and_version_invocations_exit_zero() {
    let bash = match find_bash_interpreter() {
        Some(b) => b,
        None => {
            eprintln!("Bash interpreter not found on host; skipping shell execution test.");
            return;
        }
    };

    let script_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("install.sh");

    // 1. --help
    let help_output = Command::new(&bash)
        .arg(&script_path)
        .arg("--help")
        .output()
        .expect("failed executing install.sh --help");
    assert!(
        help_output.status.success(),
        "install.sh --help must exit 0"
    );
    let help_str = String::from_utf8_lossy(&help_output.stdout);
    assert!(help_str.contains("USAGE:"));
    assert!(help_str.contains("install"));
    assert!(help_str.contains("update"));
    assert!(help_str.contains("rollback"));
    assert!(help_str.contains("uninstall"));
    assert!(help_str.contains("INVARIANTS:"));

    // 2. --version
    let ver_output = Command::new(&bash)
        .arg(&script_path)
        .arg("--version")
        .output()
        .expect("failed executing install.sh --version");
    assert!(
        ver_output.status.success(),
        "install.sh --version must exit 0"
    );
    let ver_str = String::from_utf8_lossy(&ver_output.stdout);
    assert!(ver_str.contains("dosi-tracker installer version 0.1.0"));
}

#[test]
fn installer_lifecycle_full_suite_passes() {
    let bash = match find_bash_interpreter() {
        Some(b) => b,
        None => {
            eprintln!("Bash interpreter not found on host; skipping full lifecycle test.");
            return;
        }
    };

    let test_suite_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("test_installer_lifecycle.sh");
    assert!(
        test_suite_path.is_file(),
        "test_installer_lifecycle.sh must exist"
    );

    let output = Command::new(&bash)
        .arg(&test_suite_path)
        .current_dir(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .output()
        .expect("failed executing test_installer_lifecycle.sh");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("STDOUT:\n{}", stdout);
        eprintln!("STDERR:\n{}", stderr);
    }

    assert!(
        output.status.success(),
        "Installer lifecycle test suite must pass with exit 0"
    );
    assert!(
        stdout.contains("Failed: 0"),
        "Test suite must report Failed: 0"
    );
    assert!(
        stdout.contains("Passed: ") && !stdout.contains("Passed: 0\n"),
        "Test suite must report passed tests"
    );
}
