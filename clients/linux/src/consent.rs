//! Default-inactive local consent core for Dosi-Tracker Linux.
//!
//! Enforces:
//! - Default-inactive state: absent, denied, or revoked consent cannot start.
//! - Strict default-deny schema: omitted or partial fields deserialize to false.
//! - Fail-closed handling on malformed persisted files or save errors.
//! - Immediate in-memory revocation on narrowing, even if disk persistence fails.
//! - Zero automatic tracking on load.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Granular capability permissions consented to by the local user.
/// Strict default-deny: all capabilities default to false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsentedPermissions {
    pub screenshot: bool,
    pub webcam: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
    pub blur_screenshots: bool,
}

impl Default for ConsentedPermissions {
    fn default() -> Self {
        Self {
            screenshot: false,
            webcam: false,
            keyboard: false,
            mouse: false,
            active_window: false,
            running_programs: false,
            blur_screenshots: true,
        }
    }
}

impl ConsentedPermissions {
    /// Return permissions with every capability disabled.
    pub fn all_disabled() -> Self {
        Self {
            screenshot: false,
            webcam: false,
            keyboard: false,
            mouse: false,
            active_window: false,
            running_programs: false,
            blur_screenshots: true,
        }
    }

    /// Check if at least one active collection capability is granted.
    pub fn has_any_active_capability(&self) -> bool {
        self.screenshot
            || self.webcam
            || self.keyboard
            || self.mouse
            || self.active_window
            || self.running_programs
    }

    /// Calculate strict intersection between this grant and a project policy.
    /// Invariant: A capability is admitted ONLY if both user and project grant it.
    /// Project policy CANNOT grant what the user has declined.
    pub fn intersect(&self, project: &Self) -> Self {
        Self {
            screenshot: self.screenshot && project.screenshot,
            webcam: self.webcam && project.webcam,
            keyboard: self.keyboard && project.keyboard,
            mouse: self.mouse && project.mouse,
            active_window: self.active_window && project.active_window,
            running_programs: self.running_programs && project.running_programs,
            // Blur is enforced if either party requests it (defense-in-depth)
            blur_screenshots: self.blur_screenshots || project.blur_screenshots,
        }
    }

    /// Returns true if `other` contains any capability that is not granted in `self`.
    pub fn is_narrower_than(&self, previous: &Self) -> bool {
        (previous.screenshot && !self.screenshot)
            || (previous.webcam && !self.webcam)
            || (previous.keyboard && !self.keyboard)
            || (previous.mouse && !self.mouse)
            || (previous.active_window && !self.active_window)
            || (previous.running_programs && !self.running_programs)
    }
}

/// Explicit local user consent state on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalConsent {
    /// No consent choice has been made. Fails closed (all captures blocked).
    Absent,
    /// User explicitly consented to the specified capabilities.
    Granted(ConsentedPermissions),
    /// User explicitly revoked or declined tracking consent.
    Revoked,
}

impl Default for LocalConsent {
    fn default() -> Self {
        LocalConsent::Absent
    }
}

impl LocalConsent {
    pub fn is_granted(&self) -> bool {
        matches!(self, LocalConsent::Granted(_))
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, LocalConsent::Revoked)
    }

    pub fn permissions(&self) -> ConsentedPermissions {
        match self {
            LocalConsent::Granted(p) => *p,
            LocalConsent::Absent | LocalConsent::Revoked => ConsentedPermissions::all_disabled(),
        }
    }
}

/// Consent storage and persistence errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentError {
    IoError(String),
    MalformedJson(String),
    SaveFailed(String),
}

impl std::fmt::Display for ConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsentError::IoError(s) => write!(f, "Consent I/O error: {}", s),
            ConsentError::MalformedJson(s) => write!(f, "Malformed consent JSON: {}", s),
            ConsentError::SaveFailed(s) => write!(f, "Consent save failed: {}", s),
        }
    }
}

impl std::error::Error for ConsentError {}

/// File-backed consent store.
pub struct ConsentStore;

impl ConsentStore {
    /// Load persisted consent from file.
    /// Invariant:
    /// - If file does not exist, returns `Ok(LocalConsent::Absent)`.
    /// - If file exists but is corrupted/malformed, returns `Err(ConsentError::MalformedJson)`
    ///   so caller fails closed.
    pub fn load(path: &Path) -> Result<LocalConsent, ConsentError> {
        if !path.exists() {
            return Ok(LocalConsent::Absent);
        }

        let content = fs::read_to_string(path).map_err(|e| ConsentError::IoError(e.to_string()))?;

        serde_json::from_str::<LocalConsent>(&content).map_err(|e| {
            ConsentError::MalformedJson(format!("Failed to parse {}: {}", path.display(), e))
        })
    }

    /// Save consent state atomically to file.
    /// Writes to `<path>.tmp` first, then atomically renames to `<path>`.
    pub fn save(path: &Path, consent: &LocalConsent) -> Result<(), ConsentError> {
        let parent = path.parent().ok_or_else(|| {
            ConsentError::SaveFailed("Target path has no parent directory".into())
        })?;

        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                ConsentError::SaveFailed(format!("Cannot create dir {}: {}", parent.display(), e))
            })?;
        }

        let json = serde_json::to_string_pretty(consent)
            .map_err(|e| ConsentError::SaveFailed(e.to_string()))?;

        let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
        fs::write(&tmp_path, json.as_bytes()).map_err(|e| {
            ConsentError::SaveFailed(format!(
                "Failed writing temp file {}: {}",
                tmp_path.display(),
                e
            ))
        })?;

        fs::rename(&tmp_path, path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            ConsentError::SaveFailed(format!(
                "Failed renaming {} to {}: {}",
                tmp_path.display(),
                path.display(),
                e
            ))
        })?;

        Ok(())
    }
}

/// In-memory manager for local consent with immediate revocation semantics.
#[derive(Debug, Clone)]
pub struct ConsentManager {
    current: LocalConsent,
    persist_path: Option<PathBuf>,
}

impl ConsentManager {
    /// Construct manager with an initial state and optional persistence path.
    pub fn new(initial: LocalConsent, persist_path: Option<PathBuf>) -> Self {
        Self {
            current: initial,
            persist_path,
        }
    }

    /// Load from path, failing closed to `LocalConsent::Absent` on malformed file.
    pub fn load_or_fail_closed(path: &Path) -> (Self, Option<ConsentError>) {
        match ConsentStore::load(path) {
            Ok(consent) => (Self::new(consent, Some(path.to_path_buf())), None),
            Err(err) => {
                // Fail closed: malformed file results in Absent consent
                (
                    Self::new(LocalConsent::Absent, Some(path.to_path_buf())),
                    Some(err),
                )
            }
        }
    }

    pub fn current(&self) -> LocalConsent {
        self.current
    }

    /// Compute effective permissions against a project policy.
    pub fn effective_permissions(&self, project: &ConsentedPermissions) -> ConsentedPermissions {
        match &self.current {
            LocalConsent::Granted(local) => local.intersect(project),
            LocalConsent::Absent | LocalConsent::Revoked => ConsentedPermissions::all_disabled(),
        }
    }

    /// Update consent state.
    ///
    /// CRITICAL PRIVACY INVARIANT:
    /// If consent is narrowed (capabilities removed) or revoked, `self.current` is updated
    /// in memory IMMEDIATELY. Even if disk persistence subsequently returns an Err,
    /// the in-memory admission is already revoked and cannot be exploited.
    pub fn update(&mut self, next: LocalConsent) -> Result<(), ConsentError> {
        let prev = self.current;
        let is_narrowing = match (prev, next) {
            (LocalConsent::Granted(p_old), LocalConsent::Granted(p_new)) => {
                p_new.is_narrower_than(&p_old)
            }
            (LocalConsent::Granted(_), LocalConsent::Revoked | LocalConsent::Absent) => true,
            _ => false,
        };

        if is_narrowing {
            // Immediately drop in memory before touching disk
            self.current = next;
            if let Some(ref path) = self.persist_path {
                ConsentStore::save(path, &next)?;
            }
            Ok(())
        } else {
            // Widening: attempt persistence first. If save fails, do not broaden in memory
            if let Some(ref path) = self.persist_path {
                ConsentStore::save(path, &next)?;
            }
            self.current = next;
            Ok(())
        }
    }

    /// Revoke all consent immediately.
    pub fn revoke(&mut self) -> Result<(), ConsentError> {
        self.update(LocalConsent::Revoked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_absent_and_all_permissions_are_disabled() {
        let consent = LocalConsent::default();
        assert_eq!(consent, LocalConsent::Absent);
        assert!(!consent.is_granted());
        assert!(!consent.is_revoked());

        let perms = consent.permissions();
        assert!(!perms.has_any_active_capability());
        assert!(!perms.keyboard);
        assert!(!perms.mouse);
        assert!(!perms.screenshot);
        assert!(!perms.webcam);
    }

    #[test]
    fn project_policy_cannot_grant_local_consent() {
        let manager = ConsentManager::new(LocalConsent::Absent, None);

        let mut project_policy = ConsentedPermissions::all_disabled();
        project_policy.screenshot = true;
        project_policy.keyboard = true;

        let effective = manager.effective_permissions(&project_policy);
        assert!(!effective.screenshot);
        assert!(!effective.keyboard);
        assert!(!effective.has_any_active_capability());
    }

    #[test]
    fn effective_permissions_is_strict_intersection() {
        let mut user_perms = ConsentedPermissions::all_disabled();
        user_perms.keyboard = true;
        user_perms.screenshot = false;

        let manager = ConsentManager::new(LocalConsent::Granted(user_perms), None);

        let mut project_policy = ConsentedPermissions::all_disabled();
        project_policy.keyboard = true;
        project_policy.screenshot = true;

        let effective = manager.effective_permissions(&project_policy);
        assert!(effective.keyboard);
        assert!(!effective.screenshot);
    }

    #[test]
    fn fails_closed_on_malformed_persisted_json() {
        let temp = tempfile::tempdir().unwrap();
        let consent_file = temp.path().join("consent.json");

        // Write corrupt JSON
        fs::write(&consent_file, b"{ invalid json content").unwrap();

        let (manager, err) = ConsentManager::load_or_fail_closed(&consent_file);
        assert!(err.is_some());
        assert_eq!(manager.current(), LocalConsent::Absent);
        assert!(!manager.current().permissions().has_any_active_capability());
    }

    #[test]
    fn narrowed_grant_revokes_immediately_even_if_persistence_fails() {
        let temp = tempfile::tempdir().unwrap();
        // Point to a directory that cannot be written to as a file
        let invalid_file_path = temp.path().join("directory_blocker");
        fs::create_dir_all(&invalid_file_path).unwrap();

        let mut initial = ConsentedPermissions::all_disabled();
        initial.keyboard = true;
        initial.screenshot = true;

        // Path is a directory, so save() will fail with an OS error
        let mut manager =
            ConsentManager::new(LocalConsent::Granted(initial), Some(invalid_file_path));

        let mut narrowed = ConsentedPermissions::all_disabled();
        narrowed.keyboard = true;
        narrowed.screenshot = false; // screenshot removed

        let res = manager.update(LocalConsent::Granted(narrowed));
        assert!(res.is_err(), "Persistence should have failed");

        // In-memory grant MUST be narrowed despite disk save failure!
        let effective = manager.current().permissions();
        assert!(effective.keyboard);
        assert!(
            !effective.screenshot,
            "Screenshot MUST be revoked immediately in memory"
        );
    }

    #[test]
    fn atomic_save_and_load_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let consent_file = temp.path().join("sub").join("consent.json");

        let mut perms = ConsentedPermissions::all_disabled();
        perms.keyboard = true;
        perms.mouse = true;

        ConsentStore::save(&consent_file, &LocalConsent::Granted(perms)).unwrap();
        assert!(consent_file.is_file());

        let loaded = ConsentStore::load(&consent_file).unwrap();
        assert_eq!(loaded, LocalConsent::Granted(perms));
    }
}
