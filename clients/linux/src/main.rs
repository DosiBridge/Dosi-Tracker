//! Executable CLI entrypoint for dosi-tracker-linux.
//!
//! Provides diagnostic commands (`doctor [--json]`) and help/version flags
//! without creating files, accessing credentials, opening network sockets,
//! or spawning background daemons.

use std::env;
use std::process::ExitCode;

use dosi_tracker_linux::diagnostics::{DoctorReport, SessionEnvironment};

fn print_help() {
    println!("dosi-tracker-linux {}", env!("CARGO_PKG_VERSION"));
    println!("Headless Linux client foundation and capability diagnostics for Dosi-Tracker.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker-linux [COMMAND] [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    doctor       Inspect session environment, XDG storage paths, and capability diagnostics");
    println!("    help         Print this help message");
    println!();
    println!("OPTIONS:");
    println!(
        "    --json       Output diagnostics in structured JSON format (usable with 'doctor')"
    );
    println!("    -h, --help   Print help information");
    println!("    -V, --version Print version information");
    println!();
    println!("INVARIANTS:");
    println!("    - Pure inspection: does not create files or directories during diagnostics.");
    println!("    - Zero credentials, zero network calls, zero background daemons.");
    println!("    - Unimplemented capture providers are reported honestly as NOT_IMPLEMENTED.");
}

fn print_version() {
    println!("dosi-tracker-linux {}", env!("CARGO_PKG_VERSION"));
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

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }

    let first = &args[0];
    match first.as_str() {
        "doctor" => handle_doctor(&args[1..]),
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
