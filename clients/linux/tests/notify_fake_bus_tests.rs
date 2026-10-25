#![cfg(target_os = "linux")]
//! Desktop notifications against a fake `org.freedesktop.Notifications` on a
//! private session bus: hints, actions only where supported, the resident
//! hint only with persistence, our own signals delivered to the callback and
//! other applications' ignored.

mod support;

use std::sync::mpsc;
use std::time::Duration;

use dosi_tracker_linux::agent::notify::{
    CloseReason, Notification, Notifier, NotifyError, NotifyEvent, APP_NAME,
};
use support::fakes::notifications::FakeNotifications;
use support::private_bus::PrivateTestBus;

fn notifier(bus: &PrivateTestBus) -> (Notifier, mpsc::Receiver<NotifyEvent>) {
    let (tx, rx) = mpsc::channel();
    let notifier = Notifier::connect(bus.address(), move |event| {
        let _ = tx.send(event);
    })
    .expect("connected");
    (notifier, rx)
}

#[test]
fn notifications_carry_the_desktop_entry_and_supported_actions() {
    let bus = PrivateTestBus::session();
    let server = FakeNotifications::start(bus.address(), &["body", "actions", "persistence"]);
    let (notifier, _rx) = notifier(&bus);
    assert!(notifier.supports("actions"));

    let away = Notification::new("You were away for 42 min", "10:02 - 10:44")
        .with_action("keep", "Keep as work")
        .with_action("discard", "Discard");
    let id = notifier.notify(&away).unwrap();
    let shown = server.shown();
    assert_eq!(shown.len(), 1);
    let first = &shown[0];
    assert_eq!(first.id, id);
    assert_eq!(first.app_name, APP_NAME);
    assert_eq!(first.app_icon, "io.github.dosibridge.DosiTracker");
    assert_eq!(first.summary, "You were away for 42 min");
    assert_eq!(
        first.actions,
        ["keep", "Keep as work", "discard", "Discard"]
    );
    assert_eq!(
        first.desktop_entry.as_deref(),
        Some("io.github.dosibridge.DosiTracker")
    );
    assert_eq!(first.resident, None, "only resident notices are resident");
    assert_eq!(first.urgency, Some(1));
    assert_eq!(first.expire_timeout, -1);

    let tracking = Notification::new("Dosi Tracker is tracking Website", "")
        .with_action("pause", "Pause")
        .resident();
    let resident_id = notifier.notify(&tracking).unwrap();
    let shown = server.shown();
    assert_eq!(shown[1].resident, Some(true));
    assert_eq!(shown[1].expire_timeout, 0);
    // Replacing updates the same notification.
    let again = notifier
        .notify(&tracking.clone().replacing(Some(resident_id)))
        .unwrap();
    assert_eq!(again, resident_id);
    assert_eq!(server.shown()[2].replaces_id, resident_id);
}

#[test]
fn servers_without_actions_or_persistence_get_neither() {
    let bus = PrivateTestBus::session();
    let server = FakeNotifications::start(bus.address(), &["body"]);
    let (notifier, _rx) = notifier(&bus);
    notifier
        .notify(
            &Notification::new("Tracking", "")
                .with_action("pause", "Pause")
                .resident(),
        )
        .unwrap();
    let shown = server.shown();
    assert!(
        shown[0].actions.is_empty(),
        "no buttons the server cannot show"
    );
    assert_eq!(shown[0].resident, None);
    assert_eq!(
        shown[0].desktop_entry.as_deref(),
        Some("io.github.dosibridge.DosiTracker")
    );
}

#[test]
fn our_actions_and_closes_reach_the_callback_and_others_do_not() {
    let bus = PrivateTestBus::session();
    let server = FakeNotifications::start(bus.address(), &["actions"]);
    let (notifier, rx) = notifier(&bus);
    let id = notifier
        .notify(&Notification::new("Away", "").with_action("keep", "Keep as work"))
        .unwrap();

    // Another application's notification: ignored.
    server.invoke_action(id + 100, "keep");
    server.invoke_action(id, "keep");
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        NotifyEvent::ActionInvoked {
            id,
            action: "keep".into()
        }
    );
    server.dismiss(id);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        NotifyEvent::Closed {
            id,
            reason: CloseReason::Dismissed
        }
    );
    // Closed notifications are forgotten: a late action is not delivered.
    server.invoke_action(id, "keep");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());

    let other = notifier.notify(&Notification::new("Synced", "")).unwrap();
    notifier.close(other).unwrap();
    assert_eq!(server.closed(), [other]);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        NotifyEvent::Closed {
            id: other,
            reason: CloseReason::Closed
        }
    );
}

#[test]
fn no_notification_server_is_reported_as_unavailable() {
    let bus = PrivateTestBus::session();
    let result = Notifier::connect(bus.address(), |_| {});
    assert!(
        matches!(result, Err(NotifyError::Unavailable(_))),
        "{:?}",
        result.err()
    );
}

#[test]
fn dropping_the_notifier_ends_its_listener() {
    let bus = PrivateTestBus::session();
    let server = FakeNotifications::start(bus.address(), &["actions"]);
    let (notifier, rx) = notifier(&bus);
    let id = notifier
        .notify(&Notification::new("x", "").with_action("a", "A"))
        .unwrap();
    drop(notifier);
    server.invoke_action(id, "a");
    // The callback (and its sender) went away with the listener thread.
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
}
