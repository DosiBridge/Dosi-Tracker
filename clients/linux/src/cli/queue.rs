//! `queue status/list/purge` and the hidden legacy `outbox status` inspector.
//!
//! - `queue status` is read-only: the running agent's counts, or, with it
//!   down, the read-only inspector on the signed-in account's scope (it never
//!   creates, migrates or checkpoints the database).
//! - `queue list` shows the newest rows without their contents.
//! - `queue purge --pending/--parked/--all --yes` deletes through the agent,
//!   or under `daemon.lock` with it down.
//! - `outbox status --db P --account-id A --workspace-id W [--json]` is the
//!   foundation inspector with its exact output and exit codes.

use std::path::Path;

use serde_json::{json, Value};

use super::account::local_session;
use super::output::{CliError, Done};
use super::prompt;
use super::{Ctx, QueueCommand};
use crate::ipc::{method, PurgeWhich};
use crate::outbox::{inspect_outbox_status, OutboxScope};

pub fn run(ctx: &Ctx, command: QueueCommand) -> u8 {
    match command {
        QueueCommand::Status => status(ctx),
        QueueCommand::List { parked, limit } => ctx.out("queue.list").run(|| {
            if limit == 0 || limit > crate::ipc::MAX_QUEUE_LIST {
                return Err(CliError::usage(format!(
                    "--limit is between 1 and {}",
                    crate::ipc::MAX_QUEUE_LIST
                )));
            }
            let mut client = ctx.connect_or_start()?;
            let answer = client.call(
                method::QUEUE_LIST,
                json!({ "parked": parked, "limit": limit }),
            )?;
            let mut text = String::new();
            for row in answer.result["rows"].as_array().into_iter().flatten() {
                text.push_str(&format!(
                    "#{:<6} {:<8} {} - {}{}\n",
                    row["id"],
                    row["state"].as_str().unwrap_or_default(),
                    row["started_at"].as_str().unwrap_or("?"),
                    row["ended_at"].as_str().unwrap_or("?"),
                    row["parked_reason"]
                        .as_str()
                        .or(row["last_error"].as_str())
                        .map(|r| format!("  ({r})"))
                        .unwrap_or_default()
                ));
            }
            if text.is_empty() {
                text = "The queue is empty.".into();
            }
            Ok(Done::new(answer.result, text))
        }),
        QueueCommand::Purge {
            pending,
            parked,
            all,
            yes,
        } => ctx.out("queue.purge").run(|| {
            let which = match (pending, parked, all) {
                (true, _, _) => PurgeWhich::Pending,
                (_, true, _) => PurgeWhich::Parked,
                (_, _, true) => PurgeWhich::All,
                _ => {
                    return Err(CliError::usage(
                        "say what to purge: --pending, --parked or --all",
                    ))
                }
            };
            if !yes {
                if !prompt::interactive() {
                    return Err(CliError::usage("purging deletes queued blocks; add --yes"));
                }
                if !prompt::confirm("Delete these queued blocks for good?") {
                    return Err(CliError::failed("Nothing was deleted."));
                }
            }
            let live = ctx.connect()?.is_some();
            let result = purge(ctx, which, live)?;
            Ok(Done::new(result, "Purged.").changed(Some(true)))
        }),
    }
}

/// Delete the signed-in account's rows of `which` kind: through the agent
/// when it runs (`live`), else directly under the offline lock.
pub fn purge(ctx: &Ctx, which: PurgeWhich, live: bool) -> Result<Value, CliError> {
    if live {
        if let Some(mut client) = ctx.connect()? {
            let answer = client.call(
                method::QUEUE_PURGE,
                json!({ "which": which, "confirmed": true }),
            )?;
            return Ok(answer.result);
        }
    }
    let account = local_session(ctx).and_then(|s| s.account).ok_or_else(|| {
        CliError::new(
            "not_signed_in",
            crate::ipc::exit::AUTH,
            "no account has queued blocks here",
        )
    })?;
    let db = ctx.paths.outbox_db();
    if !db.is_file() {
        return Ok(json!({ "removed": 0, "source": "local" }));
    }
    super::config::with_offline_lock(ctx, || {
        let outbox = crate::outbox::Outbox::open(&db, crate::outbox::OutboxLimits::default())
            .map_err(|e| CliError::failed(format!("the queue cannot be opened: {e}")))?;
        let target = match which {
            PurgeWhich::Pending => crate::outbox::PurgeTarget::Pending,
            PurgeWhich::Parked => crate::outbox::PurgeTarget::Parked,
            PurgeWhich::All => crate::outbox::PurgeTarget::All,
        };
        let removed = outbox
            .purge(&account, target)
            .map_err(|e| CliError::failed(format!("the queue could not be purged: {e}")))?;
        let _ = outbox.reclaim_space();
        Ok(json!({ "removed": removed, "source": "local" }))
    })
}

fn status(ctx: &Ctx) -> u8 {
    ctx.out("queue.status").run(|| {
        if let Some(mut client) = ctx.connect()? {
            let answer = client.call(method::QUEUE_STATUS, json!({}))?;
            let text = format!(
                "{} blocks waiting to upload, {} not accepted by the server.",
                answer.result["pending"], answer.result["parked"]
            );
            return Ok(Done::new(answer.result, text));
        }
        let session = local_session(ctx);
        let scope = session.as_ref().and_then(|s| {
            OutboxScope::new(s.account.clone()?, s.workspace.as_ref()?.id.clone()).ok()
        });
        let db = ctx.paths.outbox_db();
        let Some(scope) = scope else {
            return Ok(Done::new(
                json!({ "source": "local", "pending": 0, "signed_in": false }),
                "Not signed in: nothing is queued for an account here.",
            ));
        };
        if !db.is_file() {
            return Ok(Done::new(
                json!({ "source": "local", "pending": 0 }),
                "Nothing is queued.",
            ));
        }
        let report = inspect_outbox_status(&db, &scope)
            .map_err(|e| CliError::failed(format!("the queue cannot be inspected: {e}")))?;
        let value = serde_json::to_value(&report).unwrap_or(Value::Null);
        let pending = report.durable_queue.queued_count;
        Ok(Done::new(
            json!({ "source": "local", "pending": pending, "report": value }),
            format!(
                "{pending} blocks waiting to upload in this workspace (the agent is not running)."
            ),
        ))
    })
}

// ---- Legacy `outbox status` (foundation inspector, output unchanged) -----------------------

/// The hidden `outbox` command; `args` follow `outbox`.
pub fn legacy_outbox(args: &[String]) -> u8 {
    handle_outbox(args)
}

fn print_outbox_help() {
    println!("dosi-tracker outbox");
    println!("Inspect durable offline outbox storage.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker outbox <SUBCOMMAND> [OPTIONS]");
    println!();
    println!("SUBCOMMANDS:");
    println!("    status       Read-only inspection of queued records for an exact account and workspace scope");
    println!("    help         Print this help message");
    println!();
    println!("Run 'dosi-tracker outbox status --help' for options.");
}

fn print_outbox_status_help() {
    println!("dosi-tracker outbox status");
    println!("Read-only inspection of durable outbox queue for a specific scope.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker outbox status --db <PATH> --account-id <ID> --workspace-id <ID> [OPTIONS]");
    println!();
    println!("REQUIRED OPTIONS:");
    println!("    --db <PATH>             Path to existing SQLite database file (must exist)");
    println!(
        "    --account-id <ID>       Account identifier (normalized, non-empty, <= 256 bytes)"
    );
    println!("    --workspace-id <ID>     Workspace identifier (non-empty, <= 256 bytes)");
    println!();
    println!("OPTIONS:");
    println!("    --json                  Output status summary in versioned JSON format");
    println!("    -h, --help              Print this help message");
    println!();
    println!("INVARIANTS:");
    println!("    - Strictly read-only: opens SQLite with SQLITE_OPEN_READ_ONLY, no writes or WAL checkpoints.");
    println!("    - No DB creation: if path does not exist, fails closed without creating files or directories.");
    println!("    - Scoped aggregate: queries only the specified scope; does not output envelopes, payloads,");
    println!("      activity IDs, descriptions, or data from other workspaces.");
    println!("    - Remote sync state: reports offline queue status only; does not track remote delivery or acks.");
}
fn handle_outbox(args: &[String]) -> u8 {
    if args.is_empty() {
        eprintln!("Error: Missing outbox subcommand. Available: status");
        eprintln!("Run 'dosi-tracker outbox --help' for usage.");
        return 1;
    }

    match args[0].as_str() {
        "status" => handle_outbox_status(&args[1..]),
        "help" | "--help" | "-h" => {
            print_outbox_help();
            0
        }
        other => {
            eprintln!("Error: Unknown outbox subcommand: {}", other);
            eprintln!("Run 'dosi-tracker outbox --help' for available subcommands.");
            1
        }
    }
}

fn handle_outbox_status(args: &[String]) -> u8 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_outbox_status_help();
        return 0;
    }

    let mut db_path: Option<String> = None;
    let mut account_id: Option<String> = None;
    let mut workspace_id: Option<String> = None;
    let mut json_mode = false;
    let mut json_seen = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--db" {
            if db_path.is_some() {
                eprintln!("Error: Duplicate option: --db");
                return 1;
            }
            i += 1;
            if i >= args.len() || args[i].starts_with("--") {
                eprintln!("Error: Option '--db' requires a non-empty value");
                return 1;
            }
            let val = args[i].trim();
            if val.is_empty() {
                eprintln!("Error: Option '--db' requires a non-empty value");
                return 1;
            }
            db_path = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--db=") {
            if db_path.is_some() {
                eprintln!("Error: Duplicate option: --db");
                return 1;
            }
            let val = val.trim();
            if val.is_empty() {
                eprintln!("Error: Option '--db' requires a non-empty value");
                return 1;
            }
            db_path = Some(val.to_string());
        } else if arg == "--account-id" {
            if account_id.is_some() {
                eprintln!("Error: Duplicate option: --account-id");
                return 1;
            }
            i += 1;
            if i >= args.len() || args[i].starts_with("--") {
                eprintln!("Error: Option '--account-id' requires a non-empty value");
                return 1;
            }
            let val = args[i].trim();
            if val.is_empty() {
                eprintln!("Error: Option '--account-id' requires a non-empty value");
                return 1;
            }
            account_id = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--account-id=") {
            if account_id.is_some() {
                eprintln!("Error: Duplicate option: --account-id");
                return 1;
            }
            let val = val.trim();
            if val.is_empty() {
                eprintln!("Error: Option '--account-id' requires a non-empty value");
                return 1;
            }
            account_id = Some(val.to_string());
        } else if arg == "--workspace-id" {
            if workspace_id.is_some() {
                eprintln!("Error: Duplicate option: --workspace-id");
                return 1;
            }
            i += 1;
            if i >= args.len() || args[i].starts_with("--") {
                eprintln!("Error: Option '--workspace-id' requires a non-empty value");
                return 1;
            }
            let val = args[i].trim();
            if val.is_empty() {
                eprintln!("Error: Option '--workspace-id' requires a non-empty value");
                return 1;
            }
            workspace_id = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--workspace-id=") {
            if workspace_id.is_some() {
                eprintln!("Error: Duplicate option: --workspace-id");
                return 1;
            }
            let val = val.trim();
            if val.is_empty() {
                eprintln!("Error: Option '--workspace-id' requires a non-empty value");
                return 1;
            }
            workspace_id = Some(val.to_string());
        } else if arg == "--json" {
            if json_seen {
                eprintln!("Error: Duplicate option: --json");
                return 1;
            }
            json_seen = true;
            json_mode = true;
        } else {
            eprintln!("Error: Unknown option or unexpected argument provided");
            return 1;
        }
        i += 1;
    }

    let db_path = match db_path {
        Some(p) => p,
        None => {
            eprintln!("Error: Missing required option: --db");
            return 1;
        }
    };

    let account_id = match account_id {
        Some(a) => a,
        None => {
            eprintln!("Error: Missing required option: --account-id");
            return 1;
        }
    };

    let workspace_id = match workspace_id {
        Some(w) => w,
        None => {
            eprintln!("Error: Missing required option: --workspace-id");
            return 1;
        }
    };

    let scope = match OutboxScope::new(account_id, workspace_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: Invalid scope: {}", e);
            return 1;
        }
    };

    let path = Path::new(&db_path);
    match inspect_outbox_status(path, &scope) {
        Ok(report) => {
            if json_mode {
                match report.to_json() {
                    Ok(json) => {
                        println!("{}", json);
                        0
                    }
                    Err(e) => {
                        eprintln!("Error: Failed serializing report to JSON: {}", e);
                        1
                    }
                }
            } else {
                print!("{}", report.to_pretty_text());
                0
            }
        }
        Err(err) => {
            eprintln!("Error: {}", err);
            1
        }
    }
}
