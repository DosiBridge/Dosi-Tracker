# Dosi-Tracker — PM Status Report & Evidence Matrix

> **Document Class:** Production Readiness Audit & Status Tracking (Phase 3 Step 1B Correction: Testable Windows Sensor Lifecycle Foundation)  
> **Status Date:** October 19, 2026  
> **Auditor / Implementation Engineer:** Antigravity Pair Engineer  
> **Supervisor:** PM / QA Operator  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)  

---

## 1. Executive Summary & Provenance History

### A. Supervisory Baseline & Commit `18db4d5` Independent Review
At commit `18db4d5`, the PM independently reviewed and verified five focused synthetic tests and two frontend gates:
1. `cargo test --bin dosi-tracker tracking::input::tests::inactive_counter_drops_input_events_and_keeps_logs_empty -- --exact` (1 passed, exit 0)
2. `cargo test --bin dosi-tracker tracking::input::tests::activating_counter_accumulates_events_and_deactivating_halts_collection -- --exact` (1 passed, exit 0)
3. `cargo test --bin dosi-tracker tracking::usage::tests::inactive_recorder_drops_samples_and_keeps_slices_empty -- --exact` (1 passed, exit 0)
4. `cargo test --bin dosi-tracker tracking::usage::tests::activating_recorder_records_samples_and_deactivation_clears_timing_anchor -- --exact` (1 passed, exit 0)
5. `cargo test --bin dosi-tracker worker::tests::tracking_active_follows_pause_and_resume_lifecycle -- --exact` (1 passed, exit 0)
6. `npm run typecheck` (0 errors, exit 0)
7. `npx vitest run src/components/layout/workspace-switcher.live.test.tsx src/test/integration/tenant-switching.test.tsx src/test/pages/workspaces.test.tsx` (17 tests / 3 files, exit 0)

**PM Finding on `18db4d5`:** Useful partial improvement, **NOT** acceptance of P0 privacy lifecycle.
- Defect 1: `Tracker::new()` in `tracking/mod.rs` still unconditionally started Win32 hooks (`spawn_listener`) and window samplers (`spawn_sampler`) at startup.
- Defect 2: `set_tracking_active` toggled accumulator flags only; discarding captured events is NOT stopping OS sensors.
- Defect 3: `InputFilter::apply` updated `HookHealth` and `key_down_at` while inactive; hook watchdog could operate or reinstall outside active collection.
- Defect 4: `Worker::quit` returned outside the normal publish loop and never explicitly deactivated sensors before farewell/upload waits.
- Defect 5: Tracking status alone ignored individual effective local/server permissions (e.g., keyboard or mouse disabled by policy).
- Defect 6: Configuration defaults in `CapturePermissions` do not substitute for explicit local opt-in consent.
- Directive: Keep backend nullability and tenant migrations strictly deferred until a testable Windows sensor lifecycle foundation is completed and verified.

---

### B. Current Bounded Phase: Step 1B Correction (Sensor Lifecycle Decoupling)
This correction introduces an injectable sensor driver foundation without architecture rewrites:
1. **Injectable Sensor Driver Abstraction (`clients/windows/src/tracking/lifecycle.rs`)**:
   - `pub trait SensorDriver: Send + Sync` defining `start_input`, `stop_input`, `is_input_running`, `start_usage`, `stop_usage`, `is_usage_running`.
   - `FakeSensorDriver`: side-effect-free driver recording start/stop counts, running states, and permitted flags (`last_keyboard_allowed`, `last_mouse_allowed`). Includes concurrent thread barrier validation.
   - `NativeSensorDriver`: Win32 driver managing `InputThread` and `UsageThread`. Synchronous hook detachment is guaranteed: `stop_input` posts `WM_QUIT` via `PostThreadMessageW` and calls synchronous thread `join()`; `stop_usage` sets an atomic stop flag and joins the sampler thread.
2. **Side-Effect-Free Construction (`clients/windows/src/tracking/mod.rs`)**:
   - `Tracker::new()` and `Tracker::with_driver()` **NEVER start hooks or samplers at startup**.
   - Sensors activate dynamically via `apply_sensor_policy(active, perms)` which invokes driver starts/stops and propagates per-sensor permissions (`allowed_keyboard`, `allowed_mouse`).
   - Replaced old `Tracker::new()` regression test with `Tracker::new_fake()` test `tracker_active_gate_controls_both_usage_and_input` and added `tracker_per_sensor_permission_gating_with_fake_driver`.
3. **Hard Inactive & Permission Gating (`clients/windows/src/tracking/input.rs`)**:
   - `InputFilter::apply` checks `if !counter.is_active() { return; }` immediately at entry: drops events before updating `HookHealth` or `key_down_at`.
   - Gated by `is_keyboard_allowed()` and `is_mouse_allowed()`.
   - `InputFilter::reset()` clears held keys on detachment; `HookHealth::reset_silence()` resets outage and timing states.
   - `watchdog` checks `if !counter.is_active() { return; }`, preventing hook resurrection outside active collection.
4. **Worker Lifecycle Integration (`clients/windows/src/worker.rs`)**:
   - `Worker::publish()` computes `is_tracking = status == TrackerStatus::Tracking` and session permissions, calling `machine.apply_sensor_policy(is_tracking, perms)`.
   - `Worker::quit()` immediately stops sensors (`apply_sensor_policy(false, None)`) before waiting for farewell heartbeats or draining upload queues.
   - `FakeMachine` tracks driver start/stop counts and running states.
   - Added worker tests for pause/resume, lock/unlock, away/return, workspace lost, quit before upload grace, per-sensor denied policy, and repeated transitions.
5. **Privacy Scope & Open Gap Transparency**:
   - **Real Native Interactive OS Runtime**: **Unverified** in QA (tests use `FakeSensorDriver` to prevent OS side effects).
   - **Local Interactive Opt-In Consent UI**: **Not Implemented** (server policy + config defaults do NOT constitute affirmative local consent).
   - **Idle/Away Recovery Semantics**: Documented tradeoff — away state detaches hooks; return is detected via Windows `GetLastInputInfo` system idle ticks without installing hooks.

---

## 2. Changed Files in Current Correction

| File Path | Description of Changes |
|---|---|
| `clients/windows/src/tracking/lifecycle.rs` | Greenfield injectable `SensorDriver` trait, `FakeSensorDriver`, and `NativeSensorDriver` with synchronous `WM_QUIT` join detachment. |
| `clients/windows/src/tracking/mod.rs` | Refactored `Tracker` to use `driver: Arc<dyn SensorDriver>`; eliminated hook/sampler calls from constructors; added `apply_sensor_policy` and fake driver tests. |
| `clients/windows/src/tracking/input.rs` | Added `allowed_keyboard` and `allowed_mouse` atomic flags; early-exit in `InputFilter::apply` before health/timing; gated watchdog; added filter and watchdog tests. |
| `clients/windows/src/tracking/usage.rs` | Gated `record` and sampler; annotated legacy helpers with `#[allow(dead_code)]`. |
| `clients/windows/src/worker.rs` | Added `apply_sensor_policy` to `Machine` trait; wired in `Worker::publish` and immediate detachment in `Worker::quit`. |
| `clients/windows/src/worker/tests.rs` | Added sensor lifecycle tracking to `FakeMachine`; added 6 new lifecycle regression tests. |
| `docs/PRODUCT_ACCEPTANCE.md` | Reconciled evidence levels to distinguish PM-verified tests from worker-reported tests; expanded finite backlog gaps (invitation expiry/revocation, safe deletion, negative HTTP isolation, retention, local consent UI). |
| `docs/PM_STATUS.md` | Detailed Step 1B correction, test provenance, exact commands, test counts, execution times, and exit codes. |

---

## 3. Concrete Test Execution & Evidence Provenance

### A. PM Independent Baseline (Verified at Commit `18db4d5`)

| Suite / Command | Exact Command Line | Tests Passed | Exit Code | Provenance |
|---|---|:---:|:---:|---|
| **Input Counter Drop** | `cargo test --bin dosi-tracker tracking::input::tests::inactive_counter_drops_input_events_and_keeps_logs_empty -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Input Counter Accumulate** | `cargo test --bin dosi-tracker tracking::input::tests::activating_counter_accumulates_events_and_deactivating_halts_collection -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Usage Recorder Drop** | `cargo test --bin dosi-tracker tracking::usage::tests::inactive_recorder_drops_samples_and_keeps_slices_empty -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Usage Recorder Accumulate**| `cargo test --bin dosi-tracker tracking::usage::tests::activating_recorder_records_samples_and_deactivation_clears_timing_anchor -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Worker Pause/Resume** | `cargo test --bin dosi-tracker worker::tests::tracking_active_follows_pause_and_resume_lifecycle -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Frontend TypeScript Gate** | `npm run typecheck` in `frontend` | 0 errors | **0** | Verified by PM @ `18db4d5` |
| **Frontend Workspace Tests** | `npx vitest run src/components/layout/workspace-switcher.live.test.tsx src/test/integration/tenant-switching.test.tsx src/test/pages/workspaces.test.tsx` | 17 | **0** | Verified by PM @ `18db4d5` |

---

### B. Worker-Reported Test Execution (Step 1B Correction at Current Working Tree)

All tests executed with `RUST_BACKTRACE=1` in `clients/windows`. No real Win32 input hooks or window enumeration was invoked during execution.

| Target Suite | Exact Command Line | Tests Passed | Ignored | Time | Exit Code | Evidence Level |
|---|---|:---:|:---:|:---:|:---:|:---:|
| **Compilation Gate** | `cargo check --bin dosi-tracker` | N/A | 0 errors, 0 warnings | 1.48s | **0** | **[Worker-Reported]** |
| **Test Compilation Gate** | `cargo test --bin dosi-tracker --no-run` | N/A | 0 errors, 0 warnings | 7.82s | **0** | **[Worker-Reported]** |
| **Sensor Lifecycle Suite** | `cargo test --bin dosi-tracker tracking::lifecycle::tests` | 4 | 0 | 0.00s | **0** | **[Worker-Reported]** |
| **Tracking Suite** | `cargo test --bin dosi-tracker tracking::tests` | 11 | 0 | 0.00s | **0** | **[Worker-Reported]** |
| **Input Counter & Filter Suite** | `cargo test --bin dosi-tracker tracking::input::tests` | 35 | 1 | 0.01s | **0** | **[Worker-Reported]** |
| **Usage Recorder Suite** | `cargo test --bin dosi-tracker tracking::usage::tests` | 16 | 0 | 0.00s | **0** | **[Worker-Reported]** |
| **Worker Sensor Lifecycle** | `cargo test --bin dosi-tracker worker::tests::sensor_lifecycle` | 6 | 0 | 2.03s | **0** | **[Worker-Reported]** |
| **Worker Pause/Resume** | `cargo test --bin dosi-tracker worker::tests::tracking_active_follows_pause_and_resume_lifecycle` | 1 | 0 | 0.01s | **0** | **[Worker-Reported]** |
| **Total Focused Synthetic Tests**| Sum of focused suites above | **73** | 1 (`the_real_hooks_report_injected_input_as_injected`) | ~2.5s | **0** | **[Worker-Reported]** |

*Note on Ignored Test:* `tracking::input::tests::the_real_hooks_report_injected_input_as_injected` is marked `#[ignore]` in source because it requires installing live Win32 hooks and nudging the physical mouse cursor. It was intentionally not executed in QA.

---

## 4. Platform Readiness Matrix

| Evidence Dimension | Windows (Rust) | macOS (Swift) | Linux (Target) |
|---|---|---|---|
| **1. Implemented Source** | ⚠️ Source present (`clients/windows`: egui UI, Win32 hooks, xcap, DPAPI, SQLite queue); sensor lifecycle foundation decoupled; native interactive hooks, packaging, and signing open | ⚠️ Source present (`clients/macos`: SwiftUI, ScreenCaptureKit, Keychain, SQLite queue); source presence is NOT proof of complete implementation; TCC consent, runtime execution, packaging, and notarization open | ❌ **Unresolved Gap** (`clients/linux` crate not yet created) |
| **2. Synthetic / Mock Unit Tests** | ✅ Complete (**73 focused synthetic tests passing**, 0 warnings) | ⚠️ Unverified locally (Swift test files present in source; not executed locally) | ❌ 0 tests (No crate) |
| **3. Toolchain / Compile / Build** | ✅ Verified locally (Rustc / Cargo 1.97.1 on Windows 11 host) | ❌ Unverified locally (Host lacks macOS toolchain / Xcode) | ❌ Unverified locally (Host lacks Linux build container / environment) |
| **4. Native Interactive Runtime** | ⚠️ **Unverified** (Unit tests run synthetically via `FakeSensorDriver`; interactive Win32 desktop hooks unverified in QA) | ⚠️ **Outstanding** (Requires physical macOS execution environment) | ❌ **Outstanding** (Requires Linux X11/Wayland execution environment) |
| **5. Packaging / Install / Uninstall** | ⚠️ Partial (NSIS `.nsi` script present; silent install/uninstall unverified in CI) | ⚠️ Partial (`build-app.sh` creates bundle; DMG packaging unverified) | ❌ **Unresolved Gap** (No `.deb`, AppImage, or Flatpak manifest) |
| **6. Code Signing & Notarization** | ⚠️ **Outstanding Prerequisite** (Unsigned; Authenticode cert required; signing does not guarantee immediate SmartScreen elimination; SmartScreen must not be bypassed) | ⚠️ **Outstanding Prerequisite** (Unsigned; Apple Developer ID & Notarization required) | ⚠️ **Outstanding Prerequisite** (GPG package signing required) |

---

## 5. Next Steps

1. **Awaiting PM Independent Review**:
   - PM independently executes selected focused synthetic tests to verify Step 1B correction.
2. **Subsequent Phasing (Strictly Awaiting Authorization)**:
   - Backend schema nullability migration (`Activity.cs` metric nullability) and cross-workspace tenant isolation tests remain deferred until PM authorization.
