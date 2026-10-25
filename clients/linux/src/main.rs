//! Executable CLI entrypoint for `dosi-tracker`.
//!
//! Provides diagnostic commands (`doctor [--json]`), credential management (`login [--json]`, `logout [--json]`, `status [--json]`),
//! and durable outbox inspection (`outbox status [--json]`).
//!
//! Invariants:
//! - Pure inspection: `status` and `doctor` never create files or directories.
//! - Strict redaction: passwords, access tokens, and signing key bytes are NEVER emitted to stdout, stderr, or JSON reports.
//! - Default-deny / fail-closed: missing credentials or databases result in deterministic non-zero exit codes.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use serde::Serialize;

use dosi_tracker_linux::api::{check_server_url, ApiClient};
use dosi_tracker_linux::credentials::{
    CredentialStore, Credentials, FileCredentialStore, SecretServiceStore,
};
use dosi_tracker_linux::diagnostics::{DoctorReport, SessionEnvironment};
use dosi_tracker_linux::outbox::{inspect_outbox_status, OutboxScope};
use dosi_tracker_linux::signing::generate_signing_key;

fn print_help() {
    println!("dosi-tracker {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Headless Linux client foundation, capability diagnostics, and transport for Dosi-Tracker."
    );
    println!();
    println!("USAGE:");
    println!("    dosi-tracker [COMMAND] [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    login        Authenticate with backend and securely store credentials");
    println!("    logout       Remove stored credentials for an account and workspace");
    println!(
        "    status       Inspect stored credentials, identity, token validity, and queue depth"
    );
    println!("    doctor       Inspect session environment, XDG storage paths, and capability diagnostics");
    println!("    outbox       Inspect durable offline outbox storage");
    println!("    help         Print this help message");
    println!();
    println!("OPTIONS:");
    println!("    --json       Output diagnostics, status, or login/logout results in structured JSON format");
    println!("    -h, --help   Print help information");
    println!("    -V, --version Print version information");
    println!();
    println!("INVARIANTS:");
    println!("    - Pure inspection: status and doctor never create files or directories.");
    println!("    - Zero credential leaks: passwords and tokens are never printed in plaintext.");
    println!("    - Unimplemented capture providers are reported honestly as NOT_IMPLEMENTED.");
    println!("    - Outbox inspection is strictly read-only and never migrates, checkpoints, or mutates DB.");
}

fn print_version() {
    println!("dosi-tracker {}", env!("CARGO_PKG_VERSION"));
}

fn print_login_help() {
    println!("dosi-tracker login");
    println!("Authenticate with backend and securely store credentials.");
    println!();
    println!("USAGE:");
    println!(
        "    dosi-tracker login --username <USER> --password <PASS> --workspace-id <WS> [OPTIONS]"
    );
    println!();
    println!("REQUIRED OPTIONS:");
    println!("    --username <USER>         Account username / email");
    println!("    --password <PASS>         Account password");
    println!("    --workspace-id <WS>       Workspace identifier");
    println!("    --server <URL>            Backend server URL (https://, or http:// to loopback; no default)");
    println!();
    println!("OPTIONS:");
    println!("    --credentials-file <PATH> Explicit file store path for isolated storage");
    println!("    --fallback-file           Force owner-only private file store fallback");
    println!(
        "    --json                    Output authentication result in structured JSON format"
    );
    println!("    -h, --help                Print this help message");
}

fn print_logout_help() {
    println!("dosi-tracker logout");
    println!("Remove stored credentials for specified account and workspace.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker logout --account-id <ID> [OPTIONS]");
    println!();
    println!("REQUIRED OPTIONS:");
    println!("    --account-id <ID>         Account identifier");
    println!();
    println!("OPTIONS:");
    println!("    --workspace-id <WS>       Workspace identifier (clears specific workspace if provided)");
    println!("    --credentials-file <PATH> Explicit file store path");
    println!("    --json                    Output logout result in structured JSON format");
    println!("    -h, --help                Print this help message");
}

fn print_status_help() {
    println!("dosi-tracker status");
    println!("Inspect stored credentials, identity, token validity, and queue depth.");
    println!();
    println!("USAGE:");
    println!("    dosi-tracker status --account-id <ID> --workspace-id <WS> [OPTIONS]");
    println!();
    println!("REQUIRED OPTIONS:");
    println!("    --account-id <ID>         Account identifier");
    println!("    --workspace-id <WS>       Workspace identifier");
    println!();
    println!("OPTIONS:");
    println!("    --db <PATH>               Path to outbox SQLite database to inspect queue depth");
    println!("    --credentials-file <PATH> Explicit file store path");
    println!("    --json                    Output status in structured JSON format");
    println!("    -h, --help                Print this help message");
    println!();
    println!("INVARIANTS:");
    println!("    - Pure inspection: never creates files or directories during status check.");
    println!("    - Fails closed if credentials or specified database are missing.");
    println!("    - Never prints raw passwords or tokens in output.");
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

fn resolve_credential_store(
    cred_file: Option<&str>,
    fallback_file: bool,
) -> Result<Box<dyn CredentialStore>, String> {
    if let Some(path_str) = cred_file {
        return Ok(Box::new(FileCredentialStore::new(PathBuf::from(path_str))));
    }
    if fallback_file {
        let path = FileCredentialStore::default_path().map_err(|e| e.to_string())?;
        return Ok(Box::new(FileCredentialStore::new(path)));
    }
    // Never a silent fallback to the file store: without an explicit file the
    // Secret Service is used, and a missing one fails the save visibly.
    Ok(Box::new(SecretServiceStore::new("dosi-tracker")))
}

fn sanitize_unknown_arg(arg: &str) -> String {
    if arg.starts_with('-') {
        let opt_name = arg.split('=').next().unwrap_or(arg);
        format!("Unknown option: {}", opt_name)
    } else {
        "Unexpected argument provided".to_string()
    }
}

#[derive(Serialize)]
struct LoginJsonReport {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_signing_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_expires_in_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct LogoutJsonReport {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cleared_all_workspaces: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn emit_login_error(
    json_mode: bool,
    err_msg: &str,
    account_id: Option<String>,
    workspace_id: Option<String>,
    server_url: Option<String>,
) -> ExitCode {
    if json_mode {
        let report = LoginJsonReport {
            success: false,
            account_id,
            workspace_id,
            server_url,
            store_backend: None,
            has_signing_key: None,
            token_expires_in_secs: None,
            error: Some(err_msg.to_string()),
        };
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error: Failed to format login JSON: {}", e),
        }
    }
    eprintln!("Error: {}", err_msg);
    ExitCode::from(1)
}

fn emit_logout_error(
    json_mode: bool,
    err_msg: &str,
    account_id: Option<String>,
    workspace_id: Option<String>,
) -> ExitCode {
    if json_mode {
        let report = LogoutJsonReport {
            success: false,
            account_id,
            workspace_id,
            cleared_all_workspaces: None,
            store_backend: None,
            error: Some(err_msg.to_string()),
        };
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error: Failed to format logout JSON: {}", e),
        }
    }
    eprintln!("Error: {}", err_msg);
    ExitCode::from(1)
}

fn handle_login(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_login_help();
        return ExitCode::SUCCESS;
    }

    let mut json_mode = args.iter().any(|a| a == "--json");
    let mut username: Option<String> = None;
    let mut password: Option<String> = None;
    let mut workspace_id: Option<String> = None;
    let mut server = String::new();
    let mut cred_file: Option<String> = None;
    let mut fallback_file = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--username" || arg == "-u" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                username = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--username=") {
            username = Some(val.trim().to_string());
        } else if arg == "--password" || arg == "-p" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                password = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--password=") {
            password = Some(val.trim().to_string());
        } else if arg == "--workspace-id" || arg == "-w" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                workspace_id = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--workspace-id=") {
            workspace_id = Some(val.trim().to_string());
        } else if arg == "--server" || arg == "-s" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                server = args[i].trim().to_string();
            }
        } else if let Some(val) = arg.strip_prefix("--server=") {
            server = val.trim().to_string();
        } else if arg == "--credentials-file" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                cred_file = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--credentials-file=") {
            cred_file = Some(val.trim().to_string());
        } else if arg == "--fallback-file" {
            fallback_file = true;
        } else if arg == "--json" {
            json_mode = true;
        } else {
            let err_msg = sanitize_unknown_arg(arg);
            return emit_login_error(json_mode, &err_msg, username, workspace_id, Some(server));
        }
        i += 1;
    }

    if server.is_empty() {
        return emit_login_error(
            json_mode,
            "Missing required option: --server (there is no default server)",
            username,
            workspace_id,
            None,
        );
    }
    if let Err(err) = check_server_url(&server, false) {
        return emit_login_error(json_mode, &err, username, workspace_id, Some(server));
    }

    let username = match username.filter(|s| !s.is_empty()) {
        Some(u) => u,
        None => {
            return emit_login_error(
                json_mode,
                "Missing required option: --username",
                None,
                workspace_id,
                Some(server),
            );
        }
    };

    let password = match password.filter(|s| !s.is_empty()) {
        Some(p) => p,
        None => {
            return emit_login_error(
                json_mode,
                "Missing required option: --password",
                Some(username),
                workspace_id,
                Some(server),
            );
        }
    };

    let workspace_id = match workspace_id.filter(|s| !s.is_empty()) {
        Some(w) => w,
        None => {
            return emit_login_error(
                json_mode,
                "Missing required option: --workspace-id",
                Some(username),
                None,
                Some(server),
            );
        }
    };

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            return emit_login_error(
                json_mode,
                &format!("Failed to initialize runtime: {}", e),
                Some(username),
                Some(workspace_id),
                Some(server),
            );
        }
    };

    let mut client = ApiClient::new(&server, username.clone(), password.clone())
        .with_workspace(Some(workspace_id.clone()));

    let login_result = rt.block_on(client.login());
    if let Err(err) = login_result {
        return emit_login_error(
            json_mode,
            &format!("Authentication failed: {}", err),
            Some(username),
            Some(workspace_id),
            Some(server),
        );
    }

    let signing_key = generate_signing_key();
    let store = match resolve_credential_store(cred_file.as_deref(), fallback_file) {
        Ok(s) => s,
        Err(e) => {
            return emit_login_error(
                json_mode,
                &format!("Failed to resolve credential store: {}", e),
                Some(username),
                Some(workspace_id),
                Some(server),
            );
        }
    };

    let store_kind = store.kind();
    let creds = Credentials {
        account_id: username.clone(),
        workspace_id: Some(workspace_id.clone()),
        workspace_name: None,
        password: Some(password),
        access_token: client.tokens().valid_access_token(),
        refresh_token: client.tokens().refresh_token(),
        server_url: Some(server.clone()),
        device_signing_key: Some(signing_key.to_vec()),
        device_id: None,
        access_token_expires_at: client.tokens().access_token_expires_at(),
    };

    if let Err(err) = store.save(&creds) {
        return emit_login_error(
            json_mode,
            &format!("Failed to securely store credentials: {}", err),
            Some(username),
            Some(workspace_id),
            Some(server),
        );
    }

    if json_mode {
        let now = Utc::now();
        let token_expires_in_secs = client
            .tokens()
            .access_token_expires_at()
            .map(|exp| (exp - now).num_seconds().max(0));
        let report = LoginJsonReport {
            success: true,
            account_id: Some(username),
            workspace_id: Some(workspace_id),
            server_url: Some(server),
            store_backend: Some(format!("{:?}", store_kind)),
            has_signing_key: Some(true),
            token_expires_in_secs,
            error: None,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{}", json);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: Failed to format login JSON: {}", e);
                ExitCode::from(1)
            }
        }
    } else {
        println!(
            "Successfully authenticated as '{}' in workspace '{}' (Server: {}, Store: {:?}). Signing key initialized.",
            username,
            workspace_id,
            server,
            store_kind
        );
        ExitCode::SUCCESS
    }
}

fn handle_logout(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_logout_help();
        return ExitCode::SUCCESS;
    }

    let mut json_mode = args.iter().any(|a| a == "--json");
    let mut account_id: Option<String> = None;
    let mut workspace_id: Option<String> = None;
    let mut cred_file: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--account-id" || arg == "-u" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                account_id = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--account-id=") {
            account_id = Some(val.trim().to_string());
        } else if arg == "--workspace-id" || arg == "-w" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                workspace_id = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--workspace-id=") {
            workspace_id = Some(val.trim().to_string());
        } else if arg == "--credentials-file" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                cred_file = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--credentials-file=") {
            cred_file = Some(val.trim().to_string());
        } else if arg == "--json" {
            json_mode = true;
        } else {
            let err_msg = sanitize_unknown_arg(arg);
            return emit_logout_error(json_mode, &err_msg, account_id, workspace_id);
        }
        i += 1;
    }

    let account_id = match account_id.filter(|s| !s.is_empty()) {
        Some(a) => a,
        None => {
            return emit_logout_error(
                json_mode,
                "Missing required option: --account-id",
                None,
                workspace_id,
            );
        }
    };

    let store = match resolve_credential_store(cred_file.as_deref(), false) {
        Ok(s) => s,
        Err(e) => {
            return emit_logout_error(
                json_mode,
                &format!("Failed to resolve credential store: {}", e),
                Some(account_id),
                workspace_id,
            );
        }
    };

    let store_kind = store.kind();

    if let Err(err) = store.clear(&account_id, workspace_id.as_deref()) {
        return emit_logout_error(
            json_mode,
            &format!("Failed to clear credentials: {}", err),
            Some(account_id),
            workspace_id,
        );
    }

    if json_mode {
        let cleared_all = workspace_id.is_none();
        let report = LogoutJsonReport {
            success: true,
            account_id: Some(account_id),
            workspace_id,
            cleared_all_workspaces: Some(cleared_all),
            store_backend: Some(format!("{:?}", store_kind)),
            error: None,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{}", json);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: Failed to format logout JSON: {}", e);
                ExitCode::from(1)
            }
        }
    } else {
        println!(
            "Successfully logged out account '{}' (workspace: {}).",
            account_id,
            workspace_id.as_deref().unwrap_or("all")
        );
        ExitCode::SUCCESS
    }
}

#[derive(Serialize)]
struct StatusJsonReport {
    account_id: String,
    workspace_id: String,
    server_url: Option<String>,
    store_backend: String,
    token_valid: bool,
    token_expires_in_secs: Option<i64>,
    has_signing_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_depth: Option<usize>,
}

fn handle_status(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_status_help();
        return ExitCode::SUCCESS;
    }

    let mut account_id: Option<String> = None;
    let mut workspace_id: Option<String> = None;
    let mut db_path: Option<String> = None;
    let mut cred_file: Option<String> = None;
    let mut json_mode = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--account-id" || arg == "-u" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                account_id = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--account-id=") {
            account_id = Some(val.trim().to_string());
        } else if arg == "--workspace-id" || arg == "-w" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                workspace_id = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--workspace-id=") {
            workspace_id = Some(val.trim().to_string());
        } else if arg == "--db" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                db_path = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--db=") {
            db_path = Some(val.trim().to_string());
        } else if arg == "--credentials-file" {
            i += 1;
            if i < args.len() && !args[i].starts_with("--") {
                cred_file = Some(args[i].trim().to_string());
            }
        } else if let Some(val) = arg.strip_prefix("--credentials-file=") {
            cred_file = Some(val.trim().to_string());
        } else if arg == "--json" {
            json_mode = true;
        } else {
            let err_msg = sanitize_unknown_arg(arg);
            eprintln!("Error: {}", err_msg);
            return ExitCode::from(1);
        }
        i += 1;
    }

    let account_id = match account_id.filter(|s| !s.is_empty()) {
        Some(a) => a,
        None => {
            eprintln!("Error: Missing required option: --account-id");
            return ExitCode::from(1);
        }
    };

    let workspace_id = match workspace_id.filter(|s| !s.is_empty()) {
        Some(w) => w,
        None => {
            eprintln!("Error: Missing required option: --workspace-id");
            return ExitCode::from(1);
        }
    };

    // Non-mutating inspection: if explicit cred_file was given, check existence
    if let Some(ref path_str) = cred_file {
        if !Path::new(path_str).exists() {
            eprintln!("Error: Credential file does not exist: {}", path_str);
            return ExitCode::from(1);
        }
    } else {
        // Default file check
        if let Ok(default_path) = FileCredentialStore::default_path() {
            if !default_path.exists() && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
                eprintln!(
                    "Error: No credentials found for account '{}' in workspace '{}'.",
                    account_id, workspace_id
                );
                return ExitCode::from(1);
            }
        }
    }

    let store = match resolve_credential_store(cred_file.as_deref(), false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: Failed to resolve credential store: {}", e);
            return ExitCode::from(1);
        }
    };

    let creds = match store.load(&account_id, Some(&workspace_id)) {
        Ok(Some(c)) => c,
        Ok(None) => {
            eprintln!(
                "Error: No credentials found for account '{}' in workspace '{}'.",
                account_id, workspace_id
            );
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("Error: Failed reading credential store: {}", e);
            return ExitCode::from(1);
        }
    };

    let now = Utc::now();
    let token_valid = creds.is_token_valid(now);
    let token_expires_in_secs = creds
        .access_token_expires_at
        .map(|exp| (exp - now).num_seconds().max(0));
    let has_signing_key = creds.device_signing_key.is_some();

    let mut queue_depth = None;
    if let Some(ref db) = db_path {
        let p = Path::new(db);
        if !p.exists() {
            eprintln!("Error: Outbox database file does not exist: {}", db);
            return ExitCode::from(1);
        }
        let scope = match OutboxScope::new(&account_id, &workspace_id) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error: Invalid scope: {}", e);
                return ExitCode::from(1);
            }
        };
        match inspect_outbox_status(p, &scope) {
            Ok(rep) => {
                queue_depth = Some(rep.durable_queue.queued_count as usize);
            }
            Err(e) => {
                eprintln!("Error: Failed inspecting outbox: {}", e);
                return ExitCode::from(1);
            }
        }
    }

    if json_mode {
        let report = StatusJsonReport {
            account_id: creds.account_id,
            workspace_id,
            server_url: creds.server_url,
            store_backend: format!("{:?}", store.kind()),
            token_valid,
            token_expires_in_secs,
            has_signing_key,
            queue_depth,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{}", json);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: Failed to format status JSON: {}", e);
                ExitCode::from(1)
            }
        }
    } else {
        println!("Account:       {}", creds.account_id);
        println!("Workspace:     {}", workspace_id);
        println!(
            "Server:        {}",
            creds.server_url.as_deref().unwrap_or("none")
        );
        println!("Store Backend: {:?}", store.kind());
        println!(
            "Token Status:  {}",
            if token_valid {
                format!("Valid (expires in {}s)", token_expires_in_secs.unwrap_or(0))
            } else {
                "Expired / None".to_string()
            }
        );
        println!(
            "Signing Key:   {}",
            if has_signing_key {
                "Present (HMAC-SHA256)"
            } else {
                "Absent"
            }
        );
        if let Some(depth) = queue_depth {
            println!("Queue Depth:   {} pending records", depth);
        }
        ExitCode::SUCCESS
    }
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
        eprintln!("Run 'dosi-tracker outbox --help' for usage.");
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
            eprintln!("Run 'dosi-tracker outbox --help' for available subcommands.");
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

    let path = Path::new(&db_path);
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
        "login" => handle_login(&args[1..]),
        "logout" => handle_logout(&args[1..]),
        "status" => handle_status(&args[1..]),
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
            eprintln!("Run 'dosi-tracker --help' for available commands.");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dosi_tracker_linux::credentials::{CredentialStoreKind, SecretServiceStore};

    #[test]
    fn test_secret_service_store_constructor_wiring_and_all_targets_guard() {
        // Direct constructor compilation and instantiation guard covering native binary and all-target builds
        // (Guards against regressions of CI run40076700975 / job120821470972)
        let store = SecretServiceStore::new("dosi-tracker");
        assert_eq!(store.application(), "dosi-tracker");
        assert_eq!(store.kind(), CredentialStoreKind::SecretService);
    }

    #[test]
    fn test_resolve_credential_store_explicit_path_and_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let custom_path = temp.path().join("explicit_creds.json");
        let custom_str = custom_path.to_str().unwrap();

        // Explicit credentials file always yields FileFallback pointing to custom path
        let store = resolve_credential_store(Some(custom_str), false).unwrap();
        assert_eq!(store.kind(), CredentialStoreKind::FileFallback);

        // Explicit fallback_file=true always yields FileFallback pointing to default path
        let fallback_store = resolve_credential_store(None, true).unwrap();
        assert_eq!(fallback_store.kind(), CredentialStoreKind::FileFallback);
    }

    #[test]
    fn test_resolve_credential_store_default_routing() {
        // Without an explicit credentials file or the fallback flag the Secret
        // Service is used, with or without a session bus: never a silent
        // fallback to the file store.
        let resolved = resolve_credential_store(None, false).unwrap();
        assert_eq!(resolved.kind(), CredentialStoreKind::SecretService);
    }
}
