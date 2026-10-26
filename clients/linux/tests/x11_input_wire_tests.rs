//! XI2 raw events as they arrive on the wire, through the listener's own
//! decoding path ([`decode_xi2_event`] then [`apply_event`]) into an
//! [`InputLog`]. No display is needed: the bytes are crafted here.
//!
//! Proves (design I3, 7): XTEST-originated events count as injected only;
//! wheel buttons and motion are activity, not clicks; auto-repeat is dropped;
//! no keycode reaches the log; kinds outside the policy are not recorded;
//! foreign events are ignored.

use std::collections::HashSet;

use chrono::DateTime;
use dosi_tracker_linux::tracking::input::InputLog;
use dosi_tracker_linux::tracking::x11_input::{
    apply_event, decode_xi2_event, xtest_device_ids, InputKinds, Xi2Event,
};
use x11rb::protocol::xinput;

const XI_OPCODE: u8 = 131;
const T0: i64 = 1_790_000_000;
const PHYSICAL_KEYBOARD: u16 = 11;
const PHYSICAL_POINTER: u16 = 12;
const XTEST_KEYBOARD: u16 = 5;
const XTEST_POINTER: u16 = 4;
const REPEAT: u32 = 1 << 16;

/// A 32-byte XI2 raw event header (`xXIRawEvent`, no valuators) in the
/// connection's (native) byte order.
fn raw_event(evtype: u16, detail: u32, sourceid: u16, flags: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 32];
    bytes[0] = 35; // GenericEvent
    bytes[1] = XI_OPCODE;
    bytes[2..4].copy_from_slice(&7u16.to_ne_bytes()); // sequence
    bytes[4..8].copy_from_slice(&0u32.to_ne_bytes()); // extra length
    bytes[8..10].copy_from_slice(&evtype.to_ne_bytes());
    bytes[10..12].copy_from_slice(&3u16.to_ne_bytes()); // master device
    bytes[12..16].copy_from_slice(&123_456u32.to_ne_bytes()); // server time
    bytes[16..20].copy_from_slice(&detail.to_ne_bytes());
    bytes[20..22].copy_from_slice(&sourceid.to_ne_bytes());
    bytes[22..24].copy_from_slice(&0u16.to_ne_bytes()); // valuators_len
    bytes[24..28].copy_from_slice(&flags.to_ne_bytes());
    bytes
}

fn key(keycode: u32, source: u16, flags: u32) -> Vec<u8> {
    raw_event(xinput::RAW_KEY_PRESS_EVENT, keycode, source, flags)
}

fn button(button: u32, source: u16) -> Vec<u8> {
    raw_event(xinput::RAW_BUTTON_PRESS_EVENT, button, source, 0)
}

fn motion(source: u16) -> Vec<u8> {
    raw_event(xinput::RAW_MOTION_EVENT, 0, source, 0)
}

fn xtest() -> HashSet<u16> {
    HashSet::from([XTEST_KEYBOARD, XTEST_POINTER])
}

const BOTH: InputKinds = InputKinds {
    keyboard: true,
    mouse: true,
};

fn feed(log: &InputLog, events: &[Vec<u8>], kinds: InputKinds, second: i64) {
    for raw in events {
        if let Some(event) = decode_xi2_event(raw, XI_OPCODE) {
            apply_event(log, event, &xtest(), kinds, second);
        }
    }
}

fn healthy_log() -> InputLog {
    let log = InputLog::new();
    log.mark_healthy(DateTime::from_timestamp(T0 - 10, 0).unwrap());
    log
}

fn window(log: &InputLog) -> dosi_tracker_linux::tracking::input::InputMeasurements {
    log.measurements(
        DateTime::from_timestamp(T0, 0).unwrap(),
        DateTime::from_timestamp(T0 + 60, 0).unwrap(),
    )
}

#[test]
fn physical_keys_and_clicks_are_counted() {
    let log = healthy_log();
    feed(
        &log,
        &[
            key(38, PHYSICAL_KEYBOARD, 0),
            key(39, PHYSICAL_KEYBOARD, 0),
            button(1, PHYSICAL_POINTER),
            button(3, PHYSICAL_POINTER),
            button(8, PHYSICAL_POINTER),
        ],
        BOTH,
        T0 + 5,
    );
    let m = window(&log);
    assert_eq!(m.key_presses_ms.len(), 2);
    assert_eq!(m.clicks_ms.len(), 3);
    assert_eq!(m.active_seconds, vec![T0 + 5]);
    assert_eq!(m.injected, 0);
    assert!(m.healthy);
}

#[test]
fn xtest_sourced_events_are_injected_only() {
    let log = healthy_log();
    feed(
        &log,
        &[
            key(38, XTEST_KEYBOARD, 0),
            key(39, XTEST_KEYBOARD, 0),
            button(1, XTEST_POINTER),
            motion(XTEST_POINTER),
            button(4, XTEST_POINTER), // injected wheel
        ],
        BOTH,
        T0 + 7,
    );
    let m = window(&log);
    assert!(m.key_presses_ms.is_empty(), "XTEST keys are not the user's");
    assert!(m.clicks_ms.is_empty(), "XTEST clicks are not the user's");
    assert!(m.active_seconds.is_empty(), "XTEST input is not activity");
    assert_eq!(m.injected, 5);
    assert_eq!(log.last_physical_input(), None);
}

#[test]
fn wheel_buttons_and_motion_mark_activity_but_are_not_clicks() {
    let log = healthy_log();
    feed(
        &log,
        &[
            button(4, PHYSICAL_POINTER),
            button(5, PHYSICAL_POINTER),
            button(6, PHYSICAL_POINTER),
            button(7, PHYSICAL_POINTER),
        ],
        BOTH,
        T0 + 1,
    );
    feed(&log, &[motion(PHYSICAL_POINTER)], BOTH, T0 + 2);
    let m = window(&log);
    assert!(m.clicks_ms.is_empty());
    assert_eq!(m.active_seconds, vec![T0 + 1, T0 + 2]);
    for wheel in 4..=7 {
        assert_eq!(
            decode_xi2_event(&button(wheel, PHYSICAL_POINTER), XI_OPCODE),
            Some(Xi2Event::PointerActivity {
                source: PHYSICAL_POINTER
            })
        );
    }
}

#[test]
fn auto_repeat_is_dropped() {
    let log = healthy_log();
    feed(
        &log,
        &[
            key(38, PHYSICAL_KEYBOARD, 0),
            key(38, PHYSICAL_KEYBOARD, REPEAT),
            key(38, PHYSICAL_KEYBOARD, REPEAT),
            key(38, PHYSICAL_KEYBOARD, REPEAT),
        ],
        BOTH,
        T0 + 3,
    );
    assert_eq!(window(&log).key_presses_ms.len(), 1);
    assert_eq!(
        decode_xi2_event(&key(38, PHYSICAL_KEYBOARD, REPEAT), XI_OPCODE),
        None
    );
}

#[test]
fn no_keycode_reaches_the_input_log() {
    // Two different keys decode to the identical event: the keycode is gone
    // before anything is recorded (and `Xi2Event::Key` has no field for it).
    let a = decode_xi2_event(&key(38, PHYSICAL_KEYBOARD, 0), XI_OPCODE);
    let b = decode_xi2_event(&key(56, PHYSICAL_KEYBOARD, 0), XI_OPCODE);
    assert_eq!(a, b);
    assert_eq!(
        a,
        Some(Xi2Event::Key {
            source: PHYSICAL_KEYBOARD
        })
    );
    // Garbage in the keycode field changes nothing either.
    assert_eq!(
        decode_xi2_event(&key(u32::MAX, PHYSICAL_KEYBOARD, 0), XI_OPCODE),
        a
    );

    // Two logs fed different keys at the same second are indistinguishable.
    let first = healthy_log();
    let second = healthy_log();
    feed(&first, &[key(24, PHYSICAL_KEYBOARD, 0)], BOTH, T0 + 9);
    feed(&second, &[key(52, PHYSICAL_KEYBOARD, 0)], BOTH, T0 + 9);
    assert_eq!(window(&first), window(&second));
    // What a block reads is whole-second timestamps only.
    assert_eq!(window(&first).key_presses_ms, vec![(T0 + 9) * 1000]);
}

#[test]
fn kinds_outside_the_policy_are_not_recorded() {
    let keyboard_only = InputKinds {
        keyboard: true,
        mouse: false,
    };
    let log = healthy_log();
    feed(
        &log,
        &[
            key(38, PHYSICAL_KEYBOARD, 0),
            button(1, PHYSICAL_POINTER),
            motion(PHYSICAL_POINTER),
            button(1, XTEST_POINTER),
        ],
        keyboard_only,
        T0 + 4,
    );
    let m = window(&log);
    assert_eq!(m.key_presses_ms.len(), 1);
    assert!(m.clicks_ms.is_empty());
    assert_eq!(
        m.injected, 0,
        "a denied kind is not even counted as injected"
    );

    let mouse_only = InputKinds {
        keyboard: false,
        mouse: true,
    };
    let log = healthy_log();
    feed(
        &log,
        &[key(38, PHYSICAL_KEYBOARD, 0), button(2, PHYSICAL_POINTER)],
        mouse_only,
        T0 + 4,
    );
    let m = window(&log);
    assert!(m.key_presses_ms.is_empty());
    assert_eq!(m.clicks_ms.len(), 1);
}

#[test]
fn foreign_and_truncated_events_are_ignored() {
    let ours = key(38, PHYSICAL_KEYBOARD, 0);
    // Another extension's generic event.
    let mut other_extension = ours.clone();
    other_extension[1] = XI_OPCODE + 1;
    assert_eq!(decode_xi2_event(&other_extension, XI_OPCODE), None);
    // A core event (KeyPress = 2) is not an XI2 raw event.
    let mut core = ours.clone();
    core[0] = 2;
    assert_eq!(decode_xi2_event(&core, XI_OPCODE), None);
    // Truncated.
    assert_eq!(decode_xi2_event(&ours[..20], XI_OPCODE), None);
    // Non-raw XI2 events (XI_KeyPress = 2) are not selected and not counted.
    assert_eq!(
        decode_xi2_event(&raw_event(2, 38, PHYSICAL_KEYBOARD, 0), XI_OPCODE),
        None
    );
    // The "sent" bit of a SendEvent'd generic event does not hide the type.
    let mut sent = ours;
    sent[0] |= 0x80;
    assert!(decode_xi2_event(&sent, XI_OPCODE).is_some());
}

#[test]
fn hierarchy_changes_are_reported_for_an_xtest_refresh() {
    assert_eq!(
        decode_xi2_event(&raw_event(xinput::HIERARCHY_EVENT, 0, 0, 0), XI_OPCODE),
        Some(Xi2Event::HierarchyChanged)
    );
    // Applying it records nothing.
    let log = healthy_log();
    apply_event(&log, Xi2Event::HierarchyChanged, &xtest(), BOTH, T0);
    assert_eq!(window(&log).active_seconds.len(), 0);
}

#[test]
fn the_xtest_set_comes_from_device_names() {
    let names: Vec<(u16, &[u8])> = vec![
        (2, b"Virtual core pointer"),
        (3, b"Virtual core keyboard"),
        (XTEST_POINTER, b"Virtual core XTEST pointer"),
        (XTEST_KEYBOARD, b"Virtual core XTEST keyboard"),
        (PHYSICAL_POINTER, b"SynPS/2 Synaptics TouchPad"),
    ];
    assert_eq!(xtest_device_ids(names), xtest());
}
