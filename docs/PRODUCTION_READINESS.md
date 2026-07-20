# Production Readiness — Gap List

> Honest checklist vs shipping a multi-tenant activity SaaS.  
> As-built facts: [`AS_BUILT.md`](./AS_BUILT.md).  
> Target engineering course: [`bn/README.md`](./bn/README.md).  
> Product demo status: [`../DOCUMENTATION.md`](../DOCUMENTATION.md) §21.

**Verdict (2026-07-16):** **Not production-ready.** Strong demo frontend + ABP scaffold + agent skeletons. No end-to-end ingest, no product schema, no CI/CD, no real web auth.

---

## Scorecard

| Area | Ready? | Blocker |
|------|:------:|---------|
| Product UX demo | Demo-only | Mock data / localStorage |
| Identity / tenancy (platform) | Partial | OpenIddict works; not integrated with web or agents’ expected paths |
| Activity ingest API | No | Endpoints agents call do not exist |
| Product DB schema | No | Only `Abp*` / OpenIddict tables |
| Frontend ↔ API | No | No real data layer |
| Agent → API sync | No | Client awaits missing APIs; default port may mismatch Host (`44300` vs `44342`) |
| Object storage (screenshots) | No | Planned |
| Billing / payments | No | UI mock only |
| CI/CD | No | No GitHub Actions / compose |
| Secrets / cert hygiene | No | Dev passphrases in `appsettings.json` |
| Observability | Partial | Serilog + health only |
| Security hardening | No | See [`SECURITY.md`](./SECURITY.md) |
| Runbooks / DR | Draft | [`RUNBOOK.md`](./RUNBOOK.md) is local-dev oriented until prod exists |

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

6. `docker-compose` (Postgres ± API) for local parity.  
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
