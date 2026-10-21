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
| `clients/windows/src/worker/tests.rs` | Injected affirmative consent in `harness_at`; added task barrier in quit regression test proving sensor stop before upload wait; added `worker_fails_closed_when_consent_is_absent`; aligned `FakeMachine::is_tracking_active` with `Tracker::is_tracking_active`; updated `FakeMachine` to use `ProvenanceScreenCapture`. |
| `clients/windows/src/worker.rs` | Updated `Machine` trait and `Worker` struct to store and hand off `ProvenanceScreenCapture`, validating screenshot provenance generation against the snapshot publication era. |
| `docs/PRODUCT_ACCEPTANCE.md` | Reconciled evidence levels to distinguish PM-verified tests from worker-reported tests; clarified that accumulator tests do not prove absence of raw keystrokes across the pipeline; noted fail-closed consent integration; updated consent and isolated regression test counts. |
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

### D. PM Source-Confirmed Review at HEAD `cf4910d` & Targeted Defect Identification

**Supervisory Determination:** PM SOURCE-CONFIRMED CORRECTION — SAME UNACCEPTED WINDOWS CONSENT PHASE.
At clean HEAD `cf4910d`, the PM independently inspected the diff and worker consent regressions:
- *Positive Findings:* Dedicated default-deny `ConsentedPermissions`, injected `ConsentStore` boundary, and surfaced persistence failures were recognized as source improvements.
- *Privacy Defect Identified:* `Worker::update_consent` matched every `LocalConsent::Granted(_)` as an expanding grant and invoked `consent_store.save` synchronously *before* narrowing/stopping existing sensors. A user disabling keyboard/window/screenshot while retaining another capability still sent `Granted(new narrower choices)`. Previously authorized native drivers remained active under the OLD policy throughout blocked/slow filesystem I/O.
- *Directive:* Enforce withdrawn permissions BEFORE persistence can block; establish fail-closed effective state on failed save with honest restart warnings; add deterministic synthetic regressions using an injected blocking store and barriers; keep all native/runtime/signing gaps open; mark evidence as worker-reported.

---

### E. Implementation of Bounded Correction: Monotonic Narrowing Before Persistence

1. **Monotonic Per-Capability Narrowing (`ConsentedPermissions::intersect`, `LocalConsent::narrow_to`)**:
   - `ConsentedPermissions::intersect(&self, other: &Self) -> Self`: Computes the strict intersection of permissions. A capability is true ONLY IF it was true in both grants. Defense-in-depth: `blur_screenshots` is kept true if either requires it.
   - `LocalConsent::narrow_to(&self, target: &Self) -> Self`: Calculates the monotonic staged consent prior to storage:
     - `(Granted(old), Granted(new)) => Granted(old.intersect(new))`
     - `(Absent, _) => Absent`
     - Otherwise `Revoked`.
   - Any capability withdrawn in the user's request is revoked immediately in memory. No unpersisted capability expansion is granted before persistence succeeds.
2. **Immediate Sensor Withdrawal & Capture Quiescence**:
   - `Blocks::narrow_block_permissions`: Immediately updates permissions on both the pending stretch policy and the current open block, guaranteeing that `capture_due` cannot schedule screenshots for withdrawn capabilities during in-flight blocks.
   - Active screen buffer quiescence: `self.screens.clear()` drops pending screenshots immediately if screenshot permission is withdrawn.
   - Sensor drivers immediately updated via `apply_sensor_policy` *before* `consent_store.save(&consent)` is called.
3. **Fail-Closed Effective State & Durability Warnings on Save Failure**:
   - If disk persistence fails:
     - All sensors are stopped immediately (`machine.apply_sensor_policy(false, None)`).
     - State marked unsynced (`consent_unsynced = true`), blocking tracking resumption (`!consent_unsynced` in `should_track()`).
     - Displayed consent in `self.cfg.consent` set to fail-closed state (`Absent` if previously Absent, otherwise `Revoked`), ensuring the UI does not falsely display accepted withdrawal or full success.
     - Actionable alert surfaced: `"Failed to persist consent changes to disk: {e}. Tracking stopped (fail-closed) to prevent non-durable sensor operation. Changes/revocation are non-durable across app restart: prior on-disk consent remains and will reload on restart until disk save succ4. **Deterministic Blocking Store & Barrier Regressions**:
   - `MemoryConsentStore` augmented with `set_save_hook` to allow synchronous assertions and multi-threaded barrier synchronization while `save()` is blocked in flight.
   - 4 new synthetic regression tests added (`worker::tests::consent_16` through `consent_19`):
     - `consent_16`: Proves withdrawn sensors (keyboard, active_window, screenshot, webcam) are stopped *while save is blocked*, retained mouse remains active, and save completes with narrower policy.
     - `consent_17`: Mixed narrowing + expansion request proves withdrawn keyboard stops before save, newly requested webcam is NOT active during blocked save, and webcam activates only after persistence succeeds.
     - `consent_18`: Save failure proves fail-closed halt of all sensors, `consent_unsynced: true`, fail-closed `Revoked` displayed consent, restart durability warning, and resume rejection.
     - `consent_19`: Multi-threaded `Arc<Barrier>` test proves the worker thread has stopped withdrawn sensors while literally suspended inside `consent_store.save`.
5. **Tracker Acquisition Boundary Hardening (`AcquisitionProvider` Seam & Explicit Authorization)**:
   - Defect resolved: `Tracker` previously relied on `is_tracking_active()` (input OR usage driver running) and caller history (`BlockSpec.perms`) to decide on new captures, allowing withdrawn webcam or processes to be sampled if another sensor (e.g. mouse) was running, or suppressing webcam under a legitimate webcam-only policy.
   - Decoupled acquisition from OS APIs via `AcquisitionProvider` trait (`NativeAcquisitionProvider` in production, `FakeAcquisitionProvider` with atomic call counters in tests).
   - `Tracker` tracks explicit effective authorization state (`TrackerAuth: active + perms`), updated monotonically under lock by `apply_sensor_policy`.
   - `Tracker::capture_screen` gates on `auth.active && auth.perms.screenshot` with conservative blur enforcement (`blur || auth.perms.blur_screenshots`).
   - `Tracker::snapshot` strictly intersects historical block permissions with CURRENT effective authorization for every NEW acquisition (`current_window`, `running_programs`, `webcam_jpg_base64`, `screen`), while preserving legitimately authorized historical accumulators (keys, clicks, usage slices).
   - `Tracker::current_app` returns `None` unless active tracking and `active_window` are currently authorized.
   - Added 7 isolated regression tests exercised directly against production `Tracker` with `FakeAcquisitionProvider`:
     - `tracker_consent_webcam_withdrawn_while_mouse_retained_stale_spec_zero_camera_calls`: Proves withdrawn webcam with stale spec results in 0 camera calls.
     - `tracker_consent_screenshot_withdrawn_direct_capture_screen_zero_calls`: Proves withdrawn screenshot returns `None` and invokes 0 provider calls.
     - `tracker_consent_running_programs_withdrawn_active_window_retained_zero_process_calls`: Proves withdrawn running programs with active window retained results in 0 process calls while window sample succeeds.
     - `tracker_consent_inactive_or_paused_with_stale_spec_zero_acquisitions`: Proves inactive/paused state with stale spec results in 0 screen, camera, window, or process acquisitions.
     - `tracker_consent_legitimately_active_webcam_only_policy_invokes_camera_without_input_or_usage_drivers`: Proves webcam-only policy activates camera provider despite input and usage drivers not running.
     - `tracker_consent_held_block_safe_finalization_preserves_historical_accumulators_without_new_samples`: Proves held blocks finalized after revocation execute 0 new captures/samples while preserving historical keys, clicks, and slices.
     - `tracker_consent_conservative_blur_applies_if_either_current_or_block_requires_blur`: Proves conservative blur is applied whenever current policy or block policy requires blur.
6. **Per-Capability Admission/Revocation Synchronization & Decoupled Process Enumeration**:
   - Defect 1 resolved: `Tracker::snapshot` previously cloned authorization once at method entry and reused it across sequential provider calls. If an earlier call (`current_window`) was delayed while revocation occurred, a subsequent fresh `capture_webcam` was admitted under stale permissions. Additionally, `current_app` previously held its read guard across provider execution, risking blocking `apply_sensor_policy`.
   - Architectural resolution:
     - Direct query entry points (`capture_screen` and `current_app`) evaluate authorization under a brief read lock and validate generation tokens upon return.
     - Within `Tracker::snapshot`, capability era generation tokens (`window_token`, `progs_token`, `screen_token`, `webcam_token`) are captured upfront under a brief read lock at method entry.
     - Sequential provider execution performs a fresh boolean admission check under a brief read lock immediately before invoking each provider (`current_window`, `running_programs`, `capture_webcam`). If unauthorized, the provider call is skipped entirely (zero provider calls).
     - Read locks are NEVER held across provider calls. `apply_sensor_policy` avoids authorization lock contention. IMPORTANT: `apply_sensor_policy` does NOT complete instantaneously; it synchronously posts quit messages and joins native sensor driver threads (`driver.stop_input`, `driver.stop_usage`).
     - Native hardware limitation documented honestly: Synchronous OS APIs (Windows MediaFoundation, GDI/xcap, EnumWindows) cannot be cancelled mid-flight without unsafe thread aborts. Subsequent admissions are immediately blocked (zero provider calls), and any admitted in-flight capture that finishes post-revocation is verified on exit and discarded immediately.
   - Defect 2 resolved: `snapshot` previously checked `driver.is_usage_running()` for `running_programs`, but `apply_sensor_policy` only starts that driver for `active_window`. Process enumeration was silently skipped under a valid `running_programs=true, active_window=false` grant. Decoupled `running_programs` from the active-window driver, querying the process list based strictly on active authorization without starting disallowed foreground samplers.
7. **Bounded Generation-Aware Admission and Result Validation (ABA Immunity)**:
   - Defect resolved: `TrackerAuth.epoch` was incremented on policy changes but never read by result validation checks; post-admission checks evaluated only current booleans (`auth.active && auth.perms.<cap>`). An acquisition blocked inside a provider could have consent revoked and then re-granted (ABA transition), incorrectly accepting old captures from an invalidated consent interval.
   - Architectural resolution:
     - Defined `CapabilityGenerations` with per-capability generation tokens: `screen`, `webcam`, `window`, and `processes`.
     - `TrackerAuth::update(&mut self, active: bool, perms: Option<&CapturePermissions>)` advances capability generations on actual transitions: revocation (active/permitted to inactive/denied), granting (unauthorized to authorized), or tightening (blur required).
     - Semantic generation policy preserves generations on routine unchanged policy publication (`Worker::publish` ticks), ensuring legitimate in-flight sampling is not falsely invalidated.
     - Single final post-provider reconciliation: once all sequential provider calls complete, a single post-execution validation under lock validates all acquired results against current authorization, the upfront era generation tokens, and (for screen) the handed-off screen provenance generation token.
     - Conservative rejection after pre-admission generation changes: if a capability undergoes an authorization transition (revocation, regrant, or blur tightening) after snapshot entry, a subsequent provider may be admitted under new boolean permissions if re-granted, but its resulting value will be conservatively discarded at final publication reconciliation because the current generation token will no longer match the upfront era token.
     - `self.usage.record(ended_at, ...)` is deferred to this final reconciliation and invoked if and only if `window_valid` passes, guaranteeing that invalidated window samples never contaminate historical usage.
     - Clarified admission contract: a short read-lock check followed by an unlocked provider invocation linearizes logical admission, but does not provide atomic physical provider entry with revocation. Discarding the result prevents invalidated data from entering blocks or storage; it is post-admission validation, not hardware cancellation or sensor stop.
     - Snapshot handoff applies conservative blur across BOTH current authorization and block policy: `!(auth.perms.blur_screenshots || perms.blur_screenshots)`.
   - Added 9 new deterministic regressions exercising production Tracker with fake drivers/acquisition (21 total isolated regressions):
     - `tracker_consent_capture_screen_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds`: Proves blocked screen capture during revoke/regrant is rejected upon provider release, and fresh post-regrant capture succeeds.
     - `tracker_consent_webcam_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds`: Proves blocked webcam capture during revoke/regrant is rejected upon provider release, and fresh post-regrant capture succeeds.
     - `tracker_consent_current_app_and_snapshot_window_revocation_regrant_aba_rejected`: Proves blocked current_app and snapshot window queries during revoke/regrant are rejected upon provider release, and fresh call succeeds.
     - `tracker_consent_running_programs_revocation_regrant_aba_rejected`: Proves blocked running_programs query during revoke/regrant is rejected upon provider release, and fresh query succeeds.
     - `tracker_consent_unchanged_policy_republish_does_not_invalidate_in_flight_acquisition`: Proves routine unchanged policy republish does not invalidate legitimate in-flight acquisitions.
     - `tracker_consent_final_publication_rejects_earlier_results_after_later_provider_aba`: Multi-sensor later-webcam barrier regression: gathers earlier window, process, and screen data; blocks webcam; executes ABA revoke/regrant transition before provider release; proves invalidated NEW values (window, processes, screen, webcam) are excluded at final activity assembly and `usage.record` is skipped (zero usage contamination); preserves genuinely authorized historical accumulators (keyboard hits, mouse clicks, historical window slices); and verifies fresh post-regrant acquisitions.
     - `tracker_consent_screenshot_rejected_after_later_webcam_aba_revocation`: Isolated screenshot ABA regression while webcam remains continuously authorized: gathers earlier screen capture, blocks webcam, withdraws ONLY screenshot while keeping active=true and webcam=true, regrants screenshot before releasing webcam, and proves old screenshot absent while unchanged-authorized webcam result is present, with exact provider call counts and fresh post-regrant screenshot success.
     - `tracker_consent_screenshot_blur_tightening_during_blocked_webcam`: Proves unblurred screen capture is dropped if blur requirement is tightened while webcam is blocked.
8. **Source-Level Harness Isolation & Screenshot Provenance Audit**:
   - **Harness Isolation Trace**: Traced all 21 `tracker_consent_*` regressions (`tracking/mod.rs:1302-2245`). Every test constructor instantiates `Tracker::new_fake_with_acquisition()` (`tracking/mod.rs:423-431`), injecting purely in-memory atomics (`FakeSensorDriver`, `FakeAcquisitionProvider`) and static fixture metadata (`DeviceInfo::fixture()`). Real OS hooks (`Win32SensorDriver`), native capture providers (`NativeAcquisitionProvider`), desktop GDI/xcap, live webcam hardware, and user consent store writes are never invoked.
   - **Consent Store Isolation Audit**: Traced worker consent test suites (`worker/tests.rs:568-630, 2259-2280`). All 22 worker consent scenarios inject `MemoryConsentStore` with atomic counters (`load_count`, `save_count`) and programmatic fault hooks (`set_fail_saves`, `set_save_hook`), bypassing disk entirely. File-based persistence tests (`consent_8`, `consent_9`, `config::tests`) execute exclusively inside isolated UUID temporary directories (`std::env::temp_dir().join("dosi-test-consent-...")`) and clean up immediately; production `%APPDATA%` paths are never touched.
   - **End-to-End Provenance & Serialization Pipeline Audit**: Traced `capture_screen -> pending worker handoff -> snapshot -> serialization`:
     - `Tracker::capture_screen` captures with generation provenance, wrapping results in `ProvenanceScreenCapture { capture, generation: token }`.
     - `Worker::record` retains provenance token across pending blocks in `self.screens` and hands off `ProvenanceScreenCapture` directly into `Tracker::snapshot`.
     - `Tracker::snapshot` enforces final post-provider reconciliation against upfront era tokens and blur policy (`screen_valid = auth.active && auth.perms.screenshot && auth.generations.screen == screen_token && sc.generation == screen_token && (sc.capture.blurred || !blur_required)`).
     - Foreground window sample recording (`self.usage.record(ended_at, ...)`) is strictly gated under `window_valid`, preventing invalidated samples from contaminating historical usage slices.
     - `Activity` serialization (`model.rs:143-220`) configures `#[serde(skip_serializing_if = "Option::is_none")]` for `screenshot_png_base64`, `screenshot_thumb_jpg_base64`, and `webcam_jpg_base64`, ensuring invalidated/rejected data is entirely omitted from JSON payloads.
   - **Independent QA Status**: All unit test counts remain Worker-Reported only, pending PM independent verification.

---

### F. Worker-Reported Test Execution (Current Working Tree)

All tests executed in `clients/windows` with `RUST_BACKTRACE=1`. Synthetic QA only; no real Win32 input hooks, foreground window enumeration, desktop collection, screen capture, or webcam capture were invoked during execution.

| Target Suite | Exact Command Line | Tests Passed | Ignored | Time | Exit Code | Evidence Level |
|---|---|:---:|:---:|:---:|:---:|:---:|
| **Compilation Gate** | `cargo check --bin dosi-tracker --tests` | N/A | 0 errors, 0 warnings | 3.66s | **0** | **[Worker-Reported]** |
| **Config & Atomic Persistence** | `cargo test --bin dosi-tracker config::tests` | 15 | 0 | 0.07s | **0** | **[Worker-Reported]** |
| **All Consent Tests (UI+Worker+Tracker)** | `cargo test --bin dosi-tracker consent` | 57 | 0 | 0.05s | **0** | **[Worker-Reported]** |
| **Tracker Consent Isolated Regressions** | `cargo test --bin dosi-tracker tracker_consent` | 21 | 0 | 0.02s | **0** | **[Worker-Reported]** |
| **Tracking Subsystem & Boundary** | `cargo test --bin dosi-tracker tracking::tests` | 32 | 0 | 0.02s | **0** | **[Worker-Reported]** |
| **Tracking Lifecycle Suite** | `cargo test --bin dosi-tracker tracking::lifecycle::tests` | 11 | 0 | 0.10s | **0** | **[Worker-Reported]** |
| **Worker Session Suite** | `cargo test --bin dosi-tracker worker::session::tests` | 10 | 0 | 0.00s | **0** | **[Worker-Reported]** |
| **Worker Blocks Suite** | `cargo test --bin dosi-tracker worker::blocks::tests` | 19 | 0 | 0.00s | **0** | **[Worker-Reported]** |
| **Full Worker Test Suite** | `cargo test --bin dosi-tracker worker::tests` | 84 | 2 (e2e backend) | 2.05s | **0** | **[Worker-Reported]** |

#### Detailed 22-Scenario Consent Lifecycle Test Matrix (`worker::tests::consent_*`)
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
16. `consent_16_narrowing_stops_withdrawn_sensors_before_blocked_save_completes`: Injected save hook proves withdrawn sensors stop *before* save blocks, retained mouse continues, and save finishes cleanly.
17. `consent_17_mixed_narrowing_and_expansion_request_under_blocked_store`: Proves withdrawn permissions halt immediately while unpersisted expansions remain disabled until persistence succeeds.
18. `consent_18_blocked_save_failure_fails_closed_and_prevents_resume`: Injected write failure proves fail-closed transition (all sensors stopped), `consent_unsynced: true`, honest restart warning, and resume prevention.
19. `consent_19_multithreaded_barrier_proves_withdrawn_sensors_stopped_while_store_blocked`: Cross-thread barrier synchronization proves sensors are halted while the worker thread is suspended in `consent_store.save`.
20. `consent_20_withdrawn_sensors_stopped_before_reconcile_record_blocks`: Deterministic channel synchronization proves native sensor drivers are stopped/narrowed *before* `reconcile` and `record` execute, with 0 driver queries or captures while `snapshot` is in flight.
21. `consent_21_held_blocks_do_not_acquire_withdrawn_screen_or_webcam_at_finalization`: Proves held blocks released at revocation do not trigger new screenshot or webcam captures (0 new captures), purge buffered captures, yet preserve authorized historical time and key/click counts.
22. `consent_22_mixed_narrowing_stops_withdrawn_sensors_before_reconcile_record_blocks`: Proves mixed narrowing stops withdrawn native drivers immediately ahead of reconciliation/tick, ensuring no fresh sampling occurs at block boundary while retained capabilities (e.g. mouse) continue.

---

## 4. Platform Readiness Matrix

| Evidence Dimension | Windows (Rust) | macOS (Swift) | Linux (Target) |
|---|---|---|---|
| **1. Implemented Source** | ⚠️ Corrective source present (`clients/windows`: egui UI, Win32 hooks, xcap, DPAPI, SQLite queue, `ConsentedPermissions`, `ConsentStore`, `atomic_write_file`, monotonic narrowing, `AcquisitionProvider` boundary); native interactive hooks, packaging, and signing open | ⚠️ Source present (`clients/macos`: SwiftUI, ScreenCaptureKit, Keychain, SQLite queue); source presence is NOT proof of complete implementation; TCC consent, runtime execution, packaging, and notarization open | ❌ **Unresolved Gap** (`clients/linux` crate not yet created) |
| **2. Synthetic / Mock Unit Tests** | ⚠️ **Worker-Reported Only** (57 consent tests, 21 tracker consent regressions, 32 tracking tests, 11 lifecycle tests, 10 session tests, 15 config tests, 19 block tests passing locally; awaiting PM independent review) | ⚠️ Unverified locally (Swift test files present in source; not executed locally) | ❌ 0 tests (No crate) |
| **3. Toolchain / Compile / Build** | ✅ Verified locally (Rustc / Cargo 1.97.1 on Windows 11 host) | ❌ Unverified locally (Host lacks macOS toolchain / Xcode) | ❌ Unverified locally (Host lacks Linux build container / environment) |
| **4. Native Interactive Runtime** | ⚠️ **Unverified** (Unit tests run synthetically via `FakeSensorDriver`, `MemoryConsentStore`, `ConsentViewModel`; interactive Win32 desktop hooks unverified in QA) | ⚠️ **Outstanding** (Requires physical macOS execution environment) | ❌ **Outstanding** (Requires Linux X11/Wayland execution environment) |
| **5. Packaging / Install / Uninstall** | ⚠️ Partial (NSIS `.nsi` script present; silent install/uninstall unverified in CI) | ⚠️ Partial (`build-app.sh` creates bundle; DMG packaging unverified) | ❌ **Unresolved Gap** (No `.deb`, AppImage, or Flatpak manifest) |
| **6. Code Signing & Notarization** | ⚠️ **Outstanding Prerequisite** (Unsigned; Authenticode cert required; signing does not guarantee immediate SmartScreen elimination; SmartScreen must not be bypassed) | ⚠️ **Outstanding Prerequisite** (Unsigned; Apple Developer ID & Notarization required) | ⚠️ **Outstanding Prerequisite** (GPG package signing required) |

---

## 5. Bounded Engineering Phase: Multi-Workspace Server-Side Authorization & Data Isolation

### A. Context & Concrete Source Hardening
Under the renewed product mission, an end-to-end trace from authenticated identity -> workspace resolution header -> repository scope was audited:
1. **Header Validation Hardening (`WorkspaceResolution.cs`)**:
   - `WorkspaceResolutionMiddleware` previously parsed `X-Workspace-Id` with `Guid.TryParse(rawHeader, out var workspaceId)`. When a caller provided `Guid.Empty` (`00000000-0000-0000-0000-000000000000`), `TryParse` succeeded.
   - An authenticated caller was subsequently checked against `WorkspaceRoleLookup.FindRoleAsync(Guid.Empty, userId)`. Because no membership record has `WorkspaceId == Guid.Empty`, lookup returned `null`, writing `403 Forbidden` (`Tracker:NotWorkspaceMember`). It did **NOT** reach `CurrentWorkspace.Change(workspaceId, role)` or bind `CurrentWorkspace.Id` to `Guid.Empty`.
   - **Hardening Implemented**: Updated `WorkspaceResolutionMiddleware` to require `Guid.TryParse(...) && workspaceId != Guid.Empty`, rejecting empty GUID headers immediately with `400 Bad Request` and error code `Tracker:WorkspaceRequired`. This is input validation hardening preventing an unnecessary database round-trip for a known invalid workspace ID, not a cross-tenant exploit.
2. **EF Core Repository & Query Filter Data Isolation Suite (`CrossWorkspaceIsolationTests.cs`)**:
   - **Scope**: Evaluates EF Core `IWorkspaceScoped` global query filters, repository queries, and cross-entity references under ambient `ICurrentWorkspace.Change` context (`REQ-ISO-01`).
   - The test double `WorkspaceRoleAwareAuthorizationService` was removed so that replacement test policies are not substituted for production authorization enforcement.
   - Tests:
     - `Caller_In_Workspace_A_Cannot_Get_Project_Owned_By_Workspace_B`: Throws `EntityNotFoundException`.
     - `Caller_In_Workspace_A_Cannot_Update_Project_Owned_By_Workspace_B`: Throws `EntityNotFoundException`.
     - `Caller_In_Workspace_A_Cannot_Delete_Project_Owned_By_Workspace_B`: Foreign delete is a no-op; project remains intact in Workspace B.
     - `Caller_In_Workspace_A_Project_List_Does_Not_Leak_Workspace_B_Projects`: Workspace B projects excluded from Workspace A list.
     - `Caller_In_Workspace_A_Cannot_Upload_Activity_To_Project_In_Workspace_B`: Throws `BusinessException(TrackerDomainErrorCodes.NotProjectMember)`.
     - `Caller_In_Workspace_A_Cannot_Access_Screenshot_Content_Owned_By_Workspace_B`: Throws `EntityNotFoundException`.
     - `Caller_In_Workspace_A_Cannot_See_Or_Remove_Workspace_B_Team_Members`: Foreign members excluded from roster; removal fails with `TrackerDomainErrorCodes.InvalidUser`.
     - `Caller_In_Workspace_A_Heartbeat_Does_Not_Attach_To_Workspace_B_Project`: Presence recorded in Workspace A, but ProjectId clamped to null.
     - `Dual_Member_Data_Context_Switching_Excludes_Foreign_Workspace_Data`: Verifies data context isolation across switches (under WS-A, only Project A is listed; under WS-B, only Project B is listed).
     - `WorkspaceRoleLookup_Trace_Direct_Repository_Check`: Direct repository check proving role lookup matches active membership, returns null for non-member, and returns null for `Guid.Empty`.
3. **Focused Production-Boundary Integration Harness (`WorkspaceResolutionIntegrationTests.cs`)**:
   - **Scope**: In-process integration harness composing actual `WorkspaceResolutionMiddleware` + real `WorkspaceRoleLookup` (querying SQLite `:memory:` membership) + actual production `WorkspaceRolePermissionValueProvider` (registered in ABP `TrackerDomainModule`).
   - Uses synthetic claims (`ClaimsPrincipal` with `AbpClaimTypes.UserId`) and SQLite `:memory:` without test doubles or replacement authorization services.
   - Tests:
     - `A_Only_Caller_Spoofing_Workspace_B_Is_Rejected_With_403_Before_Downstream`: Rejects cross-workspace spoofing with `403 Forbidden` (`Tracker:NotWorkspaceMember`) and halts pipeline before downstream execution.
     - `Legitimate_Dual_Member_Owner_In_A_Vs_Member_In_B_Uses_Real_Production_Permissions`: Resolves real role from SQLite membership; verifies real `WorkspaceRolePermissionValueProvider` grants Owner permissions (`Projects.Create`, `Billing.Default`, `Projects.Delete`) under Workspace A, and denies them under Workspace B.
     - `Missing_Or_Unknown_Workspace_Membership_Is_Denied_With_403_Before_Downstream`: Denies unknown workspace IDs with 403 before downstream.
     - `Empty_Guid_Workspace_Header_Is_Refused_With_400_Validation_Before_Downstream`: Refuses empty GUID with 400 before downstream.
     - `Anonymous_Caller_With_Workspace_Header_Leaves_Workspace_Unset`: Leaves workspace unset when unauthenticated.

### B. Execution Evidence & Provenance Boundaries

| Dimension | Exact Target / Command | Tests Passed | Failed | Duration | Exit Code | Provenance |
|---|---|:---:|:---:|:---:|:---:|---|
| **Compilation Gate** | `dotnet build backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj` | N/A | 0 errors, 27 warnings | 10.53s | **0** | **[Worker-Reported (Build)]** |
| **Host Unit Regressions** | `dotnet test test/Dosi.Tracker.HttpApi.Host.Tests/Dosi.Tracker.HttpApi.Host.Tests.csproj` | 117 | 0 | 0.76s | **0** | **[Worker-Reported @ 39dc149]** |
| **Independent PM Execution (`qa_workspace_phase.ps1`)** | Filtered `WorkspaceResolutionTests` & `EfCoreCrossWorkspaceIsolationTests` | null | null | null | null | **[Approval-Pending / Explicitly Not Run]** |
| **New Integration Harness Execution** | `EfCoreWorkspaceResolutionIntegrationTests` & updated `EfCoreCrossWorkspaceIsolationTests` | null | null | null | null | **[Approval-Pending / Explicitly Not Run]** |

---

## 6. Bounded Engineering Phase: Workspace Invitation Recipient Decline Lifecycle (`REQ-WS-07`)

### A. Context & Source Audit Reconciliation
Following PM inspection of source at clean `2a83bc1`, documentation for `REQ-WS-07` was reconciled against existing production codebase:
1. **Pre-Existing Expiry & TTL**:
   - `WorkspaceInvitation.ExpiresAt`, `WorkspaceInvitation.IsUsable(DateTime now)` with active-state check and comparison against `ExpiresAt`.
   - `TeamAppService.InviteMemberAsync` issues tokens configured with `Clock.Now.AddDays(7)`.
   - Expiry was already present in domain and application service source; not rebuilt.
2. **Pre-Existing Admin Revocation**:
   - `ITeamAppService.DeleteInvitationAsync(Guid id)` and `TeamAppService.DeleteInvitationAsync(Guid id)` scoped by invitation ID and `CurrentWorkspace.Id`.
   - Admin invitation withdrawal was already present in existing source; not rebuilt.
3. **Actual Missing Requirement — Recipient Decline**:
   - While `AcceptInvitationAsync` and `GetInvitationAsync` existed on `IWorkspaceAppService` / `WorkspaceAppService`, no operation existed for an authenticated recipient to decline a pending invitation without joining the workspace or having prior workspace membership.

### B. Implementation Details
1. **Application Contract (`IWorkspaceAppService.cs`)**:
   - Added `Task DeclineInvitationAsync(string token);`.
2. **Domain & Application Service Implementation (`WorkspaceAppService.cs`)**:
   - Annotated with `[Authorize]` and `[WorkspaceNotRequired]` so callers do not need an active workspace header or prior membership.
   - **Evaluation Order**:
     1. Retrieves usable invitation first: `var invitation = await GetUsableInvitationAsync(token);`. This throws `BusinessException(TrackerDomainErrorCodes.InvitationNotFound)` if token is null, empty, or does not exist, and throws `BusinessException(TrackerDomainErrorCodes.InvitationExpired)` if expired or already accepted.
     2. Evaluates authenticated caller identity second: `var userId = CurrentUser.Id ?? throw new AbpAuthorizationException();`.
     3. Resolves user identity: `var user = await _userManager.GetByIdAsync(userId);`. This reads `user.Email` from identity storage. (Note: reading `user.Email` is an identity email check, not verification that `EmailConfirmed` is true).
     4. Enforces recipient identity match: `if (!invitation.IsFor(user.Email)) throw new BusinessException(TrackerDomainErrorCodes.InvitationEmailMismatch);`. Uses domain case-insensitive email match (`invitation.IsFor`), avoiding client-supplied email parameters or spoofing.
     5. Invalidation mechanism: executes schema-free pending row deletion via `_invitationRepository.DeleteAsync(invitation)`. Follows the existing admin delete pattern without requiring database migrations or schema alterations.
   - **Security, Transaction & Concurrency Trace**:
     - Never logs or returns raw tokens in error messages.
     - Decline modifies no existing memberships or acceptances and leaves other-workspace invitations intact.
     - **Concurrency Status — Unverified**: Existing tests are sequential, in-process SQLite in-memory tests where `TrackerEntityFrameworkCoreTestModule` explicitly enables `AddAlwaysDisableUnitOfWorkTransaction`. In production, both `AcceptInvitationAsync` and `RegisterAsync` resolve the invitation via `GetUsableInvitationAsync` *before* initiating their own explicit transactions. Therefore, deleting a row under a unit of work cannot be claimed to automatically prove multi-threaded race safety against concurrent accept/register-vs-decline. Concurrency behavior under concurrent execution remains **unverified** without concrete multi-threaded regression evidence; no race exploits or concurrency solutions are claimed.
3. **HTTP API Controller (`WorkspacesController.cs`)**:
   - Added endpoint `[HttpPost("{token}/decline")]` on `InvitationsController` (inheriting `TrackerController`, decorated with `[Authorize]` and `[WorkspaceNotRequired]`).
4. **Frontend API Contract (`frontend/src/lib/api-contract.ts`)**:
   - Exported `declineInvitationRoute: (token: string) => `/api/app/invitations/${encodeURIComponent(token)}/decline``.
   - Added test coverage in `frontend/src/lib/api-contract.test.ts` (30 passed, Verified by PM @ `3bebd70`).

### C. Execution Evidence & Provenance Boundaries

| Dimension | Exact Target / Command | Tests Passed | Failed | Duration | Exit Code | Provenance |
|---|---|:---:|:---:|:---:|:---:|---|
| **Isolated EF Core Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~InvitationDeclineTests"` | 10 | 0 | 9s | **0** | **[Worker-Reported (SQLite In-Memory)]** |
| **Host Route Reflection** | `dotnet test test/Dosi.Tracker.HttpApi.Host.Tests/Dosi.Tracker.HttpApi.Host.Tests.csproj --filter "FullyQualifiedName~ContractRoutesTests"` | 40 | 0 | 101ms | **0** | **[Worker-Reported (Unit Reflection Only — Not Authenticated HTTP)]** |
| **Frontend Contract Route Vitest** | `npx vitest run src/lib/api-contract.test.ts` | 30 | 0 | 1.22s | **0** | **[Verified by PM @ 3bebd70]** |
| **Frontend TypeScript Gate** | `npm run typecheck` | 0 errors | 0 | 3.5s | **0** | **[Worker-Reported (TypeScript)]** |
| **Historical PM Blocked Scripts** | `qa_workspace_phase.ps1` | null | null | null | null | **[Explicitly Not Run / No Repackaging]** |
| **Real Authenticated HTTP/Browser Journey** | End-to-end browser session decline journey | null | null | null | null | **[Pending Acceptance Requirement]** |

#### Test Cases Covered in `EfCoreInvitationDeclineTests` (10 total):
1. `Recipient_Can_Decline_Pending_Invitation_Without_Joining_Workspace`: Intended recipient declines -> invitation deleted from DB, `GetInvitationAsync` throws `TrackerDomainErrorCodes.InvitationNotFound`, user does not join workspace.
2. `Wrong_Email_Authenticated_Caller_Cannot_Decline_Another_Recipients_Invitation`: Authenticated stranger without target workspace membership is rejected with `TrackerDomainErrorCodes.InvitationEmailMismatch`; token remains intact and usable.
3. `Decline_By_Caller_With_Workspace_Membership_Throws_If_Not_Target_Email`: Authenticated caller who holds an actual active persisted membership (`WorkspaceRoles.Manager`) in the target workspace (and member of another workspace) attempting to decline another recipient's invitation is rejected with `TrackerDomainErrorCodes.InvitationEmailMismatch`; token remains usable and can be accepted by the intended recipient; caller's existing membership in target and other workspaces remains intact.
4. `Unauthenticated_Caller_Cannot_Decline_Invitation`: Unauthenticated caller throws `AbpAuthorizationException`; token remains intact.
5. `Declined_Token_Cannot_Be_Accepted_By_Existing_Account`: Attempting to accept a declined token throws `TrackerDomainErrorCodes.InvitationNotFound`.
6. `Declined_Token_Cannot_Be_Used_For_New_Registration`: Attempting to register with a declined token throws `TrackerDomainErrorCodes.InvitationNotFound`.
7. `Declining_One_Workspace_Invitation_Does_Not_Affect_Other_Workspace_Invitations`: In a multi-workspace scenario, declining one invitation throws `TrackerDomainErrorCodes.InvitationNotFound` for that token, while another workspace invitation for the same or other user remains valid and acceptable.
8. `Already_Accepted_Invitation_Cannot_Be_Declined_And_Existing_Membership_Preserved`: Attempting to decline an already accepted invitation throws `TrackerDomainErrorCodes.InvitationExpired`; existing accepted membership remains active and intact.
9. `Expired_Invitation_Cannot_Be_Declined`: Expired invitation throws `TrackerDomainErrorCodes.InvitationExpired`.
10. `Nonexistent_Or_Malformed_Token_Cannot_Be_Declined`: Unknown or whitespace token throws `TrackerDomainErrorCodes.InvitationNotFound`.

---

---

---

## 7. Bounded Engineering Phase: Hosted ASP.NET Core TestServer MVC Aggregate Authorization & Isolation Harness (`REQ-ISO-05`)

### A. Context & Architectural Boundary Correction
An earlier worker iteration relied on `InProcessHttpHandler` copying requests to `DefaultHttpContext` and mapping handwritten `MapGet`/`MapPost`/`MapDelete` delegates directly to `IProjectAppService`, manually emulating JSON and status/exception serialization without constructing an actual `TestServer`. This boundary gap could not prove conventional MVC controller discovery, model/query binding, or production ASP.NET Core exception-to-status mapping.

This harness correction replaces that manual mapping with a genuinely hosted in-process ASP.NET Core `TestServer` (`Microsoft.AspNetCore.TestHost`):
1. **Genuine Hosted ASP.NET Core MVC Pipeline (`WorkspaceHttpAuthorizationTests.cs`)**:
   - Hosted via `HostBuilder.UseAutofac().ConfigureWebHost(web => web.UseTestServer()...)`.
   - Dispatches real `HttpRequestMessage` instances through `TestServer.CreateClient()` across the full production ASP.NET Core pipeline: `UseRouting`, `UseAuthentication`, `UseUnitOfWork`, `UseWorkspaceResolution`, `UseAuthorization`, and `endpoints.MapControllers()`.
   - Real conventional MVC controller discovery: registers `options.ConventionalControllers.Create(typeof(TrackerApplicationModule).Assembly)` in `TrackerHttpApiServerTestModule`, matching production routing (`/api/app/project`, `/api/app/project/{id}`) rather than invented controller classes.
   - Real production action filter: registers `WorkspaceRequiredFilter` in `MvcOptions.Filters` to enforce header presence on conventional controller actions.
   - Restores actual ABP aggregate authorization in DI: `AbpAuthorizationService` + `PermissionChecker` + `WorkspaceRolePermissionValueProvider` (undoing `TrackerTestBaseModule`'s `AddAlwaysAllowAuthorization`).
   - Uses synthetic authentication handler (`TestAuthHandler`) to establish caller claims (`AbpClaimTypes.UserId`, `ClaimTypes.NameIdentifier`) without copying role policies or tampering with production token issuance.
   - Enforces SQLite in-memory `:memory:` persistence sharing the active connection across test fixture and `TestServer`, verifying exact entity state and zero-mutation guarantees upon authorization denials.

2. **Honest Diagnostic & Harness Setup Findings (Not Production Vulnerabilities)**:
   - *DI / Autofac Property Injection*: Initial `TestServer` construction using standard `AutofacServiceProviderFactory` threw `NullReferenceException` on `BasicRepositoryBase.CurrentTenant` because `LazyServiceProvider` was not property-injected. Resolved by configuring `HostBuilder.UseAutofac()` (which registers ABP's `AbpAutofacServiceProviderFactory` with property injection and interception) without duplicate `options.UseAutofac()` calls in `AddApplication`.
   - *SQLite In-Memory Database Sharing*: `TrackerEntityFrameworkCoreTestModule` creates a new `:memory:` SQLite connection on initialization. Reordering `ConfigureTestServerServices` after `AddApplication` ensures the test fixture's pre-opened, table-seeded SQLite connection is applied to the `TestServer`'s `DbContextOptions`, ensuring both client dispatch and fixture assertions query the identical SQLite database.
   - *Missing Header vs Guid.Empty Separation*:
     - `Guid.Empty` header: rejected directly by `WorkspaceResolutionMiddleware` with `400 BadRequest` (`Tracker:WorkspaceRequired`).
     - Missing header: passes `WorkspaceResolutionMiddleware`, then intercepted by `WorkspaceRequiredFilter` on conventional controllers which throws `BusinessException(TrackerDomainErrorCodes.WorkspaceRequired)` -> ABP maps to `400 BadRequest`. In the test harness, replacing `ICurrentWorkspace` with production `CurrentWorkspace` (`DefaultId => null`, rather than `TestCurrentWorkspace`'s hardcoded test fallback) allows `WorkspaceRequiredFilter` to truthfully evaluate `CurrentWorkspace.Id == null`.
   - *Conventional Controller Response Contract*: ABP's conventional controller for `CrudAppService.CreateAsync` returns the created DTO with HTTP `200 OK` (standard ASP.NET Core action return), not `201 Created`. Test assertions honestly reflect production HTTP status contracts.
   - *ABP Policy Error Code*: When an authenticated caller lacks policy permission (e.g. `Projects.Create`), ABP's aggregate authorization handler produces `Volo.Abp.Authorization:010002` ("Authorization failed! Given policy has not granted"), distinct from `010001` (unauthenticated).
   - *Non-Vacuous State Inspection*: The spoofing test pre-seeds a distinct sentinel project in target Workspace B. Persisted database state is inspected under explicit Workspace B context (`_currentWorkspace.Change(wsB, ...)`), proving the sentinel exists before and after denial (not merely an empty query result).
   - *Cross-Workspace Resource Read & Mutation*: Distinct sentinels A and B seeded. Caller selecting Workspace A (where caller is Manager):
     - GET `/api/app/project/{sentinelBId}` -> `404 NotFound` (`EntityNotFoundException`).
     - PUT `/api/app/project/{sentinelBId}` -> `404 NotFound` (`EntityNotFoundException`).
     - DELETE `/api/app/project/{sentinelBId}` -> `403 Forbidden` (`Projects.Delete` is withheld from Manager role per production contract).
     - Persisted target B and A entities and counts/content verified strictly unchanged.

3. **Boundary & Provenance Distinctions**:
   - **Component Composition vs Hosted MVC HTTP**: Distinguishes direct unit service invocations from genuine ASP.NET Core `TestServer` HTTP dispatch through conventional controller routing, model binding, middleware, and action filters.
   - **Worker Outcomes vs Independent QA Review**: All test executions below are worker tool outcomes. PM has **NOT** independently executed backend test commands.
   - **Open Acceptance Requirements**: Real browser session cookies (`__Host-Dosi-Workspace`), Next.js server actions, hosted OpenIddict token issuing/validation, native platform runtimes, and multi-threaded race conditions remain **Unverified / Pending Acceptance**.

### B. Execution Evidence & Provenance Boundaries

| Dimension | Exact Target / Command | Tests Passed | Failed | Duration | Exit Code | Provenance |
|---|---|:---:|:---:|:---:|:---:|---|
| **Hosted ASP.NET Core MVC TestServer** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~EfCoreWorkspaceHttpAuthorizationTests"` | 9 | 0 | 16.8s | **0** | **[Worker-Reported (Hosted MVC TestServer & Shared SQLite)]** |
| **Recipient Decline Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~InvitationDeclineTests"` | 10 | 0 | 9s | **0** | **[Worker-Reported (SQLite In-Memory)]** |
| **Host Route Reflection** | `dotnet test test/Dosi.Tracker.HttpApi.Host.Tests/Dosi.Tracker.HttpApi.Host.Tests.csproj --filter "FullyQualifiedName~ContractRoutesTests"` | 40 | 0 | 101ms | **0** | **[Worker-Reported (Unit Reflection Only)]** |
| **Frontend Contract Route Vitest** | `npx vitest run src/lib/api-contract.test.ts` | 30 | 0 | 1.22s | **0** | **[Verified by PM @ 3bebd70]** |
| **Historical PM Blocked Scripts** | `qa_workspace_phase.ps1` | null | null | null | null | **[Explicitly Not Run / No Repackaging]** |
| **Real Browser Session Acceptance** | Multi-tenant browser session & cookie verification | null | null | null | null | **[Pending Acceptance Requirement]** |

#### Test Cases Covered in `EfCoreWorkspaceHttpAuthorizationTests` (9 total):
1. `Unauthenticated_Request_To_Protected_Project_Endpoint_Is_Rejected_With_401`: Anonymous request to protected project endpoint is challenged by ASP.NET Core authentication middleware -> 401 Unauthorized (`Volo.Abp.Authorization:010001`).
2. `Empty_Workspace_Header_Guid_Empty_Is_Rejected_With_400_WorkspaceRequired_By_Resolution_Middleware`: Authenticated request with `Guid.Empty` workspace header is rejected by `WorkspaceResolutionMiddleware` -> 400 Bad Request (`Tracker:WorkspaceRequired`).
3. `Missing_Workspace_Header_Is_Rejected_With_400_WorkspaceRequired_By_Action_Filter`: Authenticated request with missing `X-Workspace-Id` header passes resolution middleware, then intercepted by `WorkspaceRequiredFilter` on conventional MVC controller -> 400 Bad Request (`Tracker:WorkspaceRequired`).
4. `Workspace_A_Only_Caller_Spoofing_Workspace_B_Header_Is_Rejected_With_403_NotWorkspaceMember`: Authenticated caller possessing membership only in Workspace A attempting to access Workspace B via header spoofing is rejected with 403 Forbidden (`Tracker:NotWorkspaceMember`). Non-vacuous SQLite DB inspection under explicit Workspace B context confirms pre-seeded B sentinel exists and remains completely unchanged (count remains 1, no mutation inserted).
5. `Cross_Workspace_Resource_Id_Read_And_Mutation_While_Selecting_Workspace_A_Are_Rejected_And_Target_Unchanged`: Seeded distinct sentinels A and B. Caller selecting Workspace A attempts cross-workspace operations on Workspace B sentinel ID: GET -> 404 NotFound, PUT -> 404 NotFound, DELETE -> 403 Forbidden (denied per production contract for Manager). Persisted database inspection verifies Workspace A and Workspace B counts and sentinel contents remain strictly unaltered.
6. `Member_Role_Caller_Is_Denied_Manager_Operation_By_Actual_Aggregate_Authorization`: Authenticated caller holding `Member` role in Workspace B attempts POST `/api/app/project`. Admitted into Workspace B context by middleware, but denied by actual ABP aggregate authorization (`AbpAuthorizationService` + `PermissionChecker` + `WorkspaceRolePermissionValueProvider`) -> 403 Forbidden (`Volo.Abp.Authorization:010002`). Verified SQLite DB state remains completely unchanged (0 projects inserted).
7. `Dual_Member_Sees_Only_Selected_Workspace_Data_And_Persisted_Roles_Govern_Operations`: Caller holding distinct roles (`Manager` in A, `Member` in B) creates Project-A1 under Workspace A (permitted as Manager -> 200 OK), queries `/api/app/project` under A -> sees only Project-A1 (never B). Under Workspace B header, queries `/api/app/project` -> sees only Project-B1 (never A). Attempting to create project under Workspace B is rejected with 403 Forbidden by role governance.
8. `Inactive_Or_Removed_Workspace_Member_Is_Denied_With_403_NotWorkspaceMember`: Authenticated caller whose membership in target workspace is marked inactive/removed (`IsActive == false`) is rejected by `WorkspaceResolutionMiddleware` -> 403 Forbidden (`Tracker:NotWorkspaceMember`).
9. `Legitimate_Permitted_Path_Creates_Project_In_Workspace_And_Persists_To_Database`: Authenticated caller with Manager role in target workspace creates project -> 200 OK. Directly verified in SQLite database: project record persisted with matching `WorkspaceId == wsA`, Title, and Color. Cross-workspace query isolation verified: project is invisible under foreign workspace context.

---

## 8. Bounded Engineering Phase: Concurrent Last-Owner Preservation Guard (`REQ-WS-06`)

### A. Context, Source Investigation & Race Reproduction
1. **Source Evidence & Vulnerability Analysis**:
   - Source inspection revealed that `WorkspaceMembershipManager.ChangeRoleAsync` and `RemoveAsync` independently awaited `CountOwnersAsync` then mutated/updated membership state without an ownership-invariant critical section.
   - `WorkspaceAppService.LeaveAsync` loaded membership and invoked `RemoveAsync` without acquiring a lock or serializing concurrent owner exits within an explicit transactional Unit of Work.
   - Sequential last-owner protections existed in source (`TrackerDomainErrorCodes.LastOwnerCannotLeave`) and were preserved.
   - However, under uncoordinated concurrent execution, two active owners in the same workspace calling `LeaveAsync` or demoting each other could both observe `CountOwnersAsync == 2`, pass the guard independently, and drop the persisted active owner count to 0 (orphaning the workspace).
   - **Modeled Interleaving Simulation**: Demonstrated in `Modeled_Unsynchronized_Last_Owner_Removal_Simulates_Orphaning_Risk` using separate real service scopes against a project-local SQLite database. This test is explicitly maintained as a bounded modeled interleaving simulation of the unsynchronized race condition, not an execution of the prior production codebase in the test runner.

2. **Production Architectural Correction & Entrypoint Tracing**:
   - Smallest architecture-compatible solution maintaining clean layering, ambient UOW handling, and committed-state visibility:
     - **`TrackerDomainService.cs`**:
       * Exposed `protected IUnitOfWorkManager UnitOfWorkManager => LazyServiceProvider.LazyGetRequiredService<IUnitOfWorkManager>();` for domain service UOW context inspection.
     - **`WorkspaceMembershipManager.cs`**:
       * Injected `IAbpDistributedLock` to provide fine-grained, per-workspace synchronization (`Tracker:WorkspaceOwners:{workspaceId:N}`).
       * Implemented `LockOwnersAsync(Guid workspaceId)`: checks ambient `currentUow.Items[itemKey]` for an existing lock handle to provide transparent re-entrancy without deadlocks.
       * Synchronized owner demotions in `ChangeRoleAsync` (`isOwnerDemotion = member.Role == WorkspaceRoles.Owner && role != WorkspaceRoles.Owner`) under `LockOwnersAsync`.
       * Synchronized owner removals in `RemoveAsync` (`isOwnerRemoval = member.Role == WorkspaceRoles.Owner`) under `LockOwnersAsync`.
     - **`WorkspaceAppService.cs` (`LeaveAsync`)**:
       * Wraps the full leave workflow inside a transactional Unit of Work (`UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true)`).
       * Acquires `await using var ownerLock = await _memberships.LockOwnersAsync(id)`.
       * Commits transaction (`await uow.CompleteAsync()`) before disposing `ownerLock`, ensuring committed-state visibility before any waiting competitor enters the critical section.
     - **`TeamAppService.cs` (`UpdateWorkspaceRoleAsync`)**:
       * Wraps role modification inside a transactional Unit of Work (`UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true)`).
       * Acquires `await using var ownerLock = await _workspaceMemberships.LockOwnersAsync(WorkspaceId)`.
       * Verifies caller owner permissions and completes transactional UOW before releasing `ownerLock`.
     - **Entrypoint Verification & Entity Scoping**:
       * `RemoveFromWorkspaceAsync` currently denies owner targets (`if (member.IsOwner ... throw RoleNotAllowed)`), so it is not an owner removal vector. Only `LeaveAsync` allows an owner to withdraw.
       * `ProjectMembershipManager` manages project-level enrollments and hourly rates, whereas `WorkspaceMembershipManager` manages workspace-level tenancy. The two managers are kept strictly distinct.

3. **Deterministic Isolated Regressions & Provider Inability (`ConcurrentOwnerProtectionTests.cs`)**:
   - Configured disposable project-local SQLite fixture in WAL mode (`SqliteWalConnectionInterceptor` with `PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 30000;`).
   - Injected `ControllableDistributedLock` (`IAbpDistributedLock`) with bounded event hooks (`LockAcquireAttempted`, `LockWaitEntered`, `LockAcquired`, `OnBeforeReleaseAsync`, `LockReleasing`), state inspection (`ActiveHolders`, `QueuedWaiters`, `MaxConcurrentHolders`, `TotalAcquisitions`), and guaranteed cleanup on `DisposeAsync` via `try...finally`.
   - **Provider Inability Honestly Documented**: SQLite single-writer locking constraints prevent concurrent write transactions across multiple connections or background initialization tasks in ABP (causes `SQLite Error 5: 'database is locked'` during module data seeding). Therefore, `AddAlwaysDisableUnitOfWorkTransaction()` is required by the SQLite test runner. While production code explicitly begins transactional UOWs and commits prior to lock disposal, assertions under this fixture reflect SQLite nontransactional persisted-state ordering rather than transactional commit visibility. Full provider-backed multi-connection transactional commit proof requires a database provider supporting concurrent write transactions (e.g. PostgreSQL / SQL Server).
   - **Deterministic Lock Exclusivity & Provider Queued Observation**:
     * `Owner_Lock_Is_Exclusive_Held_Across_Commit_And_Unrelated_Workspaces_Not_Serialized` proves `LockOwnersAsync` is strictly exclusive for the targeted workspace, blocks competitors for that workspace, does not serialize unrelated workspace keys, and allows competitors after release.
     * `Deterministic_Interleaved_Owner_Leave_Blocks_Competitor_At_Lock_Boundary_Until_Persisted_Release`: Proves that competitor (Thread 2) is genuinely queued at the lock provider boundary (`LockWaitEntered`, `QueuedWaiters >= 1`) BEFORE Thread 1 releases; asserts pre-release persisted-state ordering (`CountActiveOwnersInDbAsync == 1`); enforces bounded 10s waits/cancellation; and ensures guaranteed semaphore release in `Handle.DisposeAsync`.
     * `Deliberately_Nonexclusive_Or_Bypassed_Lock_Variant_Is_Rejected_By_Concurrency_Assertions`: Proves that our harness `MaxConcurrentHolders == 1` assertion strictly rejects a deliberately non-exclusive or bypassed lock variant.
   - **Distinct Withdrawal Targets & Cross-Workspace Retention**:
     * `Two_Owners_Concurrently_Leaving_Preserves_At_Least_One_Active_Owner_And_Retains_Same_User_Unrelated_Workspace_Data`: Two owners in WS1 leave concurrently at a barrier; winner leaves, loser gets `LastOwnerCannotLeave`, WS1 active owners = 1; User 1's seeded membership and project membership in unrelated WS2 remain strictly intact and active (proves no unintended cross-workspace cleanup for the leaving user).
     * `Concurrent_Mixed_Owner_Leave_And_Demotion_With_Distinct_Targets_Preserves_At_Least_One_Active_Owner`: Exercises two distinct withdrawal targets (Owner B leaves, Owner B demotes Owner A); under protection, exactly one succeeds, loser gets domain error (`RoleNotAllowed` or `LastOwnerCannotLeave`), and active owners NEVER drops below 1.

### B. Execution Evidence & Provenance Boundaries

| Dimension | Exact Target / Command | Tests Passed | Failed | Duration | Exit Code | Provenance |
|---|---|:---:|:---:|:---:|:---:|---|
| **Concurrent Last-Owner Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~ConcurrentOwnerProtectionTests"` | 9 | 0 | 9.23s | **0** | **[Worker-Reported (Concurrent SQLite WAL)]** |
| **Hosted ASP.NET Core MVC TestServer** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~EfCoreWorkspaceHttpAuthorizationTests"` | 9 | 0 | 15.6s | **0** | **[Worker-Reported (Hosted MVC TestServer & Shared SQLite)]** |
| **Recipient Decline Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~InvitationDeclineTests"` | 10 | 0 | 9s | **0** | **[Worker-Reported (SQLite In-Memory)]** |
| **Host Route Reflection** | `dotnet test test/Dosi.Tracker.HttpApi.Host.Tests/Dosi.Tracker.HttpApi.Host.Tests.csproj --filter "FullyQualifiedName~ContractRoutesTests"` | 40 | 0 | 101ms | **0** | **[Worker-Reported (Unit Reflection Only)]** |
| **Frontend Contract Route Vitest** | `npx vitest run src/lib/api-contract.test.ts` | 30 | 0 | 1.22s | **0** | **[Verified by PM @ 3bebd70]** |
| **Historical PM Blocked Scripts** | `qa_workspace_phase.ps1` | null | null | null | null | **[Explicitly Not Run / No Repackaging]** |
| **Real Browser Session Acceptance** | Multi-tenant browser session & cookie verification | null | null | null | null | **[Pending Acceptance Requirement]** |

#### Test Cases Covered in `ConcurrentOwnerProtectionTests` (9 total):
1. `Modeled_Unsynchronized_Last_Owner_Removal_Simulates_Orphaning_Risk`: Bounded simulation demonstrating how two uncoordinated readers both observing count == 2 bypass sequential checks and drop active owners to 0 (orphaning risk simulation, not production execution claim).
2. `Owner_Lock_Is_Exclusive_Held_Across_Commit_And_Unrelated_Workspaces_Not_Serialized`: Proves lock exclusivity, competitor blocking on same key, immediate acquisition on unrelated workspace key, and post-release competitor acquisition.
3. `Deterministic_Interleaved_Owner_Leave_Blocks_Competitor_At_Lock_Boundary_Until_Persisted_Release`: Proves with deterministic provider-boundary queue observation (`LockWaitEntered`, `QueuedWaiters >= 1`) that competitor is held blocked at lock boundary before release; asserts SQLite nontransactional persisted-state ordering at pre-release boundary; upon release, competitor unblocks, observes persisted removal, and throws `LastOwnerCannotLeave`.
4. `Deliberately_Nonexclusive_Or_Bypassed_Lock_Variant_Is_Rejected_By_Concurrency_Assertions`: Proves fixture-level mutual exclusion assertions (`MaxConcurrentHolders == 1`) detect and reject non-exclusive or bypassed locks.
5. `Two_Owners_Concurrently_Leaving_Preserves_At_Least_One_Active_Owner_And_Retains_Same_User_Unrelated_Workspace_Data`: Barrier-synchronized concurrent leaves in WS1; exactly 1 succeeds, loser gets `LastOwnerCannotLeave`, WS1 active owners = 1; User 1's seeded membership and project membership in unrelated WS2 remain strictly intact and active.
6. `Two_Owners_Concurrently_Demoting_Each_Other_Preserves_At_Least_One_Active_Owner`: Mutual concurrent demotion; winner succeeds, loser gets domain error (`RoleNotAllowed` or `LastOwnerCannotLeave`), persisted active owners = 1.
7. `Concurrent_Mixed_Owner_Leave_And_Demotion_With_Distinct_Targets_Preserves_At_Least_One_Active_Owner`: Two distinct withdrawal targets (Owner B leaves, Owner B demotes Owner A); exactly one succeeds, loser gets domain error, and active owners NEVER drops below 1.
8. `Concurrent_Owner_Leaves_In_Different_Workspaces_Execute_Concurrently_Without_Contention`: Proves fine-grained per-workspace locking: concurrent leaves in Workspace 1 and Workspace 2 execute simultaneously without contention or cross-workspace blocking.
9. `Multi_Owner_Sequential_Leaves_Allow_Demotions_Until_Last_Owner`: Proves legitimate multi-owner sequential leaves/demotions succeed until the last owner is rejected with `LastOwnerCannotLeave`.

---

## 9. Bounded Engineering Phase: Reversible Owner-Controlled Workspace Archival & Restoration (`REQ-WS-08`)

### A. Context, Architecture & Invariant Enforcement
1. **Source Evidence & Lifecycle Gap**:
   - Source inspection of `Workspace.cs` confirmed it was an `AuditedAggregateRoot<Guid>` with only `Name` and `NormalizedName`. No archive state, timestamps, or archival operators existed.
   - `IWorkspaceAppService` and `WorkspacesController` exposed `CreateAsync`, `GetMineAsync`, and `LeaveAsync`, but no `ArchiveAsync` or `RestoreAsync` routes existed.
   - In accordance with `REQ-WS-08`, reversible archival and restoration are implemented now, while destructive permanent deletion, retention grace-period policies, and subscription billing cancellations are explicitly deferred.

2. **Domain Model & State Preservation**:
   - **`Workspace.cs`**:
     * Added properties `public bool IsArchived { get; private set; }`, `public DateTime? ArchivedTime { get; private set; }`, and `public Guid? ArchivedBy { get; private set; }`.
     * Added idempotent domain methods `Archive(Guid operatorUserId, DateTime archivedTime)` and `Restore()`.
     * Invariant: Archival does NOT delete or alter child entities, memberships, projects, invitations, or subscriptions. All data remains completely intact.
   - **`WorkspaceManager.cs`**:
     * Added `ArchiveAsync(Guid workspaceId, Guid operatorUserId)` and `RestoreAsync(Guid workspaceId, Guid operatorUserId)`.
     * Strictly verifies that the operator is an active `Owner` in the target workspace via `_memberships.FindActiveAsync(workspaceId, operatorUserId)`. Outsiders, Managers, Members, and inactive/removed former owners are denied with `AbpAuthorizationException` (403 Forbidden).
     * Added `IsArchivedAsync(Guid workspaceId)` domain query.

3. **Gating Active Workspace Resolution & Ingestion**:
   - **`WorkspaceResolutionMiddleware` (`backend/src/Dosi.Tracker.HttpApi.Host/Workspaces/WorkspaceResolution.cs`)**:
     * If an incoming request targets an archived workspace (`await workspaceRoleLookup.IsArchivedAsync(workspaceId.Value)`), the middleware inspects the endpoint metadata. If the endpoint is not decorated with `[WorkspaceNotRequired]`, it immediately writes a `403 Forbidden` response with `{ "error": { "code": "Tracker:WorkspaceArchived", "message": "This workspace is archived. Contact a workspace owner to restore it." } }` and short-circuits the pipeline before downstream controllers or actions execute.
   - **`WorkspaceRequiredFilter`**:
     * Gated so that any endpoint requiring an active workspace throws `BusinessException(TrackerDomainErrorCodes.WorkspaceArchived)` if the resolved workspace is archived.
   - **`WorkspaceRolePermissionValueProvider.cs`**:
     * `WorkspaceRoleLookup` checks `_workspaceRepository.FindAsync(workspaceId)` to truthfully answer `IsArchivedAsync(workspaceId)`.

4. **Service Contracts & Endpoint Routing**:
   - **`IWorkspaceAppService.cs` & `WorkspaceAppService.cs`**:
     * Declared and implemented `ArchiveAsync(Guid id)` and `RestoreAsync(Guid id)`, decorated with `[WorkspaceNotRequired]` and `[RemoteService(IsEnabled = false)]`.
     * `GetMineAsync()` projects `IsArchived` and `ArchivedTime` into `MyWorkspaceDto`.
     * `AcceptAsync` and `RegisterWithInvitationAsync`: Gated to reject invitation acceptance or new anonymous registration into an archived workspace before any account creation, membership join, or seat count mutation occurs, throwing `BusinessException(TrackerDomainErrorCodes.WorkspaceArchived)`.
   - **`WorkspacesController.cs`**:
     * Added protected routes `POST /api/app/workspaces/{id}/archive` and `POST /api/app/workspaces/{id}/restore`, calling `_workspaceAppService.ArchiveAsync(id)` and `RestoreAsync(id)`.

5. **Test Harness & Isolated Regression Suite (`WorkspaceArchivalLifecycleTests.cs`)**:
   - Extracted `WorkspaceHttpTestBase<TStartupModule>` from `WorkspaceHttpAuthorizationTests` to allow clean reuse of the hosted ASP.NET Core `TestServer` MVC pipeline without inheriting duplicate `REQ-ISO-05` test cases.
   - Project-local disposable SQLite fixture with WAL mode, genuine production routing, real `WorkspacesController`, real `WorkspaceManager`, real `WorkspaceResolutionMiddleware`, and production `WorkspaceAppService`.
   - Seeded two separate workspaces (`ARCHIVE-WS-A`, `ARCHIVE-WS-B`) with distinct user roles (`OwnerA`, `ManagerA`, `MemberA`, `InactiveOwnerA`, `UserB`, `DualUser`, `InviteeUser`).

### B. Execution Evidence & Provenance Boundaries

| Dimension | Exact Target / Command | Tests Passed | Failed | Duration | Exit Code | Provenance |
|---|---|:---:|:---:|:---:|:---:|---|
| **Workspace Archival Lifecycle Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceArchivalLifecycleTests"` | 9 | 0 | 13.0s | **0** | **[Worker-Reported (Hosted MVC TestServer & SQLite WAL)]** |
| **Workspace Archival Migration & Upgrade Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceArchivalMigrationTests"` | 10 | 0 | 7.0s | **0** | **[Worker-Reported (Direct Snapshot/TargetModel, Npgsql DDL, Operation-Level & Tracked Migrator Upgrade)]** |
| **Workspace Capture Retention Regressions (REQ-DAT-01)** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceCaptureRetentionTests"` | 16 | 0 | 9.0s | **0** | **[Worker-Reported (Isolated SQLite & Scripted In-Memory Blob Provider)]** |
| **Concurrent Last-Owner Regressions** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~ConcurrentOwnerProtectionTests"` | 9 | 0 | 9.23s | **0** | **[Worker-Reported (Concurrent SQLite WAL)]** |
| **Hosted ASP.NET Core MVC TestServer** | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~EfCoreWorkspaceHttpAuthorizationTests"` | 9 | 0 | 15.6s | **0** | **[Worker-Reported (Hosted MVC TestServer & Shared SQLite)]** |
| **Host Route & Resolution Regressions** | `dotnet test test/Dosi.Tracker.HttpApi.Host.Tests/Dosi.Tracker.HttpApi.Host.Tests.csproj` | 125 | 0 | 1.15s | **0** | **[Worker-Reported (Host Test Suite)]** |
| **Historical PM Blocked Scripts** | `qa_workspace_phase.ps1` | null | null | null | null | **[Explicitly Not Run / No Repackaging]** |

#### Test Cases Covered in `WorkspaceArchivalLifecycleTests` (9 total):
1. `Owner_Can_Archive_Workspace_Successfully_And_Listing_Visibly_Reflects_Archived_State`: Proves an active Owner can archive target workspace via `POST /api/app/workspaces/{id}/archive`, DB persists `IsArchived = true`, `ArchivedBy = OwnerA`, and `ArchivedTime`, and listing via `GET /api/app/workspaces/mine` visibly reports `isArchived: true`.
2. `Outsider_Manager_Member_And_Inactive_Former_Owner_Are_Denied_Archive_With_Unchanged_State`: Proves Outsider (UserB), Manager (ManagerA), Member (MemberA), and Inactive former owner (InactiveOwnerA) are all denied with 403 Forbidden; DB state remains `IsArchived = false`.
3. `Archived_Workspace_Normal_Requests_Are_Denied_With_403_WorkspaceArchived`: Proves both Owner and Manager sending normal requests (GET / POST to `/api/app/project` with `X-Workspace-Id: wsA`) are intercepted by `WorkspaceResolutionMiddleware` and denied with 403 Forbidden (`Tracker:WorkspaceArchived`); DB writes rejected (0 projects inserted).
4. `Archiving_Workspace_A_Preserves_Legitimate_Access_To_Unrelated_Workspace_B`: Proves that archiving Workspace A leaves unrelated Workspace B completely unaffected; dual-member can read and create projects in Workspace B with 200 OK.
5. `Owner_Can_Restore_Archived_Workspace_Reenabling_Normal_Access`: Proves an active Owner can restore an archived workspace via `POST /api/app/workspaces/{id}/restore`, restoring `IsArchived = false`, and normal active workspace requests immediately succeed again (200 OK).
6. `Outsider_Manager_Member_And_Inactive_Former_Owner_Are_Denied_Restore`: Proves Outsider, Manager, Member, and Inactive former owner are denied restore with 403 Forbidden; workspace remains archived in DB.
7. `Archive_And_Restore_Are_Idempotent_Preserving_Audit_State`: Proves repeat archive calls return 200 OK without mutating original `ArchivedTime` or `ArchivedBy`, and repeat restore calls return 200 OK idempotently.
8. `Archival_Preserves_All_Data_Memberships_Subscriptions_And_Sentinels_Without_Deletion_Or_Cancellation`: Directly verifies that members, projects, and active billing subscriptions are NOT deleted, modified, or cancelled during archival; upon restore, all sentinel data is accessible via HTTP.
9. `Invitation_Acceptance_And_Registration_Into_Archived_Workspace_Are_Rejected_Before_Mutation`: Proves both existing user invitation acceptance (`POST /api/app/invitations/{token}/accept`) and new anonymous user registration (`_workspaceAppService.RegisterAsync`) are rejected with `Tracker:WorkspaceArchived` before any user account, membership, or database mutation occurs.

### C. Reversible Archival Schema Upgrade Path (`20261020200944_AddWorkspaceArchivalFields`)
1. **Source Evidence & Production Gap**:
   - `Workspace.cs` mapped `IsArchived`, `ArchivedTime`, and `ArchivedBy`, and `TrackerDbContext.cs` configured index `IX_AppWorkspaces_IsArchived`.
   - However, tracked EF Core migrations contained only `20261007185459_Initial.cs`, and `TrackerDbContextModelSnapshot.cs` lacked these three columns and index on `AppWorkspaces`.
   - Because `EntityFrameworkCoreTrackerDbSchemaMigrator` uses `Database.MigrateAsync`, existing migrated databases would not receive the new columns without an additive migration.
2. **Additive Migration & Model Snapshot Implementation**:
   - Generated additive migration `20261020200944_AddWorkspaceArchivalFields.cs` with matching designer `20261020200944_AddWorkspaceArchivalFields.Designer.cs`.
   - `Up`: Adds nullable `ArchivedBy` (uuid), nullable `ArchivedTime` (timestamp without time zone), non-null `IsArchived` (boolean, default `false`), and index `IX_AppWorkspaces_IsArchived`. Existing rows automatically receive active status (`IsArchived = false`).
   - `Down`: Drops index and drops the three columns reversibly.
   - Updated `TrackerDbContextModelSnapshot.cs` strictly for the `Workspace` entity block; verified zero unintended diff across other entities.
   - Verified zero pending model drift via `dotnet ef migrations has-pending-model-changes` (exit code 0).
3. **Rollback Data Consequences (Honest Disclosure)**:
   - Rolling back via `Down()` executes `ALTER TABLE "AppWorkspaces" DROP COLUMN` for `IsArchived`, `ArchivedTime`, `ArchivedBy` and drops index `IX_AppWorkspaces_IsArchived`.
   - Dropping these archive fields causes the loss of archival state and audit history, returning any previously archived workspaces to unarchived status; this is NOT a lossless rollback of archival status. Underlying workspaces, memberships, projects, and subscriptions remain intact and are not deleted.
   - Migration files are prepared for code review only; never apply migration or rollback directly to user/production database.
4. **Isolated Migration Regression Suite (`WorkspaceArchivalMigrationTests.cs`, 10 tests passed)**:
   - `Migration_Metadata_And_Attributes_Are_Correctly_Configured`: Verifies `MigrationAttribute` ID and `DbContextAttribute(typeof(TrackerDbContext))`.
   - `Up_Operations_Add_Expected_Columns_And_Index_With_Safe_Defaults`: Verifies 4 operations (3 AddColumn, 1 CreateIndex), types, names, default `false`, and nullable audit columns.
   - `Down_Operations_Reversibly_Drop_Archival_Columns_And_Index`: Verifies 4 operations (1 DropIndex, 3 DropColumn).
   - `Current_Runtime_Model_Contains_Workspace_Archival_Properties_And_Index`: Verifies runtime `TrackerDbContext.Model` entity configuration and index.
   - `Direct_Tracked_ModelSnapshot_Contains_Workspace_Archival_Properties_And_Index`: Directly asserts on `TrackerDbContextModelSnapshot.Model` verifying tracked artifact snapshot properties, nullability, false default, table mapping, and index. Protects against stale snapshot.
   - `Direct_Tracked_Migration_TargetModel_Contains_Workspace_Archival_Properties_And_Index`: Directly asserts on `AddWorkspaceArchivalFields.TargetModel` verifying migration artifact target model properties, nullability, false default, and index.
   - `Generated_PostgreSql_Up_Ddl_Contains_Non_Destructive_Alter_Table_And_Index`: Verifies PostgreSQL DDL generated by `NpgsqlMigrationsSqlGenerator` contains non-destructive `ALTER TABLE "AppWorkspaces" ADD "IsArchived" boolean NOT NULL DEFAULT FALSE;`, `ArchivedTime timestamp without time zone`, `ArchivedBy uuid`, and `CREATE INDEX "IX_AppWorkspaces_IsArchived"`.
   - `Generated_PostgreSql_Down_Ddl_Reversibly_Drops_Columns_And_Index`: Verifies PostgreSQL Down DDL drops index and drops columns.
   - `Disposable_OperationLevel_Baseline_Upgrade_Preserves_Workspace_Memberships_Projects_And_Subscriptions`: Tests reflected `Up` operations against hand-built baseline SQLite schema (`AppWorkspaces`, `AppWorkspaceMembers`, `AppProjects`, `AppSubscriptions` matching `Initial`), seeds sentinels, applies DDL, verifies all sentinels survive intact, verifies active default, verifies EF Core archival mutation, and asserts SQLite's provider boundary on DropColumn (`NotSupportedException` without rebuild).
   - `Tracked_Baseline_To_Current_Migrator_Upgrade_Preserves_Existing_Data_With_Active_Defaults`: Executes real tracked EF Core `IMigrator` service on disposable SQLite connection: migrates to `20261007185459_Initial`, verifies baseline schema without archival fields, seeds sentinels, migrates to `20261020200944_AddWorkspaceArchivalFields`, verifies `__EFMigrationsHistory`, verifies all 4 entity sentinels survive intact, and executes EF Core archival mutation and persistence.

### D. Workspace-Isolated Capture Retention Lifecycle (`REQ-DAT-01`, Safe First Slice)

1. **Source Evidence & Production Architecture**:
   - Implemented `ICaptureRetentionService` and `CaptureRetentionService` in `Dosi.Tracker.Activities.Retention`.
   - Smallest maintainable production domain service enforcing explicit workspace-scoped age-based cleanup for `SCREEN`, `THUMB`, and `WEBCAM` captures (`Screenshot` entity in `AppScreenshots` + `IBlobContainer` objects).
   - Activity time summaries (`Activity` entity in `AppActivities`), productivity metrics, mouse/keyboard counts, active window lists, timeline buckets, memberships, projects, invitations, and billing subscriptions remain 100% untouched.
   - Provided `CaptureRetentionPolicy`: enforces non-empty `WorkspaceId`, validated positive retention (`RetentionPeriod > TimeSpan.Zero`), bounded batch size (1..1000), optional capture kinds filter, explicit `AllowArchivedWorkspace` toggle (default `false`), and optional deterministic `Cutoff` override.
   - Enforces defense-in-depth: explicit `WorkspaceId` predicates on both candidate selection (`Where(s => s.WorkspaceId == policy.WorkspaceId && s.CapturedAt < cutoff)`) and database mutation (`Where(s => toDeleteIds.Contains(s.Id) && s.WorkspaceId == policy.WorkspaceId)`).
   - Scoped data filtering (`using (_dataFilter.Disable<IWorkspaceScoped>())`) within explicit UOW boundaries, preventing cross-tenant filter leakage across runs.
   - **Activity Repository Ownership & Workspace Isolation Barrier**: Before any blob mutation, `CaptureRetentionService` queries `_activityRepository` to verify that referenced `Activity` records exist and belong to `policy.WorkspaceId`. If a screenshot row in Workspace A references an Activity belonging to Workspace B or an unverified activity, the service fails closed: logs a warning, increments `CorruptKeyCount`, and bypasses both blob deletion and database metadata deletion (zero provider deletes, target workspace data and foreign blobs untouched).
   - **Strict Canonical Blob Key Convention (`IsCanonicalStorageUrl`)**: Enforces exact writer-compatible format matching `ActivityAppService` ingestion:
     * `activityId` must be a valid non-empty Guid.
     * `kind` must be a known capture kind (`screen`, `thumb`, `webcam`).
     * `storageUrl` must strictly equal `activities/{activityId:D}/{kind}{extension}` via case-sensitive ordinal equality, where `extension` is strictly `.jpg` or `.png`.
     * Rejects path traversal (`..`), nested directory separators (`/`, `\`), sibling suffixes (`screen-other.jpg`, `screen_0.jpg`), unknown kinds (`audio.jpg`), case folding (`Activities/`, `SCREEN.jpg`, `screen.JPG`), and unsupported formats (`screen.bmp`, `screen.webp`), incrementing `CorruptKeyCount` and preventing blob or row mutations.

2. **Non-Atomic Distributed Boundary & Failure Ordering**:
   - Order of operations: Blob deletion is executed first per candidate, followed by database row deletion in a transactional Unit of Work.
   - Blob deletion failures (e.g., S3 503 / timeout) preserve the database pointer: the row is omitted from `toDeleteFromDb`, incrementing `FailedBlobCount` and allowing safe retry.
   - Missing blobs (already deleted from storage or 404 from provider) increment `MissingBlobCount` and allow safe deletion of dangling database records.
   - DB transaction failure after blob deletion converges honestly on retry: the next execution finds the missing blob (`existed == false`), treats it as `MissingBlobCount`, and commits the DB deletion without orphan rows or blocking exceptions.
   - *Modeled Missing-Blob Recovery Clarification:* Manual removal of a fake blob in test `Database_Failure_After_Blob_Deletion_Converges_Honestly_On_Retry` is modeled missing-blob recovery, NOT an injected real DB commit failure.
   - Ordering explicitly does NOT claim atomic transactions across remote blob storage and SQL database.

3. **Disabled-by-Default Configuration & Worker Adapter**:
   - `CaptureRetentionWorker`: Periodic background worker adapter inheriting `AsyncPeriodicBackgroundWorkerBase`.
   - `Timer.RunOnStart = false`: Worker never runs on application or test startup.
   - `CaptureRetentionOptions`: Defaults `IsEnabled = false`.
   - `CaptureRetentionWorker.SweepAsync`: Checks `if (!_options.Value.IsEnabled) return new CaptureRetentionSweepResult { Skipped = true, SkipReason = "Disabled by configuration" };`.
   - `TrackerHttpApiHostModule.cs`: Only adds `CaptureRetentionWorker` to background workers if `retentionOptions.IsEnabled == true`.
   - No public unauthenticated HTTP endpoint is exposed.
   - Automatic execution against the host machine's configured database or real blob storage is strictly prevented.

4. **Operational Defect: Retention Liveness Starvation Behind Preserved Bad Rows**:
   - *Source-Confirmed Operational Defect:* `CaptureRetentionService` previously ordered expired rows by `CapturedAt, Id` and took `Take(BatchSize + 1)`, processing only the first `BatchSize`.
   - Bad/corrupt rows (e.g. malformed storage URLs or referencing foreign activities) and transient blob deletion failures (e.g. S3 503) fail closed and deliberately remain persisted in the database.
   - Without continuation cursor state, `CaptureRetentionWorker.SweepAsync` called the service once per active workspace per sweep, ignoring `HasMore`.
   - Consequently, when a full batch of preserved poison or failing rows existed at the head of the expired stream, every sweep repeatedly examined only that leading batch and made zero progress, permanently starving later legitimate expired captures from being reclaimed.

5. **Bounded Progress Strategy: Deterministic Keyset Cursor & Bounded Sweeps**:
   - *Domain Keyset Cursor (`CaptureRetentionCursor`):* Represents deterministic pagination position `(DateTime CapturedAt, Guid ScreenshotId)`.
   - *Policy & Result Contract:* `CaptureRetentionPolicy` carries `CaptureRetentionCursor? Cursor`. `CaptureRetentionResult` returns `CaptureRetentionCursor? NextCursor` (null when candidate stream ends, signaling wrap-around) and `LastProcessedCursor`.
   - *Keyset Predicate in Service:* `s.CapturedAt > cursorCapturedAt || (s.CapturedAt == cursorCapturedAt && s.Id.CompareTo(cursorId) > 0)`. EF Core SQLite and PostgreSQL translate `s.Id.CompareTo(cursorId) > 0` cleanly into SQL without client evaluation.
   - *Bounded Worker Orchestration (`CaptureRetentionWorker`):*
     * Freezes `cutoff = clock.Now - retentionPeriod` per workspace per sweep, preventing clock drift across multi-page sweeps.
     * Maintains in-memory workspace-scoped cursors (`ConcurrentDictionary<Guid, CaptureRetentionCursor> _workspaceCursors`).
     * Sweeps up to `MaxPagesPerWorkspace` (configurable in `CaptureRetentionOptions`, default 5) per workspace per sweep.
     * Updates cursor on `hasMore`; when `hasMore == false`, resets cursor to null (deterministic wrap-around).
     * Wrap-around ensures transient blob failures that have been repaired in storage become eligible for reclamation on subsequent sweeps.
     * Preserved bad/corrupt rows are NOT deleted or bypassed in ownership validation (fail-closed preservation strictly maintained).
     * Strictly avoids unbounded materialization, recursive `HasMore` loops, mutable global tenant DB state, or destructive quarantine.

6. **Concrete Isolated Regression Suite (`WorkspaceCaptureRetentionTests.cs`, 20 tests passed, exit 0)**:
   - `Expired_Captures_Are_Deleted_From_Database_And_Blob_Container`: A-expired captures (both screen and thumb) deleted from SQLite and fake blob provider; asserts both DB row deletion and blob key removal.
   - `Unrelated_Workspace_Expired_Captures_Are_Preserved_Untouched`: B-expired sentinel in unrelated workspace is untouched in SQLite and fake blob provider when running retention on Workspace A.
   - `Fresh_And_Exact_Cutoff_Captures_Are_Preserved_Untouched`: A-fresh (newer than cutoff) and exact-cutoff (capturedAt == cutoff) are preserved in DB and blob provider; only strictly expired (< cutoff) deleted.
   - `Activity_Time_Summaries_And_Blocks_Remain_Completely_Untouched`: Parent `Activity` record with time summaries, input counts (mouse/keyboard), and JSON payloads remains 100% intact after child screenshot cleanup.
   - `Archived_Workspace_Policy_Explicit_Default_Skips_Unless_Explicitly_Allowed`: Verifies archived workspace is skipped (`Skipped = true`, `SkipReason = "Workspace is archived"`) when `AllowArchivedWorkspace = false`, and cleans expired captures when `AllowArchivedWorkspace = true`.
   - `Zero_Empty_Or_Negative_Policy_Throws_Before_Any_Side_Effects`: Verifies `Guid.Empty`, `TimeSpan.Zero`, negative duration, zero/negative batch size, > 1000 batch size, and null policy throw before any database or storage operations.
   - `Bounded_Batching_And_HasMore_Convergence_On_Repeat_Execution`: Verifies bounded batch pagination (batchSize = 2 over 5 candidates across distinct synthetic activities with canonical keys) and `HasMore` convergence across successive idempotent passes.
   - `Missing_Blobs_In_Storage_Safely_Clean_Up_Dangling_Database_Rows`: Dangling DB rows pointing to missing blobs in storage are safely cleaned up (`MissingBlobCount = 1`, `DeletedCount = 1`).
   - `Blob_Deletion_Failure_Preserves_Database_Pointer_For_Retry`: Injected S3 503 failure during blob deletion preserves DB pointer (`FailedBlobCount = 1`, `DeletedCount = 0`); retry after recovery succeeds (`DeletedCount = 1`).
   - `Database_Failure_After_Blob_Deletion_Converges_Honestly_On_Retry`: Simulates recovery from crash between blob deletion and DB commit; retry converges honestly (`MissingBlobCount = 1`, `DeletedCount = 1`). Modeled missing-blob recovery, not injected DB commit failure.
   - `Corrupt_Or_Non_Canonical_Storage_Urls_Are_Rejected_Without_Deleting_Arbitrary_Blobs`: Non-canonical, traversal, and external URLs are rejected without deleting caller-supplied blobs (`CorruptKeyCount = 3`, `DeletedCount = 0`, delete count delta = 0).
   - `Worker_Default_Disabled_Performs_Zero_Work_And_Produces_No_Side_Effects`: Default worker with `IsEnabled = false` skips immediately (`Skipped = true`, `WorkspacesSwept = 0`, `TotalDeleted = 0`), producing 0 side effects.
   - `Worker_Enabled_Sweeps_Active_Workspaces_And_Skips_Archived`: Worker with `IsEnabled = true` sweeps active workspaces and skips archived workspaces.
   - `Cross_Workspace_Inconsistent_Activity_Reference_Fails_Closed_Without_Deleting_Blob_Or_Metadata`: Proves candidate screenshot with `WorkspaceId = wsA` referencing genuine `ActivityId = actB` (in `wsB`) and `actB`'s canonical blob key fails closed: zero provider deletes, Workspace B blob/DB row/Activity untouched, and corrupt metadata row preserved without deletion.
   - `Exact_Key_Negative_Regressions_Reject_Sibling_Nested_UnknownKind_CaseFolded_And_UnsupportedFormat`: Unit-level and DB/storage execution verifying sibling suffixes (`screen-other.jpg`, `screen_0.jpg`), nested directory segments (`screen/nested.jpg`), unknown kinds (`audio.jpg`), case folding (`Activities/`, `SCREEN.jpg`, `screen.JPG`), and unsupported formats (`screen.bmp`, `screen.webp`) are rejected with 0 provider deletes, preserving rows and blobs.
   - `Legitimate_Capture_Kinds_And_Format_Variants_Clean_Up_Successfully`: Verifies all 6 legitimate writer-compatible combinations (`screen`/`thumb`/`webcam` × `.jpg`/`.png`) across distinct synthetic activities clean up cleanly with exact provider delete count delta = 6.
   - `Valid_Later_Expired_Captures_Starve_Behind_Full_Leading_Batch_Of_Corrupt_Rows_Without_Progress`: Isolated regression demonstrating starvation defect on production path (fails before fix with exit code 1 in task-3936; passes after fix with exit 0). Verifies later valid expired captures are reclaimed past leading preserved foreign rows, and corrupt rows remain preserved.
   - `Valid_Later_Expired_Captures_Starve_Behind_Full_Leading_Batch_Of_Failing_Blob_Rows_Without_Progress`: Isolated regression demonstrating starvation defect behind leading batch of retriable blob delete failures (fails before fix with exit code 1; passes after fix with exit 0).
   - `Progress_Across_Worker_Sweeps_When_Preserved_Rows_Exceed_One_Page_And_Single_Sweep_Limit`: Proves multi-sweep worker progress when preserved poison rows (4 rows) exceed a single sweep's bounded limit (`MaxPagesPerWorkspace = 2`, `BatchSize = 2`). Demonstrates cursor advancement at sweep 1 boundary, subsequent sweep 2 resumption past cursor, deletion of valid rows, and wrap-around cursor reset.
   - `Repaired_Blob_Failures_Become_Eligible_Again_On_WrapAround`: Proves deterministic wrap-around semantics: when a previously failing blob is repaired in storage, worker wrap-around re-evaluates and reclaims the row.

7. **Operational Limitations & Remaining Gaps**:
   - **A disabled implementation is not deployed/enforced retention**: The worker is disabled by default and has not been executed on live environments.
   - **Activity Retention Gap**: 1-year activity summary pruning (e.g. deleting old `Activity` records) is deferred; only captures are pruned in this slice.
   - **Owner Settings UI Gap**: Per-workspace retention period configuration UI for workspace owners in Next.js frontend is not yet implemented.
   - **Live Storage / DB Approval Gap**: Production execution against live PostgreSQL databases and real AWS S3 / Cloudflare R2 object storage remains pending independent PM operational approval.

---

## 10. REQ-DAT-02: Authenticated Workspace-Scoped Activity CSV Export (Bounded Full-Product Slice)

1. **Architecture & Conventions**:
   - Implemented via conventional hosted HTTP endpoint: `GET /api/app/activity/export` (`ActivityExportController.cs`), backed by `IActivityAppService.ExportCsvAsync` in `ActivityAppService.cs` and RFC 4180 serialization in `CsvExportHelper.cs`.
   - Contracts: `ExportActivitiesInput.cs` (query parameters: `From`, `To`, `ProjectId`, `UserId`, `MaxResultCount`) and `ActivityExportResultDto.cs` (binary payload, UTF-8 content type `text/csv; charset=utf-8`, attachment filename, `TotalCount`, `ExportedCount`).
   - Domain Shared: `ExportExceedsLimit = "Tracker:ExportExceedsLimit"` in `TrackerDomainErrorCodes.cs`, localized in `en.json`, and mapped to HTTP 400 `BadRequest` in `TrackerHttpApiModule.cs`.

2. **Server-Enforced Authorization & Scoping**:
   - Enforced by `ICurrentWorkspace.GetId()`: fail-closed if missing (400 `Tracker:WorkspaceRequired`).
   - Enforced by `ICurrentUser.GetId()`.
   - Role-based visibility via production `WorkspaceRolePermissionValueProvider`:
     * Regular members can export only their own activities.
     * Owners and Managers holding `Tracker.Activities.ViewAll` can view all workspace activities or filter by `UserId`.
     * If a regular member supplies `UserId != currentUserId`, request is rejected with 403 Forbidden.
     * If a caller supplies `ProjectId` belonging to another workspace, query filter restricts project lookup and returns 404 NotFound.
     * Spoofed foreign workspace header (`X-Workspace-Id`) rejected with 403 `Tracker:NotWorkspaceMember`.
     * Same-user A+B switching respects workspace-specific roles and visibility (verified via `DualUser` with Owner role in Workspace A and Member role in Workspace B).

3. **Column Contract & Truthful Metrics**:
   - Minimal stable column schema: `ActivityId,UserId,ProjectId,ProjectTitle,StartedAtUtc,EndedAtUtc,DurationMinutes,DurationSeconds,Description,Productivity,IdleSeconds,MouseClicks,KeyboardHits`.
   - Omits sensitive captures/blob URLs, raw window titles, hourly pay rates, and extraneous PII fields.
   - Unavailable values remain blank (`""`), never inventing measured zeros (`0`) for missing metrics.

4. **CSV Escaping & Formula Injection Neutralization**:
   - RFC 4180 compliant: fields containing commas, double quotes, or newlines are wrapped in double quotes; double quotes are escaped by doubling (`""`).
   - Spreadsheet formula injection defense: fields starting with `=, +, -, @, \t, \r, |` (including leading whitespace/control variants) are prefixed with a single quote (`'`).

5. **Server-Enforced Date Bounds & Workload Limits**:
   - Date validation: `To >= From` and date span `<= 366` days (`Tracker:ReportRangeInvalid`).
   - Server-enforced output ceiling: `MaxAllowedResultCount = 10000`.
   - Candidate overflow protection: if the candidate count exceeds the requested or system maximum, the endpoint explicitly rejects the request with HTTP 400 `Tracker:ExportExceedsLimit` rather than returning a deceptively complete-looking partial/truncated export.
   - **Cardinality Hardening at Materialization Boundary**:
     * Prior flaw: Preliminary `CountAsync(query)` verified `totalCount <= limit`, but the query was then materialized without a server-side `Take` bound and without post-materialization cardinality validation. Concurrent ingestion between count and fetch could yield rows exceeding `limit` and stale response metadata (`X-Total-Count < X-Exported-Count`).
     * Hardened correction: Applied deterministic server-side `Take(limit + 1)` bound on production query (`query.OrderByDescending(...).ThenByDescending(...).Take(limit + 1)`).
     * Post-materialization validation: If `activities.Count > limit`, immediately rejects with `BusinessException(TrackerDomainErrorCodes.ExportExceedsLimit)` (HTTP 400 `BadRequest`). No partial CSV or header stream is emitted.
     * Snapshot-derived counts: `TotalCount = activities.Count` and `ExportedCount = activities.Count` derived honestly from the bounded materialized snapshot, guaranteeing coherence across `X-Total-Count`, `X-Exported-Count`, and actual CSV data lines.

6. **Isolated Regression Evidence**:
   - Hosted ASP.NET Core `TestServer` regression suite with isolated SQLite database: `backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/EntityFrameworkCore/Applications/Activities/WorkspaceActivityExportHttpTests.cs`.
   - 10 test scenarios (10 passed, exit 0, repeated twice):
     1. `Authorized_Workspace_Owner_Exports_Csv_With_Headers_And_Data`: Verifies 200 OK, `text/csv; charset=utf-8`, `attachment; filename=...`, `X-Total-Count`, `X-Exported-Count`, stable column contract, and exact data fields.
     2. `A_Only_Caller_Denied_B_With_Seeded_Foreign_Sentinel`: Verifies 403 Forbidden on spoofed header; Workspace B sentinel remains untouched.
     3. `Same_User_A_And_B_Switching_Respects_Workspace_Specific_Roles_And_Data`: Verifies same user with Owner role in A sees all activities and with Member role in B sees only own activity.
     4. `Regular_Member_Attempting_Other_User_Or_Foreign_Project_Filter_Is_Rejected`: Verifies 403 Forbidden when member filters by other user, and 404 NotFound when member filters by foreign workspace project.
     5. `Missing_Or_Empty_Workspace_Header_Is_Rejected_With_400_WorkspaceRequired`: Verifies 400 BadRequest with code `Tracker:WorkspaceRequired` on missing or `Guid.Empty` header.
     6. `Malformed_Or_Out_Of_Bounds_Dates_And_Oversize_Workload_Are_Rejected`: Verifies 400 BadRequest for reversed dates, excessive span (> 366 days), oversize MaxResultCount (> 10000), and candidate overflow past limit (`Tracker:ExportExceedsLimit`).
     7. `Csv_Escapes_Special_Characters_And_Neutralizes_Formula_Injection_While_Keeping_Unavailable_Values_Blank`: Verifies quotes doubling, newline handling, formula prefixing (`=`, `+`, `@`, `\t`), and blank unavailable metrics.
     8. `Old_Implementation_Without_Bounded_Take_Violates_Requested_Limit_And_Metadata_When_Row_Added_After_Count`: Proves the former defect under retained old-code query (preliminary count passes at limit 2; concurrent insert adds row 3; unconstrained materialization yields 3 rows; CSV emits 3 data rows violating requested limit; stale `TotalCount` 2 < `ExportedCount` 3). Workspace B sentinel preserved.
     9. `Production_Export_Rejects_Limit_Plus_One_Candidate_Overflow_At_Materialization_Boundary_Without_Partial_Csv`: Injected `BarrierInterceptor` executes mid-operation row insertion immediately after preliminary `CountAsync`. Proves both direct production `ActivityAppService.ExportCsvAsync` and hosted `TestServer` HTTP endpoint reject limit + 1 candidate overflow with HTTP 400 `Tracker:ExportExceedsLimit` without partial CSV. Workspace B sentinel preserved.
     10. `Production_Export_Exact_Limit_Succeeds_With_Coherent_Snapshot_Derived_Counts_And_Exact_Csv_Rows`: Verifies requested exact limit (2 activities seeded, limit = 2) returns HTTP 200 OK, matching `X-Total-Count: 2`, `X-Exported-Count: 2`, and exact CSV row count (1 header + 2 data rows).

7. **Operational Limitations & Remaining Gaps**:
   - **Async Bulk/Capture Export Gap**: Asynchronous background jobs, multi-file capture packaging, and zip archiving are deferred; this slice delivers synchronous workspace-scoped activity/time CSV export.
   - **Signed Public URLs Gap**: Temporary signed URLs (e.g. S3 presigned URLs) are not generated; CSV is streamed directly over authenticated session.
   - **Disaster Recovery Gap**: Automated database dump/restore and disaster recovery runbooks remain deferred.
   - **Frontend UI Gap**: Next.js export buttons and modal dialogs are deferred.

---

## 11. Bounded Phase: Live Team Presence Active Workspace Member Lifecycle Enforcement (REQ-TRK-04 / REQ-ISO)

### A. Defect Trace & Boundary Analysis
1. **Source Evidence & Lifecycle Gap**:
   - `PresenceAppService.GetTeamAsync` retrieved all scoped `AgentPresence` rows for the current workspace and filtered them strictly against `IdentityUser.IsActive` (global account activity), despite the code comment stating `"Only devices of current members: a deleted or deactivated account is no longer part of the team"`.
   - `WorkspaceMembershipManager.RemoveAsync` sets `member.Status = WorkspaceMemberStatus.Removed` (`IsActive = false`) and persists the inactive membership without deleting presence records.
   - Self-service member exit (`WorkspaceAppService.LeaveAsync`) delegates to `_memberships.RemoveAsync` and similarly persists `WorkspaceMemberStatus.Removed`.
   - When a user was removed from or left Workspace A while remaining active globally (e.g. holding active membership in Workspace B), their recent `AgentPresence` in Workspace A continued to be returned in Workspace A's live team presence view.
   - Presence freshness (`AgentPresence.LastSeenAt`), global account status (`IdentityUser.IsActive`), and workspace membership status (`WorkspaceMember.Status == WorkspaceMemberStatus.Active`) are distinct predicates that were conflated.

2. **Reproduction in Isolated Hosted TestServer Regression**:
   - Prior to fix, regression test `Departed_Member_Removed_From_Workspace_A_Is_Excluded_From_A_Team_Presence_While_Retaining_B_Presence` was executed against the uncorrected production code.
   - Test demonstrated the actual failure:
     ```
     Shouldly.ShouldAssertException : userIdsInA should not contain 5a701e62-0953-4dd3-910b-dc6cc93ccb03
     but was actually [5a701e62-0953-4dd3-910b-dc6cc93ccb02, 5a701e62-0953-4dd3-910b-dc6cc93ccb03]
     ```
   - Proved departed user presence remained visible to Workspace A callers after removal via the production domain path.

3. **Minimal Server-Side Query Boundary Correction**:
   - File: `backend/src/Dosi.Tracker.Application/Presence/PresenceAppService.cs`.
   - Injected `IRepository<WorkspaceMember, Guid> _memberRepository`.
   - Enforced active workspace membership at the database query boundary in `GetTeamAsync`:
     ```csharp
     var currentWorkspaceId = CurrentWorkspace.GetId();
     var memberQuery = await _memberRepository.GetQueryableAsync();
     var activeMemberUserIds = memberQuery
         .Where(m => m.WorkspaceId == currentWorkspaceId && m.Status == WorkspaceMemberStatus.Active)
         .Select(m => m.UserId);

     var query = (await _presenceRepository.GetQueryableAsync())
         .Where(p => activeMemberUserIds.Contains(p.UserId));
     ```
   - Preserves historical activities, captures, and `AgentPresence` records (non-destructive query-level scoping; no destructive history cleanup).
   - Preserves member-only-own-device visibility (`!Activities.ViewAll`) and manager/owner `ViewAll` permissions.
   - Preserves legitimate multi-workspace (A+B) roles, switching, and foreign workspace protection.

4. **Concrete Regression Suite**:
   - File: `backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/EntityFrameworkCore/Applications/Presence/WorkspaceTeamPresenceHttpTests.cs`.
   - 6 concrete test scenarios (6 passed, exit 0):
     1. `Departed_Member_Removed_From_Workspace_A_Is_Excluded_From_A_Team_Presence_While_Retaining_B_Presence`: Seeds active global user in A and B; removes from A via `_membershipManager.RemoveAsync`; verifies user excluded from A team presence, retained in B, and historical presence in A preserved.
     2. `Departed_Member_Via_Self_Service_Leave_Is_Excluded_From_Team_Presence`: User leaves A via `_workspaceAppService.LeaveAsync`; verifies user excluded from A team presence, retained in B, and presence record in A preserved.
     3. `Inactive_Membership_And_Non_Members_Are_Excluded_While_Current_Active_Members_Are_Retained`: Seeds inactive member (`Status = Removed`) and non-member (no record in A); verifies only current active member is retained; historical presence preserved.
     4. `Regular_Member_Without_ViewAll_Only_Sees_Own_Device_In_Live_Presence`: Regular member sees only own device; owner sees all active member devices.
     5. `Foreign_Workspace_Caller_Attempting_Presence_Access_Is_Denied_With_403_NotWorkspaceMember`: Caller belonging to A only attempts to query B presence; rejected with HTTP 403 `NotWorkspaceMember`; B presence untouched.
     6. `Same_User_With_Different_Roles_In_Workspaces_A_And_B_Switches_Legitimately`: DualUser is Owner in A and Member in B; verifies switching context in same hosted client correctly displays all devices in A and only own device in B.

5. **Regression Suites Verification**:
   - `WorkspaceTeamPresenceHttpTests`: 6 passed, exit 0
   - `EfCorePresenceAppServiceTests` & `EfCorePresenceMemberTests`: 24 passed, exit 0
   - `WorkspaceActivityExportHttpTests`: 10 passed, exit 0
   - `WorkspaceCaptureRetentionTests`: 20 passed, exit 0
   - `Dosi.Tracker.Application.Tests`: 166 passed, exit 0
   - `Dosi.Tracker.Domain.Tests`: 60 passed, exit 0

---

## 13. Phase 3 Step 6: Workspace-Scoped Self-Service Device Revocation (REQ-DEV-01)

### A. Context & Architectural Discovery
1. **Background**: Prior to this slice, `Activity.DeviceId` was normalized client metadata, and `PresenceAppService.HeartbeatAsync` auto-created and pruned per-(workspace, user, device) rows without a persistent revocation model. `ActivityAppService.CreateAsync` accepted caller-supplied `DeviceId` without checking durable revocation.
2. **Missing Product Capability**: Workspace members needed self-service capability to revoke their own observed logical devices in their authorized workspace, denying subsequent heartbeats and activity uploads prior to persistence or blob storage side-effects, excluding them from live team presence projection, while maintaining workspace isolation and idempotency.

### B. Implementation Details
1. **Domain & Data Model**:
   - `DeviceRevocation` entity (`backend/src/Dosi.Tracker.Domain/Devices/DeviceRevocation.cs`): implements `IWorkspaceScoped`, `CreationAuditedEntity<Guid>`. Table `AppDeviceRevocations` with composite unique index `IX_AppDeviceRevocations_WorkspaceId_UserId_DeviceId`.
   - `DeviceRevocationManager` domain service: validates device observation in presence or activity records for `(WorkspaceId, UserId)`, enforces idempotent repeated revocation, and handles revocation creation.
   - Domain errors in `TrackerDomainErrorCodes.cs`: `DeviceRevoked` (403), `DeviceNotObserved` (400), `DeviceNotFound` (404).
   - Domain constants in `DeviceConsts.cs`: `MaxReasonLength = 256`, `MaxRevocationsListing = 100`.
2. **Application & HTTP API**:
   - `DeviceAppService` & `DeviceController`: authenticated routes `POST /api/app/device/revoke`, `GET /api/app/device/my-revocations`, `GET /api/app/device/{deviceId}/status`.
   - `PresenceAppService`:
     - Injected `IRepository<DeviceRevocation, Guid> _revocationRepository`.
     - `HeartbeatAsync` rejects heartbeats for revoked device with 403 `Tracker:DeviceRevoked`.
     - `GetTeamAsync` filters out live presence for devices matching active revocations in the current workspace, without deleting historical presence records from database.
   - `ActivityAppService`:
     - Injected `IRepository<DeviceRevocation, Guid> _revocationRepository`.
     - `CreateAsync`: preserves honest idempotent replay of pre-revocation stored blocks via `FindOwnBlockAsync`; for new blocks with revoked `DeviceId`, throws 403 `Tracker:DeviceRevoked` prior to validation, project checks, or screenshot/blob storage side-effects.
3. **Database Migration**:
   - Additive migration `20261021060000_AddDeviceRevocations.cs` + Designer + `TrackerDbContextModelSnapshot.cs`.
   - Default-compatible with existing rows; tested with tracked `IMigrator` baseline-to-current upgrade regression on disposable in-memory SQLite fixture.

### C. Concrete Test Execution & Verified Evidence
1. **Device Revocation Migration Tests** (`backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/EntityFrameworkCore/Migrations/DeviceRevocationMigrationTests.cs`):
   - Command: `dotnet test backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~DeviceRevocationMigrationTests"`
   - Result: **7 passed, 0 failed, exit 0**.
   - Verified: migration metadata, Up/Down DDL operations, runtime model properties, direct snapshot synchronization, PostgreSQL DDL generation, and disposable SQLite baseline-to-current upgrade preserving existing data.
2. **Hosted HTTP Integration Suite** (`backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/EntityFrameworkCore/Applications/Devices/WorkspaceDeviceRevocationHttpTests.cs`):
   - Command: `dotnet test backend/test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceDeviceRevocationHttpTests"`
   - Result: **9 passed, 0 failed, exit 0**.
   - Verified scenarios:
     1. `Own_Device_Revocation_And_Repeated_Revoke_Are_Idempotent_And_Persisted`: Own device revoked successfully (200 OK); repeated revocation returns existing record idempotently without duplicate insertion.
     2. `Post_Revoke_Heartbeat_And_Activity_Ingestion_Are_Denied_With_Zero_Blob_Effects`: Subsequent heartbeat returns 403 `Tracker:DeviceRevoked`; subsequent activity upload returns 403 `Tracker:DeviceRevoked` with zero screenshots and zero activity DB/blob side-effects.
     3. `Idempotent_Replay_Of_Pre_Revocation_Block_Succeeds_While_New_Block_Is_Denied`: Replay of pre-revocation block succeeds (200 OK); new block from revoked device is denied (403).
     4. `Live_Presence_Excludes_Revoked_Device_While_Retaining_Historical_Records`: Live presence query excludes revoked device; historical DB presence rows preserved.
     5. `Different_Device_In_A_Works_And_Same_Device_In_B_Is_Unaffected`: Another device for caller in A works normally; same device in Workspace B works normally (strictly workspace-scoped).
     6. `Denies_Revoking_Another_Members_Device_Or_In_Foreign_Workspace_Without_Tombstones`: Attempt to revoke another member's device in A rejected with 400 `Tracker:DeviceNotObserved`; foreign workspace caller rejected with 403 `Tracker:NotWorkspaceMember`; no tombstones created.
     7. `Presence_Pruning_Does_Not_Erase_Revocation_And_Unobserved_Target_Creates_No_Record`: Stale presence pruning cannot remove revocation; unobserved/empty target rejected (400) without creating records.
     8. `Status_Inspection_And_Bounded_Listing_Contracts`: Status inspection returns accurate boolean state, timestamp, and reason; bounded listing returns current user's revocations.
     9. `Legacy_Missing_DeviceId_Activity_Behavior_Is_Preserved`: Legacy activity upload without `DeviceId` succeeds normally.
3. **Regression Suites Verification**:
   - `WorkspaceTeamPresenceHttpTests`: 6 passed, exit 0
   - `WorkspaceActivityExportHttpTests`: 10 passed, exit 0
   - `WorkspaceCaptureRetentionTests`: 20 passed, exit 0
   - `ContractRoutesTests`: 44 passed, exit 0
   - `Dosi.Tracker.Application.Tests`: 166 passed, exit 0
   - `Dosi.Tracker.Domain.Tests`: 60 passed, exit 0

### D. Explicit Deferred Limitations
1. **Device ID Binding**: `DeviceId` is a logical, client-reported UUID. It is NOT a hardware-bound cryptographic credential or TPM/Secure Enclave attestation.
2. **Session & Token Scoping**: Revoking a logical device ID does NOT revoke OAuth refresh tokens, invalidate JWT bearer tokens, or execute global account logout. New-ID rotation, token revocation, enrollment credentials, and native desktop agent handling remain explicitly deferred.
3. **Storage & Audit Preservation**: Historical activity blocks, screenshots, memberships, and presence rows are retained and never deleted upon device revocation.
4. **Environment**: All executions performed on project-local disposable in-memory SQLite fixtures; production PostgreSQL cluster execution, native OS runtimes, and PM independent acceptance remain pending.

---

## 14. Current Status & Next Steps

1. **Privacy Acceptance Pending**:
   - The privacy source audit (commit `14bbe66`) remains **Pending PM Independent Review**. This status is preserved and not bypassed.
2. **Workspace Lifecycle, Authorization, Retention, Export, Presence & Device Status**:
   - Workspace-scoped self-service device revocation (`REQ-DEV-01`), live team presence active membership scoping (`REQ-TRK-04`), authenticated workspace-scoped activity CSV export (`REQ-DAT-02`), workspace-isolated capture retention lifecycle safe first slice (`REQ-DAT-01`), reversible owner-controlled workspace archival (`REQ-WS-08`), concurrent last-owner protection (`REQ-WS-06`), recipient decline lifecycle (`REQ-WS-07`), and hosted ASP.NET Core `TestServer` MVC aggregate authorization harness (`REQ-ISO-05`) are implemented and verified via isolated regressions (99 SQLite integration tests total: 9 device revocation + 7 device migration + 6 team presence + 10 activity export + 20 capture retention + 9 archival lifecycle + 10 archival migration/upgrade + 9 concurrent owner protection + 9 hosted MVC auth + 10 decline).
   - Real browser cookie/session testing (`__Host-Dosi-Workspace`), Next.js server actions, OpenIddict token issuer, provider-backed multi-connection transactional commit proof (PostgreSQL/SQL Server), multi-node distributed lock infrastructure (Redis), and PM independent execution remain pending. All reported outcomes are worker-executed, not independent PM QA.
3. **Execution Limits & Evidence Boundaries**:
   - TestServer regressions prove conventional MVC routing, aggregate permission checks, fail-closed workspace scoping, CSV formula neutralization, Take(limit + 1) hard bounded fetch, active member presence query scoping, device revocation gating before blob side-effects, and server limits on in-memory SQLite, but do NOT replace execution on a real PostgreSQL cluster; no live user/production database was modified.
   - Retention defaults remain disabled (`IsEnabled = false`); no cleanup was executed against user DB or real object storage.
4. **Remaining Backlog Gaps**:
   - Cryptographic client machine enrollment / attestation, token/session revocation, native desktop agent revocation prompt/unenrollment UI, WebSocket / SignalR push live presence notification, asynchronous export background worker, screenshot zip archive, signed download URLs, disaster recovery / automated backup procedures, frontend UI export actions, 1-year activity summary pruning, and permanent deletion remain deferred acceptance gaps.
5. **Overall Product Incomplete**:
   - Full product remains incomplete across backend metric nullability schema migrations (`Activity.cs` truthful nulls vs 0), desktop offline ingestion queue hardening, platform-native interactive client runtimes, code signing, and hosted operational deployment.
6. **Awaiting PM Guidance**:
   - Bounded workspace-scoped self-service device revocation slice complete. Stop and hand off for PM review with exact diff and test evidence.

