//! Durable, scoped activity outbox for the Linux agent (store schema v2).
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
//! - No credentials in the queue, and no capture bytes in a payload: v1 envelopes carry no
//!   captures at all; v2 rows keep their JPEG captures in the separate `outbox_captures`
//!   table, never inside `payload_json`.
//! - Explicit bounds enforcement: v1 queue cardinality and payload size limits are enforced
//!   before mutation; overflows are rejected explicitly rather than silently dropped. v2 rows
//!   are bounded by [`QueueLimits`] through [`Outbox::enforce_limits`] (captures are shed
//!   before rows, as on Windows).
//! - Coherent store versioning: Stamped with SQLite `PRAGMA user_version`. A v1 store is
//!   migrated to v2 in one IMMEDIATE transaction; future store versions fail closed with
//!   [`OutboxError::UnsupportedStoreVersion`] without auto-downgrade.
//! - Fail-closed corruption handling: [`Outbox::open`] never deletes or resets data. Malformed
//!   persisted rows fail closed without fabricating fallback timestamps. Only
//!   [`Outbox::open_or_recover`] moves a damaged store aside (it is never deleted).
//! - Private files: the store directory is created 0700 and the database file 0600 before
//!   SQLite opens it (SQLite gives its `-wal`/`-shm` sidecars the database's mode).
//! - Parking: a row the server refused for good (or that failed its local integrity check)
//!   is parked (`state = 2`) with a bounded reason instead of blocking its scope; parked rows
//!   are never delivered again and are purged after [`PARKED_RETENTION_DAYS`].
//! - Transparent lookup errors: Row lookups use `OptionalExtension::optional` so genuine
//!   database errors propagate immediately rather than being masked as absent rows.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::agent::model::Activity;
use crate::signing::{self, CaptureDigests};

/// Current schema version for outbox storage.
pub const OUTBOX_SCHEMA_VERSION: u32 = 2;

/// Maximum byte length for account identifier strings (256 bytes).
pub const MAX_ACCOUNT_ID_BYTES: usize = 256;

/// Maximum byte length for workspace identifier strings (256 bytes).
pub const MAX_WORKSPACE_ID_BYTES: usize = 256;

/// Maximum byte length for client activity identifier strings (256 bytes).
pub const MAX_CLIENT_ACTIVITY_ID_BYTES: usize = 256;

/// Maximum byte length for retry diagnostic error messages (1024 bytes).
pub const MAX_ERROR_MESSAGE_BYTES: usize = 1024;

/// Maximum byte length of a parked row's reason (longer reasons are cut at a
/// character boundary).
pub const MAX_PARKED_REASON_BYTES: usize = 1024;

/// Maximum size of one v2 payload (the camelCase `Activity` without captures):
/// at most 200 window entries per list and 120 timeline minutes fit well within it.
pub const MAX_V2_PAYLOAD_BYTES: usize = 512 * 1024;

/// Largest screen capture the server accepts (decoded bytes).
pub const MAX_SCREEN_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// Largest thumbnail the server accepts (decoded bytes).
pub const MAX_THUMB_CAPTURE_BYTES: usize = 512 * 1024;

/// Parked rows and other accounts' pending rows are kept this long (Windows parity).
pub const PARKED_RETENTION_DAYS: u32 = 14;

/// Payload format of a v1 row (`ActivityEnvelope`).
pub const ROW_FORMAT_V1: u32 = 1;

/// Payload format of a v2 row (camelCase `Activity` plus the captures table).
pub const ROW_FORMAT_V2: u32 = 2;

/// Payload keys that never appear in a stored v2 payload: captures live in
/// `outbox_captures`, and `sentAt`/`payloadSignature` are added at send time.
pub const V2_FORBIDDEN_PAYLOAD_KEYS: [&str; 7] = [
    "screenshotPngBase64",
    "screenshotThumbJpgBase64",
    "webcamJpgBase64",
    "screenshotCapturedAt",
    "screenshotBlurred",
    "sentAt",
    "payloadSignature",
];

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
        validate_account_id(&self.account_id)?;

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

fn validate_account_id(account_id: &str) -> Result<(), OutboxError> {
    if account_id.trim().is_empty() {
        return Err(OutboxError::InvalidScope(
            "account_id cannot be empty".into(),
        ));
    }
    if account_id.len() > MAX_ACCOUNT_ID_BYTES {
        return Err(OutboxError::InvalidScope(format!(
            "account_id exceeds maximum length of {MAX_ACCOUNT_ID_BYTES} bytes"
        )));
    }
    if account_id.chars().any(|c| c.is_control()) {
        return Err(OutboxError::InvalidScope(
            "account_id cannot contain control characters".into(),
        ));
    }
    Ok(())
}

/// The account key rows are stored under (trimmed, lowercased), validated.
fn normalized_account(account_id: &str) -> Result<String, OutboxError> {
    let account = account_id.trim().to_lowercase();
    validate_account_id(&account)?;
    Ok(account)
}

/// Versioned activity envelope queued for eventual synchronization (row format 1).
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
    /// Optional HMAC-SHA256 payload signature tag (base64url, no padding).
    /// Computed over canonical JSON bytes without this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_signature: Option<String>,
}

impl ActivityEnvelope {
    pub const CURRENT_FORMAT_VERSION: u32 = 1;

    /// Validates envelope constraints before acceptance.
    pub fn validate(&self) -> Result<(), OutboxError> {
        if self.format_version != Self::CURRENT_FORMAT_VERSION {
            return Err(OutboxError::UnsupportedFormatVersion(self.format_version));
        }
        validate_client_activity_id(&self.client_activity_id)?;
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
        if let Some(ref sig) = self.payload_signature {
            if sig.len() > 128 || sig.chars().any(|c| c.is_control()) {
                return Err(OutboxError::InvalidEnvelope(
                    "payload_signature exceeds maximum length of 128 or contains control characters".into(),
                ));
            }
        }
        Ok(())
    }

    /// The summary of a v2 payload in envelope form (identity, project, times,
    /// counts, task note, device, platform, agent version), so readers written
    /// against v1 rows keep working. Lossy by design: windows, timeline and
    /// captures are only in the payload itself.
    fn summary_of(activity: &Activity) -> Self {
        let saturate = |count: u64| u32::try_from(count).unwrap_or(u32::MAX);
        Self {
            format_version: Self::CURRENT_FORMAT_VERSION,
            client_activity_id: activity.client_activity_id.clone(),
            project_id: Some(activity.project_id.clone()),
            started_at: activity.started_at,
            ended_at: activity.ended_at,
            productivity: activity.productivity.map(u32::from),
            mouse_clicks: activity.mouse_clicks.map(saturate),
            keyboard_hits: activity.keyboard_hits.map(saturate),
            description: activity.description.clone(),
            device_id: activity.device_id.clone(),
            platform: activity
                .platform
                .clone()
                .unwrap_or_else(|| crate::agent::model::PLATFORM.to_string()),
            agent_version: activity.agent_version.clone().unwrap_or_default(),
            payload_signature: None,
        }
    }
}

fn validate_client_activity_id(client_activity_id: &str) -> Result<(), OutboxError> {
    if client_activity_id.trim().is_empty() {
        return Err(OutboxError::InvalidEnvelope(
            "client_activity_id cannot be empty".into(),
        ));
    }
    if client_activity_id.len() > MAX_CLIENT_ACTIVITY_ID_BYTES {
        return Err(OutboxError::InvalidEnvelope(format!(
            "client_activity_id exceeds maximum length of {MAX_CLIENT_ACTIVITY_ID_BYTES} bytes"
        )));
    }
    if client_activity_id.chars().any(|c| c.is_control()) {
        return Err(OutboxError::InvalidEnvelope(
            "client_activity_id cannot contain control characters".into(),
        ));
    }
    Ok(())
}

/// Kind of a queued capture (`outbox_captures.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureKind {
    /// The screen capture (`screenshotPngBase64`, JPEG bytes).
    Screen,
    /// Its small JPEG rendition (`screenshotThumbJpgBase64`).
    Thumb,
}

impl CaptureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureKind::Screen => "screen",
            CaptureKind::Thumb => "thumb",
        }
    }

    pub fn parse(value: &str) -> Option<CaptureKind> {
        match value {
            "screen" => Some(CaptureKind::Screen),
            "thumb" => Some(CaptureKind::Thumb),
            _ => None,
        }
    }

    /// Largest decoded size the server accepts for this kind.
    pub fn max_bytes(self) -> usize {
        match self {
            CaptureKind::Screen => MAX_SCREEN_CAPTURE_BYTES,
            CaptureKind::Thumb => MAX_THUMB_CAPTURE_BYTES,
        }
    }
}

/// A capture handed to [`Outbox::enqueue_activity`].
#[derive(Clone, PartialEq, Eq)]
pub struct PendingCapture {
    pub kind: CaptureKind,
    /// JPEG bytes as they will be uploaded.
    pub jpeg: Vec<u8>,
    /// When the capture was taken.
    pub captured_at: DateTime<Utc>,
    /// The capture was blurred before it was queued.
    pub blurred: bool,
}

impl std::fmt::Debug for PendingCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingCapture")
            .field("kind", &self.kind)
            .field("jpeg_bytes", &self.jpeg.len())
            .field("captured_at", &self.captured_at)
            .field("blurred", &self.blurred)
            .finish()
    }
}

/// A capture as stored with a v2 row ([`Outbox::load_captures`]).
///
/// A shed or stripped capture keeps its row and digest with empty `jpeg`, so
/// the row's signature still verifies; only non-empty captures are uploaded.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredCapture {
    pub kind: CaptureKind,
    pub jpeg: Vec<u8>,
    pub captured_at: DateTime<Utc>,
    pub blurred: bool,
    /// SHA-256 of the JPEG as queued.
    pub sha256: [u8; 32],
}

impl StoredCapture {
    /// Still carries image bytes (not shed or stripped).
    pub fn has_bytes(&self) -> bool {
        !self.jpeg.is_empty()
    }

    /// The bytes are the ones queued: their SHA-256 matches the stored digest.
    /// A shed capture (no bytes) trivially matches.
    pub fn bytes_match_digest(&self) -> bool {
        !self.has_bytes() || signing::sha256(&self.jpeg) == self.sha256
    }
}

impl std::fmt::Debug for StoredCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredCapture")
            .field("kind", &self.kind)
            .field("jpeg_bytes", &self.jpeg.len())
            .field("captured_at", &self.captured_at)
            .field("blurred", &self.blurred)
            .finish()
    }
}

/// The digests a v2 row's signature binds, from its stored captures.
pub fn capture_digests(captures: &[StoredCapture]) -> CaptureDigests {
    let mut digests = CaptureDigests::default();
    for capture in captures {
        match capture.kind {
            CaptureKind::Screen => digests.screen = Some(capture.sha256),
            CaptureKind::Thumb => digests.thumb = Some(capture.sha256),
        }
    }
    digests
}

/// A completed block handed to [`Outbox::enqueue_activity`] (row format 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingActivity {
    /// UUIDv4 of the block; equals the payload's `clientActivityId`.
    pub client_activity_id: String,
    /// GUID of the project; equals the payload's `projectId`.
    pub project_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// The camelCase `Activity` without captures, `sentAt` and `payloadSignature`.
    pub payload_json: String,
    /// At most one capture of each kind.
    pub captures: Vec<PendingCapture>,
}

impl PendingActivity {
    /// Build a pending row from a finished block: its captures are moved out of
    /// the payload into `captures`, and the send-time fields are cleared.
    pub fn from_activity(
        activity: &Activity,
        captures: Vec<PendingCapture>,
    ) -> Result<Self, OutboxError> {
        let mut payload = activity.clone();
        payload.screenshot_png_base64 = None;
        payload.screenshot_thumb_jpg_base64 = None;
        payload.webcam_jpg_base64 = None;
        payload.screenshot_captured_at = None;
        payload.screenshot_blurred = None;
        payload.sent_at = None;
        payload.payload_signature = None;
        Ok(Self {
            client_activity_id: activity.client_activity_id.clone(),
            project_id: activity.project_id.clone(),
            started_at: activity.started_at,
            ended_at: activity.ended_at,
            payload_json: serde_json::to_string(&payload)?,
            captures,
        })
    }

    /// Validates a pending row before anything is written.
    fn validate(&self) -> Result<Activity, OutboxError> {
        let invalid = |msg: &str| Err(OutboxError::InvalidEnvelope(msg.to_string()));
        validate_client_activity_id(&self.client_activity_id)?;
        if uuid::Uuid::parse_str(&self.client_activity_id).is_err() {
            return invalid("client_activity_id must be a UUID");
        }
        match uuid::Uuid::parse_str(&self.project_id) {
            Ok(id) if !id.is_nil() => {}
            _ => return invalid("project_id must be a non-empty GUID"),
        }
        if self.started_at > self.ended_at {
            return invalid("started_at cannot be after ended_at");
        }
        if self.payload_json.len() > MAX_V2_PAYLOAD_BYTES {
            return Err(OutboxError::PayloadTooLarge {
                size: self.payload_json.len(),
                max: MAX_V2_PAYLOAD_BYTES,
            });
        }
        let value: serde_json::Value = serde_json::from_str(&self.payload_json)?;
        let Some(object) = value.as_object() else {
            return invalid("payload must be a JSON object");
        };
        if let Some(key) = V2_FORBIDDEN_PAYLOAD_KEYS
            .iter()
            .find(|key| object.contains_key(**key))
        {
            return Err(OutboxError::InvalidEnvelope(format!(
                "payload must not contain {key}: captures and send-time fields are stored apart"
            )));
        }
        let activity: Activity = serde_json::from_value(value)?;
        if activity.client_activity_id != self.client_activity_id
            || activity.project_id != self.project_id
            || activity.started_at != self.started_at
            || activity.ended_at != self.ended_at
        {
            return invalid("payload identity, project or times differ from the row's");
        }
        if activity.productivity.is_some_and(|p| p > 100) {
            return invalid("productivity percentage cannot exceed 100");
        }
        if let Some(device_id) = activity.device_id.as_deref() {
            if uuid::Uuid::parse_str(device_id).is_err() {
                return invalid("deviceId must be a UUID");
            }
        }
        for (index, capture) in self.captures.iter().enumerate() {
            if self.captures[..index]
                .iter()
                .any(|earlier| earlier.kind == capture.kind)
            {
                return Err(OutboxError::InvalidEnvelope(format!(
                    "more than one {} capture",
                    capture.kind.as_str()
                )));
            }
            if capture.jpeg.is_empty() {
                return invalid("a capture cannot be empty");
            }
            if capture.jpeg.len() > capture.kind.max_bytes() {
                return Err(OutboxError::PayloadTooLarge {
                    size: capture.jpeg.len(),
                    max: capture.kind.max_bytes(),
                });
            }
        }
        Ok(activity)
    }

    fn digests(&self) -> CaptureDigests {
        let mut digests = CaptureDigests::default();
        for capture in &self.captures {
            let digest = Some(signing::sha256(&capture.jpeg));
            match capture.kind {
                CaptureKind::Screen => digests.screen = digest,
                CaptureKind::Thumb => digests.thumb = digest,
            }
        }
        digests
    }
}

/// Bounded capacity and size limits for v1 envelope rows.
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

/// Bounds that keep the queue from growing without limit while the backend
/// is unreachable (Windows `QueueLimits`).
///
/// Captures are by far the bulk of a row, so they are shed first: pending rows
/// keep their captures only while they are among the newest `capture_max_rows`
/// pending rows, the captures of all newer rows plus their own stay within
/// `capture_max_bytes`, and they were queued less than `capture_max_age_days`
/// ago. Only beyond `max_pending_rows`, or once a block started more than
/// `max_age_days` ago (the server refuses it then), are pending rows dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueLimits {
    pub capture_max_rows: u32,
    pub capture_max_bytes: u64,
    pub capture_max_age_days: u32,
    pub max_pending_rows: u32,
    pub max_age_days: u32,
}

impl QueueLimits {
    /// ~500 rows is ~80 h of tracking at the default 10-minute interval, and
    /// 300 MiB holds their screenshots at typical JPEG sizes. 10,000 time-only
    /// rows are a few tens of MiB; 45 days is the server's oldest accepted block.
    pub const DEFAULT: QueueLimits = QueueLimits {
        capture_max_rows: 500,
        capture_max_bytes: 300 * 1024 * 1024,
        capture_max_age_days: 30,
        max_pending_rows: 10_000,
        max_age_days: 45,
    };
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What [`Outbox::enforce_limits`] did (for the `QueueTrimmed` notice).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimitReport {
    /// Pending rows whose captures were shed.
    pub stripped: usize,
    /// Pending rows dropped beyond `max_pending_rows`.
    pub dropped_overflow: usize,
    /// Pending rows dropped because their block is older than `max_age_days`.
    pub dropped_too_old: usize,
}

impl LimitReport {
    /// Pending rows dropped for any reason.
    pub fn dropped(&self) -> usize {
        self.dropped_overflow + self.dropped_too_old
    }
}

/// Where a row stands (`outbox_entries.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowState {
    /// Waiting to upload.
    Pending,
    /// Refused for good (or failed its local integrity check): kept for
    /// diagnostics, never delivered again.
    Parked,
}

impl RowState {
    fn as_i64(self) -> i64 {
        match self {
            RowState::Pending => 0,
            RowState::Parked => 2,
        }
    }

    fn from_i64(value: i64) -> Option<RowState> {
        match value {
            0 => Some(RowState::Pending),
            2 => Some(RowState::Parked),
            _ => None,
        }
    }
}

/// Which rows [`Outbox::purge`] removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeTarget {
    Pending,
    Parked,
    All,
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
    /// Row format 1: the queued envelope. Row format 2: its summary in
    /// envelope form (see `payload_json` for the payload itself).
    pub envelope: ActivityEnvelope,
    /// [`ROW_FORMAT_V1`] or [`ROW_FORMAT_V2`].
    #[serde(default = "default_row_format")]
    pub format_version: u32,
    /// Row format 2: the payload exactly as stored (the bytes the signature binds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_json: Option<String>,
    /// Row format 2: the v2 signature stored with the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_signature: Option<String>,
    /// Bytes of captures the row still carries.
    #[serde(default)]
    pub capture_bytes: u64,
    /// The row's captures, filled by the sync pass right before delivery
    /// ([`Outbox::load_captures`]); never serialized.
    #[serde(skip)]
    pub captures: Vec<StoredCapture>,
}

fn default_row_format() -> u32 {
    ROW_FORMAT_V1
}

impl OutboxEntry {
    /// A v2 row (payload plus captures table).
    pub fn is_v2(&self) -> bool {
        self.format_version == ROW_FORMAT_V2
    }
}

/// A row as `queue list` shows it: metadata only, never payloads or captures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueRow {
    pub id: i64,
    pub workspace_id: String,
    pub client_activity_id: String,
    pub project_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub state: RowState,
    pub retry_count: u32,
    pub last_error: Option<String>,
    pub parked_reason: Option<String>,
    pub parked_at: Option<DateTime<Utc>>,
    pub capture_bytes: u64,
    pub format_version: u32,
}

/// One account's queue in numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueSummary {
    pub pending: u64,
    pub parked: u64,
    /// Capture bytes still carried by pending rows.
    pub capture_bytes: u64,
    pub oldest_pending_enqueued_at: Option<DateTime<Utc>>,
    /// Pending rows of other accounts (kept [`PARKED_RETENTION_DAYS`]).
    pub foreign_pending: u64,
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
    /// The store directory or file could not be prepared (or is unsafe: a
    /// symlink, another user's, writable by others).
    Io(String),
    /// SQLite storage error.
    Database(rusqlite::Error),
    /// JSON serialization or deserialization failure.
    Serialization(serde_json::Error),
    /// Outbox database connection lock was poisoned by a prior panic.
    LockPoisoned,
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
            Self::Io(msg) => write!(f, "Outbox storage error: {msg}"),
            Self::Database(err) => write!(f, "Outbox database error: {err}"),
            Self::Serialization(err) => write!(f, "Serialization error: {err}"),
            Self::LockPoisoned => {
                write!(
                    f,
                    "Outbox database connection lock was poisoned by a prior panic"
                )
            }
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

/// SQLite says the file is not a database or is damaged.
fn is_damage(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if matches!(e.code, rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt)
    )
}

/// Map an error met while opening: damage becomes [`OutboxError::CorruptDatabase`].
fn open_error(err: rusqlite::Error) -> OutboxError {
    if is_damage(&err) {
        OutboxError::CorruptDatabase(err.to_string())
    } else {
        OutboxError::Database(err)
    }
}

/// Cut `s` to at most `max_bytes` at a character boundary.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 || !value.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Create the store directory as a private directory and the database file
/// as a 0600 file of the current user before SQLite opens it. Returns whether
/// the file already held data.
#[cfg(unix)]
fn prepare_store_file(db_path: &Path) -> Result<bool, OutboxError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::paths::ensure_private_dir(parent)
            .map_err(|e| OutboxError::Io(format!("Failed to prepare the outbox directory: {e}")))?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(db_path)
        .map_err(|e| OutboxError::Io(format!("Failed to open the outbox file: {e}")))?;
    let meta = file
        .metadata()
        .map_err(|e| OutboxError::Io(format!("Failed to inspect the outbox file: {e}")))?;
    if !meta.is_file() {
        return Err(OutboxError::Io(
            "the outbox path is not a regular file".into(),
        ));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(OutboxError::Io(
            "the outbox file is owned by another user".into(),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| OutboxError::Io(format!("Failed to restrict the outbox file: {e}")))?;
    }
    Ok(meta.len() > 0)
}

#[cfg(not(unix))]
fn prepare_store_file(db_path: &Path) -> Result<bool, OutboxError> {
    if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| OutboxError::Io(format!("Failed to create outbox directory: {e}")))?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(db_path)
        .map_err(|e| OutboxError::Io(format!("Failed to open the outbox file: {e}")))?;
    let len = file
        .metadata()
        .map_err(|e| OutboxError::Io(format!("Failed to inspect the outbox file: {e}")))?
        .len();
    Ok(len > 0)
}

/// Does the (non-empty) file start with the SQLite header?
fn has_sqlite_header(db_path: &Path) -> Result<bool, OutboxError> {
    use std::io::Read;
    const MAGIC: &[u8; 16] = b"SQLite format 3\0";
    let mut header = [0u8; 16];
    let mut file = std::fs::File::open(db_path)
        .map_err(|e| OutboxError::Io(format!("Failed to read the outbox file: {e}")))?;
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => {
                return Err(OutboxError::Io(format!(
                    "Failed to read the outbox file: {e}"
                )))
            }
        }
    }
    Ok(filled == header.len() && &header == MAGIC)
}

/// The v1 schema; also the base every newer schema is migrated from.
const SCHEMA_V1: &str = "
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
";

/// v1 -> v2 (design 9.3), run in one IMMEDIATE transaction.
const MIGRATE_V1_TO_V2: &str = "
    ALTER TABLE outbox_entries ADD COLUMN state INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE outbox_entries ADD COLUMN parked_at TEXT;
    ALTER TABLE outbox_entries ADD COLUMN parked_reason TEXT;
    ALTER TABLE outbox_entries ADD COLUMN capture_bytes INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE outbox_entries ADD COLUMN format_version INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE outbox_entries ADD COLUMN payload_signature TEXT;
    CREATE TABLE outbox_captures (
        entry_id INTEGER NOT NULL REFERENCES outbox_entries(id) ON DELETE CASCADE,
        kind TEXT NOT NULL CHECK (kind IN ('screen','thumb')),
        jpeg BLOB NOT NULL,
        captured_at TEXT NOT NULL,
        blurred INTEGER NOT NULL,
        sha256 TEXT NOT NULL,
        PRIMARY KEY (entry_id, kind));
    CREATE INDEX IF NOT EXISTS idx_outbox_scope_state ON outbox_entries(account_id, workspace_id, state, id);
";

/// Live data up to which [`Outbox::reclaim_space`] rebuilds a store created
/// without incremental auto-vacuum (a one-off VACUUM rewrites all live data).
const VACUUM_SWITCH_MAX_LIVE_BYTES: i64 = 64 * 1024 * 1024;

/// A store opened by [`Outbox::open_or_recover`].
#[derive(Debug)]
pub struct OpenedOutbox {
    pub outbox: Outbox,
    /// Where a damaged store was moved before a fresh one was created.
    pub moved_aside: Option<PathBuf>,
}

/// Durable, thread-safe activity outbox.
#[derive(Debug)]
pub struct Outbox {
    db_path: PathBuf,
    conn: Mutex<Connection>,
    limits: OutboxLimits,
}

const ENTRY_COLUMNS: &str = "id, account_id, workspace_id, client_activity_id, payload_json, \
     enqueued_at, retry_count, last_error, format_version, payload_signature, capture_bytes";

struct RawEntry {
    id: i64,
    account_id: String,
    workspace_id: String,
    client_activity_id: String,
    payload_json: String,
    enqueued_at: String,
    retry_count: u32,
    last_error: Option<String>,
    format_version: i64,
    payload_signature: Option<String>,
    capture_bytes: i64,
}

impl RawEntry {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            account_id: row.get(1)?,
            workspace_id: row.get(2)?,
            client_activity_id: row.get(3)?,
            payload_json: row.get(4)?,
            enqueued_at: row.get(5)?,
            retry_count: row.get(6)?,
            last_error: row.get(7)?,
            format_version: row.get(8)?,
            payload_signature: row.get(9)?,
            capture_bytes: row.get(10)?,
        })
    }

    /// Decode and validate a persisted row; fails closed on anything malformed.
    fn decode(self) -> Result<OutboxEntry, OutboxError> {
        let id = self.id;
        let corrupt = |reason: String| OutboxError::CorruptRecord { id, reason };

        let enqueued_at = DateTime::parse_from_rfc3339(&self.enqueued_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| {
                corrupt(format!(
                    "Invalid persisted enqueued_at timestamp '{}': {e}",
                    self.enqueued_at
                ))
            })?;
        let capture_bytes = u64::try_from(self.capture_bytes)
            .map_err(|_| corrupt("Negative persisted capture_bytes".into()))?;

        let (envelope, format_version, payload_json) = match self.format_version {
            1 => {
                let envelope: ActivityEnvelope = serde_json::from_str(&self.payload_json)
                    .map_err(|e| corrupt(format!("Corrupt envelope JSON: {e}")))?;
                envelope
                    .validate()
                    .map_err(|e| corrupt(format!("Persisted envelope failed validation: {e}")))?;
                if envelope.client_activity_id != self.client_activity_id {
                    return Err(corrupt(format!(
                        "Row client_activity_id '{}' does not match envelope identity '{}'",
                        self.client_activity_id, envelope.client_activity_id
                    )));
                }
                (envelope, ROW_FORMAT_V1, None)
            }
            2 => {
                let activity: Activity = serde_json::from_str(&self.payload_json)
                    .map_err(|e| corrupt(format!("Corrupt v2 payload JSON: {e}")))?;
                if activity.client_activity_id != self.client_activity_id {
                    return Err(corrupt(format!(
                        "Row client_activity_id '{}' does not match envelope identity '{}'",
                        self.client_activity_id, activity.client_activity_id
                    )));
                }
                if activity.started_at > activity.ended_at {
                    return Err(corrupt(
                        "Persisted envelope failed validation: started_at cannot be after ended_at"
                            .into(),
                    ));
                }
                (
                    ActivityEnvelope::summary_of(&activity),
                    ROW_FORMAT_V2,
                    Some(self.payload_json),
                )
            }
            other => return Err(corrupt(format!("Unknown row format version {other}"))),
        };

        let scope = OutboxScope {
            account_id: self.account_id,
            workspace_id: self.workspace_id,
        };
        scope
            .validate()
            .map_err(|e| corrupt(format!("Persisted scope failed validation: {e}")))?;

        Ok(OutboxEntry {
            id,
            scope,
            client_activity_id: self.client_activity_id,
            enqueued_at,
            retry_count: self.retry_count,
            last_error: self.last_error,
            envelope,
            format_version,
            payload_json,
            payload_signature: self.payload_signature,
            capture_bytes,
            captures: Vec::new(),
        })
    }
}

impl Outbox {
    /// Opens or initializes a durable outbox at the specified filesystem path.
    ///
    /// The directory is created private (0700) and the file 0600 before SQLite
    /// opens it. An existing file is verified with `PRAGMA quick_check` and its
    /// schema version via `PRAGMA user_version`; a v1 store is migrated to v2 in
    /// one IMMEDIATE transaction. Corrupt databases return
    /// [`OutboxError::CorruptDatabase`] and are NOT destroyed. Databases with a
    /// future store version return [`OutboxError::UnsupportedStoreVersion`].
    pub fn open(path: impl AsRef<Path>, limits: OutboxLimits) -> Result<Self, OutboxError> {
        limits.validate()?;

        let db_path = path.as_ref().to_path_buf();
        let is_existing = prepare_store_file(&db_path)?;
        if is_existing && !has_sqlite_header(&db_path)? {
            // Refused before SQLite touches the file or its sidecars, so a
            // recovery can move all of them aside untouched.
            return Err(OutboxError::CorruptDatabase(
                "file is not a SQLite database".into(),
            ));
        }

        let mut conn = Connection::open(&db_path).map_err(open_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(open_error)?;

        if is_existing {
            // Read-only checks first: a newer or damaged store is refused
            // before any pragma could rewrite its header.
            let user_ver: u32 = conn
                .query_row("PRAGMA user_version;", [], |row| row.get(0))
                .map_err(open_error)?;
            if user_ver > OUTBOX_SCHEMA_VERSION {
                return Err(OutboxError::UnsupportedStoreVersion(user_ver));
            }
            let status: String = conn
                .query_row("PRAGMA quick_check;", [], |row| row.get(0))
                .map_err(open_error)?;
            if status != "ok" {
                return Err(OutboxError::CorruptDatabase(format!(
                    "Integrity check failed: {status}"
                )));
            }
        } else {
            // Only possible before the first table exists: a new store is
            // created with incremental auto-vacuum.
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
                .map_err(open_error)?;
        }

        // Configure connection pragmas for reliability and durability
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(open_error)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(open_error)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(open_error)?;

        Self::migrate(&mut conn)?;

        Ok(Self {
            db_path,
            conn: Mutex::new(conn),
            limits,
        })
    }

    /// Bring the schema to [`OUTBOX_SCHEMA_VERSION`] in one IMMEDIATE
    /// transaction (all or nothing); refuse a newer store.
    fn migrate(conn: &mut Connection) -> Result<(), OutboxError> {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(open_error)?;
        let user_ver: u32 = tx
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .map_err(open_error)?;
        if user_ver > OUTBOX_SCHEMA_VERSION {
            return Err(OutboxError::UnsupportedStoreVersion(user_ver));
        }
        if user_ver < OUTBOX_SCHEMA_VERSION {
            tx.execute_batch(SCHEMA_V1)?;
            let has_state: bool = tx
                .query_row(
                    "SELECT 1 FROM pragma_table_info('outbox_entries') WHERE name = 'state'",
                    [],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !has_state {
                tx.execute_batch(MIGRATE_V1_TO_V2)?;
            }
            tx.pragma_update(None, "user_version", OUTBOX_SCHEMA_VERSION)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// [`Outbox::open`], moving a damaged store aside first: the database and
    /// its `-wal`/`-shm` sidecars are renamed to `<name>.damaged-<utc>` (never
    /// deleted) and a fresh store is created. A store of a newer version is
    /// never moved: it belongs to a newer build.
    pub fn open_or_recover(
        path: impl AsRef<Path>,
        limits: OutboxLimits,
    ) -> Result<OpenedOutbox, OutboxError> {
        let path = path.as_ref();
        match Self::open(path, limits.clone()) {
            Ok(outbox) => Ok(OpenedOutbox {
                outbox,
                moved_aside: None,
            }),
            Err(OutboxError::CorruptDatabase(_)) => {
                let aside = set_aside(path, Utc::now())?;
                let outbox = Self::open(path, limits)?;
                Ok(OpenedOutbox {
                    outbox,
                    moved_aside: Some(aside),
                })
            }
            Err(e) => Err(e),
        }
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, OutboxError> {
        self.conn.lock().map_err(|_| OutboxError::LockPoisoned)
    }

    /// Intentionally poisons the internal mutex by inducing a panic while holding the lock.
    /// Used in tests to verify recoverable [`OutboxError::LockPoisoned`] error handling.
    pub fn poison_lock_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.conn.lock().unwrap();
            panic!("Intentional test panic to poison Mutex");
        }));
    }

    /// Enqueues an activity envelope (row format 1) under the specified scope.
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

        let mut conn = self.lock_conn()?;
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

        tx.execute(
            "INSERT INTO outbox_entries (
                account_id, workspace_id, client_activity_id, project_id,
                started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count,
                format_version
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 1)",
            params![
                &scope.account_id,
                &scope.workspace_id,
                &envelope.client_activity_id,
                &envelope.project_id,
                rfc3339(envelope.started_at),
                rfc3339(envelope.ended_at),
                &payload_json,
                payload_bytes as i64,
                rfc3339(Utc::now()),
            ],
        )?;

        let row_id = tx.last_insert_rowid();
        tx.commit()?;

        Ok(EnqueueResult::Inserted(row_id))
    }

    /// Enqueue a finished block (row format 2) with its captures, signed with
    /// the device key, in one transaction.
    ///
    /// The ids must be GUIDs, the payload must match the row's identity, project
    /// and times and must not carry captures or send-time fields, and captures
    /// must fit the server's size limits. Re-enqueueing the identical row is
    /// idempotent; different content under a queued identity is refused. v2 rows
    /// are bounded by [`Outbox::enforce_limits`], not refused for space.
    pub fn enqueue_activity(
        &self,
        scope: &OutboxScope,
        pending: &PendingActivity,
        signing_key: &[u8],
    ) -> Result<EnqueueResult, OutboxError> {
        scope.validate()?;
        if signing_key.is_empty() {
            return Err(OutboxError::InvalidEnvelope(
                "a device signing key is required".into(),
            ));
        }
        pending.validate()?;
        let digests = pending.digests();
        let signature = signing::sign_v2(signing_key, pending.payload_json.as_bytes(), &digests);
        let capture_bytes: usize = pending.captures.iter().map(|c| c.jpeg.len()).sum();

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let existing: Option<(i64, String, Option<String>)> = tx
            .query_row(
                "SELECT id, payload_json, payload_signature FROM outbox_entries
                 WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3",
                params![
                    &scope.account_id,
                    &scope.workspace_id,
                    &pending.client_activity_id
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((existing_id, existing_json, existing_sig)) = existing {
            // Same key, same payload and same captures sign to the same tag.
            if existing_json == pending.payload_json
                && existing_sig.as_deref() == Some(signature.as_str())
            {
                tx.commit()?;
                return Ok(EnqueueResult::Idempotent(existing_id));
            }
            tx.rollback()?;
            return Err(OutboxError::ConflictingIdentity {
                client_activity_id: pending.client_activity_id.clone(),
            });
        }

        tx.execute(
            "INSERT INTO outbox_entries (
                account_id, workspace_id, client_activity_id, project_id,
                started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count,
                state, capture_bytes, format_version, payload_signature
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, ?10, 2, ?11)",
            params![
                &scope.account_id,
                &scope.workspace_id,
                &pending.client_activity_id,
                &pending.project_id,
                rfc3339(pending.started_at),
                rfc3339(pending.ended_at),
                &pending.payload_json,
                pending.payload_json.len() as i64,
                rfc3339(Utc::now()),
                capture_bytes as i64,
                &signature,
            ],
        )?;
        let row_id = tx.last_insert_rowid();
        for capture in &pending.captures {
            tx.execute(
                "INSERT INTO outbox_captures (entry_id, kind, jpeg, captured_at, blurred, sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    row_id,
                    capture.kind.as_str(),
                    &capture.jpeg,
                    rfc3339(capture.captured_at),
                    capture.blurred,
                    hex(&signing::sha256(&capture.jpeg)),
                ],
            )?;
        }
        tx.commit()?;
        Ok(EnqueueResult::Inserted(row_id))
    }

    /// Reads pending entries for the specified scope up to the requested limit.
    ///
    /// Same as [`Outbox::peek_ready`]: parked rows are never returned.
    pub fn peek_pending(
        &self,
        scope: &OutboxScope,
        limit: usize,
    ) -> Result<Vec<OutboxEntry>, OutboxError> {
        self.peek_ready(scope, limit)
    }

    /// Reads the scope's pending rows (v1 and v2) up to `limit`, oldest first,
    /// without their capture bytes (see [`Outbox::load_captures`]).
    ///
    /// Fails closed on corrupted timestamps, invalid envelopes, or identity mismatches
    /// without destroying stored rows or inventing timestamps.
    pub fn peek_ready(
        &self,
        scope: &OutboxScope,
        limit: usize,
    ) -> Result<Vec<OutboxEntry>, OutboxError> {
        scope.validate()?;
        let clamped_limit = limit.max(1).min(self.limits.max_page_limit);

        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS}
             FROM outbox_entries
             WHERE account_id = ?1 AND workspace_id = ?2 AND state = 0
             ORDER BY id ASC
             LIMIT ?3"
        ))?;
        let rows = stmt.query_map(
            params![&scope.account_id, &scope.workspace_id, clamped_limit as i64],
            RawEntry::from_row,
        )?;

        let mut entries = Vec::new();
        for raw in rows {
            entries.push(raw?.decode()?);
        }
        Ok(entries)
    }

    /// The captures stored with the scope's row `entry_id`, shed ones included
    /// (with empty bytes), in kind order (screen, thumb).
    pub fn load_captures(
        &self,
        scope: &OutboxScope,
        entry_id: i64,
    ) -> Result<Vec<StoredCapture>, OutboxError> {
        scope.validate()?;
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT c.kind, c.jpeg, c.captured_at, c.blurred, c.sha256
             FROM outbox_captures c JOIN outbox_entries e ON e.id = c.entry_id
             WHERE c.entry_id = ?1 AND e.account_id = ?2 AND e.workspace_id = ?3
             ORDER BY CASE c.kind WHEN 'screen' THEN 0 ELSE 1 END",
        )?;
        let rows = stmt.query_map(
            params![entry_id, &scope.account_id, &scope.workspace_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?;
        let corrupt = |reason: &str| OutboxError::CorruptRecord {
            id: entry_id,
            reason: reason.to_string(),
        };
        let mut captures = Vec::new();
        for row in rows {
            let (kind, jpeg, captured_at, blurred, sha256) = row?;
            captures.push(StoredCapture {
                kind: CaptureKind::parse(&kind).ok_or_else(|| corrupt("Unknown capture kind"))?,
                jpeg,
                captured_at: parse_utc(&captured_at)
                    .ok_or_else(|| corrupt("Invalid persisted capture timestamp"))?,
                blurred,
                sha256: unhex32(&sha256).ok_or_else(|| corrupt("Invalid capture digest"))?,
            });
        }
        Ok(captures)
    }

    /// Acknowledges successful delivery and removes the specified record
    /// (with its captures).
    ///
    /// Removal is strictly bound to the provided scope. Attempting to acknowledge
    /// an ID using another account or workspace scope fails with [`OutboxError::RecordNotFound`].
    pub fn ack_delivered(
        &self,
        scope: &OutboxScope,
        client_activity_id: &str,
    ) -> Result<(), OutboxError> {
        scope.validate()?;
        check_identity_arg(client_activity_id)?;

        let mut conn = self.lock_conn()?;
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
        check_identity_arg(client_activity_id)?;

        if error_message.len() > MAX_ERROR_MESSAGE_BYTES {
            return Err(OutboxError::DiagnosticMessageTooLarge {
                size: error_message.len(),
                max: MAX_ERROR_MESSAGE_BYTES,
            });
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let affected = tx.execute(
            "UPDATE outbox_entries
             SET retry_count = retry_count + 1, last_error = ?4
             WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3
               AND state = 0",
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

    /// Park one pending row: it is kept with `reason` (cut to
    /// [`MAX_PARKED_REASON_BYTES`]) for diagnostics, never delivered again, and
    /// its captures are deleted.
    pub fn park(
        &self,
        scope: &OutboxScope,
        client_activity_id: &str,
        reason: &str,
    ) -> Result<(), OutboxError> {
        scope.validate()?;
        check_identity_arg(client_activity_id)?;
        let reason = truncate_utf8(reason, MAX_PARKED_REASON_BYTES);

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: Option<i64> = tx
            .query_row(
                "SELECT id FROM outbox_entries
                 WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3
                   AND state = 0",
                params![&scope.account_id, &scope.workspace_id, client_activity_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            tx.rollback()?;
            return Err(OutboxError::RecordNotFound {
                client_activity_id: client_activity_id.to_string(),
            });
        };
        tx.execute("DELETE FROM outbox_captures WHERE entry_id = ?1", [id])?;
        tx.execute(
            "UPDATE outbox_entries
             SET state = 2, parked_at = ?2, parked_reason = ?3, capture_bytes = 0
             WHERE id = ?1",
            params![id, rfc3339(Utc::now()), reason],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Park every pending row of `scope` (a revoked device). Returns how many.
    pub fn park_scope(&self, scope: &OutboxScope, reason: &str) -> Result<usize, OutboxError> {
        scope.validate()?;
        let reason = truncate_utf8(reason, MAX_PARKED_REASON_BYTES);
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM outbox_captures WHERE entry_id IN (
                 SELECT id FROM outbox_entries
                 WHERE account_id = ?1 AND workspace_id = ?2 AND state = 0)",
            params![&scope.account_id, &scope.workspace_id],
        )?;
        let parked = tx.execute(
            "UPDATE outbox_entries
             SET state = 2, parked_at = ?3, parked_reason = ?4, capture_bytes = 0
             WHERE account_id = ?1 AND workspace_id = ?2 AND state = 0",
            params![
                &scope.account_id,
                &scope.workspace_id,
                rfc3339(Utc::now()),
                reason
            ],
        )?;
        tx.commit()?;
        Ok(parked)
    }

    /// Remove the image bytes of one pending row, keeping its time data and
    /// the captures' digests (so its signature still verifies) — what a 413
    /// calls for before resending. Returns whether there was anything to
    /// strip: `false` for a row without images, which resending cannot make
    /// any smaller.
    pub fn strip_captures(
        &self,
        scope: &OutboxScope,
        client_activity_id: &str,
    ) -> Result<bool, OutboxError> {
        scope.validate()?;
        check_identity_arg(client_activity_id)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: Option<i64> = tx
            .query_row(
                "SELECT id FROM outbox_entries
                 WHERE account_id = ?1 AND workspace_id = ?2 AND client_activity_id = ?3
                   AND state = 0",
                params![&scope.account_id, &scope.workspace_id, client_activity_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            tx.rollback()?;
            return Err(OutboxError::RecordNotFound {
                client_activity_id: client_activity_id.to_string(),
            });
        };
        let stripped = strip_entry_captures(&tx, id)?;
        tx.commit()?;
        Ok(stripped)
    }

    /// Workspaces of `account` that have pending rows, oldest first.
    pub fn pending_scopes(&self, account_id: &str) -> Result<Vec<OutboxScope>, OutboxError> {
        let account = normalized_account(account_id)?;
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT workspace_id FROM outbox_entries
             WHERE account_id = ?1 AND state = 0
             GROUP BY workspace_id ORDER BY MIN(id)",
        )?;
        let rows = stmt.query_map([&account], |row| row.get::<_, String>(0))?;
        let mut scopes = Vec::new();
        for workspace in rows {
            if let Ok(scope) = OutboxScope::new(account.clone(), workspace?) {
                scopes.push(scope);
            }
        }
        Ok(scopes)
    }

    /// The queue of `account` in numbers, plus other accounts' pending rows.
    pub fn queue_summary(&self, account_id: &str) -> Result<QueueSummary, OutboxError> {
        let account = normalized_account(account_id)?;
        let conn = self.lock_conn()?;
        let (pending, parked, capture_bytes, oldest): (i64, i64, i64, Option<String>) = conn
            .query_row(
                "SELECT
                     COALESCE(SUM(CASE WHEN state = 0 THEN 1 ELSE 0 END), 0),
                     COALESCE(SUM(CASE WHEN state = 2 THEN 1 ELSE 0 END), 0),
                     COALESCE(SUM(CASE WHEN state = 0 THEN capture_bytes ELSE 0 END), 0),
                     MIN(CASE WHEN state = 0 THEN enqueued_at END)
                 FROM outbox_entries WHERE account_id = ?1",
                [&account],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let foreign_pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM outbox_entries WHERE account_id <> ?1 AND state = 0",
            [&account],
            |row| row.get(0),
        )?;
        Ok(QueueSummary {
            pending: pending.max(0) as u64,
            parked: parked.max(0) as u64,
            capture_bytes: capture_bytes.max(0) as u64,
            oldest_pending_enqueued_at: oldest.as_deref().and_then(parse_utc),
            foreign_pending: foreign_pending.max(0) as u64,
        })
    }

    /// The newest `limit` rows of `account` (pending and parked), metadata only:
    /// never payloads or captures.
    pub fn recent_rows(
        &self,
        account_id: &str,
        limit: usize,
    ) -> Result<Vec<QueueRow>, OutboxError> {
        let account = normalized_account(account_id)?;
        let limit = limit.max(1).min(self.limits.max_page_limit);
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, client_activity_id, project_id, started_at, ended_at,
                    state, retry_count, last_error, parked_reason, parked_at, capture_bytes,
                    format_version
             FROM outbox_entries WHERE account_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![&account, limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                workspace_id,
                client_activity_id,
                project_id,
                started_at,
                ended_at,
                state,
                retry_count,
                last_error,
                parked_reason,
                parked_at,
                capture_bytes,
                format_version,
            ) = row?;
            let state = RowState::from_i64(state).ok_or_else(|| OutboxError::CorruptRecord {
                id,
                reason: format!("Unknown row state {state}"),
            })?;
            out.push(QueueRow {
                id,
                workspace_id,
                client_activity_id,
                project_id,
                started_at: parse_utc(&started_at),
                ended_at: parse_utc(&ended_at),
                state,
                retry_count: u32::try_from(retry_count).unwrap_or(u32::MAX),
                last_error,
                parked_reason,
                parked_at: parked_at.as_deref().and_then(parse_utc),
                capture_bytes: capture_bytes.max(0) as u64,
                format_version: u32::try_from(format_version).unwrap_or(0),
            });
        }
        Ok(out)
    }

    /// Apply [`QueueLimits`] now (see [`Outbox::enforce_limits_at`]).
    pub fn enforce_limits(&self, limits: &QueueLimits) -> Result<LimitReport, OutboxError> {
        self.enforce_limits_at(limits, Utc::now())
    }

    /// Apply [`QueueLimits`] as of `now`, across all accounts: first shed the
    /// captures of pending rows past the newest `capture_max_rows`, past the
    /// `capture_max_bytes` budget (newest rows keep theirs) or queued more than
    /// `capture_max_age_days` ago; then drop pending rows beyond the newest
    /// `max_pending_rows`, and those whose block started more than
    /// `max_age_days` ago. Parked rows are left to the retention purge.
    pub fn enforce_limits_at(
        &self,
        limits: &QueueLimits,
        now: DateTime<Utc>,
    ) -> Result<LimitReport, OutboxError> {
        let capture_cutoff =
            rfc3339(now - chrono::Duration::days(i64::from(limits.capture_max_age_days)));
        let age_cutoff = rfc3339(now - chrono::Duration::days(i64::from(limits.max_age_days)));
        let max_bytes = i64::try_from(limits.capture_max_bytes).unwrap_or(i64::MAX);

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let shed_ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM (
                     SELECT id, capture_bytes, enqueued_at,
                            ROW_NUMBER() OVER newest AS rank,
                            SUM(capture_bytes) OVER newest AS kept_bytes
                     FROM outbox_entries WHERE state = 0
                     WINDOW newest AS (ORDER BY id DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW))
                 WHERE capture_bytes > 0
                   AND (rank > ?1 OR kept_bytes > ?2 OR julianday(enqueued_at) < julianday(?3))",
            )?;
            let ids = stmt.query_map(
                params![
                    i64::from(limits.capture_max_rows),
                    max_bytes,
                    &capture_cutoff
                ],
                |row| row.get(0),
            )?;
            ids.collect::<Result<_, _>>()?
        };
        let mut stripped = 0;
        for id in shed_ids {
            if strip_entry_captures(&tx, id)? {
                stripped += 1;
            }
        }

        let dropped_too_old = tx.execute(
            "DELETE FROM outbox_entries
             WHERE state = 0 AND julianday(started_at) < julianday(?1)",
            [&age_cutoff],
        )?;
        let dropped_overflow = tx.execute(
            "DELETE FROM outbox_entries WHERE state = 0 AND id NOT IN
                 (SELECT id FROM outbox_entries WHERE state = 0 ORDER BY id DESC LIMIT ?1)",
            [i64::from(limits.max_pending_rows)],
        )?;
        tx.commit()?;
        Ok(LimitReport {
            stripped,
            dropped_overflow,
            dropped_too_old,
        })
    }

    /// Drop parked rows parked more than `days` ago. Returns how many.
    pub fn purge_parked_older_than(&self, days: u32) -> Result<usize, OutboxError> {
        let cutoff = rfc3339(Utc::now() - chrono::Duration::days(i64::from(days)));
        let conn = self.lock_conn()?;
        Ok(conn.execute(
            "DELETE FROM outbox_entries
             WHERE state = 2 AND julianday(COALESCE(parked_at, enqueued_at)) < julianday(?1)",
            [&cutoff],
        )?)
    }

    /// Drop pending rows of any account other than `current_account` once they
    /// were queued more than `days` ago (with nobody signed in, every pending
    /// row older than `days`). Another account's rows wait for that account to
    /// sign in again, but not forever. Returns how many.
    pub fn purge_foreign_pending_older_than(
        &self,
        current_account: Option<&str>,
        days: u32,
    ) -> Result<usize, OutboxError> {
        let account = current_account.map(normalized_account).transpose()?;
        let cutoff = rfc3339(Utc::now() - chrono::Duration::days(i64::from(days)));
        let conn = self.lock_conn()?;
        Ok(conn.execute(
            "DELETE FROM outbox_entries
             WHERE state = 0 AND (?1 IS NULL OR account_id <> ?1)
               AND julianday(enqueued_at) < julianday(?2)",
            params![account, &cutoff],
        )?)
    }

    /// Delete `account`'s rows of the given kind (with their captures).
    /// Returns how many.
    pub fn purge(&self, account_id: &str, target: PurgeTarget) -> Result<usize, OutboxError> {
        let account = normalized_account(account_id)?;
        let conn = self.lock_conn()?;
        let removed = match target {
            PurgeTarget::All => conn.execute(
                "DELETE FROM outbox_entries WHERE account_id = ?1",
                [&account],
            )?,
            PurgeTarget::Pending | PurgeTarget::Parked => {
                let state = if target == PurgeTarget::Pending {
                    RowState::Pending
                } else {
                    RowState::Parked
                };
                conn.execute(
                    "DELETE FROM outbox_entries WHERE account_id = ?1 AND state = ?2",
                    params![&account, state.as_i64()],
                )?
            }
        };
        Ok(removed)
    }

    /// Give the pages freed by uploads, parked rows and shed captures back to
    /// the disk. Cheap when there is nothing to free. A store created without
    /// incremental auto-vacuum is rebuilt for it once its live data is small
    /// enough to rewrite quickly.
    pub fn reclaim_space(&self) -> Result<(), OutboxError> {
        let conn = self.lock_conn()?;
        const INCREMENTAL: i64 = 2;
        let pragma =
            |name: &str| conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0));
        if pragma("auto_vacuum")? != INCREMENTAL {
            let live_bytes =
                (pragma("page_count")? - pragma("freelist_count")?) * pragma("page_size")?;
            if live_bytes > VACUUM_SWITCH_MAX_LIVE_BYTES {
                return Ok(());
            }
            conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")?;
        }
        let mut stmt = conn.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = stmt.query([])?;
        while rows.next()?.is_some() {}
        Ok(())
    }

    /// Returns the total number of records (pending and parked) in the outbox,
    /// optionally filtered by scope.
    pub fn count(&self, scope: Option<&OutboxScope>) -> Result<u32, OutboxError> {
        if let Some(s) = scope {
            s.validate()?;
        }
        let conn = self.lock_conn()?;
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

    /// The number of pending (deliverable) rows of `scope`.
    pub fn pending_count(&self, scope: &OutboxScope) -> Result<u32, OutboxError> {
        scope.validate()?;
        let conn = self.lock_conn()?;
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM outbox_entries
             WHERE account_id = ?1 AND workspace_id = ?2 AND state = 0",
            params![&scope.account_id, &scope.workspace_id],
            |row| row.get(0),
        )?)
    }

    /// Returns the database file path.
    pub fn path(&self) -> &Path {
        &self.db_path
    }
}

fn check_identity_arg(client_activity_id: &str) -> Result<(), OutboxError> {
    if client_activity_id.trim().is_empty()
        || client_activity_id.len() > MAX_CLIENT_ACTIVITY_ID_BYTES
        || client_activity_id.chars().any(|c| c.is_control())
    {
        return Err(OutboxError::InvalidEnvelope(
            "client_activity_id invalid or too long".into(),
        ));
    }
    Ok(())
}

/// Empty the image bytes of row `id` (digests kept) and zero its
/// `capture_bytes`. Returns whether any bytes were removed.
fn strip_entry_captures(tx: &rusqlite::Transaction<'_>, id: i64) -> Result<bool, OutboxError> {
    let changed = tx.execute(
        "UPDATE outbox_captures SET jpeg = X'' WHERE entry_id = ?1 AND length(jpeg) > 0",
        [id],
    )?;
    tx.execute(
        "UPDATE outbox_entries SET capture_bytes = 0 WHERE id = ?1",
        [id],
    )?;
    Ok(changed > 0)
}

/// Rename a damaged store and its sidecars to `<name>.damaged-<utc>[-n]`.
fn set_aside(path: &Path, now: DateTime<Utc>) -> Result<PathBuf, OutboxError> {
    let name = path
        .file_name()
        .ok_or_else(|| OutboxError::Io("the outbox path has no file name".into()))?
        .to_string_lossy()
        .into_owned();
    let stamp = now.format("%Y%m%dT%H%M%SZ");
    let mut suffix = format!("damaged-{stamp}");
    let mut n = 1;
    while path.with_file_name(format!("{name}.{suffix}")).exists() {
        n += 1;
        suffix = format!("damaged-{stamp}-{n}");
    }
    let aside = path.with_file_name(format!("{name}.{suffix}"));
    std::fs::rename(path, &aside)
        .map_err(|e| OutboxError::Io(format!("Failed to move the damaged outbox aside: {e}")))?;
    for sidecar in ["-wal", "-shm"] {
        let from = path.with_file_name(format!("{name}{sidecar}"));
        if from.exists() {
            let to = path.with_file_name(format!("{name}{sidecar}.{suffix}"));
            std::fs::rename(&from, &to).map_err(|e| {
                OutboxError::Io(format!("Failed to move the damaged outbox aside: {e}"))
            })?;
        }
    }
    Ok(aside)
}

/// Versioned allowlisted summary of durable outbox queue status for a specific scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxStatusReport {
    /// Report format version.
    pub format_version: u32,
    /// Scoped tenant and workspace context.
    pub scope: OutboxScope,
    /// Aggregated metrics for the durable local queue.
    pub durable_queue: DurableQueueSummary,
    /// Explicit statement of remote synchronization/acknowledgement state.
    pub remote_delivery_state: String,
}

impl OutboxStatusReport {
    /// Serializes report to formatted JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Renders human-readable summary text.
    pub fn to_pretty_text(&self) -> String {
        format!(
            "=======================================================\n\
             Dosi-Tracker Linux Offline Outbox Status\n\
             =======================================================\n\n\
             [Scope]\n\
               Account ID:   {}\n\
               Workspace ID: {}\n\n\
             [Durable Queue Summary]\n\
               Queued Records:       {}\n\
               Entries With Retries: {}\n\
               Max Retry Count:      {}\n\
               Total Payload Bytes:  {} B\n\
               Parked Records:       {}\n\
               Capture Bytes:        {} B\n\
               Oldest Enqueued:      {}\n\
               Newest Enqueued:      {}\n\n\
             [Remote Delivery State]\n\
               Status: {} (offline storage only; remote sync state is not retained locally)\n",
            self.scope.account_id,
            self.scope.workspace_id,
            self.durable_queue.queued_count,
            self.durable_queue.entries_with_retries,
            self.durable_queue.max_retry_count,
            self.durable_queue.total_payload_bytes,
            self.durable_queue.parked_count,
            self.durable_queue.capture_bytes,
            self.durable_queue
                .oldest_enqueued_at
                .map(|t| t.to_rfc3339())
                .as_deref()
                .unwrap_or("none"),
            self.durable_queue
                .newest_enqueued_at
                .map(|t| t.to_rfc3339())
                .as_deref()
                .unwrap_or("none"),
            self.remote_delivery_state,
        )
    }
}

/// Aggregated durable queue summary for a selected scope.
///
/// The queue figures describe pending (deliverable) records; parked records
/// are only counted in `parked_count`. A v1 store has no parked records and
/// no captures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableQueueSummary {
    /// Total number of durable records queued for this scope.
    pub queued_count: u64,
    /// Number of records in this scope that have experienced at least one retry attempt.
    pub entries_with_retries: u64,
    /// Highest retry count observed among records in this scope.
    pub max_retry_count: u32,
    /// Total payload bytes of records in this scope.
    pub total_payload_bytes: u64,
    /// Oldest enqueued timestamp in this scope, if any.
    pub oldest_enqueued_at: Option<DateTime<Utc>>,
    /// Newest enqueued timestamp in this scope, if any.
    pub newest_enqueued_at: Option<DateTime<Utc>>,
    /// Records of this scope parked after a definitive refusal (store v2).
    #[serde(default)]
    pub parked_count: u64,
    /// Capture bytes still queued with this scope's pending records (store v2).
    #[serde(default)]
    pub capture_bytes: u64,
}

/// Allowlisted categories for scope errors in read-only status inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeErrorKind {
    /// Account identifier is empty.
    EmptyAccountId,
    /// Account identifier exceeds maximum length limit.
    OversizedAccountId,
    /// Account identifier contains illegal control characters.
    ControlCharAccountId,
    /// Workspace identifier is empty.
    EmptyWorkspaceId,
    /// Workspace identifier exceeds maximum length limit.
    OversizedWorkspaceId,
    /// Workspace identifier contains illegal control characters.
    ControlCharWorkspaceId,
}

impl ScopeErrorKind {
    /// Returns static allowlisted description of the scope validation error.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmptyAccountId => "account_id cannot be empty",
            Self::OversizedAccountId => "account_id exceeds maximum length of 256 bytes",
            Self::ControlCharAccountId => "account_id cannot contain control characters",
            Self::EmptyWorkspaceId => "workspace_id cannot be empty",
            Self::OversizedWorkspaceId => "workspace_id exceeds maximum length of 256 bytes",
            Self::ControlCharWorkspaceId => "workspace_id cannot contain control characters",
        }
    }
}

/// Allowlisted categories for schema errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaErrorKind {
    /// Database has not been initialized (user_version is 0).
    Uninitialized,
    /// The required table `outbox_entries` was not found.
    MissingTable,
    /// One or more required columns are missing from `outbox_entries`.
    MissingRequiredColumns,
}

impl SchemaErrorKind {
    /// Returns static allowlisted description of the schema validation error.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Uninitialized => "database is uninitialized (user_version is 0)",
            Self::MissingTable => "outbox_entries table not found",
            Self::MissingRequiredColumns => "required columns missing from outbox_entries",
        }
    }
}

/// Allowlisted categories for corrupt record metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCorruptionKind {
    /// A negative numeric value was detected in retry_count or payload_bytes.
    NegativeValue,
    /// Persisted retry count exceeds maximum supported 32-bit unsigned integer (u32::MAX).
    RetryCountOverflow,
    /// An invalid or unparseable ISO/RFC3339 timestamp was encountered.
    InvalidTimestamp,
    /// A row state other than pending (0) or parked (2) was encountered.
    UnknownRowState,
}

impl RecordCorruptionKind {
    /// Returns static allowlisted description of the record corruption error.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NegativeValue => "negative retry_count or payload_bytes detected in scope",
            Self::RetryCountOverflow => "persisted retry_count exceeds u32::MAX limit",
            Self::InvalidTimestamp => "invalid persisted timestamp in RFC3339 format",
            Self::UnknownRowState => "unknown persisted row state",
        }
    }
}

/// Allowlisted categories for database corruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseCorruptionKind {
    /// Database header or file structure is invalid or not a SQLite database.
    InvalidFormat,
    /// SQLite reports database disk image corruption or malformed pages.
    MalformedDiskImage,
    /// SQLite query execution failed on database structures.
    QueryExecutionFailed,
}

impl DatabaseCorruptionKind {
    /// Returns static allowlisted description of the database corruption error.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidFormat => "file is not a valid SQLite database",
            Self::MalformedDiskImage => "database disk image is malformed or corrupted",
            Self::QueryExecutionFailed => "query execution failed on database structures",
        }
    }
}

/// Errors returned by read-only outbox status inspection.
///
/// Invariant: All errors are bounded, allowlisted categories.
/// They never contain database file paths, payloads, foreign identities,
/// or arbitrary engine error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxStatusError {
    /// Database file does not exist.
    DatabaseNotFound,
    /// Database path is invalid (e.g. is a directory).
    InvalidPath,
    /// Scope validation failed.
    InvalidScope(ScopeErrorKind),
    /// Database file is corrupt or fails integrity check.
    CorruptDatabase(DatabaseCorruptionKind),
    /// Database record contains corrupt metadata.
    CorruptRecord(RecordCorruptionKind),
    /// Database schema does not match outbox requirements.
    UnsupportedSchema(SchemaErrorKind),
    /// Store schema version is unsupported.
    UnsupportedStoreVersion(u32),
    /// Read-only database access failed.
    AccessDenied,
    /// Scoped record count exceeds finite work budget limit.
    WorkloadExceeded,
}

impl std::fmt::Display for OutboxStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DatabaseNotFound => write!(f, "Database file does not exist"),
            Self::InvalidPath => {
                write!(
                    f,
                    "Invalid database path: specified path is a directory, not a database file"
                )
            }
            Self::InvalidScope(kind) => write!(f, "Invalid outbox scope: {}", kind.as_str()),
            Self::CorruptDatabase(kind) => write!(f, "Database is corrupted: {}", kind.as_str()),
            Self::CorruptRecord(kind) => write!(f, "Database record corrupted: {}", kind.as_str()),
            Self::UnsupportedSchema(kind) => {
                write!(f, "Unsupported database schema: {}", kind.as_str())
            }
            Self::UnsupportedStoreVersion(v) => {
                write!(
                    f,
                    "Unsupported outbox store version: {v} (current supported version is {OUTBOX_SCHEMA_VERSION})"
                )
            }
            Self::AccessDenied => {
                write!(
                    f,
                    "Database access error: unable to open database in read-only mode"
                )
            }
            Self::WorkloadExceeded => {
                write!(
                    f,
                    "Workload bound exceeded: scoped record count exceeds maximum inspection limit of {MAX_STATUS_ROW_BOUND}"
                )
            }
        }
    }
}

impl std::error::Error for OutboxStatusError {}

/// Maximum records examined for scoped read-only outbox status inspection.
pub const MAX_STATUS_ROW_BOUND: u64 = 10_000;

/// Number of SQLite virtual machine opcodes per progress step.
pub const PROGRESS_OPCODES_PER_STEP: i32 = 20;

/// Default maximum SQLite execution work steps allowed across all status inspection queries.
/// At 20 opcodes per step, 25,000 steps corresponds to 500,000 SQLite VDBE opcodes,
/// providing ample headroom for bounded (<= 10,000 row) indexed scope queries while
/// strictly halting full table scans on missing/malformed indexes or massive foreign datasets.
pub const DEFAULT_MAX_SQLITE_WORK_STEPS: u64 = 25_000;

/// Configurable finite SQLite execution work budget for status queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SqliteWorkBudget {
    pub max_work_steps: u64,
}

impl Default for SqliteWorkBudget {
    fn default() -> Self {
        Self {
            max_work_steps: DEFAULT_MAX_SQLITE_WORK_STEPS,
        }
    }
}

#[derive(Debug)]
struct ProgressContext<'a> {
    steps_taken: AtomicU64,
    max_steps: u64,
    external_counter: Option<&'a AtomicU64>,
}

unsafe extern "C" fn sqlite_progress_callback(arg: *mut std::ffi::c_void) -> std::ffi::c_int {
    if arg.is_null() {
        return 1;
    }
    let ctx = &*(arg as *const ProgressContext);
    let prev = ctx.steps_taken.fetch_add(1, Ordering::Relaxed);
    if let Some(ext) = ctx.external_counter {
        ext.store(prev + 1, Ordering::Relaxed);
    }
    if prev >= ctx.max_steps {
        1 // Non-zero interrupts execution (produces SQLITE_INTERRUPT)
    } else {
        0
    }
}

/// Safe scoped RAII wrapper for SQLite progress handler with guaranteed Drop deregistration.
/// Encapsulates raw FFI calls to `sqlite3_progress_handler` and ensures the callback is cleanly removed.
struct ScopedProgressHandler<'a> {
    handle: *mut rusqlite::ffi::sqlite3,
    _marker: std::marker::PhantomData<(&'a Connection, &'a ProgressContext<'a>)>,
}

impl<'a> ScopedProgressHandler<'a> {
    /// Installs a scoped progress handler on the SQLite connection.
    ///
    /// # Safety Invariant
    /// Borrows the connection and context for lifetime `'a` to ensure
    /// neither is dropped or relocated while the raw FFI callback is registered.
    fn install(conn: &'a Connection, opcodes: i32, ctx: &'a ProgressContext<'a>) -> Self {
        let handle = unsafe { conn.handle() };
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                handle,
                opcodes,
                Some(sqlite_progress_callback),
                ctx as *const ProgressContext<'a> as *mut std::ffi::c_void,
            );
        }
        Self {
            handle,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> Drop for ScopedProgressHandler<'a> {
    fn drop(&mut self) {
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(self.handle, 0, None, std::ptr::null_mut());
        }
    }
}

fn validate_scope_for_status(scope: &OutboxScope) -> Result<(), OutboxStatusError> {
    if scope.account_id.trim().is_empty() {
        return Err(OutboxStatusError::InvalidScope(
            ScopeErrorKind::EmptyAccountId,
        ));
    }
    if scope.account_id.len() > MAX_ACCOUNT_ID_BYTES {
        return Err(OutboxStatusError::InvalidScope(
            ScopeErrorKind::OversizedAccountId,
        ));
    }
    if scope.account_id.chars().any(|c| c.is_control()) {
        return Err(OutboxStatusError::InvalidScope(
            ScopeErrorKind::ControlCharAccountId,
        ));
    }
    if scope.workspace_id.trim().is_empty() {
        return Err(OutboxStatusError::InvalidScope(
            ScopeErrorKind::EmptyWorkspaceId,
        ));
    }
    if scope.workspace_id.len() > MAX_WORKSPACE_ID_BYTES {
        return Err(OutboxStatusError::InvalidScope(
            ScopeErrorKind::OversizedWorkspaceId,
        ));
    }
    if scope.workspace_id.chars().any(|c| c.is_control()) {
        return Err(OutboxStatusError::InvalidScope(
            ScopeErrorKind::ControlCharWorkspaceId,
        ));
    }
    Ok(())
}

fn map_sqlite_open_error(err: rusqlite::Error) -> OutboxStatusError {
    match err {
        rusqlite::Error::SqliteFailure(ffi_err, _) => match ffi_err.code {
            rusqlite::ErrorCode::NotADatabase => {
                OutboxStatusError::CorruptDatabase(DatabaseCorruptionKind::InvalidFormat)
            }
            rusqlite::ErrorCode::DatabaseCorrupt => {
                OutboxStatusError::CorruptDatabase(DatabaseCorruptionKind::MalformedDiskImage)
            }
            _ => OutboxStatusError::AccessDenied,
        },
        _ => OutboxStatusError::AccessDenied,
    }
}

fn map_sqlite_query_error(err: rusqlite::Error) -> OutboxStatusError {
    match err {
        rusqlite::Error::SqliteFailure(ffi_err, _) => match ffi_err.code {
            rusqlite::ErrorCode::NotADatabase => {
                OutboxStatusError::CorruptDatabase(DatabaseCorruptionKind::InvalidFormat)
            }
            rusqlite::ErrorCode::DatabaseCorrupt => {
                OutboxStatusError::CorruptDatabase(DatabaseCorruptionKind::MalformedDiskImage)
            }
            rusqlite::ErrorCode::OperationInterrupted => OutboxStatusError::WorkloadExceeded,
            _ => OutboxStatusError::CorruptDatabase(DatabaseCorruptionKind::QueryExecutionFailed),
        },
        _ => OutboxStatusError::CorruptDatabase(DatabaseCorruptionKind::QueryExecutionFailed),
    }
}

/// Performs strictly read-only inspection of the durable outbox queue for a specific scope.
///
/// Invariants:
/// - Explicit existing DB path required; fails closed if path does not exist.
/// - Never creates files, directories, schemas, or tables.
/// - Never executes PRAGMA writes, migrations, checkpoints, enqueues, or retries.
/// - Opens SQLite using `OpenFlags::SQLITE_OPEN_READ_ONLY`.
/// - Evaluates SQLite WAL mode safely; uncheckpointed WAL frames are read if WAL/SHM
///   files are readable, and connection conservatively fails closed if SHM/WAL are unreadable.
/// - Strictly bounded work budget: evaluates at most [`MAX_STATUS_ROW_BOUND`] records
///   and [`DEFAULT_MAX_SQLITE_WORK_STEPS`] SQLite virtual machine execution steps,
///   rejecting overflows without returning partial successful counts.
/// - Excludes envelopes, payloads, descriptions, foreign identities, and raw engine strings.
pub fn inspect_outbox_status(
    path: impl AsRef<Path>,
    scope: &OutboxScope,
) -> Result<OutboxStatusReport, OutboxStatusError> {
    inspect_outbox_status_bounded(path, scope, MAX_STATUS_ROW_BOUND)
}

/// Performs strictly read-only inspection of the durable outbox queue with an explicit finite row bound.
pub fn inspect_outbox_status_bounded(
    path: impl AsRef<Path>,
    scope: &OutboxScope,
    row_limit: u64,
) -> Result<OutboxStatusReport, OutboxStatusError> {
    inspect_outbox_status_with_budget(path, scope, row_limit, DEFAULT_MAX_SQLITE_WORK_STEPS)
}

/// Performs strictly read-only inspection of the durable outbox queue with explicit row bound and SQLite work step budget.
pub fn inspect_outbox_status_with_budget(
    path: impl AsRef<Path>,
    scope: &OutboxScope,
    row_limit: u64,
    max_work_steps: u64,
) -> Result<OutboxStatusReport, OutboxStatusError> {
    inspect_outbox_status_instrumented(path, scope, row_limit, max_work_steps, None)
}

/// Performs strictly read-only inspection of the durable outbox queue with step instrumentation.
pub fn inspect_outbox_status_instrumented(
    path: impl AsRef<Path>,
    scope: &OutboxScope,
    row_limit: u64,
    max_work_steps: u64,
    step_counter: Option<&AtomicU64>,
) -> Result<OutboxStatusReport, OutboxStatusError> {
    // 1. Check row_limit range BEFORE narrowing or arithmetic
    if row_limit == 0 || row_limit > MAX_STATUS_ROW_BOUND {
        return Err(OutboxStatusError::WorkloadExceeded);
    }

    // 2. Check max_work_steps range against documented non-bypassable limit
    if max_work_steps == 0 || max_work_steps > DEFAULT_MAX_SQLITE_WORK_STEPS {
        return Err(OutboxStatusError::WorkloadExceeded);
    }

    let db_path = path.as_ref();

    // 3. Mandatory check: DB path must exist and not be a directory
    if !db_path.exists() {
        return Err(OutboxStatusError::DatabaseNotFound);
    }
    if db_path.is_dir() {
        return Err(OutboxStatusError::InvalidPath);
    }

    // 4. Validate scope strictly with allowlisted categories
    validate_scope_for_status(scope)?;

    // 5. Open connection strictly READ-ONLY
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db_path, flags).map_err(map_sqlite_open_error)?;

    // 6. Install finite execution work budget handler covering all validation and summary queries
    let progress_ctx = ProgressContext {
        steps_taken: AtomicU64::new(0),
        max_steps: max_work_steps,
        external_counter: step_counter,
    };
    let _progress_guard =
        ScopedProgressHandler::install(&conn, PROGRESS_OPCODES_PER_STEP, &progress_ctx);

    // 7. Check schema version via PRAGMA user_version
    let user_ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(map_sqlite_query_error)?;
    if user_ver == 0 {
        return Err(OutboxStatusError::UnsupportedSchema(
            SchemaErrorKind::Uninitialized,
        ));
    }
    if user_ver > OUTBOX_SCHEMA_VERSION {
        return Err(OutboxStatusError::UnsupportedStoreVersion(user_ver));
    }

    // 8. Check table existence
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'outbox_entries';",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_sqlite_query_error)?
        .is_some();

    if !table_exists {
        return Err(OutboxStatusError::UnsupportedSchema(
            SchemaErrorKind::MissingTable,
        ));
    }

    // 9. Verify required columns in table
    let mut table_info_stmt = conn
        .prepare("PRAGMA table_info(outbox_entries);")
        .map_err(map_sqlite_query_error)?;
    // Row errors propagate: an interrupted (budget-exhausted) column listing
    // must fail as WorkloadExceeded, not pass as a partial column set.
    let columns: Vec<String> = table_info_stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(map_sqlite_query_error)?
        .collect::<Result<_, _>>()
        .map_err(map_sqlite_query_error)?;

    let required_cols = [
        "account_id",
        "workspace_id",
        "client_activity_id",
        "payload_bytes",
        "enqueued_at",
        "retry_count",
        "last_error",
    ];
    for req in required_cols {
        if !columns.iter().any(|c| c == req) {
            return Err(OutboxStatusError::UnsupportedSchema(
                SchemaErrorKind::MissingRequiredColumns,
            ));
        }
    }
    // A v2 store has row states and capture sizes; a v1 store has neither
    // (every row pending, no captures). Decided by the columns present, so a
    // v1-shaped table is read as v1 whatever its version stamp.
    let has_v2_columns = ["state", "capture_bytes"]
        .iter()
        .all(|req| columns.iter().any(|c| c == req));

    // 10. Execute single bounded SQL aggregate summary query with folded metadata validation.
    // fetch_limit is safe from overflow because row_limit <= MAX_STATUS_ROW_BOUND (10,000).
    // The CTE bounds examined rows to fetch_limit (row_limit + 1). No unbounded prescan is performed.
    let fetch_limit = (row_limit as i64) + 1;

    let summary_sql = if has_v2_columns {
        "WITH scoped_rows AS (
             SELECT retry_count, payload_bytes, enqueued_at, state, capture_bytes
             FROM outbox_entries
             WHERE account_id = ?1 AND workspace_id = ?2
             LIMIT ?3
         )
         SELECT \
             COUNT(*), \
             COALESCE(SUM(state = 0), 0), \
             COALESCE(SUM(state = 0 AND retry_count > 0), 0), \
             COALESCE(MAX(CASE WHEN state = 0 THEN retry_count END), 0), \
             MIN(retry_count), \
             MIN(payload_bytes), \
             COALESCE(SUM(CASE WHEN state = 0 THEN payload_bytes ELSE 0 END), 0), \
             MIN(CASE WHEN state = 0 THEN enqueued_at END), \
             MAX(CASE WHEN state = 0 THEN enqueued_at END), \
             COALESCE(SUM(state = 2), 0), \
             COALESCE(SUM(state NOT IN (0, 2)), 0), \
             MIN(capture_bytes), \
             COALESCE(SUM(CASE WHEN state = 0 THEN capture_bytes ELSE 0 END), 0) \
         FROM scoped_rows"
    } else {
        "WITH scoped_rows AS (
             SELECT retry_count, payload_bytes, enqueued_at
             FROM outbox_entries
             WHERE account_id = ?1 AND workspace_id = ?2
             LIMIT ?3
         )
         SELECT \
             COUNT(*), \
             COUNT(*), \
             COALESCE(SUM(CASE WHEN retry_count > 0 THEN 1 ELSE 0 END), 0), \
             COALESCE(MAX(retry_count), 0), \
             MIN(retry_count), \
             MIN(payload_bytes), \
             COALESCE(SUM(payload_bytes), 0), \
             MIN(enqueued_at), \
             MAX(enqueued_at), \
             0, 0, 0, 0 \
         FROM scoped_rows"
    };

    let summary_row = conn
        .query_row(
            summary_sql,
            params![&scope.account_id, &scope.workspace_id, fetch_limit],
            |row| {
                Ok(SummaryRow {
                    examined_count: row.get(0)?,
                    queued_count: row.get(1)?,
                    entries_with_retries: row.get(2)?,
                    max_retry_count: row.get(3)?,
                    min_retry_count: row.get(4)?,
                    min_payload_bytes: row.get(5)?,
                    total_payload_bytes: row.get(6)?,
                    oldest_raw: row.get(7)?,
                    newest_raw: row.get(8)?,
                    parked_count: row.get(9)?,
                    unknown_state_count: row.get(10)?,
                    min_capture_bytes: row.get(11)?,
                    capture_bytes: row.get(12)?,
                })
            },
        )
        .map_err(map_sqlite_query_error)?;

    let SummaryRow {
        examined_count,
        queued_count,
        entries_with_retries,
        max_retry_count,
        min_retry_count,
        min_payload_bytes,
        total_payload_bytes,
        oldest_raw,
        newest_raw,
        parked_count,
        unknown_state_count,
        min_capture_bytes,
        capture_bytes,
    } = summary_row;

    // Check workload overflow: if count exceeds the bounded limit, fail closed.
    // Rejects partial counts completely!
    if (examined_count as u64) > row_limit {
        return Err(OutboxStatusError::WorkloadExceeded);
    }
    if unknown_state_count != 0 {
        return Err(OutboxStatusError::CorruptRecord(
            RecordCorruptionKind::UnknownRowState,
        ));
    }
    if min_capture_bytes.is_some_and(|m| m < 0) || capture_bytes < 0 {
        return Err(OutboxStatusError::CorruptRecord(
            RecordCorruptionKind::NegativeValue,
        ));
    }

    // Fail-closed checks on examined records:
    if let Some(min_r) = min_retry_count {
        if min_r < 0 {
            return Err(OutboxStatusError::CorruptRecord(
                RecordCorruptionKind::NegativeValue,
            ));
        }
    }
    if let Some(min_p) = min_payload_bytes {
        if min_p < 0 {
            return Err(OutboxStatusError::CorruptRecord(
                RecordCorruptionKind::NegativeValue,
            ));
        }
    }
    if max_retry_count > u32::MAX as i64 {
        return Err(OutboxStatusError::CorruptRecord(
            RecordCorruptionKind::RetryCountOverflow,
        ));
    }
    if total_payload_bytes < 0 {
        return Err(OutboxStatusError::CorruptRecord(
            RecordCorruptionKind::NegativeValue,
        ));
    }

    // Checked numeric conversions
    let queued_count = u64::try_from(queued_count)
        .map_err(|_| OutboxStatusError::CorruptRecord(RecordCorruptionKind::NegativeValue))?;
    let entries_with_retries = u64::try_from(entries_with_retries)
        .map_err(|_| OutboxStatusError::CorruptRecord(RecordCorruptionKind::NegativeValue))?;
    let max_retry_count = u32::try_from(max_retry_count)
        .map_err(|_| OutboxStatusError::CorruptRecord(RecordCorruptionKind::RetryCountOverflow))?;
    let total_payload_bytes = u64::try_from(total_payload_bytes)
        .map_err(|_| OutboxStatusError::CorruptRecord(RecordCorruptionKind::NegativeValue))?;

    // 11. Validate ISO timestamps if present
    let oldest_enqueued_at = match oldest_raw {
        Some(ref s) => {
            let dt = s.parse::<DateTime<Utc>>().map_err(|_| {
                OutboxStatusError::CorruptRecord(RecordCorruptionKind::InvalidTimestamp)
            })?;
            Some(dt)
        }
        None => None,
    };

    let newest_enqueued_at = match newest_raw {
        Some(ref s) => {
            let dt = s.parse::<DateTime<Utc>>().map_err(|_| {
                OutboxStatusError::CorruptRecord(RecordCorruptionKind::InvalidTimestamp)
            })?;
            Some(dt)
        }
        None => None,
    };

    Ok(OutboxStatusReport {
        format_version: 1,
        scope: scope.clone(),
        durable_queue: DurableQueueSummary {
            queued_count,
            entries_with_retries,
            max_retry_count,
            total_payload_bytes,
            oldest_enqueued_at,
            newest_enqueued_at,
            parked_count: parked_count.max(0) as u64,
            capture_bytes: capture_bytes.max(0) as u64,
        },
        remote_delivery_state: "untracked_in_offline_outbox".to_string(),
    })
}

/// One row of the inspector's aggregate query.
struct SummaryRow {
    examined_count: i64,
    queued_count: i64,
    entries_with_retries: i64,
    max_retry_count: i64,
    min_retry_count: Option<i64>,
    min_payload_bytes: Option<i64>,
    total_payload_bytes: i64,
    oldest_raw: Option<String>,
    newest_raw: Option<String>,
    parked_count: i64,
    unknown_state_count: i64,
    min_capture_bytes: Option<i64>,
    capture_bytes: i64,
}
