//! Deterministic integration tests for Linux Phase 2 native capture adapters.
//!
//! Enforces:
//! - Default-deny: absent or unconfigured consent fails closed without touching OS/display/proc.
//! - Capability-specific consent: granting active_window does NOT grant running_programs (and vice-versa).
//! - In-flight ABA generation invalidation: revoking or updating consent mid-acquisition discards telemetry.
//! - Honest Wayland unsupported behavior: reports unsupported without synthetic or fake telemetry.
//! - Pure `/proc` parsing, path stripping, kernel thread filtering, case-insensitive deduplication, and 200 cap.
//! - X11 EWMH title, WM_CLASS fallback, PID resolution, and unfocused handling.
//! - Truthful doctor capability matrix diagnosis.

use std::sync::{Arc, Mutex};

use dosi_tracker_linux::consent::{ConsentManager, ConsentedPermissions, LocalConsent};
use dosi_tracker_linux::diagnostics::{
    ArchitecturalSupport, DiagnosticStatus, DoctorReport, SessionEnvironment, SessionType,
};
use dosi_tracker_linux::lifecycle::{AcquisitionCapability, PauseReason, TrackerLifecycle};
use dosi_tracker_linux::tracking::{
    file_name, program_list, AcquisitionProviderExt, ActiveWindowProvider, ConsentGatedTracker,
    FakeAcquisitionProvider, MockProcFsReader, MockX11PropertyReader, ProcFsRunningAppsProvider,
    RunningAppsProvider, TrackingError, WaylandActiveWindowProvider, WindowInfo,
    X11ActiveWindowProvider, MAX_RUNNING_PROGRAMS,
};

fn create_test_lifecycle() -> Arc<Mutex<TrackerLifecycle>> {
    let consent_mgr = ConsentManager::new(LocalConsent::Absent, None);
    let lifecycle = TrackerLifecycle::new(consent_mgr);
    Arc::new(Mutex::new(lifecycle))
}

#[test]
fn test_default_deny_unconfigured_and_stopped_refuse_acquisition() {
    let lifecycle = create_test_lifecycle();
    let fake = Arc::new(FakeAcquisitionProvider::new());
    let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle);

    // Initial state: Unconfigured -> ConsentRequired
    assert_eq!(
        tracker.current_active_window(),
        Err(TrackingError::ConsentRequired)
    );
    assert_eq!(
        tracker.running_programs(),
        Err(TrackingError::ConsentRequired)
    );

    // Zero calls passed to underlying provider
    assert_eq!(fake.window_calls(), 0);
    assert_eq!(fake.process_calls(), 0);
}

#[test]
fn test_capability_specific_consent_granting_window_denies_processes() {
    let lifecycle = create_test_lifecycle();
    let fake = Arc::new(FakeAcquisitionProvider::new());
    fake.set_canned_window(Some(WindowInfo {
        app_name: "code".into(),
        window_title: "main.rs".into(),
        ..Default::default()
    }));

    let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());

    let policy = ConsentedPermissions {
        active_window: true,
        running_programs: true,
        ..ConsentedPermissions::all_disabled()
    };
    let local = ConsentedPermissions {
        active_window: true,
        running_programs: false, // Denied locally
        ..ConsentedPermissions::all_disabled()
    };

    {
        let mut lc = lifecycle.lock().unwrap();
        lc.configure(policy);
        lc.update_consent(LocalConsent::Granted(local)).unwrap();
        lc.start().unwrap();
    }

    // Active window query succeeds
    let win = tracker
        .current_active_window()
        .expect("active window must succeed")
        .expect("must return window info");
    assert_eq!(win.app_name, "code");
    assert_eq!(win.window_title, "main.rs");
    assert_eq!(fake.window_calls(), 1);

    // Running programs query fails closed with CapabilityDenied
    let err = tracker.running_programs();
    assert_eq!(
        err,
        Err(TrackingError::CapabilityDenied(
            AcquisitionCapability::RunningPrograms
        ))
    );
    assert_eq!(fake.process_calls(), 0);
}

#[test]
fn test_capability_specific_consent_granting_processes_denies_window() {
    let lifecycle = create_test_lifecycle();
    let fake = Arc::new(FakeAcquisitionProvider::new());
    fake.set_canned_processes(Some(vec![WindowInfo {
        app_name: "slack".into(),
        window_title: String::new(),
        ..Default::default()
    }]));

    let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());

    let policy = ConsentedPermissions {
        active_window: true,
        running_programs: true,
        ..ConsentedPermissions::all_disabled()
    };
    let local = ConsentedPermissions {
        active_window: false, // Denied locally
        running_programs: true,
        ..ConsentedPermissions::all_disabled()
    };

    {
        let mut lc = lifecycle.lock().unwrap();
        lc.configure(policy);
        lc.update_consent(LocalConsent::Granted(local)).unwrap();
        lc.start().unwrap();
    }

    // Active window fails closed with CapabilityDenied
    let err = tracker.current_active_window();
    assert_eq!(
        err,
        Err(TrackingError::CapabilityDenied(
            AcquisitionCapability::ActiveWindow
        ))
    );
    assert_eq!(fake.window_calls(), 0);

    // Running programs succeeds
    let procs = tracker
        .running_programs()
        .expect("running programs must succeed");
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].app_name, "slack");
    assert_eq!(fake.process_calls(), 1);
}

#[test]
fn test_revocation_and_in_flight_aba_invalidation() {
    let lifecycle = create_test_lifecycle();
    let fake = Arc::new(FakeAcquisitionProvider::new());

    let policy = ConsentedPermissions {
        active_window: true,
        running_programs: true,
        ..ConsentedPermissions::all_disabled()
    };
    let local = ConsentedPermissions {
        active_window: true,
        running_programs: true,
        ..ConsentedPermissions::all_disabled()
    };

    {
        let mut lc = lifecycle.lock().unwrap();
        lc.configure(policy);
        lc.update_consent(LocalConsent::Granted(local)).unwrap();
        lc.start().unwrap();
    }

    // Case 1: In-flight window acquisition invalidated by pause
    let lc_hook1 = lifecycle.clone();
    fake.set_on_current_window(Box::new(move || {
        let mut lc = lc_hook1.lock().unwrap();
        lc.pause(PauseReason::User).unwrap();
    }));

    let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());
    let res = tracker.current_active_window();
    assert!(matches!(
        res,
        Err(TrackingError::GenerationMismatch { .. }) | Err(TrackingError::ConsentRequired)
    ));

    // Re-start tracking with new generation
    {
        let mut lc = lifecycle.lock().unwrap();
        lc.start().unwrap();
    }

    // Case 2: In-flight process acquisition invalidated by full consent revocation
    let lc_hook2 = lifecycle.clone();
    fake.set_on_running_programs(Box::new(move || {
        let mut lc = lc_hook2.lock().unwrap();
        lc.update_consent(LocalConsent::Revoked).unwrap();
    }));

    let res_proc = tracker.running_programs();
    assert!(matches!(
        res_proc,
        Err(TrackingError::GenerationMismatch { .. }) | Err(TrackingError::ConsentRequired)
    ));
}

#[test]
fn test_wayland_active_window_honest_unsupported_behavior() {
    let wayland = WaylandActiveWindowProvider::new();
    let res = wayland.current_active_window();

    match res {
        Err(TrackingError::UnsupportedSession(reason)) => {
            assert!(reason.contains("unprivileged Wayland sessions"));
        }
        _ => panic!("Wayland provider must return TrackingError::UnsupportedSession"),
    }

    // Ergonomic helper current_window() returns None without panic or fabrication
    struct CombinedWayland(WaylandActiveWindowProvider, FakeAcquisitionProvider);
    impl ActiveWindowProvider for CombinedWayland {
        fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
            self.0.current_active_window()
        }
    }
    impl RunningAppsProvider for CombinedWayland {
        fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError> {
            self.1.running_programs()
        }
    }

    let combined = CombinedWayland(wayland, FakeAcquisitionProvider::new());
    assert_eq!(combined.current_window(), None);
}

#[test]
fn test_procfs_parsing_aggregation_filtering_and_max_cap() {
    let mock = Arc::new(MockProcFsReader::new());

    // 1. App with exe symlink
    mock.add_process(
        101,
        "code",
        Some(vec!["/usr/bin/code".into(), "--no-sandbox".into()]),
        Some("/usr/share/code/bin/code".into()),
    );

    // 2. Case-insensitive duplicate (should be dropped)
    mock.add_process(
        102,
        "CODE",
        Some(vec!["/usr/bin/code".into()]),
        Some("/usr/share/code/bin/code".into()),
    );

    // 3. App with cmdline argv[0] path
    mock.add_process(
        201,
        "chrome",
        Some(vec!["/opt/google/chrome/google-chrome".into()]),
        None,
    );

    // 4. App with comm only
    mock.add_process(301, "slack", None, None);

    // 5. Kernel thread in brackets (must be filtered out)
    mock.add_process(2, "[kworker/u16:0]", None, None);
    mock.add_process(3, "[migration/0]", None, None);

    // 6. Empty name / whitespace entry (must be filtered out)
    mock.add_process(401, "   ", None, None);

    let provider = ProcFsRunningAppsProvider::new(mock.clone());
    let procs = provider
        .running_programs()
        .expect("procfs scan must succeed");

    let names: Vec<&str> = procs.iter().map(|p| p.app_name.as_str()).collect();
    assert_eq!(names, vec!["code", "google-chrome", "slack"]);

    // Privacy invariant: titles must be empty and metrics None
    for p in &procs {
        assert!(p.window_title.is_empty());
        assert!(p.seconds.is_none());
        assert!(p.keyboard_hits.is_none());
        assert!(p.mouse_clicks.is_none());
    }

    // 7. Cap at MAX_RUNNING_PROGRAMS (200)
    for i in 1000..1350 {
        mock.add_process(
            i,
            format!("daemon_{}", i),
            None,
            Some(format!("/usr/sbin/daemon_{}", i)),
        );
    }
    let capped_procs = provider.running_programs().expect("must succeed");
    assert_eq!(capped_procs.len(), MAX_RUNNING_PROGRAMS);
}

#[test]
fn test_procfs_error_resilience_and_vanished_pids() {
    let mock = Arc::new(MockProcFsReader::new());

    // Process 500 will list in list_pids, but its comm/cmdline/exe are None (simulating exit)
    mock.add_process(500, "", None, None);
    mock.add_process(501, "valid_app", None, Some("/usr/bin/valid_app".into()));

    let provider = ProcFsRunningAppsProvider::new(mock.clone());
    let procs = provider.running_programs().expect("must succeed");
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].app_name, "valid_app");

    // Total I/O failure on list_pids
    mock.set_fail_on_list(Some("Disk I/O error on /proc".into()));
    let err = provider.running_programs();
    assert!(matches!(err, Err(TrackingError::IoError(_))));
}

#[test]
fn test_x11_ewmh_title_wm_class_pid_and_unfocused_handling() {
    let mock_x11 = Arc::new(MockX11PropertyReader::new());
    let mock_proc = Arc::new(MockProcFsReader::new());

    // Case 1: Unfocused (None) or desktop focused (0)
    mock_x11.set_active_window(None);
    let provider = X11ActiveWindowProvider::new(mock_x11.clone(), Some(mock_proc.clone()));
    assert_eq!(provider.current_active_window().unwrap(), None);

    mock_x11.set_active_window(Some(0));
    assert_eq!(provider.current_active_window().unwrap(), None);

    // Case 2: Focused window with PID mapped through /proc
    let xid = 0x2400001;
    let pid = 8899;
    mock_x11.set_active_window(Some(xid));
    mock_x11.set_window_info(
        xid,
        "Dosi-Tracker Linux Specification",
        "dosi-tracker-linux",
        "Dosi-Tracker",
        Some(pid),
    );
    mock_proc.add_process(
        pid,
        "dosi-tracker",
        Some(vec!["/usr/bin/dosi-tracker-linux".into()]),
        Some("/usr/bin/dosi-tracker-linux".into()),
    );

    let win = provider
        .current_active_window()
        .unwrap()
        .expect("must return active window");
    assert_eq!(win.app_name, "dosi-tracker-linux");
    assert_eq!(win.window_title, "Dosi-Tracker Linux Specification");

    // Case 3: Focused window with missing PID, falling back to WM_CLASS
    let xid2 = 0x2400002;
    mock_x11.set_active_window(Some(xid2));
    mock_x11.set_window_info(xid2, "Terminal - user@box", "xterm", "XTerm", None);

    let win2 = provider
        .current_active_window()
        .unwrap()
        .expect("must return window");
    assert_eq!(win2.app_name, "XTerm");
    assert_eq!(win2.window_title, "Terminal - user@box");

    // Case 4: X11 communication error propagates truthfully
    mock_x11.set_error(Some(TrackingError::DisplayUnavailable(
        "X11 connection socket closed".into(),
    )));
    let err = provider.current_active_window();
    assert!(matches!(err, Err(TrackingError::DisplayUnavailable(_))));
}

#[test]
fn test_doctor_reports_supported_status_for_x11_and_procfs() {
    // Injected X11 session
    let x11_env = SessionEnvironment {
        xdg_session_type: Some("x11".into()),
        display: Some(":0".into()),
        ..Default::default()
    };
    let x11_report = DoctorReport::generate(&x11_env);
    assert_eq!(x11_report.session, SessionType::X11);
    assert_eq!(
        x11_report.capabilities.active_window.adapter_status,
        DiagnosticStatus::Supported
    );
    assert_eq!(
        x11_report.capabilities.active_window.architectural_support,
        ArchitecturalSupport::SupportedUnconfined
    );
    assert_eq!(
        x11_report.capabilities.running_programs.adapter_status,
        DiagnosticStatus::Supported
    );
    assert_eq!(
        x11_report
            .capabilities
            .running_programs
            .architectural_support,
        ArchitecturalSupport::SupportedProcFs
    );

    // Injected Wayland session: honest Unsupported
    let wayland_env = SessionEnvironment {
        xdg_session_type: Some("wayland".into()),
        wayland_display: Some("wayland-0".into()),
        ..Default::default()
    };
    let wayland_report = DoctorReport::generate(&wayland_env);
    assert_eq!(wayland_report.session, SessionType::Wayland);
    assert_eq!(
        wayland_report.capabilities.active_window.adapter_status,
        DiagnosticStatus::Unsupported
    );
    assert_eq!(
        wayland_report
            .capabilities
            .active_window
            .architectural_support,
        ArchitecturalSupport::UnsupportedOnWayland
    );
    assert_eq!(
        wayland_report.capabilities.running_programs.adapter_status,
        DiagnosticStatus::Supported
    );
}

#[test]
fn test_backend_dto_compatibility_and_nullability() {
    let win = WindowInfo {
        app_name: "code".into(),
        window_title: "main.rs".into(),
        seconds: None,
        keyboard_hits: None,
        mouse_clicks: None,
    };

    let json = serde_json::to_string(&win).expect("serialization must succeed");
    assert!(json.contains("\"appName\":\"code\""));
    assert!(json.contains("\"windowTitle\":\"main.rs\""));
    // Optional usage metrics omitted when null/None (preserving CONTRACT §1 shape)
    assert!(!json.contains("seconds"));
    assert!(!json.contains("keyboardHits"));
    assert!(!json.contains("mouseClicks"));

    let deserialized: WindowInfo =
        serde_json::from_str(&json).expect("deserialization must succeed");
    assert_eq!(deserialized, win);
}

#[test]
fn test_pure_filename_and_program_list_aggregation() {
    assert_eq!(file_name("/usr/bin/bash"), "bash");
    assert_eq!(file_name(r"C:\Program Files\App\test.exe"), "test.exe");
    assert_eq!(file_name(""), "");

    let raw_list = vec![
        "/usr/bin/code".to_string(),
        "Code".to_string(),
        "[kworker/u16:0]".to_string(),
        "[rcu_sched]".to_string(),
        "chrome".to_string(),
        "".to_string(),
        "CHROME".to_string(),
    ];

    let aggregated = program_list(raw_list);
    let names: Vec<&str> = aggregated.iter().map(|p| p.app_name.as_str()).collect();
    assert_eq!(names, vec!["code", "chrome"]);
}
