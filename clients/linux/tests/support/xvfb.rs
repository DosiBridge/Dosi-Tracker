//! Access to the harness session started by `ci/xvfb-session.sh`.
//!
//! Display tests are `#[ignore]` and begin with
//! `let Some(harness) = support::xvfb::require_harness() else { return };`.
//! Outside the harness they skip, so a developer's own display is never used.
//! The harness addresses are returned explicitly: code under test gets them
//! as arguments or in a child's explicit environment, never by inheriting
//! `DISPLAY` or the bus variables.

/// The harness session: its Xvfb display and its two private buses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Harness {
    /// The Xvfb display, e.g. `:99`.
    pub display: String,
    /// Address of the private session bus.
    pub session_bus: String,
    /// Address of the private bus standing in for the system bus.
    pub system_bus: String,
}

/// The harness, or `None` (with a note on stderr) when this test does not run
/// inside `ci/xvfb-session.sh`.
pub fn require_harness() -> Option<Harness> {
    let harness = harness_from(|name| std::env::var(name).ok());
    if harness.is_none() {
        eprintln!("skipped: needs the harness session (bash ci/xvfb-session.sh cargo test ...)");
    }
    harness
}

/// [`require_harness`] over an explicit variable lookup.
///
/// Requires `DOSI_TEST_HARNESS=1`, non-empty `DOSI_TEST_DISPLAY`,
/// `DOSI_TEST_SESSION_BUS` and `DOSI_TEST_SYSTEM_BUS`, and `DISPLAY` equal to
/// the harness display (the script sets both), so a stray `DOSI_TEST_HARNESS`
/// on a workstation cannot point a test at the developer's own display.
pub fn harness_from(lookup: impl Fn(&str) -> Option<String>) -> Option<Harness> {
    let non_empty = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
    if lookup("DOSI_TEST_HARNESS").as_deref() != Some("1") {
        return None;
    }
    let display = non_empty("DOSI_TEST_DISPLAY")?;
    if lookup("DISPLAY").as_deref() != Some(display.as_str()) {
        return None;
    }
    Some(Harness {
        display,
        session_bus: non_empty("DOSI_TEST_SESSION_BUS")?,
        system_bus: non_empty("DOSI_TEST_SYSTEM_BUS")?,
    })
}
