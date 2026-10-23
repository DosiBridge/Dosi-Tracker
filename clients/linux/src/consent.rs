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
    UnsafeStorage(String),
}

impl std::fmt::Display for ConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsentError::IoError(s) => write!(f, "Consent I/O error: {}", s),
            ConsentError::MalformedJson(s) => write!(f, "Malformed consent JSON: {}", s),
            ConsentError::SaveFailed(s) => write!(f, "Consent save failed: {}", s),
            ConsentError::UnsafeStorage(s) => write!(f, "Unsafe consent storage: {}", s),
        }
    }
}

impl std::error::Error for ConsentError {}

#[cfg(unix)]
extern "C" {
    fn geteuid() -> u32;
}

static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn generate_nonce() -> u64 {
    #[cfg(unix)]
    {
        if let Ok(mut f) = fs::File::open("/dev/urandom") {
            use std::io::Read;
            let mut buf = [0u8; 8];
            if f.read_exact(&mut buf).is_ok() {
                return u64::from_ne_bytes(buf);
            }
        }
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let counter = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let addr = &counter as *const _ as usize as u64;
    (ts as u64) ^ (pid << 32) ^ (counter << 16) ^ addr
}

fn validate_target_file(path: &Path) -> Result<(), ConsentError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(ConsentError::UnsafeStorage(format!(
                    "Target path {} is a symbolic link",
                    path.display()
                )));
            }
            if !meta.file_type().is_file() {
                return Err(ConsentError::UnsafeStorage(format!(
                    "Target path {} is not a regular file",
                    path.display()
                )));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                use std::os::unix::fs::PermissionsExt;
                let current_uid = unsafe { geteuid() };
                if meta.uid() != current_uid {
                    return Err(ConsentError::UnsafeStorage(format!(
                        "Target path {} is owned by UID {}, expected current UID {}",
                        path.display(),
                        meta.uid(),
                        current_uid
                    )));
                }
                let mode = meta.permissions().mode();
                if (mode & 0o022) != 0 {
                    return Err(ConsentError::UnsafeStorage(format!(
                        "Target path {} has unsafe permissions {:o} (group or world writable)",
                        path.display(),
                        mode & 0o777
                    )));
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ConsentError::IoError(format!(
            "Failed inspecting metadata for {}: {}",
            path.display(),
            e
        ))),
    }
}

/// File-backed consent store with private, exclusive atomic persistence.
pub struct ConsentStore;

impl ConsentStore {
    /// Load persisted consent from file.
    /// Invariants:
    /// - If file does not exist, returns `Ok(LocalConsent::Absent)`.
    /// - Target path is inspected with `symlink_metadata` (never following symlinks).
    ///   Rejects symlinks, directories, non-regular files, and (on Unix) files with
    ///   foreign UID or group/world-writable bits.
    /// - If file exists but is corrupted/malformed, returns `Err(ConsentError::MalformedJson)`
    ///   so caller fails closed.
    pub fn load(path: &Path) -> Result<LocalConsent, ConsentError> {
        validate_target_file(path)?;

        if !path.exists() {
            return Ok(LocalConsent::Absent);
        }

        let content = fs::read_to_string(path).map_err(|e| ConsentError::IoError(e.to_string()))?;

        serde_json::from_str::<LocalConsent>(&content).map_err(|e| {
            ConsentError::MalformedJson(format!("Failed to parse {}: {}", path.display(), e))
        })
    }

    /// Save consent state atomically to file.
    /// Invariants:
    /// - Inspects target file if present: refuses symlinks, directories, or unsafe permissions.
    /// - Newly created parent directories are given private Unix mode 0700; existing ancestors are never modified.
    /// - Creates a fresh, collision-resistant temporary file exclusively (`create_new(true)`) with Unix mode 0600
    ///   directly at creation (no chmod after exposure).
    /// - Does not follow pre-existing temporary symlinks or overwrite unrelated temporary files.
    /// - Flushes and syncs data to disk before atomic replacement via `fs::rename`.
    /// - On failure, removes ONLY the operation's owned temporary file artifact.
    pub fn save(path: &Path, consent: &LocalConsent) -> Result<(), ConsentError> {
        let parent = path.parent().ok_or_else(|| {
            ConsentError::SaveFailed("Target path has no parent directory".into())
        })?;
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };

        if let Ok(meta) = fs::symlink_metadata(parent) {
            if meta.file_type().is_symlink() {
                return Err(ConsentError::SaveFailed(format!(
                    "Parent directory {} is a symbolic link",
                    parent.display()
                )));
            }
            if !meta.file_type().is_dir() {
                return Err(ConsentError::SaveFailed(format!(
                    "Parent path {} is not a directory",
                    parent.display()
                )));
            }
        }

        if !parent.exists() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(parent).map_err(|e| {
                ConsentError::SaveFailed(format!("Cannot create dir {}: {}", parent.display(), e))
            })?;
        }

        validate_target_file(path)?;

        let json = serde_json::to_string_pretty(consent)
            .map_err(|e| ConsentError::SaveFailed(e.to_string()))?;

        let file_stem = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("consent");

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        const MAX_TEMP_RETRIES: usize = 64;
        let mut temp_file = None;
        let mut owned_temp_path = None;

        for attempt in 0..MAX_TEMP_RETRIES {
            let nonce = generate_nonce();
            let temp_name = format!(
                ".{}.tmp.{}.{}.{:016x}",
                file_stem,
                std::process::id(),
                attempt,
                nonce
            );
            let candidate = parent.join(temp_name);
            match options.open(&candidate) {
                Ok(f) => {
                    temp_file = Some(f);
                    owned_temp_path = Some(candidate);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(e) => {
                    return Err(ConsentError::SaveFailed(format!(
                        "Failed opening exclusive temporary file {}: {}",
                        candidate.display(),
                        e
                    )));
                }
            }
        }

        let (mut file, temp_path) = match (temp_file, owned_temp_path) {
            (Some(f), Some(p)) => (f, p),
            _ => {
                return Err(ConsentError::SaveFailed(
                    "Exceeded maximum collision retries creating temporary consent file".into(),
                ));
            }
        };

        let write_result = (|| -> Result<(), std::io::Error> {
            use std::io::Write;
            file.write_all(json.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            Ok(())
        })();

        if let Err(e) = write_result {
            let _ = fs::remove_file(&temp_path);
            return Err(ConsentError::SaveFailed(format!(
                "Failed writing temporary consent file {}: {}",
                temp_path.display(),
                e
            )));
        }

        if let Err(e) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(ConsentError::SaveFailed(format!(
                "Failed atomically replacing consent file {} with {}: {}",
                path.display(),
                temp_path.display(),
                e
            )));
        }

        Ok(())
    }

    /// Check if target file exists and is a regular, non-symlink file.
    pub fn exists(path: &Path) -> bool {
        match fs::symlink_metadata(path) {
            Ok(meta) => !meta.file_type().is_symlink() && meta.file_type().is_file(),
            Err(_) => false,
        }
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
    fn test_consent_store_positive_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("dedicated_dir").join("consent.json");

        let mut perms = ConsentedPermissions::all_disabled();
        perms.keyboard = true;
        perms.mouse = true;
        let initial = LocalConsent::Granted(perms);

        let store = FileConsentStore::new(file.clone());
        assert!(!ConsentStore::exists(&file));

        store.save(&initial).expect("save should succeed");
        assert!(ConsentStore::exists(&file));

        let loaded = store.load().expect("load should succeed");
        assert_eq!(loaded, initial);

        let revoked = LocalConsent::Revoked;
        store.save(&revoked).expect("save revoked should succeed");
        let loaded_revoked = store.load().expect("load revoked should succeed");
        assert_eq!(loaded_revoked, revoked);
    }

    #[cfg(unix)]
    #[test]
    fn test_private_file_and_new_directory_modes_on_unix() {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let sub = temp.path().join("private_consent_dir");
        let file = sub.join("consent.json");

        let perms = ConsentedPermissions::all_disabled();
        ConsentStore::save(&file, &LocalConsent::Granted(perms)).unwrap();

        let dir_meta = fs::symlink_metadata(&sub).unwrap();
        assert_eq!(
            dir_meta.permissions().mode() & 0o777,
            0o700,
            "Newly created dedicated directory must have private mode 0700"
        );

        let file_meta = fs::symlink_metadata(&file).unwrap();
        assert_eq!(
            file_meta.permissions().mode() & 0o777,
            0o600,
            "Saved consent file must have private mode 0600 directly at creation"
        );

        let current_uid = unsafe { geteuid() };
        assert_eq!(
            file_meta.uid(),
            current_uid,
            "Saved consent file must be owned by current UID"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_precreated_predictable_temp_symlink_plus_sentinel_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel_path = temp.path().join("victim_sentinel.txt");
        fs::write(&sentinel_path, b"CONFIDENTIAL_SENTINEL_CONTENT").unwrap();

        let consent_path = temp.path().join("consent.json");

        // Old predictable temp file format: path.with_extension("tmp.<PID>")
        let old_predictable_tmp = consent_path.with_extension(format!("tmp.{}", std::process::id()));
        std::os::unix::fs::symlink(&sentinel_path, &old_predictable_tmp).unwrap();

        // Also pre-create symlink matching dot-stem pattern if an attacker guessed it
        let candidate_symlink = temp
            .path()
            .join(format!(".consent.json.tmp.{}.0.0000000000000000", std::process::id()));
        std::os::unix::fs::symlink(&sentinel_path, &candidate_symlink).unwrap();

        let mut perms = ConsentedPermissions::all_disabled();
        perms.keyboard = true;

        ConsentStore::save(&consent_path, &LocalConsent::Granted(perms))
            .expect("Save should succeed using fresh exclusive temp file");

        let sentinel_content = fs::read_to_string(&sentinel_path).unwrap();
        assert_eq!(
            sentinel_content, "CONFIDENTIAL_SENTINEL_CONTENT",
            "Sentinel file MUST NOT be truncated or overwritten via precreated symlinks"
        );

        assert!(fs::symlink_metadata(&old_predictable_tmp)
            .unwrap()
            .file_type()
            .is_symlink());

        let loaded = ConsentStore::load(&consent_path).unwrap();
        assert_eq!(loaded, LocalConsent::Granted(perms));
    }

    #[test]
    fn test_target_directory_refusal() {
        let temp = tempfile::tempdir().unwrap();
        let target_dir = temp.path().join("blocker_directory.json");
        fs::create_dir_all(&target_dir).unwrap();

        let load_err = ConsentStore::load(&target_dir).unwrap_err();
        assert!(
            matches!(load_err, ConsentError::UnsafeStorage(_)),
            "Directory load must fail with UnsafeStorage"
        );

        let save_err = ConsentStore::save(&target_dir, &LocalConsent::Absent).unwrap_err();
        assert!(
            matches!(save_err, ConsentError::UnsafeStorage(_)),
            "Directory save must fail with UnsafeStorage"
        );

        assert!(target_dir.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn test_target_symlink_refusal_under_policy() {
        let temp = tempfile::tempdir().unwrap();
        let legitimate_target = temp.path().join("real_consent.json");
        fs::write(&legitimate_target, b"\"Absent\"").unwrap();

        let symlink_path = temp.path().join("symlink_consent.json");
        std::os::unix::fs::symlink(&legitimate_target, &symlink_path).unwrap();

        let load_res = ConsentStore::load(&symlink_path);
        assert!(
            matches!(load_res, Err(ConsentError::UnsafeStorage(_))),
            "load must refuse symlink target"
        );

        let mut perms = ConsentedPermissions::all_disabled();
        perms.screenshot = true;
        let save_res = ConsentStore::save(&symlink_path, &LocalConsent::Granted(perms));
        assert!(
            matches!(save_res, Err(ConsentError::UnsafeStorage(_))),
            "save must refuse symlink target"
        );

        let content = fs::read_to_string(&legitimate_target).unwrap();
        assert_eq!(content, "\"Absent\"");
    }

    #[test]
    fn test_exclusive_temp_collision_and_unrelated_artifact_preservation() {
        let temp = tempfile::tempdir().unwrap();
        let consent_path = temp.path().join("consent.json");

        let unrelated_file = temp.path().join("unrelated_data.bin");
        fs::write(&unrelated_file, b"UNRELATED_DO_NOT_DELETE").unwrap();

        let unrelated_tmp = temp.path().join(".consent.json.tmp.other_worker");
        fs::write(&unrelated_tmp, b"OTHER_WORKER_ARTIFACT").unwrap();

        let mut perms = ConsentedPermissions::all_disabled();
        perms.mouse = true;
        ConsentStore::save(&consent_path, &LocalConsent::Granted(perms)).unwrap();

        assert_eq!(
            fs::read(&unrelated_file).unwrap(),
            b"UNRELATED_DO_NOT_DELETE",
            "Unrelated file must not be modified or deleted"
        );
        assert_eq!(
            fs::read(&unrelated_tmp).unwrap(),
            b"OTHER_WORKER_ARTIFACT",
            "Unrelated temp file must not be modified or deleted"
        );

        assert_eq!(
            ConsentStore::load(&consent_path).unwrap(),
            LocalConsent::Granted(perms)
        );
    }

    #[test]
    fn test_failed_save_retaining_previous_valid_record_and_manager_not_widening() {
        let temp = tempfile::tempdir().unwrap();
        let consent_file = temp.path().join("consent.json");

        let mut initial = ConsentedPermissions::all_disabled();
        initial.keyboard = true;
        ConsentStore::save(&consent_file, &LocalConsent::Granted(initial)).unwrap();

        let store = std::sync::Arc::new(FileConsentStore::new(consent_file.clone()));
        let mut manager = ConsentManager::with_storage(LocalConsent::Granted(initial), Some(store));
        assert_eq!(manager.current().permissions().keyboard, true);

        // Replace consent_file with an unsafe directory so subsequent disk saves fail validation
        fs::remove_file(&consent_file).unwrap();
        fs::create_dir_all(&consent_file).unwrap();

        let mut expanded = initial;
        expanded.screenshot = true;

        let res = manager.update(LocalConsent::Granted(expanded));
        assert!(
            res.is_err(),
            "Save should fail because target path is an unsafe directory"
        );

        assert!(
            !manager.current().permissions().screenshot,
            "Newly widened permission (screenshot) must NOT be admitted when disk save fails"
        );
        assert_eq!(
            manager.current().permissions().keyboard,
            true,
            "Previously consented permission (keyboard) remains intact"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_unsafe_permissions_load_fails_closed_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let consent_file = temp.path().join("unsafe_mode.json");

        let mut perms = ConsentedPermissions::all_disabled();
        perms.screenshot = true;
        let json = serde_json::to_string_pretty(&LocalConsent::Granted(perms)).unwrap();
        fs::write(&consent_file, json.as_bytes()).unwrap();

        fs::set_permissions(&consent_file, PermissionsExt::from_mode(0o666)).unwrap();

        let load_err = ConsentStore::load(&consent_file).unwrap_err();
        assert!(
            matches!(load_err, ConsentError::UnsafeStorage(_)),
            "Load must fail with UnsafeStorage for group/world-writable file"
        );

        let (manager, err) = ConsentManager::load_or_fail_closed(&consent_file);
        assert!(err.is_some());
        assert_eq!(
            manager.current(),
            LocalConsent::Absent,
            "ConsentManager must fail closed to Absent on unsafe file permissions"
        );
    }
}
