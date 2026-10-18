# Dosi-Tracker — PM Status Report & Evidence Matrix

> **Document Class:** Production Readiness Audit & Status Tracking (Phase 3 Step 1: P0 Sensor Lifecycle & Product Acceptance Matrix)  
> **Status Date:** October 19, 2026  
> **Auditor / Implementation Engineer:** Antigravity Pair Engineer  
> **Supervisor:** PM / QA Operator  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)  

---

## 1. Executive Status & Current Bounded Phase Summary

Following independent baseline observation and supervisory direction by the PM/QA operator at commit `4f4946c`, this bounded phase executed three critical deliverables:
1. **Reconciliation of Production Readiness Documentation**:
   - Reconciled [docs/PRODUCTION_READINESS.md](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/docs/PRODUCTION_READINESS.md), replacing an outdated July 2026 "scaffold-only" label with verified October 2026 reality (domain aggregates, EF Core PostgreSQL persistence, OpenIddict auth, multi-tenant resolution, Next.js 16 workspace switching, and desktop agent logic present, while keeping open gaps explicit).
2. **Product Acceptance Matrix & Execution Plan**:
   - Authored [docs/PRODUCT_ACCEPTANCE.md](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/docs/PRODUCT_ACCEPTANCE.md) establishing a finite, traceable acceptance matrix mapping requirements (`REQ-WS-01` to `REQ-OPS-03`) to exact code paths, test commands, verified evidence status (`[Verified]`, `[Worker-Reported]`, `[Source-Only]`, `[Blocked]`, `[Not Implemented]`), and concrete gaps.
   - Updated [docs/PM_EXECUTION_PLAN.md](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/docs/PM_EXECUTION_PLAN.md) ordering remaining implementation slices by risk (P0 Sensor Lifecycle $\rightarrow$ Schema Nullability $\rightarrow$ Multi-Workspace Lifecycle & Isolation $\rightarrow$ Linux Spike $\rightarrow$ Device Enrollment $\rightarrow$ Data Retention $\rightarrow$ Packaging).
3. **P0 Privacy Sensor Lifecycle Gating Implemented & Verified (`clients/windows`)**:
   - Resolved the critical privacy blocker identified in the audit where Windows OS input hooks and 3s window sampling loops accumulated telemetry before login and continued accumulating during pause/stopped states.
   - **Default-Safe Invariant**: `UsageRecorder` and `InputCounter` now initialize inactive (`active = false`). Pre-login and unconsented states drop all events.
   - **Active Window Polling Suppression**: `UsageRecorder::spawn_sampler` checks `is_active()` before querying the Win32 foreground window (`active_window::current()`), eliminating unconsented OS polling when paused.
   - **Lifecycle Wiring**: Connected `Worker::publish()` state changes through `Tracker::set_tracking_active(bool)` so sensors dynamically activate **only** when `status == TrackerStatus::Tracking`, and immediately deactivate on `Paused`, `Away`, `Locked`, `SignedOut`, or `ChooseProject`.
   - **Regression Suite**: Added 6 new unit and lifecycle tests; all 351 unit tests in `cargo test --bin dosi-tracker` passed cleanly.

---

## 2. Changed Files in Current Phase

| File Path | Description of Changes |
|---|---|
| `docs/PRODUCTION_READINESS.md` | Reconciled outdated scaffold verdict with active architecture and documented open gaps. |
| `docs/PRODUCT_ACCEPTANCE.md` | Created finite traceable acceptance matrix across workspaces, privacy, isolation, and ops. |
| `docs/PM_EXECUTION_PLAN.md` | Reordered execution phases by risk, prioritizing P0 sensor lifecycle and schema nullability. |
| `clients/windows/src/tracking/usage.rs` | Added `active: AtomicBool` gate; gated `record()` and `spawn_sampler` loop; added 2 unit regression tests. |
| `clients/windows/src/tracking/input.rs` | Added `active: AtomicBool` gate; gated key/mouse event accumulators; added 2 unit regression tests. |
| `clients/windows/src/tracking/mod.rs` | Added `set_tracking_active` and `is_tracking_active` on `Tracker`; added unit regression test. |
| `clients/windows/src/worker.rs` | Added `set_tracking_active` to `Machine` trait and implemented on `Tracker`; wired into `Worker::publish`. |
| `clients/windows/src/worker/tests.rs` | Added `tracking_active` tracking to `FakeMachine`; added pause/resume lifecycle regression test. |
| `docs/PM_STATUS.md` | Updated status report, test commands, exit codes, and next phase recommendations. |

---

## 3. Actual Concrete Test Execution & Verification

### A. Phase Verification Gates (Executed Locally)

| Target Suite / Command | Exact Command Line | Tests Passed | Tests Failed | Skipped / Ignored | Execution Time | Exit Code | Notes |
|---|---|:---:|:---:|:---:|:---:|:---:|---|
| **Windows Client Cargo Tests** | `cargo test --bin dosi-tracker` in `clients/windows` | **351** | 0 | 5 | 5.86s | **0** | Verified locally on Windows host. 6 new regression tests included. 5 ignored tests require live backend and interactive desktop session. |
| **Frontend TypeScript Gate** | `npm run typecheck` in `frontend` | N/A | 0 errors | 0 | 4.8s | **0** | TypeScript 5 strict compilation with no emit. |
| **Frontend Workspace Integration Tests** | `npx vitest run src/components/layout/workspace-switcher.live.test.tsx src/test/integration/tenant-switching.test.tsx src/test/pages/workspaces.test.tsx` | **17** | 0 | 0 | 3.42s | **0** | 3 test files: workspace switcher, tenant switching, and workspace settings/management. |
| **Frontend Accessibility Gate** | `npm run test:a11y` in `frontend` | **53** | 0 | 1,912 skipped | 18.5s | **0** | Matches `--testNamePattern a11y`. Programmatic contrast and axe tests pass in jsdom. |

### B. Suites Not Run in This Step & Justification
- **Full Backend Suite (`dotnet test backend/Dosi.Tracker.slnx`, 823 tests)**: Not re-run in this step because no backend C# code or database migrations were modified in Step 1. Scheduled to run in Step 2 during the `Activity.cs` / `CreateActivityDto.cs` nullability migration.
- **Full Frontend Vitest Suite (`npm test`, 1,965 tests)**: Focused on workspace switcher, tenant switching, workspaces page, and a11y suites (17 + 53 tests) since frontend changes in Step 1 were zero. Full suite will run when workspace management UI / nullability rendering is updated.
- **macOS Client Suite**: Host is Windows 11; Xcode / macOS toolchain not available locally.
- **Linux Client Suite**: `clients/linux` crate scheduled for creation in Phase 4.

---

## 4. Evidence-Based Feature Checklist

| Category | Feature / Requirement | State | Verification Evidence / Location |
|---|---|:---:|---|
| **Auth & Identity** | OpenIddict password grant & token endpoint (`/connect/token`) | **Implemented (Test-Host Verified)** | `OpenIddictDataSeedContributor.cs`, `AuthRateLimitingTests.cs` (10 req/min rate limit active) |
| | Multi-tenant workspace resolution via query/header | **Implemented (Test-Host Verified)** | `WorkspaceResolutionTests.cs`, `CurrentWorkspace.cs`, `frontend/src/hooks/useApi.ts` |
| | Client token cache & refresh handling | **Implemented (Unit Mocked)** | `clients/windows/src/worker/control.rs`, `clients/macos/Sources/DosiTrackerCore/ApiClient.swift` |
| | Role-based permissions (Owner, Admin, Worker) | **Implemented (Test-Host Verified)** | `TrackerPermissions.cs`, `WorkspacePermissionTests.cs`, `MemberScopingTests.cs` |
| **Tenant Isolation** | ABP global query filters on `IWorkspaceScoped` | **Implemented (EF Verified)** | `WorkspacePersistenceTests.cs`, `ActivityPersistenceTests.cs` |
| | Foreign project membership / tenant injection guard | **Implemented (EF Verified)** | `ActivityAppService.cs` line 118, `ProjectsAndTeamSteps.cs` |
| | Hourly pay-rate masking for non-managers | **Implemented (EF Verified)** | `ProjectMembershipManagerTests.cs`, `TeamAppServiceTests.cs` |
| **Multi-Workspace UX**| Workspace switcher component & live switching | **Verified (Vitest Mocked)** | `workspace-switcher.live.test.tsx`, `tenant-switching.test.tsx` (17 tests passing) |
| | Workspace invitation lifecycle & revocation | **Source Present** | `WorkspaceAppService.cs`, `WorkspaceInvitation.cs`; needs integration test expansion |
| | Prevention of last-owner orphaning | **Source Present** | `WorkspaceManager.cs`; requires negative integration tests |
| **Privacy & Consent** | Windows sensor lifecycle active-state gating | **Implemented (Unit Verified)** | `clients/windows/src/tracking/` (`usage.rs`, `input.rs`, `worker.rs`); 6 new regression tests passing |
| | Metric nullability (unmeasured vs zero) | **In Progress** | Audited in `CONSENT_CAPABILITY_AUDIT.md`; schema migration scheduled for Step 2 |
| | macOS input & window monitor lifecycle gating | **Open Gap** | Requires Swift port of atomic active-state gating in `DosiTrackerCore` |
| | Linux X11/Wayland truthful capability reporting | **Open Gap** | Crate not yet created; requires Wayland portal session with denial handling |
| **Device Lifecycle** | Random `DeviceId` generation per agent install | **Implemented (Source Verified)** | `AgentPresence.cs`, `clients/windows/src/state.rs` |
| | Device presence heartbeat (`/api/app/presence/heartbeat`) | **Implemented (EF Verified)** | `PresenceAppServiceTests.cs` (`Heartbeat_Should_Accept_And_Store_Linux_Platform`), 150 s online window |
| | Automatic pruning of stale/surplus devices (>10 or >30d) | **Implemented (EF Verified)** | `PresenceAppService.cs` `RemoveSurplusDevicesAsync` |
| | Cryptographic keypair device enrollment | **Unresolved Gap** | Device authentication relies on user session JWT; public-key enrollment needed |
| | Server-side device revocation endpoint | **Unresolved Gap** | Dedicated token/device revocation API needed |
| **Ingestion & Sync** | Time block upload (`POST /api/app/activity`) | **Implemented (EF Verified)** | `ActivityAppServiceTests.cs`, `ActivityPersistenceTests.cs` |
| | Idempotency Key & Database Index | **Implemented (EF Core Source)** | Unique index `IX_AppActivities_WorkspaceId_UserId_ClientActivityId` on `(WorkspaceId, UserId, ClientActivityId)` |
| | Atomic block & capture validation | **Implemented (EF Verified)** | `ActivityAppService.cs` `PendingCapture`; invalid captures skipped while preserving time block |
| | Screenshot blob storage (S3/R2/Database fallback) | **Implemented (Unit Mocked)** | `ActivityCaptureStorageTests.cs`, `Screenshot.cs`, `BlobStoring` configuration |
| | Offline SQLite queue on client | **Implemented (Unit Mocked)** | `clients/windows/src/storage.rs`, `clients/macos/Sources/DosiTrackerCore/QueueDatabase.swift` |
| | Poison row parking (`synced = 2`) | **Implemented (Unit Mocked)** | `clients/windows/src/worker/uploader.rs` unit tests |
| **Time Reporting** | Tracked time & productivity duration-weighted math | **Implemented (Unit Verified)** | `ReportingAppService.cs`, `analytics.ts`, `work-report.ts` |
| | Calendar & timezone normalization (IANA conversion) | **Implemented (Unit Verified)** | `AgentContextNormalizer.cs`, `TimeZoneIds.cs`, `time-zone.ts` |
| | Member work rhythm & hourly distribution | **Implemented (Unit Verified)** | `ReportingAppServiceTests.cs`, `analytics-charts.tsx` |
| **Observability & Ops**| Serilog structured logging & health checks | **Implemented (Test-Host Verified)** | `HttpApi.Host/Program.cs`, `/health-status` endpoint |
| | Automated screenshot & activity retention cleanup | **Unresolved Gap** | Retention limits defined in host settings, but periodic pruning background worker not wired |
| **Accessibility** | Programmatic component axe & machine contrast checks | **Partially Verified (Automated in jsdom)** | Automated `jest-axe` checks in jsdom and programmatic contrast calculations pass (53 a11y tests) |
| | Machine contrast verification on card tokens | **Implemented (Unit Verified)** | `analytics-charts.test.ts` derives actual `--card` tokens from `globals.css` and asserts $\ge 3:1$ against `#ffffff` and `#1a2226` |

---

## 5. Platform Readiness Matrix

| Evidence Dimension | Windows (Rust) | macOS (Swift) | Linux (Target) |
|---|---|---|---|
| **1. Implemented Source** | ⚠️ Source present (`clients/windows`: egui UI, Win32 hooks, xcap, DPAPI, SQLite queue); sensor lifecycle active gating implemented; native hardware capture, packaging, and signing open | ⚠️ Source present (`clients/macos`: SwiftUI, ScreenCaptureKit, Keychain, SQLite queue); source presence is NOT proof of complete implementation; TCC consent, runtime execution, packaging, and notarization open | ❌ **Unresolved Gap** (`clients/linux` crate not yet created) |
| **2. Synthetic / Mock Unit Tests** | ✅ Complete (**351 cargo tests passing**, including 6 new sensor lifecycle regression tests) | ⚠️ Unverified locally (Swift test files present in source; not executed locally) | ❌ 0 tests (No crate) |
| **3. Toolchain / Compile / Build** | ✅ Verified locally (Rustc / Cargo 1.97.1 on Windows 11 host) | ❌ Unverified locally (Host lacks macOS toolchain / Xcode) | ❌ Unverified locally (Host lacks Linux build container / environment) |
| **4. Native Interactive Runtime** | ⚠️ **Outstanding** (Unit tests run synthetically; interactive Win32 desktop session unverified) | ⚠️ **Outstanding** (Requires physical macOS execution environment) | ❌ **Outstanding** (Requires Linux X11/Wayland execution environment) |
| **5. Packaging / Install / Uninstall** | ⚠️ Partial (NSIS `.nsi` script present; silent install/uninstall unverified in CI) | ⚠️ Partial (`build-app.sh` creates bundle; DMG packaging unverified) | ❌ **Unresolved Gap** (No `.deb`, AppImage, or Flatpak manifest) |
| **6. Code Signing & Notarization** | ⚠️ **Outstanding Prerequisite** (Unsigned; Authenticode cert required; signing does not guarantee immediate SmartScreen elimination; SmartScreen must not be bypassed) | ⚠️ **Outstanding Prerequisite** (Unsigned; Apple Developer ID & Notarization required) | ⚠️ **Outstanding Prerequisite** (GPG package signing required) |

---

## 6. Recommended Safe Next Bounded Phase

### Recommended Phase: Phase 3 Step 2 — Backend Schema Nullability Migration & Cross-Workspace Tenant Isolation Tests
- **Scope (Smallest High-Impact Slices)**:
  1. **Schema Nullability Migration**:
     - Change `Activity.KeyboardHits` and `Activity.MouseClicks` in `Activity.cs` from `int` to `int?`.
     - Update `CreateActivityDto.cs` and `ActivityDto.cs` to use `int?`.
     - Generate and apply EF Core migration `MakeActivityMetricsNullable` for PostgreSQL / SQLite test host.
     - Verify both legacy numeric `0` and `null` inputs persist accurately and return without coercion.
  2. **Cross-Workspace Tenant Isolation Integration Tests**:
     - Author concrete negative integration tests asserting that a user belonging to Workspace A cannot read or write projects, tasks, activities, screenshots, or members in Workspace B.
     - Verify tenant switching race conditions and header spoofing protections.
  3. **Verification Gate**:
     - Run `dotnet test backend/Dosi.Tracker.slnx`.
     - Run `npm run validate`.
- **Constraints**: Isolated test databases only; no remote pushes, no production deployments.
