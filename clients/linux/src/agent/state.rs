//! Agent snapshot shared with the IPC server, tray and GUI (`SharedState`,
//! `StateHandle` watch counter), ported from the Windows agent's `state.rs`.
//!
//! The worker owns all network/capture work; every other part only reads this
//! snapshot and pushes [`Command`]s down a channel. Everything here is cheap
//! to clone/lock so no reader ever blocks the worker.
//!
//! Linux deltas (design 5.1 and 10):
//! - Two more statuses: [`TrackerStatus::NoSession`] (signed in, but no
//!   attached X11/Wayland session) and [`TrackerStatus::DeviceRevoked`].
//! - The snapshot is serde-serializable for IPC (snake_case). Screenshot bytes
//!   are never serialized: only `last_screenshot_at` goes over the wire.
//! - Extras: per-capability states with the reason a capability is off
//!   ([`CapabilityState`]), the session, the visible indicators,
//!   `credentials_locked`, `manual_mode` and the daemon's identity.
//! - Counts and activity levels are `Option`: `None` means not measured.
//! - [`QueuedRow`] is a local DTO, so this module does not depend on the outbox.
//! - [`StateHandle`] carries a tokio `watch` counter bumped on every update, so
//!   the tray, IPC subscriptions and the GUI link wake up on changes.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::consent::LocalConsent;

/// What the tracker is currently doing (drives the status pill + tray tooltip).
/// Serialized in snake_case (`signed_out`, `no_session`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// The session is locked, inactive (VT switch, user switching) or the
    /// machine is going to sleep: nothing is recorded.
    Locked,
    /// Signed in, but no project is chosen: there are several and none was
    /// remembered, the remembered one is gone, or there are none at all.
    ChooseProject,
    /// The server no longer accepts the saved sign-in (refresh refused and the
    /// saved password missing or refused). Tracking stopped; the queue is kept
    /// and uploads once the password is entered again.
    SignInExpired,
    /// Affirmative local consent has not been granted, or was revoked.
    ConsentRequired,
    /// Linux: signed in, but no graphical (X11/Wayland) session is attached —
    /// a TTY, SSH, or before login. Sign-in, queue, sync and config still
    /// work; tracking never starts.
    NoSession,
    /// Linux: the workspace removed this device (403 `Tracker:DeviceRevoked`
    /// or the device status says so). Tracking and heartbeats stop there.
    DeviceRevoked,
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
            TrackerStatus::NoSession => "No graphical session",
            TrackerStatus::DeviceRevoked => "Device removed from the workspace",
        }
    }
}

/// A project the user may track against (mirrors the backend's my-projects DTO).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectOption {
    pub id: String,
    pub title: String,
}

/// What the current project captures — the server's policy combined with the
/// local consent, the local config and what this desktop can measure. Shown
/// read-only in the window; the live view also hides input counts the
/// project does not record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadState {
    /// Saved locally, waiting to upload.
    Queued,
    Uploaded,
    /// The server refused it; it is kept locally (parked) but never uploaded.
    Rejected,
}

/// A block recorded during this run, for the window's "recent activity" list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentBlock {
    /// The local queue row, so the upload result can be matched to it.
    pub row_id: i64,
    pub project: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Activity level (0-100); `None` when input was not measured.
    pub productivity: Option<u8>,
    /// `None` when not measured (consent off, Wayland, listener unhealthy).
    pub keyboard_hits: Option<u64>,
    pub mouse_clicks: Option<u64>,
    pub screenshot: bool,
    pub upload: UploadState,
}

/// How many recent blocks the window lists.
pub const RECENT_BLOCKS: usize = 12;

/// How many queue rows a snapshot carries.
pub const MAX_QUEUE_ROWS: usize = 20;

/// How many projects a snapshot carries.
pub const MAX_PROJECTS: usize = 1000;

/// The thumbnail of the latest screenshot, shown so people can see exactly
/// what was captured. Only `taken_at` is ever serialized; the bytes stay in
/// the daemon (`screenshot.latest` hands them out on request).
#[derive(Debug, PartialEq)]
pub struct LastScreenshot {
    pub taken_at: DateTime<Utc>,
    /// JPEG bytes (the small list thumbnail, not the full capture).
    pub jpeg: Vec<u8>,
}

/// A pause/resume the user asked for that the worker has not applied yet. The
/// window reflects it at once, so the buttons never feel unresponsive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingAction {
    Pausing,
    Resuming,
}

/// The time the user was away, waiting for "Keep as work" or "Discard"
/// ([`Command::ResolveAway`]). Discarding is the default: a question nobody
/// answers records nothing (CONTRACT §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn minutes(&self) -> i64 {
        (self.to - self.from).num_minutes().max(0)
    }
}

/// Why queued blocks are not reaching the server right now (CONTRACT §1,
/// "How agents treat upload responses"). The blocks stay queued; each kind
/// is retried on its own schedule, and "Sync now" retries at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UploadIssue {
    /// The server could not be reached or failed: retried with a growing
    /// pause (30 s up to 30 min).
    Retrying {
        next_attempt_at: DateTime<Utc>,
        reason: String,
    },
    /// HTTP 402: the workspace's subscription is inactive. Retried hourly.
    SubscriptionInactive {
        next_attempt_at: DateTime<Utc>,
        message: String,
    },
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
                "Workspace subscription inactive — your work is kept and uploads once it is renewed."
                    .to_owned()
            }
            UploadIssue::ClockAhead { .. } => {
                "Your computer clock is ahead — uploads wait until the server's time catches up."
                    .to_owned()
            }
        }
    }

    pub fn next_attempt_at(&self) -> DateTime<Utc> {
        match self {
            UploadIssue::Retrying {
                next_attempt_at, ..
            }
            | UploadIssue::SubscriptionInactive {
                next_attempt_at, ..
            }
            | UploadIssue::ClockAhead { next_attempt_at } => *next_attempt_at,
        }
    }
}

/// Something worth telling the user once (a desktop notification or toast).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// Increases with every notice, so each client shows each one once.
    pub id: u64,
    pub at: DateTime<Utc>,
    pub kind: NoticeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NoticeKind {
    /// Tracking started without the user doing anything (the agent started at
    /// login and resumed).
    TrackingStartedAutomatically { project_title: String },
    /// Signed in with several projects and none chosen: pick one to start.
    ChooseProject,
    /// The project being tracked is no longer available (archived, or the
    /// membership was removed). Tracking moved to `now_tracking` when it is
    /// the only project left, otherwise it stopped until one is chosen.
    ProjectUnavailable {
        title: String,
        now_tracking: Option<String>,
    },
    /// The local queue was damaged; it was set aside at `moved_to` and a fresh
    /// one started.
    QueueReset { moved_to: String },
    /// Old or excess queued blocks were dropped to keep the queue bounded.
    QueueTrimmed { dropped: u64 },
    /// The saved sign-in no longer works.
    SignInExpired,
    /// The account lost access to the workspace it tracked in.
    WorkspaceLost { name: String },
    /// Linux: the workspace removed this device; its unsent blocks are kept
    /// but not uploaded.
    DeviceRevoked { workspace: String },
    /// Linux: the project asks to record more than this computer's consent
    /// allows; nothing more is captured (design 6).
    PolicyRequestsConsent {
        project_title: String,
        capabilities: Vec<String>,
    },
    /// Linux: manual mode (no idle source) paused tracking after the
    /// configured maximum (design 5.2).
    ManualModePaused { hours: u32 },
    /// Queued blocks the server will never accept were parked (kept 14 days).
    BlocksNotAccepted { count: u32 },
    /// The server refused the screenshots of queued blocks; the blocks were
    /// sent without them.
    ScreenshotsRemoved { count: u32 },
}

impl Notice {
    /// One line for a notification.
    pub fn text(&self) -> String {
        match &self.kind {
            NoticeKind::TrackingStartedAutomatically { project_title } => {
                format!("Dosi Tracker started at login and is tracking {project_title}.")
            }
            NoticeKind::ChooseProject => {
                "Choose the project you are working on to start tracking.".to_owned()
            }
            NoticeKind::ProjectUnavailable {
                title,
                now_tracking: Some(next),
            } => {
                format!("{title} is no longer available. Now tracking {next}.")
            }
            NoticeKind::ProjectUnavailable {
                title,
                now_tracking: None,
            } => {
                format!("{title} is no longer available. Choose a project to continue tracking.")
            }
            NoticeKind::QueueReset { moved_to } => {
                format!(
                    "The local queue was damaged and was set aside ({moved_to}). Tracking continues."
                )
            }
            NoticeKind::QueueTrimmed { dropped } => {
                format!("{dropped} old queued blocks were removed to keep the local queue within its limits.")
            }
            NoticeKind::SignInExpired => {
                "Your sign-in has expired. Enter your password to continue tracking.".to_owned()
            }
            NoticeKind::WorkspaceLost { name } => {
                format!("You no longer have access to {name}. Choose another workspace.")
            }
            NoticeKind::DeviceRevoked { workspace } => format!(
                "This computer was removed from {workspace}. Tracking stopped there; choose another workspace or sign out."
            ),
            NoticeKind::PolicyRequestsConsent {
                project_title,
                capabilities,
            } => format!(
                "{project_title} asks to record {} - review in Settings.",
                capabilities.join(", ")
            ),
            NoticeKind::ManualModePaused { hours } => format!(
                "Tracking paused after {hours} hours: this desktop cannot tell whether you are still there. Resume when you are back."
            ),
            NoticeKind::BlocksNotAccepted { count } => format!(
                "{count} queued blocks were not accepted by the server; they are kept on this computer for 14 days."
            ),
            NoticeKind::ScreenshotsRemoved { count } => format!(
                "The server refused the screenshots of {count} blocks; the blocks were sent without them."
            ),
        }
    }
}

/// A workspace offered by the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceChoice {
    /// The workspace's id (a GUID), so clients can choose by id.
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// The user's role there ("Owner", "Admin", "Member"…).
    pub role: Option<String>,
    /// The workspace tracked in now.
    pub is_current: bool,
}

/// Why the last sign-in attempt failed, for clients that act on it (the CLI
/// maps it to its exit codes); `login_error` has the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginErrorKind {
    /// The server refused the email or password.
    Refused,
    /// The server could not be reached.
    Offline,
    /// The server answered with an error that is not about the sign-in.
    Server,
    /// The server address is invalid or not allowed (plain `http://`).
    InvalidServer,
    /// No server address was given and none is configured.
    NoServer,
    /// The account is in no workspace (or not in the one named).
    NoWorkspace,
    /// The workspace named is archived.
    ArchivedWorkspace,
    /// The saved sign-in is no longer accepted.
    Expired,
}

/// How many notices are kept for the clients to pick up.
pub const MAX_NOTICES: usize = 8;

/// Where a queued row stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    /// Waiting to upload.
    Queued,
    /// Parked: the server refused it (or it failed the local integrity
    /// check); kept for diagnostics, never retried.
    Parked,
}

/// One row of the queue as the clients list it. Never carries a payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedRow {
    pub id: i64,
    pub project_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: RowStatus,
    /// Parked: the server's reason. Queued: why the last attempt failed, if
    /// one did.
    pub reason: Option<String>,
    /// Still carries screenshot images.
    pub has_captures: bool,
}

/// What kind of desktop session the agent is attached to (design 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    X11,
    Wayland,
    /// A text console or SSH login: no graphical session.
    Tty,
    /// Nothing attached (before login, or the session was rejected).
    #[default]
    None,
}

impl SessionKind {
    /// Can tracking run in this session at all?
    pub fn is_graphical(self) -> bool {
        matches!(self, SessionKind::X11 | SessionKind::Wayland)
    }
}

/// The session as the snapshot shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub kind: SessionKind,
    /// `XDG_CURRENT_DESKTOP` (or logind's `Desktop`), e.g. `ubuntu:GNOME`.
    pub desktop: Option<String>,
    /// Where idle time comes from (`xi2`, `x11-screensaver`, `mutter`,
    /// `screensaver`, `logind`); `None` means manual mode.
    pub idle_source: Option<String>,
}

/// Which visible indicators exist (design invariant I5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorInfo {
    /// A StatusNotifierItem host shows the tray item.
    pub tray: bool,
    /// A notification server is running.
    pub notifications: bool,
}

/// The daemon serving this snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub version: String,
    pub pid: u32,
}

/// The state of one capability, first reason first (design 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    /// Recorded while tracking.
    On,
    /// The local user has not consented to it (the default).
    #[default]
    OffByYou,
    /// The project's policy does not allow it.
    OffByWorkspace,
    /// The local or admin configuration restricts it.
    OffByConfig,
    /// This desktop cannot provide it (Wayland input counts, for example).
    Unsupported,
    /// The desktop is waiting for the user to approve it (portal dialog).
    NeedsApproval,
    /// The user refused it in the desktop's permission dialog.
    Denied,
    /// It would be available, but failed for now.
    Unavailable,
    /// Allowed, but tracking is paused.
    Paused,
}

/// What the platform says about a capability, as the sensors report it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum PlatformAvailability {
    #[default]
    Available,
    Unsupported(String),
    NeedsApproval,
    Denied,
    Unavailable(String),
}

impl PlatformAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, PlatformAvailability::Available)
    }
}

/// One capability as the snapshot shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInfo {
    pub state: CapabilityState,
    /// Why it is not on, in words (`None` when on).
    pub reason: Option<String>,
    pub consented: bool,
    pub project_allows: bool,
    pub config_allows: bool,
    /// The sensor backing it (`xi2`, `x11`, `portal`, `procfs`), when one exists.
    pub backend: Option<String>,
}

impl CapabilityInfo {
    /// Combine the four sources and the pause into one state: consent first,
    /// then the workspace, then the config, then the platform, then the pause.
    pub fn evaluate(
        consented: bool,
        project_allows: bool,
        config_allows: bool,
        platform: &PlatformAvailability,
        paused: bool,
        backend: Option<&str>,
    ) -> Self {
        let (state, reason) = if !consented {
            (
                CapabilityState::OffByYou,
                Some("You have not allowed it on this computer.".to_owned()),
            )
        } else if !project_allows {
            (
                CapabilityState::OffByWorkspace,
                Some("The project does not record it.".to_owned()),
            )
        } else if !config_allows {
            (
                CapabilityState::OffByConfig,
                Some("The configuration on this computer turns it off.".to_owned()),
            )
        } else {
            match platform {
                PlatformAvailability::Unsupported(why) => {
                    (CapabilityState::Unsupported, Some(why.clone()))
                }
                PlatformAvailability::NeedsApproval => (
                    CapabilityState::NeedsApproval,
                    Some("The desktop is asking for permission.".to_owned()),
                ),
                PlatformAvailability::Denied => (
                    CapabilityState::Denied,
                    Some("Permission was refused in the desktop's dialog.".to_owned()),
                ),
                PlatformAvailability::Unavailable(why) => {
                    (CapabilityState::Unavailable, Some(why.clone()))
                }
                PlatformAvailability::Available if paused => (
                    CapabilityState::Paused,
                    Some("Tracking is paused.".to_owned()),
                ),
                PlatformAvailability::Available => (CapabilityState::On, None),
            }
        };
        Self {
            state,
            reason,
            consented,
            project_allows,
            config_allows,
            backend: backend.map(str::to_owned),
        }
    }
}

/// Every capability of the capture matrix (design 7). The webcam is listed
/// so clients can say it is not available on Linux in this version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub keyboard: CapabilityInfo,
    pub mouse: CapabilityInfo,
    pub active_window: CapabilityInfo,
    pub running_programs: CapabilityInfo,
    pub screenshot: CapabilityInfo,
    pub webcam: CapabilityInfo,
}

/// Snapshot the clients render. Updated by the worker, read by the IPC
/// server, the tray and (through IPC) the GUI.
///
/// The clocks are published as instants rather than elapsed values, so the
/// clients can animate timers every second without any help from the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedState {
    pub status: TrackerStatus,
    pub pending_action: Option<PendingAction>,
    pub display_name: String,
    /// The name of the workspace tracked in.
    pub workspace: String,
    /// The server this account talks to.
    pub server_url: String,
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
    /// not used (the clients show the live total instead).
    pub week_day_secs: [f64; 7],
    /// Local midnight of the day `week_day_secs` was fetched on, and when —
    /// so past days are fetched again once a day changes or they get stale.
    pub week_days_fetched: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Today's average activity level (0-100), from the backend; `None` when
    /// nothing today was measured.
    pub last_productivity: Option<u8>,
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
    /// The latest thumbnail. Serialized as `last_screenshot_at` (the time
    /// only); the bytes never leave the daemon in a snapshot.
    #[serde(
        rename = "last_screenshot_at",
        serialize_with = "screenshot_time",
        deserialize_with = "screenshot_from_time",
        default
    )]
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
    /// Uploads the server permanently rejected (parked). Kept visible so
    /// silently dropped data is never invisible to the user.
    pub rejected_uploads: u32,
    /// The newest queued and parked rows with the server's reason for each
    /// parked one (and why the last attempt of a queued one failed).
    pub queue_rows: Vec<QueuedRow>,
    /// Why uploads are held back right now, if they are.
    pub upload_issue: Option<UploadIssue>,
    /// Minutes this computer's clock is off from the server's, once that
    /// exceeds 2 minutes: positive = behind, negative = ahead. Timestamps are
    /// never rewritten; the clients warn.
    pub clock_skew_minutes: Option<i64>,
    pub last_error: Option<String>,
    /// Set once a sign-in attempt fails so the login view can show it.
    pub login_error: Option<String>,
    /// What kind of failure `login_error` is.
    #[serde(default)]
    pub login_error_kind: Option<LoginErrorKind>,
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
    pub consent: LocalConsent,
    /// Persistence error from saving consent to disk (if any).
    pub consent_error: Option<String>,
    /// Whether in-memory consent differs from on-disk consent due to a write failure.
    pub consent_unsynced: bool,
    /// Linux: every capability with its state and the reason it is off.
    pub capabilities: Capabilities,
    /// Linux: the attached desktop session.
    pub session: SessionSummary,
    /// Linux: which visible indicators exist.
    pub indicator: IndicatorInfo,
    /// Linux: the keyring is locked; uploads and heartbeats wait for an
    /// explicit unlock.
    pub credentials_locked: bool,
    /// Linux: the session has no idle source, so tracking never starts or
    /// resumes by itself (design 5.2).
    pub manual_mode: bool,
    /// Linux: the daemon serving this snapshot.
    pub daemon: Option<DaemonInfo>,
    /// How many [`Command`]s the worker has handled. Never serialized: the
    /// IPC server compares it with the sequence number of a command it sent
    /// to know when that command was applied.
    #[serde(skip)]
    pub commands_handled: u64,
}

fn screenshot_time<S: Serializer>(
    value: &Option<Arc<LastScreenshot>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .as_ref()
        .map(|shot| shot.taken_at)
        .serialize(serializer)
}

fn screenshot_from_time<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Arc<LastScreenshot>>, D::Error> {
    Ok(
        Option::<DateTime<Utc>>::deserialize(deserializer)?.map(|taken_at| {
            Arc::new(LastScreenshot {
                taken_at,
                jpeg: Vec::new(),
            })
        }),
    )
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            status: TrackerStatus::SignedOut,
            pending_action: None,
            display_name: String::new(),
            workspace: String::new(),
            server_url: String::new(),
            server_reachable: true,
            projects: Vec::new(),
            selected_project: None,
            tracked_today_secs: 0.0,
            tracked_week_secs: 0.0,
            week_day_secs: [0.0; 7],
            week_days_fetched: None,
            last_productivity: None,
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
            login_error_kind: None,
            signing_in: false,
            workspace_choices: Vec::new(),
            finding_workspaces: false,
            workspace_note: None,
            workspace_lost: None,
            notices: Vec::new(),
            consent: LocalConsent::Absent,
            consent_error: None,
            consent_unsynced: false,
            capabilities: Capabilities::default(),
            session: SessionSummary::default(),
            indicator: IndicatorInfo::default(),
            credentials_locked: false,
            manual_mode: false,
            daemon: None,
            commands_handled: 0,
        }
    }
}

impl SharedState {
    /// The project being tracked, if one is selected and still listed.
    pub fn current_project(&self) -> Option<&ProjectOption> {
        let id = self.selected_project.as_deref()?;
        self.projects.iter().find(|p| p.id == id)
    }

    /// Queue a notice for the clients, keeping the newest [`MAX_NOTICES`].
    pub fn push_notice(&mut self, kind: NoticeKind, at: DateTime<Utc>) {
        let id = self.notices.last().map_or(1, |last| last.id + 1);
        self.notices.push(Notice { id, at, kind });
        let excess = self.notices.len().saturating_sub(MAX_NOTICES);
        self.notices.drain(..excess);
    }

    /// The snapshot as it goes over IPC: lists cut to their bounds (12 recent
    /// blocks, 20 queue rows, 8 notices, 1000 projects).
    pub fn bounded(mut self) -> Self {
        self.recent_blocks.truncate(RECENT_BLOCKS);
        self.queue_rows.truncate(MAX_QUEUE_ROWS);
        let excess = self.notices.len().saturating_sub(MAX_NOTICES);
        self.notices.drain(..excess);
        self.projects.truncate(MAX_PROJECTS);
        self
    }
}

/// Cheap shared handle to the snapshot, with a change counter.
///
/// Every [`StateHandle::update`] bumps the counter, so a subscriber
/// ([`StateHandle::subscribe`]) wakes up after each change and reads a fresh
/// [`StateHandle::snapshot`]. Bursts of updates coalesce naturally: a
/// subscriber only sees the latest value.
#[derive(Clone)]
pub struct StateHandle {
    state: Arc<Mutex<SharedState>>,
    changes: Arc<watch::Sender<u64>>,
}

impl Default for StateHandle {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(SharedState::default())),
            changes: Arc::new(watch::Sender::new(0)),
        }
    }
}

impl std::fmt::Debug for StateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateHandle")
            .field("version", &self.version())
            .finish()
    }
}

impl StateHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a clone of the current snapshot.
    ///
    /// A poisoned lock is recovered rather than propagated: one panicking thread
    /// must not cascade into killing every reader on every subsequent read.
    pub fn snapshot(&self) -> SharedState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Mutate the snapshot in place, then tell the subscribers.
    pub fn update(&self, f: impl FnOnce(&mut SharedState)) {
        {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut guard);
        }
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// The change counter: increases with every update.
    pub fn version(&self) -> u64 {
        *self.changes.borrow()
    }

    /// Wake up on every change (`changed().await`, or `has_changed()`).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
}

/// A password in transit (sign-in): zeroized when dropped, redacted in
/// `Debug`, serialized as a plain string only because the IPC request that
/// carries it needs it.
#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Secret(Zeroizing::new(String::deserialize(deserializer)?)))
    }
}

/// Commands the clients (IPC, tray, notifications) send to the worker.
///
/// Every command that ends the block in progress carries `at`, the moment the
/// user acted: the block ends exactly there, however long the worker takes to
/// get to it (CONTRACT §4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Sign in, then pick the workspace: the one named by `workspace` (a
    /// name or id), the only one, or one of [`SharedState::workspace_choices`].
    /// `server` is the address typed at sign-in; `None` keeps the one saved
    /// with this account (or the configured one).
    SignIn {
        workspace: Option<String>,
        username: String,
        password: Secret,
        server: Option<String>,
        /// Keep the password in the keyring; `None` follows
        /// `credentials.remember_password`.
        #[serde(default)]
        remember_password: Option<bool>,
    },
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
    SelectProject {
        project_id: String,
        at: DateTime<Utc>,
    },
    /// Stop tracking. The work recorded so far is saved first (ended by
    /// `pause`, or `break` for a timed one). With a duration, tracking resumes
    /// by itself when it runs out (once the user is at the computer); sent
    /// while already paused, it just changes when (or whether) that happens.
    /// The pause survives a restart.
    Pause {
        resume_after: Option<std::time::Duration>,
        at: DateTime<Utc>,
    },
    /// End a pause: a new block starts at `at`.
    Resume { at: DateTime<Utc> },
    /// Save the block in progress now and upload — or, while paused or with a
    /// block under a minute old, only upload what is already queued. Uploads
    /// are retried at once, whatever their back-off.
    SyncNow { at: DateTime<Utc> },
    /// Change the task note. Sent when the user commits the text, not on
    /// every keystroke: a change ends the block in progress (by `task`).
    SetTaskNote { note: String, at: DateTime<Utc> },
    /// Answer the [`AwayPrompt`]: keep the time away as work (recorded as idle
    /// blocks, CONTRACT §4) or discard it.
    ResolveAway { keep: bool },
    /// Quit: save the block in progress (ended by `quit`), tell the server,
    /// upload what fits in the grace period, and stop.
    Shutdown { at: DateTime<Utc> },
    /// Update affirmative local user consent (grant, decline, revoke, modify).
    UpdateConsent { consent: LocalConsent },
    /// Linux: the keyring was unlocked (after its own prompt): read the
    /// saved sign-in's secrets again.
    ReloadCredentials,
    /// Linux: ask the screenshot portal again after the user refused it.
    RetryCapture,
    /// Linux: fetch the project list (and each project's policy) now.
    RefreshProjects,
    /// Linux: delete the signed-in account's queued rows of `which` kind
    /// (confirmed by the user).
    PurgeQueue { which: PurgeQueue },
}

/// Which queued rows [`Command::PurgeQueue`] deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeQueue {
    /// Waiting to upload.
    Pending,
    /// Refused by the server and kept for diagnostics.
    Parked,
    All,
}

/// Requests from outside the clients (a second GUI launch, the installer,
/// the session ending) that the GUI acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSignal {
    /// Bring the window to the front (the GUI was launched again).
    Show,
    /// Quit gracefully.
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
        assert_eq!(
            state.notices.last().unwrap().id,
            (MAX_NOTICES + 3) as u64,
            "ids keep increasing"
        );
        assert!(
            state.notices.windows(2).all(|w| w[0].id < w[1].id),
            "oldest first"
        );
    }

    #[test]
    fn notices_and_issues_read_as_sentences() {
        let at = Utc::now();
        let unavailable = Notice {
            id: 1,
            at,
            kind: NoticeKind::ProjectUnavailable {
                title: "Website".into(),
                now_tracking: None,
            },
        };
        assert!(unavailable.text().contains("Choose a project"));
        let moved = Notice {
            id: 2,
            at,
            kind: NoticeKind::ProjectUnavailable {
                title: "Website".into(),
                now_tracking: Some("Mobile".into()),
            },
        };
        assert!(moved.text().ends_with("Now tracking Mobile."));
        let clock = UploadIssue::ClockAhead {
            next_attempt_at: at,
        };
        assert!(clock.message().contains("clock is ahead"));
        assert_eq!(clock.next_attempt_at(), at);
        let inactive = UploadIssue::SubscriptionInactive {
            next_attempt_at: at,
            message: "x".into(),
        };
        assert!(inactive
            .message()
            .starts_with("Workspace subscription inactive"));
    }

    #[test]
    fn away_minutes_are_whole_and_never_negative() {
        let from = Utc::now();
        let prompt = |seconds: i64| AwayPrompt {
            from,
            to: from + chrono::Duration::seconds(seconds),
            project_title: "P".into(),
        };
        assert_eq!(prompt(42 * 60 + 59).minutes(), 42);
        assert_eq!(prompt(-5).minutes(), 0);
    }

    #[test]
    fn statuses_go_over_ipc_in_snake_case_with_linux_labels() {
        let wire = |status: TrackerStatus| serde_json::to_value(status).unwrap();
        assert_eq!(wire(TrackerStatus::SignedOut), "signed_out");
        assert_eq!(wire(TrackerStatus::SignInExpired), "sign_in_expired");
        assert_eq!(wire(TrackerStatus::ChooseProject), "choose_project");
        assert_eq!(wire(TrackerStatus::ConsentRequired), "consent_required");
        assert_eq!(wire(TrackerStatus::NoSession), "no_session");
        assert_eq!(wire(TrackerStatus::DeviceRevoked), "device_revoked");
        assert_eq!(TrackerStatus::NoSession.label(), "No graphical session");
        assert!(TrackerStatus::DeviceRevoked.label().contains("removed"));
        let back: TrackerStatus = serde_json::from_value("no_session".into()).unwrap();
        assert_eq!(back, TrackerStatus::NoSession);
    }

    #[test]
    fn the_snapshot_never_serializes_screenshot_bytes() {
        let taken_at = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let state = SharedState {
            last_screenshot: Some(Arc::new(LastScreenshot {
                taken_at,
                jpeg: vec![0xff, 0xd8, 0xff, 0xe0],
            })),
            ..SharedState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["last_screenshot_at"], "2026-10-05T10:00:00Z");
        assert!(json.get("last_screenshot").is_none());
        assert!(
            !json.to_string().contains("jpeg"),
            "no byte field in {json}"
        );
        let back: SharedState = serde_json::from_value(json).unwrap();
        let shot = back.last_screenshot.unwrap();
        assert_eq!(shot.taken_at, taken_at);
        assert!(
            shot.jpeg.is_empty(),
            "bytes never come back from a snapshot"
        );

        let none = serde_json::to_value(SharedState::default()).unwrap();
        assert!(none["last_screenshot_at"].is_null());
    }

    #[test]
    fn unmeasured_counts_stay_null_in_the_snapshot() {
        let at = Utc::now();
        let block = RecentBlock {
            row_id: 1,
            project: "P".into(),
            started_at: at,
            ended_at: at,
            productivity: None,
            keyboard_hits: None,
            mouse_clicks: Some(0),
            screenshot: false,
            upload: UploadState::Queued,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert!(json["keyboard_hits"].is_null());
        assert!(json["productivity"].is_null());
        assert_eq!(json["mouse_clicks"], 0, "measured zero stays zero");
    }

    #[test]
    fn the_ipc_snapshot_is_bounded() {
        let at = Utc::now();
        let state = SharedState {
            projects: (0..1_200)
                .map(|i| ProjectOption {
                    id: format!("p{i}"),
                    title: format!("P{i}"),
                })
                .collect(),
            queue_rows: (0..30)
                .map(|id| QueuedRow {
                    id,
                    project_id: None,
                    started_at: None,
                    ended_at: None,
                    status: RowStatus::Queued,
                    reason: None,
                    has_captures: false,
                })
                .collect(),
            recent_blocks: (0..20)
                .map(|row_id| RecentBlock {
                    row_id,
                    project: "P".into(),
                    started_at: at,
                    ended_at: at,
                    productivity: None,
                    keyboard_hits: None,
                    mouse_clicks: None,
                    screenshot: false,
                    upload: UploadState::Uploaded,
                })
                .collect(),
            ..SharedState::default()
        };
        let bounded = state.bounded();
        assert_eq!(bounded.projects.len(), MAX_PROJECTS);
        assert_eq!(bounded.queue_rows.len(), MAX_QUEUE_ROWS);
        assert_eq!(bounded.recent_blocks.len(), RECENT_BLOCKS);
        assert_eq!(bounded.recent_blocks[0].row_id, 0, "newest first is kept");
    }

    #[test]
    fn every_update_bumps_the_change_counter() {
        let handle = StateHandle::new();
        let mut changes = handle.subscribe();
        assert_eq!(handle.version(), 0);
        assert!(!changes.has_changed().unwrap());
        handle.update(|s| s.status = TrackerStatus::Tracking);
        handle.update(|s| s.task_note = "Invoices".into());
        assert_eq!(handle.version(), 2);
        assert!(changes.has_changed().unwrap(), "subscribers are woken");
        assert_eq!(*changes.borrow_and_update(), 2, "bursts coalesce");
        assert!(!changes.has_changed().unwrap());
        let clone = handle.clone();
        clone.update(|s| s.manual_mode = true);
        assert!(handle.snapshot().manual_mode, "clones share the snapshot");
        assert!(changes.has_changed().unwrap());
    }

    #[test]
    fn commands_round_trip_and_never_print_the_password() {
        let sign_in = Command::SignIn {
            workspace: Some("Acme".into()),
            username: "ayesha@acme.test".into(),
            password: Secret::new("canary-pw-4711"),
            server: None,
            remember_password: None,
        };
        let debug = format!("{sign_in:?}");
        assert!(!debug.contains("canary-pw-4711"), "{debug}");
        assert!(debug.contains("<redacted>"));
        let json = serde_json::to_value(&sign_in).unwrap();
        assert_eq!(json["command"], "sign_in");
        let back: Command = serde_json::from_value(json).unwrap();
        match back {
            Command::SignIn { password, .. } => assert_eq!(password.expose(), "canary-pw-4711"),
            other => panic!("{other:?}"),
        }

        let at = Utc::now();
        let pause = Command::Pause {
            resume_after: Some(std::time::Duration::from_secs(900)),
            at,
        };
        let back: Command = serde_json::from_str(&serde_json::to_string(&pause).unwrap()).unwrap();
        match back {
            Command::Pause {
                resume_after,
                at: back_at,
            } => {
                assert_eq!(resume_after, Some(std::time::Duration::from_secs(900)));
                assert_eq!(back_at, at);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn capability_states_name_the_first_reason_it_is_off() {
        let ok = PlatformAvailability::Available;
        let wayland = PlatformAvailability::Unsupported("Not available on Wayland.".into());
        let eval = |c, p, k, platform: &PlatformAvailability, paused| {
            CapabilityInfo::evaluate(c, p, k, platform, paused, Some("xi2")).state
        };
        assert_eq!(eval(true, true, true, &ok, false), CapabilityState::On);
        assert_eq!(
            eval(false, true, true, &ok, false),
            CapabilityState::OffByYou
        );
        assert_eq!(
            eval(true, false, true, &ok, false),
            CapabilityState::OffByWorkspace
        );
        assert_eq!(
            eval(true, true, false, &ok, false),
            CapabilityState::OffByConfig
        );
        assert_eq!(
            eval(true, true, true, &wayland, false),
            CapabilityState::Unsupported
        );
        assert_eq!(
            eval(true, true, true, &PlatformAvailability::Denied, false),
            CapabilityState::Denied
        );
        assert_eq!(
            eval(true, true, true, &PlatformAvailability::NeedsApproval, true),
            CapabilityState::NeedsApproval
        );
        assert_eq!(eval(true, true, true, &ok, true), CapabilityState::Paused);
        let info = CapabilityInfo::evaluate(true, true, true, &wayland, false, None);
        assert_eq!(info.reason.as_deref(), Some("Not available on Wayland."));
        let on = CapabilityInfo::evaluate(true, true, true, &ok, false, Some("x11"));
        assert_eq!((on.reason, on.backend.as_deref()), (None, Some("x11")));
        assert_eq!(
            serde_json::to_value(CapabilityState::OffByWorkspace).unwrap(),
            "off_by_workspace"
        );
    }
}
