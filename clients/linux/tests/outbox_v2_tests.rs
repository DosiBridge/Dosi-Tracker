//! Outbox store v2: migration, captures, signatures, parking, limits, private
//! files, recovery and the read-only inspector.

use std::fs;
use std::path::Path;

use chrono::{Duration, TimeZone, Utc};
use dosi_tracker_linux::agent::model::{Activity, PLATFORM};
use dosi_tracker_linux::api::verify_v2_entry;
use dosi_tracker_linux::outbox::{
    capture_digests, inspect_outbox_status, ActivityEnvelope, CaptureKind, EnqueueResult, Outbox,
    OutboxError, OutboxLimits, OutboxScope, PendingActivity, PendingCapture, PurgeTarget,
    QueueLimits, RowState, OUTBOX_SCHEMA_VERSION, ROW_FORMAT_V1, ROW_FORMAT_V2,
};
use dosi_tracker_linux::signing::{sign_v2, verify_v2};
use proptest::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

const KEY: [u8; 32] = [3u8; 32];
const PROJECT: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
const WS: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const WS2: &str = "7f9619ff-8b86-d011-b42d-00c04fc964ff";
const ACCOUNT: &str = "alice@acme.test";

/// The exact v1 schema (`outbox.rs` before store v2).
const V1_SCHEMA: &str = "
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

fn scope() -> OutboxScope {
    OutboxScope::new(ACCOUNT, WS).unwrap()
}

fn envelope(id: &str) -> ActivityEnvelope {
    let now = Utc::now();
    ActivityEnvelope {
        format_version: 1,
        client_activity_id: id.to_string(),
        project_id: Some(PROJECT.to_string()),
        started_at: now - Duration::minutes(10),
        ended_at: now,
        productivity: None,
        mouse_clicks: None,
        keyboard_hits: Some(5),
        description: Some(format!("v1 {id}")),
        device_id: None,
        platform: "linux".into(),
        agent_version: "0.1.0".into(),
        payload_signature: None,
    }
}

/// A store written by the v1 code: its schema, `user_version = 1` and v1 rows.
fn build_v1_store(path: &Path, rows: &[(&str, &str, &str)]) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch(V1_SCHEMA).unwrap();
    for (account, workspace, id) in rows {
        let env = envelope(id);
        let json = serde_json::to_string(&env).unwrap();
        conn.execute(
            "INSERT INTO outbox_entries (account_id, workspace_id, client_activity_id, project_id,
                 started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
            rusqlite::params![
                account,
                workspace,
                id,
                env.project_id,
                env.started_at.to_rfc3339(),
                env.ended_at.to_rfc3339(),
                json,
                json.len() as i64,
                Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
    }
    conn.pragma_update(None, "user_version", 1).unwrap();
}

fn block(start_offset_min: i64) -> Activity {
    let start = Utc::now() - Duration::minutes(start_offset_min);
    let mut a = Activity::new(PROJECT, start, start + Duration::minutes(10));
    a.platform = Some(PLATFORM.into());
    a.description = Some("design review".into());
    a
}

fn jpeg(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.extend((0..len.saturating_sub(4)).map(|i| seed.wrapping_add(i as u8)));
    bytes
}

fn shots(screen_len: usize) -> Vec<PendingCapture> {
    vec![
        PendingCapture {
            kind: CaptureKind::Screen,
            jpeg: jpeg(screen_len, 1),
            captured_at: Utc::now(),
            blurred: false,
        },
        PendingCapture {
            kind: CaptureKind::Thumb,
            jpeg: jpeg(64, 9),
            captured_at: Utc::now(),
            blurred: false,
        },
    ]
}

fn enqueue(
    outbox: &Outbox,
    scope: &OutboxScope,
    activity: &Activity,
    captures: Vec<PendingCapture>,
) -> i64 {
    let pending = PendingActivity::from_activity(activity, captures).unwrap();
    match outbox.enqueue_activity(scope, &pending, &KEY).unwrap() {
        EnqueueResult::Inserted(id) => id,
        other => panic!("expected an insert, got {other:?}"),
    }
}

fn columns(path: &Path) -> Vec<String> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn.prepare("PRAGMA table_info(outbox_entries)").unwrap();
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
}

fn user_version(path: &Path) -> u32 {
    Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

// --- Migration -----------------------------------------------------------------

#[test]
fn a_v1_store_is_migrated_and_its_rows_stay_readable() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    build_v1_store(
        &path,
        &[
            (ACCOUNT, WS, "v1-a"),
            (ACCOUNT, WS, "v1-b"),
            ("bob@acme.test", WS, "v1-c"),
        ],
    );
    assert_eq!(user_version(&path), 1);

    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    assert_eq!(user_version(&path), 2);
    let cols = columns(&path);
    for col in [
        "state",
        "parked_at",
        "parked_reason",
        "capture_bytes",
        "format_version",
        "payload_signature",
    ] {
        assert!(cols.contains(&col.to_string()), "{col} added");
    }

    let rows = outbox.peek_ready(&scope(), 10).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].client_activity_id, "v1-a");
    assert_eq!(rows[0].format_version, ROW_FORMAT_V1);
    assert_eq!(rows[0].envelope.description.as_deref(), Some("v1 v1-a"));
    assert_eq!(
        rows[0].envelope.mouse_clicks, None,
        "unmeasured stays unmeasured"
    );
    assert_eq!(rows[0].capture_bytes, 0);

    // v1 and v2 rows live side by side.
    enqueue(&outbox, &scope(), &block(30), shots(1000));
    let rows = outbox.peek_ready(&scope(), 10).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].format_version, ROW_FORMAT_V2);

    // Reopening is a no-op.
    drop(outbox);
    let reopened = Outbox::open(&path, OutboxLimits::default()).unwrap();
    assert_eq!(reopened.peek_ready(&scope(), 10).unwrap().len(), 3);
    assert_eq!(user_version(&path), OUTBOX_SCHEMA_VERSION);
}

#[test]
fn a_failed_migration_rolls_back_completely() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    build_v1_store(&path, &[(ACCOUNT, WS, "v1-a")]);
    // Something in the way of the new table makes the migration fail halfway.
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE outbox_captures (unrelated TEXT);")
        .unwrap();
    let before = columns(&path);

    let err = Outbox::open(&path, OutboxLimits::default()).unwrap_err();
    assert!(matches!(err, OutboxError::Database(_)), "{err}");
    assert_eq!(user_version(&path), 1, "not stamped");
    assert_eq!(columns(&path), before, "no column was added");
}

#[test]
fn a_newer_store_is_refused_and_left_byte_identical() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
    }
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        Outbox::open(&path, OutboxLimits::default()),
        Err(OutboxError::UnsupportedStoreVersion(3))
    ));
    assert!(matches!(
        Outbox::open_or_recover(&path, OutboxLimits::default()),
        Err(OutboxError::UnsupportedStoreVersion(3))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "nothing moved aside"
    );
}

// --- Enqueue, captures and signatures -----------------------------------------

#[test]
fn a_block_and_its_captures_round_trip_and_verify() {
    let dir = tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    let activity = block(30);
    let id = enqueue(&outbox, &scope(), &activity, shots(5000));

    let mut rows = outbox.peek_ready(&scope(), 10).unwrap();
    assert_eq!(rows.len(), 1);
    let mut row = rows.remove(0);
    assert_eq!(row.id, id);
    assert_eq!(row.format_version, ROW_FORMAT_V2);
    assert_eq!(row.capture_bytes, 5000 + 64);
    assert!(row.captures.is_empty(), "bytes are loaded separately");
    let payload = row.payload_json.clone().unwrap();
    assert!(
        !payload.contains("screenshot"),
        "no capture bytes in the payload"
    );
    assert!(!payload.contains("sentAt") && !payload.contains("payloadSignature"));
    assert_eq!(row.envelope.description.as_deref(), Some("design review"));
    assert_eq!(row.envelope.project_id.as_deref(), Some(PROJECT));

    row.captures = outbox.load_captures(&scope(), id).unwrap();
    assert_eq!(row.captures.len(), 2);
    assert_eq!(row.captures[0].kind, CaptureKind::Screen);
    assert_eq!(row.captures[0].jpeg, jpeg(5000, 1));
    assert!(verify_v2_entry(&KEY, &row));
    assert!(!verify_v2_entry(&[4u8; 32], &row), "another key");

    // The stored signature is the documented v2 input.
    let digests = capture_digests(&row.captures);
    let sig = row.payload_signature.clone().unwrap();
    assert!(verify_v2(&KEY, payload.as_bytes(), &digests, &sig));
    assert_eq!(sign_v2(&KEY, payload.as_bytes(), &digests), sig);

    // Another scope never sees the captures.
    let other = OutboxScope::new(ACCOUNT, WS2).unwrap();
    assert!(outbox.load_captures(&other, id).unwrap().is_empty());
}

#[test]
fn identical_rows_are_idempotent_and_different_ones_conflict() {
    let dir = tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    let activity = block(30);
    let pending = PendingActivity::from_activity(&activity, shots(100)).unwrap();
    let EnqueueResult::Inserted(id) = outbox.enqueue_activity(&scope(), &pending, &KEY).unwrap()
    else {
        panic!("first enqueue inserts");
    };
    assert_eq!(
        outbox.enqueue_activity(&scope(), &pending, &KEY).unwrap(),
        EnqueueResult::Idempotent(id)
    );

    let mut changed = pending.clone();
    changed.captures[0].jpeg.push(1);
    assert!(matches!(
        outbox.enqueue_activity(&scope(), &changed, &KEY),
        Err(OutboxError::ConflictingIdentity { .. })
    ));
    let mut other_note = activity.clone();
    other_note.description = Some("changed".into());
    let other_note = PendingActivity::from_activity(&other_note, vec![]).unwrap();
    assert!(matches!(
        outbox.enqueue_activity(&scope(), &other_note, &KEY),
        Err(OutboxError::ConflictingIdentity { .. })
    ));
    assert_eq!(outbox.count(None).unwrap(), 1);
}

#[test]
fn invalid_rows_are_refused_before_anything_is_written() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let good = PendingActivity::from_activity(&block(30), shots(100)).unwrap();
    let refuse = |pending: &PendingActivity, key: &[u8]| {
        let err = outbox.enqueue_activity(&scope(), pending, key).unwrap_err();
        assert!(
            matches!(
                err,
                OutboxError::InvalidEnvelope(_) | OutboxError::PayloadTooLarge { .. }
            ),
            "{err}"
        );
    };

    refuse(&good, &[]);
    let mut p = good.clone();
    p.project_id = "website".into();
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.project_id = uuid::Uuid::nil().to_string();
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.client_activity_id = "act-1".into();
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.payload_json = p
        .payload_json
        .replacen('{', r#"{"screenshotPngBase64":"AA==","#, 1);
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.payload_json = p
        .payload_json
        .replacen('{', r#"{"sentAt":"2026-10-05T10:00:00Z","#, 1);
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.started_at += Duration::seconds(1);
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.captures.push(p.captures[0].clone());
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.captures[1].jpeg = vec![0u8; 512 * 1024 + 1];
    refuse(&p, &KEY);
    let mut p = good.clone();
    p.captures[0].jpeg.clear();
    refuse(&p, &KEY);
    let mut device = block(30);
    device.device_id = Some("my-laptop".into());
    refuse(
        &PendingActivity::from_activity(&device, vec![]).unwrap(),
        &KEY,
    );

    assert_eq!(outbox.count(None).unwrap(), 0);
    let conn = Connection::open(&path).unwrap();
    let captures: i64 = conn
        .query_row("SELECT COUNT(*) FROM outbox_captures", [], |r| r.get(0))
        .unwrap();
    assert_eq!(captures, 0);
}

#[test]
fn tampering_with_a_payload_or_a_capture_fails_verification() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let id = enqueue(&outbox, &scope(), &block(30), shots(300));
    let load = || {
        let mut row = outbox.peek_ready(&scope(), 1).unwrap().remove(0);
        row.captures = outbox.load_captures(&scope(), id).unwrap();
        row
    };
    assert!(verify_v2_entry(&KEY, &load()));

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE outbox_captures SET jpeg = X'FFD8FFE0' WHERE kind = 'thumb'",
        [],
    )
    .unwrap();
    let row = load();
    assert!(!row.captures[1].bytes_match_digest());
    assert!(!verify_v2_entry(&KEY, &row));

    // Updating the digest too does not help: the signature binds it.
    conn.execute(
        "UPDATE outbox_captures SET sha256 = ?1 WHERE kind = 'thumb'",
        [hex_sha256(&[0xFF, 0xD8, 0xFF, 0xE0])],
    )
    .unwrap();
    assert!(!verify_v2_entry(&KEY, &load()));
}

fn hex_sha256(bytes: &[u8]) -> String {
    dosi_tracker_linux::signing::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn stripping_keeps_the_digests_so_the_row_still_verifies() {
    let dir = tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    let activity = block(30);
    let id = enqueue(&outbox, &scope(), &activity, shots(300));
    assert!(outbox
        .strip_captures(&scope(), &activity.client_activity_id)
        .unwrap());
    assert!(
        !outbox
            .strip_captures(&scope(), &activity.client_activity_id)
            .unwrap(),
        "nothing left to strip"
    );
    let mut row = outbox.peek_ready(&scope(), 1).unwrap().remove(0);
    assert_eq!(row.capture_bytes, 0);
    row.captures = outbox.load_captures(&scope(), id).unwrap();
    assert!(row.captures.iter().all(|c| !c.has_bytes()));
    assert!(verify_v2_entry(&KEY, &row));
    assert!(matches!(
        outbox.strip_captures(&scope(), "unknown"),
        Err(OutboxError::RecordNotFound { .. })
    ));
}

// --- Parking, summaries and purges --------------------------------------------

#[test]
fn parked_rows_leave_the_queue_but_stay_visible() {
    let dir = tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    let a = block(50);
    let b = block(40);
    enqueue(&outbox, &scope(), &a, shots(200));
    enqueue(&outbox, &scope(), &b, vec![]);
    let long_reason = "é".repeat(800);
    outbox
        .park(&scope(), &a.client_activity_id, &long_reason)
        .unwrap();

    let ready = outbox.peek_ready(&scope(), 10).unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].client_activity_id, b.client_activity_id);
    assert_eq!(outbox.pending_count(&scope()).unwrap(), 1);
    assert_eq!(outbox.count(Some(&scope())).unwrap(), 2);

    let rows = outbox.recent_rows(ACCOUNT, 10).unwrap();
    let parked = rows
        .iter()
        .find(|r| r.client_activity_id == a.client_activity_id)
        .unwrap();
    assert_eq!(parked.state, RowState::Parked);
    let reason = parked.parked_reason.as_deref().unwrap();
    assert!(
        reason.len() <= 1024 && reason.starts_with('é'),
        "bounded at a char boundary"
    );
    assert!(parked.parked_at.is_some());
    assert_eq!(parked.capture_bytes, 0, "a parked row drops its captures");
    let listed = serde_json::to_string(&rows).unwrap();
    assert!(!listed.contains("design review"), "never payloads");

    let summary = outbox.queue_summary(ACCOUNT).unwrap();
    assert_eq!((summary.pending, summary.parked), (1, 1));
    assert!(summary.oldest_pending_enqueued_at.is_some());

    // Parked rows never take attempts or another park.
    assert!(matches!(
        outbox.mark_attempt_failed(&scope(), &a.client_activity_id, "x"),
        Err(OutboxError::RecordNotFound { .. })
    ));
    assert!(matches!(
        outbox.park(&scope(), &a.client_activity_id, "again"),
        Err(OutboxError::RecordNotFound { .. })
    ));
}

#[test]
fn scopes_summaries_and_purges_are_per_account() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let ws2 = OutboxScope::new(ACCOUNT, WS2).unwrap();
    let bob = OutboxScope::new("bob@acme.test", WS).unwrap();
    enqueue(&outbox, &ws2, &block(90), vec![]);
    enqueue(&outbox, &scope(), &block(80), shots(100));
    enqueue(&outbox, &scope(), &block(70), vec![]);
    enqueue(&outbox, &bob, &block(60), vec![]);

    assert_eq!(
        outbox.pending_scopes("  ALICE@acme.test ").unwrap(),
        vec![ws2.clone(), scope()]
    );
    let summary = outbox.queue_summary(ACCOUNT).unwrap();
    assert_eq!(summary.pending, 3);
    assert_eq!(summary.capture_bytes, 164);
    assert_eq!(summary.foreign_pending, 1);

    assert_eq!(outbox.park_scope(&scope(), "device revoked").unwrap(), 2);
    assert_eq!(outbox.pending_scopes(ACCOUNT).unwrap(), vec![ws2.clone()]);
    assert_eq!(outbox.purge(ACCOUNT, PurgeTarget::Parked).unwrap(), 2);
    assert_eq!(outbox.purge(ACCOUNT, PurgeTarget::Pending).unwrap(), 1);
    assert_eq!(outbox.purge(ACCOUNT, PurgeTarget::All).unwrap(), 0);
    assert_eq!(outbox.count(None).unwrap(), 1, "bob's row is untouched");
    let captures: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM outbox_captures", [], |r| r.get(0))
        .unwrap();
    assert_eq!(captures, 0, "purged with their rows");
}

#[test]
fn retention_purges_old_parked_and_old_foreign_rows_only() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let bob = OutboxScope::new("bob@acme.test", WS).unwrap();
    let old_parked = block(100);
    let new_parked = block(90);
    enqueue(&outbox, &scope(), &old_parked, vec![]);
    enqueue(&outbox, &scope(), &new_parked, vec![]);
    outbox
        .park(&scope(), &old_parked.client_activity_id, "x")
        .unwrap();
    outbox
        .park(&scope(), &new_parked.client_activity_id, "x")
        .unwrap();
    let old_foreign = block(80);
    enqueue(&outbox, &bob, &old_foreign, vec![]);
    enqueue(&outbox, &bob, &block(70), vec![]);
    let mine = block(60);
    enqueue(&outbox, &scope(), &mine, vec![]);
    {
        let conn = Connection::open(&path).unwrap();
        let old = (Utc::now() - Duration::days(20)).to_rfc3339();
        conn.execute(
            "UPDATE outbox_entries SET parked_at = ?1 WHERE client_activity_id = ?2",
            [&old, &old_parked.client_activity_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE outbox_entries SET enqueued_at = ?1 WHERE client_activity_id IN (?2, ?3)",
            [
                &old,
                &old_foreign.client_activity_id,
                &mine.client_activity_id,
            ],
        )
        .unwrap();
    }
    assert_eq!(outbox.purge_parked_older_than(14).unwrap(), 1);
    assert_eq!(
        outbox
            .purge_foreign_pending_older_than(Some(ACCOUNT), 14)
            .unwrap(),
        1
    );
    let summary = outbox.queue_summary(ACCOUNT).unwrap();
    assert_eq!(
        (summary.pending, summary.parked, summary.foreign_pending),
        (1, 1, 1)
    );
    // Nobody signed in: every old pending row goes.
    assert_eq!(
        outbox.purge_foreign_pending_older_than(None, 14).unwrap(),
        1
    );
}

// --- Queue limits ----------------------------------------------------------------

#[test]
fn limits_shed_captures_first_and_drop_only_beyond_the_row_cap_or_age() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let limits = QueueLimits {
        capture_max_rows: 2,
        capture_max_bytes: 10_000,
        capture_max_age_days: 30,
        max_pending_rows: 4,
        max_age_days: 45,
    };
    for i in 0..5 {
        enqueue(&outbox, &scope(), &block(300 - i * 20), shots(1000));
    }
    let report = outbox.enforce_limits(&limits).unwrap();
    assert_eq!(
        report.stripped, 3,
        "only the newest two keep their captures"
    );
    assert_eq!(report.dropped_overflow, 1);
    assert_eq!(report.dropped_too_old, 0);
    let rows = outbox.peek_ready(&scope(), 10).unwrap();
    assert_eq!(rows.len(), 4);
    let with_captures: Vec<u64> = rows.iter().map(|r| r.capture_bytes).collect();
    assert_eq!(with_captures, vec![0, 0, 1064, 1064]);

    // A block older than the server accepts is dropped.
    let ancient = {
        let start = Utc::now() - Duration::days(46);
        let mut a = Activity::new(PROJECT, start, start + Duration::minutes(10));
        a.platform = Some(PLATFORM.into());
        a
    };
    enqueue(&outbox, &scope(), &ancient, vec![]);
    let report = outbox.enforce_limits(&limits).unwrap();
    assert_eq!(report.dropped_too_old, 1);
    assert_eq!(report.dropped(), 1);

    // Parked rows are not counted against the pending cap.
    let parked = outbox.peek_ready(&scope(), 1).unwrap().remove(0);
    outbox
        .park(&scope(), &parked.client_activity_id, "x")
        .unwrap();
    let report = outbox.enforce_limits(&limits).unwrap();
    assert_eq!(report.dropped(), 0);
    assert_eq!(outbox.count(None).unwrap(), 4);
}

#[test]
fn captures_older_than_the_capture_age_are_shed() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let id = enqueue(&outbox, &scope(), &block(60), shots(500));
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE outbox_entries SET enqueued_at = ?1 WHERE id = ?2",
            rusqlite::params![(Utc::now() - Duration::days(31)).to_rfc3339(), id],
        )
        .unwrap();
    let report = outbox.enforce_limits(&QueueLimits::DEFAULT).unwrap();
    assert_eq!(report.stripped, 1);
    assert_eq!(report.dropped(), 0);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]

    #[test]
    fn limits_hold_after_enforcement_and_captures_go_before_rows(
        sizes in prop::collection::vec(prop::option::of(1usize..4000), 1..30),
        capture_max_rows in 1u32..8,
        capture_max_bytes in 1u64..20_000,
        max_pending_rows in 1u32..25,
    ) {
        let dir = tempdir().unwrap();
        let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
        let limits = QueueLimits {
            capture_max_rows,
            capture_max_bytes,
            capture_max_age_days: 30,
            max_pending_rows,
            max_age_days: 45,
        };
        let total = sizes.len();
        for (i, size) in sizes.iter().enumerate() {
            let captures = size.map(|len| vec![PendingCapture {
                kind: CaptureKind::Screen,
                jpeg: jpeg(len.max(4), i as u8),
                captured_at: Utc::now(),
                blurred: false,
            }]).unwrap_or_default();
            enqueue(&outbox, &scope(), &block(1000 - i as i64 * 11), captures);
        }
        let report = outbox.enforce_limits(&limits).unwrap();

        let rows = outbox.peek_ready(&scope(), 200).unwrap();
        // Rows are dropped only beyond the row cap, oldest first.
        prop_assert_eq!(rows.len(), total.min(max_pending_rows as usize));
        prop_assert_eq!(report.dropped_overflow, total.saturating_sub(max_pending_rows as usize));
        // Captures stay within their row and byte budgets, newest first.
        let kept: Vec<&_> = rows.iter().filter(|r| r.capture_bytes > 0).collect();
        prop_assert!(kept.len() <= capture_max_rows as usize);
        let bytes: u64 = rows.iter().map(|r| r.capture_bytes).sum();
        prop_assert!(bytes <= capture_max_bytes);
        let newest_rank = |id: i64| rows.iter().rev().position(|r| r.id == id).unwrap();
        for row in &kept {
            prop_assert!(newest_rank(row.id) < capture_max_rows as usize);
        }
        // A second pass has nothing left to do.
        let again = outbox.enforce_limits(&limits).unwrap();
        prop_assert_eq!(again.stripped + again.dropped(), 0);
    }
}

// --- Private files and recovery -----------------------------------------------

#[cfg(unix)]
#[test]
fn the_store_directory_and_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let data = dir.path().join("data").join("dosi-tracker");
    let path = data.join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    enqueue(&outbox, &scope(), &block(30), shots(100));
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&data), 0o700);
    assert_eq!(mode(&path), 0o600);
    let wal = data.join("outbox.db-wal");
    assert!(wal.exists());
    assert_eq!(mode(&wal), 0o600);
    assert_eq!(mode(&data.join("outbox.db-shm")), 0o600);
    drop(outbox);

    // A looser directory and file of ours are tightened on the next open.
    fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    Outbox::open(&path, OutboxLimits::default()).unwrap();
    assert_eq!(mode(&data), 0o700);
    assert_eq!(mode(&path), 0o600);

    // A directory others can write to is refused.
    fs::set_permissions(&data, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        Outbox::open(&path, OutboxLimits::default()),
        Err(OutboxError::Io(_))
    ));
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();

    // A symlinked database is refused.
    let link = data.join("link.db");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(matches!(
        Outbox::open(&link, OutboxLimits::default()),
        Err(OutboxError::Io(_))
    ));
}

#[test]
fn a_damaged_store_is_moved_aside_with_its_sidecars() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    fs::write(&path, b"NOT A SQLITE DATABASE AT ALL, JUST BYTES").unwrap();
    fs::write(dir.path().join("outbox.db-wal"), b"wal").unwrap();
    fs::write(dir.path().join("outbox.db-shm"), b"shm").unwrap();

    assert!(matches!(
        Outbox::open(&path, OutboxLimits::default()),
        Err(OutboxError::CorruptDatabase(_))
    ));
    let opened = Outbox::open_or_recover(&path, OutboxLimits::default()).unwrap();
    let aside = opened.moved_aside.clone().expect("moved aside");
    let name = aside.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("outbox.db.damaged-"), "{name}");
    assert_eq!(
        fs::read(&aside).unwrap(),
        b"NOT A SQLITE DATABASE AT ALL, JUST BYTES"
    );
    let suffix = name.trim_start_matches("outbox.db");
    assert_eq!(
        fs::read(dir.path().join(format!("outbox.db-wal{suffix}"))).unwrap(),
        b"wal"
    );
    assert_eq!(
        fs::read(dir.path().join(format!("outbox.db-shm{suffix}"))).unwrap(),
        b"shm"
    );

    // The fresh store works.
    enqueue(&opened.outbox, &scope(), &block(30), vec![]);
    assert_eq!(opened.outbox.pending_count(&scope()).unwrap(), 1);
    drop(opened);

    // A healthy store is opened as is.
    let again = Outbox::open_or_recover(&path, OutboxLimits::default()).unwrap();
    assert!(again.moved_aside.is_none());
    assert_eq!(again.outbox.pending_count(&scope()).unwrap(), 1);
}

#[test]
fn reclaim_space_switches_a_v1_store_to_incremental_vacuum() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    build_v1_store(&path, &[(ACCOUNT, WS, "v1-a")]);
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let mode = |p: &Path| -> i64 {
        Connection::open(p)
            .unwrap()
            .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(mode(&path), 0);
    outbox.reclaim_space().unwrap();
    assert_eq!(mode(&path), 2, "INCREMENTAL");
    assert_eq!(
        outbox.peek_ready(&scope(), 10).unwrap().len(),
        1,
        "data intact"
    );

    let fresh = dir.path().join("fresh.db");
    let new_store = Outbox::open(&fresh, OutboxLimits::default()).unwrap();
    assert_eq!(mode(&fresh), 2, "a new store starts incremental");
    new_store.reclaim_space().unwrap();
}

// --- Read-only inspector -----------------------------------------------------------

#[test]
fn the_inspector_reads_v1_files_without_touching_them() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    build_v1_store(&path, &[(ACCOUNT, WS, "v1-a"), (ACCOUNT, WS, "v1-b")]);
    let before = fs::read(&path).unwrap();
    let report = inspect_outbox_status(&path, &scope()).unwrap();
    assert_eq!(report.durable_queue.queued_count, 2);
    assert_eq!(report.durable_queue.parked_count, 0);
    assert_eq!(report.durable_queue.capture_bytes, 0);
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "byte-identical, no migration"
    );
    assert_eq!(user_version(&path), 1);
    assert!(!columns(&path).contains(&"state".to_string()));
}

#[test]
fn the_inspector_reports_parked_rows_and_capture_bytes_of_v2_files() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&path, OutboxLimits::default()).unwrap();
    let parked = block(50);
    enqueue(&outbox, &scope(), &parked, vec![]);
    enqueue(&outbox, &scope(), &block(40), shots(700));
    enqueue(&outbox, &scope(), &block(30), vec![]);
    outbox
        .park(&scope(), &parked.client_activity_id, "x")
        .unwrap();
    drop(outbox);

    let before = fs::read(&path).unwrap();
    let report = inspect_outbox_status(&path, &scope()).unwrap();
    assert_eq!(report.durable_queue.queued_count, 2, "pending only");
    assert_eq!(report.durable_queue.parked_count, 1);
    assert_eq!(report.durable_queue.capture_bytes, 764);
    assert_eq!(fs::read(&path).unwrap(), before);
    let json = report.to_json().unwrap();
    assert!(json.contains("\"parked_count\": 1") && json.contains("\"capture_bytes\": 764"));
    assert!(report.to_pretty_text().contains("Parked Records:       1"));

    // An unknown row state fails closed.
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE outbox_entries SET state = 7 WHERE id = 1", [])
        .unwrap();
    assert!(inspect_outbox_status(&path, &scope()).is_err());
}

#[test]
fn timestamps_round_trip_through_the_v2_payload() {
    let dir = tempdir().unwrap();
    let outbox = Outbox::open(dir.path().join("outbox.db"), OutboxLimits::default()).unwrap();
    let start = Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap() + Duration::milliseconds(123);
    let mut activity = Activity::new(PROJECT, start, start + Duration::minutes(10));
    activity.platform = Some(PLATFORM.into());
    // Old enough to be refused by the server, but enqueueing never judges age.
    enqueue(&outbox, &scope(), &activity, vec![]);
    let row = outbox.peek_ready(&scope(), 1).unwrap().remove(0);
    assert_eq!(row.envelope.started_at, start);
    let parsed: Activity = serde_json::from_str(row.payload_json.as_deref().unwrap()).unwrap();
    assert_eq!(parsed, activity);
}
