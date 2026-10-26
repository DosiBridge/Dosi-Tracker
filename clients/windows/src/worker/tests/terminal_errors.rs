//! Terminal server errors: a removed computer, a lost or archived
//! workspace.

use super::*;

// ---- Terminal server errors ------------------------------------------------------------

#[test]
fn a_revoked_device_stops_everything_and_forgets_the_sign_in() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    credentials::save_at(&file, &saved(vec![project("p1", 5)], None)).unwrap();
    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), false, Some(file.clone()));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    h.machine.input_at(at("10:04:00"));
    h.machine.set(at("10:04:00"));
    h.control_requests();
    h.upload_commands();

    h.worker.on_control(ControlEvent::DeviceRevoked("This device was removed.".into()));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::SignedOut);
    assert_eq!(snapshot.login_error.as_deref(), Some(crate::state::DEVICE_REVOKED_TEXT));
    assert!(snapshot.notices.iter().any(|n| n.kind == NoticeKind::DeviceRevoked));
    assert!(!h.machine.is_input_running() && !h.machine.is_usage_running() && !h.machine.is_tracking_active(), "capture stopped");
    assert!(credentials::load_from(&file).is_none(), "the saved sign-in is forgotten");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Account(None))));
    assert!(h.control_requests().iter().any(|r| matches!(r, ControlRequest::UseAccount(None))));
    // The work so far is kept, never sent from here meanwhile.
    let owner = storage::owner_key("https://localhost:6006", "alice@acme.com");
    let spans: Vec<_> = h.worker.store.pending_batch(&owner, 0, 10).unwrap().into_iter().map(|(_, a)| (a.started_at, a.ended_at, a.end_reason)).collect();
    assert_eq!(spans, [(at("10:00:00"), at("10:04:00"), Some(EndReason::SignOut))]);
    h.tick(at("10:06:00"));
    assert!(h.heartbeats().is_empty(), "no heartbeats");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn a_revoked_device_reported_by_the_uploader_of_this_account_stops_too() {
    let mut h = tracking();
    let other = storage::owner_key("https://localhost:6006", "bob@acme.com");
    h.worker.on_upload(UploadEvent::DeviceRevoked { owner: other, reason: "x".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking, "another account's: ignored");
    h.worker.on_upload(UploadEvent::DeviceRevoked { owner: h.owner(), reason: "x".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignedOut);
}

#[test]
fn an_archived_workspace_stops_tracking_there_and_says_so() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.machine.set(at("10:04:00"));
    h.control_requests();
    h.worker.on_control(ControlEvent::WorkspaceLost("w1".into(), crate::api::WorkspaceRefusal::Archived));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert!(snapshot.tracking_since.is_none());
    assert_eq!(snapshot.workspace_note.as_deref(), Some("Acme was archived; nothing more can be tracked there."));
    assert!(snapshot
        .notices
        .iter()
        .any(|n| matches!(&n.kind, NoticeKind::WorkspaceLost { refusal: crate::api::WorkspaceRefusal::Archived, .. })));
    assert_eq!(h.worker.store.due_count(&h.owner()).unwrap(), 0, "its rows are held");
    assert!(h.heartbeats().is_empty());
}

#[test]
fn archived_workspaces_are_never_offered() {
    let mut archived = workspace("w0", "Old");
    archived.is_archived = true;
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![archived.clone(), workspace("w1", "Acme")]) });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w1"), "the only usable one, at once");

    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("Old"));
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![archived.clone()]) });
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.login_error.as_deref(), Some(ALL_ARCHIVED));
    assert!(h.worker.saved.is_none() && snapshot.workspace_choices.is_empty());

    // Switching: not listed either.
    let mut h = tracking();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w1", "Acme"), archived])));
    assert_eq!(h.state.snapshot().workspace_note.as_deref(), Some(ONE_WORKSPACE));
}

#[test]
fn a_workspace_used_again_sends_its_held_rows_again() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.machine.set(at("10:04:00"));
    h.worker.on_control(ControlEvent::WorkspaceLost("w1".into(), crate::api::WorkspaceRefusal::NotMember));
    assert_eq!(h.worker.store.due_count(&h.owner()).unwrap(), 0);
    // Access restored: picked again.
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w1", "Acme")])));
    h.command(at("10:05:00"), Command::SwitchToWorkspace { index: 0, at: at("10:05:00") });
    assert_eq!(h.worker.store.due_count(&h.owner()).unwrap(), 1, "released for one more try");
}
