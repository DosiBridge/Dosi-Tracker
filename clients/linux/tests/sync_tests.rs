//! Integration tests for the scoped offline sync-pass coordinator.
//!
//! Verifies:
//! - Successful bounded drain and persistence across reopen
//! - Tenant/scope isolation (same ID under different scopes)
//! - Identical payload byte identity and keys across retry
//! - Finite attempt limits and leftover handling
//! - Non-accepted row preservation (transient, auth-required, definitive rejection)
//! - Mismatched acknowledgement refusal and halt
//! - Injected SQLite deletion failure reports acceptance-but-persistence-pending
//! - Corrupt store read halts without invoking transport
//! - Invalid configuration bounds refuse transport and mutation
//! - Doctor and help commands remain non-mutating and offline

use std::collections::HashMap;
use std::process::Command;

use chrono::Utc;
use dosi_tracker_linux::outbox::{
    ActivityEnvelope, Outbox, OutboxEntry, OutboxError, OutboxLimits, OutboxScope,
};
use dosi_tracker_linux::sync::{
    run_sync_pass, DeliveryOutcome, DeliveryTransport, SyncError, SyncPassConfig,
    SyncPassCoordinator, SyncStopReason, MAX_ALLOWED_ATTEMPTS, MAX_ALLOWED_BATCH_LIMIT,
    MAX_RECEIVED_ID_BYTES, MAX_REPORT_DIAGNOSTIC_BYTES,
};
use tempfile::tempdir;

fn synthetic_envelope(client_activity_id: &str, description: Option<&str>) -> ActivityEnvelope {
    let now = Utc::now();
    ActivityEnvelope {
        format_version: 1,
        client_activity_id: client_activity_id.to_string(),
        project_id: None,
        started_at: now - chrono::Duration::seconds(60),
        ended_at: now,
        mouse_clicks: Some(10),
        keyboard_hits: Some(25),
        description: description.map(|s| s.to_string()),
        device_id: None,
        platform: "linux".to_string(),
        agent_version: "0.1.0".to_string(),
        productivity: Some(85),
        payload_signature: None,
    }
}

#[derive(Debug, Default)]
struct FakeDeliveryLedger {
    delivered_calls: Vec<OutboxEntry>,
    handlers: HashMap<String, DeliveryOutcome>,
    default_outcome: Option<DeliveryOutcome>,
}

impl FakeDeliveryLedger {
    fn new() -> Self {
        Self::default()
    }

    fn on_id(&mut self, id: &str, outcome: DeliveryOutcome) {
        self.handlers.insert(id.to_string(), outcome);
    }
}

impl DeliveryTransport for FakeDeliveryLedger {
    fn deliver(&mut self, entry: &OutboxEntry) -> DeliveryOutcome {
        self.delivered_calls.push(entry.clone());
        if let Some(outcome) = self.handlers.get(&entry.client_activity_id) {
            outcome.clone()
        } else if let Some(ref def) = self.default_outcome {
            def.clone()
        } else {
            DeliveryOutcome::Accepted {
                client_activity_id: entry.client_activity_id.clone(),
            }
        }
    }
}

#[test]
fn test_successful_bounded_drain_and_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("drain.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-1", "ws-1").unwrap();

    let env1 = synthetic_envelope("act-1", Some("First"));
    let env2 = synthetic_envelope("act-2", Some("Second"));
    let env3 = synthetic_envelope("act-3", Some("Third"));

    outbox.enqueue(&scope, &env1).unwrap();
    outbox.enqueue(&scope, &env2).unwrap();
    outbox.enqueue(&scope, &env3).unwrap();
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 3);

    let mut ledger = FakeDeliveryLedger::new();
    let config = SyncPassConfig::default();

    let coordinator = SyncPassCoordinator::new(&outbox, config);
    let report = coordinator.run_pass(&scope, &mut ledger).unwrap();

    assert_eq!(report.attempted_count, 3);
    assert_eq!(
        report.transport_accepted,
        vec![
            "act-1".to_string(),
            "act-2".to_string(),
            "act-3".to_string()
        ]
    );
    assert_eq!(
        report.persisted_acknowledged,
        vec![
            "act-1".to_string(),
            "act-2".to_string(),
            "act-3".to_string()
        ]
    );
    assert!(report.drained);
    assert_eq!(report.stop_reason, None);

    // Verify queue is completely empty
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 0);

    // Verify reopen from disk sees 0 rows
    let reopened = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    assert_eq!(reopened.count(Some(&scope)).unwrap(), 0);
    assert!(reopened.peek_pending(&scope, 10).unwrap().is_empty());
}

#[test]
fn test_same_activity_id_in_different_scopes_stays_isolated() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("scope_isolation.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope_a = OutboxScope::new("user-a", "ws-a").unwrap();
    let scope_b = OutboxScope::new("user-b", "ws-b").unwrap();

    let env_a = synthetic_envelope("act-shared", Some("Payload A"));
    let env_b = synthetic_envelope("act-shared", Some("Payload B"));

    outbox.enqueue(&scope_a, &env_a).unwrap();
    outbox.enqueue(&scope_b, &env_b).unwrap();
    assert_eq!(outbox.count(None).unwrap(), 2);

    let mut ledger = FakeDeliveryLedger::new();
    let config = SyncPassConfig::default();

    // Execute sync pass strictly for Scope A
    let report = run_sync_pass(&outbox, &scope_a, config, &mut ledger).unwrap();

    assert_eq!(report.attempted_count, 1);
    assert_eq!(
        report.persisted_acknowledged,
        vec!["act-shared".to_string()]
    );
    assert!(report.drained);

    // Scope A is empty
    assert_eq!(outbox.count(Some(&scope_a)).unwrap(), 0);

    // Scope B is 100% intact and was never touched by transport
    assert_eq!(outbox.count(Some(&scope_b)).unwrap(), 1);
    let pending_b = outbox.peek_pending(&scope_b, 10).unwrap();
    assert_eq!(pending_b.len(), 1);
    assert_eq!(pending_b[0].client_activity_id, "act-shared");
    assert_eq!(pending_b[0].retry_count, 0);
    assert_eq!(pending_b[0].last_error, None);
    assert_eq!(
        pending_b[0].envelope.description.as_deref(),
        Some("Payload B")
    );

    // Ledger only delivered Scope A
    assert_eq!(ledger.delivered_calls.len(), 1);
    assert_eq!(ledger.delivered_calls[0].scope, scope_a);
}

#[test]
fn test_identical_payload_and_key_preserved_across_retry_and_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("retry_keys.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-retry", "ws-retry").unwrap();

    let env = synthetic_envelope("act-durable", Some("Complex payload with metrics"));
    outbox.enqueue(&scope, &env).unwrap();

    // Pass 1: Transient failure
    let mut ledger1 = FakeDeliveryLedger::new();
    ledger1.on_id(
        "act-durable",
        DeliveryOutcome::TransientFailure {
            reason: "HTTP 503 Service Unavailable".to_string(),
        },
    );

    let report1 = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger1).unwrap();
    assert_eq!(report1.attempted_count, 1);
    assert!(report1.transport_accepted.is_empty());
    assert!(report1.persisted_acknowledged.is_empty());
    assert_eq!(
        report1.stop_reason,
        Some(SyncStopReason::TransientFailure {
            client_activity_id: "act-durable".to_string(),
            reason: "HTTP 503 Service Unavailable".to_string(),
        })
    );

    // Verify row still in outbox with updated retry metadata and unmodified payload
    let pending1 = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending1.len(), 1);
    assert_eq!(pending1[0].retry_count, 1);
    assert_eq!(
        pending1[0].last_error.as_deref(),
        Some("HTTP 503 Service Unavailable")
    );
    assert_eq!(
        pending1[0].envelope.description.as_deref(),
        Some("Complex payload with metrics")
    );
    assert_eq!(pending1[0].envelope.productivity, Some(85));
    assert_eq!(pending1[0].envelope.mouse_clicks, Some(10));
    assert_eq!(pending1[0].envelope.keyboard_hits, Some(25));

    // Reopen database from disk and verify persistence of all keys and payload bytes
    drop(outbox);
    let reopened = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let pending_reopened = reopened.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending_reopened.len(), 1);
    assert_eq!(pending_reopened[0].retry_count, 1);
    assert_eq!(
        pending_reopened[0].envelope.description.as_deref(),
        Some("Complex payload with metrics")
    );

    // Pass 2: Retry succeeds
    let mut ledger2 = FakeDeliveryLedger::new();
    let report2 =
        run_sync_pass(&reopened, &scope, SyncPassConfig::default(), &mut ledger2).unwrap();
    assert_eq!(report2.attempted_count, 1);
    assert_eq!(
        report2.persisted_acknowledged,
        vec!["act-durable".to_string()]
    );
    assert!(report2.drained);
    assert_eq!(reopened.count(Some(&scope)).unwrap(), 0);
}

#[test]
fn test_max_attempts_and_leftovers_handled_cleanly() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("leftovers.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-cap", "ws-cap").unwrap();

    for i in 1..=7 {
        let env = synthetic_envelope(&format!("act-{i:02}"), Some(&format!("Item {i}")));
        outbox.enqueue(&scope, &env).unwrap();
    }
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 7);

    let mut ledger = FakeDeliveryLedger::new();
    let config = SyncPassConfig::new(3, 2).unwrap(); // max_attempts = 3, batch_limit = 2

    let report = run_sync_pass(&outbox, &scope, config, &mut ledger).unwrap();

    assert_eq!(report.attempted_count, 3);
    assert_eq!(report.transport_accepted.len(), 3);
    assert_eq!(report.persisted_acknowledged.len(), 3);
    assert!(!report.drained);
    assert_eq!(
        report.stop_reason,
        Some(SyncStopReason::MaxAttemptsReached { limit: 3 })
    );

    // 4 leftover entries remain intact in FIFO order
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 4);
    let leftovers = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(leftovers.len(), 4);
    assert_eq!(leftovers[0].client_activity_id, "act-04");
    assert_eq!(leftovers[1].client_activity_id, "act-05");
    assert_eq!(leftovers[2].client_activity_id, "act-06");
    assert_eq!(leftovers[3].client_activity_id, "act-07");

    // Next pass with ceiling 4 drains all leftovers
    let mut ledger2 = FakeDeliveryLedger::new();
    let config2 = SyncPassConfig::new(4, 2).unwrap();
    let report2 = run_sync_pass(&outbox, &scope, config2, &mut ledger2).unwrap();
    assert_eq!(report2.attempted_count, 4);
    assert!(report2.drained);
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 0);
}

#[test]
fn test_transient_auth_and_rejected_outcomes_preserve_unaccepted_rows() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outcomes.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-outcomes", "ws-outcomes").unwrap();

    // 1. Transient failure halts and preserves subsequent rows
    let env1 = synthetic_envelope("act-t-1", None);
    let env2 = synthetic_envelope("act-t-2", None);
    outbox.enqueue(&scope, &env1).unwrap();
    outbox.enqueue(&scope, &env2).unwrap();

    let mut ledger_transient = FakeDeliveryLedger::new();
    ledger_transient.on_id(
        "act-t-1",
        DeliveryOutcome::TransientFailure {
            reason: "Connection timeout".to_string(),
        },
    );

    let rep1 = run_sync_pass(
        &outbox,
        &scope,
        SyncPassConfig::default(),
        &mut ledger_transient,
    )
    .unwrap();
    assert_eq!(rep1.attempted_count, 1);
    assert_eq!(
        rep1.stop_reason,
        Some(SyncStopReason::TransientFailure {
            client_activity_id: "act-t-1".to_string(),
            reason: "Connection timeout".to_string(),
        })
    );
    // Both items remain in outbox; item 2 was never attempted
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 2);
    assert_eq!(ledger_transient.delivered_calls.len(), 1);

    // 2. Auth required halts pass without deleting or claiming token refresh
    outbox.ack_delivered(&scope, "act-t-1").unwrap();
    outbox.ack_delivered(&scope, "act-t-2").unwrap();

    let env_auth1 = synthetic_envelope("act-auth-1", None);
    let env_auth2 = synthetic_envelope("act-auth-2", None);
    outbox.enqueue(&scope, &env_auth1).unwrap();
    outbox.enqueue(&scope, &env_auth2).unwrap();

    let mut ledger_auth = FakeDeliveryLedger::new();
    ledger_auth.on_id(
        "act-auth-1",
        DeliveryOutcome::AuthRequired {
            reason: "Bearer token signature expired".to_string(),
        },
    );

    let rep2 = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger_auth).unwrap();
    assert_eq!(rep2.attempted_count, 1);
    assert_eq!(
        rep2.stop_reason,
        Some(SyncStopReason::AuthRequired {
            client_activity_id: "act-auth-1".to_string(),
            reason: "Bearer token signature expired".to_string(),
        })
    );
    // Both items remain in outbox; item 1 retry_count not incremented
    let auth_pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(auth_pending.len(), 2);
    assert_eq!(auth_pending[0].retry_count, 0);

    // 3. Definitive rejection halts pass and is visibly reported without silent dropping
    outbox.ack_delivered(&scope, "act-auth-1").unwrap();
    outbox.ack_delivered(&scope, "act-auth-2").unwrap();

    let env_rej1 = synthetic_envelope("act-rej-1", None);
    let env_rej2 = synthetic_envelope("act-rej-2", None);
    outbox.enqueue(&scope, &env_rej1).unwrap();
    outbox.enqueue(&scope, &env_rej2).unwrap();

    let mut ledger_rej = FakeDeliveryLedger::new();
    ledger_rej.on_id(
        "act-rej-1",
        DeliveryOutcome::Rejected {
            reason: "422 Unprocessable: timeline interval overlapping".to_string(),
        },
    );

    let rep3 = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger_rej).unwrap();
    assert_eq!(rep3.attempted_count, 1);
    assert_eq!(
        rep3.stop_reason,
        Some(SyncStopReason::DefinitiveRejection {
            client_activity_id: "act-rej-1".to_string(),
            reason: "422 Unprocessable: timeline interval overlapping".to_string(),
        })
    );
    assert_eq!(rep3.definitive_rejections.len(), 1);
    assert_eq!(
        rep3.definitive_rejections[0].client_activity_id,
        "act-rej-1"
    );
    // Both items remain stored; zero silent dropping
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 2);
}

#[test]
fn test_mismatched_acknowledgement_preserves_queue_and_halts() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("mismatched_ack.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-mismatch", "ws-mismatch").unwrap();

    let env = synthetic_envelope("act-expected", Some("Sensitive Entry"));
    outbox.enqueue(&scope, &env).unwrap();

    // Transport maliciously or erroneously returns another ID
    let mut ledger = FakeDeliveryLedger::new();
    ledger.on_id(
        "act-expected",
        DeliveryOutcome::Accepted {
            client_activity_id: "act-unrelated".to_string(),
        },
    );

    let report = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();

    assert_eq!(report.attempted_count, 1);
    assert!(report.transport_accepted.is_empty());
    assert!(report.persisted_acknowledged.is_empty());
    assert_eq!(
        report.stop_reason,
        Some(SyncStopReason::MismatchedAcknowledgement {
            expected: "act-expected".to_string(),
            received: "act-unrelated".to_string(),
        })
    );

    // Entry is NOT removed from Outbox
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending[0].client_activity_id, "act-expected");
}

#[test]
fn test_injected_store_failure_reports_acceptance_but_persistence_pending() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("store_failure.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-fail", "ws-fail").unwrap();

    let env = synthetic_envelope("act-db-error", Some("Will fail delete"));
    outbox.enqueue(&scope, &env).unwrap();

    // Inject a SQLite failure on DELETE using a native trigger
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TRIGGER fail_delete BEFORE DELETE ON outbox_entries
             BEGIN
                 SELECT RAISE(FAIL, 'simulated disk write failure on delete');
             END;",
            [],
        )
        .unwrap();
    }

    let mut ledger = FakeDeliveryLedger::new();
    let report = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();

    assert_eq!(report.attempted_count, 1);
    // Transport accepted the delivery
    assert_eq!(report.transport_accepted, vec!["act-db-error".to_string()]);
    // BUT local acknowledgement persistence failed
    assert!(report.persisted_acknowledged.is_empty());
    assert!(matches!(
        report.stop_reason,
        Some(SyncStopReason::AcknowledgementPersistenceFailed {
            ref client_activity_id,
            ref error,
        }) if client_activity_id == "act-db-error" && error.contains("simulated disk write failure")
    ));

    // Entry is STILL in Outbox (never claimed deleted if DB removal failed)
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
}

#[test]
fn test_corrupt_read_does_not_invoke_transport_or_reset_queue() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("corrupt_read.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-corrupt", "ws-corrupt").unwrap();

    let env = synthetic_envelope("act-valid", None);
    outbox.enqueue(&scope, &env).unwrap();

    // Corrupt the persisted payload_json in SQLite
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE outbox_entries SET payload_json = '{\"corrupted\": true}' WHERE client_activity_id = 'act-valid'",
            [],
        )
        .unwrap();
    }

    let mut ledger = FakeDeliveryLedger::new();
    let res = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger);

    // Fails closed with Outbox error
    assert!(matches!(
        res,
        Err(SyncError::Outbox(OutboxError::CorruptRecord { .. }))
    ));

    // Transport was NEVER invoked
    assert_eq!(ledger.delivered_calls.len(), 0);

    // Queue was NOT deleted or reset
    let raw_conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = raw_conn
        .query_row("SELECT COUNT(*) FROM outbox_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_invalid_limits_and_scope_refuse_transport_and_mutation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("invalid_config.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let valid_scope = OutboxScope::new("user-1", "ws-1").unwrap();
    let valid_env = synthetic_envelope("act-1", None);
    outbox.enqueue(&valid_scope, &valid_env).unwrap();

    let mut ledger = FakeDeliveryLedger::new();

    // 1. max_attempts = 0
    let cfg_zero_attempts = SyncPassConfig {
        max_attempts: 0,
        batch_limit: 10,
        deadline: None,
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_zero_attempts, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 2. batch_limit = 0
    let cfg_zero_batch = SyncPassConfig {
        max_attempts: 10,
        batch_limit: 0,
        deadline: None,
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_zero_batch, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 3. max_attempts > MAX_ALLOWED_ATTEMPTS (200)
    let cfg_overflow_attempts = SyncPassConfig {
        max_attempts: MAX_ALLOWED_ATTEMPTS + 1,
        batch_limit: 10,
        deadline: None,
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_overflow_attempts, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 4. batch_limit > MAX_ALLOWED_BATCH_LIMIT (100)
    let cfg_overflow_batch = SyncPassConfig {
        max_attempts: 10,
        batch_limit: MAX_ALLOWED_BATCH_LIMIT + 1,
        deadline: None,
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_overflow_batch, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 5. Invalid scope (empty account)
    let invalid_scope = OutboxScope {
        account_id: "".to_string(),
        workspace_id: "ws-1".to_string(),
    };
    assert!(matches!(
        run_sync_pass(
            &outbox,
            &invalid_scope,
            SyncPassConfig::default(),
            &mut ledger
        )
        .unwrap_err(),
        SyncError::InvalidScope(_)
    ));

    // Zero transport invocations occurred
    assert_eq!(ledger.delivered_calls.len(), 0);
    // Zero database mutations occurred
    assert_eq!(outbox.count(None).unwrap(), 1);
}

#[test]
fn test_doctor_and_help_remain_strictly_nonmutating_and_offline() {
    let exe = env!("CARGO_BIN_EXE_dosi-tracker");

    // 1. Help invocation
    let help_out = Command::new(exe).arg("--help").output().unwrap();
    assert!(help_out.status.success());
    let help_str = String::from_utf8_lossy(&help_out.stdout);
    assert!(help_str.contains("USAGE:"));
    assert!(help_str.contains("doctor"));

    // 2. Doctor JSON invocation
    let doc_out = Command::new(exe)
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(doc_out.status.success());
    let doc_str = String::from_utf8_lossy(&doc_out.stdout);

    // backend_sync is reported truthfully: the daemon's uploader exists now
    // (the foundation said NOT_IMPLEMENTED before it did).
    let parsed: serde_json::Value = serde_json::from_str(&doc_str).unwrap();
    assert_eq!(
        parsed["adapters"]["backend_sync"], "SUPPORTED",
        "backend_sync adapter must be truthfully reported as SUPPORTED"
    );
    assert_eq!(parsed["adapters"]["webcam_capture"], "NOT_IMPLEMENTED");
    assert_eq!(parsed["privacy_invariants"]["unprivileged_process"], true);
    assert_eq!(
        parsed["privacy_invariants"]["zero_synthetic_telemetry"],
        true
    );
}

#[test]
fn test_retry_metadata_persistence_failure_reports_accurately_and_halts() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("retry_meta_fail.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-target", "ws-target").unwrap();
    let foreign_scope = OutboxScope::new("acct-foreign", "ws-foreign").unwrap();

    // Enqueue 2 entries in target scope:
    // First will experience transient failure, second should NOT be attempted
    let env1 = synthetic_envelope("act-transient-fail", Some("First envelope"));
    let env2 = synthetic_envelope("act-second", Some("Second envelope"));
    outbox.enqueue(&scope, &env1).unwrap();
    outbox.enqueue(&scope, &env2).unwrap();

    // Enqueue 1 sentinel in foreign scope: must remain 100% untouched
    let env_foreign = synthetic_envelope("act-foreign-sentinel", Some("Foreign sentinel"));
    outbox.enqueue(&foreign_scope, &env_foreign).unwrap();

    // Create a native SQLite trigger blocking UPDATE on retry_count
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TRIGGER fail_retry_update BEFORE UPDATE OF retry_count ON outbox_entries
             BEGIN
                 SELECT RAISE(FAIL, 'simulated disk write failure on retry update');
             END;",
            [],
        )
        .unwrap();
    }

    let mut ledger = FakeDeliveryLedger::new();
    ledger.on_id(
        "act-transient-fail",
        DeliveryOutcome::TransientFailure {
            reason: "simulated upstream 503".to_string(),
        },
    );
    ledger.on_id(
        "act-second",
        DeliveryOutcome::Accepted {
            client_activity_id: "act-second".to_string(),
        },
    );

    let report = run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();

    // 1. Pass attempted exactly 1 item and halted immediately
    assert_eq!(report.attempted_count, 1);
    assert_eq!(ledger.delivered_calls.len(), 1);
    assert_eq!(
        ledger.delivered_calls[0].client_activity_id,
        "act-transient-fail"
    );

    // 2. Report does NOT claim retry metadata was recorded when persistence failed
    assert!(
        report.transient_failures.is_empty(),
        "transient_failures must NOT claim persistence when SQLite UPDATE failed"
    );

    // 3. Exact failure reporting: RetryMetadataPersistenceFailed with accurate components
    assert!(matches!(
        report.stop_reason,
        Some(SyncStopReason::RetryMetadataPersistenceFailed {
            ref client_activity_id,
            ref transport_reason,
            ref error,
        }) if client_activity_id == "act-transient-fail"
            && transport_reason == "simulated upstream 503"
            && error.contains("simulated disk write failure on retry update")
    ));

    // 4. In SQLite: row is preserved, retry_count remains 0, last_error remains None
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let (retry_count, last_error): (i64, Option<String>) = conn
            .query_row(
                "SELECT retry_count, last_error FROM outbox_entries WHERE client_activity_id = 'act-transient-fail'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(retry_count, 0, "retry_count must NOT be incremented in DB");
        assert_eq!(last_error, None, "last_error must NOT be updated in DB");

        // Second entry is intact and unattempted
        let (retry_count2, last_error2): (i64, Option<String>) = conn
            .query_row(
                "SELECT retry_count, last_error FROM outbox_entries WHERE client_activity_id = 'act-second'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(retry_count2, 0);
        assert_eq!(last_error2, None);

        // Foreign sentinel is 100% untouched
        let (foreign_retry, foreign_err): (i64, Option<String>) = conn
            .query_row(
                "SELECT retry_count, last_error FROM outbox_entries WHERE client_activity_id = 'act-foreign-sentinel'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(foreign_retry, 0);
        assert_eq!(foreign_err, None);
    }

    // 5. Positive control: drop trigger, verify normal retry metadata persistence succeeds
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("DROP TRIGGER fail_retry_update", []).unwrap();
    }

    let mut positive_ledger = FakeDeliveryLedger::new();
    positive_ledger.on_id(
        "act-transient-fail",
        DeliveryOutcome::TransientFailure {
            reason: "simulated upstream 503".to_string(),
        },
    );

    let positive_report = run_sync_pass(
        &outbox,
        &scope,
        SyncPassConfig::default(),
        &mut positive_ledger,
    )
    .unwrap();

    assert_eq!(positive_report.attempted_count, 1);
    assert_eq!(positive_report.transient_failures.len(), 1);
    assert_eq!(
        positive_report.transient_failures[0].client_activity_id,
        "act-transient-fail"
    );
    assert_eq!(
        positive_report.transient_failures[0].reason,
        "simulated upstream 503"
    );
    assert!(matches!(
        positive_report.stop_reason,
        Some(SyncStopReason::TransientFailure {
            ref client_activity_id,
            ref reason,
        }) if client_activity_id == "act-transient-fail" && reason == "simulated upstream 503"
    ));

    // In SQLite: retry metadata was actually committed
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let (retry_count, last_error): (i64, Option<String>) = conn
            .query_row(
                "SELECT retry_count, last_error FROM outbox_entries WHERE client_activity_id = 'act-transient-fail'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(retry_count, 1);
        assert_eq!(last_error, Some("simulated upstream 503".to_string()));
    }
}

#[test]
fn test_oversized_ascii_and_multibyte_diagnostics_truncated_across_all_variants() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("oversized_diag.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-diag", "ws-diag").unwrap();

    // 1. TransientFailure with oversized ASCII (3,000 bytes) and multibyte Bengali
    {
        let env = synthetic_envelope("act-transient-ascii", Some("Transient ASCII"));
        outbox.enqueue(&scope, &env).unwrap();

        let mut ledger = FakeDeliveryLedger::new();
        let huge_ascii = "E".repeat(3000);
        ledger.on_id(
            "act-transient-ascii",
            DeliveryOutcome::TransientFailure { reason: huge_ascii },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        assert_eq!(report.transient_failures.len(), 1);
        assert_eq!(
            report.transient_failures[0].reason.len(),
            MAX_REPORT_DIAGNOSTIC_BYTES
        );
        match report.stop_reason {
            Some(SyncStopReason::TransientFailure { ref reason, .. }) => {
                assert_eq!(reason.len(), MAX_REPORT_DIAGNOSTIC_BYTES);
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }

        // Clean up row for next test
        outbox.ack_delivered(&scope, "act-transient-ascii").unwrap();
    }

    // Multibyte Bengali transient failure: "ত্রুটি-" repeated (16 bytes each)
    {
        let env = synthetic_envelope("act-transient-bengali", Some("Transient Bengali"));
        outbox.enqueue(&scope, &env).unwrap();

        let mut ledger = FakeDeliveryLedger::new();
        let huge_bengali = "ত্রুটি-সার্ভার-ত্রুটি-".repeat(150); // ~3300 bytes
        ledger.on_id(
            "act-transient-bengali",
            DeliveryOutcome::TransientFailure {
                reason: huge_bengali,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        assert_eq!(report.transient_failures.len(), 1);
        let reason = &report.transient_failures[0].reason;
        assert!(reason.len() <= MAX_REPORT_DIAGNOSTIC_BYTES);
        assert!(std::str::from_utf8(reason.as_bytes()).is_ok());

        // Envelope payload preserved in DB
        let peek = outbox.peek_pending(&scope, 1).unwrap();
        assert_eq!(peek[0].envelope.client_activity_id, "act-transient-bengali");
        assert_eq!(
            peek[0].envelope.description.as_deref(),
            Some("Transient Bengali")
        );

        outbox
            .ack_delivered(&scope, "act-transient-bengali")
            .unwrap();
    }

    // 2. AuthRequired with oversized ASCII and multibyte
    {
        let env = synthetic_envelope("act-auth-oversize", Some("Auth oversize"));
        outbox.enqueue(&scope, &env).unwrap();

        let mut ledger = FakeDeliveryLedger::new();
        let huge_auth = "A".repeat(4000);
        ledger.on_id(
            "act-auth-oversize",
            DeliveryOutcome::AuthRequired { reason: huge_auth },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        match report.stop_reason {
            Some(SyncStopReason::AuthRequired {
                ref client_activity_id,
                ref reason,
            }) => {
                assert_eq!(client_activity_id, "act-auth-oversize");
                assert_eq!(reason.len(), MAX_REPORT_DIAGNOSTIC_BYTES);
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }

        // Queue entry is preserved
        assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
        outbox.ack_delivered(&scope, "act-auth-oversize").unwrap();
    }

    // 3. Rejected with oversized ASCII and multibyte
    {
        let env = synthetic_envelope("act-rejected-oversize", Some("Rejected oversize"));
        outbox.enqueue(&scope, &env).unwrap();

        let mut ledger = FakeDeliveryLedger::new();
        let huge_rejected = "অস্বীকৃত-ডেটা-".repeat(200); // multibyte Bengali
        ledger.on_id(
            "act-rejected-oversize",
            DeliveryOutcome::Rejected {
                reason: huge_rejected,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        assert_eq!(report.definitive_rejections.len(), 1);
        assert!(report.definitive_rejections[0].reason.len() <= MAX_REPORT_DIAGNOSTIC_BYTES);
        match report.stop_reason {
            Some(SyncStopReason::DefinitiveRejection {
                ref client_activity_id,
                ref reason,
            }) => {
                assert_eq!(client_activity_id, "act-rejected-oversize");
                assert!(reason.len() <= MAX_REPORT_DIAGNOSTIC_BYTES);
                assert!(std::str::from_utf8(reason.as_bytes()).is_ok());
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }

        // Queue entry is preserved
        assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
        outbox
            .ack_delivered(&scope, "act-rejected-oversize")
            .unwrap();
    }

    // 4. MismatchedAcknowledgement with oversized rogue ID (ASCII and multibyte)
    {
        let env = synthetic_envelope("act-orig-id", Some("Original valid identity"));
        outbox.enqueue(&scope, &env).unwrap();

        let mut ledger = FakeDeliveryLedger::new();
        // Rogue ID starts with the original ID, but has 1000 extra bytes appended.
        // Verifies exact matching: must NOT falsely match, and received must be bounded!
        let rogue_id = format!("act-orig-id-{}", "R".repeat(1000));
        ledger.on_id(
            "act-orig-id",
            DeliveryOutcome::Accepted {
                client_activity_id: rogue_id,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        match report.stop_reason {
            Some(SyncStopReason::MismatchedAcknowledgement {
                ref expected,
                ref received,
            }) => {
                // Expected must be the full valid queue ID, NOT truncated
                assert_eq!(expected, "act-orig-id");
                // Received must be bounded to MAX_RECEIVED_ID_BYTES
                assert_eq!(received.len(), MAX_RECEIVED_ID_BYTES);
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }

        // Queue entry is preserved
        assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
        outbox.ack_delivered(&scope, "act-orig-id").unwrap();
    }

    // Multibyte rogue ID in MismatchedAcknowledgement
    {
        let env = synthetic_envelope("act-orig-2", None);
        outbox.enqueue(&scope, &env).unwrap();

        let mut ledger = FakeDeliveryLedger::new();
        let rogue_multibyte = format!("রগ-আইডি-{}", "ভুল-".repeat(100));
        ledger.on_id(
            "act-orig-2",
            DeliveryOutcome::Accepted {
                client_activity_id: rogue_multibyte,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        match report.stop_reason {
            Some(SyncStopReason::MismatchedAcknowledgement {
                ref expected,
                ref received,
            }) => {
                assert_eq!(expected, "act-orig-2");
                assert!(received.len() <= MAX_RECEIVED_ID_BYTES);
                assert!(std::str::from_utf8(received.as_bytes()).is_ok());
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }

        assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
    }
}

#[test]
fn test_exact_diagnostic_and_received_id_byte_boundaries() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("boundary_check.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acct-bounds", "ws-bounds").unwrap();

    // 1. Diagnostic boundary: exactly 1024 bytes -> kept untruncated
    {
        let env = synthetic_envelope("act-diag-1024", None);
        outbox.enqueue(&scope, &env).unwrap();

        let exact_1024 = "D".repeat(1024);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-diag-1024",
            DeliveryOutcome::TransientFailure {
                reason: exact_1024.clone(),
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        assert_eq!(report.transient_failures[0].reason.len(), 1024);
        assert_eq!(report.transient_failures[0].reason, exact_1024);
        outbox.ack_delivered(&scope, "act-diag-1024").unwrap();
    }

    // 2. Diagnostic boundary: exactly 1025 bytes -> truncated to 1024 bytes
    {
        let env = synthetic_envelope("act-diag-1025", None);
        outbox.enqueue(&scope, &env).unwrap();

        let exact_1025 = "D".repeat(1025);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-diag-1025",
            DeliveryOutcome::TransientFailure { reason: exact_1025 },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        assert_eq!(report.transient_failures[0].reason.len(), 1024);
        outbox.ack_delivered(&scope, "act-diag-1025").unwrap();
    }

    // 3. Diagnostic boundary: multibyte character crossing 1024 bytes
    // 1023 ASCII bytes + 3-byte Bengali 'অ' (0xE0 0xA6 0x85) = 1026 bytes.
    // Truncation must stop at byte 1023 without splitting the 3-byte character.
    {
        let env = synthetic_envelope("act-diag-multi-boundary", None);
        outbox.enqueue(&scope, &env).unwrap();

        let mut boundary_string = "M".repeat(1023);
        boundary_string.push('অ');
        assert_eq!(boundary_string.len(), 1026);

        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-diag-multi-boundary",
            DeliveryOutcome::TransientFailure {
                reason: boundary_string,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        assert_eq!(report.transient_failures[0].reason.len(), 1023);
        assert!(std::str::from_utf8(report.transient_failures[0].reason.as_bytes()).is_ok());
        outbox
            .ack_delivered(&scope, "act-diag-multi-boundary")
            .unwrap();
    }

    // 4. Received ID boundary: exactly 256 bytes -> kept at 256
    {
        let env = synthetic_envelope("act-rcv-256", None);
        outbox.enqueue(&scope, &env).unwrap();

        let exact_256 = "I".repeat(256);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-rcv-256",
            DeliveryOutcome::Accepted {
                client_activity_id: exact_256.clone(),
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        match report.stop_reason {
            Some(SyncStopReason::MismatchedAcknowledgement { ref received, .. }) => {
                assert_eq!(received.len(), 256);
                assert_eq!(received, &exact_256);
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }
        outbox.ack_delivered(&scope, "act-rcv-256").unwrap();
    }

    // 5. Received ID boundary: exactly 257 bytes -> truncated to 256
    {
        let env = synthetic_envelope("act-rcv-257", None);
        outbox.enqueue(&scope, &env).unwrap();

        let exact_257 = "I".repeat(257);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-rcv-257",
            DeliveryOutcome::Accepted {
                client_activity_id: exact_257,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        match report.stop_reason {
            Some(SyncStopReason::MismatchedAcknowledgement { ref received, .. }) => {
                assert_eq!(received.len(), 256);
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }
        outbox.ack_delivered(&scope, "act-rcv-257").unwrap();
    }

    // 6. Received ID boundary: multibyte character crossing 256 bytes
    // 255 ASCII bytes + 3-byte Bengali 'অ' = 258 bytes.
    // Truncation must stop at byte 255 without splitting the character.
    {
        let env = synthetic_envelope("act-rcv-multi", None);
        outbox.enqueue(&scope, &env).unwrap();

        let mut boundary_id = "I".repeat(255);
        boundary_id.push('অ');
        assert_eq!(boundary_id.len(), 258);

        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-rcv-multi",
            DeliveryOutcome::Accepted {
                client_activity_id: boundary_id,
            },
        );

        let report =
            run_sync_pass(&outbox, &scope, SyncPassConfig::default(), &mut ledger).unwrap();
        match report.stop_reason {
            Some(SyncStopReason::MismatchedAcknowledgement { ref received, .. }) => {
                assert_eq!(received.len(), 255);
                assert!(std::str::from_utf8(received.as_bytes()).is_ok());
            }
            ref other => panic!("Unexpected stop reason: {:?}", other),
        }
    }
}

/// Park-and-continue and the other outcomes of the server contract (design 8.4).
mod contract_outcomes {
    use super::*;
    use std::time::{Duration, Instant};

    use dosi_tracker_linux::agent::model::Activity;
    use dosi_tracker_linux::outbox::{CaptureKind, PendingActivity, PendingCapture};
    use dosi_tracker_linux::sync::{RetryKind, ScopeBlock};

    fn setup(ids: &[&str]) -> (tempfile::TempDir, Outbox, OutboxScope) {
        let dir = tempdir().unwrap();
        let outbox = Outbox::open(dir.path().join("outcomes.db"), OutboxLimits::default()).unwrap();
        let scope = OutboxScope::new("acct-1", "ws-1").unwrap();
        for id in ids {
            outbox
                .enqueue(&scope, &synthetic_envelope(id, Some(id)))
                .unwrap();
        }
        (dir, outbox, scope)
    }

    #[test]
    fn a_parked_row_does_not_block_the_rows_after_it() {
        let (_dir, outbox, scope) = setup(&["act-1", "act-2", "act-3"]);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-1",
            DeliveryOutcome::Park {
                reason: "Overlaps another block.".into(),
            },
        );
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert_eq!(report.parked.len(), 1);
        assert_eq!(report.parked[0].client_activity_id, "act-1");
        assert_eq!(report.parked[0].reason, "Overlaps another block.");
        assert_eq!(
            report.persisted_acknowledged,
            vec!["act-2".to_string(), "act-3".to_string()]
        );
        assert!(report.drained);
        assert_eq!(report.stop_reason, None);
        assert_eq!(report.attempted_count, 3);
        // The parked row stays in the store, out of the queue.
        assert_eq!(outbox.pending_count(&scope).unwrap(), 0);
        assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);

        // A second pass never offers it again.
        let mut second = FakeDeliveryLedger::new();
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut second)
            .unwrap();
        assert!(second.delivered_calls.is_empty());
        assert!(report.drained);
    }

    #[test]
    fn retry_later_records_the_attempt_and_stops() {
        let (_dir, outbox, scope) = setup(&["act-1", "act-2"]);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-1",
            DeliveryOutcome::RetryLater {
                reason: "HTTP 429".into(),
                kind: RetryKind::Transient,
                retry_after: Some(Duration::from_secs(90)),
            },
        );
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert_eq!(
            report.stop_reason,
            Some(SyncStopReason::RetryLater {
                client_activity_id: "act-1".into(),
                reason: "HTTP 429".into(),
                kind: RetryKind::Transient,
                retry_after: Some(Duration::from_secs(90)),
            })
        );
        assert_eq!(report.transient_failures.len(), 1);
        assert_eq!(ledger.delivered_calls.len(), 1, "the pass stopped");
        let pending = outbox.peek_pending(&scope, 10).unwrap();
        assert_eq!(pending[0].retry_count, 1);
        assert_eq!(pending[0].last_error.as_deref(), Some("HTTP 429"));
        assert!(!report.drained);
    }

    #[test]
    fn terminal_answers_block_the_scope() {
        for (outcome, block, parked) in [
            (
                DeliveryOutcome::WorkspaceAccessLost {
                    reason: "Not a workspace member".into(),
                },
                ScopeBlock::WorkspaceAccessLost,
                0,
            ),
            (
                DeliveryOutcome::DeviceRevoked {
                    reason: "Device revoked".into(),
                },
                ScopeBlock::DeviceRevoked,
                1,
            ),
        ] {
            let (_dir, outbox, scope) = setup(&["act-1", "act-2"]);
            let mut ledger = FakeDeliveryLedger::new();
            ledger.on_id("act-1", outcome);
            let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
                .run_pass(&scope, &mut ledger)
                .unwrap();
            match report.stop_reason {
                Some(SyncStopReason::ScopeBlocked {
                    ref client_activity_id,
                    block: got,
                    ..
                }) => {
                    assert_eq!(client_activity_id, "act-1");
                    assert_eq!(got, block);
                }
                ref other => panic!("expected ScopeBlocked, got {other:?}"),
            }
            assert_eq!(report.parked.len(), parked);
            assert_eq!(ledger.delivered_calls.len(), 1);
            assert_eq!(outbox.pending_count(&scope).unwrap(), 2 - parked as u32);
        }
    }

    #[derive(Default)]
    struct CaptureLedger {
        seen_capture_bytes: Vec<usize>,
        answers: Vec<DeliveryOutcome>,
    }

    impl DeliveryTransport for CaptureLedger {
        fn deliver(&mut self, entry: &OutboxEntry) -> DeliveryOutcome {
            self.seen_capture_bytes
                .push(entry.captures.iter().map(|c| c.jpeg.len()).sum());
            if self.answers.is_empty() {
                DeliveryOutcome::Accepted {
                    client_activity_id: entry.client_activity_id.clone(),
                }
            } else {
                self.answers.remove(0)
            }
        }
    }

    fn v2_row(outbox: &Outbox, scope: &OutboxScope, with_captures: bool) -> String {
        let now = Utc::now();
        let activity = Activity::new(
            "3fa85f64-5717-4562-b3fc-2c963f66afa6",
            now - chrono::Duration::minutes(10),
            now,
        );
        let captures = if with_captures {
            vec![PendingCapture {
                kind: CaptureKind::Screen,
                jpeg: vec![0xFF, 0xD8, 1, 2, 3, 4],
                captured_at: now,
                blurred: false,
            }]
        } else {
            vec![]
        };
        let pending = PendingActivity::from_activity(&activity, captures).unwrap();
        outbox
            .enqueue_activity(scope, &pending, &[1u8; 32])
            .unwrap();
        activity.client_activity_id
    }

    #[test]
    fn a_strip_redelivers_once_without_captures() {
        let (_dir, outbox, scope) = setup(&[]);
        let id = v2_row(&outbox, &scope, true);
        let mut ledger = CaptureLedger {
            answers: vec![DeliveryOutcome::StripCaptures {
                reason: "HTTP 413".into(),
            }],
            ..Default::default()
        };
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert_eq!(
            ledger.seen_capture_bytes,
            vec![6, 0],
            "loaded, then stripped"
        );
        assert_eq!(report.stripped, vec![id.clone()]);
        assert_eq!(report.persisted_acknowledged, vec![id]);
        assert_eq!(report.attempted_count, 1, "one row, two deliveries");
    }

    #[test]
    fn a_second_strip_or_nothing_to_strip_parks() {
        let (_dir, outbox, scope) = setup(&[]);
        let id = v2_row(&outbox, &scope, true);
        let strip = || DeliveryOutcome::StripCaptures {
            reason: "HTTP 413".into(),
        };
        let mut ledger = CaptureLedger {
            answers: vec![strip(), strip()],
            ..Default::default()
        };
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert_eq!(report.stripped, vec![id.clone()]);
        assert_eq!(report.parked[0].client_activity_id, id);
        assert!(report.drained);

        // A row without captures cannot get any smaller: parked at once.
        let (_dir, outbox, scope) = setup(&["act-v1"]);
        let mut ledger = CaptureLedger {
            answers: vec![strip()],
            ..Default::default()
        };
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert!(report.stripped.is_empty());
        assert_eq!(report.parked[0].client_activity_id, "act-v1");
        assert_eq!(ledger.seen_capture_bytes.len(), 1);
    }

    #[test]
    fn a_passed_deadline_starts_no_new_row() {
        let (_dir, outbox, scope) = setup(&["act-1"]);
        let mut ledger = FakeDeliveryLedger::new();
        let config = SyncPassConfig::default().with_deadline(Some(Instant::now()));
        let report = SyncPassCoordinator::new(&outbox, config)
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert_eq!(report.stop_reason, Some(SyncStopReason::DeadlineReached));
        assert!(ledger.delivered_calls.is_empty());
        assert_eq!(report.attempted_count, 0);

        let future =
            SyncPassConfig::default().with_deadline(Some(Instant::now() + Duration::from_secs(60)));
        let report = SyncPassCoordinator::new(&outbox, future)
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert!(report.drained);
    }

    #[test]
    fn existing_outcomes_keep_their_semantics_next_to_the_new_ones() {
        // Rejected still halts and retains (the transport's scope guard uses it).
        let (_dir, outbox, scope) = setup(&["act-1", "act-2"]);
        let mut ledger = FakeDeliveryLedger::new();
        ledger.on_id(
            "act-1",
            DeliveryOutcome::Rejected {
                reason: "Scope mismatch".into(),
            },
        );
        let report = SyncPassCoordinator::new(&outbox, SyncPassConfig::default())
            .run_pass(&scope, &mut ledger)
            .unwrap();
        assert!(matches!(
            report.stop_reason,
            Some(SyncStopReason::DefinitiveRejection { .. })
        ));
        assert!(report.parked.is_empty());
        assert_eq!(outbox.pending_count(&scope).unwrap(), 2);
    }
}
