# Dosi-Tracker — PM Status Report & Evidence Matrix

> **Document Class:** Production Readiness Audit & Status Tracking (Phase 1 Follow-up)  
> **Status Date:** October 18, 2026  
> **Auditor / Implementation Engineer:** Antigravity Pair Engineer  
> **Supervisor:** PM / QA Operator  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)

---

## 1. Executive Status & Audit Summary

Following a comprehensive audit of the Dosi-Tracker codebase against product documentation (`DOCUMENTATION.md`, `docs/AS_BUILT.md`, `docs/PRODUCTION_READINESS.md`, `docs/SECURITY.md`, `docs/RUNBOOK.md`, `docs/QUALITY.md`, and `ENGINEERING_BLUEPRINT.md`), the repository status has been verified using real local toolchains and truthful evidence boundaries.

### Key Audit Findings & Evidence Distinctions:
1. **Pipeline Core is Real (Synthetic / Test-Host Verified)**:
   - Backend domain entities (`Activity`, `Screenshot`, `AgentPresence`, `Project`, `ProjectMember`, `Workspace`, `Subscription`, `Invoice`), app services, OpenIddict token endpoints, and screenshot decoding into ABP blob containers are implemented.
   - Verification is conducted via concrete Entity Framework Core SQLite in-memory test host fixtures (`Dosi.Tracker.EntityFrameworkCore.Tests`) and simulated authentication principals.
   - This proves source correctness and schema consistency, but does **not** constitute a live multi-node PostgreSQL / OpenIddict / S3 production cluster deployment verification.
2. **Frontend Integration (Component & Simulated Session Verified)**:
   - Next.js 16 dashboard includes comprehensive UI flows for workspace signups, login, projects, team member rates, real-clock reporting (`/api/app/reporting/summary`), and presence monitors.
   - Tests run in Node.js / jsdom using mocked fetch handlers (`useApi`), memory datasets, and `@testing-library/react`.
   - This proves component contracts, accessibility compliance, and state transitions, but does **not** substitute for live end-to-end browser testing over WAN.
3. **Desktop Agent Reality & Strict OS Evidence Separation**:
   - **Windows Client**: Mature Rust desktop client source with 345 passing cargo unit/integration tests.
     - *Evidence Reality*: Cargo tests verify models, SQLite queue FIFO ordering, exponential backoff, poison row parking (`synced = 2`), and synthetic worker state machines.
     - *Crucial Distinction*: Cargo tests use mock inputs and synthetic structures; they do **NOT** verify native interactive hardware execution (live display capture via DXGI/GDI, live global low-level keyboard/mouse hooks, or live Win32 focus events on an active desktop session).
   - **macOS Client**: Complete Swift 5.9 source code present under `clients/macos/` using `ScreenCaptureKit`, `AVFoundation`, and `Accessibility` APIs.
     - *Evidence Reality*: Source presence is **NOT** runtime validation. CI configuration files (`.github/workflows/macos.yml`) are not proof tests ran.
     - *Crucial Distinction*: On this Windows 11 host, Swift toolchain and macOS hardware are absent; native build, test execution, and runtime validation remain unexecuted and outstanding.
   - **Linux Client**: **Explicit Architectural Gap**.
     - No Linux client crate existed (`clients/linux` was not present).
     - Both backend and frontend previously hardcoded a two-platform assumption (`"windows"` and `"macos"` only), actively discarding `"linux"` payloads.
4. **Phase 1 Baseline & Follow-up Actions Completed**:
   - Backend `ActivityConsts.Platforms` and frontend contracts (`api-contract.ts`, `types.ts`, `devices.ts`, `analytics.ts`, `report-math.ts`, `live-dataset.ts`) updated to accept and normalize `"linux"`.
   - Corrected accessibility contrast regression: replaced low-contrast Linux color `#e97b35` (2.86:1 on white) with `#cb6e17` (3.63:1 on `#ffffff`, 4.43:1 on `#1a2226`), satisfying the $\ge 3:1$ requirement on both card themes.
   - Added automated regression testing importing actual `PLATFORM_COLORS` source constants.
   - Added explicit Linux `platformMix` regression test in `analytics.test.ts`.
   - Split backend Linux tests into dedicated independent `[Fact]` methods (`Create_Should_Accept_And_Store_Linux_Platform` in `ActivityAppServiceTests.cs` and `Heartbeat_Should_Accept_And_Store_Linux_Platform` in `PresenceAppServiceTests.cs`), executed by the concrete EF Core host harness (`EfCoreActivityAppServiceTests` and `EfCorePresenceAppServiceTests`).

---

## 2. Evidence-Based Feature Checklist

| Category | Feature / Requirement | State | Verification Evidence / Location |
|---|---|:---:|---|
| **Auth & Identity** | OpenIddict password grant & token endpoint (`/connect/token`) | **Implemented (Test-Host Verified)** | `OpenIddictDataSeedContributor.cs`, `AuthRateLimitingTests.cs` (10 req/min rate limit active) |
| | Multi-tenant workspace resolution via query/header | **Implemented (Test-Host Verified)** | `WorkspaceResolutionTests.cs`, `CurrentWorkspace.cs`, `frontend/src/hooks/useApi.ts` |
| | Client token cache & refresh handling | **Implemented (Unit Mocked)** | `clients/windows/src/worker/control.rs`, `clients/macos/Sources/DosiTrackerCore/ApiClient.swift` |
| | Role-based permissions (Owner, Admin, Worker) | **Implemented (Test-Host Verified)** | `TrackerPermissions.cs`, `WorkspacePermissionTests.cs`, `MemberScopingTests.cs` |
| **Tenant Isolation** | ABP global query filters on `IWorkspaceScoped` | **Implemented (EF Verified)** | `WorkspacePersistenceTests.cs`, `ActivityPersistenceTests.cs` |
| | Foreign project membership / tenant injection guard | **Implemented (EF Verified)** | `ActivityAppService.cs` line 118, `ProjectsAndTeamSteps.cs` |
| | Hourly pay-rate masking for non-managers | **Implemented (EF Verified)** | `ProjectMembershipManagerTests.cs`, `TeamAppServiceTests.cs` |
| **Device Lifecycle** | Random `DeviceId` generation per agent install | **Implemented (Source Verified)** | `AgentPresence.cs`, `clients/windows/src/state.rs` |
| | Device presence heartbeat (`/api/app/presence/heartbeat`) | **Implemented (EF Verified)** | `PresenceAppServiceTests.cs` (`Heartbeat_Should_Accept_And_Store_Linux_Platform`), 150 s online window |
| | Automatic pruning of stale/surplus devices (>10 or >30d) | **Implemented (EF Verified)** | `PresenceAppService.cs` `RemoveSurplusDevicesAsync` |
| | Cryptographic keypair device enrollment | **Unresolved Gap** | Device authentication currently relies on user session JWT; no mutual device public-key registration |
| | Server-side device revocation endpoint | **Unresolved Gap** | Revoking a stolen device without changing the user's password is not yet an explicit API |
| **Ingestion & Sync** | Time block upload (`POST /api/app/activity`) | **Implemented (EF Verified)** | `ActivityAppServiceTests.cs`, `ActivityPersistenceTests.cs` |
| | Idempotency Key & Database Index | **Implemented (EF Core Source)** | Unique index `IX_AppActivities_WorkspaceId_UserId_ClientActivityId` on `(WorkspaceId, UserId, ClientActivityId)`; supported by non-unique index `IX_AppActivities_WorkspaceId_ClientActivityId` for fast cross-member conflict checks (`ActivityClientIdConflict`). Duplicate insert caught and returns existing row. |
| | Atomic block & capture validation | **Implemented (EF Verified)** | `ActivityAppService.cs` `PendingCapture`; invalid captures skipped while preserving time block |
| | Screenshot blob storage (S3/R2/Database fallback) | **Implemented (Unit Mocked)** | `ActivityCaptureStorageTests.cs`, `Screenshot.cs`, `BlobStoring` configuration |
| | Offline SQLite queue on client | **Implemented (Unit Mocked)** | `clients/windows/src/storage.rs`, `clients/macos/Sources/DosiTrackerCore/QueueDatabase.swift` |
| | Poison row parking (`synced = 2`) | **Implemented (Unit Mocked)** | `clients/windows/src/worker/uploader.rs` unit tests |
| **Time Reporting** | Tracked time & productivity duration-weighted math | **Implemented (Unit Verified)** | `ReportingAppService.cs`, `analytics.ts`, `work-report.ts` |
| | Calendar & timezone normalization (IANA conversion) | **Implemented (Unit Verified)** | `AgentContextNormalizer.cs`, `TimeZoneIds.cs`, `time-zone.ts` |
| | Member work rhythm & hourly distribution | **Implemented (Unit Verified)** | `ReportingAppServiceTests.cs`, `analytics-charts.tsx` |
| **Native Permissions** | Windows low-level input counting (no raw keys) | **Implemented (Synthetic Only)** | `clients/windows/src/worker.rs` (rdev event counter in source; unit tested via simulated events; native hardware hooks unverified) |
| | Windows active window & process enumeration | **Implemented (Synthetic Only)** | `EnumWindows` visible-window enumeration in source; native interactive window enumeration unverified |
| | macOS Screen Recording, Accessibility, Camera TCC | **Implemented (Source Only)** | `clients/macos/Sources/DosiTracker/Capture/ScreenCaptureService.swift`; runtime execution unverified |
| | Linux X11/Wayland input & display handling | **Unresolved Gap** | `clients/linux` crate not yet created; Wayland portal session required |
| **Packaging & Dist** | Windows NSIS installer & uninstaller | **Implemented (Source Only)** | `clients/windows/installer/dosi-tracker.nsi`, autostart registry configuration |
| | Windows Authenticode code signing | **Outstanding Prerequisite** | Unsigned in local development builds; code signing certificate required. SmartScreen must not be bypassed in production. |
| | macOS App Bundle & Hardened Runtime | **Implemented (Source Only)** | `clients/macos/scripts/build-app.sh`; Apple Developer ID signing and notarization are outstanding prerequisites. |
| | Linux packaging (.deb, AppImage, Flatpak) | **Unresolved Gap** | No Linux package configuration exists |
| **Observability & Ops**| Serilog structured logging & health checks | **Implemented (Test-Host Verified)** | `HttpApi.Host/Program.cs`, `/health-status` endpoint |
| | Prometheus metrics / OpenTelemetry tracing | **Unresolved Gap** | No Prometheus scraper or OTel exporter configured |
| | Automated screenshot & activity retention cleanup | **Unresolved Gap** | Retention limits defined in host settings, but periodic pruning background worker not wired |
| **Accessibility** | WCAG 2.1 AA jest-axe & Playwright axe scans | **Implemented (Unit Verified)** | Vitest a11y suite (49 passed axe assertions across dashboard components) |
| | Machine contrast verification on card tokens | **Implemented (Unit Verified)** | `analytics-charts.test.ts` asserts every member of `PLATFORM_COLORS` $\ge 3:1$ against `#ffffff` and `#1a2226` |

---

## 3. Security & Privacy Audit

### Consent & Surveillance Prevention Invariants
1. **Strictly Non-Stealth & Visible User Controls**:
   - The desktop agent must run with a visible system tray icon (Windows), menu-bar item (macOS), or status indicator (Linux).
   - The agent UI displays live state: "Tracking", "On a break", "Paused", or "Stopped".
   - Capture requires **per-user opt-in and visible local controls**, not just an administrative server setting. Users must have direct access to "Start", "Pause", "Resume", and "Quit" actions. Taking a break or locking the screen immediately terminates the active time block with an explicit `endReason` (`"break"`, `"lock"`).
2. **No Privilege Escalation as Default**:
   - Under no circumstances should the Linux agent require root, `sudo`, direct `/dev/input/event*` reading via `evdev`, or `uinput` privilege escalation by default.
   - Global input sniffing without user consent violates desktop security models. On Wayland, unsupported input capture or window tracking must be reported as **explicitly unavailable**, never fabricated as measured zeros.
3. **No Input Synthesis**:
   - The agent is an observer, not an actor. It must never synthesize mouse movements, inject keypresses, or generate fake activity to simulate presence.
4. **No Raw Keystrokes or Cadence Logging**:
   - Both Windows (`clients/windows/src/worker.rs`) and macOS (`clients/macos/.../InputMonitoringService.swift`) agents strictly discard keycodes, virtual keys, unicode characters, and inter-keystroke intervals. Only monotonic counts of keyboard events are stored in the SQLite block.
5. **No Silent Webcam Capture**:
   - Project settings default `AllowWebcam = false`. When enabled on the server, webcam captures require local user consent, visible indicator, and capture only discrete downscaled JPEGs once per interval.
6. **No Default Hardware Fingerprinting**:
   - The agent identifies installations using a generated UUIDv4 `DeviceId`. It does not collect MAC addresses, BIOS serial numbers, CPU IDs, or other persistent hardware fingerprints by default.
7. **Graceful Degradation vs Fabrication**:
   - When an OS capability (e.g. active window title or input count on Wayland) is unsupported or denied by user permissions, the agent and server must record that the metric is **unsupported / not measured** (`null`).
   - The system must never emit fake zeroes that fabricate the appearance of an idle employee when the sensor was simply unavailable.

### Unresolved Security & Privacy Gaps
- **Device Credential Hardening**: Windows uses DPAPI; macOS uses Keychain. Linux requires Secret Service API (`libsecret`) integration.
- **Revocation Propagation**: When an employee is removed from a workspace, existing device tokens must be immediately revoked via explicit server endpoint.
- **Data Retention Sweeper**: Ensure aged screenshot blobs are physically expunged from S3/R2 after the configured retention period.
- **Per-User Opt-in Transparency**: Ensure UI explicitly prompts the user for local opt-in before activating any screen or webcam capture pipelines.

---

## 4. Platform Readiness Matrix (Separated by Evidence Dimension)

To maintain absolute technical truth, evidence is divided into six concrete dimensions for each operating system:

| Evidence Dimension | Windows (Rust) | macOS (Swift) | Linux (Target) |
|---|---|---|---|
| **1. Implemented Source** | ✅ Complete (`clients/windows`: egui UI, Win32 hooks, xcap, DPAPI, SQLite queue) | ✅ Complete (`clients/macos`: SwiftUI, ScreenCaptureKit, Keychain, SQLite queue) | ❌ **Unresolved Gap** (`clients/linux` crate not yet created) |
| **2. Synthetic / Mock Unit Tests** | ✅ Complete (345 cargo tests passing: queue FIFO, backoff, parser, models) | ⚠️ Unverified locally (Swift test files present in source; not executed locally) | ❌ 0 tests (No crate) |
| **3. Toolchain / Compile / Build** | ✅ Verified locally (Rustc / Cargo 1.97.1 on Windows 11 host) | ❌ Unverified locally (Host lacks macOS toolchain / Xcode) | ❌ Unverified locally (Host lacks Linux build container / environment) |
| **4. Native Interactive Runtime** | ⚠️ **Outstanding** (Unit tests run synthetically; interactive Win32 desktop session unverified) | ⚠️ **Outstanding** (Requires physical macOS execution environment) | ❌ **Outstanding** (Requires Linux X11/Wayland execution environment) |
| **5. Packaging / Install / Uninstall** | ⚠️ Partial (NSIS `.nsi` script present; silent install/uninstall unverified in CI) | ⚠️ Partial (`build-app.sh` creates bundle; DMG packaging unverified) | ❌ **Unresolved Gap** (No `.deb`, AppImage, or Flatpak manifest) |
| **6. Code Signing & Notarization** | ⚠️ **Outstanding Prerequisite** (Unsigned; Authenticode EV cert required; SmartScreen must not be bypassed) | ⚠️ **Outstanding Prerequisite** (Unsigned; Apple Developer ID & Notarization required) | ⚠️ **Outstanding Prerequisite** (GPG package signing required) |

---

## 5. Actual Baseline Test Execution Matrix

All tests were executed on the local environment without assumptions.

| Tier / Suite | Target Command | Tests Passed | Tests Failed | Skipped / Ignored | Execution Time | Exit Code | Notes |
|---|---|:---:|:---:|:---:|:---:|:---:|---|
| **Windows Client** | `cargo test` in `clients/windows` | **345** | 0 | 5 | 6.12s | **0** | 5 ignored tests are end-to-end integration tests requiring a live backend and interactive desktop session. |
| **Backend Domain Tests**| `dotnet test` (Domain.Tests) | **60** | 0 | 0 | 0.08s | **0** | In-memory domain invariants and entity validations. |
| **Backend Host Tests** | `dotnet test` (HttpApi.Host.Tests) | **116** | 0 | 0 | 0.43s | **0** | HTTP routing, rate limiting, and controller filters. |
| **Backend App Tests** | `dotnet test` (Application.Tests) | **166** | 0 | 0 | 0.31s | **0** | Abstract application service tests and normalizers (`AgentContextNormalizerTests`). |
| **Backend EF Core Tests**| `dotnet test` (EntityFrameworkCore.Tests) | **466** | 0 | 0 | 2m 34s | **0** | Concrete SQLite EF Core host harness. Includes newly split `Create_Should_Accept_And_Store_Linux_Platform` and `Heartbeat_Should_Accept_And_Store_Linux_Platform` (increased from 464 to 466). |
| **Backend Acceptance** | `dotnet test` (Acceptance.Tests) | **15** | 0 | 0 | 13.0s | **0** | BDD multi-step workflow acceptance scenarios. |
| **Backend Total** | `dotnet test backend/Dosi.Tracker.slnx` | **823** | 0 | 0 | 2m 48s | **0** | **Reconciled count**: 820 baseline + 1 normalizer test (Phase 1) + 2 split EF Core Linux tests = 823 tests. |
| **Frontend Vitest** | `npm test` in `frontend` | **1,961** | 0 | 0 | 33.76s | **0** | 92 test files. Includes 5 new tests in `analytics-charts.test.ts` and 1 new test in `analytics.test.ts`. |
| **Frontend Accessibility**| `npm run test:a11y` in `frontend` | **49** | 0 | 1912 skipped | 23.09s | **0** | Runs all axe component tests and machine-contrast regressions (`PLATFORM_COLORS` $\ge 3:1$). |
| **Frontend TypeCheck** | `npm run typecheck` | N/A | 0 errors | 0 | 5.2s | **0** | TypeScript 5 strict compilation with no emit. |
| **Frontend ESLint** | `npm run lint` | N/A | 0 errors | 3 warnings (max 45)| 18.0s | **0** | 3 pre-existing warnings in unedited files. |
| **Frontend Architecture**| `npm run test:arch` | **12** | 0 | 0 | 0.67s | **0** | Dependency cruiser layering and `architecture.spec.ts` content invariants. |

---

## 6. Implementation & Follow-up Actions (Phase 1 Follow-up)

### Problems Identified in PM Review:
1. **Accessibility Contrast Regression**:
   - `frontend/src/components/reports/analytics-charts.tsx` previously selected `#e97b35` for Linux.
   - On light card background (`#ffffff`), this yielded only 2.857:1 contrast, failing the $\ge 3:1$ requirement.
2. **Missing Automated Contrast Regression**:
   - No automated test checked `PLATFORM_COLORS` contrast on actual light (`#ffffff`) and dark (`#1a2226`) card design tokens.
3. **Missing Linux `platformMix` Regression**:
   - `frontend/src/lib/analytics.test.ts` did not assert Linux durations, device counts, member counts, or ordering alongside Windows/macOS/unknown.
4. **Bolted-on Backend Linux Test Cases**:
   - Linux assertions had been appended to negative "unusable context" tests in `ActivityAppServiceTests.cs` and `PresenceAppServiceTests.cs` rather than running as independent test methods.
5. **Hidden Two-Platform Assumptions in Frontend**:
   - `frontend/src/lib/types.ts` (`DevicePlatform`), `analytics.ts` (`platformOf`), `report-math.ts` (`DevicePlatform`, `platformOf`), and `live-dataset.ts` (`PLATFORMS`) still restricted platform values to `"windows"` and `"macos"`.
6. **Documentation Ambiguities**:
   - Windows cargo test results were described as verifying native interactive capture rather than synthetic units.
   - macOS source presence was conflated with runtime verification.
   - Idempotency key was inaccurately documented as `(WorkspaceId, ClientActivityId)` instead of the unique index `(WorkspaceId, UserId, ClientActivityId)`.
   - SmartScreen guidance suggested bypass rather than marking code signing as an outstanding prerequisite.

### Solutions Applied & Verified:
1. **Accessible Linux Color Selection**:
   - Selected `#cb6e17` for Linux in `analytics-charts.tsx`.
   - Contrast on `#ffffff` (light card): **3.63:1** ($\ge 3:1$).
   - Contrast on `#1a2226` (dark card): **4.43:1** ($\ge 3:1$).
2. **Automated Accessibility Contrast Test**:
   - Created `frontend/src/components/reports/analytics-charts.test.ts`.
   - Directly imports source `PLATFORM_COLORS` constant (no duplicated driftable colors).
   - Verifies all platform keys (`windows`, `macos`, `linux`, `unknown`) achieve $\ge 3:1$ contrast against actual `--card` tokens (`#ffffff` and `#1a2226`).
3. **Dedicated `platformMix` Linux Regression**:
   - Added `platformMix orders Windows, macOS, Linux, and Not reported, computing durations, devices, members, and apportioned shares` in `analytics.test.ts`.
   - Asserts exact ordering (`["windows", "macos", "linux", null]`), device counts (2), member counts (1), durations (150m), and whole-percentage shares.
4. **Independent Backend Test Methods**:
   - Split Linux activity creation into `[Fact] public async Task Create_Should_Accept_And_Store_Linux_Platform()` in `ActivityAppServiceTests.cs`.
   - Split Linux heartbeat into `[Fact] public async Task Heartbeat_Should_Accept_And_Store_Linux_Platform()` in `PresenceAppServiceTests.cs`.
   - Verified concrete execution under `EfCoreActivityAppServiceTests` and `EfCorePresenceAppServiceTests` (466 passing tests).
5. **Full Platform Normalization Alignment**:
   - Updated `types.ts`, `analytics.ts`, `report-math.ts`, and `live-dataset.ts` to include `"linux"` across all platform types and parsers.
6. **Truthful Documentation Updates**:
   - Corrected `PM_STATUS.md` and `PM_EXECUTION_PLAN.md` with explicit evidence dimensions, corrected idempotency key indexes, and clarified privacy guardrails.

---

## 7. Recommended Safe Next Bounded Phase

### Recommended Phase: Bounded Linux Client Architecture Spike (`clients/linux`)
- **Safe Next Scope**:
  1. Initialize `clients/linux` Rust crate structure matching domain models and SQLite queue schema.
  2. Implement display server detection (detect X11 vs Wayland).
  3. Implement Wayland consent policy: explicitly report unsupported window/input metrics as **unavailable** (`null`) rather than fabricating zero metrics or attempting root privilege escalation.
  4. Implement X11 basic capture spike using user-space interfaces.
  5. Provide non-interactive unit tests (`cargo test`) for models, queue, and backoff logic.
- **Constraints**:
  - No deployment, no upstream push, no paid resources.
  - Do not attempt root/evdev/uinput privilege escalation.
  - Keep all changes strictly local and bounded.
