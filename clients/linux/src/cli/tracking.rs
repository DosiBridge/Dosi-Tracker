//! `start`, `stop`, `pause`, `break`, `resume`, `sync`, `away`, `project`,
//! `task` and `status [--watch]`.
//!
//! `status` is read-only: with the agent down it prints what the local files
//! say (account, consent, queue counts) and exits 3, creating nothing.

use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use serde_json::{json, Value};

use super::account::{local_session, snapshot};
use super::output::{human_duration, CliError, Done};
use super::{Ctx, ProjectCommand, TaskCommand};
use crate::agent::state::{CapabilityState, SharedState, TrackerStatus};
use crate::ipc::{exit, method, snapshot_value};

/// The longest pause or break.
pub const MAX_PAUSE: Duration = Duration::from_secs(24 * 3600);

/// `15m`, `30m`, `1h`, `1h30m`, `90s`, or a bare number of minutes.
pub fn parse_duration(text: &str) -> Result<Duration, CliError> {
    let text = text.trim().to_ascii_lowercase();
    let invalid = || {
        CliError::usage(format!(
            "`{text}` is not a duration (try 15m, 30m, 1h or 1h30m)"
        ))
    };
    if text.is_empty() {
        return Err(invalid());
    }
    let secs = if let Ok(minutes) = text.parse::<u64>() {
        minutes.checked_mul(60).ok_or_else(invalid)?
    } else {
        let mut total = 0u64;
        let mut number = String::new();
        for c in text.chars() {
            if c.is_ascii_digit() {
                number.push(c);
                continue;
            }
            let n: u64 = number.parse().map_err(|_| invalid())?;
            number.clear();
            let unit = match c {
                'h' => 3600,
                'm' => 60,
                's' => 1,
                _ => return Err(invalid()),
            };
            total = total
                .checked_add(n.checked_mul(unit).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
        }
        if !number.is_empty() {
            return Err(invalid());
        }
        total
    };
    let duration = Duration::from_secs(secs);
    if duration.is_zero() || duration > MAX_PAUSE {
        return Err(CliError::usage(
            "a pause lasts between 1 second and 24 hours",
        ));
    }
    Ok(duration)
}

fn local_time(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

fn at_now() -> Value {
    json!({ "at": Utc::now() })
}

pub fn start(ctx: &Ctx) -> u8 {
    let out = ctx.out("tracking.start");
    out.run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(method::TRACKING_START, at_now())?;
        let indicator = &answer.result["indicator"];
        if indicator["tray"] != true && indicator["notifications"] != true {
            out.warn("no tray or notification server shows that tracking is on; keep `dosi-tracker status` or the window at hand");
        }
        if answer.result["manual_mode"] == true {
            out.warn("this desktop cannot tell when you are away: pause when you stop working");
        }
        let text = if answer.changed == Some(false) {
            "Already tracking."
        } else {
            "Tracking."
        };
        Ok(Done::new(answer.result, text).changed(answer.changed))
    })
}

pub fn pause(ctx: &Ctx, duration: Option<Duration>, command: &'static str) -> u8 {
    let out = ctx.out(command);
    out.run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(
            method::TRACKING_PAUSE,
            json!({ "resume_after_secs": duration.map(|d| d.as_secs()), "at": Utc::now() }),
        )?;
        let resume_at = answer.result["resume_at"]
            .as_str()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));
        let text = match (answer.changed, resume_at) {
            (Some(false), _) => "Already paused.".to_owned(),
            (_, Some(at)) => format!("Paused; tracking resumes at {}.", local_time(at)),
            (_, None) => "Paused until you resume (dosi-tracker resume).".to_owned(),
        };
        Ok(Done::new(answer.result, text).changed(answer.changed))
    })
}

pub fn resume(ctx: &Ctx) -> u8 {
    let out = ctx.out("tracking.resume");
    out.run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(method::TRACKING_RESUME, at_now())?;
        let text = if answer.changed == Some(false) {
            "Already tracking."
        } else {
            "Resumed."
        };
        Ok(Done::new(answer.result, text).changed(answer.changed))
    })
}

pub fn sync(ctx: &Ctx) -> u8 {
    let out = ctx.out("tracking.sync");
    out.run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call_with_timeout(
            method::TRACKING_SYNC_NOW,
            at_now(),
            Duration::from_secs(45),
        )?;
        let pending = answer.result["pending"].as_u64().unwrap_or(0);
        let text = if pending == 0 {
            "Everything is uploaded.".to_owned()
        } else {
            format!("{pending} blocks are still waiting to upload.")
        };
        Ok(Done::new(answer.result, text).changed(answer.changed))
    })
}

pub fn away(ctx: &Ctx, keep: bool) -> u8 {
    let out = ctx.out(if keep { "away.keep" } else { "away.discard" });
    out.run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(method::AWAY_RESOLVE, json!({ "keep": keep }))?;
        let text = if keep {
            "The time away was kept as work."
        } else {
            "The time away was discarded."
        };
        Ok(Done::new(answer.result, text).changed(answer.changed))
    })
}

pub fn project(ctx: &Ctx, command: ProjectCommand) -> u8 {
    match command {
        ProjectCommand::List { refresh } => ctx.out("project.list").run(|| {
            let mut client = ctx.connect_or_start()?;
            let answer = client.call(method::PROJECT_LIST, json!({ "refresh": refresh }))?;
            let selected = answer.result["selected"].as_str().map(str::to_owned);
            let mut text = String::new();
            for project in answer.result["projects"].as_array().into_iter().flatten() {
                let id = project["id"].as_str().unwrap_or_default();
                let mark = if selected.as_deref() == Some(id) {
                    "*"
                } else {
                    " "
                };
                text.push_str(&format!(
                    "{mark} {}  ({id})\n",
                    project["title"].as_str().unwrap_or_default()
                ));
            }
            if text.is_empty() {
                text = "No projects.".into();
            }
            Ok(Done::new(answer.result, text))
        }),
        ProjectCommand::Use { project } => ctx.out("project.use").run(|| {
            let mut client = ctx.connect_or_start()?;
            let answer = client.call(
                method::PROJECT_SELECT,
                json!({ "project": project, "at": Utc::now() }),
            )?;
            let title = answer.result["project"]["title"]
                .as_str()
                .unwrap_or_default();
            let text = if answer.changed == Some(false) {
                format!("Already tracking {title}.")
            } else {
                format!("Tracking {title}.")
            };
            Ok(Done::new(answer.result.clone(), text).changed(answer.changed))
        }),
    }
}

pub fn task(ctx: &Ctx, command: TaskCommand) -> u8 {
    match command {
        TaskCommand::Show => ctx.out("task.show").run(|| {
            let mut client = ctx.connect_or_start()?;
            let s = snapshot(&mut client)?;
            let text = if s.task_note.is_empty() {
                "No task note.".to_owned()
            } else {
                s.task_note.clone()
            };
            Ok(Done::new(json!({ "task_note": s.task_note }), text))
        }),
        TaskCommand::Set { text } => set_task(ctx, text.join(" "), "task.set"),
        TaskCommand::Clear => set_task(ctx, String::new(), "task.clear"),
    }
}

fn set_task(ctx: &Ctx, note: String, command: &'static str) -> u8 {
    ctx.out(command).run(|| {
        if note.chars().count() > crate::agent::worker::TASK_NOTE_MAX_CHARS {
            return Err(CliError::usage(
                "the task note is longer than 200 characters",
            ));
        }
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(method::TASK_SET, json!({ "note": note, "at": Utc::now() }))?;
        let text = match answer.result["task_note"].as_str() {
            Some("") | None => "Task note cleared.".to_owned(),
            Some(note) => format!("Task: {note}"),
        };
        Ok(Done::new(answer.result, text).changed(answer.changed))
    })
}

fn capability_line(name: &str, info: &crate::agent::state::CapabilityInfo) -> String {
    let state = match info.state {
        CapabilityState::On => "on".to_owned(),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.replace('_', " ")))
            .unwrap_or_default(),
    };
    match &info.reason {
        Some(reason) if info.state != CapabilityState::On => {
            format!("  {name:<17} {state} - {reason}")
        }
        _ => format!("  {name:<17} {state}"),
    }
}

/// The human `status` text of `s`.
pub fn status_text(s: &SharedState, verbose: bool) -> String {
    let mut lines = vec![format!("Status:     {}", s.status.label())];
    if !s.display_name.is_empty() {
        lines.push(format!("Account:    {} ({})", s.display_name, s.server_url));
        lines.push(format!("Workspace:  {}", s.workspace));
    }
    if let Some(project) = s.current_project() {
        lines.push(format!("Project:    {}", project.title));
    }
    if !s.task_note.is_empty() {
        lines.push(format!("Task:       {}", s.task_note));
    }
    if let Some(started) = s.block_started_at {
        lines.push(format!("Block:      since {}", local_time(started)));
    }
    if let (Some(since), status) = (s.paused_since, s.status) {
        if status == TrackerStatus::Paused {
            match s.resume_at {
                Some(at) => lines.push(format!(
                    "Paused:     since {}, resumes at {}",
                    local_time(since),
                    local_time(at)
                )),
                None => lines.push(format!("Paused:     since {}", local_time(since))),
            }
        }
    }
    if let Some(prompt) = &s.away_prompt {
        lines.push(format!(
            "Away:       {} min ({} - {}): dosi-tracker away keep / away discard",
            prompt.minutes(),
            local_time(prompt.from),
            local_time(prompt.to)
        ));
    }
    lines.push(format!(
        "Today:      {}   This week: {}",
        human_duration(s.tracked_today_secs),
        human_duration(s.tracked_week_secs)
    ));
    lines.push(format!(
        "Queue:      {} waiting, {} not accepted{}",
        s.pending_uploads,
        s.rejected_uploads,
        s.last_sync
            .map(|t| format!(", last upload {}", local_time(t)))
            .unwrap_or_default()
    ));
    if let Some(issue) = &s.upload_issue {
        lines.push(format!("Uploads:    {}", issue.message()));
    }
    if let Some(skew) = s.clock_skew_minutes {
        lines.push(format!("Clock:      {skew} min off from the server"));
    }
    let session = serde_json::to_value(s.session.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    lines.push(format!(
        "Session:    {session}{}{}",
        s.session
            .desktop
            .as_deref()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default(),
        if s.manual_mode {
            ", manual mode: no idle detection"
        } else {
            ""
        }
    ));
    if s.credentials_locked {
        lines.push("Keyring:    locked - run dosi-tracker unlock".into());
    }
    if let Some(error) = s.last_error.as_ref().or(s.login_error.as_ref()) {
        lines.push(format!("Problem:    {error}"));
    }
    if verbose || s.status == TrackerStatus::Tracking {
        lines.push("Recording:".into());
        let caps = &s.capabilities;
        for (name, info) in [
            ("keyboard", &caps.keyboard),
            ("mouse", &caps.mouse),
            ("active window", &caps.active_window),
            ("running programs", &caps.running_programs),
            ("screenshots", &caps.screenshot),
        ] {
            lines.push(capability_line(name, info));
        }
    }
    lines.join("\n")
}

/// What the local files say while the agent is down (read-only).
pub fn local_facts(ctx: &Ctx) -> Value {
    let session = local_session(ctx);
    let consent = if ctx.paths.consent_file_exists() {
        crate::consent::ConsentStore::load(&ctx.paths.consent_file)
            .map(|c| json!(c))
            .unwrap_or_else(|e| json!({ "error": e.to_string() }))
    } else {
        json!("Absent")
    };
    let queue = session.as_ref().and_then(|s| {
        let account = s.account.as_deref()?;
        let workspace = s.workspace.as_ref()?.id.clone();
        let scope = crate::outbox::OutboxScope::new(account, workspace).ok()?;
        let db = ctx.paths.outbox_db();
        if !db.is_file() {
            return Some(json!({ "pending": 0 }));
        }
        crate::outbox::inspect_outbox_status(&db, &scope)
            .ok()
            .map(|report| {
                json!({
                    "pending": report.durable_queue.queued_count,
                    "workspace_id": scope.workspace_id,
                })
            })
    });
    json!({
        "source": "local",
        "daemon": "not_running",
        "account": session.as_ref().and_then(|s| s.account.clone()),
        "workspace": session.as_ref().and_then(|s| s.workspace.as_ref().map(|w| w.name.clone())),
        "server": session.as_ref().and_then(|s| s.server.clone()),
        "paused": session.as_ref().is_some_and(|s| s.pause.is_some()),
        "consent": consent,
        "queue": queue,
    })
}

pub fn status(ctx: &Ctx, watch: bool) -> u8 {
    let out = ctx.out("status");
    let client = match ctx.connect() {
        Ok(client) => client,
        Err(e) => return out.failure(&e),
    };
    let Some(mut client) = client else {
        let facts = local_facts(ctx);
        let error = CliError::not_running("The Dosi Tracker agent is not running.");
        return out.failure_with_result(&error, facts.clone(), || {
            let mut text = vec!["Status:     agent not running".to_owned()];
            if let Some(account) = facts["account"].as_str() {
                text.push(format!("Account:    {account}"));
            }
            if let Some(workspace) = facts["workspace"].as_str() {
                text.push(format!("Workspace:  {workspace}"));
            }
            if let Some(pending) = facts["queue"]["pending"].as_u64() {
                text.push(format!("Queue:      {pending} waiting"));
            }
            text.join("\n")
        });
    };
    if !watch {
        return match snapshot(&mut client) {
            Ok(s) => {
                let value = snapshot_value(s.clone());
                out.success(None, value, || status_text(&s, ctx.verbose > 0))
            }
            Err(e) => out.failure(&e),
        };
    }
    let mut subscription = match client.subscribe() {
        Ok(subscription) => subscription,
        Err(e) => return out.failure(&e.into()),
    };
    loop {
        match subscription.next_event(None) {
            Ok(event) if event.event == crate::ipc::EVENT_STATE => {
                if ctx.json {
                    println!(
                        "{}",
                        json!({ "schema": crate::ipc::CLI_SCHEMA, "command": "status.watch", "seq": event.seq, "state": event.data })
                    );
                } else if let Ok(s) = serde_json::from_value::<SharedState>(event.data) {
                    println!("{}\n", status_text(&s, ctx.verbose > 0));
                }
            }
            Ok(_) => {}
            Err(crate::ipc::client::ClientError::Timeout) => {}
            Err(e) => {
                let error: CliError = e.into();
                if ctx.json {
                    println!(
                        "{}",
                        out.error_envelope(&CliError::not_running(error.message.clone()))
                    );
                } else {
                    eprintln!("The agent went away: {}", error.message);
                }
                return exit::DAEMON;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_like_the_design_says() {
        let secs = |t: &str| parse_duration(t).unwrap().as_secs();
        assert_eq!(secs("15m"), 900);
        assert_eq!(secs("30m"), 1800);
        assert_eq!(secs("1h"), 3600);
        assert_eq!(secs("1h30m"), 5400);
        assert_eq!(secs("90s"), 90);
        assert_eq!(secs("45"), 2700, "a bare number is minutes");
        assert_eq!(secs(" 2H "), 7200);
        for bad in [
            "",
            "m",
            "15x",
            "1h30",
            "-5m",
            "25h",
            "0m",
            "99999999999999999999h",
        ] {
            assert_eq!(parse_duration(bad).unwrap_err().exit_code, 2, "{bad}");
        }
    }

    #[test]
    fn status_text_says_what_matters() {
        let s = SharedState {
            status: TrackerStatus::Paused,
            display_name: "ayesha@acme.test".into(),
            workspace: "Acme".into(),
            pending_uploads: 3,
            manual_mode: true,
            ..SharedState::default()
        };
        let text = status_text(&s, false);
        assert!(text.contains("Paused"), "{text}");
        assert!(text.contains("ayesha@acme.test"));
        assert!(text.contains("3 waiting"));
        assert!(text.contains("manual mode"));
        assert!(
            !text.contains("Recording:"),
            "capabilities only when tracking or -v"
        );
        assert!(status_text(&s, true).contains("Recording:"));
    }
}
