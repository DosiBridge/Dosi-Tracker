//! `consent show/grant/revoke`, `capabilities` and `capture retry`.
//!
//! `consent grant` replaces the earlier choice. It prints what is and what
//! is never collected and asks for confirmation unless `--yes` (no terminal
//! and no `--yes`: exit 2). The webcam cannot be granted in this version
//! (exit 2). With the agent down, grants and revocations are written to
//! `consent.json` directly (under `daemon.lock`); revocation always works.

use serde_json::{json, Value};

use super::output::{CliError, Done};
use super::prompt;
use super::{ConsentCommand, Ctx, GrantArgs};
use crate::agent::worker::WEBCAM_UNAVAILABLE;
use crate::consent::{ConsentStore, ConsentedPermissions, LocalConsent};
use crate::ipc::method;

/// What is never collected, whatever is granted (design 7).
pub const NEVER_COLLECTED: [&str; 10] = [
    "the keys you press or what you type",
    "the clipboard",
    "mouse positions",
    "anything below one-second timing",
    "titles of windows in the background",
    "command lines of programs",
    "other users' programs",
    "the microphone",
    "your files",
    "hardware identifiers (a random device id is used)",
];

/// The permissions `args` asks for; `None` when nothing was chosen.
pub fn requested(args: &GrantArgs) -> Result<ConsentedPermissions, CliError> {
    if args.webcam {
        return Err(CliError::usage(WEBCAM_UNAVAILABLE));
    }
    let any = args.all
        || args.keyboard
        || args.mouse
        || args.active_window
        || args.running_programs
        || args.screenshots
        || args.time_only;
    if !any {
        return Err(CliError::usage(
            "say what may be recorded: --time-only, --all, or --keyboard --mouse --active-window --running-programs --screenshots",
        ));
    }
    Ok(ConsentedPermissions {
        screenshot: args.all || args.screenshots,
        webcam: false,
        keyboard: args.all || args.keyboard,
        mouse: args.all || args.mouse,
        active_window: args.all || args.active_window,
        running_programs: args.all || args.running_programs,
        blur_screenshots: !args.no_blur,
    })
}

/// The "collected" list of `perms`.
pub fn collected(perms: &ConsentedPermissions) -> Vec<&'static str> {
    let mut list = vec!["working time in blocks (start, end, project, task note)"];
    if perms.keyboard {
        list.push("how many keys you press per minute (counts only)");
    }
    if perms.mouse {
        list.push("how many times you click per minute (counts only)");
    }
    if perms.active_window {
        list.push("the title and program of the window in front");
    }
    if perms.running_programs {
        list.push("the names of your programs with a window");
    }
    if perms.screenshot {
        list.push(if perms.blur_screenshots {
            "one blurred screenshot per block"
        } else {
            "one screenshot per block (not blurred)"
        });
    }
    list
}

fn explain(perms: &ConsentedPermissions) -> String {
    let mut text = String::from("Dosi Tracker will record:\n");
    for item in collected(perms) {
        text.push_str(&format!("  + {item}\n"));
    }
    text.push_str("It never records:\n");
    for item in NEVER_COLLECTED {
        text.push_str(&format!("  - {item}\n"));
    }
    text
}

/// Write `consent` to the file while the agent is down (under `daemon.lock`).
fn write_offline(ctx: &Ctx, consent: &LocalConsent, origin: &str) -> Result<(), CliError> {
    super::config::with_offline_lock(ctx, || {
        let before = ConsentStore::load(&ctx.paths.consent_file).unwrap_or(LocalConsent::Absent);
        ConsentStore::save(&ctx.paths.consent_file, consent)
            .map_err(|e| CliError::failed(format!("the consent could not be saved: {e}")))?;
        if let Err(e) = crate::agent::daemon::methods::append_consent_receipt(
            &ctx.paths, &before, consent, origin,
        ) {
            eprintln!("Warning: the consent receipt could not be written: {e}");
        }
        Ok(())
    })
}

fn update(ctx: &Ctx, consent: LocalConsent) -> Result<(Value, Option<bool>, bool), CliError> {
    match ctx.connect()? {
        Some(mut client) => {
            let answer = client.call(
                method::CONSENT_UPDATE,
                json!({ "consent": consent, "origin": "cli" }),
            )?;
            Ok((answer.result, answer.changed, true))
        }
        None => {
            write_offline(ctx, &consent, "cli")?;
            Ok((
                json!({ "consent": consent, "source": "local" }),
                Some(true),
                false,
            ))
        }
    }
}

pub fn run(ctx: &Ctx, command: ConsentCommand) -> u8 {
    match command {
        ConsentCommand::Show => show(ctx),
        ConsentCommand::Grant(args) => ctx.out("consent.grant").run(|| {
            let perms = requested(&args)?;
            let text = explain(&perms);
            if !args.yes {
                if !prompt::interactive() {
                    return Err(CliError::usage(
                        "granting consent needs a confirmation: run it on a terminal or add --yes",
                    ));
                }
                eprint!("{text}");
                if !prompt::confirm("Allow this?") {
                    return Err(CliError::failed("Nothing was changed."));
                }
            }
            let (mut result, changed, live) = update(ctx, LocalConsent::Granted(perms))?;
            result["collected"] = json!(collected(&perms));
            result["never_collected"] = json!(NEVER_COLLECTED);
            let mut human = if args.yes { text } else { String::new() };
            human.push_str(if live {
                "Consent saved."
            } else {
                "Consent saved (the agent is not running; it applies when it starts)."
            });
            Ok(Done::new(result, human).changed(changed))
        }),
        ConsentCommand::Revoke { purge_queue, yes } => ctx.out("consent.revoke").run(|| {
            let (mut result, changed, live) = update(ctx, LocalConsent::Revoked)?;
            let mut text = "Consent withdrawn: nothing is recorded any more.".to_owned();
            if purge_queue {
                if !yes
                    && !(prompt::interactive()
                        && prompt::confirm("Delete every block that was not uploaded yet?"))
                {
                    return Err(CliError::usage(
                        "--purge-queue deletes queued blocks; add --yes",
                    ));
                }
                let purged = super::queue::purge(ctx, crate::ipc::PurgeWhich::All, live)?;
                result["purged"] = purged;
                text.push_str(" Queued blocks were deleted.");
            }
            Ok(Done::new(result, text).changed(changed))
        }),
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn show(ctx: &Ctx) -> u8 {
    ctx.out("consent.show").run(|| match ctx.connect()? {
        Some(mut client) => {
            let answer = client.call(method::CONSENT_GET, json!({}))?;
            let mut text = format!("Consent: {}\n", consent_word(&answer.result["consent"]));
            let caps = &answer.result["capabilities"];
            for name in ["keyboard", "mouse", "active_window", "running_programs", "screenshot", "webcam"] {
                let cap = &caps[name];
                text.push_str(&format!(
                    "  {:<17} you: {:<3}  workspace: {:<3}  config: {:<3}  now: {}{}\n",
                    name.replace('_', " "),
                    yes_no(cap["consented"] == true),
                    yes_no(cap["project_allows"] == true),
                    yes_no(cap["config_allows"] == true),
                    cap["state"].as_str().unwrap_or("?").replace('_', " "),
                    cap["reason"]
                        .as_str()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                ));
            }
            Ok(Done::new(answer.result, text))
        }
        None => {
            let consent = ConsentStore::load(&ctx.paths.consent_file).map_err(|e| {
                CliError::failed(format!("the consent file cannot be read: {e}"))
            })?;
            let config = ctx.config().config;
            let perms = consent.permissions();
            let restrictions = &config.capture;
            let rows = json!({
                "keyboard": { "consented": perms.keyboard, "config_allows": restrictions.keyboard },
                "mouse": { "consented": perms.mouse, "config_allows": restrictions.mouse },
                "active_window": { "consented": perms.active_window, "config_allows": restrictions.active_window },
                "running_programs": { "consented": perms.running_programs, "config_allows": restrictions.running_programs },
                "screenshot": { "consented": perms.screenshot, "config_allows": restrictions.screenshot },
                "webcam": { "consented": false, "config_allows": false },
            });
            let mut text = format!(
                "Consent: {} (the agent is not running: workspace policy and availability are unknown)\n",
                consent_word(&json!(consent))
            );
            for (name, row) in rows.as_object().into_iter().flatten() {
                text.push_str(&format!(
                    "  {:<17} you: {:<3}  config: {}\n",
                    name.replace('_', " "),
                    yes_no(row["consented"] == true),
                    yes_no(row["config_allows"] == true),
                ));
            }
            Ok(Done::new(
                json!({ "consent": consent, "capabilities": rows, "source": "local" }),
                text,
            ))
        }
    })
}

fn consent_word(value: &Value) -> &'static str {
    match value {
        Value::String(s) if s == "Revoked" => "withdrawn",
        Value::String(s) if s == "Absent" => "not given yet",
        Value::Object(o) if o.contains_key("Granted") => "given",
        _ => "unknown",
    }
}

pub fn capabilities(ctx: &Ctx) -> u8 {
    ctx.out("capabilities").run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(method::CAPABILITIES_GET, json!({}))?;
        let mut text = String::new();
        if let Some(caps) = answer.result["capabilities"].as_object() {
            for (name, cap) in caps {
                text.push_str(&format!(
                    "  {:<17} {}{}\n",
                    name.replace('_', " "),
                    cap["state"].as_str().unwrap_or("?").replace('_', " "),
                    cap["reason"]
                        .as_str()
                        .map(|r| format!(" - {r}"))
                        .unwrap_or_default()
                ));
            }
        }
        Ok(Done::new(answer.result, text))
    })
}

pub fn capture_retry(ctx: &Ctx) -> u8 {
    ctx.out("capture.retry").run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call(method::CAPTURE_RETRY_PERMISSION, json!({}))?;
        let state = answer.result["screenshot"]["state"]
            .as_str()
            .unwrap_or("unknown")
            .replace('_', " ");
        Ok(Done::new(answer.result, format!("Screenshots: {state}")).changed(answer.changed))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_explicit_and_never_include_the_webcam() {
        assert_eq!(requested(&GrantArgs::default()).unwrap_err().exit_code, 2);
        let webcam = GrantArgs {
            webcam: true,
            all: true,
            ..GrantArgs::default()
        };
        assert_eq!(requested(&webcam).unwrap_err().exit_code, 2);
        let all = requested(&GrantArgs {
            all: true,
            ..GrantArgs::default()
        })
        .unwrap();
        assert!(
            all.screenshot
                && all.keyboard
                && all.mouse
                && all.active_window
                && all.running_programs
        );
        assert!(!all.webcam && all.blur_screenshots);
        let time_only = requested(&GrantArgs {
            time_only: true,
            ..GrantArgs::default()
        })
        .unwrap();
        assert!(!time_only.has_any_active_capability());
        let unblurred = requested(&GrantArgs {
            screenshots: true,
            no_blur: true,
            ..GrantArgs::default()
        })
        .unwrap();
        assert!(unblurred.screenshot && !unblurred.blur_screenshots);
        assert!(collected(&unblurred)
            .iter()
            .any(|c| c.contains("not blurred")));
        assert_eq!(collected(&time_only).len(), 1);
        assert!(explain(&time_only).contains("never records"));
    }
}
