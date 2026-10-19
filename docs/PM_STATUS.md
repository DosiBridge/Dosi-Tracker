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
| `clients/windows/src/config.rs` | Added `LocalConsent` enum (`Absent`, `Granted`, `Revoked`) defaulting to `Absent`; updated `session_for` to fail closed if consent is absent or revoked; derive `PartialEq, Eq` on `CapturePermissions`. |
| `clients/windows/src/tracking/system.rs` | Added `DeviceInfo::fixture()` with mock constants for side-effect-free fake tracker construction without local filesystem access. |
| `clients/windows/src/tracking/input.rs` | Added `InputCounter::set_policy` eliminating transient dual-permission grant; refactored `Hooks` to store optional handles (`Option<HHOOK>`); updated `install(keyboard, mouse)` and `watchdog` to attach only permitted hooks; added `run_hooks_thread` forcing Win32 message queue creation via `PeekMessageW(..., PM_NOREMOVE)` before sending thread ID handshake; updated `InputFilter::apply` to drop denied events before touching health or key timing. |
| `clients/windows/src/tracking/lifecycle.rs` | Implemented platform-neutral `InputLifecycleController` and `UsageLifecycleController` with explicit queue-ready handshake, serial transitions under lock, bounded shutdown retry, truthful running state, and `Drop` cleanup; updated `NativeSensorDriver` to consume lifecycle controllers; expanded tests with 7 new synthetic lifecycle edge tests (11 total). |
| `clients/windows/src/tracking/mod.rs` | Updated `Tracker::with_driver_and_device` and `Tracker::new_fake` to use `DeviceInfo::fixture()`; updated `apply_sensor_policy` to use `set_policy`; closed `Tracker::snapshot` bypass so `active_window::current` and `running_programs` are queried only when permitted and sampler is active. |
| `clients/windows/src/worker/session.rs` | Evaluates effective permissions via `cfg.consent.effective_permissions`; added 3 consent tests. |
| `clients/windows/src/worker/tests.rs` | Injected affirmative consent in `harness_at`; added task barrier in quit regression test proving sensor stop before upload wait; added `worker_fails_closed_when_consent_is_absent`; aligned `FakeMachine::is_tracking_active` with `Tracker::is_tracking_active`. |
| `docs/PRODUCT_ACCEPTANCE.md` | Reconciled evidence levels to distinguish PM-verified tests from worker-reported tests; clarified that accumulator tests do not prove absence of raw keystrokes across the pipeline; noted fail-closed consent integration. |
| `docs/PM_STATUS.md` | Recorded PM baselines at `18db4d5` and `8c1b201`, documented Step 1C hardening results and test provenance. |

---

## 3. Concrete Test Execution & Evidence Provenance

### A. PM Independent Baselines

#### 1. Baseline at Commit `18db4d5` (Verified by PM)

| Suite / Command | Exact Command Line | Tests Passed | Exit Code | Provenance |
|---|---|:---:|:---:|---|
| **Input Counter Drop** | `cargo test --bin dosi-tracker tracking::input::tests::inactive_counter_drops_input_events_and_keeps_logs_empty -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Input Counter Accumulate** | `cargo test --bin dosi-tracker tracking::input::tests::activating_counter_accumulates_events_and_deactivating_halts_collection -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Usage Recorder Drop** | `cargo test --bin dosi-tracker tracking::usage::tests::inactive_recorder_drops_samples_and_keeps_slices_empty -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Usage Recorder Accumulate**| `cargo test --bin dosi-tracker tracking::usage::tests::activating_recorder_records_samples_and_deactivation_clears_timing_anchor -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Worker Pause/Resume** | `cargo test --bin dosi-tracker worker::tests::tracking_active_follows_pause_and_resume_lifecycle -- --exact` | 1 | **0** | Verified by PM @ `18db4d5` |
| **Frontend TypeScript Gate** | `npm run typecheck` in `frontend` | 0 errors | **0** | Verified by PM @ `18db4d5` |
| **Frontend Workspace Tests** | `npx vitest run src/components/layout/workspace-switcher.live.test.tsx src/test/integration/tenant-switching.test.tsx src/test/pages/workspaces.test.tsx` | 17 | **0** | Verified by PM @ `18db4d5` |

#### 2. Baseline at Commit `8c1b201` (Verified by PM)

| Suite / Command | Exact Command Line | Tests Passed | Exit Code | Provenance |
|---|---|:---:|:---:|---|
| **Sensor Lifecycle Decoupling** | `cargo test --bin dosi-tracker tracking::lifecycle::tests` | 4 | **0** | Verified by PM @ `8c1b201` |
| **Filter Inactive Health/Timing** | `cargo test --bin dosi-tracker tracking::input::tests::inactive_filter_does_not_update_health_or_key_down_state -- --exact` | 1 | **0** | Verified by PM @ `8c1b201` |
| **Filter Per-Sensor Gating** | `cargo test --bin dosi-tracker tracking::input::tests::per_sensor_permission_gating_in_filter -- --exact` | 1 | **0** | Verified by PM @ `8c1b201` |

#### 3. Baseline at Commit `95a4ba7` (Verified by PM)

| Suite / Command | Exact Command Line | Tests Passed | Exit Code | Provenance |
|---|---|:---:|:---:|---|
| **Tracking Lifecycle Suite** | `cargo test --bin dosi-tracker tracking::lifecycle::tests` | 11 | **0** | Verified by PM @ `95a4ba7` |
| **Worker Session Suite** | `cargo test --bin dosi-tracker worker::session::tests` | 10 | **0** | Verified by PM @ `95a4ba7` |

*Supervisory Note on `95a4ba7`:* Bounded synthetic unit evidence ONLY. It is NOT acceptance of native hook detachment/runtime, full product readiness, or whole product readiness.

---

### B. PM Independent Review at Clean HEAD `fb80245ceb5cd6500e2f602f22c0a27a10be8095`

**Supervisory Determination:** PM INDEPENDENT SOURCE REVIEW FAILED — CORRECTION REQUIRED, NOT PHASE ACCEPTANCE.
The claim that the consent prerequisite was complete was rejected. Four concrete source defects were identified:
1. `clients/windows/src/config.rs`: `LocalConsent::Granted(CapturePermissions)` reused `CapturePermissions` which carries `#[serde(default)]` with historically permissive config defaults (`screenshot: true`, `keyboard: true`, `mouse: true`, etc.). Deserializing a partial Granted JSON payload silently broadened permissions and granted omitted sensors.
2. `clients/windows/src/config.rs`: `LocalConsent::save_to` directly called `std::fs::write(path, data)`, risking partial writes or file corruption on failure and contradicting atomic durability.
3. `clients/windows/src/worker.rs`: `update_consent` assigned the grant then ignored `let _ = self.cfg.consent.save()`. Write failure could start sensors without persistence or leave an old grant on disk after user revocation without surfacing errors.
4. `clients/windows/src/worker/tests.rs`: Worker consent test commands called the production persistence handler, risking touching or corrupting the real user's `%APPDATA%\DosiTracker\consent.json`.

---

### C. Corrective Implementation (Privacy Schema, Durability & Isolation)

1. **Consent-Specific Default-Deny Schema (`ConsentedPermissions`)**:
   - Greenfield `ConsentedPermissions` struct where `Default` strictly enforces all-false permissions (`screenshot: false, webcam: false, keyboard: false, mouse: false, active_window: false, running_programs: false, blur_screenshots: true`).
   - `LocalConsent::Granted(ConsentedPermissions)` ensures partial JSON deserialization (e.g. `{"Granted": {"keyboard": true}}`) strictly defaults all omitted capabilities to `false`.
   - Global config and server defaults in `CapturePermissions` remain untouched, preserving server/config compatibility.
2. **Failure-Aware Atomic Replacement (`atomic_write_file`)**:
   - Writes to a sibling ephemeral file (`.tmp-consent-<uuid>.tmp`) in the same directory, flushes data to disk via `file.sync_all()`, and executes atomic file replacement via `std::fs::rename` (Win32 `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` semantics).
   - If writing, syncing, or renaming fails, the temp file is removed and the prior valid file is preserved without corruption.
   - *Durability limits documented honestly:* Atomic replacement guarantees all-or-nothing file replacement and crash consistency on standard Windows NTFS volumes; catastrophic sudden power-loss before hardware write-cache flushing or external antivirus filesystem lockouts can still cause I/O failures, which are surfaced to the user.
3. **Safe Worker State Machine & Actionable Error Reporting**:
   - `update_consent` enforces safe transitions:
     - **Granting:** Consent is saved to disk *first*. If persistence fails, the grant is rejected, sensors are NOT started, and an actionable error (`consent_error`) and `consent_unsynced: true` are surfaced.
     - **Revoking:** Sensors are immediately stopped in memory (`apply_sensor_policy(false, None)` and `reconcile(now, EndReason::Pause)`) before/regardless of disk save result. If disk save fails, the user is notified that revocation is active in-memory but non-durable on disk across restart, and `!self.consent_unsynced` in `should_track()` strictly prevents unintended tracking resumption.
   - UI (`clients/windows/src/ui/settings.rs`) displays prominent critical alert banners when `consent_error` is set or `consent_unsynced` is true.
4. **Injected Storage Boundary (`ConsentStore`)**:
   - Introduced `ConsentStore: Send + Sync` trait with `FileConsentStore` (production) and `MemoryConsentStore` (in-memory fake with atomic counters and fault injection `set_fail_saves(bool)`).
   - Decoupled `AppConfig::load()` from direct on-disk consent reading.
   - `Worker::new` accepts injected `Arc<dyn ConsentStore>`. All synthetic test harnesses (`Harness::new`, `harness_at`, `harness_with_consent`) strictly inject `MemoryConsentStore`.
   - Proved 100% test isolation: synthetic tests never access real `%APPDATA%\DosiTracker\consent.json`.

---

### D. Worker-Reported Test Execution (Current Working Tree)

All tests executed in `clients/windows` with `RUST_BACKTRACE=1`. Synthetic QA only; no real Win32 input hooks, foreground window enumeration, desktop collection, screen capture, or webcam capture were invoked during execution.

| Target Suite | Exact Command Line | Tests Passed | Ignored | Time | Exit Code | Evidence Level |
|---|---|:---:|:---:|:---:|:---:|:---:|
| **Compilation Gate** | `cargo check --bin dosi-tracker` | N/A | 0 errors, 0 warnings | 1.61s | **0** | **[Worker-Reported]** |
| **Config & Atomic Persistence** | `cargo test --bin dosi-tracker config::tests` | 14 | 0 | 0.03s | **0** | **[Worker-Reported]** |
| **All Consent Tests (UI+Worker)** | `cargo test --bin dosi-tracker consent` | 28 | 0 | 0.02s | **0** | **[Worker-Reported]** |
| **Tracking Lifecycle Suite** | `cargo test --bin dosi-tracker tracking::lifecycle::tests` | 11 | 0 | 0.10s | **0** | **[Worker-Reported]** |
| **Worker Session Suite** | `cargo test --bin dosi-tracker worker::session::tests` | 10 | 0 | 0.00s | **0** | **[Worker-Reported]** |

#### Detailed 15-Scenario Consent Lifecycle Test Matrix (`worker::tests::consent_*`)
1. `consent_1_fresh_config_absent`: Fresh config with `LocalConsent::Absent` yields `TrackerStatus::ConsentRequired`, 0 sensor starts, 0 running drivers.
2. `consent_2_legacy_config_absent`: Deserializing legacy empty JSON defaults to `LocalConsent::Absent`; missing `consent.json` fails closed to `Absent`.
3. `consent_3_explicit_granular_grant`: Granting keyboard-only attaches input hooks for keyboard without mouse or active window samplers.
4. `consent_4_decline_prevents_tracking`: Explicit decline sets `Revoked`, keeps status `ConsentRequired`, ignores project selection/resume attempts.
5. `consent_5_revoke_while_active`: Revoking while actively tracking immediately terminates in-flight blocks and detaches sensors before acknowledge.
6. `consent_6_policy_narrowing`: User unchecking capability stops sampler; workspace policy disallowing keyboard/mouse stops input hooks immediately.
7. `consent_7_project_workspace_logout_transition`: Sign-out stops sensors; signing in when consent is revoked does not auto-start tracking.
8. `consent_8_persisted_explicit_choice`: Exact granular choices round-trip to disk with full fidelity via atomic replacement.
9. `consent_9_corrupted_config_fail_closed`: Empty file, malformed JSON, and schema corruptions in `consent.json` fail closed to `LocalConsent::Absent`.
10. `consent_10_pause_resume_quit`: Truthful pause/resume; revoking while paused prevents resume; revoking during lock prevents resume on unlock.
11. `consent_11_no_duplicate_thread_start_on_repeated_user_actions`: Repeated consent grants, resumes, and project picks never duplicate threads.
12. `consent_12_grant_persistence_failure_blocks_sensors_and_reports_error`: Fault injection on grant save keeps sensors stopped (0 starts), sets `consent_error` and `consent_unsynced: true`.
13. `consent_13_revoke_persistence_failure_stops_sensors_and_reports_non_durable`: Fault injection on revoke immediately stops sensors in memory, reports non-durable persistence error honestly, and blocks resume.
14. `consent_14_fake_worker_isolation_proves_no_disk_access`: Proves all test harnesses use `MemoryConsentStore`, verifying save count increments in memory without touching filesystem.
15. `consent_15_partial_grant_omitted_capabilities_strictly_default_deny`: Deserializing partial grant `{"Granted": {"mouse": true}}` enables mouse only while keyboard, active window, and screenshot remain strictly denied.

---

## 4. Platform Readiness Matrix

| Evidence Dimension | Windows (Rust) | macOS (Swift) | Linux (Target) |
|---|---|---|---|
| **1. Implemented Source** | ⚠️ Corrective source present (`clients/windows`: egui UI, Win32 hooks, xcap, DPAPI, SQLite queue, `ConsentedPermissions`, `ConsentStore`, `atomic_write_file`); native interactive hooks, packaging, and signing open | ⚠️ Source present (`clients/macos`: SwiftUI, ScreenCaptureKit, Keychain, SQLite queue); source presence is NOT proof of complete implementation; TCC consent, runtime execution, packaging, and notarization open | ❌ **Unresolved Gap** (`clients/linux` crate not yet created) |
| **2. Synthetic / Mock Unit Tests** | ⚠️ **Worker-Reported Only** (28 consent tests, 11 lifecycle tests, 10 session tests, 14 config tests passing locally; awaiting PM independent review) | ⚠️ Unverified locally (Swift test files present in source; not executed locally) | ❌ 0 tests (No crate) |
| **3. Toolchain / Compile / Build** | ✅ Verified locally (Rustc / Cargo 1.97.1 on Windows 11 host) | ❌ Unverified locally (Host lacks macOS toolchain / Xcode) | ❌ Unverified locally (Host lacks Linux build container / environment) |
| **4. Native Interactive Runtime** | ⚠️ **Unverified** (Unit tests run synthetically via `FakeSensorDriver`, `MemoryConsentStore`, `ConsentViewModel`; interactive Win32 desktop hooks unverified in QA) | ⚠️ **Outstanding** (Requires physical macOS execution environment) | ❌ **Outstanding** (Requires Linux X11/Wayland execution environment) |
| **5. Packaging / Install / Uninstall** | ⚠️ Partial (NSIS `.nsi` script present; silent install/uninstall unverified in CI) | ⚠️ Partial (`build-app.sh` creates bundle; DMG packaging unverified) | ❌ **Unresolved Gap** (No `.deb`, AppImage, or Flatpak manifest) |
| **6. Code Signing & Notarization** | ⚠️ **Outstanding Prerequisite** (Unsigned; Authenticode cert required; signing does not guarantee immediate SmartScreen elimination; SmartScreen must not be bypassed) | ⚠️ **Outstanding Prerequisite** (Unsigned; Apple Developer ID & Notarization required) | ⚠️ **Outstanding Prerequisite** (GPG package signing required) |

---

## 5. Next Steps

1. **Awaiting PM Independent Review**:
   - PM independently executes selected synthetic tests to verify the corrective implementation slice (`config::tests`, `consent` tests, `tracking::lifecycle::tests`, `worker::session::tests`).
2. **Subsequent Phasing (Strictly Awaiting PM Authorization)**:
   - Backend schema nullability migration (`Activity.cs` metric nullability) and cross-workspace tenant isolation tests remain deferred until PM authorization.
   - Do NOT proceed to metric migrations or mark this phase passed until PM independently accepts.



