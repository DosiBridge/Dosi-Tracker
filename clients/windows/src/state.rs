//! State shared between the UI thread and the background tracking worker.
//!
//! The worker owns all network/capture work; the UI only reads this snapshot and
//! pushes commands down a channel. Everything here is cheap to clone/lock so the
//! UI never blocks on the tracker.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

/// What the tracker is currently doing (drives the status pill + tray tooltip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerStatus {
    /// No credentials yet — the login view is showing.
    SignedOut,
    /// Signing in / resolving projects.
    Connecting,
    /// Actively tracking on a project.
    Tracking,
    /// User pressed pause; no captures are taken.
    Paused,
    /// Signed in but the backend could not be reached.
    Offline,
    /// The background worker died (panicked). Nothing is being captured — the
    /// user must restart the app. Surfaced loudly rather than silently pretending
    /// to track.
    Stopped,
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
        }
    }
}

/// A project the user may track against (mirrors the backend's my-projects DTO).
#[derive(Debug, Clone)]
pub struct ProjectOption {
    pub id: String,
    pub title: String,
}

/// A pause/resume the user asked for that the worker has not applied yet. The
/// window reflects it at once, so the buttons never feel unresponsive while
/// the worker finishes an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingAction {
    Pausing,
    Resuming,
}

/// Snapshot the UI renders. Updated by the worker, read by the UI each frame.
///
/// The clocks are published as instants rather than elapsed values, so the
/// window can animate timers every second without any help from the worker.
#[derive(Debug, Clone)]
pub struct SharedState {
    pub status: TrackerStatus,
    pub pending_action: Option<PendingAction>,
    pub display_name: String,
    pub projects: Vec<ProjectOption>,
    pub selected_project: Option<String>,
    /// Seconds tracked today / this week: the backend's totals, plus blocks
    /// recorded since they were fetched. The block in progress is added live.
    pub tracked_today_secs: f64,
    pub tracked_week_secs: f64,
    /// Today's average activity level (0-100), from the backend.
    pub last_productivity: u8,
    pub interval_minutes: u64,
    /// Which input kinds the project counts (the live view hides the rest).
    pub counts_keyboard: bool,
    pub counts_mouse: bool,
    /// While tracking: when this uninterrupted stretch began (sign-in,
    /// resume, project switch), when the unsaved block began, and when the
    /// next automatic snapshot is due.
    pub tracking_since: Option<DateTime<Utc>>,
    pub block_started_at: Option<DateTime<Utc>>,
    pub next_capture_at: Option<DateTime<Utc>>,
    /// While paused: since when, and when tracking resumes by itself (a
    /// timed pause) — `None` means until the user resumes.
    pub paused_since: Option<DateTime<Utc>>,
    pub resume_at: Option<DateTime<Utc>>,
    pub last_sync: Option<DateTime<Utc>>,
    pub pending_uploads: u32,
    /// Uploads the server permanently rejected. Kept visible so silently dropped
    /// data is never invisible to the user.
    pub rejected_uploads: u32,
    pub last_error: Option<String>,
    /// Set once a sign-in attempt fails so the login view can show it.
    pub login_error: Option<String>,
    pub signing_in: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            status: TrackerStatus::SignedOut,
            pending_action: None,
            display_name: String::new(),
            projects: Vec::new(),
            selected_project: None,
            tracked_today_secs: 0.0,
            tracked_week_secs: 0.0,
            last_productivity: 0,
            interval_minutes: 10,
            counts_keyboard: true,
            counts_mouse: true,
            tracking_since: None,
            block_started_at: None,
            next_capture_at: None,
            paused_since: None,
            resume_at: None,
            last_sync: None,
            pending_uploads: 0,
            rejected_uploads: 0,
            last_error: None,
            login_error: None,
            signing_in: false,
        }
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
#[derive(Debug, Clone)]
pub enum Command {
    SignIn { workspace: String, username: String, password: String },
    SignOut,
    SelectProject(String),
    /// Stop tracking. The work recorded so far is saved first. With a
    /// duration, tracking resumes by itself when it runs out; sent while
    /// already paused, it just changes when (or whether) that happens.
    Pause { resume_after: Option<std::time::Duration> },
    Resume,
    /// Capture + sync immediately instead of waiting for the next interval.
    /// While paused it only uploads already-queued rows and never captures.
    SyncNow,
    Shutdown,
}
