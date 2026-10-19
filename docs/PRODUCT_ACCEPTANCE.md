# Dosi-Tracker — Product Acceptance Specification & Traceable Matrix

> **Document Class:** Product Acceptance & Traceable Verification Standard  
> **Status:** Active Engineering Benchmark  
> **Date:** October 2026  
> **Repository:** `DosiBridge/Dosi-Tracker` (`main` branch)  
> **Supervisor / QA Operator:** PM / Independent QA  
> **Implementation Engineer:** Antigravity Pair Engineer  

---

## 1. Product Acceptance Vision & Operational Rules

Dosi-Tracker is a multi-tenant work activity and time-tracking system designed around the paradigm of **multi-workspace accounts** (Notion-like workspace model):
- A single user account can create, own, or belong to multiple workspaces.
- Each workspace is an isolated tenant with its own members, projects, policies, activity logs, screenshots, and billing tier.
- User intent strictly excludes building a document editor or changing the web framework (Next.js 16 App Router / React 19 is the canonical web UI).
- Privacy invariants are non-negotiable: the product is strictly non-stealth, non-surveillance, observer-only, with granular local user opt-in and truthful metric reporting.

### Evidence Classification Standard
Every backlog item and verification check in this specification is categorized under one of five strict evidence levels:
1. **[Verified]**: Independently verified locally by the PM / QA Operator with exact command outputs, exact commit SHA, and clean exit code `0`.
2. **[Worker-Reported]**: Unit/integration test suites executed and reported green by the implementation engineer in local environments; not yet independently repeated by the PM operator.
3. **[Source-Only]**: Code, configuration, or UI implementation exists in the repository, but runtime execution, end-to-end delivery, or native OS hooks remain unverified.
4. **[Blocked]**: Implementation depends on external prerequisites (e.g., Apple Developer ID certificate, Authenticode EV cert, physical hardware/OS toolchain).
5. **[Not Implemented]**: Feature or architectural component not yet present in codebase.

---

## 2. Finite Traceable Backlog & Requirements

### Priority 1: Multi-Workspace Account UX & Lifecycle
- **REQ-WS-01: Account Registration & Initial Workspace Creation**
  - *Requirement:* Single-step registration registers the user account and creates their initial owned workspace with default settings.
  - *Code:* `backend/.../WorkspaceAppService.cs` (`RegisterAsync`), `frontend/src/app/(auth)/register/page.tsx`.
  - *Evidence Level:* **[Worker-Reported]** (Backend `WorkspaceAppServiceTests`), **[Source-Only]** (Frontend UI).
- **REQ-WS-02: Multi-Workspace Membership & Listing**
  - *Requirement:* A user can view all active workspaces they belong to via `GET /api/app/workspaces/mine` and the dashboard workspace switcher.
  - *Code:* `backend/.../WorkspaceAppService.cs` (`GetMineAsync`), `frontend/src/components/layout/workspace-switcher.tsx`.
  - *Evidence Level:* **[Verified by PM @ 18db4d5]** (Frontend `workspace-switcher.live.test.tsx` 7 passed; Vitest exit 0).
- **REQ-WS-03: Dynamic Workspace Creation**
  - *Requirement:* An authenticated user can create an additional workspace at any time; the creator is automatically assigned the `Owner` role.
  - *Code:* `backend/.../WorkspaceAppService.cs` (`CreateAsync`), `frontend/src/app/(dashboard)/workspaces/page.tsx`.
  - *Evidence Level:* **[Verified by PM @ 18db4d5]** (Frontend `workspaces.test.tsx` 2 passed; Vitest exit 0).
- **REQ-WS-04: Workspace Switching & Context Switching**
  - *Requirement:* Switching active workspace updates cookies (`__Host-Dosi-Workspace`), headers (`X-Workspace-Id`), clears cache/queries, and swaps tenant roster.
  - *Code:* `frontend/src/components/session-provider.tsx`, `frontend/src/test/integration/tenant-switching.test.tsx`, `backend/.../CurrentWorkspace.cs`.
  - *Evidence Level:* **[Verified by PM @ 18db4d5]** (`tenant-switching.test.tsx` 8 passed; Vitest exit 0).
- **REQ-WS-05: Workspace Invitations Lifecycle**
  - *Requirement:* Admins/Owners invite members by email with single-use cryptographically secure tokens; invitee can inspect and accept; duplicate invites rejected.
  - *Code:* `backend/.../TeamAppService.cs` (`InviteMemberAsync`), `backend/.../WorkspaceAppService.cs` (`GetInvitationAsync`, `AcceptInvitationAsync`).
  - *Evidence Level:* **[Worker-Reported]** (`TeamAppServiceTests.cs`, `WorkspaceAppServiceTests.cs`).
- **REQ-WS-06: Role Governance & Last-Owner Orphaning Guard**
  - *Requirement:* Roles include Owner, Admin, Member. Self-service role modification is forbidden (`403 Tracker:CannotModifySelf`). Leaving a workspace is supported (`LeaveAsync`), but the last owner is strictly prevented from leaving or being demoted (`403 Tracker:CannotLeaveLastOwner`).
  - *Code:* `backend/.../WorkspaceAppService.cs` (`LeaveAsync`), `backend/.../TeamAppService.cs` (`UpdateWorkspaceRoleAsync`, `RemoveFromWorkspaceAsync`).
  - *Evidence Level:* **[Worker-Reported]** (EF Core `EfCoreWorkspaceAppServiceTests`, `TeamOwnerRulesTests`).
- **REQ-WS-07: Workspace Invitation Expiry, Revocation & Decline**
  - *Requirement:* Invitations must enforce cryptographic token expiry (e.g., 7 days). Inviters and admins can explicitly revoke pending invitations. Recipients can decline invitations without joining.
  - *Code:* `backend/.../WorkspaceAppService.cs`, `backend/.../TeamAppService.cs`.
  - *Evidence Level:* **[Not Implemented / Backlog Gap]** (Accept flow present; decline, revocation, and TTL expiry endpoints open).
- **REQ-WS-08: Safe Workspace Archive, Deletion & Settings Governance**
  - *Requirement:* Workspace Owners can archive or safely schedule deletion with confirmation and a retention grace period. Deletion must be blocked if active billing subscriptions or un-archived member data exist. Workspace settings changes must be audited.
  - *Code:* `backend/.../WorkspaceAppService.cs`.
  - *Evidence Level:* **[Not Implemented / Backlog Gap]** (Hard deletion / soft-archive lifecycle open).

---

## 3. Priority 2: Tenant & Workspace Isolation
- **REQ-ISO-01: Backend Entity Query Scoping**
  - *Requirement:* Every tenant-scoped entity implements `IWorkspaceScoped`. ABP global query filters automatically restrict reads/writes to `CurrentWorkspace.Id`.
  - *Code:* `backend/.../DosiTrackerDbContext.cs`, `backend/.../CurrentWorkspace.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`WorkspacePersistenceTests.cs`, `ActivityPersistenceTests.cs`).
- **REQ-ISO-02: Cross-Tenant Injection Guard**
  - *Requirement:* Ingestion and mutation endpoints verify that referenced projects, members, and resources belong to the caller's active workspace.
  - *Code:* `backend/.../ActivityAppService.cs` line 118, `backend/.../ProjectAppService.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`ProjectsAndTeamSteps.cs`, `MemberScopingTests.cs`).
- **REQ-ISO-03: Pay-Rate & Sensitive Metric Masking**
  - *Requirement:* Hourly pay rates and sensitive team finances are masked from non-owner/non-manager members.
  - *Code:* `backend/.../TeamAppService.cs`, `backend/.../ProjectMembershipManager.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`ProjectMemberRatePersistenceTests.cs`).
- **REQ-ISO-04: Client & Cache Partitioning**
  - *Requirement:* Frontend SWR / query cache keys and desktop offline SQLite databases partition data by tenant workspace ID. Switching workspaces flushes stale requests.
  - *Code:* `frontend/src/hooks/useApi.ts`, `frontend/src/lib/tenant-data.ts`, `clients/windows/src/worker.rs`.
  - *Evidence Level:* **[Verified by PM @ 18db4d5]** (Frontend `tenant-switching.test.tsx` 8 passed).
- **REQ-ISO-05: Authenticated HTTP/Browser Negative Tenant Isolation**
  - *Requirement:* Authenticated API requests and Next.js server actions must strictly enforce tenant boundaries against malicious parameter tampering (e.g., User from Workspace A attempting `GET /api/app/activity?workspaceId=WorkspaceB` or spoofing `X-Workspace-Id` header). Must return `403 Forbidden` and prevent data leakage.
  - *Code:* `backend/.../CurrentWorkspace.cs`, `frontend/src/middleware.ts`.
  - *Evidence Level:* **[Source-Only / Backlog Gap]** (Entity query filters exist in EF Core; comprehensive negative HTTP test suite open).

---

## 4. Priority 3: Tracker Product Core & Offline Queue
- **REQ-TRK-01: Time Block Recording & Idempotent Ingestion**
  - *Requirement:* Time blocks recorded in minute slices and uploaded to `POST /api/app/activity`. Idempotent unique index `(WorkspaceId, UserId, ClientActivityId)` prevents duplicate billing.
  - *Code:* `backend/.../ActivityAppService.cs`, `clients/windows/src/storage.rs`, `clients/macos/.../QueueDatabase.swift`.
  - *Evidence Level:* **[Worker-Reported]** (`ActivityAppServiceTests.cs`, `ActivityPersistenceTests.cs`).
- **REQ-TRK-02: Offline SQLite Queue with Poison-Row Parking**
  - *Requirement:* Client buffers activity when offline. Retries use exponential backoff with jitter. Malformed or rejected records park as `synced = 2` without blocking queue FIFO.
  - *Code:* `clients/windows/src/worker/uploader.rs`, `clients/windows/src/storage.rs`.
  - *Evidence Level:* **[Worker-Reported]** (Windows cargo tests: 345 passed).
- **REQ-TRK-03: Timezone Normalization & Productivity Math**
  - *Requirement:* Server normalizes client timestamps against IANA workspace timezones. Duration-weighted productivity calculations exclude idle/break intervals.
  - *Code:* `backend/.../AgentContextNormalizer.cs`, `frontend/src/lib/report-math.ts`.
  - *Evidence Level:* **[Worker-Reported]** (`ReportingAppServiceTests.cs`, `report-math.test.ts`).
- **REQ-TRK-04: Device Heartbeat & Presence**
  - *Requirement:* Periodic heartbeat (`POST /api/app/presence/heartbeat`) maintains online presence (150s timeout). Surplus/stale devices pruned.
  - *Code:* `backend/.../PresenceAppService.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`EfCorePresenceAppServiceTests.Heartbeat_Should_Accept_And_Store_Linux_Platform`).

---

## 5. Priority 4: Privacy & Surveillance Prevention Gate
- **REQ-PRV-01: Non-Stealth Desktop Agent & Visible User Controls**
  - *Requirement:* Desktop agent displays visible tray icon, active tracking banner, and prominent Start/Pause/Stop/Quit controls.
  - *Code:* `clients/windows/src/ui/mod.rs`, `clients/windows/src/tray.rs`.
  - *Evidence Level:* **[Worker-Reported]** (`tray::tests::tracking_view_offers_pause_and_shows_the_project`).
- **REQ-PRV-02: Windows Sensor Lifecycle Foundation**
  - *Requirement:* Injectable sensor driver decoupling. Construction (`Tracker::new()`) must NEVER install OS hooks or query foreground windows at startup. OS hooks (`NativeSensorDriver`) and usage samplers start only upon authorized tracking activation with affirmative local consent, and cleanly detach/stop upon pause, lock, away, workspace loss, sign-out, or quit. Message queue creation handshake (`PeekMessageW`), serial transitions under lock, bounded quit retries, and thread join guarantee synchronous detachment. Denied sensors must not attach handles or mutate health/key timing (`InputFilter::apply`).
  - *Code:* `clients/windows/src/tracking/lifecycle.rs`, `clients/windows/src/tracking/input.rs`, `clients/windows/src/tracking/usage.rs`, `clients/windows/src/worker.rs`.
  - *Evidence Level:* **[PM-Verified @ 8c1b201 for initial decoupling (6 tests)]**, **[Worker-Reported for Step 1C Hardening (89 synthetic tests)]**; native interactive desktop runtime, packaging, and signing remain **[Unverified]**.
- **REQ-PRV-03: Per-User Local Opt-In & Policy Intersection**
  - *Requirement:* Server policy intersection alone (`ProjectPolicy ∩ LocalConsent`) does NOT constitute local user consent. Captures and sensing occur only if:
    $$\text{Effective} = \text{ProjectPolicy} \cap \text{LocalConsent}$$
    Configuration defaults (`CapturePermissions`) do NOT constitute explicit consent. Production runtime defaults to `LocalConsent::Absent` and fails closed (all sensors disabled) until an explicit affirmative local choice is made. A dedicated `ConsentedPermissions` schema enforces strict default-deny: partial or malformed consent records never broaden sensor permissions or inherit permissive config defaults. Monotonic per-capability narrowing (`LocalConsent::narrow_to`) enforces immediate sensor withdrawal, capture buffer quiescence, and block policy updates BEFORE persistence can block on slow/interrupted I/O; unpersisted expansions are never granted prematurely. Actual native sensor driver stopping/narrowing (`machine.apply_sensor_policy`) is invoked strictly BEFORE reconciliation/finalization and potentially blocking record/persistence work (`Worker::record`, `machine.snapshot`, storage enqueue), ensuring slow storage or finalization cannot leave withdrawn native drivers running after consent withdrawal. Furthermore, held and historical block finalization is strictly clamped against current effective permissions and tracking lifecycle (`Worker::current_effective_perms`, `Tracker::snapshot` active gate): held blocks cannot acquire fresh screenshots, webcam captures, or foreground/process samples for withdrawn sensors upon finalization, while legitimately authorized historical time, keystroke, click, and usage slice metrics are preserved. If persistence fails, the runtime transitions to a fail-closed effective state (all sensors stopped), marks the session unsynced, blocks tracking resumption, and surfaces actionable restart durability warnings. Atomic replacement (`atomic_write_file` using ephemeral sibling staging, `sync_all`, and atomic rename) guarantees persistence safety without leaving corrupted or truncated files. Injected `ConsentStore` boundary (`FileConsentStore` / `MemoryConsentStore`) prevents tests from touching the real user's `%APPDATA%\DosiTracker\consent.json`.
  - *Code:* `clients/windows/src/config.rs`, `clients/windows/src/worker/session.rs`, `clients/windows/src/worker/blocks.rs`, `clients/windows/src/worker.rs`, `clients/windows/src/tracking/mod.rs`.
  - *Evidence Level:* **[Worker-Reported (Synthetic Integration)]** (`cargo test --bin dosi-tracker consent` 36 passed; 22-scenario worker consent suite passes, exit 0; pending independent PM verification).
- **REQ-PRV-04: Monotonic Counts Only — No Raw Keystrokes**
  - *Requirement:* Keyboard and mouse inputs are tallied as monotonic integer increments. Keycodes, text characters, clipboard contents, and inter-keystroke timing are discarded.
  - *Code:* `clients/windows/src/tracking/input.rs` (`InputFilter::apply`).
  - *Evidence Level:* **[Partial / Source Verified in Filter]**; note: accumulator gating tests prove monotonic count increment and drop behaviors in `InputCounter`, but do NOT prove absence of raw data across serialization, storage, or backend transmission. Full end-to-end audit remains open.
- **REQ-PRV-05: Truthful Metric Nullability — No Fabricated Zeros**
  - *Requirement:* Denied or unsupported metrics must remain `null` end-to-end (DB, API, frontend), distinguishing measured zero (`0`) from unavailable (`null`).
  - *Code:* `docs/CONSENT_CAPABILITY_AUDIT.md` §4, `backend/.../Activity.cs`.
  - *Evidence Level:* **[Not Implemented / Deferred to Reviewed Backend Phase]**.
- **REQ-PRV-06: Explicit Local Interactive Consent UI & Capability Opt-In**
  - *Requirement:* Dedicated desktop prompt on Timer view and management section in Settings explaining exactly what is collected (and what is never collected: raw keystrokes, typed text, clipboard, microphone, personal files) with granular toggles (input counts, active window, screenshots + blur, webcam). All default OFF upon fresh install/revocation via `ConsentedPermissions`. Fail-closed persistence in `consent.json` with actionable warnings if persistence fails. Capability matrix shows effective status (`Active`, `DeniedByUser`, `DisabledByProject`, `Disabled`).
  - *Code:* `clients/windows/src/ui/consent.rs`, `clients/windows/src/ui/timer.rs`, `clients/windows/src/ui/settings.rs`.
  - *Evidence Level:* **[Worker-Reported (View Model & Controller)]** (`ui::consent::tests` 6 passed, exit 0; `worker::tests::consent_*` 15 passed, exit 0; settings banner displays unsynced/persistence errors); native interactive display session rendering is reported as **[Unverified / Not run in automation]**.
- **REQ-PRV-07: Idle & Away Recovery Semantics & Privacy Tradeoff**
  - *Requirement:* Transitioning to `Away` state stops input hooks and foreground window samplers to preserve privacy. User return is detected via Windows system idle timer (`GetLastInputInfo`) without installing hooks.
  - *Tradeoff:* The system-level idle clock does not distinguish between physical typing and software jigglers while away; this tradeoff avoids persistent background input surveillance.
  - *Code:* `clients/windows/src/worker.rs`, `clients/windows/src/tracking/input.rs`.
  - *Evidence Level:* **[Worker-Reported]** (Synthetically verified via `sensor_lifecycle_stops_on_away_and_resumes_on_return`).

---

## 6. Priority 5: Native Platform Clients
- **REQ-CLI-01: Windows Desktop Agent (Rust)**
  - *Requirement:* Rust agent with Win32 low-level hooks, DPAPI credential security, xcap screen capture, MediaFoundation webcam, and local SQLite.
  - *Evidence Level:* **[Worker-Reported]** (Focused synthetic test suites pass; native interactive session, Authenticode code signing, and NSIS installation unverified).
- **REQ-CLI-02: macOS Desktop Agent (Swift)**
  - *Requirement:* Swift native app with ScreenCaptureKit, Keychain, and Quartz event tap.
  - *Evidence Level:* **[Source-Only / Blocked]** (Source present; execution blocked on host lacking macOS toolchain/hardware; signing/notarization open).
- **REQ-CLI-03: Linux Desktop Agent (Rust Target)**
  - *Requirement:* Greenfield crate supporting X11 (`_NET_ACTIVE_WINDOW`) and Wayland (`xdg-desktop-portal` ScreenCast). Strictly no root, sudo, evdev, or uinput escalation.
  - *Evidence Level:* **[Not Implemented / Greenfield Target]**.

---

## 7. Priority 6: Production Engineering & Operability
- **REQ-OPS-01: Automated Frontend Quality Gates**
  - *Requirement:* Zero TypeScript errors, ESLint <= 45 warnings, 100% passing Vitest suite, zero dependency architecture violations.
  - *Evidence Level:* **[Verified by PM @ 18db4d5]** (`npm run typecheck` exit 0; focused 3-file Vitest suite 17 tests exit 0). Full 1,965-test suite is **[Worker-Reported (Historical)]**.
- **REQ-OPS-02: Accessibility Invariants (Programmatic)**
  - *Requirement:* Programmatic axe component checks and machine contrast calculations ($\ge 3:1$ for graphics/cards).
  - *Evidence Level:* **[Worker-Reported (Historical)]** (53 a11y tests passed in jsdom). Real-browser assistive audits remain open.
- **REQ-OPS-03: Secrets Hygiene & Isolation**
  - *Requirement:* No plaintext secrets in repository or client builds. Test suites run against ephemeral SQLite in-memory databases only.
  - *Evidence Level:* **[Worker-Reported (Historical)]** (Checked across git tree).

---

## 8. Priority 7: Data Retention & Compliance Operations
- **REQ-DAT-01: Activity & Screenshot Retention Cleanup Worker**
  - *Requirement:* Background worker enforcing workspace retention policies (e.g. 30/90 days for screenshots, 1 year for activity summaries). Prunes both blob storage objects and database records.
  - *Code:* `backend/.../BlobStoring`, `backend/.../ActivityAppService.cs`.
  - *Evidence Level:* **[Not Implemented / Backlog Gap]** (Settings exist in host config; periodic pruning background worker not implemented).
- **REQ-DAT-02: Data Export, Extraction & Disaster Recovery**
  - *Requirement:* Workspace Owners can request full data export (CSV/JSON activity summaries + zip archive of screenshots) for compliance/GDPR. Export jobs execute asynchronously with signed download URLs. Automated database backup and restore procedures documented.
  - *Evidence Level:* **[Not Implemented / Backlog Gap]**.

---

## 9. Traceable Verification Matrix

| Req ID | Component | Implementation File(s) | Concrete Test Command | Evidence Status | Gaps / Outstanding Items |
|---|---|---|---|:---:|---|
| **REQ-WS-02** | Frontend | `workspace-switcher.tsx` | `npx vitest run src/components/layout/workspace-switcher.live.test.tsx` | **[Verified by PM @ 18db4d5]** (7 passed) | Real browser Playwright E2E journey open |
| **REQ-WS-03** | Frontend | `workspaces/page.tsx` | `npx vitest run src/test/pages/workspaces.test.tsx` | **[Verified by PM @ 18db4d5]** (2 passed) | Server error toast handling |
| **REQ-WS-04** | Frontend | `session-provider.tsx` | `npx vitest run src/test/integration/tenant-switching.test.tsx` | **[Verified by PM @ 18db4d5]** (8 passed) | Browser navigation URL sync |
| **REQ-WS-05** | Backend | `TeamAppService.cs` | `dotnet test --filter TeamMembershipLifecycleTests` | **[Worker-Reported]** | Real SMTP invitation email delivery |
| **REQ-WS-06** | Backend | `WorkspaceAppService.cs` | `dotnet test --filter EfCoreWorkspaceAppServiceTests` | **[Worker-Reported]** | Frontend leave-workspace confirmation modal |
| **REQ-WS-07** | Backend | `WorkspaceAppService.cs` | Endpoints for decline, revoke, TTL | **[Not Implemented]** | Token expiry & decline endpoints |
| **REQ-WS-08** | Backend | `WorkspaceAppService.cs` | Soft-delete & archive workflows | **[Not Implemented]** | Grace period & active billing checks |
| **REQ-ISO-01**| Backend | `DosiTrackerDbContext.cs`| `dotnet test --filter WorkspacePersistenceTests` | **[Worker-Reported]** | Multi-node tenant cache invalidation |
| **REQ-ISO-04**| Frontend | `tenant-switching.test.tsx`| `npx vitest run src/test/integration/tenant-switching.test.tsx` | **[Verified by PM @ 18db4d5]** (8 passed) | Browser multi-tab synchronization |
| **REQ-ISO-05**| Backend / Web | `CurrentWorkspace.cs`, `middleware.ts` | Negative cross-workspace HTTP tests | **[Not Implemented]** | Header spoofing negative integration tests |
| **REQ-TRK-01**| Backend | `ActivityAppService.cs` | `dotnet test --filter ActivityPersistenceTests` | **[Worker-Reported]** | S3/R2 direct multipart upload |
| **REQ-TRK-02**| Windows | `uploader.rs`, `storage.rs`| `cargo test --bin dosi-tracker worker` | **[Worker-Reported]** | Interactive offline network simulation |
| **REQ-TRK-04**| Backend | `PresenceAppService.cs` | `dotnet test --filter EfCorePresenceAppServiceTests` | **[Worker-Reported]** | WebSocket / SignalR push live presence |
| **REQ-PRV-02**| Windows | `lifecycle.rs`, `input.rs`, `usage.rs`, `worker.rs` | `cargo test --bin dosi-tracker worker::tests::sensor_lifecycle` | **[Worker-Reported (Synthetic Driver Verified)]** (6 passed) | Native OS interactive Win32 desktop unverified |
| **REQ-PRV-03**| Windows | `config.rs`, `session.rs`, `worker.rs`, `blocks.rs`, `tracking` | `cargo test --bin dosi-tracker consent` | **[Worker-Reported (Synthetic Integration)]** (36 passed) | Native Win32 desktop display unverified; pending PM replication |
| **REQ-PRV-04**| Windows | `input.rs` | `cargo test --bin dosi-tracker tracking::input::tests::inactive_counter_drops_input_events_and_keeps_logs_empty` | **[Verified by PM @ 18db4d5]** (1 passed, exit 0) | Real hook hardware injection ignored in test |
| **REQ-PRV-05**| Cross | `Activity.cs`, `types.ts` | Schema Migration + `npm test` | **[Not Implemented]** | Deferred to reviewed backend migration phase |
| **REQ-PRV-06**| Windows UI | `consent.rs`, `timer.rs`, `settings.rs` | `cargo test --bin dosi-tracker ui::consent::tests` | **[Worker-Reported (View Model & Controller)]** (6 passed) | Interactive GUI verification not run in automation |
| **REQ-PRV-07**| Windows | `worker.rs`, `input.rs` | `cargo test --bin dosi-tracker worker::tests::sensor_lifecycle_stops_on_away_and_resumes_on_return` | **[Worker-Reported]** (1 passed, exit 0) | Idle clock doesn't distinguish typing vs jiggler |
| **REQ-CLI-01**| Windows | `clients/windows/src` | `cargo test --bin dosi-tracker` (focused) | **[Worker-Reported]** | Authenticode code signing & NSIS packaging |
| **REQ-CLI-02**| macOS | `clients/macos/Sources` | `swift test` (macOS host) | **[Blocked]** | Requires macOS hardware & Apple Notarization |
| **REQ-CLI-03**| Linux | `clients/linux` (greenfield) | Crate build + tests | **[Not Implemented]** | Greenfield implementation |
| **REQ-OPS-01**| Frontend | Full frontend suite | `npm run typecheck` | **[Verified by PM @ 18db4d5]** (exit 0) | Full CI pipeline setup |
| **REQ-OPS-02**| Frontend | `analytics-charts.tsx` | `npm run test:a11y` | **[Worker-Reported (Historical)]** (53 passed) | Manual screen reader / zoom audit |
| **REQ-DAT-01**| Backend | Background worker | Retention pruning test suite | **[Not Implemented]** | Periodic pruning job not implemented |
| **REQ-DAT-02**| Backend | Export service | Asynchronous export & backup suite | **[Not Implemented]** | Async bulk zip & signed URL generator |
