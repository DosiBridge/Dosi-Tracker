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
    craft_get_property_wire_reply, file_name, program_list, AcquisitionProviderExt,
    ActiveWindowProvider, ConsentGatedTracker, FakeAcquisitionProvider, MockProcFsReader,
    MockX11PropertyReader, ProcFsRunningAppsProvider, RawPropertyReply, RunningAppsProvider,
    SyntheticX11Transport, SysX11PropertyReader, TrackingError, WaylandActiveWindowProvider,
    WindowInfo, X11ActiveWindowProvider, X11PropertyReader, MAX_RUNNING_PROGRAMS,
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
fn test_wrapper_level_in_flight_pause_resume_and_revoke_regrant_aba_generation_mismatch() {
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

    // Sub-case 1: Active Window - pause -> resume inside blocked acquisition
    {
        let lifecycle = create_test_lifecycle();
        {
            let mut lc = lifecycle.lock().unwrap();
            lc.configure(policy);
            lc.update_consent(LocalConsent::Granted(local)).unwrap();
            lc.start().unwrap();
        }

        let fake = Arc::new(FakeAcquisitionProvider::new());
        let lc_hook = lifecycle.clone();
        fake.set_on_current_window(Box::new(move || {
            let mut lc = lc_hook.lock().unwrap();
            lc.pause(PauseReason::User).unwrap();
            lc.resume().unwrap();
        }));

        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());
        let res = tracker.current_active_window();
        match res {
            Err(TrackingError::GenerationMismatch {
                ticket_gen,
                current_gen,
            }) => {
                assert!(
                    current_gen > ticket_gen,
                    "Expected current_gen ({}) > ticket_gen ({})",
                    current_gen,
                    ticket_gen
                );
            }
            other => panic!(
                "Active window pause->resume must fail with GenerationMismatch, got {:?}",
                other
            ),
        }
    }

    // Sub-case 2: Active Window - revoke -> regrant/start inside blocked acquisition
    {
        let lifecycle = create_test_lifecycle();
        {
            let mut lc = lifecycle.lock().unwrap();
            lc.configure(policy);
            lc.update_consent(LocalConsent::Granted(local)).unwrap();
            lc.start().unwrap();
        }

        let fake = Arc::new(FakeAcquisitionProvider::new());
        let lc_hook = lifecycle.clone();
        let local_regrant = local;
        fake.set_on_current_window(Box::new(move || {
            let mut lc = lc_hook.lock().unwrap();
            lc.update_consent(LocalConsent::Revoked).unwrap();
            lc.update_consent(LocalConsent::Granted(local_regrant))
                .unwrap();
            lc.start().unwrap();
        }));

        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());
        let res = tracker.current_active_window();
        match res {
            Err(TrackingError::GenerationMismatch {
                ticket_gen,
                current_gen,
            }) => {
                assert!(
                    current_gen > ticket_gen,
                    "Expected current_gen ({}) > ticket_gen ({})",
                    current_gen,
                    ticket_gen
                );
            }
            other => panic!(
                "Active window revoke->regrant must fail with GenerationMismatch, got {:?}",
                other
            ),
        }
    }

    // Sub-case 3: Running Apps - pause -> resume inside blocked acquisition
    {
        let lifecycle = create_test_lifecycle();
        {
            let mut lc = lifecycle.lock().unwrap();
            lc.configure(policy);
            lc.update_consent(LocalConsent::Granted(local)).unwrap();
            lc.start().unwrap();
        }

        let fake = Arc::new(FakeAcquisitionProvider::new());
        let lc_hook = lifecycle.clone();
        fake.set_on_running_programs(Box::new(move || {
            let mut lc = lc_hook.lock().unwrap();
            lc.pause(PauseReason::User).unwrap();
            lc.resume().unwrap();
        }));

        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());
        let res = tracker.running_programs();
        match res {
            Err(TrackingError::GenerationMismatch {
                ticket_gen,
                current_gen,
            }) => {
                assert!(
                    current_gen > ticket_gen,
                    "Expected current_gen ({}) > ticket_gen ({})",
                    current_gen,
                    ticket_gen
                );
            }
            other => panic!(
                "Running programs pause->resume must fail with GenerationMismatch, got {:?}",
                other
            ),
        }
    }

    // Sub-case 4: Running Apps - revoke -> regrant/start inside blocked acquisition
    {
        let lifecycle = create_test_lifecycle();
        {
            let mut lc = lifecycle.lock().unwrap();
            lc.configure(policy);
            lc.update_consent(LocalConsent::Granted(local)).unwrap();
            lc.start().unwrap();
        }

        let fake = Arc::new(FakeAcquisitionProvider::new());
        let lc_hook = lifecycle.clone();
        let local_regrant = local;
        fake.set_on_running_programs(Box::new(move || {
            let mut lc = lc_hook.lock().unwrap();
            lc.update_consent(LocalConsent::Revoked).unwrap();
            lc.update_consent(LocalConsent::Granted(local_regrant))
                .unwrap();
            lc.start().unwrap();
        }));

        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());
        let res = tracker.running_programs();
        match res {
            Err(TrackingError::GenerationMismatch {
                ticket_gen,
                current_gen,
            }) => {
                assert!(
                    current_gen > ticket_gen,
                    "Expected current_gen ({}) > ticket_gen ({})",
                    current_gen,
                    ticket_gen
                );
            }
            other => panic!(
                "Running programs revoke->regrant must fail with GenerationMismatch, got {:?}",
                other
            ),
        }
    }
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

#[test]
fn test_sys_x11_transport_protocol_decoding_full_stack() {
    let transport = SyntheticX11Transport::new();
    let root = 0x0000_0100;
    let active_win = 0x0240_0001u32;
    let pid = 54321u32;
    let title_text = "Visual Studio Code — project/src/main.rs 🚀";

    transport.set_root_window(root);

    // 1. Root window active window property pointing to 0x0240_0001 via wire bytes
    let active_window_wire = craft_get_property_wire_reply(32, 33, &active_win.to_ne_bytes());
    let active_reply =
        RawPropertyReply::from_wire_bytes(&active_window_wire).expect("wire parsing must succeed");
    transport.set_property(root, "_NET_ACTIVE_WINDOW", active_reply);

    // 2. Focused window title (_NET_WM_NAME in UTF-8 with multibyte unicode and emoji) via wire bytes
    let title_wire = craft_get_property_wire_reply(8, 242, title_text.as_bytes());
    let title_reply =
        RawPropertyReply::from_wire_bytes(&title_wire).expect("wire parsing must succeed");
    transport.set_property(active_win, "_NET_WM_NAME", title_reply);

    // 3. Focused window class (WM_CLASS null-delimited instance + class) via wire bytes
    let class_wire = craft_get_property_wire_reply(8, 31, b"code\0Code\0");
    let class_reply =
        RawPropertyReply::from_wire_bytes(&class_wire).expect("wire parsing must succeed");
    transport.set_property(active_win, "WM_CLASS", class_reply);

    // 4. Focused window PID (_NET_WM_PID cardinal 32-bit) via wire bytes
    let pid_wire = craft_get_property_wire_reply(32, 6, &pid.to_ne_bytes());
    let pid_reply =
        RawPropertyReply::from_wire_bytes(&pid_wire).expect("wire parsing must succeed");
    transport.set_property(active_win, "_NET_WM_PID", pid_reply);

    // 5. Mock procfs mapping PID 54321 -> exe "/usr/share/code/code"
    let mock_proc = Arc::new(MockProcFsReader::new());
    mock_proc.add_process(
        pid,
        "code",
        Some(vec!["/usr/share/code/code".into()]),
        Some("/usr/share/code/code".into()),
    );

    let reader = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport)));
    let provider = X11ActiveWindowProvider::new(reader, Some(mock_proc));

    let win = provider
        .current_active_window()
        .expect("query must succeed")
        .expect("must return active window");

    assert_eq!(win.app_name, "code");
    assert_eq!(win.window_title, title_text);
    assert_eq!(win.seconds, None);
    assert_eq!(win.keyboard_hits, None);
    assert_eq!(win.mouse_clicks, None);
}

#[test]
fn test_sys_x11_transport_fallback_wm_name_and_wm_class() {
    let transport = SyntheticX11Transport::new();
    let root = 0x0000_0200;
    let win_id = 0x0500_0001u32;
    let legacy_title = "Legacy XTerm Terminal";

    transport.set_root_window(root);

    // Root window points to window 0x0500_0001
    let active_window_wire = craft_get_property_wire_reply(32, 33, &win_id.to_ne_bytes());
    let active_reply = RawPropertyReply::from_wire_bytes(&active_window_wire).unwrap();
    transport.set_property(root, "_NET_ACTIVE_WINDOW", active_reply);

    // No _NET_WM_NAME property set.
    // WM_NAME fallback set as STRING (format 8)
    let wm_name_wire = craft_get_property_wire_reply(8, 31, legacy_title.as_bytes());
    let wm_name_reply = RawPropertyReply::from_wire_bytes(&wm_name_wire).unwrap();
    transport.set_property(win_id, "WM_NAME", wm_name_reply);

    // No _NET_WM_PID property set.
    // WM_CLASS set
    let class_wire = craft_get_property_wire_reply(8, 31, b"xterm\0XTerm\0");
    let class_reply = RawPropertyReply::from_wire_bytes(&class_wire).unwrap();
    transport.set_property(win_id, "WM_CLASS", class_reply);

    let reader = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport)));
    // No procfs reader provided -> falls back to WM_CLASS class name
    let provider = X11ActiveWindowProvider::new(reader, None);

    let win = provider
        .current_active_window()
        .expect("query must succeed")
        .expect("must return active window");

    // Falls back to WM_NAME for title
    assert_eq!(win.window_title, legacy_title);
    // Falls back to WM_CLASS class name for app_name
    assert_eq!(win.app_name, "XTerm");
}

#[test]
fn test_sys_x11_transport_unfocused_window_and_zero_xid() {
    let root = 0x0000_0300;

    // Case 1: Root has no _NET_ACTIVE_WINDOW property (returns None)
    let transport1 = SyntheticX11Transport::new();
    transport1.set_root_window(root);
    let reader1 = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport1)));
    let provider1 = X11ActiveWindowProvider::new(reader1, None);
    assert_eq!(provider1.current_active_window().unwrap(), None);

    // Case 2: Root _NET_ACTIVE_WINDOW contains 0 (X11 desktop / unfocused convention)
    let transport2 = SyntheticX11Transport::new();
    transport2.set_root_window(root);
    let zero_wire = craft_get_property_wire_reply(32, 33, &(0u32).to_ne_bytes());
    let zero_reply = RawPropertyReply::from_wire_bytes(&zero_wire).unwrap();
    transport2.set_property(root, "_NET_ACTIVE_WINDOW", zero_reply);
    let reader2 = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport2)));
    let provider2 = X11ActiveWindowProvider::new(reader2, None);
    assert_eq!(provider2.current_active_window().unwrap(), None);

    // Case 3: Empty value bytes
    let transport3 = SyntheticX11Transport::new();
    transport3.set_root_window(root);
    let empty_wire = craft_get_property_wire_reply(32, 33, &[]);
    let empty_reply = RawPropertyReply::from_wire_bytes(&empty_wire).unwrap();
    transport3.set_property(root, "_NET_ACTIVE_WINDOW", empty_reply);
    let reader3 = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport3)));
    let provider3 = X11ActiveWindowProvider::new(reader3, None);
    assert_eq!(provider3.current_active_window().unwrap(), None);
}

#[test]
fn test_sys_x11_transport_malformed_wire_and_error_handling() {
    let root = 0x0000_0400;

    // Case 1: Corrupted wire packet fails wire parser with ProtocolError
    let corrupted_wire = [0u8; 12];
    let wire_err = RawPropertyReply::from_wire_bytes(&corrupted_wire);
    assert!(matches!(wire_err, Err(TrackingError::ProtocolError(_))));

    // Case 2: Truncated window ID (less than 4 bytes for format 32) fails safe to None
    let transport1 = SyntheticX11Transport::new();
    transport1.set_root_window(root);
    transport1.set_property(
        root,
        "_NET_ACTIVE_WINDOW",
        RawPropertyReply {
            format: 32,
            type_atom: 33,
            value: vec![0x42, 0x00],
        },
    );
    let reader1 = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport1)));
    let provider1 = X11ActiveWindowProvider::new(reader1.clone(), None);
    assert_eq!(provider1.current_active_window().unwrap(), None);

    // Case 3: Invalid window ID exceeding u32::MAX returns ProtocolError
    let win_err = reader1.get_window_title(u64::MAX);
    assert!(matches!(win_err, Err(TrackingError::ProtocolError(_))));

    // Case 2: Transport communication failure propagates cleanly
    let transport2 = SyntheticX11Transport::new();
    transport2.set_root_window(root);
    transport2.set_property_error(
        root,
        "_NET_ACTIVE_WINDOW",
        TrackingError::DisplayUnavailable("X11 connection broken".into()),
    );
    let reader2 = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport2)));
    let provider2 = X11ActiveWindowProvider::new(reader2, None);
    let err2 = provider2.current_active_window();
    assert!(matches!(err2, Err(TrackingError::DisplayUnavailable(_))));

    // Case 3: Malformed PID (format 16 instead of 32) fails safe to None and uses WM_CLASS fallback
    let transport3 = SyntheticX11Transport::new();
    transport3.set_root_window(root);
    let win_id = 0x0040_0005u32;
    let win_wire = craft_get_property_wire_reply(32, 33, &win_id.to_ne_bytes());
    transport3.set_property(
        root,
        "_NET_ACTIVE_WINDOW",
        RawPropertyReply::from_wire_bytes(&win_wire).unwrap(),
    );
    let title_wire = craft_get_property_wire_reply(8, 242, b"Malformed PID Window");
    transport3.set_property(
        win_id,
        "_NET_WM_NAME",
        RawPropertyReply::from_wire_bytes(&title_wire).unwrap(),
    );
    let bad_pid_wire = craft_get_property_wire_reply(16, 6, &[0x12, 0x34]);
    transport3.set_property(
        win_id,
        "_NET_WM_PID",
        RawPropertyReply::from_wire_bytes(&bad_pid_wire).unwrap(),
    );
    let class_wire = craft_get_property_wire_reply(8, 31, b"fallback-app\0FallbackApp\0");
    transport3.set_property(
        win_id,
        "WM_CLASS",
        RawPropertyReply::from_wire_bytes(&class_wire).unwrap(),
    );

    let reader3 = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport3)));
    let provider3 = X11ActiveWindowProvider::new(reader3, None);
    let win3 = provider3
        .current_active_window()
        .expect("must succeed despite bad PID")
        .expect("window present");
    assert_eq!(win3.window_title, "Malformed PID Window");
    assert_eq!(win3.app_name, "FallbackApp");
}

#[test]
fn test_consent_gated_tracker_with_real_x11_transport_and_revocation() {
    let lifecycle = create_test_lifecycle();

    let root = 0x0000_0500;
    let win_id = 0x0050_0001u32;
    let transport = SyntheticX11Transport::new();
    transport.set_root_window(root);

    let active_wire = craft_get_property_wire_reply(32, 33, &win_id.to_ne_bytes());
    transport.set_property(
        root,
        "_NET_ACTIVE_WINDOW",
        RawPropertyReply::from_wire_bytes(&active_wire).unwrap(),
    );

    let title_wire = craft_get_property_wire_reply(8, 242, b"Gated Secure Window");
    transport.set_property(
        win_id,
        "_NET_WM_NAME",
        RawPropertyReply::from_wire_bytes(&title_wire).unwrap(),
    );

    let class_wire = craft_get_property_wire_reply(8, 31, b"secure-app\0SecureApp\0");
    transport.set_property(
        win_id,
        "WM_CLASS",
        RawPropertyReply::from_wire_bytes(&class_wire).unwrap(),
    );

    let reader = Arc::new(SysX11PropertyReader::with_transport(Box::new(transport)));
    let window_provider = Arc::new(X11ActiveWindowProvider::new(reader, None));
    let fake_apps = Arc::new(FakeAcquisitionProvider::new());

    let tracker = ConsentGatedTracker::new(window_provider, fake_apps, lifecycle.clone());

    // 1. Unconfigured: default-deny
    assert_eq!(
        tracker.current_active_window(),
        Err(TrackingError::ConsentRequired)
    );

    // 2. Configure with active_window granted, running_programs disabled
    let policy = ConsentedPermissions {
        active_window: true,
        running_programs: false,
        ..ConsentedPermissions::all_disabled()
    };
    {
        let mut lc = lifecycle.lock().unwrap();
        lc.configure(policy);
        lc.update_consent(LocalConsent::Granted(policy)).unwrap();
        lc.start().unwrap();
    }

    // 3. Active window succeeds through the real X11 decoding pipeline
    let win = tracker
        .current_active_window()
        .expect("must succeed")
        .expect("window returned");
    assert_eq!(win.window_title, "Gated Secure Window");
    assert_eq!(win.app_name, "SecureApp");

    // 4. Running programs fails closed with CapabilityDenied
    assert_eq!(
        tracker.running_programs(),
        Err(TrackingError::CapabilityDenied(
            AcquisitionCapability::RunningPrograms
        ))
    );

    // 5. Revoke consent: subsequent queries immediately fail closed
    {
        let mut lc = lifecycle.lock().unwrap();
        lc.update_consent(LocalConsent::Revoked).unwrap();
    }
    assert_eq!(
        tracker.current_active_window(),
        Err(TrackingError::ConsentRequired)
    );
}
