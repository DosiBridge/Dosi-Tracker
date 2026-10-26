//! What a block records: the project, its policy, the task note, "Sync
//! now" and the screenshot moment.

use super::*;

#[test]
fn switching_project_ends_the_block_at_the_click() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))), false, None);
    h.machine.input_at(at("10:05:00"));
    h.command(at("10:06:01"), Command::SelectProject { project_id: "p2".into(), at: at("10:06:00") });
    let queued = h.queued();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Project))]);
    assert_eq!(queued[0].project_id, "p1", "the work so far belongs to the project it was done on");
    assert_eq!(h.state.snapshot().selected_project.as_deref(), Some("p2"));
    assert_eq!(h.tracking_since(), Some(at("10:06:00")));
}

#[test]
fn a_project_that_disappears_stops_tracking_or_moves_to_the_only_one_left() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))), false, None);
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:06:00"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p2", 5)]));
    h.worker.publish();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Project))]);
    assert_eq!(h.state.snapshot().selected_project.as_deref(), Some("p2"), "exactly one left");
    assert!(h.state.snapshot().notices.iter().any(|n| matches!(&n.kind,
        NoticeKind::ProjectUnavailable { title, now_tracking: Some(next) } if title == "Project p1" && next == "Project p2")));

    // Gone again, with two others left: choose.
    h.machine.set(at("10:20:00"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p3", 5), project("p4", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::ChooseProject);
    assert!(h.tracking_since().is_none());
}

#[test]
fn a_refreshed_policy_applies_from_the_next_block() {
    let mut h = tracking();
    let mut shorter = project("p1", 5);
    shorter.interval_minutes = 5;
    h.machine.input_at(at("10:01:00"));
    h.worker.on_control(ControlEvent::Projects(vec![shorter]));
    h.worker.publish();
    assert_eq!(h.state.snapshot().interval_minutes, 5);
    for minute in 2..=10 {
        h.machine.input_at(at(&format!("10:{minute:02}:00")));
        h.tick(at(&format!("10:{minute:02}:00")));
    }
    // The block in progress kept its ten minutes.
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:10:00"), Some(EndReason::Interval))]);
    assert_eq!(h.state.snapshot().next_capture_at, Some(at("10:15:00")));
}

#[test]
fn a_new_task_note_ends_the_block_or_relabels_a_young_one() {
    let mut h = tracking();
    h.machine.input_at(at("10:00:30"));
    h.command(at("10:00:30"), Command::SetTaskNote { note: "Invoices".into(), at: at("10:00:30") });
    assert!(h.queued().is_empty(), "a block under a minute takes the note");
    assert_eq!(h.state.snapshot().task_note, "Invoices");

    h.machine.input_at(at("10:05:00"));
    h.command(at("10:05:00"), Command::SetTaskNote { note: "Client call".into(), at: at("10:05:00") });
    let queued = h.queued();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::Task))]);
    assert_eq!(queued[0].description.as_deref(), Some("Invoices"));
    assert_eq!(h.tracking_since(), Some(at("10:00:00")), "tracking goes on");
    assert_eq!(h.state.snapshot().block_started_at, Some(at("10:05:00")));

    // The same note again is no change.
    h.command(at("10:06:00"), Command::SetTaskNote { note: " Client  call ".into(), at: at("10:06:00") });
    assert_eq!(h.queued().len(), 1);
}

#[test]
fn sync_now_saves_a_block_over_a_minute_old_and_retries_uploads() {
    let mut h = tracking();
    h.machine.input_at(at("10:00:20"));
    h.command(at("10:00:30"), Command::SyncNow { at: at("10:00:30") });
    assert!(h.queued().is_empty(), "under a minute: keeps running");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Now)));

    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:00"), Command::SyncNow { at: at("10:04:00") });
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:00"), Some(EndReason::Interval))]);
    assert_eq!(h.state.snapshot().next_capture_at, Some(at("10:14:00")), "a full interval from the cut");
}

#[test]
fn a_screenshot_is_taken_at_the_random_moment_or_at_an_early_end() {
    let mut h = tracking();
    // 50%: 10:05.
    h.machine.input_at(at("10:04:59"));
    h.tick(at("10:04:59"));
    assert_eq!(h.machine.captures(), 0);
    h.tick(at("10:05:01"));
    assert_eq!(h.machine.captures(), 1);
    h.machine.input_at(at("10:09:59"));
    h.tick(at("10:10:00"));
    let queued = h.queued();
    assert_eq!(queued[0].screenshot_captured_at, Some(at("10:05:01")));
    assert_eq!(h.machine.captures(), 1, "one per block");
    let preview = h.state.snapshot().last_screenshot.expect("shown in the window");
    assert_eq!(preview.taken_at, at("10:05:01"), "when it was taken, not when the block was saved");

    // Paused at 10:12, before the next block's moment (10:15): taken then.
    h.machine.input_at(at("10:12:00"));
    h.command(at("10:12:00"), Command::Pause { resume_after: None, at: at("10:12:00") });
    assert_eq!(h.machine.captures(), 2);
    assert_eq!(h.queued()[1].screenshot_captured_at, Some(at("10:12:00")));
}
