//! State shared between the UI thread and the background tracking worker.
//!
//! The worker owns all network/capture work; the UI only reads this snapshot and
//! pushes commands down a channel. Everything here is cheap to clone/lock so the
//! UI never blocks on the tracker.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::storage::QueuedRow;

/// What the tracker is currently doing (drives the status pill + tray tooltip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerStatus {
    /// No credentials yet — the login view is showing.
    SignedOut,
    /// Signing in / resolving projects.
    Connecting,
    /// Actively tracking on a project.
    Tracking,
    /// User pressed pause (or took a timed break); no captures are taken.
    Paused,
    /// Signed in, but the server cannot be reached and no recent project list
    /// is saved to track offline with.
    Offline,
    /// Nothing is being captured and the user must act: the background worker
    /// died, or the local queue cannot be opened (`last_error` says why; it is
    /// retried every minute). Surfaced loudly rather than silently pretending
    /// to track.
    Stopped,
    /// No keyboard or mouse input for the project's idle threshold: the block
    /// was ended at the last input and nothing is recorded until the user is
    /// back (CONTRACT §4).
    Away,
    /// The Windows session is locked (or disconnected): nothing is recorded.
    Locked,
    /// Signed in, but no project is chosen: there are several and none was
    /// remembered, the remembered one is gone, or there are none at all.
    ChooseProject,
    /// The server no longer accepts the saved sign-in (the refresh token was
    /// revoked or expired, the password changed, the account deactivated).
    /// Tracking stopped; the queue is kept and uploads once the password is
    /// entered again (workspace and email prefilled).
    SignInExpired,
    /// Affirmative local consent has not been granted, or was revoked.
    ConsentRequired,
}

impl TrackerStatus {
    pub fn label(self) -> &'static str {
        match self {
            TrackerStatus::SignedOut => "Signed out",
            TrackerStatus::Connecting => "Connecting…",
            TrackerStatus::Tracking => "Tracking",
            TrackerStatus::Paused => "Paused",
            TrackerStatus::Offline => "Offline — will retry",
            TrackerStatus::Stopped => "Stopped",
            TrackerStatus::Away => "Away",
            TrackerStatus::Locked => "Locked",
            TrackerStatus::ChooseProject => "Choose a project",
            TrackerStatus::SignInExpired => "Sign-in expired",
            TrackerStatus::ConsentRequired => "Consent required",
        }
    }
}

/// A project the user may track against (mirrors the backend's my-projects DTO).
#[derive(Debug, Clone)]
pub struct ProjectOption {
    pub id: String,
    pub title: String,
}

/// What the current project captures — the server's policy combined with the
/// local config. Shown read-only in the window; the live view also hides
/// input counts the project does not record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFlags {
    pub screenshot: bool,
    pub webcam: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
    /// Screen captures are blurred (text unreadable) before they leave the
    /// computer.
    pub blur_screenshots: bool,
}

impl Default for CaptureFlags {
    fn default() -> Self {
        Self {
            screenshot: false,
            webcam: false,
            keyboard: true,
            mouse: true,
            active_window: false,
            running_programs: false,
            blur_screenshots: false,
        }
    }
}

/// Where a recorded block is on its way to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadState {
    /// Saved locally, waiting to upload.
    Queued,
    Uploaded,
    /// The server refused it; it is kept locally but never uploaded.
    Rejected,
}

/// A block recorded during this run, for the window's "recent activity" list.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentBlock {
    /// The local queue row, so the upload result can be matched to it.
    pub row_id: i64,
    pub project: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub productivity: u8,
    /// `None` when that kind of input was not measured.
    pub keyboard_hits: Option<u64>,
    pub mouse_clicks: Option<u64>,
    pub screenshot: bool,
    pub upload: UploadState,
}

/// How many recent blocks the window lists.
pub const RECENT_BLOCKS: usize = 12;

/// The thumbnail of the latest screenshot, shown so people can see exactly
/// what was captured.
#[derive(Debug, PartialEq)]
pub struct LastScreenshot {
    pub taken_at: DateTime<Utc>,
    /// JPEG bytes (the small list thumbnail, not the full capture).
    pub jpeg: Vec<u8>,
}

/// A pause/resume the user asked for that the worker has not applied yet. The
/// window reflects it at once, so the buttons never feel unresponsive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    Pausing,
    Resuming,
}

/// The time the user was away, waiting for "Keep as work" or "Discard"
/// ([`Command::ResolveAway`]). Discarding is the default: a question nobody
/// answers records nothing (CONTRACT §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwayPrompt {
    /// The last input before the user left.
    pub from: DateTime<Utc>,
    /// When they came back (or the screen locked / the computer slept, which
    /// is never tracked).
    pub to: DateTime<Utc>,
    /// The project the time would be kept on: the one tracked when they left.
    pub project_title: String,
}

impl AwayPrompt {
    /// Whole minutes away, for "You were away for 42 min".
    #[allow(dead_code, reason = "rendered by the window's away prompt")]
    pub fn minutes(&self) -> i64 {
        (self.to - self.from).num_minutes().max(0)
    }
}

/// Why queued blocks are not reaching the server right now (CONTRACT §1,
/// "How agents treat upload responses"). The blocks stay queued; each kind
/// is retried on its own schedule, and "Sync now" retries at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadIssue {
    /// The server could not be reached or failed: retried with a growing
    /// pause (30 s up to 30 min).
    Retrying { next_attempt_at: DateTime<Utc>, reason: String },
    /// HTTP 402: the workspace's subscription is inactive. Retried hourly.
    SubscriptionInactive { next_attempt_at: DateTime<Utc>, message: String },
    /// HTTP 425: the blocks end in the server's future — this computer's
    /// clock is ahead. Retried every 5 minutes.
    ClockAhead { next_attempt_at: DateTime<Utc> },
}

impl UploadIssue {
    /// One line for the window.
    pub fn message(&self) -> String {
        match self {
            UploadIssue::Retrying { .. } => "Offline — uploads will retry.".to_owned(),
            UploadIssue::SubscriptionInactive { .. } => {
                "Workspace subscription inactive — your work is kept and uploads once it is renewed.".to_owned()
            }
            UploadIssue::ClockAhead { .. } => {
                "Your computer clock is ahead — uploads wait until the server's time catches up.".to_owned()
            }
        }
    }

    pub fn next_attempt_at(&self) -> DateTime<Utc> {
        match self {
            UploadIssue::Retrying { next_attempt_at, .. }
            | UploadIssue::SubscriptionInactive { next_attempt_at, .. }
            | UploadIssue::ClockAhead { next_attempt_at } => *next_attempt_at,
        }
    }
}

/// Something worth telling the user once (a toast or tray balloon).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Increases with every notice, so the window shows each one once.
    pub id: u64,
    pub at: DateTime<Utc>,
    pub kind: NoticeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    /// Tracking started without the user doing anything (Windows started the
    /// app at sign-in).
    TrackingStartedAutomatically { project_title: String },
    /// Signed in with several projects and none chosen: pick one to start.
    ChooseProject,
    /// The project being tracked is no longer available (archived, or the
    /// membership was removed). Tracking moved to `now_tracking` when it is
    /// the only project left, otherwise it stopped until one is chosen.
    ProjectUnavailable { title: String, now_tracking: Option<String> },
    /// The local queue was damaged; it was set aside at `moved_to` and a fresh
    /// one started.
    QueueReset { moved_to: String },
    /// The saved sign-in no longer works.
    SignInExpired,
    /// The server refused the workspace tracked in: access removed, the
    /// workspace archived, or (a setup problem) the workspace never named.
    WorkspaceLost { name: String, refusal: crate::api::WorkspaceRefusal },
    /// A workspace admin removed this computer: tracking stopped and the
    /// saved sign-in was forgotten.
    DeviceRevoked,
}

impl Notice {
    /// One line for a toast.
    #[allow(dead_code, reason = "shown by the window and the tray as toasts")]
    pub fn text(&self) -> String {
        match &self.kind {
            NoticeKind::TrackingStartedAutomatically { project_title } => {
                format!("Dosi Tracker started with Windows and is tracking {project_title}.")
            }
            NoticeKind::ChooseProject => "Choose the project you are working on to start tracking.".to_owned(),
            NoticeKind::ProjectUnavailable { title, now_tracking: Some(next) } => {
                format!("{title} is no longer available. Now tracking {next}.")
            }
            NoticeKind::ProjectUnavailable { title, now_tracking: None } => {
                format!("{title} is no longer available. Choose a project to continue tracking.")
            }
            NoticeKind::QueueReset { moved_to } => {
                format!("The local queue was damaged and was set aside ({moved_to}). Tracking continues.")
            }
            NoticeKind::SignInExpired => "Your sign-in has expired. Enter your password to continue tracking.".to_owned(),
            NoticeKind::WorkspaceLost { name, refusal } => format!("{} Choose another workspace.", workspace_lost_text(name, *refusal)),
            NoticeKind::DeviceRevoked => DEVICE_REVOKED_TEXT.to_owned(),
        }
    }
}

/// Why tracking stopped after a workspace admin removed this computer.
pub const DEVICE_REVOKED_TEXT: &str = "This computer was removed from Dosi Tracker by your workspace admin. \
     Tracking stopped and you were signed out; your recorded time is kept on this computer.";

/// One sentence on why the workspace `name` can no longer be tracked in.
pub fn workspace_lost_text(name: &str, refusal: crate::api::WorkspaceRefusal) -> String {
    match refusal {
        crate::api::WorkspaceRefusal::NotMember => format!("You no longer have access to {name}."),
        crate::api::WorkspaceRefusal::Archived => format!("{name} was archived; nothing more can be tracked there."),
        crate::api::WorkspaceRefusal::Required => {
            format!("The server did not receive which workspace to use ({name}); a proxy may be removing it.")
        }
    }
}

/// A workspace offered by the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChoice {
    pub name: String,
    /// The user's role there ("Owner", "Admin", "Member"…).
    pub role: Option<String>,
    /// The workspace tracked in now.
    pub is_current: bool,
}

/// How many notices are kept for the window to pick up.
pub const MAX_NOTICES: usize = 8;

/// Snapshot the UI renders. Updated by the worker, read by the UI each frame.
///
/// The clocks are published as instants rather than elapsed values, so the
/// window can animate timers every second without any help from the worker.
#[derive(Debug, Clone)]
pub struct SharedState {
    pub status: TrackerStatus,
    pub pending_action: Option<PendingAction>,
    pub display_name: String,
    /// The name of the workspace tracked in.
    pub workspace: String,
    /// The server this account talks to (the address saved at sign-in, else
    /// the configured one). Set with [`SharedState::set_server`].
    pub server_url: String,
    /// `server_url` is plain `http://` to another computer (the admin allowed
    /// it): the window warns that the connection is not encrypted.
    pub unencrypted_connection: bool,
    /// Whether the server answered the latest request. Tracking goes on
    /// offline (on the saved project list); blocks upload once it is back.
    pub server_reachable: bool,
    pub projects: Vec<ProjectOption>,
    pub selected_project: Option<String>,
    /// Seconds tracked today / this week: the backend's totals, plus blocks
    /// recorded since they were fetched. The block in progress is added live.
    pub tracked_today_secs: f64,
    pub tracked_week_secs: f64,
    /// Seconds tracked on each local day of this week, Monday first, for the
    /// week chart. Days before today come from the backend; today's slot is
    /// not used (the window shows the live total instead).
    pub week_day_secs: [f64; 7],
    /// Local midnight of the day `week_day_secs` was fetched on, and when —
    /// so past days are fetched again once a day changes or they get stale.
    pub week_days_fetched: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Today's average activity level (0-100), from the backend.
    pub last_productivity: u8,
    pub interval_minutes: u64,
    /// Minutes without input before the block is ended as idle on the current
    /// project; `None` when the project turned idle detection off.
    pub idle_threshold_minutes: Option<u32>,
    /// What the current project captures.
    pub capture: CaptureFlags,
    /// "What are you working on?" — sent as each block's description.
    pub task_note: String,
    /// Blocks recorded during this run, newest first (at most [`RECENT_BLOCKS`]).
    pub recent_blocks: Vec<RecentBlock>,
    pub last_screenshot: Option<Arc<LastScreenshot>>,
    /// While tracking: when this uninterrupted stretch began (sign-in,
    /// resume, project switch, unlock, return), when the block in progress
    /// began, and when it is saved (its planned end — never the screenshot
    /// moment, which is random and not shown).
    pub tracking_since: Option<DateTime<Utc>>,
    pub block_started_at: Option<DateTime<Utc>>,
    pub next_capture_at: Option<DateTime<Utc>>,
    /// While tracking: the start of the time not yet saved locally. Earlier
    /// than `block_started_at` while finished blocks wait to see whether the
    /// user is still there; the live totals count from here.
    pub unsaved_since: Option<DateTime<Utc>>,
    /// While paused: since when, and when tracking resumes by itself (a
    /// timed pause) — `None` means until the user resumes.
    pub paused_since: Option<DateTime<Utc>>,
    pub resume_at: Option<DateTime<Utc>>,
    /// While away: the last input before the user left.
    pub away_since: Option<DateTime<Utc>>,
    /// After coming back: keep the time away as work, or discard it?
    pub away_prompt: Option<AwayPrompt>,
    pub last_sync: Option<DateTime<Utc>>,
    pub pending_uploads: u32,
    /// Uploads the server permanently rejected. Kept visible so silently dropped
    /// data is never invisible to the user.
    pub rejected_uploads: u32,
    /// The newest queued and parked rows with the server's reason for each
    /// parked one (and why the last attempt of a queued one failed).
    pub queue_rows: Vec<QueuedRow>,
    /// Why uploads are held back right now, if they are.
    pub upload_issue: Option<UploadIssue>,
    /// Minutes this computer's clock is off from the server's, once that
    /// exceeds 2 minutes: positive = behind, negative = ahead. Timestamps are
    /// never rewritten; the window warns.
    pub clock_skew_minutes: Option<i64>,
    pub last_error: Option<String>,
    /// Set once a sign-in attempt fails so the login view can show it.
    pub login_error: Option<String>,
    pub signing_in: bool,
    /// Workspaces to choose from after a sign-in or "Switch workspace".
    pub workspace_choices: Vec<WorkspaceChoice>,
    /// The workspaces of the saved sign-in are being looked up.
    pub finding_workspaces: bool,
    /// The answer to "Switch workspace" when there is nothing to choose.
    pub workspace_note: Option<String>,
    /// The account lost access to this workspace: another must be chosen.
    pub workspace_lost: Option<String>,
    /// Things to tell the user once, oldest first (at most [`MAX_NOTICES`]).
    pub notices: Vec<Notice>,
    /// Current affirmative local user consent state.
    pub consent: crate::config::LocalConsent,
    /// Persistence error from saving consent to disk (if any).
    pub consent_error: Option<String>,
    /// Whether in-memory consent differs from on-disk consent due to a write failure.
    pub consent_unsynced: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            status: TrackerStatus::SignedOut,
            pending_action: None,
            display_name: String::new(),
            workspace: String::new(),
            server_url: String::new(),
            unencrypted_connection: false,
            server_reachable: true,
            projects: Vec::new(),
            selected_project: None,
            tracked_today_secs: 0.0,
            tracked_week_secs: 0.0,
            week_day_secs: [0.0; 7],
            week_days_fetched: None,
            last_productivity: 0,
            interval_minutes: 10,
            idle_threshold_minutes: None,
            capture: CaptureFlags::default(),
            task_note: String::new(),
            recent_blocks: Vec::new(),
            last_screenshot: None,
            tracking_since: None,
            block_started_at: None,
            next_capture_at: None,
            unsaved_since: None,
            paused_since: None,
            resume_at: None,
            away_since: None,
            away_prompt: None,
            last_sync: None,
            pending_uploads: 0,
            rejected_uploads: 0,
            queue_rows: Vec::new(),
            upload_issue: None,
            clock_skew_minutes: None,
            last_error: None,
            login_error: None,
            signing_in: false,
            workspace_choices: Vec::new(),
            finding_workspaces: false,
            workspace_note: None,
            workspace_lost: None,
            notices: Vec::new(),
            consent: crate::config::LocalConsent::Absent,
            consent_error: None,
            consent_unsynced: false,
        }
    }
}

impl SharedState {
    /// The project being tracked, if one is selected and still listed.
    pub fn current_project(&self) -> Option<&ProjectOption> {
        let id = self.selected_project.as_deref()?;
        self.projects.iter().find(|p| p.id == id)
    }

    /// Show `server` as the one talked to, and whether that is unencrypted.
    pub fn set_server(&mut self, server: String) {
        self.unencrypted_connection = crate::config::is_unencrypted_remote(&server);
        self.server_url = server;
    }

    /// Queue a notice for the window, keeping the newest [`MAX_NOTICES`].
    pub fn push_notice(&mut self, kind: NoticeKind, at: DateTime<Utc>) {
        let id = self.notices.last().map_or(1, |last| last.id + 1);
        self.notices.push(Notice { id, at, kind });
        let excess = self.notices.len().saturating_sub(MAX_NOTICES);
        self.notices.drain(..excess);
    }
}

/// Cheap shared handle to the snapshot.
#[derive(Clone, Default)]
pub struct StateHandle(Arc<Mutex<SharedState>>);

impl StateHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a clone of the current snapshot (UI renders from this).
    ///
    /// A poisoned lock is recovered rather than propagated: one panicking thread
    /// must not cascade into killing the UI on every subsequent frame.
    pub fn snapshot(&self) -> SharedState {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Mutate the snapshot in place.
    pub fn update(&self, f: impl FnOnce(&mut SharedState)) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
    }
}

/// Commands the UI sends to the background worker.
///
/// Every command that ends the block in progress carries `at`, the moment the
/// user acted (taken by the window when the click happened): the block ends
/// exactly there, however long the worker takes to get to it (CONTRACT §4).
#[derive(Debug, Clone)]
pub enum Command {
    /// Sign in, then pick the workspace: the one named by `workspace` (a
    /// name or id), the only one, or one of [`SharedState::workspace_choices`].
    /// `server` is the address typed on the sign-in screen; `None` keeps the
    /// one saved with this account (or the configured one).
    SignIn { workspace: Option<String>, username: String, password: String, server: Option<String> },
    /// Use `workspace_choices[index]` after signing in.
    ChooseWorkspace { index: usize },
    /// Close the workspace picker.
    CancelWorkspaceChoice,
    /// Signed in: list the user's workspaces.
    SwitchWorkspace,
    /// Track in `workspace_choices[index]` from now on (same sign-in). The
    /// block ends at `at`.
    SwitchToWorkspace { index: usize, at: DateTime<Utc> },
    /// Save the block in progress (ended by `signout`), say so to the server
    /// and forget the saved sign-in.
    SignOut { at: DateTime<Utc> },
    /// Track another project; the block in progress ends (by `project`).
    SelectProject { project_id: String, at: DateTime<Utc> },
    /// Stop tracking. The work recorded so far is saved first (ended by
    /// `pause`, or `break` for a timed one). With a duration, tracking resumes
    /// by itself when it runs out (once the user is at the computer); sent
    /// while already paused, it just changes when (or whether) that happens.
    /// The pause survives a restart.
    Pause { resume_after: Option<std::time::Duration>, at: DateTime<Utc> },
    /// End a pause: a new block starts at `at`.
    Resume { at: DateTime<Utc> },
    /// Save the block in progress now and upload — or, while paused or with a
    /// block under a minute old, only upload what is already queued. Uploads
    /// are retried at once, whatever their back-off.
    SyncNow { at: DateTime<Utc> },
    /// Change the task note. Send it when the user commits the text (Enter,
    /// or leaving the field), not on every keystroke: a change ends the block
    /// in progress (by `task`).
    #[allow(dead_code, reason = "sent by the window's task note field")]
    SetTaskNote { note: String, at: DateTime<Utc> },
    /// Answer the [`AwayPrompt`]: keep the time away as work (recorded as idle
    /// blocks, CONTRACT §4) or discard it.
    #[allow(dead_code, reason = "sent by the window's away prompt")]
    ResolveAway { keep: bool },
    /// Quit: save the block in progress (ended by `quit`), tell the server,
    /// upload what fits in the grace period, and stop.
    Shutdown { at: DateTime<Utc> },
    /// Update affirmative local user consent (grant, decline, revoke, modify).
    UpdateConsent { consent: crate::config::LocalConsent },
}

/// Requests from outside the window (a second launch, the installer, the
/// session ending) that the window acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSignal {
    /// Bring the window to the front (the app was launched again).
    Show,
    /// Quit gracefully (`dosi-tracker --quit`, used by the installer).
    Quit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_are_numbered_and_bounded() {
        let mut state = SharedState::default();
        let at = Utc::now();
        for _ in 0..(MAX_NOTICES + 3) {
            state.push_notice(NoticeKind::ChooseProject, at);
        }
        assert_eq!(state.notices.len(), MAX_NOTICES);
        assert_eq!(state.notices.last().unwrap().id, (MAX_NOTICES + 3) as u64, "ids keep increasing");
        assert!(state.notices.windows(2).all(|w| w[0].id < w[1].id), "oldest first");
    }

    #[test]
    fn notices_and_issues_read_as_sentences() {
        let at = Utc::now();
        let unavailable = Notice {
            id: 1,
            at,
            kind: NoticeKind::ProjectUnavailable { title: "Website".into(), now_tracking: None },
        };
        assert!(unavailable.text().contains("Choose a project"));
        let moved = Notice {
            id: 2,
            at,
            kind: NoticeKind::ProjectUnavailable { title: "Website".into(), now_tracking: Some("Mobile".into()) },
        };
        assert!(moved.text().ends_with("Now tracking Mobile."));
        let clock = UploadIssue::ClockAhead { next_attempt_at: at };
        assert!(clock.message().contains("clock is ahead"));
        assert_eq!(clock.next_attempt_at(), at);
        let inactive = UploadIssue::SubscriptionInactive { next_attempt_at: at, message: "x".into() };
        assert!(inactive.message().starts_with("Workspace subscription inactive"));
    }

    #[test]
    fn away_minutes_are_whole_and_never_negative() {
        let from = Utc::now();
        let prompt = |seconds: i64| AwayPrompt { from, to: from + chrono::Duration::seconds(seconds), project_title: "P".into() };
        assert_eq!(prompt(42 * 60 + 59).minutes(), 42);
        assert_eq!(prompt(-5).minutes(), 0);
    }
}
