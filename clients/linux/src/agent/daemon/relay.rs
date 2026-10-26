//! The notice relay (design 13): new [`Notice`]s of the snapshot become
//! desktop notifications, the away question gets Keep/Discard actions when
//! the server offers actions, consent widenings post a receipt, and without
//! a tray a resident "Dosi Tracker is tracking <project>" notification stays
//! up while tracking (invariant I5). It also keeps the logind delay
//! inhibitor in step with tracking and reports the status to systemd.
//!
//! Runs on `dosi-relay`; the notification server's `ActionInvoked` signals
//! come back as [`RelayMessage::Notification`] and turn into worker commands.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use super::CommandSink;
use crate::agent::notify::{Notification, Notifier, NotifyEvent, Urgency};
use crate::agent::session_events::InhibitorControl;
use crate::agent::state::{Command, Notice, SharedState, StateHandle, TrackerStatus};

/// How often the relay looks at the snapshot.
const POLL: Duration = Duration::from_millis(250);

/// Notices older than this when the relay first sees them are not shown.
const STALE_NOTICE: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// Notification action keys.
pub const ACTION_KEEP: &str = "keep";
pub const ACTION_DISCARD: &str = "discard";
pub const ACTION_PAUSE: &str = "pause";
pub const ACTION_OPEN: &str = "open";

/// What the relay is told besides the snapshot.
#[derive(Debug)]
pub enum RelayMessage {
    /// A signal about one of our notifications.
    Notification(NotifyEvent),
    /// Show a consent receipt (`Screenshots were turned on ... from the
    /// command line`).
    Receipt(String),
    Stop,
}

/// Everything the relay needs.
pub struct RelayConfig {
    pub state: StateHandle,
    pub sink: CommandSink,
    pub notifier: Option<Arc<Notifier>>,
    pub inhibitor: Option<InhibitorControl>,
    pub notifications_enabled: bool,
    pub rx: Receiver<RelayMessage>,
    /// `dosi-tracker-gui`, for the Open action.
    pub gui_exe: Option<PathBuf>,
    /// The machine's clock.
    pub clock: super::Clock,
}

/// The running relay.
pub struct RelayHandle {
    tx: Sender<RelayMessage>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Sends receipts to the relay; cheap to clone.
#[derive(Clone)]
pub struct RelaySender(Option<Sender<RelayMessage>>);

impl RelaySender {
    /// A sender that drops everything (tests).
    pub fn none() -> Self {
        Self(None)
    }

    pub fn receipt(&self, text: String) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(RelayMessage::Receipt(text));
        }
    }
}

impl RelayHandle {
    pub fn sender(&self) -> RelaySender {
        RelaySender(Some(self.tx.clone()))
    }

    pub fn stop(mut self) {
        let _ = self.tx.send(RelayMessage::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start `dosi-relay`. Without the thread the daemon still runs (no
/// notifications).
pub fn spawn(config: RelayConfig) -> RelayHandle {
    let (tx, own_rx) = std::sync::mpsc::channel::<RelayMessage>();
    // Messages from the notifier callback arrive on `config.rx`; ours (the
    // receipts, stop) on `own_rx`. One forwarding thread merges them.
    let forward_tx = tx.clone();
    let RelayConfig {
        state,
        sink,
        notifier,
        inhibitor,
        notifications_enabled,
        rx,
        gui_exe,
        clock,
    } = config;
    let _ = std::thread::Builder::new()
        .name("dosi-relay-rx".into())
        .spawn(move || {
            while let Ok(message) = rx.recv() {
                if forward_tx.send(message).is_err() {
                    break;
                }
            }
        });
    let mut relay = Relay {
        state,
        sink,
        notifier: notifier.filter(|_| notifications_enabled),
        inhibitor,
        gui_exe,
        clock,
        last_notice: None,
        away: None,
        resident: None,
        dismissed: None,
        status: None,
        tracking: None,
    };
    let thread = std::thread::Builder::new()
        .name("dosi-relay".into())
        .spawn(move || relay.run(own_rx))
        .map_err(|e| tracing::warn!(error = %e, "no notice relay"))
        .ok();
    RelayHandle { tx, thread }
}

struct Relay {
    state: StateHandle,
    sink: CommandSink,
    notifier: Option<Arc<Notifier>>,
    inhibitor: Option<InhibitorControl>,
    gui_exe: Option<PathBuf>,
    clock: super::Clock,
    /// The newest notice shown.
    last_notice: Option<u64>,
    /// The away question's notification and the stretch it asks about.
    away: Option<(u32, chrono::DateTime<Utc>)>,
    /// The resident tracking notification and the project it names.
    resident: Option<(u32, String)>,
    /// The user dismissed the resident notification: it comes back when
    /// tracking starts again or the project changes.
    dismissed: Option<String>,
    status: Option<TrackerStatus>,
    tracking: Option<bool>,
}

impl Relay {
    fn run(&mut self, rx: Receiver<RelayMessage>) {
        let mut seen = u64::MAX;
        loop {
            match rx.recv_timeout(POLL) {
                Ok(RelayMessage::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(RelayMessage::Notification(event)) => self.on_event(event),
                Ok(RelayMessage::Receipt(text)) => {
                    self.show(Notification::new("Dosi Tracker", text));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            let version = self.state.version();
            if version != seen {
                seen = version;
                let snapshot = self.state.snapshot();
                self.apply(&snapshot);
            }
        }
        if let (Some(notifier), Some((id, _))) = (&self.notifier, self.resident.take()) {
            let _ = notifier.close(id);
        }
    }

    fn show(&self, notification: Notification) -> Option<u32> {
        let notifier = self.notifier.as_ref()?;
        match notifier.notify(&notification) {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::debug!(error = %e, "notification not shown");
                None
            }
        }
    }

    fn apply(&mut self, snapshot: &SharedState) {
        if self.status != Some(snapshot.status) {
            self.status = Some(snapshot.status);
            let _ = sd_notify::notify(
                false,
                &[sd_notify::NotifyState::Status(snapshot.status.label())],
            );
        }
        let tracking = snapshot.status == TrackerStatus::Tracking;
        if self.tracking != Some(tracking) {
            self.tracking = Some(tracking);
            if let Some(inhibitor) = &self.inhibitor {
                inhibitor.set_tracking(tracking);
            }
        }
        self.notices(snapshot);
        self.away_question(snapshot);
        self.resident_notification(snapshot, tracking);
    }

    fn notices(&mut self, snapshot: &SharedState) {
        let newest = snapshot.notices.last().map(|n| n.id);
        let first = self.last_notice.is_none();
        let fresh: Vec<&Notice> = snapshot
            .notices
            .iter()
            .filter(|n| self.last_notice.is_none_or(|last| n.id > last))
            .filter(|n| !first || (self.clock)() - n.at < STALE_NOTICE)
            .collect();
        for notice in fresh {
            self.show(Notification::new("Dosi Tracker", notice.text()));
        }
        if newest.is_some() {
            self.last_notice = newest;
        } else if first {
            self.last_notice = Some(0);
        }
    }

    fn away_question(&mut self, snapshot: &SharedState) {
        match (&snapshot.away_prompt, self.away) {
            (Some(prompt), current) if current.map(|(_, from)| from) != Some(prompt.from) => {
                if let (Some(notifier), Some((id, _))) = (&self.notifier, current) {
                    let _ = notifier.close(id);
                }
                let text = format!(
                    "You were away for {} min ({} - {}). Keep it as work on {}?",
                    prompt.minutes(),
                    prompt.from.with_timezone(&chrono::Local).format("%H:%M"),
                    prompt.to.with_timezone(&chrono::Local).format("%H:%M"),
                    prompt.project_title
                );
                let notification = Notification::new("Dosi Tracker", text)
                    .with_action(ACTION_KEEP, "Keep as work")
                    .with_action(ACTION_DISCARD, "Discard")
                    .resident()
                    .urgency(Urgency::Normal);
                self.away = self.show(notification).map(|id| (id, prompt.from));
            }
            (None, Some((id, _))) => {
                if let Some(notifier) = &self.notifier {
                    let _ = notifier.close(id);
                }
                self.away = None;
            }
            _ => {}
        }
    }

    fn resident_notification(&mut self, snapshot: &SharedState, tracking: bool) {
        let wanted = (tracking && !snapshot.indicator.tray)
            .then(|| snapshot.current_project().map(|p| p.title.clone()))
            .flatten();
        if wanted.is_none() || wanted != self.dismissed {
            self.dismissed = None;
        }
        let wanted = wanted.filter(|_| self.dismissed.is_none());
        match (wanted, self.resident.clone()) {
            (Some(project), current) if current.as_ref().map(|(_, p)| p) != Some(&project) => {
                let notification = Notification::new(
                    "Dosi Tracker",
                    format!("Dosi Tracker is tracking {project}"),
                )
                .with_action(ACTION_PAUSE, "Pause")
                .with_action(ACTION_OPEN, "Open")
                .resident()
                .urgency(Urgency::Low)
                .replacing(current.map(|(id, _)| id));
                self.resident = self.show(notification).map(|id| (id, project));
            }
            (None, Some((id, _))) => {
                if let Some(notifier) = &self.notifier {
                    let _ = notifier.close(id);
                }
                self.resident = None;
            }
            _ => {}
        }
    }

    fn on_event(&mut self, event: NotifyEvent) {
        match event {
            NotifyEvent::ActionInvoked { id, action } => match action.as_str() {
                ACTION_KEEP | ACTION_DISCARD if self.away.is_some_and(|(away, _)| away == id) => {
                    self.away = None;
                    self.sink.send(Command::ResolveAway {
                        keep: action == ACTION_KEEP,
                    });
                    // The question is resident: answered, it goes.
                    if let Some(notifier) = &self.notifier {
                        let _ = notifier.close(id);
                    }
                }
                ACTION_PAUSE => {
                    self.sink.send(Command::Pause {
                        resume_after: None,
                        at: (self.clock)(),
                    });
                }
                ACTION_OPEN => {
                    if let Some(gui) = &self.gui_exe {
                        if let Err(e) = std::process::Command::new(gui)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                        {
                            tracing::warn!(error = %e, "could not open the window");
                        }
                    }
                }
                _ => {}
            },
            NotifyEvent::Closed { id, .. } => {
                if let Some((_, project)) = self.resident.take_if(|(r, _)| *r == id) {
                    self.dismissed = Some(project);
                }
                if self.away.is_some_and(|(a, _)| a == id) {
                    self.away = None;
                }
            }
        }
    }
}
