//! Against a live backend (ignored unless `DOSI_E2E_*` is set).

use super::*;

// ---- Against a live backend ------------------------------------------------------------

/// End-to-end against a live backend: real input + window sampling and
/// screenshot → local queue → upload → today/week totals. Run with
/// `DOSI_E2E_URL`, `DOSI_E2E_WORKSPACE`, `DOSI_E2E_USER`,
/// `DOSI_E2E_PASSWORD`, `DOSI_E2E_PROJECT` set:
/// `cargo test e2e_ -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "needs a running backend and a desktop session"]
async fn e2e_capture_upload_and_totals() {
    let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
    let cfg = AppConfig { api_base_url: env("DOSI_E2E_URL"), ..AppConfig::default() };
    let account = Account::new(cfg.api_base_url.clone(), env("DOSI_E2E_USER"), env("DOSI_E2E_PASSWORD"));
    let mut client = account.client(&ServerClock::default());
    client.login().await.expect("sign-in");
    let wanted = env("DOSI_E2E_WORKSPACE");
    let workspaces = client.workspaces_mine().await.expect("workspaces");
    let workspace = workspaces.iter().find(|w| w.id == wanted || w.name.eq_ignore_ascii_case(&wanted)).expect("the workspace is listed");
    client.set_workspace(Some(workspace.id.clone()));
    let projects = client.my_projects().await.expect("projects");
    let project = projects.iter().find(|p| p.id == env("DOSI_E2E_PROJECT")).expect("the project is listed");
    let session = session_for(project, &cfg);

    let tracker = Tracker::new();
    let path = std::env::temp_dir().join(format!("dosi-e2e-{}.db", uuid::Uuid::new_v4()));
    let store = Storage::open(&path.to_string_lossy(), None).unwrap();
    let owner = account.owner();

    // Two minute buckets' worth of real sampling.
    let started = Utc::now();
    tokio::time::sleep(Duration::from_secs(125)).await;
    let ended = Utc::now();
    let screen = Machine::capture_screen(&tracker, session.policy.perms.blur_screenshots);
    let spec = BlockSpec {
        project_id: session.project_id(),
        perms: &session.policy.perms,
        started_at: started,
        ended_at: ended,
        end_reason: Some(EndReason::Interval),
        description: Some("e2e"),
    };
    let activity = Machine::snapshot(&tracker, &spec, screen);
    store.enqueue(&activity, &owner, &workspace.id).unwrap();
    let queued = store.pending_batch(&owner, 0, 1).unwrap().remove(0).1;
    println!(
        "queued: {} windows, {} timeline minutes, screenshot {}, thumb {}, keys {:?}, clicks {:?}, {:?}%",
        queued.active_windows.len(),
        queued.timeline.len(),
        queued.screenshot_png_base64.is_some(),
        queued.screenshot_thumb_jpg_base64.is_some(),
        queued.keyboard_hits,
        queued.mouse_clicks,
        queued.productivity,
    );
    assert_eq!(queued.timeline.len(), 2);
    assert!(queued.active_windows.iter().all(|w| w.seconds.is_some()));

    client.submit_activity(&queued).await.expect("uploaded");
    store.remove_synced(store.pending_batch(&owner, 0, 1).unwrap()[0].0).unwrap();
    let now = Utc::now();
    let (today_start, _) = local_day_and_week_start(now, &Local);
    let today = client.summary(today_start, now, crate::tracking::system::time_zone().as_deref()).await.unwrap();
    println!("server total today: {:.1} min", today.total_tracked_minutes);
    assert!(today.total_tracked_minutes >= 2.0, "today's total includes the block");
    drop(store);
    let _ = std::fs::remove_file(&path);
}

/// End-to-end drive of the worker loop through the commands the window and
/// tray send: sign-in, a timed pause that saves the block and resumes by
/// itself, an open-ended pause + resume, a too-short "Sync now", and quit.
/// Uses the per-user queue and saved sign-in, so run it only on a machine
/// whose tracker data may be replaced. Same env vars as
/// [`e2e_capture_upload_and_totals`].
#[test]
#[ignore = "needs a running backend; replaces the saved sign-in"]
fn e2e_pause_resume_flow() {
    use std::time::Instant;
    let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
    let cfg = AppConfig { api_base_url: env("DOSI_E2E_URL"), ..AppConfig::default() };
    let state = StateHandle::new();
    let worker = spawn(state.clone(), cfg, Arc::new(Tracker::new()), Launch::default(), Arc::new(|| {}));
    let tx = worker.commands.clone();
    let wait_for = |what: &str, secs: u64, done: &dyn Fn(&crate::state::SharedState) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let s = state.snapshot();
            if done(&s) {
                return s;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {:?}", s.status);
            std::thread::sleep(Duration::from_millis(200));
        }
    };

    tx.send(Command::SignIn {
        workspace: std::env::var("DOSI_E2E_WORKSPACE").ok(),
        username: env("DOSI_E2E_USER"),
        password: env("DOSI_E2E_PASSWORD"),
        server: None,
    })
    .unwrap();
    let s = wait_for("tracking", 30, &|s| {
        matches!(s.status, TrackerStatus::Tracking | TrackerStatus::ChooseProject)
    });
    if s.status == TrackerStatus::ChooseProject {
        tx.send(Command::SelectProject { project_id: env("DOSI_E2E_PROJECT"), at: Utc::now() }).unwrap();
    }
    let s = wait_for("tracking", 30, &|s| s.status == TrackerStatus::Tracking);
    println!("signed in, tracking since {:?}, block saved at {:?}", s.tracking_since, s.next_capture_at);

    // Work for over a minute, then take a 5-second timed pause.
    std::thread::sleep(Duration::from_secs(65));
    let synced_before = state.snapshot().last_sync;
    tx.send(Command::Pause { resume_after: Some(Duration::from_secs(5)), at: Utc::now() }).unwrap();
    let s = wait_for("paused", 20, &|s| s.status == TrackerStatus::Paused);
    assert!(s.resume_at.is_some() && s.tracking_since.is_none());
    let s = wait_for("the paused block uploaded", 60, &|s| s.last_sync != synced_before && s.pending_uploads == 0);
    println!("paused; block saved and uploaded; today {:.0}s", s.tracked_today_secs);
    let s = wait_for("auto-resume", 20, &|s| s.status == TrackerStatus::Tracking);
    assert!(s.resume_at.is_none() && s.paused_since.is_none());
    println!("auto-resumed");

    // Open-ended pause, then an explicit resume.
    tx.send(Command::Pause { resume_after: None, at: Utc::now() }).unwrap();
    let s = wait_for("paused again", 20, &|s| s.status == TrackerStatus::Paused);
    assert!(s.resume_at.is_none());
    tx.send(Command::Resume { at: Utc::now() }).unwrap();
    wait_for("resumed", 20, &|s| s.status == TrackerStatus::Tracking);
    println!("paused and resumed");

    // A block under a minute is not cut by "Sync now".
    let block = state.snapshot().block_started_at;
    tx.send(Command::SyncNow { at: Utc::now() }).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(state.snapshot().block_started_at, block, "short block kept running");
    println!("short sync kept the block");

    tx.send(Command::Shutdown { at: Utc::now() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !worker.thread.is_finished() {
        assert!(Instant::now() < deadline, "worker did not shut down");
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("shut down cleanly");
}
