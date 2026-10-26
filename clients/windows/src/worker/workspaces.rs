//! Choosing the workspace to track in, switching it, and losing it.

use chrono::{DateTime, Utc};
use tokio::time::Instant;

use crate::api::{ApiError, AuthError, Workspace, WorkspaceRefusal};
use crate::model::EndReason;
use crate::state::{CaptureFlags, NoticeKind, WorkspaceChoice};

use super::account::Account;
use super::control::ControlRequest;
use super::recording::Speed;
use super::uploader::UploadCommand;
use super::{Worker, RECONNECT_EVERY};

pub(super) const ONE_WORKSPACE: &str = "You aren't in any other workspace.";
pub const NO_WORKSPACE: &str =
    "You aren't in any workspace yet. Create one on the web dashboard or ask for an invite.";

pub const ALL_ARCHIVED: &str =
    "All of your workspaces are archived. Ask a workspace owner to restore one, or create a new one on the web dashboard.";

impl Worker {
    /// The server accepted the sign-in: pick the workspace to track in.
    pub(super) fn signed_in_to_server(&mut self, account: Account, workspaces: Result<Vec<Workspace>, ApiError>) {
        if !self.signing_in {
            return;
        }
        self.signing_in = false;
        let preselect = self.preselect.take();
        let workspaces = match workspaces {
            Ok(workspaces) => workspaces,
            Err(error) => {
                let message = match &error {
                    ApiError::Auth(e) => e.user_message(),
                    ApiError::Offline(_) => AuthError::Offline(String::new()).user_message(),
                    other => format!("Signed in, but your workspaces could not be loaded ({other})."),
                };
                return self.state.update(|s| {
                    s.signing_in = false;
                    s.login_error = Some(message);
                });
            }
        };
        // An archived workspace refuses every tracker call: never offered.
        let listed = !workspaces.is_empty();
        let workspaces: Vec<Workspace> = workspaces.into_iter().filter(|w| !w.is_archived).collect();
        if listed && workspaces.is_empty() {
            return self.state.update(|s| {
                s.signing_in = false;
                s.login_error = Some(ALL_ARCHIVED.to_owned());
            });
        }
        let preselected = preselect.as_deref().and_then(|wanted| {
            workspaces.iter().find(|w| w.id.eq_ignore_ascii_case(wanted)).or_else(|| workspaces.iter().find(|w| w.name.trim().eq_ignore_ascii_case(wanted)))
        });
        match (preselected, workspaces.as_slice()) {
            (_, []) => self.state.update(|s| {
                s.signing_in = false;
                s.login_error = Some(NO_WORKSPACE.to_owned());
            }),
            (Some(workspace), _) | (None, [workspace]) => {
                let account = account.in_workspace(&workspace.id, &workspace.name);
                self.state.update(|s| s.signing_in = false);
                self.signed_in(account);
            }
            (None, _) => {
                self.offer(&workspaces, None);
                self.choices = workspaces;
                self.pending = Some(account);
                self.state.update(|s| s.signing_in = false);
            }
        }
    }

    /// Show `workspaces` in the picker.
    pub(super) fn offer(&self, workspaces: &[Workspace], current: Option<&str>) {
        let choices: Vec<WorkspaceChoice> = workspaces
            .iter()
            .map(|w| WorkspaceChoice { name: w.name.clone(), role: w.role.clone(), is_current: current == Some(w.id.as_str()) })
            .collect();
        self.state.update(|s| s.workspace_choices = choices);
    }

    /// The workspace picked after signing in.
    pub(super) fn choose_workspace(&mut self, index: usize) {
        let (Some(workspace), Some(account)) = (self.choices.get(index).cloned(), self.pending.take()) else { return };
        self.clear_choices();
        self.signed_in(account.in_workspace(&workspace.id, &workspace.name));
    }

    pub(super) fn clear_choices(&mut self) {
        self.pending = None;
        self.choices.clear();
        self.state.update(|s| {
            s.workspace_choices.clear();
            s.finding_workspaces = false;
            s.workspace_note = None;
        });
    }

    /// "Switch workspace": list the workspaces of the signed-in user.
    pub(super) fn switch_workspace(&mut self) {
        if self.account.is_none() || self.expired {
            return;
        }
        self.clear_choices();
        self.state.update(|s| s.finding_workspaces = true);
        self.control.send(ControlRequest::MyWorkspaces);
    }

    pub(super) fn my_workspaces_found(&mut self, found: Result<Vec<Workspace>, String>) {
        self.state.update(|s| s.finding_workspaces = false);
        let Some(current) = self.account.as_ref().filter(|_| !self.expired).map(|a| a.workspace_id.clone()) else { return };
        let others = |items: &[Workspace]| items.iter().filter(|w| current.as_deref() != Some(w.id.as_str())).count();
        // Archived workspaces are never offered.
        let found = found.map(|items| items.into_iter().filter(|w| !w.is_archived).collect::<Vec<_>>());
        let items = match found {
            Err(message) => return self.state.update(|s| s.workspace_note = Some(message)),
            Ok(items) if items.is_empty() => return self.state.update(|s| s.workspace_note = Some(NO_WORKSPACE.to_owned())),
            Ok(items) if others(&items) == 0 => return self.state.update(|s| s.workspace_note = Some(ONE_WORKSPACE.to_owned())),
            Ok(items) => items,
        };
        self.offer(&items, current.as_deref());
        self.choices = items;
    }

    /// Track in `workspace_choices[index]` from now on: the block so far is
    /// queued in the workspace it was recorded in, and the project is chosen
    /// again. The sign-in (and its token) stays.
    pub(super) fn switch_to(&mut self, index: usize, at: DateTime<Utc>) {
        let Some(old) = self.account.clone().filter(|_| !self.expired) else { return };
        let Some(target) = self.choices.get(index).cloned() else { return };
        if old.workspace_id.as_deref() == Some(target.id.as_str()) {
            return;
        }
        let now = self.now();
        let finished = self.blocks.end(at.min(now), now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.clear_choices();
        self.use_workspace(old.in_workspace(&target.id, &target.name), now);
    }

    /// Carry on in `account`'s workspace: what belonged to the previous one
    /// (projects, the chosen project, the question about time away) is dropped.
    pub(super) fn use_workspace(&mut self, account: Account, now: DateTime<Utc>) {
        if let Some(saved) = self.saved.as_mut() {
            saved.workspace_id = account.workspace_id.clone();
            saved.workspace_name = account.workspace_name.clone();
            saved.project_id = None;
            saved.projects = None;
        }
        self.persist();
        self.projects = None;
        self.session = None;
        self.choose_notified = false;
        self.away = None;
        self.prompt = None;
        self.presence_sent = None;
        self.contacting = account.workspace_id.is_some();
        let (name, lost) = (account.workspace_name.clone(), account.workspace_id.is_none());
        self.state.update(|s| {
            s.workspace = if lost { String::new() } else { name };
            s.projects.clear();
            s.selected_project = None;
            s.tracked_today_secs = 0.0;
            s.tracked_week_secs = 0.0;
            s.week_day_secs = [0.0; 7];
            s.week_days_fetched = None;
            s.last_productivity = 0;
            s.capture = CaptureFlags::default();
            s.last_error = None;
            if !lost {
                s.workspace_lost = None;
            }
        });
        self.account = Some(account.clone());
        self.release_held(&account);
        self.control.send(ControlRequest::UseAccount(Some(account.clone())));
        self.uploader.send(UploadCommand::Account(Some(account.clone())));
        if account.workspace_id.is_some() {
            self.control.send(ControlRequest::RefreshProjects);
            self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
            self.totals_wanted = true;
        }
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
    }

    /// The server refused the workspace tracked in (access removed, the
    /// workspace archived, or it never arrived): tracking and heartbeats stop
    /// there, what was recorded is held in the queue, and another workspace
    /// is asked for (CONTRACT, terminal server errors).
    pub(super) fn workspace_lost(&mut self, workspace_id: &str, refusal: WorkspaceRefusal) {
        let Some(account) = self.account.clone().filter(|_| !self.expired) else { return };
        if account.workspace_id.as_deref() != Some(workspace_id) {
            return;
        }
        let now = self.now();
        let name = if account.workspace_name.trim().is_empty() { "this workspace".to_owned() } else { account.workspace_name.clone() };
        tracing::warn!(%workspace_id, %refusal, "the server refused the workspace; tracking stops there");
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        // Nothing more is sent there until it is used again.
        let note = crate::state::workspace_lost_text(&name, refusal);
        if let Err(e) = self.store.hold_workspace(&account.owner(), workspace_id, &note) {
            tracing::warn!(?e, "could not hold the workspace's queued blocks");
        }
        self.uploader.send(UploadCommand::RowsQueued);
        let mut account = account;
        account.workspace_id = None;
        self.use_workspace(account, now);
        self.state.update(|s| {
            s.workspace_lost = Some(name.clone());
            s.workspace_note = Some(note);
            s.finding_workspaces = true;
            s.push_notice(NoticeKind::WorkspaceLost { name, refusal }, now);
        });
        self.control.send(ControlRequest::MyWorkspaces);
    }

    /// The account tracks in its workspace again: rows held there after the
    /// server refused it are sent again (once; a refusal holds them again).
    pub(super) fn release_held(&self, account: &Account) {
        let Some(workspace) = account.workspace_id.as_deref() else { return };
        match self.store.release_workspace(&account.owner(), workspace) {
            Ok(released) if released > 0 => tracing::info!(released, %workspace, "held blocks of the workspace are sent again"),
            Err(e) => tracing::warn!(?e, "could not release the workspace's held blocks"),
            _ => {}
        }
    }
}
