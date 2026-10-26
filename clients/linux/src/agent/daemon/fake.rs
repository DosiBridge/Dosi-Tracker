//! Test builds only (`test-support`): a [`Machine`] with synthetic input, a
//! synthetic screen and a time-scale knob, so the end-to-end tests drive the
//! real daemon, worker, uploader and IPC without a display, a bus or a real
//! person. Release builds never contain it (checked by `ci/check.sh release`).
//!
//! Environment of the daemon under test:
//! - `DOSI_TEST_MACHINE=fake` selects this machine.
//! - `DOSI_TEST_TIME_SCALE=60` makes one real second count as a minute
//!   (blocks, idle thresholds and breaks run 60 times faster).
//! - `DOSI_TEST_INPUT_FILE=<path>`: while the file says `away` the synthetic
//!   user is gone (no input); anything else (or no file) means present.
//!
//! Nothing here reads a real input device, screen or window.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::blocks::BlockId;
use crate::agent::model::Activity;
use crate::agent::state::{SessionKind, SessionSummary};
use crate::agent::worker::{CaptureEvent, Machine, PlatformCapabilities};
use crate::consent::ConsentedPermissions;
use crate::lifecycle::LifecycleGeneration;
use crate::tracking::build::{
    build_activity, BlockSpec, BuiltBlock, Measurements, ProvenanceScreenCapture, Zone,
};
use crate::tracking::capture::ScreenCapture;
use crate::tracking::model::WindowInfo;
use crate::tracking::system::DeviceInfo;
use crate::tracking::usage::UsageSlice;

/// A synthetic key press and click this often while the user is present.
const INPUT_EVERY_SECS: i64 = 6;

/// The synthetic user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    Present { since: DateTime<Utc> },
    Away { since: DateTime<Utc> },
}

/// See the module documentation.
pub struct FakeMachine {
    started_wall: DateTime<Utc>,
    started: Instant,
    scale: f64,
    input_file: Option<PathBuf>,
    presence: Mutex<Presence>,
    device: DeviceInfo,
    sink: Mutex<Option<UnboundedSender<CaptureEvent>>>,
    active: AtomicBool,
    perms: Mutex<Option<ConsentedPermissions>>,
}

impl FakeMachine {
    /// Read the knobs from `vars`.
    pub fn from_env(device: DeviceInfo, vars: &[(String, String)]) -> Self {
        let get = |name: &str| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        let scale = get("DOSI_TEST_TIME_SCALE")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s >= 1.0 && *s <= 3600.0)
            .unwrap_or(1.0);
        let now = Utc::now();
        Self {
            started_wall: now,
            started: Instant::now(),
            scale,
            input_file: get("DOSI_TEST_INPUT_FILE").map(PathBuf::from),
            presence: Mutex::new(Presence::Present { since: now }),
            device,
            sink: Mutex::new(None),
            active: AtomicBool::new(false),
            perms: Mutex::new(None),
        }
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed().mul_f64(self.scale)
    }

    /// Follow the input file: the synthetic user leaves or comes back.
    fn presence(&self, now: DateTime<Utc>) -> Presence {
        let away = self
            .input_file
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|text| text.trim() == "away");
        let mut presence = self.presence.lock().unwrap_or_else(|e| e.into_inner());
        *presence = match (*presence, away) {
            (Presence::Present { .. }, true) => Presence::Away { since: now },
            (Presence::Away { .. }, false) => Presence::Present { since: now },
            (current, _) => current,
        };
        *presence
    }

    fn effective(&self) -> ConsentedPermissions {
        if self.active.load(Ordering::SeqCst) {
            self.perms
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or_else(ConsentedPermissions::all_disabled)
        } else {
            ConsentedPermissions::all_disabled()
        }
    }

    fn shot(&self, blurred: bool) -> ProvenanceScreenCapture {
        ProvenanceScreenCapture {
            capture: ScreenCapture {
                // A synthetic JPEG-looking payload: never a real screen.
                jpeg: vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'F', b'A', b'K', b'E'],
                thumb_jpeg: Some(vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x08, b'T', b'N']),
                captured_at: self.now(),
                blurred,
            },
            generation: LifecycleGeneration(0),
        }
    }

    fn zone() -> Zone {
        Zone {
            time_zone: Some("UTC".into()),
            utc_offset_minutes: 0,
        }
    }
}

impl Machine for FakeMachine {
    fn now(&self) -> DateTime<Utc> {
        self.started_wall
            + TimeDelta::from_std(self.elapsed()).unwrap_or_else(|_| TimeDelta::zero())
    }

    fn awake_millis(&self) -> u64 {
        u64::try_from(self.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self.presence(now) {
            Presence::Present { .. } => Some(now),
            Presence::Away { since } => Some(since),
        }
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self.presence(now) {
            Presence::Present { since: back } => {
                Some(back.max(since + TimeDelta::seconds(1)).min(now))
            }
            Presence::Away { .. } => None,
        }
    }

    fn set_capture_sink(&self, sink: UnboundedSender<CaptureEvent>) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    fn request_capture(&self, block: BlockId, blur: bool) -> bool {
        let perms = self.effective();
        if !perms.screenshot {
            return false;
        }
        let event = CaptureEvent {
            block_id: block,
            capture: Some(self.shot(blur || perms.blur_screenshots)),
        };
        if let Some(sink) = self.sink.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = sink.send(event);
        }
        true
    }

    fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture> {
        let perms = self.effective();
        perms
            .screenshot
            .then(|| self.shot(blur || perms.blur_screenshots))
    }

    fn snapshot(
        &self,
        block: &BlockSpec<'_>,
        screen: Option<ProvenanceScreenCapture>,
    ) -> BuiltBlock {
        let current = self.effective();
        let (start, end) = (block.started_at.timestamp(), block.ended_at.timestamp());
        let inputs: Vec<i64> = (start..end)
            .filter(|second| second % INPUT_EVERY_SECS == 0)
            .map(|second| second * 1000)
            .collect();
        let recorded = block.perms.keyboard || block.perms.mouse;
        let measured = Measurements {
            key_presses: block.perms.keyboard.then(|| inputs.clone()),
            clicks: block.perms.mouse.then(|| inputs.clone()),
            active_seconds: if recorded {
                inputs.iter().map(|ms| ms / 1000).collect()
            } else {
                Vec::new()
            },
            injected: 0,
            input_available: true,
            slices: if block.perms.active_window && current.active_window {
                vec![UsageSlice {
                    app: "fake-editor".into(),
                    title: "synthetic window".into(),
                    start_ms: block.started_at.timestamp_millis(),
                    end_ms: block.ended_at.timestamp_millis(),
                }]
            } else {
                Vec::new()
            },
            window_now: None,
            running_programs: if block.perms.running_programs && current.running_programs {
                vec![WindowInfo {
                    app_name: "fake-editor".into(),
                    ..WindowInfo::default()
                }]
            } else {
                Vec::new()
            },
            screen: screen
                .filter(|_| current.screenshot)
                .map(|shot| shot.capture)
                .filter(|c| c.blurred || !current.blur_screenshots),
        };
        build_activity(block, measured, &self.device, Self::zone())
    }

    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        crate::tracking::build::kept_idle_blocks(
            project_id,
            from,
            to,
            TimeDelta::from_std(interval).unwrap_or_else(|_| TimeDelta::minutes(10)),
            description,
            &self.device,
            |_| Self::zone(),
        )
    }

    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn current_app(&self) -> Option<String> {
        self.effective().active_window.then(|| "fake-editor".into())
    }

    fn random_fraction(&self) -> f64 {
        0.5
    }

    fn apply_sensor_policy(&self, active: bool, perms: Option<&ConsentedPermissions>) {
        self.active.store(active, Ordering::SeqCst);
        *self.perms.lock().unwrap_or_else(|e| e.into_inner()) = perms.copied();
    }

    fn session_summary(&self) -> SessionSummary {
        SessionSummary {
            kind: SessionKind::X11,
            desktop: Some("fake".into()),
            idle_source: Some("fake".into()),
        }
    }

    fn platform(&self) -> PlatformCapabilities {
        PlatformCapabilities::everything()
    }

    fn retry_capture_permission(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceInfo {
        DeviceInfo {
            device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
            device_name: Some("test".into()),
            os_version: "Test (Linux 6.0)".into(),
            agent_version: "0".into(),
            platform: "linux".into(),
            time_zone: Some("UTC".into()),
        }
    }

    #[test]
    fn the_clock_runs_scaled_and_the_input_file_moves_the_user() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        let vars = vec![
            ("DOSI_TEST_TIME_SCALE".to_string(), "60".to_string()),
            (
                "DOSI_TEST_INPUT_FILE".to_string(),
                input.to_string_lossy().into_owned(),
            ),
        ];
        let machine = FakeMachine::from_env(device(), &vars);
        let t0 = machine.now();
        std::thread::sleep(Duration::from_millis(100));
        assert!(machine.now() - t0 >= TimeDelta::seconds(5), "60x faster");

        let now = machine.now();
        assert_eq!(machine.last_input(now), Some(now), "present by default");
        std::fs::write(&input, "away").unwrap();
        let left = machine.now();
        assert_eq!(machine.last_input(left), Some(left));
        let later = left + TimeDelta::minutes(5);
        assert_eq!(machine.last_input(later), Some(left), "input stopped");
        assert_eq!(machine.first_input_after(left, later), None);
        std::fs::write(&input, "present").unwrap();
        let back = later + TimeDelta::minutes(1);
        assert_eq!(machine.first_input_after(left, back), Some(back));
        assert_eq!(machine.last_input(back), Some(back));
    }

    #[test]
    fn nothing_is_captured_or_counted_without_effective_permissions() {
        let machine = FakeMachine::from_env(device(), &[]);
        assert!(!machine.request_capture(1, false));
        assert!(machine.capture_screen(false).is_none());
        assert_eq!(machine.current_app(), None);
        let perms = ConsentedPermissions {
            screenshot: true,
            ..ConsentedPermissions::all_disabled()
        };
        machine.apply_sensor_policy(true, Some(&perms));
        let shot = machine.capture_screen(false).unwrap();
        assert!(shot.capture.blurred, "consent blurs by default");
    }
}
