//! The `dosi-tracker` command line: clap definitions, dispatch to the daemon
//! over IPC, offline fallbacks and the stable exit codes (design 11).
//!
//! - Passwords on argv are refused before parsing ([`prompt::reject_password_argv`]).
//! - The hidden legacy `outbox status` inspector keeps its exact output and
//!   exit codes ([`queue::legacy_outbox`]).
//! - Daemon-backed commands talk IPC; mutating ones start the daemon when it
//!   is not running (unless `--no-start` / `DOSI_NO_AUTOSTART=1`, exit 3).
//! - Read-only commands (`status`, `doctor`, `queue status`, `consent show`,
//!   `config show/get/path`) never start it and fall back to a local,
//!   read-only view that creates nothing.
//! - Offline writes (`consent grant/revoke`, `config set/unset`, `queue
//!   purge`) go through the same validators while holding `daemon.lock`.

pub mod account;
pub mod config;
pub mod consent;
pub mod doctor;
pub mod misc;
pub mod output;
pub mod prompt;
pub mod queue;
pub mod service;
pub mod tracking;

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::agent::config::{AppConfig, ConfigSources, LoadedConfig};
use crate::ipc::client::{connect_or_start, Client, ClientError, StartOptions};
use crate::ipc::exit;
use crate::paths::LinuxPaths;
use output::{CliError, Output};

const ABOUT: &str = "Per-user activity agent for Dosi-Tracker: daemon, command line and status.";

const HELP_TEMPLATE: &str = "\
{name} {version}
{about}

USAGE:
    {usage}

{all-args}{after-help}";

const AFTER_HELP: &str = "\
INVARIANTS:
    - Never runs as root; never reads keystrokes, input devices or the clipboard.
    - Records only what you consented to (dosi-tracker consent show).
    - status, doctor and queue status never create files or start the agent.
    - Passwords are never accepted on the command line.";

#[derive(Debug, Parser)]
#[command(
    name = "dosi-tracker",
    version,
    about = ABOUT,
    help_template = HELP_TEMPLATE,
    after_help = AFTER_HELP,
    disable_help_subcommand = true,
    arg_required_else_help = false
)]
pub struct Cli {
    /// Print exactly one JSON document (JSON Lines for streams).
    #[arg(long, global = true)]
    pub json: bool,
    /// The agent's socket (tests).
    #[arg(
        long,
        global = true,
        hide = true,
        env = "DOSI_SOCKET",
        value_name = "PATH"
    )]
    pub socket: Option<PathBuf>,
    /// Never start the agent (also DOSI_NO_AUTOSTART=1).
    #[arg(long, global = true)]
    pub no_start: bool,
    /// Print nothing on success.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// More detail.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// No colours.
    #[arg(long, global = true)]
    pub no_color: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start at login, start/stop/restart the agent, show its state.
    #[command(subcommand)]
    Service(ServiceCommand),
    /// Run the agent (the service's ExecStart).
    Daemon(DaemonArgs),
    /// Remove Dosi Tracker (tarball install) or say how (packages).
    Uninstall {
        /// Also delete your data (see reset).
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Sign out and delete all local data, unsent blocks included.
    Reset {
        #[arg(long)]
        yes: bool,
    },
    /// Sign in.
    Login(LoginArgs),
    /// Sign out (the block in progress is saved and uploaded first).
    Logout {
        /// Delete blocks that were not uploaded yet.
        #[arg(long)]
        discard_unsent: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Unlock the keyring (shows the keyring's own prompt).
    Unlock,
    /// Who is signed in, where.
    Whoami,
    /// List or choose workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    /// List or choose projects.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// What you are working on (sent with each block).
    #[command(subcommand)]
    Task(TaskCommand),
    /// Start tracking (or say what is missing).
    Start,
    /// Pause until resumed (the agent keeps running).
    Stop,
    /// Pause, optionally for a while.
    Pause {
        /// e.g. 15m, 30m, 1h, 1h30m.
        #[arg(long = "for", value_name = "DURATION")]
        for_: Option<String>,
    },
    /// Take a break: pause and resume by itself.
    Break {
        /// e.g. 15m, 30m, 1h.
        duration: String,
    },
    /// Resume tracking.
    Resume,
    /// Save the block in progress and upload now.
    Sync,
    /// Keep or discard the time you were away.
    #[command(subcommand)]
    Away(AwayCommand),
    /// Show the agent's state.
    Status {
        /// Stream every change (JSON Lines with --json).
        #[arg(long)]
        watch: bool,
    },
    /// What may be recorded on this computer.
    #[command(subcommand)]
    Consent(ConsentCommand),
    /// Every capability with its state and why it is off.
    Capabilities,
    /// Screenshot permission (Wayland portal).
    #[command(subcommand)]
    Capture(CaptureCommand),
    /// This device's identity.
    #[command(subcommand)]
    Device(DeviceCommand),
    /// Blocks waiting to upload.
    #[command(subcommand)]
    Queue(QueueCommand),
    /// The configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Check this computer (never changes anything).
    Doctor {
        /// Exit 1 when something blocks tracking.
        #[arg(long)]
        strict: bool,
    },
    /// Show the agent's log.
    Logs {
        /// Keep printing new lines.
        #[arg(short, long)]
        follow: bool,
        /// How many lines.
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
    },
    /// Open the web dashboard, the logs or the data folder.
    Open { target: OpenTarget },
    /// Open the Dosi Tracker window.
    Gui,
    /// Shell completions.
    Completions { shell: clap_complete::Shell },
    /// Print the version.
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OpenTarget {
    Dashboard,
    Logs,
    Data,
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Start the agent at every login (writes the autostart entry).
    Enable {
        /// Also start it now.
        #[arg(long)]
        now: bool,
    },
    /// Stop starting it at login.
    Disable {
        /// Also stop it now.
        #[arg(long)]
        now: bool,
    },
    Start,
    Stop,
    Restart,
    Status,
    /// The login autostart hook.
    #[command(hide = true)]
    SessionStart,
}

#[derive(Debug, Clone, Default, Args)]
pub struct DaemonArgs {
    /// Started by the systemd user unit.
    #[arg(long)]
    pub systemd: bool,
    /// Without a supervisor (detach from the terminal).
    #[arg(long)]
    pub detach: bool,
    /// error, warn, info, debug or trace.
    #[arg(long, value_name = "LEVEL")]
    pub log_level: Option<String>,
}

#[derive(Debug, Clone, Default, Args)]
pub struct LoginArgs {
    /// The server (https://..., no default).
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,
    #[arg(long, value_name = "EMAIL")]
    pub email: Option<String>,
    /// Name or id of the workspace to track in.
    #[arg(long, value_name = "NAME_OR_ID")]
    pub workspace: Option<String>,
    /// Read the password from the first line of stdin.
    #[arg(long, conflicts_with = "password_file")]
    pub password_stdin: bool,
    /// Read the password from a private (0600/0400) file.
    #[arg(long, value_name = "PATH")]
    pub password_file: Option<PathBuf>,
    /// Do not keep the password in the keyring.
    #[arg(long)]
    pub no_remember_password: bool,
    /// Allow a plain http:// server that is not on this computer (writes
    /// allow_insecure_http = true to your configuration).
    #[arg(long)]
    pub allow_insecure_http: bool,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    List,
    /// Track in another workspace.
    Use {
        workspace: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    List {
        /// Ask the server again now.
        #[arg(long)]
        refresh: bool,
    },
    /// Track on a project (id or title).
    Use { project: String },
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    Show,
    Set {
        /// At most 200 characters.
        #[arg(num_args = 1.., trailing_var_arg = true)]
        text: Vec<String>,
    },
    Clear,
}

#[derive(Debug, Subcommand)]
pub enum AwayCommand {
    /// Keep the time away as work.
    Keep,
    /// Discard it.
    Discard,
}

#[derive(Debug, Subcommand)]
pub enum ConsentCommand {
    /// Per capability: your choice, the workspace policy, the configuration,
    /// what this desktop can do, and what is in effect.
    Show,
    /// Choose what may be recorded (replaces the earlier choice).
    Grant(GrantArgs),
    /// Withdraw consent (works without the agent).
    Revoke {
        /// Also delete blocks that were not uploaded.
        #[arg(long)]
        purge_queue: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Clone, Default, Args)]
pub struct GrantArgs {
    /// Everything this version can record (never the webcam).
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub keyboard: bool,
    #[arg(long)]
    pub mouse: bool,
    #[arg(long)]
    pub active_window: bool,
    #[arg(long)]
    pub running_programs: bool,
    #[arg(long)]
    pub screenshots: bool,
    /// Blur screenshots (default).
    #[arg(long, conflicts_with = "no_blur")]
    pub blur: bool,
    #[arg(long)]
    pub no_blur: bool,
    /// Only time blocks, nothing else.
    #[arg(long, conflicts_with_all = ["all", "keyboard", "mouse", "active_window", "running_programs", "screenshots"])]
    pub time_only: bool,
    /// Not available on Linux in this version.
    #[arg(long, hide = true)]
    pub webcam: bool,
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum CaptureCommand {
    /// Ask the desktop for screenshot permission again.
    Retry,
}

#[derive(Debug, Subcommand)]
pub enum DeviceCommand {
    Show,
}

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// Counts (works without the agent; never changes anything).
    Status,
    /// The newest rows (never their contents).
    List {
        /// Only blocks the server did not accept.
        #[arg(long)]
        parked: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Delete queued blocks.
    Purge {
        #[arg(long, group = "which")]
        pending: bool,
        #[arg(long, group = "which")]
        parked: bool,
        #[arg(long, group = "which")]
        all: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show,
    Path,
    Get { key: String },
    Set { key: String, value: String },
    Unset { key: String },
}

/// What every command gets.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub json: bool,
    pub quiet: bool,
    pub verbose: u8,
    pub no_start: bool,
    pub socket_override: Option<PathBuf>,
    pub paths: LinuxPaths,
    pub env: Vec<(String, String)>,
}

impl Ctx {
    pub fn out(&self, command: &'static str) -> Output {
        Output::new(self.json, self.quiet, command)
    }

    /// The agent's socket, if this session has a runtime directory.
    pub fn socket(&self) -> Option<PathBuf> {
        self.socket_override
            .clone()
            .filter(|p| p.is_absolute())
            .or_else(|| self.paths.socket_path())
    }

    /// The effective configuration (read-only).
    pub fn config(&self) -> LoadedConfig {
        AppConfig::load(&ConfigSources::for_paths(
            &self.paths,
            self.env.iter().cloned(),
        ))
    }

    /// The running agent, never starting it; `None` when it is not running.
    pub fn connect(&self) -> Result<Option<Client>, CliError> {
        let Some(socket) = self.socket() else {
            return Ok(None);
        };
        match Client::connect(&socket, "cli") {
            Ok(client) => Ok(Some(client)),
            Err(ClientError::NotRunning(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The running agent, starting it when allowed.
    pub fn connect_or_start(&self) -> Result<Client, CliError> {
        let socket = self.socket().ok_or_else(|| {
            CliError::environment(
                "XDG_RUNTIME_DIR is not set: there is no login session to run the agent in",
            )
        })?;
        let options = StartOptions {
            no_start: self.no_start || StartOptions::no_autostart_env(&self.env),
            exe: std::env::current_exe()
                .map_err(|e| CliError::failed(format!("cannot find this program: {e}")))?,
            client: "cli",
            env: self.env.clone(),
        };
        let (client, _) = connect_or_start(&socket, &options).map_err(|e| match e {
            ClientError::NotRunning(why) if options.no_start => CliError::not_running(format!(
                "The Dosi Tracker agent is not running ({why}) and --no-start was given."
            )),
            other => CliError::from(other),
        })?;
        Ok(client)
    }

    /// The env value of `name`.
    pub fn var(&self, name: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn unknown_command(args: &[OsString]) -> Option<String> {
    let commands = [
        "service",
        "daemon",
        "uninstall",
        "reset",
        "login",
        "logout",
        "unlock",
        "whoami",
        "workspace",
        "project",
        "task",
        "start",
        "stop",
        "pause",
        "break",
        "resume",
        "sync",
        "away",
        "status",
        "consent",
        "capabilities",
        "capture",
        "device",
        "queue",
        "config",
        "doctor",
        "logs",
        "open",
        "gui",
        "completions",
        "version",
        "help",
    ];
    let first = args
        .iter()
        .skip(1)
        .filter_map(|a| a.to_str())
        .find(|a| !a.starts_with('-'))?;
    (!commands.contains(&first)).then(|| first.to_owned())
}

fn wants_json(args: &[OsString]) -> bool {
    args.iter().any(|a| a == "--json")
}

/// Run the command line `args` (with the program name first); returns the
/// exit code.
pub fn run(args: Vec<OsString>) -> u8 {
    if let Some(error) = prompt::reject_password_argv(&args) {
        return Output::new(wants_json(&args), false, "usage").failure(&error);
    }
    if args.get(1).is_some_and(|a| a == "outbox") {
        let rest: Vec<String> = args
            .iter()
            .skip(2)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        return queue::legacy_outbox(&rest);
    }
    if args.len() <= 1 || (args.len() == 2 && args[1] == "help") {
        use clap::CommandFactory;
        let mut command = Cli::command();
        let _ = command.print_help();
        println!();
        return exit::OK;
    }
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    let _ = e.print();
                    return if e.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                        exit::USAGE
                    } else {
                        exit::OK
                    };
                }
                ErrorKind::InvalidSubcommand => {
                    let name = unknown_command(&args).unwrap_or_default();
                    let error = CliError::usage(format!("Unknown command or option: {name}"))
                        .with_hint("run 'dosi-tracker --help' for available commands");
                    return Output::new(wants_json(&args), false, "usage").failure(&error);
                }
                _ => {
                    if wants_json(&args) {
                        let error = CliError::usage(e.kind().to_string());
                        return Output::new(true, false, "usage").failure(&error);
                    }
                    let _ = e.print();
                    return exit::USAGE;
                }
            }
        }
    };
    let env: Vec<(String, String)> = std::env::vars().collect();
    let ctx = Ctx {
        json: cli.json,
        quiet: cli.quiet,
        verbose: cli.verbose,
        no_start: cli.no_start,
        socket_override: cli.socket.clone(),
        paths: LinuxPaths::from_env(),
        env,
    };
    // Root refusal (invariant I1): only help, version and doctor run as root.
    // SAFETY: geteuid has no preconditions and cannot fail.
    let root = unsafe { libc::geteuid() } == 0;
    let Some(command) = cli.command else {
        use clap::CommandFactory;
        let _ = Cli::command().print_help();
        return exit::OK;
    };
    if root && !matches!(command, Commands::Doctor { .. } | Commands::Version) {
        return ctx.out("usage").failure(&CliError::environment(
            "dosi-tracker never runs as root; run it as the signed-in user",
        ));
    }
    dispatch(&ctx, command)
}

fn dispatch(ctx: &Ctx, command: Commands) -> u8 {
    match command {
        Commands::Service(sub) => service::run(ctx, sub),
        Commands::Daemon(args) => crate::agent::daemon::run(crate::agent::daemon::DaemonArgs {
            systemd: args.systemd,
            detach: args.detach,
            log_level: args.log_level,
            socket: ctx.socket_override.clone(),
        }),
        Commands::Uninstall { purge, yes } => service::uninstall(ctx, purge, yes),
        Commands::Reset { yes } => service::reset(ctx, yes),
        Commands::Login(args) => account::login(ctx, args),
        Commands::Logout {
            discard_unsent,
            yes,
        } => account::logout(ctx, discard_unsent, yes),
        Commands::Unlock => account::unlock(ctx),
        Commands::Whoami => account::whoami(ctx),
        Commands::Workspace(sub) => account::workspace(ctx, sub),
        Commands::Project(sub) => tracking::project(ctx, sub),
        Commands::Task(sub) => tracking::task(ctx, sub),
        Commands::Start => tracking::start(ctx),
        Commands::Stop => tracking::pause(ctx, None, "tracking.stop"),
        Commands::Pause { for_ } => match for_.as_deref().map(tracking::parse_duration).transpose()
        {
            Ok(duration) => tracking::pause(ctx, duration, "tracking.pause"),
            Err(e) => ctx.out("tracking.pause").failure(&e),
        },
        Commands::Break { duration } => match tracking::parse_duration(&duration) {
            Ok(duration) => tracking::pause(ctx, Some(duration), "tracking.break"),
            Err(e) => ctx.out("tracking.break").failure(&e),
        },
        Commands::Resume => tracking::resume(ctx),
        Commands::Sync => tracking::sync(ctx),
        Commands::Away(sub) => tracking::away(ctx, matches!(sub, AwayCommand::Keep)),
        Commands::Status { watch } => tracking::status(ctx, watch),
        Commands::Consent(sub) => consent::run(ctx, sub),
        Commands::Capabilities => consent::capabilities(ctx),
        Commands::Capture(CaptureCommand::Retry) => consent::capture_retry(ctx),
        Commands::Device(DeviceCommand::Show) => misc::device_show(ctx),
        Commands::Queue(sub) => queue::run(ctx, sub),
        Commands::Config(sub) => config::run(ctx, sub),
        Commands::Doctor { strict } => doctor::run(ctx, strict),
        Commands::Logs { follow, lines } => misc::logs(ctx, follow, lines),
        Commands::Open { target } => misc::open(ctx, target),
        Commands::Gui => misc::gui(ctx),
        Commands::Completions { shell } => misc::completions(shell),
        Commands::Version => {
            println!("dosi-tracker {}", env!("CARGO_PKG_VERSION"));
            exit::OK
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn unknown_commands_are_named() {
        let args: Vec<OsString> = ["dosi-tracker", "--json", "start-tracking"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(unknown_command(&args).as_deref(), Some("start-tracking"));
        let known: Vec<OsString> = ["dosi-tracker", "status"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(unknown_command(&known), None);
    }

    #[test]
    fn no_short_option_is_p() {
        // prompt::reject_password_argv relies on it.
        fn walk(command: &clap::Command) {
            for arg in command.get_arguments() {
                assert_ne!(arg.get_short(), Some('p'), "{}", arg.get_id());
            }
            for sub in command.get_subcommands() {
                walk(sub);
            }
        }
        walk(&Cli::command());
    }
}
