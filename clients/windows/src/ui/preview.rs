//! Debug builds only: scenes on top of the UI preview's demo data, so every
//! state the window can be in is reviewed without a backend or waiting for
//! it to happen. `DOSI_UI_SCENE` picks one; it only applies together with
//! `DOSI_UI_PREVIEW` (which runs without a worker), so a real session's
//! state is never touched.

use chrono::{Duration, Utc};

use crate::state::{AwayPrompt, NoticeKind, ProjectOption, StateHandle, TrackerStatus, UploadIssue, UploadState};
use crate::storage::{QueuedRow, RowStatus};

/// What a scene changes besides the shared state.
#[derive(Debug, Default)]
pub struct Scene {
    /// Show the window as if the input hooks were down.
    pub input_down: bool,
}

/// Apply `DOSI_UI_SCENE` (when previewing).
pub fn apply(state: &StateHandle) -> Scene {
    if std::env::var_os("DOSI_UI_PREVIEW").is_none() {
        return Scene::default();
    }
    match std::env::var("DOSI_UI_SCENE") {
        Ok(name) if !name.trim().is_empty() => scene(state, name.trim()),
        _ => Scene::default(),
    }
}

fn projects(titles: &[&str]) -> Vec<ProjectOption> {
    titles
        .iter()
        .enumerate()
        .map(|(i, title)| ProjectOption { id: format!("p{}", i + 1), title: (*title).to_owned() })
        .collect()
}

fn scene(state: &StateHandle, name: &str) -> Scene {
    let now = Utc::now();
    let mut result = Scene::default();
    state.update(|s| match name {
        // Back from 42 minutes away: keep it, or not?
        "away" => {
            s.away_prompt = Some(AwayPrompt {
                from: now - Duration::minutes(49),
                to: now - Duration::minutes(7),
                project_title: "Website Redesign".into(),
            });
        }
        // Several projects, none remembered.
        "choose" | "choose-many" => {
            s.status = TrackerStatus::ChooseProject;
            s.selected_project = None;
            s.tracking_since = None;
            s.block_started_at = None;
            s.unsaved_since = None;
            s.next_capture_at = None;
            let many = ["Website Redesign", "Mobile App", "Acme — Billing & Invoices", "Internal tools", "Zeta onboarding",
                "Marketing site", "Data warehouse migration for the finance department", "Support rota", "Hiring"];
            let count = if name == "choose" { 4 } else { many.len() };
            s.projects = projects(&many[..count]);
            s.push_notice(NoticeKind::ChooseProject, now);
        }
        // Signed in, but a member of no active project.
        "no-projects" => {
            s.status = TrackerStatus::ChooseProject;
            s.selected_project = None;
            s.projects.clear();
            s.tracking_since = None;
            s.block_started_at = None;
            s.unsaved_since = None;
            s.last_error = Some("You are not a member of any active project.".into());
        }
        // Everything that can hold uploads back at once.
        "alerts" => {
            result.input_down = true;
            s.clock_skew_minutes = Some(-7);
            s.upload_issue = Some(UploadIssue::SubscriptionInactive {
                next_attempt_at: now + Duration::minutes(54),
                message: "The workspace's trial has ended.".into(),
            });
            s.rejected_uploads = 1;
            s.pending_uploads = 6;
        }
        // The server is out of reach; tracking goes on.
        "offline" => {
            s.server_reachable = false;
            s.pending_uploads = 3;
            s.upload_issue = Some(UploadIssue::Retrying {
                next_attempt_at: now + Duration::minutes(4),
                reason: "error sending request for url (https://tracker.acme.com/api/app/activity): client error \
                         (Connect): tcp connect error: No connection could be made because the target machine \
                         actively refused it. (os error 10061)"
                    .into(),
            });
        }
        // The local queue cannot be opened.
        "stopped" => {
            s.status = TrackerStatus::Stopped;
            s.tracking_since = None;
            s.block_started_at = None;
            s.unsaved_since = None;
            s.last_error = Some(
                "The local queue cannot be opened (database is locked). Nothing is recorded until it can — retrying every minute."
                    .into(),
            );
        }
        // The saved sign-in stopped working.
        "expired" => {
            s.status = TrackerStatus::SignInExpired;
            s.display_name = "ayesha@acme.com".into();
            s.workspace = "acme".into();
            s.server_url = "https://tracker.acme.com".into();
            s.pending_uploads = 4;
            s.login_error = Some(
                "Your sign-in has expired (Invalid username or password!). Enter your password to continue — your \
                 tracked time is kept."
                    .into(),
            );
        }
        // A failed sign-in (use with DOSI_UI_PREVIEW=signed-out).
        "signin-error" => {
            s.server_url = "https://tracker.acme.com".into();
            s.login_error = Some(crate::api::AuthError::Offline("tcp connect error".into()).user_message());
        }
        "signin-refused" => {
            s.server_url = "https://tracker.acme.com".into();
            s.login_error = Some("Invalid username or password!".into());
        }
        // Blocks the server refused, this session and earlier.
        "queue" => {
            if let Some(block) = s.recent_blocks.get_mut(1) {
                block.upload = UploadState::Rejected;
            }
            let row = |id: i64, ago: i64, status: RowStatus, reason: Option<&str>| QueuedRow {
                id,
                project_id: Some("p1".into()),
                started_at: Some(now - Duration::minutes(ago + 10)),
                ended_at: Some(now - Duration::minutes(ago)),
                status,
                reason: reason.map(str::to_owned),
                has_captures: status == RowStatus::Queued,
            };
            s.queue_rows = vec![
                row(0, 3, RowStatus::Queued, Some("HTTP 503")),
                row(1, 13, RowStatus::Rejected, Some("This block overlaps another one by 4 minutes.")),
                row(100, 26 * 60, RowStatus::Queued, None),
                row(
                    101,
                    3 * 24 * 60,
                    RowStatus::Rejected,
                    Some("The project was archived before this block started."),
                ),
            ];
            s.rejected_uploads = 2;
            s.pending_uploads = 2;
        }
        // Blurred screenshots, idle detection off (Settings).
        "privacy" => {
            s.capture.blur_screenshots = true;
            s.idle_threshold_minutes = None;
        }
        // What the worker tells the user once.
        "notices" => {
            s.push_notice(NoticeKind::TrackingStartedAutomatically { project_title: "Website Redesign".into() }, now);
            s.push_notice(NoticeKind::ProjectUnavailable { title: "Zeta onboarding".into(), now_tracking: None }, now);
        }
        other => tracing::warn!(scene = other, "unknown DOSI_UI_SCENE"),
    });
    result
}
