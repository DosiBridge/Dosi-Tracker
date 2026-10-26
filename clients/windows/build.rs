//! Build script: stamps the version into the program.
//!
//! - `DOSI_AGENT_VERSION` (compile-time env for the crate): the
//!   `agentVersion` sent with every block and heartbeat, shown in the window,
//!   and in the HTTP user agent — `Cargo.toml`'s version (or CI's
//!   `DOSI_VERSION`) plus the git commit when CI provides one; see
//!   `build-support/version.rs`.
//! - On Windows, a version resource (what Explorer's Properties → Details,
//!   Defender and support tools read) and the application manifest
//!   (`build-support/dosi-tracker.exe.manifest`), compiled with the Windows
//!   SDK's `rc.exe` (`RC_PATH` overrides where it is found).

#[path = "build-support/version.rs"]
mod version;

use std::env;
use std::path::PathBuf;

const ENV_INPUTS: [&str; 4] = ["DOSI_VERSION", "DOSI_GIT_SHA", "GITHUB_SHA", "DOSI_BUILD_NUMBER"];
const MANIFEST: &str = "build-support/dosi-tracker.exe.manifest";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build-support/version.rs");
    println!("cargo:rerun-if-changed={MANIFEST}");
    for name in ENV_INPUTS {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let input = |name: &str| env::var(name).ok();
    let cargo_version = input("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let release = version::release_version(&cargo_version, input("DOSI_VERSION").as_deref()).unwrap_or_else(|e| fail("DOSI_VERSION", &e));
    let sha_input = input("DOSI_GIT_SHA").filter(|s| !s.trim().is_empty()).or_else(|| input("GITHUB_SHA"));
    let sha = version::short_sha(sha_input.as_deref()).unwrap_or_else(|e| fail("DOSI_GIT_SHA / GITHUB_SHA", &e));
    let agent = version::agent_version(&release, sha.as_deref()).unwrap_or_else(|e| fail("the agent version", &e));
    let build = version::build_number(input("DOSI_BUILD_NUMBER").as_deref()).unwrap_or_else(|e| fail("DOSI_BUILD_NUMBER", &e));
    let core = version::numeric_core(&release).unwrap_or_else(|e| fail("the release version", &e));

    println!("cargo:rustc-env=DOSI_AGENT_VERSION={agent}");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resources(&agent, core, build);
    }
}

/// The version resource and the manifest, linked into every executable of
/// the crate (the agent and its test harness alike).
fn embed_resources(agent: &str, core: [u16; 3], build: u16) {
    let [major, minor, patch] = core;
    let numeric = format!("{major}.{minor}.{patch}.{build}");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));

    let template = std::fs::read_to_string(MANIFEST).unwrap_or_else(|e| fail(MANIFEST, &e.to_string()));
    let manifest = out_dir.join("dosi-tracker.exe.manifest");
    std::fs::write(&manifest, template.replace("@VERSION@", &numeric)).unwrap_or_else(|e| fail("the manifest", &e.to_string()));

    let packed = version::packed(core, build);
    let mut res = winresource::WindowsResource::new();
    res.set("ProductName", "Dosi Tracker")
        // What Task Manager and Windows' dialogs call the program.
        .set("FileDescription", "Dosi Tracker")
        .set("CompanyName", "DosiBridge")
        .set("LegalCopyright", "Copyright (c) DosiBridge. MIT License.")
        .set("OriginalFilename", "dosi-tracker.exe")
        .set("InternalName", "dosi-tracker")
        .set("FileVersion", agent)
        .set("ProductVersion", agent)
        .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed)
        .set_manifest_file(manifest.to_str().unwrap_or_else(|| fail("OUT_DIR", "not valid UTF-8")));
    if let Err(e) = res.compile() {
        fail(
            "the Windows resources",
            &format!("{e} (rc.exe comes with the Windows SDK; set RC_PATH to point at it)"),
        );
    }
}

fn fail(what: &str, why: &str) -> ! {
    eprintln!("error: {what}: {why}");
    std::process::exit(1);
}
