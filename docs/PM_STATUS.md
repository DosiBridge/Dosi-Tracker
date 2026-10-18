# Dosi-Tracker — PM Status Report & Evidence Matrix

> **Document Class:** Production Readiness Audit & Status Tracking (Phase 1 Follow-up 2)  
> **Status Date:** October 18, 2026  
> **Auditor / Implementation Engineer:** Antigravity Pair Engineer  
> **Supervisor:** PM / QA Operator  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)

---

## 1. Executive Status & Audit Summary

Following independent review by the PM/QA operator and follow-up verification across the Dosi-Tracker codebase against product documentation (`DOCUMENTATION.md`, `docs/AS_BUILT.md`, `docs/PRODUCTION_READINESS.md`, `docs/SECURITY.md`, `docs/RUNBOOK.md`, `docs/QUALITY.md`, and `ENGINEERING_BLUEPRINT.md`), repository evidence is tracked with strict technical boundaries.

### Key Audit Findings & Evidence Distinctions:
1. **Pipeline Core is Real (Synthetic / Test-Host Verified)**:
   - Backend domain entities (`Activity`, `Screenshot`, `AgentPresence`, `Project`, `ProjectMember`, `Workspace`, `Subscription`, `Invoice`), app services, OpenIddict token endpoints, and screenshot decoding into ABP blob containers are implemented.
   - Verification is conducted via concrete Entity Framework Core SQLite in-memory test host fixtures (`Dosi.Tracker.EntityFrameworkCore.Tests`) and simulated authentication principals.
   - This proves source correctness and schema consistency, but does **not** constitute a live multi-node PostgreSQL / OpenIddict / S3 production cluster deployment verification.
2. **Frontend Integration (Component & Simulated Session Verified)**:
   - Next.js 16 dashboard includes comprehensive UI flows for workspace signups, login, projects, team member rates, real-clock reporting (`/api/app/reporting/summary`), and presence monitors.
   - Tests run in Node.js / jsdom using mocked fetch handlers (`useApi`), memory datasets, and `@testing-library/react`.
   - **Accessibility Boundary**: Automated `jest-axe` checks in jsdom and programmatic contrast calculations cover only automated, programmatic accessibility invariants. Axe in jsdom does **not** prove full WCAG 2.1 AA/AAA compliance; real-browser keyboard focus management, screen reader announcements, focus traps, visual layout reflow, and assistive tech audits remain open.
3. **Desktop Agent Reality & Strict OS Evidence Separation**:
   - **Windows Client**: Source code present (`clients/windows`) with 345 passing cargo unit/integration tests.
     - *Evidence Reality*: Cargo tests verify models, SQLite queue FIFO ordering, exponential backoff, poison row parking (`synced = 2`), and synthetic worker state machines.
     - *Crucial Distinction*: Source presence is **not** proof of complete implementation. Cargo tests use mock inputs and synthetic structures; they do **NOT** verify native interactive hardware execution (live display capture via DXGI/GDI, live global low-level keyboard/mouse hooks, or live Win32 focus events on an active desktop session). Native runtime, per-user consent UI, packaging, and signing remain open.
   - **macOS Client**: Swift 5.9 source code present under `clients/macos/` using `ScreenCaptureKit`, `AVFoundation`, and `Accessibility` APIs.
     - *Evidence Reality*: Source presence is **NOT** runtime validation or complete implementation. CI configuration files (`.github/workflows/macos.yml`) are not proof tests ran.
     - *Crucial Distinction*: On this Windows 11 host, Swift toolchain and macOS hardware are absent; native build, test execution, TCC consent handling, packaging, notarization, and runtime validation remain unexecuted and outstanding.
   - **Linux Client**: **Explicit Architectural Gap**.
     - No Linux client crate exists (`clients/linux` is not yet created).
4. **Phase 1 Follow-up Refinements Completed**:
   - Fixed accessibility contrast regression: replaced low-contrast Linux color `#e97b35` (2.86:1 on white) with `#cb6e17` (3.63:1 on `#ffffff`, 4.43:1 on `#1a2226`).
   - Derived actual `--card` background tokens dynamically from [globals.css](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/frontend/src/app/globals.css) in [analytics-charts.test.ts](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/frontend/src/components/reports/analytics-charts.test.ts) rather than hardcoding literals.
   - Added guards against stale literals, malformed tokens, and media-query print overrides (`.dark .work-report`).
   - Explicitly prefixed every test in `analytics-charts.test.ts` with `a11y:`, ensuring `npm run test:a11y` explicitly matches and executes all 9 contrast/token tests.
   - Split backend Linux tests into independent `[Fact]` methods executed by concrete EF Core host harness.

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
The PM/QA operator independently verified the following checks on commit `3e5ed99`:
1. **Clean Working Tree & HEAD**: Verified key-based SSH, clean tree, and HEAD commit `3e5ed99`.
2. **Concrete EF Core Linux Tests**:
   - `EfCorePresenceAppServiceTests.Heartbeat_Should_Accept_And_Store_Linux_Platform`: **Passed**
   - `EfCoreActivityAppServiceTests.Create_Should_Accept_And_Store_Linux_Platform`: **Passed**
   - Exit code: **0** (synthetic/test-host results, not native runtime).
3. **Frontend Validation Gate (`npm run validate`)**:
   - TypeScript typecheck: **0 errors**
   - ESLint: **0 errors**, 3 pre-existing warnings (under 45 ceiling)
   - Vitest: **1,961 tests in 92 files passed**
   - Architecture spec: **12 passed**, dependency graph clean
   - Exit code: **0** (synthetic/test-host results in jsdom, not live browser WAN).

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
| **Frontend Vitest (Full)** | `npm test` in `frontend` | **1,965** | 0 | 0 | 49.04s | **0** | 92 test files. Includes 9 tests in `analytics-charts.test.ts` (+4 guard tests added) and 1 test in `analytics.test.ts`. |
| **Frontend Accessibility**| `npm run test:a11y` in `frontend` | **53** | 0 | 1912 skipped | 22.26s | **0** | Matches `--testNamePattern a11y`. Now explicitly executes all 9 tests in `analytics-charts.test.ts` (up from 49). |
| **Frontend TypeCheck** | `npm run typecheck` | N/A | 0 errors | 0 | 5.2s | **0** | TypeScript 5 strict compilation with no emit. |
| **Frontend ESLint** | `npm run lint` | N/A | 0 errors | 3 warnings (max 45)| 18.0s | **0** | 3 pre-existing warnings in unedited files. |
| **Frontend Architecture**| `npm run test:arch` | **12** | 0 | 0 | 0.72s | **0** | Dependency cruiser layering and `architecture.spec.ts` content invariants. |

---

## 6. Implementation & Follow-up Actions (Phase 1 Follow-up 2)

### Problems Identified in PM Review:
1. **Hardcoded Background Tokens in Contrast Tests**:
   - `analytics-charts.test.ts` previously hardcoded `#ffffff` and `#1a2226` as literals rather than deriving the real CSS tokens from `globals.css`.
2. **Missing Proof of Contrast Test Inclusion in `test:a11y`**:
   - `npm run test:a11y` filters tests by `--testNamePattern a11y`. The contrast tests lacked explicit `a11y:` prefixes in their test names, leaving claims of their execution ambiguous.
3. **Overclaimed Implementation Completeness & WCAG Compliance**:
   - Windows and macOS clients were labeled as "Complete" based on source presence alone, ignoring missing runtime and packaging validation.
   - Blanket claims suggested automated axe in jsdom established full WCAG compliance.
   - Code signing was claimed to eliminate SmartScreen warnings (which requires reputation accumulation).
   - Event counting was associated with legal compliance without supporting organizational evidence.

### Solutions Applied & Verified:
1. **Dynamic Background Token Parsing from CSS**:
   - In [analytics-charts.test.ts](file:///d:/Github%20Projects/DosiBridge/Open%20Source/Dosi-Tracker/frontend/src/components/reports/analytics-charts.test.ts), implemented `extractCardTokens` to parse `:root` (`#ffffff`) and `.dark` (`#1a2226`) `--card` tokens directly from `src/app/globals.css`.
   - Added guards verifying that synthetic token changes are picked up, missing/malformed tokens throw descriptive errors, and print overrides (`.dark .work-report`) are not accidentally selected.
2. **Explicit `a11y:` Test Naming & Verified Execution**:
   - Prefixed every test in `analytics-charts.test.ts` with `a11y:`.
   - Verified that `npm run test:a11y` executes all 9 contrast/token tests, passing 53 a11y tests total.
3. **Truthful Documentation Boundaries**:
   - Clarified that source presence is not complete implementation.
   - Separated operator independently verified results from worker-reported claims.
   - Clarified that automated axe and contrast tests in jsdom do not prove full WCAG compliance.
   - Clarified that Authenticode signing builds reputation but does not guarantee immediate SmartScreen warning elimination.
   - Clarified that monotonic event counting is data minimization, not full legal compliance.

---

## 7. Recommended Safe Next Bounded Phase

### Recommended Phase: Bounded Consent & Capability Audit
- **Scope**:
  1. Define and audit user consent contracts across desktop platforms:
     - Require an explicit local user opt-in before activating screen, webcam, or activity monitors.
     - Ensure the client interface prominently displays active capture state and visible controls ("Start", "Pause", "Resume", "Quit").
  2. Audit Wayland capability detection:
     - Map unsupported Wayland capabilities (e.g. global input counting without root, active-window title sniffing) to an **explicitly unavailable** status (`null`).
     - Prohibit zero-metric fabrication (reporting 0 clicks/keys when the sensor is simply unavailable).
     - Prohibit root, `sudo`, `evdev`, or `uinput` privilege escalation as a default operational mode.
  3. No Linux collection adapter or device enrollment rewrite should be undertaken prior to completing this consent and capability audit.
- **Constraints**: Preparation only; no push, no public release, no deployment. Preserves existing codebase integrity.
