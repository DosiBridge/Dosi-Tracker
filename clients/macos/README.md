# Dosi Tracker for macOS

A menu-bar agent that records time and activity for your Dosi Tracker workspace,
written in Swift with Apple's own frameworks. It behaves like the Windows agent
(`clients/windows`) — same blocks, same activity %, same upload rules — so a
mixed team gets the same numbers from every computer.

- **Menu bar** — an icon whose centre shows the state (● tracking, ❚❚ paused,
  ○ away / locked / asleep, a dashed ring while connecting or choosing a project,
  ! when it needs you, an empty ring when signed out), the session timer while
  tracking, and a menu with the
  status and today / this week totals, Open, Pause / Resume, timed breaks
  (15 / 30 / 60 min), Switch project, Set task…, Sync now, Open web dashboard,
  Settings, Start at login and Quit.
- **Window** — *Timer* (project, task note, session clock, goals, the block in
  progress minute by minute and the app in front), *Activity* (this week day by
  day, top apps of the last hour, the latest screenshot exactly as uploaded, the
  blocks recorded and whether each was uploaded or rejected and why) and
  *Settings* (start at login, goals, privacy permissions, what your project
  records, this Mac, the account, logs).
- **Offline first** — every block is saved on the Mac before it is uploaded, and
  tracking starts and goes on without a connection (on the project list saved at
  the last sign-in, for up to 7 days).

Requires **macOS 14 Sonoma or later** (Apple silicon or Intel).

## Install

1. Download `DosiTracker-<version>-macos.zip` (from the release, or the
   `dosi-tracker-macos` artifact of a CI run) and open it.
2. Move **Dosi Tracker** to **Applications** — macOS keeps your permission
   grants and the login item for an app that stays in one place.
3. Open it. A build signed with a Developer ID and notarized opens normally. A
   build signed only ad hoc (the default of `scripts/build-app.sh` and CI) is
   blocked the first time: open **System Settings → Privacy & Security**, find
   *"Dosi Tracker" was blocked* and choose **Open Anyway** (or Control-click the
   app in Finder and choose **Open**).
4. Sign in (below). The app lives in the menu bar; it has no Dock icon.

To update, quit Dosi Tracker from its menu and replace the app. Work not
uploaded yet stays in the local queue and goes up after the update.

## Sign in

The sign-in window asks for:

- **Workspace** — your workspace (tenant) name, e.g. `acme`. Leave it empty only
  for a host account.
- **Email** and **Password** — the ones you use on the web dashboard. Enter
  submits; the eye button shows the password.
- **Advanced → Server address** — the API address of your Dosi Tracker server,
  e.g. `https://tracker-api.example.com`. The web dashboard's **Download** page
  shows the exact address to paste. Your organisation may have set it already
  (the field is then locked), or built it into the app.

The sign-in is kept in your **login Keychain** (item *Dosi Tracker sign-in*,
service `com.dosibridge.tracker`, this Mac only); the password is never written
anywhere else. When the server refuses it later — you changed or reset your
password, or the account was deactivated — the agent stops tracking, keeps
everything it recorded, and asks for the password again (*Sign-in expired*).
If the server cannot be reached, the sign-in is kept and the agent retries
every 30 seconds. When the server demands a new password, the form asks for one.

With several projects, choose the one you work on (the window asks, and the
menu's **Switch project** changes it at any time). With one project, tracking
starts by itself.

After the first sign-in the app adds itself to your login items (macOS shows
a notice), so tracking resumes after a restart — like the Windows installer's
default. Turn it off with **Start at login** in the menu or in Settings; it is
not turned back on.

## Permissions, and why

macOS asks before an app may see the screen, the keyboard and mouse, other
apps' windows, or the camera. Dosi Tracker only asks for what your project
records, the first time it needs it; **Settings → Permissions** shows each one
with a button that asks again or opens the right pane of **System Settings →
Privacy & Security**.

| Permission | Used for | Without it |
|---|---|---|
| **Screen Recording** | The screenshot of each block (of the display you work on). Also reveals window titles when Accessibility is off. | No screenshots; time is still tracked. Takes effect after restarting the app. |
| **Input Monitoring** | Keyboard and mouse **activity**: when you typed or clicked, never what — the activity %, key and click counts, and noticing that you are away. | Activity is reported as *not measured* (never a misleading 0%); idle detection falls back to the system's idle clock. |
| **Accessibility** | The title of the window you work in, when the project records it. | Titles come from Screen Recording if granted, else apps are recorded by name only. |
| **Camera** | One webcam photo per block, only if the project turns it on. | No webcam photos. |
| **Notifications** | Telling you that tracking started at login, a project needs choosing, or the sign-in expired. | Nothing is shown; the menu bar still says it. |

macOS ties every grant to the app's code signature. A Developer ID build keeps
them across updates; each new **ad-hoc** build counts as a new app, so the
grants must be given again (remove the old *Dosi Tracker* entries in each pane
first).

## What is recorded

What is captured is your workspace admin's choice, per project, and shown
read-only in **Settings → What is recorded**. The agent records:

- **Time blocks** of the project's interval (5–60 min), back to back while you
  work; never longer than one interval, never shorter than a minute.
- **Activity %** — the share of minutes with any keyboard or mouse input
  (pointer movement and scrolling count), counts of key presses and clicks, and
  how many seconds of each minute had input. Keys you type are never recorded,
  and input posted by software (mouse jigglers, remote tools) is counted aside,
  never as yours.
- **Apps** — the app in front and, if allowed, its window title, with the time
  spent; the names of other open apps (never their windows).
- **One screenshot per block**, at a random moment, of the display holding the
  app you work in — blurred beyond reading before it leaves the Mac when the
  project requires it. **Activity → Latest screenshot** shows exactly what was
  sent.
- **The task note** ("What are you working on?"), sent with each block.
- **Device context** — this Mac's name, macOS and app version, a random install
  id, and your time zone.

Nothing is recorded while paused, on a break, locked, asleep or away. After
the project's idle time without input, the block ends at your last input; when
you come back you are asked whether to **keep** the time away as (idle) work or
**discard** it — no answer discards it.

## For administrators: configuration

Settings are read in this order, each overriding the one before:

1. built-in defaults (a local development server: `https://localhost:6006`);
2. the app's `Info.plist` — set at build time with `DOSI_API_BASE_URL` /
   `DOSI_DASHBOARD_URL` (see *Build*), so a download for your server signs in
   there without typing an address;
3. `~/Library/Application Support/DosiTracker/config.json` (shape of
   [`config.example.json`](config.example.json));
4. the preferences of `com.dosibridge.tracker` — `defaults write` or an **MDM
   configuration profile** (a profile that sets `DosiApiBaseURL` locks the
   server field of the sign-in form);
5. the server address the user typed at sign-in (unless a profile fixes it).

| Key | Type | Meaning |
|---|---|---|
| `DosiApiBaseURL` | string | The API address, e.g. `https://tracker-api.example.com` |
| `DosiDashboardURL` | string | The web dashboard ("Open web dashboard") |
| `DosiIntervalMinutes` | integer | Interval shown before a project is known (5–60) |
| `DosiCaptureScreenshot`, `DosiCaptureWebcam`, `DosiCaptureKeyboard`, `DosiCaptureMouse`, `DosiCaptureActiveWindow`, `DosiCaptureRunningPrograms` | boolean | Switch a capture **off** on this Mac (they can never switch on what the project does not allow) |
| `DosiBlurScreenshots` | boolean | Always blur screenshots on this Mac (can add blur, never remove it) |
| `DosiLogDebug` | boolean | Also write debug lines to the log file |

For example:

```bash
defaults write com.dosibridge.tracker DosiApiBaseURL "https://tracker-api.example.com"
defaults write com.dosibridge.tracker DosiDashboardURL "https://tracker.example.com"
```

Start at login uses the system's login items (System Settings → General →
Login Items); an MDM profile can manage it like any other login item.

## Where things are

| What | Where |
|---|---|
| Offline queue (blocks waiting to upload, rejected ones for 14 days) | `~/Library/Application Support/DosiTracker/dosi-tracker.db` |
| Optional configuration | `~/Library/Application Support/DosiTracker/config.json` |
| Logs (rotated, ~10 MB at most) | `~/Library/Logs/DosiTracker/dosi-tracker.log` — also in Console.app under subsystem `com.dosibridge.tracker` |
| Preferences (goals, task note, pause, project, device id, server) | `defaults read com.dosibridge.tracker` |
| Sign-in | login Keychain, item *Dosi Tracker sign-in* |

**Settings → Help & support** opens the logs and data folders.

## Troubleshooting

- **"Cannot reach the server"** — check the server address under *Advanced*
  (exactly as the Download page shows it), the network and any VPN. Tracking
  goes on offline once a project is known; blocks upload when the server is back.
- **"The server's security certificate is not trusted"** — the server uses a
  certificate this Mac does not trust. A development server on
  `https://localhost:6006` uses the ASP.NET development certificate: trust it in
  Keychain Access, or put the API behind a real certificate.
- **Sign-in expired** — enter your password again; nothing recorded is lost.
- **"Keyboard and mouse activity is not being measured"** — allow Dosi Tracker
  in *Input Monitoring*. If it is listed and on but still not measured, remove
  it with **−**, then quit and open the app again.
- **No screenshots** — allow *Screen Recording* and restart the app.
- **The permissions reset after an update** — ad-hoc builds are a new app each
  time (see *Permissions*); use a Developer ID build to keep them.
- **macOS asks to allow access to the Keychain after an update** — same cause;
  choose *Always Allow*.
- **"Your computer clock is off by N min"** — turn on *Set time and date
  automatically* in System Settings → General → Date & Time. Blocks that end in
  the server's future wait until the clock is right.
- **Blocks rejected by the server** — **Activity** lists each one with the
  server's reason (for example an overlap with another computer tracking at the
  same time). Rejected blocks are kept 14 days for your records.
- **"Workspace subscription inactive"** — uploads wait (hourly retries) until the
  workspace is renewed; nothing is lost.
- **Reporting a problem** — attach the files from **Open logs**. For more detail:
  `defaults write com.dosibridge.tracker DosiLogDebug -bool true`, then restart.
- **Starting over** — sign out (Settings → Account), quit, and delete
  `~/Library/Application Support/DosiTracker`. Uninstall by quitting and moving
  the app to the Bin.

## Build

Requirements: macOS 14 and Xcode 15 or later (Swift 5.9+). No third-party
dependencies; SQLite is the system library.

```bash
cd clients/macos
swift build                    # the core library and the app executable
swift test                     # the core's and the worker's tests (no permissions or UI needed)
bash scripts/build-app.sh      # "Dosi Tracker.app" + zip, in .build/dist
```

`swift run` starts the agent without a bundle — handy for development, but
macOS needs the app bundle for the camera, notifications and the login item,
and attributes privacy permissions to the terminal instead. Use the app from
`scripts/build-app.sh` for anything beyond a quick check.

`scripts/build-app.sh` compiles a universal binary (arm64 + x86_64), assembles
the bundle with `Resources/Info.plist` and an icon drawn by
`scripts/make-icon.swift`, signs it with the hardened runtime and
`Resources/DosiTracker.entitlements` (the camera; no App Sandbox — reading other
apps' windows and input is not possible from a sandbox), verifies the
signature, and packs `DosiTracker-<version>-macos.zip`. Its options:

| Variable | Default | Meaning |
|---|---|---|
| `VERSION` | `Info.plist` | `CFBundleShortVersionString` |
| `BUILD_NUMBER` | `1` | `CFBundleVersion` |
| `UNIVERSAL` | `1` | `0` builds for this Mac's architecture only |
| `DOSI_API_BASE_URL`, `DOSI_DASHBOARD_URL` | — | Bake the server and dashboard addresses into the app |
| `CODESIGN_IDENTITY` | `-` (ad hoc) | e.g. `Developer ID Application: Example Ltd (TEAMID1234)` |
| `NOTARY_PROFILE` | — | Notarize with this `notarytool` keychain profile and staple the ticket |
| `MAKE_DMG` | `0` | `1` also builds a disk image with an Applications link |
| `DIST_DIR` | `.build/dist` | Output folder |

### Signing and notarization

1. Install a **Developer ID Application** certificate in the login keychain.
2. Once, store notary credentials (with an app-specific password of the Apple ID):
   ```bash
   xcrun notarytool store-credentials dosi-notary \
     --apple-id you@example.com --team-id TEAMID1234 --password abcd-efgh-ijkl-mnop
   ```
3. Build, sign, notarize and staple:
   ```bash
   CODESIGN_IDENTITY="Developer ID Application: Example Ltd (TEAMID1234)" \
   NOTARY_PROFILE=dosi-notary MAKE_DMG=1 \
   DOSI_API_BASE_URL=https://tracker-api.example.com \
   bash scripts/build-app.sh
   ```

CI (`.github/workflows/ci.yml`, job *macOS agent*) runs `swift build`,
`swift test` and `scripts/build-app.sh` (universal, ad hoc) on every push and
uploads the zip as the `dosi-tracker-macos` artifact.

## How it is built

Two targets (see `Package.swift`):

- **`DosiTrackerCore`** — the platform-independent agent, a port of the Windows
  agent's logic so both produce the same numbers: the wire models
  (`CreateActivityDto`, `MyProjectDto`, `HeartbeatDto`, …), the activity math
  (`tracking/input.rs` bucketing), foreground usage (`tracking/usage.rs`), block
  assembly, the SQLite offline queue (`storage.rs`), the upload policy and sync
  pass, the API client (`api.rs`), configuration, preferences and Keychain
  credentials. Foundation, SQLite3, Security and os only, covered by
  `swift test`.
- **`DosiTracker`** — the app:
  - `System/` — capture services: the event tap (Input Monitoring), the front
    app and window title (NSWorkspace, Accessibility, the window list), screen
    captures (ScreenCaptureKit), webcam photos (AVFoundation), running apps, the
    idle clock, and the lock / sleep / power events.
  - `Worker/` — `TrackerWorker`, an actor porting `worker.rs`: block timing
    (`BlockTimeline`, from `worker/blocks.rs`: blocks back to back, held while
    you are inactive and cut at your last input when you went idle), lock /
    sleep / shutdown handling, pause and breaks (kept across restarts), the idle
    prompt, the task note, the random screenshot moment, the presence heartbeat
    (every 60 s and on every change), the project refresh (every 5 min), the
    totals; `UploadLoop`, an actor draining the queue apart from the worker.
  - `UI/` — the menu-bar item, the SwiftUI windows (in `NSWindow`s) and their
    model on the main actor.
  - `App/` — the `@main` entry, the application delegate (one copy only,
    SIGTERM, saving the block before quitting), logging, configuration, the
    login item and notifications.

It talks to the backend exactly as the Windows agent does (the shared contract):

- **Sign-in** — OpenIddict password grant on `POST /connect/token?__tenant=<workspace>`
  (client `Tracker_App`, scopes `Tracker offline_access`); the refresh token
  renews the access token, the saved password is the fallback.
- **Projects** — `GET /api/app/project/my-projects`: the projects you track, with
  their capture policy, interval, idle threshold and blur.
- **Blocks** — `POST /api/app/activity` with a client-generated id (re-sending is
  harmless). The answer decides the row's fate: accepted rows leave the queue;
  network trouble, 5xx and throttling back off from 30 s to 30 min; an inactive
  subscription retries hourly; a clock ahead of the server retries every 5 min;
  oversized captures are stripped and the block resent; a project you left or
  that was archived is checked against the project list; definitive refusals are
  kept as *rejected* with the server's reason.
- **Presence** — `POST /api/app/presence/heartbeat` every minute and on every
  change (tracking, paused, break, idle, locked, choosing, signed out, stopped).
- **Totals** — `GET /api/app/reporting/summary` per local day, in your time zone.
