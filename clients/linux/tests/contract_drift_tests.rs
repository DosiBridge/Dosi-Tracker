//! Contract drift between the agent's wire model and the backend sources.
//!
//! Reads the backend DTOs and constants from `../../backend/src` (relative to
//! this crate) and fails when:
//! - a key the agent serializes is not a property of the backend DTO it is
//!   bound to (`CreateActivityDto`, `WindowInfoDto`, `ActivityMinuteDto`,
//!   `HeartbeatDto`), or such a DTO gained a property the model lacks;
//! - a field the agent reads (`Project`, `Workspace`) is not a property of
//!   `MyProjectDto` / `MyWorkspaceDto`;
//! - the agent's platform, end reasons, presence statuses or business error
//!   codes are unknown to `ActivityConsts`, `AgentPresenceConsts` or
//!   `TrackerDomainErrorCodes`.
//!
//! The C# reader is deliberately small: it understands the plain
//! `public <Type> <Name> { get; set; }` properties and `const`/array
//! declarations these files use.

use std::collections::BTreeSet;
use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use dosi_tracker_linux::agent::model::{
    Activity, ActivityMinute, EndReason, Heartbeat, PresenceStatus, Project, WindowInfo, Workspace,
    PLATFORM,
};
use dosi_tracker_linux::api::codes;

fn backend_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("backend")
        .join("src")
}

fn read_backend(relative: &str) -> String {
    let path = backend_src().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read backend source {}: {e}", path.display()))
}

/// The body of `class <name>` (between its braces).
fn class_body<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("class {name}");
    let start = source
        .match_indices(&marker)
        .find(|(i, _)| {
            let after = &source[i + marker.len()..];
            after.starts_with(|c: char| c.is_whitespace() || c == ':' || c == '{')
        })
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("class {name} not found"));
    let open = start + source[start..].find('{').expect("class body");
    let mut depth = 0usize;
    for (offset, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces in class {name}");
}

/// The public auto-properties of `class <name>`, as camelCase JSON keys.
fn properties(source: &str, class: &str) -> BTreeSet<String> {
    let body = class_body(source, class);
    let mut keys = BTreeSet::new();
    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("public ") || !line.contains("{ get;") {
            continue;
        }
        let head = &line[..line.find('{').unwrap()];
        let name = head
            .split_whitespace()
            .last()
            .expect("property name")
            .trim();
        let mut chars = name.chars();
        let first = chars.next().unwrap().to_ascii_lowercase();
        keys.insert(std::iter::once(first).chain(chars).collect());
    }
    assert!(!keys.is_empty(), "no properties found in {class}");
    keys
}

/// The string literals inside `<name> = { ... };`.
fn string_array(source: &str, name: &str) -> BTreeSet<String> {
    let start = source
        .find(&format!("{name} ="))
        .unwrap_or_else(|| panic!("{name} not found"));
    let open = start + source[start..].find('{').unwrap();
    let close = open + source[open..].find('}').unwrap();
    source[open..close]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Every `const string X = "value";` value.
fn const_strings(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter(|line| line.contains("const string"))
        .filter_map(|line| line.split('"').nth(1))
        .map(str::to_string)
        .collect()
}

/// The JSON keys of a serialized value (an object).
fn keys_of(value: &impl serde::Serialize) -> BTreeSet<String> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect()
}

/// Drift of an outbound model against its DTO: keys the backend does not
/// know, and backend properties the model does not send.
fn drift(dto_source: &str, class: &str, model_keys: &BTreeSet<String>) -> Vec<String> {
    let backend = properties(dto_source, class);
    let mut problems = Vec::new();
    for key in model_keys.difference(&backend) {
        problems.push(format!(
            "{class}: the agent sends `{key}`, which the backend does not know"
        ));
    }
    for key in backend.difference(model_keys) {
        problems.push(format!(
            "{class}: the backend has `{key}`, which the agent does not model"
        ));
    }
    problems
}

fn at(minute: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 5, 10, minute, 0).unwrap()
}

/// A block with every optional field set, so every key is serialized.
fn full_activity() -> Activity {
    let mut activity = Activity::new("3fa85f64-5717-4562-b3fc-2c963f66afa6", at(0), at(3));
    activity.description = Some("note".into());
    activity.productivity = Some(50);
    activity.mouse_clicks = Some(1);
    activity.keyboard_hits = Some(2);
    activity.active_windows = vec![full_window()];
    activity.running_programs = vec![WindowInfo {
        app_name: "code".into(),
        ..Default::default()
    }];
    activity.timeline = vec![full_minute()];
    activity.screenshot_png_base64 = Some("AA==".into());
    activity.screenshot_thumb_jpg_base64 = Some("AA==".into());
    activity.webcam_jpg_base64 = Some("AA==".into());
    activity.platform = Some(PLATFORM.into());
    activity.os_version = Some("os".into());
    activity.agent_version = Some("0.1.0".into());
    activity.device_id = Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into());
    activity.device_name = Some("host".into());
    activity.time_zone = Some("UTC".into());
    activity.utc_offset_minutes = Some(0);
    activity.idle_seconds = Some(0);
    activity.end_reason = Some(EndReason::Interval);
    activity.sent_at = Some(at(3));
    activity.screenshot_captured_at = Some(at(1));
    activity.screenshot_blurred = Some(false);
    activity.injected_input_events = Some(0);
    activity.payload_signature = Some("sig".into());
    activity
}

fn full_window() -> WindowInfo {
    WindowInfo {
        app_name: "code".into(),
        window_title: "main.rs".into(),
        seconds: Some(60),
        keyboard_hits: Some(1),
        mouse_clicks: Some(1),
    }
}

fn full_minute() -> ActivityMinute {
    ActivityMinute {
        minute: 0,
        keyboard_hits: Some(1),
        mouse_clicks: Some(1),
        active: true,
        app_name: Some("code".into()),
        active_seconds: Some(30),
    }
}

fn full_heartbeat() -> Heartbeat {
    Heartbeat {
        device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
        device_name: Some("host".into()),
        platform: Some(PLATFORM.into()),
        os_version: Some("os".into()),
        agent_version: Some("0.1.0".into()),
        status: PresenceStatus::Tracking,
        project_id: Some("3fa85f64-5717-4562-b3fc-2c963f66afa6".into()),
        status_since: Some(at(0)),
        current_app: Some("code".into()),
        time_zone: Some("UTC".into()),
        utc_offset_minutes: Some(0),
        sent_at: Some(at(1)),
    }
}

/// Every presence status; the match makes a new variant a compile error here.
fn all_statuses() -> Vec<PresenceStatus> {
    let all = [
        PresenceStatus::Tracking,
        PresenceStatus::Paused,
        PresenceStatus::Break,
        PresenceStatus::Idle,
        PresenceStatus::Locked,
        PresenceStatus::Choosing,
        PresenceStatus::SignedOut,
        PresenceStatus::Stopped,
    ];
    for status in all {
        match status {
            PresenceStatus::Tracking
            | PresenceStatus::Paused
            | PresenceStatus::Break
            | PresenceStatus::Idle
            | PresenceStatus::Locked
            | PresenceStatus::Choosing
            | PresenceStatus::SignedOut
            | PresenceStatus::Stopped => {}
        }
    }
    all.to_vec()
}

const ACTIVITY_DTO: &str = "Dosi.Tracker.Application.Contracts/Activities/CreateActivityDto.cs";

#[test]
fn the_activity_upload_matches_create_activity_dto() {
    let source = read_backend(ACTIVITY_DTO);
    let mut problems = drift(&source, "CreateActivityDto", &keys_of(&full_activity()));
    problems.extend(drift(&source, "WindowInfoDto", &keys_of(&full_window())));
    problems.extend(drift(
        &source,
        "ActivityMinuteDto",
        &keys_of(&full_minute()),
    ));
    // `payloadSignature` is local tamper evidence the server ignores (design 0.3).
    problems.retain(|p| !p.contains("`payloadSignature`, which the backend does not know"));
    assert!(
        problems.is_empty(),
        "contract drift:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_drift_check_fails_when_the_backend_dto_gains_a_property() {
    let source = read_backend(ACTIVITY_DTO);
    let marker = "    public Guid ProjectId { get; set; }";
    assert!(source.contains(marker), "CreateActivityDto changed shape");
    let temp = tempfile::tempdir().unwrap();
    let copy = temp.path().join("CreateActivityDto.cs");
    std::fs::write(
        &copy,
        source.replacen(
            marker,
            &format!("{marker}\n\n    public int? FocusScore {{ get; set; }}"),
            1,
        ),
    )
    .unwrap();
    let grown = std::fs::read_to_string(&copy).unwrap();
    let problems = drift(&grown, "CreateActivityDto", &keys_of(&full_activity()));
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`focusScore`") && p.contains("which the agent does not model")),
        "{problems:?}"
    );

    // And an agent key the backend does not know is reported too.
    let mut keys = keys_of(&full_activity());
    keys.insert("keystrokeText".into());
    let problems = drift(&source, "CreateActivityDto", &keys);
    assert!(
        problems.iter().any(|p| p.contains("`keystrokeText`")),
        "{problems:?}"
    );
}

#[test]
fn the_heartbeat_matches_heartbeat_dto() {
    let source = read_backend("Dosi.Tracker.Application.Contracts/Presence/HeartbeatDto.cs");
    let problems = drift(&source, "HeartbeatDto", &keys_of(&full_heartbeat()));
    assert!(
        problems.is_empty(),
        "contract drift:\n{}",
        problems.join("\n")
    );
}

#[test]
fn projects_and_workspaces_read_only_backend_properties() {
    let projects = read_backend("Dosi.Tracker.Application.Contracts/Projects/MyProjectDto.cs");
    let project: Project = serde_json::from_str(
        r#"{"id":"p","title":"t","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
            "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,
            "allowRunningPrograms":true,"idleThresholdMinutes":5,"blurScreenshots":false}"#,
    )
    .unwrap();
    let backend = properties(&projects, "MyProjectDto");
    let unknown: Vec<_> = keys_of(&project).difference(&backend).cloned().collect();
    assert!(unknown.is_empty(), "Project reads unknown keys {unknown:?}");
    // The project policy is complete: every MyProjectDto field is honoured.
    let missing: Vec<_> = backend.difference(&keys_of(&project)).cloned().collect();
    assert!(missing.is_empty(), "Project ignores {missing:?}");

    let workspaces =
        read_backend("Dosi.Tracker.Application.Contracts/SaaS/RegisterWorkspaceDto.cs");
    let workspace: Workspace = serde_json::from_str(r#"{"id":"w"}"#).unwrap();
    let backend = properties(&workspaces, "MyWorkspaceDto");
    let unknown: Vec<_> = keys_of(&workspace).difference(&backend).cloned().collect();
    assert!(
        unknown.is_empty(),
        "Workspace reads unknown keys {unknown:?}"
    );
    assert!(
        backend.contains("isArchived"),
        "archived workspaces must be visible"
    );
}

#[test]
fn platform_end_reasons_and_statuses_are_known_to_the_backend() {
    let consts = read_backend("Dosi.Tracker.Domain.Shared/Activities/ActivityConsts.cs");
    assert!(string_array(&consts, "Platforms").contains(PLATFORM));
    let backend_reasons = string_array(&consts, "EndReasons");
    let ours: BTreeSet<String> = EndReason::ALL
        .iter()
        .map(|r| r.as_str().to_string())
        .collect();
    assert_eq!(ours, backend_reasons, "end reasons differ");

    let presence = read_backend("Dosi.Tracker.Domain.Shared/Presence/AgentPresenceConsts.cs");
    let backend_statuses = string_array(&presence, "Statuses");
    let ours: BTreeSet<String> = all_statuses()
        .into_iter()
        .map(|s| {
            serde_json::to_value(s)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(ours, backend_statuses, "presence statuses differ");
}

#[test]
fn every_error_code_the_agent_acts_on_exists() {
    let source = read_backend("Dosi.Tracker.Domain.Shared/TrackerDomainErrorCodes.cs");
    let backend = const_strings(&source);
    let unknown: Vec<_> = codes::ALL
        .iter()
        .filter(|code| !backend.contains(**code))
        .collect();
    assert!(
        unknown.is_empty(),
        "codes unknown to the backend: {unknown:?}"
    );
    for terminal in [
        codes::DEVICE_REVOKED,
        codes::WORKSPACE_ARCHIVED,
        codes::WORKSPACE_REQUIRED,
        codes::NOT_WORKSPACE_MEMBER,
    ] {
        assert!(codes::ALL.contains(&terminal));
    }
}
