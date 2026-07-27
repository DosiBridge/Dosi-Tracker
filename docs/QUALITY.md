# Dosi-Tracker — Quality Charter (the "gauntlet")

> The single source of truth for how we prove Dosi-Tracker is correct. Every change runs this
> gauntlet before it is trusted. Thresholds **ratchet up** — they may only ever increase.

**Owner:** engineering · **Status:** living document · **Established:** 2026-07 (Phase 0)

---

## 1. Principles

1. **Tests are a gate, not decoration.** CI blocks on them (once Phase 1 lands). A red gate is a broken build.
2. **Ratchet, never regress.** Coverage and mutation thresholds start at the measured baseline and only rise. A PR may not lower a threshold or drop below it.
3. **Test behavior, not implementation.** Prefer public-surface tests (app services, domain invariants, rendered UI, agent I/O) over white-box tests that break on refactor.
4. **Every tier is covered** — backend, web frontend, and the desktop clients (Rust/Windows, Swift/macOS).
5. **Fast feedback first.** Unit → integration → acceptance (BDD) → mutation, cheapest and most numerous at the bottom.

---

## 2. Test taxonomy

| Level | What it proves | Backend (.NET) | Web (Next.js) | Clients |
|---|---|---|---|---|
| **Unit** | One pure unit's logic/invariants | xUnit (`*.Domain.Tests`) | Vitest + Testing Library | `cargo test` (Rust), XCTest (Swift) |
| **Integration** | A slice through real infra | xUnit + SQLite in-memory (`*.EntityFrameworkCore.Tests`) | Vitest + MSW (mocked HTTP) | `cargo test` against in-memory SQLite |
| **Acceptance (BDD)** | A user-facing scenario, in business language | ✅ Reqnroll `*.feature` over app services (`Dosi.Tracker.Acceptance.Tests`) | Gherkin `.feature` (cucumber) over the UI/API boundary | — (covered by backend BDD) |
| **E2E** | The whole app, driven like a user | — | Playwright (future) | manual smoke (documented QA) |
| **Mutation** | The tests actually *detect* faults | Stryker.NET | StrykerJS | (Rust: `cargo-mutants`, best-effort) |
| **Property** | Invariants hold over many inputs | (where valuable) | fast-check (where valuable) | proptest (Rust, where valuable) |

Naming: unit/integration files end `Tests.cs` / `.test.ts(x)`; acceptance files are `*.feature` with step bindings alongside.

---

## 3. Baseline (measured 2026-07, Phase 0)

Backend coverage — `dotnet test` (integration suite) with coverlet, product assemblies only:

| Assembly | Line | Branch |
|---|---:|---:|
| Dosi.Tracker.Application | 95.5% | 80.0% |
| Dosi.Tracker.Application.Contracts | 100% | 100% |
| Dosi.Tracker.Domain | 62.6% | 37.5% |
| Dosi.Tracker.Domain.Shared | 100% | 100% |
| Dosi.Tracker.EntityFrameworkCore | 85.2% | 50.0% |
| **Overall** | **82.8%** | **47.8%** |

| Tier | Tests today | Coverage today |
|---|---|---|
| Backend | 72 — 10 unit + 47 integration + **15 BDD acceptance** | **86.7% line / 47.8% branch** (merged, all suites) |
| Web frontend | **87** (vitest, logic layer) | **98.2% line / 87.4% branch / 100% func** (scoped to `src/lib` + middleware) |
| Rust client | 9 — incl. 2 **property-based** (proptest) | not measured |
| Swift client | 0 — deferred (needs macOS, see §11) | 0% |
| Mutation score | backend domain **100%** · web logic **82.6%** | nightly gate (Stryker) |

Notes:
- The single-suite integration run reports 82.8% line; the **merged** number across all test
  projects (Domain unit + integration) is 86.7% and is what the CI gate checks.
- The web figure is the **logic layer** (`src/lib` business logic + the route middleware); UI
  components and pages enter the coverage denominator in the component-test phase.
- The Domain gap (62.6%) is the untested background workers (`TrialExpiryWorker`,
  `InvoiceGenerationWorker`) and the DB migration service — targeted in Phase 3/5.

---

## 4. Gate thresholds & ratchet schedule

Gates start at (or just below) the baseline so nothing is red on day one, then rise each phase. **A threshold is a floor: CI fails below it.**

| Gate | Phase 1 (floor) | Phase 3 | Target | Status |
|---|---:|---:|---:|:--|
| Backend line coverage | 80% | 85% | 90% | ✅ enforced (CI) |
| Backend branch coverage | 45% | 55% | 70% | ✅ enforced (CI) |
| Web line coverage (logic layer) | 95% | 95% | 95% | ✅ enforced (CI) |
| Web branch coverage (logic layer) | 85% | 85% | 85% | ✅ enforced (CI) |
| Web function coverage (logic layer) | 95% | 95% | 95% | ✅ enforced (CI) |
| Frontend lint-warning cap | 48 | 20 | 0 | ✅ enforced (CI) |
| Backend domain mutation score | 85% | 90% | 90% | ✅ enforced (nightly) — **100%** today |
| Web logic mutation score | 78% | 82% | 85% | ✅ enforced (nightly) — **82.6%** today |

Rule: to raise a threshold, first land tests that clear the new bar, then bump the number in the same PR. Never bump the number alone.

Where the gates live: backend line/branch — `backend/scripts/coverage-gate.sh` (called by CI with
the floor args); web coverage — `frontend/vitest.config.ts` `test.coverage.thresholds`; lint cap —
`frontend/package.json` `lint` script (`--max-warnings`). All run in `.github/workflows/ci.yml`.

---

## 5. Definition of Done (every change)

A change is **not done** until:

- [ ] New/changed behavior has tests at the lowest level that can prove it (unit first).
- [ ] A user-facing flow change has an acceptance (`.feature`) scenario.
- [ ] `typecheck` + `lint` + `format:check` pass (web); `dotnet build -warnaserror` clean (backend); `clippy -D warnings` clean (Rust).
- [ ] The full test suite for the touched tier is green **locally**.
- [ ] Coverage did not drop below the current gate.
- [ ] No new mutation survivors introduced in touched core logic (Phase 3+).
- [ ] The QA checklist (§7) items relevant to the change are ticked.

---

## 6. CI gates (enforced from Phase 1)

CI runs, in order, and **blocks merge** on any failure:

1. **Build** — all tiers compile (`-warnaserror` / `-D warnings`).
2. **Lint + format + typecheck** — web + backend analyzers + Rust clippy.
3. **Unit + integration tests** — backend (`dotnet test`), web (`vitest run`), clients (`cargo test`, `swift test`).
4. **Coverage gate** — fail if below §4 floor; upload the report artifact.
5. **Acceptance (BDD)** — `.feature` suites (Phase 2+).
6. **Mutation** — scheduled (nightly) not per-PR to keep PRs fast; PR fails only if the mutation score for *changed files* regresses (Phase 3+).

**Metrics surface:** the coverage gate writes a per-assembly table to the GitHub Actions run
summary each build (`backend/scripts/coverage-gate.sh` → `$GITHUB_STEP_SUMMARY`); vitest prints the
web coverage summary; the nightly Stryker jobs upload HTML mutation reports as artifacts. The human
half of the gate is `.github/PULL_REQUEST_TEMPLATE.md`, which reproduces the §5 Definition of Done
and the §7 QA checklist on every PR.

---

## 7. QA procedures (manual, per release)

Automated gates cover regression; a human still signs off on the things a machine can't feel. Before a release:

- [ ] **Backend**: migrator runs clean on a fresh DB; API health green; a real sign-up → sign-in → tracked-activity → screenshot round-trips (see `docs/RUNBOOK.md`).
- [ ] **Web**: middleware route-protection redirects; login/logout; each dashboard/report renders with real API data (not mock fallback) when signed in.
- [ ] **Windows client**: fresh install → sign-in → tracking pill green → capture uploaded → tray Pause/Resume/Quit → single-instance guard.
- [ ] **macOS client**: builds on a Mac; permissions prompts appear; one capture cycle uploads.
- [ ] **Secrets**: no template dev secret is live in the target environment (the prod guard must pass).
- [ ] Record the run in the release notes with the commit SHA and the gate results.

---

## 8. Tooling reference

| Concern | Backend | Web | Clients |
|---|---|---|---|
| Test runner | `dotnet test` (xUnit) | `vitest` | `cargo test` / `swift test` |
| Assertions | Shouldly | Testing Library + vitest `expect` | std assert |
| HTTP isolation | SQLite in-memory | MSW | in-memory SQLite |
| Coverage | coverlet (`backend/test/Directory.Build.props`) | `@vitest/coverage-v8` | `cargo-llvm-cov` (best-effort) |
| BDD | Reqnroll | `@cucumber/cucumber` or vitest-cucumber | — |
| Mutation | Stryker.NET | StrykerJS | `cargo-mutants` |
| CI | `.github/workflows/ci.yml` | same | same |

---

## 9. How to run the gauntlet locally

```bash
# Backend (unit + integration + BDD acceptance all run from the solution)
cd backend && dotnet test /p:CollectCoverage=true /p:CoverletOutputFormat=cobertura

# Backend BDD acceptance only (Reqnroll — business-language scenarios over the app services)
cd backend && dotnet test test/Dosi.Tracker.Acceptance.Tests

# Mutation testing (slow — runs nightly in CI; break threshold fails below the floor)
cd backend/test/Dosi.Tracker.Domain.Tests && dotnet stryker   # Domain layer
cd frontend && npx stryker run                                # web logic layer

# Web (from Phase 1)
cd frontend && npm run validate        # typecheck + lint + test
cd frontend && npm run test:coverage

# Rust client
cd clients/windows && cargo test && cargo clippy --all-targets -- -D warnings
```

---

## 10. Lint debt (grandfathered baseline)

The web app carries pre-existing ESLint warnings (48 as of Phase 1, **47 now**), almost all
`react-hooks/set-state-in-effect` on data-fetch effects, plus a few `no-explicit-any` /
`no-unused-vars` in legacy data-shaping code. Rather than blind-refactor untested effects (the exact
risk the gauntlet exists to prevent), these rules are set to **warn** in
`frontend/eslint.config.mjs` and the total is **capped via `npm run lint --max-warnings`** (currently
47). New data-fetch effects added while wiring pages use a line-scoped `eslint-disable` with a
reason, so the cap holds and the debt can only shrink.

Consequences:
- The build is green, but the debt is **visible** and **cannot grow** — a new violation pushes the
  count over the cap and fails CI, so new code is still held to the strict bar.
- **Burn-down:** ratchet the cap down each phase (48 → 20 → 0). Fix effects only once the owning
  component has a test (Phase 3), so the refactor is safe. When the cap reaches 0, delete the
  grandfather block in the eslint config and let the rules return to `error`.

---

## 11. Client testing status

**Windows (Rust) — done.** `cargo test` runs 9 tests including two **property-based** tests
(`proptest`) over the offline queue's core invariants: `pending()` returns exactly the enqueued
rows in FIFO order, and every row ends up uploaded-or-parked with `rejected_count` conserved — for
*any* operation sequence. CI already runs `cargo test --locked` and `clippy -D warnings`.

**macOS (Swift) — deferred, needs a Mac.** The agent is a single SwiftPM executable target;
XCTest cannot import an executable, so testing requires splitting the models/config into a
`DosiTrackerCore` library. That split is a cross-module `public`-API change across ~6 files
(`Models`, `AppConfig`/`CapturePermissions`, `ApiClient`, `Storage`, `Tracker`,
`ActiveWindowMonitor`) and **cannot be compiled or validated on the Windows dev host**, so it was
not shipped blind (a mistake would break the green `swift build` with no way to verify a fix here).

Ready-to-execute plan (run on macOS):
1. New `DosiTrackerCore` library target; move `Models.swift` + `AppConfig.swift` into it; make the
   types + members + memberwise inits `public`; add `import DosiTrackerCore` to the executable files.
2. `.testTarget("DosiTrackerCoreTests")` with: `Activity`/`Project` **Codable round-trip** against the
   backend JSON contract, and `AppConfig.interval` **clamping to 5…60 minutes** (a pure invariant).
3. Add a `swift test` step to the `macos-agent` CI job; `swift build` proves the split first.

---

*This document is updated at the end of each phase with the new baseline and ratcheted thresholds.*
