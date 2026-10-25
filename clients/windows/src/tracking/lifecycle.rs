//! Sensor driver lifecycle and hardware abstraction.
//!
//! Separates the OS-level hook installation and active-window sampling from
//! the tracker logic, enabling safe, synthetic unit testing without real OS hooks,
//! active window queries, or desktop collection during QA.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

    /// Seconds since Windows last saw keyboard or mouse input
    /// (`GetLastInputInfo`), hooks or not; `None` when it will not say.
    fn system_idle_seconds(&self) -> Option<u64>;

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
    /// The simulated Windows idle clock: seconds since the last input, or
    /// `None` while it says nothing.
    pub system_idle: Mutex<Option<u64>>,
}

#[cfg_attr(not(test), expect(dead_code, reason = "for synthetic tests and simulation"))]
impl FakeSensorDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulate Windows' idle clock (no real input is ever read).
    pub fn set_system_idle_seconds(&self, idle: Option<u64>) {
        *self.system_idle.lock().unwrap_or_else(|e| e.into_inner()) = idle;
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
        counter.set_policy(keyboard || mouse, keyboard, mouse);
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

    fn system_idle_seconds(&self) -> Option<u64> {
        *self.system_idle.lock().unwrap_or_else(|e| e.into_inner())
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

// ---- Platform-Neutral Lifecycle Controller & Delegates ----------------------

/// Pluggable hook operations used by the lifecycle controller.
#[allow(dead_code)]
pub trait HookLifecycleDelegate: Send + Sync + 'static {
    /// Prepare message queue before handshake.
    fn prepare_queue(&self) -> bool;

    /// Attach requested hooks. Returns thread id / identifier on success.
    fn attach_hooks(&self, keyboard: bool, mouse: bool) -> Result<u32, String>;

    /// Detach any active hooks.
    fn detach_hooks(&self);

    /// Post quit message to hook thread message loop. Returns true if posted successfully.
    fn post_quit(&self, thread_id: u32) -> bool;

    /// Run message loop until WM_QUIT is processed.
    fn run_loop(&self, counter: Arc<InputCounter>);
}

struct ActiveInputThread {
    thread_id: u32,
    join_handle: std::thread::JoinHandle<()>,
    keyboard: bool,
    mouse: bool,
}

/// Generic thread manager for input hooks enforcing serial transitions,
/// explicit readiness handshake, truthful running states, synchronous join on stop,
/// and safe Drop cleanup.
#[allow(dead_code)]
pub struct InputLifecycleController<D: HookLifecycleDelegate> {
    delegate: Arc<D>,
    slot: Mutex<Option<ActiveInputThread>>,
}

#[allow(dead_code)]
impl<D: HookLifecycleDelegate> InputLifecycleController<D> {
    pub fn new(delegate: Arc<D>) -> Self {
        Self { delegate, slot: Mutex::new(None) }
    }

    pub fn start(&self, counter: Arc<InputCounter>, keyboard: bool, mouse: bool) -> bool {
        let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(ref current) = *guard {
            if current.keyboard == keyboard && current.mouse == mouse {
                return true;
            }
            // Permissions changed (e.g. policy narrowing): stop old thread synchronously first
            self.stop_locked(&mut guard);
        }

        if !keyboard && !mouse {
            return true;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let delegate = self.delegate.clone();
        let counter_clone = counter.clone();

        let spawned = std::thread::Builder::new()
            .name("input-hooks".into())
            .spawn(move || {
                if !delegate.prepare_queue() {
                    let _ = tx.send(Err("failed to prepare message queue".into()));
                    return;
                }
                match delegate.attach_hooks(keyboard, mouse) {
                    Ok(thread_id) => {
                        let _ = tx.send(Ok(thread_id));
                        delegate.run_loop(counter_clone);
                        delegate.detach_hooks();
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            });

        match spawned {
            Ok(join_handle) => match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(Ok(thread_id)) => {
                    *guard = Some(ActiveInputThread {
                        thread_id,
                        join_handle,
                        keyboard,
                        mouse,
                    });
                    true
                }
                Ok(Err(e)) => {
                    tracing::error!(?e, "hook attachment failed during handshake");
                    let _ = join_handle.join();
                    false
                }
                Err(e) => {
                    tracing::error!(?e, "handshake timed out waiting for hook thread readiness");
                    let _ = join_handle.join();
                    false
                }
            },
            Err(e) => {
                tracing::error!(?e, "failed to spawn hook thread");
                false
            }
        }
    }

    pub fn stop(&self) {
        let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        self.stop_locked(&mut guard);
    }

    fn stop_locked(&self, guard: &mut Option<ActiveInputThread>) {
        if let Some(thread) = guard.take() {
            let mut posted = false;
            for _ in 0..10 {
                if self.delegate.post_quit(thread.thread_id) {
                    posted = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if !posted {
                tracing::error!("failed to post quit to hook thread");
            }
            if let Err(e) = thread.join_handle.join() {
                tracing::warn!(?e, "error joining hook thread");
            }
        }
    }

    pub fn is_running(&self) -> bool {
        let guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_some()
    }
}

impl<D: HookLifecycleDelegate> Drop for InputLifecycleController<D> {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Pluggable active-window sampling operations used by the usage lifecycle controller.
#[allow(dead_code)]
pub trait SamplerLifecycleDelegate: Send + Sync + 'static {
    fn sample_once(&self, recorder: &UsageRecorder);
}

struct ActiveUsageThread {
    stop: Arc<AtomicBool>,
    join_handle: std::thread::JoinHandle<()>,
}

/// Generic thread manager for active-window usage sampling enforcing bounded stops,
/// serialized restarts, and safe Drop cleanup.
#[allow(dead_code)]
pub struct UsageLifecycleController<D: SamplerLifecycleDelegate> {
    delegate: Arc<D>,
    slot: Mutex<Option<ActiveUsageThread>>,
}

#[allow(dead_code)]
impl<D: SamplerLifecycleDelegate> UsageLifecycleController<D> {
    pub fn new(delegate: Arc<D>) -> Self {
        Self { delegate, slot: Mutex::new(None) }
    }

    pub fn start(&self, recorder: Arc<UsageRecorder>) -> bool {
        let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return true;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let delegate = self.delegate.clone();

        let spawned = std::thread::Builder::new()
            .name("usage-sampler".into())
            .spawn(move || {
                while !stop_clone.load(Ordering::SeqCst) {
                    if recorder.is_active() {
                        delegate.sample_once(&recorder);
                    }
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
                *guard = Some(ActiveUsageThread { stop, join_handle });
                true
            }
            Err(e) => {
                tracing::error!(?e, "could not start usage sampler thread");
                false
            }
        }
    }

    pub fn stop(&self) {
        let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(thread) = guard.take() {
            thread.stop.store(true, Ordering::SeqCst);
            if let Err(e) = thread.join_handle.join() {
                tracing::warn!(?e, "error joining usage sampler thread");
            }
        }
    }

    pub fn is_running(&self) -> bool {
        let guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_some()
    }
}

impl<D: SamplerLifecycleDelegate> Drop for UsageLifecycleController<D> {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- Native Production Implementations --------------------------------------

#[cfg(windows)]
pub struct NativeSensorDriver {
    input_lock: Mutex<Option<ActiveInputThread>>,
    usage_lock: Mutex<Option<ActiveUsageThread>>,
}

#[cfg(windows)]
impl Default for NativeSensorDriver {
    fn default() -> Self {
        Self {
            input_lock: Mutex::new(None),
            usage_lock: Mutex::new(None),
        }
    }
}

#[cfg(windows)]
impl NativeSensorDriver {
    pub fn new() -> Self {
        Self::default()
    }

    fn stop_input_locked(&self, guard: &mut Option<ActiveInputThread>) {
        if let Some(thread) = guard.take() {
            let mut posted = false;
            for _ in 0..10 {
                let res = unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                        thread.thread_id,
                        windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    )
                };
                if res.is_ok() {
                    posted = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if !posted {
                tracing::error!("failed to post WM_QUIT to input hook thread");
            }
            if let Err(e) = thread.join_handle.join() {
                tracing::warn!(?e, "error joining input hook thread");
            }
        }
    }
}

#[cfg(windows)]
impl SensorDriver for NativeSensorDriver {
    fn start_input(&self, counter: Arc<InputCounter>, keyboard: bool, mouse: bool) {
        let mut guard = self.input_lock.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(ref current) = *guard {
            if current.keyboard == keyboard && current.mouse == mouse {
                return;
            }
            self.stop_input_locked(&mut guard);
        }

        if !keyboard && !mouse {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let counter_clone = counter.clone();
        let spawned = std::thread::Builder::new()
            .name("input-hooks".into())
            .spawn(move || {
                super::input::run_hooks_thread(counter_clone, keyboard, mouse, tx);
            });

        match spawned {
            Ok(join_handle) => match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(handshake) if handshake.success => {
                    *guard = Some(ActiveInputThread {
                        thread_id: handshake.thread_id,
                        join_handle,
                        keyboard,
                        mouse,
                    });
                }
                Ok(handshake) => {
                    tracing::error!(?handshake, "hook attachment failed during handshake");
                    let _ = join_handle.join();
                }
                Err(e) => {
                    tracing::error!(?e, "handshake timeout waiting for hook thread");
                    let _ = join_handle.join();
                }
            },
            Err(e) => {
                tracing::error!(?e, "could not start input hook thread");
            }
        }
    }

    fn stop_input(&self) {
        let mut guard = self.input_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.stop_input_locked(&mut guard);
    }

    fn is_input_running(&self) -> bool {
        let guard = self.input_lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_some()
    }

    fn system_idle_seconds(&self) -> Option<u64> {
        super::input::idle_seconds()
    }

    fn start_usage(&self, recorder: Arc<UsageRecorder>) {
        let mut guard = self.usage_lock.lock().unwrap_or_else(|e| e.into_inner());
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
                *guard = Some(ActiveUsageThread { stop, join_handle });
            }
            Err(e) => {
                tracing::error!(?e, "could not start usage sampler thread");
            }
        }
    }

    fn stop_usage(&self) {
        let mut guard = self.usage_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(thread) = guard.take() {
            thread.stop.store(true, Ordering::SeqCst);
            if let Err(e) = thread.join_handle.join() {
                tracing::warn!(?e, "error joining usage sampler thread");
            }
        }
    }

    fn is_usage_running(&self) -> bool {
        let guard = self.usage_lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_some()
    }
}

#[cfg(windows)]
impl Drop for NativeSensorDriver {
    fn drop(&mut self) {
        self.stop_input();
        self.stop_usage();
    }
}

#[cfg(not(windows))]
#[derive(Default)]
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
    fn is_input_running(&self) -> bool { false }
    fn system_idle_seconds(&self) -> Option<u64> { None }
    fn start_usage(&self, _recorder: Arc<UsageRecorder>) {}
    fn stop_usage(&self) {}
    fn is_usage_running(&self) -> bool { false }
}

#[cfg(test)]
pub mod tests {
    use super::*;

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

        driver.start_input(counter.clone(), true, false);
        assert!(driver.is_input_running());
        assert!(driver.last_keyboard_allowed());
        assert!(!driver.last_mouse_allowed());
        assert_eq!(driver.input_started_count(), 1);

        driver.start_input(counter.clone(), true, false);
        assert_eq!(driver.input_started_count(), 1, "idempotent start does not double count");

        driver.stop_input();
        assert!(!driver.is_input_running());
        assert_eq!(driver.input_stopped_count(), 1);

        driver.stop_input();
        assert_eq!(driver.input_stopped_count(), 1, "idempotent stop does not double count");
    }

    #[test]
    fn fake_driver_repeated_transitions_balance_counts() {
        let driver = FakeSensorDriver::new();
        let counter = Arc::new(InputCounter::new());

        for i in 1..=10 {
            driver.start_input(counter.clone(), true, true);
            assert!(driver.is_input_running());
            assert_eq!(driver.input_started_count(), i);

            driver.stop_input();
            assert!(!driver.is_input_running());
            assert_eq!(driver.input_stopped_count(), i);
        }
        assert_eq!(driver.input_started_count(), driver.input_stopped_count());
    }

    #[test]
    fn fake_driver_concurrent_barrier_transitions() {
        let driver = Arc::new(FakeSensorDriver::new());
        let counter = Arc::new(InputCounter::new());
        let barrier = Arc::new(std::sync::Barrier::new(4));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let d = driver.clone();
            let c = counter.clone();
            let b = barrier.clone();
            handles.push(std::thread::spawn(move || {
                b.wait();
                for _ in 0..50 {
                    d.start_input(c.clone(), true, true);
                    d.start_usage(Arc::new(UsageRecorder::new()));
                    d.stop_input();
                    d.stop_usage();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Final state assertion: when all threads finish their last stop, sensors must be stopped
        assert!(!driver.is_input_running(), "input must be stopped after concurrent transitions");
        assert!(!driver.is_usage_running(), "usage must be stopped after concurrent transitions");
        assert_eq!(
            driver.input_started_count(),
            driver.input_stopped_count(),
            "input starts and stops must balance exactly"
        );
        assert_eq!(
            driver.usage_started_count(),
            driver.usage_stopped_count(),
            "usage starts and stops must balance exactly"
        );
    }

    // ---- Platform-Neutral Lifecycle Controller Tests ------------------------

    #[derive(Default)]
    struct MockHookDelegate {
        prepared: AtomicBool,
        keyboard_installed: AtomicUsize,
        mouse_installed: AtomicUsize,
        keyboard_uninstalled: AtomicUsize,
        mouse_uninstalled: AtomicUsize,
        should_fail_install: AtomicBool,
        should_fail_post_quit: AtomicBool,
        quit_posted: AtomicUsize,
        loop_running: AtomicBool,
        quit_signal: Arc<AtomicBool>,
    }

    impl HookLifecycleDelegate for MockHookDelegate {
        fn prepare_queue(&self) -> bool {
            self.prepared.store(true, Ordering::SeqCst);
            true
        }

        fn attach_hooks(&self, keyboard: bool, mouse: bool) -> Result<u32, String> {
            if self.should_fail_install.load(Ordering::SeqCst) {
                return Err("simulated hook install failure".into());
            }
            if keyboard {
                self.keyboard_installed.fetch_add(1, Ordering::SeqCst);
            }
            if mouse {
                self.mouse_installed.fetch_add(1, Ordering::SeqCst);
            }
            Ok(101)
        }

        fn detach_hooks(&self) {
            self.keyboard_uninstalled.fetch_add(1, Ordering::SeqCst);
            self.mouse_uninstalled.fetch_add(1, Ordering::SeqCst);
        }

        fn post_quit(&self, _thread_id: u32) -> bool {
            if self.should_fail_post_quit.swap(false, Ordering::SeqCst) {
                // simulate 1 retry
                return false;
            }
            self.quit_posted.fetch_add(1, Ordering::SeqCst);
            self.quit_signal.store(true, Ordering::SeqCst);
            true
        }

        fn run_loop(&self, _counter: Arc<InputCounter>) {
            self.loop_running.store(true, Ordering::SeqCst);
            while !self.quit_signal.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            self.loop_running.store(false, Ordering::SeqCst);
        }
    }

    #[test]
    fn immediate_stop_before_readiness() {
        let delegate = Arc::new(MockHookDelegate::default());
        let controller = InputLifecycleController::new(delegate.clone());
        let counter = Arc::new(InputCounter::new());

        let started = controller.start(counter, true, true);
        assert!(started);
        assert!(controller.is_running());

        // Stop immediately
        controller.stop();
        assert!(!controller.is_running());
        assert!(!delegate.loop_running.load(Ordering::SeqCst));
        assert!(delegate.quit_posted.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn failed_queue_post_or_install() {
        let delegate = Arc::new(MockHookDelegate::default());
        delegate.should_fail_install.store(true, Ordering::SeqCst);

        let controller = InputLifecycleController::new(delegate.clone());
        let counter = Arc::new(InputCounter::new());

        let started = controller.start(counter, true, true);
        assert!(!started, "start must report failure when hook installation fails");
        assert!(!controller.is_running(), "running state must be false on failure");
    }

    #[test]
    fn stop_versus_concurrent_restart() {
        let delegate = Arc::new(MockHookDelegate::default());
        let controller = Arc::new(InputLifecycleController::new(delegate.clone()));
        let counter = Arc::new(InputCounter::new());

        let c1 = controller.clone();
        let cnt1 = counter.clone();
        let h1 = std::thread::spawn(move || {
            for _ in 0..20 {
                c1.start(cnt1.clone(), true, true);
                std::thread::sleep(Duration::from_millis(2));
            }
        });

        let c2 = controller.clone();
        let h2 = std::thread::spawn(move || {
            for _ in 0..20 {
                c2.stop();
                std::thread::sleep(Duration::from_millis(2));
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();

        controller.stop();
        assert!(!controller.is_running(), "controller must be cleanly stopped");
    }

    #[test]
    fn policy_narrowing_restarts_cleanly() {
        let delegate = Arc::new(MockHookDelegate::default());
        let controller = InputLifecycleController::new(delegate.clone());
        let counter = Arc::new(InputCounter::new());

        // Start with both keyboard and mouse
        assert!(controller.start(counter.clone(), true, true));
        assert!(controller.is_running());
        assert_eq!(delegate.keyboard_installed.load(Ordering::SeqCst), 1);
        assert_eq!(delegate.mouse_installed.load(Ordering::SeqCst), 1);

        // Narrow to mouse only
        assert!(controller.start(counter, false, true));
        assert!(controller.is_running());
        assert_eq!(delegate.mouse_installed.load(Ordering::SeqCst), 2);
        assert_eq!(delegate.keyboard_installed.load(Ordering::SeqCst), 1, "keyboard not installed when narrowed");

        controller.stop();
        assert!(!controller.is_running());
    }

    #[test]
    fn per_sensor_exact_install_uninstall_counts() {
        let delegate = Arc::new(MockHookDelegate::default());
        let controller = InputLifecycleController::new(delegate.clone());
        let counter = Arc::new(InputCounter::new());

        // Keyboard only
        controller.start(counter.clone(), true, false);
        assert_eq!(delegate.keyboard_installed.load(Ordering::SeqCst), 1);
        assert_eq!(delegate.mouse_installed.load(Ordering::SeqCst), 0);

        controller.stop();
        assert_eq!(delegate.keyboard_uninstalled.load(Ordering::SeqCst), 1);

        // Mouse only
        controller.start(counter, false, true);
        assert_eq!(delegate.keyboard_installed.load(Ordering::SeqCst), 1);
        assert_eq!(delegate.mouse_installed.load(Ordering::SeqCst), 1);

        controller.stop();
        assert_eq!(delegate.mouse_uninstalled.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn drop_cleanup_stops_running_thread() {
        let delegate = Arc::new(MockHookDelegate::default());
        let counter = Arc::new(InputCounter::new());
        {
            let controller = InputLifecycleController::new(delegate.clone());
            controller.start(counter, true, true);
            assert!(controller.is_running());
        } // controller dropped here
        assert!(delegate.quit_signal.load(Ordering::SeqCst), "quit must be posted upon drop");
        assert!(!delegate.loop_running.load(Ordering::SeqCst), "thread must be stopped and joined upon drop");
    }

    #[derive(Default)]
    struct MockSamplerDelegate {
        sample_count: AtomicUsize,
    }

    impl SamplerLifecycleDelegate for MockSamplerDelegate {
        fn sample_once(&self, _recorder: &UsageRecorder) {
            self.sample_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn stop_during_startup_or_in_flight_sample() {
        let delegate = Arc::new(MockSamplerDelegate::default());
        let controller = UsageLifecycleController::new(delegate.clone());
        let recorder = Arc::new(UsageRecorder::new());
        recorder.set_active(true);

        assert!(controller.start(recorder));
        assert!(controller.is_running());

        // Let one slice execute
        std::thread::sleep(Duration::from_millis(50));

        controller.stop();
        assert!(!controller.is_running());
    }
}
