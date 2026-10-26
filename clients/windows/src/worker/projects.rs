//! The workspace's projects: which one is tracked, its refreshed policy,
//! and one that disappears.

use chrono::{DateTime, Utc};
use tokio::time::Instant;

use crate::credentials::ProjectCache;
use crate::model::{EndReason, Project};
use crate::state::{NoticeKind, ProjectOption};

use super::recording::Speed;
use super::session::{pick_project, session_for, Pick};
use super::{Worker, PROJECTS_REFRESH};

pub(super) const NO_PROJECTS: &str = "You are not a member of any active project.";

impl Worker {
    /// Without a session: pick the project to track (CONTRACT §2) — or ask.
    pub(super) fn choose_session(&mut self, now: DateTime<Utc>) {
        self.publish_projects();
        if self.session.is_some() {
            return;
        }
        let Some(projects) = self.projects.as_ref() else {
            return;
        };
        let remembered = self.saved.as_ref().and_then(|saved| saved.project_id.clone());
        match pick_project(projects, remembered.as_deref()) {
            Pick::Project(project) => {
                let project = project.clone();
                self.select(&project);
            }
            Pick::Choose => {
                self.clear_error(NO_PROJECTS);
                if !std::mem::replace(&mut self.choose_notified, true) {
                    self.state.update(|s| s.push_notice(NoticeKind::ChooseProject, now));
                }
            }
            Pick::NoProjects => {
                tracing::warn!("signed in, but not a member of any active project");
                self.state.update(|s| s.last_error = Some(NO_PROJECTS.into()));
            }
        }
    }

    /// Track `project` from the next block on, and remember it.
    pub(super) fn select(&mut self, project: &Project) {
        tracing::info!(project = %project.title, "tracking session resolved");
        self.session = Some(session_for(project, &self.cfg));
        self.choose_notified = false;
        self.clear_error(NO_PROJECTS);
        if let Some(saved) = self.saved.as_mut() {
            if saved.project_id.as_deref() != Some(project.id.as_str()) {
                saved.project_id = Some(project.id.clone());
                self.persist();
            }
        }
    }

    pub(super) fn publish_projects(&self) {
        let options: Vec<ProjectOption> = self
            .projects
            .iter()
            .flatten()
            .map(|p| ProjectOption { id: p.id.clone(), title: p.title.clone() })
            .collect();
        self.state.update(|s| s.projects = options);
    }

    /// The server's project list: saved for offline starts, the tracked
    /// project's refreshed policy applied from the next block on — or, if
    /// that project is gone, tracking stopped there.
    pub(super) fn apply_projects(&mut self, projects: Vec<Project>) {
        let now = self.now();
        self.contacting = false;
        self.set_reachable(true);
        self.next_projects_refresh = Instant::now() + PROJECTS_REFRESH;
        if let Some(saved) = self.saved.as_mut() {
            saved.projects = Some(ProjectCache { resolved_at: now, projects: projects.clone() });
        }
        self.persist();
        self.projects = Some(projects);

        if let Some(current) = self.session.as_ref().map(|s| s.project_id().to_owned()) {
            let listed = self.projects.iter().flatten().find(|p| p.id == current).cloned();
            match listed {
                Some(project) => {
                    let refreshed = session_for(&project, &self.cfg);
                    self.blocks.update_policy(refreshed.policy.clone());
                    self.session = Some(refreshed);
                }
                None => self.project_gone(now),
            }
        }
        self.choose_session(now);
        self.reconcile(now, EndReason::Project);
    }

    /// The tracked project was archived or the membership removed. What was
    /// tracked on it so far is kept (the server takes a block that started
    /// before the archiving); tracking stops, unless exactly one project is
    /// left to move to.
    pub(super) fn project_gone(&mut self, now: DateTime<Utc>) {
        let Some(gone) = self.session.take() else {
            return;
        };
        tracing::warn!(project = %gone.title, "the tracked project is no longer available");
        let finished = self.blocks.end(now, now, EndReason::Project);
        self.record(finished, Speed::Normal);
        if let Some(saved) = self.saved.as_mut() {
            saved.project_id = None;
        }
        self.persist();
        let next = match self.projects.as_deref() {
            Some([only]) => Some(only.title.clone()),
            _ => None,
        };
        // The notice already asks to choose when that is needed.
        self.choose_notified = true;
        self.state.update(|s| s.push_notice(NoticeKind::ProjectUnavailable { title: gone.title, now_tracking: next }, now));
    }

    pub(super) fn select_project(&mut self, project_id: &str, at: DateTime<Utc>) {
        if self.session.as_ref().is_some_and(|s| s.project_id() == project_id) {
            return;
        }
        // The saved list works offline too.
        let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() else {
            tracing::warn!(project_id, "cannot switch to a project that is not listed");
            return;
        };
        let now = self.now();
        // The work so far belongs to the project it was done on.
        let finished = self.blocks.end(at, now, EndReason::Project);
        self.record(finished, Speed::Normal);
        self.select(&project);
        self.reconcile(at, EndReason::Project);
    }
}
