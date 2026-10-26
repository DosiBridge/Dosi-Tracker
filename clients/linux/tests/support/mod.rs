//! Shared helpers for the integration tests (`mod support;` in a test file).
//!
//! Ground rules (design 18.1): tests never touch the developer's display, bus,
//! keyring, camera or real user data. Child processes get an explicit
//! environment from [`xdg`]; D-Bus tests use private buses from
//! [`private_bus`] and the fakes in [`fakes`]; display tests run only inside
//! the harness started by `ci/xvfb-session.sh` ([`xvfb`]).

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

#[cfg(unix)]
pub mod agent;
pub mod fake_backend;
pub mod fake_daemon;
pub mod fakes;
pub mod private_bus;
pub mod schema;
pub mod xdg;
pub mod xvfb;
