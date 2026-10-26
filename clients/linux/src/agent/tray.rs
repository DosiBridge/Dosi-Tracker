//! The daemon-owned StatusNotifierItem tray (ksni) and the ported Windows
//! `TrayView` (labels, toggle, totals, tooltip, project list).
//!
//! The tray shows the tracker's state at a glance (the badge colour of the
//! icon, a tooltip with the session and today's total) and puts the everyday
//! actions one click away: pause/resume, a timed break, switching project, the
//! task note, syncing, the web dashboard, settings, start at login and quit.
//! Activating the item opens the window (`dosi-tracker-gui`).
//!
//! What is pure lives here unconditionally (Linux): [`TrayView`], the badge
//! pixmaps painted in code, the action dispatch and the GUI launcher, so the
//! GUI and the tests use them without the `tray` feature. The ksni service
//! itself ([`spawn`]) needs the `tray` feature: it runs on `dosi-tray`, is
//! refreshed from the [`StateHandle`] change counter, sends every menu action
//! as a [`Command`] through the daemon's [`CommandSink`], and tracks the owner
//! of `org.kde.StatusNotifierWatcher` (through ksni's `NameOwnerChanged`
//! watch) into `snapshot.indicator.tray`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};

use super::daemon::{Clock, CommandSink};
use super::state::{Command, PendingAction, SharedState, StateHandle, TrackerStatus};
use super::worker::{local_day_and_week_start, tracked_secs_since};

/// The timed breaks offered in the tray (the window offers the same).
pub const BREAKS: [(&str, Duration); 3] = [
    ("15 minutes", Duration::from_secs(15 * 60)),
    ("30 minutes", Duration::from_secs(30 * 60)),
    ("1 hour", Duration::from_secs(60 * 60)),
];

/// The task note as the menu quotes it, at most this many characters.
const TASK_LABEL_CHARS: usize = 36;

/// The tooltip's description, at most this many characters.
const TOOLTIP_CHARS: usize = 255;

/// The pixmap sizes the item carries (hosts pick the closest one).
pub const ICON_SIZES: [u32; 5] = [16, 22, 24, 32, 48];

// ---- Formatting shared with the window ----------------------------------------------------

/// "3h 42m", or "42m" under an hour.
pub fn minutes_label(total: u64) -> String {
    let hours = total / 60;
    let minutes = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

/// Wall-clock `HH:MM` in the user's time zone.
pub fn local_hm(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

/// At most `max` characters, ending in "…" when cut (long names would stretch
/// menus, tooltips and pills).
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// Pausing applies while signed in and not already paused — including while
/// offline, so a flaky connection never stops someone from taking a break,
/// and while away from the computer.
pub fn can_pause(status: TrackerStatus) -> bool {
    matches!(
        status,
        TrackerStatus::Tracking | TrackerStatus::Offline | TrackerStatus::Away
    )
}

/// Today / this week including the block in progress, in seconds. The worker
/// publishes the saved totals; the time not saved yet is added here.
pub fn live_totals(snapshot: &SharedState, now: DateTime<Utc>) -> (f64, f64) {
    let (mut today, mut week) = (snapshot.tracked_today_secs, snapshot.tracked_week_secs);
    if let Some(block) = snapshot.unsaved_since.or(snapshot.block_started_at) {
        let (today_start, week_start) = local_day_and_week_start(now, &Local);
        today += tracked_secs_since(block, now, today_start);
        week += tracked_secs_since(block, now, week_start);
    }
    (today, week)
}

/// A lone `_` in a DBusMenu label marks the access key instead of showing.
fn escape_menu_text(text: &str) -> String {
    text.replace('_', "__")
}

// ---- The view ---------------------------------------------------------------------------

/// Which badge the icon wears (design 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Badge {
    /// Green.
    Tracking,
    /// Amber: paused, away or locked.
    Paused,
    /// Grey: offline, connecting, choosing a project, no graphical session.
    Offline,
    /// All grey.
    SignedOut,
    /// Red: stopped, sign-in expired, consent missing, device removed.
    Stopped,
}

/// Something the user did in the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    /// Pause when tracking, resume when paused (or open the window to sign
    /// in, choose a project or another workspace).
    ToggleTracking,
    Break(Duration),
    SelectProject(String),
    /// Open the window at the "What are you working on?" field.
    SetTask,
    SyncNow,
    OpenDashboard,
    OpenSettings,
    ToggleAutostart,
    Quit,
}

/// Everything the tray shows, derived from a snapshot. Comparing two views
/// tells whether anything needs to be published.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayView {
    pub badge: Badge,
    /// The away question or an expired sign-in wants the user.
    pub needs_attention: bool,
    pub tooltip: String,
    pub status_line: String,
    pub totals_line: String,
    pub toggle_label: &'static str,
    pub toggle_enabled: bool,
    pub breaks_enabled: bool,
    /// (id, escaped title).
    pub projects: Vec<(String, String)>,
    pub selected_project: Option<String>,
    pub projects_enabled: bool,
    pub task_label: String,
    pub task_enabled: bool,
    pub sync_label: &'static str,
    pub sync_enabled: bool,
    pub dashboard_enabled: bool,
    pub autostart: bool,
}

impl TrayView {
    pub fn new(
        snapshot: &SharedState,
        now: DateTime<Utc>,
        (today_secs, week_secs): (f64, f64),
        autostart: bool,
        dashboard: bool,
    ) -> Self {
        let idle = snapshot.pending_action.is_none();
        let signed_in = snapshot.status != TrackerStatus::SignedOut;
        // Signed in, with an account the server still accepts, a device it
        // still knows and a queue that works: what the everyday actions need.
        let usable = signed_in
            && !matches!(
                snapshot.status,
                TrackerStatus::SignInExpired
                    | TrackerStatus::Stopped
                    | TrackerStatus::ConsentRequired
                    | TrackerStatus::DeviceRevoked
            );
        let paused = snapshot.status == TrackerStatus::Paused;
        let can_pause = idle && can_pause(snapshot.status);
        let project = snapshot.current_project().map(|p| p.title.as_str());

        let badge = match snapshot.status {
            TrackerStatus::Tracking => Badge::Tracking,
            TrackerStatus::Paused | TrackerStatus::Away | TrackerStatus::Locked => Badge::Paused,
            TrackerStatus::Offline
            | TrackerStatus::Connecting
            | TrackerStatus::ChooseProject
            | TrackerStatus::NoSession => Badge::Offline,
            TrackerStatus::SignedOut => Badge::SignedOut,
            TrackerStatus::Stopped
            | TrackerStatus::SignInExpired
            | TrackerStatus::ConsentRequired
            | TrackerStatus::DeviceRevoked => Badge::Stopped,
        };

        let since = |at: Option<DateTime<Utc>>| {
            minutes_label(at.map_or(0, |at| (now - at).num_minutes().max(0) as u64))
        };
        let state = match (snapshot.pending_action, snapshot.status) {
            (Some(PendingAction::Pausing), _) => "Pausing…".to_owned(),
            (Some(PendingAction::Resuming), _) => "Resuming…".to_owned(),
            (None, TrackerStatus::Tracking) => {
                format!("Tracking · {}", since(snapshot.tracking_since))
            }
            (None, TrackerStatus::Paused) => match snapshot.resume_at {
                Some(at) => format!("Paused · resumes at {}", local_hm(at)),
                None => format!("Paused · {}", since(snapshot.paused_since)),
            },
            (None, TrackerStatus::Away) => match snapshot.away_since {
                Some(at) => format!("Away since {}", local_hm(at)),
                None => TrackerStatus::Away.label().to_owned(),
            },
            (None, status) => status.label().to_owned(),
        };
        let today = minutes_label((today_secs / 60.0) as u64);
        let week = minutes_label((week_secs / 60.0) as u64);

        let mut tooltip = state.clone();
        if signed_in {
            match project {
                Some(title) => tooltip.push_str(&format!("\n{title} · Today {today}")),
                None => tooltip.push_str(&format!("\nToday {today}")),
            }
        }

        let status_line = match project.filter(|_| signed_in) {
            Some(title) => format!("● {state} — {title}"),
            None => format!("● {state}"),
        };
        let totals_line = if signed_in {
            format!("Today {today} · This week {week}")
        } else {
            "Sign in to start tracking".to_owned()
        };

        let (toggle_label, toggle_enabled) = match (snapshot.pending_action, snapshot.status) {
            (Some(PendingAction::Pausing), _) => ("Pausing…", false),
            (Some(PendingAction::Resuming), _) => ("Resuming…", false),
            (None, TrackerStatus::SignedOut | TrackerStatus::SignInExpired) => ("Sign in…", true),
            (None, TrackerStatus::ChooseProject) => ("Choose a project…", true),
            (None, TrackerStatus::DeviceRevoked) => ("Choose a workspace…", true),
            (None, TrackerStatus::Paused) => ("Resume tracking", true),
            (None, _) => ("Pause tracking", can_pause),
        };

        let note = snapshot.task_note.trim();
        let task_label = if note.is_empty() {
            "Set task…".to_owned()
        } else {
            format!("Task: {}", truncate_chars(note, TASK_LABEL_CHARS))
        };

        Self {
            badge,
            needs_attention: snapshot.away_prompt.is_some()
                || snapshot.status == TrackerStatus::SignInExpired,
            tooltip: truncate_chars(&tooltip, TOOLTIP_CHARS),
            status_line: escape_menu_text(&status_line),
            totals_line: escape_menu_text(&totals_line),
            toggle_label,
            toggle_enabled,
            breaks_enabled: can_pause || (idle && paused),
            projects: snapshot
                .projects
                .iter()
                .map(|p| (p.id.clone(), escape_menu_text(&p.title)))
                .collect(),
            selected_project: snapshot.selected_project.clone(),
            projects_enabled: usable && idle && !snapshot.projects.is_empty(),
            task_label: escape_menu_text(&task_label),
            task_enabled: usable,
            sync_label: if paused { "Upload queued" } else { "Sync now" },
            sync_enabled: usable,
            dashboard_enabled: dashboard,
            autostart,
        }
    }
}

// ---- Actions ------------------------------------------------------------------------------

/// Which page a launched window opens on (`dosi-tracker-gui --view`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiView {
    Timer,
    /// The timer with the task field focused.
    Task,
    Settings,
}

impl GuiView {
    pub fn as_str(self) -> &'static str {
        match self {
            GuiView::Timer => "timer",
            GuiView::Task => "task",
            GuiView::Settings => "settings",
        }
    }
}

/// Opens the window (at a view), a URL, or flips start-at-login.
pub type OpenGui = Arc<dyn Fn(Option<GuiView>) + Send + Sync>;
pub type OpenUrl = Arc<dyn Fn(&str) + Send + Sync>;
/// Set start-at-login; returns whether it is on afterwards.
pub type SetAutostart = Arc<dyn Fn(bool) -> bool + Send + Sync>;
pub type AutostartEnabled = Arc<dyn Fn() -> bool + Send + Sync>;

/// What the tray's actions reach: the worker (numbered commands), the window
/// launcher, the browser and the autostart entry.
#[derive(Clone)]
pub struct TrayActions {
    pub sink: CommandSink,
    pub clock: Clock,
    pub open_gui: OpenGui,
    pub open_url: OpenUrl,
    pub set_autostart: SetAutostart,
    pub autostart_enabled: AutostartEnabled,
    /// The web dashboard (`dashboard_url`); `None` greys the item out.
    pub dashboard_url: Option<String>,
}

impl TrayActions {
    /// Carry out `action` for the state in `snapshot`.
    pub fn perform(&self, snapshot: &SharedState, action: TrayAction) {
        let at = (self.clock)();
        match action {
            TrayAction::Open => (self.open_gui)(None),
            TrayAction::ToggleTracking => match snapshot.status {
                TrackerStatus::SignedOut
                | TrackerStatus::SignInExpired
                | TrackerStatus::ChooseProject => (self.open_gui)(Some(GuiView::Timer)),
                TrackerStatus::DeviceRevoked => (self.open_gui)(Some(GuiView::Settings)),
                TrackerStatus::Paused if snapshot.pending_action.is_none() => {
                    self.sink.send(Command::Resume { at });
                }
                status if snapshot.pending_action.is_none() && can_pause(status) => {
                    self.sink.send(Command::Pause {
                        resume_after: None,
                        at,
                    });
                }
                _ => {}
            },
            TrayAction::Break(duration) => {
                let paused = snapshot.status == TrackerStatus::Paused;
                if snapshot.pending_action.is_none() && (paused || can_pause(snapshot.status)) {
                    self.sink.send(Command::Pause {
                        resume_after: Some(duration),
                        at,
                    });
                }
            }
            TrayAction::SelectProject(id) => {
                if snapshot.selected_project.as_deref() != Some(id.as_str()) {
                    self.sink
                        .send(Command::SelectProject { project_id: id, at });
                }
            }
            TrayAction::SetTask => (self.open_gui)(Some(GuiView::Task)),
            TrayAction::SyncNow => {
                self.sink.send(Command::SyncNow { at });
            }
            TrayAction::OpenDashboard => {
                if let Some(url) = &self.dashboard_url {
                    (self.open_url)(url);
                }
            }
            TrayAction::OpenSettings => (self.open_gui)(Some(GuiView::Settings)),
            TrayAction::ToggleAutostart => {
                let wanted = !(self.autostart_enabled)();
                (self.set_autostart)(wanted);
            }
            TrayAction::Quit => {
                self.sink.send(Command::Shutdown { at });
            }
        }
    }

    /// The view for `snapshot` now.
    pub fn view(&self, snapshot: &SharedState) -> TrayView {
        let now = Utc::now();
        TrayView::new(
            snapshot,
            now,
            live_totals(snapshot, now),
            (self.autostart_enabled)(),
            self.dashboard_url.is_some(),
        )
    }
}

// ---- Launching the window -----------------------------------------------------------------

/// Starts `dosi-tracker-gui`: in its own transient systemd scope when the user
/// manager runs (`systemd-run --user --scope --collect`, so a daemon restart
/// never takes the window with it), else in a new session (`setsid`). The
/// child is reaped by a short `dosi-gui` thread that waits for it.
#[derive(Debug, Clone)]
pub struct GuiLauncher {
    pub gui: PathBuf,
    /// `systemd-run` and a user manager are there.
    pub systemd_run: Option<PathBuf>,
}

/// `name` on `path` (a `PATH` value), if it is an executable file.
pub fn find_in_path(path: Option<&std::ffi::OsStr>, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

impl GuiLauncher {
    /// The launcher for `gui`, using `systemd-run` from `path` when the user
    /// manager's private socket exists in `runtime_base` (`XDG_RUNTIME_DIR`).
    pub fn new(gui: PathBuf, path: Option<&std::ffi::OsStr>, runtime_base: Option<&Path>) -> Self {
        let manager = runtime_base.is_some_and(|dir| dir.join("systemd/private").exists());
        Self {
            gui,
            systemd_run: manager.then(|| find_in_path(path, "systemd-run")).flatten(),
        }
    }

    /// The program and arguments that open the window at `view`.
    pub fn command_line(&self, view: Option<GuiView>) -> (PathBuf, Vec<String>) {
        let mut gui_args = Vec::new();
        if let Some(view) = view {
            gui_args.push("--view".to_owned());
            gui_args.push(view.as_str().to_owned());
        }
        match &self.systemd_run {
            Some(systemd_run) => {
                let mut args: Vec<String> = [
                    "--user",
                    "--scope",
                    "--collect",
                    "--quiet",
                    "--description=Dosi Tracker window",
                    "--",
                ]
                .iter()
                .map(|s| (*s).to_owned())
                .collect();
                args.push(self.gui.to_string_lossy().into_owned());
                args.extend(gui_args);
                (systemd_run.clone(), args)
            }
            None => (self.gui.clone(), gui_args),
        }
    }

    /// Open the window (a running one is brought to the front by the window
    /// itself). Never blocks the caller.
    pub fn launch(&self, view: Option<GuiView>) {
        use std::os::unix::process::CommandExt;
        let (program, args) = self.command_line(view);
        let mut command = std::process::Command::new(&program);
        command
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if self.systemd_run.is_none() {
            // SAFETY: setsid is async-signal-safe and only detaches the child
            // from the daemon's session.
            unsafe {
                command.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        match command.spawn() {
            Ok(mut child) => {
                let reaped = std::thread::Builder::new()
                    .name("dosi-gui".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                if let Err(e) = reaped {
                    tracing::debug!(error = %e, "the window's process is not reaped");
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not open the window"),
        }
    }
}

/// Open `url` with `xdg-open` (never blocks; the child is reaped).
pub fn xdg_open(url: &str) {
    match std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = std::thread::Builder::new()
                .name("dosi-open".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
        Err(e) => tracing::warn!(error = %e, "xdg-open could not run"),
    }
}

// ---- The icon -----------------------------------------------------------------------------

/// The brand mark with an optional status badge as straight-alpha RGBA,
/// `size` x `size` pixels, anti-aliased: a brand-blue disc with a white ring
/// and centre dot, the badge at the bottom right. Painted in code, so no icon
/// asset is needed at run time. Also the window icon (without a badge).
pub fn icon_rgba(badge: Option<Badge>, size: u32) -> Vec<u8> {
    let brand = match badge {
        Some(Badge::SignedOut) => [0x9c, 0xa3, 0xaf],
        _ => [0x00, 0x6b, 0xff],
    };
    let badge_color = badge.and_then(|b| match b {
        Badge::Tracking => Some([0x00, 0xa3, 0x89]),
        Badge::Paused => Some([0xf0, 0xa8, 0x00]),
        Badge::Offline => Some([0x6b, 0x72, 0x80]),
        Badge::Stopped => Some([0xd9, 0x36, 0x3e]),
        Badge::SignedOut => None,
    });
    // The 32 px design, scaled.
    let k = size as f32 / 32.0;
    // Coverage of a disc of radius r at distance d (1 px soft edge).
    let cover = |d: f32, r: f32| (r - d + 0.5).clamp(0.0, 1.0);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    let (bx, by, br) = (24.5 * k, 24.5 * k, 7.0 * k);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            let d = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
            // Layers, back to front: disc, ring, centre dot, badge.
            let mut px = [0.0_f32; 4];
            let mut over = |color: [u8; 3], alpha: f32| {
                let a = alpha.clamp(0.0, 1.0);
                for i in 0..3 {
                    px[i] = color[i] as f32 / 255.0 * a + px[i] * (1.0 - a);
                }
                px[3] = a + px[3] * (1.0 - a);
            };
            over(brand, cover(d, 15.5 * k));
            let ring = cover(d, 10.0 * k) - cover(d, 7.25 * k);
            over([0xff, 0xff, 0xff], ring);
            over([0xff, 0xff, 0xff], cover(d, 3.5 * k));
            if let Some(color) = badge_color {
                let bd = ((fx - bx).powi(2) + (fy - by).powi(2)).sqrt();
                // A white outline separates the badge from the disc.
                over([0xff, 0xff, 0xff], cover(bd, br));
                over(color, cover(bd, br - 2.0 * k));
            }
            // Un-premultiply for straight alpha.
            let a = px[3];
            let channel = |v: f32| {
                if a > 0.0 {
                    (v / a * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            rgba.extend_from_slice(&[
                channel(px[0]),
                channel(px[1]),
                channel(px[2]),
                (a * 255.0).round() as u8,
            ]);
        }
    }
    rgba
}

/// [`icon_rgba`] as the StatusNotifierItem wants it: ARGB32 in network byte
/// order (A, R, G, B per pixel).
pub fn icon_argb(badge: Option<Badge>, size: u32) -> Vec<u8> {
    let mut data = icon_rgba(badge, size);
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }
    data
}

// ---- The StatusNotifierItem ---------------------------------------------------------------

/// What [`spawn`] needs.
#[derive(Clone)]
pub struct TrayConfig {
    pub state: StateHandle,
    pub actions: TrayActions,
}

/// How often the view is recomputed without a state change (the tooltip's
/// minutes move on).
pub const REFRESH_EVERY: Duration = Duration::from_secs(30);

#[cfg(feature = "tray")]
mod service {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use ksni::blocking::TrayMethods as _;
    use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};

    use super::{
        icon_argb, TrayAction, TrayActions, TrayConfig, TrayView, BREAKS, ICON_SIZES, REFRESH_EVERY,
    };

    /// How often the refresh thread looks at the change counter.
    const POLL: std::time::Duration = std::time::Duration::from_millis(250);
    use crate::agent::state::StateHandle;
    use crate::paths::APP_ID;

    /// The item ksni serves.
    struct DosiTray {
        view: TrayView,
        actions: TrayActions,
        state: StateHandle,
        shown: Arc<AtomicBool>,
    }

    impl DosiTray {
        fn act(&mut self, action: TrayAction) {
            if action == TrayAction::ToggleAutostart {
                // Reflect the real entry, whether or not the write worked.
                let wanted = !(self.actions.autostart_enabled)();
                self.view.autostart = (self.actions.set_autostart)(wanted);
                return;
            }
            let snapshot = self.state.snapshot();
            self.actions.perform(&snapshot, action);
        }

        /// The watcher (and so a host) came or went.
        fn set_shown(&self, shown: bool) {
            if self.shown.swap(shown, Ordering::SeqCst) != shown {
                tracing::info!(shown, "tray host changed");
                self.state.update(|s| s.indicator.tray = shown);
            }
        }
    }

    fn item(label: &str, enabled: bool, action: TrayAction) -> MenuItem<DosiTray> {
        StandardItem {
            label: label.to_owned(),
            enabled,
            activate: Box::new(move |tray: &mut DosiTray| tray.act(action.clone())),
            ..Default::default()
        }
        .into()
    }

    fn info(label: &str) -> MenuItem<DosiTray> {
        StandardItem {
            label: label.to_owned(),
            enabled: false,
            ..Default::default()
        }
        .into()
    }

    impl ksni::Tray for DosiTray {
        fn id(&self) -> String {
            APP_ID.to_owned()
        }

        fn title(&self) -> String {
            "Dosi Tracker".to_owned()
        }

        fn status(&self) -> ksni::Status {
            if self.view.needs_attention {
                ksni::Status::NeedsAttention
            } else {
                ksni::Status::Active
            }
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            ICON_SIZES
                .iter()
                .map(|&size| ksni::Icon {
                    width: size as i32,
                    height: size as i32,
                    data: icon_argb(Some(self.view.badge), size),
                })
                .collect()
        }

        fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icon_pixmap()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "Dosi Tracker".to_owned(),
                description: self.view.tooltip.clone(),
                ..Default::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.act(TrayAction::Open);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let v = &self.view;
            let selected = v
                .selected_project
                .as_deref()
                .and_then(|id| v.projects.iter().position(|(p, _)| p == id))
                // Out of range: no item is checked.
                .unwrap_or(v.projects.len());
            let ids: Vec<String> = v.projects.iter().map(|(id, _)| id.clone()).collect();
            vec![
                info(&v.status_line),
                info(&v.totals_line),
                MenuItem::Separator,
                item("Open Dosi Tracker", true, TrayAction::Open),
                MenuItem::Separator,
                item(v.toggle_label, v.toggle_enabled, TrayAction::ToggleTracking),
                SubMenu {
                    label: "Take a break".to_owned(),
                    enabled: v.breaks_enabled,
                    submenu: BREAKS
                        .iter()
                        .map(|(label, duration)| item(label, true, TrayAction::Break(*duration)))
                        .collect(),
                    ..Default::default()
                }
                .into(),
                SubMenu {
                    label: "Switch project".to_owned(),
                    enabled: v.projects_enabled,
                    submenu: vec![RadioGroup {
                        selected,
                        select: Box::new(move |tray: &mut DosiTray, index| {
                            if let Some(id) = ids.get(index) {
                                tray.act(TrayAction::SelectProject(id.clone()));
                            }
                        }),
                        options: v
                            .projects
                            .iter()
                            .map(|(_, title)| RadioItem {
                                label: title.clone(),
                                ..Default::default()
                            })
                            .collect(),
                    }
                    .into()],
                    ..Default::default()
                }
                .into(),
                item(&v.task_label, v.task_enabled, TrayAction::SetTask),
                item(v.sync_label, v.sync_enabled, TrayAction::SyncNow),
                MenuItem::Separator,
                item(
                    "Open web dashboard",
                    v.dashboard_enabled,
                    TrayAction::OpenDashboard,
                ),
                item("Settings…", true, TrayAction::OpenSettings),
                CheckmarkItem {
                    label: "Start at login".to_owned(),
                    checked: v.autostart,
                    activate: Box::new(|tray: &mut DosiTray| tray.act(TrayAction::ToggleAutostart)),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                item("Quit Dosi Tracker", true, TrayAction::Quit),
            ]
        }

        fn watcher_online(&self) {
            self.set_shown(true);
        }

        fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
            tracing::info!(?reason, "no tray host");
            self.set_shown(false);
            // Keep serving: a host may appear later (an extension enabled, a
            // panel restarted), and ksni registers again then.
            true
        }
    }

    /// The running tray.
    pub struct TrayHandle {
        handle: ksni::blocking::Handle<DosiTray>,
        shown: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl TrayHandle {
        /// A host shows the item right now.
        pub fn shown(&self) -> bool {
            self.shown.load(Ordering::SeqCst)
        }

        /// Unregister and stop refreshing.
        pub fn stop(mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            self.handle.shutdown().wait();
        }
    }

    /// Register the item on the session bus (`DBUS_SESSION_BUS_ADDRESS`) and
    /// start `dosi-tray`. A missing watcher is not an error: the item waits
    /// for one, and `indicator.tray` says whether a host shows it.
    pub fn spawn(config: TrayConfig) -> Result<TrayHandle, String> {
        let TrayConfig { state, actions } = config;
        let shown = Arc::new(AtomicBool::new(true));
        let first = actions.view(&state.snapshot());
        let tray = DosiTray {
            view: first.clone(),
            actions: actions.clone(),
            state: state.clone(),
            shown: shown.clone(),
        };
        let handle = tray
            .assume_sni_available(true)
            .spawn()
            .map_err(|e| e.to_string())?;
        let shown_now = shown.load(Ordering::SeqCst);
        state.update(|s| s.indicator.tray = shown_now);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let handle = handle.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("dosi-tray".into())
                .spawn(move || {
                    let mut seen = state.version();
                    let mut current = first;
                    let mut refreshed = Instant::now();
                    while !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(POLL);
                        let version = state.version();
                        if version == seen && refreshed.elapsed() < REFRESH_EVERY {
                            continue;
                        }
                        seen = version;
                        refreshed = Instant::now();
                        let view = actions.view(&state.snapshot());
                        if view != current {
                            current = view.clone();
                            if handle.update(move |tray| tray.view = view).is_none() {
                                break;
                            }
                        }
                    }
                })
                .map_err(|e| e.to_string())?
        };
        Ok(TrayHandle {
            handle,
            shown,
            stop,
            thread: Some(thread),
        })
    }
}

#[cfg(feature = "tray")]
pub use service::{spawn, TrayHandle};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{AwayPrompt, ProjectOption};
    use std::sync::Mutex;

    fn signed_in(status: TrackerStatus) -> SharedState {
        SharedState {
            status,
            display_name: "ayesha@acme.com".into(),
            projects: vec![
                ProjectOption {
                    id: "p1".into(),
                    title: "Website_SEO".into(),
                },
                ProjectOption {
                    id: "p2".into(),
                    title: "Mobile".into(),
                },
            ],
            selected_project: Some("p1".into()),
            ..SharedState::default()
        }
    }

    /// Actions that record what they did.
    struct Recorder {
        actions: TrayActions,
        commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
        opened: Arc<Mutex<Vec<Option<GuiView>>>>,
        urls: Arc<Mutex<Vec<String>>>,
        autostart: Arc<Mutex<bool>>,
    }

    fn recorder(dashboard: Option<&str>) -> Recorder {
        let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
        let opened = Arc::new(Mutex::new(Vec::new()));
        let urls = Arc::new(Mutex::new(Vec::new()));
        let autostart = Arc::new(Mutex::new(false));
        let (o, u, a, b) = (
            opened.clone(),
            urls.clone(),
            autostart.clone(),
            autostart.clone(),
        );
        let at = Utc::now();
        Recorder {
            actions: TrayActions {
                sink: CommandSink::new(tx),
                clock: Arc::new(move || at),
                open_gui: Arc::new(move |view| o.lock().unwrap().push(view)),
                open_url: Arc::new(move |url| u.lock().unwrap().push(url.to_owned())),
                set_autostart: Arc::new(move |on| {
                    *a.lock().unwrap() = on;
                    on
                }),
                autostart_enabled: Arc::new(move || *b.lock().unwrap()),
                dashboard_url: dashboard.map(str::to_owned),
            },
            commands,
            opened,
            urls,
            autostart,
        }
    }

    #[test]
    fn underscores_are_escaped_for_dbusmenu() {
        let view = TrayView::new(
            &signed_in(TrackerStatus::Tracking),
            Utc::now(),
            (0.0, 0.0),
            false,
            true,
        );
        assert_eq!(view.projects[0].1, "Website__SEO");
        assert!(
            view.status_line.ends_with("— Website__SEO"),
            "{}",
            view.status_line
        );
        assert!(
            view.tooltip.contains("Website_SEO"),
            "the tooltip is not a menu label"
        );
    }

    #[test]
    fn the_toggle_pauses_resumes_or_opens_the_window() {
        let mut r = recorder(None);
        r.actions.perform(
            &signed_in(TrackerStatus::Tracking),
            TrayAction::ToggleTracking,
        );
        assert!(matches!(
            r.commands.try_recv(),
            Ok(Command::Pause {
                resume_after: None,
                ..
            })
        ));
        r.actions.perform(
            &signed_in(TrackerStatus::Paused),
            TrayAction::ToggleTracking,
        );
        assert!(matches!(r.commands.try_recv(), Ok(Command::Resume { .. })));
        r.actions
            .perform(&SharedState::default(), TrayAction::ToggleTracking);
        r.actions.perform(
            &signed_in(TrackerStatus::DeviceRevoked),
            TrayAction::ToggleTracking,
        );
        assert_eq!(
            *r.opened.lock().unwrap(),
            vec![Some(GuiView::Timer), Some(GuiView::Settings)]
        );
        assert!(r.commands.try_recv().is_err(), "nothing sent to the worker");
        // A pause already on its way is not sent twice.
        let mut pausing = signed_in(TrackerStatus::Tracking);
        pausing.pending_action = Some(PendingAction::Pausing);
        r.actions.perform(&pausing, TrayAction::ToggleTracking);
        assert!(r.commands.try_recv().is_err());
    }

    #[test]
    fn breaks_projects_sync_and_quit_become_commands() {
        let mut r = recorder(Some("https://app.acme.test"));
        let s = signed_in(TrackerStatus::Tracking);
        r.actions
            .perform(&s, TrayAction::Break(Duration::from_secs(1800)));
        assert!(matches!(
            r.commands.try_recv(),
            Ok(Command::Pause { resume_after: Some(d), .. }) if d == Duration::from_secs(1800)
        ));
        r.actions
            .perform(&s, TrayAction::SelectProject("p1".into()));
        assert!(r.commands.try_recv().is_err(), "already tracked");
        r.actions
            .perform(&s, TrayAction::SelectProject("p2".into()));
        assert!(matches!(
            r.commands.try_recv(),
            Ok(Command::SelectProject { project_id, .. }) if project_id == "p2"
        ));
        r.actions.perform(&s, TrayAction::SyncNow);
        assert!(matches!(r.commands.try_recv(), Ok(Command::SyncNow { .. })));
        r.actions.perform(&s, TrayAction::Quit);
        assert!(matches!(
            r.commands.try_recv(),
            Ok(Command::Shutdown { .. })
        ));
        r.actions.perform(&s, TrayAction::OpenDashboard);
        assert_eq!(*r.urls.lock().unwrap(), vec!["https://app.acme.test"]);
        r.actions.perform(&s, TrayAction::SetTask);
        r.actions.perform(&s, TrayAction::OpenSettings);
        assert_eq!(
            *r.opened.lock().unwrap(),
            vec![Some(GuiView::Task), Some(GuiView::Settings)]
        );
        r.actions.perform(&s, TrayAction::ToggleAutostart);
        assert!(*r.autostart.lock().unwrap());
        assert!(r.actions.view(&s).autostart);
    }

    #[test]
    fn no_dashboard_means_nothing_opens() {
        let r = recorder(None);
        r.actions.perform(
            &signed_in(TrackerStatus::Tracking),
            TrayAction::OpenDashboard,
        );
        assert!(r.urls.lock().unwrap().is_empty());
        assert!(!r.actions.view(&SharedState::default()).dashboard_enabled);
    }

    #[test]
    fn the_away_question_and_an_expired_sign_in_ask_for_attention() {
        let now = Utc::now();
        let mut s = signed_in(TrackerStatus::Tracking);
        assert!(!TrayView::new(&s, now, (0.0, 0.0), false, false).needs_attention);
        s.away_prompt = Some(AwayPrompt {
            from: now - chrono::Duration::minutes(30),
            to: now,
            project_title: "Website".into(),
        });
        assert!(TrayView::new(&s, now, (0.0, 0.0), false, false).needs_attention);
        let expired = signed_in(TrackerStatus::SignInExpired);
        assert!(TrayView::new(&expired, now, (0.0, 0.0), false, false).needs_attention);
    }

    #[test]
    fn the_launcher_uses_a_systemd_scope_when_it_can() {
        let scoped = GuiLauncher {
            gui: PathBuf::from("/usr/bin/dosi-tracker-gui"),
            systemd_run: Some(PathBuf::from("/usr/bin/systemd-run")),
        };
        let (program, args) = scoped.command_line(Some(GuiView::Settings));
        assert_eq!(program, Path::new("/usr/bin/systemd-run"));
        assert_eq!(
            args,
            [
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--description=Dosi Tracker window",
                "--",
                "/usr/bin/dosi-tracker-gui",
                "--view",
                "settings"
            ]
        );
        let plain = GuiLauncher {
            systemd_run: None,
            ..scoped
        };
        let (program, args) = plain.command_line(None);
        assert_eq!(program, Path::new("/usr/bin/dosi-tracker-gui"));
        assert!(args.is_empty());
    }

    #[test]
    fn systemd_run_is_used_only_with_a_user_manager() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("systemd-run");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runtime = temp.path().join("run");
        std::fs::create_dir_all(runtime.join("systemd")).unwrap();
        let gui = PathBuf::from("/opt/dosi/bin/dosi-tracker-gui");
        let path = bin.as_os_str();
        assert!(GuiLauncher::new(gui.clone(), Some(path), Some(&runtime))
            .systemd_run
            .is_none());
        std::fs::write(runtime.join("systemd/private"), "").unwrap();
        assert_eq!(
            GuiLauncher::new(gui.clone(), Some(path), Some(&runtime)).systemd_run,
            Some(tool)
        );
        assert!(GuiLauncher::new(gui, None, Some(&runtime))
            .systemd_run
            .is_none());
    }

    #[test]
    fn pixmaps_come_in_every_size_in_argb_order() {
        for size in ICON_SIZES {
            let argb = icon_argb(Some(Badge::Tracking), size);
            assert_eq!(argb.len(), (size * size * 4) as usize);
            assert_eq!(argb[0], 0, "transparent corner: alpha first");
            let centre = (((size / 2) * size + size / 2) * 4) as usize;
            assert_eq!(argb[centre], 255, "opaque centre");
        }
        // ARGB is RGBA rotated: the badge pixel reads A, R, G, B.
        let rgba = icon_rgba(Some(Badge::Tracking), 32);
        let argb = icon_argb(Some(Badge::Tracking), 32);
        let at = ((25 * 32 + 25) * 4) as usize;
        assert_eq!(&rgba[at..at + 4], &[0x00, 0xa3, 0x89, 0xff]);
        assert_eq!(&argb[at..at + 4], &[0xff, 0x00, 0xa3, 0x89]);
        // Signed out: the whole mark is grey and carries no badge.
        let grey = icon_rgba(Some(Badge::SignedOut), 32);
        let edge = ((16 * 32 + 2) * 4) as usize;
        assert_eq!(&grey[edge..edge + 3], &[0x9c, 0xa3, 0xaf]);
    }

    #[test]
    fn small_helpers_read_as_before() {
        assert_eq!(minutes_label(0), "0m");
        assert_eq!(minutes_label(222), "3h 42m");
        assert_eq!(truncate_chars("Checkout page redesign", 10), "Checkout…");
        assert!(can_pause(TrackerStatus::Away));
        assert!(!can_pause(TrackerStatus::Paused));
        assert!(!can_pause(TrackerStatus::NoSession));
    }
}
