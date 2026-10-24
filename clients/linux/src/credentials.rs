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
use std::sync::{Arc, Mutex};

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

/// Transport abstraction for Freedesktop Secret Service over D-Bus (`org.freedesktop.secrets`).
/// Allows deterministic protocol-shaped testing without connecting to a real session bus or keyring.
pub trait SecretServiceTransport: Send + Sync + fmt::Debug {
    /// Search items matching the attributes dictionary.
    /// Returns (unlocked_item_paths, locked_item_paths).
    fn search_items(
        &self,
        attributes: &HashMap<String, String>,
    ) -> Result<(Vec<String>, Vec<String>), CredentialError>;

    /// Attempt to unlock the specified object paths.
    /// Returns (unlocked_paths, prompt_required).
    fn unlock(&self, objects: &[String]) -> Result<(Vec<String>, bool), CredentialError>;

    /// Open a "plain" session for secret transfer.
    /// Returns the session object path.
    fn open_session(&self) -> Result<String, CredentialError>;

    /// Retrieve the secret payload for an unlocked item path using an open session.
    fn get_secret(&self, item_path: &str, session_path: &str) -> Result<Vec<u8>, CredentialError>;

    /// Create or replace an item in the default collection.
    /// Returns the created item path, or fails if prompt required or service error.
    fn create_item(
        &self,
        label: &str,
        attributes: &HashMap<String, String>,
        session_path: &str,
        secret: &[u8],
        replace: bool,
    ) -> Result<String, CredentialError>;

    /// Delete an item by object path.
    fn delete_item(&self, item_path: &str) -> Result<(), CredentialError>;
}

/// Freedesktop Secret Service backend store over D-Bus (`org.freedesktop.secrets`).
#[derive(Clone, Debug)]
pub struct SecretServiceStore {
    application: String,
    transport: Arc<dyn SecretServiceTransport>,
}

impl SecretServiceStore {
    /// Create a new SecretServiceStore with default transport for the specified application.
    pub fn new(application: impl Into<String>) -> Self {
        let app = application.into();
        #[cfg(target_os = "linux")]
        {
            Self {
                application: app,
                transport: Arc::new(ZbusSecretServiceTransport::new()),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self {
                application: app,
                transport: Arc::new(UnsupportedSecretServiceTransport),
            }
        }
    }

    /// Construct a SecretServiceStore with an explicit transport seam for testing.
    pub fn with_transport(
        application: impl Into<String>,
        transport: Arc<dyn SecretServiceTransport>,
    ) -> Self {
        Self {
            application: application.into(),
            transport,
        }
    }

    /// Application name identifier used for scoping.
    pub fn application(&self) -> &str {
        &self.application
    }

    fn build_attributes(
        &self,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        attrs.insert("application".to_string(), self.application.clone());
        attrs.insert("account_id".to_string(), account_id.trim().to_lowercase());
        attrs.insert(
            "workspace_id".to_string(),
            workspace_id
                .map(|w| w.trim().to_string())
                .unwrap_or_default(),
        );
        attrs
    }
}

impl Default for SecretServiceStore {
    fn default() -> Self {
        Self::new("dosi-tracker")
    }
}

impl CredentialStore for SecretServiceStore {
    fn load(
        &self,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Option<Credentials>, CredentialError> {
        let norm_acc = account_id.trim().to_lowercase();
        let norm_ws = workspace_id.map(|w| w.trim().to_string());
        let attrs = self.build_attributes(&norm_acc, norm_ws.as_deref());

        let (mut unlocked, locked) = self.transport.search_items(&attrs)?;
        if !locked.is_empty() {
            let (newly_unlocked, prompt_required) = self.transport.unlock(&locked)?;
            if prompt_required {
                return Err(CredentialError::SecretServiceUnavailable(
                    "Freedesktop Secret Service item or collection is locked and requires interactive unlock prompt".into(),
                ));
            }
            unlocked.extend(newly_unlocked);
        }

        if unlocked.is_empty() {
            return Ok(None);
        }

        let session_path = self.transport.open_session()?;

        for item_path in &unlocked {
            let secret_bytes = self.transport.get_secret(item_path, &session_path)?;
            if secret_bytes.is_empty() {
                continue;
            }
            let creds: Credentials = serde_json::from_slice(&secret_bytes).map_err(|e| {
                CredentialError::Serialization(format!(
                    "Corrupt secret payload in Secret Service: {e}"
                ))
            })?;

            // Scoping validation: verify identity and workspace match query exactly
            let cred_acc = creds.account_id.trim().to_lowercase();
            let cred_ws = creds.workspace_id.as_ref().map(|w| w.trim().to_string());
            if cred_acc == norm_acc && cred_ws == norm_ws {
                return Ok(Some(creds));
            }
        }

        Ok(None)
    }

    fn save(&self, credentials: &Credentials) -> Result<(), CredentialError> {
        let norm_acc = credentials.account_id.trim().to_lowercase();
        let norm_ws = credentials
            .workspace_id
            .as_ref()
            .map(|w| w.trim().to_string());
        let attrs = self.build_attributes(&norm_acc, norm_ws.as_deref());

        let secret_bytes = serde_json::to_vec_pretty(credentials)
            .map_err(|e| CredentialError::Serialization(e.to_string()))?;

        let session_path = self.transport.open_session()?;

        let label = match &norm_ws {
            Some(ws) => format!("Dosi-Tracker ({norm_acc}/{ws})"),
            None => format!("Dosi-Tracker ({norm_acc})"),
        };

        self.transport.create_item(
            &label,
            &attrs,
            &session_path,
            &secret_bytes,
            true, // replace existing item with matching attributes
        )?;

        Ok(())
    }

    fn clear(&self, account_id: &str, workspace_id: Option<&str>) -> Result<(), CredentialError> {
        let norm_acc = account_id.trim().to_lowercase();
        let norm_ws = workspace_id.map(|w| w.trim().to_string());
        let attrs = self.build_attributes(&norm_acc, norm_ws.as_deref());

        let (mut unlocked, locked) = self.transport.search_items(&attrs)?;
        if !locked.is_empty() {
            let (newly_unlocked, prompt_required) = self.transport.unlock(&locked)?;
            if prompt_required {
                return Err(CredentialError::SecretServiceUnavailable(
                    "Freedesktop Secret Service item or collection is locked and requires interactive unlock prompt to clear".into(),
                ));
            }
            unlocked.extend(newly_unlocked);
        }

        for item_path in &unlocked {
            self.transport.delete_item(item_path)?;
        }

        Ok(())
    }

    fn kind(&self) -> CredentialStoreKind {
        CredentialStoreKind::SecretService
    }
}

/// Fallback transport for non-Linux targets where Freedesktop Secret Service D-Bus is unavailable.
#[derive(Debug)]
pub struct UnsupportedSecretServiceTransport;

impl SecretServiceTransport for UnsupportedSecretServiceTransport {
    fn search_items(
        &self,
        _attributes: &HashMap<String, String>,
    ) -> Result<(Vec<String>, Vec<String>), CredentialError> {
        Err(CredentialError::SecretServiceUnavailable(
            "Freedesktop Secret Service requires Linux session D-Bus".into(),
        ))
    }

    fn unlock(&self, _objects: &[String]) -> Result<(Vec<String>, bool), CredentialError> {
        Err(CredentialError::SecretServiceUnavailable(
            "Freedesktop Secret Service requires Linux session D-Bus".into(),
        ))
    }

    fn open_session(&self) -> Result<String, CredentialError> {
        Err(CredentialError::SecretServiceUnavailable(
            "Freedesktop Secret Service requires Linux session D-Bus".into(),
        ))
    }

    fn get_secret(
        &self,
        _item_path: &str,
        _session_path: &str,
    ) -> Result<Vec<u8>, CredentialError> {
        Err(CredentialError::SecretServiceUnavailable(
            "Freedesktop Secret Service requires Linux session D-Bus".into(),
        ))
    }

    fn create_item(
        &self,
        _label: &str,
        _attributes: &HashMap<String, String>,
        _session_path: &str,
        _secret: &[u8],
        _replace: bool,
    ) -> Result<String, CredentialError> {
        Err(CredentialError::SecretServiceUnavailable(
            "Freedesktop Secret Service requires Linux session D-Bus".into(),
        ))
    }

    fn delete_item(&self, _item_path: &str) -> Result<(), CredentialError> {
        Err(CredentialError::SecretServiceUnavailable(
            "Freedesktop Secret Service requires Linux session D-Bus".into(),
        ))
    }
}

/// In-memory, deterministic protocol-shaped fake Secret Service daemon for testing without a real session bus or keyring.
#[derive(Debug, Default)]
pub struct MockSecretServiceTransport {
    items: Mutex<HashMap<String, MockSecretItem>>,
    counter: std::sync::atomic::AtomicU64,
    pub prompt_required_on_unlock: AtomicBool,
    pub prompt_required_on_create: AtomicBool,
    pub prompt_required_on_delete: AtomicBool,
    pub daemon_available: AtomicBool,
}

#[derive(Clone, Debug)]
struct MockSecretItem {
    label: String,
    attributes: HashMap<String, String>,
    secret: Vec<u8>,
    locked: bool,
}

impl MockSecretServiceTransport {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
            counter: std::sync::atomic::AtomicU64::new(1),
            prompt_required_on_unlock: AtomicBool::new(false),
            prompt_required_on_create: AtomicBool::new(false),
            prompt_required_on_delete: AtomicBool::new(false),
            daemon_available: AtomicBool::new(true),
        }
    }

    pub fn insert_locked_item(
        &self,
        path: &str,
        label: &str,
        attributes: HashMap<String, String>,
        secret: Vec<u8>,
    ) {
        let mut map = self.items.lock().unwrap();
        map.insert(
            path.to_string(),
            MockSecretItem {
                label: label.to_string(),
                attributes,
                secret,
                locked: true,
            },
        );
    }

    pub fn insert_corrupted_item(
        &self,
        path: &str,
        attributes: HashMap<String, String>,
        corrupt_bytes: Vec<u8>,
    ) {
        let mut map = self.items.lock().unwrap();
        map.insert(
            path.to_string(),
            MockSecretItem {
                label: "Corrupted".to_string(),
                attributes,
                secret: corrupt_bytes,
                locked: false,
            },
        );
    }
}

impl SecretServiceTransport for MockSecretServiceTransport {
    fn search_items(
        &self,
        attributes: &HashMap<String, String>,
    ) -> Result<(Vec<String>, Vec<String>), CredentialError> {
        if !self.daemon_available.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "org.freedesktop.secrets D-Bus daemon is unavailable".into(),
            ));
        }
        let map = self.items.lock().unwrap();
        let mut unlocked = Vec::new();
        let mut locked = Vec::new();
        for (path, item) in map.iter() {
            let mut matches = true;
            for (k, v) in attributes {
                if item.attributes.get(k) != Some(v) {
                    matches = false;
                    break;
                }
            }
            if matches {
                if item.locked {
                    locked.push(path.clone());
                } else {
                    unlocked.push(path.clone());
                }
            }
        }
        unlocked.sort();
        locked.sort();
        Ok((unlocked, locked))
    }

    fn unlock(&self, objects: &[String]) -> Result<(Vec<String>, bool), CredentialError> {
        if !self.daemon_available.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "org.freedesktop.secrets D-Bus daemon is unavailable".into(),
            ));
        }
        if self.prompt_required_on_unlock.load(Ordering::Relaxed) {
            return Ok((Vec::new(), true));
        }
        let mut map = self.items.lock().unwrap();
        let mut newly_unlocked = Vec::new();
        for path in objects {
            if let Some(item) = map.get_mut(path) {
                item.locked = false;
                newly_unlocked.push(path.clone());
            }
        }
        Ok((newly_unlocked, false))
    }

    fn open_session(&self) -> Result<String, CredentialError> {
        if !self.daemon_available.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "org.freedesktop.secrets D-Bus daemon is unavailable".into(),
            ));
        }
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        Ok(format!("/org/freedesktop/secrets/session/{id}"))
    }

    fn get_secret(&self, item_path: &str, _session_path: &str) -> Result<Vec<u8>, CredentialError> {
        if !self.daemon_available.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "org.freedesktop.secrets D-Bus daemon is unavailable".into(),
            ));
        }
        let map = self.items.lock().unwrap();
        let item = map.get(item_path).ok_or_else(|| {
            CredentialError::SecretServiceUnavailable(format!(
                "Secret item {item_path} not found in Secret Service"
            ))
        })?;
        if item.locked {
            return Err(CredentialError::SecretServiceUnavailable(format!(
                "Secret item {item_path} is locked"
            )));
        }
        Ok(item.secret.clone())
    }

    fn create_item(
        &self,
        label: &str,
        attributes: &HashMap<String, String>,
        _session_path: &str,
        secret: &[u8],
        replace: bool,
    ) -> Result<String, CredentialError> {
        if !self.daemon_available.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "org.freedesktop.secrets D-Bus daemon is unavailable".into(),
            ));
        }
        if self.prompt_required_on_create.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "Creating Secret Service item requires interactive unlock prompt".into(),
            ));
        }
        let mut map = self.items.lock().unwrap();
        if replace {
            // Find existing matching item
            for (path, item) in map.iter_mut() {
                let mut matches = true;
                for (k, v) in attributes {
                    if item.attributes.get(k) != Some(v) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    item.label = label.to_string();
                    item.secret = secret.to_vec();
                    item.locked = false;
                    return Ok(path.clone());
                }
            }
        }
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let path = format!("/org/freedesktop/secrets/collection/default/{id}");
        map.insert(
            path.clone(),
            MockSecretItem {
                label: label.to_string(),
                attributes: attributes.clone(),
                secret: secret.to_vec(),
                locked: false,
            },
        );
        Ok(path)
    }

    fn delete_item(&self, item_path: &str) -> Result<(), CredentialError> {
        if !self.daemon_available.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "org.freedesktop.secrets D-Bus daemon is unavailable".into(),
            ));
        }
        if self.prompt_required_on_delete.load(Ordering::Relaxed) {
            return Err(CredentialError::SecretServiceUnavailable(
                "Deleting Secret Service item requires interactive prompt".into(),
            ));
        }
        let mut map = self.items.lock().unwrap();
        map.remove(item_path);
        Ok(())
    }
}

/// Real Freedesktop Secret Service transport connecting over session D-Bus.
#[cfg(target_os = "linux")]
use zbus::zvariant;

#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
pub struct ZbusSecretServiceTransport;

#[cfg(target_os = "linux")]
impl ZbusSecretServiceTransport {
    pub fn new() -> Self {
        Self
    }

    fn connection(&self) -> Result<zbus::blocking::Connection, CredentialError> {
        zbus::blocking::Connection::session().map_err(|e| {
            CredentialError::SecretServiceUnavailable(format!(
                "Failed connecting to session D-Bus: {e}"
            ))
        })
    }
}

#[cfg(target_os = "linux")]
impl SecretServiceTransport for ZbusSecretServiceTransport {
    fn search_items(
        &self,
        attributes: &HashMap<String, String>,
    ) -> Result<(Vec<String>, Vec<String>), CredentialError> {
        let conn = self.connection()?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.secrets"),
                "/org/freedesktop/secrets",
                Some("org.freedesktop.Secret.Service"),
                "SearchItems",
                &(attributes,),
            )
            .map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Secret Service SearchItems call failed: {e}"
                ))
            })?;

        let (unlocked, locked): (
            Vec<zvariant::OwnedObjectPath>,
            Vec<zvariant::OwnedObjectPath>,
        ) = reply.body().deserialize().map_err(|e| {
            CredentialError::SecretServiceUnavailable(format!(
                "Failed deserializing SearchItems response: {e}"
            ))
        })?;

        Ok((
            unlocked.into_iter().map(|p| p.to_string()).collect(),
            locked.into_iter().map(|p| p.to_string()).collect(),
        ))
    }

    fn unlock(&self, objects: &[String]) -> Result<(Vec<String>, bool), CredentialError> {
        let conn = self.connection()?;
        let obj_paths: Vec<zvariant::OwnedObjectPath> = objects
            .iter()
            .filter_map(|p| zvariant::OwnedObjectPath::try_from(p.as_str()).ok())
            .collect();

        let reply = conn
            .call_method(
                Some("org.freedesktop.secrets"),
                "/org/freedesktop/secrets",
                Some("org.freedesktop.Secret.Service"),
                "Unlock",
                &(obj_paths,),
            )
            .map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Secret Service Unlock call failed: {e}"
                ))
            })?;

        let (unlocked, prompt): (Vec<zvariant::OwnedObjectPath>, zvariant::OwnedObjectPath) =
            reply.body().deserialize().map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Failed deserializing Unlock response: {e}"
                ))
            })?;

        let prompt_required = prompt.as_str() != "/";
        Ok((
            unlocked.into_iter().map(|p| p.to_string()).collect(),
            prompt_required,
        ))
    }

    fn open_session(&self) -> Result<String, CredentialError> {
        let conn = self.connection()?;
        let input_val = zvariant::Value::from("");
        let reply = conn
            .call_method(
                Some("org.freedesktop.secrets"),
                "/org/freedesktop/secrets",
                Some("org.freedesktop.Secret.Service"),
                "OpenSession",
                &("plain", &input_val),
            )
            .map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Secret Service OpenSession call failed: {e}"
                ))
            })?;

        let (_output, session_path): (zvariant::OwnedValue, zvariant::OwnedObjectPath) =
            reply.body().deserialize().map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Failed deserializing OpenSession response: {e}"
                ))
            })?;

        Ok(session_path.to_string())
    }

    fn get_secret(&self, item_path: &str, session_path: &str) -> Result<Vec<u8>, CredentialError> {
        let conn = self.connection()?;
        let sess_obj = zvariant::OwnedObjectPath::try_from(session_path).map_err(|e| {
            CredentialError::SecretServiceUnavailable(format!("Invalid session object path: {e}"))
        })?;

        let reply = conn
            .call_method(
                Some("org.freedesktop.secrets"),
                item_path,
                Some("org.freedesktop.Secret.Item"),
                "GetSecret",
                &(sess_obj,),
            )
            .map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Secret Service GetSecret call failed for {item_path}: {e}"
                ))
            })?;

        let (_session, _params, secret_bytes, _content_type): (
            zvariant::OwnedObjectPath,
            Vec<u8>,
            Vec<u8>,
            String,
        ) = reply.body().deserialize().map_err(|e| {
            CredentialError::SecretServiceUnavailable(format!(
                "Failed deserializing Secret struct from GetSecret: {e}"
            ))
        })?;

        Ok(secret_bytes)
    }

    fn create_item(
        &self,
        label: &str,
        attributes: &HashMap<String, String>,
        session_path: &str,
        secret: &[u8],
        replace: bool,
    ) -> Result<String, CredentialError> {
        let conn = self.connection()?;
        let sess_obj = zvariant::OwnedObjectPath::try_from(session_path).map_err(|e| {
            CredentialError::SecretServiceUnavailable(format!("Invalid session object path: {e}"))
        })?;

        let mut prop_dict: HashMap<&str, zvariant::Value> = HashMap::new();
        prop_dict.insert(
            "org.freedesktop.Secret.Item.Label",
            zvariant::Value::from(label),
        );
        let attr_val = zvariant::Value::from(attributes.clone());
        prop_dict.insert("org.freedesktop.Secret.Item.Attributes", attr_val);

        let empty_params: Vec<u8> = Vec::new();
        let secret_struct = (
            sess_obj,
            empty_params,
            secret.to_vec(),
            "application/json".to_string(),
        );

        let reply = conn
            .call_method(
                Some("org.freedesktop.secrets"),
                "/org/freedesktop/secrets/aliases/default",
                Some("org.freedesktop.Secret.Collection"),
                "CreateItem",
                &(prop_dict, secret_struct, replace),
            )
            .map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Secret Service CreateItem call failed: {e}"
                ))
            })?;

        let (item_path, prompt): (zvariant::OwnedObjectPath, zvariant::OwnedObjectPath) =
            reply.body().deserialize().map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Failed deserializing CreateItem response: {e}"
                ))
            })?;

        if prompt.as_str() != "/" {
            return Err(CredentialError::SecretServiceUnavailable(
                "Creating Secret Service item requires interactive unlock prompt".into(),
            ));
        }

        Ok(item_path.to_string())
    }

    fn delete_item(&self, item_path: &str) -> Result<(), CredentialError> {
        let conn = self.connection()?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.secrets"),
                item_path,
                Some("org.freedesktop.Secret.Item"),
                "Delete",
                &(),
            )
            .map_err(|e| {
                CredentialError::SecretServiceUnavailable(format!(
                    "Secret Service Delete call failed for {item_path}: {e}"
                ))
            })?;

        let prompt: zvariant::OwnedObjectPath = reply.body().deserialize().map_err(|e| {
            CredentialError::SecretServiceUnavailable(format!(
                "Failed deserializing Delete response: {e}"
            ))
        })?;

        if prompt.as_str() != "/" {
            return Err(CredentialError::SecretServiceUnavailable(
                "Deleting Secret Service item requires interactive prompt".into(),
            ));
        }

        Ok(())
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

    #[test]
    fn test_file_credential_store_validation_and_ownership_invariants() {
        let temp = tempfile::tempdir().unwrap();
        let cred_file = temp.path().join("creds").join("credentials.json");
        let store = FileCredentialStore::new(cred_file.clone());

        // 1. Non-existent file validates as clean
        assert!(FileCredentialStore::validate_target_file(&cred_file).is_ok());

        // 2. Directory as target file fails closed
        let dir_target = temp.path().join("creds_dir");
        fs::create_dir_all(&dir_target).unwrap();
        let err = FileCredentialStore::validate_target_file(&dir_target).unwrap_err();
        assert!(matches!(err, CredentialError::UnsafeStorage(_)));
        assert!(err.to_string().contains("not a regular file"));

        // 3. Save valid credentials
        let mut cred = Credentials::new("admin@corp.com", Some("secret".into()));
        cred.workspace_id = Some("ws-test".into());
        store.save(&cred).unwrap();
        assert!(FileCredentialStore::validate_target_file(&cred_file).is_ok());

        // 4. Unix-specific invariants: permissions and UID ownership
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::fs::PermissionsExt;

            let meta = fs::symlink_metadata(&cred_file).unwrap();
            let current_uid = unsafe { libc::geteuid() };
            assert_eq!(
                meta.uid(),
                current_uid,
                "Credentials file must be owned by current effective UID"
            );
            assert_eq!(
                meta.mode() & 0o777,
                0o600,
                "Credentials file must have 0600 permissions"
            );

            // Widen permissions to group/world writable -> validation must fail closed
            fs::set_permissions(&cred_file, fs::Permissions::from_mode(0o666)).unwrap();
            let err = FileCredentialStore::validate_target_file(&cred_file).unwrap_err();
            assert!(matches!(err, CredentialError::UnsafeStorage(_)));
            assert!(err.to_string().contains("unsafe permissions"));

            // Restore 0600 permissions -> validation succeeds again
            fs::set_permissions(&cred_file, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(FileCredentialStore::validate_target_file(&cred_file).is_ok());
        }
    }

    #[test]
    fn test_secret_service_store_scoped_crud_and_cross_scope_isolation() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        assert_eq!(store.application(), "dosi-tracker");
        assert_eq!(store.kind(), CredentialStoreKind::SecretService);

        // 1. Save Alice in ws-alpha
        let mut alice_alpha = Credentials::new("alice@corp.com", Some("pass-alpha".into()));
        alice_alpha.workspace_id = Some("ws-alpha".into());
        alice_alpha.access_token = Some("tok-alpha-123".into());
        alice_alpha.server_url = Some("https://api.dosi.local".into());
        alice_alpha.device_signing_key = Some(vec![10, 20, 30]);
        store
            .save(&alice_alpha)
            .expect("Save Alice ws-alpha must succeed");

        // 2. Save Alice in ws-beta
        let mut alice_beta = Credentials::new("alice@corp.com", Some("pass-beta".into()));
        alice_beta.workspace_id = Some("ws-beta".into());
        alice_beta.access_token = Some("tok-beta-456".into());
        store
            .save(&alice_beta)
            .expect("Save Alice ws-beta must succeed");

        // 3. Save Bob in ws-alpha
        let mut bob_alpha = Credentials::new("bob@corp.com", Some("pass-bob".into()));
        bob_alpha.workspace_id = Some("ws-alpha".into());
        bob_alpha.access_token = Some("tok-bob-789".into());
        store
            .save(&bob_alpha)
            .expect("Save Bob ws-alpha must succeed");

        // 4. Verify exact loading for each scope
        let loaded_a_alpha = store
            .load("alice@corp.com", Some("ws-alpha"))
            .unwrap()
            .expect("Alice ws-alpha must exist");
        assert_eq!(loaded_a_alpha.password.as_deref(), Some("pass-alpha"));
        assert_eq!(
            loaded_a_alpha.access_token.as_deref(),
            Some("tok-alpha-123")
        );
        assert_eq!(
            loaded_a_alpha.device_signing_key.as_deref(),
            Some(&[10, 20, 30][..])
        );

        let loaded_a_beta = store
            .load("alice@corp.com", Some("ws-beta"))
            .unwrap()
            .expect("Alice ws-beta must exist");
        assert_eq!(loaded_a_beta.password.as_deref(), Some("pass-beta"));
        assert_eq!(loaded_a_beta.access_token.as_deref(), Some("tok-beta-456"));

        let loaded_b_alpha = store
            .load("bob@corp.com", Some("ws-alpha"))
            .unwrap()
            .expect("Bob ws-alpha must exist");
        assert_eq!(loaded_b_alpha.password.as_deref(), Some("pass-bob"));
        assert_eq!(loaded_b_alpha.access_token.as_deref(), Some("tok-bob-789"));

        // 5. Querying non-existent scope or identity returns None
        assert_eq!(
            store.load("alice@corp.com", Some("ws-gamma")).unwrap(),
            None
        );
        assert_eq!(
            store.load("charlie@corp.com", Some("ws-alpha")).unwrap(),
            None
        );

        // 6. Case insensitive and trimmed matching consistency
        let loaded_case = store
            .load("  ALICE@CORP.COM  ", Some(" ws-alpha "))
            .unwrap()
            .expect("Case-insensitive lookup must find record");
        assert_eq!(loaded_case.password.as_deref(), Some("pass-alpha"));

        // 7. Update Alice in ws-alpha
        alice_alpha.access_token = Some("tok-alpha-updated".into());
        store
            .save(&alice_alpha)
            .expect("Update Alice ws-alpha must succeed");
        let loaded_updated = store
            .load("alice@corp.com", Some("ws-alpha"))
            .unwrap()
            .expect("Updated record must exist");
        assert_eq!(
            loaded_updated.access_token.as_deref(),
            Some("tok-alpha-updated")
        );

        // 8. Clear Alice in ws-alpha: must NOT delete Alice in ws-beta or Bob in ws-alpha
        store
            .clear("alice@corp.com", Some("ws-alpha"))
            .expect("Clear must succeed");
        assert_eq!(
            store.load("alice@corp.com", Some("ws-alpha")).unwrap(),
            None
        );

        // Cross-scope preservation
        let still_beta = store
            .load("alice@corp.com", Some("ws-beta"))
            .unwrap()
            .expect("Alice ws-beta must still exist after clearing ws-alpha");
        assert_eq!(still_beta.access_token.as_deref(), Some("tok-beta-456"));

        let still_bob = store
            .load("bob@corp.com", Some("ws-alpha"))
            .unwrap()
            .expect("Bob ws-alpha must still exist after clearing Alice");
        assert_eq!(still_bob.access_token.as_deref(), Some("tok-bob-789"));
    }

    #[test]
    fn test_secret_service_store_locked_item_or_prompt_fails_closed() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        let mut attrs = HashMap::new();
        attrs.insert("application".into(), "dosi-tracker".into());
        attrs.insert("account_id".into(), "locked@corp.com".into());
        attrs.insert("workspace_id".into(), "ws-locked".into());

        // Insert locked item
        mock.insert_locked_item(
            "/org/freedesktop/secrets/collection/default/locked_1",
            "Dosi-Tracker (locked@corp.com/ws-locked)",
            attrs,
            br#"{"account_id":"locked@corp.com","workspace_id":"ws-locked"}"#.to_vec(),
        );

        // Case A: Unlock prompt is required -> fails closed with SecretServiceUnavailable
        mock.prompt_required_on_unlock
            .store(true, Ordering::Relaxed);
        let err = store
            .load("locked@corp.com", Some("ws-locked"))
            .unwrap_err();
        assert!(matches!(err, CredentialError::SecretServiceUnavailable(_)));
        assert!(err
            .to_string()
            .contains("requires interactive unlock prompt"));

        // Case B: Unlock prompt not required -> unlocks cleanly and loads item
        mock.prompt_required_on_unlock
            .store(false, Ordering::Relaxed);
        let creds = store
            .load("locked@corp.com", Some("ws-locked"))
            .unwrap()
            .expect("Item should be unlocked and loaded");
        assert_eq!(creds.account_id, "locked@corp.com");
    }

    #[test]
    fn test_secret_service_store_create_prompt_fails_closed() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        mock.prompt_required_on_create
            .store(true, Ordering::Relaxed);

        let cred = Credentials::new("user@corp.com", Some("pass".into()));
        let err = store.save(&cred).unwrap_err();
        assert!(matches!(err, CredentialError::SecretServiceUnavailable(_)));
        assert!(err
            .to_string()
            .contains("requires interactive unlock prompt"));
    }

    #[test]
    fn test_secret_service_store_delete_prompt_fails_closed() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        let cred = Credentials::new("user@corp.com", Some("pass".into()));
        store.save(&cred).unwrap();

        mock.prompt_required_on_delete
            .store(true, Ordering::Relaxed);
        let err = store.clear("user@corp.com", None).unwrap_err();
        assert!(matches!(err, CredentialError::SecretServiceUnavailable(_)));
        assert!(err.to_string().contains("requires interactive prompt"));
    }

    #[test]
    fn test_secret_service_store_daemon_unavailable_fails_closed() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        mock.daemon_available.store(false, Ordering::Relaxed);

        let cred = Credentials::new("user@corp.com", Some("pass".into()));
        let save_err = store.save(&cred).unwrap_err();
        assert!(matches!(
            save_err,
            CredentialError::SecretServiceUnavailable(_)
        ));

        let load_err = store.load("user@corp.com", None).unwrap_err();
        assert!(matches!(
            load_err,
            CredentialError::SecretServiceUnavailable(_)
        ));

        let clear_err = store.clear("user@corp.com", None).unwrap_err();
        assert!(matches!(
            clear_err,
            CredentialError::SecretServiceUnavailable(_)
        ));
    }

    #[test]
    fn test_secret_service_store_corrupted_payload_fails_closed() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        let mut attrs = HashMap::new();
        attrs.insert("application".into(), "dosi-tracker".into());
        attrs.insert("account_id".into(), "corrupt@corp.com".into());
        attrs.insert("workspace_id".into(), "ws-corrupt".into());

        mock.insert_corrupted_item(
            "/org/freedesktop/secrets/collection/default/corrupt_1",
            attrs,
            b"NOT_VALID_JSON{{{}}}".to_vec(),
        );

        let err = store
            .load("corrupt@corp.com", Some("ws-corrupt"))
            .unwrap_err();
        assert!(matches!(err, CredentialError::Serialization(_)));
        assert!(err.to_string().contains("Corrupt secret payload"));
    }

    #[test]
    fn test_secret_service_store_zero_leakage_in_diagnostics_and_errors() {
        let mock = Arc::new(MockSecretServiceTransport::new());
        let store = SecretServiceStore::with_transport("dosi-tracker", mock.clone());

        mock.prompt_required_on_create
            .store(true, Ordering::Relaxed);

        let mut cred = Credentials::new("victim@corp.com", Some("SUPER_SENSITIVE_SECRET".into()));
        cred.access_token = Some("ACCESS_TOKEN_SECRET_999".into());
        cred.device_signing_key = Some(vec![9, 9, 9, 9]);

        let err = store.save(&cred).unwrap_err();
        let err_msg = err.to_string();
        assert!(!err_msg.contains("SUPER_SENSITIVE_SECRET"));
        assert!(!err_msg.contains("ACCESS_TOKEN_SECRET_999"));

        let debug_msg = format!("{store:?}");
        assert!(!debug_msg.contains("SUPER_SENSITIVE_SECRET"));
        assert!(!debug_msg.contains("ACCESS_TOKEN_SECRET_999"));
    }

    #[test]
    fn test_secret_service_store_constructor_and_all_targets_guard() {
        let store = SecretServiceStore::new("dosi-tracker");
        assert_eq!(store.application(), "dosi-tracker");
        assert_eq!(store.kind(), CredentialStoreKind::SecretService);

        let default_store = SecretServiceStore::default();
        assert_eq!(default_store.application(), "dosi-tracker");

        #[cfg(not(target_os = "linux"))]
        {
            let res = store.load("user@example.com", None);
            assert!(matches!(
                res.unwrap_err(),
                CredentialError::SecretServiceUnavailable(_)
            ));
        }
    }
}
