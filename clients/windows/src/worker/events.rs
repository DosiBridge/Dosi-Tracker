//! What the control task and the uploader report.

use tokio::time::Instant;

use super::control::ControlEvent;
use super::uploader::{UploadCommand, UploadEvent};
use super::{Worker, RECONNECT_EVERY};

impl Worker {
    pub(super) fn on_control(&mut self, event: ControlEvent) {
        match event {
            ControlEvent::SignedIn { account, workspaces } => self.signed_in_to_server(account, workspaces),
            ControlEvent::SignInFailed(error) => self.sign_in_failed(&error),
            ControlEvent::MyWorkspaces(found) => self.my_workspaces_found(found),
            ControlEvent::WorkspaceLost(workspace, refusal) => self.workspace_lost(&workspace, refusal),
            ControlEvent::DeviceRevoked(reason) => self.device_revoked(&reason),
            ControlEvent::Projects(projects) => {
                if self.account.as_ref().is_some_and(|a| a.workspace_id.is_some()) && !self.expired {
                    self.apply_projects(projects);
                }
            }
            ControlEvent::ProjectsFailed(error) => {
                self.contacting = false;
                if !error.is_auth_rejected() {
                    self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
                }
            }
            ControlEvent::SignInExpired(reason) => self.expire(&reason),
            ControlEvent::Reachable(reachable) => self.set_reachable(reachable),
        }
    }

    pub(super) fn on_upload(&mut self, event: UploadEvent) {
        match event {
            // A pass of the previous account (a sign-out's final drain) may
            // report after another account signed in: not about this one.
            UploadEvent::SignInExpired { owner, .. }
            | UploadEvent::Projects { owner, .. }
            | UploadEvent::Parked { owner, .. }
            | UploadEvent::WorkspaceLost { owner, .. }
            | UploadEvent::DeviceRevoked { owner, .. }
                if !self.is_current_owner(&owner) =>
            {
                tracing::debug!("an upload event of the previous account ignored");
            }
            UploadEvent::SignInExpired { reason, .. } => self.expire(&reason),
            UploadEvent::DeviceRevoked { reason, .. } => self.device_revoked(&reason),
            UploadEvent::WorkspaceLost { workspace_id, refusal, .. } => self.workspace_lost(&workspace_id, refusal),
            UploadEvent::Projects { projects, .. } => {
                if !self.expired && self.account.as_ref().is_some_and(|a| a.workspace_id.is_some()) {
                    self.apply_projects(projects);
                }
            }
            UploadEvent::Uploaded(count) => {
                tracing::debug!(count, "blocks reached the server");
                self.totals_wanted = true;
            }
            // After a 403/409 the projects are fetched again (CONTRACT §2).
            UploadEvent::Parked { count, .. } => {
                tracing::info!(count, "blocks rejected by the server; checking the projects again");
                self.next_projects_refresh = Instant::now();
            }
        }
    }

    /// Whether `owner` is the account signed in now.
    pub(super) fn is_current_owner(&self, owner: &str) -> bool {
        self.account.as_ref().is_some_and(|account| account.owner() == owner)
    }

    /// Back online: retry the uploads now, and re-validate the projects.
    pub(super) fn set_reachable(&mut self, reachable: bool) {
        if reachable && !self.reachable {
            tracing::info!("the server is reachable again");
            self.uploader.send(UploadCommand::Now);
            self.next_projects_refresh = Instant::now();
        }
        if !reachable {
            self.contacting = false;
        }
        self.reachable = reachable;
    }
}
