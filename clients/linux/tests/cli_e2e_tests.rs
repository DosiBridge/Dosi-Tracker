//! End-to-end: the `dosi-tracker` daemon as a subprocess on the fake machine
//! (test-support builds), a stateful fake backend, and the CLI driving it as
//! a user would: login through stdin, workspace choice, consent, project,
//! pause/resume/sync/away, queue, logout; every exit-code class; read-only
//! commands that leave the temporary HOME unchanged.
//!
//! Nothing here touches the developer's session: each test has its own
//! HOME/XDG tree and runtime dir, the file credential store, no D-Bus.

#![cfg(all(target_os = "linux", feature = "test-support"))]

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use support::agent::{parse, Agent};
use support::fake_backend::{
    workspace, FakeBackend, PROJECT_WEBSITE, WS_ACME, WS_ARCHIVED, WS_OTHER,
};
use support::schema;

const EMAIL: &str = "ayesha@acme.test";
const PASSWORD: &str = "correct-horse-e2e";

fn envelope_ok(value: &Value) {
    schema::assert_valid(&schema::load("cli-envelope.v1.json"), value);
}

fn snapshot_ok(value: &Value) {
    schema::assert_valid(&schema::load("snapshot.v1.json"), value);
}

fn login(agent: &Agent, backend: &FakeBackend, extra: &[&str]) -> (Option<i32>, Value) {
    let mut args = vec![
        "--json",
        "login",
        "--server",
        backend.url.as_str(),
        "--email",
        EMAIL,
        "--password-stdin",
    ];
    args.extend_from_slice(extra);
    let output = agent.cli_stdin(&args, &format!("{PASSWORD}\n"));
    let value = parse(&output);
    envelope_ok(&value);
    (output.status.code(), value)
}

fn granted(agent: &Agent, flags: &[&str]) {
    let mut args = vec!["consent", "grant", "--yes"];
    args.extend_from_slice(flags);
    let (code, value) = agent.json(&args);
    envelope_ok(&value);
    assert_eq!(code, Some(0), "{value}");
}

/// Signed in to Acme with keyboard and mouse consent, tracking Website.
fn tracking_agent() -> (Agent, FakeBackend) {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = Agent::new();
    agent.start_daemon();
    let (code, value) = login(&agent, &backend, &[]);
    assert_eq!(code, Some(0), "{value}");
    granted(&agent, &["--keyboard", "--mouse"]);
    let (code, value) = agent.json(&["project", "use", "Website"]);
    assert_eq!(code, Some(0), "{value}");
    agent.wait_state(Duration::from_secs(15), |s| s["status"] == "tracking");
    (agent, backend)
}

#[test]
fn a_user_signs_in_chooses_and_tracks_through_the_cli() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    backend.with(|s| {
        s.workspaces = vec![
            workspace(WS_ACME, "Acme", false),
            workspace(WS_OTHER, "Other", false),
            workspace(WS_ARCHIVED, "Old", true),
        ]
    });
    let mut agent = Agent::new();
    agent.start_daemon();

    // Several workspaces and no terminal: exit 7 with the choices.
    let (code, value) = login(&agent, &backend, &[]);
    assert_eq!(code, Some(7), "{value}");
    assert_eq!(value["error"]["code"], "choice_required");
    let choices = value["error"]["data"]["choices"].as_array().unwrap();
    let names: Vec<&str> = choices.iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(
        names,
        ["Acme", "Other"],
        "archived workspaces are not offered"
    );

    // Choose one by name.
    let (code, value) = agent.json(&["workspace", "use", "Other"]);
    envelope_ok(&value);
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(value["result"]["workspace"], "Other");

    // The token request: no tenant, the install's device id.
    let tokens = backend.requests_to("/connect/token");
    let first = &tokens[0];
    assert!(!first.target.contains("__tenant"), "{}", first.target);
    let form = first.form();
    assert_eq!(form["grant_type"], "password");
    let device_file = agent.xdg.data_dir().join("device.json");
    let device: Value =
        serde_json::from_str(&std::fs::read_to_string(device_file).unwrap()).unwrap();
    assert_eq!(form["device_id"], device["device_id"].as_str().unwrap());
    let mine = backend.requests_to("/api/app/workspaces/mine");
    assert!(mine.iter().all(|r| r.header("x-workspace-id").is_none()));

    // Signed in: consent first.
    let snapshot = agent.wait_state(Duration::from_secs(15), |s| s["workspace"] == "Other");
    snapshot_ok(&snapshot);
    assert_eq!(snapshot["status"], "consent_required");
    let (code, value) = agent.json(&["start"]);
    assert_eq!(code, Some(5), "{value}");
    assert_eq!(value["error"]["code"], "consent_required");

    // Switch to Acme and record.
    let (code, value) = agent.json(&["workspace", "use", "Acme"]);
    assert_eq!(code, Some(0), "{value}");
    granted(&agent, &["--mouse"]);
    let receipts =
        std::fs::read_to_string(agent.xdg.state_dir().join("consent-log.jsonl")).unwrap();
    let receipt: Value = serde_json::from_str(receipts.lines().last().unwrap()).unwrap();
    assert_eq!(receipt["origin"], "cli");
    assert_eq!(receipt["to"]["Granted"]["mouse"], true);

    let (code, value) = agent.json(&["project", "use", "Nothing like it"]);
    assert_eq!(code, Some(7), "{value}");
    assert_eq!(value["error"]["code"], "not_found");
    let (code, value) = agent.json(&["project", "use", "website"]);
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(value["changed"], true);
    let snapshot = agent.wait_state(Duration::from_secs(15), |s| s["status"] == "tracking");
    snapshot_ok(&snapshot);
    assert_eq!(snapshot["selected_project"], PROJECT_WEBSITE);
    let (code, value) = agent.json(&["start"]);
    assert_eq!(
        (code, value["changed"].clone()),
        (Some(0), Value::Bool(false)),
        "idempotent"
    );

    // A pause of 15 minutes, then resume.
    let (code, value) = agent.json(&["pause", "--for", "15m"]);
    envelope_ok(&value);
    assert_eq!(code, Some(0), "{value}");
    assert!(value["result"]["resume_at"].is_string(), "{value}");
    assert_eq!(agent.snapshot()["status"], "paused");
    let (code, value) = agent.json(&["resume"]);
    assert_eq!(code, Some(0), "{value}");
    agent.wait_state(Duration::from_secs(10), |s| s["status"] == "tracking");

    // Three fake minutes, then sync: the block reaches the server with the
    // mouse counted and the keyboard null (not consented).
    std::thread::sleep(Duration::from_secs(4));
    let (code, value) = agent.json(&["sync"]);
    envelope_ok(&value);
    assert_eq!(code, Some(0), "{value}");
    assert!(
        backend.wait_for(Duration::from_secs(20), |r| r
            .iter()
            .any(|r| r.path() == "/api/app/activity")),
        "no block was uploaded"
    );
    let upload = &backend.requests_to("/api/app/activity")[0];
    assert_eq!(upload.header("x-workspace-id"), Some(WS_ACME));
    let block = upload.json();
    assert_eq!(block["platform"], "linux");
    assert_eq!(block["deviceId"], device["device_id"]);
    assert!(block["keyboardHits"].is_null(), "{block}");
    assert!(block["mouseClicks"].as_u64().is_some(), "{block}");

    // Away for more than the idle threshold (5 fake minutes), then back:
    // the question, then discard; a second discard has nothing to act on.
    agent.set_present(false);
    agent.wait_state(Duration::from_secs(20), |s| s["status"] == "away");
    agent.set_present(true);
    let snapshot = agent.wait_state(Duration::from_secs(20), |s| s["away_prompt"].is_object());
    snapshot_ok(&snapshot);
    let (code, value) = agent.json(&["away", "discard"]);
    assert_eq!(code, Some(0), "{value}");
    let (code, value) = agent.json(&["away", "discard"]);
    assert_eq!(code, Some(8), "{value}");
    assert_eq!(value["error"]["code"], "precondition");

    // The queue, then sign out.
    let (code, value) = agent.json(&["queue", "status"]);
    envelope_ok(&value);
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(value["result"]["source"], "daemon");
    let (code, value) = agent.json(&["queue", "list"]);
    assert_eq!(code, Some(0), "{value}");
    let (code, value) = agent.json(&["logout"]);
    envelope_ok(&value);
    assert_eq!(code, Some(0), "{value}");
    let snapshot = agent.snapshot();
    snapshot_ok(&snapshot);
    assert_eq!(snapshot["status"], "signed_out");
    assert!(
        backend.wait_for(Duration::from_secs(10), |r| r.iter().any(|r| r.path()
            == "/api/app/presence/heartbeat"
            && r.json()["status"] == "signedout")),
        "the farewell heartbeat says signedout"
    );
}

#[test]
fn every_exit_code_class_is_reachable() {
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = Agent::new();

    // 2: usage, a password on argv, an unknown command, a bad duration, the webcam.
    for args in [
        &["--json", "login", "--password", "hunter2-argv-canary"][..],
        &["--json", "start-tracking"],
        &["--json", "pause", "--for", "soon"],
        &["--json", "consent", "grant", "--webcam", "--yes"],
        &["--json", "consent", "grant", "--yes"],
    ] {
        let output = agent.cli(args);
        let value = parse(&output);
        envelope_ok(&value);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {value}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("hunter2-argv-canary"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("hunter2-argv-canary"));
    }

    // 3: no agent running, and --no-start.
    let (code, value) = agent.json(&["status"]);
    envelope_ok(&value);
    assert_eq!(code, Some(3), "{value}");
    assert_eq!(value["error"]["code"], "daemon_not_running");
    assert_eq!(value["result"]["source"], "local");
    let (code, _) = agent.json(&["--no-start", "start"]);
    assert_eq!(code, Some(3));

    agent.start_daemon();
    // 4: not signed in; a refused password.
    let (code, value) = agent.json(&["start"]);
    assert_eq!(code, Some(4), "{value}");
    assert_eq!(value["error"]["code"], "not_signed_in");
    let output = agent.cli_stdin(
        &[
            "--json",
            "login",
            "--server",
            &backend.url,
            "--email",
            EMAIL,
            "--password-stdin",
        ],
        "wrong-password\n",
    );
    let value = parse(&output);
    assert_eq!(output.status.code(), Some(4), "{value}");
    assert_eq!(value["error"]["code"], "sign_in_failed");

    // 6: the server cannot be reached.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let unreachable = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let output = agent.cli_stdin(
        &[
            "--json",
            "login",
            "--server",
            &unreachable,
            "--email",
            EMAIL,
            "--password-stdin",
        ],
        &format!("{PASSWORD}\n"),
    );
    let value = parse(&output);
    assert_eq!(output.status.code(), Some(6), "{value}");
    assert_eq!(value["error"]["code"], "offline");

    // 7 is covered by the workspace and project choices above; 8: another
    // account while signed in, a second daemon, nothing to keep.
    let (code, value) = login(&agent, &backend, &[]);
    assert_eq!(code, Some(0), "{value}");
    let output = agent.cli_stdin(
        &[
            "--json",
            "login",
            "--server",
            &backend.url,
            "--email",
            "other@acme.test",
            "--password-stdin",
        ],
        "x\n",
    );
    assert_eq!(
        output.status.code(),
        Some(8),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let second = agent
        .command(support::agent::EXE)
        .arg("daemon")
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(8), "the single-instance lock");
    let (code, _) = agent.json(&["away", "keep"]);
    assert_eq!(code, Some(8));

    // 9: no runtime dir for the daemon; a password file others can read.
    let no_runtime = agent
        .command(support::agent::EXE)
        .arg("daemon")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(no_runtime.status.code(), Some(9));
    let pw = agent.xdg.home.join("pw.txt");
    std::fs::write(&pw, format!("{PASSWORD}\n")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&pw, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let (code, value) = agent.json(&["logout"]);
    assert_eq!(code, Some(0), "{value}");
    let (code, value) = agent.json(&[
        "login",
        "--server",
        &backend.url,
        "--email",
        EMAIL,
        "--password-file",
        pw.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(9), "{value}");

    // 1: doctor --strict with a blocking problem (no credential store here
    // without the file opt-in).
    let strict = agent
        .command(support::agent::EXE)
        .args(["doctor", "--strict", "--json"])
        .env_remove("DOSI__CREDENTIALS__STORE")
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(1));
}

/// Every file of the tree (path -> contents), to compare before and after.
fn tree(agent: &Agent) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    agent.all_files().into_iter().collect()
}

#[test]
fn read_only_commands_leave_the_home_unchanged() {
    let (agent, _backend) = tracking_agent();
    let mut agent = agent;
    // Stop the agent: read-only commands must work from the files alone.
    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    agent.daemon = None;
    let before = tree(&agent);
    for args in [
        &["status"][..],
        &["--json", "status"],
        &["doctor"],
        &["--json", "doctor"],
        &["queue", "status"],
        &["--json", "queue", "status"],
        &["consent", "show"],
        &["config", "show"],
        &["config", "get", "interval_minutes"],
        &["config", "path"],
        &["whoami"],
    ] {
        let output = agent.cli(args);
        assert!(
            matches!(output.status.code(), Some(0) | Some(3)),
            "{args:?}: {:?} {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let after = tree(&agent);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "no file was created or removed"
    );
    for (path, contents) in &before {
        if path.to_string_lossy().contains("/logs/") {
            continue;
        }
        assert_eq!(
            Some(contents),
            after.get(path),
            "{} changed",
            path.display()
        );
    }
    let (code, value) = agent.json(&["status"]);
    assert_eq!(code, Some(3));
    assert_eq!(value["result"]["account"], EMAIL);
}

#[test]
fn status_on_a_fresh_home_creates_nothing() {
    let xdg = support::xdg::TestXdg::builder().create_dirs(false).build();
    let agent = Agent::with_xdg(xdg);
    for args in [
        &["status"][..],
        &["--json", "status", "--no-start"],
        &["doctor", "--json"],
        &["queue", "status"],
    ] {
        let output = agent.cli(args);
        assert!(
            matches!(output.status.code(), Some(0) | Some(3)),
            "{args:?}"
        );
    }
    assert!(!agent.xdg.home.exists(), "HOME was created");
    let runtime = agent.xdg.runtime_dir.clone().unwrap();
    assert_eq!(std::fs::read_dir(runtime).unwrap().count(), 0);
}

#[test]
fn status_json_matches_the_snapshot_schema_while_tracking() {
    let (agent, backend) = tracking_agent();
    let (code, value) = agent.json(&["status"]);
    assert_eq!(code, Some(0));
    envelope_ok(&value);
    snapshot_ok(&value["result"]);
    assert_eq!(value["command"], "status");
    assert_eq!(value["result"]["capabilities"]["keyboard"]["state"], "on");
    assert_eq!(
        value["result"]["capabilities"]["webcam"]["consented"],
        false
    );
    // The heartbeat says tracking, with the workspace header.
    assert!(
        backend.wait_for(Duration::from_secs(10), |r| r.iter().any(|r| r.path()
            == "/api/app/presence/heartbeat"
            && r.json()["status"] == "tracking"))
    );
}

#[test]
fn a_mutating_command_starts_a_detached_agent() {
    let agent = Agent::new();
    let output = agent
        .command(support::agent::EXE)
        .args(["--json", "start"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let value = parse(&output);
    // It started (and says what is missing).
    assert_eq!(output.status.code(), Some(4), "{value}");
    assert!(agent.wait_socket(Duration::from_secs(10)));
    let (code, value) = agent.json(&["service", "stop"]);
    assert_eq!(code, Some(0), "{value}");
    let (code, _) = agent.json(&["status"]);
    assert_eq!(code, Some(3), "stopped");
}

#[test]
fn a_daemon_of_another_protocol_is_reported_with_exit_3() {
    let agent = Agent::new();
    let _fake = support::fake_daemon::FakeDaemon::start(
        &agent.socket(),
        std::sync::Arc::new(|request: &Value| {
            Some(serde_json::json!({
                "v": 1, "id": request["id"],
                "error": {"code": "unsupported_version", "message": "This agent speaks IPC protocol 2."}
            }))
        }),
    );
    let (code, value) = agent.json(&["status"]);
    assert_eq!(code, Some(3), "{value}");
    assert_eq!(value["error"]["code"], "unsupported_version");
    assert!(value["error"]["hint"]
        .as_str()
        .unwrap()
        .contains("service restart"));
}

#[test]
fn the_notice_relay_posts_receipts_the_resident_notice_and_the_away_question() {
    use support::fakes::notifications::FakeNotifications;
    use support::private_bus::PrivateTestBus;
    let bus = PrivateTestBus::session();
    let server = FakeNotifications::start(bus.address(), &["body", "actions", "persistence"]);
    let backend = FakeBackend::start(EMAIL, PASSWORD);
    let mut agent = Agent::new().var("DBUS_SESSION_BUS_ADDRESS", bus.address());
    agent.start_daemon();
    let snapshot = agent.snapshot();
    assert_eq!(snapshot["indicator"]["notifications"], true);
    assert_eq!(snapshot["indicator"]["tray"], false);

    assert_eq!(login(&agent, &backend, &[]).0, Some(0));
    granted(&agent, &["--keyboard", "--mouse"]);
    let wait_shown = |pred: &dyn Fn(&support::fakes::notifications::Shown) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(shown) = server.shown().into_iter().find(|s| pred(s)) {
                return shown;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "not shown; shown so far: {:?}",
                server.shown()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let receipt = wait_shown(&|s| s.body.contains("from the command line"));
    assert!(receipt.body.contains("Keyboard counts"), "{}", receipt.body);

    // Tracking without a tray: a resident notification says so (I5).
    assert_eq!(agent.json(&["project", "use", "Website"]).0, Some(0));
    let resident = wait_shown(&|s| s.body == "Dosi Tracker is tracking Website");
    assert_eq!(resident.resident, Some(true));
    assert_eq!(resident.actions, ["pause", "Pause", "open", "Open"]);

    // Away and back: the question with Keep / Discard; Keep answers it.
    agent.set_present(false);
    agent.wait_state(Duration::from_secs(20), |s| s["status"] == "away");
    agent.set_present(true);
    agent.wait_state(Duration::from_secs(20), |s| s["away_prompt"].is_object());
    let question = wait_shown(&|s| s.body.starts_with("You were away for"));
    assert_eq!(
        question.actions,
        ["keep", "Keep as work", "discard", "Discard"]
    );
    server.invoke_action(question.id, "keep");
    agent.wait_state(Duration::from_secs(10), |s| s["away_prompt"].is_null());

    // Away closed the resident notification; tracking again posted a new
    // one. Its Pause pauses.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let resident = loop {
        let latest = server
            .shown()
            .into_iter()
            .rev()
            .find(|s| s.body == "Dosi Tracker is tracking Website")
            .unwrap();
        if latest.id != resident.id || std::time::Instant::now() >= deadline {
            break latest;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        server.closed().len() >= 2,
        "the away question and the first resident notice were closed"
    );
    server.invoke_action(resident.id, "pause");
    agent.wait_state(Duration::from_secs(10), |s| s["status"] == "paused");
    assert!(
        {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                if server.closed().contains(&resident.id) {
                    break true;
                }
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        },
        "the resident notification goes when tracking stops"
    );
}

#[test]
fn status_watch_streams_json_lines() {
    let (agent, _backend) = tracking_agent();
    let mut child = agent
        .command(support::agent::EXE)
        .args(["--json", "status", "--watch"])
        .env("DOSI_NO_AUTOSTART", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let pause = std::thread::spawn({
        let agent_cli = support::agent::EXE.to_string();
        let mut command = agent.command(agent_cli);
        command.args(["pause"]).env("DOSI_NO_AUTOSTART", "1");
        move || {
            std::thread::sleep(Duration::from_millis(500));
            command.output().unwrap()
        }
    });
    let mut lines = std::io::BufRead::lines(std::io::BufReader::new(stdout));
    let mut statuses = Vec::new();
    for _ in 0..20 {
        let Some(Ok(line)) = lines.next() else { break };
        let value: Value = serde_json::from_str(&line).expect("one JSON document per line");
        assert_eq!(value["schema"], "dosi-tracker.cli/1");
        assert_eq!(value["command"], "status.watch");
        snapshot_ok(&value["state"]);
        statuses.push(
            value["state"]["status"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        if statuses.last().map(String::as_str) == Some("paused") {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(pause.join().unwrap().status.code(), Some(0));
    assert_eq!(statuses.first().map(String::as_str), Some("tracking"));
    assert_eq!(
        statuses.last().map(String::as_str),
        Some("paused"),
        "{statuses:?}"
    );
}

#[test]
fn consent_needs_a_confirmation_or_yes_and_completions_are_generated() {
    let agent = Agent::new();
    let (code, value) = agent.json(&["consent", "grant", "--keyboard"]);
    assert_eq!(code, Some(2), "no terminal and no --yes: {value}");
    assert!(!agent.xdg.config_dir().join("consent.json").exists());
    // Offline grant (the agent is down): written under the lock, with a receipt.
    let (code, value) = agent.json(&["consent", "grant", "--time-only", "--yes"]);
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(value["result"]["source"], "local");
    assert!(value["result"]["never_collected"].as_array().unwrap().len() >= 5);
    assert!(agent.xdg.config_dir().join("consent.json").exists());
    assert!(agent.xdg.state_dir().join("consent-log.jsonl").exists());
    // Revocation works offline too.
    let (code, value) = agent.json(&["consent", "revoke"]);
    assert_eq!(code, Some(0), "{value}");
    let (_, value) = agent.json(&["consent", "show"]);
    assert_eq!(value["result"]["consent"], "Revoked");

    for shell in ["bash", "zsh", "fish"] {
        let output = agent.cli(&["completions", shell]);
        assert_eq!(output.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&output.stdout).contains("dosi-tracker"));
    }
}
