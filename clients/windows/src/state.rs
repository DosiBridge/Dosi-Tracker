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

/// Snapshot the UI renders. Updated by the worker, read by the UI each frame.
#[derive(Debug, Clone)]
pub struct SharedState {
    pub status: TrackerStatus,
    pub display_name: String,
    pub projects: Vec<ProjectOption>,
    pub selected_project: Option<String>,
    /// Minutes tracked today / this week, as reported by the backend.
    pub tracked_today_minutes: u64,
    pub tracked_week_minutes: u64,
    /// Activity level of the most recent interval (0-100).
    pub last_productivity: u8,
    pub interval_minutes: u64,
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
            display_name: String::new(),
            projects: Vec::new(),
            selected_project: None,
            tracked_today_minutes: 0,
            tracked_week_minutes: 0,
            last_productivity: 0,
            interval_minutes: 10,
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
    Pause,
    Resume,
    /// Capture + sync immediately instead of waiting for the next interval.
    SyncNow,
    Shutdown,
}
