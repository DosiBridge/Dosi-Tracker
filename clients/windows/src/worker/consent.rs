//! The user's local consent, changed from the window.

use crate::model::EndReason;

use super::session::session_for;
use super::Worker;

impl Worker {
    pub(super) fn update_consent(&mut self, consent: crate::config::LocalConsent) {
        tracing::info!(?consent, "local consent update requested by user");
        let now = self.now();
        let prior_consent = self.cfg.consent;

        // Step 1: Enforce withdrawn permissions BEFORE persistence can block.
        // Monotonic per-capability narrowing:
        // - Any capability withdrawn in `consent` becomes false immediately.
        // - No capability newly expanded in `consent` is granted yet.
        // - If `consent` is Revoked/Absent, all permissions are withdrawn immediately.
        let staged_consent = self.cfg.consent.narrow_to(&consent);
        self.cfg.consent = staged_consent;

        // Apply narrowed consent to session policy and active blocks
        if let Some(current_session) = self.session.as_ref() {
            let project_id = current_session.project_id().to_owned();
            if let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() {
                let refreshed = session_for(&project, &self.cfg);
                self.blocks.narrow_block_permissions(&refreshed.policy.perms);
                self.session = Some(refreshed);
            }
        }

        // Quiesce active capture path: clear any pending screenshots if screenshot permission was withdrawn
        let screenshot_permitted = self.session.as_ref().is_some_and(|s| s.policy.perms.screenshot);
        if !screenshot_permitted {
            self.screens.clear();
        }

        // Apply narrowed policy to native sensor drivers immediately BEFORE reconcile / record / persistence
        if self.should_track() {
            if let Some(session) = self.session.as_ref() {
                self.machine.apply_sensor_policy(true, Some(&session.policy.perms));
            }
            self.reconcile(now, EndReason::Interval);
        } else {
            self.machine.apply_sensor_policy(false, None);
            self.reconcile(now, EndReason::Pause);
        }

        // Step 2: Now invoke persistence (which may block on slow I/O or fail)
        match self.consent_store.save(&consent) {
            Ok(()) => {
                // Safely persisted! Now apply the full target consent (including any persisted expansions)
                self.cfg.consent = consent;
                self.consent_error = None;
                self.consent_unsynced = false;
                tracing::info!("consent successfully persisted to disk");

                if let Some(current_session) = self.session.as_ref() {
                    let project_id = current_session.project_id().to_owned();
                    if let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() {
                        let refreshed = session_for(&project, &self.cfg);
                        self.blocks.update_policy(refreshed.policy.clone());
                        self.session = Some(refreshed);
                    }
                } else {
                    self.choose_session(now);
                }

                if self.should_track() {
                    if let Some(session) = self.session.as_ref() {
                        self.machine.apply_sensor_policy(true, Some(&session.policy.perms));
                    }
                    self.reconcile(now, EndReason::Interval);
                } else {
                    self.machine.apply_sensor_policy(false, None);
                    self.reconcile(now, EndReason::Pause);
                }
            }
            Err(e) => {
                // Persistence failed!
                // Establish fail-closed effective state:
                // - Stop all sensors immediately
                // - Halt tracking
                // - Mark consent_unsynced = true to prevent resume
                // - Set displayed consent to fail-closed state (Absent if previously Absent, otherwise Revoked)
                // - Clear actionable error with restart durability warning
                let err_msg = format!(
                    "Failed to persist consent changes to disk: {e}. Tracking stopped (fail-closed) to prevent non-durable sensor operation. Changes/revocation are non-durable across app restart: prior on-disk consent remains and will reload on restart until disk save succeeds."
                );
                tracing::error!(%err_msg);
                self.consent_error = Some(err_msg);
                self.consent_unsynced = true;

                self.cfg.consent = match prior_consent {
                    crate::config::LocalConsent::Absent => crate::config::LocalConsent::Absent,
                    _ => crate::config::LocalConsent::Revoked,
                };
                if let Some(current_session) = self.session.as_ref() {
                    let project_id = current_session.project_id().to_owned();
                    if let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() {
                        let refreshed = session_for(&project, &self.cfg);
                        self.blocks.update_policy(refreshed.policy.clone());
                        self.session = Some(refreshed);
                    }
                }

                self.machine.apply_sensor_policy(false, None);
                self.reconcile(now, EndReason::Pause);
                self.screens.clear();
            }
        }

        self.publish();
    }
}
