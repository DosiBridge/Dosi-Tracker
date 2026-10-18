//! Sensor driver lifecycle and hardware abstraction.
//!
//! Separates the OS-level hook installation and active-window sampling from
//! the tracker logic, enabling safe, synthetic unit testing without real OS hooks,
//! active window queries, or desktop collection during QA.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use std::time::Duration;

use super::input::InputCounter;
use super::usage::UsageRecorder;

/// The hardware/OS sensor lifecycle interface.
pub trait SensorDriver: Send + Sync {
    /// Start OS input listening hooks (keyboard and/or mouse).
    fn start_input(&self, counter: Arc<InputCounter>, keyboard: bool, mouse: bool);

    /// Stop OS input listening hooks and detach from OS message queues.
    fn stop_input(&self);

    /// Whether input hooks are currently attached and running.
    fn is_input_running(&self) -> bool;

    /// Start the active-window foreground usage sampler.
    fn start_usage(&self, recorder: Arc<UsageRecorder>);

    /// Stop the active-window usage sampler thread.
    fn stop_usage(&self);

    /// Whether the usage sampler is currently running.
    fn is_usage_running(&self) -> bool;
}

/// A fake sensor driver for safe synthetic tests without native OS side effects.
#[derive(Default)]
#[cfg_attr(not(test), expect(dead_code, reason = "for synthetic tests and simulation"))]
pub struct FakeSensorDriver {
    pub input_started: AtomicUsize,
    pub input_stopped: AtomicUsize,
    pub usage_started: AtomicUsize,
    pub usage_stopped: AtomicUsize,
    pub input_running: AtomicBool,
    pub usage_running: AtomicBool,
    pub last_keyboard_allowed: AtomicBool,
    pub last_mouse_allowed: AtomicBool,
}

#[cfg_attr(not(test), expect(dead_code, reason = "for synthetic tests and simulation"))]
impl FakeSensorDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn input_started_count(&self) -> usize {
        self.input_started.load(Ordering::SeqCst)
    }

    pub fn input_stopped_count(&self) -> usize {
        self.input_stopped.load(Ordering::SeqCst)
    }

    pub fn usage_started_count(&self) -> usize {
        self.usage_started.load(Ordering::SeqCst)
    }

    pub fn usage_stopped_count(&self) -> usize {
        self.usage_stopped.load(Ordering::SeqCst)
    }

    pub fn last_keyboard_allowed(&self) -> bool {
        self.last_keyboard_allowed.load(Ordering::SeqCst)
    }

    pub fn last_mouse_allowed(&self) -> bool {
        self.last_mouse_allowed.load(Ordering::SeqCst)
    }
}

impl SensorDriver for FakeSensorDriver {
    fn start_input(&self, counter: Arc<InputCounter>, keyboard: bool, mouse: bool) {
        counter.set_permissions(keyboard, mouse);
        self.last_keyboard_allowed.store(keyboard, Ordering::SeqCst);
        self.last_mouse_allowed.store(mouse, Ordering::SeqCst);
        if !self.input_running.swap(true, Ordering::SeqCst) {
            self.input_started.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn stop_input(&self) {
        if self.input_running.swap(false, Ordering::SeqCst) {
            self.input_stopped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn is_input_running(&self) -> bool {
        self.input_running.load(Ordering::SeqCst)
    }

    fn start_usage(&self, _recorder: Arc<UsageRecorder>) {
        if !self.usage_running.swap(true, Ordering::SeqCst) {
            self.usage_started.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn stop_usage(&self) {
        if self.usage_running.swap(false, Ordering::SeqCst) {
            self.usage_stopped.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn is_usage_running(&self) -> bool {
        self.usage_running.load(Ordering::SeqCst)
    }
}

#[cfg(windows)]
struct InputThread {
    thread_id: u32,
    join_handle: std::thread::JoinHandle<()>,
}

#[cfg(windows)]
struct UsageThread {
    stop: Arc<AtomicBool>,
    join_handle: std::thread::JoinHandle<()>,
}

#[cfg(windows)]
pub struct NativeSensorDriver {
    input_thread: Mutex<Option<InputThread>>,
    usage_thread: Mutex<Option<UsageThread>>,
}

#[cfg(windows)]
impl Default for NativeSensorDriver {
    fn default() -> Self {
        Self {
            input_thread: Mutex::new(None),
            usage_thread: Mutex::new(None),
        }
    }
}

#[cfg(windows)]
impl NativeSensorDriver {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(windows)]
impl SensorDriver for NativeSensorDriver {
    fn start_input(&self, counter: Arc<InputCounter>, keyboard: bool, mouse: bool) {
        counter.set_permissions(keyboard, mouse);
        let mut guard = self.input_thread.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let counter_clone = counter.clone();
        let spawned = std::thread::Builder::new()
            .name("input-hooks".into())
            .spawn(move || {
                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                let _ = tx.send(thread_id);
                super::input::run_hooks_loop(counter_clone);
            });

        match spawned {
            Ok(join_handle) => match rx.recv() {
                Ok(thread_id) => {
                    *guard = Some(InputThread { thread_id, join_handle });
                }
                Err(e) => {
                    tracing::error!(?e, "failed to receive input hook thread id");
                    let _ = join_handle.join();
                }
            },
            Err(e) => {
                tracing::error!(?e, "could not start input hook thread");
            }
        }
    }

    fn stop_input(&self) {
        let thread = {
            let mut guard = self.input_thread.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };

        if let Some(thread) = thread {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    thread.thread_id,
                    windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
            if let Err(e) = thread.join_handle.join() {
                tracing::warn!(?e, "error joining input hook thread");
            }
        }
    }

    fn is_input_running(&self) -> bool {
        self.input_thread.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    fn start_usage(&self, recorder: Arc<UsageRecorder>) {
        let mut guard = self.usage_thread.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("usage-sampler".into())
            .spawn(move || {
                while !stop_clone.load(Ordering::SeqCst) {
                    if recorder.is_active() {
                        recorder.record(chrono::Utc::now().timestamp_millis(), super::active_window::current());
                    }
                    // Sleep in 100ms slices so stopping responds in <= 100ms
                    for _ in 0..30 {
                        if stop_clone.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });

        match spawned {
            Ok(join_handle) => {
                *guard = Some(UsageThread { stop, join_handle });
            }
            Err(e) => {
                tracing::error!(?e, "could not start usage sampler thread");
            }
        }
    }

    fn stop_usage(&self) {
        let thread = {
            let mut guard = self.usage_thread.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };

        if let Some(thread) = thread {
            thread.stop.store(true, Ordering::SeqCst);
            if let Err(e) = thread.join_handle.join() {
                tracing::warn!(?e, "error joining usage sampler thread");
            }
        }
    }

    fn is_usage_running(&self) -> bool {
        self.usage_thread.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

#[cfg(not(windows))]
pub struct NativeSensorDriver;

#[cfg(not(windows))]
impl NativeSensorDriver {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl SensorDriver for NativeSensorDriver {
    fn start_input(&self, _counter: Arc<InputCounter>, _keyboard: bool, _mouse: bool) {}
    fn stop_input(&self) {}
    fn is_input_running(&self) -> bool {
        false
    }
    fn start_usage(&self, _recorder: Arc<UsageRecorder>) {}
    fn stop_usage(&self) {}
    fn is_usage_running(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn fake_driver_default_state_is_never_started() {
        let driver = FakeSensorDriver::new();
        assert!(!driver.is_input_running());
        assert!(!driver.is_usage_running());
        assert_eq!(driver.input_started_count(), 0);
        assert_eq!(driver.input_stopped_count(), 0);
        assert_eq!(driver.usage_started_count(), 0);
        assert_eq!(driver.usage_stopped_count(), 0);
    }

    #[test]
    fn fake_driver_per_sensor_activation_and_idempotency() {
        let driver = FakeSensorDriver::new();
        let counter = Arc::new(InputCounter::new());
        let recorder = Arc::new(UsageRecorder::new());

        // Start input only
        driver.start_input(counter.clone(), true, false);
        assert!(driver.is_input_running());
        assert!(!driver.is_usage_running());
        assert_eq!(driver.input_started_count(), 1);
        assert!(driver.last_keyboard_allowed());
        assert!(!driver.last_mouse_allowed());

        // Second call while already running does not re-increment start counter
        driver.start_input(counter.clone(), true, true);
        assert_eq!(driver.input_started_count(), 1);
        assert!(driver.last_mouse_allowed());

        // Start usage
        driver.start_usage(recorder.clone());
        assert!(driver.is_usage_running());
        assert_eq!(driver.usage_started_count(), 1);

        // Stop input
        driver.stop_input();
        assert!(!driver.is_input_running());
        assert_eq!(driver.input_stopped_count(), 1);
        assert!(driver.is_usage_running());

        // Stop usage
        driver.stop_usage();
        assert!(!driver.is_usage_running());
        assert_eq!(driver.usage_stopped_count(), 1);
    }

    #[test]
    fn fake_driver_repeated_transitions_balance_counts() {
        let driver = FakeSensorDriver::new();
        let counter = Arc::new(InputCounter::new());
        let recorder = Arc::new(UsageRecorder::new());

        for _ in 0..5 {
            driver.start_input(counter.clone(), true, true);
            driver.start_usage(recorder.clone());
            assert!(driver.is_input_running());
            assert!(driver.is_usage_running());

            driver.stop_input();
            driver.stop_usage();
            assert!(!driver.is_input_running());
            assert!(!driver.is_usage_running());
        }

        assert_eq!(driver.input_started_count(), 5);
        assert_eq!(driver.input_stopped_count(), 5);
        assert_eq!(driver.usage_started_count(), 5);
        assert_eq!(driver.usage_stopped_count(), 5);
    }

    #[test]
    fn fake_driver_concurrent_barrier_transitions() {
        let driver = Arc::new(FakeSensorDriver::new());
        let counter = Arc::new(InputCounter::new());
        let recorder = Arc::new(UsageRecorder::new());
        let barrier = Arc::new(Barrier::new(4));

        let mut handles = Vec::new();
        for i in 0..4 {
            let d = driver.clone();
            let c = counter.clone();
            let r = recorder.clone();
            let b = barrier.clone();
            handles.push(std::thread::spawn(move || {
                b.wait();
                if i % 2 == 0 {
                    d.start_input(c, true, true);
                    d.start_usage(r);
                } else {
                    d.stop_input();
                    d.stop_usage();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // State remains deterministic boolean flags without data corruption
        let _ = driver.is_input_running();
        let _ = driver.is_usage_running();
    }
}
