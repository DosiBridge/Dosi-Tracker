# Dosi-Tracker Web — Test Strategy

> How the Next.js frontend is proven correct. Companion to [`docs/QUALITY.md`](../QUALITY.md) (the
> cross-tier quality charter); this document is the frontend-specific playbook. All paths below are
> relative to `frontend/` unless noted.

**Owner:** engineering · **Status:** living document

This is a **living document**: wherever a count would rot, the command that produces it is given
instead. The only hardcoded numbers here are **thresholds**, which are config-sourced facts — each
one names the file it lives in, and per the charter they only ever ratchet up.

---

## 1. Philosophy

**Do not trust the implementation. Trust independently verified behavior.**

Much of this codebase is AI-generated or AI-assisted. That changes the job of the test suite: a
test derived *from* the implementation ("assert whatever the code currently does") merely
notarizes the code, bugs included. Tests here must be derived from the **requirements and domain
facts** — the role matrix in `src/lib/roles.ts`, the plan limits in `src/lib/saas-data.ts`, the
tenant-isolation contract — and then run against the implementation as an independent check. When
a test and the code disagree, the test is not automatically wrong.

Consequences:

1. **Test behavior, not implementation.** Assert what a user can observe: rendered text, enabled
   controls, navigation outcomes, persisted state. No "function was called once" assertions unless
   the call *is* the behavior (e.g. `router.push("/login")` on an expired session).
2. **Behavioral test names.** `"blocks a worker from /reports"`, never `"test 3"`. A failure
   message should read as a broken product requirement.
3. **Every layer earns its keep.** A layer exists because it catches a class of defect the layers
   below cannot (see the portfolio table). Redundant tests are deleted, not accumulated.
4. **Green is binary.** A flaky test is a defect (§9). A skipped test is a lie — delete it and
   record the gap instead.

---

## 2. The test portfolio

| Layer | Tool | Location | Command | What it protects |
|---|---|---|---|---|
| **Static — types** | `tsc` | whole `src/` | `npm run typecheck` | Contract drift between modules; nullability |
| **Static — lint** | ESLint, warning cap **45** (`package.json` `lint` script) | whole `src/` | `npm run lint` | Style + correctness rules; the cap is a ratchet (QUALITY.md §10) — new debt fails CI |
| **Static — architecture** | dependency-cruiser (`.dependency-cruiser.cjs`) + `src/test/architecture.spec.ts` | whole `src/` | `npm run test:arch` | Layering (`app → components → hooks → lib`), no cycles, no test code in production modules |
| **Unit + property** | Vitest + fast-check | `src/lib/*.test.ts`, `src/middleware.test.ts` | `npx vitest run src/lib src/middleware.test.ts` | Pure logic invariants (role access, filters, exports, scope math) over many inputs, not just examples |
| **Component** | Vitest + Testing Library (jsdom) | colocated `*.test.tsx` next to the component, plus page-level suites in `src/test/pages/` | `npx vitest run src` | One component or page behaves correctly for a real user: queries by role/label, user-event interaction |
| **Integration** | Vitest + Testing Library, real providers via the harness | `src/test/integration/` | `npx vitest run src/test/integration` | Multiple units wired together: session → guard → page; tenant switch → data isolation |
| **Acceptance (BDD)** | Vitest with Given/When/Then structure + `.feature` documents | `src/test/acceptance/` | `npx vitest run src/test/acceptance` | User-facing scenarios in business language; the `.feature` files are the reviewable spec (§11.1) |
| **E2E + a11y + visual + responsive** | Playwright (+ `@axe-core/playwright`) | `e2e/` | `npm run build` then `npm run test:e2e` | The whole app, production build, driven like a user — in hermetic demo mode (§3) |
| **Mutation** | StrykerJS (`stryker.config.json`) | mutates `src/lib/**/*.ts` + `src/middleware.ts`, minus the fixture blobs (`mock-data.ts`, `monitor-data.ts`, `host-data.ts`) and `types.ts` | `npm run test:mutation` | That the unit tests actually *detect* faults. Break threshold **78** (ratchet floor); nightly in CI |
| **Coverage gate** | `@vitest/coverage-v8` (`vitest.config.ts`) | logic layer only (§4) | `npm run test:coverage` | Regression floor: lines **94**, statements **94**, functions **85**, branches **91** |
| **Dependency audit** | `npm audit` | `package-lock.json` | `npm audit --audit-level=critical` | Known-vuln floor; gate level is itself a ratchet (see below) |
| **Bundle budget** | `scripts/bundle-budget.mjs` | `.next/static/chunks` after build | `npm run build:budget` | Single chunk ≤ **480 KB**, total JS ≤ **1900 KB** (env-overridable ratchet; tighten, never loosen without written justification) |

CI wiring: `.github/workflows/ci.yml` has two frontend jobs on every PR. **`frontend`** runs
typecheck → lint → `test:coverage` → `test:arch` → `npm audit --audit-level=critical` → build →
`build:budget`. **`frontend-e2e`** installs chromium + firefox + webkit, builds, and runs
`npx playwright test --grep-invert "@visual"` — the whole Playwright suite except the screenshot
assertions, which stay local-only until Linux baselines exist (§11.3).
`.github/workflows/mutation.yml` runs Stryker nightly (03:00 UTC) and on `workflow_dispatch`.
`npm run validate` is the local pre-push aggregate (typecheck + lint + test + arch).

The audit gate sits at **`critical`**, not `high`, deliberately: the 8 high-severity advisories in
the tree are transitive through `next@16.2.10` (sharp → libvips) with no override available — they
clear only when Next is upgraded. Gating at `high` today would mean a permanently red pipeline that
teaches everyone to ignore it. **Ratchet the level to `high` in the same PR that lands the Next
upgrade** (tracked in `TEST_MATRIX.md`).

Tag conventions:

- Vitest a11y tests include **`a11y` in the test name** — `npm run test:a11y` selects them via
  `--testNamePattern a11y`.
- Playwright tags in test titles: `@smoke` (cross-browser subset), `@mobile` (Pixel 7 project),
  `@visual` (screenshot assertions). Projects in `playwright.config.ts`: chromium runs everything
  except `@mobile`; firefox/webkit run `@smoke` only; the mobile project runs `@mobile` only.

---

## 3. E2E demo-mode design — why it is deterministic and backend-free

E2E runs against a **production build** (`next build` + `next start`, port 3123 by default —
`E2E_PORT` overrides) with every page in **demo mode**. The fixture in `e2e/fixtures.ts` arranges
three things per test context:

1. **The `dosi-token` cookie is set (value `1`) — with no JWT behind it.** The middleware
   (`src/middleware.ts`) gates routes only on cookie *presence*, so navigation works. But the app's
   live-backend path (`src/hooks/useApi.ts`) reads the JWT from `localStorage["dosi-token"]`, which
   the fixture never sets — so every page takes its demo-data branch.
2. **Every request to the real API origins (`https://localhost:44342`, `http://localhost:8080`) is
   aborted** via `context.route(...).abort()`. A backend that happens to be running on the dev
   machine can never leak nondeterministic data into a test run.
3. **The demo clock is frozen**: `NOW = 2026-07-14T15:30:00Z` (`src/lib/mock-data.ts`). "Today",
   trial countdowns, and every timestamp on screen are constants. Playwright additionally pins
   `timezoneId: "UTC"`, `locale: "en-US"`, `colorScheme: "light"`, and `reducedMotion: "reduce"`.

The result is a hermetic suite: **no network, no backend, no clock drift, no animation timing** —
the same DOM and the same pixels on every run, which is what makes `@visual` screenshot assertions
(tolerance `maxDiffPixelRatio: 0.02`, animations disabled) and a11y scans trustworthy. Tests are
order-independent by construction: each test gets a fresh context, and app-persisted state
(localStorage) dies with it. Never chain tests.

`primeSession(context, { userId, workspaceId })` (in `e2e/fixtures.ts`) switches the demo session
to a seeded persona **before** `page.goto` — this is how role-based E2E flows sign in without a
login form or a backend.

What demo mode deliberately does **not** cover: the real OpenIddict login round-trip, JWT refresh/
expiry against a live API, and screenshot blob fetches. Those are backend-integration concerns —
covered by the backend suite and the manual QA checklist (`QA_CHECKLIST.md`), and honestly listed
as gaps in `TEST_MATRIX.md`.

**The a11y gate.** `e2e/a11y.spec.ts` runs a full axe scan on five screens — `/login`,
`/dashboard`, `/projects`, `/team`, `/host` — and **fails on any `serious` or `critical`
violation**. Lower-impact findings are pushed to `testInfo.annotations` so they show up in the
report without gating. All five pages pass today. The known remaining moderates are
`landmark-one-main` and `region` on `/login` (the auth screen has no `<main>` landmark); the
heading-order moderates are fixed. One caveat worth stating plainly: because `playwright.config.ts`
pins `colorScheme: "light"`, **every automated a11y scan is a light-theme scan**. The dark-theme
`--*-on-tint` values in `globals.css` are set but proven only by eye — dark-theme scanning is an
open follow-up in `TEST_MATRIX.md`, and dark contrast stays a manual QA item until it lands.

---

## 4. Coverage policy

Coverage is scoped to the **logic layer**: `src/lib/**/*.ts` + `src/middleware.ts`, excluding only
`types.ts` and the three literal fixture blobs — `mock-data.ts`, `monitor-data.ts`, `host-data.ts`
(see `vitest.config.ts` `test.coverage`). Thresholds — lines **94**, statements **94**, functions
**85**, branches **91** — are a **floor plus ratchet**: set just below the measured baseline so the
gate is green today, raised only in the same PR that lands the tests clearing the new bar, never
lowered (QUALITY.md §4).

UI components and pages are **not** in the coverage denominator, on purpose. They are covered
*behaviorally* — component, integration, acceptance, and E2E layers — where the assertion is "a
worker cannot see Reports", not "line 141 executed". Chasing a global percentage that is dominated
by JSX markup is vanity metrics: it rewards rendering a component once over proving anything about
it. Per the project principle: do not trust the implementation (or a number derived from merely
executing it) — trust independently verified behavior.

**The denominator was widened, not the percentage lowered.** The gate used to exclude everything
matching `*-data.ts`, which swept up three modules that are mostly *logic* wearing a data-shaped
filename: `saas-data.ts` (plan/usage/invoice math), `tenant-data.ts` (tenant lifecycle +
localStorage persistence) and `reports-data.ts` (report aggregation). Their behavior was tested,
but neither the coverage floor nor the mutation gate could see it — the old 95/95/95/85 was a high
score on a small, easy denominator. All three are now inside both scopes. Only the literal fixture
blobs stay out, because they are arrays of demo rows with no branching to get wrong.

Measured baseline on the widened denominator: **lines/statements 95.12%, functions 86.74%,
branches 92.81%** — comfortably above the floors, with the newly visible files carrying real
numbers: `saas-data.ts` 100% lines / 96.4% branches, `tenant-data.ts` 91.47% lines / 92.3%
branches, `reports-data.ts` 91.22% lines, and `roles.ts`, `export.ts`, `middleware.ts` at 100%/100%.
The functions floor is the loosest (85) because the widening exposed genuine cold spots —
`reports-data.ts` at 50% functions, `tenant-data.ts` at 75%, `scope.ts` at 85% lines. Those are
named work items in `TEST_MATRIX.md`, not permanent residents. What did **not** change: UI
components and pages stay out of the denominator, for the reason above.

Report: `npm run test:coverage` prints the summary and writes HTML to `coverage/index.html`.

---

## 5. Mutation testing

`stryker.config.json` mutates the decision logic — `src/lib` plus `src/middleware.ts`, **including**
`saas-data.ts` and `reports-data.ts`, excluding the fixture blobs, `types.ts`, and `tenant-data.ts`
(§4) — and reruns the Vitest suite per mutant (`coverageAnalysis: "perTest"`). `tenant-data.ts` is
the one file measured by coverage but not mutation: ~400 of its 650 lines are seeded data
generators, so its mutants are low-signal, and including it pushed a run past 50 minutes without
finishing. Everything else that counts toward the coverage floor also has its assertions checked
for teeth, because a percentage without that is just execution. Thresholds: **break 85** (the run
fails below this — the ratcheting floor), low 85, high 90. Runs nightly
(`.github/workflows/mutation.yml`) and on demand, not per-PR — mutation is minutes-slow and the
PR loop must stay fast.

**Measured: 87.76%** (764 killed, 68 timeout, 91 survived, 25 no-coverage; 24 min). Per file:
`reports-data.ts` 100%, `saas-data.ts` 98.1%, `roles.ts` 97.4%, `export.ts` 93.6%, `utils.ts` 85.7%,
`middleware.ts` 84.2%, `scope.ts` 81.8%, `activity-filters.ts` 77.0% ← the remaining weak spot.

This gate has already earned its keep. `reports-data.ts` sat at **91% line coverage and a 20.5%
mutation score**: its date-preset logic (`today`/`yesterday`/`7d`/`30d`/`month`) was executed by
tests but barely asserted, so mutants flipping `getDate() - 1` to `+ 1` survived untouched — every
report's date range could have broken with the suite still green. Targeted boundary assertions took
that file to 100% and the overall score from 77.7% to 87.8%. Coverage is not test quality; this is
the number that tells you the difference.

A surviving mutant is a fact: some line can be wrong without any test noticing. Treat survivors in
touched files as work items for the same PR series, and raise `break` as the score climbs.

Report: `reports/mutation/mutation.html` (and `mutation.json`) after `npm run test:mutation`; CI
uploads the same as the `frontend-mutation-report` artifact.

---

## 6. Test harness — how to write a test here

Everything below lives in `src/test/`. **Do not** hand-roll providers, session state, or
navigation mocks in individual tests — the harness exists so tests exercise the *real* wiring.

### 6.1 Reset between tests — always

The app keeps client state in three places that leak across tests: localStorage (`dosi-*` keys),
the module-level tenant-data singleton (with in-place seed-array mutations), and the navigation
stub. `resetPrototypeState()` clears all three. **Every component/integration test file calls it
in `beforeEach`.** No exceptions.

Its counterpart is `simulateReload()`: it drops the in-memory dataset cache while **leaving
localStorage intact**, which is the only honest way to assert "this survives a refresh" in jsdom.
Use it for persistence claims (a created project or workspace still being there afterwards) —
`resetPrototypeState()` would wipe the very thing under test. Unmount the tree first; the next
render rebuilds from the seeds plus whatever localStorage kept.

### 6.2 The `next/navigation` stub

`vi.mock` is hoisted, so the mock must be the first line of the **test file** (it cannot live in
the harness). Required whenever the component under test — or anything it renders — imports
`usePathname` / `useRouter` / `useSearchParams` / `Link`:

```tsx
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
```

Then, optionally: `import { __setPathname, __router } from "@/test/next-navigation-stub";` —
`__setPathname("/reports")` positions the app, and `__router.push` is a spy you can assert
redirects against (the one case where "was called with" *is* the behavior).

### 6.3 Rendering with real providers

`renderWithProviders(ui, opts)` renders inside the real `ThemeProvider` + `SessionProvider`,
seeding identity through localStorage **before** render — the exact mechanism the app itself uses,
so no provider internals are mocked:

```tsx
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderAsRole, resetPrototypeState } from "@/test/harness";
import { Sidebar } from "@/components/layout/sidebar";

describe("sidebar navigation", () => {
  beforeEach(() => resetPrototypeState());

  it("hides Reports from a worker", () => {
    renderAsRole(<Sidebar />, "worker");
    expect(screen.queryByRole("link", { name: /reports/i })).not.toBeInTheDocument();
  });
});
```

Options: `role` (resolves to a seeded user of that role — roles are
`host | owner | admin | worker | client`), `userId` (wins over `role`), `workspaceId`
(`w1` Dosi Labs/business, `w2` Acme Studio/starter-trialing, `w3` Nimbus Co/free), `route`
(pathname the stub reports — what `RoleGuard` sees), `withToasts` (mounts the `ToastViewport` so
`toast()` output is queryable). Sugar wrappers: `renderAsRole(ui, role, opts)` and
`renderForWorkspace(ui, wsId, opts)`. `seededUserId(role, wsId)` resolves a seeded user id when
you need it directly.

### 6.4 Factories

`src/test/factories.ts` builds deterministic domain objects with `t-`-prefixed ids that can never
collide with seeded demo data (`u1…`, `p1…`, `w1…`): `buildUser`, `buildProject`, `buildActivity`,
`buildWindowInfo`, `buildWorkspace` — each takes partial overrides. To put a factory object in
front of a component, push it into the tenant dataset before rendering:

```tsx
import { buildUser } from "@/test/factories";
import { datasetFor } from "@/lib/tenant-data";

datasetFor("w2").users.push(buildUser({ role: "client", name: "Carla Client" }));
```

`resetPrototypeState()` resets both the dataset and the factory id counter.

### 6.5 Accessibility assertions

The jest-axe matcher is registered globally (`src/test/setup.ts`). Include `a11y` in the test name
so `npm run test:a11y` finds it:

```tsx
import { axe } from "jest-axe";

it("a11y: sidebar has no axe violations for an owner", async () => {
  const { container } = renderAsRole(<Sidebar />, "owner");
  expect(await axe(container)).toHaveNoViolations();
});
```

### 6.6 Determinism rules

- **No real network.** Demo-mode components fetch and fail over; where a test must control the
  fetch, `vi.stubGlobal("fetch", …)` and restore it. Never rely on a locally running backend.
- **No `Date.now` assertions.** The demo dataset is frozen at `2026-07-14T15:30Z`; if a test needs
  to move time, `vi.useFakeTimers()` and restore in `afterEach`.
- **No test-order dependence.** Each file must pass alone: `npx vitest run <file>`.

---

## 7. Debugging how-to

| Task | Command |
|---|---|
| Run one Vitest file | `npx vitest run src/lib/roles.test.ts` |
| Run one test by name | `npx vitest run src/lib/roles.test.ts -t "blocks a worker"` |
| Watch mode | `npm run test:watch` |
| Coverage HTML | `npm run test:coverage` → open `coverage/index.html` |
| Mutation HTML | `npm run test:mutation` → open `reports/mutation/mutation.html` |
| E2E (full) | `npm run build` then `npm run test:e2e` |
| E2E interactive UI | `npm run test:e2e:ui` |
| E2E one file / one project | `npx playwright test e2e/<file>.spec.ts --project=chromium` |
| E2E smoke only | `npm run test:smoke` |
| E2E HTML report | `npm run e2e:report` (serves `playwright-report/`) |
| Trace viewer | `npx playwright test --trace on` then `npx playwright show-trace test-results/<test-dir>/trace.zip` (CI captures traces automatically on first retry) |
| Update visual baselines | `npx playwright test --grep @visual --update-snapshots` — review the diff like code (§11.3) |
| Architecture check | `npm run test:arch` |
| Bundle budget | `npm run build` then `npm run build:budget` (prints the five largest chunks) |
| Everything before pushing | `npm run validate` |

---

## 8. Flaky-test policy

**A flaky test is a defect in the test**, with the same priority as a product bug — it either gets
fixed or deleted the day it flakes; it is never retried into submission or skipped.

Rules that prevent flakes in the first place:

- **No sleeps.** No `setTimeout`-and-hope in Vitest, no `page.waitForTimeout` in Playwright.
- **Wait on observable conditions**: Testing Library `findBy*` / `waitFor` on a rendered outcome;
  Playwright web-first assertions (`await expect(locator).toBeVisible()`), which auto-retry.
- **Frozen clock, no network, fresh state** (§3, §6.6) remove the three usual flake sources.
- CI retries (`retries: 2` in `playwright.config.ts`) exist to *surface* flakes with a trace
  attached — a test that passed on retry is logged as broken, not as passed.

---

## 9. Regression policy

Every bug fix follows the same sequence, no exceptions:

1. **Write the failing test first**, at the lowest layer that can reproduce the bug. The test name
   references the observed defect, not the fix.
2. **Apply the minimal fix** until that test (and the rest of the suite) is green.
3. **The test stays forever.** It is the executable memory of the bug; deleting it re-opens the
   bug's return path.

If a bug cannot be reproduced in an automated layer (e.g. it needs a live backend), it goes into
`QA_CHECKLIST.md` with a precise repro so the manual gate catches a recurrence.

### 9.1 Worked examples — bugs this suite actually found

These are not hypotheticals. Each was found by writing the test **against the requirement** rather
than against the code, each was a real user-visible defect, and each is now pinned by a test that
fails if the fix is reverted. They are the argument for §1 in concrete form.

| Defect | Where it lived | Now pinned by |
|---|---|---|
| **4 unguarded `JSON.parse` crashes.** A corrupted or wrong-type `dosi-*` value white-screened the app: spreading a parsed non-array, `.includes` on a parsed `null`, `.length` on a parsed object | `src/components/session-provider.tsx` (`dosi-workspaces-created` / `-patches` / `-deleted`), `src/lib/tenant-data.ts` `getLocalStorageProjects` | the hostile-payload gauntlet in `src/test/integration/storage-robustness.test.tsx` — every `dosi-*` key × 6 hostile payloads |
| **Demo-created projects were never persisted.** The wizard added a project to the in-memory dataset only; a refresh lost it | `src/app/(dashboard)/projects/page.tsx` — now calls `createTenantProject()` | `src/test/pages/projects-create.test.tsx`, asserting across `simulateReload()` |
| **Activities rendered an empty state in demo mode.** Unlike `/projects`, `/team` and `/dashboard`, it had no seeded fallback, so demo users saw "No activities match your filters" | `src/app/(dashboard)/activities/page.tsx` | `src/test/pages/activities-demo.test.tsx` |
| **A failed hydration wiped demo tenant data.** An unreachable backend resolved to `[]` and overwrote the active tenant's seeded data with nothing | `src/components/session-provider.tsx` — failures now resolve to `null` sentinels and hydration only runs on a real array payload | the storage/session integration suites (rejecting `fetch` is part of the gauntlet) |
| **`usagePct(used, 0)` returned `NaN`**, rendering "NaN%" in usage meters; `n/0` returned `Infinity` | `src/lib/saas-data.ts` — explicit `limit <= 0` guard | `src/lib/saas-data.test.ts` boundary cases + `saas-data.property.test.ts` (integer in `[0,100]` for *any* input) |
| **The create-project wizard retained input after Cancel.** Reopening showed the abandoned draft | `src/components/projects/create-project-modal.tsx` — `reset()` on every close path, not just submit | `src/test/pages/projects-create.test.tsx` |
| **The invite form accepted malformed emails.** Anything non-empty was sent | `src/app/(dashboard)/team/page.tsx` — validates each address and **names the offender** in the error | `src/test/pages/team-invite.test.tsx` |
| **Accessibility, a cluster of them.** Unnamed dialogs (modal + drawer), a progress bar with no `role`/`aria-valuenow`, an unnamed sidebar-collapse control and toolbar search field, unlabeled login/settings/invite fields, an unnamed password toggle, `CardTitle` emitting `h3` under an `h1` (heading-order), and a **systemic colour-contrast failure** on every status tint | `src/components/ui/*` and the affected pages; contrast fixed with new `--*-on-tint` design tokens in `src/app/globals.css`, consumed by `badge.tsx` | `e2e/a11y.spec.ts` (5 pages, serious+critical = fail) plus the jsdom `a11y:` tests (`npm run test:a11y`) |

---

## 10. Living counts

Do not hardcode test counts in docs or PRs — produce them:

```bash
npx vitest run                 # per-file and total test counts in the summary
npx playwright test --list     # enumerates E2E tests per project
npm run test:coverage          # current coverage vs. the §4 floor
npm run test:mutation          # current mutation score vs. break=78
npm run build:budget           # chunk count, total KB and largest chunk vs. the §2 budget
npm audit                      # current advisory counts behind the §2 audit gate
```

---

## 11. Deliberate adaptations

Where this frontend departs from the charter's default tooling (QUALITY.md §2/§8), the departure
is deliberate and recorded here — not drift.

### 11.1 No Cucumber runner — BDD as vitest + `.feature` documents

The charter names `@cucumber/cucumber` for web acceptance tests. We run acceptance scenarios in
**Vitest with Given/When/Then structure** instead, keeping the `.feature` files in
`src/test/acceptance/` as the human-reviewable specification the tests mirror. Why: a Cucumber
runner adds a second test runner, a second transform pipeline, and a step-binding indirection
layer to a TypeScript/JSX codebase — for zero additional proof. One runner means one coverage
report, one watch mode, one debugging story. The business-language value of BDD lives in the
`.feature` documents and the scenario-shaped test names, not in the glue framework.

### 11.2 No Prettier — lint is the style gate

The charter's Definition of Done mentions `format:check`; this package has no format script, and
that is intentional. ESLint (flat config, `eslint-config-next`) is the single style-and-correctness
gate, capped via `--max-warnings 45` (ratcheting down per QUALITY.md §10). Adding Prettier now
would reformat files wholesale and destroy `git blame` across a codebase mid-ratchet, for a purely
cosmetic win. Revisit when the lint cap reaches 0.

### 11.3 Visual baselines are win32-local until CI generates Linux baselines

Four `@visual` baselines exist — **login, dashboard, host-overview, reports** — in
`e2e/visual.spec.ts-snapshots/`, each suffixed `-chromium-win32` because they were generated on the
Windows dev host. Font rasterization and subpixel rendering differ per OS, so they **cannot gate
Linux CI**: the `frontend-e2e` job runs `--grep-invert "@visual"` and `@visual` stays a local-only
check until a one-time ubuntu job generates and commits Linux baselines
(`npx playwright test --grep @visual --update-snapshots`). Everything else — functional, role,
responsive and a11y E2E — does gate CI.

The current baselines were **regenerated after the a11y colour-token changes** (the `--*-on-tint`
work in §9.1 moved pixels on every status pill) and then verified stable across two consecutive
runs, so a diff today means something actually changed. Baseline updates are reviewed like code: an
unexplained pixel diff is a finding, not an inconvenience.

### 11.4 Next.js app, not Vite

The testing prompt this suite was built from assumed a Vite SPA. This is a Next.js 16 App Router
app, so: Vitest uses `@vitejs/plugin-react` + jsdom directly (no `next dev` involvement in unit or
component tests); `next/navigation` is stubbed via `src/test/next-navigation-stub.ts` (§6.2); the
route gate is a real Next middleware tested as a pure function (`src/middleware.test.ts`); and E2E
runs against `next build` + `next start` — the production server, not a dev server.
