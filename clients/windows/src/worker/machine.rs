//! What the worker needs from the machine, and the real one ([`Tracker`]).

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::config::CapturePermissions;
use crate::model::Activity;
use crate::power;
use crate::tracking::system::DeviceInfo;
use crate::tracking::{BlockSpec, Tracker};

use super::blocks;

/// What the worker needs from the machine — the time, the user's input,
/// the screen, the device — behind thin functions, so the rules above them
/// are tested without hooks, a screen or a real clock.
pub trait Machine: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    /// Milliseconds the machine has been awake ([`power::awake_millis`]).
    fn awake_millis(&self) -> u64;
    /// The user's latest keyboard or mouse input: as the input hooks saw it
    /// (the user's own input only) while they are attached and working;
    /// otherwise Windows' idle clock (`GetLastInputInfo`, a single time, no
    /// content), which software-injected input moves too. Idle detection
    /// never depends on hooks the policy does not allow.
    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
    /// The user's first input after `since`, up to `now` (coming back): from
    /// the hooks while they are attached and working; otherwise the latest
    /// input Windows' idle clock saw, if it is after `since`.
    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
    /// Are the input hooks attached (so the two above see only the user's
    /// own input)?
    fn input_hooks_running(&self) -> bool;
    fn capture_screen(&self, blur: bool) -> Option<crate::tracking::ProvenanceScreenCapture>;
    /// Build the block's upload payload.
    fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<crate::tracking::ProvenanceScreenCapture>) -> Activity;

    /// The time away kept as work (CONTRACT §4).
    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity>;
    fn device(&self) -> &DeviceInfo;
    /// The app in front now (its executable name).
    fn current_app(&self) -> Option<String>;
    /// A uniformly random fraction in `[0, 1)` (the screenshot moment).
    fn random_fraction(&self) -> f64;
    /// Turn active sensor collection on or off (usage sampler and input counting).
    #[allow(dead_code)]
    fn set_tracking_active(&self, active: bool);
    /// Apply effective sensor policy based on active tracking status and session permissions.
    fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>);
}

impl Machine for Tracker {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn awake_millis(&self) -> u64 {
        power::awake_millis()
    }

    /// The hooks' view (the user's own input) while they are attached, work
    /// and see both kinds of input; otherwise Windows' idle clock, which
    /// mouse jigglers also reset. Detached hooks report "healthy" (nothing
    /// proves them silent), so whether they run is checked first; hooks that
    /// may only count keys would take someone scrolling for someone away.
    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.hooks_see_all_input() {
            self.input().last_input_at()
        } else {
            self.system_idle_seconds().map(|idle| now - seconds(idle))
        }
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let input = self.input();
        if self.hooks_see_all_input() {
            input
                .active_seconds(since + TimeDelta::seconds(1), now, true, true)
                .into_iter()
                .min()
                .and_then(|second| DateTime::from_timestamp(second, 0))
        } else {
            self.last_input(now).filter(|last| *last > since)
        }
    }

    fn input_hooks_running(&self) -> bool {
        Tracker::input_hooks_running(self)
    }

    fn capture_screen(&self, blur: bool) -> Option<crate::tracking::ProvenanceScreenCapture> {
        Tracker::capture_screen(self, blur)
    }

    fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<crate::tracking::ProvenanceScreenCapture>) -> Activity {
        Tracker::snapshot(self, block, screen)
    }

    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        Tracker::kept_idle_blocks(self, project_id, from, to, interval, description)
    }

    fn device(&self) -> &DeviceInfo {
        Tracker::device(self)
    }

    fn current_app(&self) -> Option<String> {
        Tracker::current_app(self)
    }

    fn random_fraction(&self) -> f64 {
        blocks::random_fraction()
    }

    fn set_tracking_active(&self, active: bool) {
        Tracker::set_tracking_active(self, active);
    }

    fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>) {
        Tracker::apply_sensor_policy(self, active, perms);
    }
}

/// Seconds as a delta, bounded to a year (idle times beyond that are noise).
pub(super) fn seconds(secs: u64) -> TimeDelta {
    TimeDelta::seconds(secs.min(366 * 24 * 3600) as i64)
}
