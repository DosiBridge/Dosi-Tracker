//! `dosi-tracker daemon`: refuses root, umask 077, single-instance lock,
//! logging, session probe, thread wiring, sd_notify and the notice relay.
//!
//! The main thread (design 3.2):
//! 1. refuses euid 0 (exit 9), sets umask 077, needs `XDG_RUNTIME_DIR`
//!    (exit 9) and takes `daemon.lock` for life (exit 8 when another daemon
//!    holds it);
//! 2. logs to stderr for journald (`--systemd` or `JOURNAL_STREAM`), else to
//!    daily files in `$XDG_STATE_HOME/dosi-tracker/logs` (7 kept). Logs never
//!    carry titles, app names, tokens or passwords;
//! 3. loads the configuration, probes the session (logind first), resolves
//!    the credential store and builds the tracker for the session;
//! 4. spawns the worker (with its control task and uploader), the session
//!    events, the notice relay ([`relay`]), the X connection watch and the IPC
//!    server ([`methods`] maps the requests to worker commands);
//! 5. tells systemd `READY=1` once IPC listens, then waits: SIGTERM, SIGINT
//!    and SIGHUP save the block, send the farewell and give uploads at most
//!    15 s (exit within 20 s); the worker quitting by itself (session removed,
//!    X connection lost, `daemon.shutdown`) ends the daemon with 0;
//! 6. every 60 s checks whether its executable was replaced (package
//!    upgrade); at the next block boundary it saves and exits 75, so the unit
//!    restarts the new binary (a detached daemon re-executes itself).
//!
//! Builds with `test-support` can run the worker on a fake machine
//! (`DOSI_TEST_MACHINE=fake`, [`fake`]); release builds cannot.

pub mod methods;
pub mod relay;

#[cfg(feature = "test-support")]
pub mod fake;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::mpsc::UnboundedSender;

use super::config::{AppConfig, ConfigSources};
use super::session_events::{SessionEvents, SessionEventsConfig, SystemEvent, SystemNotice};
use super::state::{Command, DaemonInfo, StateHandle, TrackerStatus};
use super::worker::{self, Launch, Machine, WorkerConfig};
use crate::consent::ConsentManager;
use crate::credentials::{CredentialError, CredentialStore, CredentialStoreKind, Credentials};
use crate::ipc::exit;
use crate::lifecycle::TrackerLifecycle;
use crate::paths::LinuxPaths;
use crate::tracking::session_probe::{self, SessionEnv, SessionInfo, SessionKind};
use crate::tracking::system::DeviceInfo;

/// How often the daemon checks whether its executable was replaced.
pub const EXE_CHECK_EVERY: Duration = Duration::from_secs(60);

/// How long a stop may take before the daemon exits anyway.
pub const STOP_BUDGET: Duration = Duration::from_secs(20);

/// The machine's clock (the fake machine's runs faster): every `at` the
/// daemon itself stamps comes from here, so blocks end where the worker
/// expects them.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// `daemon` arguments.
#[derive(Debug, Clone, Default)]
pub struct DaemonArgs {
    /// Started by the systemd user unit (log to the journal).
    pub systemd: bool,
    /// No supervisor: detach from the terminal; re-execute after an upgrade.
    pub detach: bool,
    /// `error`, `warn`, `info`, `debug` or `trace` (default: `log.level`).
    pub log_level: Option<String>,
    /// The socket (tests); default `$XDG_RUNTIME_DIR/dosi-tracker/daemon.sock`.
    pub socket: Option<PathBuf>,
}

/// Every [`Command`] goes through here, numbered, so a sender can wait until
/// the worker applied it (`SharedState::commands_handled`).
#[derive(Clone)]
pub struct CommandSink {
    tx: UnboundedSender<Command>,
    sent: Arc<Mutex<u64>>,
}

impl CommandSink {
    pub fn new(tx: UnboundedSender<Command>) -> Self {
        Self {
            tx,
            sent: Arc::new(Mutex::new(0)),
        }
    }

    /// Queue `command`; returns its sequence number.
    pub fn send(&self, command: Command) -> u64 {
        let mut sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        *sent += 1;
        if self.tx.send(command).is_err() {
            tracing::warn!("the worker is gone; command dropped");
        }
        *sent
    }
}

/// A credential store that is not there (`credentials.store = auto` without a
/// Secret Service): every call fails with the reason, so the worker keeps the
/// queue and the CLI explains what to do.
pub struct UnavailableStore {
    reason: String,
}

impl UnavailableStore {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn error(&self) -> CredentialError {
        CredentialError::NoSecretService(self.reason.clone())
    }
}

impl CredentialStore for UnavailableStore {
    fn load(&self, _: &str, _: Option<&str>) -> Result<Option<Credentials>, CredentialError> {
        Err(self.error())
    }

    fn save(&self, _: &Credentials) -> Result<(), CredentialError> {
        Err(self.error())
    }

    fn clear(&self, _: &str, _: Option<&str>) -> Result<(), CredentialError> {
        Err(self.error())
    }

    fn kind(&self) -> CredentialStoreKind {
        CredentialStoreKind::SecretService
    }
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The socket `args` asks for: `--socket`, `DOSI_SOCKET`, or the default.
pub fn socket_path(paths: &LinuxPaths, explicit: Option<&Path>) -> Option<PathBuf> {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("DOSI_SOCKET")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        })
        .or_else(|| paths.socket_path())
}

/// The lock next to `socket`.
pub fn lock_path(socket: &Path) -> PathBuf {
    socket.with_file_name("daemon.lock")
}

/// Keeps the log writer alive.
pub struct LogGuard(#[allow(dead_code)] Option<tracing_appender::non_blocking::WorkerGuard>);

/// Log to the journal (stderr) or to daily files in `logs_dir` (7 kept).
pub fn init_logging(level: &str, journal: bool, logs_dir: &Path) -> LogGuard {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::new(format!("warn,dosi_tracker_linux={level}"));
    if journal {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_writer(std::io::stderr)
            .try_init();
        return LogGuard(None);
    }
    let appender = crate::paths::ensure_private_dir(logs_dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("dosi-tracker")
                .filename_suffix("log")
                .max_log_files(7)
                .build(logs_dir)
                .map_err(|e| e.to_string())
        });
    match appender {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_target(false)
                .with_writer(writer)
                .try_init();
            LogGuard(Some(guard))
        }
        Err(why) => {
            eprintln!(
                "dosi-tracker: cannot write logs to {}: {why}",
                logs_dir.display()
            );
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .try_init();
            LogGuard(None)
        }
    }
}

/// Was the daemon started by the login hook? The hook sets `DOSI_LAUNCH=login`
/// in the session env file it has just written; the file lives on for later
/// restarts, so it counts only while it is fresh.
pub fn launched_at_login(env: &[(String, String)], paths: &LinuxPaths) -> bool {
    let marked = env.iter().any(|(k, v)| {
        k == crate::service::LAUNCH_VAR && v.trim() == crate::service::LAUNCH_AT_LOGIN
    });
    marked
        && paths
            .session_env_file()
            .and_then(|file| std::fs::metadata(file).ok())
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < LOGIN_LAUNCH_WINDOW)
}

/// How long after the login hook wrote the env file a start counts as a
/// login autostart.
pub const LOGIN_LAUNCH_WINDOW: Duration = Duration::from_secs(120);

/// The executable as it was at start, to notice a package upgrade.
#[derive(Debug, Clone)]
pub struct ExeIdentity {
    pub path: PathBuf,
    dev: u64,
    ino: u64,
    mtime: i64,
}

impl ExeIdentity {
    pub fn current() -> Option<Self> {
        let path = std::env::current_exe().ok()?;
        Self::of(&path)
    }

    pub fn of(path: &Path) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            path: path.to_path_buf(),
            dev: meta.dev(),
            ino: meta.ino(),
            mtime: meta.mtime(),
        })
    }

    /// Was the running binary deleted or replaced at its path?
    pub fn replaced(&self) -> bool {
        let deleted = std::fs::read_link("/proc/self/exe")
            .map(|p| p.to_string_lossy().ends_with(" (deleted)"))
            .unwrap_or(false);
        deleted
            || match Self::of(&self.path) {
                Some(now) => (now.dev, now.ino, now.mtime) != (self.dev, self.ino, self.mtime),
                None => true,
            }
    }
}

/// Hook for the tray (step L7): start the StatusNotifierItem and return
/// whether a tray host shows it. Until then there is no tray.
fn spawn_tray(_state: &StateHandle, _sink: &CommandSink) -> bool {
    // L7: spawn the daemon-owned tray here (agent::tray).
    false
}

fn system_bus(env: &SessionEnv) -> Option<zbus::blocking::Connection> {
    match crate::tracking::dbus_idle::connect_bus(env.system_bus_address()) {
        Ok(conn) => Some(conn),
        Err(e) => {
            tracing::info!(error = %e, "no system bus: session detection uses the environment");
            None
        }
    }
}

fn build_machine(
    info: &SessionInfo,
    env: &SessionEnv,
    system: Option<&zbus::blocking::Connection>,
    lifecycle: Arc<Mutex<TrackerLifecycle>>,
    device: DeviceInfo,
    vars: &[(String, String)],
) -> Arc<dyn Machine> {
    #[cfg(feature = "test-support")]
    if vars
        .iter()
        .any(|(k, v)| k == "DOSI_TEST_MACHINE" && v == "fake")
    {
        tracing::warn!("test build: the worker runs on a fake machine");
        return Arc::new(fake::FakeMachine::from_env(device, vars));
    }
    let _ = vars;
    let session = match (info.kind, env.dbus_session_bus_address.as_deref()) {
        (SessionKind::Wayland, Some(address)) => {
            crate::tracking::dbus_idle::connect_bus(address).ok()
        }
        _ => None,
    };
    let buses = crate::tracking::dbus_idle::IdleBuses {
        session,
        system: system.cloned(),
        logind_session: info.session_path.clone(),
    };
    Arc::new(crate::tracking::tracker::Tracker::for_session(
        info,
        &buses,
        env.dbus_session_bus_address.as_deref(),
        lifecycle,
        device,
    ))
}

/// Watch the X connection on `dosi-xwatch`: when the X server goes away, the
/// session is over (end the block, farewell, exit 0).
fn watch_x_connection(display: &str, system: UnboundedSender<SystemNotice>) {
    let display = display.to_owned();
    let spawned = std::thread::Builder::new()
        .name("dosi-xwatch".into())
        .spawn(move || {
            use x11rb::connection::Connection;
            let conn = match x11rb::rust_connection::RustConnection::connect(Some(&display)) {
                Ok((conn, _)) => conn,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot watch the X connection");
                    return;
                }
            };
            loop {
                if conn.wait_for_event().is_err() {
                    break;
                }
            }
            tracing::warn!("the X connection was lost; the session is over");
            let _ = system.send(SystemNotice {
                event: SystemEvent::SessionRemoved,
                at: Utc::now(),
                done: None,
            });
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not start the X connection watch");
    }
}

/// Run the daemon; returns its exit code.
pub fn run(args: DaemonArgs) -> u8 {
    if euid() == 0 {
        eprintln!("dosi-tracker: refusing to run as root; the agent runs as the signed-in user");
        return exit::ENVIRONMENT;
    }
    // SAFETY: umask only swaps the process file-mode mask.
    unsafe { libc::umask(0o077) };
    if args.detach {
        // SAFETY: setsid has no preconditions; it fails harmlessly when the
        // process already leads a session.
        unsafe { libc::setsid() };
    }
    let vars: Vec<(String, String)> = std::env::vars().collect();
    let paths = LinuxPaths::from_env();
    if paths.runtime_dir().is_none() {
        eprintln!("dosi-tracker: XDG_RUNTIME_DIR is not set; the agent needs a login session's runtime directory");
        return exit::ENVIRONMENT;
    }
    let Some(socket) = socket_path(&paths, args.socket.as_deref()) else {
        eprintln!("dosi-tracker: no socket path");
        return exit::ENVIRONMENT;
    };
    let lock = match crate::ipc::server::DaemonLock::try_acquire(&lock_path(&socket)) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            eprintln!("dosi-tracker: another dosi-tracker daemon is already running");
            return exit::PRECONDITION;
        }
        Err(e) => {
            eprintln!("dosi-tracker: cannot take the daemon lock: {e}");
            return exit::ENVIRONMENT;
        }
    };

    let loaded = AppConfig::load(&ConfigSources::for_paths(&paths, vars.iter().cloned()));
    let cfg = loaded.config;
    let level = args
        .log_level
        .clone()
        .unwrap_or_else(|| cfg.log.level.as_str().to_owned());
    let journal = args.systemd || vars.iter().any(|(k, _)| k == "JOURNAL_STREAM");
    let _log = init_logging(&level, journal, &paths.logs_dir());
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "the agent is starting");
    for issue in &loaded.issues {
        tracing::warn!(%issue, "configuration issue");
    }

    let started_at = Utc::now();
    let exe = ExeIdentity::current();
    let env = SessionEnv::from_vars(vars.iter().cloned());
    let system = system_bus(&env);
    let info = session_probe::probe(&env, system.as_ref(), euid());
    tracing::info!(kind = ?info.kind, source = ?info.source, "session detected");

    let device = match DeviceInfo::collect(&paths.device_file()) {
        Ok(device) => device,
        Err(e) => {
            tracing::error!(error = %e, "the device id cannot be stored");
            eprintln!("dosi-tracker: the device id cannot be stored: {e}");
            return exit::ENVIRONMENT;
        }
    };

    let (consent, consent_error) = ConsentManager::load_or_fail_closed(&paths.consent_file);
    if let Some(e) = consent_error {
        tracing::warn!(error = %e, "the consent file cannot be used; nothing is recorded until consent is given again");
    }
    let lifecycle = Arc::new(Mutex::new(TrackerLifecycle::new(consent)));

    let (credentials, credential_problem): (Arc<dyn CredentialStore>, Option<String>) =
        match crate::credentials::resolve_store(
            cfg.credentials.store,
            &paths,
            cfg.credentials.remember_password,
        ) {
            Ok(store) => (Arc::from(store), None),
            Err(e) => {
                tracing::warn!(error = %e, "no credential store");
                let reason = e.to_string();
                (
                    Arc::new(UnavailableStore::new(reason.clone())),
                    Some(reason),
                )
            }
        };

    let machine = build_machine(
        &info,
        &env,
        system.as_ref(),
        lifecycle.clone(),
        device,
        &vars,
    );
    let clock: Clock = {
        let machine = machine.clone();
        Arc::new(move || machine.now())
    };
    let state = StateHandle::new();

    // Notifications first: whether one can be shown decides whether tracking
    // may resume by itself (invariant I5).
    let (relay_tx, relay_rx) = std::sync::mpsc::channel();
    let notifier = match (
        cfg.notifications.enabled,
        env.dbus_session_bus_address.as_deref(),
    ) {
        (true, Some(address)) => {
            let tx = relay_tx.clone();
            match super::notify::Notifier::connect(address, move |event| {
                let _ = tx.send(relay::RelayMessage::Notification(event));
            }) {
                Ok(notifier) => Some(Arc::new(notifier)),
                Err(e) => {
                    tracing::info!(error = %e, "no notification server");
                    None
                }
            }
        }
        _ => None,
    };
    state.update(|s| {
        s.daemon = Some(DaemonInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
        });
        s.indicator.notifications = notifier.is_some();
    });

    let handle = match worker::spawn(WorkerConfig {
        state: state.clone(),
        cfg: cfg.clone(),
        machine: machine.clone(),
        lifecycle,
        credentials: credentials.clone(),
        outbox_path: paths.outbox_db(),
        session_file: Some(paths.session_file()),
        launch: Launch {
            autostart: launched_at_login(&vars, &paths),
            // The tray is decided below; notifications already are.
            indicator: notifier.is_some(),
        },
    }) {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(error = %e, "the worker cannot start");
            return exit::FAILED;
        }
    };
    let sink = CommandSink::new(handle.commands.clone());
    let tray = spawn_tray(&state, &sink);
    state.update(|s| s.indicator.tray = tray);

    // Session events: lock, sleep, shutdown, session removed.
    let events = SessionEvents::spawn(
        SessionEventsConfig {
            system_bus: system.as_ref().map(|_| env.system_bus_address().to_owned()),
            session_bus: env.dbus_session_bus_address.clone(),
            session_path: info.session_path.clone(),
        },
        handle.system.clone(),
    );
    let inhibitor = match &events {
        Ok(events) => {
            let initial = events.initial();
            let now = clock();
            if initial.locked {
                let _ = handle.system.send(SystemNotice {
                    event: SystemEvent::Locked,
                    at: now,
                    done: None,
                });
            }
            if !initial.active {
                let _ = handle.system.send(SystemNotice {
                    event: SystemEvent::Disconnected,
                    at: now,
                    done: None,
                });
            }
            Some(events.control())
        }
        Err(e) => {
            tracing::info!(error = %e, "no session events");
            None
        }
    };
    if let (SessionKind::X11, Some(display)) = (info.kind, info.display.as_deref()) {
        if !machine_is_fake(&vars) {
            watch_x_connection(display, handle.system.clone());
        }
    }

    let relay = relay::spawn(relay::RelayConfig {
        state: state.clone(),
        sink: sink.clone(),
        notifier: notifier.clone(),
        inhibitor,
        notifications_enabled: cfg.notifications.enabled,
        rx: relay_rx,
        gui_exe: gui_exe(),
        clock: clock.clone(),
    });

    let listener = match crate::ipc::server::bind(&socket, &lock) {
        Ok(listener) => listener,
        Err(crate::ipc::server::BindError::Unsafe(why)) => {
            tracing::error!(%why, "unsafe IPC socket");
            eprintln!("dosi-tracker: {why}");
            sink.send(Command::Shutdown { at: clock() });
            wait_finished(&handle.thread, STOP_BUDGET);
            return exit::ENVIRONMENT;
        }
        Err(e) => {
            tracing::error!(error = %e, "cannot listen");
            eprintln!("dosi-tracker: {e}");
            sink.send(Command::Shutdown { at: clock() });
            wait_finished(&handle.thread, STOP_BUDGET);
            return exit::FAILED;
        }
    };
    let dispatch = Arc::new(methods::Methods::new(methods::MethodsConfig {
        state: state.clone(),
        sink: sink.clone(),
        paths: paths.clone(),
        cfg: cfg.clone(),
        credentials,
        credential_problem,
        started_at,
        socket: socket.clone(),
        relay: relay.sender(),
        session_kind: info.kind,
        clock: clock.clone(),
    }));
    let server = match crate::ipc::server::spawn(
        listener,
        crate::ipc::server::ServerConfig::default(),
        dispatch,
    ) {
        Ok(server) => server,
        Err(e) => {
            tracing::error!(error = %e, "the IPC server cannot start");
            let _ = std::fs::remove_file(&socket);
            sink.send(Command::Shutdown { at: clock() });
            wait_finished(&handle.thread, STOP_BUDGET);
            return exit::FAILED;
        }
    };
    let _ = sd_notify::notify(
        false,
        &[
            sd_notify::NotifyState::Ready,
            sd_notify::NotifyState::Status("Running"),
        ],
    );
    tracing::info!("the agent is ready");

    let exe_check = exe_check_interval(&vars);
    let outcome = main_loop(
        &state,
        &sink,
        &clock,
        &handle.thread,
        exe.as_ref(),
        exe_check,
    );

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]);
    server.stop();
    let _ = std::fs::remove_file(&socket);
    drop(events);
    relay.stop();
    drop(notifier);
    tracing::info!(?outcome, "the agent stopped");
    match outcome {
        Outcome::Stopped => exit::OK,
        Outcome::Upgraded => {
            if args.detach {
                if let Some(exe) = exe {
                    drop(lock);
                    reexec(&exe.path);
                }
            }
            exit::RESTART
        }
    }
}

#[cfg(feature = "test-support")]
fn machine_is_fake(vars: &[(String, String)]) -> bool {
    vars.iter()
        .any(|(k, v)| k == "DOSI_TEST_MACHINE" && v == "fake")
}

/// Release builds have no fake machine (and no trace of its knobs).
#[cfg(not(feature = "test-support"))]
fn machine_is_fake(_vars: &[(String, String)]) -> bool {
    false
}

fn exe_check_interval(vars: &[(String, String)]) -> Duration {
    #[cfg(feature = "test-support")]
    if let Some(secs) = vars
        .iter()
        .find(|(k, _)| k == "DOSI_TEST_EXE_CHECK_SECS")
        .and_then(|(_, v)| v.parse::<u64>().ok())
    {
        return Duration::from_secs(secs.max(1));
    }
    let _ = vars;
    EXE_CHECK_EVERY
}

/// `dosi-tracker-gui` next to this binary, if installed.
fn gui_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let gui = exe.with_file_name("dosi-tracker-gui");
    gui.is_file().then_some(gui)
}

/// Replace this process with the (new) binary at `path` (detached mode).
fn reexec(path: &Path) {
    use std::os::unix::process::CommandExt;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    tracing::info!("re-executing the updated binary");
    let error = std::process::Command::new(path).args(args).exec();
    tracing::error!(%error, "could not re-execute; exiting for a restart");
}

fn wait_finished(thread: &std::thread::JoinHandle<()>, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while !thread.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// Why the main loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Stopped,
    /// The executable was replaced: restart into the new one.
    Upgraded,
}

fn main_loop(
    state: &StateHandle,
    sink: &CommandSink,
    clock: &Clock,
    worker: &std::thread::JoinHandle<()>,
    exe: Option<&ExeIdentity>,
    exe_check: Duration,
) -> Outcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!(error = %e, "the main loop cannot start");
            sink.send(Command::Shutdown { at: clock() });
            wait_finished(worker, STOP_BUDGET);
            return Outcome::Stopped;
        }
    };
    runtime.block_on(async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).ok();
        let mut int = signal(SignalKind::interrupt()).ok();
        let mut hup = signal(SignalKind::hangup()).ok();
        async fn next(signal: &mut Option<tokio::signal::unix::Signal>) {
            match signal {
                Some(signal) => {
                    signal.recv().await;
                }
                None => std::future::pending().await,
            }
        }
        let mut poll = tokio::time::interval(Duration::from_millis(200));
        let mut exe_tick = tokio::time::interval(exe_check);
        exe_tick.tick().await;
        // Some(block start at detection) once an upgrade is waiting.
        let mut upgrade: Option<Option<DateTime<Utc>>> = None;
        loop {
            tokio::select! {
                _ = next(&mut term) => break stop(sink, clock, worker, Outcome::Stopped, "SIGTERM").await,
                _ = next(&mut int) => break stop(sink, clock, worker, Outcome::Stopped, "SIGINT").await,
                _ = next(&mut hup) => break stop(sink, clock, worker, Outcome::Stopped, "SIGHUP").await,
                _ = poll.tick() => {
                    if worker.is_finished() {
                        break if upgrade.is_some() { Outcome::Upgraded } else { Outcome::Stopped };
                    }
                    if let Some(started) = upgrade {
                        let snapshot = state.snapshot();
                        let boundary = snapshot.status != TrackerStatus::Tracking
                            || snapshot.block_started_at != started;
                        if boundary {
                            break stop(sink, clock, worker, Outcome::Upgraded, "upgrade").await;
                        }
                    }
                }
                _ = exe_tick.tick() => {
                    if upgrade.is_none() && exe.is_some_and(ExeIdentity::replaced) {
                        tracing::info!("the executable was replaced; restarting at the next block boundary");
                        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Status("Updated: restarting at the next block boundary")]);
                        upgrade = Some(state.snapshot().block_started_at);
                    }
                }
            }
        }
    })
}

async fn stop(
    sink: &CommandSink,
    clock: &Clock,
    worker: &std::thread::JoinHandle<()>,
    outcome: Outcome,
    why: &str,
) -> Outcome {
    tracing::info!(%why, "stopping: saving the block in progress");
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]);
    sink.send(Command::Shutdown { at: clock() });
    let deadline = tokio::time::Instant::now() + STOP_BUDGET;
    while !worker.is_finished() {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("the worker did not stop in time");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_numbered_in_the_order_they_are_sent() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = CommandSink::new(tx);
        let other = sink.clone();
        assert_eq!(sink.send(Command::RefreshProjects), 1);
        assert_eq!(
            other.send(Command::RetryCapture),
            2,
            "clones share the counter"
        );
        assert!(matches!(rx.try_recv(), Ok(Command::RefreshProjects)));
        assert!(matches!(rx.try_recv(), Ok(Command::RetryCapture)));
        drop(rx);
        assert_eq!(
            sink.send(Command::RefreshProjects),
            3,
            "a gone worker still counts"
        );
    }

    #[test]
    fn the_login_marker_counts_only_while_the_env_file_is_fresh() {
        let temp = tempfile::tempdir().unwrap();
        let paths = LinuxPaths::resolve(Some(temp.path()), None, None, None)
            .with_runtime_dir(Some(temp.path()));
        let marked = vec![("DOSI_LAUNCH".to_string(), "login".to_string())];
        assert!(!launched_at_login(&marked, &paths), "no env file");
        std::fs::write(paths.session_env_file().unwrap(), "DOSI_LAUNCH=\"login\"").unwrap();
        assert!(launched_at_login(&marked, &paths));
        assert!(!launched_at_login(&[], &paths), "not marked");
        let old = std::time::SystemTime::now() - (LOGIN_LAUNCH_WINDOW + Duration::from_secs(5));
        std::fs::File::options()
            .write(true)
            .open(paths.session_env_file().unwrap())
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(
            !launched_at_login(&marked, &paths),
            "a later restart is no login"
        );
    }

    #[test]
    fn a_replaced_or_removed_executable_is_noticed() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("dosi-tracker");
        std::fs::write(&exe, b"v1").unwrap();
        let identity = ExeIdentity::of(&exe).unwrap();
        assert_eq!(identity.path, exe);
        let staged = temp.path().join(".new");
        std::fs::write(&staged, b"v2").unwrap();
        std::fs::rename(&staged, &exe).unwrap();
        assert!(identity.replaced(), "a new inode at the path");
        std::fs::remove_file(&exe).unwrap();
        assert!(identity.replaced(), "gone");
    }

    #[test]
    fn the_socket_and_its_lock_live_in_the_runtime_dir() {
        let paths = LinuxPaths::resolve(Some(Path::new("/home/u")), None, None, None)
            .with_runtime_dir(Some(Path::new("/run/user/1000")));
        let socket = socket_path(&paths, None).unwrap();
        assert_eq!(socket, Path::new("/run/user/1000/dosi-tracker/daemon.sock"));
        assert_eq!(
            lock_path(&socket),
            Path::new("/run/user/1000/dosi-tracker/daemon.lock")
        );
        let explicit = socket_path(&paths, Some(Path::new("/tmp/t/x.sock"))).unwrap();
        assert_eq!(lock_path(&explicit), Path::new("/tmp/t/daemon.lock"));
        let none = LinuxPaths::resolve(Some(Path::new("/home/u")), None, None, None);
        if std::env::var_os("DOSI_SOCKET").is_none() {
            assert!(
                socket_path(&none, None).is_none(),
                "no runtime dir, no socket"
            );
        }
    }

    #[test]
    fn an_unavailable_store_explains_itself() {
        let store = UnavailableStore::new("no keyring");
        match store.load_account("a@b.test") {
            Err(CredentialError::NoSecretService(why)) => assert_eq!(why, "no keyring"),
            other => panic!("{other:?}"),
        }
        assert!(store.save(&Credentials::new("a@b.test", None)).is_err());
    }
}
