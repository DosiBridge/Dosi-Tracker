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
    /// Check if target file exists.
    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }
}

/// Abstract storage trait for consent persistence.
pub trait ConsentStorage: Send + Sync {
    fn load(&self) -> Result<LocalConsent, ConsentError>;
    fn save(&self, consent: &LocalConsent) -> Result<(), ConsentError>;
}

/// File-backed implementation of `ConsentStorage`.
#[derive(Debug, Clone)]
pub struct FileConsentStore {
    path: PathBuf,
}

impl FileConsentStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConsentStorage for FileConsentStore {
    fn load(&self) -> Result<LocalConsent, ConsentError> {
        ConsentStore::load(&self.path)
    }

    fn save(&self, consent: &LocalConsent) -> Result<(), ConsentError> {
        ConsentStore::save(&self.path, consent)
    }
}

pub type SaveBarrier = Box<dyn Fn(&LocalConsent) -> Result<(), ConsentError> + Send + Sync>;

/// In-memory fake consent storage with fault injection and barrier callbacks for deterministic testing.
pub struct MemoryConsentStore {
    consent: std::sync::Mutex<LocalConsent>,
    fail_saves: std::sync::atomic::AtomicBool,
    fail_loads: std::sync::atomic::AtomicBool,
    save_count: std::sync::atomic::AtomicUsize,
    load_count: std::sync::atomic::AtomicUsize,
    save_barrier: std::sync::Mutex<Option<SaveBarrier>>,
}

impl Default for MemoryConsentStore {
    fn default() -> Self {
        Self::new(LocalConsent::Absent)
    }
}

impl MemoryConsentStore {
    pub fn new(initial: LocalConsent) -> Self {
        Self {
            consent: std::sync::Mutex::new(initial),
            fail_saves: std::sync::atomic::AtomicBool::new(false),
            fail_loads: std::sync::atomic::AtomicBool::new(false),
            save_count: std::sync::atomic::AtomicUsize::new(0),
            load_count: std::sync::atomic::AtomicUsize::new(0),
            save_barrier: std::sync::Mutex::new(None),
        }
    }

    pub fn set_fail_saves(&self, fail: bool) {
        self.fail_saves
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_fail_loads(&self, fail: bool) {
        self.fail_loads
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_save_barrier<F>(&self, barrier: F)
    where
        F: Fn(&LocalConsent) -> Result<(), ConsentError> + Send + Sync + 'static,
    {
        *self.save_barrier.lock().unwrap() = Some(Box::new(barrier));
    }

    pub fn clear_save_barrier(&self) {
        *self.save_barrier.lock().unwrap() = None;
    }

    pub fn save_count(&self) -> usize {
        self.save_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn load_count(&self) -> usize {
        self.load_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn current(&self) -> LocalConsent {
        *self.consent.lock().unwrap()
    }
}

impl ConsentStorage for MemoryConsentStore {
    fn load(&self) -> Result<LocalConsent, ConsentError> {
        self.load_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail_loads.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ConsentError::IoError("Simulated load failure".into()));
        }
        Ok(*self.consent.lock().unwrap())
    }

    fn save(&self, consent: &LocalConsent) -> Result<(), ConsentError> {
        self.save_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(ref barrier) = *self.save_barrier.lock().unwrap() {
            barrier(consent)?;
        }
        if self.fail_saves.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ConsentError::SaveFailed(
                "Simulated persistence failure".into(),
            ));
        }
        *self.consent.lock().unwrap() = *consent;
        Ok(())
    }
}

/// In-memory manager for local consent with immediate revocation and persistence-first widening semantics.
#[derive(Clone)]
pub struct ConsentManager {
    current: LocalConsent,
    storage: Option<std::sync::Arc<dyn ConsentStorage>>,
}

impl ConsentManager {
    /// Construct manager with an initial state and optional file path.
    pub fn new(initial: LocalConsent, persist_path: Option<PathBuf>) -> Self {
        let storage: Option<std::sync::Arc<dyn ConsentStorage>> = persist_path.map(|p| {
            std::sync::Arc::new(FileConsentStore::new(p)) as std::sync::Arc<dyn ConsentStorage>
        });
        Self::with_storage(initial, storage)
    }

    /// Construct manager with custom storage provider.
    pub fn with_storage(
        initial: LocalConsent,
        storage: Option<std::sync::Arc<dyn ConsentStorage>>,
    ) -> Self {
        Self {
            current: initial,
            storage,
        }
    }

    /// Set or update the file persistence path.
    pub fn set_persist_path(&mut self, path: Option<PathBuf>) {
        self.storage = path.map(|p| {
            std::sync::Arc::new(FileConsentStore::new(p)) as std::sync::Arc<dyn ConsentStorage>
        });
    }

    /// Set or update the storage provider.
    pub fn set_storage(&mut self, storage: Option<std::sync::Arc<dyn ConsentStorage>>) {
        self.storage = storage;
    }

    /// Load from path, failing closed to `LocalConsent::Absent` on malformed file.
    pub fn load_or_fail_closed(path: &Path) -> (Self, Option<ConsentError>) {
        let store = FileConsentStore::new(path.to_path_buf());
        match store.load() {
            Ok(consent) => (
                Self::with_storage(consent, Some(std::sync::Arc::new(store))),
                None,
            ),
            Err(err) => (
                Self::with_storage(LocalConsent::Absent, Some(std::sync::Arc::new(store))),
                Some(err),
            ),
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
    /// CRITICAL PRIVACY & PERSISTENCE INVARIANTS:
    /// - Capabilities removed in `next` are withdrawn IMMEDIATELY in-memory before disk I/O.
    /// - Blur tightening (`blur_screenshots` false -> true) is applied IMMEDIATELY as defense-in-depth.
    /// - Newly added capabilities in `next` are admitted ONLY AFTER persistence succeeds.
    /// - If persistence fails on a mixed update (narrowing + expansion), removed permissions
    ///   remain withdrawn and tightened blur remains enforced, but new permissions are NOT granted.
    /// - Pure widening remains strictly persistence-first.
    pub fn update(&mut self, next: LocalConsent) -> Result<(), ConsentError> {
        let prev = self.current;
        if prev == next {
            return Ok(());
        }

        match (prev, next) {
            (LocalConsent::Granted(p_old), LocalConsent::Granted(p_new)) => {
                // Compute immediate in-memory grant:
                // - Only retain capabilities present in BOTH p_old and p_new (withdraw removed; do not admit new).
                // - Apply blur tightening immediately (defense-in-depth).
                let p_immediate = ConsentedPermissions {
                    screenshot: p_old.screenshot && p_new.screenshot,
                    webcam: p_old.webcam && p_new.webcam,
                    keyboard: p_old.keyboard && p_new.keyboard,
                    mouse: p_old.mouse && p_new.mouse,
                    active_window: p_old.active_window && p_new.active_window,
                    running_programs: p_old.running_programs && p_new.running_programs,
                    blur_screenshots: p_old.blur_screenshots || p_new.blur_screenshots,
                };

                // Immediately drop removed capabilities and enforce tightened blur
                self.current = LocalConsent::Granted(p_immediate);

                if let Some(ref storage) = self.storage {
                    storage.save(&next)?;
                }

                // Persistence succeeded: admit newly added capabilities and loosened blur
                self.current = next;
                Ok(())
            }
            (LocalConsent::Granted(_), LocalConsent::Revoked | LocalConsent::Absent) => {
                // Immediate withdrawal of all capabilities before persistence
                self.current = next;

                if let Some(ref storage) = self.storage {
                    storage.save(&next)?;
                }
                Ok(())
            }
            (LocalConsent::Absent | LocalConsent::Revoked, LocalConsent::Granted(_)) => {
                // Pure widening from an inactive state: persistence-first. No permissions admitted on save failure.
                if let Some(ref storage) = self.storage {
                    storage.save(&next)?;
                }
                self.current = next;
                Ok(())
            }
            _ => {
                // Absent -> Revoked or Revoked -> Absent
                self.current = next;
                if let Some(ref storage) = self.storage {
                    storage.save(&next)?;
                }
                Ok(())
            }
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
