# Production Readiness — Gap List

> Honest checklist vs shipping a multi-tenant activity SaaS.  
> As-built facts: [`AS_BUILT.md`](./AS_BUILT.md).  
> Target engineering course: [`bn/README.md`](./bn/README.md).  
> Product demo status: [`../DOCUMENTATION.md`](../DOCUMENTATION.md) §21.

**Verdict (Updated October 2026 — Phase 2/3 Reconciled):** **In Progress (Not Full Production-Ready).**  
Core backend DDD architecture (.NET 10 / ABP), EF Core PostgreSQL persistence, OpenIddict auth, multi-tenant workspace resolution, idempotent activity ingestion (`POST /api/app/activity`), frontend Next.js 16 workspace switching, and desktop agent logic (Windows Rust, macOS Swift) are implemented and unit/integration tested. However, full production readiness is gated by unresolved P0/P1 items: privacy sensor opt-in/lifecycle controls, schema nullability alignment for denied/unsupported metrics, cryptographic device enrollment/revocation, Linux desktop agent implementation, native code signing/notarization, automated blob retention purges, and assistive accessibility audits.

---

## Scorecard

| Area | Ready? | Status & Verified Reality |
|------|:------:|---------------------------|
| Product UX & Workspaces | Partial | Next.js 16 App Router dashboard with multi-workspace switching, role management, invitation flows, and live API hooks; browser E2E coverage ongoing. |
| Identity & Tenancy | Implemented (EF/Host Verified) | OpenIddict password grant, multi-tenant resolution (`X-Workspace-Id`, `__Host-Dosi-Workspace`), workspace-scoped EF Core query filters. |
| Activity Ingest API | Implemented (EF Verified) | `POST /api/app/activity` with idempotent composite index `(WorkspaceId, UserId, ClientActivityId)` and pending capture validation. |
| Product DB Schema | Implemented (EF Verified) | Fully migrated EF Core schema (`Workspaces`, `Projects`, `Activities`, `Screenshots`, `Subscriptions`, `AgentPresences`). |
| Frontend ↔ API | Implemented (Unit/Component Verified) | `useApi` hook, contract clients, and optimistic state updates; live dataset integrates with backend DTOs. |
| Agent → API Sync | Implemented (Client Unit Verified) | Windows and macOS offline SQLite queues with FIFO ordering, exponential backoff, and poison-row parking (`synced = 2`). |
| Object Storage (Screenshots) | Partial (Unit Mocked) | ABP Blob Storing abstraction configured for S3/R2/Database fallback; automated physical blob retention sweeper open. |
| Billing & Subscriptions | Partial (EF Verified) | Subscription and seat quota checks enforced on backend; payment provider webhook integration sandbox-only. |
| CI / Automation | Partial | Local validate gates (`typecheck`, `lint`, `vitest`, `depcruise`) and container configurations present; hosted CI pipeline configuration open. |
| Secrets & Cert Hygiene | Partial | Configuration uses environment variable overrides; Windows uses DPAPI, macOS uses Keychain; Linux `libsecret` open. |
| Observability | Partial | Serilog structured logging and `/health-status` endpoint active; Prometheus/OTel exporters open. |
| Privacy & Sensor Lifecycle | P0 In Progress | Prior audit revealed hooks/sampler ran at launch; explicit per-user opt-in and active-tracking sensor lifecycle being implemented. |
| Native Desktop Packaging | Partial / Blocked | Windows NSIS and macOS scripts present; code signing, Apple Notarization, and Linux packaging are outstanding prerequisites. |

---

## Must-have before any “production” claim

### P0 — Core vertical slice

1. **Domain:** `Project`, `Activity` (and screenshot metadata) aggregates + EF migration.  
2. **APIs** matching agents (or change agents to OpenIddict + real routes):
   - login / token
   - `GET` my projects
   - `POST` activity (idempotent `client_activity_id`)
3. **Wire frontend** auth + one read path (e.g. activities list) off mock.  
4. **Align URLs:** single documented `api_base_url` / CORS origin.  
5. **Tenant isolation tests** on every activity query.

### P1 — Operability

6. ~~`docker-compose` (Postgres ± API) for local parity.~~ Done: production-like self-host stack, see [`SELF_HOSTING.md`](./SELF_HOSTING.md).  
7. CI: build + test backend + frontend lint on PR.  
8. Secrets out of git (User Secrets / env / Key Vault).  
9. Backup job + restore drill documented with RTO/RPO targets.  
10. Screenshot blobs to private storage (not DB).

### P2 — SaaS maturity

11. Host settings (SMTP, storage, retention) persisted — not mock.  
12. Plan limits / retention enforcement.  
13. SignalR (or equivalent) for live monitor.  
14. Payment provider + invoice webhooks.  
15. Staging environment + smoke tests post-deploy.

---

## Known product bugs (frontend demo) to fix before API cutover

| Issue | Risk | Status (2026-07-16) |
|-------|------|---------------------|
| Admin tracking inclusion inconsistent | Wrong payroll/timesheet membership | **Fixed** — shared `isTrackedMember` / `trackedMembers` |
| Timesheet not scoped to current worker | Privacy / role leak in UI | **Fixed** — workers see own week only |
| Invite allows admin → owner | Privilege escalation in UX | **Fixed** — `inviteableRoles()` |
| Seats used: Team live vs Billing `workspace.seatsUsed` | Billing drift | **Fixed** — `liveSeatsUsed()` from dataset |

Server must still enforce permissions even after UI fixes.

---

## Suggested build order

```text
1. Activity + Project domain + migration
2. Agent-compatible ingest + OpenIddict/device auth design
3. Frontend: login + activities from API (feature flag keep mock)
4. Screenshot upload path
5. Compose + CI
6. Host settings + retention
7. Billing + monitoring stack
```

---

## Doc hygiene

| Doc | Role |
|-----|------|
| `DOCUMENTATION.md` | Product / UI SSOT |
| `docs/bn/*` | Target engineering **course** (Bengali) |
| `docs/AS_BUILT.md` | Code reality |
| This file | Gap / readiness |
| `docs/RUNBOOK.md` | How to run / recover |
| `docs/SECURITY.md` | Threats + controls |

Course chapters stay **Target** until code catches up; do not pretend Grafana/IIS/Hangfire are shipped.
