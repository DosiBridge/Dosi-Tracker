//! `doctor [--json] [--strict]`: non-mutating diagnostics with live probes.
//!
//! Never creates files, never starts the agent (it only reads its `hello`),
//! never activates a D-Bus service. `--json` prints the report itself (the
//! v1 fields plus the v2 `schema`, `probes` and `problems`), so existing
//! consumers keep parsing it. `--strict` exits 1 when a problem blocks
//! tracking.

use super::Ctx;
use crate::diagnostics::{DoctorInputs, DoctorReport, SessionEnvironment};
use crate::ipc::exit;

pub fn run(ctx: &Ctx, strict: bool) -> u8 {
    let env = SessionEnvironment::from_env();
    let loaded = ctx.config();
    let store = loaded.config.credentials.store.as_str().to_owned();
    let probes = crate::diagnostics::live::run(&crate::diagnostics::live::ProbeContext {
        env: &env,
        vars: &ctx.env,
        paths: &ctx.paths,
        socket: ctx.socket(),
        credentials_store: store.clone(),
        config_issues: loaded.issues.iter().map(ToString::to_string).collect(),
    });
    let report = DoctorReport::generate_with(
        &env,
        &DoctorInputs {
            credentials_store: Some(store),
            probes: Some(probes),
        },
    );
    if ctx.json {
        match report.to_json() {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("Error serializing diagnostic report to JSON: {e}");
                return exit::FAILED;
            }
        }
    } else {
        print!("{}", report.to_pretty_text());
    }
    if strict && report.has_blocking_problems() {
        exit::FAILED
    } else {
        exit::OK
    }
}
