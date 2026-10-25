use std::fs;
use std::process::Command;

use chrono::{Duration, Utc};
use dosi_tracker_linux::outbox::{
    ActivityEnvelope, EnqueueResult, Outbox, OutboxError, OutboxLimits, OutboxScope,
    OUTBOX_SCHEMA_VERSION,
};
use tempfile::tempdir;

fn synthetic_envelope(client_activity_id: &str, description: Option<&str>) -> ActivityEnvelope {
    ActivityEnvelope {
        format_version: 1,
        client_activity_id: client_activity_id.to_string(),
        project_id: Some("proj-uuid-1".to_string()),
        started_at: Utc::now() - Duration::minutes(10),
        ended_at: Utc::now(),
        productivity: None,
        mouse_clicks: None,
        keyboard_hits: None,
        description: description.map(|s| s.to_string()),
        device_id: Some("dev-uuid-1".to_string()),
        platform: "linux".to_string(),
        agent_version: "0.1.0".to_string(),
        payload_signature: None,
    }
}

#[test]
fn test_outbox_enqueue_reopen_and_peek() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    let env1 = synthetic_envelope("act-1", Some("First block"));
    let env2 = synthetic_envelope("act-2", Some("Second block"));

    {
        let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
        let res1 = outbox.enqueue(&scope, &env1).unwrap();
        let res2 = outbox.enqueue(&scope, &env2).unwrap();

        assert!(matches!(res1, EnqueueResult::Inserted(1)));
        assert!(matches!(res2, EnqueueResult::Inserted(2)));
        assert_eq!(outbox.count(Some(&scope)).unwrap(), 2);
    } // outbox dropped and closed

    // Reopen from disk
    let reopened = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let pending = reopened.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].client_activity_id, "act-1");
    assert_eq!(
        pending[0].envelope.description.as_deref(),
        Some("First block")
    );
    assert_eq!(pending[1].client_activity_id, "act-2");
    assert_eq!(
        pending[1].envelope.description.as_deref(),
        Some("Second block")
    );
}

#[test]
fn test_outbox_same_scope_identical_retry_is_idempotent() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env = synthetic_envelope("act-idempotent", Some("Test block"));

    let res1 = outbox.enqueue(&scope, &env).unwrap();
    assert_eq!(res1, EnqueueResult::Inserted(1));

    let res2 = outbox.enqueue(&scope, &env).unwrap();
    assert_eq!(res2, EnqueueResult::Idempotent(1));

    assert_eq!(outbox.count(Some(&scope)).unwrap(), 1);
}

#[test]
fn test_outbox_same_scope_conflicting_retry_fails_without_clobbering() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env1 = synthetic_envelope("act-conflict", Some("Original payload"));
    outbox.enqueue(&scope, &env1).unwrap();

    let mut env2 = synthetic_envelope("act-conflict", Some("Conflicting payload"));
    env2.ended_at += Duration::seconds(30);

    let err = outbox.enqueue(&scope, &env2).unwrap_err();
    assert!(
        matches!(err, OutboxError::ConflictingIdentity { client_activity_id } if client_activity_id == "act-conflict")
    );

    // Verify original content was NOT clobbered
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].envelope.description.as_deref(),
        Some("Original payload")
    );
}

#[test]
fn test_outbox_different_workspace_same_identity_isolated() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope_a = OutboxScope::new("user-1", "workspace-a").unwrap();
    let scope_b = OutboxScope::new("user-1", "workspace-b").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env_a = synthetic_envelope("shared-act-id", Some("Workspace A task"));
    let env_b = synthetic_envelope("shared-act-id", Some("Workspace B task"));

    let res_a = outbox.enqueue(&scope_a, &env_a).unwrap();
    let res_b = outbox.enqueue(&scope_b, &env_b).unwrap();

    assert!(matches!(res_a, EnqueueResult::Inserted(1)));
    assert!(matches!(res_b, EnqueueResult::Inserted(2)));

    // Scope A view contains only Scope A
    let pending_a = outbox.peek_pending(&scope_a, 10).unwrap();
    assert_eq!(pending_a.len(), 1);
    assert_eq!(
        pending_a[0].envelope.description.as_deref(),
        Some("Workspace A task")
    );

    // Scope B view contains only Scope B
    let pending_b = outbox.peek_pending(&scope_b, 10).unwrap();
    assert_eq!(pending_b.len(), 1);
    assert_eq!(
        pending_b[0].envelope.description.as_deref(),
        Some("Workspace B task")
    );

    assert_eq!(outbox.count(None).unwrap(), 2);
    assert_eq!(outbox.count(Some(&scope_a)).unwrap(), 1);
    assert_eq!(outbox.count(Some(&scope_b)).unwrap(), 1);
}

#[test]
fn test_outbox_different_account_same_identity_isolated() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope_1 = OutboxScope::new("user-alpha", "workspace-common").unwrap();
    let scope_2 = OutboxScope::new("user-beta", "workspace-common").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env1 = synthetic_envelope("act-same-id", Some("Alpha payload"));
    let env2 = synthetic_envelope("act-same-id", Some("Beta payload"));

    outbox.enqueue(&scope_1, &env1).unwrap();
    outbox.enqueue(&scope_2, &env2).unwrap();

    let pending_1 = outbox.peek_pending(&scope_1, 10).unwrap();
    let pending_2 = outbox.peek_pending(&scope_2, 10).unwrap();

    assert_eq!(pending_1.len(), 1);
    assert_eq!(
        pending_1[0].envelope.description.as_deref(),
        Some("Alpha payload")
    );
    assert_eq!(pending_2.len(), 1);
    assert_eq!(
        pending_2[0].envelope.description.as_deref(),
        Some("Beta payload")
    );
}

#[test]
fn test_outbox_acknowledgement_scope_guard_refuses_cross_workspace_removal() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope_target = OutboxScope::new("user-1", "workspace-target").unwrap();
    let scope_foreign = OutboxScope::new("user-1", "workspace-foreign").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env = synthetic_envelope("act-to-ack", Some("Protected block"));
    outbox.enqueue(&scope_target, &env).unwrap();

    // Attempting to ack under foreign scope fails
    let err = outbox
        .ack_delivered(&scope_foreign, "act-to-ack")
        .unwrap_err();
    assert!(
        matches!(err, OutboxError::RecordNotFound { client_activity_id } if client_activity_id == "act-to-ack")
    );

    // Target record remains completely intact
    assert_eq!(outbox.count(Some(&scope_target)).unwrap(), 1);

    // Correct scope ack succeeds
    outbox.ack_delivered(&scope_target, "act-to-ack").unwrap();
    assert_eq!(outbox.count(Some(&scope_target)).unwrap(), 0);
}

#[test]
fn test_outbox_failed_retry_preservation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env = synthetic_envelope("act-retry", Some("Retry block"));
    outbox.enqueue(&scope, &env).unwrap();

    outbox
        .mark_attempt_failed(&scope, "act-retry", "HTTP 503 Backend Busy")
        .unwrap();

    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].retry_count, 1);
    assert_eq!(
        pending[0].last_error.as_deref(),
        Some("HTTP 503 Backend Busy")
    );

    outbox
        .mark_attempt_failed(&scope, "act-retry", "Connection reset by peer")
        .unwrap();

    let pending2 = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending2.len(), 1);
    assert_eq!(pending2[0].retry_count, 2);
    assert_eq!(
        pending2[0].last_error.as_deref(),
        Some("Connection reset by peer")
    );
}

#[test]
fn test_outbox_cardinality_limit_overflow_rejected() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    let limits = OutboxLimits {
        max_records: 3,
        max_payload_bytes: 64 * 1024,
        max_total_payload_bytes: 10 * 1024 * 1024,
        default_page_limit: 10,
        max_page_limit: 50,
    };

    let outbox = Outbox::open(&db_path, limits).unwrap();

    outbox
        .enqueue(&scope, &synthetic_envelope("act-1", None))
        .unwrap();
    outbox
        .enqueue(&scope, &synthetic_envelope("act-2", None))
        .unwrap();
    outbox
        .enqueue(&scope, &synthetic_envelope("act-3", None))
        .unwrap();

    // 4th enqueue must be rejected
    let err = outbox
        .enqueue(&scope, &synthetic_envelope("act-4", None))
        .unwrap_err();

    assert!(matches!(err, OutboxError::QueueFull { count: 3, max: 3 }));
    assert_eq!(outbox.count(None).unwrap(), 3);
}

#[test]
fn test_outbox_payload_size_limit_overflow_rejected() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    let limits = OutboxLimits {
        max_records: 100,
        max_payload_bytes: 150, // very tight limit
        max_total_payload_bytes: 10 * 1024 * 1024,
        default_page_limit: 10,
        max_page_limit: 50,
    };

    let outbox = Outbox::open(&db_path, limits).unwrap();

    let mut env = synthetic_envelope("act-oversize", None);
    env.description = Some("a".repeat(300)); // pushes JSON over 150 bytes

    let err = outbox.enqueue(&scope, &env).unwrap_err();
    assert!(matches!(err, OutboxError::PayloadTooLarge { .. }));
    assert_eq!(outbox.count(None).unwrap(), 0);
}

#[test]
fn test_outbox_deterministic_paging() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    for i in 1..=6 {
        let env = synthetic_envelope(&format!("act-{i}"), Some(&format!("Item {i}")));
        outbox.enqueue(&scope, &env).unwrap();
    }

    let page1 = outbox.peek_pending(&scope, 3).unwrap();
    assert_eq!(page1.len(), 3);
    assert_eq!(page1[0].client_activity_id, "act-1");
    assert_eq!(page1[1].client_activity_id, "act-2");
    assert_eq!(page1[2].client_activity_id, "act-3");

    let page_all = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(page_all.len(), 6);
    assert_eq!(page_all[5].client_activity_id, "act-6");
}

#[test]
fn test_outbox_corrupt_database_fails_closed() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("corrupted.db");

    // Write invalid garbage bytes
    fs::write(&db_path, b"CORRUPT_NOT_A_VALID_SQLITE_DATABASE_HEADER_DATA").unwrap();

    let res = Outbox::open(&db_path, OutboxLimits::default());
    assert!(res.is_err());

    // Assert file was NOT destroyed or overwritten
    assert!(db_path.exists());
    let content = fs::read(&db_path).unwrap();
    assert_eq!(content, b"CORRUPT_NOT_A_VALID_SQLITE_DATABASE_HEADER_DATA");
}

#[test]
fn test_unmeasured_productivity_preserved_truthfully_as_none() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env = ActivityEnvelope {
        format_version: 1,
        client_activity_id: "act-unmeasured".to_string(),
        project_id: None,
        started_at: Utc::now() - Duration::minutes(5),
        ended_at: Utc::now(),
        productivity: None,
        mouse_clicks: None,
        keyboard_hits: None,
        description: None,
        device_id: None,
        platform: "linux".to_string(),
        agent_version: "0.1.0".to_string(),
        payload_signature: None,
    };

    outbox.enqueue(&scope, &env).unwrap();

    let pending = outbox.peek_pending(&scope, 1).unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].envelope.productivity.is_none());
    assert!(pending[0].envelope.mouse_clicks.is_none());
    assert!(pending[0].envelope.keyboard_hits.is_none());
}

#[test]
fn test_measured_productivity_preserved_when_present() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env = ActivityEnvelope {
        format_version: 1,
        client_activity_id: "act-measured".to_string(),
        project_id: Some("proj-uuid-99".to_string()),
        started_at: Utc::now() - Duration::minutes(10),
        ended_at: Utc::now(),
        productivity: Some(88),
        mouse_clicks: Some(150),
        keyboard_hits: Some(600),
        description: Some("Deep focus".to_string()),
        device_id: Some("dev-uuid-99".to_string()),
        platform: "linux".to_string(),
        agent_version: "0.1.0".to_string(),
        payload_signature: None,
    };

    outbox.enqueue(&scope, &env).unwrap();

    let pending = outbox.peek_pending(&scope, 1).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].envelope.productivity, Some(88));
    assert_eq!(pending[0].envelope.mouse_clicks, Some(150));
    assert_eq!(pending[0].envelope.keyboard_hits, Some(600));
    assert_eq!(
        pending[0].envelope.description.as_deref(),
        Some("Deep focus")
    );
}

#[test]
fn test_doctor_and_help_remain_strictly_nonmutating() {
    let temp = tempdir().unwrap();
    let fake_home = temp.path().join("home");
    let fake_xdg_data = temp.path().join("data");
    let fake_xdg_config = temp.path().join("config");

    fs::create_dir_all(&fake_home).unwrap();

    let bin_path = env!("CARGO_BIN_EXE_dosi-tracker");

    // Run doctor command
    let doctor_status = Command::new(bin_path)
        .arg("doctor")
        .env("HOME", &fake_home)
        .env("XDG_DATA_HOME", &fake_xdg_data)
        .env("XDG_CONFIG_HOME", &fake_xdg_config)
        .status()
        .expect("Failed to execute doctor");

    assert!(doctor_status.success());

    // Run --help command
    let help_status = Command::new(bin_path)
        .arg("--help")
        .env("HOME", &fake_home)
        .env("XDG_DATA_HOME", &fake_xdg_data)
        .env("XDG_CONFIG_HOME", &fake_xdg_config)
        .status()
        .expect("Failed to execute --help");

    assert!(help_status.success());

    // Assert neither command created outbox or storage directories
    assert!(
        !fake_xdg_data.exists(),
        "Doctor/Help must not create XDG_DATA_HOME"
    );
    assert!(
        !fake_xdg_config.exists(),
        "Doctor/Help must not create XDG_CONFIG_HOME"
    );
}

#[test]
fn test_invalid_scope_direct_and_constructed_fails_on_all_public_apis() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let valid_env = synthetic_envelope("act-valid", None);

    // Empty account_id in OutboxScope::new
    assert!(matches!(
        OutboxScope::new("", "workspace-1").unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // Direct instantiation with empty account_id
    let invalid_direct_scope = OutboxScope {
        account_id: "".to_string(),
        workspace_id: "workspace-1".to_string(),
    };

    // Verify all public APIs reject invalid scope
    assert!(matches!(
        outbox
            .enqueue(&invalid_direct_scope, &valid_env)
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
    assert!(matches!(
        outbox.peek_pending(&invalid_direct_scope, 10).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
    assert!(matches!(
        outbox
            .ack_delivered(&invalid_direct_scope, "act-valid")
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
    assert!(matches!(
        outbox
            .mark_attempt_failed(&invalid_direct_scope, "act-valid", "error")
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
    assert!(matches!(
        outbox.count(Some(&invalid_direct_scope)).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // Direct instantiation with oversize account_id (> 256 bytes)
    let oversize_scope = OutboxScope {
        account_id: "a".repeat(257),
        workspace_id: "workspace-1".to_string(),
    };
    assert!(matches!(
        outbox.enqueue(&oversize_scope, &valid_env).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // Direct instantiation with control characters
    let control_scope = OutboxScope {
        account_id: "user\x00evil".to_string(),
        workspace_id: "workspace-1".to_string(),
    };
    assert!(matches!(
        outbox.enqueue(&control_scope, &valid_env).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
}

#[test]
fn test_invalid_numeric_limits_fail_closed_without_touching_fs() {
    let dir = tempdir().unwrap();

    let invalid_limits_cases = [
        OutboxLimits {
            max_records: 0,
            ..Default::default()
        },
        OutboxLimits {
            max_payload_bytes: 0,
            ..Default::default()
        },
        OutboxLimits {
            max_total_payload_bytes: 100,
            max_payload_bytes: 200,
            ..Default::default()
        },
        OutboxLimits {
            max_page_limit: 0,
            ..Default::default()
        },
        OutboxLimits {
            default_page_limit: 50,
            max_page_limit: 20,
            ..Default::default()
        },
    ];

    for (idx, limits) in invalid_limits_cases.iter().enumerate() {
        let db_path = dir.path().join(format!("invalid_{idx}.db"));
        let res = Outbox::open(&db_path, limits.clone());
        assert!(
            matches!(res.unwrap_err(), OutboxError::InvalidLimits(_)),
            "Limits case {idx} should be rejected with InvalidLimits"
        );
        assert!(
            !db_path.exists(),
            "No file should be created for invalid limits"
        );
    }
}

#[test]
fn test_oversize_diagnostic_message_rejected_preserving_state() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env = synthetic_envelope("act-diag", Some("Initial payload"));
    outbox.enqueue(&scope, &env).unwrap();

    // Attempting to record diagnostic message > 1024 bytes
    let oversize_error = "E".repeat(1025);
    let err = outbox
        .mark_attempt_failed(&scope, "act-diag", &oversize_error)
        .unwrap_err();

    assert!(matches!(
        err,
        OutboxError::DiagnosticMessageTooLarge {
            size: 1025,
            max: 1024
        }
    ));

    // Verify record state is untouched: retry_count is 0, last_error is None
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].retry_count, 0);
    assert_eq!(pending[0].last_error, None);
    assert_eq!(
        pending[0].envelope.description.as_deref(),
        Some("Initial payload")
    );
}

#[test]
fn test_raw_persisted_corrupt_timestamp_fails_closed_without_fabrication() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    // Create database and initialize schema
    {
        let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
        assert_eq!(outbox.count(Some(&scope)).unwrap(), 0);
    }

    // Insert raw corrupt row directly via SQLite
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let env = synthetic_envelope("act-corrupt-time", Some("Valid envelope"));
        let json = serde_json::to_string(&env).unwrap();

        conn.execute(
            "INSERT INTO outbox_entries (
                account_id, workspace_id, client_activity_id, project_id,
                started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count
            ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, 0)",
            rusqlite::params![
                "user-1",
                "workspace-1",
                "act-corrupt-time",
                "2026-10-22T10:00:00Z",
                "2026-10-22T10:10:00Z",
                &json,
                json.len() as i64,
                "NOT_AN_RFC3339_TIMESTAMP_STRING"
            ],
        )
        .unwrap();
    }

    // Reopen via Outbox
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    // peek_pending must fail closed with CorruptRecord rather than fabricating Utc::now()
    let err = outbox.peek_pending(&scope, 10).unwrap_err();
    assert!(
        matches!(err, OutboxError::CorruptRecord { id, ref reason } if id == 1 && reason.contains("Invalid persisted enqueued_at timestamp")),
        "Expected CorruptRecord error on corrupt enqueued_at timestamp, got {err:?}"
    );

    // Verify row was NOT deleted or mutated
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM outbox_entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "Corrupted row must not be deleted");
}

#[test]
fn test_raw_persisted_envelope_validation_failure_fails_closed() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    {
        let _ = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    }

    // Insert an envelope that is valid JSON but fails envelope.validate() (e.g. started_at > ended_at)
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mut bad_env = synthetic_envelope("act-bad-env", None);
        bad_env.started_at = Utc::now();
        bad_env.ended_at = bad_env.started_at - Duration::minutes(10); // started_at > ended_at!
        let json = serde_json::to_string(&bad_env).unwrap();

        conn.execute(
            "INSERT INTO outbox_entries (
                account_id, workspace_id, client_activity_id, project_id,
                started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count
            ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, 0)",
            rusqlite::params![
                "user-1",
                "workspace-1",
                "act-bad-env",
                "2026-10-22T10:10:00Z",
                "2026-10-22T10:00:00Z",
                &json,
                json.len() as i64,
                "2026-10-22T10:05:00Z"
            ],
        )
        .unwrap();
    }

    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let err = outbox.peek_pending(&scope, 10).unwrap_err();
    assert!(
        matches!(err, OutboxError::CorruptRecord { id, ref reason } if id == 1 && reason.contains("Persisted envelope failed validation")),
        "Expected validation failure on semantically invalid envelope, got {err:?}"
    );
}

#[test]
fn test_raw_persisted_identity_mismatch_fails_closed() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("outbox.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    {
        let _ = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    }

    // Insert row where column client_activity_id != envelope.client_activity_id
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let env = synthetic_envelope("inner-id", None);
        let json = serde_json::to_string(&env).unwrap();

        conn.execute(
            "INSERT INTO outbox_entries (
                account_id, workspace_id, client_activity_id, project_id,
                started_at, ended_at, payload_json, payload_bytes, enqueued_at, retry_count
            ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, 0)",
            rusqlite::params![
                "user-1",
                "workspace-1",
                "outer-row-id-mismatch",
                "2026-10-22T10:00:00Z",
                "2026-10-22T10:10:00Z",
                &json,
                json.len() as i64,
                "2026-10-22T10:05:00Z"
            ],
        )
        .unwrap();
    }

    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let err = outbox.peek_pending(&scope, 10).unwrap_err();
    assert!(
        matches!(err, OutboxError::CorruptRecord { id, ref reason } if id == 1 && reason.contains("does not match envelope identity")),
        "Expected identity mismatch error, got {err:?}"
    );
}

#[test]
fn test_future_store_version_fails_closed_without_downgrade() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("future.db");

    // Manually create SQLite file stamped with future user_version = 3 (this
    // build writes store version 2)
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
    }

    let res = Outbox::open(&db_path, OutboxLimits::default());
    assert!(
        matches!(res.unwrap_err(), OutboxError::UnsupportedStoreVersion(3)),
        "Expected UnsupportedStoreVersion for future store"
    );

    // Verify user_version was NOT modified or downgraded
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ver, 3, "Future version must not be downgraded");
}

#[test]
fn test_current_store_version_persisted_and_reopened_cleanly() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("version1.db");
    let scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    {
        let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
        outbox
            .enqueue(&scope, &synthetic_envelope("act-v1", None))
            .unwrap();
    }

    // Check user_version pragma directly on disk
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        ver, OUTBOX_SCHEMA_VERSION,
        "The current store version (2) must be stamped on database"
    );
    assert_eq!(OUTBOX_SCHEMA_VERSION, 2);

    // Reopen via Outbox
    let reopened = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let pending = reopened.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].client_activity_id, "act-v1");
}

#[test]
fn test_metadata_update_failure_preserves_sentinels() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("sentinels.db");
    let scope_a = OutboxScope::new("user-1", "workspace-a").unwrap();
    let scope_b = OutboxScope::new("user-1", "workspace-b").unwrap();
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let env_a = synthetic_envelope("act-a", Some("Sentinel A"));
    let env_b = synthetic_envelope("act-b", Some("Sentinel B"));

    outbox.enqueue(&scope_a, &env_a).unwrap();
    outbox.enqueue(&scope_b, &env_b).unwrap();

    // Attempting to record failure for Scope A's record under Scope B fails
    let err = outbox
        .mark_attempt_failed(&scope_b, "act-a", "failed")
        .unwrap_err();
    assert!(matches!(err, OutboxError::RecordNotFound { .. }));

    // Both sentinels must remain completely intact
    let pending_a = outbox.peek_pending(&scope_a, 10).unwrap();
    assert_eq!(pending_a.len(), 1);
    assert_eq!(pending_a[0].retry_count, 0);
    assert_eq!(pending_a[0].last_error, None);
    assert_eq!(
        pending_a[0].envelope.description.as_deref(),
        Some("Sentinel A")
    );

    let pending_b = outbox.peek_pending(&scope_b, 10).unwrap();
    assert_eq!(pending_b.len(), 1);
    assert_eq!(pending_b[0].retry_count, 0);
    assert_eq!(pending_b[0].last_error, None);
    assert_eq!(
        pending_b[0].envelope.description.as_deref(),
        Some("Sentinel B")
    );
}

#[test]
fn test_directly_constructed_and_deserialized_scopes_with_oversized_ascii_and_unicode_whitespace_rejected(
) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("scope_whitespace.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let valid_env = synthetic_envelope("act-valid", None);

    // 1. Direct construction with huge leading ASCII whitespace (> 256 bytes raw)
    let direct_oversized_ascii = OutboxScope {
        account_id: format!("{}user-1", " ".repeat(300)),
        workspace_id: "ws-1".to_string(),
    };
    assert!(matches!(
        outbox
            .enqueue(&direct_oversized_ascii, &valid_env)
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
    assert!(matches!(
        outbox
            .peek_pending(&direct_oversized_ascii, 10)
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));
    assert!(matches!(
        outbox.count(Some(&direct_oversized_ascii)).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // 2. Direct construction with huge trailing Unicode whitespace (\u{3000} ideographic space, 100 chars = 300 bytes)
    let direct_oversized_unicode = OutboxScope {
        account_id: "user-1".to_string(),
        workspace_id: format!("ws-1{}", "\u{3000}".repeat(100)),
    };
    assert!(matches!(
        outbox
            .enqueue(&direct_oversized_unicode, &valid_env)
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // 3. Deserialized from JSON with huge ASCII whitespace
    let json_ascii = format!(
        r#"{{"account_id": "{}user-1", "workspace_id": "ws-1"}}"#,
        " ".repeat(300)
    );
    let deserialized_ascii: OutboxScope = serde_json::from_str(&json_ascii).unwrap();
    assert!(matches!(
        outbox.enqueue(&deserialized_ascii, &valid_env).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // 4. Deserialized from JSON with huge Unicode whitespace (\u{2003} em space)
    let json_unicode = format!(
        r#"{{"account_id": "user-1", "workspace_id": "ws-1{}"}}"#,
        "\u{2003}".repeat(100)
    );
    let deserialized_unicode: OutboxScope = serde_json::from_str(&json_unicode).unwrap();
    assert!(matches!(
        outbox
            .enqueue(&deserialized_unicode, &valid_env)
            .unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // Assert zero mutations occurred in database
    assert_eq!(outbox.count(None).unwrap(), 0);
}

#[test]
fn test_oversized_padded_activity_ids_rejected_before_mutation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("padded_activity_ids.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let valid_scope = OutboxScope::new("user-1", "workspace-1").unwrap();

    // 1. Direct envelope with huge leading ASCII whitespace in client_activity_id (raw len > 256)
    let direct_padded_ascii = synthetic_envelope(&format!("{}act-1", " ".repeat(300)), None);
    let err_ascii = outbox
        .enqueue(&valid_scope, &direct_padded_ascii)
        .unwrap_err();
    assert!(matches!(err_ascii, OutboxError::InvalidEnvelope(_)));

    // 2. Direct envelope with huge Unicode whitespace padding (\u{3000}, raw len > 256)
    let direct_padded_unicode =
        synthetic_envelope(&format!("act-1{}", "\u{3000}".repeat(100)), None);
    let err_unicode = outbox
        .enqueue(&valid_scope, &direct_padded_unicode)
        .unwrap_err();
    assert!(matches!(err_unicode, OutboxError::InvalidEnvelope(_)));

    // 3. Deserialized envelope with padded client_activity_id
    let mut bad_json_envelope = synthetic_envelope("placeholder", None);
    bad_json_envelope.client_activity_id = format!("{}act-1", " ".repeat(300));
    let json_str = serde_json::to_string(&bad_json_envelope).unwrap();
    let deserialized_env: ActivityEnvelope = serde_json::from_str(&json_str).unwrap();
    let err_deserialized = outbox.enqueue(&valid_scope, &deserialized_env).unwrap_err();
    assert!(matches!(err_deserialized, OutboxError::InvalidEnvelope(_)));

    // Assert zero records in outbox (rejected before mutation)
    assert_eq!(outbox.count(None).unwrap(), 0);
}

#[test]
fn test_exact_byte_boundaries_and_multibyte_identities() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("exact_boundaries.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    // Exactly 256 ASCII bytes for account_id, workspace_id, client_activity_id
    let exact_256_acc = "a".repeat(256);
    let exact_256_ws = "w".repeat(256);
    let exact_256_act = "c".repeat(256);

    let scope_256 = OutboxScope::new(&exact_256_acc, &exact_256_ws).unwrap();
    let env_256 = synthetic_envelope(&exact_256_act, Some("Exact 256 boundary"));

    let res = outbox.enqueue(&scope_256, &env_256).unwrap();
    assert!(matches!(res, EnqueueResult::Inserted(1)));
    assert_eq!(outbox.count(Some(&scope_256)).unwrap(), 1);

    // 257 ASCII bytes for client_activity_id: must be rejected
    let oversize_257_act = "c".repeat(257);
    let env_257 = synthetic_envelope(&oversize_257_act, None);
    assert!(matches!(
        outbox.enqueue(&scope_256, &env_257).unwrap_err(),
        OutboxError::InvalidEnvelope(_)
    ));

    // Multibyte characters: Bengali "বাংলা" is 15 bytes in UTF-8 (5 chars * 3 bytes)
    // 17 repeats * 15 bytes = 255 bytes + 1 ASCII byte = 256 bytes exact!
    let bengali_part = "বাংলা".repeat(17); // 255 bytes
    let exact_256_multibyte = format!("{bengali_part}X"); // 256 bytes exact
    assert_eq!(exact_256_multibyte.len(), 256);

    let env_mb_256 = synthetic_envelope(&exact_256_multibyte, Some("Bengali 256"));
    let res_mb = outbox.enqueue(&scope_256, &env_mb_256).unwrap();
    assert!(matches!(res_mb, EnqueueResult::Inserted(2)));

    // Multibyte 258 bytes: 86 Japanese chars (each 3 bytes) = 258 bytes: must be rejected
    let jp_258 = "あ".repeat(86);
    assert_eq!(jp_258.len(), 258);
    let env_mb_258 = synthetic_envelope(&jp_258, None);
    assert!(matches!(
        outbox.enqueue(&scope_256, &env_mb_258).unwrap_err(),
        OutboxError::InvalidEnvelope(_)
    ));

    // Verify stored count remains 2
    assert_eq!(outbox.count(Some(&scope_256)).unwrap(), 2);
}

#[test]
fn test_rejection_preserves_unchanged_persisted_sentinels_a_and_b() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("sentinels_preserve.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope_a = OutboxScope::new("user-a", "ws-a").unwrap();
    let scope_b = OutboxScope::new("user-b", "ws-b").unwrap();

    let env_a = synthetic_envelope("act-sentinel-a", Some("Sentinel A Body"));
    let env_b = synthetic_envelope("act-sentinel-b", Some("Sentinel B Body"));

    outbox.enqueue(&scope_a, &env_a).unwrap();
    outbox.enqueue(&scope_b, &env_b).unwrap();
    assert_eq!(outbox.count(None).unwrap(), 2);

    // Attempt invalid operations with oversized padded identities
    let invalid_padded_scope = OutboxScope {
        account_id: format!("{}user-a", " ".repeat(300)),
        workspace_id: "ws-a".to_string(),
    };
    let invalid_padded_env = synthetic_envelope(&format!("{}act-new", " ".repeat(300)), None);

    // Enqueue with invalid scope rejected
    assert!(matches!(
        outbox.enqueue(&invalid_padded_scope, &env_a).unwrap_err(),
        OutboxError::InvalidScope(_)
    ));

    // Enqueue with invalid padded env rejected
    assert!(matches!(
        outbox.enqueue(&scope_a, &invalid_padded_env).unwrap_err(),
        OutboxError::InvalidEnvelope(_)
    ));

    // Mark failed with invalid padded activity ID rejected
    let padded_id_str = format!("{}act-sentinel-a", " ".repeat(300));
    assert!(matches!(
        outbox
            .mark_attempt_failed(&scope_a, &padded_id_str, "err")
            .unwrap_err(),
        OutboxError::InvalidEnvelope(_)
    ));

    // Ack with invalid padded activity ID rejected
    assert!(matches!(
        outbox.ack_delivered(&scope_a, &padded_id_str).unwrap_err(),
        OutboxError::InvalidEnvelope(_)
    ));

    // Assert Sentinels A and B are 100% unchanged
    let pending_a = outbox.peek_pending(&scope_a, 10).unwrap();
    assert_eq!(pending_a.len(), 1);
    assert_eq!(pending_a[0].client_activity_id, "act-sentinel-a");
    assert_eq!(pending_a[0].retry_count, 0);
    assert_eq!(pending_a[0].last_error, None);
    assert_eq!(
        pending_a[0].envelope.description.as_deref(),
        Some("Sentinel A Body")
    );

    let pending_b = outbox.peek_pending(&scope_b, 10).unwrap();
    assert_eq!(pending_b.len(), 1);
    assert_eq!(pending_b[0].client_activity_id, "act-sentinel-b");
    assert_eq!(pending_b[0].retry_count, 0);
    assert_eq!(pending_b[0].last_error, None);
    assert_eq!(
        pending_b[0].envelope.description.as_deref(),
        Some("Sentinel B Body")
    );
}

#[test]
fn test_valid_enqueue_peek_retry_ack_lifecycle_with_bounded_identity() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("full_lifecycle.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

    let scope = OutboxScope::new("acct-lifecycle", "ws-lifecycle").unwrap();
    let env = synthetic_envelope("act-lifecycle-1", Some("Lifecycle Payload"));

    // 1. Enqueue
    let insert_res = outbox.enqueue(&scope, &env).unwrap();
    assert!(matches!(insert_res, EnqueueResult::Inserted(1)));

    // 2. Peek pending
    let pending = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].client_activity_id, "act-lifecycle-1");
    assert_eq!(pending[0].retry_count, 0);
    assert_eq!(pending[0].last_error, None);

    // 3. Mark attempt failed (retry)
    outbox
        .mark_attempt_failed(&scope, "act-lifecycle-1", "Temporary network failure")
        .unwrap();

    // 4. Peek pending verifies retry count and error message
    let pending_after_retry = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending_after_retry.len(), 1);
    assert_eq!(pending_after_retry[0].retry_count, 1);
    assert_eq!(
        pending_after_retry[0].last_error.as_deref(),
        Some("Temporary network failure")
    );

    // 5. Ack delivered
    outbox.ack_delivered(&scope, "act-lifecycle-1").unwrap();

    // 6. Peek pending is now empty
    let pending_final = outbox.peek_pending(&scope, 10).unwrap();
    assert_eq!(pending_final.len(), 0);
    assert_eq!(outbox.count(Some(&scope)).unwrap(), 0);
}

#[test]
fn test_outbox_lock_poisoning_returns_recoverable_error() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("poison_test.db");
    let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();
    let scope = OutboxScope::new("acc-poison", "ws-poison").unwrap();
    let env = synthetic_envelope("act-poison", Some("Testing lock poisoning recovery"));

    // Verify healthy first
    outbox.enqueue(&scope, &env).unwrap();

    // Poison the internal mutex
    outbox.poison_lock_for_test();

    // Verify all outbox methods return Err(OutboxError::LockPoisoned) without panicking
    let err_enqueue = outbox.enqueue(&scope, &env).unwrap_err();
    assert!(matches!(err_enqueue, OutboxError::LockPoisoned));

    let err_peek = outbox.peek_pending(&scope, 10).unwrap_err();
    assert!(matches!(err_peek, OutboxError::LockPoisoned));

    let err_ack = outbox.ack_delivered(&scope, "act-poison").unwrap_err();
    assert!(matches!(err_ack, OutboxError::LockPoisoned));

    let err_fail = outbox
        .mark_attempt_failed(&scope, "act-poison", "err")
        .unwrap_err();
    assert!(matches!(err_fail, OutboxError::LockPoisoned));

    let err_count = outbox.count(Some(&scope)).unwrap_err();
    assert!(matches!(err_count, OutboxError::LockPoisoned));
}
