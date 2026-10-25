//! The `dosi-tracker` command line: clap definitions, dispatch to the daemon
//! over IPC, offline fallbacks and the stable exit codes.

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
