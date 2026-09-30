# Dosi-Tracker — Windows Client (Rust)

A lightweight background activity-tracking agent for Windows, with a system-tray
presence and a small desktop window.

## The app

- **Sign-in window** — email/password against your workspace. The sign-in is
  persisted **encrypted with Windows DPAPI** (keyed to your Windows user), so the
  password is never stored in clear text or in environment variables.
- **Status view** — a live session stopwatch with the countdown to the next
  snapshot and the block in progress so far (activity %, key presses, clicks);
  tracked today / this week (ticking live) / today's activity %, the current
  project (switchable), snapshot interval and pending-upload count.
- **Pause / Resume / Sync now** from the window or the tray menu. Pausing ends
  the current block and **saves it** (queued and uploaded), so no worked time is
  lost; nothing is captured while paused — the window's button becomes
  **Upload queued**, which only uploads what was already recorded. Input made
  while paused or signed out never counts towards the next block.
- **Timed pauses** — "take a break for" 15 min / 30 min / 1 hour (also in the
  tray: 30 min / 1 hour) resume tracking automatically; while paused the same
  buttons re-arm the auto-resume and **Never** turns it off. The window shows
  how long the pause has lasted and when it ends. Pause/resume show at once
  ("Pausing…") even while an upload is still running.
- **Settings** — "Start when I sign in to Windows" toggle and sign-out. Capture
  permissions are set per project by the workspace admin, not here.
- **Closing the window hides to the tray**; tracking continues until you choose
  Quit from the tray menu.

## Why Rust

- Small idle footprint (no runtime/GC) and a self-contained binary.
- Event-driven input listening → ~0% CPU while idle.
- The UI uses the lightweight OpenGL (`glow`) egui backend rather than wgpu, so
  the agent stays modest in memory for a background app.

## Architecture

Two threads, so the window never blocks on network or capture work:

| Thread | Owns |
|--------|------|
| UI     | egui window + tray icon; renders a cheap state snapshot |
| Worker | auth, capture, the offline SQLite queue, and sync |

They communicate through a command channel (`state::Command`) and a shared
snapshot (`state::SharedState`).

## What it does

Once per interval (5–60 min) it captures a snapshot for the active project:

- Screenshot (primary monitor), plus a small JPEG thumbnail (480 px wide) for
  the dashboard's lists
- Webcam frame (optional)
- **App & window usage** — the foreground window is sampled every 3 s, so each
  block reports every (app, window title) that was in front with its seconds
  and the key presses / clicks made while it was, most-used first (≤ 50)
- **Per-minute timeline** — for each minute: key presses, clicks, active or
  idle, and the app in front the longest
- Running programs
- Keyboard / mouse **counts** (never keystroke content)
- **Activity %** (sent as `productivity`, 0–100): the share of the block's
  whole minutes, counted from the block start, that had any keyboard or mouse
  input (clicks, movement, wheel). Only input kinds the project allows count.
  The timeline's active minutes use the same buckets, so they always agree.

Snapshots are stored locally first (SQLite, offline-first) and then synced to
the backend REST API. Unsynced snapshots survive restarts and network outages.

## How it talks to the backend

- **Auth:** OpenIddict resource-owner password flow — `POST /connect/token`
  with `client_id=Tracker_App`, `scope=Tracker`. The bearer token is cached and
  refreshed via re-login shortly before expiry (401s also force a re-login).
- **Projects:** `GET /api/app/project/my-projects` returns the projects you are
  a member of, including the per-project capture permissions the server
  enforces (screenshot/webcam/keyboard/mouse/active window/running programs)
  and the snapshot interval. The user picks the project in the window (the choice
  is remembered); without any project it captures nothing and retries next interval.
- **Totals:** `GET /api/app/reporting/summary` powers the today / this-week cards.
  "Today" and "this week" (Monday-based) start at **local** midnight.
- **Upload:** `POST /api/app/activity`. Rows the server permanently rejects
  (validation, unknown project) are parked locally (`synced = 2`, with the
  server's reason) so they never block the queue; transient errors are retried.
  Successfully synced rows are deleted, so the local DB stays small. The queue
  is read 20 rows at a time (at most 100 uploads per pass, so a large backlog
  never stalls Pause/Quit for long).
- **Per-account queue:** every queued row is tagged with the account that
  captured it (workspace + email). Only the signed-in account's rows are ever
  uploaded; another account's rows wait for it to sign in again and are purged
  after 14 days. Rows queued by older builds are assigned to the account that
  was signed in when the new build first starts (or, if nobody was, are never
  uploaded and expire after 14 days).
- **Timeouts:** 10 s to connect, 60 s per request (uploads carry screenshots).

## Project layout

```
src/
├── main.rs              # entry: window + tray icon + wiring
├── app.rs               # egui views (login, status, settings)
├── worker.rs            # background thread: auth, capture, queue, sync
├── state.rs             # shared snapshot + command channel types
├── credentials.rs       # DPAPI-encrypted sign-in persistence
├── autostart.rs         # "start with Windows" (HKCU Run key)
├── config.rs            # config.toml + DOSI__* env vars
├── model.rs             # shared data types (Project, Activity, ...)
├── api.rs               # backend REST client (token, projects, submit, summary)
├── storage.rs           # local SQLite queue (offline-first)
└── tracking/
    ├── mod.rs           # Tracker: builds one Activity snapshot
    ├── input.rs         # event-driven keyboard/mouse event log + activity %
    ├── usage.rs         # foreground-window sampler, per-app usage + minute timeline
    ├── screenshot.rs    # primary-monitor PNG capture + JPEG thumbnail
    ├── webcam.rs        # single webcam JPEG frame
    └── active_window.rs # foreground window + running programs (Win32)
```

Data lives in `%APPDATA%\DosiTracker` (`credentials.bin`, `dosi-tracker.db`), not
the working directory — the agent autostarts from arbitrary locations.

## Prerequisites

- **Rust** — `winget install Rustlang.Rustup` (or https://rustup.rs)
- **MSVC C++ build tools + Windows SDK** — required by the `windows`, `rusqlite`
  (bundled SQLite) and `nokhwa` crates:
  ```powershell
  winget install --id Microsoft.VisualStudio.2022.BuildTools `
    --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
- **NSIS** (only to build the installer) — `winget install NSIS.NSIS`

## Installer

A **per-user** installer (no administrator rights) that installs to
`%LOCALAPPDATA%\Programs\DosiTracker`, adds a Start Menu shortcut, enables
start-at-login, and registers an Add/Remove Programs entry:

```powershell
cargo build --release
cd installer
makensis /DVERSION=0.1.0 dosi-tracker.nsi   # -> dosi-tracker-setup-0.1.0.exe
```

CI builds this on every push and uploads it as a workflow artifact.

## Build & run

```powershell
cd clients/windows
copy config.example.toml config.toml   # optional: change the API URL / interval

cargo run --release
```

Then sign in through the window — no environment variables required.

`config.toml` is read from **next to the executable** first (the installed
location — the agent autostarts with an arbitrary working directory) and only
then from the working directory, which is what `cargo run` picks up. Every key
is optional: a file that sets only `api_base_url` keeps the defaults for the
rest. Any key can also be set as an env var, e.g. `DOSI__API_BASE_URL`.

```powershell
cargo clippy --all-targets -- -D warnings   # lints (CI enforces this)
```

## Notes

- Sign-in is stored **DPAPI-encrypted** in `%APPDATA%\DosiTracker\credentials.bin`,
  readable only by the same Windows user on the same machine.
- **Logs:** `%APPDATA%\DosiTracker\logs\dosi-tracker.<date>.log`, rotated daily and
  capped at 7 files. Set `RUST_LOG=debug` for verbose output. (Release builds are
  a GUI subsystem app with no console, so file logging is the only diagnostic
  trail there.)
- **Single instance:** a named mutex guarantees one agent per user session.
  Launching a second copy focuses the existing window and exits. This is a
  correctness guard, not just polish — two agents would each upload activities
  under their own idempotency keys, double-counting tracked (billable) time.
- **Capture timing:** the tick uses `MissedTickBehavior::Delay` and each block is
  timed from the previous capture, clamped to one interval. A slow sync or a
  machine suspend can therefore never burst-fire and invent tracked time.
- **Short blocks:** a block under one minute is never recorded on its own —
  **Sync now** leaves it running (and only uploads the queue), while pausing,
  switching project, signing out or quitting drops it. Longer blocks are always
  saved at those moments, and after a manual sync the next automatic snapshot
  is a full interval away.
- **Quitting** queues the block in progress and waits up to 20 s for the upload;
  anything not uploaded by then goes up on the next launch.
- **Rejected uploads** are parked for 14 days (visible in the window), then purged
  so the local queue cannot grow without bound. Parked rows keep their time data
  and the server's reason, but not their screenshot/webcam images.
- **Offline queue bounds:** screenshots are the bulk of the queue, so they go
  first. Pending rows older than 30 days, or beyond the newest 500 (~80 h of
  tracking at a 10-minute interval), lose their screenshot/webcam images but
  keep their time data (start/end, counts, activity %, windows). Beyond 10,000
  pending rows the oldest are dropped outright.
- If the background worker ever panics, the window shows a red **Stopped** state
  instead of a green "Tracking" pill that captures nothing.
- `running_programs` enumerates visible top-level windows (`EnumWindows`),
  deduplicated by owning process.
- **TLS:** certificates are always verified. The agent (rustls) trusts the
  **Windows certificate store** plus the bundled Mozilla roots, so a corporate
  CA deployed to Windows works without extra setup. Dev: the backend's
  `https://localhost:6006` uses the ASP.NET dev certificate — run
  `dotnet dev-certs https --trust` once (as the same Windows user) and restart
  the agent. Untrusted or self-signed certificates that are not in the store
  are refused.

## Known gaps

- Screenshots capture the **primary monitor only** (multi-monitor users get one
  screen).
- No automatic updater — new versions are installed by re-running the setup.
- A panicked worker is reported to the user but not auto-restarted (restarting
  the app recovers; auto-restart risks a panic loop).
- Autostart uses the per-user Run key rather than a Windows Service. That is
  deliberate: screen/input capture requires the user's interactive session, which
  a session-0 service cannot access.
