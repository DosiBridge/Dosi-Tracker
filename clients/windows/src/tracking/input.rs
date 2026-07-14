use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Thread-safe counters for keyboard hits and mouse clicks.
///
/// We only keep counts (never keystrokes content) to respect privacy and to
/// keep the footprint tiny.
#[derive(Default)]
pub struct InputCounter {
    keyboard: AtomicU64,
    mouse: AtomicU64,
}

impl InputCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_keyboard(&self) {
        self.keyboard.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_mouse(&self) {
        self.mouse.fetch_add(1, Ordering::Relaxed);
    }

    /// Read counts and reset them to zero. Returns (keyboard_hits, mouse_clicks).
    pub fn take(&self) -> (u64, u64) {
        (
            self.keyboard.swap(0, Ordering::Relaxed),
            self.mouse.swap(0, Ordering::Relaxed),
        )
    }
}

/// Spawn a background thread with a global, event-driven input hook.
/// Uses `rdev::listen`, which blocks on the OS event queue (no polling).
pub fn spawn_listener(counter: Arc<InputCounter>) {
    std::thread::spawn(move || {
        let callback = move |event: rdev::Event| match event.event_type {
            rdev::EventType::KeyPress(_) => counter.inc_keyboard(),
            rdev::EventType::ButtonPress(_) => counter.inc_mouse(),
            _ => {}
        };

        if let Err(e) = rdev::listen(callback) {
            tracing::error!(?e, "global input listener stopped");
        }
    });
}
