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
1. **[Verified]**: Independently verified locally by the PM / QA Operator with exact command outputs and clean exit code `0`.
2. **[Worker-Reported]**: Unit/integration test suites executed and reported green by the implementation engineer in local environments; not yet independently run by operator.
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
  - *Evidence Level:* **[Verified]** (Frontend `workspace-switcher.live.test.tsx` 7 passed; backend unit tests).
- **REQ-WS-03: Dynamic Workspace Creation**
  - *Requirement:* An authenticated user can create an additional workspace at any time; the creator is automatically assigned the `Owner` role.
  - *Code:* `backend/.../WorkspaceAppService.cs` (`CreateAsync`), `frontend/src/app/(dashboard)/workspaces/page.tsx`.
  - *Evidence Level:* **[Verified]** (Frontend `workspace-switcher.live.test.tsx` creates workspace and plan).
- **REQ-WS-04: Workspace Switching & Context Switching**
  - *Requirement:* Switching active workspace updates cookies (`__Host-Dosi-Workspace`), headers (`X-Workspace-Id`), clears cache/queries, and swaps tenant roster.
  - *Code:* `frontend/src/components/session-provider.tsx`, `frontend/src/test/integration/tenant-switching.test.tsx`, `backend/.../CurrentWorkspace.cs`.
  - *Evidence Level:* **[Verified]** (`tenant-switching.test.tsx` 8 passed).
- **REQ-WS-05: Workspace Invitations Lifecycle**
  - *Requirement:* Admins/Owners invite members by email with single-use cryptographically secure tokens; invitee can inspect and accept/decline; duplicate invites rejected.
  - *Code:* `backend/.../TeamAppService.cs` (`InviteMemberAsync`), `backend/.../WorkspaceAppService.cs` (`GetInvitationAsync`, `AcceptInvitationAsync`).
  - *Evidence Level:* **[Worker-Reported]** (`TeamAppServiceTests.cs`, `WorkspaceAppServiceTests.cs`).
- **REQ-WS-06: Role Governance & Last-Owner Orphaning Guard**
  - *Requirement:* Roles include Owner, Admin, Member. Self-service role modification is forbidden (`403 Tracker:CannotModifySelf`). Leaving a workspace is supported (`LeaveAsync`), but the last owner is strictly prevented from leaving or being demoted (`403 Tracker:CannotLeaveLastOwner`).
  - *Code:* `backend/.../WorkspaceAppService.cs` (`LeaveAsync`), `backend/.../TeamAppService.cs` (`UpdateWorkspaceRoleAsync`, `RemoveFromWorkspaceAsync`).
  - *Evidence Level:* **[Worker-Reported]** (EF Core `EfCoreWorkspaceAppServiceTests`, `TeamOwnerRulesTests`).

---

### Priority 2: Tenant & Workspace Isolation
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
  - *Evidence Level:* **[Verified]** (Frontend `tenant-switching.test.tsx`).

---

### Priority 3: Tracker Product Core & Offline Queue
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
  - *Evidence Level:* **[Worker-Reported]** (`ReportingAppServiceTests.cs`), **[Verified]** (`report-math.test.ts`).
- **REQ-TRK-04: Device Heartbeat & Presence**
  - *Requirement:* Periodic heartbeat (`POST /api/app/presence/heartbeat`) maintains online presence (150s timeout). Surplus/stale devices pruned.
  - *Code:* `backend/.../PresenceAppService.cs`.
  - *Evidence Level:* **[Verified]** (`EfCorePresenceAppServiceTests.Heartbeat_Should_Accept_And_Store_Linux_Platform`).

---

### Priority 4: Privacy & Surveillance Prevention Gate
- **REQ-PRV-01: Non-Stealth Desktop Agent & Visible User Controls**
  - *Requirement:* Desktop agent displays visible tray icon, active tracking banner, and prominent Start/Pause/Stop/Quit controls.
  - *Code:* `clients/windows/src/ui/mod.rs`, `clients/windows/src/tray.rs`.
  - *Evidence Level:* **[Worker-Reported]** (`tray::tests::tracking_view_offers_pause_and_shows_the_project`).
- **REQ-PRV-02: Sensor Lifecycle & Pause/Stop Detachment**
  - *Requirement:* Input hooks and foreground window samplers must NOT run at application launch or capture data when paused/stopped/signed out.
  - *Code:* `clients/windows/src/tracking/usage.rs`, `clients/windows/src/tracking/input.rs`, `clients/windows/src/worker.rs`.
  - *Evidence Level:* **[P0 In Progress]** (Active tracking gate implemented in Phase 3).
- **REQ-PRV-03: Per-User Local Opt-In & Policy Intersection**
  - *Requirement:* Admin project policies on the server (`allow_screenshot`, `allow_keyboard`, etc.) do not substitute for local user consent. Captures occur only if:
    $$\text{Effective} = \text{ProjectPolicy} \cap \text{LocalConsent}$$
  - *Code:* `clients/windows/src/worker/session.rs`, `docs/CONSENT_CAPABILITY_AUDIT.md`.
  - *Evidence Level:* **[Source-Only]** (Policy intersection present; GUI toggles open).
- **REQ-PRV-04: Monotonic Counts Only — No Raw Keystrokes**
  - *Requirement:* Keyboard and mouse inputs are tallied as monotonic integer increments. Keycodes, text characters, clipboard contents, and inter-keystroke timing are discarded.
  - *Code:* `clients/windows/src/tracking/input.rs` (`InputFilter::apply`).
  - *Evidence Level:* **[Worker-Reported]** (Cargo tests in `tracking::input::tests`).
- **REQ-PRV-05: Truthful Metric Nullability — No Fabricated Zeros**
  - *Requirement:* Denied or unsupported metrics must remain `null` end-to-end (DB, API, frontend), distinguishing measured zero (`0`) from unavailable (`null`).
  - *Code:* `docs/CONSENT_CAPABILITY_AUDIT.md` §4, `backend/.../Activity.cs`.
  - *Evidence Level:* **[Not Implemented / Phase 3 Scope]**.

---

### Priority 5: Native Platform Clients
- **REQ-CLI-01: Windows Desktop Agent (Rust)**
  - *Requirement:* Rust agent with Win32 low-level hooks, DPAPI credential security, xcap screen capture, MediaFoundation webcam, and local SQLite.
  - *Evidence Level:* **[Worker-Reported]** (345 unit tests pass; native interactive session and code signing open).
- **REQ-CLI-02: macOS Desktop Agent (Swift)**
  - *Requirement:* Swift native app with ScreenCaptureKit, Keychain, and Quartz event tap.
  - *Evidence Level:* **[Source-Only / Blocked]** (Source present; execution blocked on host lacking macOS toolchain/hardware; signing/notarization open).
- **REQ-CLI-03: Linux Desktop Agent (Rust Target)**
  - *Requirement:* Greenfield crate supporting X11 (`_NET_ACTIVE_WINDOW`) and Wayland (`xdg-desktop-portal` ScreenCast). Strictly no root, sudo, evdev, or uinput escalation.
  - *Evidence Level:* **[Not Implemented / Greenfield Target]**.

---

### Priority 6: Production Engineering & Operability
- **REQ-OPS-01: Automated Frontend Quality Gates**
  - *Requirement:* Zero TypeScript errors, ESLint <= 45 warnings, 100% passing Vitest suite, zero dependency architecture violations.
  - *Evidence Level:* **[Verified]** (Operator verified clean exit 0 on commit `59403f8`).
- **REQ-OPS-02: Accessibility Invariants (Programmatic)**
  - *Requirement:* Programmatic axe component checks and machine contrast calculations ($\ge 3:1$ for graphics/cards).
  - *Evidence Level:* **[Verified]** (53 a11y tests passed, operator verified). Note: real-browser assistive audits remain open.
- **REQ-OPS-03: Secrets Hygiene & Isolation**
  - *Requirement:* No plaintext secrets in repository or client builds. Test suites run against ephemeral SQLite in-memory databases only.
  - *Evidence Level:* **[Verified]** (Checked across git tree).

---

## 3. Traceable Verification Matrix

| Req ID | Component | Implementation File(s) | Concrete Test Command | Evidence Status | Gaps / Outstanding Items |
|---|---|---|---|:---:|---|
| **REQ-WS-02** | Frontend | `workspace-switcher.tsx` | `npx vitest run src/components/layout/workspace-switcher.live.test.tsx` | **[Verified]** (7 passed) | Real browser Playwright E2E journey open |
| **REQ-WS-04** | Frontend | `session-provider.tsx` | `npx vitest run src/test/integration/tenant-switching.test.tsx` | **[Verified]** (8 passed) | Browser navigation URL sync |
| **REQ-WS-03** | Frontend | `workspaces/page.tsx` | `npx vitest run src/test/pages/workspaces.test.tsx` | **[Verified]** (2 passed) | Server error toast handling |
| **REQ-WS-05** | Backend | `TeamAppService.cs` | `dotnet test --filter TeamMembershipLifecycleTests` | **[Worker-Reported]** | Real SMTP invitation email delivery |
| **REQ-WS-06** | Backend | `WorkspaceAppService.cs` | `dotnet test --filter EfCoreWorkspaceAppServiceTests` | **[Worker-Reported]** | Frontend leave-workspace confirmation modal |
| **REQ-ISO-01**| Backend | `DosiTrackerDbContext.cs`| `dotnet test --filter WorkspacePersistenceTests` | **[Worker-Reported]** | Multi-node tenant cache invalidation |
| **REQ-TRK-01**| Backend | `ActivityAppService.cs` | `dotnet test --filter ActivityPersistenceTests` | **[Worker-Reported]** | S3/R2 direct multipart upload |
| **REQ-TRK-02**| Windows | `uploader.rs`, `storage.rs`| `cargo test --bin dosi-tracker worker` | **[Worker-Reported]** | Interactive offline network simulation |
| **REQ-TRK-04**| Backend | `PresenceAppService.cs` | `dotnet test --filter EfCorePresenceAppServiceTests` | **[Verified]** | WebSocket / SignalR push live presence |
| **REQ-PRV-02**| Windows | `usage.rs`, `input.rs` | `cargo test --bin dosi-tracker tracking` | **[P0 In Progress]** | Synchronous hook detachment on pause |
| **REQ-PRV-05**| Cross | `Activity.cs`, `types.ts` | Schema Migration + `npm test` | **[Not Implemented]** | DB column nullability migration |
| **REQ-CLI-01**| Windows | `clients/windows/src` | `cargo test --bin dosi-tracker` | **[Worker-Reported]** | Authenticode code signing & NSIS packaging |
| **REQ-CLI-02**| macOS | `clients/macos/Sources` | `swift test` (macOS host) | **[Blocked]** | Requires macOS hardware & Apple Notarization |
| **REQ-CLI-03**| Linux | `clients/linux` (greenfield) | Crate build + tests | **[Not Implemented]** | Greenfield implementation |
| **REQ-OPS-01**| Frontend | Full frontend suite | `npm run validate` | **[Verified]** (1,965 passed) | Full CI pipeline setup |
| **REQ-OPS-02**| Frontend | `analytics-charts.tsx` | `npm run test:a11y` | **[Verified]** (53 passed) | Manual screen reader / zoom audit |
