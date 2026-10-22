use std::fs;
use std::process::Command;

use chrono::{Duration, Utc};
use dosi_tracker_linux::outbox::{
    ActivityEnvelope, EnqueueResult, Outbox, OutboxError, OutboxLimits, OutboxScope,
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
    env2.ended_at = env2.ended_at + Duration::seconds(30);

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

    let bin_path = env!("CARGO_BIN_EXE_dosi-tracker-linux");

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
