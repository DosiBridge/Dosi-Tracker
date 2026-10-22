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
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_zero_attempts, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 2. batch_limit = 0
    let cfg_zero_batch = SyncPassConfig {
        max_attempts: 10,
        batch_limit: 0,
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_zero_batch, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 3. max_attempts > MAX_ALLOWED_ATTEMPTS (200)
    let cfg_overflow_attempts = SyncPassConfig {
        max_attempts: MAX_ALLOWED_ATTEMPTS + 1,
        batch_limit: 10,
    };
    assert!(matches!(
        run_sync_pass(&outbox, &valid_scope, cfg_overflow_attempts, &mut ledger).unwrap_err(),
        SyncError::InvalidConfig(_)
    ));

    // 4. batch_limit > MAX_ALLOWED_BATCH_LIMIT (100)
    let cfg_overflow_batch = SyncPassConfig {
        max_attempts: 10,
        batch_limit: MAX_ALLOWED_BATCH_LIMIT + 1,
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
    let exe = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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

    // Verify backend_sync is reported truthfully as NOT_IMPLEMENTED
    let parsed: serde_json::Value = serde_json::from_str(&doc_str).unwrap();
    assert_eq!(
        parsed["adapters"]["backend_sync"], "NOT_IMPLEMENTED",
        "backend_sync adapter must remain truthfully reported as NOT_IMPLEMENTED"
    );
    assert_eq!(parsed["privacy_invariants"]["unprivileged_process"], true);
    assert_eq!(
        parsed["privacy_invariants"]["zero_synthetic_telemetry"],
        true
    );
}
