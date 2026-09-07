# Dosi-Tracker Web — Manual QA Checklist

> The human half of the gate (QUALITY.md §7). Automated layers cover regression; this checklist
> covers what a machine can't feel — and the few flows that genuinely need a live backend. Run it
> per release, and the relevant sections per risky change. Record the run (date, commit SHA,
> tester) in the release notes.

All commands run from `frontend/` unless noted. Demo personas are switched via the workspace/user
mechanisms inside the app; where a step says "demo mode", it means: the `dosi-token` **cookie** is
present but no JWT is in localStorage, so pages render seeded demo data (frozen clock
2026-07-14). To enter demo mode without a backend, set the cookie in devtools:
`document.cookie = "dosi-token=1; Path=/"` — then reload.

---

## 1. Install & boot

- [ ] `npm ci` completes clean (no peer-dependency errors, lockfile untouched afterward: `git status`).
- [ ] `npm run build` succeeds with no errors.
- [ ] `npx next start -p 3123` serves the production build; `http://localhost:3123` responds.
- [ ] `npm run validate` is green (typecheck + lint cap + tests + architecture).
- [ ] `npm audit --audit-level=critical` passes. Note the **high** count in the release record — the
      gate is deliberately set at `critical` while `next@16.2.10` drags in sharp/libvips advisories
      (strategy §2); if that count moved, find out why.

## 2. Demo login & session

- [ ] Visiting any app route without the cookie redirects to `/login`.
- [ ] With the cookie set, `/login` redirects to `/dashboard` (middleware inverse gate).
- [ ] Real backend up: sign in with a tenant workspace name → lands on `/dashboard`; sign in with
      the workspace field **empty** (host account) → lands on `/host`.
- [ ] Bad credentials show "Invalid username, password or workspace" — no crash, no redirect.
- [ ] Signup flow: "Create a workspace" → tenant created → logged in as its owner on `/dashboard`.
- [ ] **Hydration must not erase demo data.** Sign in against a backend that is up, then stop it and
      reload while the token is still in localStorage: pages keep showing seeded tenant data with
      the honest demo notice. An unreachable backend must never blank out projects/activities into
      empty lists.

## 3. Navigation — expected sidebar per role

Switch persona per role and verify the sidebar shows exactly the expected items (source of truth:
`src/lib/roles.ts` `navAccess`):

- [ ] **Host**: platform console only — Overview, Tenants, Users, Plans, Billing (under `/host`);
      no tenant nav (Dashboard/Projects/etc.).
- [ ] **Owner**: Dashboard, Projects, Team, Monitor, Activities, Timesheet, Reports, Billing, Settings.
- [ ] **Admin**: as Owner **minus Billing**.
- [ ] **Worker**: Dashboard, Projects, Monitor, Activities, Timesheet, Settings — no Team, no
      Reports, no Billing.
- [ ] **Client**: Dashboard and Settings only.
- [ ] Active route is visually highlighted; no dead links (every item navigates).

## 4. Tenant operations

- [ ] Switch workspace w1 (Dosi Labs) → w2 (Acme Studio) → w3 (Nimbus Co); the header/switcher
      reflects the active tenant and the identity follows (you become that tenant's owner).
- [ ] Create a workspace from the switcher; it appears in the list and survives a page reload.
- [ ] Rename a workspace; new name shows in switcher and header after reload.
- [ ] Delete a created workspace; it disappears and the app falls back to a remaining one.
- [ ] **Isolation spot-check**: note 2–3 member names and project titles in w1, switch to w2 —
      none of them appear anywhere (team, projects, activities, reports).

## 5. Roles & permissions — direct-URL probes

Type URLs directly into the address bar (not via nav) per role; expect a redirect or an
access-denied state, never the page content:

- [ ] Worker → `/reports`, `/team`, `/billing`, `/host` — all blocked.
- [ ] Client → `/projects`, `/activities`, `/monitor`, `/timesheet`, `/reports`, `/billing` — all blocked.
- [ ] Admin → `/billing`, `/host` — blocked; `/reports`, `/team` — allowed.
- [ ] Owner → `/host` — blocked; everything tenant-side allowed.
- [ ] Host → `/dashboard` (tenant-side) — blocked/redirected to `/host`.
- [ ] Legacy `/screenshots` redirects to `/activities?view=screens`; `/insights` redirects to `/dashboard`.

## 6. Modules per plan

On `/billing` per workspace, verify the usage meters against the plan (source:
`src/lib/saas-data.ts`):

- [ ] **w3 Nimbus Co (Free)**: limits show 3 seats / 2 projects / 7-day retention / 1 GB.
- [ ] **w2 Acme Studio (Starter, trialing)**: 10 seats / 10 projects / 30-day / 20 GB; trial state
      is visible and dated relative to the frozen demo clock.
- [ ] **w1 Dosi Labs (Business)**: 50 seats / unlimited projects / 180-day / 500 GB.
- [ ] Meters show used vs. limit and don't render nonsense for unlimited (no "∞%" or NaN). Check a
      **zero limit** too (a metric whose plan allowance is 0) — the bar must read 0% or 100%, never
      "NaN%".
- [ ] Plan comparison table renders all four plans with correct prices ($0 / $6 / $12 / custom).

## 7. Members / team

- [ ] `/team` lists members with role labels (Owner/Administrator/Member/Client) and statuses.
- [ ] Invite dialog: an **owner** can assign worker/admin/client; an **admin** can assign only
      worker/client (no owner or admin escalation offered).
- [ ] Invite validation: submit `not-an-email`, then `ok@x.com, broken@, also@fine.com`. Each
      attempt is refused with a message that **names the offending address**; valid entries in the
      same list are not silently sent.
- [ ] Role change and deactivate/reactivate update the list immediately.
- [ ] Seat count matches the billing meter (clients don't consume seats).

## 8. Projects lifecycle

- [ ] Create a project (name, color, capture permissions); it appears in the list and in the
      project pickers elsewhere.
- [ ] **Persistence across a real reload.** After creating it, press F5 (and separately: close the
      tab, reopen `/projects`) — the project is still there. jsdom can only simulate this; a real
      browser is the only place the localStorage round-trip is genuinely proven.
- [ ] **Cancel discards the draft.** Open the create wizard, type a name and advance a step, then
      Cancel — and also close via the X and via Esc. Reopening shows an **empty** form every time,
      not the abandoned input.
- [ ] Edit and archive; archived projects drop out of default lists but remain reachable.
- [ ] Project detail page renders members, logged-time stats, and per-project permission toggles.
- [ ] Assign/unassign a member; reflected on both project and member views.

## 9. Activities & screenshots

- [ ] `/activities` renders activity cards with productivity, clicks/keystrokes, and window info.
- [ ] In demo mode the page shows **seeded activities, not an empty state** — "No activities match
      your filters" on a fresh load with default filters is a bug, not an empty dataset. Confirm as
      an owner (whole team visible) and as a worker (own rows only).
- [ ] Screens view (`?view=screens`) renders the capture grid.
- [ ] In demo mode, screenshot/live-data areas show the **honest demo notice** ("showing demo
      data" wording) — they must not silently pretend data is live.
- [ ] Date navigation respects the frozen demo clock — "today" is 2026-07-14 in demo mode.

## 10. Reports

- [ ] All 7 reports open and render from `/reports`: Time & Activity, Productivity, Apps &
      Websites, Project Breakdown, Attendance & Shifts, Weekly Summary, Payroll & Billing.
- [ ] Date presets (e.g. this week / last week / this month) change the rendered data.
- [ ] Charts render without console errors; empty ranges show an empty state, not a crash.
- [ ] Export (where offered) downloads a file with plausible contents.

## 11. Responsive

At 375 px, 768 px, 1280 px, 1920 px widths:

- [ ] Sidebar collapses/overlays sensibly at narrow widths and is openable/closable.
- [ ] Wide tables (team, activities, reports) **scroll horizontally within their container** —
      content is never clipped and the page body never scrolls sideways.
- [ ] Modals fit the viewport and remain fully usable (buttons reachable) at 375 px.
- [ ] No overlapping text or controls at any of the four widths on Dashboard, Team, Reports, Billing.

## 12. Accessibility — manual pass

The Playwright gate (`e2e/a11y.spec.ts`) already fails CI on serious/critical axe violations across
`/login`, `/dashboard`, `/projects`, `/team` and `/host` — **in light theme only**. This section
covers what it cannot: judgment, keyboard feel, and everything dark.

- [ ] **Keyboard-only tour**: Tab through login → dashboard → open a modal (focus trapped inside,
      Esc closes, focus returns to the trigger) → open the command palette → navigate a table.
      Nothing requires a mouse.
- [ ] Focus is **visible** on every interactive element (no `outline: none` dead zones).
- [ ] Heading hierarchy is sane per page (one h1, no level skips) — check with a headings
      bookmarklet or the axe devtools extension.
- [ ] Status information is not conveyed by **color alone** (productivity levels, online/offline,
      trial states carry a label or icon too).
- [ ] **Badges and status pills are legible in BOTH themes.** Toggle light ↔ dark and read every
      variant (success / warning / danger / info / muted) on `/team`, `/billing` and `/host`. The
      light values are gated automatically; the dark `--*-on-tint` tokens are **not scanned by
      anything** — this checkbox is the only gate they have. Run the axe devtools extension over one
      dark page while you are there.
- [ ] Screen-reader spot-check on the recently-named controls: every dialog and drawer announces a
      title, progress bars announce a value, and the sidebar collapse toggle, toolbar search field
      and password reveal all have names (not "button").
- [ ] Zoom to 200%: no loss of content or function.

## 13. Browsers

Smoke pass (login-or-demo-entry, dashboard, one report, one modal) in:

- [ ] Chrome
- [ ] Firefox
- [ ] Safari
- [ ] Edge

## 14. Performance sanity

- [ ] Scroll a large table/list (activities or host users) — no obvious jank or multi-second hangs.
- [ ] Route transitions feel instant on the production build (no white flashes, no layout jumps).
- [ ] `npm run build:budget` passes (single chunk ≤ 480 KB, total ≤ 1900 KB) and the "largest
      chunks" listing contains no surprises. Baseline for comparison: 35 chunks, 1552 KB total,
      390 KB largest — a jump toward the budget ceiling wants explaining even though it passes.

## 15. Error handling

- [ ] **Corrupt every `dosi-*` key, one at a time**, in devtools → reload after each → the app
      recovers to a sane default (owner Ayesha Rahman in Dosi Labs); no white screen, no crash loop.
      Work through the full list: `dosi-user`, `dosi-workspace`, `dosi-workspaces-created`,
      `dosi-workspaces-patches`, `dosi-workspaces-deleted`, `dosi-projects-created-w1`,
      `dosi-impersonating`, `dosi-theme`, `dosi-token`, `dosi-tenant`. Use more than one shape of
      garbage — `{`, `{}`, `[]`, `null`, `0` and a very long string all behave differently, and the
      wrong-type-but-valid-JSON cases (`{}` where an array is expected) are the ones that used to
      crash. The automated gauntlet covers this in jsdom; here you are proving it in a real browser.
- [ ] Set `dosi-workspace` to a nonexistent id → reload → the app falls back rather than rendering
      an empty shell.
- [ ] Fill localStorage to its quota, then create a project/workspace → the app degrades with a
      message instead of throwing (this path is **not** automated — see `TEST_MATRIX.md`).
- [ ] Kill/stop the backend while signed in with a live JWT → pages fall back to demo data **with
      the honest notice visible** ("Live stats unavailable — showing demo data" or equivalent);
      nothing pretends to be live.
- [ ] A 401 from the backend (expired token) clears the session and lands on `/login`.

## 16. Storage

- [ ] Log out → `dosi-token` (localStorage **and** cookie), `dosi-tenant`, and `dosi-user` are
      cleared; protected routes redirect to `/login` again.
- [ ] Theme preference (`dosi-theme`) persists across logout/login — it's a device preference, not
      session state.
- [ ] Log in again after logout: no stale identity or workspace leaks from the previous session.
