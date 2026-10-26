# Dosi Tracker for macOS

A menu-bar agent that records time and activity for your Dosi Tracker workspace,
written in Swift with Apple's own frameworks. It behaves like the Windows agent
(`clients/windows`) — same blocks, same activity %, same upload rules — so a
mixed team gets the same numbers from every computer.

- **Menu bar** — an icon whose centre shows the state (● tracking, ❚❚ paused,
  ○ away / locked / asleep, a dashed ring while connecting or choosing a project,
  ! when it needs you — sign-in expired, consent required —, an empty ring when
  signed out), the session timer while tracking, and a menu with the status and
  today / this week totals, Open, Pause / Resume, timed breaks (15 / 30 / 60 min),
  Switch project, Switch workspace…, Set task…, Sync now, Open web dashboard,
  Privacy consent…, Settings, Start at login and Quit.
- **Window** — *Timer* (project, task note, session clock, goals, the block in
  progress minute by minute and the app in front), *Activity* (this week day by
  day, top apps of the last hour, the latest screenshot exactly as uploaded, the
  blocks recorded and whether each was uploaded or rejected and why) and
  *Settings* (start at login, goals, your consent, privacy permissions, what is
  recorded, this Mac, the account, logs).
- **Offline first** — every block is saved on the Mac before it is uploaded, and
  tracking starts and goes on without a connection (on the project list saved at
  the last sign-in, for up to 7 days).

Requires **macOS 14 Sonoma or later** (Apple silicon or Intel).

## Install

1. Download `DosiTracker-macos.zip` — from your organisation, or from the
   project's GitHub Releases (`/releases/latest/download/DosiTracker-macos.zip`)
   once a signed release is published — and open it.
2. Move **Dosi Tracker** to **Applications** — macOS keeps your permission
   grants and the login item for an app that stays in one place.
3. Open it. A release is signed with a Developer ID and notarized, and opens
   normally. A build signed only ad hoc — what `scripts/build-app.sh` makes
   without signing credentials, including the `dosi-tracker-macos` artifact of a
   CI run — is **for development only**: on any other Mac Gatekeeper blocks it,
   and it opens only after **System Settings → Privacy & Security → Open Anyway**
   (on macOS 15, Control-click → Open no longer bypasses this).
4. Sign in (below). The app lives in the menu bar; it has no Dock icon.

To update, quit Dosi Tracker from its menu and replace the app. Work not
uploaded yet stays in the local queue and goes up after the update.

## Sign in

The sign-in window asks for:

- **Email** and **Password** — the ones you use on the web dashboard. Enter
  submits; the eye button shows the password.
- **Server address** — the API address of your Dosi Tracker server, e.g.
  `https://tracker-api.example.com`. The web dashboard's **Download** page
  shows the exact address to paste. When the app was built for your server, or
  your organisation set it (the field is then locked), it is filled in and
  folded under **Advanced**; it opens by itself when a sign-in fails because of
  the address (nothing answers there, a redirect, a page that is not the API).
  When nothing names a server, the field comes first and must be filled: the
  app never signs in to a development address on its own. Once a sign-in
  succeeds the address is remembered. It must start with `https://`; `http://`
  works only for a server on this Mac, unless your administrator allows it
  (`DosiAllowInsecureHTTP`).

Then the **workspace**: with one workspace, tracking starts there by itself;
with several, a window lists them with your role in each (Owner, Manager,
Member) — archived workspaces are never offered. Signing in again later goes
back to the workspace you tracked in. **Switch workspace…** in the menu (or in
Settings → Account) moves to another one at any time: the block so far is saved
in the workspace it was recorded in, and its blocks still upload there.

The Keychain keeps the sign-in — your email, the workspace and the server's
refresh token — in your **login Keychain** (item *Dosi Tracker sign-in*,
service `com.dosibridge.tracker`, this Mac only). **Your password is never
stored**: it is sent once, at sign-in. When the server refuses the sign-in
later — you changed or reset your password, the account was deactivated, or the
session ran out — the agent stops tracking, keeps everything it recorded, and
asks for the password again (*Sign-in expired*); it never retries a password
behind your back. If the server cannot be reached, the sign-in is kept and the
agent retries every 30 seconds. When the server demands a new password, the
form asks for one.

The server can also end tracking on this Mac for good:

- **This Mac was removed** — an administrator revoked it. Tracking stops at
  once, the saved sign-in is forgotten, and signing in again works only once
  the administrator restores the device.
- **No longer a member / workspace archived** — tracking (and the presence
  heartbeat) stops in that workspace and you are asked to choose another one —
  even when only one is left, the agent never moves you there by itself, and a
  relaunch or signing in again asks as well until you have chosen.

Either way the blocks already recorded stay in the local queue; they upload if
access comes back, and expire after 14 days otherwise.

With several projects, choose the one you work on (the window asks, and the
menu's **Switch project** changes it at any time). With one project, tracking
starts by itself.

After the first sign-in the app adds itself to your login items (macOS shows
a notice), so tracking resumes after a restart — like the Windows installer's
default. That happens even while it still waits for your consent (below). Turn
it off with **Start at login** in the menu or in Settings; it is not turned
back on.

## Your consent on this Mac

Your workspace admin decides what each project may capture; on this Mac, you
decide too. Nothing is recorded until you choose at least one kind of capture:
after signing in the status is **Consent required**, and the Timer tab shows
one switch per capture — keyboard counts, mouse counts, the app and window in
front, running programs, screenshots (and whether to blur them), webcam photos
— with **Grant Standard Consent** (keyboard and mouse counts, the app in front,
blurred screenshots; never the webcam or running programs). **Privacy
consent…** in the menu and **Settings → Local Consent** change it at any time,
and **Revoke All Consent** stops everything.

- What is recorded is what you switched on **and** the project allows. Never
  collected, whatever the settings: what you type, which keys, the clipboard,
  or the timing between keystrokes.
- Waiting for consent counts as signed in: you can sign out, switch project or
  workspace, and upload what is already queued; the dashboard shows the device
  as *stopped*.
- Each block is recorded under the consent in force when it started. When you
  change it, capture follows at once and a pending screenshot is dropped. A
  change to keyboard, mouse or window time ends the block in progress there and
  starts a new one under your new choice; a block under a minute old is kept
  instead, with a capture you turned off left out of it from its start (one
  you turned on waits for the next block). Screenshots, webcam photos and
  running programs, taken at one moment, follow your choice within the block
  in progress.
- Input you did not consent to is sent as *not measured* (null), never as a 0
  that would read as idleness.

The choice is stored in `~/Library/Application Support/DosiTracker/consent.json`
and never leaves the Mac.

## Permissions, and why

macOS asks before an app may see the screen, the keyboard and mouse, other
apps' windows, or the camera. Dosi Tracker asks only for what will be recorded
— what your project records **and** you consented to — the first time it is
needed, so no system prompt appears for a capture you did not switch on.
**Settings → Permissions** shows each one with a button that asks again or
opens the right pane of **System Settings → Privacy & Security**. The Input
Monitoring warning appears only while tracking with keyboard or mouse recorded,
never because tracking is paused.

| Permission | Used for | Without it |
|---|---|---|
| **Screen Recording** | The screenshot of each block (of the display you work on). Also reveals window titles when Accessibility is off. | No screenshots; time is still tracked. Takes effect after restarting the app. |
| **Input Monitoring** | Keyboard and mouse **activity**: when you typed or clicked, never what — the activity %, key and click counts, and noticing that you are away. | Activity is reported as *not measured* (never a misleading 0%); idle detection falls back to the system's idle clock. |
| **Accessibility** | The title of the window you work in, when the project records it and you consent to it. | Titles come from Screen Recording if granted, else apps are recorded by name only. |
| **Camera** | One webcam photo per block, only if the project turns it on and you switch it on. | No webcam photos. |
| **Notifications** | Telling you that tracking started at login, a project needs choosing, or the sign-in expired. | Nothing is shown; the menu bar still says it. |

macOS ties every grant to the app's code signature. A Developer ID build keeps
them across updates; each new **ad-hoc** build counts as a new app, so the
grants must be given again (remove the old *Dosi Tracker* entries in each pane
first).

On macOS 15 Sequoia and later, macOS itself periodically asks again whether
Dosi Tracker may keep capturing the screen. Until you allow it, screenshots are
skipped (the time is still tracked). Organisations can deploy the permission
through an MDM (below).

## What is recorded

What may be captured is your workspace admin's choice, per project; what is
captured on this Mac is that, narrowed to your consent. **Settings → What is
recorded** shows each capture as *On*, *Off — your choice* or *Off — by your
workspace*. The agent records:

- **Time blocks** of the project's interval (5–60 min), back to back while you
  work; never longer than one interval, never shorter than a minute.
- **Activity %** — the share of minutes with any keyboard or mouse input
  (pointer movement and scrolling count), counts of key presses and clicks, and
  how many seconds of each minute had input. Keys you type are never recorded,
  and input posted by software (mouse jigglers, remote tools) is counted aside,
  never as yours. Input that is not measured — you did not consent to it, the
  project leaves it out, or Input Monitoring is off — is sent as *not measured*
  (null), never as a 0 that would read as idleness.
- **Apps** — the app in front and, if allowed, its window title, with the time
  spent; the names of other open apps (never their windows).
- **One screenshot per block**, at a random moment, of the display holding the
  app you work in — blurred beyond reading before it leaves the Mac when the
  project requires it. **Activity → Latest screenshot** shows exactly what was
  sent.
- **The task note** ("What are you working on?"), sent with each block.
- **Device context** — this Mac's name, macOS and app version, a random install
  id, and your time zone.

Nothing is recorded while paused, on a break, locked, asleep or away. A block
ended by a pause, a quit or a sign-out before its screenshot moment takes its
screenshot (and the list of open apps) at that end, as on Windows, if you still
consent to it; one ended by locking, sleep or shutdown, or by the server (the
sign-in expired, the device or the workspace was removed), takes nothing more.
After the project's idle time without input, the block ends at your last input;
when you come back you are asked whether to **keep** the time away as (idle)
work or **discard** it — no answer discards it.

## For administrators: configuration

Settings are read in this order, each overriding the one before:

1. built-in defaults — a development placeholder (`https://localhost:6006`,
   dashboard `http://localhost:6005`) that never counts as a configured server:
   without one of the sources below the sign-in form asks for the address, and
   **Open web dashboard** says the address is not set instead of opening
   `localhost`;
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
| `DosiAllowInsecureHTTP` | boolean | Accept an `http://` server that is not this Mac (off by default: the password and every block would cross the network in clear text; App Transport Security still allows cleartext only to local-network hosts) |
| `DosiLogDebug` | boolean | Also write debug lines to the log file |

For example:

```bash
defaults write com.dosibridge.tracker DosiApiBaseURL "https://tracker-api.example.com"
defaults write com.dosibridge.tracker DosiDashboardURL "https://tracker.example.com"
```

Start at login uses the system's login items (System Settings → General →
Login Items); an MDM profile can manage it like any other login item.

### Deploying with an MDM

[`deploy/DosiTracker-sample.mobileconfig`](deploy/DosiTracker-sample.mobileconfig)
is a sample configuration profile for Jamf, Intune, Kandji and the like, with:

- **managed preferences** for `com.dosibridge.tracker` — the server and
  dashboard addresses (the sign-in form then cannot change the server);
- **Privacy Preferences Policy Control (PPPC)** — Accessibility pre-approved;
  Screen Recording and Input Monitoring cannot be pre-approved under Apple's
  rules, so standard users are allowed to approve them themselves (no admin
  password); the camera cannot be pre-approved at all;
- a **managed login item** for `com.dosibridge.tracker`, so start at login
  holds after every restart.

Replace `TEAMID1234` with the Team ID of the Developer ID certificate the app
is signed with — `scripts/build-app.sh` prints the app's designated
requirement, which the PPPC `CodeRequirement` must match — and the example
addresses with yours; give each payload a fresh `PayloadUUID`. PPPC and managed
login items take effect only from a user-approved or supervised MDM enrolment,
and only for a Developer ID signed build. None of this replaces the user's own
consent in the app.

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

- **"Cannot reach the server"** — check the server address (the sign-in form
  opens it by itself; exactly as the Download page shows it), the network and
  any VPN. Tracking goes on offline once a project is known; blocks upload when
  the server is back.
- **"Enter your server address"** — this copy of the app names no server; paste
  the address from the web dashboard's Download page.
- **"Consent required"** — choose what may be recorded on the Timer tab (or
  *Privacy consent…* in the menu); nothing is tracked until you do.
- **"The server's security certificate is not trusted"** — the server uses a
  certificate this Mac does not trust. A development server on
  `https://localhost:6006` uses the ASP.NET development certificate: trust it in
  Keychain Access, or put the API behind a real certificate.
- **Sign-in expired** — enter your password again; nothing recorded is lost.
- **"This Mac was removed from tracking"** — your administrator revoked this
  device; ask them to restore it, then sign in again.
- **"The server address must use https://"** — `http://` is refused except for
  a server on this Mac; use the `https://` address (or, for a test server on
  the local network, have an administrator set `DosiAllowInsecureHTTP`).
- **"The server answered with a redirect"** — the agent never follows
  redirects; enter the API server's own address, not one that forwards to it.
- **"Keyboard and mouse activity is not being measured"** (while tracking with
  keyboard or mouse switched on) — allow Dosi Tracker in *Input Monitoring*. If it is listed and on but still not measured, remove
  it with **−**, then quit and open the app again.
- **No screenshots** — check that screenshots are on in your consent and in
  *What is recorded*, allow *Screen Recording* and restart the app. On macOS 15
  and later, answer macOS's periodic *"…is requesting to bypass the system
  private window picker"* confirmation.
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
swift build                        # the core library and the app executable
swift test                         # the core's and the worker's tests (no permissions or UI needed)
bash scripts/build-app.sh          # "Dosi Tracker.app" + zips, in .build/dist
bash scripts/build-app.sh --plan   # what would be built and how it is signed, without building
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
signature, notarizes and staples it when notary credentials are present, and
packs `DosiTracker-<version>-macos.zip` plus the version-less release asset
`DosiTracker-macos.zip` (and `.dmg` files with `MAKE_DMG=1`), with their
`SHA256SUMS`. No server address is baked in unless `DOSI_API_BASE_URL` is set.

| Variable | Default | Meaning |
|---|---|---|
| `VERSION` | the tag being built (`GITHUB_REF` `refs/tags/v1.4.0` → `1.4.0`), else `Info.plist` | `CFBundleShortVersionString`; a leading `v` and a pre-release suffix (`-rc.1`) are dropped |
| `BUILD_NUMBER` | `GITHUB_RUN_NUMBER`, else `1` | `CFBundleVersion` |
| `UNIVERSAL` | `1` | `0` builds for this Mac's architecture only |
| `DOSI_API_BASE_URL`, `DOSI_DASHBOARD_URL` | — | Bake the server and dashboard addresses into the app. A Developer ID build refuses a local or `http://` server |
| `MAKE_DMG` | `0` | `1` also builds disk images with an Applications link |
| `DIST_DIR` | `.build/dist` | Output folder |
| `REQUIRE_NOTARIZATION` | `0` | `1` fails unless the build is Developer ID signed and notarized (for release jobs) |

Signing and notarization run only when their credentials are present;
otherwise the app is signed ad hoc (development only — the script warns). A
half-configured set of credentials fails the build rather than falling back.

| Signing (one of) | |
|---|---|
| `DEVELOPER_ID_P12_BASE64`, `DEVELOPER_ID_P12_PASSWORD` | A *Developer ID Application* certificate with its key, as a base64 `.p12` (CI secrets). It is imported into a temporary keychain that is deleted when the script ends |
| `CODESIGN_IDENTITY` | An identity in your keychain, e.g. `Developer ID Application: Example Ltd (TEAMID1234)` (with the `.p12`, picks one of its identities) |

| Notarization (one of) | |
|---|---|
| `NOTARY_PROFILE` | A `notarytool` keychain profile (`xcrun notarytool store-credentials`) |
| `NOTARY_API_KEY_BASE64` (or `NOTARY_API_KEY_PATH`), `NOTARY_API_KEY_ID`, `NOTARY_API_ISSUER_ID` | An App Store Connect API key (`.p8`) |
| `NOTARY_APPLE_ID`, `NOTARY_TEAM_ID`, `NOTARY_PASSWORD` | An Apple ID with an app-specific password |

On a Mac with the certificate in the login keychain:

```bash
xcrun notarytool store-credentials dosi-notary \
  --apple-id you@example.com --team-id TEAMID1234 --password <app-specific password>
CODESIGN_IDENTITY="Developer ID Application: Example Ltd (TEAMID1234)" \
NOTARY_PROFILE=dosi-notary MAKE_DMG=1 REQUIRE_NOTARIZATION=1 \
bash scripts/build-app.sh
```

In CI, pass the secrets as environment variables of the step that runs
`scripts/build-app.sh`. The app is submitted to Apple's notary service, the
ticket is stapled to it, and the zips (and disk images) are made from the
stapled app.

CI (`.github/workflows/ci.yml`, job *macOS agent*) runs `swift build`,
`swift test` and `scripts/build-app.sh` on every push and uploads the zip as the
`dosi-tracker-macos` artifact. Without signing secrets that artifact is ad hoc
signed: a development build, not something to hand to users.

There is no in-app update yet: install a new release over the old one (the
queue, the sign-in and — for Developer ID builds — the permissions are kept).

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

It talks to the backend as every agent does (the cross-agent contract):

- **Consent** — local only (`consent.json`); each block's policy is the
  project's, narrowed by the local configuration and by the consent in force
  when the block started.
- **Sign-in** — OpenIddict password grant on `POST /connect/token` (client
  `Tracker_App`, scopes `Tracker offline_access`, plus `device_id`, this
  install's random v4 UUID); no workspace travels with it. The refresh token
  (sent with the same `device_id`) renews the access token and is saved rotated;
  once it is refused (`invalid_grant`) the state is *Sign-in expired* — the
  password is never stored nor re-sent silently.
- **Workspaces** — `GET /api/app/workspaces/mine` (no workspace header): the
  workspaces you belong to, with your role. The chosen workspace's GUID goes in
  `X-Workspace-Id` on every workspace-scoped call (projects, blocks, presence,
  totals); queued rows remember theirs.
- **Terminal answers** — 403 `Tracker:DeviceRevoked` (also forgets the saved
  sign-in), `Tracker:NotWorkspaceMember`, `Tracker:WorkspaceArchived` and 400
  `Tracker:WorkspaceRequired` stop capture and heartbeats for that workspace;
  its queued rows wait. The next workspace is then always chosen by you, even
  when only one is left or it is the only one offered after a relaunch: it may
  belong to another organisation, so nothing is taken on your behalf.
- **Transport** — `https://` only (`http://` only to this Mac, unless
  `DosiAllowInsecureHTTP`), and redirects are never followed.
- **Projects** — `GET /api/app/project/my-projects`: the projects you track, with
  their capture policy, interval, idle threshold and blur.
- **Blocks** — `POST /api/app/activity` with a client-generated id (re-sending is
  harmless). The answer decides the row's fate: accepted rows leave the queue;
  network trouble, 5xx and throttling back off from 30 s to 30 min; an inactive
  subscription retries hourly; a clock ahead of the server retries every 5 min;
  oversized captures are stripped and the block resent; a project you left or
  that was archived is checked against the project list; definitive refusals are
  kept as *rejected* with the server's reason. Each workspace waits on its own:
  blocks left from a workspace whose subscription lapsed never delay the
  current workspace's uploads (and only the current workspace's wait is shown).
- **Presence** — `POST /api/app/presence/heartbeat` every minute and on every
  change (tracking, paused, break, idle, locked, choosing, signed out, stopped).
- **Totals** — `GET /api/app/reporting/summary` per local day, in your time zone.
