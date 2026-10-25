#![cfg(target_os = "linux")]
//! The Screenshot portal source against a fake portal on a private session
//! bus: responses 0/1/2, the 30 s timeout path (shortened), no portal,
//! portals that ignore `handle_token`, and the returned file's deletion.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use dosi_tracker_linux::tracking::portal_screen::PortalScreenSource;
use dosi_tracker_linux::tracking::traits::{
    Availability, CaptureTarget, ScreenSource, TrackingError,
};
use support::fakes::portal::{FakePortal, PortalAnswer, SHOT_COLOUR};
use support::private_bus::PrivateTestBus;

const SHOT: PortalAnswer = PortalAnswer::Screenshot {
    width: 320,
    height: 200,
};

fn source(bus: &PrivateTestBus) -> PortalScreenSource {
    PortalScreenSource::new(bus.address()).with_timeout(Duration::from_secs(5))
}

#[test]
fn a_granted_screenshot_is_decoded_and_its_file_deleted() {
    let bus = PrivateTestBus::session();
    let portal = FakePortal::start(bus.address(), SHOT);
    let source = source(&bus);

    let frame = source.capture(CaptureTarget::ActiveMonitor).unwrap();
    assert_eq!(frame.image.dimensions(), (320, 200));
    assert_eq!(frame.image.get_pixel(10, 10).0, SHOT_COLOUR);
    assert!(frame.monitor.is_none());

    // The request was non-interactive, without a parent window, with our token.
    let calls = portal.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].parent_window, "");
    assert_eq!(calls[0].interactive, Some(false));
    let token = calls[0].handle_token.as_deref().expect("handle_token sent");
    assert!(calls[0].handle.ends_with(&format!("/{token}")));

    // Exactly the returned file is gone; the directory is untouched.
    let files = portal.files();
    assert_eq!(files.len(), 1);
    assert!(!files[0].exists(), "the portal's PNG must be deleted");
    assert!(portal.dir().exists());
    assert_eq!(source.cleanup_failure(), None);
    assert_eq!(source.availability(), Availability::Available);
    assert!(portal.closes().is_empty());
}

#[test]
fn a_refusal_is_final_until_the_user_retries() {
    for refusal in [PortalAnswer::Cancelled, PortalAnswer::Other] {
        let bus = PrivateTestBus::session();
        let portal = FakePortal::start(bus.address(), refusal.clone());
        let source = source(&bus);

        assert!(matches!(
            source.capture(CaptureTarget::ActiveMonitor),
            Err(TrackingError::PermissionDenied(_))
        ));
        assert_eq!(source.availability(), Availability::Denied);

        // Five further attempts never reach the portal (no dialog spam).
        for _ in 0..5 {
            assert!(matches!(
                source.capture(CaptureTarget::ActiveMonitor),
                Err(TrackingError::PermissionDenied(_))
            ));
        }
        assert_eq!(
            portal.calls().len(),
            1,
            "{refusal:?}: exactly one Screenshot call"
        );
        assert_eq!(source.requests_made(), 1);

        // The user's explicit retry asks once more.
        portal.set_answer(SHOT);
        source.reset();
        assert!(source.capture(CaptureTarget::ActiveMonitor).is_ok());
        assert_eq!(portal.calls().len(), 2);
        assert_eq!(source.availability(), Availability::Available);
    }
}

#[test]
fn an_unanswered_request_is_closed_and_needs_approval() {
    let bus = PrivateTestBus::session();
    let portal = FakePortal::start(bus.address(), PortalAnswer::Silent);
    let source = PortalScreenSource::new(bus.address()).with_timeout(Duration::from_millis(400));

    let started = Instant::now();
    assert!(matches!(
        source.capture(CaptureTarget::ActiveMonitor),
        Err(TrackingError::NeedsApproval(_))
    ));
    assert!(started.elapsed() >= Duration::from_millis(400));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(portal.closes(), vec![portal.calls()[0].handle.clone()]);
    assert_eq!(source.availability(), Availability::NeedsApproval);

    // Unlike a refusal, a later attempt may ask again.
    portal.set_answer(SHOT);
    assert!(source.capture(CaptureTarget::ActiveMonitor).is_ok());
    assert_eq!(portal.calls().len(), 2);
}

#[test]
fn no_portal_on_the_bus_is_unsupported() {
    let bus = PrivateTestBus::session();
    let source = source(&bus);
    assert!(matches!(
        source.capture(CaptureTarget::ActiveMonitor),
        Err(TrackingError::UnsupportedSession(_))
    ));
    assert!(matches!(
        source.availability(),
        Availability::Unsupported(_)
    ));
}

#[test]
fn a_portal_choosing_its_own_handle_still_works() {
    let bus = PrivateTestBus::session();
    let portal = FakePortal::start(bus.address(), SHOT);
    portal.ignore_handle_token(true);
    let source = source(&bus);
    let frame = source.capture(CaptureTarget::WholeScreen).unwrap();
    assert_eq!(frame.image.dimensions(), (320, 200));
    assert!(portal.calls()[0].handle.contains("portal_chosen_"));
    assert!(!portal.files()[0].exists());
}

#[test]
fn a_non_local_uri_is_refused_and_nothing_is_deleted() {
    let bus = PrivateTestBus::session();
    let _portal = FakePortal::start(bus.address(), PortalAnswer::RemoteUri);
    let source = source(&bus);
    let err = source.capture(CaptureTarget::ActiveMonitor).unwrap_err();
    assert!(matches!(err, TrackingError::IoError(_)), "{err:?}");
    assert!(matches!(
        source.availability(),
        Availability::Unavailable(_)
    ));
}

#[test]
fn a_file_that_cannot_be_deleted_is_reported_and_the_frame_kept() {
    let bus = PrivateTestBus::session();
    let portal = FakePortal::start(bus.address(), SHOT);
    // After writing the PNG the fake makes its directory read-only, so the
    // unlink fails (tests never run as root).
    portal.lock_dir_after_write(true);
    let source = source(&bus);
    let frame = source.capture(CaptureTarget::ActiveMonitor);
    std::fs::set_permissions(portal.dir(), std::fs::Permissions::from_mode(0o700)).unwrap();

    let frame = frame.expect("a cleanup failure never costs the frame");
    assert_eq!(frame.image.dimensions(), (320, 200));
    let failure = source.cleanup_failure().expect("the failure is reported");
    assert!(!failure.is_empty());
    assert!(portal.files()[0].exists(), "left in place, reported");
}
