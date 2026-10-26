//! `dosi-tracker-gui` (feature `gui`): eframe/egui thin client of the daemon,
//! ported from the Windows agent's UI.
//!
//! The window never records anything itself: [`link`] keeps a live copy of
//! the daemon's snapshot and sends each action as one IPC request. Without a
//! daemon the window says so (with Start and Start-when-I-log-in) instead of
//! showing stale data as live; a daemon of another protocol major makes it
//! read-only with a restart banner. Closing the window ends this process —
//! tracking goes on in the daemon.
//!
//! - One window per user: [`run`] takes `gui.lock` (`flock`); a second
//!   launch asks the daemon to `ui.show` the first one and exits 0.
//! - Backend: `--backend x11|wayland`, `DOSI_GUI_BACKEND` or `gui.backend`;
//!   `auto` picks X11 when `DISPLAY` is set (XWayland on GNOME/KDE), else
//!   Wayland ([`choose_backend`]). The backend never affects capture.
//! - App id `io.github.dosibridge.DosiTracker`; refuses root; SIGTERM,
//!   SIGINT and SIGHUP close the window cleanly.
//! - Builds with `test-support` add `--preview <scene>` (synthetic data only,
//!   [`preview`]) and a layout probe for the Xvfb smoke test.

pub mod activity;
pub mod alerts;
pub mod consent;
pub mod format;
pub mod link;
pub mod login;
#[cfg(feature = "test-support")]
pub mod preview;
pub mod settings;
pub mod theme;
pub mod timer;
pub mod widgets;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use eframe::egui;
use egui::{Align, Color32, Layout, Pos2, Rect, Stroke, UiBuilder, Vec2};
use serde_json::{json, Value};

use crate::agent::config::{AppConfig, ConfigSources, GuiBackend};
use crate::agent::state::{NoticeKind, PendingAction, SharedState, TrackerStatus, UploadIssue};
use crate::ipc::{method, ErrorBody};
use crate::paths::{LinuxPaths, APP_ID};
use link::{Link, LinkStatus, Replied};
use widgets::{ButtonKind, Icon, Tone};

/// Window title, also how the smoke test finds the window.
pub const WINDOW_TITLE: &str = "Dosi Tracker";

/// The timed pauses offered next to the pause button (and in the tray).
pub const PAUSE_CHOICES: [(&str, Duration); 3] = [
    ("15 min", Duration::from_secs(15 * 60)),
    ("30 min", Duration::from_secs(30 * 60)),
    ("1 hour", Duration::from_secs(60 * 60)),
];

/// The window's preferred inner size.
pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [480.0, 780.0];
/// Small enough for a 1366×768 laptop at 125 % scaling.
pub const MIN_WINDOW_SIZE: [f32; 2] = [400.0, 520.0];

/// Content wider than this gets a two-column layout.
const WIDE_LAYOUT: f32 = 760.0;
const HEADER_H: f32 = 60.0;
const TABS_H: f32 = 56.0;
const FOOTER_H: f32 = 30.0;
/// Below this (either way) the window is not really on screen.
const MIN_LAYOUT_SIZE: f32 = 200.0;

/// How long a message at the top of the window stays once seen.
const TOAST_FOR: Duration = Duration::from_secs(8);
/// At most this many at once (the newest win).
const MAX_TOASTS: usize = 3;
/// The device's time zone is read again this often (it follows travel).
const ZONE_REFRESH: Duration = Duration::from_secs(60);

/// Why the dashboard buttons are greyed out.
pub const DASHBOARD_UNSET: &str = "The web dashboard's address isn't set on this computer. Ask your admin, or run dosi-tracker config set dashboard_url https://…";

// ---- Start-up ------------------------------------------------------------------------------

/// The windowing backend the GUI runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    X11,
    Wayland,
}

/// Pick the backend: the flag, else `DOSI_GUI_BACKEND`, else `gui.backend`;
/// `auto` is X11 when `DISPLAY` is set, else Wayland. A choice whose display
/// is missing is an error, worded for the user.
pub fn choose_backend(
    flag: Option<GuiBackend>,
    env: Option<&str>,
    config: GuiBackend,
    display: bool,
    wayland_display: bool,
) -> Result<Backend, String> {
    let wanted = match (flag, env.map(str::trim).filter(|v| !v.is_empty())) {
        (Some(flag), _) => flag,
        (None, Some(value)) => GuiBackend::parse(value).ok_or_else(|| {
            format!("DOSI_GUI_BACKEND must be auto, x11 or wayland (not {value})")
        })?,
        (None, None) => config,
    };
    match wanted {
        GuiBackend::X11 if display => Ok(Backend::X11),
        GuiBackend::X11 => Err("the X11 backend needs DISPLAY".to_owned()),
        GuiBackend::Wayland if wayland_display => Ok(Backend::Wayland),
        GuiBackend::Wayland => Err("the Wayland backend needs WAYLAND_DISPLAY".to_owned()),
        GuiBackend::Auto if display => Ok(Backend::X11),
        GuiBackend::Auto if wayland_display => Ok(Backend::Wayland),
        GuiBackend::Auto => {
            Err("no graphical display (neither DISPLAY nor WAYLAND_DISPLAY is set)".to_owned())
        }
    }
}

/// `dosi-tracker-gui` arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub version: bool,
    pub help: bool,
    pub backend: Option<GuiBackend>,
    /// `timer`, `task`, `activity` or `settings`.
    pub view: Option<String>,
    pub socket: Option<PathBuf>,
    /// Test builds only: a synthetic scene.
    pub preview: Option<String>,
}

pub const USAGE: &str = "usage: dosi-tracker-gui [--backend auto|x11|wayland] [--view timer|task|activity|settings] [--version]";

/// Parse the command line (without the program name).
pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .ok_or_else(|| format!("{name} needs {what}"))
        };
        match name {
            "--version" | "-V" => parsed.version = true,
            "--help" | "-h" => parsed.help = true,
            "--backend" => {
                let v = value("auto, x11 or wayland")?;
                parsed.backend =
                    Some(GuiBackend::parse(&v).ok_or_else(|| {
                        format!("--backend must be auto, x11 or wayland (not {v})")
                    })?);
            }
            "--view" => {
                let v = value("a view")?;
                if !matches!(v.as_str(), "timer" | "task" | "activity" | "settings") {
                    return Err(format!(
                        "--view must be timer, task, activity or settings (not {v})"
                    ));
                }
                parsed.view = Some(v);
            }
            "--socket" => parsed.socket = Some(PathBuf::from(value("a path")?)),
            #[cfg(feature = "test-support")]
            "--preview" => parsed.preview = Some(value("a scene")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(parsed)
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Block SIGTERM, SIGINT and SIGHUP in every thread (call before any thread
/// starts) and turn them into `quit` plus a close request on `dosi-gui-signals`.
fn watch_signals(quit: Arc<AtomicBool>, ctx: Arc<Mutex<Option<egui::Context>>>) {
    // SAFETY: plain signal-set manipulation on a local set; pthread_sigmask
    // only changes this thread's mask (inherited by threads started later).
    let set = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGHUP);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    };
    let spawned = std::thread::Builder::new()
        .name("dosi-gui-signals".into())
        .spawn(move || loop {
            let mut signal = 0;
            // SAFETY: `set` lives in this closure; sigwait only reads it.
            let rc = unsafe { libc::sigwait(&set, &mut signal) };
            if rc != 0 {
                continue;
            }
            tracing::info!(signal, "closing the window");
            quit.store(true, Ordering::SeqCst);
            if let Some(ctx) = ctx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "signals will end the window abruptly");
    }
}

/// Run the window; returns the exit code.
pub fn run(args: &[String]) -> u8 {
    let args = match parse_args(args) {
        Ok(args) => args,
        Err(why) => {
            eprintln!("dosi-tracker-gui: {why}\n{USAGE}");
            return crate::ipc::exit::USAGE;
        }
    };
    if args.version {
        println!("dosi-tracker-gui {}", env!("CARGO_PKG_VERSION"));
        return crate::ipc::exit::OK;
    }
    if args.help {
        println!("{USAGE}");
        return crate::ipc::exit::OK;
    }
    if euid() == 0 {
        eprintln!(
            "dosi-tracker-gui: refusing to run as root; open the window as the signed-in user"
        );
        return crate::ipc::exit::ENVIRONMENT;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "warn,dosi_tracker_linux=info",
        ))
        .with_writer(std::io::stderr)
        .try_init();

    let vars: Vec<(String, String)> = std::env::vars().collect();
    let var = |name: &str| {
        vars.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    };
    let paths = LinuxPaths::from_env();
    let config = AppConfig::load(&ConfigSources::for_paths(&paths, vars.iter().cloned())).config;
    let backend = match choose_backend(
        args.backend,
        var("DOSI_GUI_BACKEND"),
        config.gui.backend,
        var("DISPLAY").is_some(),
        var("WAYLAND_DISPLAY").is_some(),
    ) {
        Ok(backend) => backend,
        Err(why) => {
            eprintln!("dosi-tracker-gui: {why}");
            return crate::ipc::exit::ENVIRONMENT;
        }
    };
    let socket = args
        .socket
        .clone()
        .or_else(|| var("DOSI_SOCKET").map(PathBuf::from))
        .or_else(|| paths.socket_path());

    let ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let quit = Arc::new(AtomicBool::new(false));

    #[cfg(feature = "test-support")]
    if let Some(scene) = &args.preview {
        let Some(snapshot) = preview::scene(scene) else {
            eprintln!(
                "dosi-tracker-gui: unknown preview scene {scene} (one of {})",
                preview::SCENES.join(", ")
            );
            return crate::ipc::exit::USAGE;
        };
        watch_signals(quit.clone(), ctx.clone());
        return run_window(
            backend,
            Launch {
                link: Link::preview(snapshot),
                paths,
                config,
                env: vars,
                socket,
                view: args.view,
                quit,
                ctx,
            },
        );
    }

    let Some(socket) = socket else {
        eprintln!("dosi-tracker-gui: XDG_RUNTIME_DIR is not set; the window needs a login session");
        return crate::ipc::exit::ENVIRONMENT;
    };
    // One window per user: a second launch brings the first forward.
    let lock = paths
        .gui_lock_path()
        .map(|path| crate::ipc::server::DaemonLock::try_acquire(&path));
    // Held until the window closes.
    let _held = match lock {
        Some(Ok(Some(lock))) => Some(lock),
        Some(Ok(None)) => return show_running(&socket, args.view.as_deref()),
        Some(Err(e)) => {
            eprintln!("dosi-tracker-gui: cannot take the window lock: {e}");
            return crate::ipc::exit::ENVIRONMENT;
        }
        None => None,
    };
    let repaint_ctx = ctx.clone();
    let repaint: link::Repaint = Arc::new(move || {
        if let Some(ctx) = repaint_ctx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            ctx.request_repaint();
        }
    });
    watch_signals(quit.clone(), ctx.clone());
    let link = Link::start(socket.clone(), repaint);
    run_window(
        backend,
        Launch {
            link,
            paths,
            config,
            env: vars,
            socket: Some(socket),
            view: args.view,
            quit,
            ctx,
        },
    )
}

/// The window is open already: ask the daemon to bring it forward.
fn show_running(socket: &Path, view: Option<&str>) -> u8 {
    match crate::ipc::client::Client::connect(socket, "cli")
        .and_then(|mut client| client.call(method::UI_SHOW, json!({ "view": view })))
    {
        Ok(_) => {}
        Err(e) => eprintln!("dosi-tracker-gui: the window is already open ({e})"),
    }
    crate::ipc::exit::OK
}

/// Everything the window starts with.
pub struct Launch {
    pub link: Link,
    pub paths: LinuxPaths,
    pub config: AppConfig,
    pub env: Vec<(String, String)>,
    pub socket: Option<PathBuf>,
    pub view: Option<String>,
    pub quit: Arc<AtomicBool>,
    /// Filled with the egui context once it exists (repaints, signals).
    pub ctx: Arc<Mutex<Option<egui::Context>>>,
}

fn run_window(backend: Backend, launch: Launch) -> u8 {
    let icon = egui::IconData {
        rgba: crate::agent::tray::icon_rgba(None, 64),
        width: 64,
        height: 64,
    };
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(DEFAULT_WINDOW_SIZE)
        .with_min_inner_size(MIN_WINDOW_SIZE)
        .with_resizable(true)
        .with_title(WINDOW_TITLE)
        .with_app_id(APP_ID)
        .with_icon(Arc::new(icon));
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        event_loop_builder: Some(Box::new(move |builder| match backend {
            Backend::X11 => {
                use winit::platform::x11::EventLoopBuilderExtX11 as _;
                builder.with_x11();
            }
            Backend::Wayland => {
                use winit::platform::wayland::EventLoopBuilderExtWayland as _;
                builder.with_wayland();
            }
        })),
        ..Default::default()
    };
    let result = eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            *launch.ctx.lock().unwrap_or_else(|e| e.into_inner()) = Some(cc.egui_ctx.clone());
            Ok(Box::new(TrackerApp::new(&cc.egui_ctx, launch)) as Box<dyn eframe::App>)
        }),
    );
    match result {
        Ok(()) => crate::ipc::exit::OK,
        Err(e) => {
            eprintln!("dosi-tracker-gui: the window could not open: {e}");
            crate::ipc::exit::FAILED
        }
    }
}

// ---- The app -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Timer,
    Activity,
    Settings,
}

/// A message at the top of the window for a few seconds.
struct Toast {
    tone: Tone,
    text: String,
    /// When it was first on screen: the countdown starts there.
    shown_at: Option<Instant>,
}

/// This computer as the Settings page shows it (read once, never created).
#[derive(Debug, Clone, Default)]
pub struct DeviceSummary {
    pub id: Option<String>,
    pub name: Option<String>,
    pub os: String,
}

impl DeviceSummary {
    pub fn read(paths: &LinuxPaths) -> Self {
        let id = std::fs::read_to_string(paths.device_file())
            .ok()
            .and_then(|text| {
                serde_json::from_str::<crate::tracking::system::DeviceFile>(&text).ok()
            })
            .map(|f| f.device_id.to_string());
        let (node, release) = crate::tracking::system::uname().unzip();
        Self {
            id,
            name: node
                .as_deref()
                .and_then(crate::tracking::system::device_name_from),
            os: crate::tracking::system::os_version_from(Path::new("/"), release.as_deref()),
        }
    }
}

/// A `dosi-tracker` command run for the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    Status,
    Enable,
    Disable,
    Start,
    StartAtLogin,
    Restart,
    Stop,
    Doctor,
}

impl JobKind {
    fn args(self) -> &'static [&'static str] {
        match self {
            JobKind::Status => &["service", "status"],
            JobKind::Enable => &["service", "enable"],
            JobKind::Disable => &["service", "disable"],
            JobKind::Start => &["service", "start"],
            JobKind::StartAtLogin => &["service", "enable", "--now"],
            JobKind::Restart => &["service", "restart"],
            JobKind::Stop => &["service", "stop"],
            JobKind::Doctor => &["doctor"],
        }
    }
}

/// The finished command: its exit code and its JSON.
struct JobOutcome {
    code: Option<i32>,
    json: Value,
}

struct Job {
    kind: JobKind,
    slot: Arc<Mutex<Option<JobOutcome>>>,
}

pub struct TrackerApp {
    pub(crate) link: Link,
    pub(crate) paths: LinuxPaths,
    pub(crate) config: AppConfig,
    env: Vec<(String, String)>,
    socket: Option<PathBuf>,
    pub(crate) view: View,
    pub(crate) login: login::LoginForm,
    pub(crate) task: timer::TaskField,
    pub(crate) project_filter: String,
    pub(crate) consent_vm: consent::ConsentViewModel,
    pub(crate) autostart_enabled: bool,
    pub(crate) settings_error: Option<String>,
    pub(crate) confirm_sign_out: bool,
    pub(crate) confirm_purge: bool,
    /// The lost workspace last shown: a new loss opens Settings once.
    lost_seen: Option<String>,
    pub(crate) device: DeviceSummary,
    zone: Option<(String, Instant)>,
    toasts: Vec<Toast>,
    /// The newest notice already shown (`None` until the first snapshot).
    notices_seen: Option<u64>,
    pub(crate) shot: Option<activity::Shot>,
    /// `daemon.info` of the daemon connected to.
    daemon_info: Option<Value>,
    /// The daemon (pid, start) `daemon.info` was asked of.
    asked_info: Option<(u32, DateTime<Utc>)>,
    pub(crate) service: Option<Value>,
    pub(crate) doctor: Option<Vec<(Tone, String)>>,
    jobs: Vec<Job>,
    history: Option<(Instant, Vec<settings::Receipt>)>,
    quit: Arc<AtomicBool>,
    /// The egui context (background jobs repaint through it).
    ctx: Arc<Mutex<Option<egui::Context>>>,
    /// The window was asked to come forward and paint at once.
    show_requested: bool,
    #[cfg(feature = "test-support")]
    probe: probe::Probe,
}

impl TrackerApp {
    pub fn new(ctx: &egui::Context, launch: Launch) -> Self {
        theme::apply(ctx);
        let snapshot = launch.link.snapshot();
        let entry = launch.paths.autostart_entry();
        let login = login::LoginForm::new(
            launch.config.api_base_url.as_deref(),
            launch.config.credentials.remember_password,
        );
        let mut app = Self {
            consent_vm: consent::ConsentViewModel::new(snapshot.consent),
            autostart_enabled: crate::service::autostart_enabled(&entry),
            device: DeviceSummary::read(&launch.paths),
            link: launch.link,
            paths: launch.paths,
            config: launch.config,
            env: launch.env,
            socket: launch.socket,
            view: View::Timer,
            login,
            task: timer::TaskField::default(),
            project_filter: String::new(),
            settings_error: None,
            confirm_sign_out: false,
            confirm_purge: false,
            lost_seen: None,
            zone: None,
            toasts: Vec::new(),
            notices_seen: None,
            shot: None,
            daemon_info: None,
            asked_info: None,
            service: None,
            doctor: None,
            jobs: Vec::new(),
            history: None,
            quit: launch.quit,
            ctx: launch.ctx,
            show_requested: false,
            #[cfg(feature = "test-support")]
            probe: probe::Probe::from_env(),
        };
        if let Some(view) = launch.view.as_deref() {
            app.open_view(view);
        }
        app
    }

    /// The snapshot is live and requests can be sent.
    pub(crate) fn writable(&self) -> bool {
        self.link.status().is_live()
    }

    /// Record where a control is (test builds: the smoke test clicks it).
    pub(crate) fn probe(&mut self, _name: &'static str, _rect: Rect) {
        #[cfg(feature = "test-support")]
        self.probe.control(_name, _rect);
    }

    fn open_view(&mut self, view: &str) {
        match view {
            "activity" => self.view = View::Activity,
            "settings" => self.view = View::Settings,
            "task" => {
                self.view = View::Timer;
                self.task.focus_requested = true;
            }
            _ => self.view = View::Timer,
        }
    }

    pub(crate) fn daily_goal_secs(&self) -> f64 {
        f64::from(self.config.gui.daily_goal_hours.max(1)) * 3600.0
    }

    pub(crate) fn weekly_goal_secs(&self) -> f64 {
        self.daily_goal_secs() * f64::from(self.config.gui.work_days_per_week.clamp(1, 7))
    }

    /// No keyring: sign-in is refused until one runs or the file store is
    /// allowed.
    pub(crate) fn no_keyring(&self) -> bool {
        self.daemon_info
            .as_ref()
            .and_then(|info| info.get("credentials_store"))
            .and_then(Value::as_str)
            == Some("unavailable")
    }

    // ---- Requests -----------------------------------------------------------------------

    /// Pause (or re-arm a pause). The click's time travels with the request:
    /// the block ends exactly there, however busy the daemon is.
    pub(crate) fn pause(&mut self, snapshot: &SharedState, resume_after: Option<Duration>) {
        if !self.writable() {
            return;
        }
        let idle = snapshot.pending_action.is_none();
        let params = json!({
            "resume_after_secs": resume_after.map(|d| d.as_secs()),
            "at": Utc::now(),
        });
        if snapshot.status == TrackerStatus::Paused {
            // Already paused: only when (or whether) it ends changes.
            self.link.call(method::TRACKING_PAUSE, params);
        } else if idle && format::can_pause(snapshot.status) {
            self.link
                .update_local(|s| s.pending_action = Some(PendingAction::Pausing));
            self.link.call(method::TRACKING_PAUSE, params);
        }
    }

    pub(crate) fn resume(&mut self, snapshot: &SharedState) {
        if self.writable()
            && snapshot.status == TrackerStatus::Paused
            && snapshot.pending_action.is_none()
        {
            self.link
                .update_local(|s| s.pending_action = Some(PendingAction::Resuming));
            self.link
                .call(method::TRACKING_RESUME, json!({ "at": Utc::now() }));
        }
    }

    pub(crate) fn select_project(&mut self, id: String) {
        if self.writable() {
            self.link.call(
                method::PROJECT_SELECT,
                json!({ "project": id, "at": Utc::now() }),
            );
        }
    }

    pub(crate) fn sync_now(&mut self) {
        if self.writable() {
            self.link
                .call(method::TRACKING_SYNC_NOW, json!({ "at": Utc::now() }));
        }
    }

    pub(crate) fn consent_action(&mut self, action: consent::ConsentAction) {
        if !self.writable() {
            return;
        }
        match action {
            consent::ConsentAction::Save(consent) => self.link.call(
                method::CONSENT_UPDATE,
                json!({ "consent": consent, "origin": "gui" }),
            ),
            consent::ConsentAction::TestScreenshotPermission => {
                self.link.call(method::CAPTURE_RETRY_PERMISSION, json!({}))
            }
        }
    }

    /// Change a preference through the daemon (validated there); the local
    /// copy of the configuration is read again once it answers.
    pub(crate) fn set_config(&mut self, key: &str, value: String) {
        if self.writable() {
            self.link
                .call(method::CONFIG_SET, json!({ "key": key, "value": value }));
        }
    }

    fn reload_config(&mut self) {
        self.config = AppConfig::load(&ConfigSources::for_paths(
            &self.paths,
            self.env.iter().cloned(),
        ))
        .config;
    }

    pub(crate) fn dashboard_available(&self) -> bool {
        self.config.dashboard_url.is_some()
    }

    pub(crate) fn open_dashboard(&mut self) {
        match self.config.dashboard_url.clone() {
            Some(url) => crate::agent::tray::xdg_open(&url),
            None => self.show_toast(Tone::Warning, DASHBOARD_UNSET),
        }
    }

    /// Run `dosi-tracker <kind>` in the background (one of each at a time).
    pub(crate) fn run_cli(&mut self, kind: JobKind) {
        if self.job_running(kind) {
            return;
        }
        let exe = std::env::current_exe()
            .map(|exe| exe.with_file_name("dosi-tracker"))
            .ok()
            .filter(|exe| exe.is_file())
            .unwrap_or_else(|| PathBuf::from("dosi-tracker"));
        let mut command = std::process::Command::new(exe);
        command.arg("--json");
        if let Some(socket) = &self.socket {
            command.arg("--socket").arg(socket);
        }
        command
            .args(kind.args())
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let slot = Arc::new(Mutex::new(None));
        let done = slot.clone();
        let ctx = self.link_ctx();
        let spawned = std::thread::Builder::new()
            .name("dosi-gui-job".into())
            .spawn(move || {
                let outcome = match command.output() {
                    Ok(output) => JobOutcome {
                        code: output.status.code(),
                        json: serde_json::from_slice(&output.stdout).unwrap_or(Value::Null),
                    },
                    Err(e) => JobOutcome {
                        code: None,
                        json: json!({ "ok": false, "error": { "message": e.to_string() } }),
                    },
                };
                *done.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
        match spawned {
            Ok(_) => self.jobs.push(Job { kind, slot }),
            Err(e) => self.show_toast(Tone::Danger, format!("Could not run dosi-tracker: {e}")),
        }
    }

    fn link_ctx(&self) -> Option<egui::Context> {
        self.ctx.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn job_running(&self, kind: JobKind) -> bool {
        self.jobs.iter().any(|job| job.kind == kind)
    }

    pub(crate) fn jobs_running(&self) -> bool {
        self.jobs
            .iter()
            .any(|job| job.kind != JobKind::Status && job.kind != JobKind::Doctor)
    }

    fn finish_jobs(&mut self) {
        let mut done = Vec::new();
        self.jobs.retain(
            |job| match job.slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(outcome) => {
                    done.push((job.kind, outcome));
                    false
                }
                None => true,
            },
        );
        for (kind, outcome) in done {
            let ok = outcome.code == Some(0);
            let message = outcome
                .json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("the command failed")
                .to_owned();
            match kind {
                JobKind::Status => self.service = outcome.json.get("result").cloned(),
                JobKind::Doctor => {
                    self.doctor = Some(if outcome.json.is_object() {
                        settings::doctor_problems(&outcome.json)
                    } else {
                        vec![(Tone::Danger, "The diagnostics could not run.".to_owned())]
                    })
                }
                JobKind::Enable | JobKind::Disable | JobKind::StartAtLogin => {
                    self.autostart_enabled =
                        crate::service::autostart_enabled(&self.paths.autostart_entry());
                    self.service = None;
                    if !ok {
                        self.settings_error =
                            Some(format!("Start at login was not changed: {message}"));
                    } else {
                        self.settings_error = None;
                    }
                }
                JobKind::Start | JobKind::Restart | JobKind::Stop => {
                    self.service = None;
                    if !ok {
                        self.show_toast(Tone::Danger, message);
                    }
                }
            }
        }
    }

    // ---- Messages -----------------------------------------------------------------------

    /// A message at the top of the window; the same one again only restarts
    /// its countdown.
    pub(crate) fn show_toast(&mut self, tone: Tone, text: impl Into<String>) {
        let text = text.into();
        self.toasts.retain(|toast| toast.text != text);
        self.toasts.push(Toast {
            tone,
            text,
            shown_at: None,
        });
        let excess = self.toasts.len().saturating_sub(MAX_TOASTS);
        self.toasts.drain(..excess);
    }

    /// The pending messages, each with a close button.
    pub(crate) fn toasts(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        self.toasts.retain(|toast| {
            toast
                .shown_at
                .is_none_or(|at| now.duration_since(at) < TOAST_FOR)
        });
        let mut closed = None;
        for (i, toast) in self.toasts.iter_mut().enumerate() {
            toast.shown_at.get_or_insert(now);
            if widgets::toast(ui, toast.tone, &toast.text) {
                closed = Some(i);
            }
        }
        if let Some(i) = closed {
            self.toasts.remove(i);
        }
    }

    /// Each new notice once, as a message in the window (the daemon posts
    /// the desktop notification).
    fn deliver_notices(&mut self, snapshot: &SharedState) {
        let newest = snapshot.notices.last().map_or(0, |n| n.id);
        let Some(seen) = self.notices_seen else {
            self.notices_seen = Some(newest);
            return;
        };
        let fresh: Vec<_> = snapshot
            .notices
            .iter()
            .filter(|n| n.id > seen)
            .cloned()
            .collect();
        for notice in fresh {
            // An expired sign-in replaces the window with its own form.
            if notice.kind != NoticeKind::SignInExpired {
                self.show_toast(notice_tone(&notice.kind), notice.text());
            }
        }
        // A restarted daemon numbers from 1 again.
        self.notices_seen = Some(newest);
    }

    /// The daemon's answers.
    fn on_reply(&mut self, ctx: &egui::Context, reply: Replied) {
        let Replied { method: m, outcome } = reply;
        match (m, outcome) {
            (method::SCREENSHOT_LATEST, outcome) => {
                self.on_screenshot(ctx, outcome.ok().map(|r| r.result))
            }
            (method::DAEMON_INFO, Ok(result)) => self.daemon_info = Some(result.result),
            (method::CONFIG_SET, Ok(_)) => self.reload_config(),
            (method::AUTH_SIGN_IN, Err(error)) => {
                self.link.update_local(|s| s.signing_in = false);
                self.login.reply_error = Some(error);
            }
            (method::AUTH_SIGN_IN, Ok(_)) => self.login.reply_error = None,
            (method::TRACKING_PAUSE | method::TRACKING_RESUME, Err(error)) => {
                self.link.update_local(|s| s.pending_action = None);
                self.error_toast(&error);
            }
            (method::TRACKING_PAUSE | method::TRACKING_RESUME, Ok(_)) => {
                self.link.update_local(|s| s.pending_action = None);
            }
            (method::QUEUE_PURGE, Ok(_)) => {
                self.show_toast(Tone::Success, "Waiting blocks deleted.")
            }
            (_, Err(error)) => self.error_toast(&error),
            (_, Ok(_)) => {}
        }
    }

    fn error_toast(&mut self, error: &ErrorBody) {
        let text = match &error.hint {
            Some(hint) => format!("{} ({hint})", error.message),
            None => error.message.clone(),
        };
        self.show_toast(Tone::Warning, text);
    }

    /// "Asia/Dhaka (UTC+06:00)": the device's zone, read again now and then.
    pub(crate) fn zone_label(&mut self) -> String {
        let fresh = self
            .zone
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < ZONE_REFRESH);
        if !fresh {
            let offset =
                format::utc_offset_label(crate::tracking::system::utc_offset_minutes(Utc::now()));
            let label = match crate::tracking::system::time_zone() {
                Some(zone) => format!("{zone} ({offset})"),
                None => offset,
            };
            self.zone = Some((label, Instant::now()));
        }
        self.zone
            .as_ref()
            .map(|(label, _)| label.clone())
            .unwrap_or_default()
    }

    /// The newest consent receipts, read again every few seconds.
    pub(crate) fn consent_history(&mut self) -> Vec<settings::Receipt> {
        let fresh = self
            .history
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(5));
        if !fresh {
            let text = std::fs::read_to_string(self.paths.consent_log_file()).unwrap_or_default();
            self.history = Some((
                Instant::now(),
                settings::parse_receipts(&text, settings::HISTORY_SHOWN),
            ));
        }
        self.history
            .as_ref()
            .map(|(_, receipts)| receipts.clone())
            .unwrap_or_default()
    }

    // ---- Pages --------------------------------------------------------------------------

    /// No daemon answers: say so, and offer to start it.
    fn not_running_page(&mut self, ui: &mut egui::Ui, status: &LinkStatus) {
        let (title, body, can_start) = match status {
            LinkStatus::Connecting => (
                "Connecting to Dosi Tracker…",
                "The window is looking for the agent on this computer.".to_owned(),
                false,
            ),
            LinkStatus::Mismatch { message } => (
                "Dosi Tracker needs a restart",
                format!(
                    "{message} The window is read-only until the agent runs the same version."
                ),
                false,
            ),
            _ => (
                "Dosi Tracker isn't running",
                "The agent that records your time is not running on this computer, so nothing is tracked right now."
                    .to_owned(),
                true,
            ),
        };
        let page = match status {
            LinkStatus::Connecting => "connecting",
            LinkStatus::Mismatch { .. } => "mismatch",
            _ => "not_running",
        };
        self.probe_page(page);
        let busy = self.jobs_running();
        centered_column(ui, 380.0, |ui| {
            ui.vertical_centered(|ui| {
                theme::logo_mark(ui, 52.0);
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(title)
                        .font(theme::semibold(21.0))
                        .color(theme::INK),
                );
            });
            self.toasts(ui);
            let tone = if matches!(status, LinkStatus::Mismatch { .. }) {
                Tone::Warning
            } else {
                Tone::Info
            };
            widgets::banner(ui, tone, &body);
            if matches!(status, LinkStatus::Mismatch { .. })
                && widgets::button(
                    ui,
                    ButtonKind::Primary,
                    Some(Icon::Sync),
                    "Restart Dosi Tracker",
                    Some(ui.available_width()),
                    !busy,
                )
                .clicked()
            {
                self.run_cli(JobKind::Restart);
            }
            if can_start {
                if widgets::button(
                    ui,
                    ButtonKind::Primary,
                    Some(Icon::Play),
                    if busy { "Starting…" } else { "Start" },
                    Some(ui.available_width()),
                    !busy,
                )
                .clicked()
                {
                    self.run_cli(JobKind::Start);
                }
                if widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(Icon::Timer),
                    "Start when I log in",
                    Some(ui.available_width()),
                    !busy,
                )
                .on_hover_text("Start the agent now and at every login to this desktop.")
                .clicked()
                {
                    self.run_cli(JobKind::StartAtLogin);
                }
                widgets::muted(ui, "Or run dosi-tracker service start in a terminal.");
            }
        });
    }

    fn probe_page(&mut self, _page: &'static str) {
        #[cfg(feature = "test-support")]
        self.probe.page(_page);
    }

    fn signed_in_layout(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        self.probe_page("main");
        let full = ui.max_rect();
        let painter = ui.painter().clone();

        // Header bar.
        let header = Rect::from_min_size(full.min, Vec2::new(full.width(), HEADER_H));
        painter.rect_filled(header, 0, theme::CARD);
        painter.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );
        let mut header_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(header.shrink2(Vec2::new(theme::PAGE_PAD, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        self.header(&mut header_ui, snapshot);

        // Footer bar.
        let footer = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - FOOTER_H), full.max);
        painter.rect_filled(footer, 0, theme::CARD);
        painter.line_segment(
            [footer.left_top(), footer.right_top()],
            Stroke::new(1.0, theme::BORDER),
        );
        let mut footer_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(footer.shrink2(Vec2::new(theme::PAGE_PAD, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        self.footer(&mut footer_ui, snapshot);

        // Tabs, fixed under the header.
        let tabs = Rect::from_min_max(
            Pos2::new(full.left(), header.bottom()),
            Pos2::new(full.right(), header.bottom() + TABS_H),
        );
        let mut tabs_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(tabs)
                .layout(Layout::top_down(Align::Center)),
        );
        self.tabs(&mut tabs_ui);

        // Scrolling content.
        let body = Rect::from_min_max(
            Pos2::new(full.left(), tabs.bottom()),
            Pos2::new(full.right(), footer.top()),
        );
        let mut body_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(body)
                .layout(Layout::top_down(Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt(("content", self.view as u8))
            .auto_shrink([false, false])
            .show(&mut body_ui, |ui| {
                let available = ui.available_width();
                let max_width = if available - 2.0 * theme::PAGE_PAD >= WIDE_LAYOUT {
                    1040.0
                } else {
                    640.0
                };
                let width = (available - 2.0 * theme::PAGE_PAD).min(max_width).max(0.0);
                let side = ((available - width) / 2.0).max(0.0);
                ui.add_space(2.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(side - ui.spacing().item_spacing.x);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = theme::GAP;
                        self.toasts(ui);
                        self.away_prompt_card(ui, snapshot);
                        let wide = width >= WIDE_LAYOUT;
                        match self.view {
                            View::Timer => self.timer_view(ui, snapshot, now, wide),
                            View::Activity => self.activity_view(ui, snapshot, now, wide),
                            View::Settings => self.settings_view(ui, snapshot),
                        }
                        ui.add_space(theme::PAGE_PAD);
                    });
                });
            });
    }

    fn header(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
        theme::logo_mark(ui, 32.0);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(12.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                egui::RichText::new("Dosi Tracker")
                    .font(theme::semibold(15.0))
                    .color(theme::INK),
            );
            let account = match (snapshot.workspace.trim(), snapshot.display_name.trim()) {
                ("", "") => String::new(),
                ("", name) => name.to_owned(),
                (workspace, "") => workspace.to_owned(),
                (workspace, name) => format!("{workspace} · {name}"),
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(account)
                        .font(theme::regular(11.5))
                        .color(theme::MUTED_FG),
                )
                .truncate(),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (tone, label) = status_visual(snapshot.status, snapshot.pending_action);
            widgets::status_pill(ui, tone, label);
        });
    }

    /// The segmented control that switches views.
    fn tabs(&mut self, ui: &mut egui::Ui) {
        let tabs = [
            (View::Timer, Icon::Timer, "Timer"),
            (View::Activity, Icon::Chart, "Activity"),
            (View::Settings, Icon::Sliders, "Settings"),
        ];
        let width = (ui.available_width() - 2.0 * theme::PAGE_PAD).min(440.0);
        let height = 38.0;
        ui.add_space(10.0);
        let (outer, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(
            outer,
            egui::CornerRadius::same(theme::RADIUS),
            theme::mix(theme::BORDER, theme::BACKGROUND, 0.55),
        );
        let inner = outer.shrink(3.0);
        let slot = inner.width() / tabs.len() as f32;
        for (i, (view, icon, label)) in tabs.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(inner.left() + slot * i as f32, inner.top()),
                Vec2::new(slot, inner.height()),
            );
            let response = ui
                .interact(rect, ui.id().with(("tab", i)), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                self.view = view;
                self.confirm_sign_out = false;
                self.confirm_purge = false;
            }
            let selected = self.view == view;
            let painter = ui.painter();
            if selected {
                painter.rect(
                    rect,
                    egui::CornerRadius::same(theme::RADIUS - 2),
                    theme::CARD,
                    Stroke::new(1.0, theme::BORDER),
                    egui::StrokeKind::Inside,
                );
            } else if response.hovered() {
                painter.rect_filled(
                    rect,
                    egui::CornerRadius::same(theme::RADIUS - 2),
                    theme::mix(theme::CARD, theme::BORDER, 0.5),
                );
            }
            let color = if selected {
                theme::INK
            } else {
                theme::MUTED_FG
            };
            let font = if selected {
                theme::semibold(13.0)
            } else {
                theme::regular(13.0)
            };
            let galley = painter.layout_no_wrap(label.to_owned(), font, color);
            let content_w = 16.0 + 7.0 + galley.size().x;
            let x = rect.center().x - content_w / 2.0;
            let icon_color = if selected {
                theme::PRIMARY
            } else {
                theme::MUTED_FG
            };
            widgets::paint_icon(
                painter,
                Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(15.0)),
                icon,
                icon_color,
            );
            painter.galley(
                Pos2::new(x + 23.0, rect.center().y - galley.size().y / 2.0),
                galley,
                color,
            );
        }
    }

    fn footer(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let (tone, text) = sync_status(snapshot);
        let (dot, _, _) = tone.colors();
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, dot);
        let label = ui.label(
            egui::RichText::new(text)
                .font(theme::regular(11.5))
                .color(theme::MUTED_FG),
        );
        // Why uploads are held back, and when they are tried again.
        if let Some(issue) = &snapshot.upload_issue {
            label.on_hover_text(format!(
                "{} Next try at {}.",
                issue.message(),
                format::local_hm(issue.next_attempt_at())
            ));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                    .font(theme::regular(11.5))
                    .color(theme::FAINT),
            );
        });
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

/// Tone + label for the status pill; a pause/resume in flight shows first.
pub fn status_visual(
    status: TrackerStatus,
    pending: Option<PendingAction>,
) -> (Tone, &'static str) {
    match pending {
        Some(PendingAction::Pausing) => return (Tone::Warning, "Pausing…"),
        Some(PendingAction::Resuming) => return (Tone::Success, "Resuming…"),
        None => {}
    }
    match status {
        TrackerStatus::Tracking => (Tone::Success, status.label()),
        TrackerStatus::Paused => (Tone::Warning, status.label()),
        TrackerStatus::Offline => (Tone::Warning, "Offline"),
        TrackerStatus::Connecting => (Tone::Info, status.label()),
        TrackerStatus::SignedOut => (Tone::Neutral, status.label()),
        TrackerStatus::Stopped => (Tone::Danger, status.label()),
        TrackerStatus::Away | TrackerStatus::Locked => (Tone::Warning, status.label()),
        TrackerStatus::ChooseProject => (Tone::Info, status.label()),
        TrackerStatus::SignInExpired => (Tone::Danger, status.label()),
        TrackerStatus::ConsentRequired => (Tone::Warning, status.label()),
        TrackerStatus::NoSession => (Tone::Warning, "No desktop session"),
        TrackerStatus::DeviceRevoked => (Tone::Danger, "Device removed"),
    }
}

/// The footer's one-line upload state: the most important thing first.
pub fn sync_status(snapshot: &SharedState) -> (Tone, String) {
    let waiting = snapshot.pending_uploads;
    let offline = snapshot.status == TrackerStatus::Offline || !snapshot.server_reachable;
    if snapshot.rejected_uploads > 0 {
        (
            Tone::Danger,
            format!(
                "{} not accepted by the server",
                format::count(snapshot.rejected_uploads, "block")
            ),
        )
    } else if snapshot.credentials_locked {
        (
            Tone::Warning,
            format!(
                "Keyring locked — {} waiting to sync",
                format::count(waiting, "block")
            ),
        )
    } else if offline {
        if waiting > 0 {
            (
                Tone::Warning,
                format!(
                    "Offline — {} waiting to sync",
                    format::count(waiting, "block")
                ),
            )
        } else {
            (Tone::Warning, "Offline — uploads will retry".to_owned())
        }
    } else if let Some(UploadIssue::SubscriptionInactive { .. }) = snapshot.upload_issue {
        (
            Tone::Warning,
            format!(
                "Subscription inactive — {} kept here",
                format::count(waiting, "block")
            ),
        )
    } else if let Some(UploadIssue::ClockAhead { .. }) = snapshot.upload_issue {
        (
            Tone::Warning,
            format!(
                "Clock ahead — {} waiting to sync",
                format::count(waiting, "block")
            ),
        )
    } else if waiting > 0 {
        (
            Tone::Warning,
            format!("{} waiting to sync", format::count(waiting, "block")),
        )
    } else if let Some(synced) = snapshot.last_sync {
        (
            Tone::Success,
            format!("All synced · {}", format::local_hm(synced)),
        )
    } else {
        (Tone::Neutral, "Nothing to sync yet".to_owned())
    }
}

/// How a daemon notice looks in the window.
pub fn notice_tone(kind: &NoticeKind) -> Tone {
    match kind {
        NoticeKind::TrackingStartedAutomatically { .. } | NoticeKind::ChooseProject => Tone::Info,
        NoticeKind::SignInExpired | NoticeKind::DeviceRevoked { .. } => Tone::Danger,
        NoticeKind::ProjectUnavailable { .. }
        | NoticeKind::QueueReset { .. }
        | NoticeKind::QueueTrimmed { .. }
        | NoticeKind::WorkspaceLost { .. }
        | NoticeKind::PolicyRequestsConsent { .. }
        | NoticeKind::ManualModePaused { .. }
        | NoticeKind::BlocksNotAccepted { .. }
        | NoticeKind::ScreenshotsRemoved { .. } => Tone::Warning,
    }
}

impl eframe::App for TrackerApp {
    /// Clear to the app surface colour, so nothing egui does not draw shows
    /// through as black.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = theme::BACKGROUND;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }

    /// Runs every frame, shown or not: answers, show requests, jobs, quit.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        while let Some(reply) = self.link.try_reply() {
            self.on_reply(ctx, reply);
        }
        while let Some(view) = self.link.try_show() {
            if let Some(view) = view.as_deref() {
                self.open_view(view);
            }
            self.show_requested = true;
        }
        if std::mem::take(&mut self.show_requested) {
            self.show_window(ctx);
        }
        self.finish_jobs();

        let status = self.link.status();
        if let LinkStatus::Connected { hello } = &status {
            let daemon = (hello.pid, hello.started_at);
            if self.asked_info != Some(daemon) {
                self.asked_info = Some(daemon);
                self.link.call(method::DAEMON_INFO, json!({}));
            }
        }
        if status.is_live() {
            let snapshot = self.link.snapshot();
            self.deliver_notices(&snapshot);
            // Follow a consent changed elsewhere (the CLI, the tray, another
            // window) unless the user is editing a draft here.
            if self.consent_vm.current != snapshot.consent && !self.consent_vm.is_dirty() {
                self.consent_vm = consent::ConsentViewModel::new(snapshot.consent);
            }
            if snapshot.workspace_lost != self.lost_seen {
                self.lost_seen = snapshot.workspace_lost.clone();
                if self.lost_seen.is_some() {
                    self.view = View::Settings;
                }
            }
        }
        // Timers tick every second on screen.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let viewport = ui.max_rect();
        ui.painter()
            .rect_filled(viewport, egui::CornerRadius::ZERO, theme::BACKGROUND);
        if viewport.width() < MIN_LAYOUT_SIZE || viewport.height() < MIN_LAYOUT_SIZE {
            return;
        }
        let status = self.link.status();
        if !status.is_live() {
            self.not_running_page(ui, &status);
        } else {
            let snapshot = self.link.snapshot();
            let now = Utc::now();
            // An expired sign-in asks for the password on the same screen.
            if matches!(
                snapshot.status,
                TrackerStatus::SignedOut | TrackerStatus::SignInExpired
            ) {
                self.probe_page("login");
                self.confirm_sign_out = false;
                self.view = View::Timer;
                self.login_view(ui, &snapshot);
            } else {
                self.login.confirm_switch = false;
                self.signed_in_layout(ui, &snapshot, now);
            }
        }
        #[cfg(feature = "test-support")]
        self.probe.flush(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.login.wipe_password();
        self.link.stop();
    }
}

/// A centred column at most `max` wide, a little above the middle.
fn centered_column(ui: &mut egui::Ui, max: f32, body: impl FnOnce(&mut egui::Ui)) {
    let full = ui.max_rect();
    let available = ui.available_width();
    let width = (available - 2.0 * theme::PAGE_PAD).clamp(0.0, max);
    let side = ((available - width) / 2.0).max(0.0);
    ui.add_space(((full.height() - 480.0) / 3.0).max(theme::PAGE_PAD + 8.0));
    ui.horizontal_top(|ui| {
        ui.add_space(side - ui.spacing().item_spacing.x);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = theme::GAP;
            body(ui);
        });
    });
}

/// A card with a section header; the body fills the card's width.
pub(crate) fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    trailing: Option<&str>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    theme::card(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            widgets::section_header(ui, title, trailing);
            body(ui)
        })
        .inner
}

/// Two equal columns; `column` receives the column index and its Ui.
pub(crate) fn two_columns(ui: &mut egui::Ui, mut column: impl FnMut(usize, &mut egui::Ui)) {
    ui.columns(2, |columns| {
        for (i, col) in columns.iter_mut().enumerate() {
            col.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = theme::GAP;
                column(i, ui);
            });
        }
    });
}

/// A row of `n` equal cells across the available width.
pub(crate) fn equal_row(
    ui: &mut egui::Ui,
    n: usize,
    gap: f32,
    mut cell: impl FnMut(usize, &mut egui::Ui),
) {
    let width = ui.available_width();
    let cell_w = ((width - gap * (n as f32 - 1.0)) / n as f32).max(0.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for i in 0..n {
            ui.allocate_ui_with_layout(
                Vec2::new(cell_w, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(cell_w);
                    cell(i, ui);
                },
            );
        }
    });
}

/// Paint a "label: value" metric pair (small caps label, strong value).
pub(crate) fn metric(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(
            egui::RichText::new(label.to_uppercase())
                .font(theme::semibold(10.0))
                .color(theme::MUTED_FG),
        );
        ui.label(
            egui::RichText::new(value)
                .font(theme::semibold(15.0))
                .color(color),
        );
    });
}

/// Test builds: where the window's controls are, for the Xvfb smoke test
/// (`DOSI_TEST_GUI_PROBE=<file>`). Written only when it changes.
#[cfg(feature = "test-support")]
mod probe {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use eframe::egui;

    #[derive(Default)]
    pub struct Probe {
        file: Option<PathBuf>,
        page: &'static str,
        controls: BTreeMap<&'static str, [f32; 4]>,
        written: String,
    }

    impl Probe {
        pub fn from_env() -> Self {
            Self {
                file: std::env::var_os("DOSI_TEST_GUI_PROBE").map(PathBuf::from),
                ..Self::default()
            }
        }

        pub fn page(&mut self, page: &'static str) {
            if self.page != page {
                self.controls.clear();
            }
            self.page = page;
        }

        pub fn control(&mut self, name: &'static str, rect: egui::Rect) {
            self.controls
                .insert(name, [rect.min.x, rect.min.y, rect.width(), rect.height()]);
        }

        /// Write `{page, pixels_per_point, controls}` when it changed.
        pub fn flush(&mut self, ctx: &egui::Context) {
            let Some(file) = &self.file else {
                return;
            };
            let text = serde_json::json!({
                "page": self.page,
                "pixels_per_point": ctx.pixels_per_point(),
                "controls": self.controls,
            })
            .to_string();
            if text != self.written {
                let staged = file.with_extension("tmp");
                if std::fs::write(&staged, &text).is_ok() && std::fs::rename(&staged, file).is_ok()
                {
                    self.written = text;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::Notice;

    #[test]
    fn footer_reports_the_most_important_upload_state() {
        let mut s = SharedState {
            status: TrackerStatus::Tracking,
            ..SharedState::default()
        };
        assert_eq!(
            sync_status(&s),
            (Tone::Neutral, "Nothing to sync yet".to_owned())
        );
        s.pending_uploads = 2;
        assert_eq!(
            sync_status(&s),
            (Tone::Warning, "2 blocks waiting to sync".to_owned())
        );
        s.server_reachable = false;
        assert_eq!(sync_status(&s).1, "Offline — 2 blocks waiting to sync");
        s.status = TrackerStatus::Offline;
        s.pending_uploads = 0;
        assert_eq!(sync_status(&s).1, "Offline — uploads will retry");
        s.credentials_locked = true;
        assert_eq!(
            sync_status(&s).1,
            "Keyring locked — 0 blocks waiting to sync"
        );
        s.rejected_uploads = 1;
        assert_eq!(
            sync_status(&s),
            (
                Tone::Danger,
                "1 block not accepted by the server".to_owned()
            )
        );
    }

    #[test]
    fn held_back_uploads_say_why_in_the_footer() {
        let at = Utc::now();
        let mut s = SharedState {
            status: TrackerStatus::Tracking,
            pending_uploads: 3,
            ..SharedState::default()
        };
        s.upload_issue = Some(UploadIssue::SubscriptionInactive {
            next_attempt_at: at,
            message: String::new(),
        });
        assert_eq!(
            sync_status(&s).1,
            "Subscription inactive — 3 blocks kept here"
        );
        s.upload_issue = Some(UploadIssue::ClockAhead {
            next_attempt_at: at,
        });
        assert_eq!(sync_status(&s).1, "Clock ahead — 3 blocks waiting to sync");
        s.upload_issue = None;
        s.pending_uploads = 0;
        s.last_sync = Some(at);
        assert_eq!(sync_status(&s).0, Tone::Success);
    }

    #[test]
    fn a_request_in_flight_shows_before_the_status() {
        assert_eq!(
            status_visual(TrackerStatus::Tracking, Some(PendingAction::Pausing)),
            (Tone::Warning, "Pausing…")
        );
        assert_eq!(
            status_visual(TrackerStatus::Paused, None),
            (Tone::Warning, "Paused")
        );
        assert_eq!(status_visual(TrackerStatus::Stopped, None).0, Tone::Danger);
        assert_eq!(
            status_visual(TrackerStatus::DeviceRevoked, None).0,
            Tone::Danger
        );
        assert_eq!(
            status_visual(TrackerStatus::SignInExpired, None),
            (Tone::Danger, "Sign-in expired")
        );
    }

    #[test]
    fn notices_read_with_their_weight() {
        assert_eq!(notice_tone(&NoticeKind::SignInExpired), Tone::Danger);
        assert_eq!(
            notice_tone(&NoticeKind::QueueReset {
                moved_to: "x".into()
            }),
            Tone::Warning
        );
        let started = NoticeKind::TrackingStartedAutomatically {
            project_title: "Website".into(),
        };
        assert_eq!(notice_tone(&started), Tone::Info);
        let notice = Notice {
            id: 1,
            at: Utc::now(),
            kind: started,
        };
        assert!(notice.text().len() < 256);
    }

    #[test]
    fn the_backend_follows_flag_then_env_then_config() {
        use GuiBackend::*;
        assert_eq!(
            choose_backend(None, None, Auto, true, true),
            Ok(Backend::X11)
        );
        assert_eq!(
            choose_backend(None, None, Auto, false, true),
            Ok(Backend::Wayland)
        );
        assert!(choose_backend(None, None, Auto, false, false).is_err());
        assert_eq!(
            choose_backend(None, None, Wayland, true, true),
            Ok(Backend::Wayland)
        );
        assert_eq!(
            choose_backend(None, Some("x11"), Wayland, true, true),
            Ok(Backend::X11)
        );
        assert_eq!(
            choose_backend(Some(Wayland), Some("x11"), X11, true, true),
            Ok(Backend::Wayland)
        );
        assert!(choose_backend(None, Some("mir"), Auto, true, true).is_err());
        assert_eq!(
            choose_backend(None, Some(" "), X11, true, false),
            Ok(Backend::X11)
        );
        assert!(
            choose_backend(Some(X11), None, Auto, false, true).is_err(),
            "no DISPLAY"
        );
        assert!(choose_backend(Some(Wayland), None, Auto, true, false).is_err());
    }

    #[test]
    fn arguments_are_parsed_strictly() {
        let args =
            |list: &[&str]| parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(args(&["--version"]).unwrap().version);
        let parsed = args(&[
            "--backend",
            "wayland",
            "--view=settings",
            "--socket",
            "/run/x.sock",
        ])
        .unwrap();
        assert_eq!(parsed.backend, Some(GuiBackend::Wayland));
        assert_eq!(parsed.view.as_deref(), Some("settings"));
        assert_eq!(parsed.socket, Some(PathBuf::from("/run/x.sock")));
        assert!(args(&["--view", "nowhere"]).is_err());
        assert!(args(&["--backend"]).is_err());
        assert!(args(&["--password", "x"]).is_err());
        #[cfg(not(feature = "test-support"))]
        assert!(
            args(&["--preview", "main"]).is_err(),
            "release builds have no previews"
        );
    }
}
