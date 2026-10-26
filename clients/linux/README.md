# Dosi Tracker for Linux

The Linux agent of Dosi Tracker, a team time-tracking service. It records your
working time in blocks and, only with your consent on this computer, activity
details, and uploads them to your workspace's Dosi Tracker server.

It is three programs in one package:

- `dosi-tracker`: the command line, and the agent itself (`dosi-tracker daemon`), which
  runs per user as a systemd user service;
- the tray icon, shown by the agent where the desktop has a tray;
- `dosi-tracker-gui`: the window ("Dosi Tracker" in your applications menu).

It never runs as root, never reads what you type, and starts nothing by itself
after installation.

- [Install](#install)
- [First run](#first-run)
- [The window and the tray](#the-window-and-the-tray)
- [What is recorded](#what-is-recorded)
- [Permissions](#permissions)
- [Command line](#command-line)
- [Files and locations](#files-and-locations)
- [Configuration](#configuration)
- [Troubleshooting](#troubleshooting)
- [Upgrade, uninstall and purge](#upgrade-uninstall-and-purge)
- [Development](#development)

## Install

Release artifacts (x86_64, glibc 2.35 or newer) are attached to the `linux-vX.Y.Z`
releases on the [GitHub releases page](https://github.com/DosiBridge/Dosi-Tracker/releases),
as versioned files (`dosi-tracker-X.Y.Z-x86_64-linux-gnu.tar.gz`,
`dosi-tracker_X.Y.Z-1_amd64.deb`, `dosi-tracker-X.Y.Z-1.x86_64.rpm`) and as the same
files under version-less names. The version-less names are the ones download links
use with GitHub's latest-release address,
`https://github.com/DosiBridge/Dosi-Tracker/releases/latest/download/<name>`:

| Artifact | File | For |
|---|---|---|
| Tarball (no root) | `dosi-tracker-linux-x86_64.tar.gz` | any distribution; installs into `~/.local` |
| Debian package | `dosi-tracker_amd64.deb` | Ubuntu 22.04 and 24.04, Debian 12 and 13 |
| RPM package | `dosi-tracker.x86_64.rpm` | Fedora 43 and 44 |
| Checksums | `SHA256SUMS` | all of the above |

These are the distributions CI installs the artifacts on (Arch Linux through the
tarball). Older systems (Ubuntu 20.04, Debian 11, RHEL 8 and 9) have an older glibc and
need a source build.

Check the download first:

```bash
sha256sum -c --ignore-missing SHA256SUMS
```

**Tarball** (rootless; needs `python3`, which validates the installer's manifest):

```bash
tar -xzf dosi-tracker-linux-x86_64.tar.gz
./dosi-tracker-*/install.sh --server https://tracker.example.com
```

It installs into `~/.local` (`--prefix DIR` for another place): the two programs in
`~/.local/bin`, the user unit, the applications-menu entry, icons and shell
completions. `--server` and `--dashboard` are saved with `dosi-tracker config set`
(an invalid address stops the installer with exit 2 before anything changes). The
installer refuses root, records every file it writes in
`~/.local/lib/dosi-tracker/install/manifest.json`, and never overwrites or deletes a
file it does not own.

**Debian and Ubuntu**:

```bash
sudo apt install ./dosi-tracker_amd64.deb
```

**Fedora**:

```bash
sudo dnf install ./dosi-tracker.x86_64.rpm
```

The packages have no install scripts: they put the programs in `/usr/bin`, the user
unit in `/usr/lib/systemd/user` (installed, not enabled), and the menu entry, icons,
AppStream metadata and completions under `/usr/share`. They recommend the libraries
the window loads at run time (EGL/GL, xkbcommon-x11, Xcursor, Xrandr, Xi), the
screenshot portal and a keyring; the command line and the agent work without them.

Administrators can preset the server for every user in `/etc/dosi-tracker/config.toml`
(see [Configuration](#configuration)) and, if the agent should start for everyone at
login, add their own `/etc/xdg/autostart` entry running
`/usr/bin/dosi-tracker service session-start`. Neither the packages nor the installer
do that.

## First run

Nothing runs until you start it. Inside your desktop session:

```bash
dosi-tracker service enable --now     # start the agent now and at every login
dosi-tracker login                    # asks for the server (unless set), email and password
dosi-tracker consent show             # what may be recorded on this computer, and why not
dosi-tracker consent grant --keyboard --mouse --active-window --running-programs --screenshots
dosi-tracker project list
dosi-tracker project use "Website Redesign"
dosi-tracker start
dosi-tracker status
```

- `login` reads the password without echo, then lists your workspaces (archived ones
  are not offered) and asks you to choose when there is more than one. `--server`,
  `--email` and `--workspace NAME_OR_ID` skip the questions. Passwords are never
  accepted on the command line; for scripts use `--password-stdin` or
  `--password-file FILE` (a file of yours, mode 0600 or 0400).
- `consent grant` replaces the previous choice and asks for confirmation (`--yes` skips
  it). `--time-only` records time blocks only. What is actually recorded is your
  consent AND the project's policy AND the local configuration AND what this desktop
  can provide; `dosi-tracker capabilities` shows the result with the reason for
  everything that is off.
- `service enable --now` writes `~/.config/autostart/io.github.dosibridge.DosiTracker.desktop`,
  which starts the agent through the systemd user unit at every login (or, without a
  user manager, as a background process). `dosi-tracker service disable --now` undoes it.

The same flow works from the window: open "Dosi Tracker" from the applications menu.

For automation (the password comes from stdin, here from a variable):

```bash
printf '%s\n' "$PW" | dosi-tracker --json login --server https://tracker.example.com \
  --email bot@example.com --workspace Acme --password-stdin
dosi-tracker consent grant --time-only --yes && dosi-tracker --json start
```

## The window and the tray

**The window** (`dosi-tracker-gui`, or `dosi-tracker gui`) is a client of the agent: it
shows the agent's state and sends your actions to it, so closing the window never stops
tracking. Only one window opens per user; starting it again brings the open one to
the front. It has the sign-in, workspace and project pickers, consent with what this
desktop can provide, the timer with breaks and today/week totals against your goals,
recent blocks with the latest screenshot exactly as queued, and settings (start at
login, goals, what is recorded, consent history, this device, account, service, logs).
When the agent is not running it says so and offers Start and "Start when I log in".
It uses X11 (XWayland) when `DISPLAY` is set, otherwise Wayland; `--backend wayland`,
`DOSI_GUI_BACKEND` or `gui.backend` choose.

**The tray icon** belongs to the agent (StatusNotifierItem) and shows the state as a
badge: green tracking, amber paused or away or locked, grey offline or choosing, red
stopped or sign-in expired or device removed. Its menu has pause/resume, breaks, project
switching, the task note, sync, the dashboard, settings, start at login and quit.

| Desktop | Tray |
|---|---|
| KDE Plasma 5.27 and 6 (X11 and Wayland) | built in |
| Ubuntu GNOME | built in (the Ubuntu AppIndicators extension is on by default) |
| Fedora, Debian and Arch GNOME | only with the AppIndicator extension, see [Permissions](#permissions) |
| XFCE 4.16+, Cinnamon, LXQt, Budgie, MATE | the panel's status-notifier applet |
| Sway, Hyprland | waybar's tray module |

Without a tray, the agent keeps a "Dosi Tracker is tracking ..." notification on screen
while tracking, and the applications-menu entry has Pause, Resume and Sync actions. A
tracking session that starts by itself at login needs a way to show it is on (a
notification server); an explicit `dosi-tracker start` always works and warns when
nothing on screen can show that tracking is on.

## What is recorded

Everything below needs your consent for that kind of detail and a project policy that
asks for it. A value that cannot be measured is sent as not measured (`null`), never as
`0`; the window shows it as "—".

| Detail | X11 session | Wayland session (GNOME, KDE, wlroots) |
|---|---|---|
| Time blocks: start, end, project, task note, why the block ended, computer name, OS, time zone | yes | yes |
| Keyboard activity: key presses counted per minute (never which key) | yes (XInput 2 raw events; repeats and synthetic XTEST input are not counted as yours) | not measured |
| Mouse activity: clicks counted per minute (wheel and movement only mark activity) | yes | not measured |
| Activity %, idle seconds, per-minute timeline | yes, when input was measured for the whole block | not measured |
| Active window title and program, per-app time | yes | not measured |
| Running programs (names of your own programs with a window or in an app scope) | yes | yes |
| Screenshot of the screen with the active window, plus a thumbnail; optionally blurred | yes | through the desktop's screenshot permission (portal) |
| Idle and away detection (stays on this computer) | XInput 2, else MIT-SCREEN-SAVER | GNOME Mutter, `org.freedesktop.ScreenSaver` or logind; none: manual mode |
| Webcam | not available on Linux in this version (cannot be granted) | same |

Never recorded: what you type (no key codes, text or passwords), the clipboard, pointer
positions, sub-second timing, titles of background windows, command lines, other
users' processes, the microphone, your files, hardware identifiers. Nothing is
recorded while tracking is paused, the screen is locked, the session is inactive or
you are signed out; lock, sleep and logout end the current block without a screenshot.

The device id is a random UUID created once in `~/.local/share/dosi-tracker/device.json`
(never the machine id). For full detail on a Wayland desktop, choose the X11 session at
the login screen ("Ubuntu on Xorg", "Plasma (X11)"); Dosi Tracker never uses root or
input devices to get around Wayland's isolation.

Servers that do not yet accept `null` counts refuse such blocks; they are kept as
"not accepted" (`dosi-tracker queue list --parked`) for 14 days instead of being sent
with invented zeros.

## Permissions

- **Screenshots on Wayland** go through `org.freedesktop.portal.Screenshot` (package
  `xdg-desktop-portal` plus your desktop's backend). The first screenshot may show the
  desktop's permission dialog; consent to screenshots asks for it right away, while you
  are there. If you refuse, screenshots stay off ("denied") without asking again until
  you run `dosi-tracker capture retry` or use "Test screenshot permission" in the
  window. The portal's temporary file is deleted after reading.
- **The keyring** keeps your sign-in (refresh token, the device's signing key and, if you
  keep "remember the password", the password) in one Secret Service item: GNOME
  Keyring, KWallet with its Secret Service interface enabled, or KeePassXC with Secret
  Service integration on. A locked keyring is never unlocked behind your back:
  `dosi-tracker unlock` or the window's Unlock button shows the keyring's own prompt.
  Without any Secret Service, sign-in is refused (exit 9) unless you choose the file
  store: `dosi-tracker config set credentials.store file` keeps the refresh token (never
  the password) in a 0600 file, and `doctor` warns about it.
- **The tray on GNOME** outside Ubuntu needs the AppIndicator extension:

  ```bash
  sudo dnf install gnome-shell-extension-appindicator     # Fedora
  sudo apt install gnome-shell-extension-appindicator     # Debian
  gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com
  ```

  Log out and in once afterwards.

## Command line

```bash
dosi-tracker --help
dosi-tracker status --watch          # every change, as it happens
dosi-tracker pause --for 30m
dosi-tracker break 15m
dosi-tracker resume
dosi-tracker sync
dosi-tracker away keep               # or: dosi-tracker away discard
dosi-tracker task set Reviewing the release notes
dosi-tracker workspace list
dosi-tracker workspace use Acme
dosi-tracker whoami
dosi-tracker queue status
dosi-tracker queue list --parked
dosi-tracker device show
dosi-tracker doctor --strict
dosi-tracker logs -f
dosi-tracker open dashboard
dosi-tracker completions bash > ~/.local/share/bash-completion/completions/dosi-tracker
```

| Area | Commands |
|---|---|
| Service | `service enable [--now]`, `service disable [--now]`, `service start`, `service stop`, `service restart`, `service status`, `daemon [--systemd] [--detach] [--log-level L]`, `uninstall [--purge] [--yes]`, `reset [--yes]` |
| Account | `login [--server URL] [--email E] [--workspace NAME_OR_ID] [--password-stdin \| --password-file P] [--no-remember-password] [--allow-insecure-http]`, `logout [--discard-unsent --yes]`, `unlock`, `whoami`, `workspace list`, `workspace use NAME_OR_ID` |
| Tracking | `start`, `stop` (pause until resumed), `pause [--for DURATION]`, `break DURATION`, `resume`, `sync`, `away keep`, `away discard`, `project list [--refresh]`, `project use ID_OR_TITLE`, `task show`, `task set TEXT`, `task clear`, `status [--watch]` |
| Privacy | `consent show`, `consent grant [--all] [--keyboard] [--mouse] [--active-window] [--running-programs] [--screenshots] [--blur \| --no-blur] [--time-only] [--yes]`, `consent revoke [--purge-queue] [--yes]` (works without the agent), `capabilities`, `capture retry` |
| Data | `queue status`, `queue list [--parked] [--limit N]`, `queue purge --pending \| --parked \| --all --yes`, `device show` |
| Configuration and help | `config show`, `config path`, `config get KEY`, `config set KEY VALUE`, `config unset KEY`, `doctor [--json] [--strict]`, `logs [-f] [-n N]`, `open dashboard \| logs \| data`, `completions bash \| zsh \| fish`, `gui`, `version` |

- Every command takes `--json`: exactly one document
  `{"schema":"dosi-tracker.cli/1","command":...,"ok":true,"changed":...,"result":{...}}`
  on stdout (`{"ok":false,"error":{"code","message","hint","exit_code"}}` on failure;
  JSON Lines for `status --watch` and `logs -f`). Prompts and warnings go to stderr.
- Commands that change something start the agent when it is not running; `--no-start`
  or `DOSI_NO_AUTOSTART=1` turn that off. `status`, `doctor`, `queue status`,
  `consent show` and `config show/get/path` never start it and never create a file;
  with the agent down they show what the local files say.
- Exit codes: 0 success (also "already so"), 1 failed, 2 usage (also a password on the
  command line, or a question without a terminal), 3 the agent is not running, 4 not
  signed in / sign-in refused or expired / keyring locked, 5 consent required or
  capability unavailable, 6 server unreachable, 7 a choice is needed or the name was
  not found, 8 precondition (no project, another account, no graphical session, device
  removed), 9 unsupported environment (root, no `XDG_RUNTIME_DIR`, unsafe permissions,
  no Secret Service), 75 the agent restarts after an upgrade.

## Files and locations

| Path | What | Mode |
|---|---|---|
| `/etc/dosi-tracker/config.toml` | administrator defaults (optional) | root 0644 |
| `~/.config/dosi-tracker/config.toml` | your configuration | 0600 |
| `~/.config/dosi-tracker/consent.json` | your consent | 0600 |
| `~/.config/autostart/io.github.dosibridge.DosiTracker.desktop` | start at login; written only by `service enable` | 0600 |
| `~/.local/state/dosi-tracker/session.json` | account, workspace, project, pause, task note (no secrets) | 0600 |
| `~/.local/state/dosi-tracker/consent-log.jsonl` | every consent change | 0600 |
| `~/.local/state/dosi-tracker/logs/` | daily logs, 7 kept (under systemd: the journal) | 0700 |
| `~/.local/share/dosi-tracker/outbox.db` | blocks waiting to upload | 0600 |
| `~/.local/share/dosi-tracker/device.json` | this install's device id | 0600 |
| `~/.local/share/dosi-tracker/credentials.json` | only with `credentials.store = "file"` | 0600 |
| `$XDG_RUNTIME_DIR/dosi-tracker/` | the agent's socket and locks | 0700 |
| `$XDG_RUNTIME_DIR/dosi-tracker-session.env` | the desktop session's variables for the unit | 0600 |

`~/.config`, `~/.local/state` and `~/.local/share` follow `XDG_CONFIG_HOME`,
`XDG_STATE_HOME` and `XDG_DATA_HOME`. Logs never contain window titles, program names,
image bytes, tokens or passwords. `dosi-tracker config path` prints the actual paths.

## Configuration

Layers, lowest first: built-in defaults, `/etc/dosi-tracker/config.toml`,
`~/.config/dosi-tracker/config.toml`, then `DOSI__*` environment variables
(`DOSI__API_BASE_URL=https://...`, `DOSI__CAPTURE__SCREENSHOT=false`). The `[capture]`
switches only restrict (a `false` in any layer cannot be turned back on by a later
one) and `blur_screenshots` only adds. Every key, its default and its range are in
[`config.example.toml`](config.example.toml).

```bash
dosi-tracker config show
dosi-tracker config set api_base_url https://tracker.example.com
dosi-tracker config set capture.screenshot false
dosi-tracker config unset capture.screenshot
dosi-tracker service restart          # the agent reads its configuration at start
```

There is no built-in server address. Plain `http://` is accepted only for this computer
(loopback) unless `allow_insecure_http = true` (`login --allow-insecure-http` sets it);
TLS certificates are always verified and redirects are never followed.

## Troubleshooting

Start with `dosi-tracker doctor` (`--json` for scripts). It changes nothing and checks
the login session, the idle source, X11 extensions, the screenshot portal, the
keyring and its lock state, the tray host, the notification server, the window's
libraries, the unit and the login entry, the running agent and its queue.

- **"Keyring locked" or sign-in refused with exit 9.** Unlock with `dosi-tracker unlock`.
  On window managers without a keyring, install and start `gnome-keyring` (or
  KeePassXC with Secret Service integration), or opt into the file store (see
  [Permissions](#permissions)). While the keyring is locked the agent keeps tracking
  offline on the cached projects and uploads after the unlock.
- **Keyboard, mouse or window details missing on Wayland.** Expected: see
  [What is recorded](#what-is-recorded). `dosi-tracker capabilities` names the reason.
- **Screenshots "denied" or "needs approval".** Run `dosi-tracker capture retry` and
  answer the desktop's dialog; check that `xdg-desktop-portal` and your desktop's portal
  backend are installed.
- **The agent says `no_session`, or it started without a display.** Some desktops do not
  pass `DISPLAY`/`WAYLAND_DISPLAY` to the systemd user manager. The login entry written
  by `service enable` hands them over at every login; after enabling, log out and in
  once, or run `dosi-tracker service session-start` in a terminal inside the desktop
  session. Over SSH or on a text console the agent never tracks.
- **The window does not open.** The window loads EGL/GL, xkbcommon-x11, Xcursor, Xrandr
  and Xi at run time (`doctor` lists what is missing):

  ```bash
  sudo apt install libegl1 libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 libx11-xcb1
  sudo dnf install mesa-libEGL libxkbcommon-x11 libXcursor libXrandr libXi
  ```

  The command line and the tray do not need them.
- **Manual mode.** When the desktop offers no idle source (some wlroots compositors),
  the agent cannot tell when you are away: tracking never starts or resumes by itself,
  pauses after `tracking.manual_max_hours` (10 by default), and the window shows a
  banner. Pause when you stop working; lock, sleep and logout still end blocks.
- **Blocks not uploading.** `dosi-tracker status` shows the upload issue and
  `dosi-tracker queue list --parked` the blocks the server did not accept, with its
  reason. `dosi-tracker sync` retries now.
- **Logs.** `dosi-tracker logs -f`, or `journalctl --user -u dosi-tracker -f` when the
  agent runs under systemd.

## Upgrade, uninstall and purge

**Upgrade.** Packages: install the new `.deb`/`.rpm` the same way; a running agent
notices that its program was replaced and restarts at the end of the current block.
Tarball: run the new tarball's installer with `update`; it stops the agent (the block is
saved), swaps the files, restarts the agent if it was running, checks the result and
puts the previous version back if anything fails. `rollback` returns to the version the
last update replaced.

```bash
./dosi-tracker-*/install.sh update
./dosi-tracker-*/install.sh rollback
```

An installation made by the earlier source-build installer (`share/dosi-tracker/manifest.json`)
is taken over by `install.sh`.

**Uninstall** keeps your configuration, consent and queued blocks, so a reinstall
continues where you left off:

```bash
dosi-tracker uninstall                  # tarball: runs ~/.local/lib/dosi-tracker/install/uninstall.sh
sudo apt remove dosi-tracker            # Debian and Ubuntu packages
sudo dnf remove dosi-tracker            # Fedora package
```

For packages, `dosi-tracker uninstall` prints the package command instead of running
`sudo`. Removing a package removes only the package's files; per-user data stays.

**Purge** also signs out, deletes the keyring item and removes the configuration,
consent, state, logs and the queue, including blocks that were not uploaded (their
count is shown first):

```bash
dosi-tracker uninstall --purge          # asks first; --yes in scripts
dosi-tracker reset                      # the same deletion, keeping the programs
```

Run `dosi-tracker reset` before removing a package if you want your data gone too.

## Development

The crate is `clients/linux` (`dosi-tracker-linux`, Rust 1.92 or newer). Build and
install from a checkout with `bash scripts/install.sh install` (it runs
`cargo build --locked --release --bins`; SQLite is bundled and no `-dev` packages are
needed).

Every check runs in the CI image (`ci/Dockerfile`: Ubuntu 22.04, glibc 2.35, the pinned
toolchain, Xvfb, private D-Bus buses and the packaging validators), locally exactly as in
`.github/workflows/linux.yml`. Builds go to named volumes, never to the checkout:

```bash
R="$(git rev-parse --show-toplevel)"
docker build -t dosi-linux-ci:22.04 -f "$R/clients/linux/ci/Dockerfile" "$R/clients/linux/ci"
dr() { docker run --rm --user builder -v "$R:/repo" -v dosi-linux-target:/target \
  -v dosi-linux-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target \
  -w /repo/clients/linux dosi-linux-ci:22.04 "$@"; }
dr bash ci/check.sh all                 # fmt clippy test harness installer release
dr bash ci/check.sh package             # tarball, .deb, .rpm and SHA256SUMS in /target/dist
```

On Git Bash for Windows, prefix `docker` with `MSYS_NO_PATHCONV=1`. The stages:

| Stage | Runs |
|---|---|
| `fmt` | `cargo fmt --check` |
| `clippy` | `-D warnings` for the default, `--no-default-features` and `--all-features` builds |
| `test` | `cargo test`, then with `--features test-support` (fakes, scripted daemon) |
| `harness` | the ignored display and D-Bus tests inside `ci/xvfb-session.sh` (Xvfb, openbox, private buses; only synthetic input) |
| `installer` | `tests/test_installer_lifecycle.sh` as a non-root user |
| `release` | release build and tests, the shared-library allowlist, `--version`, `doctor --json`, no test hooks in the binaries, fail-closed CLI checks |
| `package` | `packaging/build-dist.sh` with pinned `cargo-deb` and `cargo-generate-rpm`, package contents, validators, every `dosi-tracker` command of this README, a tarball install |

The distro smoke test installs the artifacts on stock images, as CI's `package-smoke`
job does:

```bash
for img in ubuntu:22.04 ubuntu:24.04 debian:12 debian:13 fedora:43 fedora:44 archlinux:latest; do
  docker run --rm -v dosi-linux-target:/target:ro -v "$R/clients/linux/packaging:/pkg:ro" \
    "$img" bash /pkg/smoke.sh /target/dist || break
done
```

Releases are made by pushing a `linux-vX.Y.Z` tag that matches the crate version (and
has a `<release>` entry in the AppStream metainfo); the workflow runs every check,
builds and smoke-tests the artifacts and publishes them with `SHA256SUMS` as a GitHub
release. That release is not marked "latest": the latest-release address belongs to the
product release that carries every platform's downloads.

Layout: `src/agent` (daemon, worker, uploader, tray), `src/tracking` (session
detection and sensors), `src/ipc` (the agent's socket protocol), `src/cli`, `src/gui`
(feature `gui`), `packaging/` (unit, desktop entries, icons, AppStream metadata,
`build-dist.sh`, `smoke.sh`), `scripts/` (installer), `tests/` (integration suites and
fakes) and `ci/`. The icons are rendered by `packaging/icons/render-icons.py` from the
same geometry as the SVG.
