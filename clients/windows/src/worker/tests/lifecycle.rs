//! Starting, pausing, breaks, idle, the screen lock, sleep and shutdown,
//! and the presence heartbeat that follows them.

use super::*;

#[test]
fn a_saved_sign_in_starts_tracking_at_once_on_the_saved_projects() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p2"))), false, None);
    assert_eq!(h.status(), TrackerStatus::Tracking, "offline first: no waiting for the server");
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.selected_project.as_deref(), Some("p2"), "the remembered project");
    assert_eq!(snapshot.tracking_since, Some(at("10:00:00")));
    assert_eq!(snapshot.projects.len(), 2);
    assert_eq!(snapshot.display_name, "alice@acme.com");

    let requests = h.control_requests();
    assert!(matches!(&requests[0], ControlRequest::UseAccount(Some(account)) if account.username == "alice@acme.com"));
    assert!(matches!(requests[1], ControlRequest::RefreshProjects), "re-validated with the server");
    assert!(requests.iter().any(|r| matches!(r, ControlRequest::Heartbeat(h) if h.status == PresenceStatus::Tracking)));
    assert!(matches!(&h.upload_commands()[0], UploadCommand::Account(Some(_))));
}

#[test]
fn several_projects_and_none_remembered_ask_the_user_to_choose() {
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("acme", 5), project("zeta", 5)], None)), false, None);
    assert_eq!(h.status(), TrackerStatus::ChooseProject, "never silently the first one");
    assert!(h.tracking_since().is_none());
    assert!(h.state.snapshot().notices.iter().any(|n| n.kind == NoticeKind::ChooseProject));
    assert!(h.heartbeats().contains(&PresenceStatus::Choosing));

    h.command(at("10:01:00"), Command::SelectProject { project_id: "zeta".into(), at: at("10:01:00") });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:01:00")));
    assert_eq!(h.worker.saved.as_ref().unwrap().project_id.as_deref(), Some("zeta"), "remembered");
}

#[test]
fn without_a_recent_project_list_it_waits_for_the_server() {
    let mut old = saved(vec![project("p1", 5)], Some("p1"));
    old.projects.as_mut().unwrap().resolved_at = at("10:00:00") - TimeDelta::days(8);
    let mut h = harness_at(at("10:00:00"), Some(old), false, None);
    assert_eq!(h.status(), TrackerStatus::Connecting, "a list older than a week is not used");

    h.worker.on_control(ControlEvent::Reachable(false));
    h.worker.on_control(ControlEvent::ProjectsFailed(ApiError::Offline("tcp connect error".into())));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Offline);

    h.machine.set(at("10:03:00"));
    h.worker.on_control(ControlEvent::Projects(vec![project("p1", 5)]));
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:03:00")));
    assert!(h.state.snapshot().server_reachable);
    // The fresh list is saved for the next offline start.
    assert_eq!(h.worker.saved.as_ref().unwrap().projects.as_ref().unwrap().resolved_at, at("10:03:00"));
}

#[test]
fn the_block_ends_where_the_user_paused_and_the_pause_survives_a_restart() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    credentials::save_at(&file, &saved(vec![project("p1", 5)], None)).unwrap();
    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), false, Some(file.clone()));
    h.machine.input_at(at("10:03:30"));
    h.tick(at("10:04:00"));

    // Clicked at 10:05; the worker got to it two seconds later.
    h.command(at("10:05:02"), Command::Pause { resume_after: None, at: at("10:05:00") });
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::Pause))]);
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(h.state.snapshot().paused_since, Some(at("10:05:00")));
    assert!(matches!(h.upload_commands().last(), Some(UploadCommand::RowsQueued)));

    // Restarted by Windows at sign-in: still paused.
    let mut restarted = harness_at(at("18:00:00"), credentials::load_from(&file), true, Some(file.clone()));
    assert_eq!(restarted.status(), TrackerStatus::Paused);
    assert!(restarted.tracking_since().is_none());
    assert!(restarted.heartbeats().contains(&PresenceStatus::Paused));
    restarted.command(at("18:01:00"), Command::Resume { at: at("18:01:00") });
    assert_eq!(restarted.tracking_since(), Some(at("18:01:00")), "a fresh block from the click");
    assert!(credentials::load_from(&file).unwrap().pause.is_none(), "resuming is saved too");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn a_timed_break_never_resumes_silently_after_an_autostart() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    let mut on_break = saved(vec![project("p1", 5)], None);
    on_break.pause = Some(SavedPause { since: at("09:50:00"), until: Some(at("10:05:00")) });
    credentials::save_at(&file, &on_break).unwrap();

    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), true, Some(file.clone()));
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert_eq!(h.state.snapshot().resume_at, None, "paused until the user resumes");
    h.machine.input_at(at("10:06:00"));
    h.tick(at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Paused);

    // Opened by the user instead: the break goes on and ends by itself.
    let mut h = harness_at(at("10:00:00"), Some(on_break), false, None);
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:05:00")));
    assert!(h.heartbeats().contains(&PresenceStatus::Break));
    h.machine.input_at(at("10:04:00"));
    h.tick(at("10:05:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:05:00")), "from the end of the break");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn a_break_that_ends_while_the_user_is_away_resumes_with_their_return() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::Break))]);
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:20:00")));

    // Nobody touched the computer: the break ends, tracking waits.
    h.tick(at("10:20:01"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(h.tracking_since().is_none());

    h.come_back(at("10:31:10"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:31:10")));
    assert!(h.state.snapshot().away_prompt.is_none(), "a break is not time to ask about");
}

#[test]
fn a_break_that_runs_out_during_a_sleep_resumes_at_the_wake_not_inside_the_sleep() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
    h.system(SystemEvent::Suspending, at("10:10:00"));
    h.machine.sleep_until(at("13:00:00"));
    h.worker.on_system(SystemNotice { event: SystemEvent::Resumed, at: at("13:00:00"), done: None });
    // The key press that woke it is recent: someone is there.
    h.machine.input_at(at("13:00:01"));
    h.tick(at("13:00:02"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("13:00:00")), "from the wake, not from the end of the break");
    h.machine.input_at(at("13:10:30"));
    h.tick(at("13:10:31"));
    assert!(h.queued().iter().skip(1).all(|block| block.started_at >= at("13:00:00")), "{:?}", h.spans());
}

#[test]
fn a_break_that_runs_out_during_a_sleep_nobody_announced_resumes_after_it() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
    h.machine.input_at(at("10:09:59"));
    h.tick(at("10:10:00"));
    h.machine.sleep_until(at("13:00:00"));
    h.machine.input_at(at("13:00:01"));
    h.machine.set(at("13:00:02"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.tracking_since(), Some(at("13:00:02")));
}

#[test]
fn time_away_can_be_kept_after_the_sign_in_expired() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    h.come_back(at("10:40:00"));
    assert!(h.state.snapshot().away_prompt.is_some());
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::SignInExpired { owner, reason: "Invalid username or password!".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert!(h.state.snapshot().away_prompt.is_some(), "the question stays");
    h.command(at("10:40:05"), Command::ResolveAway { keep: true });
    let kept: Vec<Activity> = h.queued().into_iter().filter(|a| a.end_reason == Some(EndReason::Idle) && a.started_at >= at("10:02:00")).collect();
    assert_eq!(kept.first().map(|a| a.started_at), Some(at("10:02:00")));
    assert_eq!(kept.last().map(|a| a.ended_at), Some(at("10:40:00")));
}

#[test]
fn upload_events_of_the_previous_account_are_ignored() {
    let mut h = tracking();
    let other = storage::owner_key("https://localhost:6006", "bob@acme.com");
    h.worker.on_upload(UploadEvent::Projects { owner: other.clone(), projects: vec![project("p9", 5)] });
    h.worker.on_upload(UploadEvent::SignInExpired { owner: other, reason: "Invalid username or password!".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.state.snapshot().projects.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p1"]);
}

#[test]
fn going_idle_ends_the_block_at_the_last_input_and_asks_about_the_time_away() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking, "not idle for five minutes yet");
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert_eq!(h.state.snapshot().away_since, Some(at("10:02:00")));
    let queued = h.queued();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:02:00"), Some(EndReason::Idle))]);
    assert!(queued[0].screenshot_png_base64.is_none(), "no capture for an idle end");
    assert!(h.heartbeats().contains(&PresenceStatus::Idle));

    // Back at 10:30: a new block starts there, and the question is asked.
    h.come_back(at("10:30:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:30:00")));
    let prompt = h.state.snapshot().away_prompt.expect("asked");
    assert_eq!((prompt.from, prompt.to, prompt.minutes()), (at("10:02:00"), at("10:30:00"), 28));
    assert_eq!(prompt.project_title, "Project p1");

    // Kept: 28 minutes as idle blocks of at most one interval, with the note.
    h.command(at("10:30:05"), Command::SetTaskNote { note: "Client call".into(), at: at("10:30:05") });
    h.command(at("10:30:10"), Command::ResolveAway { keep: true });
    let kept: Vec<Activity> = h.queued().into_iter().skip(1).collect();
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0].started_at, at("10:02:00"));
    assert_eq!(kept[2].ended_at, at("10:30:00"));
    for block in &kept {
        let length = (block.ended_at - block.started_at).num_seconds();
        assert!(length <= 600);
        assert_eq!(block.productivity, Some(0));
        assert_eq!(block.idle_seconds, Some(length as u32));
        assert_eq!(block.end_reason, Some(EndReason::Idle));
        assert!(block.timeline.is_empty() && block.active_windows.is_empty() && !block.has_captures());
        assert_eq!(block.description.as_deref(), Some("Client call"));
        assert_eq!(block.project_id, "p1");
    }
    assert!(h.state.snapshot().away_prompt.is_none());
}

#[test]
fn time_away_is_discarded_unless_kept() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    h.come_back(at("10:40:00"));
    assert!(h.state.snapshot().away_prompt.is_some());
    h.command(at("10:40:05"), Command::ResolveAway { keep: false });
    assert_eq!(h.queued().len(), 1, "only the block before leaving");
    assert!(h.state.snapshot().away_prompt.is_none());
}

#[test]
fn an_idle_cut_lands_in_a_block_held_across_the_interval_boundary() {
    // Idle after 15 minutes; the user stops at 10:07.
    let mut h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 15)], None)), false, None);
    h.machine.input_at(at("10:07:00"));
    h.tick(at("10:10:01"));
    assert!(h.queued().is_empty(), "the 10:00-10:10 block waits for the user");
    assert_eq!(h.state.snapshot().unsaved_since, Some(at("10:00:00")), "and still counts in the live totals");
    h.tick(at("10:22:00"));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:07:00"), Some(EndReason::Idle))]);
}

#[test]
fn locking_ends_the_block_without_a_capture_and_unlocking_starts_a_new_one() {
    let mut h = tracking();
    // Locked before the block's screenshot moment (50%: 10:05).
    h.machine.input_at(at("10:03:30"));
    h.tick(at("10:04:00"));
    h.system(SystemEvent::Locked, at("10:04:30"));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:30"), Some(EndReason::Lock))]);
    assert!(h.queued()[0].screenshot_png_base64.is_none(), "never a capture of a locked screen");
    assert_eq!(h.machine.captures(), 0);
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(h.heartbeats().contains(&PresenceStatus::Locked));

    // Locked for lunch: nothing is recorded, idle or not.
    h.tick(at("11:00:00"));
    assert_eq!(h.queued().len(), 1);
    h.system(SystemEvent::Unlocked, at("11:30:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("11:30:00")));
}

#[test]
fn a_disconnected_session_is_not_tracked_until_it_is_back_and_unlocked() {
    let mut h = tracking();
    h.machine.input_at(at("10:03:00"));
    h.system(SystemEvent::Locked, at("10:04:00"));
    h.system(SystemEvent::Disconnected, at("10:04:01"));
    // Reconnected, but still on the lock screen.
    h.system(SystemEvent::Connected, at("10:30:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    h.system(SystemEvent::Unlocked, at("10:30:20"));
    assert_eq!(h.tracking_since(), Some(at("10:30:20")));
}

#[test]
fn the_block_is_saved_locally_before_the_machine_sleeps() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:30"));
    h.tick(at("10:05:30"));
    h.upload_commands();
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    h.machine.set(at("10:06:00"));
    h.worker.on_system(SystemNotice { event: SystemEvent::Suspending, at: at("10:06:00"), done: Some(done) });
    assert!(saved.try_recv().is_ok(), "Windows is told the block is saved");
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Sleep))]);
    assert!(h.upload_commands().is_empty(), "no network before sleep");

    // Three hours later.
    h.machine.sleep_until(at("13:06:00"));
    h.worker.on_system(SystemNotice { event: SystemEvent::Resumed, at: at("13:06:00"), done: None });
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.tracking_since(), Some(at("13:06:00")), "a new block from the wake");
    assert_eq!(h.queued().len(), 1, "the sleep is not reported twice");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::RowsQueued)));
}

#[test]
fn a_sleep_nobody_announced_ends_the_block_at_the_tick_before_it() {
    let mut h = tracking();
    h.machine.input_at(at("10:03:59"));
    h.tick(at("10:04:00"));
    // The lid closed; Windows said nothing. Three hours on the wall clock,
    // two seconds of awake time.
    h.machine.sleep_until(at("13:04:00"));
    h.machine.set(at("13:04:02"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:00"), Some(EndReason::Sleep))]);
    assert_eq!(h.tracking_since(), Some(at("13:04:02")));
}

#[test]
fn a_sleep_that_never_came_does_not_stop_tracking_for_good() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:00"));
    h.control_requests();
    h.system(SystemEvent::Suspending, at("10:06:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(h.heartbeats().is_empty(), "nothing goes over the network before sleep");
    // No resume was heard, but the user types: the machine is awake.
    h.machine.input_at(at("10:06:40"));
    h.tick(at("10:06:41"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:06:40")));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:06:00"), Some(EndReason::Sleep))]);
}

#[test]
fn the_session_ending_saves_the_block_and_a_cancelled_shutdown_goes_on() {
    let mut h = tracking();
    h.machine.input_at(at("10:07:00"));
    h.tick(at("10:07:00"));
    let (done, saved) = std::sync::mpsc::sync_channel(1);
    h.machine.set(at("10:07:30"));
    h.worker.on_system(SystemNotice { event: SystemEvent::SessionEnding, at: at("10:07:30"), done: Some(done) });
    assert!(saved.try_recv().is_ok());
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:07:30"), Some(EndReason::Shutdown))]);
    // A second notice (WM_ENDSESSION after WM_QUERYENDSESSION) changes nothing.
    h.system(SystemEvent::SessionEnding, at("10:07:31"));
    assert_eq!(h.queued().len(), 1);

    h.system(SystemEvent::SessionContinues, at("10:08:00"));
    assert_eq!(h.tracking_since(), Some(at("10:08:00")));
}

#[test]
fn heartbeats_carry_the_device_project_and_app_and_follow_each_change() {
    let mut h = tracking();
    let requests = h.control_requests();
    let heartbeat = requests
        .iter()
        .find_map(|r| match r {
            ControlRequest::Heartbeat(hb) => Some(hb),
            _ => None,
        })
        .expect("sent at once");
    assert_eq!(heartbeat.status, PresenceStatus::Tracking);
    assert_eq!(heartbeat.device_id, "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11");
    assert_eq!(heartbeat.project_id.as_deref(), Some("p1"));
    assert_eq!(heartbeat.current_app.as_deref(), Some("code.exe"), "the project records the active window");
    assert_eq!(heartbeat.agent_version.as_deref(), Some("9.9.9"));

    // No repeat within the minute while nothing changes...
    h.machine.input_at(at("10:00:30"));
    h.tick(at("10:00:30"));
    assert!(h.heartbeats().is_empty());
    // ...but at once on a change, without the app while paused.
    h.command(at("10:01:30"), Command::Pause { resume_after: None, at: at("10:01:30") });
    let paused: Vec<Heartbeat> = h
        .control_requests()
        .into_iter()
        .filter_map(|r| match r {
            ControlRequest::Heartbeat(hb) => Some(*hb),
            _ => None,
        })
        .collect();
    assert_eq!(paused.len(), 1);
    assert_eq!(paused[0].status, PresenceStatus::Paused);
    assert_eq!(paused[0].current_app, None);
    assert_eq!(paused[0].status_since, Some(at("10:01:30")));
}

#[test]
fn an_autostart_that_starts_tracking_says_so() {
    let h = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)), true, None);
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.state.snapshot().notices.iter().any(|n| matches!(&n.kind,
        NoticeKind::TrackingStartedAutomatically { project_title } if project_title == "Project p1")));
    let manual = harness_at(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)), false, None);
    assert!(manual.state.snapshot().notices.is_empty());
}

#[test]
fn starting_a_stretch_resets_the_clocks_and_clears_a_pause() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(30 * 60)), at: at("10:05:00") });
    assert_eq!(h.state.snapshot().resume_at, Some(at("10:35:00")));
    h.command(at("10:10:00"), Command::Resume { at: at("10:10:00") });
    let s = h.state.snapshot();
    assert_eq!(s.status, TrackerStatus::Tracking);
    assert!(s.paused_since.is_none() && s.resume_at.is_none());
    assert_eq!((s.tracking_since, s.block_started_at), (Some(at("10:10:00")), Some(at("10:10:00"))));
    assert_eq!(s.next_capture_at, Some(at("10:20:00")), "a full interval until the block is saved");
}
