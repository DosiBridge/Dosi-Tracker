#![cfg(all(target_os = "linux", feature = "tray"))]
//! The daemon's StatusNotifierItem against a fake
//! `org.kde.StatusNotifierWatcher` on a private session bus: the item
//! registers; its Id, Title, Status, ToolTip and IconPixmap (sizes, ARGB
//! order) are what the view says; the DBusMenu layout matches the menu; a
//! click on Pause sends `Command::Pause`; and losing the watcher turns
//! `indicator.tray` off (and finding one again turns it back on).

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use dosi_tracker_linux::agent::daemon::CommandSink;
use dosi_tracker_linux::agent::state::{
    AwayPrompt, Command, ProjectOption, SharedState, StateHandle, TrackerStatus,
};
use dosi_tracker_linux::agent::tray::{self, TrayActions, TrayConfig, TrayHandle, ICON_SIZES};
use support::fakes::sni_watcher::{self, FakeWatcher};
use support::private_bus::PrivateTestBus;

/// ksni connects to `DBUS_SESSION_BUS_ADDRESS`: setting it and connecting
/// happen under this lock, so parallel tests each reach their own bus.
static SESSION_ENV: Mutex<()> = Mutex::new(());

const WAIT: Duration = Duration::from_secs(10);

fn tracking() -> SharedState {
    let now = Utc::now();
    SharedState {
        status: TrackerStatus::Tracking,
        display_name: "ayesha@acme.test".into(),
        projects: vec![
            ProjectOption {
                id: "p1".into(),
                title: "Website".into(),
            },
            ProjectOption {
                id: "p2".into(),
                title: "Mobile".into(),
            },
        ],
        selected_project: Some("p1".into()),
        tracking_since: Some(now - chrono::Duration::minutes(5)),
        ..SharedState::default()
    }
}

struct Tray {
    state: StateHandle,
    handle: Option<TrayHandle>,
    commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
}

impl Tray {
    fn command(&mut self) -> Option<Command> {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(command) = self.commands.try_recv() {
                return Some(command);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }
}

fn spawn_tray(bus: &PrivateTestBus, snapshot: SharedState) -> Tray {
    let state = StateHandle::new();
    state.update(|s| *s = snapshot);
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let actions = TrayActions {
        sink: CommandSink::new(tx),
        clock: Arc::new(Utc::now),
        open_gui: Arc::new(|_| {}),
        open_url: Arc::new(|_| {}),
        set_autostart: Arc::new(|on| on),
        autostart_enabled: Arc::new(|| false),
        dashboard_url: Some("https://app.acme.test".into()),
    };
    let handle = {
        let _env = SESSION_ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        tray::spawn(TrayConfig {
            state: state.clone(),
            actions,
        })
        .expect("the tray starts")
    };
    Tray {
        state,
        handle: Some(handle),
        commands,
    }
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn the_item_registers_with_its_identity_icon_and_tooltip() {
    let bus = PrivateTestBus::session();
    let watcher = FakeWatcher::start(bus.address());
    let tray = spawn_tray(&bus, tracking());
    let item = watcher.wait_item(WAIT).expect("the item registered");
    assert!(item.starts_with("org.kde.StatusNotifierItem-"), "{item}");
    assert!(tray.state.snapshot().indicator.tray, "a host shows it");

    let host = sni_watcher::host(bus.address());
    let get = |name| sni_watcher::item_property(&host, &item, name);
    assert_eq!(
        sni_watcher::string(&get("Id")),
        "io.github.dosibridge.DosiTracker"
    );
    assert_eq!(sni_watcher::string(&get("Title")), "Dosi Tracker");
    assert_eq!(sni_watcher::string(&get("Status")), "Active");
    let (_, title, description) = sni_watcher::tooltip(&get("ToolTip"));
    assert_eq!(title, "Dosi Tracker");
    assert!(description.starts_with("Tracking · 5m"), "{description}");
    assert!(description.contains("Website · Today"), "{description}");

    let pixmaps = sni_watcher::pixmaps(&get("IconPixmap"));
    let sizes: Vec<i32> = pixmaps.iter().map(|(w, _, _)| *w).collect();
    assert_eq!(sizes, ICON_SIZES.map(|s| s as i32));
    for (w, h, data) in &pixmaps {
        assert_eq!(w, h);
        assert_eq!(data.len(), (w * h * 4) as usize);
        assert_eq!(
            data[0], 0,
            "ARGB: the transparent corner's alpha comes first"
        );
    }
    // The green tracking badge at the bottom right of the 32 px pixmap.
    let (_, _, data) = pixmaps.iter().find(|(w, _, _)| *w == 32).unwrap();
    let at = (25 * 32 + 25) * 4;
    assert_eq!(&data[at..at + 4], &[0xff, 0x00, 0xa3, 0x89]);
}

#[test]
fn the_menu_layout_matches_the_tray_menu() {
    let bus = PrivateTestBus::session();
    let watcher = FakeWatcher::start(bus.address());
    let _tray = spawn_tray(&bus, tracking());
    let item = watcher.wait_item(WAIT).expect("the item registered");
    let host = sni_watcher::host(bus.address());
    let menu = sni_watcher::menu_layout(&host, &item);
    let labels = menu.labels();
    assert!(
        labels[0].starts_with("● Tracking · 5m — Website"),
        "{labels:?}"
    );
    assert!(labels[1].starts_with("Today "), "{labels:?}");
    assert_eq!(
        labels[2..],
        [
            "",
            "Open Dosi Tracker",
            "",
            "Pause tracking",
            "Take a break",
            "Switch project",
            "Set task…",
            "Sync now",
            "",
            "Open web dashboard",
            "Settings…",
            "Start at login",
            "",
            "Quit Dosi Tracker",
        ]
    );
    assert!(!menu.children[0].enabled, "the status line is information");
    let breaks = menu.find("Take a break").unwrap();
    assert_eq!(breaks.labels(), ["15 minutes", "30 minutes", "1 hour"]);
    let projects = menu.find("Switch project").unwrap();
    assert_eq!(projects.labels(), ["Website", "Mobile"]);
    assert_eq!(
        projects.children[0].checked,
        Some(true),
        "the tracked project"
    );
    assert_eq!(projects.children[1].checked, Some(false));
    assert_eq!(menu.find("Start at login").unwrap().checked, Some(false));
}

#[test]
fn a_click_on_pause_sends_the_pause_command() {
    let bus = PrivateTestBus::session();
    let watcher = FakeWatcher::start(bus.address());
    let mut tray = spawn_tray(&bus, tracking());
    let item = watcher.wait_item(WAIT).expect("the item registered");
    let host = sni_watcher::host(bus.address());
    let menu = sni_watcher::menu_layout(&host, &item);
    let pause = menu.find("Pause tracking").expect("a pause item").id;
    sni_watcher::click(&host, &item, pause);
    match tray.command() {
        Some(Command::Pause { resume_after, .. }) => assert_eq!(resume_after, None),
        other => panic!("expected Pause, got {other:?}"),
    }

    // A break is a pause that resumes by itself.
    let half_hour = menu.find("30 minutes").expect("a break item").id;
    sni_watcher::click(&host, &item, half_hour);
    match tray.command() {
        Some(Command::Pause { resume_after, .. }) => {
            assert_eq!(resume_after, Some(Duration::from_secs(1800)))
        }
        other => panic!("expected a timed pause, got {other:?}"),
    }
    let mobile = menu.find("Mobile").expect("a project item").id;
    sni_watcher::click(&host, &item, mobile);
    match tray.command() {
        Some(Command::SelectProject { project_id, .. }) => assert_eq!(project_id, "p2"),
        other => panic!("expected SelectProject, got {other:?}"),
    }
}

#[test]
fn the_item_follows_the_state() {
    let bus = PrivateTestBus::session();
    let watcher = FakeWatcher::start(bus.address());
    let tray = spawn_tray(&bus, tracking());
    let item = watcher.wait_item(WAIT).expect("the item registered");
    let host = sni_watcher::host(bus.address());
    tray.state.update(|s| {
        s.status = TrackerStatus::Paused;
        s.paused_since = Some(Utc::now());
    });
    wait_until("Resume tracking in the menu", || {
        sni_watcher::menu_layout(&host, &item)
            .find("Resume tracking")
            .is_some()
    });
    assert!(sni_watcher::menu_layout(&host, &item)
        .find("Upload queued")
        .is_some());
    let now = Utc::now();
    tray.state.update(|s| {
        s.away_prompt = Some(AwayPrompt {
            from: now - chrono::Duration::minutes(30),
            to: now,
            project_title: "Website".into(),
        })
    });
    wait_until("NeedsAttention for the away question", || {
        sni_watcher::string(&sni_watcher::item_property(&host, &item, "Status")) == "NeedsAttention"
    });
}

#[test]
fn losing_the_watcher_turns_the_indicator_off_and_back_on() {
    let bus = PrivateTestBus::session();
    let watcher = FakeWatcher::start(bus.address());
    let tray = spawn_tray(&bus, tracking());
    watcher.wait_item(WAIT).expect("the item registered");
    assert!(tray.state.snapshot().indicator.tray);

    drop(watcher);
    wait_until("indicator.tray false", || {
        !tray.state.snapshot().indicator.tray
    });

    // A host comes back (a panel restarted): the item registers again.
    let watcher = FakeWatcher::start(bus.address());
    watcher.wait_item(WAIT).expect("registered again");
    wait_until("indicator.tray true", || {
        tray.state.snapshot().indicator.tray
    });
}

#[test]
fn without_a_watcher_the_item_waits_for_one() {
    let bus = PrivateTestBus::session();
    let tray = spawn_tray(&bus, tracking());
    assert!(
        !tray.state.snapshot().indicator.tray,
        "no host: the resident notification stands in"
    );
    let watcher = FakeWatcher::start(bus.address());
    watcher
        .wait_item(WAIT)
        .expect("the item registered once a watcher came");
    wait_until("indicator.tray true", || {
        tray.state.snapshot().indicator.tray
    });
}
