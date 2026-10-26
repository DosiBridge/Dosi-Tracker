#![cfg(target_os = "linux")]
//! `session.json` (design 9.1): format, 0600/0700 permissions, atomic
//! rewrites, the 7-day project cache, damaged files moved aside, and
//! symlinks refused.

mod support;

use std::os::unix::fs::PermissionsExt;

use chrono::{DateTime, TimeDelta, Utc};
use dosi_tracker_linux::agent::model::Project;
use dosi_tracker_linux::agent::session_store::{
    self, ProjectCache, SavedPause, SavedWorkspace, SessionData, SessionStoreError,
};
use dosi_tracker_linux::paths::LinuxPaths;
use support::xdg::TestXdg;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn paths(xdg: &TestXdg) -> LinuxPaths {
    LinuxPaths::resolve(
        Some(&xdg.home),
        Some(&xdg.config_home),
        Some(&xdg.data_home),
        Some(&xdg.state_home),
    )
}

fn project(id: &str) -> Project {
    serde_json::from_value(serde_json::json!({
        "id": id, "title": format!("Project {id}"), "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": true, "allowMouse": true,
        "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": 15,
        "blurScreenshots": true
    }))
    .unwrap()
}

fn full_session() -> SessionData {
    let resolved = at("2026-10-05T09:00:00Z");
    SessionData {
        server: Some("https://tracker.acme.test".into()),
        account: Some("ayesha@acme.test".into()),
        workspace: Some(SavedWorkspace {
            id: "8f3c2d4e-1b2a-4c5d-9e8f-0a1b2c3d4e5f".into(),
            name: "Acme".into(),
        }),
        project_id: Some("p2".into()),
        projects: Some(ProjectCache {
            resolved_at: resolved,
            projects: vec![project("p1"), project("p2")],
        }),
        pause: Some(SavedPause {
            since: resolved,
            until: Some(resolved + TimeDelta::minutes(15)),
        }),
        task_note: Some("Invoice export".into()),
        ..SessionData::default()
    }
}

#[test]
fn a_missing_file_is_a_fresh_session_and_nothing_is_created() {
    let xdg = TestXdg::builder().create_dirs(false).build();
    let file = paths(&xdg).session_file();
    let loaded = session_store::load(&file, Utc::now()).unwrap();
    assert_eq!(loaded.data, SessionData::default());
    assert_eq!(loaded.moved_aside, None);
    assert!(!xdg.state_home.exists(), "loading creates nothing");
}

#[test]
fn everything_remembered_survives_a_round_trip_in_a_private_file() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).session_file();
    let saved = full_session();
    session_store::save(&file, &saved).unwrap();

    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(file.parent().unwrap()), 0o700);
    let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["session.json"], "no temporary file left behind");

    let back = session_store::load(&file, Utc::now()).unwrap();
    assert_eq!(back.data, saved);
    assert_eq!(
        back.data.projects.as_ref().unwrap().projects[1].idle_threshold_minutes,
        15
    );

    // Saving again replaces the file.
    let mut changed = saved.clone();
    changed.pause = None;
    session_store::save(&file, &changed).unwrap();
    assert_eq!(
        session_store::load(&file, Utc::now()).unwrap().data.pause,
        None
    );
}

#[test]
fn the_file_has_the_design_shape_and_no_secrets() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).session_file();
    session_store::save(&file, &full_session()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(json["format_version"], 1);
    assert_eq!(json["server"], "https://tracker.acme.test");
    assert_eq!(json["account"], "ayesha@acme.test");
    assert_eq!(json["workspace"]["name"], "Acme");
    assert_eq!(json["project_id"], "p2");
    assert_eq!(json["projects"]["resolvedAt"], "2026-10-05T09:00:00Z");
    assert_eq!(json["projects"]["projects"][0]["id"], "p1");
    assert_eq!(json["pause"]["until"], "2026-10-05T09:15:00Z");
    assert_eq!(json["task_note"], "Invoice export");
    let text = json.to_string().to_lowercase();
    for secret in ["password", "token", "signing"] {
        assert!(!text.contains(secret), "{secret} never stored here");
    }
}

#[test]
fn the_saved_project_list_is_usable_for_seven_days() {
    let data = full_session();
    let resolved = at("2026-10-05T09:00:00Z");
    assert_eq!(
        data.usable_projects(resolved + TimeDelta::days(7))
            .map(<[Project]>::len),
        Some(2)
    );
    assert!(data
        .usable_projects(resolved + TimeDelta::days(7) + TimeDelta::seconds(1))
        .is_none());
    assert!(
        data.usable_projects(resolved - TimeDelta::minutes(11))
            .is_none(),
        "more than 10 min in the future"
    );
    assert!(data
        .usable_projects(resolved - TimeDelta::minutes(10))
        .is_some());
}

#[test]
fn a_damaged_file_is_moved_aside_and_a_fresh_session_starts() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).session_file();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let now = at("2026-10-05T09:30:00Z");
    for damaged in [
        b"{not json".to_vec(),
        br#"{"format_version": 2, "account": "a@b.test"}"#.to_vec(),
        br#"{"account": "no version"}"#.to_vec(),
    ] {
        std::fs::write(&file, &damaged).unwrap();
        let loaded = session_store::load(&file, now).unwrap();
        assert_eq!(loaded.data, SessionData::default());
        let aside = loaded.moved_aside.expect("moved aside");
        assert_eq!(
            aside.file_name().unwrap().to_str().unwrap(),
            "session.json.damaged-20261005T093000Z"
        );
        assert_eq!(std::fs::read(&aside).unwrap(), damaged, "kept as it was");
        assert!(!file.exists());
        std::fs::remove_file(aside).unwrap();
    }
}

#[test]
fn a_symlinked_session_file_is_refused_and_its_target_untouched() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).session_file();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let target = xdg.home.join("elsewhere.json");
    std::fs::write(&target, b"{}").unwrap();
    std::os::unix::fs::symlink(&target, &file).unwrap();
    assert!(matches!(
        session_store::load(&file, Utc::now()),
        Err(SessionStoreError::Refused(_))
    ));
    assert!(
        session_store::save(&file, &full_session()).is_err(),
        "never written through"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"{}");
}

#[test]
fn signing_out_keeps_the_server_and_clear_removes_the_file() {
    let xdg = TestXdg::new();
    let file = paths(&xdg).session_file();
    session_store::save(&file, &full_session().signed_out()).unwrap();
    let back = session_store::load(&file, Utc::now()).unwrap().data;
    assert_eq!(back.server.as_deref(), Some("https://tracker.acme.test"));
    assert_eq!(
        (back.account, back.workspace, back.projects),
        (None, None, None)
    );
    session_store::clear(&file).unwrap();
    assert!(!file.exists());
    session_store::clear(&file).unwrap();
}
