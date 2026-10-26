//! `login`, `logout`, `unlock`, `whoami` and `workspace list/use`.
//!
//! `login` signs in through the daemon (`auth.sign_in`): the password comes
//! from the no-echo prompt, `--password-stdin` or `--password-file`, never
//! from argv. Several workspaces and none named: a numbered choice on a
//! terminal, otherwise exit 7 with the choices. After an expired sign-in,
//! `login` alone asks only for the password.

use serde_json::{json, Value};

use super::output::{CliError, Done};
use super::prompt::{self, PasswordSource};
use super::{Ctx, LoginArgs, WorkspaceCommand};
use crate::agent::session_store::SessionData;
use crate::agent::state::{SharedState, TrackerStatus, WorkspaceChoice};
use crate::ipc::client::Client;
use crate::ipc::{exit, method, ErrorCode, SignInParams, SignInResult, SIGN_IN_TIMEOUT};

/// The snapshot from the daemon.
pub fn snapshot(client: &mut Client) -> Result<SharedState, CliError> {
    let answer = client.call(method::STATE_GET, json!({}))?;
    serde_json::from_value(answer.result)
        .map_err(|e| CliError::failed(format!("the agent's state was not understood: {e}")))
}

/// `session.json`, read without changing anything (a damaged file is
/// reported as absent, never moved aside here).
pub fn local_session(ctx: &Ctx) -> Option<SessionData> {
    let path = ctx.paths.session_file();
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn choice_names(choices: &[WorkspaceChoice]) -> Vec<String> {
    choices
        .iter()
        .map(|c| match &c.role {
            Some(role) => format!("{} ({role})", c.name),
            None => c.name.clone(),
        })
        .collect()
}

fn choice_required(choices: &[WorkspaceChoice]) -> CliError {
    CliError::from(
        crate::ipc::ErrorBody::new(ErrorCode::ChoiceRequired, "Choose a workspace.")
            .with_hint("dosi-tracker workspace use <name>, or login --workspace <name>")
            .with_data(json!({ "choices": choices })),
    )
}

fn insecure_opt_in(ctx: &Ctx) -> Result<(), CliError> {
    let loaded = ctx.config();
    if loaded.config.allow_insecure_http {
        return Ok(());
    }
    if ctx.connect()?.is_some() {
        return Err(CliError::new(
            "precondition",
            exit::PRECONDITION,
            "allow_insecure_http is not set in the running agent's configuration",
        )
        .with_hint(
            "dosi-tracker config set allow_insecure_http true, then dosi-tracker service restart",
        ));
    }
    super::config::write_offline(ctx, |file, effective| {
        crate::agent::config::set_user_value(file, "allow_insecure_http", "true", effective)
            .map_err(super::config::config_error)
    })?;
    if !ctx.quiet {
        eprintln!(
            "Warning: allow_insecure_http = true was written to {}: passwords and tokens may cross the network unencrypted.",
            ctx.paths.config_file().display()
        );
    }
    Ok(())
}

pub fn login(ctx: &Ctx, args: LoginArgs) -> u8 {
    let out = ctx.out("auth.login");
    out.run(|| {
        if args.allow_insecure_http {
            insecure_opt_in(ctx)?;
        }
        let source = if args.password_stdin {
            PasswordSource::Stdin
        } else if let Some(file) = &args.password_file {
            PasswordSource::File(file.clone())
        } else {
            PasswordSource::Prompt
        };
        // Without a terminal every input must be given up front.
        if source == PasswordSource::Prompt && !prompt::interactive() {
            return Err(CliError::usage("no terminal to ask for the password")
                .with_hint("use --password-stdin or --password-file"));
        }
        let mut client = ctx.connect_or_start()?;
        let state = snapshot(&mut client)?;
        let expired = state.status == TrackerStatus::SignInExpired;
        let email = match args.email.clone() {
            Some(email) => email,
            None if expired || !state.display_name.is_empty() => state.display_name.clone(),
            None => prompt::ask("Email: ")
                .ok_or_else(|| CliError::usage("the email address is missing (--email)"))?,
        };
        if !state.display_name.is_empty()
            && !expired
            && state.status != TrackerStatus::SignedOut
        {
            if crate::agent::worker::owner_key(&state.display_name)
                == crate::agent::worker::owner_key(&email)
            {
                return Ok(Done::new(
                    json!({ "status": state.status, "account": state.display_name, "workspace": state.workspace }),
                    format!("Already signed in as {} ({}).", state.display_name, state.workspace),
                )
                .changed(Some(false)));
            }
            return Err(CliError::new(
                "precondition",
                exit::PRECONDITION,
                format!("Signed in as {}; sign out first.", state.display_name),
            )
            .with_hint("dosi-tracker logout"));
        }
        let mut server = args.server.clone();
        if server.is_none() && state.server_url.is_empty() && !expired {
            server = prompt::ask("Server address: ");
        }
        let password = prompt::read_password(&source, "Password: ")?;
        let params = SignInParams {
            server,
            email: email.clone(),
            password,
            workspace: args.workspace.clone(),
            remember_password: args.no_remember_password.then_some(false),
        };
        let value = serde_json::to_value(&params).unwrap_or(Value::Null);
        drop(params);
        let answer = client.call_with_timeout(
            method::AUTH_SIGN_IN,
            value,
            SIGN_IN_TIMEOUT + std::time::Duration::from_secs(5),
        )?;
        let result: SignInResult = serde_json::from_value(answer.result)
            .map_err(|e| CliError::failed(format!("the sign-in answer was not understood: {e}")))?;
        if result.workspace.is_some() {
            let workspace = result.workspace.clone().unwrap_or_default();
            return Ok(Done::new(
                json!({ "status": result.status, "account": email, "workspace": workspace }),
                format!("Signed in as {email} in {workspace}."),
            )
            .changed(answer.changed));
        }
        // Several workspaces: choose.
        let names = choice_names(&result.workspace_choices);
        let Some(index) = prompt::choose("Choose the workspace to track in:", &names) else {
            return Err(choice_required(&result.workspace_choices));
        };
        let chosen = client.call_with_timeout(
            method::WORKSPACE_CHOOSE,
            json!({ "index": index }),
            SIGN_IN_TIMEOUT,
        )?;
        let workspace = chosen.result["workspace"].as_str().unwrap_or_default().to_owned();
        Ok(Done::new(
            json!({ "status": "connecting", "account": email, "workspace": workspace }),
            format!("Signed in as {email} in {workspace}."),
        )
        .changed(Some(true)))
    })
}

pub fn logout(ctx: &Ctx, discard_unsent: bool, yes: bool) -> u8 {
    let out = ctx.out("auth.logout");
    out.run(|| {
        let Some(mut client) = ctx.connect()? else {
            if local_session(ctx).is_some_and(|s| s.account.is_some()) {
                // Saving the block and saying goodbye needs the agent.
                let mut client = ctx.connect_or_start()?;
                return sign_out(&mut client, discard_unsent, yes);
            }
            return Ok(
                Done::new(json!({ "status": "signed_out" }), "Not signed in.").changed(Some(false)),
            );
        };
        sign_out(&mut client, discard_unsent, yes)
    })
}

fn sign_out(client: &mut Client, discard_unsent: bool, yes: bool) -> Result<Done, CliError> {
    let state = snapshot(client)?;
    if discard_unsent && !yes {
        let question = format!(
            "Delete {} blocks that were not uploaded?",
            state.pending_uploads + state.rejected_uploads
        );
        if !prompt::interactive() {
            return Err(CliError::usage(
                "--discard-unsent deletes queued blocks; add --yes",
            ));
        }
        if !prompt::confirm(&question) {
            return Err(CliError::failed("Nothing was changed."));
        }
    }
    let answer = client.call_with_timeout(
        method::AUTH_SIGN_OUT,
        json!({ "at": chrono::Utc::now(), "discard_unsent": discard_unsent }),
        std::time::Duration::from_secs(30),
    )?;
    let text = if answer.changed == Some(false) {
        "Not signed in.".to_owned()
    } else if discard_unsent {
        "Signed out; blocks that were not uploaded were deleted.".to_owned()
    } else {
        "Signed out. Blocks not uploaded yet are kept for 14 days and upload when you sign in again."
            .to_owned()
    };
    Ok(Done::new(answer.result, text).changed(answer.changed))
}

pub fn unlock(ctx: &Ctx) -> u8 {
    let out = ctx.out("auth.unlock");
    out.run(|| {
        let mut client = ctx.connect_or_start()?;
        let answer = client.call_with_timeout(
            method::AUTH_UNLOCK_KEYRING,
            json!({}),
            std::time::Duration::from_secs(120),
        )?;
        Ok(Done::new(answer.result, "The keyring is unlocked.").changed(answer.changed))
    })
}

pub fn whoami(ctx: &Ctx) -> u8 {
    let out = ctx.out("auth.whoami");
    out.run(|| match ctx.connect()? {
        Some(mut client) => {
            let s = snapshot(&mut client)?;
            if s.display_name.is_empty() {
                return Err(CliError::from(crate::ipc::ErrorBody::new(
                    ErrorCode::NotSignedIn,
                    "You are not signed in.",
                ))
                .with_hint("dosi-tracker login"));
            }
            Ok(Done::new(
                json!({
                    "account": s.display_name, "workspace": s.workspace,
                    "server": s.server_url, "status": s.status, "source": "daemon",
                }),
                format!(
                    "{} in {} on {} ({})",
                    s.display_name,
                    s.workspace,
                    s.server_url,
                    s.status.label()
                ),
            ))
        }
        None => {
            let session = local_session(ctx).filter(|s| s.account.is_some()).ok_or_else(|| {
                CliError::from(crate::ipc::ErrorBody::new(
                    ErrorCode::NotSignedIn,
                    "You are not signed in.",
                ))
                .with_hint("dosi-tracker login")
            })?;
            let account = session.account.clone().unwrap_or_default();
            let workspace = session.workspace.as_ref().map(|w| w.name.clone()).unwrap_or_default();
            let server = session.server.clone().unwrap_or_default();
            Ok(Done::new(
                json!({ "account": account, "workspace": workspace, "server": server, "source": "local" }),
                format!("{account} in {workspace} on {server} (the agent is not running)"),
            ))
        }
    })
}

pub fn workspace(ctx: &Ctx, command: WorkspaceCommand) -> u8 {
    match command {
        WorkspaceCommand::List => ctx.out("workspace.list").run(|| {
            let mut client = ctx.connect_or_start()?;
            let answer = client.call(method::WORKSPACE_LIST, json!({}))?;
            let choices: Vec<WorkspaceChoice> =
                serde_json::from_value(answer.result["choices"].clone()).unwrap_or_default();
            let current = answer.result["current"].as_str().map(str::to_owned);
            let mut text = String::new();
            if let Some(current) = &current {
                text.push_str(&format!("* {current} (current)\n"));
            }
            for choice in choices.iter().filter(|c| !c.is_current) {
                text.push_str(&format!(
                    "  {}{}\n",
                    choice.name,
                    choice
                        .role
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                ));
            }
            if let Some(note) = answer.result["note"].as_str() {
                text.push_str(note);
            }
            Ok(Done::new(answer.result, text))
        }),
        WorkspaceCommand::Use { workspace } => ctx.out("workspace.use").run(|| {
            let mut client = ctx.connect_or_start()?;
            let answer = client.call_with_timeout(
                method::WORKSPACE_CHOOSE,
                json!({ "name": workspace, "at": chrono::Utc::now() }),
                SIGN_IN_TIMEOUT,
            )?;
            let name = answer.result["workspace"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let text = if answer.changed == Some(false) {
                format!("Already tracking in {name}.")
            } else {
                format!("Now tracking in {name}.")
            };
            Ok(Done::new(answer.result, text).changed(answer.changed))
        }),
    }
}
