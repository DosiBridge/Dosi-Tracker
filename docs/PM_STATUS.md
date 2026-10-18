# Dosi-Tracker — PM Status Report & Evidence Matrix

> **Document Class:** Production Readiness Audit & Status Tracking (Phase 2: Consent & Capability Audit)
> **Status Date:** October 18, 2026  
> **Auditor / Implementation Engineer:** Antigravity Pair Engineer  
> **Supervisor:** PM / QA Operator  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)

---

## 1. Executive Status & Audit Summary

Following independent review and supervisory acceptance by the PM/QA operator of the contrast and evidence corrections at commit `59403f8`, this phase executed a comprehensive, evidence-based **consent & capability audit and implementation-ready specification** before implementing Linux collection or device enrollment redesign. The full audit findings and specifications are established in the dedicated artifact [docs/CONSENT_CAPABILITY_AUDIT.md](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/docs/CONSENT_CAPABILITY_AUDIT.md).

### Key Audit Findings & Evidence Distinctions:
1. **Desktop Hook Lifetime vs User Consent (Source-Verified)**:
   - **Windows Client**: `main.rs:125` invokes `Tracker::new()`, which unconditionally spawns `input::spawn_listener` (`WH_KEYBOARD_LL` and `WH_MOUSE_LL` hooks via `SetWindowsHookExW`) and `usage::spawn_sampler` (3s active window poll) at application launch **before** user authentication, project selection, or local consent.
   - **macOS Client**: `AppDelegate.swift:86` invokes `services.start()`, which starts `inputMonitor` (`CGEvent.tapCreate` on dedicated `TapThread` with a 10s retry timer) and `activeApp` at application launch **before** local opt-in.
   - **Paused/Stopped State Reality**: In both Windows (`worker.rs:1654`) and macOS (`TrackerWorker.swift:1262`), pausing or stopping tracking only halts block creation. It does **NOT** detach OS input hooks or stop background window monitoring; events continue logging into ring buffers and are merely discarded outside block boundaries.
   - **Admin Policy vs Local Consent**: Administrative project settings on the server (`allow_screenshot`, `allow_keyboard`, etc.) and OS permission grants (macOS TCC prompts) do not constitute per-user application consent. The settings UI currently displays admin policy as read-only and lacks local interactive refusal controls.
2. **End-to-End Metric Nullability Obstacles**:
   - Concrete non-nullable primitive integer fields in `Activity.KeyboardHits` / `MouseClicks` ([Activity.cs:20-21](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/backend/src/Dosi.Tracker.Domain/Activities/Activity.cs#L20-L21)) and upload DTOs ([CreateActivityDto.cs:40-43](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/backend/src/Dosi.Tracker.Application.Contracts/Activities/CreateActivityDto.cs#L40-L43), [ActivityDto.cs:17-18](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/backend/src/Dosi.Tracker.Application.Contracts/Activities/ActivityDto.cs#L17-L18)) force unmeasured or denied input metrics to serialize/persist as `0`.
   - Windows `Activity.keyboard_hits` ([model.rs:157](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/clients/windows/src/model.rs#L157)) and macOS `ActivityPayload.keyboardHits` ([ActivityPayload.swift:197](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/clients/macos/Sources/DosiTrackerCore/Models/ActivityPayload.swift#L197)) are primitive numbers, preventing client-side `null` transmission.
   - Frontend ingestion ([live-dataset.ts:444](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/frontend/src/lib/live-dataset.ts#L444), [work-report.ts:1259](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/frontend/src/lib/work-report.ts#L1259)) coerces missing metrics to `0`.
   - A formal state taxonomy is defined separating **measured zero**, **denied**, **unsupported**, **temporarily unavailable**, and **paused**.
3. **Truthful Linux X11/Wayland Capability Matrix**:
   - Wayland compositor client isolation prevents passive global input monitoring and active-window title sniffing without privilege escalation.
   - Wayland screen capture requires `xdg-desktop-portal` ScreenCast session with explicit user display selection; cancellation (code 1) or session close must be recorded as `denied`/`unavailable`.
   - Product policy strictly prohibits `root`, `sudo`, direct `/dev/input/event*` reading via `evdev`, and `uinput` bypasses.
4. **Independent Operator Baseline Maintained**:
   - Operator independently verified clean tree, commit `59403f8`, `test:a11y` (53 passed, 1912 skipped, all 9 analytics-charts token/contrast tests included, exit 0), and `validate` (1,965 passed in 92 files, exit 0).
   - Full backend 823 and Windows cargo 345 remain worker-reported. Synthetic/jsdom only, no native collection.

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
| **Native Permissions** | Windows low-level input counting (no raw keys) | **Source Present (Synthetic Only)** | `clients/windows/src/worker.rs` (rdev event counter in source; unit tested via simulated events; native hardware hooks and consent unverified) |
| | Windows active window & process enumeration | **Source Present (Synthetic Only)** | `EnumWindows` visible-window enumeration in source; native interactive window enumeration unverified |
| | macOS Screen Recording, Accessibility, Camera TCC | **Source Present Only** | `clients/macos/Sources/DosiTracker/Capture/ScreenCaptureService.swift`; runtime execution and TCC consent unverified |
| | Linux X11/Wayland input & display handling | **Unresolved Gap** | `clients/linux` crate not yet created; Wayland portal session required |
| **Packaging & Dist** | Windows NSIS installer & uninstaller | **Partial (Source Script)** | `clients/windows/installer/dosi-tracker.nsi`; packaging and silent install/uninstall unverified in CI |
| | Windows Authenticode code signing | **Outstanding Prerequisite** | Unsigned in local builds; code signing certificate required. Signing does not guarantee immediate elimination of SmartScreen warnings (reputation accumulation required); SmartScreen must not be bypassed. |
| | macOS App Bundle & Hardened Runtime | **Partial (Source Script)** | `clients/macos/scripts/build-app.sh`; Apple Developer ID signing and notarization are outstanding prerequisites. |
| | Linux packaging (.deb, AppImage, Flatpak) | **Unresolved Gap** | No Linux package configuration exists |
| **Observability & Ops**| Serilog structured logging & health checks | **Implemented (Test-Host Verified)** | `HttpApi.Host/Program.cs`, `/health-status` endpoint |
| | Prometheus metrics / OpenTelemetry tracing | **Unresolved Gap** | No Prometheus scraper or OTel exporter configured |
| | Automated screenshot & activity retention cleanup | **Unresolved Gap** | Retention limits defined in host settings, but periodic pruning background worker not wired |
| **Accessibility** | Programmatic component axe & machine contrast checks | **Partially Verified (Automated in jsdom)** | Automated `jest-axe` checks in jsdom and programmatic contrast calculations pass (53 a11y tests); does NOT prove full WCAG compliance. Real-browser keyboard traps, screen reader verification, zoom reflow, and manual assistive audits remain open. |
| | Machine contrast verification on card tokens | **Implemented (Unit Verified)** | `analytics-charts.test.ts` derives actual `--card` tokens from `globals.css` and asserts every member of `PLATFORM_COLORS` $\ge 3:1$ against `#ffffff` and `#1a2226` |

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
   - *Legal Precision*: Recording monotonic event tallies rather than raw keycodes is a privacy-conscious data minimization measure, but does **not** by itself establish legal compliance with GDPR, CCPA, or labor laws. Compliance depends on lawful basis, employee notice, data processing agreements, proportional monitoring policies, and data subject rights.
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
| **1. Implemented Source** | ⚠️ Source present (`clients/windows`: egui UI, Win32 hooks, xcap, DPAPI, SQLite queue); per-user consent UI, native hardware capture, packaging, and signing are NOT complete and remain open | ⚠️ Source present (`clients/macos`: SwiftUI, ScreenCaptureKit, Keychain, SQLite queue); source presence is NOT proof of complete implementation; TCC consent, runtime execution, packaging, and notarization remain open | ❌ **Unresolved Gap** (`clients/linux` crate not yet created) |
| **2. Synthetic / Mock Unit Tests** | ✅ Complete (345 cargo tests passing: queue FIFO, backoff, parser, models) | ⚠️ Unverified locally (Swift test files present in source; not executed locally) | ❌ 0 tests (No crate) |
| **3. Toolchain / Compile / Build** | ✅ Verified locally (Rustc / Cargo 1.97.1 on Windows 11 host) | ❌ Unverified locally (Host lacks macOS toolchain / Xcode) | ❌ Unverified locally (Host lacks Linux build container / environment) |
| **4. Native Interactive Runtime** | ⚠️ **Outstanding** (Unit tests run synthetically; interactive Win32 desktop session unverified) | ⚠️ **Outstanding** (Requires physical macOS execution environment) | ❌ **Outstanding** (Requires Linux X11/Wayland execution environment) |
| **5. Packaging / Install / Uninstall** | ⚠️ Partial (NSIS `.nsi` script present; silent install/uninstall unverified in CI) | ⚠️ Partial (`build-app.sh` creates bundle; DMG packaging unverified) | ❌ **Unresolved Gap** (No `.deb`, AppImage, or Flatpak manifest) |
| **6. Code Signing & Notarization** | ⚠️ **Outstanding Prerequisite** (Unsigned; Authenticode cert required; signing does not guarantee immediate SmartScreen elimination; SmartScreen must not be bypassed) | ⚠️ **Outstanding Prerequisite** (Unsigned; Apple Developer ID & Notarization required) | ⚠️ **Outstanding Prerequisite** (GPG package signing required) |

---

## 5. Actual Baseline Test Execution Matrix

### A. Operator Independently Verified (Windows SSH Session)
The PM/QA operator independently verified the following checks on commit `59403f8`:
1. **Clean Working Tree & HEAD**: Verified key-based SSH, clean tree, and unchanged HEAD commit `59403f8`.
2. **Frontend Accessibility Suite (`npm run test:a11y`)**:
   - Matches `--testNamePattern a11y`.
   - **53 passed, 1912 skipped**, all 9 `analytics-charts.test.ts` token/contrast tests included, exit code **0**.
3. **Frontend Validation Gate (`npm run validate`)**:
   - TypeScript typecheck: **0 errors**, exit code **0**.
   - ESLint: **0 errors**, 3 pre-existing warnings (under 45 ceiling), exit code **0**.
   - Vitest: **1,965 tests in 92 files passed**, exit code **0**.
   - Architecture spec: **12 passed with no dependency violations**, exit code **0**.
   - Exit code: **0** (synthetic/jsdom only, no native collection).
4. **Concrete EF Core Linux Tests**:
   - `EfCorePresenceAppServiceTests.Heartbeat_Should_Accept_And_Store_Linux_Platform`: **Passed** (exit 0)
   - `EfCoreActivityAppServiceTests.Create_Should_Accept_And_Store_Linux_Platform`: **Passed** (exit 0)
   - Synthetic/test-host results, not native runtime.

### B. Worker-Reported Suite Executions

| Tier / Suite | Target Command | Tests Passed | Tests Failed | Skipped / Ignored | Execution Time | Exit Code | Notes |
|---|---|:---:|:---:|:---:|:---:|:---:|---|
| **Windows Client** | `cargo test` in `clients/windows` | **345** | 0 | 5 | 6.12s | **0** | Worker-reported; 5 ignored tests require live backend and interactive desktop session. |
| **Backend Domain Tests**| `dotnet test` (Domain.Tests) | **60** | 0 | 0 | 0.08s | **0** | Worker-reported; domain invariants and entity validations. |
| **Backend Host Tests** | `dotnet test` (HttpApi.Host.Tests) | **116** | 0 | 0 | 0.43s | **0** | Worker-reported; HTTP routing, rate limiting, and controller filters. |
| **Backend App Tests** | `dotnet test` (Application.Tests) | **166** | 0 | 0 | 0.31s | **0** | Worker-reported; abstract application service tests and normalizers. |
| **Backend EF Core Tests**| `dotnet test` (EntityFrameworkCore.Tests) | **466** | 0 | 0 | 2m 34s | **0** | Worker-reported; concrete SQLite EF Core host harness. Includes the two independently verified Linux tests. |
| **Backend Acceptance** | `dotnet test` (Acceptance.Tests) | **15** | 0 | 0 | 13.0s | **0** | Worker-reported; BDD multi-step workflow acceptance scenarios. |
| **Backend Total** | `dotnet test backend/Dosi.Tracker.slnx` | **823** | 0 | 0 | 2m 48s | **0** | Worker-reported full backend suite. |
| **Frontend Vitest (Full)** | `npm test` in `frontend` | **1,965** | 0 | 0 | 49.04s | **0** | 92 test files. Includes 9 tests in `analytics-charts.test.ts` and 1 test in `analytics.test.ts`. |
| **Frontend Accessibility**| `npm run test:a11y` in `frontend` | **53** | 0 | 1912 skipped | 22.26s | **0** | Matches `--testNamePattern a11y`. All 9 tests in `analytics-charts.test.ts` included. |
| **Frontend TypeCheck** | `npm run typecheck` | N/A | 0 errors | 0 | 5.2s | **0** | TypeScript 5 strict compilation with no emit. |
| **Frontend ESLint** | `npm run lint` | N/A | 0 errors | 3 warnings (max 45)| 18.0s | **0** | 3 pre-existing warnings in unedited files. |
| **Frontend Architecture**| `npm run test:arch` | **12** | 0 | 0 | 0.72s | **0** | Dependency cruiser layering and `architecture.spec.ts` content invariants. |

---

## 6. Implementation & Follow-up Actions (Phase 2: Consent & Capability Audit)

### Actions Completed in Current Sprint:
1. **Established Full Audit Specification Artifact**:
   - Authored [docs/CONSENT_CAPABILITY_AUDIT.md](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/docs/CONSENT_CAPABILITY_AUDIT.md) defining collection call chains, local opt-in contracts, end-to-end metric nullability tracing, and Linux X11/Wayland capability matrices.
2. **Call Chain & Hook Lifetime Investigation**:
   - Verified that Windows (`main.rs:125`, `tracking/mod.rs:69`, `input.rs:694-850`) and macOS (`AppDelegate.swift:86`, `AgentServices.swift:58-63`, `InputMonitor.swift:95-260`) spawn OS input hooks and window samplers at launch before local user consent or project selection.
   - Proved that paused/stopped state does not detach hooks or stop background window monitoring; it merely discards output outside recorded blocks.
   - Clarified that admin project policies and OS permission grants are not per-user application consent.
3. **End-to-End Metric Nullability Obstacle Identification**:
   - Traced non-nullable primitive integers in `Activity.KeyboardHits` / `MouseClicks` (`Activity.cs:20-21`, `CreateActivityDto.cs:40-43`, `ActivityDto.cs:17-18`, `model.rs:157-158`, `ActivityPayload.swift:197-198`).
   - Defined formal 5-state taxonomy: **measured zero**, **denied**, **unsupported**, **temporarily unavailable**, and **paused**.
4. **Truthful Linux X11/Wayland Capability Matrix**:
   - Mapped Wayland compositor isolation rendering global input and active window titles unsupported without root.
   - Documented `xdg-desktop-portal` ScreenCast session cancellation and close signals.
   - Formally prohibited root, `sudo`, `evdev`, and `uinput` privilege elevation.
5. **Policy & Constraint Documentation Correction**:
   - In [PM_EXECUTION_PLAN.md](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/docs/PM_EXECUTION_PLAN.md), removed the unsupported blanket statement that privacy laws strictly prohibit all key logging; accurately framed the product prohibition without providing legal advice.
   - Consistently labeled all previous native suite counts (Windows cargo 345, backend 823) as worker-reported.

---

## 7. Recommended Safe Next Bounded Phase

### Recommended Phase: Phase 3 — Schema Nullability Alignment & Linux Desktop Architecture Spike
- **Scope (Smallest Next Implementation Slice First)**:
  1. **Backend Nullability Migration**:
     - Update `Activity.cs`: change `MouseClicks` and `KeyboardHits` to `int?`.
     - Update `CreateActivityDto.cs` and `ActivityDto.cs` to `int?`.
     - Generate and apply EF Core migration for nullable columns.
     - Update `ActivityAppService.cs` to store incoming nulls faithfully.
  2. **Frontend Ingestion Handling**:
     - Update `live-dataset.ts` and `work-report.ts` to preserve nulls and display "—" / "Not Measured" rather than coercing to 0.
  3. **Verification Gate**:
     - Verify both legacy numeric 0 and null metrics via EF Core host harness.
     - Verify full frontend suite (`npm run validate`).
  4. **Linux Component Creation (Subsequent Slice)**:
     - Establish `clients/linux` crate with X11 display capture and Wayland portal session, without privilege escalation.
- **Constraints**: Preparation only; no push, no public release, no deployment. Preserves existing codebase integrity.
