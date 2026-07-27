<!--
  This checklist is the human half of the quality gauntlet (docs/QUALITY.md). CI enforces the
  automated gates; this covers what a machine can't. Tick what applies; strike through (~~…~~) what
  genuinely doesn't, with a one-line reason.
-->

## What & why

<!-- One or two sentences: what changed and why. Link the issue. -->

## Definition of Done (docs/QUALITY.md §5)

- [ ] New/changed behavior has tests at the **lowest level that can prove it** (unit first).
- [ ] A user-facing flow change has an **acceptance (`.feature`) scenario**.
- [ ] `typecheck` + `lint` + `format` pass (web) · `dotnet build` clean (backend) · `clippy -D warnings` clean (Rust).
- [ ] The touched tier's full test suite is **green locally**.
- [ ] **Coverage did not drop** below the current gate (backend line ≥ 80% / branch ≥ 45%; web logic line ≥ 95% / branch ≥ 85%).
- [ ] No **new mutation survivors** in touched core logic (checked if the nightly Stryker run is red for this area).
- [ ] The frontend **lint-warning cap was not raised** (only lowered, docs/QUALITY.md §10).

## QA (for release-affecting changes, docs/QUALITY.md §7)

- [ ] Migrator runs clean on a fresh DB; API health green.
- [ ] Route protection / auth still redirect correctly.
- [ ] Windows client: sign-in → tracking → capture upload → tray controls smoke-tested.
- [ ] No template dev secret is live in the target environment.

## Notes for the reviewer

<!-- Anything non-obvious: trade-offs, follow-ups, deliberately-skipped gates (with reasons). -->
