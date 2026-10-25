//! `dosi-tracker-gui`: the desktop window, a thin client of the
//! `dosi-tracker` daemon.
//!
//! Placeholder until the GUI lands: `--version` works (packaging and smoke
//! tests rely on it), everything else explains that this build has no GUI.

use std::process::ExitCode;

fn main() -> ExitCode {
    let first = std::env::args().nth(1);
    match first.as_deref() {
        Some("--version") | Some("-V") => {
            println!("dosi-tracker-gui {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "dosi-tracker-gui: GUI not available in this build; use the dosi-tracker command."
            );
            ExitCode::from(1)
        }
    }
}
