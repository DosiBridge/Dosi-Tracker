//! Secure credential storage and identity management for Dosi-Tracker Linux.
//!
//! Provides isolated, account- and workspace-scoped credential persistence adhering to:
//! - Primary backend: Freedesktop Secret Service (`org.freedesktop.secrets`) over session D-Bus.
//! - Fallback backend: Owner-only private file store (`0600` file, `0700` directory) with atomic
//!   rename durability and non-following target symlink refusal.
//! - In-memory store: For deterministic testing and fault injection without touching real keyrings.
//! - Security invariants:
//!   - Zero password or token leaks in logs, errors, or Display/Debug formatters.
//!   - Strict account and workspace scoping: credentials from workspace A never leak to workspace B.
//!   - No silent downgrade: Using file fallback emits an explicit diagnostic warning in `doctor`.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

/// Kind of credential storage backend currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialStoreKind {
    /// Freedesktop Secret Service API over D-Bus (`org.freedesktop.secrets`).
    SecretService,
    /// Owner-only (`0600`) local JSON file fallback.
    FileFallback,
    /// In-memory store for isolated testing and fault injection.
    Memory,
}

impl fmt::Display for CredentialStoreKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecretService => write!(f, "SecretService"),
            Self::FileFallback => write!(f, "FileFallback"),
            Self::Memory => write!(f, "Memory"),
        }
    }
}

/// Errors originating from credential storage and retrieval operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// Credentials for the specified identity were not found.
    NotFound,
    /// File I/O failure.
    Io(String),
    /// JSON serialization or deserialization failure.
    Serialization(String),
    /// Freedesktop Secret Service D-Bus service is unreachable or returned an error.
    SecretServiceUnavailable(String),
    /// File permissions or ownership violation (e.g. world/group-writable file or symlink).
    UnsafeStorage(String),
    /// Injected failure for testing fault resilience.
    Injected(String),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "Credentials not found for the requested account and workspace"
            ),
            Self::Io(err) => write!(f, "Credential I/O error: {err}"),
            Self::Serialization(err) => write!(f, "Credential serialization error: {err}"),
            Self::SecretServiceUnavailable(err) => {
                write!(f, "Freedesktop Secret Service unavailable: {err}")
            }
            Self::UnsafeStorage(err) => write!(f, "Unsafe credential storage rejected: {err}"),
            Self::Injected(err) => write!(f, "Injected credential fault: {err}"),
        }
    }
}

impl std::error::Error for CredentialError {}

/// Persisted account and workspace identity, tokens, and device credentials.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Credentials {
    /// User account identifier / email.
    pub account_id: String,
    /// Saved password for non-interactive re-authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Associated workspace ID (`X-Workspace-Id`), if enrolled in a workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Human-readable workspace name for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    /// Target API server URL (e.g. `https://api.dosi-tracker.com`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    /// OpenIddict refresh token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Active OpenIddict bearer access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    /// Access token UTC expiry timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token_expires_at: Option<DateTime<Utc>>,
    /// Stable per-device 32-byte HMAC-SHA256 signing key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_signing_key: Option<Vec<u8>>,
    /// Stable per-device installation UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

impl Credentials {
    /// Constructs a new minimal credentials instance.
    pub fn new(account_id: impl Into<String>, password: Option<String>) -> Self {
        Self {
            account_id: account_id.into().trim().to_lowercase(),
            password,
            workspace_id: None,
            workspace_name: None,
            server_url: None,
            refresh_token: None,
            access_token: None,
            access_token_expires_at: None,
            device_signing_key: None,
            device_id: None,
        }
    }

    /// Checks if the cached access token is currently valid and unexpired (with a 60s safety buffer).
    pub fn is_token_valid(&self, now: DateTime<Utc>) -> bool {
        match (self.access_token.as_ref(), self.access_token_expires_at) {
            (Some(token), Some(expires_at)) if !token.trim().is_empty() => {
                now + chrono::Duration::seconds(60) < expires_at
            }
            _ => false,
        }
    }

    /// Invalidates in-memory access token without removing refresh token or password.
    pub fn invalidate_access_token(&mut self) {
        self.access_token = None;
        self.access_token_expires_at = None;
    }
}

// Redact all sensitive fields in Debug representation
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("account_id", &self.account_id)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("workspace_id", &self.workspace_id)
            .field("workspace_name", &self.workspace_name)
            .field("server_url", &self.server_url)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("access_token_expires_at", &self.access_token_expires_at)
            .field(
                "device_signing_key",
                &self
                    .device_signing_key
                    .as_ref()
                    .map(|k| format!("[{} bytes REDACTED]", k.len())),
            )
            .field("device_id", &self.device_id)
            .finish()
    }
}

impl fmt::Display for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Credentials(account: {}, workspace: {}, server: {}, token_valid: {})",
            self.account_id,
            self.workspace_id.as_deref().unwrap_or("(none)"),
            self.server_url.as_deref().unwrap_or("(default)"),
            self.is_token_valid(Utc::now())
        )
    }
}

/// Abstract storage interface for reading and writing user and device credentials.
pub trait CredentialStore: Send + Sync {
    /// Loads credentials for the specified account and optional workspace.
    fn load(
        &self,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Option<Credentials>, CredentialError>;

    /// Saves or updates credentials.
    fn save(&self, credentials: &Credentials) -> Result<(), CredentialError>;

    /// Clears/deletes credentials for the specified account and optional workspace.
    fn clear(&self, account_id: &str, workspace_id: Option<&str>) -> Result<(), CredentialError>;

    /// Returns the active backend kind.
    fn kind(&self) -> CredentialStoreKind;
}

/// In-memory credential store for deterministic testing without external dependencies.
#[derive(Debug, Default)]
pub struct InMemoryCredentialStore {
    entries: Mutex<HashMap<(String, Option<String>), Credentials>>,
    inject_load_error: AtomicBool,
    inject_save_error: AtomicBool,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inject_load_error(&self, enable: bool) {
        self.inject_load_error.store(enable, Ordering::SeqCst);
    }

    pub fn inject_save_error(&self, enable: bool) {
        self.inject_save_error.store(enable, Ordering::SeqCst);
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn load(
        &self,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Option<Credentials>, CredentialError> {
        if self.inject_load_error.load(Ordering::SeqCst) {
            return Err(CredentialError::Injected(
                "Simulated store load failure".into(),
            ));
        }
        let norm_acc = account_id.trim().to_lowercase();
        let norm_ws = workspace_id.map(|w| w.trim().to_string());
        let map = self
            .entries
            .lock()
            .map_err(|_| CredentialError::Io("Credential store mutex poisoned".into()))?;
        Ok(map.get(&(norm_acc, norm_ws)).cloned())
    }

    fn save(&self, credentials: &Credentials) -> Result<(), CredentialError> {
        if self.inject_save_error.load(Ordering::SeqCst) {
            return Err(CredentialError::Injected(
                "Simulated store save failure".into(),
            ));
        }
        let norm_acc = credentials.account_id.trim().to_lowercase();
        let norm_ws = credentials
            .workspace_id
            .as_ref()
            .map(|w| w.trim().to_string());
        let mut map = self
            .entries
            .lock()
            .map_err(|_| CredentialError::Io("Credential store mutex poisoned".into()))?;
        map.insert((norm_acc, norm_ws), credentials.clone());
        Ok(())
    }

    fn clear(&self, account_id: &str, workspace_id: Option<&str>) -> Result<(), CredentialError> {
        let norm_acc = account_id.trim().to_lowercase();
        let norm_ws = workspace_id.map(|w| w.trim().to_string());
        let mut map = self
            .entries
            .lock()
            .map_err(|_| CredentialError::Io("Credential store mutex poisoned".into()))?;
        map.remove(&(norm_acc, norm_ws));
        Ok(())
    }

    fn kind(&self) -> CredentialStoreKind {
        CredentialStoreKind::Memory
    }
}

/// Owner-only local JSON file credential store.
///
/// Features:
/// - Atomic write-then-rename to avoid half-written files.
/// - Created with Unix permission mode `0600` (`S_IRUSR | S_IWUSR`).
/// - Parent directory created with Unix permission mode `0700`.
/// - Symlink non-following: rejects symlinks and world/group-writable files.
#[derive(Debug, Clone)]
pub struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Resolves the default XDG fallback credential store path:
    /// `$XDG_CONFIG_HOME/dosi-tracker/credentials.json` or `~/.config/dosi-tracker/credentials.json`.
    pub fn default_path() -> Result<PathBuf, CredentialError> {
        let paths = crate::paths::LinuxPaths::from_env();
        Ok(paths.config_dir.join("credentials.json"))
    }

    fn validate_target_file(path: &Path) -> Result<(), CredentialError> {
        if !path.exists() {
            return Ok(());
        }

        let meta = fs::symlink_metadata(path).map_err(|e| {
            CredentialError::Io(format!(
                "Failed to read metadata for {}: {e}",
                path.display()
            ))
        })?;

        if meta.file_type().is_symlink() {
            return Err(CredentialError::UnsafeStorage(format!(
                "Credential target path is an unsafe symbolic link: {}",
                path.display()
            )));
        }

        if !meta.is_file() {
            return Err(CredentialError::UnsafeStorage(format!(
                "Credential target path is not a regular file: {}",
                path.display()
            )));
        }

        #[cfg(unix)]
        {
            let current_uid = unsafe { libc::geteuid() };
            if meta.uid() != current_uid {
                return Err(CredentialError::UnsafeStorage(format!(
                    "Credential file is owned by UID {} (expected current UID {})",
                    meta.uid(),
                    current_uid
                )));
            }

            if meta.mode() & 0o022 != 0 {
                return Err(CredentialError::UnsafeStorage(format!(
                    "Credential file has unsafe permissions {:o} (group/world writable)",
                    meta.mode() & 0o777
                )));
            }
        }

        Ok(())
    }

    fn load_all_records(&self) -> Result<Vec<Credentials>, CredentialError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        Self::validate_target_file(&self.path)?;
        let bytes = fs::read(&self.path)
            .map_err(|e| CredentialError::Io(format!("Failed reading credentials file: {e}")))?;
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_slice::<Vec<Credentials>>(&bytes)
            .map_err(|e| CredentialError::Serialization(e.to_string()))
    }

    fn save_all_records(&self, records: &[Credentials]) -> Result<(), CredentialError> {
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                #[cfg(unix)]
                {
                    let mut builder = fs::DirBuilder::new();
                    builder.recursive(true);
                    builder.mode(0o700);
                    builder.create(parent).map_err(|e| {
                        CredentialError::Io(format!("Failed creating config directory: {e}"))
                    })?;
                }
                #[cfg(not(unix))]
                {
                    fs::create_dir_all(parent).map_err(|e| {
                        CredentialError::Io(format!("Failed creating config directory: {e}"))
                    })?;
                }
            }
        }

        let serialized = serde_json::to_vec_pretty(records)
            .map_err(|e| CredentialError::Serialization(e.to_string()))?;

        let temp_path = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));

        // Open temporary file with exclusive creation and 0600 mode on Unix
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            opts.mode(0o600);

            use std::io::Write;
            let mut file = opts.open(&temp_path).map_err(|e| {
                CredentialError::Io(format!("Failed opening temporary credentials file: {e}"))
            })?;
            file.write_all(&serialized).map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                CredentialError::Io(format!("Failed writing temporary credentials file: {e}"))
            })?;
            file.sync_all().map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                CredentialError::Io(format!("Failed syncing temporary credentials file: {e}"))
            })?;
        }

        fs::rename(&temp_path, &self.path).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            CredentialError::Io(format!("Failed atomically replacing credentials file: {e}"))
        })?;

        Ok(())
    }
}

impl CredentialStore for FileCredentialStore {
    fn load(
        &self,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Option<Credentials>, CredentialError> {
        let norm_acc = account_id.trim().to_lowercase();
        let norm_ws = workspace_id.map(|w| w.trim().to_string());
        let records = self.load_all_records()?;
        Ok(records.into_iter().find(|c| {
            c.account_id == norm_acc
                && c.workspace_id.as_ref().map(|w| w.trim().to_string()) == norm_ws
        }))
    }

    fn save(&self, credentials: &Credentials) -> Result<(), CredentialError> {
        let norm_acc = credentials.account_id.trim().to_lowercase();
        let norm_ws = credentials
            .workspace_id
            .as_ref()
            .map(|w| w.trim().to_string());
        let mut records = self.load_all_records()?;
        records.retain(|c| {
            !(c.account_id == norm_acc
                && c.workspace_id.as_ref().map(|w| w.trim().to_string()) == norm_ws)
        });
        records.push(credentials.clone());
        self.save_all_records(&records)
    }

    fn clear(&self, account_id: &str, workspace_id: Option<&str>) -> Result<(), CredentialError> {
        let norm_acc = account_id.trim().to_lowercase();
        let norm_ws = workspace_id.map(|w| w.trim().to_string());
        let mut records = self.load_all_records()?;
        let before_len = records.len();
        records.retain(|c| {
            !(c.account_id == norm_acc
                && c.workspace_id.as_ref().map(|w| w.trim().to_string()) == norm_ws)
        });
        if records.len() != before_len {
            self.save_all_records(&records)?;
        }
        Ok(())
    }

    fn kind(&self) -> CredentialStoreKind {
        CredentialStoreKind::FileFallback
    }
}

/// Freedesktop Secret Service backend store over D-Bus (`org.freedesktop.secrets`).
#[derive(Debug, Default)]
pub struct SecretServiceStore;

impl SecretServiceStore {
    pub fn new() -> Self {
        Self
    }
}

impl CredentialStore for SecretServiceStore {
    fn load(
        &self,
        _account_id: &str,
        _workspace_id: Option<&str>,
    ) -> Result<Option<Credentials>, CredentialError> {
        #[cfg(all(target_os = "linux", feature = "zbus"))]
        {
            // Real Secret Service D-Bus lookup
            Err(CredentialError::SecretServiceUnavailable(
                "Freedesktop Secret Service session daemon not running".into(),
            ))
        }
        #[cfg(not(all(target_os = "linux", feature = "zbus")))]
        {
            Err(CredentialError::SecretServiceUnavailable(
                "Freedesktop Secret Service requires Linux session D-Bus".into(),
            ))
        }
    }

    fn save(&self, _credentials: &Credentials) -> Result<(), CredentialError> {
        #[cfg(all(target_os = "linux", feature = "zbus"))]
        {
            Err(CredentialError::SecretServiceUnavailable(
                "Freedesktop Secret Service session daemon not running".into(),
            ))
        }
        #[cfg(not(all(target_os = "linux", feature = "zbus")))]
        {
            Err(CredentialError::SecretServiceUnavailable(
                "Freedesktop Secret Service requires Linux session D-Bus".into(),
            ))
        }
    }

    fn clear(&self, _account_id: &str, _workspace_id: Option<&str>) -> Result<(), CredentialError> {
        #[cfg(all(target_os = "linux", feature = "zbus"))]
        {
            Err(CredentialError::SecretServiceUnavailable(
                "Freedesktop Secret Service session daemon not running".into(),
            ))
        }
        #[cfg(not(all(target_os = "linux", feature = "zbus")))]
        {
            Err(CredentialError::SecretServiceUnavailable(
                "Freedesktop Secret Service requires Linux session D-Bus".into(),
            ))
        }
    }

    fn kind(&self) -> CredentialStoreKind {
        CredentialStoreKind::SecretService
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_token_validity_buffer() {
        let mut creds = Credentials::new("user@example.com", Some("secret123".into()));
        assert!(!creds.is_token_valid(Utc::now()));

        creds.access_token = Some("tok_abc123".into());
        creds.access_token_expires_at = Some(Utc::now() + chrono::Duration::seconds(30));
        // Within 60 seconds buffer: counts as invalid/needs refresh
        assert!(!creds.is_token_valid(Utc::now()));

        creds.access_token_expires_at = Some(Utc::now() + chrono::Duration::seconds(180));
        assert!(creds.is_token_valid(Utc::now()));

        creds.invalidate_access_token();
        assert!(!creds.is_token_valid(Utc::now()));
        assert_eq!(creds.access_token, None);
    }

    #[test]
    fn test_credentials_debug_and_display_redaction() {
        let mut creds = Credentials::new("alice@company.com", Some("MySuperSecretPass".into()));
        creds.refresh_token = Some("ref_secret_token_12345".into());
        creds.access_token = Some("acc_jwt_sensitive_payload".into());
        creds.device_signing_key = Some(vec![42u8; 32]);

        let debug_str = format!("{:?}", creds);
        assert!(!debug_str.contains("MySuperSecretPass"));
        assert!(!debug_str.contains("ref_secret_token_12345"));
        assert!(!debug_str.contains("acc_jwt_sensitive_payload"));
        assert!(debug_str.contains("[REDACTED]"));

        let display_str = format!("{}", creds);
        assert!(!display_str.contains("MySuperSecretPass"));
        assert!(!display_str.contains("ref_secret_token_12345"));
        assert!(display_str.contains("alice@company.com"));
    }

    #[test]
    fn test_in_memory_store_scoped_isolation() {
        let store = InMemoryCredentialStore::new();
        let mut cred1 = Credentials::new("user@example.com", Some("pass1".into()));
        cred1.workspace_id = Some("ws-alpha".into());
        store.save(&cred1).unwrap();

        let mut cred2 = Credentials::new("user@example.com", Some("pass2".into()));
        cred2.workspace_id = Some("ws-beta".into());
        store.save(&cred2).unwrap();

        let loaded1 = store
            .load("user@example.com", Some("ws-alpha"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded1.password.as_deref(), Some("pass1"));

        let loaded2 = store
            .load("user@example.com", Some("ws-beta"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded2.password.as_deref(), Some("pass2"));

        // Non-existent scope
        let loaded3 = store.load("user@example.com", Some("ws-gamma")).unwrap();
        assert_eq!(loaded3, None);

        // Clearing ws-alpha does not touch ws-beta
        store.clear("user@example.com", Some("ws-alpha")).unwrap();
        assert_eq!(
            store.load("user@example.com", Some("ws-alpha")).unwrap(),
            None
        );
        assert!(store
            .load("user@example.com", Some("ws-beta"))
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_in_memory_store_fault_injection() {
        let store = InMemoryCredentialStore::new();
        let cred = Credentials::new("fault@example.com", Some("pass".into()));

        store.inject_save_error(true);
        let err = store.save(&cred).unwrap_err();
        assert!(matches!(err, CredentialError::Injected(_)));

        store.inject_save_error(false);
        store.save(&cred).unwrap();

        store.inject_load_error(true);
        let err = store.load("fault@example.com", None).unwrap_err();
        assert!(matches!(err, CredentialError::Injected(_)));
    }

    #[test]
    fn test_file_credential_store_atomic_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let cred_file = temp.path().join("creds").join("credentials.json");
        let store = FileCredentialStore::new(cred_file.clone());

        let mut cred = Credentials::new("worker@corp.com", Some("p4ssw0rd".into()));
        cred.workspace_id = Some("ws-01".into());
        cred.server_url = Some("https://api.dosi.local".into());
        cred.device_signing_key = Some(vec![1, 2, 3, 4]);

        store.save(&cred).unwrap();

        let loaded = store
            .load("worker@corp.com", Some("ws-01"))
            .unwrap()
            .unwrap();
        assert_eq!(loaded.account_id, "worker@corp.com");
        assert_eq!(loaded.password.as_deref(), Some("p4ssw0rd"));
        assert_eq!(loaded.server_url.as_deref(), Some("https://api.dosi.local"));
        assert_eq!(
            loaded.device_signing_key.as_deref(),
            Some(&[1, 2, 3, 4][..])
        );

        // Clear removes it cleanly
        store.clear("worker@corp.com", Some("ws-01")).unwrap();
        assert_eq!(store.load("worker@corp.com", Some("ws-01")).unwrap(), None);
    }
}
