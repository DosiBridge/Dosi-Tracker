# Dosi-Tracker — Windows agent (Rust)

A lightweight time and activity tracker for Windows 10 and 11. It lives in the
notification area (system tray), records work in **blocks** on the project you
choose, and uploads them to your workspace's Dosi Tracker server. A small window
shows the timer, what was recorded, and what your workspace captures.

- [Install](#install) · [Sign in](#sign-in) · [Using the app](#using-the-app) ·
  [What is collected](#what-is-collected) · [Permissions](#permissions) ·
  [Files and uninstall](#files-and-uninstall) · [Configuration](#configuration) ·
  [How it talks to the server](#how-it-talks-to-the-server) ·
  [Troubleshooting](#troubleshooting) · [Development](#development)

## Install

1. Download `dosi-tracker-setup-<version>.exe` (CI builds it on every push).
2. Run it. It installs **for your Windows user only** — no administrator rights —
   to `%LOCALAPPDATA%\Programs\DosiTracker`, adds a Start menu shortcut, and
   starts Dosi Tracker when you sign in to Windows. If SmartScreen says
   *Windows protected your PC*, choose **More info → Run anyway**.
3. Sign in (below).

Admins can deploy it silently and point it at their server, so nobody has to
type the address:

```powershell
dosi-tracker-setup-0.1.0.exe /S /SERVER=https://tracker.example.com /DASHBOARD=https://app.example.com
```

| Option | Effect |
|---|---|
| `/SERVER=<url>` | The server address (API) the agent signs in to; written to `config.toml` next to the program (`api_base_url`). |
| `/DASHBOARD=<url>` | The web dashboard the app links to (`dashboard_url`). |
| `/S` | Silent install. |
| `/D=<folder>` | Another install folder (must be last). |

Other settings already in that `config.toml` are kept. An invalid address stops
the installer (exit code 2) instead of installing an agent that cannot connect.

Installing over a running agent first asks it to **quit gracefully**
(`dosi-tracker.exe --quit`: the block in progress is saved and uploaded), and
only ends it if it has not quit within 30 s — or if it is an older build that
does not know `--quit`.

## Sign in

Enter your **email** and **password** (Enter submits). The
server address is under **Advanced**: it is filled in from the installer's
`/SERVER` (or `config.toml`), and opens by itself when no server was ever set on
this computer or when the server cannot be reached. Paste the address shown on
the web dashboard's **Download** page; a bare host (`tracker.example.com`)
becomes `https://tracker.example.com`. The address you sign in with is saved
with your sign-in. A plain `http://` address is refused unless it is this
computer (`localhost`, `127.x.x.x`, `[::1]`) — or an admin set
`allow_insecure_http = true` in `config.toml`, in which case the window warns
that the connection is not encrypted.

After signing in the agent lists your workspaces and uses the only one, or
asks; **archived workspaces are never offered**. The sign-in is stored
**encrypted with Windows DPAPI**, keyed to your Windows user. Only the refresh
token the server grants is kept — **not the password**: it is sent once, when
you sign in.

When signing in fails, the form says why:

| Message | Meaning |
|---|---|
| **Can't reach the server** | No answer: wrong address, no network or VPN, a firewall, or a certificate Windows does not trust. Check the address under Advanced. |
| **Check the server address** | What was typed is not a usable address, or it is plain `http://` to another computer. |
| **No workspace yet** | The account is in no workspace, or all of its workspaces are archived. |
| **Sign-in refused** | The server's own words: wrong email or password, a locked-out or deactivated account. |
| **Too many sign-in attempts** | Wait a minute and try again. |
| **The server couldn't sign you in** | The server failed (5xx), answered with a redirect (never followed), or the address is not a Dosi Tracker server (e.g. HTTP 404). |
| **This computer was removed** | A workspace admin removed this computer: tracking stopped and the saved sign-in was forgotten. |

**Sign-in expired.** When the server refuses the saved sign-in (the refresh
token was revoked or expired, the password changed, the account was
deactivated), tracking stops and the window asks for the **password only** —
the email is kept. The agent never signs in again with a password behind your
back. Everything recorded so far stays queued and uploads once you are back.
*Use a different account* signs out instead; that account's unsent blocks stay
on the computer and upload if it signs in again (on the same server) within 14
days.

**Workspace removed or archived.** When the server says you are no longer a
member of the workspace you track in, or that it was archived, tracking and
presence stop there and the window asks for another workspace. The blocks
recorded there are **held** — not sent again — until you track in that
workspace again; held blocks are deleted after 14 days.

**Computer removed.** When a workspace admin removes this computer, tracking,
presence and uploads stop at once and the saved sign-in is forgotten. The
blocks recorded so far stay on the computer (and upload if the computer is
restored and you sign in again).

## Using the app

The window has three tabs — **Timer**, **Activity**, **Settings** — under a
header with the account and a status pill, and a footer with the upload state.
It is resizable (the content scrolls, and switches to two columns when wide)
and opens sized to fit the screen. **Closing the window hides it to the tray**;
tracking goes on until you choose *Quit* from the tray menu.

### Projects

If you are on exactly one project, tracking starts on it. With several, the
project you last chose is used; otherwise — first sign-in, or your project was
archived or you were removed from it — nothing is tracked until you **choose a
project** (the Timer tab lists them, with a filter when there are many; the
tray's *Switch project* menu works too), and you are notified. The list is
refreshed every 5 minutes, so a project's new interval or capture settings
apply from the next block.

### Timer

- **Project** picker and **What are you working on?** — a task note (up to 200
  characters) sent with every block. **Enter** (or leaving the field) applies a
  new note: the block in progress ends and the next starts with it. **Esc**
  cancels the edit.
- The session clock with **Pause / Resume**, and **timed breaks** (15 min,
  30 min, 1 hour) that resume by themselves — once you are at the computer.
  While paused, the same buttons change when the break ends, and *Never* turns
  the auto-resume off. A pause **survives a restart**; after Windows starts the
  app, a break never resumes silently.
- **Today / This week** against your goals, and today's activity %.
- **Current block**: when it is saved, a bar per minute (green = keyboard or
  mouse input), activity %, key presses, clicks, and the app in front — only
  what the project records.
- **Sync now** saves the block in progress (if it is at least a minute old) and
  uploads at once; while paused it only uploads what is queued.

### Time away (idle)

After the project's **idle threshold** without keyboard or mouse input (5 min
by default; the admin can change it or turn it off), the block ends at your
**last input** and nothing is recorded. When you are back a new block starts,
and the window — and a tray notification, if the window is closed — asks:

> **You were away for 42 min (10:05 – 10:47).** [Keep as work] [Discard]

*Keep as work* records that time on the project you were on, as blocks with
no activity (each at most one interval long), with your task note.
*Discard* — or no answer — records nothing.

While you are away (or paused, or locked) the input hooks are off. Your return
is noticed through Windows' own idle clock (`GetLastInputInfo`: when the last
input happened, nothing else). That clock also moves for software-injected
input such as a mouse jiggler, so when the project records keyboard and mouse
counts the agent attaches its input hooks — counting only, nothing else runs —
and the time away ends at your own next input. A timed break that ends while
nobody is at the computer resumes the same way. When keyboard and mouse counts
are not allowed, idle detection relies on Windows' idle clock alone, so time is
still recorded and still ends when you leave.

### Locking, sleep and shutdown

- **Locked screen** (Win+L, the screen saver, fast user switching): the block
  ends when the screen locks; nothing is recorded until it is unlocked.
- **Sleep / hibernate**: the block is saved locally before the computer
  sleeps, and a new one starts when it wakes. A sleep Windows did not announce
  is detected too (the block ends at the last moment before the gap).
- **Shutdown, restart, signing out of Windows**: the block in progress is saved
  locally (Windows waits briefly for it) and uploads on the next start.
- **Quit** saves the block, tells the server you stopped, and uploads for up to
  15 seconds; the rest goes up on the next launch.

### Activity tab

This week day by day (hours per local day against your daily goal), the
**latest screenshot exactly as uploaded** (blurred if the project requires
it), the apps you used in the last hour, and the blocks **recorded this
session** with their status — waiting, uploaded, or **not accepted** with the
**server's reason**. Blocks from earlier sessions that are still waiting or were
not accepted are listed below them.

### Settings tab

- **General**: start with Windows, start in the tray, minimize to the tray, and
  **show notifications** (from the tray while the window is closed).
- **Goals**: daily goal and working days (personal, only on this computer).
- **What is recorded** on the current project — read-only, set by the admin.
- **This device**: computer name, device ID (copy button), agent version,
  Windows version and time zone — what each block carries.
- **Account**: the server and web dashboard in use; sign out (the block in
  progress is saved and uploaded first).
- **Help & support**: open the logs or the data folder; the version.

### Notices in the window

Banners at the top of the Timer tab explain anything that needs attention:
**tracking stopped** (the local queue cannot be opened — retried every minute),
**keyboard and mouse activity can't be measured**, **workspace subscription
inactive** (your work is kept and uploads once it is renewed; retried hourly),
**your computer's clock is off** by more than 2 minutes from the server's,
**uploads waiting** (offline or a server error, with the last error and *Try
again now*), and **blocks the server did not accept** (*See why*). One-off
messages (tracking started with Windows, a project that went away, a damaged
queue set aside) appear briefly at the top, and as tray notifications while the
window is closed.

### The tray icon

The icon's badge shows the state: **green** tracking, **amber** paused, away or
locked, **grey** offline, connecting or choosing a project, **red** stopped or
sign-in expired; the whole icon is grey when signed out. Hover for the state,
project and today's total. **Left click** opens the window. **Right click**:

- the status line and today / this week totals
- Open Dosi Tracker
- **Pause / Resume tracking** (or *Sign in…*, *Choose a project…*)
- **Take a break** → 15 minutes, 30 minutes, 1 hour
- **Switch project** → your projects
- **Set task…** (shows the current note; opens the window at the field)
- **Sync now** (*Upload queued* while paused)
- Open web dashboard · Settings… · ✓ Start with Windows
- Quit Dosi Tracker

Clicking one of its notifications opens the window.

## What is collected

The workspace admin decides per project what is captured; the agent can only
capture **less** (see `config.toml`). Each **block** (the project's interval,
5–60 min; a block never exceeds it, and blocks under a minute are dropped)
carries:

| Data | When | Details |
|---|---|---|
| Start and end, project, task note | always | UTC times; the reason the block ended (interval, pause, break, idle, lock, sleep, project, task, sign-out, quit, shutdown). |
| Keyboard activity | if allowed | **How many** key presses, per minute — never which keys or what you type. A held key counts once. Sent as *not measured* (`null`, never 0) when not allowed or when input capture was unavailable during the block. |
| Mouse activity | if allowed | Click counts; movement and scrolling mark a minute as active. *Not measured* (`null`) like the keyboard. |
| Activity % and idle time | keyboard or mouse allowed | The share of the block's minutes with input, and the seconds without it. Software-injected input (mouse jigglers, remote tools) is counted aside, never as activity. *Not measured* when input capture is unavailable. |
| Apps and window titles | if allowed | The app in front, its window title, how long, and the input while it was in front; per minute, the app mostly in front. |
| Running programs | if allowed | **Names only** (e.g. `chrome.exe`) — never the titles of background windows. |
| Screenshot | if allowed | One per block, at a random moment between 10% and 90% of the interval, of the **monitor holding the window you work in**; JPEG at most 1920 px wide, plus a 480 px thumbnail. **Blurred on this computer** (no text readable) when the project requires it. None for blocks that end by lock, sleep, idle or shutdown. |
| Webcam photo | if allowed | One per block (after the camera warms up; given up after 5 s). |
| Device | always | A random device ID created at install, the computer name, Windows version, agent version, time zone and UTC offset, and the computer's clock when the block is sent. |

While signed in, the agent also tells the server every minute (and whenever it
changes) whether you are **tracking, paused, on a break, away, locked or
choosing a project**, with the device details above — and the app in front
only if the project records it. Nothing of this is queued.

**Never collected:** what you type, passwords, the clipboard, microphone
audio, files, or the window titles of programs in the background.

## Permissions

- **No administrator rights**: per-user install, per-user autostart
  (`HKCU\…\Run`), and capture in your own session.
- Windows does not prompt desktop apps for screen or input access. The agent
  counts input with low-level keyboard and mouse hooks and captures the screen
  with GDI (no yellow capture border).
- **Camera** (only if a project takes webcam photos): *Settings → Privacy &
  security → Camera → Let desktop apps access your camera* must be on.
- **Notifications**: *Settings → System → Notifications* (and Do not disturb)
  decide whether the tray notifications appear; the window shows them anyway.
- **Network**: HTTPS to your server. Certificates are always verified, against
  the Windows certificate store (so a company CA deployed to Windows works) and
  the bundled Mozilla roots. Plain `http://` only to this computer, unless an
  admin sets `allow_insecure_http`; redirects are never followed.

## Files and uninstall

| Where | What |
|---|---|
| `%LOCALAPPDATA%\Programs\DosiTracker\` | `dosi-tracker.exe`, `config.toml` (if written by the installer), `uninstall.exe` |
| `%APPDATA%\DosiTracker\credentials.bin` | the sign-in, DPAPI-encrypted, with the chosen project, the last project list (for offline starts), a pause and the task note |
| `%APPDATA%\DosiTracker\preferences.json` | your preferences (window behaviour, notifications, goals) |
| `%LOCALAPPDATA%\DosiTracker\dosi-tracker.db` | the offline queue (SQLite), with screenshots waiting to upload |
| `%LOCALAPPDATA%\DosiTracker\logs\` | daily logs, kept 7 days |
| `%LOCALAPPDATA%\DosiTracker\device.json` | the device ID |

Builds before this one kept the queue and logs in `%APPDATA%`; they are moved to
`%LOCALAPPDATA%` on first start (so roaming profiles do not copy them around).

**Uninstall** from *Settings → Apps* (or `uninstall.exe`). It asks the agent to
quit gracefully, removes the program, shortcut and autostart entry, and then
**asks whether to delete your data** (sign-in, preferences, logs, and any time
not uploaded yet). A silent uninstall (`/S`) deletes it; add `/KEEPDATA` to
keep it.

## Configuration

`config.toml` next to `dosi-tracker.exe` (or, when there is none, in the working
directory — handy for `cargo run`); every key is optional. See
[`config.example.toml`](config.example.toml).

| Key | Default | |
|---|---|---|
| `api_base_url` | `https://localhost:6006` | the server; an address typed at sign-in wins |
| `allow_insecure_http` | `false` | allow plain `http://` to a server on another computer (the window warns); `http://localhost` is always allowed |
| `dashboard_url` | `http://localhost:6005` | the web dashboard; greyed out in the app when left at this local default while the server is elsewhere |
| `interval_minutes` | `10` | until the project's interval is known (5–60) |
| `[capture]` | all on but `webcam` and `blur_screenshots` | can only turn captures off (or blur on) |

Any key can also be set as an environment variable, e.g. `DOSI__API_BASE_URL`,
`DOSI__CAPTURE__WEBCAM=false`. Set `RUST_LOG=debug` for verbose logs.

## How it talks to the server

| | |
|---|---|
| Sign-in | OpenIddict password grant, `POST /connect/token` (`client_id=Tracker_App`, scope `openid profile email roles offline_access Tracker`, no `__tenant`), with `device_id` (this install's UUID) on the password and every refresh grant. Then `GET /api/app/workspaces/mine`; every workspace-scoped call carries `X-Workspace-Id`. The token is renewed with the refresh token before it expires and once after a 401; a refused refresh token means *sign-in expired*. |
| Projects | `GET /api/app/project/my-projects` every 5 minutes and after a 403/409 from an upload: interval, capture permissions, idle threshold, blur. |
| Upload | `POST /api/app/activity`, one block at a time, oldest first, on a thread of its own (so pausing, locking or quitting never waits for a slow upload). |
| Presence | `POST /api/app/presence/heartbeat` every 60 s and on every status change; "stopped" / "signed out" when quitting or signing out. |
| Totals | `GET /api/app/reporting/summary` for today / this week, with your time zone so days are your days. |
| Timeouts | 10 s to connect; 30 s per request; 120 s per upload. |

What an upload's answer does (the blocks are always saved locally first):

| Answer | What happens |
|---|---|
| 2xx | uploaded; removed from the queue |
| network error, timeout, a redirect, 5xx, 404/405/408/429, a 4xx without an error code | kept; retried after 30 s, doubling up to 30 min (`Retry-After` honoured) |
| sign-in refused | kept; uploads stop and the window asks for the password |
| 402 subscription inactive | kept; retried hourly |
| 403 device revoked | kept; uploads, tracking and presence stop and the saved sign-in is forgotten |
| 403 not a workspace member / workspace archived, 400 workspace required | kept and **held** with the rest of that workspace's blocks (not sent again until the workspace is used again); the other blocks go on; the window asks for another workspace |
| 403 not a project member / 409 project archived | the project list is fetched again; not accepted only if the project is really gone |
| 425 in the future | kept; retried in 5 minutes (your clock is ahead) |
| 413, capture too large or invalid | sent again at once without its images |
| an error code this agent does not know, or still too large without images | kept; **that block alone** is retried later (5 min, doubling up to 6 h) while the others go on; not accepted after 10 tries |
| overlap, id conflict, too long, too old, invalid range, validation errors | **not accepted**: kept 14 days with the server's reason, never retried |

**Offline**: tracking goes on with the project list saved in the last 7 days;
blocks upload when the server answers again (*Sync now* retries at once). The
queue keeps every block's time data; screenshots are dropped first — beyond the
newest 500 blocks, 300 MB of images, or 30 days — and beyond 10,000 waiting
blocks the oldest are dropped. Each block belongs to the account — the user
**on the server** — that recorded it: it uploads only under that account's
token and only to that server, never to another server where the same email
signs in. Another account's blocks are kept 14 days for it.

## Troubleshooting

- **"Can't reach the server"** — compare the address under *Advanced* with the
  one on the web dashboard's Download page. Open it in a browser
  (`<address>/swagger` or any page): if the browser warns about the certificate,
  so will the agent — install your company's CA in Windows, or for local
  development run `dotnet dev-certs https --trust` as the same Windows user and
  restart the agent.
- **"Sign-in refused" with the right password** — check the server address
  under *Advanced*: the same email can have another password on another
  server.
- **Nothing is tracked** — the status pill says why: *Choose a project*, *Paused*
  (a pause survives restarts — press Resume), *Away* (type or click: a mouse
  jiggler alone does not end it),
  *Locked*, *Sign-in expired*, or *Offline* (no server and no saved project
  list yet).
- **The time-away question keeps appearing in meetings** — idle detection ends
  the block after the project's threshold without input; *Keep as work*
  records the time. The admin can raise the threshold or turn it off per
  project.
- **Activity is low, or *Away* appears, while you work in an app run as
  administrator** — Windows does not pass input in elevated windows to normal
  apps. Run that app without administrator rights if you can.
- **"Keyboard and mouse activity can't be measured"** — Windows stopped passing
  input to the agent; it reinstalls its hooks by itself. Time is still recorded
  (activity *not measured*). Restart the agent if it lasts.
- **"Your computer's clock is off"** — turn on *Set the time automatically* in
  *Settings → Time & language → Date & time*. Times are never rewritten; a clock
  far ahead holds uploads back until the server catches up.
- **Blocks "not accepted"** — the Activity tab shows the server's reason (for
  example an overlap with a block from another computer). They are kept 14 days
  for reference.
- **The web dashboard button is greyed out** — the dashboard's address is not
  set on this computer: reinstall with `/DASHBOARD=<url>` or set `dashboard_url`
  in `config.toml`.
- **No tray notifications** — check *Settings → System → Notifications* (and Do
  not disturb) in Windows, and *Show notifications* in the app.
- **Two copies?** — only one agent runs per Windows user; starting it again
  brings the running one to the front.
- **Logs** — *Settings → Help & support → Open logs* (attach the latest file
  when reporting a problem).

## Development

### Prerequisites

- **Rust** — `winget install Rustlang.Rustup` (or https://rustup.rs)
- **MSVC C++ build tools + Windows SDK** — needed by the `windows`, `rusqlite`
  (bundled SQLite) and `nokhwa` crates:
  ```powershell
  winget install --id Microsoft.VisualStudio.2022.BuildTools `
    --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
- **NSIS** (only for the installer) — `winget install NSIS.NSIS`

### Build, test, lint

```powershell
cd clients/windows
copy config.example.toml config.toml   # optional: point it at your server
cargo run                              # or cargo run --release
cargo test
cargo clippy --all-targets -- -D warnings   # CI enforces this
```

### The installer

```powershell
cargo build --release
cd installer
makensis /DVERSION=0.1.0 dosi-tracker.nsi   # -> dosi-tracker-setup-0.1.0.exe
```

CI takes the version from `Cargo.toml`, builds the installer on every push and
uploads it as a workflow artifact.

### Reviewing the window without a backend

A debug build runs on demo data with no worker (the saved sign-in is not
touched):

```powershell
$env:DOSI_UI_PREVIEW = "main"     # or "paused", "signed-out"
$env:DOSI_UI_VIEW = "activity"    # optional: open on "activity" or "settings"
$env:DOSI_UI_SCENE = "away"       # optional, see below
cargo run
```

Scenes: `away` (the time-away question), `choose` / `choose-many` /
`no-projects`, `alerts` (input unavailable, subscription inactive, clock ahead,
a rejected block), `offline`, `stopped`, `expired`, `signin-error` /
`signin-refused` (with `signed-out`), `queue` (rejected and waiting blocks on the
Activity tab), `privacy` (blurred screenshots, idle detection off), `notices`.

A debug build can also sign in by itself for end-to-end runs:
`DOSI_WORKSPACE`, `DOSI_USERNAME`, `DOSI_PASSWORD`, optional `DOSI_SERVER`, and
`DOSI_AUTO_SYNC` to save and upload at once.

### Architecture

| Part | Owns |
|------|------|
| UI thread | the egui window and the tray icon; renders a cheap state snapshot (`state::SharedState`) and sends commands (`state::Command`) |
| Worker thread (`worker.rs`) | when blocks start and end — from commands, session and power events, input and the clocks — and queues them locally; never waits on the network |
| Control task (`worker/control.rs`) | sign-in, the project list, presence, totals |
| Uploader thread (`worker/uploader.rs`) | drains the queue with its own connection to it |
| Session events (`session_events.rs`) | a hidden window hearing lock/unlock, sleep/resume and shutdown |
| Input hooks (`tracking/input.rs`) | low-level keyboard/mouse hooks that only timestamp events, with a watchdog |

```
src/
├── main.rs              # entry: window options, wiring, --quit, UI preview seed
├── ui/
│   ├── mod.rs           # TrackerApp: window behaviour, tray events, notices, layout
│   ├── login.rs         # sign-in, server address (Advanced), sign-in expired
│   ├── timer.rs         # Timer tab, project chooser, task note, time-away question
│   ├── activity.rs      # Activity tab (blocks, upload status, server reasons)
│   ├── settings.rs      # Settings tab (privacy, this device, account)
│   ├── alerts.rs        # what needs attention (banners), decided from the state
│   ├── preview.rs       # debug-only preview scenes
│   ├── widgets.rs       # painted icons, buttons, toggles, callouts, charts
│   └── format.rs        # shared formatting (clock, durations, app names)
├── tray.rs              # tray icon: badge, tooltip, menu, notifications
├── theme.rs             # palette (web design tokens) + Segoe UI fonts
├── prefs.rs             # local preferences
├── worker.rs            # the worker loop (+ worker/: blocks, session, control, uploader)
├── state.rs             # shared snapshot + commands
├── session_events.rs    # lock / sleep / shutdown events
├── power.rs             # awake-time clock (missed-suspend detection)
├── single_instance.rs   # one agent per user; --quit and "show" signals
├── shell.rs             # folders, opening URLs, work area, window visibility
├── credentials.rs       # DPAPI-encrypted sign-in
├── autostart.rs         # "Start with Windows" (HKCU Run key, --autostart)
├── config.rs            # config.toml + DOSI__* env vars, server address rules
├── model.rs             # wire types (Project, Activity, Heartbeat, …)
├── api.rs               # REST client: token, projects, upload, presence, totals
├── storage.rs           # the offline SQLite queue
└── tracking/
    ├── mod.rs           # Tracker: builds a block's upload
    ├── input.rs         # input hooks, activity %, idle
    ├── usage.rs         # foreground-window usage, per-minute timeline
    ├── screenshot.rs    # foreground-monitor JPEG capture, thumbnail, blur
    ├── webcam.rs        # one webcam photo, with warm-up and timeout
    ├── active_window.rs # foreground window, running programs
    └── system.rs        # device id, computer name, Windows version, time zone
```

## Known gaps

- No automatic updater — new versions are installed by running the setup again.
- A worker that panics is reported (red *Stopped*) but not restarted
  automatically; restarting the app recovers.
- Autostart uses the per-user Run key, not a Windows service: capture needs the
  user's interactive session, which a session-0 service cannot reach.
- Input to apps running as administrator is not counted (see Troubleshooting).
