//! `config show/path/get/set/unset` over the validated key allowlist.
//!
//! `show`, `path` and `get` read the layered configuration and never start
//! the agent. `set`/`unset` go through the running agent, or, with it down,
//! write the user file directly while holding `daemon.lock`; either way the
//! values pass the same validators (capture keys can only restrict what a
//! lower layer allows). The agent reads its configuration when it starts.

use serde_json::{json, Value};

use super::output::{CliError, Done};
use super::{ConfigCommand, Ctx};
use crate::agent::config::{AppConfig, ConfigError};
use crate::ipc::{exit, method};

pub fn config_error(e: ConfigError) -> CliError {
    match e {
        ConfigError::UnknownKey(_) | ConfigError::InvalidValue { .. } => {
            CliError::usage(e.to_string())
        }
        ConfigError::UserFileUnusable(_) => CliError::environment(e.to_string()),
        ConfigError::Io(_) => CliError::failed(e.to_string()),
    }
}

/// Run `write` while holding `daemon.lock`, so no agent starts mid-write.
/// Without `XDG_RUNTIME_DIR` no agent can run at all, so no lock is needed.
pub fn with_offline_lock<T>(
    ctx: &Ctx,
    write: impl FnOnce() -> Result<T, CliError>,
) -> Result<T, CliError> {
    let Some(socket) = ctx.socket() else {
        return write();
    };
    let lock_path = crate::agent::daemon::lock_path(&socket);
    match crate::ipc::server::DaemonLock::try_acquire(&lock_path) {
        Ok(Some(lock)) => {
            let result = write();
            drop(lock);
            result
        }
        Ok(None) => Err(CliError::new(
            "busy",
            exit::FAILED,
            "the agent is starting or running; try again",
        )),
        Err(e) => Err(CliError::environment(format!(
            "cannot take the agent's lock: {e}"
        ))),
    }
}

/// `write(user_file, effective)` under the offline lock.
pub fn write_offline<T>(
    ctx: &Ctx,
    write: impl FnOnce(&std::path::Path, &AppConfig) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let effective = ctx.config().config;
    with_offline_lock(ctx, || write(&ctx.paths.config_file(), &effective))
}

fn toml_to_json(value: Option<toml::Value>) -> Value {
    value
        .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
        .unwrap_or(Value::Null)
}

pub fn run(ctx: &Ctx, command: ConfigCommand) -> u8 {
    match command {
        ConfigCommand::Show => ctx.out("config.show").run(|| {
            let loaded = ctx.config();
            let issues: Vec<String> = loaded.issues.iter().map(ToString::to_string).collect();
            let mut text = loaded.config.to_toml();
            for issue in &issues {
                text.push_str(&format!("\n# warning: {issue}"));
            }
            Ok(Done::new(
                json!({
                    "config": serde_json::to_value(&loaded.config).unwrap_or(Value::Null),
                    "issues": issues,
                    "files": {
                        "system": ctx.paths.system_config_file(),
                        "user": ctx.paths.config_file(),
                    },
                }),
                text,
            ))
        }),
        ConfigCommand::Path => ctx.out("config.path").run(|| {
            Ok(Done::new(
                json!({
                    "user": ctx.paths.config_file(),
                    "system": ctx.paths.system_config_file(),
                    "consent": ctx.paths.consent_file,
                    "state": ctx.paths.state_dir,
                    "data": ctx.paths.data_dir,
                    "logs": ctx.paths.logs_dir(),
                    "socket": ctx.socket(),
                }),
                ctx.paths.config_file().display().to_string(),
            ))
        }),
        ConfigCommand::Get { key } => ctx.out("config.get").run(|| {
            if !crate::agent::config::keys().any(|k| k == key) {
                return Err(CliError::usage(format!("unknown configuration key `{key}`")));
            }
            let value = ctx.config().config.value(&key);
            let text = match &value {
                Some(toml::Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => "(not set)".to_owned(),
            };
            Ok(Done::new(json!({ "key": key, "value": toml_to_json(value) }), text))
        }),
        ConfigCommand::Set { key, value } => ctx.out("config.set").run(|| {
            match ctx.connect()? {
                Some(mut client) => {
                    let answer =
                        client.call(method::CONFIG_SET, json!({ "key": key, "value": value }))?;
                    Ok(Done::new(
                        answer.result,
                        format!("{key} saved; restart the agent to apply it (dosi-tracker service restart)."),
                    )
                    .changed(answer.changed))
                }
                None => {
                    write_offline(ctx, |file, effective| {
                        crate::agent::config::set_user_value(file, &key, &value, effective)
                            .map_err(config_error)
                    })?;
                    Ok(Done::new(json!({ "key": key, "source": "local" }), format!("{key} saved."))
                        .changed(Some(true)))
                }
            }
        }),
        ConfigCommand::Unset { key } => ctx.out("config.unset").run(|| match ctx.connect()? {
            Some(mut client) => {
                let answer = client.call(method::CONFIG_UNSET, json!({ "key": key }))?;
                Ok(Done::new(answer.result, format!("{key} removed from your configuration."))
                    .changed(answer.changed))
            }
            None => {
                let removed = write_offline(ctx, |file, _| {
                    crate::agent::config::unset_user_value(file, &key).map_err(config_error)
                })?;
                Ok(Done::new(
                    json!({ "key": key, "removed": removed, "source": "local" }),
                    if removed {
                        format!("{key} removed from your configuration.")
                    } else {
                        format!("{key} was not set in your configuration.")
                    },
                )
                .changed(Some(removed)))
            }
        }),
    }
}
