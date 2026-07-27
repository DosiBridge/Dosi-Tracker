# Dosi-Tracker — Full-Stack Code Audit

> **Update (same day):** the P0/P1 roadmap below has since been implemented — see
> **§8 Implementation record** at the end of this document for what changed and how it is tested.

**Audit date:** 2026-07-21
**Scope:** backend (`backend/`), frontend (`frontend/`), desktop agents (`clients/windows`, `clients/macos`) — code-level only.
**Method:** file-by-file review of all product code, cross-tier contract matching, plus new automated unit/integration tests (SQLite in-memory) to prove/disprove behavior.

---

## Verdict

**The application is NOT production grade today.** The three tiers do not form a working pipeline:

- The **frontend** is ~95% a mock/demo (localStorage session, deterministic fake data); only login/register, project list/create, activity list, and my-profile hit the real API.
- The **desktop agents** call three endpoints, of which **two do not exist** on the backend (`POST /api/app/account/login`, `GET /api/app/project/my-projects`), and the third had contract bugs that made real uploads fail (fixed below).
- The **backend** had **zero authorization** on every product endpoint and silently drops the screenshot/webcam payloads it accepts.

This audit fixed the highest-impact, unambiguous defects (see §5) and added a 34-test suite (see §6). The remaining work to production is listed as a prioritized roadmap (§7).

---

## 1. Backend findings

### 1.1 API surface (auto-API at `/api/app/*`)

| Service | Endpoints | State before audit |
|---|---|---|
| `ActivityAppService` | `POST/GET /api/app/activity` | No auth; anonymous writes attributed to `Guid.Empty` user; no time-range validation; unsorted paging; screenshots/webcam **silently dropped** |
| `ProjectAppService` | full CRUD `/api/app/project` | No auth; no `IntervalMinutes` bounds |
| `TeamAppService` | `/api/app/team/*` | No auth; add anyone to any project, any free-form role, duplicates allowed, no project-existence check |
| `NotificationAppService` | `/api/app/notification/*` | No auth; **IDOR** — anyone could mark any user's notification read |
| `BillingAppService` | `GET /api/app/billing/invoices` | No auth; unsorted paging; **nothing ever creates invoices** |
| `WorkspaceAppService` | `/api/app/workspace/*` | No auth; **plans are never seeded, subscriptions never created** — always empty/null |

### 1.2 Structural findings (still open)

- **Screenshot pipeline is a stub.** `CreateActivityDto` accepts `ScreenshotPngBase64`/`WebcamJpgBase64`, but `CreateAsync` never reads them. The `Screenshot` entity, its table, and the configured S3/R2 blob container are entirely unused. No retrieval endpoint exists.
- **Billing/SaaS is read-only scaffolding.** No seeding or creation path for `Plan`, `Subscription`, `Invoice`; no trial-expiry (`TrialEndsAt` never set), no seat enforcement against `Plan.MaxSeats`, no invoice generation.
- **Notifications have no producer.** Nothing ever inserts a `Notification`.
- **Missing endpoints a tracker product needs:** `my-projects` (per-user projects + capture permissions for agents), agent login/session, activity filtering by user/project/date, reporting/aggregation (daily/weekly, productivity rollups), screenshot list/download, seat management.
- **Idempotency race:** `CreateAsync`'s check-then-insert can throw an uncaught unique-index violation on concurrent duplicate `ClientActivityId` (should catch and return the existing row).
- **Permissions are empty.** `TrackerPermissions`/`TrackerPermissionDefinitionProvider` define no permission names — role-granular authorization (owner/admin/worker/client) cannot be expressed yet.
- **Secrets committed:** `appsettings.json` carries the DB password, `AuthServer:CertificatePassPhrase`, `StringEncryption:DefaultPassPhrase`; admin seed password `1q2w3E*` in `TrackerConsts`. `App:DisablePII=false` enables PII logging of security artifacts.
- **Project capture-permission flags don't exist.** Agents expect `allowScreenshot/allowWebcam/allowKeyboard/allowMouse/allowActiveWindow/allowRunningPrograms` per project; `Project` has no such fields.

## 2. Frontend findings

### 2.1 Real API vs mock, per page

| Page | Data source |
|---|---|
| `/login` | **Real** (`/connect/token`, `/api/account/register`) — was broken (§5), now fixed |
| `/projects` | **Real** list/create — but member avatars resolve via mock users |
| `/activities` | **Real** list — but user/project names resolve via mock lookups; screen previews are mock |
| `/projects/[id]` | **Mock** — real project GUIDs → "Project not found"; Edit/Archive buttons dead |
| `/dashboard`, `/monitor`, `/timesheet`, `/reports/*`, `/team`, `/billing`, `/settings`, `/host/*` | **Mock** (deterministic fake data / localStorage) |

### 2.2 Key issues (still open)

- **No route protection.** No `middleware.ts`; anyone lands on `/dashboard` as a mock "owner". `RoleGuard` checks mock roles, not authentication.
- **Authorization is client-side only** (`lib/roles.ts`, `RoleGuard`, host gate) — decorative until the backend enforces permissions.
- Any real logged-in user is hardcoded to role `"admin"` (`session-provider.tsx:96`).
- **Business rules live only in the frontend, on mock data** — these must move server-side or they will never be authoritative:
  - tracked minutes = activity-count × `project.intervalMinutes` (repeated in 6+ report components)
  - payroll: `amount = round(hours × rate)`; blended project cost at hardcoded `$48/hr`
  - productivity split magic constants (`0.35` unproductive share, `0.08` idle) — implemented **twice, divergently** (`productivity-report.tsx` vs `tenant-data.deriveSplit`)
  - MRR = `pricePerUser × seats`; seats = owner+admin+worker
  - UTC/local timezone handling is inconsistent; user `timezone` fields unused
- Dead/demo controls: billing actions, team invite, settings save, "Forgot password?", schedule report, project Edit/Archive, host plan price edit (in-memory only).
- Duplicated API layers: `lib/api-client.ts` fully unreferenced (superseded by `hooks/useApi.ts`); `putApi` unused; no 401/refresh handling anywhere.

## 3. Desktop agents (Windows Rust / macOS Swift)

Well-structured, low-footprint capture loops (event-driven input counting, offline-first SQLite queue, per-interval screenshot/webcam). **Neither agent can work end-to-end against today's backend:**

| # | Issue | Impact |
|---|---|---|
| 1 | `POST /api/app/account/login` **does not exist** (backend auth is OpenIddict `/connect/token`, form-encoded, different response shape) | Agents can never authenticate |
| 2 | `GET /api/app/project/my-projects` **does not exist**; agents also expect `allow*` capture-permission fields absent from `ProjectDto`; agents parse a bare JSON array while ABP returns `{items, totalCount}` | Agents fall back to `projectId="local"` |
| 3 | Fallback `projectId="local"` cannot bind to the backend's `Guid ProjectId` → 400 forever; both sync loops `break` on first error | **Poison-message queue: one bad row blocks all later uploads permanently** |
| 4 | macOS `Activity` model **omits `clientActivityId`** → would serialize as `Guid.Empty` server-side; with the old backend every macOS upload after the first was silently swallowed by the idempotency dedup | Silent data loss (server now rejects empty ids — §5) |
| 5 | macOS uses `JSONDecoder.dateDecodingStrategy = .iso8601`, which does **not** parse fractional seconds; ABP emits fractional-second timestamps | `AuthSession`/date decoding fails on macOS |
| 6 | Bearer token expiry is never checked/refreshed on either agent | Long-running agents go 401 and never recover |
| 7 | Default `api_base_url = https://localhost:44300` mismatches the Host's `https://localhost:44342`; dev TLS is self-signed (rustls/ATS will refuse) | Out-of-the-box dev connection fails |
| 8 | Windows `running_programs()` is a stub (returns only the foreground window); autostart/tray/service packaging absent on both | Feature gaps |
| 9 | Synced rows (including base64 screenshots) are never purged from the local SQLite queue | Unbounded disk growth |

*(Toolchain note: Rust/Swift toolchains are unavailable on this machine — agents were reviewed statically; the Windows crate graph in `Cargo.toml` is coherent (tokio/reqwest/rusqlite/xcap/rdev/nokhwa), and the Swift package targets macOS 14+ for `SCScreenshotManager`.)*

## 4. Cross-tier contract matrix

| Contract | Client(s) | Backend | Status |
|---|---|---|---|
| `POST /connect/token` (password grant, `Tracker_App`) | frontend | OpenIddict, seeded client incl. password grant | ✅ works |
| `POST /api/account/register` | frontend | ABP Account module | ✅ works |
| `GET/POST /api/app/project` | frontend | `ProjectAppService` | ✅ works (now `[Authorize]`d) |
| `GET /api/app/activity` | frontend | `ActivityAppService` | ✅ works (now ordered + `[Authorize]`d) |
| `POST /api/app/account/login` | both agents | **missing** | ❌ |
| `GET /api/app/project/my-projects` | both agents | **missing** | ❌ |
| `POST /api/app/activity` | both agents | exists | ⚠️ works only after agents fix items 3–5 in §3; screenshots still dropped |

## 5. Fixes applied in this audit

**Backend** (all verified by the new test suite):

1. `[Authorize]` added to all six app services (`Activity`, `Project`, `Team`, `Notification`, `Billing`, `Workspace`) — endpoints now require authentication.
2. `Activity` aggregate now enforces invariants: `ClientActivityId != Guid.Empty` (`Tracker:ActivityClientIdRequired`) and `EndedAt > StartedAt` (`Tracker:ActivityTimeRangeInvalid`); JSON columns default to `"[]"`. New error codes in `TrackerDomainErrorCodes`.
3. `ActivityAppService`: `CurrentUser.GetId()` (throws if unauthenticated) instead of silently attributing to `Guid.Empty`; `GetListAsync` ordered by `StartedAt` descending (Skip/Take without OrderBy was nondeterministic).
4. `NotificationAppService.MarkAsReadAsync`: ownership-scoped query — foreign notifications now behave as not-found (IDOR closed).
5. `TeamAppService.AddMemberAsync`: project must exist; duplicate membership rejected (`Tracker:DuplicateProjectMember`); `CreateProjectMemberDto` gained `[Required]`/`[MaxLength(64)]`.
6. `CreateUpdateProjectDto.IntervalMinutes` gained `[Range(5, 60)]`, matching the interval both agents clamp to.
7. `BillingAppService.GetInvoicesAsync` ordered by `DueDate` descending.
8. **Schema bug found by the new integration tests:** `AppActivities.Description` and `AppProjects.Description` were `NOT NULL`, while agents send `description: null` on every activity → every real agent upload would have been a 500. Made nullable end-to-end (entity, DTOs) + EF migration `20260721042542_MakeDescriptionColumnsOptional`.

**Frontend:**

9. `src/app/login/page.tsx` **did not compile** (a half-finished refactor removed all form state and the signup `<form>` opening tag). Rebuilt: full state restored, controlled inputs, proper `PlanId` import — `tsc --noEmit` and `next build` now pass.
10. Logout now clears the real bearer token (`logoutApi()` wired into `session-provider.logout`) — previously the ABP JWT survived sign-out in localStorage.

## 6. Tests added (all green)

Run with: `dotnet test backend/Dosi.Tracker.slnx`

- **Unit — `Dosi.Tracker.Domain.Tests` (10 tests):** `Activity` invariants (valid block, empty client id, inverted/zero time range), `Project`/`ProjectMember` defaults, `Invoice` starts `pending`, `Subscription` starts `trialing`, `Plan` pricing fields, `Notification` starts unread.
- **Integration — `Dosi.Tracker.EntityFrameworkCore.Tests` (24 tests, in-memory SQLite, full ABP DI + UoW pipeline):**
  - Activity: create-for-current-user, idempotent retry (one row), empty-client-id rejected, bad time range rejected, paging ordered by `StartedAt` desc.
  - Project: create/get, list contains created, update, soft-delete hides from reads, interval outside 5–60 rejected.
  - Team: add+list members, duplicate rejected, unknown project rejected, remove.
  - Notification: only own notifications listed, mark-own-read, foreign mark-read rejected (IDOR guard).
  - Workspace: no subscription → null, tenant subscription returned, seeded plans returned.
  - Billing: invoice paging ordered by due date desc.

The abstract test classes live in `Dosi.Tracker.Application.Tests` and are bound to the SQLite EF Core module in `Dosi.Tracker.EntityFrameworkCore.Tests` (repo's established pattern), so the same suites can later be re-bound to a PostgreSQL test module.

## 7. Roadmap to production (prioritized)

**P0 — make the pipeline real**
1. Agent-facing API: `GET /api/app/project/my-projects` (per-user projects + capture-permission flags — add the `allow*` fields to `Project`), and agent auth via OpenIddict password/device flow (drop the fictional `/api/app/account/login` from both agents, or implement it as a thin token-endpoint proxy).
2. Screenshot ingestion: decode base64 → blob container (S3/R2 already configured) → `Screenshot` row; add list/download endpoints with authorization; then remove base64 from the activity row path.
3. Agent fixes: macOS `clientActivityId` + fractional-second ISO8601 parsing; both agents parse `{items:[…]}`; never enqueue with non-GUID project id; skip/park poison rows instead of `break`; token refresh.
4. Frontend: route protection middleware on `dosi-token`, real role from ABP (`/api/abp/application-configuration`), stop resolving real data through mock lookups, wire `/projects/[id]` to `GET /api/app/project/{id}`.

**P1 — business logic server-side**
5. Move the reporting math (tracked time, payroll, productivity split, MRR/seats) into backend reporting endpoints; the two divergent frontend implementations of the productivity split must be unified server-side.
6. Define permissions in `TrackerPermissionDefinitionProvider` (per-feature, per-role) and apply policy names to the CRUD service + methods; scope `GetListAsync(activities)` to the caller (worker sees own, admin sees tenant).
7. Billing engine: seed `Plans`, create `Subscription` on tenant registration, trial expiry job, seat enforcement, invoice generation.
8. Catch the idempotency unique-violation race in `ActivityAppService.CreateAsync` and return the existing row.

**P2 — hardening/ops**
9. Move secrets to user-secrets/env (`ConnectionStrings`, passphrases), rotate the committed ones, set `App:DisablePII=true`, change the seeded admin password flow.
10. Agents: purge synced queue rows, EnumWindows-based running-programs, tray/service packaging, auto-update.
11. CI (build + tests + `next build`), docker-compose, and a PostgreSQL-bound test module for provider-parity tests.

---

## 8. Implementation record (production-grade pass, 2026-07-21)

Everything in P0 and P1 of §7, plus most of P2, has been implemented and verified.

### Backend (all covered by the test suite — **52 tests green**: 10 domain unit + 42 SQLite integration)

| Area | What was built |
|---|---|
| **Capture permissions** | `Project` gained `AllowScreenshot/Webcam/Keyboard/Mouse/ActiveWindow/RunningPrograms` (privacy-first defaults: webcam off), exposed through `ProjectDto`/`CreateUpdateProjectDto`; migration `AddCaptureSettingsAndScreenshotKinds` (existing rows default to the same policy). |
| **Agent API** | `GET /api/app/project/my-projects` — active projects the caller is a member of, as the plain array both agents parse. Project creators are auto-enrolled as `Admin` members. `Archive`/`Unarchive` endpoints added. |
| **Screenshot pipeline** | `CreateActivityDto` captures are now decoded (validated base64, 10 MB cap), written to the ABP blob container (S3/R2 when configured, database provider otherwise) and recorded as `Screenshot` rows (`Kind` = screen/webcam, 1:N per activity). Retrieval: `GET /api/app/activity/screenshots?activityId=`, plus a binary `GET /api/app/activity/screenshot/{id}/content` controller. Ownership enforced (non-ViewAll callers only see their own captures). |
| **Permissions** | `TrackerPermissions` defined (Projects.Create/Edit/Delete/Archive, Activities.ViewAll, Team.Manage, Billing, Reporting) with localized display names; applied to CRUD policies and management endpoints. Member-scoped endpoints stay authenticated-only so workers can track out of the box; tenant admins receive all permissions via ABP's admin-role seeding. |
| **Activity queries** | `GetActivitiesInput` filters (project, user, date range); non-ViewAll callers are always scoped to their own rows. Idempotency insert race now caught (separate UoW + re-query) instead of surfacing a 500. |
| **Workspace signup** | `POST /api/app/workspace/register` (anonymous): validates the plan, creates the tenant, seeds its admin user via the Identity data seeder, and starts the subscription (`trialing` + `TrialEndsAt` on paid plans, `active` on Free). |
| **Billing engine** | `PlanDataSeedContributor` seeds Free ($0/3 seats), Starter ($6/25/14d), Business ($12/100/14d). `InvoiceGenerator` domain service bills `PricePerUser × distinct seats` at most once per calendar month (min. one seat). `TrialExpiryWorker` (hourly) marks expired trials `past_due`; `InvoiceGenerationWorker` (daily) sweeps billable subscriptions across tenants. Seat limits enforced in `AddMemberAsync` (existing seat-holders can join more projects). |
| **Notifications** | Producer added: adding a member notifies that user ("You were added to project …"). |
| **Reporting** | `GET /api/app/reporting/summary` — totals, per-user and per-project rollups using **real block durations** and duration-weighted productivity (replacing the frontend's `count × interval` and double-implemented split constants as the source of truth). |
| **Ops** | `App:DisablePII` now `true`; S3 blob config only applied when credentials exist (database provider fallback); background workers registered in the Host. |

### Desktop agents (both)

- Auth rewritten to the real **OpenIddict** flow (`POST /connect/token`, password grant, client `Tracker_App`, scope `Tracker`) with cached tokens and early re-login before expiry; 401 drops the token and re-authenticates.
- `my-projects` consumed with the new capture-permission fields; effective permissions = server ∧ local config.
- **No more `"local"` fallback:** without a resolved project the agent skips capture and retries next interval (a made-up project id could never sync and used to poison the queue).
- Sync loop distinguishes **permanent rejections** (parked with the server's reason, `synced = 2`) from transient failures (retried) — one bad payload can no longer block the queue. Synced rows are **deleted** so the local DB no longer grows unboundedly.
- Default `api_base_url` corrected to `https://localhost:44342`.
- **Windows:** `running_programs` upgraded from a single-window stub to a real `EnumWindows` enumeration (visible, titled, deduped by process).
- **macOS:** `clientActivityId` added to the payload (was silently dropping all uploads after the first); ISO-8601 decoding now handles .NET's 7-digit fractional seconds.
- *Note: Rust/Swift toolchains are not available on the audit machine; the CI workflow added below compile-checks both agents on every push.*

### Frontend (typecheck + production `next build` green)

- **Route protection:** `src/middleware.ts` gates every route on a `dosi-token` session cookie (set/cleared by `loginApi`/`logoutApi`; the JWT itself never leaves localStorage) with `/login` redirects both ways.
- **Real SaaS signup:** "Create workspace" now calls `/api/app/workspace/register` and logs into the fresh tenant; sign-in has a workspace (tenant) field which is passed as `__tenant` to the token endpoint.
- **Real roles:** session hydration reads `/api/abp/application-configuration` — host users map to `host`, tenant `admin` role to `owner`, everyone else to `worker` (the hardcoded `"admin"` override is gone).
- **Project detail page** now loads the real project, members, and activities (`/api/app/project/{id}`, `/api/app/team/project-members`, `/api/app/activity?ProjectId=`), computes week/month/total from real durations, and has a working **Archive/Unarchive** button; the demo dataset remains only as a fallback when no session exists.
- **Billing page** shows the real subscription (plan, trial countdown from `TrialEndsAt`, past-due state) and real invoices when a session exists.

### CI / deployment

- `.github/workflows/ci.yml`: backend build + full test suite, frontend typecheck + lint + build, `cargo check` (Windows agent) and `swift build` (macOS agent).
- `docker-compose.yml`: PostgreSQL + DbMigrator + API host + web dashboard, secrets overridable via environment.

### Remaining (known, deliberate)

- Committed dev secrets (DB password, cert/encryption passphrases, seed admin password) still need rotation and a secret store for real deployments; the compose file exposes env overrides for all of them.
- Plan changes/payment methods in the UI remain demo stubs (no payment provider integrated).
- The mock demo dataset still renders the reports/monitor/timesheet pages; the authoritative numbers now come from `/api/app/reporting/summary`, and wiring every report component to it is UI work that can proceed page by page.
- Screenshot blur pipeline and agent tray/service packaging are not implemented.

---

## 9. Adversarial review round (multi-agent, post-implementation)

After the production pass, a 3-lens adversarial review (independent Opus reviewers over the full diff, instructed to find only defects with a concrete failure path) surfaced 10 real issues — all now fixed and re-verified (**57 backend tests green; frontend tsc + build green**).

### Critical

1. **Activities page showed nothing for real users.** The page filtered live activity through the shared date presets, which pivot on the frozen demo clock (`NOW = 2026-07-14`); every real row (timestamped "now") fell outside the window. Fixed: the activities page converts the selected preset into explicit **real-clock** bounds (`liveEffectiveFilters`).
2. **Windows agent didn't compile against the pinned `windows = 0.58`.** `OpenProcess(.., false, ..)` and `GetModuleFileNameExW(Some(handle), None, ..)` use a newer ergonomic API; 0.58 uses the `Param`-trait signatures. Fixed to the 0.58-correct forms (`BOOL(0)`, bare `HANDLE`, `HMODULE::default()`). (CI `cargo check` is the compile gate.)
3. **macOS webcam hung the whole agent.** `AVCaptureVideoDataOutput` retains its delegate weakly; the `FrameHandler` was only held by a local that released as soon as setup returned, tearing down the session before any frame arrived — the `await` never returned, freezing the capture loop. Fixed: the capturer holds the handler strongly for the capture's lifetime, resumes exactly once, and a 10-second watchdog guarantees the await always completes.

### Major

4. **Pay-rate leak.** `GetProjectMembersAsync` (only `[Authorize]`) returned every member's `HourlyRate` to any authenticated user, and accepted any `projectId`. Fixed: non-managers must be a member of the project and never receive rate data (`HourlyRate` zeroed unless the caller has `Team.Manage`).
5. **Screenshot lost + masked as success.** The activity committed in its own transaction *before* captures were validated; an oversized/invalid capture then threw after commit, and the agent's idempotent retry returned 200 while the screenshot was gone forever. Fixed: captures are decoded/validated up front and persisted **atomically** with the activity in one unit of work; an invalid/oversized capture is skipped (logged) so the time block is never lost.
6. **Payroll report always showed $0** for real data — same frozen-`NOW` query-window bug as (1). Fixed with a real-clock `liveRangeFor`.
7. **Host sign-in dead-ended.** Login always pushed `/dashboard`, where the host role is blocked and the sidebar is empty. Fixed: a host sign-in (no workspace) now lands on `/host`.
8. **Monitor undercounted out-of-hours work.** Live blocks were clamped to an 08:00–20:00 UTC window and dropped outside it, corrupting tracked-time/productivity KPIs for other timezones. Fixed: live blocks map to their true minute-of-day within the full 24 h (accurate KPIs; only cosmetic clipping remains on the fixed visual axis).

### Minor

9. **Activity injection.** `CreateAsync` didn't verify the caller belongs to `input.ProjectId`. Fixed: membership is now required (also blocks cross-tenant project ids).
10. **Invoice sweep fragility.** `InvoiceGenerationWorker` billed all tenants in one transaction, so a single tenant's failure rolled back everyone and aborted the run. Fixed: one unit of work per tenant with per-tenant try/catch-and-continue.

New/updated tests cover the membership requirement, graceful capture-skip, invite flow, hourly rates, and the daily-series endpoint.

---

## 10. End-to-end verification against a live stack (2026-07-22)

The pipeline was finally run for real: PostgreSQL 18 → DbMigrator → HttpApi.Host → Windows agent.

### Confirmed working

| Step | Result |
|---|---|
| **Migrations** | All 6 applied to PostgreSQL (first non-SQLite run). `Allow*` capture flags, `Screenshot.Kind` and `ProjectMember.HourlyRate` all present. |
| **Seeding** | `PlanDataSeedContributor` seeded Free/Starter/Business with correct pricing and trial days. |
| **Host startup** | Health check green; **`TrialExpiryWorker` and `InvoiceGenerationWorker` both started** — validating Host-module DI that the test suite cannot reach. |
| **Workspace signup** | `POST /api/app/workspace/register` (anonymous) created tenant `acme`, seeded its admin, and started a **trialing** subscription with `TrialEndsAt` +14 days. |
| **Rate limiting** | The token endpoint returned **429** after 10 requests/minute — the limiter works as designed. |
| **Project + membership** | Project created with capture flags; `GET /api/app/project/my-projects` returned it, proving creator auto-membership. |
| **Agent round trip** | `sign in → resolve project → capture → queue → upload`, all in the agent's own log. |
| **Screenshot pipeline** | Activity row with the correct tenant/project, real `ActiveWindowsJson` (`WindowsTerminal.exe` + title), a `Screenshot` row (`kind=screen`, `image/png`, 362,970 bytes) and the **same 362,970 bytes persisted in blob storage**. This path was a complete stub before the audit. |
| **Capture timing** | A capture 1 s after sign-in recorded 0.02 min — the missed-tick/clamping fix behaving correctly instead of claiming a full interval. |

### Defect found only by running it

**`__tenant` sent in the POST body is silently ignored.** ABP's tenant resolvers read
route/query/header/cookie — never the form body. A tenant sign-in therefore fell
through to the **host** tenant: the returned JWT had `email=admin@abp.io` and no
`tenantid`. Depending on passwords this either fails confusingly or authenticates
as the wrong tenant.

Fixed in all three consumers by moving it to a query parameter:
- `frontend/src/hooks/useApi.ts` — `loginApi` builds `/connect/token?__tenant=…`
- `clients/windows/src/api.rs` — plus a new **Workspace** field in the sign-in
  window, persisted (DPAPI) with the credentials
- `clients/macos/.../ApiClient.swift` — `tenant` parameter, `DOSI_WORKSPACE` env var

Verified after the fix: the JWT carried `email=owner@acme.test` and the correct
`tenantid`, and the agent completed a full capture-and-upload cycle.

### Note for operators

ABP's identity seeder names every tenant admin **`admin`** — the email supplied at
signup becomes the address, not the username. Sign in with `admin` + the workspace
name, not the email address.
