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
  - *Requirement:* Roles include Owner, Admin, Member. Self-service role modification is forbidden (`403 Tracker:CannotModifySelf`). Leaving a workspace is supported (`LeaveAsync`), but the last owner is strictly prevented from leaving or being demoted (`403 Tracker:CannotLeaveLastOwner`). Under concurrent leaves, demotions, or removals, per-workspace ownership synchronization ensures at least one active owner remains in the workspace and rejects losing concurrent callers with domain errors (`Tracker:LastOwnerCannotLeave` or `Tracker:RoleNotAllowed`).
  - *Code:* `backend/.../WorkspaceAppService.cs` (`LeaveAsync`), `backend/.../TeamAppService.cs` (`UpdateWorkspaceRoleAsync`, `RemoveFromWorkspaceAsync`), `backend/.../Workspaces/WorkspaceMembershipManager.cs` (`LockOwnersAsync`, `ChangeRoleAsync`, `RemoveAsync`).
  - *Evidence Level:* **[Worker-Reported (Concurrent / EF Core SQLite WAL Integration)]** (`ConcurrentOwnerProtectionTests` 9 passed, exit 0, 9.2s). Features: bounded modeled interleaving simulation of unsynchronized race (orphaning risk simulation, not production execution claim); sequential multi-owner leaves until last owner; concurrent leaves with identical-user unrelated workspace data retention; concurrent mutual demotion; reachable mixed leave/demotion with distinct withdrawal targets; non-global lock key isolation across unrelated workspaces; deterministic lock exclusivity proof; deterministic competitor queued observation at actual provider boundary (`LockWaitEntered`, `QueuedWaiters >= 1`) before release; pre-release SQLite nontransactional persisted-state ordering verification; bounded 10s cancellation/waits; guaranteed cleanup on `DisposeAsync` via `try...finally`; and concrete detection/rejection of deliberately non-exclusive or bypassed-lock variants (`MaxConcurrentHolders == 1`). **Fixture Limitation & Accurate Designation:** SQLite provider requires `AddAlwaysDisableUnitOfWorkTransaction()` due to single-writer locking constraints (`SQLite Error 5: 'database is locked'` under concurrent transactions), so assertions reflect SQLite nontransactional persisted-state ordering rather than transactional commit visibility. Full provider-backed multi-connection transactional commit proof and multi-node distributed locking infrastructure (Redis) remain unresolved acceptance gaps. PM independent execution pending.
- **REQ-WS-07: Workspace Invitation Expiry, Revocation & Decline**
  - *Requirement:* Invitations must enforce cryptographic token expiry (e.g., 7 days). Inviters and admins can explicitly revoke pending invitations. Authenticated recipients can decline invitations without joining or requiring prior membership.
  - *Code:*
    - Expiry & TTL: `backend/.../WorkspaceInvitation.cs` (`ExpiresAt`, `IsUsable(Clock.Now)`), `TeamAppService.cs` (`Clock.Now.AddDays(...)`) — pre-existing in baseline source.
    - Admin Revocation: `backend/.../TeamAppService.cs` (`ITeamAppService.DeleteInvitationAsync`, scoped lookup by invitation ID + `CurrentWorkspace.Id`) — pre-existing in baseline source.
    - Recipient Decline: `backend/.../WorkspaceAppService.cs` (`DeclineInvitationAsync`), `backend/.../Controllers/WorkspacesController.cs` (`POST /api/app/invitations/{token}/decline`) — authenticated recipient verified via identity email `invitation.IsFor(user.Email)` with schema-free pending row invalidation. Implementation executes `GetUsableInvitationAsync` before checking `CurrentUser.Id`; email lookup reads `user.Email` without verifying `EmailConfirmed`; multi-threaded concurrency between decline and accept/register remains unverified.
  - *Evidence Level:* **[Worker-Reported (Component / EF Core In-Memory)]** (`EfCoreInvitationDeclineTests` 10 passed, `ContractRoutesTests` 40 passed [route reflection only], `api-contract.test.ts` 30 passed [Verified by PM @ 3bebd70]). Real end-to-end authenticated HTTP/browser acceptance and multi-threaded concurrency remain open.
- **REQ-WS-08: Reversible Owner-Controlled Workspace Archival & Restoration**
  - *Requirement:* Reversible owner-controlled workspace archival and unarchival (`ArchiveAsync`, `RestoreAsync`). Strictly restricted to active owners of the target workspace; outsiders, managers, members, and inactive former owners are denied (`403 Forbidden` / `AbpAuthorizationException`) with unchanged database state. Ambient workspace headers are untrusted. Archival and restore are strictly idempotent and preserve audit properties (`ArchivedTime`, `ArchivedBy`). Archival preserves all workspace data, memberships, invitations, and active subscriptions without deletion or cancellation; archival does NOT cancel billing or mutate subscriptions. Archived state is visibly projected in listings (`MyWorkspaceDto.IsArchived`, `WorkspaceDto`). Normal active-workspace resolution (`WorkspaceResolutionMiddleware`, `WorkspaceRequiredFilter`) rejects requests targeted at archived workspaces with `403 Forbidden` (`Tracker:WorkspaceArchived`), while unrelated workspace access for dual-members remains unaffected. Invitation acceptance and new user registration into archived workspaces are rejected before any account/membership creation or database mutation. Restore is accessible only to a verified active owner via explicit workspace-not-required route (`POST /api/app/workspaces/{id}/restore`). Destructive hard deletion, retention grace-period policies, and automated billing cancellation are explicitly deferred.
  - *Code:* `backend/src/Dosi.Tracker.Domain/Workspaces/Workspace.cs` (`IsArchived`, `Archive`, `Restore`), `backend/src/Dosi.Tracker.Domain/Workspaces/WorkspaceManager.cs` (`ArchiveAsync`, `RestoreAsync`, `IsArchivedAsync`), `backend/src/Dosi.Tracker.Application/SaaS/WorkspaceAppService.cs` (`ArchiveAsync`, `RestoreAsync`, `GetMineAsync`, `RegisterWithInvitationAsync`, `AcceptAsync`), `backend/src/Dosi.Tracker.HttpApi/Controllers/WorkspacesController.cs` (`POST {id}/archive`, `POST {id}/restore`), `backend/src/Dosi.Tracker.HttpApi.Host/Workspaces/WorkspaceResolution.cs` (`WorkspaceResolutionMiddleware`, `WorkspaceRequiredFilter`), `backend/src/Dosi.Tracker.Domain/Workspaces/WorkspaceRolePermissionValueProvider.cs`, `backend/src/Dosi.Tracker.EntityFrameworkCore/Migrations/20261020200944_AddWorkspaceArchivalFields.cs`, `backend/src/Dosi.Tracker.EntityFrameworkCore/Migrations/TrackerDbContextModelSnapshot.cs`.
  - *Evidence Level:* **[Worker-Reported (Hosted ASP.NET Core TestServer MVC, EF Core Migrations & SQLite WAL)]** (`WorkspaceArchivalLifecycleTests` 9 passed, exit 0, 14.7s; `WorkspaceArchivalMigrationTests` 10 passed, exit 0, 7.8s). Features: owner success with visible listing projection; outsider/manager/member/inactive former owner denial with unchanged persisted state; dual-member unaffected access to unrelated active workspace; ordinary requests denied with `403 Tracker:WorkspaceArchived`; verified owner restore re-enabling normal access; idempotent repeated archive and restore preserving audit timestamps; non-destructive preservation of memberships, projects, and subscriptions; invitation acceptance and registration rejection before account/membership creation; smallest additive backward-compatible migration (`20261020200944_AddWorkspaceArchivalFields`) with non-null `IsArchived` (default `false`) and nullable audit fields; zero pending model changes verified (`dotnet ef migrations has-pending-model-changes` exit 0); PostgreSQL DDL verification (`ALTER TABLE "AppWorkspaces" ADD ...`); direct assertions on tracked `TrackerDbContextModelSnapshot` and migration `TargetModel` protecting against stale snapshot; strengthened operation-level baseline upgrade proving surviving membership, project, and subscription sentinels; and tracked `IMigrator` upgrade (`Initial` -> `AddWorkspaceArchivalFields`) with `__EFMigrationsHistory` tracking on disposable SQLite.
  - *Evidence Limits & Remaining Gaps:* The archival fixture disables UOW transactions due to SQLite locking constraints (`AddAlwaysDisableUnitOfWorkTransaction()`); registration rejection in tests is verified via direct service invocation (`RegisterWithInvitationAsync`), not hosted anonymous HTTP. Disposable SQLite baseline-to-current upgrades (operation-level and tracked migrator) prove upgrade logic, but do NOT replace execution on a real PostgreSQL cluster; no live user/production database was modified. Rollback consequence: dropping archive columns via Down() removes archival fields and loses archival state/audit history, returning previously archived workspaces to unarchived status (not a lossless rollback of archival status), though underlying workspaces, memberships, projects, and subscriptions are preserved and not deleted. Destructive permanent deletion, retention grace-period enforcement, automated billing subscription cancellation/downgrades, and frontend UI workspace archive/restore management actions remain deferred acceptance gaps. PM independent execution pending.

---

## 3. Priority 2: Tenant & Workspace Isolation
- **REQ-ISO-01: Backend Entity Query Scoping**
  - *Requirement:* Every tenant-scoped entity implements `IWorkspaceScoped`. ABP global query filters automatically restrict reads/writes to `CurrentWorkspace.Id`. Empty GUID workspace header (`00000000-0000-0000-0000-000000000000`) is refused with `400 Bad Request (Tracker:WorkspaceRequired)` as malformed header validation hardening before DB lookups.
  - *Code:* `backend/.../TrackerDbContext.cs`, `backend/.../CurrentWorkspace.cs`, `backend/.../WorkspaceResolution.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`WorkspacePersistenceTests.cs`, `ActivityPersistenceTests.cs`, `CrossWorkspaceIsolationTests.cs` [EF query filter scoping], `WorkspaceResolutionTests.An_Empty_Guid_Header_Should_Be_Refused_With_400`). Independent PM execution remains pending.
- **REQ-ISO-02: Cross-Tenant Injection Guard**
  - *Requirement:* Ingestion and mutation endpoints verify that referenced projects, members, and resources belong to the caller's active workspace.
  - *Code:* `backend/.../ActivityAppService.cs` line 118, `backend/.../ProjectAppService.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`ProjectsAndTeamSteps.cs`, `MemberScopingTests.cs`, `CrossWorkspaceIsolationTests.Caller_In_Workspace_A_Cannot_Upload_Activity_To_Project_In_Workspace_B`).
- **REQ-ISO-03: Pay-Rate & Sensitive Metric Masking**
  - *Requirement:* Hourly pay rates and sensitive team finances are masked from non-owner/non-manager members.
  - *Code:* `backend/.../TeamAppService.cs`, `backend/.../ProjectMembershipManager.cs`.
  - *Evidence Level:* **[Worker-Reported]** (`ProjectMemberRatePersistenceTests.cs`).
- **REQ-ISO-04: Client & Cache Partitioning**
  - *Requirement:* Frontend SWR / query cache keys and desktop offline SQLite databases partition data by tenant workspace ID. Switching workspaces flushes stale requests.
  - *Code:* `frontend/src/hooks/useApi.ts`, `frontend/src/lib/tenant-data.ts`, `clients/windows/src/worker.rs`.
  - *Evidence Level:* **[Verified by PM @ 18db4d5]** (Frontend `tenant-switching.test.tsx` 8 passed).
- **REQ-ISO-05: Authenticated HTTP/Browser Negative Tenant Isolation**
  - *Requirement:* Authenticated API requests and Next.js server actions must strictly enforce tenant boundaries against malicious parameter tampering (e.g., User from Workspace A attempting to read, mutate, delete, or list Workspace B resources or spoofing `X-Workspace-Id` header). Must return `403 Forbidden` and prevent data leakage. Dual-membership context switching must preserve legitimate permissions per workspace and forbid cross-workspace leakage.
  - *Code:* `backend/.../CurrentWorkspace.cs`, `backend/.../WorkspaceResolution.cs`, `backend/.../WorkspaceResolutionIntegrationTests.cs`, `backend/.../Security/WorkspaceHttpAuthorizationTests.cs`, `backend/.../Applications/EfCoreWorkspaceHttpAuthorizationTests.cs`, `frontend/src/middleware.ts`.
  - *Evidence Level:* **[Worker-Reported (Hosted ASP.NET Core TestServer MVC Aggregate Authorization & Isolation Harness)]** (`EfCoreWorkspaceHttpAuthorizationTests` 9 passed, exit 0, 16.8s; historical provenance: earlier in-process handler 7 passed, exit 0). Exercises actual ASP.NET Core pipeline (`UseRouting`, `UseAuthentication`, `UseUnitOfWork`, `UseWorkspaceResolution`, `UseAuthorization`, `endpoints.MapControllers()`), conventional MVC controller routing, production `ProjectAppService`, real ABP aggregate authorization (`AbpAuthorizationService` + `PermissionChecker` + `WorkspaceRolePermissionValueProvider`), and SQLite `:memory:` database persistence/scoping. Covers unauthenticated (401), empty GUID header (400), missing header action filter interception (400), cross-tenant spoofing (403 + 0 DB writes), cross-workspace resource read/mutation rejection (404/403 + target unchanged), role-denied mutation by aggregate checker (403 + 0 DB writes), dual-member context isolation/role governance (permitted in A, denied in B), inactive member denial (403), and legitimate permitted mutation (200 OK + verified SQLite persistence + cross-workspace query invisibility). **Separate Open Acceptance Gaps:** Real browser cookie/session flows (`__Host-Dosi-Workspace`), Next.js server actions, and hosted production token issuer (OpenIddict validation) remain separate acceptance gaps even though synthetic-auth HTTP passes. PM independent execution approval pending.

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
    Configuration defaults (`CapturePermissions`) do NOT constitute explicit consent. Production runtime defaults to `LocalConsent::Absent` and fails closed (all sensors disabled) until an explicit affirmative local choice is made. A dedicated `ConsentedPermissions` schema enforces strict default-deny: partial or malformed consent records never broaden sensor permissions or inherit permissive config defaults. Monotonic per-capability narrowing (`LocalConsent::narrow_to`) enforces immediate sensor withdrawal, capture buffer quiescence, and block policy updates BEFORE persistence can block on slow/interrupted I/O; unpersisted expansions are never granted prematurely. Actual native sensor driver stopping/narrowing (`machine.apply_sensor_policy`) is invoked strictly BEFORE reconciliation/finalization and potentially blocking record/persistence work (`Worker::record`, `machine.snapshot`, storage enqueue), ensuring slow storage or finalization cannot leave withdrawn native drivers running after consent withdrawal. At the `Tracker` acquisition boundary, an explicit `AcquisitionProvider` seam decouples heavy/privacy-sensitive queries (`capture_screen`, `snapshot` webcam, `current_window`, `running_programs`) from native OS calls, tracking explicit effective authorization (`TrackerAuth`) updated monotonically under write lock. Every entry point executes an atomic pre-admission check immediately before invoking the provider (no single upfront snapshot reused across steps), never holds locks across blocking provider calls (guaranteeing `apply_sensor_policy` is never blocked), and discards any in-flight capture upon provider return if revoked while executing. Process enumeration (`running_programs`) is queried strictly based on active running_programs authorization without inappropriate dependency on the active-window usage driver. Furthermore, held and historical block finalization is strictly clamped against current effective permissions and tracking lifecycle (`Worker::current_effective_perms`, `Tracker::snapshot` active gate): held blocks cannot acquire fresh screenshots, webcam captures, or foreground/process samples for withdrawn sensors upon finalization, while legitimately authorized historical time, keystroke, click, and usage slice metrics are preserved. If persistence fails, the runtime transitions to a fail-closed effective state (all sensors stopped), marks the session unsynced, blocks tracking resumption, and surfaces actionable restart durability warnings. Atomic replacement (`atomic_write_file` using ephemeral sibling staging, `sync_all`, and atomic rename) guarantees persistence safety without leaving corrupted or truncated files. Injected `ConsentStore` boundary (`FileConsentStore` / `MemoryConsentStore`) prevents tests from touching the real user's `%APPDATA%\DosiTracker\consent.json`.
  - *Code:* `clients/windows/src/config.rs`, `clients/windows/src/worker/session.rs`, `clients/windows/src/worker/blocks.rs`, `clients/windows/src/worker.rs`, `clients/windows/src/tracking/mod.rs`.
  - *Evidence Level:* **[Worker-Reported (Synthetic Integration)]** (`cargo test --bin dosi-tracker consent` 57 passed; 22-scenario worker consent suite + 21 isolated Tracker acquisition regressions pass, exit 0; pending independent PM verification).
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
  - *Requirement:* Explicit workspace-scoped age-based capture retention lifecycle service and background worker. Prunes both blob storage objects and database records for screenshots/thumbnails/webcam captures while strictly preserving activity time summaries, blocks, and unrelated workspaces.
  - *Code:* `backend/src/Dosi.Tracker.Domain/Activities/Retention/CaptureRetentionService.cs`, `CaptureRetentionPolicy.cs`, `CaptureRetentionOptions.cs`, `CaptureRetentionWorker.cs`, `TrackerHttpApiHostModule.cs`.
  - *Evidence Level:* **[Worker-Reported (Isolated SQLite & Scripted Blob Provider)]** (16 passed, exit 0). Safe first slice implemented with disabled-by-default configuration (`IsEnabled = false`), bounded batching across distinct synthetic activities, deterministic cutoff, strict writer-compatible canonical blob key validation (`activities/{activityId:D}/{kind}{ext}`), Activity repository workspace ownership check failing closed without deleting foreign blobs or metadata, retriable storage failure handling, and idempotent DB retry convergence (with modeled missing-blob recovery). Activity/time summary pruning, per-workspace owner settings UI, and production operational rollout remain deferred.
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
| **REQ-WS-06** | Backend | `WorkspaceAppService.cs`, `WorkspaceMembershipManager.cs`, `TeamAppService.cs` | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~ConcurrentOwnerProtectionTests"` (9 passed, exit 0) | **[Worker-Reported (Concurrent SQLite WAL)]** | Modeled simulation of race; per-workspace lock protects last owner under concurrent leaves & demotions; distinct withdrawal targets tested; same-user cross-workspace retention verified; deterministic lock boundary queuing assertion before release (`LockWaitEntered`, `QueuedWaiters >= 1`); pre-release SQLite nontransactional persisted-state ordering assertion; guaranteed cleanup on `DisposeAsync`; non-exclusive lock variant rejection verified (`MaxConcurrentHolders == 1`); multi-node Redis lock, transactional commit proof, and PM independent execution pending |
| **REQ-WS-07** | Backend / API | `WorkspaceAppService.cs`, `WorkspacesController.cs`, `TeamAppService.cs` | `dotnet test --filter InvitationDeclineTests` (10 passed), `ContractRoutesTests` (40 passed [reflection only]), `api-contract.test.ts` (30 passed [Verified by PM @ 3bebd70]) | **[Worker-Reported (Component)]** | Expiry & admin revoke pre-exist in source; decline implemented & verified in-memory; authenticated HTTP/browser acceptance and multi-threaded concurrency unverified |
| **REQ-WS-08** | Backend | `Workspace.cs`, `WorkspaceManager.cs`, `WorkspaceAppService.cs`, `WorkspacesController.cs`, `20261020200944_AddWorkspaceArchivalFields.cs` | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceArchivalLifecycleTests"` (9 passed, exit 0) & `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceArchivalMigrationTests"` (10 passed, exit 0) | **[Worker-Reported (Hosted MVC TestServer & EF Core Migrations)]** | Reversible owner-only archival/restore verified; audit fields preserved; listings reflect state; tracked snapshot and target model asserted; operation-level sentinels preserved; tracked IMigrator baseline-to-current upgrade verified on disposable SQLite; real PostgreSQL cluster execution, UOW transaction disabling limit, direct-service registration limit, destructive deletion, grace-period, automated billing cancellation, and frontend UI remain deferred |
| **REQ-ISO-01**| Backend | `DosiTrackerDbContext.cs`| `dotnet test --filter WorkspacePersistenceTests` | **[Worker-Reported]** | Multi-node tenant cache invalidation |
| **REQ-ISO-04**| Frontend | `tenant-switching.test.tsx`| `npx vitest run src/test/integration/tenant-switching.test.tsx` | **[Verified by PM @ 18db4d5]** (8 passed) | Browser multi-tab synchronization |
| **REQ-ISO-05**| Backend / Web | `WorkspaceResolution.cs`, `ProjectAppService.cs`, `WorkspaceRolePermissionValueProvider.cs` | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~EfCoreWorkspaceHttpAuthorizationTests"` (9 passed, exit 0) | **[Worker-Reported (Hosted MVC TestServer & In-Memory SQLite)]** | Genuine TestServer with conventional MVC controller routing, production middleware, action filter & aggregate permissions. Browser cookies, Next.js server actions, OpenIddict issuer & concurrency open; PM execution pending |
| **REQ-TRK-01**| Backend | `ActivityAppService.cs` | `dotnet test --filter ActivityPersistenceTests` | **[Worker-Reported]** | S3/R2 direct multipart upload |
| **REQ-TRK-02**| Windows | `uploader.rs`, `storage.rs`| `cargo test --bin dosi-tracker worker` | **[Worker-Reported]** | Interactive offline network simulation |
| **REQ-TRK-04**| Backend | `PresenceAppService.cs` | `dotnet test --filter EfCorePresenceAppServiceTests` | **[Worker-Reported]** | WebSocket / SignalR push live presence |
| **REQ-PRV-02**| Windows | `lifecycle.rs`, `input.rs`, `usage.rs`, `worker.rs` | `cargo test --bin dosi-tracker worker::tests::sensor_lifecycle` | **[Worker-Reported (Synthetic Driver Verified)]** (6 passed) | Native OS interactive Win32 desktop unverified |
| **REQ-PRV-03**| Windows | `config.rs`, `session.rs`, `worker.rs`, `blocks.rs`, `tracking` | `cargo test --bin dosi-tracker consent` | **[Worker-Reported (Synthetic Integration)]** (48 passed) | Native Win32 desktop display unverified; pending PM replication |
| **REQ-PRV-04**| Windows | `input.rs` | `cargo test --bin dosi-tracker tracking::input::tests::inactive_counter_drops_input_events_and_keeps_logs_empty` | **[Verified by PM @ 18db4d5]** (1 passed, exit 0) | Real hook hardware injection ignored in test |
| **REQ-PRV-05**| Cross | `Activity.cs`, `types.ts` | Schema Migration + `npm test` | **[Not Implemented]** | Deferred to reviewed backend migration phase |
| **REQ-PRV-06**| Windows UI | `consent.rs`, `timer.rs`, `settings.rs` | `cargo test --bin dosi-tracker ui::consent::tests` | **[Worker-Reported (View Model & Controller)]** (6 passed) | Interactive GUI verification not run in automation |
| **REQ-PRV-07**| Windows | `worker.rs`, `input.rs` | `cargo test --bin dosi-tracker worker::tests::sensor_lifecycle_stops_on_away_and_resumes_on_return` | **[Worker-Reported]** (1 passed, exit 0) | Idle clock doesn't distinguish typing vs jiggler |
| **REQ-CLI-01**| Windows | `clients/windows/src` | `cargo test --bin dosi-tracker` (focused) | **[Worker-Reported]** | Authenticode code signing & NSIS packaging |
| **REQ-CLI-02**| macOS | `clients/macos/Sources` | `swift test` (macOS host) | **[Blocked]** | Requires macOS hardware & Apple Notarization |
| **REQ-CLI-03**| Linux | `clients/linux` (greenfield) | Crate build + tests | **[Not Implemented]** | Greenfield implementation |
| **REQ-OPS-01**| Frontend | Full frontend suite | `npm run typecheck` | **[Verified by PM @ 18db4d5]** (exit 0) | Full CI pipeline setup |
| **REQ-OPS-02**| Frontend | `analytics-charts.tsx` | `npm run test:a11y` | **[Worker-Reported (Historical)]** (53 passed) | Manual screen reader / zoom audit |
| **REQ-DAT-01**| Backend | `CaptureRetentionService.cs`, `CaptureRetentionWorker.cs`, `CaptureRetentionPolicy.cs` | `dotnet test test/Dosi.Tracker.EntityFrameworkCore.Tests/Dosi.Tracker.EntityFrameworkCore.Tests.csproj --filter "FullyQualifiedName~WorkspaceCaptureRetentionTests"` (16 passed, exit 0) | **[Worker-Reported (Isolated SQLite & Scripted In-Memory Blob Provider)]** | First slice: explicit workspace age capture cleanup (SCREEN/THUMB/WEBCAM), bounded batches across distinct activities, deterministic cutoff, strict writer-compatible canonical key check, Activity repository ownership check failing closed without deleting foreign blobs/metadata, retriable storage failure handling, crash convergence (modeled missing blob), disabled-by-default registration (`IsEnabled = false`). Remaining gaps: Activity time summary retention (1yr), per-workspace owner settings UI, operational rollout/approval |
| **REQ-DAT-02**| Backend | Export service | Asynchronous export & backup suite | **[Not Implemented]** | Async bulk zip & signed URL generator |
