//! Executable CLI entrypoint for dosi-tracker-linux.
//!
//! Provides diagnostic commands (`doctor [--json]`) and help/version flags
//! without creating files, accessing credentials, opening network sockets,
//! or spawning background daemons.

use std::env;
use std::process::ExitCode;

use dosi_tracker_linux::diagnostics::{DoctorReport, SessionEnvironment};
use dosi_tracker_linux::outbox::{inspect_outbox_status, OutboxScope};

fn print_help() {
    println!("dosi-tracker-linux {}", env!("CARGO_PKG_VERSION"));
    println!("Headless Linux client foundation and capability diagnostics for Dosi-Tracker.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker-linux [COMMAND] [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    doctor       Inspect session environment, XDG storage paths, and capability diagnostics");
    println!("    outbox       Inspect durable offline outbox storage");
    println!("    help         Print this help message");
    println!();
    println!("OPTIONS:");
    println!(
        "    --json       Output diagnostics in structured JSON format (usable with 'doctor' or 'outbox status')"
    );
    println!("    -h, --help   Print help information");
    println!("    -V, --version Print version information");
    println!();
    println!("INVARIANTS:");
    println!("    - Pure inspection: does not create files or directories during diagnostics.");
    println!("    - Zero credentials, zero network calls, zero background daemons.");
    println!("    - Unimplemented capture providers are reported honestly as NOT_IMPLEMENTED.");
    println!("    - Outbox inspection is strictly read-only and never migrates, checkpoints, or mutates DB.");
}

fn print_version() {
    println!("dosi-tracker-linux {}", env!("CARGO_PKG_VERSION"));
}

fn print_outbox_help() {
    println!("dosi-tracker-linux outbox");
    println!("Inspect durable offline outbox storage.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker-linux outbox <SUBCOMMAND> [OPTIONS]");
    println!();
    println!("SUBCOMMANDS:");
    println!("    status       Read-only inspection of queued records for an exact account and workspace scope");
    println!("    help         Print this help message");
    println!();
    println!("Run 'dosi-tracker-linux outbox status --help' for options.");
}

fn print_outbox_status_help() {
    println!("dosi-tracker-linux outbox status");
    println!("Read-only inspection of durable outbox queue for a specific scope.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker-linux outbox status --db <PATH> --account-id <ID> --workspace-id <ID> [OPTIONS]");
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

fn handle_doctor(args: &[String]) -> ExitCode {
    let json_mode = args.iter().any(|arg| arg == "--json");
    let env = SessionEnvironment::from_env();
    let report = DoctorReport::generate(&env);

    if json_mode {
        match report.to_json() {
            Ok(json) => {
                println!("{}", json);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error serializing diagnostic report to JSON: {}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        print!("{}", report.to_pretty_text());
        ExitCode::SUCCESS
    }
}

fn handle_outbox(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Error: Missing outbox subcommand. Available: status");
        eprintln!("Run 'dosi-tracker-linux outbox --help' for usage.");
        return ExitCode::from(1);
    }

    match args[0].as_str() {
        "status" => handle_outbox_status(&args[1..]),
        "help" | "--help" | "-h" => {
            print_outbox_help();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Error: Unknown outbox subcommand: {}", other);
            eprintln!("Run 'dosi-tracker-linux outbox --help' for available subcommands.");
            ExitCode::from(1)
        }
    }
}

fn handle_outbox_status(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_outbox_status_help();
        return ExitCode::SUCCESS;
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
                return ExitCode::from(1);
            }
            i += 1;
            if i >= args.len() || args[i].starts_with("--") {
                eprintln!("Error: Option '--db' requires a non-empty value");
                return ExitCode::from(1);
            }
            let val = args[i].trim();
            if val.is_empty() {
                eprintln!("Error: Option '--db' requires a non-empty value");
                return ExitCode::from(1);
            }
            db_path = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--db=") {
            if db_path.is_some() {
                eprintln!("Error: Duplicate option: --db");
                return ExitCode::from(1);
            }
            let val = val.trim();
            if val.is_empty() {
                eprintln!("Error: Option '--db' requires a non-empty value");
                return ExitCode::from(1);
            }
            db_path = Some(val.to_string());
        } else if arg == "--account-id" {
            if account_id.is_some() {
                eprintln!("Error: Duplicate option: --account-id");
                return ExitCode::from(1);
            }
            i += 1;
            if i >= args.len() || args[i].starts_with("--") {
                eprintln!("Error: Option '--account-id' requires a non-empty value");
                return ExitCode::from(1);
            }
            let val = args[i].trim();
            if val.is_empty() {
                eprintln!("Error: Option '--account-id' requires a non-empty value");
                return ExitCode::from(1);
            }
            account_id = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--account-id=") {
            if account_id.is_some() {
                eprintln!("Error: Duplicate option: --account-id");
                return ExitCode::from(1);
            }
            let val = val.trim();
            if val.is_empty() {
                eprintln!("Error: Option '--account-id' requires a non-empty value");
                return ExitCode::from(1);
            }
            account_id = Some(val.to_string());
        } else if arg == "--workspace-id" {
            if workspace_id.is_some() {
                eprintln!("Error: Duplicate option: --workspace-id");
                return ExitCode::from(1);
            }
            i += 1;
            if i >= args.len() || args[i].starts_with("--") {
                eprintln!("Error: Option '--workspace-id' requires a non-empty value");
                return ExitCode::from(1);
            }
            let val = args[i].trim();
            if val.is_empty() {
                eprintln!("Error: Option '--workspace-id' requires a non-empty value");
                return ExitCode::from(1);
            }
            workspace_id = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--workspace-id=") {
            if workspace_id.is_some() {
                eprintln!("Error: Duplicate option: --workspace-id");
                return ExitCode::from(1);
            }
            let val = val.trim();
            if val.is_empty() {
                eprintln!("Error: Option '--workspace-id' requires a non-empty value");
                return ExitCode::from(1);
            }
            workspace_id = Some(val.to_string());
        } else if arg == "--json" {
            if json_seen {
                eprintln!("Error: Duplicate option: --json");
                return ExitCode::from(1);
            }
            json_seen = true;
            json_mode = true;
        } else {
            eprintln!("Error: Unknown option or unexpected argument provided");
            return ExitCode::from(1);
        }
        i += 1;
    }

    let db_path = match db_path {
        Some(p) => p,
        None => {
            eprintln!("Error: Missing required option: --db");
            return ExitCode::from(1);
        }
    };

    let account_id = match account_id {
        Some(a) => a,
        None => {
            eprintln!("Error: Missing required option: --account-id");
            return ExitCode::from(1);
        }
    };

    let workspace_id = match workspace_id {
        Some(w) => w,
        None => {
            eprintln!("Error: Missing required option: --workspace-id");
            return ExitCode::from(1);
        }
    };

    let scope = match OutboxScope::new(account_id, workspace_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: Invalid scope: {}", e);
            return ExitCode::from(1);
        }
    };

    let path = std::path::Path::new(&db_path);
    match inspect_outbox_status(path, &scope) {
        Ok(report) => {
            if json_mode {
                match report.to_json() {
                    Ok(json) => {
                        println!("{}", json);
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("Error: Failed serializing report to JSON: {}", e);
                        ExitCode::from(1)
                    }
                }
            } else {
                print!("{}", report.to_pretty_text());
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!("Error: {}", err);
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }

    let first = &args[0];
    match first.as_str() {
        "doctor" => handle_doctor(&args[1..]),
        "outbox" => handle_outbox(&args[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        "version" | "--version" | "-V" => {
            print_version();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Unknown command or option: {}", other);
            eprintln!("Run 'dosi-tracker-linux --help' for available commands.");
            ExitCode::from(1)
        }
    }
}
