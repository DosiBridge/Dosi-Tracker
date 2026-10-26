//! Privacy guards (design 2 and 18.3):
//! - static scans of `src/` and `packaging/`: no input devices, evdev,
//!   uinput, XRecord or synthetic input; the XI2 key path never reads the
//!   event's `detail`; nothing installs a system-wide autostart entry;
//! - `--password` on the command line is refused without echoing it;
//! - a sentinel password given through stdin never reaches the logs,
//!   `session.json`, the configuration, the outbox, any other file of the
//!   agent, or the daemon's command line and environment.

#![cfg(target_os = "linux")]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use support::agent::Agent;
use support::fake_backend::FakeBackend;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The non-comment code of a Rust source line.
fn code_of(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return "";
    }
    match line.find("//") {
        Some(i) if !line[..i].contains('"') => &line[..i],
        _ => line,
    }
}

#[test]
fn no_source_reaches_for_input_devices_or_synthetic_input() {
    let forbidden = ["/dev/input", "evdev", "uinput", "XRecord", "fake_input"];
    let mut hits = Vec::new();
    for root in ["src", "packaging"] {
        for path in files_under(&crate_dir().join(root)) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (n, line) in text.lines().enumerate() {
                for word in forbidden {
                    if line.contains(word) {
                        hits.push(format!("{}:{}: {word}", path.display(), n + 1));
                    }
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "forbidden input access:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_xi2_key_path_never_reads_the_keycode() {
    let source = std::fs::read_to_string(crate_dir().join("src/tracking/x11_input.rs")).unwrap();
    let reads: Vec<String> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let code = code_of(line);
            code.contains(".detail") || code.contains("detail:") || code.contains("detail)")
        })
        .map(|(n, line)| format!("{}: {line}", n + 1))
        .collect();
    assert!(
        reads.is_empty(),
        "the keycode field is read:\n{}",
        reads.join("\n")
    );
    assert!(
        !source.contains("xtest_fake_input"),
        "no synthetic input is ever sent"
    );
}

#[test]
fn nothing_installs_a_system_wide_autostart_entry() {
    for path in files_under(&crate_dir().join("packaging")) {
        let text = path.to_string_lossy();
        assert!(!text.contains("etc/xdg/autostart"), "{text}");
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !contents.contains("/etc/xdg/autostart"),
            "{} installs a system-wide autostart entry",
            path.display()
        );
    }
    let installer = std::fs::read_to_string(crate_dir().join("scripts/install.sh")).unwrap();
    assert!(!installer.contains("/etc/xdg/autostart"));
}

#[test]
fn a_password_on_the_command_line_is_refused_without_echo() {
    let agent = Agent::new();
    for args in [
        &[
            "login",
            "--email",
            "a@b.test",
            "--password",
            "sentinel-argv-31337",
        ][..],
        &["login", "--password=sentinel-argv-31337"],
        &["login", "-psentinel-argv-31337"],
    ] {
        let output = agent.cli(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!all.contains("sentinel-argv-31337"), "{all}");
        assert!(
            all.contains("--password-stdin"),
            "says what to use instead: {all}"
        );
    }
    assert!(
        !agent.xdg.data_dir().exists() && !agent.xdg.state_dir().exists(),
        "nothing was written"
    );
}

#[test]
fn a_sentinel_password_is_never_stored_logged_or_passed_on() {
    const SENTINEL: &str = "Sentinel-Pw-9c41e7d2";
    let backend = FakeBackend::start("ayesha@acme.test", SENTINEL);
    let mut agent = Agent::new().var("DOSI__LOG__LEVEL", "trace");
    if !cfg!(feature = "test-support") {
        agent = agent.without_var("DOSI_TEST_MACHINE");
    }
    agent.start_daemon();
    let output = agent.cli_stdin(
        &[
            "login",
            "--server",
            &backend.url,
            "--email",
            "ayesha@acme.test",
            "--password-stdin",
        ],
        &format!("{SENTINEL}\n"),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        agent
            .cli(&["consent", "grant", "--all", "--yes"])
            .status
            .code(),
        Some(0)
    );
    let _ = agent.cli(&["project", "use", "Website"]);
    std::thread::sleep(Duration::from_secs(2));
    let _ = agent.cli(&["sync"]);

    // The daemon's command line and environment.
    let pid = agent.daemon_pid().unwrap();
    for file in ["cmdline", "environ"] {
        let bytes = std::fs::read(format!("/proc/{pid}/{file}")).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(SENTINEL),
            "the password is in the daemon's {file}"
        );
    }

    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    agent.daemon = None;

    // Every file the agent wrote: logs, session.json, config, consent,
    // credentials (file store), outbox.db with its WAL, device.json.
    let files = agent.all_files();
    let names: Vec<String> = files
        .iter()
        .map(|(p, _)| p.to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("session.json")),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.contains("outbox.db")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("/logs/")), "{names:?}");
    for (path, bytes) in &files {
        let haystack = String::from_utf8_lossy(bytes);
        assert!(
            !haystack.contains(SENTINEL),
            "the password is in {}",
            path.display()
        );
    }
    // The server did get it (once, in the token request).
    let tokens = backend.requests_to("/connect/token");
    assert!(tokens
        .iter()
        .any(|r| r.form().get("password").map(String::as_str) == Some(SENTINEL)));
}
