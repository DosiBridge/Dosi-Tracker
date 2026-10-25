//! Choosing, switching and losing the workspace.

use super::*;

// ---- Choosing the workspace -----------------------------------------------------------

#[test]
fn no_workspace_asks_to_create_one_or_be_invited() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![]) });
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.login_error.as_deref(), Some(NO_WORKSPACE));
    assert!(!snapshot.signing_in && snapshot.workspace_choices.is_empty());
    assert_eq!(snapshot.status, TrackerStatus::SignedOut);
    assert!(h.worker.saved.is_none());
}

#[test]
fn one_workspace_is_used_at_once_and_remembered() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme")]) });
    let used = used_account(&mut h);
    assert_eq!((used.workspace_id.as_deref(), used.workspace_name.as_str()), (Some("w1"), "Acme"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let saved = h.worker.saved.as_ref().unwrap();
    assert_eq!((saved.workspace_id.as_deref(), saved.workspace_name.as_str()), (Some("w1"), "Acme"));
    assert_eq!(h.state.snapshot().workspace, "Acme");
    h.machine.input_at(at("10:04:30"));
    h.tick(at("10:05:00"));
    h.command(at("10:05:00"), Command::Pause { resume_after: None, at: at("10:05:00") });
    assert_eq!(h.worker.store.pending_rows(&h.owner(), 0, 10).unwrap()[0].workspace_id, "w1", "blocks are queued in their workspace");
}

#[test]
fn several_workspaces_are_offered_with_their_roles_and_the_chosen_one_is_used() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    let mut globex = workspace("w2", "Globex");
    globex.role = Some("Owner".into());
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme"), globex]) });
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.workspace_choices.len(), 2);
    assert_eq!(snapshot.workspace_choices[1].role.as_deref(), Some("Owner"));
    assert!(!snapshot.signing_in && snapshot.status == TrackerStatus::SignedOut);
    assert!(h.worker.saved.is_none(), "nothing saved before a workspace is chosen");

    h.command(at("10:00:10"), Command::ChooseWorkspace { index: 1 });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w2"));
    assert!(h.state.snapshot().workspace_choices.is_empty());
    assert_eq!(h.worker.saved.as_ref().unwrap().workspace_id.as_deref(), Some("w2"));
}

#[test]
fn a_named_workspace_is_picked_without_asking() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("globex"));
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")]) });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w2"));

    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, Some("w1"));
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")]) });
    assert_eq!(used_account(&mut h).workspace_id.as_deref(), Some("w1"), "by id too");
}

#[test]
fn going_back_from_the_picker_forgets_the_sign_in() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("a", "A"), workspace("b", "B")]) });
    h.command(at("10:00:05"), Command::CancelWorkspaceChoice);
    assert!(h.worker.pending.is_none() && h.worker.choices.is_empty());
    assert!(h.state.snapshot().workspace_choices.is_empty());
    h.command(at("10:00:06"), Command::ChooseWorkspace { index: 0 });
    assert!(h.control_requests().is_empty());
}

#[test]
fn a_saved_sign_in_of_an_older_build_must_sign_in_again() {
    let mut old = saved(vec![project("p1", 5)], None);
    old.workspace_id = None;
    let h = harness_at(at("10:00:00"), Some(old), false, None);
    assert_eq!(h.status(), TrackerStatus::SignedOut);
}

fn list_workspaces(h: &mut Harness) {
    h.control_requests();
    h.upload_commands();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    assert!(h.state.snapshot().finding_workspaces);
    assert!(matches!(h.control_requests().pop(), Some(ControlRequest::MyWorkspaces)));
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w1", "Acme"), workspace("w2", "Globex")])));
    let snapshot = h.state.snapshot();
    assert!(snapshot.workspace_choices[0].is_current && !snapshot.workspace_choices[1].is_current);
}

#[test]
fn switching_workspace_saves_the_block_and_keeps_the_sign_in() {
    let mut h = tracking();
    let tokens = h.worker.account.as_ref().unwrap().tokens.clone();
    list_workspaces(&mut h);
    h.machine.input_at(at("10:01:59"));
    h.command(at("10:02:01"), Command::SwitchToWorkspace { index: 1, at: at("10:02:00") });
    assert!(h.farewells.try_recv().is_err(), "no sign-out");
    let rows = h.worker.store.pending_rows(&h.owner(), 0, 10).unwrap();
    assert_eq!((rows[0].activity.ended_at, rows[0].workspace_id.as_str()), (at("10:02:00"), "w1"), "queued in the old workspace");

    let requests = h.control_requests();
    assert!(!requests.iter().any(|r| matches!(r, ControlRequest::SignIn(_))), "no new token, no password");
    let used = requests.iter().rev().find_map(|r| match r {
        ControlRequest::UseAccount(Some(account)) => Some(account.clone()),
        _ => None,
    });
    let used = used.expect("the new workspace is used");
    assert_eq!(used.workspace_id.as_deref(), Some("w2"));
    assert!(used.tokens == tokens, "the same token");
    assert!(requests.iter().any(|r| matches!(r, ControlRequest::RefreshProjects)));
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Account(Some(a)) if a.workspace_id.as_deref() == Some("w2"))));

    let saved = h.worker.saved.as_ref().unwrap();
    assert_eq!((saved.workspace_id.as_deref(), saved.workspace_name.as_str(), saved.project_id.as_deref()), (Some("w2"), "Globex", None));
    assert!(saved.projects.is_none());
    let snapshot = h.state.snapshot();
    assert_eq!((snapshot.status, snapshot.workspace.as_str()), (TrackerStatus::Connecting, "Globex"));
    assert!(snapshot.projects.is_empty() && snapshot.workspace_choices.is_empty());

    h.worker.on_control(ControlEvent::Projects(vec![project("p7", 5), project("p8", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::ChooseProject);
}

#[test]
fn switching_with_no_other_workspace_says_so() {
    let mut h = tracking();
    h.command(at("10:01:00"), Command::SwitchWorkspace);
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w1", "Acme")])));
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.workspace_note.as_deref(), Some(ONE_WORKSPACE));
    assert!(!snapshot.finding_workspaces && snapshot.workspace_choices.is_empty());
    assert_eq!(h.status(), TrackerStatus::Tracking, "still signed in");
}

#[test]
fn losing_access_to_the_workspace_stops_tracking_and_asks_for_another() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.machine.set(at("10:04:00"));
    h.control_requests();
    h.worker.on_upload(UploadEvent::WorkspaceLost { owner: h.owner(), workspace_id: "w-other".into(), refusal: crate::api::WorkspaceRefusal::NotMember });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking, "rows of another workspace: not about this one");

    h.worker.on_control(ControlEvent::WorkspaceLost("w1".into(), crate::api::WorkspaceRefusal::NotMember));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::ChooseProject);
    assert!(snapshot.tracking_since.is_none());
    assert_eq!(snapshot.workspace_lost.as_deref(), Some("Acme"));
    assert_eq!(snapshot.workspace_note.as_deref(), Some("You no longer have access to Acme."));
    assert!(snapshot.notices.iter().any(|n| matches!(&n.kind, NoticeKind::WorkspaceLost { name, .. } if name == "Acme")));
    assert!(snapshot.finding_workspaces);
    assert!(h.control_requests().iter().any(|r| matches!(r, ControlRequest::MyWorkspaces)), "the picker is opened");
    // The work is kept, held: never re-sent to a workspace that refuses it.
    let rows = h.worker.store.recent_rows(&h.owner(), 10).unwrap();
    assert_eq!((rows.len(), rows[0].ended_at, rows[0].status), (1, Some(at("10:04:00")), storage::RowStatus::Queued));
    assert_eq!(rows[0].reason.as_deref(), Some("You no longer have access to Acme."));
    assert_eq!(h.worker.store.due_count(&h.owner()).unwrap(), 0);
    assert!(h.heartbeats().is_empty(), "no heartbeat without a workspace");

    // Another workspace is chosen: tracking can start there.
    h.worker.on_control(ControlEvent::MyWorkspaces(Ok(vec![workspace("w2", "Globex")])));
    h.command(at("10:05:00"), Command::SwitchToWorkspace { index: 0, at: at("10:05:00") });
    h.worker.on_control(ControlEvent::Projects(vec![project("p9", 5)]));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!((snapshot.status, snapshot.workspace.as_str(), snapshot.workspace_lost.as_deref()), (TrackerStatus::Tracking, "Globex", None));
    assert_eq!(h.worker.store.due_count(&h.owner()).unwrap(), 0, "Acme's rows stay held while tracking elsewhere");
}
