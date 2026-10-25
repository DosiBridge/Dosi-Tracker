//! In-memory fake acquisition provider matching Windows `FakeAcquisitionProvider`.
//!
//! Enforces:
//! - Completely side-effect-free: never calls native OS, display, process, or input APIs.
//! - Atomic query counters for deterministic assertion of call counts.
//! - Canned responses and in-flight interception hooks for ABA race testing.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use super::model::WindowInfo;
use super::traits::{ActiveWindowProvider, RunningAppsProvider, TrackingError};

/// Side-effect-free fake acquisition provider with atomic counters and canned responses.
pub struct FakeAcquisitionProvider {
    window_calls: AtomicU32,
    process_calls: AtomicU32,
    canned_window: Mutex<Option<WindowInfo>>,
    canned_processes: Mutex<Option<Vec<WindowInfo>>>,
    canned_error: Mutex<Option<TrackingError>>,
    on_current_window: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    on_running_programs: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl FakeAcquisitionProvider {
    pub fn new() -> Self {
        Self {
            window_calls: AtomicU32::new(0),
            process_calls: AtomicU32::new(0),
            canned_window: Mutex::new(Some(WindowInfo {
                app_name: "code".into(),
                window_title: "main.rs - Dosi-Tracker".into(),
                ..Default::default()
            })),
            canned_processes: Mutex::new(Some(vec![
                WindowInfo {
                    app_name: "code".into(),
                    window_title: String::new(),
                    ..Default::default()
                },
                WindowInfo {
                    app_name: "slack".into(),
                    window_title: String::new(),
                    ..Default::default()
                },
            ])),
            canned_error: Mutex::new(None),
            on_current_window: Mutex::new(None),
            on_running_programs: Mutex::new(None),
        }
    }

    pub fn window_calls(&self) -> u32 {
        self.window_calls.load(Ordering::SeqCst)
    }

    pub fn process_calls(&self) -> u32 {
        self.process_calls.load(Ordering::SeqCst)
    }

    pub fn set_canned_window(&self, window: Option<WindowInfo>) {
        *self.canned_window.lock().unwrap() = window;
    }

    pub fn set_canned_processes(&self, processes: Option<Vec<WindowInfo>>) {
        *self.canned_processes.lock().unwrap() = processes;
    }

    pub fn set_canned_error(&self, error: Option<TrackingError>) {
        *self.canned_error.lock().unwrap() = error;
    }

    pub fn set_on_current_window(&self, hook: Box<dyn Fn() + Send + Sync>) {
        *self.on_current_window.lock().unwrap() = Some(hook);
    }

    pub fn set_on_running_programs(&self, hook: Box<dyn Fn() + Send + Sync>) {
        *self.on_running_programs.lock().unwrap() = Some(hook);
    }
}

impl Default for FakeAcquisitionProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveWindowProvider for FakeAcquisitionProvider {
    fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
        self.window_calls.fetch_add(1, Ordering::SeqCst);

        if let Some(ref hook) = *self.on_current_window.lock().unwrap() {
            hook();
        }

        if let Some(ref err) = *self.canned_error.lock().unwrap() {
            return Err(err.clone());
        }

        let win = self.canned_window.lock().unwrap().clone();
        Ok(win)
    }
}

impl RunningAppsProvider for FakeAcquisitionProvider {
    fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError> {
        self.process_calls.fetch_add(1, Ordering::SeqCst);

        if let Some(ref hook) = *self.on_running_programs.lock().unwrap() {
            hook();
        }

        if let Some(ref err) = *self.canned_error.lock().unwrap() {
            return Err(err.clone());
        }

        let procs = self
            .canned_processes
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
        Ok(procs)
    }
}
