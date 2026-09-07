# Dosi-Tracker Web — Feature × Layer Test Matrix

> Honest map of what is protected where, and what is not protected at all. Layers are defined in
> [`TEST_STRATEGY.md`](TEST_STRATEGY.md) §2. This is a **living document**: several suites land in
> parallel work streams, so before trusting a cell, verify it —

```bash
npx vitest run                          # unit/property/component/integration/acceptance — names & counts
npx playwright test --list              # E2E inventory per project
npm run test:coverage                   # what the coverage gate actually sees
grep -rl "fast-check" src               # which files actually use property testing today
```

**Legend** — `YES`: deliberate, direct coverage at that layer. `SOME`: partial or indirect (the
feature is exercised, not systematically asserted). `NO`: no coverage at that layer — a gap.
`N-A`: layer structurally not applicable (e.g. mutation only runs on the `src/lib` logic scope;
pure-UI rows can't be mutation-tested by design).

| Feature | Unit | Property | Component | Integration | Acceptance | E2E | A11y | Visual | Mutation | Notably uncovered |
|---|---|---|---|---|---|---|---|---|---|---|
| Session / auth gate (middleware, SessionProvider) | YES | NO | YES | YES | YES | YES | YES | YES | YES | Real OpenIddict login round-trip and JWT expiry/401 handling against a **live** backend — mocked fetch only; manual QA §2/§15 covers it |
| Roles & guards (`canAccess`, `RoleGuard`, landing) | YES | SOME | YES | YES | YES | YES | N-A | N-A | YES | — |
| Tenant switching / lifecycle (switch, create, rename, delete, isolation) | SOME | NO | SOME | YES | YES | SOME | SOME | NO | SOME | `tenant-data.ts` is now **inside** both gates (91.47% lines / 92.3% branches) but only **75% of its functions** are exercised, and its mutants are killed indirectly by integration tests — there is no dedicated `tenant-data.test.ts` aiming at the lifecycle logic head-on |
| Plan entitlements / usage meters | SOME | NO | SOME | YES | SOME | SOME | SOME | SOME | YES | `saas-data.ts` is in scope and well covered (100% lines / 96.4% branches, with a fast-check property suite). Remaining gap is behavioral, not scope: limit *enforcement* at creation time (seat/project caps) is thin |
| Projects (list, create, edit, archive, detail) | SOME | NO | YES | YES | YES | SOME | YES | NO | SOME | Creation persistence now runs through `createTenantProject` and is asserted across a simulated reload; archived-project edge flows remain thin |
| Team / invites (roles assignable, seats) | YES | NO | YES | SOME | SOME | SOME | YES | NO | YES | Actual invite delivery needs a backend — demo mode stops at the dialog. Address *validation* is now asserted (malformed emails rejected and named) |
| Activities / monitor | YES | SOME | YES | SOME | SOME | SOME | SOME | SOME | SOME | Screenshot **blob** fetches (`getAuthedBlobUrl`) need a backend; live presence is demo-data only; `monitor-data.ts` is a fixture blob and stays scope-excluded by design |
| Reports (7 reports, presets, export) | SOME | SOME | SOME | SOME | SOME | YES | SOME | SOME | SOME | `reports-data.ts` is in scope now (91.22% lines) but only **50% of its functions** run — the aggregation entry points the UI never calls in demo mode are still unproven |
| Billing UI | NO | NO | SOME | SOME | SOME | SOME | SOME | SOME | N-A | **Plan-change flow is demo-toast only** — there is no real subscription mutation to assert until the backend endpoint is wired |
| Host console (tenants, users, plans, billing) | NO | NO | SOME | SOME | SOME | SOME | YES | YES | N-A | Cross-tenant listing tested via mocked fetch only; impersonation depth beyond enter/exit |
| Notifications (toasts) | NO | NO | YES | SOME | NO | SOME | SOME | NO | N-A | Auto-dismiss timing behavior |
| Theme (light/dark, persistence) | NO | NO | YES | SOME | NO | SOME | SOME | SOME | N-A | **Dark mode is unscanned.** E2E pins `colorScheme: "light"`, so dark theme has no screenshot baseline *and no axe run*; the dark `--*-on-tint` contrast tokens in `globals.css` are set but verified only by eye — manual QA §12 |
| Command palette | NO | NO | YES | SOME | NO | SOME | SOME | NO | N-A | — |
| Forms validation (login, signup, dialogs) | SOME | NO | SOME | SOME | SOME | SOME | SOME | N-A | N-A | Server-side validation messages exercised only through mocked fetch responses |
| Storage robustness (corrupt/missing `dosi-*` keys) | SOME | SOME | SOME | YES | NO | SOME | N-A | N-A | SOME | Every `dosi-*` key × 6 hostile payloads is now a gauntlet (`src/test/integration/storage-robustness.test.tsx`); the `tenant-data.ts` guard is in mutation scope, the `session-provider.tsx` guards are not (component layer). Still open: localStorage quota-exceeded; cross-tab session sync |
| Navigation chrome (sidebar, topbar, responsive shell) | NO | NO | YES | YES | SOME | YES | YES | YES | N-A | 4 win32 baselines (login, dashboard, host-overview, reports); CI runs E2E with `--grep-invert "@visual"`, so `@visual` still does not gate (strategy §11.3) |

Cross-cutting facts behind the cells:

- **Mutation column** reflects `stryker.config.json` scope: `src/lib/**/*.ts` + `src/middleware.ts`
  minus the fixture blobs (`mock-data.ts`, `monitor-data.ts`, `host-data.ts`) and `types.ts`. The
  old blanket `*-data.ts` exclusion is gone — `saas-data.ts` and `reports-data.ts` are now measured
  by both the coverage floor and mutation, which is why several `NO`s above became `SOME`/`YES`.
  `tenant-data.ts` is measured by **coverage but not mutation**: ~400 of its 650 lines are seeded
  data generators, whose mutants are low-signal and expensive (a full run exceeded 50 minutes
  without finishing). Any row whose logic lives in components/pages is still `N-A` by design.
- **UX regression gates** live in `e2e/ux-regressions.spec.ts`. They pin defects a UX audit found
  that no functional test would have caught, because the feature "worked" while being unusable:
  KPI cards rendered invisible under `prefers-reduced-motion`; activity session cards built as
  clickable `<div>`s (unreachable by keyboard); dialogs with no focus trap or restore; the blocked-
  route fallback stranding the platform admin; the attendance matrix collapsing into one row; and
  a **dark-theme** contrast scan across four routes. That last one is the only automated check of
  dark mode anywhere in the suite — the `a11y.spec.ts` gate is light-theme only.
- **A11y column**: jsdom axe scans (`npm run test:a11y`) + the Playwright gate in `e2e/a11y.spec.ts`,
  which fails on any **serious or critical** violation across `/login`, `/dashboard`, `/projects`,
  `/team` and `/host` — all five pass today. Moderates are annotated, not gated; the known
  survivors are `landmark-one-main` and `region` on `/login`. Rows scoring `YES` are ones whose page
  is in that scan set. Every scan runs in **light theme only**; keyboard/focus judgments and all
  dark-theme contrast stay in `QA_CHECKLIST.md` §12.
- **Live-API paths** (`useApi` and friends against a real backend) are integration-tested only via
  mocked `fetch`. E2E deliberately aborts all API-origin requests (strategy §3), so no automated
  frontend layer ever talks to the real backend. The backend's own suite plus manual QA hold that
  line today.

---

## Next testing priorities (ranked)

> The previous #1 — *"un-blind the gates: split logic out of `*-data.ts`"* — is **done**, and was
> resolved by fixing the scope rather than moving code: `saas-data.ts`, `tenant-data.ts` and
> `reports-data.ts` now sit inside both the coverage denominator and the mutation scope, with only
> the literal fixture blobs excluded (strategy §4). The follow-on is #1 below.

1. **Close the cold spots the widened gate now shows.** Inclusion made the weak points visible and
   measurable: `reports-data.ts` runs only **50% of its functions** (uncovered `117-121`),
   `tenant-data.ts` **75%** (`605-635`), and `scope.ts` sits at **85% lines** (`13-15`). None has a
   dedicated unit suite — they are killed indirectly through integration tests. Add targeted
   `*.test.ts` files for the aggregation and lifecycle entry points, then ratchet the functions
   floor (85) up behind them.
2. **Billing plan-change flow.** Currently a demo toast — the single most user-visible flow with
   zero end-state assertion. When the backend subscription mutation lands, add integration +
   acceptance + E2E in the same PR.
3. **API contract tests.** `src/test/api-contract.json` exists; wire a check that the mocked-fetch
   shapes used in tests actually match the backend contract, so mocks can't drift silently.
4. **Dark-theme coverage.** Add a dark `colorScheme` E2E pass (at minimum an axe scan + one
   screenshot per key page). This is now the largest *known* blind spot: the `--*-on-tint` tokens
   fixed a systemic contrast failure in light theme, and their dark counterparts have never been
   machine-checked.
5. **Linux visual baselines in CI.** The `frontend-e2e` job already runs everything else; one-time
   ubuntu baseline generation is all that stands between `@visual` and being a real gate
   (strategy §11.3).
6. **Property-test the planned invariants.** fast-check is installed but adoption is partial —
   verify with `grep -rl "fast-check" src`. Priority targets: `canAccess` prefix matching (no
   path traversal past a guard), activity-filter invariants (filtered ⊆ input, order preserved),
   export round-trip (parse(serialize(x)) = x), corrupt-localStorage fuzzing of session recovery.
7. **Ratchet the dependency audit to `high`.** The gate sits at `--audit-level=critical` only
   because 8 high advisories come in transitively via `next@16.2.10` (sharp/libvips). Upgrade Next,
   then tighten the level in the same PR (strategy §2).
8. **Screenshot blob flows.** `getAuthedBlobUrl` and capture rendering need a live backend; cover
   via a nightly job against a seeded backend, or accept and document manual-QA-only status.
9. **Host impersonation depth.** Enter/exit is covered; assert what an impersonating host can and
   cannot do inside the tenant while impersonating.
