//! CLI output: human text, the `--json` envelope (`dosi-tracker.cli/1`) and
//! the stable exit-code table.
//!
//! With `--json` exactly one JSON document goes to stdout:
//! `{"schema":"dosi-tracker.cli/1","command":"tracking.pause","ok":true,"changed":true,"result":{...}}`
//! or `{"schema":...,"command":...,"ok":false,"error":{"code","message","hint","exit_code"}}`.
//! Streams (`status --watch`, `logs -f`) print JSON Lines instead. Prompts,
//! warnings and human errors go to stderr. Nothing printed here ever holds a
//! secret: results come from the snapshot and local files, which carry none.

use serde_json::{json, Value};

use crate::ipc::client::ClientError;
use crate::ipc::{exit, ErrorBody, ErrorCode, CLI_SCHEMA};

/// A failed command: what to print and how to exit.
#[derive(Debug, Clone, PartialEq)]
pub struct CliError {
    /// An IPC error code, or `daemon_not_running` / `usage`.
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
    pub exit_code: u8,
    pub data: Option<Value>,
}

impl CliError {
    pub fn new(code: &str, exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            exit_code,
            data: None,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest.as_str(), exit::USAGE, message)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal.as_str(), exit::FAILED, message)
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::PermissionDenied.as_str(),
            exit::ENVIRONMENT,
            message,
        )
    }

    pub fn not_running(message: impl Into<String>) -> Self {
        Self::new("daemon_not_running", exit::DAEMON, message)
            .with_hint("dosi-tracker service start")
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl From<ErrorBody> for CliError {
    fn from(error: ErrorBody) -> Self {
        Self {
            code: error.code.as_str().into(),
            exit_code: error.code.exit_code(),
            message: error.message,
            hint: error.hint,
            data: error.data,
        }
    }
}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::NotRunning(why) => {
                CliError::not_running(format!("The Dosi Tracker agent is not running ({why})."))
            }
            ClientError::Unsafe(why) => {
                CliError::environment(format!("Refusing to talk to the agent's socket: {why}."))
            }
            ClientError::Version(error) => {
                let mut cli = CliError::from(error);
                cli.exit_code = exit::DAEMON;
                cli.hint
                    .get_or_insert_with(|| "restart with dosi-tracker service restart".into());
                cli
            }
            ClientError::Daemon(error) => CliError::from(error),
            ClientError::Timeout => CliError::new(
                ErrorCode::Timeout.as_str(),
                exit::FAILED,
                "The agent did not answer in time.",
            ),
            ClientError::Protocol(why) => {
                CliError::failed(format!("Talking to the agent failed: {why}."))
            }
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// How results are printed.
#[derive(Debug, Clone, Copy)]
pub struct Output {
    pub json: bool,
    pub quiet: bool,
    /// The command's dotted name in the envelope (`tracking.pause`).
    pub command: &'static str,
}

impl Output {
    pub fn new(json: bool, quiet: bool, command: &'static str) -> Self {
        Self {
            json,
            quiet,
            command,
        }
    }

    /// The success envelope.
    pub fn envelope(&self, changed: Option<bool>, result: &Value) -> Value {
        let mut envelope = json!({
            "schema": CLI_SCHEMA,
            "command": self.command,
            "ok": true,
            "result": result,
        });
        if let Some(changed) = changed {
            envelope["changed"] = json!(changed);
        }
        envelope
    }

    /// The failure envelope.
    pub fn error_envelope(&self, error: &CliError) -> Value {
        let mut body = json!({
            "code": error.code,
            "message": error.message,
            "hint": error.hint,
            "exit_code": error.exit_code,
        });
        if let Some(data) = &error.data {
            body["data"] = data.clone();
        }
        json!({
            "schema": CLI_SCHEMA,
            "command": self.command,
            "ok": false,
            "error": body,
        })
    }

    /// Print a success: the envelope with `--json`, else `human` (unless `-q`).
    pub fn success(
        &self,
        changed: Option<bool>,
        result: Value,
        human: impl FnOnce() -> String,
    ) -> u8 {
        if self.json {
            println!("{}", self.envelope(changed, &result));
        } else if !self.quiet {
            let text = human();
            if !text.is_empty() {
                println!("{}", text.trim_end());
            }
        }
        exit::OK
    }

    /// Print a failure and return its exit code.
    pub fn failure(&self, error: &CliError) -> u8 {
        if self.json {
            println!("{}", self.error_envelope(error));
        } else {
            eprintln!("Error: {}", error.message);
            if let Some(hint) = &error.hint {
                eprintln!("Hint: {hint}");
            }
        }
        error.exit_code
    }

    /// Print a failure that still has a result to show (`status` with the
    /// daemon down: local facts, exit 3).
    pub fn failure_with_result(
        &self,
        error: &CliError,
        result: Value,
        human: impl FnOnce() -> String,
    ) -> u8 {
        if self.json {
            let mut envelope = self.error_envelope(error);
            envelope["result"] = result;
            println!("{envelope}");
        } else {
            let text = human();
            if !text.is_empty() {
                println!("{}", text.trim_end());
            }
            eprintln!("Error: {}", error.message);
            if let Some(hint) = &error.hint {
                eprintln!("Hint: {hint}");
            }
        }
        error.exit_code
    }

    /// A warning on stderr (never in the JSON document).
    pub fn warn(&self, text: &str) {
        if !self.quiet {
            eprintln!("Warning: {text}");
        }
    }

    /// Run `f` and print its outcome.
    pub fn run(&self, f: impl FnOnce() -> Result<Done, CliError>) -> u8 {
        match f() {
            Ok(done) => self.success(done.changed, done.result, || done.text),
            Err(error) => self.failure(&error),
        }
    }
}

/// A finished command: its result, the human text, and whether it changed anything.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub result: Value,
    pub text: String,
    pub changed: Option<bool>,
}

impl Done {
    pub fn new(result: Value, text: impl Into<String>) -> Self {
        Self {
            result,
            text: text.into(),
            changed: None,
        }
    }

    pub fn changed(mut self, changed: Option<bool>) -> Self {
        self.changed = changed;
        self
    }
}

/// A duration as `1h 05m` / `12m` / `40s`.
pub fn human_duration(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_carry_the_schema_command_and_exit_code() {
        let out = Output::new(true, false, "tracking.pause");
        let ok = out.envelope(Some(true), &json!({"status": "paused"}));
        assert_eq!(ok["schema"], "dosi-tracker.cli/1");
        assert_eq!(ok["command"], "tracking.pause");
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["changed"], true);
        let query = out.envelope(None, &json!({}));
        assert!(query.get("changed").is_none());

        let error = CliError::from(ErrorBody::new(ErrorCode::ConsentRequired, "m").with_hint("h"));
        assert_eq!(error.exit_code, 5);
        let failed = out.error_envelope(&error);
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["error"]["code"], "consent_required");
        assert_eq!(failed["error"]["exit_code"], 5);
        assert_eq!(failed["error"]["hint"], "h");
    }

    #[test]
    fn client_errors_get_their_exit_classes() {
        let code = |e: ClientError| CliError::from(e).exit_code;
        assert_eq!(code(ClientError::NotRunning("x".into())), 3);
        assert_eq!(code(ClientError::Unsafe("x".into())), 9);
        assert_eq!(code(ClientError::Timeout), 1);
        assert_eq!(
            code(ClientError::Version(ErrorBody::new(
                ErrorCode::UnsupportedVersion,
                "v"
            ))),
            3
        );
        assert_eq!(
            code(ClientError::Daemon(ErrorBody::new(ErrorCode::Offline, "o"))),
            6
        );
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(human_duration(3_900.0), "1h 05m");
        assert_eq!(human_duration(720.0), "12m");
        assert_eq!(human_duration(40.0), "40s");
        assert_eq!(human_duration(-3.0), "0s");
    }
}
