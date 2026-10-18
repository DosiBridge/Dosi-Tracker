# Dosi-Tracker — PM Status Report & Evidence Matrix

> **Document Class:** Production Readiness Audit & Status Tracking (Phase 1 Baseline)  
> **Status Date:** October 18, 2026  
> **Auditor / Implementation Engineer:** Antigravity Pair Engineer  
> **Supervisor:** PM / QA Operator  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)

---

## 1. Executive Status & Audit Summary

Following a comprehensive audit of the Dosi-Tracker codebase against product documentation (`DOCUMENTATION.md`, `docs/AS_BUILT.md`, `docs/PRODUCTION_READINESS.md`, `docs/SECURITY.md`, `docs/RUNBOOK.md`, `docs/QUALITY.md`, and `ENGINEERING_BLUEPRINT.md`), the repository status has been verified using real local toolchains.

### Key Audit Findings:
1. **Pipeline Core is Real**: Unlike earlier historical scaffolds from July 2026, the backend domain entities (`Activity`, `Screenshot`, `AgentPresence`, `Project`, `ProjectMember`, `Workspace`, `Subscription`, `Invoice`), app services, OpenIddict token endpoints, and screenshot decoding into ABP blob containers are implemented and functional.
2. **Frontend Live Integration**: Next.js 16 dashboard includes working live modes for workspace signups, login, projects, team member rates, real-clock reporting (`/api/app/reporting/summary`), and presence monitors.
3. **Desktop Agent Reality**:
   - **Windows Client**: Mature Rust desktop client (345 passing tests) implementing the real OpenIddict password grant (`Tracker_App`), local SQLite queue, exponential backoff, poison row parking (`synced = 2`), and DPAPI credential protection.
   - **macOS Client**: Complete Swift 5.9 implementation with `ScreenCaptureKit` and `DosiTrackerCore`. Runtime validation on this machine is blocked because the local environment is Windows 11 without macOS hardware or Xcode.
   - **Linux Client**: **Explicit Architectural Gap**. No Linux client codebase existed (`clients/linux` was not present). Furthermore, both backend and frontend previously hardcoded a two-platform assumption (`"windows"` and `"macos"` only), actively discarding `"linux"` payloads.
4. **Foundational Blocker Resolved in Phase 1**:
   - Updated backend `ActivityConsts.Platforms` to include `"linux"`.
   - Updated `AgentContextNormalizer` test suite to verify `"linux"` is accepted and unknown platforms (`"freebsd"`) are normalized to null.
   - Updated frontend `AgentPlatform` type, `PLATFORM_LABELS`, `devices.ts` normalizers, and `analytics-charts.tsx` to support Linux natively.

---

## 2. Evidence-Based Feature Checklist

| Category | Feature / Requirement | State | Verification Evidence / Location |
|---|---|:---:|---|
| **Auth & Identity** | OpenIddict password grant & token endpoint (`/connect/token`) | **Implemented** | `OpenIddictDataSeedContributor.cs`, `AuthRateLimitingTests.cs` (10 req/min rate limit active) |
| | Multi-tenant workspace resolution via query/header | **Implemented** | `WorkspaceResolutionTests.cs`, `CurrentWorkspace.cs`, `frontend/src/hooks/useApi.ts` |
| | Client token cache & refresh handling | **Implemented** | `clients/windows/src/worker/control.rs`, `clients/macos/Sources/DosiTrackerCore/ApiClient.swift` |
| | Role-based permissions (Owner, Admin, Worker) | **Implemented** | `TrackerPermissions.cs`, `WorkspacePermissionTests.cs`, `MemberScopingTests.cs` |
| **Tenant Isolation** | ABP global query filters on `IWorkspaceScoped` | **Implemented** | `WorkspacePersistenceTests.cs`, `ActivityPersistenceTests.cs` |
| | Foreign project membership / tenant injection guard | **Implemented** | `ActivityAppService.cs` line 118, `ProjectsAndTeamSteps.cs` |
| | Hourly pay-rate masking for non-managers | **Implemented** | `ProjectMembershipManagerTests.cs`, `TeamAppServiceTests.cs` |
| **Device Lifecycle** | Random `DeviceId` generation per agent install | **Implemented** | `AgentPresence.cs`, `clients/windows/src/state.rs` |
| | Device presence heartbeat (`/api/app/presence/heartbeat`) | **Implemented** | `PresenceAppServiceTests.cs`, `PresenceAppService.cs` (150 s online window) |
| | Automatic pruning of stale/surplus devices (>10 or >30d) | **Implemented** | `PresenceAppService.cs` `RemoveSurplusDevicesAsync` |
| | Cryptographic keypair device enrollment | **Untested / Gap** | Device authentication currently relies on user session JWT; no mutual device public-key registration |
| | Server-side device revocation endpoint | **Stubbed / Gap** | Revoking a stolen device without changing the user's password is not yet an explicit API |
| **Ingestion & Sync** | Time block upload (`POST /api/app/activity`) | **Implemented** | `ActivityAppServiceTests.cs`, `ActivityPersistenceTests.cs` |
| | Idempotency on `ClientActivityId` | **Implemented** | Unique index on `(WorkspaceId, ClientActivityId)`; duplicate insert caught and returns existing row |
| | Atomic block & capture validation | **Implemented** | `ActivityAppService.cs` `PendingCapture`; invalid captures skipped while preserving time block |
| | Screenshot blob storage (S3/R2/Database fallback) | **Implemented** | `ActivityCaptureStorageTests.cs`, `Screenshot.cs`, `BlobStoring` configuration |
| | Offline SQLite queue on client | **Implemented** | `clients/windows/src/storage.rs`, `clients/macos/Sources/DosiTrackerCore/QueueDatabase.swift` |
| | Poison row parking (`synced = 2`) | **Implemented** | `clients/windows/src/worker/uploader.rs` tests |
| **Time Reporting** | Tracked time & productivity duration-weighted math | **Implemented** | `ReportingAppService.cs`, `analytics.ts`, `work-report.ts` |
| | Calendar & timezone normalization (IANA conversion) | **Implemented** | `AgentContextNormalizer.cs`, `TimeZoneIds.cs`, `time-zone.ts` |
| | Member work rhythm & hourly distribution | **Implemented** | `ReportingAppServiceTests.cs`, `analytics-charts.tsx` |
| **Native Permissions** | Windows low-level input counting (no raw keys) | **Implemented** | `clients/windows/src/worker.rs` (rdev event counter) |
| | Windows active window & process enumeration | **Implemented** | `EnumWindows` visible-window enumeration in `clients/windows/src/worker.rs` |
| | macOS Screen Recording, Accessibility, Camera TCC | **Implemented** | `clients/macos/Sources/DosiTracker/Capture/ScreenCaptureService.swift` |
| | Linux X11/Wayland input & display handling | **Blocked / Gap** | `clients/linux` crate not yet created; Wayland portal session required |
| **Packaging & Dist** | Windows NSIS installer & uninstaller | **Implemented** | `clients/windows/installer/dosi-tracker.nsi`, autostart registry configuration |
| | Windows Authenticode code signing | **Untested / Gap** | Unsigned in local development builds; SmartScreen warning bypass required |
| | macOS App Bundle & Hardened Runtime | **Implemented (Code)**| `clients/macos/scripts/build-app.sh`; notarization requires Apple Developer account |
| | Linux packaging (.deb, AppImage, Flatpak) | **Blocked / Gap** | No Linux package configuration exists |
| **Observability & Ops**| Serilog structured logging & health checks | **Implemented** | `HttpApi.Host/Program.cs`, `/health-status` endpoint |
| | Prometheus metrics / OpenTelemetry tracing | **Stubbed / Gap** | No Prometheus scraper or OTel exporter configured |
| | Automated screenshot & activity retention cleanup | **Stubbed / Gap** | Retention limits defined in host settings, but periodic pruning background worker not wired |
| **Accessibility** | WCAG 2.1 AA jest-axe & Playwright axe scans | **Implemented** | 91 vitest test files include direct axe assertions across all dashboard hubs |
| | Dark mode machine-contrast verification | **Untested / Gap** | Dark theme tokens defined in CSS; automated axe test suite currently runs in light theme |

---

## 3. Security & Privacy Audit

### Consent & Surveillance Prevention Invariants
1. **Strictly Non-Stealth**: The desktop agent must run with a visible system tray icon (Windows), menu-bar item (macOS), or status indicator (Linux). The agent UI displays live state: "Tracking", "On a break", "Paused", or "Stopped".
2. **Transparent Employee Control**: Users have direct access to "Start", "Pause", "Resume", and "Quit" actions. Taking a break or locking the screen immediately terminates the active time block with an explicit `endReason` (`"break"`, `"lock"`).
3. **No Raw Keystrokes**: Both Windows (`clients/windows/src/worker.rs`) and macOS (`clients/macos/.../InputMonitoringService.swift`) agents strictly discard keycodes, virtual keys, and unicode characters. Only monotonic counts of keyboard events are stored in the SQLite block.
4. **No Silent Webcam Capture**: Project settings default `AllowWebcam = false`. When enabled, photos are downscaled JPEGs taken once per interval; webcam photos are never captured when the project disables them.
5. **Private Screenshot Storage**: Screenshot payloads are authenticated, decoded into private blob storage (never public S3 buckets), and restricted to authorized project managers or the block's owner.

### Open Security Gaps for Subsequent Phases
- **Device Credential Hardening**: Windows uses DPAPI; macOS uses Keychain. Linux will require libsecret / Secret Service integration.
- **Revocation Propagation**: When an employee is removed from a workspace, the server immediately rejects future activity uploads (`Tracker:NotProjectMember`), but existing local device tokens should be explicitly invalidated.
- **Data Retention Sweeper**: Ensure aged screenshot blobs are physically expunged from S3/R2 after the configured retention period.

---

## 4. Platform Readiness Matrix

| Feature / Capability | Windows | macOS | Linux | Notes & Blockers |
|---|:---:|:---:|:---:|---|
| **Client Codebase** | ✅ Implemented | ✅ Implemented | ❌ **Blocked (Gap)** | Rust Windows client ready; Swift macOS client ready; Linux client crate needs creation. |
| **Offline SQLite Queue** | ✅ Verified | ✅ Implemented | ❌ Blocked | Tested with FIFO order, backoff doubling, poison-row parking. |
| **Screen Capture** | ✅ Verified (`xcap`) | ✅ Implemented (`ScreenCaptureKit`) | ❌ Blocked | Linux requires X11 (`XShmGetImage`) and Wayland (`xdg-desktop-portal` PipeWire). |
| **Input Event Counting** | ✅ Verified (`rdev`) | ✅ Implemented (`CGEvent` tap) | ❌ Blocked | Wayland blocks global input sniffing without root; requires compositor protocol or fallback. |
| **Active App Tracking** | ✅ Verified (`EnumWindows`)| ✅ Implemented (`NSWorkspace`) | ❌ Blocked | X11 uses `_NET_ACTIVE_WINDOW`; Wayland requires desktop-specific APIs. |
| **System Tray / UI** | ✅ Verified (`egui`) | ✅ Implemented (Menu bar) | ❌ Blocked | Linux needs StatusNotifierItem / AppIndicator. |
| **Local Toolchain & Build** | ✅ Cargo 1.97.1 | ❌ No Xcode on host | ❌ No Linux host | Local host is Windows 11; macOS and Linux compilation require appropriate runners/containers. |
| **Packaging & Installers** | ✅ NSIS Script | ⚠️ Zip bundle | ❌ None | Windows NSIS ready; macOS requires Developer ID notarization; Linux needs AppImage/Deb. |
| **Runtime Test Verification**| ✅ 345 cargo tests | ⚠️ Swift tests in CI only | ❌ 0 tests | 345 Windows client tests green locally; macOS runtime validation blocked locally. |

---

## 5. Actual Baseline Test Execution Matrix

All baseline checks were executed on the local environment without assumptions.

| Tier / Suite | Target Command | Tests Passed | Tests Failed | Skipped / Ignored | Execution Time | Exit Code |
|---|---|:---:|:---:|:---:|:---:|:---:|
| **Windows Client** | `cargo test` in `clients/windows` | **345** | 0 | 5 (e2e needing live host) | 6.14s | **0** |
| **Backend Host Tests** | `dotnet test` (HttpApi.Host.Tests) | **116** | 0 | 0 | 1.0s | **0** |
| **Backend Domain Tests**| `dotnet test` (Domain.Tests) | **60** | 0 | 0 | 0.8s | **0** |
| **Backend App Tests** | `dotnet test` (Application.Tests) | **166** | 0 | 0 | 0.6s | **0** |
| **Backend EF Core Tests**| `dotnet test` (EntityFrameworkCore.Tests) | **464** | 0 | 0 | 1m 58s | **0** |
| **Backend Acceptance** | `dotnet test` (Acceptance.Tests) | **15** | 0 | 0 | 14.0s | **0** |
| **Backend Total** | `dotnet test backend/Dosi.Tracker.slnx` | **821** | 0 | 0 | 2m 15s | **0** |
| **Frontend Vitest** | `npm test` in `frontend` | **1,955** | 0 | 0 | 30.29s | **0** |
| **Frontend TypeCheck** | `npm run typecheck` | N/A | 0 errors | 0 | 5.2s | **0** |
| **Frontend ESLint** | `npm run lint` | N/A | 0 errors | 3 warnings (max 45)| 18.0s | **0** |
| **Frontend Architecture**| `npm run test:arch` | **12** | 0 | 0 | 0.7s | **0** |
| **Frontend Next Build** | `npm run build` | N/A (33 routes) | 0 | 0 | 15.6s | **0** |
| **Frontend Bundle Budget**| `npm run build:budget` | N/A (2340 KB / 2400 KB) | 0 | 0 | 0.8s | **0** |

---

## 6. Implementation of Foundational Blocker (Phase 1)

### Identified Problem:
Cross-platform platform normalization in the backend and frontend was previously locked to `"windows"` and `"macos"` only:
- Backend: `ActivityConsts.Platforms = { "windows", "macos" }` caused any incoming Linux agent activity block or heartbeat to be normalized to `null`.
- Frontend: `AgentPlatform = "windows" | "macos"` in `api-contract.ts`, `platformOf` in `devices.ts`, and `PLATFORM_COLORS` in `analytics-charts.tsx` had no support for Linux, causing TypeScript compiler errors or dropping Linux devices to "Unknown OS" / "Not reported".

### Solution Applied & Verified:
1. `backend/src/Dosi.Tracker.Domain.Shared/Activities/ActivityConsts.cs`: Added `"linux"` to `ActivityConsts.Platforms`.
2. `backend/test/Dosi.Tracker.Application.Tests/Activities/AgentContextNormalizerTests.cs`: Updated unit tests to verify `"linux"` is accepted as a valid platform and `"freebsd"` is normalized to null.
3. `backend/test/Dosi.Tracker.Application.Tests/Activities/ActivityAppServiceTests.cs`: Updated unusable context tests to use `"freebsd"` and verified `"linux"` activity block creation.
4. `backend/test/Dosi.Tracker.Application.Tests/Presence/PresenceAppServiceTests.cs`: Updated unusable context tests to use `"freebsd"` and verified `"linux"` heartbeat presence storage.
5. `backend/src/Dosi.Tracker.Domain/Presence/AgentPresence.cs`: Updated doc comments to reflect Linux platform support.
6. `frontend/src/lib/api-contract.ts`: Extended `AgentPlatform` type with `"linux"` and updated `PLATFORM_LABELS` and `platformLabel`.
7. `frontend/src/lib/api-contract.test.ts`: Added assertions for `platformLabel("linux") === "Linux"` and `platformLabel("freebsd") === null`.
8. `frontend/src/components/dashboard/devices.ts`: Updated `platformOf` to accept `"linux"`.
9. `frontend/src/components/dashboard/devices.test.ts`: Added test asserting `osText({ platform: "linux", osVersion: null }) === "Linux"`.
10. `frontend/src/lib/activity-report.test.ts`: Updated malformed-platform fixture to test rejection of unknown platforms (`"gameboy"`) and acceptance of `"linux"`.
11. `frontend/src/lib/analytics.ts`: Added `"linux"` to `platformMix` ordering.
12. `frontend/src/components/reports/analytics-charts.tsx`: Added Linux theme color (`#e97b35`) to `PLATFORM_COLORS`.

All tests passed with zero regressions across both backend and frontend suites.

---

## 7. Recommended Next Bounded Phase (Phase 2)

### Proposed Scope:
1. **Linux Desktop Agent Architecture & Shared Core (`clients/linux`)**:
   - Initialize `clients/linux` Rust crate sharing configuration and SQLite queue schemas with `clients/windows`.
   - Implement Linux display server detector (X11 vs Wayland).
   - Implement X11 input and window title monitor.
   - Design Wayland `xdg-desktop-portal` user-prompt screencast capture module.
   - Provide a command-line test runner (`cargo test`) verifying the queue, models, skew calculation, and upload backoff on Linux.
2. **Device Enrollment & Cryptographic Registration Blueprint**:
   - Design the device keypair enrollment flow to transition from ad-hoc `DeviceId` GUIDs to authenticated device credentials.
