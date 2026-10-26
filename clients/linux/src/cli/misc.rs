//! `device show`, `logs`, `open`, `completions`, `gui` and `version`.

use std::io::{BufRead, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use super::output::{CliError, Done};
use super::{Cli, Ctx, OpenTarget};
use crate::ipc::exit;

/// `device show`: the device id (read, never created here), the computer's
/// name, the OS, the agent's version and the time zone.
pub fn device_show(ctx: &Ctx) -> u8 {
    ctx.out("device.show").run(|| {
        let file = ctx.paths.device_file();
        let device_id = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| {
                serde_json::from_str::<crate::tracking::system::DeviceFile>(&text).ok()
            })
            .map(|f| f.device_id.to_string());
        let (node, release) = crate::tracking::system::uname().unzip();
        let name = node
            .as_deref()
            .and_then(crate::tracking::system::device_name_from);
        let os = crate::tracking::system::os_version_from(Path::new("/"), release.as_deref());
        let zone = crate::tracking::system::time_zone();
        let text =
            format!(
            "Device id:   {}\nComputer:    {}\nSystem:      {os}\nAgent:       {}\nTime zone:   {}",
            device_id.as_deref().unwrap_or("(created when the agent first starts)"),
            name.as_deref().unwrap_or("?"),
            env!("CARGO_PKG_VERSION"),
            zone.as_deref().unwrap_or("?"),
        );
        Ok(Done::new(
            json!({
                "device_id": device_id,
                "device_name": name,
                "os_version": os,
                "agent_version": env!("CARGO_PKG_VERSION"),
                "platform": "linux",
                "time_zone": zone,
            }),
            text,
        ))
    })
}

/// The newest log file in `dir`.
fn newest_log(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("dosi-tracker"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort();
    files.pop().map(|(_, path)| path)
}

/// The last `n` lines of `path`.
fn tail(path: &Path, n: usize) -> std::io::Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let mut lines: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    for line in std::io::BufReader::new(file).lines() {
        lines.push_back(line?);
        if lines.len() > n {
            lines.pop_front();
        }
    }
    Ok(lines.into())
}

pub fn logs(ctx: &Ctx, follow: bool, lines: usize) -> u8 {
    let out = ctx.out("logs");
    let dir = ctx.paths.logs_dir();
    let Some(file) = newest_log(&dir) else {
        return out.failure(
            &CliError::new(
                "not_found",
                exit::CHOICE,
                format!("no log files in {}", dir.display()),
            )
            .with_hint(
                "under systemd the agent logs to the journal: journalctl --user -u dosi-tracker",
            ),
        );
    };
    let shown = match tail(&file, lines) {
        Ok(shown) => shown,
        Err(e) => return out.failure(&CliError::failed(format!("{}: {e}", file.display()))),
    };
    let print = |line: &str| {
        if ctx.json {
            println!(
                "{}",
                json!({ "schema": crate::ipc::CLI_SCHEMA, "command": "logs", "line": line })
            );
        } else {
            println!("{line}");
        }
    };
    for line in &shown {
        print(line);
    }
    if !follow {
        return exit::OK;
    }
    let Ok(mut reader) = std::fs::File::open(&file).map(std::io::BufReader::new) else {
        return exit::FAILED;
    };
    let _ = reader.seek(std::io::SeekFrom::End(0));
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => std::thread::sleep(Duration::from_millis(500)),
            Ok(_) => print(line.trim_end()),
            Err(_) => return exit::FAILED,
        }
    }
}

fn xdg_open(target: &str) -> Result<(), CliError> {
    std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| CliError::failed(format!("xdg-open could not run: {e}")))
}

pub fn open(ctx: &Ctx, target: OpenTarget) -> u8 {
    ctx.out("open").run(|| {
        let what = match target {
            OpenTarget::Dashboard => ctx.config().config.dashboard_url.ok_or_else(|| {
                CliError::new(
                    "not_found",
                    exit::CHOICE,
                    "no dashboard address is configured",
                )
                .with_hint("dosi-tracker config set dashboard_url https://...")
            })?,
            OpenTarget::Logs => ctx.paths.logs_dir().display().to_string(),
            OpenTarget::Data => ctx.paths.data_dir.display().to_string(),
        };
        xdg_open(&what)?;
        Ok(Done::new(
            json!({ "opened": what }),
            format!("Opened {what}"),
        ))
    })
}

pub fn gui(ctx: &Ctx) -> u8 {
    ctx.out("gui").run(|| {
        let exe = std::env::current_exe()
            .map_err(|e| CliError::failed(e.to_string()))?
            .with_file_name("dosi-tracker-gui");
        if !exe.is_file() {
            return Err(CliError::new(
                "not_found",
                exit::CHOICE,
                "the Dosi Tracker window is not installed",
            ));
        }
        std::process::Command::new(&exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| CliError::failed(format!("the window could not start: {e}")))?;
        Ok(Done::new(json!({ "started": true }), ""))
    })
}

pub fn completions(shell: clap_complete::Shell) -> u8 {
    use clap::CommandFactory;
    let mut command = Cli::command();
    clap_complete::generate(shell, &mut command, "dosi-tracker", &mut std::io::stdout());
    exit::OK
}
