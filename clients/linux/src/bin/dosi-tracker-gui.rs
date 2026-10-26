//! `dosi-tracker-gui`: the desktop window, a thin client of the
//! `dosi-tracker` daemon (see `dosi_tracker_linux::gui`).
//!
//! `--version` works everywhere (packaging and smoke tests rely on it); the
//! window itself refuses root, needs a login session's runtime directory and
//! a graphical display, and ends without stopping the agent.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(dosi_tracker_linux::gui::run(&args))
}

/// The GUI is Linux-only; elsewhere the binary only answers `--version`.
#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version") | Some("-V") => {
            println!("dosi-tracker-gui {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("dosi-tracker-gui: the window is only available on Linux");
            ExitCode::from(1)
        }
    }
}
