//! Durable, sensor-free activity outbox for offline-first Linux diagnostic foundation.
//!
//! Provides transactional local persistence for activity blocks awaiting eventual
//! synchronization with the Dosi-Tracker backend.
//!
//! Key Invariants:
//! - Strictly scoped by `(account_id, workspace_id)`. Entries from different workspaces
//!   or user accounts are isolated; identical activity IDs in different scopes do not collide.
//! - Bounded metadata on all public boundaries: Account IDs, workspace IDs, client activity IDs,
//!   diagnostic messages, and numeric limits have explicit finite byte bounds. Bypassing constructors
//!   via direct struct instantiation or deserialization is caught on every API boundary.
//! - Idempotent re-enqueue: Re-submitting identical payload with the same identity succeeds
//!   without creating duplicate entries.
//! - Conflict safety: Re-submitting different content under an already-queued identity
//!   fails closed with [`OutboxError::ConflictingIdentity`], leaving the original intact.
//! - Unmeasured metrics preserved: Unavailable metrics (e.g. Wayland input or window capture)
//!   are explicitly retained as `None` rather than fabricating measured zeros.
//! - No credentials or captures: Raw tokens, passwords, screenshots, and webcam frames
//!   are strictly forbidden from the outbox envelope.
//! - Explicit bounds enforcement: Queue cardinality and payload size limits are enforced
//!   before mutation; overflows are rejected explicitly rather than silently dropped.
//! - Coherent store versioning: Stamped with SQLite `PRAGMA user_version`. Future store
//!   versions fail closed with [`OutboxError::UnsupportedStoreVersion`] without auto-downgrade.
//! - Fail-closed corruption recovery: Corrupt database files or malformed persisted rows fail
//!   closed without destructive resets, deleting user data, or fabricating fallback timestamps.
//! - Transparent lookup errors: Row lookups use `OptionalExtension::optional` so genuine
//!   database errors propagate immediately rather than being masked as absent rows.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

/// Current schema version for outbox storage.
pub const OUTBOX_SCHEMA_VERSION: u32 = 1;

/// Maximum byte length for account identifier strings (256 bytes).
pub const MAX_ACCOUNT_ID_BYTES: usize = 256;

/// Maximum byte length for workspace identifier strings (256 bytes).
pub const MAX_WORKSPACE_ID_BYTES: usize = 256;

/// Maximum byte length for client activity identifier strings (256 bytes).
pub const MAX_CLIENT_ACTIVITY_ID_BYTES: usize = 256;

/// Maximum byte length for retry diagnostic error messages (1024 bytes).
pub const MAX_ERROR_MESSAGE_BYTES: usize = 1024;

/// Scope identifying the user account and workspace for an outbox entry.
///
/// Both identifiers are normalized and required to be non-empty and bounded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutboxScope {
    pub account_id: String,
    pub workspace_id: String,
}

impl OutboxScope {
    /// Creates a new validated outbox scope.
    ///
    /// Account IDs are case-insensitively normalized (trimmed and lowercased).
    /// Workspace IDs are trimmed.
    pub fn new(
        account_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Result<Self, OutboxError> {
        let account = account_id.into().trim().to_lowercase();
        let workspace = workspace_id.into().trim().to_string();

        let scope = Self {
            account_id: account,
            workspace_id: workspace,
        };
        scope.validate()?;
        Ok(scope)
    }

    /// Validates scope fields against length, empty, and control character invariants.
    ///
    /// Called on all public API boundaries to enforce constraints even if the struct
    /// was instantiated directly or deserialized.
    pub fn validate(&self) -> Result<(), OutboxError> {
        if self.account_id.trim().is_empty() {
            return Err(OutboxError::InvalidScope(
                "account_id cannot be empty".into(),
            ));
        }
        if self.account_id.len() > MAX_ACCOUNT_ID_BYTES {
            return Err(OutboxError::InvalidScope(format!(
                "account_id exceeds maximum length of {MAX_ACCOUNT_ID_BYTES} bytes"
            )));
        }
        if self.account_id.chars().any(|c| c.is_control()) {
            return Err(OutboxError::InvalidScope(
                "account_id cannot contain control characters".into(),
            ));
        }

        if self.workspace_id.trim().is_empty() {
            return Err(OutboxError::InvalidScope(
                "workspace_id cannot be empty".into(),
            ));
        }
        if self.workspace_id.len() > MAX_WORKSPACE_ID_BYTES {
            return Err(OutboxError::InvalidScope(format!(
                "workspace_id exceeds maximum length of {MAX_WORKSPACE_ID_BYTES} bytes"
            )));
        }
        if self.workspace_id.chars().any(|c| c.is_control()) {
            return Err(OutboxError::InvalidScope(
                "workspace_id cannot contain control characters".into(),
            ));
        }

        Ok(())
    }
}

/// Versioned activity envelope queued for eventual synchronization.
///
/// Designed to align with the backend `CreateActivityDto` contract while strictly
/// excluding credentials and bulk capture media (screenshots/webcam).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEnvelope {
    /// Envelope format version for compatibility migrations.
    pub format_version: u32,
    /// Stable client-generated activity block ID.
    pub client_activity_id: String,
    /// Target project ID, if tracking against a specific project.
    pub project_id: Option<String>,
    /// Activity block interval start (UTC).
    pub started_at: DateTime<Utc>,
    /// Activity block interval end (UTC).
    pub ended_at: DateTime<Utc>,
    /// Activity / productivity percentage [0, 100].
    /// `None` indicates input capture was unavailable, denied, or not measured.
    /// Never fabricate 0 for unmeasured intervals.
    pub productivity: Option<u32>,
    /// Optional measured mouse click count.
    /// `None` indicates click counting was unavailable or unpermitted.
    pub mouse_clicks: Option<u32>,
    /// Optional measured keyboard hit count.
    /// `None` indicates keypress detection was unavailable or unpermitted.
    pub keyboard_hits: Option<u32>,
    /// Optional task note / description.
    pub description: Option<String>,
    /// Client device identifier.
    pub device_id: Option<String>,
    /// Operating system platform identifier (e.g. "linux").
    pub platform: String,
    /// Client agent version (e.g. "0.1.0").
    pub agent_version: String,
}

impl ActivityEnvelope {
    pub const CURRENT_FORMAT_VERSION: u32 = 1;

    /// Validates envelope constraints before acceptance.
    pub fn validate(&self) -> Result<(), OutboxError> {
        if self.format_version != Self::CURRENT_FORMAT_VERSION {
            return Err(OutboxError::UnsupportedFormatVersion(self.format_version));
        }
        if self.client_activity_id.trim().is_empty() {
            return Err(OutboxError::InvalidEnvelope(
                "client_activity_id cannot be empty".into(),
            ));
        }
        if self.client_activity_id.len() > MAX_CLIENT_ACTIVITY_ID_BYTES {
            return Err(OutboxError::InvalidEnvelope(format!(
                "client_activity_id exceeds maximum length of {MAX_CLIENT_ACTIVITY_ID_BYTES} bytes"
            )));
        }
        if self.client_activity_id.chars().any(|c| c.is_control()) {
            return Err(OutboxError::InvalidEnvelope(
                "client_activity_id cannot contain control characters".into(),
            ));
        }
        if let Some(ref proj_id) = self.project_id {
            if proj_id.len() > MAX_CLIENT_ACTIVITY_ID_BYTES
                || proj_id.chars().any(|c| c.is_control())
            {
                return Err(OutboxError::InvalidEnvelope(
                    "project_id contains invalid characters or exceeds maximum length".into(),
                ));
            }
        }
        if self.started_at > self.ended_at {
            return Err(OutboxError::InvalidEnvelope(
                "started_at cannot be after ended_at".into(),
            ));
        }
        if let Some(prod) = self.productivity {
            if prod > 100 {
                return Err(OutboxError::InvalidEnvelope(
                    "productivity percentage cannot exceed 100".into(),
                ));
            }
        }
        if self.platform.len() > 64 || self.agent_version.len() > 64 {
            return Err(OutboxError::InvalidEnvelope(
                "platform or agent_version exceeds maximum length".into(),
            ));
        }
        if let Some(ref desc) = self.description {
            if desc.len() > 2048 {
                return Err(OutboxError::InvalidEnvelope(
                    "description exceeds maximum length of 2048 bytes".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Bounded capacity and size limits for the outbox.
///
/// NOTE: `max_total_payload_bytes` bounds envelope payload JSON storage across all queued
/// rows; it is not a complete disk-space guarantee because database file allocation also
/// includes SQLite page headers, B-tree indexes, WAL journals, and bounded metadata columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxLimits {
    /// Maximum number of queued records across all scopes.
    pub max_records: u32,
    /// Maximum payload size in bytes for a single envelope.
    pub max_payload_bytes: usize,
    /// Maximum total payload bytes across all queued records.
    pub max_total_payload_bytes: u64,
    /// Default page limit for bounded reads.
    pub default_page_limit: usize,
    /// Maximum page limit for bounded reads.
    pub max_page_limit: usize,
}

impl Default for OutboxLimits {
    fn default() -> Self {
        Self {
            max_records: 5_000,
            max_payload_bytes: 64 * 1024,              // 64 KiB
            max_total_payload_bytes: 32 * 1024 * 1024, // 32 MiB
            default_page_limit: 50,
            max_page_limit: 200,
        }
    }
}

impl OutboxLimits {
    /// Validates that numeric limits are positive and coherently bounded.
    pub fn validate(&self) -> Result<(), OutboxError> {
        if self.max_records == 0 {
            return Err(OutboxError::InvalidLimits(
                "max_records must be greater than zero".into(),
            ));
        }
        if self.max_payload_bytes == 0 {
            return Err(OutboxError::InvalidLimits(
                "max_payload_bytes must be greater than zero".into(),
            ));
        }
        if self.max_total_payload_bytes < self.max_payload_bytes as u64 {
            return Err(OutboxError::InvalidLimits(
                "max_total_payload_bytes cannot be less than max_payload_bytes".into(),
            ));
        }
        if self.max_page_limit == 0 || self.max_page_limit > (i64::MAX as usize) {
            return Err(OutboxError::InvalidLimits(
                "max_page_limit must be greater than zero and within positive integer range".into(),
            ));
        }
        if self.default_page_limit == 0 || self.default_page_limit > self.max_page_limit {
            return Err(OutboxError::InvalidLimits(
                "default_page_limit must be greater than zero and <= max_page_limit".into(),
            ));
        }
        Ok(())
    }
}

/// An entry retrieved from the outbox queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// Database record ID.
    pub id: i64,
    /// Tenant and user scope.
    pub scope: OutboxScope,
    /// Client activity identifier.
    pub client_activity_id: String,
    /// Timestamp when enqueued into the outbox.
    pub enqueued_at: DateTime<Utc>,
    /// Number of delivery attempts that failed.
    pub retry_count: u32,
    /// Last delivery failure reason, if any.
    pub last_error: Option<String>,
    /// The decoded activity envelope.
    pub envelope: ActivityEnvelope,
}

/// Result of an enqueue operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    /// Newly inserted record with allocated row ID.
    Inserted(i64),
    /// Identical record was already queued in this scope; handled idempotently.
    Idempotent(i64),
}

/// Errors returned by the durable outbox.
#[derive(Debug)]
pub enum OutboxError {
    /// Invalid tenant or account scope.
    InvalidScope(String),
    /// Invalid envelope content.
    InvalidEnvelope(String),
    /// Invalid queue configuration limits.
    InvalidLimits(String),
    /// Unsupported envelope format version.
    UnsupportedFormatVersion(u32),
    /// Unsupported database store schema version.
    UnsupportedStoreVersion(u32),
    /// Single envelope payload exceeds configured byte limit.
    PayloadTooLarge { size: usize, max: usize },
    /// Diagnostic error message exceeds configured byte limit.
    DiagnosticMessageTooLarge { size: usize, max: usize },
    /// Total record count exceeds queue cardinality limit.
    QueueFull { count: u32, max: u32 },
    /// Total storage quota exceeded.
    QuotaExceeded {
        current_bytes: u64,
        incoming_bytes: usize,
        max_bytes: u64,
    },
    /// Enqueue failed because a conflicting payload is already queued with this ID.
    ConflictingIdentity { client_activity_id: String },
    /// Operation targeted an entry that does not exist in the specified scope.
    RecordNotFound { client_activity_id: String },
    /// Persisted record is corrupted or semantically invalid.
    CorruptRecord { id: i64, reason: String },
    /// Database file is corrupt or invalid.
    CorruptDatabase(String),
    /// SQLite storage error.
    Database(rusqlite::Error),
    /// JSON serialization or deserialization failure.
    Serialization(serde_json::Error),
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScope(msg) => write!(f, "Invalid outbox scope: {msg}"),
            Self::InvalidEnvelope(msg) => write!(f, "Invalid activity envelope: {msg}"),
            Self::InvalidLimits(msg) => write!(f, "Invalid outbox limits: {msg}"),
            Self::UnsupportedFormatVersion(v) => write!(f, "Unsupported format version: {v}"),
            Self::UnsupportedStoreVersion(v) => {
                write!(
                    f,
                    "Unsupported outbox store version: {v} (current supported version is {OUTBOX_SCHEMA_VERSION})"
                )
            }
            Self::PayloadTooLarge { size, max } => {
                write!(
                    f,
                    "Payload size ({size} bytes) exceeds maximum limit ({max} bytes)"
                )
            }
            Self::DiagnosticMessageTooLarge { size, max } => {
                write!(
                    f,
                    "Diagnostic message size ({size} bytes) exceeds maximum limit ({max} bytes)"
                )
            }
            Self::QueueFull { count, max } => {
                write!(f, "Outbox queue is full ({count} records, limit is {max})")
            }
            Self::QuotaExceeded {
                current_bytes,
                incoming_bytes,
                max_bytes,
            } => {
                write!(
                    f,
                    "Outbox quota exceeded (current: {current_bytes} B, incoming: {incoming_bytes} B, limit: {max_bytes} B)"
                )
            }
            Self::ConflictingIdentity { client_activity_id } => {
                write!(
                    f,
                    "Conflicting payload already queued for client_activity_id '{client_activity_id}'"
                )
            }
            Self::RecordNotFound { client_activity_id } => {
                write!(
                    f,
                    "Record '{client_activity_id}' not found in specified scope"
                )
            }
            Self::CorruptRecord { id, reason } => {
                write!(f, "Corrupted outbox record #{id}: {reason}")
            }
            Self::CorruptDatabase(msg) => {
                write!(f, "Outbox database is corrupted or damaged: {msg}")
            }
            Self::Database(err) => write!(f, "Outbox database error: {err}"),
            Self::Serialization(err) => write!(f, "Serialization error: {err}"),
        }
    }
}

impl std::error::Error for OutboxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(err) => Some(err),
            Self::Serialization(err) => Some(err),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for OutboxError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err)
    }
}

impl From<serde_json::Error> for OutboxError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err)
    }
}

/// Durable, thread-safe activity outbox.
#[derive(Debug)]
pub struct Outbox {
    db_path: PathBuf,
    conn: Mutex<Connection>,
    limits: OutboxLimits,
}

impl Outbox {
    /// Opens or initializes a durable outbox at the specified filesystem path.
    ///
    /// If the file already exists, it verifies integrity using `PRAGMA quick_check` and
    /// checks schema version compatibility via `PRAGMA user_version`.
    /// Corrupt databases return [`OutboxError::CorruptDatabase`] and are NOT destroyed.
    /// Databases with a future store version return [`OutboxError::UnsupportedStoreVersion`].
    pub fn open(path: impl AsRef<Path>, limits: OutboxLimits) -> Result<Self, OutboxError> {
        limits.validate()?;

        let db_path = path.as_ref().to_path_buf();

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                OutboxError::CorruptDatabase(format!("Failed to create outbox directory: {e}"))
            })?;
        }

        let is_existing = db_path.exists();

        let conn = Connection::open(&db_path)?;

        // Configure connection pragmas for reliability and durability
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;

        if is_existing {
            let mut check_stmt = conn.prepare("PRAGMA quick_check;")?;
            let status: String = check_stmt.query_row([], |row| row.get(0))?;
            if status != "ok" {
                return Err(OutboxError::CorruptDatabase(format!(
                    "Integrity check failed: {status}"
                )));
            }

            // Check store schema version
            let user_ver: u32 = conn.query_row("PRAGMA user_version;", [], |row| row.get(0))?;
            if user_ver > OUTBOX_SCHEMA_VERSION {
                return Err(OutboxError::UnsupportedStoreVersion(user_ver));
            }
        }

        // Initialize schema idempotently
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS outbox_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                client_activity_id TEXT NOT NULL,
                project_id TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                payload_bytes INTEGER NOT NULL,
                enqueued_at TEXT NOT NULL,
                retry_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                UNIQUE(account_id, workspace_id, client_activity_id)
            );

            CREATE INDEX IF NOT EXISTS idx_outbox_scope_id 
            ON outbox_entries (account_id, workspace_id, id ASC);
            ",
        )?;

        // Ensure user_version pragma is stamped
        conn.pragma_update(None, "user_version", OUTBOX_SCHEMA_VERSION)?;

        Ok(Self {
            db_path,
            conn: Mutex::new(conn),
            limits,
        })
    }

    /// Enqueues an activity envelope under the specified scope.
    ///
    /// - Enforces configured size, cardinality, and quota limits before mutating state.
    /// - If an entry with identical content already exists, returns [`EnqueueResult::Idempotent`].
    /// - If an entry with conflicting content exists under the same identity, returns [`OutboxError::ConflictingIdentity`].
    /// - Genuine query errors propagate immediately via `OptionalExtension::optional` without silent masking.
    pub fn enqueue(
        &self,
        scope: &OutboxScope,
        envelope: &ActivityEnvelope,
    ) -> Result<EnqueueResult, OutboxError> {
        scope.validate()?;
        envelope.validate()?;

        let payload_json = serde_json::to_string(envelope)?;
        let payload_bytes = payload_json.len();

        if payload_bytes > self.limits.max_payload_bytes {
            return Err(OutboxError::PayloadTooLarge {
                size: payload_bytes,
                max: self.limits.max_payload_bytes,
            });
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Check if an entry with this scope and client_activity_id already exists.
        // Use .optional()? so real database errors propagate rather than collapsing to not-found.
        let existing: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, payload_json FROM outbox_entries 
                 WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3",
                params![
                    &scope.account_id,
                    &scope.workspace_id,
                    &envelope.client_activity_id
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((existing_id, existing_json)) = existing {
            if existing_json == payload_json {
                tx.commit()?;
                return Ok(EnqueueResult::Idempotent(existing_id));
            } else {
                tx.rollback()?;
                return Err(OutboxError::ConflictingIdentity {
                    client_activity_id: envelope.client_activity_id.clone(),
                });
            }
        }

        // Check capacity and storage bounds before insertion
        let (current_count, current_bytes): (u32, u64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(payload_bytes), 0) FROM outbox_entries",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        if current_count >= self.limits.max_records {
            tx.rollback()?;
            return Err(OutboxError::QueueFull {
                count: current_count,
                max: self.limits.max_records,
            });
        }

        if current_bytes + (payload_bytes as u64) > self.limits.max_total_payload_bytes {
            tx.rollback()?;
            return Err(OutboxError::QuotaExceeded {
                current_bytes,
                incoming_bytes: payload_bytes,
                max_bytes: self.limits.max_total_payload_bytes,
            });
        }

        let now_utc = Utc::now().to_rfc3339();
        let started_at_str = envelope.started_at.to_rfc3339();
        let ended_at_str = envelope.ended_at.to_rfc3339();

        tx.execute(
            "INSERT INTO outbox_entries (
                account_id, workspace_id, client_activity_id, project_id,
                started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            params![
                &scope.account_id,
                &scope.workspace_id,
                &envelope.client_activity_id,
                &envelope.project_id,
                &started_at_str,
                &ended_at_str,
                &payload_json,
                payload_bytes as i64,
                &now_utc,
            ],
        )?;

        let row_id = tx.last_insert_rowid();
        tx.commit()?;

        Ok(EnqueueResult::Inserted(row_id))
    }

    /// Reads pending entries for the specified scope up to the requested limit.
    ///
    /// Results are returned in deterministic order (`id ASC`).
    /// Fails closed on corrupted timestamps, invalid envelopes, or identity mismatches
    /// without destroying stored rows or inventing timestamps.
    pub fn peek_pending(
        &self,
        scope: &OutboxScope,
        limit: usize,
    ) -> Result<Vec<OutboxEntry>, OutboxError> {
        scope.validate()?;
        let clamped_limit = limit.max(1).min(self.limits.max_page_limit);

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, workspace_id, client_activity_id, payload_json, enqueued_at, retry_count, last_error
             FROM outbox_entries
             WHERE account_id = ?1 AND workspace_id = ?2
             ORDER BY id ASC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(
            params![&scope.account_id, &scope.workspace_id, clamped_limit as i64],
            |row| {
                let id: i64 = row.get(0)?;
                let account_id: String = row.get(1)?;
                let workspace_id: String = row.get(2)?;
                let client_activity_id: String = row.get(3)?;
                let payload_json: String = row.get(4)?;
                let enqueued_at_str: String = row.get(5)?;
                let retry_count: u32 = row.get(6)?;
                let last_error: Option<String> = row.get(7)?;

                Ok((
                    id,
                    account_id,
                    workspace_id,
                    client_activity_id,
                    payload_json,
                    enqueued_at_str,
                    retry_count,
                    last_error,
                ))
            },
        )?;

        let mut entries = Vec::new();
        for item in rows {
            let (
                id,
                account_id,
                workspace_id,
                client_activity_id,
                payload_json,
                enqueued_at_str,
                retry_count,
                last_error,
            ) = item?;

            let enqueued_at = DateTime::parse_from_rfc3339(&enqueued_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| OutboxError::CorruptRecord {
                    id,
                    reason: format!(
                        "Invalid persisted enqueued_at timestamp '{enqueued_at_str}': {e}"
                    ),
                })?;

            let envelope: ActivityEnvelope =
                serde_json::from_str(&payload_json).map_err(|e| OutboxError::CorruptRecord {
                    id,
                    reason: format!("Corrupt envelope JSON: {e}"),
                })?;

            envelope
                .validate()
                .map_err(|e| OutboxError::CorruptRecord {
                    id,
                    reason: format!("Persisted envelope failed validation: {e}"),
                })?;

            if envelope.client_activity_id != client_activity_id {
                return Err(OutboxError::CorruptRecord {
                    id,
                    reason: format!(
                        "Row client_activity_id '{client_activity_id}' does not match envelope identity '{}'",
                        envelope.client_activity_id
                    ),
                });
            }

            let scope = OutboxScope {
                account_id,
                workspace_id,
            };
            scope.validate().map_err(|e| OutboxError::CorruptRecord {
                id,
                reason: format!("Persisted scope failed validation: {e}"),
            })?;

            entries.push(OutboxEntry {
                id,
                scope,
                client_activity_id,
                enqueued_at,
                retry_count,
                last_error,
                envelope,
            });
        }

        Ok(entries)
    }

    /// Acknowledges successful delivery and removes the specified record.
    ///
    /// Removal is strictly bound to the provided scope. Attempting to acknowledge
    /// an ID using another account or workspace scope fails with [`OutboxError::RecordNotFound`].
    pub fn ack_delivered(
        &self,
        scope: &OutboxScope,
        client_activity_id: &str,
    ) -> Result<(), OutboxError> {
        scope.validate()?;

        if client_activity_id.trim().is_empty()
            || client_activity_id.len() > MAX_CLIENT_ACTIVITY_ID_BYTES
            || client_activity_id.chars().any(|c| c.is_control())
        {
            return Err(OutboxError::InvalidEnvelope(
                "client_activity_id invalid or too long".into(),
            ));
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let affected = tx.execute(
            "DELETE FROM outbox_entries 
             WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3",
            params![&scope.account_id, &scope.workspace_id, client_activity_id],
        )?;

        if affected == 0 {
            tx.rollback()?;
            return Err(OutboxError::RecordNotFound {
                client_activity_id: client_activity_id.to_string(),
            });
        }

        tx.commit()?;
        Ok(())
    }

    /// Records a failed delivery attempt, incrementing retry count and updating last error.
    ///
    /// The entry remains intact in the queue for future retry.
    /// The diagnostic error message is bounded by [`MAX_ERROR_MESSAGE_BYTES`] to prevent
    /// unbounded metadata accumulation.
    pub fn mark_attempt_failed(
        &self,
        scope: &OutboxScope,
        client_activity_id: &str,
        error_message: &str,
    ) -> Result<(), OutboxError> {
        scope.validate()?;

        if client_activity_id.trim().is_empty()
            || client_activity_id.len() > MAX_CLIENT_ACTIVITY_ID_BYTES
            || client_activity_id.chars().any(|c| c.is_control())
        {
            return Err(OutboxError::InvalidEnvelope(
                "client_activity_id invalid or too long".into(),
            ));
        }

        if error_message.len() > MAX_ERROR_MESSAGE_BYTES {
            return Err(OutboxError::DiagnosticMessageTooLarge {
                size: error_message.len(),
                max: MAX_ERROR_MESSAGE_BYTES,
            });
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let affected = tx.execute(
            "UPDATE outbox_entries 
             SET retry_count = retry_count + 1, last_error = ?4
             WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3",
            params![
                &scope.account_id,
                &scope.workspace_id,
                client_activity_id,
                error_message
            ],
        )?;

        if affected == 0 {
            tx.rollback()?;
            return Err(OutboxError::RecordNotFound {
                client_activity_id: client_activity_id.to_string(),
            });
        }

        tx.commit()?;
        Ok(())
    }

    /// Returns the total number of records in the outbox, optionally filtered by scope.
    pub fn count(&self, scope: Option<&OutboxScope>) -> Result<u32, OutboxError> {
        if let Some(s) = scope {
            s.validate()?;
        }
        let conn = self.conn.lock().unwrap();
        let count: u32 = match scope {
            Some(s) => conn.query_row(
                "SELECT COUNT(*) FROM outbox_entries WHERE account_id = ?1 AND workspace_id = ?2",
                params![&s.account_id, &s.workspace_id],
                |row| row.get(0),
            )?,
            None => conn.query_row("SELECT COUNT(*) FROM outbox_entries", [], |row| row.get(0))?,
        };
        Ok(count)
    }

    /// Returns the database file path.
    pub fn path(&self) -> &Path {
        &self.db_path
    }
}
