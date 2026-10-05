//! The notification-area (system tray) icon.
//!
//! It shows the tracker's state at a glance — the icon's badge colour, and a
//! tooltip with the session and today's total — and puts the everyday
//! actions one right-click away: pause/resume, a timed break, switching
//! project, the task note, syncing, the web dashboard, settings and quit. A
//! left click opens the window. It also shows notifications (the time-away
//! question, a sign-in that expired, tracking that started by itself); a
//! click on one opens the window.
//!
//! Menu items are not thread-safe, so the [`Tray`] lives on the UI thread and
//! is refreshed from the shared snapshot every frame (only what changed is
//! touched). Clicks arrive on background channels and are forwarded to the app
//! as [`TrayEvent`]s, waking the UI so they are handled even while the window
//! is hidden.

use std::sync::mpsc::Sender;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use eframe::egui;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::state::{PendingAction, SharedState, TrackerStatus};
use crate::ui::format;

/// The timed breaks offered in the tray (the window offers the same).
pub const BREAKS: [(&str, &str, Duration); 3] = [
    ("break-15", "15 minutes", Duration::from_secs(15 * 60)),
    ("break-30", "30 minutes", Duration::from_secs(30 * 60)),
    ("break-60", "1 hour", Duration::from_secs(60 * 60)),
];

const ID_OPEN: &str = "open";
const ID_TOGGLE: &str = "toggle";
const ID_TASK: &str = "task";
const ID_SYNC: &str = "sync";
const ID_DASHBOARD: &str = "dashboard";
const ID_SETTINGS: &str = "settings";
const ID_AUTOSTART: &str = "autostart";
const ID_QUIT: &str = "quit";
const PROJECT_PREFIX: &str = "project:";

/// The task note as the menu quotes it, at most this many characters.
const TASK_LABEL_CHARS: usize = 36;

/// Something the user did in the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    /// Pause when tracking, resume when paused (or open the window to sign in).
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    /// The icon itself was clicked.
    IconClicked,
    /// A notification the tray showed was clicked.
    NotificationClicked,
    Action(TrayAction),
}

/// Map a menu item id back to its action.
pub fn parse_action(id: &str) -> Option<TrayAction> {
    if let Some(project) = id.strip_prefix(PROJECT_PREFIX) {
        return Some(TrayAction::SelectProject(project.to_owned()));
    }
    if let Some((_, _, duration)) = BREAKS.iter().find(|(break_id, _, _)| *break_id == id) {
        return Some(TrayAction::Break(*duration));
    }
    Some(match id {
        ID_OPEN => TrayAction::Open,
        ID_TOGGLE => TrayAction::ToggleTracking,
        ID_TASK => TrayAction::SetTask,
        ID_SYNC => TrayAction::SyncNow,
        ID_DASHBOARD => TrayAction::OpenDashboard,
        ID_SETTINGS => TrayAction::OpenSettings,
        ID_AUTOSTART => TrayAction::ToggleAutostart,
        ID_QUIT => TrayAction::Quit,
        _ => return None,
    })
}

/// Forward tray clicks and menu picks to the app and wake the UI, so they are
/// handled even while the window is hidden. Pointer moves over the icon are
/// dropped here rather than waking the UI for nothing.
pub fn spawn_event_forwarders(ctx: egui::Context, events: Sender<TrayEvent>) {
    let menu_ctx = ctx.clone();
    let menu_events = events.clone();
    let spawned = std::thread::Builder::new().name("tray-menu".into()).spawn(move || {
        while let Ok(event) = MenuEvent::receiver().recv() {
            if let Some(action) = parse_action(event.id.as_ref()) {
                if menu_events.send(TrayEvent::Action(action)).is_err() {
                    return;
                }
                menu_ctx.request_repaint();
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!(?e, "could not start the tray menu listener");
    }

    let spawned = std::thread::Builder::new().name("tray-icon".into()).spawn(move || {
        while let Ok(event) = TrayIconEvent::receiver().recv() {
            let clicked = matches!(
                event,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            );
            if clicked {
                if events.send(TrayEvent::IconClicked).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!(?e, "could not start the tray icon listener");
    }
}

/// Which badge the icon wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Tracking,
    Paused,
    Offline,
    SignedOut,
    Stopped,
}

/// Everything the tray shows, derived from a snapshot. Comparing two views
/// tells whether anything needs to be touched.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayView {
    pub badge: Badge,
    pub tooltip: String,
    pub status_line: String,
    pub totals_line: String,
    pub toggle_label: &'static str,
    pub toggle_enabled: bool,
    pub breaks_enabled: bool,
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
    pub fn new(snapshot: &SharedState, now: DateTime<Utc>, (today_secs, week_secs): (f64, f64), autostart: bool, dashboard: bool) -> Self {
        let idle = snapshot.pending_action.is_none();
        let signed_in = snapshot.status != TrackerStatus::SignedOut;
        // Signed in, with an account the server still accepts and a queue
        // that works: what the everyday actions need.
        let usable = signed_in && !matches!(snapshot.status, TrackerStatus::SignInExpired | TrackerStatus::Stopped);
        let paused = snapshot.status == TrackerStatus::Paused;
        let can_pause = idle && format::can_pause(snapshot.status);
        let project = snapshot.current_project().map(|p| p.title.as_str());

        let badge = match snapshot.status {
            TrackerStatus::Tracking => Badge::Tracking,
            TrackerStatus::Paused | TrackerStatus::Away | TrackerStatus::Locked => Badge::Paused,
            TrackerStatus::Offline | TrackerStatus::Connecting | TrackerStatus::ChooseProject => Badge::Offline,
            TrackerStatus::SignedOut => Badge::SignedOut,
            TrackerStatus::Stopped | TrackerStatus::SignInExpired => Badge::Stopped,
        };

        let since = |at: Option<DateTime<Utc>>| format::minutes_label(at.map_or(0, |at| (now - at).num_minutes().max(0) as u64));
        let state = match (snapshot.pending_action, snapshot.status) {
            (Some(PendingAction::Pausing), _) => "Pausing…".to_owned(),
            (Some(PendingAction::Resuming), _) => "Resuming…".to_owned(),
            (None, TrackerStatus::Tracking) => format!("Tracking · {}", since(snapshot.tracking_since)),
            (None, TrackerStatus::Paused) => match snapshot.resume_at {
                Some(at) => format!("Paused · resumes at {}", format::local_hm(at)),
                None => format!("Paused · {}", since(snapshot.paused_since)),
            },
            (None, TrackerStatus::Away) => match snapshot.away_since {
                Some(at) => format!("Away since {}", format::local_hm(at)),
                None => TrackerStatus::Away.label().to_owned(),
            },
            (None, status) => status.label().to_owned(),
        };
        let today = format::minutes_label((today_secs / 60.0) as u64);
        let week = format::minutes_label((week_secs / 60.0) as u64);

        let mut tooltip = format!("Dosi Tracker — {state}");
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
            (None, TrackerStatus::Paused) => ("Resume tracking", true),
            (None, _) => ("Pause tracking", can_pause),
        };

        let note = snapshot.task_note.trim();
        let task_label = if note.is_empty() {
            "Set task…".to_owned()
        } else {
            format!("Task: {}", format::truncate_chars(note, TASK_LABEL_CHARS))
        };

        Self {
            badge,
            tooltip: format::truncate_chars(&tooltip, 127),
            status_line: escape_menu_text(&status_line),
            totals_line,
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

/// A lone `&` in a Windows menu underlines the next letter instead of showing.
fn escape_menu_text(text: &str) -> String {
    text.replace('&', "&&")
}

/// A value Windows holds for the icon (its picture, its tooltip), as far as
/// we know. Setting one fails while the taskbar is not there yet — early in
/// a sign-in, or while Explorer restarts — and tray-icon then keeps the old
/// value; a failed update leaves it unknown here, so the next sync tries
/// again instead of trusting it. (Trusting it once left a grey "signed out"
/// badge on an agent that was tracking.)
#[derive(Debug)]
struct Shown<T>(Option<T>);

impl<T: PartialEq + Clone> Shown<T> {
    /// Set `want` with `set`, unless Windows already holds it. Remembered
    /// only once `set` worked.
    fn update<E>(&mut self, want: &T, set: impl FnOnce(&T) -> Result<(), E>) -> Result<(), E> {
        if self.0.as_ref() == Some(want) {
            return Ok(());
        }
        set(want)?;
        self.0 = Some(want.clone());
        Ok(())
    }
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    totals: MenuItem,
    toggle: MenuItem,
    breaks: Submenu,
    projects: Submenu,
    project_items: Vec<CheckMenuItem>,
    task: MenuItem,
    sync: MenuItem,
    dashboard: MenuItem,
    autostart: CheckMenuItem,
    /// What the menu last showed, so only changes touch it.
    applied: Option<TrayView>,
    badge_shown: Shown<Badge>,
    tooltip_shown: Shown<String>,
    /// The icon's id in the notification area, once found (for notifications).
    notify_id: Option<u32>,
}

impl Tray {
    /// Create the icon and its menu. Must run on the UI thread (it needs the
    /// message loop). Clicks on its notifications are sent to `events`.
    pub fn build(events: Sender<TrayEvent>, ctx: egui::Context) -> Result<Self> {
        let status = MenuItem::new("Dosi Tracker", false, None);
        let totals = MenuItem::new("", false, None);
        let toggle = MenuItem::with_id(ID_TOGGLE, "Pause tracking", false, None);
        let breaks = Submenu::new("Take a break", false);
        for (id, label, _) in BREAKS {
            breaks.append(&MenuItem::with_id(id, label, true, None))?;
        }
        let projects = Submenu::new("Switch project", false);
        let task = MenuItem::with_id(ID_TASK, "Set task…", false, None);
        let sync = MenuItem::with_id(ID_SYNC, "Sync now", false, None);
        let dashboard = MenuItem::with_id(ID_DASHBOARD, "Open web dashboard", true, None);
        let autostart = CheckMenuItem::with_id(ID_AUTOSTART, "Start with Windows", true, false, None);

        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &totals,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_OPEN, "Open Dosi Tracker", true, None),
            &PredefinedMenuItem::separator(),
            &toggle,
            &breaks,
            &projects,
            &task,
            &sync,
            &PredefinedMenuItem::separator(),
            &dashboard,
            &MenuItem::with_id(ID_SETTINGS, "Settings…", true, None),
            &autostart,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_QUIT, "Quit Dosi Tracker", true, None),
        ])?;

        let icon = TrayIconBuilder::new()
            .with_tooltip("Dosi Tracker")
            .with_menu(Box::new(menu))
            // Left click opens the window; the menu is on the right button.
            .with_menu_on_left_click(false)
            .with_icon(status_icon(Badge::SignedOut))
            .build()?;

        if !notification::watch_clicks(icon.window_handle(), events, ctx) {
            tracing::warn!("clicks on tray notifications will not open the window");
        }

        Ok(Self {
            icon,
            status,
            totals,
            toggle,
            breaks,
            projects,
            project_items: Vec::new(),
            task,
            sync,
            dashboard,
            autostart,
            applied: None,
            // What the builder set up; confirmed once Windows accepts an update.
            badge_shown: Shown(None),
            tooltip_shown: Shown(None),
            notify_id: None,
        })
    }

    /// Re-apply the menu on the next sync. Check items flip themselves when
    /// clicked, so after any menu pick their state is re-asserted from the
    /// truth rather than trusted.
    pub fn invalidate(&mut self) {
        self.applied = None;
    }

    /// Bring the icon and menu in line with `view`, touching only what changed.
    pub fn sync(&mut self, view: TrayView) {
        let icon = &self.icon;
        if let Err(e) = self.badge_shown.update(&view.badge, |badge| icon.set_icon(Some(status_icon(*badge)))) {
            tracing::debug!(?e, "could not update the tray icon yet; retrying");
        }
        if let Err(e) = self.tooltip_shown.update(&view.tooltip, |tooltip| icon.set_tooltip(Some(tooltip))) {
            tracing::debug!(?e, "could not update the tray tooltip yet; retrying");
        }

        let old = self.applied.take();
        let changed = |pick: &dyn Fn(&TrayView) -> bool| old.as_ref().is_none_or(pick);
        if changed(&|o| o.status_line != view.status_line) {
            self.status.set_text(&view.status_line);
        }
        if changed(&|o| o.totals_line != view.totals_line) {
            self.totals.set_text(&view.totals_line);
        }
        if changed(&|o| o.toggle_label != view.toggle_label || o.toggle_enabled != view.toggle_enabled) {
            self.toggle.set_text(view.toggle_label);
            self.toggle.set_enabled(view.toggle_enabled);
        }
        if changed(&|o| o.breaks_enabled != view.breaks_enabled) {
            self.breaks.set_enabled(view.breaks_enabled);
        }
        if changed(&|o| o.projects != view.projects) {
            for item in self.project_items.drain(..) {
                let _ = self.projects.remove(&item);
            }
            for (id, title) in &view.projects {
                let item = CheckMenuItem::with_id(format!("{PROJECT_PREFIX}{id}"), title, true, false, None);
                if self.projects.append(&item).is_ok() {
                    self.project_items.push(item);
                }
            }
        }
        if changed(&|o| o.projects != view.projects || o.selected_project != view.selected_project) {
            for (item, (id, _)) in self.project_items.iter().zip(&view.projects) {
                item.set_checked(view.selected_project.as_deref() == Some(id.as_str()));
            }
        }
        if changed(&|o| o.projects_enabled != view.projects_enabled) {
            self.projects.set_enabled(view.projects_enabled);
        }
        if changed(&|o| o.task_label != view.task_label || o.task_enabled != view.task_enabled) {
            self.task.set_text(&view.task_label);
            self.task.set_enabled(view.task_enabled);
        }
        if changed(&|o| o.sync_label != view.sync_label || o.sync_enabled != view.sync_enabled) {
            self.sync.set_text(view.sync_label);
            self.sync.set_enabled(view.sync_enabled);
        }
        if changed(&|o| o.dashboard_enabled != view.dashboard_enabled) {
            self.dashboard.set_enabled(view.dashboard_enabled);
        }
        if changed(&|o| o.autostart != view.autostart) {
            self.autostart.set_checked(view.autostart);
        }
        self.applied = Some(view);
    }

    /// Show a notification from the icon (a toast on Windows 10 and 11).
    /// `false` when it could not be shown — the taskbar is not there yet —
    /// so the caller can say it in the window instead.
    pub fn notify(&mut self, title: &str, body: &str, warning: bool) -> bool {
        let hwnd = self.icon.window_handle();
        let Some(id) = self.notify_id.or_else(|| notification::find_icon(hwnd)) else {
            return false;
        };
        if notification::show(hwnd, id, title, body, warning) {
            self.notify_id = Some(id);
            true
        } else {
            // Explorer may have restarted: look the icon up again next time.
            self.notify_id = None;
            false
        }
    }
}

/// `text` as UTF-16 for a fixed Windows buffer of `capacity` units: cut at
/// a character boundary (never half a surrogate pair), NUL-terminated.
fn to_wide_fixed(text: &str, capacity: usize) -> Vec<u16> {
    let room = capacity.saturating_sub(1);
    let mut wide = Vec::with_capacity(capacity);
    for c in text.chars() {
        let mut units = [0u16; 2];
        let encoded = c.encode_utf16(&mut units);
        if wide.len() + encoded.len() > room {
            break;
        }
        wide.extend_from_slice(encoded);
    }
    wide.push(0);
    wide
}

/// Notifications from the tray icon (`Shell_NotifyIcon` with `NIF_INFO`),
/// and their clicks.
#[cfg(windows)]
mod notification {
    use std::sync::mpsc::Sender;

    use eframe::egui;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_INFO,
        NIIF_INFO, NIIF_WARNING, NIM_MODIFY, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    };
    use windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY;

    use super::{to_wide_fixed, TrayEvent};

    /// The message tray-icon (0.24) has the notification area send to the
    /// icon's window (its `uCallbackMessage`); a click on a notification
    /// arrives as this message with `NIN_BALLOONUSERCLICK`. tray-icon itself
    /// only handles mouse events on it.
    const TRAY_CALLBACK: u32 = 6002;

    /// Our subclass of the icon's window, among any others (muda's menus).
    const SUBCLASS_ID: usize = 0xD051;

    /// tray-icon numbers its icons from 1 (one per icon this process
    /// creates), each in a window of its own; the id of ours is found by
    /// asking for the first one that exists in our window.
    const MAX_ICON_ID: u32 = 16;

    /// Where a click goes.
    struct ClickTarget {
        events: Sender<TrayEvent>,
        ctx: egui::Context,
    }

    fn hwnd(raw: *mut core::ffi::c_void) -> HWND {
        HWND(raw)
    }

    /// Hear clicks on the notifications of the icon whose window is `raw`.
    pub fn watch_clicks(raw: *mut core::ffi::c_void, events: Sender<TrayEvent>, ctx: egui::Context) -> bool {
        let target = Box::into_raw(Box::new(ClickTarget { events, ctx }));
        // Safety: the window belongs to this (the UI) thread; the target is
        // freed when the window is destroyed (WM_NCDESTROY) or right here.
        let installed = unsafe { SetWindowSubclass(hwnd(raw), Some(on_message), SUBCLASS_ID, target as usize) }.as_bool();
        if !installed {
            drop(unsafe { Box::from_raw(target) });
        }
        installed
    }

    unsafe extern "system" fn on_message(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, id: usize, data: usize) -> LRESULT {
        if msg == TRAY_CALLBACK && (lparam.0 as u32 & 0xFFFF) == NIN_BALLOONUSERCLICK {
            let target = &*(data as *const ClickTarget);
            if target.events.send(TrayEvent::NotificationClicked).is_ok() {
                target.ctx.request_repaint();
            }
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(window, Some(on_message), id);
            drop(Box::from_raw(data as *mut ClickTarget));
        }
        DefSubclassProc(window, msg, wparam, lparam)
    }

    /// The id of the icon in `raw`, if the notification area has it.
    pub fn find_icon(raw: *mut core::ffi::c_void) -> Option<u32> {
        (1..=MAX_ICON_ID).find(|&id| {
            let identifier = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: hwnd(raw),
                uID: id,
                ..Default::default()
            };
            unsafe { Shell_NotifyIconGetRect(&identifier) }.is_ok()
        })
    }

    /// Show a notification from icon `id` of `raw`.
    pub fn show(raw: *mut core::ffi::c_void, id: u32, title: &str, body: &str, warning: bool) -> bool {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd(raw),
            uID: id,
            uFlags: NIF_INFO,
            dwInfoFlags: if warning { NIIF_WARNING } else { NIIF_INFO },
            ..Default::default()
        };
        let title = to_wide_fixed(title, data.szInfoTitle.len());
        data.szInfoTitle[..title.len()].copy_from_slice(&title);
        let body = to_wide_fixed(body, data.szInfo.len());
        data.szInfo[..body.len()].copy_from_slice(&body);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
    }
}

#[cfg(not(windows))]
mod notification {
    use std::sync::mpsc::Sender;

    use eframe::egui;

    use super::TrayEvent;

    pub fn watch_clicks(_raw: *mut core::ffi::c_void, _events: Sender<TrayEvent>, _ctx: egui::Context) -> bool {
        false
    }

    pub fn find_icon(_raw: *mut core::ffi::c_void) -> Option<u32> {
        None
    }

    pub fn show(_raw: *mut core::ffi::c_void, _id: u32, _title: &str, _body: &str, _warning: bool) -> bool {
        false
    }
}

/// The brand mark with a status badge, drawn in code so the binary stays
/// self-contained (no .ico asset to ship or lose). Also the window icon.
pub fn status_icon(badge: Badge) -> tray_icon::Icon {
    let (rgba, size) = icon_pixels(Some(badge));
    tray_icon::Icon::from_rgba(rgba, size, size).expect("valid generated icon")
}

/// The plain brand mark as window/taskbar icon data.
pub fn window_icon() -> egui::IconData {
    let (rgba, size) = icon_pixels(None);
    egui::IconData { rgba, width: size, height: size }
}

/// Rasterise the icon (anti-aliased) at 32×32: a brand-blue disc with a white
/// ring and centre dot, plus an optional status badge at the bottom right.
fn icon_pixels(badge: Option<Badge>) -> (Vec<u8>, u32) {
    const SIZE: u32 = 32;
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

    // Coverage of a disc of radius r at distance d (1 px soft edge).
    let cover = |d: f32, r: f32| (r - d + 0.5).clamp(0.0, 1.0);
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let c = (SIZE as f32 - 1.0) / 2.0;
    let (bx, by, br) = (24.5_f32, 24.5_f32, 7.0_f32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32, y as f32);
            let d = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
            // Layers, back to front: disc, ring, centre dot.
            let mut px = [0.0_f32; 4];
            let mut over = |color: [u8; 3], alpha: f32| {
                let a = alpha.clamp(0.0, 1.0);
                for i in 0..3 {
                    px[i] = color[i] as f32 / 255.0 * a + px[i] * (1.0 - a);
                }
                px[3] = a + px[3] * (1.0 - a);
            };
            over(brand, cover(d, 15.5));
            let ring = cover(d, 10.0) - cover(d, 7.25);
            over([0xff, 0xff, 0xff], ring);
            over([0xff, 0xff, 0xff], cover(d, 3.5));
            if let Some(color) = badge_color {
                let bd = ((fx - bx).powi(2) + (fy - by).powi(2)).sqrt();
                // A white outline separates the badge from the disc.
                over([0xff, 0xff, 0xff], cover(bd, br));
                over(color, cover(bd, br - 2.0));
            }
            // Un-premultiply for straight-alpha RGBA.
            let a = px[3];
            let channel = |v: f32| if a > 0.0 { (v / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            rgba.extend_from_slice(&[channel(px[0]), channel(px[1]), channel(px[2]), (a * 255.0).round() as u8]);
        }
    }
    (rgba, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ProjectOption;

    fn signed_in(status: TrackerStatus) -> SharedState {
        SharedState {
            status,
            display_name: "ayesha@acme.com".into(),
            projects: vec![
                ProjectOption { id: "p1".into(), title: "Website & SEO".into() },
                ProjectOption { id: "p2".into(), title: "Mobile".into() },
            ],
            selected_project: Some("p1".into()),
            ..SharedState::default()
        }
    }

    fn view(snapshot: &SharedState) -> TrayView {
        TrayView::new(snapshot, Utc::now(), (0.0, 0.0), false, true)
    }

    #[test]
    fn menu_ids_round_trip_to_actions() {
        assert_eq!(parse_action(ID_OPEN), Some(TrayAction::Open));
        assert_eq!(parse_action(ID_QUIT), Some(TrayAction::Quit));
        assert_eq!(parse_action(ID_TASK), Some(TrayAction::SetTask));
        assert_eq!(parse_action("break-30"), Some(TrayAction::Break(Duration::from_secs(1800))));
        assert_eq!(parse_action("project:abc-1"), Some(TrayAction::SelectProject("abc-1".into())));
        assert_eq!(parse_action("nonsense"), None);
    }

    #[test]
    fn tracking_view_offers_pause_and_shows_the_project() {
        let now = Utc::now();
        let mut snapshot = signed_in(TrackerStatus::Tracking);
        snapshot.tracking_since = Some(now - chrono::Duration::minutes(65));
        let view = TrayView::new(&snapshot, now, (3.0 * 3600.0 + 42.0 * 60.0, 18.0 * 3600.0), true, true);
        assert_eq!(view.badge, Badge::Tracking);
        assert_eq!(view.toggle_label, "Pause tracking");
        assert!(view.toggle_enabled && view.breaks_enabled && view.projects_enabled && view.task_enabled);
        assert_eq!(view.status_line, "● Tracking · 1h 05m — Website && SEO");
        assert_eq!(view.totals_line, "Today 3h 42m · This week 18h 00m");
        assert!(view.tooltip.starts_with("Dosi Tracker — Tracking · 1h 05m\nWebsite & SEO · Today 3h 42m"));
        assert_eq!(view.projects[0].1, "Website && SEO");
        assert_eq!(view.task_label, "Set task…");
        assert!(view.autostart && view.dashboard_enabled);
    }

    #[test]
    fn the_task_note_is_quoted_short_and_escaped() {
        let mut snapshot = signed_in(TrackerStatus::Tracking);
        snapshot.task_note = "Q&A for the checkout page redesign and the release notes".into();
        let label = view(&snapshot).task_label;
        assert!(label.starts_with("Task: Q&&A for the checkout"), "{label}");
        assert!(label.ends_with('…'));
    }

    #[test]
    fn paused_view_offers_resume_and_upload_only() {
        let now = Utc::now();
        let mut snapshot = signed_in(TrackerStatus::Paused);
        snapshot.paused_since = Some(now);
        let view = TrayView::new(&snapshot, now, (0.0, 0.0), false, true);
        assert_eq!(view.badge, Badge::Paused);
        assert_eq!(view.toggle_label, "Resume tracking");
        assert!(view.toggle_enabled && view.breaks_enabled);
        assert_eq!(view.sync_label, "Upload queued");
    }

    #[test]
    fn away_view_says_since_when() {
        let now = Utc::now();
        let mut snapshot = signed_in(TrackerStatus::Away);
        snapshot.away_since = Some(now - chrono::Duration::minutes(7));
        let view = TrayView::new(&snapshot, now, (0.0, 0.0), false, true);
        assert_eq!(view.badge, Badge::Paused);
        assert!(view.status_line.starts_with("● Away since "), "{}", view.status_line);
        assert!(view.toggle_enabled, "a break can still be taken");
    }

    #[test]
    fn a_request_in_flight_disables_the_actions() {
        let mut snapshot = signed_in(TrackerStatus::Tracking);
        snapshot.pending_action = Some(PendingAction::Pausing);
        let view = view(&snapshot);
        assert_eq!(view.toggle_label, "Pausing…");
        assert!(!view.toggle_enabled && !view.breaks_enabled && !view.projects_enabled);
    }

    #[test]
    fn signed_out_view_invites_sign_in() {
        let view = view(&SharedState::default());
        assert_eq!(view.badge, Badge::SignedOut);
        assert_eq!(view.toggle_label, "Sign in…");
        assert!(view.toggle_enabled);
        assert!(!view.sync_enabled && !view.projects_enabled && !view.breaks_enabled && !view.task_enabled);
        assert_eq!(view.tooltip, "Dosi Tracker — Signed out");
    }

    #[test]
    fn an_expired_sign_in_only_offers_signing_in_again() {
        let view = view(&signed_in(TrackerStatus::SignInExpired));
        assert_eq!(view.badge, Badge::Stopped);
        assert_eq!((view.toggle_label, view.toggle_enabled), ("Sign in…", true));
        assert!(!view.sync_enabled && !view.projects_enabled && !view.breaks_enabled && !view.task_enabled);
    }

    #[test]
    fn choosing_a_project_offers_the_list() {
        let mut snapshot = signed_in(TrackerStatus::ChooseProject);
        snapshot.selected_project = None;
        let view = view(&snapshot);
        assert_eq!(view.toggle_label, "Choose a project…");
        assert!(view.projects_enabled && view.task_enabled);
        assert_eq!(view.selected_project, None);
    }

    #[test]
    fn an_unconfigured_dashboard_is_greyed_out() {
        let snapshot = signed_in(TrackerStatus::Tracking);
        assert!(!TrayView::new(&snapshot, Utc::now(), (0.0, 0.0), false, false).dashboard_enabled);
    }

    #[test]
    fn a_failed_icon_update_is_retried_and_a_successful_one_is_not_repeated() {
        let mut shown = Shown(None);
        let mut calls = 0;
        // The taskbar is not there yet: Windows refuses.
        let refused = shown.update(&Badge::Tracking, |_| {
            calls += 1;
            Err("no taskbar")
        });
        assert!(refused.is_err());
        assert_eq!(shown.0, None, "not remembered as shown");
        // The next sync tries again, and it works.
        assert!(shown.update(&Badge::Tracking, |_| {
            calls += 1;
            Ok::<(), &str>(())
        })
        .is_ok());
        assert_eq!(shown.0, Some(Badge::Tracking));
        // Nothing changed: Windows is not asked again.
        assert!(shown.update(&Badge::Tracking, |_| -> Result<(), &str> { panic!("not called") }).is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    fn notification_text_fits_its_buffer_without_splitting_characters() {
        assert_eq!(to_wide_fixed("Hi", 64), vec![u16::from(b'H'), u16::from(b'i'), 0]);
        let long = "x".repeat(300);
        let wide = to_wide_fixed(&long, 256);
        assert_eq!(wide.len(), 256);
        assert_eq!(wide.last(), Some(&0));
        // An emoji is two UTF-16 units: it is left out whole, never halved.
        let wide = to_wide_fixed("ab😀", 4);
        assert_eq!(wide, vec![u16::from(b'a'), u16::from(b'b'), 0]);
        assert_eq!(to_wide_fixed("", 64), vec![0]);
    }

    #[test]
    fn icons_are_32px_rgba_with_transparent_corners() {
        for badge in [None, Some(Badge::Tracking), Some(Badge::SignedOut), Some(Badge::Stopped)] {
            let (rgba, size) = icon_pixels(badge);
            assert_eq!(size, 32);
            assert_eq!(rgba.len(), 32 * 32 * 4);
            assert_eq!(rgba[3], 0, "top-left corner is transparent");
            let center = ((16 * 32 + 16) * 4) as usize;
            assert_eq!(rgba[center + 3], 255, "centre is opaque");
        }
        // The badge colours the bottom-right corner area.
        let (rgba, _) = icon_pixels(Some(Badge::Tracking));
        let at = ((25 * 32 + 25) * 4) as usize;
        assert_eq!(&rgba[at..at + 3], &[0x00, 0xa3, 0x89]);
    }
}
