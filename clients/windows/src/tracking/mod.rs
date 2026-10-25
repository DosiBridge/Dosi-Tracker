//! Tracking subsystem.
//!
//! Design goal: keep idle CPU near zero. Input listening is event-driven
//! (the OS pushes events; we only record timestamps), the foreground window
//! is sampled every few seconds with a couple of Win32 calls, and heavy work
//! (screenshot, webcam) happens once per block, then the agent sleeps.

mod acquisition;
pub mod active_window;
pub mod input;
pub mod lifecycle;
mod payload;
pub mod screenshot;
pub mod system;
pub mod usage;
pub mod webcam;

use std::sync::{Arc, RwLock};

use chrono::{DateTime, TimeDelta, Utc};

use crate::config::CapturePermissions;
use crate::model::{Activity, EndReason};
pub use acquisition::{AcquisitionProvider, FakeAcquisitionProvider, NativeAcquisitionProvider};
use input::InputCounter;
pub use lifecycle::{FakeSensorDriver, NativeSensorDriver, SensorDriver};
pub use payload::{build_activity, kept_idle_blocks, Measurements, Zone};
use screenshot::ScreenCapture;
use system::DeviceInfo;
use usage::UsageRecorder;

#[derive(Debug, Clone)]
pub struct ProvenanceScreenCapture {
    pub capture: ScreenCapture,
    pub generation: u64,
}

/// A block shorter than this is never recorded (CONTRACT §1).
pub const MIN_BLOCK: TimeDelta = TimeDelta::seconds(60);

/// One block to record: its project and capture policy, its time window, and
/// why it ended.
#[derive(Debug, Clone, Copy)]
pub struct BlockSpec<'a> {
    pub project_id: &'a str,
    pub perms: &'a CapturePermissions,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub end_reason: Option<EndReason>,
    /// The task note ("What are you working on?").
    pub description: Option<&'a str>,
}

impl BlockSpec<'_> {
    /// May the block take a capture now that it ends? Not when the machine
    /// is locked, asleep, idle or shutting down.
    pub fn captures_at_end(&self) -> bool {
        self.end_reason.is_none_or(EndReason::allows_capture_at_end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityGenerations {
    pub screen: u64,
    pub webcam: u64,
    pub window: u64,
    pub processes: u64,
}

impl Default for CapabilityGenerations {
    fn default() -> Self {
        Self {
            screen: 1,
            webcam: 1,
            window: 1,
            processes: 1,
        }
    }
}

#[derive(Debug, Clone)]
struct TrackerAuth {
    active: bool,
    perms: CapturePermissions,
    generations: CapabilityGenerations,
}

impl Default for TrackerAuth {
    fn default() -> Self {
        Self {
            active: false,
            perms: CapturePermissions::all_disabled(),
            generations: CapabilityGenerations::default(),
        }
    }
}

impl TrackerAuth {
    /// Update effective authorization and advance capability generations if and only if
    /// an authorized capability was revoked, paused/stopped, or tightened (e.g. blur mandated),
    /// or if a new authorization era begins (granting from unauthorized).
    ///
    /// Preserves existing generations for unchanged or unaffected capabilities
    /// so routine repeated publication of identical policy does not invalidate
    /// legitimate in-flight sampling.
    fn update(&mut self, active: bool, perms: Option<&CapturePermissions>) {
        let new_active = active && perms.is_some();
        let new_perms = if active {
            perms.cloned().unwrap_or_else(CapturePermissions::all_disabled)
        } else {
            CapturePermissions::all_disabled()
        };

        // 1. Screen generation: transitions in authorization or blur tightening
        let screen_was_auth = self.active && self.perms.screenshot;
        let screen_is_auth = new_active && new_perms.screenshot;
        let blur_tightened = screen_was_auth && screen_is_auth && !self.perms.blur_screenshots && new_perms.blur_screenshots;
        if screen_was_auth != screen_is_auth || blur_tightened {
            self.generations.screen = self.generations.screen.wrapping_add(1);
        }

        // 2. Webcam generation: transitions in authorization
        let webcam_was_auth = self.active && self.perms.webcam;
        let webcam_is_auth = new_active && new_perms.webcam;
        if webcam_was_auth != webcam_is_auth {
            self.generations.webcam = self.generations.webcam.wrapping_add(1);
        }

        // 3. Window generation: transitions in authorization
        let window_was_auth = self.active && self.perms.active_window;
        let window_is_auth = new_active && new_perms.active_window;
        if window_was_auth != window_is_auth {
            self.generations.window = self.generations.window.wrapping_add(1);
        }

        // 4. Processes generation: transitions in authorization
        let progs_was_auth = self.active && self.perms.running_programs;
        let progs_is_auth = new_active && new_perms.running_programs;
        if progs_was_auth != progs_is_auth {
            self.generations.processes = self.generations.processes.wrapping_add(1);
        }

        self.active = new_active;
        self.perms = new_perms;
    }
}

/// Collects one activity snapshot for the given project time block.
///
/// Capture permissions are passed per snapshot because the effective set
/// (server-side project settings ∧ local config) may only become known after
/// the first successful sync with the backend.
pub struct Tracker {
    input: Arc<InputCounter>,
    usage: Arc<UsageRecorder>,
    device: DeviceInfo,
    driver: Arc<dyn SensorDriver>,
    acquisition: Arc<dyn AcquisitionProvider>,
    auth: RwLock<TrackerAuth>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::with_driver_device_and_acquisition(
            Arc::new(NativeSensorDriver::new()),
            DeviceInfo::collect(),
            Arc::new(NativeAcquisitionProvider::new()),
        )
    }

    /// Construct a Tracker with an explicit sensor driver.
    ///
    /// Invariant: Constructor NEVER starts input hooks or window sampler!
    /// Sensors remain completely stopped and unattached until explicit authorized activation.
    #[allow(dead_code)]
    pub fn with_driver_and_device(driver: Arc<dyn SensorDriver>, device: DeviceInfo) -> Self {
        Self::with_driver_device_and_acquisition(
            driver,
            device,
            Arc::new(NativeAcquisitionProvider::new()),
        )
    }

    #[allow(dead_code)]
    pub fn with_driver(driver: Arc<dyn SensorDriver>) -> Self {
        Self::with_driver_and_device(driver, DeviceInfo::collect())
    }

    pub fn with_driver_device_and_acquisition(
        driver: Arc<dyn SensorDriver>,
        device: DeviceInfo,
        acquisition: Arc<dyn AcquisitionProvider>,
    ) -> Self {
        let input = Arc::new(InputCounter::new());
        let usage = Arc::new(UsageRecorder::new());
        Self {
            input,
            usage,
            device,
            driver,
            acquisition,
            auth: RwLock::new(TrackerAuth::default()),
        }
    }

    /// Construct a fake tracker for safe synthetic testing without OS hooks and without disk side effects.
    #[cfg_attr(not(test), expect(dead_code, reason = "for testing and inspection"))]
    pub fn new_fake() -> (Self, Arc<FakeSensorDriver>) {
        let (tracker, driver, _acq) = Self::new_fake_with_acquisition();
        (tracker, driver)
    }

    /// Construct a fake tracker exposing both fake driver and fake acquisition provider.
    #[cfg_attr(not(test), expect(dead_code, reason = "for testing and inspection"))]
    pub fn new_fake_with_acquisition() -> (Self, Arc<FakeSensorDriver>, Arc<FakeAcquisitionProvider>) {
        let driver = Arc::new(FakeSensorDriver::new());
        let acquisition = Arc::new(FakeAcquisitionProvider::new());
        (
            Self::with_driver_device_and_acquisition(driver.clone(), DeviceInfo::fixture(), acquisition.clone()),
            driver,
            acquisition,
        )
    }

    /// Read-only access to input for the live view (counts and activity % of
    /// the block in progress, whether input capture works).
    pub fn input(&self) -> Arc<InputCounter> {
        self.input.clone()
    }

    /// Read-only access to foreground-window usage, for the window's "top
    /// apps" list (shown only when the project records app usage).
    pub fn usage(&self) -> Arc<UsageRecorder> {
        self.usage.clone()
    }

    /// Access the underlying sensor driver.
    #[expect(dead_code, reason = "for testing and inspection")]
    pub fn driver(&self) -> Arc<dyn SensorDriver> {
        self.driver.clone()
    }

    /// Access the underlying acquisition provider.
    #[expect(dead_code, reason = "for testing and inspection")]
    pub fn acquisition(&self) -> Arc<dyn AcquisitionProvider> {
        self.acquisition.clone()
    }

    /// Apply effective sensor policy based on active tracking status and session permissions.
    ///
    /// When inactive or perms are None, detaches OS hooks and halts sampling immediately,
    /// failing closed to unauthorized for all capabilities.
    /// When active, attaches only the sensors explicitly permitted by policy and retains
    /// current effective authorization at the acquisition boundary.
    ///
    /// Synchronization Contract:
    /// - Atomically updates `auth` under a write lock, advancing per-capability invalidation
    ///   generations on authorization transitions (revocation, grant, or tightening).
    /// - Routine repeated publication of an IDENTICAL policy preserves generations, avoiding
    ///   false invalidation of legitimate in-flight acquisitions.
    /// - Because acquisition operations NEVER hold a read lock across provider calls,
    ///   `apply_sensor_policy` does not suffer authorization lock contention.
    /// - IMPORTANT: `apply_sensor_policy` does NOT complete instantaneously; it synchronously
    ///   posts quit messages and joins native sensor driver threads (`driver.stop_input`, `driver.stop_usage`).
    ///   Absence of auth-lock contention is narrower than total nonblocking completion.
    pub fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>) {
        let active_window = active && perms.is_some_and(|p| p.active_window);
        let keyboard = active && perms.is_some_and(|p| p.keyboard);
        let mouse = active && perms.is_some_and(|p| p.mouse);
        let input_active = keyboard || mouse;

        // 1. Monotonically update internal acquisition authorization under lock.
        // Fails closed to inactive and all-disabled when active is false or perms are None.
        // Advances capability generations when authorized capabilities are revoked, granted,
        // or tightened, preventing ABA revocation/regrant resurrection of in-flight results.
        {
            let mut auth = self.auth.write().unwrap();
            auth.update(active, perms);
        }

        // 2. Update usage & input accumulators.
        self.usage.set_active(active_window);
        self.input.set_policy(input_active, keyboard, mouse);

        // 3. Update sensor drivers.
        if input_active {
            self.driver.start_input(self.input.clone(), keyboard, mouse);
        } else {
            self.driver.stop_input();
        }

        if active_window {
            self.driver.start_usage(self.usage.clone());
        } else {
            self.driver.stop_usage();
        }
    }

    /// Turn active sensor collection on or off (legacy helper using default permissions).
    #[expect(dead_code, reason = "for testing and inspection")]
    pub fn set_tracking_active(&self, active: bool) {
        if active {
            let default_perms = CapturePermissions::default();
            self.apply_sensor_policy(true, Some(&default_perms));
        } else {
            self.apply_sensor_policy(false, None);
        }
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "for UI and inspection"))]
    pub fn is_tracking_active(&self) -> bool {
        self.auth.read().unwrap().active
    }

    /// Are the input hooks attached?
    pub fn input_hooks_running(&self) -> bool {
        self.driver.is_input_running()
    }

    /// Do the input hooks see all of the user's input right now: attached,
    /// working, and allowed to see both keys and the mouse? Only then is
    /// their view of "the last input" complete — anything less and Windows'
    /// own idle clock knows better.
    pub fn hooks_see_all_input(&self) -> bool {
        self.input_hooks_running()
            && self.input.hook_healthy()
            && self.input.is_keyboard_allowed()
            && self.input.is_mouse_allowed()
    }

    /// Seconds since Windows last saw any keyboard or mouse input
    /// (`GetLastInputInfo`): one tick count, read without any hook. What idle
    /// and return detection fall back on while the hooks are off.
    pub fn system_idle_seconds(&self) -> Option<u64> {
        self.driver.system_idle_seconds()
    }

    /// The app in front now (its executable name), queried through the acquisition provider
    /// ONLY IF actively tracking and active_window is authorized in current policy.
    ///
    /// Admission / Revocation Synchronization Contract:
    /// - Linearized logical admission: evaluates authorization and captures capability admission
    ///   generation token under a brief read lock.
    /// - Drops read lock BEFORE delegating to provider so sensor driver state changes do not contend
    ///   for authorization locks across slow foreground queries.
    /// - A brief read-lock check followed by an unlocked provider invocation linearizes logical admission,
    ///   not atomic physical provider entry with revocation.
    /// - Post-admission validation: validates generation token upon return. If active_window was revoked
    ///   or policy tightened while in flight (even if later re-granted before return), generation token
    ///   mismatch proves the sample originated in an invalidated consent interval, and the result is discarded.
    /// - Note: Discarding the result prevents admission into storage or state; it does not cancel in-flight OS work.
    pub fn current_app(&self) -> Option<String> {
        let (authorized, token) = {
            let auth = self.auth.read().unwrap();
            (auth.active && auth.perms.active_window, auth.generations.window)
        };
        if !authorized {
            return None;
        }

        let win = self.acquisition.current_window()?;

        // Post-admission verification: validate authorization and generation token.
        let still_authorized = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.active_window && auth.generations.window == token
        };
        if !still_authorized {
            return None;
        }

        let name = win.app_name;
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// This device: install id, name, OS and agent version (also for the
    /// presence heartbeat).
    #[expect(dead_code, reason = "for the worker's presence heartbeat")]
    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// Capture the screen now. The caller picks the moment — a random point
    /// inside the block — and hands the capture to [`Tracker::snapshot`] when
    /// the block ends.
    ///
    /// Admission / Revocation Synchronization Contract:
    /// 1. Linearized logical admission: gated on CURRENT effective authorization (`auth.active && auth.perms.screenshot`).
    /// 2. If unauthorized, returns `None` immediately with 0 provider calls.
    /// 3. Read lock is dropped before entering the provider so `apply_sensor_policy`
    ///    does not suffer authorization lock contention.
    /// 4. Blur requirement is evaluated conservatively: blurred if either the caller
    ///    requested blur OR current effective policy requires blur.
    /// 5. Generation token captured at admission: on provider return, verifies that
    ///    `auth.generations.screen == token`. If screenshot was revoked or blur requirement
    ///    was added while in flight (even if later re-granted before return), generation
    ///    token mismatch ensures the invalidated capture is discarded.
    ///
    /// Note on physical execution vs logical admission:
    /// A short read-lock check followed by an unlocked provider invocation linearizes logical admission,
    /// but does not guarantee atomic physical provider entry with revocation. Synchronous OS screenshot
    /// capture cannot be cancelled mid-flight without unsafe aborts. Discarding the result prevents
    /// invalidated captures from entering storage or activity blocks; it does not constitute hardware
    /// cancellation or sensor stopping. Holding a global auth lock across blocking OS work is deliberately
    /// avoided to prevent deadlocks and UI thread lockups.
    pub fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture> {
        let (authorized, effective_blur, token) = {
            let auth = self.auth.read().unwrap();
            (
                auth.active && auth.perms.screenshot,
                blur || auth.perms.blur_screenshots,
                auth.generations.screen,
            )
        };

        if !authorized {
            return None;
        }

        let capture = match self.acquisition.capture_screen(effective_blur) {
            Ok(capture) => capture,
            Err(e) => {
                tracing::warn!(?e, "screenshot capture failed");
                return None;
            }
        };

        // Post-admission verification: check if revoked, regranted, or tightened while in-flight
        let still_valid = {
            let auth = self.auth.read().unwrap();
            auth.active
                && auth.perms.screenshot
                && auth.generations.screen == token
                && (!auth.perms.blur_screenshots || capture.blurred)
        };

        if !still_valid {
            tracing::debug!("screenshot completed after revocation or policy change; discarding capture");
            return None;
        }

        Some(ProvenanceScreenCapture {
            capture,
            generation: token,
        })
    }

    /// Build the activity for `block`, with `screen` taken earlier by
    /// [`Tracker::capture_screen`] (if any).
    ///
    /// Admission / Revocation Synchronization Contract:
    /// - Upfront Generation Era Token Capture: Generation era tokens are captured upfront
    ///   under a brief read lock for all capabilities (`window_token`, `progs_token`, `screen_token`,
    ///   `webcam_token`) at method entry.
    /// - Sequential Provider Execution with Fresh Boolean Checks: Immediately before calling each
    ///   provider (`current_window`, `running_programs`, `capture_webcam`), a fresh boolean admission
    ///   check is evaluated under a brief read lock (`auth.active && auth.perms.<cap> && perms.<cap>`).
    ///   If unauthorized, the provider call is skipped entirely (zero provider calls).
    /// - Read locks are NEVER held across provider calls, avoiding authorization lock contention
    ///   with `apply_sensor_policy`.
    /// - Conservative Rejection after Generation Changes: If a capability undergoes an authorization
    ///   transition (revocation, re-grant, or blur tightening) after snapshot start, a later provider
    ///   may execute under current boolean permissions if re-granted, but the resulting acquisition
    ///   will be conservatively rejected at final publication reconciliation because the current
    ///   generation token will no longer match the upfront era token.
    /// - Final Post-Provider Reconciliation: Once all sequential provider calls finish, a single
    ///   post-execution validation under lock validates all acquired results against current authorization,
    ///   the upfront era tokens, and (for screen) the handed-off screen provenance generation token.
    ///   - Discarding invalidated results prevents unconsented data from entering activity blocks
    ///     or persistent storage; it is post-execution validation, not hardware cancellation or
    ///     atomic physical provider entry with revocation.
    ///   - `self.usage.record(ended_at, ...)` is deferred to this final reconciliation and invoked
    ///     if and only if `window_valid` passes, guaranteeing that invalidated window samples
    ///     never contaminate historical usage.
    /// - Process Decoupling: `running_programs` is queried based strictly on active running_programs
    ///   authorization and historical block permission, independent of the active_window usage driver.
    /// - Historical Accumulator Preservation: Historical accumulator data (`slices`, `key_presses`,
    ///   `clicks`, `active_seconds`) recorded while consent was active is preserved in the emitted block
    ///   without acquiring new data.
    /// - Screenshot Provenance Handoff: Handed-off screenshots must have been captured under an authorized
    ///   generation matching the current publication era, and conservative blur is enforced if either
    ///   the caller or current policy requires blur.
    pub fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<ProvenanceScreenCapture>) -> Activity {
        let perms = block.perms;
        let (started_at, ended_at) = (block.started_at, block.ended_at);

        // 0. Capture generation tokens for all capabilities upfront before invoking any providers
        let (window_token, progs_token, screen_token, webcam_token) = {
            let auth = self.auth.read().unwrap();
            (
                auth.generations.window,
                auth.generations.processes,
                auth.generations.screen,
                auth.generations.webcam,
            )
        };

        // 1. Current window admission check: requires actively authorized in current policy,
        // allowed in historical block, and usage sampler is running.
        let admit_window = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.active_window && perms.active_window && self.driver.is_usage_running()
        };
        let window_raw = if admit_window {
            self.acquisition.current_window()
        } else {
            None
        };

        // 2. Running programs admission check: requires actively authorized in current policy
        // and allowed in historical block. Strictly independent of the active_window usage driver.
        let admit_programs = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.running_programs && perms.running_programs
        };
        let running_programs_raw = if admit_programs {
            match self.acquisition.running_programs() {
                Ok(progs) => progs,
                Err(e) => {
                    tracing::warn!(?e, "could not list running programs");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // 3. Screen capture provenance preparation: verify screen passed in from caller against historical block permission.
        let screen_raw = if perms.screenshot {
            screen
        } else {
            None
        };

        // 4. Webcam capture admission check: requires actively authorized in current policy,
        // allowed in historical block, and block allows capture at end.
        let admit_webcam = {
            let auth = self.auth.read().unwrap();
            auth.active && auth.perms.webcam && perms.webcam && block.captures_at_end()
        };
        let webcam_jpg_base64_raw = if admit_webcam {
            match self.acquisition.capture_webcam() {
                Ok(b64) => Some(b64),
                Err(e) => {
                    tracing::warn!(?e, "webcam capture failed");
                    None
                }
            }
        } else {
            None
        };

        // 5. Final Logical Publication Boundary & Post-All-Providers Revalidation:
        // Verify that NONE of the capability generations were revoked, re-granted (ABA), or altered
        // during the execution of any earlier or later provider calls.
        let (window_now, running_programs, screen, webcam_jpg_base64) = {
            let auth = self.auth.read().unwrap();
            let window_valid = auth.active && auth.perms.active_window && auth.generations.window == window_token;
            let progs_valid = auth.active && auth.perms.running_programs && auth.generations.processes == progs_token;
            let blur_required = auth.perms.blur_screenshots || perms.blur_screenshots;
            let screen_valid = auth.active
                && auth.perms.screenshot
                && auth.generations.screen == screen_token
                && screen_raw.as_ref().is_some_and(|sc| sc.generation == screen_token)
                && screen_raw.as_ref().is_some_and(|sc| sc.capture.blurred || !blur_required);
            let webcam_valid = auth.active && auth.perms.webcam && auth.generations.webcam == webcam_token;

            let final_window = if window_valid {
                if let Some(ref win) = window_raw {
                    self.usage.record(ended_at.timestamp_millis(), Some(win.clone()));
                }
                window_raw
            } else {
                None
            };

            let final_progs = if progs_valid {
                running_programs_raw
            } else {
                Vec::new()
            };

            let final_screen = if screen_valid {
                screen_raw.map(|sc| sc.capture)
            } else {
                None
            };

            let final_webcam = if webcam_valid {
                webcam_jpg_base64_raw
            } else {
                None
            };

            (final_window, final_progs, final_screen, final_webcam)
        };

        let measured = Measurements {
            key_presses: if perms.keyboard { self.input.key_presses_between(started_at, ended_at) } else { Vec::new() },
            clicks: if perms.mouse { self.input.clicks_between(started_at, ended_at) } else { Vec::new() },
            active_seconds: self.input.active_seconds(started_at, ended_at, perms.keyboard, perms.mouse),
            injected: self.input.injected_between(started_at, ended_at, perms.keyboard, perms.mouse),
            input_available: self.input.capture_available_between(started_at, ended_at),
            slices: if perms.active_window {
                self.usage.slices_between(started_at.timestamp_millis(), ended_at.timestamp_millis())
            } else {
                Vec::new()
            },
            window_now,
            running_programs,
            screen,
            webcam_jpg_base64,
        };
        let zone = Zone { time_zone: system::time_zone(), utc_offset_minutes: system::utc_offset_minutes(started_at) };
        build_activity(block, measured, &self.device, zone)
    }

    /// The away span kept as work after the idle prompt; see [`kept_idle_blocks`].
    #[expect(dead_code, reason = "called by the worker's idle prompt")]
    pub fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: std::time::Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        let max = TimeDelta::from_std(interval).unwrap_or(TimeDelta::minutes(10));
        kept_idle_blocks(project_id, from, to, max, description, &self.device, system::time_zone().as_deref())
    }
}

#[cfg(test)]
mod tests;
