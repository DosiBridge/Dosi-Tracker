//! `dosi-tracker`: the command line and the `daemon` subcommand. Everything
//! lives in the library ([`dosi_tracker_linux::cli`]); this is the thin entry
//! point that hands it the arguments and returns its exit code.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    ExitCode::from(dosi_tracker_linux::cli::run(std::env::args_os().collect()))
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("dosi-tracker runs on Linux only");
    ExitCode::from(9)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use dosi_tracker_linux::credentials::{
        CredentialStore, CredentialStoreKind, SecretServiceStore,
    };

    #[test]
    fn test_secret_service_store_constructor_wiring_and_all_targets_guard() {
        // Direct constructor compilation and instantiation guard covering native binary and all-target builds
        // (Guards against regressions of CI run40076700975 / job120821470972)
        let store = SecretServiceStore::new("dosi-tracker");
        assert_eq!(store.application(), "dosi-tracker");
        assert_eq!(store.kind(), CredentialStoreKind::SecretService);
    }
}
