//! IPC protocol v1 and the daemon's process rules (design 10, 14):
//! codec limits, version negotiation, socket modes, stale sockets, the peer
//! uid check (injected credentials), connection limits, coalesced
//! subscriptions, the single-instance lock, sd_notify, SIGTERM and the
//! upgrade restart (exit 75).
//!
//! Server tests run the real `ipc::server` against a scripted dispatcher in
//! this process; daemon tests run the `dosi-tracker daemon` binary in a
//! temporary HOME/XDG tree.

#![cfg(target_os = "linux")]

mod support;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use dosi_tracker_linux::agent::state::StateHandle;
use dosi_tracker_linux::ipc::client::{Client, ClientError};
use dosi_tracker_linux::ipc::server::{
    self, BoxFuture, DaemonLock, Dispatch, Reply, ServerConfig, ServerHandle,
};
use dosi_tracker_linux::ipc::{ErrorBody, ErrorCode, HelloResult, MAX_FRAME_BYTES};
use support::agent::{wait_child, Agent, EXE};

/// A dispatcher that answers `echo` and counts calls.
struct Scripted {
    state: StateHandle,
    calls: AtomicUsize,
}

impl Dispatch for Scripted {
    fn hello(&self) -> HelloResult {
        HelloResult {
            protocol: 1,
            daemon_version: "test".into(),
            pid: std::process::id(),
            started_at: chrono::Utc::now(),
        }
    }

    fn call(&self, method: &str, params: Value) -> BoxFuture<Result<Reply, ErrorBody>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let method = method.to_owned();
        Box::pin(async move {
            match method.as_str() {
                "state.get" => Ok(Reply::value(params)),
                _ => Err(ErrorBody::new(ErrorCode::Internal, "scripted")),
            }
        })
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.state.subscribe()
    }

    fn snapshot(&self) -> Value {
        json!({ "version": self.state.version() })
    }
}

struct Served {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    dispatch: Arc<Scripted>,
    handle: Option<ServerHandle>,
    _lock: DaemonLock,
}

fn serve(config: ServerConfig) -> Served {
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("dosi-tracker");
    let socket = runtime.join("daemon.sock");
    let lock = DaemonLock::try_acquire(&runtime.join("daemon.lock"))
        .unwrap()
        .unwrap();
    let listener = server::bind(&socket, &lock).unwrap();
    let dispatch = Arc::new(Scripted {
        state: StateHandle::new(),
        calls: AtomicUsize::new(0),
    });
    let handle = server::spawn(listener, config, dispatch.clone()).unwrap();
    Served {
        _dir: dir,
        socket,
        dispatch,
        handle: Some(handle),
        _lock: lock,
    }
}

fn raw(socket: &Path) -> (UnixStream, BufReader<UnixStream>) {
    let stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    (stream, reader)
}

fn send(stream: &mut UnixStream, value: Value) {
    let mut line = value.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).unwrap();
}

fn recv(reader: &mut BufReader<UnixStream>) -> Option<Value> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => serde_json::from_str(&line).ok(),
    }
}

fn hello() -> Value {
    json!({"v": 1, "id": 0, "method": "hello", "params": {"client": "cli", "version": "0"}})
}

#[test]
fn the_socket_is_ours_0600_in_a_0700_directory() {
    let served = serve(ServerConfig::default());
    let meta = std::fs::symlink_metadata(&served.socket).unwrap();
    assert!(std::os::unix::fs::FileTypeExt::is_socket(&meta.file_type()));
    assert_eq!(meta.mode() & 0o777, 0o600);
    // SAFETY: geteuid has no preconditions.
    assert_eq!(meta.uid(), unsafe { libc::geteuid() });
    let dir = std::fs::metadata(served.socket.parent().unwrap()).unwrap();
    assert_eq!(dir.mode() & 0o777, 0o700);
    assert!(server::check_socket(&served.socket).is_ok());
}

#[test]
fn hello_comes_first_and_another_major_is_refused_and_closed() {
    let served = serve(ServerConfig::default());
    let (mut stream, mut reader) = raw(&served.socket);
    send(&mut stream, json!({"v": 1, "id": 3, "method": "state.get"}));
    let answer = recv(&mut reader).unwrap();
    assert_eq!(answer["error"]["code"], "bad_request", "{answer}");
    send(&mut stream, hello());
    let answer = recv(&mut reader).unwrap();
    assert_eq!(answer["result"]["protocol"], 1);
    send(
        &mut stream,
        json!({"v": 1, "id": 4, "method": "no.such.method"}),
    );
    assert_eq!(
        recv(&mut reader).unwrap()["error"]["code"],
        "unknown_method"
    );
    send(
        &mut stream,
        json!({"v": 1, "id": 5, "method": "state.get", "params": {"x": 1}, "extra": "ignored"}),
    );
    let answer = recv(&mut reader).unwrap();
    assert_eq!(
        (answer["id"].clone(), answer["result"]["x"].clone()),
        (json!(5), json!(1))
    );

    send(&mut stream, json!({"v": 2, "id": 6, "method": "state.get"}));
    let answer = recv(&mut reader).unwrap();
    assert_eq!(answer["error"]["code"], "unsupported_version");
    assert!(recv(&mut reader).is_none(), "the connection is closed");

    // The client turns it into a version error (the CLI's exit 3).
    let fake = support::fake_daemon::FakeDaemon::start(
        &served.socket.with_file_name("other.sock"),
        Arc::new(|r: &Value| {
            Some(
                json!({"v": 1, "id": r["id"], "result": {"protocol": 2, "daemon_version": "9", "pid": 1, "started_at": "2026-01-01T00:00:00Z"}}),
            )
        }),
    );
    match Client::connect(&fake.socket, "cli") {
        Err(ClientError::Version(error)) => assert_eq!(error.code, ErrorCode::UnsupportedVersion),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_oversized_frame_is_refused_and_the_connection_closed() {
    let served = serve(ServerConfig::default());
    let (mut stream, mut reader) = raw(&served.socket);
    send(&mut stream, hello());
    recv(&mut reader).unwrap();
    let chunk = vec![b'x'; 64 * 1024];
    let mut sent = 0;
    while sent <= MAX_FRAME_BYTES {
        if stream.write_all(&chunk).is_err() {
            break;
        }
        sent += chunk.len();
    }
    let answer = recv(&mut reader);
    if let Some(answer) = answer {
        assert_eq!(answer["error"]["code"], "bad_request");
        assert!(recv(&mut reader).is_none(), "closed after the refusal");
    }
    assert_eq!(served.dispatch.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn another_uid_is_closed_before_anything_is_read() {
    let config = ServerConfig {
        // SAFETY: geteuid has no preconditions.
        peer_credentials: Arc::new(|_| Ok((unsafe { libc::geteuid() } + 1, Some(4242)))),
        ..ServerConfig::default()
    };
    let served = serve(config);
    let (mut stream, mut reader) = raw(&served.socket);
    let _ = stream.write_all(b"{\"v\":1,\"id\":0,\"method\":\"hello\"}\n");
    assert!(recv(&mut reader).is_none(), "no answer, closed");
    assert_eq!(served.dispatch.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        Client::connect(&served.socket, "cli"),
        Err(ClientError::Protocol(_))
    ));
}

#[test]
fn connections_are_limited_and_idle_ones_dropped() {
    let config = ServerConfig {
        max_connections: 2,
        idle_timeout: Duration::from_millis(300),
        ..ServerConfig::default()
    };
    let served = serve(config);
    let first = Client::connect(&served.socket, "cli").unwrap();
    let second = Client::connect(&served.socket, "cli").unwrap();
    match Client::connect(&served.socket, "cli") {
        Err(ClientError::Daemon(error)) => assert_eq!(error.code, ErrorCode::Busy),
        other => panic!("{other:?}"),
    }
    drop((first, second));
    std::thread::sleep(Duration::from_millis(100));
    let (mut stream, mut reader) = raw(&served.socket);
    send(&mut stream, hello());
    recv(&mut reader).unwrap();
    let started = Instant::now();
    assert!(recv(&mut reader).is_none(), "dropped when idle");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn subscriptions_are_coalesced_to_four_per_second() {
    let served = serve(ServerConfig::default());
    let state = served.dispatch.state.clone();
    let client = Client::connect(&served.socket, "cli").unwrap();
    let mut subscription = client.subscribe().unwrap();
    let first = subscription
        .next_event(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(first.event, "state");
    let updater = std::thread::spawn(move || {
        for _ in 0..200 {
            state.update(|s| s.tracked_today_secs += 1.0);
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let started = Instant::now();
    let mut events = 0;
    let mut last_seq = first.seq;
    while started.elapsed() < Duration::from_millis(1_500) {
        match subscription.next_event(Some(Duration::from_millis(300))) {
            Ok(event) => {
                assert!(event.seq > last_seq, "sequence numbers increase");
                last_seq = event.seq;
                events += 1;
            }
            Err(ClientError::Timeout) => {}
            Err(e) => panic!("{e}"),
        }
    }
    updater.join().unwrap();
    assert!(events >= 2, "changes are delivered ({events})");
    assert!(events <= 7, "at most 4 per second ({events} in 1.5 s)");
}

#[test]
fn ui_show_reaches_subscribed_guis_only() {
    let served = serve(ServerConfig::default());
    let gui = Client::connect(&served.socket, "gui").unwrap();
    let mut gui = gui.subscribe().unwrap();
    gui.next_event(Some(Duration::from_secs(2))).unwrap();
    let cli = Client::connect(&served.socket, "cli").unwrap();
    let mut cli = cli.subscribe().unwrap();
    cli.next_event(Some(Duration::from_secs(2))).unwrap();
    let mut other = Client::connect(&served.socket, "cli").unwrap();
    let answer = other.call("ui.show", json!({"view": "settings"})).unwrap();
    assert_eq!(answer.result["forwarded"], true);
    let event = gui.next_event(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(
        (event.event.as_str(), event.data["view"].clone()),
        ("show", json!("settings"))
    );
    assert!(matches!(
        cli.next_event(Some(Duration::from_millis(500))),
        Err(ClientError::Timeout)
    ));
    drop(served.handle);
}

#[test]
fn a_stale_socket_is_only_replaced_while_the_lock_is_free() {
    let agent = Agent::new().without_var("DOSI_TEST_MACHINE");
    let socket = agent.socket();
    let dir = socket.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    let identity = |p: &Path| {
        let meta = std::fs::symlink_metadata(p).unwrap();
        (meta.ino(), meta.ctime(), meta.ctime_nsec())
    };
    let stale = identity(&socket);
    assert!(
        UnixStream::connect(&socket).is_err(),
        "nobody listens on a stale socket"
    );

    // Someone holds the lock: the daemon refuses (8) and leaves the socket.
    let lock = DaemonLock::try_acquire(&dir.join("daemon.lock"))
        .unwrap()
        .unwrap();
    let refused = agent.command(EXE).arg("daemon").output().unwrap();
    assert_eq!(refused.status.code(), Some(8));
    assert_eq!(identity(&socket), stale, "untouched");
    drop(lock);

    // The lock is free: the stale socket is replaced by a live one.
    let mut agent = agent;
    agent.start_daemon();
    assert!(UnixStream::connect(&socket).is_ok());
    assert_ne!(identity(&socket), stale);
    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    agent.daemon = None;
    assert!(!socket.exists(), "the socket is removed on exit");
}

#[test]
fn something_else_at_the_socket_path_is_refused() {
    let agent = Agent::new().without_var("DOSI_TEST_MACHINE");
    let socket = agent.socket();
    let dir = socket.parent().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&socket, b"not a socket").unwrap();
    let refused = agent.command(EXE).arg("daemon").output().unwrap();
    assert_eq!(refused.status.code(), Some(9), "unsafe socket path");
    assert_eq!(std::fs::read(&socket).unwrap(), b"not a socket");
}

#[test]
fn two_daemons_race_and_exactly_one_wins() {
    let agent = Agent::new().without_var("DOSI_TEST_MACHINE");
    let spawn = || {
        agent
            .command(EXE)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let (mut a, mut b) = (spawn(), spawn());
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut code_a, mut code_b) = (None, None);
    while Instant::now() < deadline && code_a.is_none() && code_b.is_none() {
        code_a = a.try_wait().unwrap().and_then(|s| s.code());
        code_b = b.try_wait().unwrap().and_then(|s| s.code());
        std::thread::sleep(Duration::from_millis(50));
    }
    let (loser, mut winner) = match (code_a, code_b) {
        (Some(code), None) => (code, b),
        (None, Some(code)) => (code, a),
        other => panic!("exactly one must exit: {other:?}"),
    };
    assert_eq!(loser, 8);
    assert!(agent.wait_socket(Duration::from_secs(10)));
    // SAFETY: kill on our own child.
    unsafe { libc::kill(winner.id() as i32, libc::SIGTERM) };
    assert_eq!(wait_child(&mut winner, Duration::from_secs(25)), Some(0));
}

#[test]
fn ready_and_stopping_reach_the_notify_socket() {
    let mut agent = Agent::new().without_var("DOSI_TEST_MACHINE");
    let notify = agent.xdg.home.join("notify.sock");
    let receiver = std::os::unix::net::UnixDatagram::bind(&notify).unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    agent.start_daemon_with(EXE, &[("NOTIFY_SOCKET", notify.to_str().unwrap())]);
    let mut messages = String::new();
    let mut buffer = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(15);
    while !messages.contains("READY=1") && Instant::now() < deadline {
        if let Ok(n) = receiver.recv(&mut buffer) {
            messages.push_str(&String::from_utf8_lossy(&buffer[..n]));
            messages.push('\n');
        }
    }
    assert!(messages.contains("READY=1"), "{messages}");
    assert!(messages.contains("STATUS="), "{messages}");
    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    agent.daemon = None;
    let mut rest = String::new();
    receiver
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    while let Ok(n) = receiver.recv(&mut buffer) {
        rest.push_str(&String::from_utf8_lossy(&buffer[..n]));
    }
    assert!(rest.contains("STOPPING=1"), "{rest}");
}

#[test]
fn the_daemon_needs_a_runtime_dir_and_never_runs_as_root() {
    let agent = Agent::new();
    let output = agent
        .command(EXE)
        .arg("daemon")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(9));
    assert!(String::from_utf8_lossy(&output.stderr).contains("XDG_RUNTIME_DIR"));
    // (The root refusal is checked by privacy_guards when the tests run as root.)
}

#[test]
fn a_password_canary_never_reaches_the_logs() {
    const CANARY: &str = "canary-pw-ipc-7f3a9";
    let backend = support::fake_backend::FakeBackend::start("ayesha@acme.test", CANARY);
    let mut agent = Agent::new().without_var("DOSI_TEST_MACHINE");
    agent.start_daemon_with(EXE, &[("DOSI__LOG__LEVEL", "trace")]);
    let output = agent.cli_stdin(
        &[
            "login",
            "--server",
            &backend.url,
            "--email",
            "ayesha@acme.test",
            "--password-stdin",
        ],
        &format!("{CANARY}\n"),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    agent.daemon = None;
    let logs = agent.logs();
    assert!(
        !logs.is_empty(),
        "the daemon logged: {:?} / {}",
        agent
            .all_files()
            .iter()
            .map(|(p, _)| p.clone())
            .collect::<Vec<_>>(),
        std::fs::read_to_string(&agent.stderr_file).unwrap_or_default()
    );
    assert!(!logs.contains(CANARY), "the password is in the logs");
    assert!(
        !logs.contains("access-1") && !logs.contains("refresh-1"),
        "tokens in the logs"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn sigterm_leaves_a_queued_row_and_sends_the_stopped_heartbeat() {
    use support::fake_backend::{ActivityMode, FakeBackend};
    let backend = FakeBackend::start("ayesha@acme.test", "pw-sigterm");
    backend.with(|s| s.activity_mode = ActivityMode::Unavailable);
    let mut agent = Agent::new();
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
        "pw-sigterm\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        agent
            .cli(&["consent", "grant", "--time-only", "--yes"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        agent.cli(&["project", "use", "Website"]).status.code(),
        Some(0)
    );
    agent.wait_state(Duration::from_secs(15), |s| s["status"] == "tracking");
    std::thread::sleep(Duration::from_secs(3));

    let started = Instant::now();
    assert_eq!(
        agent.signal_daemon(libc::SIGTERM, Duration::from_secs(25)),
        Some(0)
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "exits within 20 s"
    );
    agent.daemon = None;

    let heartbeats = backend.requests_to("/api/app/presence/heartbeat");
    assert_eq!(
        heartbeats.last().map(|r| r.json()["status"].clone()),
        Some(json!("stopped")),
        "the farewell says stopped"
    );
    let scope = dosi_tracker_linux::outbox::OutboxScope::new(
        "ayesha@acme.test",
        support::fake_backend::WS_ACME,
    )
    .unwrap();
    let report = dosi_tracker_linux::outbox::inspect_outbox_status(
        agent.xdg.data_dir().join("outbox.db"),
        &scope,
    )
    .unwrap();
    assert_eq!(
        report.durable_queue.queued_count,
        1,
        "the block was saved: {report:?}
uploads: {:?}
logs: {}",
        backend.requests_to("/api/app/activity").len(),
        agent.logs()
    );
}

#[cfg(feature = "test-support")]
#[test]
fn a_replaced_executable_exits_75_at_a_block_boundary() {
    let mut agent = Agent::new();
    let bin = agent.xdg.home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let copy = bin.join("dosi-tracker");
    std::fs::copy(EXE, &copy).unwrap();
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o755)).unwrap();
    agent.start_daemon_with(&copy, &[("DOSI_TEST_EXE_CHECK_SECS", "1")]);

    // A package upgrade: a new file is renamed over the old one.
    let staged = bin.join(".dosi-tracker.new");
    std::fs::write(&staged, std::fs::read(EXE).unwrap()).unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &copy).unwrap();

    let child = agent.daemon.as_mut().unwrap();
    assert_eq!(wait_child(child, Duration::from_secs(30)), Some(75));
    agent.daemon = None;
}

fn fixture(name: &str) -> (Vec<u8>, Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ipc")
        .join(name);
    let bytes = std::fs::read(&path).unwrap();
    let value = serde_json::from_slice(&bytes).unwrap();
    (bytes, value)
}

#[test]
fn golden_frames_decode_and_encode_unchanged() {
    use dosi_tracker_linux::agent::state::SharedState;
    use dosi_tracker_linux::ipc::{decode, PauseParams, Request, ServerFrame, SignInParams};

    for name in [
        "hello.request.json",
        "pause.request.json",
        "sign_in.request.json",
    ] {
        let (bytes, value) = fixture(name);
        let request: Request = decode(bytes.trim_ascii_end()).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), value, "{name}");
    }
    let (_, pause) = fixture("pause.request.json");
    let params: PauseParams = serde_json::from_value(pause["params"].clone()).unwrap();
    assert_eq!(params.resume_after_secs, Some(900));
    let (_, sign_in) = fixture("sign_in.request.json");
    let params: SignInParams = serde_json::from_value(sign_in["params"].clone()).unwrap();
    assert_eq!(params.remember_password, Some(true));
    assert!(!format!("{params:?}").contains("<the password>"));

    for name in [
        "hello.response.json",
        "pause.response.json",
        "consent_required.response.json",
    ] {
        let (bytes, value) = fixture(name);
        match decode::<ServerFrame>(bytes.trim_ascii_end()).unwrap() {
            ServerFrame::Response(response) => {
                assert_eq!(serde_json::to_value(&response).unwrap(), value, "{name}")
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    let (_, hello) = fixture("hello.response.json");
    let result: HelloResult = serde_json::from_value(hello["result"].clone()).unwrap();
    assert_eq!(result.protocol, 1);

    // The snapshot: the default state is exactly the golden event's data, so
    // a renamed or removed field breaks this test (the GUI reads them).
    let (bytes, value) = fixture("state.event.json");
    match decode::<ServerFrame>(&bytes).unwrap() {
        ServerFrame::Event(event) => {
            assert_eq!((event.event.as_str(), event.seq), ("state", 42));
            let state: SharedState = serde_json::from_value(event.data.clone()).unwrap();
            assert_eq!(serde_json::to_value(state).unwrap(), event.data);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        dosi_tracker_linux::ipc::snapshot_value(SharedState::default()),
        value["data"]
    );
    support::schema::assert_valid(&support::schema::load("snapshot.v1.json"), &value["data"]);
}
