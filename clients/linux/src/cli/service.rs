//! `service enable/disable/start/stop/restart/status/session-start`, `daemon`,
//! `uninstall` and `reset`.
//!
//! - `enable` writes the managed autostart entry (and, when no unit is
//!   loadable, the managed user unit); installers never do this.
//! - `start` uses the systemd user unit when there is one, else a detached
//!   daemon; `stop` uses systemd for a supervised daemon, else asks the
//!   daemon to shut down over IPC (block saved, farewell, uploads).
//! - `status` exits 3 when the agent is not running.
//! - `reset` signs out, deletes the keyring item and removes the config,
//!   state and data directories, unsent blocks included (counted first).
//! - `uninstall` runs the tarball's uninstaller, or says which package
//!   command removes Dosi Tracker (never runs sudo).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use super::output::{CliError, Done};
use super::prompt;
use super::{Ctx, ServiceCommand};
use crate::ipc::client::{socket_present, Client};
use crate::ipc::{exit, method};
use crate::service::{self as svc, ServiceEnv, Systemctl};

fn systemctl(ctx: &Ctx) -> Systemctl {
    Systemctl::new(ServiceEnv::from_vars(ctx.env.iter().cloned()))
}

fn exe() -> Result<PathBuf, CliError> {
    std::env::current_exe().map_err(|e| CliError::failed(format!("cannot find this program: {e}")))
}

fn service_error(e: svc::ServiceError) -> CliError {
    match e {
        svc::ServiceError::Unmanaged(path) => CliError::new(
            "precondition",
            exit::PRECONDITION,
            format!(
                "{} was not written by Dosi Tracker; it is left alone",
                path.display()
            ),
        ),
        other => CliError::failed(other.to_string()),
    }
}

/// Wait until the agent answers (at most 5 s).
fn wait_running(ctx: &Ctx) -> Option<Client> {
    let socket = ctx.socket()?;
    let deadline = Instant::now() + crate::ipc::client::START_WAIT;
    loop {
        if let Ok(client) = Client::connect(&socket, "cli") {
            return Some(client);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Wait until the agent's socket is gone (at most `budget`).
fn wait_stopped(ctx: &Ctx, budget: Duration) -> bool {
    let Some(socket) = ctx.socket() else {
        return true;
    };
    let deadline = Instant::now() + budget;
    loop {
        if !matches!(socket_present(&socket), Ok(true)) || Client::connect(&socket, "cli").is_err()
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn start(ctx: &Ctx) -> Result<Done, CliError> {
    if let Some(client) = ctx.connect()? {
        return Ok(Done::new(
            json!({ "running": true, "pid": client.hello().pid }),
            "The agent is already running.",
        )
        .changed(Some(false)));
    }
    let ctl = systemctl(ctx);
    let supervised = ctl.user_manager_available(None)
        && ctl.unit_state(None).is_ok_and(|s| s.loaded())
        && ctl.unit("start", true, None).is_ok();
    if !supervised {
        // The daemon refuses to start without a runtime dir: say so here.
        ctx.socket()
            .ok_or_else(|| CliError::environment("XDG_RUNTIME_DIR is not set"))?;
        crate::ipc::client::spawn_detached(&exe()?, &ctx.env)
            .map_err(|e| CliError::failed(format!("the agent could not be started: {e}")))?;
    }
    let client = wait_running(ctx)
        .ok_or_else(|| CliError::not_running("The agent did not start within 5 seconds."))?;
    Ok(Done::new(
        json!({ "running": true, "pid": client.hello().pid, "supervisor": if supervised { "systemd" } else { "none" } }),
        "The agent is running.",
    )
    .changed(Some(true)))
}

fn stop(ctx: &Ctx) -> Result<Done, CliError> {
    let Some(mut client) = ctx.connect()? else {
        return Ok(
            Done::new(json!({ "running": false }), "The agent is not running.")
                .changed(Some(false)),
        );
    };
    let pid = client.hello().pid;
    let ctl = systemctl(ctx);
    let supervised = ctl.user_manager_available(None)
        && ctl
            .unit_state(None)
            .is_ok_and(|s| s.active() && s.main_pid == Some(pid));
    if supervised {
        ctl.unit(
            "stop",
            false,
            Some(Instant::now() + Duration::from_secs(30)),
        )
        .map_err(|e| CliError::failed(e.to_string()))?;
    } else {
        client.call(method::DAEMON_SHUTDOWN, json!({ "at": chrono::Utc::now() }))?;
    }
    drop(client);
    if !wait_stopped(ctx, Duration::from_secs(25)) {
        return Err(CliError::failed(
            "The agent did not stop within 25 seconds.",
        ));
    }
    Ok(Done::new(json!({ "running": false }), "The agent stopped.").changed(Some(true)))
}

pub fn run(ctx: &Ctx, command: ServiceCommand) -> u8 {
    match command {
        ServiceCommand::Enable { now } => ctx.out("service.enable").run(|| {
            let exe = exe()?;
            let entry = svc::enable_autostart(&ctx.paths.autostart_entry(), &exe)
                .map_err(service_error)?;
            let ctl = systemctl(ctx);
            let unit = if ctl.user_manager_available(None) {
                let bindir = exe.parent().unwrap_or(Path::new("/usr/bin"));
                Some(format!("{:?}", svc::install_user_unit(&ctx.paths, bindir, &ctl).map_err(service_error)?))
            } else {
                None
            };
            let mut text = format!(
                "Dosi Tracker starts at every login ({}).",
                ctx.paths.autostart_entry().display()
            );
            if now {
                let started = start(ctx)?;
                text.push('\n');
                text.push_str(&started.text);
            }
            Ok(Done::new(
                json!({ "autostart": ctx.paths.autostart_entry(), "entry": format!("{entry:?}"), "unit": unit }),
                text,
            )
            .changed(Some(entry != svc::FileChange::Unchanged)))
        }),
        ServiceCommand::Disable { now } => ctx.out("service.disable").run(|| {
            let entry = svc::disable_autostart(&ctx.paths.autostart_entry()).map_err(service_error)?;
            let mut text = "Dosi Tracker no longer starts at login.".to_owned();
            if now {
                let stopped = stop(ctx)?;
                text.push('\n');
                text.push_str(&stopped.text);
            }
            Ok(Done::new(json!({ "entry": format!("{entry:?}") }), text)
                .changed(Some(entry == svc::FileChange::Removed)))
        }),
        ServiceCommand::Start => ctx.out("service.start").run(|| start(ctx)),
        ServiceCommand::Stop => ctx.out("service.stop").run(|| stop(ctx)),
        ServiceCommand::Restart => ctx.out("service.restart").run(|| {
            stop(ctx)?;
            start(ctx).map(|done| done.changed(Some(true)))
        }),
        ServiceCommand::Status => status(ctx),
        ServiceCommand::SessionStart => session_start(ctx),
    }
}

fn status(ctx: &Ctx) -> u8 {
    let out = ctx.out("service.status");
    let service = svc::status(&ctx.paths, &systemctl(ctx));
    let daemon = match ctx.connect() {
        Ok(client) => client.map(|c| c.hello().clone()),
        Err(e) => return out.failure(&e),
    };
    let result = json!({
        "user_manager": service.user_manager,
        "installed": service.installed,
        "enabled_at_login": service.enabled_at_login,
        "active": service.active,
        "unit_path": service.unit_path,
        "daemon": daemon,
    });
    let text = format!(
        "Unit installed:    {}\nStarts at login:   {}\nRunning:           {}",
        service.installed,
        service.enabled_at_login,
        daemon
            .as_ref()
            .map(|h| format!("yes (pid {}, version {})", h.pid, h.daemon_version))
            .unwrap_or_else(|| "no".into())
    );
    match daemon {
        Some(_) => out.success(None, result, || text),
        None => out.failure_with_result(
            &CliError::not_running("The Dosi Tracker agent is not running."),
            result,
            || text,
        ),
    }
}

fn session_start(ctx: &Ctx) -> u8 {
    ctx.out("service.session_start").run(|| {
        let session_vars = ctx
            .env
            .iter()
            .filter(|(k, _)| svc::SESSION_ENV_KEYS.contains(&k.as_str()))
            .cloned()
            .collect();
        let ctl = systemctl(ctx);
        let outcome = svc::session_start(&svc::SessionStartContext {
            paths: &ctx.paths,
            session_vars,
            exe: exe()?,
            systemctl: &ctl,
        })
        .map_err(|e| CliError::failed(e.to_string()))?;
        if ctx.config().config.gui.open_window_at_login {
            let gui = exe()?.with_file_name("dosi-tracker-gui");
            if gui.is_file() {
                let _ = std::process::Command::new(gui)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
        }
        Ok(Done::new(json!({ "outcome": format!("{outcome:?}") }), ""))
    })
}

/// How many blocks would be lost, as far as can be told.
fn unsent_blocks(ctx: &Ctx) -> u64 {
    if let Ok(Some(mut client)) = ctx.connect() {
        if let Ok(answer) = client.call(method::QUEUE_STATUS, json!({})) {
            return answer.result["pending"].as_u64().unwrap_or(0)
                + answer.result["parked"].as_u64().unwrap_or(0);
        }
    }
    super::tracking::local_facts(ctx)["queue"]["pending"]
        .as_u64()
        .unwrap_or(0)
}

/// Remove `dir` if it is a real directory (never a symlink's target).
fn remove_dir(dir: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::remove_file(dir)?;
            Ok(true)
        }
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir_all(dir)?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn reset(ctx: &Ctx, yes: bool) -> u8 {
    ctx.out("reset").run(|| reset_inner(ctx, yes))
}

fn reset_inner(ctx: &Ctx, yes: bool) -> Result<Done, CliError> {
    let unsent = unsent_blocks(ctx);
    if !yes {
        if !prompt::interactive() {
            return Err(CliError::usage(format!(
                "reset deletes everything Dosi Tracker keeps on this computer ({unsent} blocks not uploaded); add --yes"
            )));
        }
        eprintln!("This signs out and deletes Dosi Tracker's configuration, consent, queue ({unsent} blocks not uploaded) and logs.");
        if !prompt::confirm("Delete everything?") {
            return Err(CliError::failed("Nothing was deleted."));
        }
    }
    // Sign out (and stop) a running agent first.
    if let Some(mut client) = ctx.connect()? {
        let _ = client.call_with_timeout(
            method::AUTH_SIGN_OUT,
            json!({ "at": chrono::Utc::now(), "discard_unsent": true }),
            Duration::from_secs(30),
        );
        let _ = client.call(method::DAEMON_SHUTDOWN, json!({}));
        drop(client);
        wait_stopped(ctx, Duration::from_secs(25));
    }
    // The keyring item of the account that was signed in.
    if let Some(account) = super::account::local_session(ctx).and_then(|s| s.account) {
        let cfg = ctx.config().config;
        if let Ok(store) =
            crate::credentials::resolve_store(cfg.credentials.store, &ctx.paths, false)
        {
            if let Err(e) = store.clear_account(&account) {
                eprintln!("Warning: the keyring item could not be deleted: {e}");
            }
        }
    }
    let mut removed = Vec::new();
    for dir in [
        &ctx.paths.config_dir,
        &ctx.paths.state_dir,
        &ctx.paths.data_dir,
    ] {
        if remove_dir(dir).map_err(|e| CliError::failed(format!("{}: {e}", dir.display())))? {
            removed.push(dir.display().to_string());
        }
    }
    Ok(Done::new(
        json!({ "removed": removed, "unsent_deleted": unsent }),
        "Dosi Tracker's local data was deleted.",
    )
    .changed(Some(true)))
}

pub fn uninstall(ctx: &Ctx, purge: bool, yes: bool) -> u8 {
    ctx.out("uninstall").run(|| {
        let exe = exe()?;
        let prefix = exe
            .parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("/usr"));
        let script = prefix.join("lib/dosi-tracker/install/uninstall.sh");
        if purge {
            reset_inner(ctx, yes)?;
        }
        if script.is_file() && !exe.starts_with("/usr/bin") {
            // The uninstaller is a bash script (arrays, process substitution);
            // `sh` is dash on Debian and Ubuntu.
            // Its progress goes to stderr: stdout carries only this command's
            // result (one JSON document with --json).
            let status = std::process::Command::new("bash")
                .arg(&script)
                .args(if yes { vec!["--yes"] } else { Vec::new() })
                .stdout(std::process::Stdio::from(std::io::stderr()))
                .status()
                .map_err(|e| CliError::failed(format!("the uninstaller could not run: {e}")))?;
            if !status.success() {
                return Err(CliError::failed("the uninstaller failed"));
            }
            return Ok(
                Done::new(json!({ "uninstalled": true }), "Dosi Tracker was removed.")
                    .changed(Some(true)),
            );
        }
        let command = if Path::new("/etc/debian_version").exists() {
            "sudo apt remove dosi-tracker"
        } else {
            "sudo dnf remove dosi-tracker"
        };
        Ok(Done::new(
            json!({ "uninstalled": false, "command": command }),
            format!("Dosi Tracker was installed as a package; remove it with: {command}"),
        ))
    })
}
