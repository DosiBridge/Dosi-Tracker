# Dosi-Tracker Linux Desktop Agent (`dosi-tracker`)

Lightweight, privacy-first, rootless desktop agent foundation for Linux written in Rust.

## 1. Current Status

| Attribute | State | Details |
|---|---|---|
| **Crate** | `dosi-tracker-linux` v0.1.0 | Located in [`clients/linux`](./); binaries `dosi-tracker` (CLI) and `dosi-tracker-gui` (feature `gui`, placeholder until the GUI lands) |
| **Release Tier** | `v1` (in progress) | Daemon, full CLI and doctor v2 delivered; tray, GUI and packages follow |
| **Telemetry Adapters** | First Bounded Slice Delivered | X11 EWMH active window + `/proc` running-apps inventory wired with consent gating; Wayland honest unsupported (Phase 2 first slice) |
| **Privilege Model** | Strictly Rootless | Runs as normal user. Refuses `sudo`, `root`, `setuid`, `evdev`, and `uinput` |
| **Offline Storage** | Transactional SQLite | Durable outbox queue with raw identity bounding and read-only inspection |
| **Credential Store** | SecretService + opt-in 0600 file | One Freedesktop SecretService item per account (`org.freedesktop.secrets` over session D-Bus, `workspace_id` attribute empty); the password only in the keyring and only when remembered; a locked keyring is reported (`Locked`) and only unlocked on an explicit prompt; mock transport injection seam (`SecretServiceTransport`) for deterministic protocol testing; the owner-only 0600 file store (`credentials.store = "file"`, an explicit opt-in) never holds a password; `auto` never falls back to it: without a Secret Service the agent refuses to sign in (exit 9) and doctor says why. The foundation's plaintext `~/.config/dosi-tracker/credentials.json` is reported by doctor and deleted after the next sign-in |
| **Transport & Auth** | Real HTTP Delivery Transport | Reqwest rustls, OpenIddict password grant (`POST /connect/token` with the install's `device_id`, no tenant parameter, no workspace header), workspace discovery (`GET /api/app/workspaces/mine`), `X-Workspace-Id` on workspace-scoped calls, pre-expiry refresh with rotation listener, bounded 401 retry, Windows upload-outcome table plus DeviceRevoked/WorkspaceArchived/WorkspaceRequired, park-and-continue, 413 strip-and-resend, heartbeat; no default server, `http://` only to loopback unless `allow_insecure_http`, redirects never followed |
| **Payload Signing** | HMAC-SHA256 (32 bytes) | URL_SAFE_NO_PAD base64url tag over canonical JSON, mirroring Windows byte semantics and test vectors |
| **Agent Core** | Worker ported and run by the daemon | `agent::{state, blocks, session, power}` ported from the Windows agent with their tests (statuses add `no_session` and `device_revoked`; effective permissions = project AND consent AND config AND what the session can provide); `agent::config` layers `/etc/dosi-tracker/config.toml` < `$XDG_CONFIG_HOME/dosi-tracker/config.toml` < `DOSI__*` (capture switches only narrow, blur only adds; no built-in server); `agent::session_store` keeps `session.json` (0600) with the 7-day project cache; `tracking::system` creates a random device id in `device.json` once (never the machine id). `agent::worker` (the `dosi-worker` loop), `agent::control` (sign-in, `workspaces/mine`, projects, heartbeats, totals, device status) and `agent::uploader` (`dosi-uploader`: one transport per workspace scope, Windows back-off, park-and-continue; a workspace or device the server calls terminal has its queued rows parked) are ported with the Windows tests plus the Linux cases: manual mode never starts or resumes by itself and pauses after `tracking.manual_max_hours`, `no_session` never tracks, a locked keyring tracks offline and keeps blocks in memory until the unlock, a refused refresh is "sign-in expired" (never a silent password sign-in), rotated refresh tokens are saved by a persister thread. `tracking::tracker` is the production machine: one consent lifecycle shared with the gate, XI2 and the usage sampler started only while tracking, idle from XI2 / MIT-SCREEN-SAVER / Mutter / ScreenSaver / logind, screenshots on `dosi-capture` validated against the lifecycle; `tracking::build` turns the measurements into the payload with unmeasured fields null or left out |
| **Session & Desktop** | logind first | `tracking::session_probe` accepts only our own active local X11/Wayland session (never an X11 display on Wayland); `agent::session_events` turns logind/screensaver lock, inactive-session, sleep and shutdown signals into one event each and holds a logind delay inhibitor only while tracking (released after the save ack, at most 1.5 s / 2 s); `agent::notify` talks to `org.freedesktop.Notifications` |
| **Service Integration** | systemd user unit + opt-in autostart | `packaging/systemd/dosi-tracker.service.in` (`Type=notify`, hardened, no `[Install]`) and `packaging/desktop/autostart.desktop.in`; `service` writes and removes only entries marked `X-DosiTracker-Managed=true`, writes the session env file (0600) before `systemctl --user --no-block start`, restarts on a new login session, and falls back to `dosi-tracker daemon --detach` without a user manager (the hook marks the start with `DOSI_LAUNCH=login`). `dosi-tracker service enable/disable/start/stop/restart/status` drive it |
| **Daemon & IPC** | `dosi-tracker daemon` | `agent::daemon`: refuses root (exit 9), umask 077, needs `XDG_RUNTIME_DIR` (exit 9), holds `daemon.lock` for life (a second daemon exits 8), logs to the journal under `--systemd`/`JOURNAL_STREAM` or to daily files in `$XDG_STATE_HOME/dosi-tracker/logs` (7 kept; never titles, tokens or passwords); wires the worker, session events, the notice relay (notices, Keep/Discard away question, consent receipts, a resident "tracking" notification when there is no tray) and the IPC server; `READY=1`/`STATUS`/`STOPPING` to systemd; SIGTERM/SIGINT/SIGHUP save the block, send the `stopped` farewell and exit within 20 s; a replaced executable restarts at the next block boundary (exit 75, re-exec when detached). `ipc`: protocol v1, JSON lines (1 MiB cap) on a 0600 socket in a 0700 runtime dir, peer uid checked before any byte is read, `hello` first, at most 16 clients, idle clients dropped after 5 min, state events coalesced to 4/s |
| **CI Gate** | Dedicated GitHub workflow | `.github/workflows/linux.yml` runs `ci/check.sh` inside the `ci/Dockerfile` image (ubuntu:22.04, glibc 2.35) |

---

## 2. Privacy & Security Invariants

1. **Strictly Rootless & Unprivileged**:
   The agent runs entirely in user space. Escalation to `root` or `sudo`, access to `/dev/input/event*` (evdev), and `/dev/uinput` are strictly prohibited. Any solution requiring elevated privileges is rejected.
2. **Anti-Keylogger Policy**:
   Only keyboard event **counts** and active time percentages are recorded. Keycodes, keystroke sequences, and text content are never captured or reconstructed.
3. **Default-Deny Consent Core**:
   All capabilities are disabled by default. Widening requires explicit user action and successful disk persistence (`ConsentStore`). Failures during persistence immediately revoke memory permissions to prevent unauthorized collection. ABA lifecycle tickets invalidate stale authorization across permission cycles.
4. **Truthful Diagnostics & Non-Mutating Inspection**:
   The diagnostic CLI (`doctor`) and status CLI (`status`) report capabilities and store statuses honestly. Missing databases, configurations, or credentials are never created by diagnostic or inspection commands.
5. **Strict Secret Redaction**:
   Passwords, bearer access tokens, refresh tokens, and raw signing key bytes are never leaked to logs, error messages, stdout, or JSON inspection outputs.

---

## 3. Installation & Lifecycle

### Prerequisites
- Linux distribution (Ubuntu 22.04+, Fedora 38+, Arch Linux, Debian 12+)
- Rust toolchain 1.92+ (CI pins 1.97.1); SQLite is bundled, no `-dev` packages are needed
- Standard POSIX shell (`bash`)

### Rootless Source Installation
The installer script compiles and activates the agent strictly inside a user-owned prefix without touching system directories:

```bash
# Install to default user prefix (~/.local/opt/dosi-tracker)
bash clients/linux/scripts/install.sh install

# Or install to an explicit user prefix
bash clients/linux/scripts/install.sh install --prefix "$HOME/apps/dosi-tracker"
```

The installer verifies prefix boundary confinement, validates manifest ownership (`manifest.json`), stages artifacts atomically (`.staging.<pid>`), and preserves user configuration and consent at `~/.config/dosi-tracker`.

### Verification
```bash
# Check installed binary status
~/.local/opt/dosi-tracker/bin/dosi-tracker doctor
```

### Uninstallation
```bash
# Uninstall managed files (preserves user configuration and consent files)
bash clients/linux/scripts/uninstall.sh

# Or with custom prefix
bash clients/linux/scripts/uninstall.sh --prefix "$HOME/apps/dosi-tracker"
```

---

## 4. Command Line (`dosi-tracker`)

Every command takes `--json` (exactly one `dosi-tracker.cli/1` document on stdout; JSON Lines for `status --watch` and `logs -f`), `-q`, `-v` and `--no-start`. Commands that change something start the agent when it is not running (`systemctl --user start dosi-tracker.service`, else a detached `dosi-tracker daemon --detach`); `--no-start` or `DOSI_NO_AUTOSTART=1` turn that off (exit 3). `status`, `doctor`, `queue status`, `consent show` and `config show/get/path` never start it and never create a file: with the agent down they show what the local files say (`status` then exits 3).

```bash
dosi-tracker service enable --now                   # start at every login (never done by installers)
dosi-tracker login --server https://tracker.example.com --email you@example.com
                                                    # Password: (no echo), then choose the workspace
dosi-tracker consent grant --keyboard --mouse --active-window --running-programs --screenshots
dosi-tracker project use "Website Redesign"
dosi-tracker status                                 # status --watch streams every change
dosi-tracker break 15m ; dosi-tracker resume ; dosi-tracker sync
dosi-tracker away keep                              # or: away discard
dosi-tracker logout

# Automation: the password from stdin (or --password-file, a 0600/0400 file of yours)
printf '%s\n' "$PW" | dosi-tracker --json login --server https://tracker.example.com \
  --email bot@example.com --workspace Acme --password-stdin
dosi-tracker consent grant --time-only --yes && dosi-tracker --json start
```

Passwords are never accepted on the command line: `--password` and `-p` exit 2 without echoing the value.

| Area | Commands |
|---|---|
| Service | `service enable [--now]`, `service disable [--now]`, `service start/stop/restart/status`, `daemon [--systemd] [--detach] [--log-level L]`, `uninstall [--purge] [--yes]`, `reset [--yes]` |
| Account | `login [--server URL] [--email E] [--workspace NAME/ID] [--password-stdin / --password-file P] [--no-remember-password] [--allow-insecure-http]`, `logout [--discard-unsent --yes]`, `unlock`, `whoami`, `workspace list`, `workspace use NAME/ID` |
| Tracking | `start`, `stop`, `pause [--for 15m]`, `break DURATION`, `resume`, `sync`, `away keep/discard`, `project list [--refresh]`, `project use ID/TITLE`, `task show/set TEXT/clear`, `status [--watch]` |
| Privacy | `consent show`, `consent grant [--all] [--keyboard] [--mouse] [--active-window] [--running-programs] [--screenshots] [--blur/--no-blur] [--time-only] [--yes]` (the webcam cannot be granted in this version), `consent revoke [--purge-queue] [--yes]`, `capabilities`, `capture retry` |
| Data | `queue status` (read-only), `queue list [--parked] [--limit N]`, `queue purge --pending/--parked/--all --yes`, `device show` |
| Config & diagnostics | `config show/path/get K/set K V/unset K`, `doctor [--json] [--strict]`, `logs [-f] [-n N]`, `open dashboard/logs/data`, `completions bash/zsh/fish`, `gui`, `version` |

Exit codes: 0 success (also when already in that state), 1 failed, 2 usage (including a password on argv or missing input without a terminal), 3 the agent is not running / protocol mismatch, 4 not signed in / refused / expired / keyring locked, 5 consent required or capability unavailable, 6 server unreachable, 7 a choice is required or the named thing does not exist, 8 precondition (no project, another account signed in, no graphical session, device removed, nothing to keep), 9 unsupported environment (root, no `XDG_RUNTIME_DIR`, unsafe permissions, no Secret Service without the file-store opt-in), 75 daemon restart after an upgrade.

`doctor` keeps every v1 JSON field and adds `schema` (`dosi-tracker.doctor/2`), live read-only `probes` (logind session, idle source, X11 extensions, Screenshot portal, Secret Service and its lock state, tray host, notification server, GUI libraries, unit and autostart, the running agent, the legacy plaintext credentials file) and `problems`; `--strict` exits 1 when one blocks tracking.

The hidden foundation inspector `outbox status --db P --account-id A --workspace-id W [--json]` keeps its exact output: a bounded, strictly read-only look at one scope that never creates, migrates or checkpoints the database.

### Local CI (Docker)
The same image and stages run locally and in `.github/workflows/linux.yml`. Builds go to named volumes, never to the checkout's `target/`:

```bash
R="$(git rev-parse --show-toplevel)"
docker build -t dosi-linux-ci:22.04 -f "$R/clients/linux/ci/Dockerfile" "$R/clients/linux/ci"
docker run --rm --user builder -v "$R:/repo" -v dosi-linux-target:/target \
  -v dosi-linux-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target \
  -w /repo/clients/linux dosi-linux-ci:22.04 bash ci/check.sh all
```

Stages: `fmt`, `clippy`, `test`, `harness` (ignored display/bus tests inside `ci/xvfb-session.sh`), `installer`, `release`, `all`. On Git Bash for Windows prefix `docker` with `MSYS_NO_PATHCONV=1`.

---

## 5. Platform Capability & Architecture Matrix

| Capability | X11 Protocol | Wayland Compositor | Implementation Status |
|---|---|---|---|
| **Active Window** | Supported via `_NET_ACTIVE_WINDOW` & `WM_CLASS` | **UNSUPPORTED** (reported as `null` / unsupported) | **Delivered in Phase 2** (`X11ActiveWindowProvider`, `WaylandActiveWindowProvider`, root normalization, priority title fallback) |
| **Running Applications** | Own-user `/proc` processes that own a `_NET_CLIENT_LIST` window | Own-user `/proc` processes in a systemd app scope (`app.slice/app-*.scope` / `app-*.service`) | **Sensor delivered** (`ProcFsRunningAppsProvider` with a mandatory `ProcessScope`: own euid only, never the agent itself, 200 cap, deduplication, `PF_KTHREAD` filtering); used by the worker through `tracking::tracker` |
| **Screen Capture** | `GetImage` of the RandR monitor showing most of the active window (fallback: whole root), 24/32-bit visuals (`X11ScreenSource`) | `org.freedesktop.portal.Screenshot`, non-interactive; returned file deleted; refusal = Denied until an explicit retry; 30 s timeout = needs approval (`PortalScreenSource`) | **Sensor delivered**; shared encoder (`tracking::capture`: ≤1920 px JPEG ladder within 8 MB, 480 px thumbnail within 512 KB, unreadable blur); used by the worker through `tracking::tracker` |
| **Keyboard Counting** | XInput 2.2 raw key presses on the root for master devices; keycodes never read; auto-repeat dropped; XTEST counted as injected only (`X11InputListener` → `InputLog`) | **UNSUPPORTED** (reported as `null`) | **Sensor delivered**; used by the worker through `tracking::tracker` |
| **Mouse Counting** | XInput 2.2 raw buttons 1-3/8-9 as clicks; wheel and motion as activity only; XTEST counted as injected only | **UNSUPPORTED** (reported as `null`) | **Sensor delivered**; used by the worker through `tracking::tracker` |
| **Idle Detection (local only)** | Last physical XI2 input, else MIT-SCREEN-SAVER (`X11ScreenSaverIdleSource`) | Mutter IdleMonitor, `org.freedesktop.ScreenSaver`, logind `IdleHint` (`probe_idle_sources`); none = manual mode | **Sensor delivered**; used by the worker through `tracking::tracker` |
| **Webcam Photo** | Architecturally possible via V4L2 (`/dev/video*`) | Architecturally possible via V4L2 / PipeWire camera portal | Not in v1: consent to it is refused (planned for v1.1) |
| **Credential Storage** | Freedesktop Secret Service (D-Bus) | Freedesktop Secret Service (D-Bus) | **Delivered** (account-level items; opt-in owner-only 0600 file store without the password, with a doctor warning) |
| **Backend Sync & Transport** | Real HTTP Delivery Transport | Real HTTP Delivery Transport | **Delivered** (`HttpDeliveryTransport::new_shared` per workspace scope, driven by the daemon's uploader; `dosi-tracker sync` uploads at once) |
| **Payload Signing** | HMAC-SHA256 (32 bytes) | HMAC-SHA256 (32 bytes) | **Delivered in Phase 1** (Windows byte semantics parity) |

---

## 6. Implementation Roadmap

- **Phase 0 (Completed Foundation & Hygiene)**:
  - Scoped GitHub Actions CI workflow (`.github/workflows/linux.yml`, Docker image `ci/Dockerfile`, stages in `ci/check.sh`).
  - Strict formatting and clippy gates (`cargo fmt --check`; `cargo clippy --locked --all-targets -- -D warnings` for the default, `--no-default-features` and `--all-features` sets).
  - Safe scoped SQLite progress handler wrapper (`ScopedProgressHandler`) with guaranteed RAII deregistration.
  - Graceful recoverable mutex poisoning handling in outbox (`OutboxError::LockPoisoned`).
  - Documentation and test count synchronization across repository.
- **Phase 1 (Delivered Foundation: Identity, Real Transport, Signed Queue)**:
  - Freedesktop Secret Service credential storage per account and workspace over session D-Bus (`org.freedesktop.secrets`) matching standard `SearchItems`, `OpenSession`, `Unlock`, `GetSecret`, `CreateItem`, and `Delete` interfaces with fail-closed prompt handling, accompanied by `SecretServiceTransport` injected seam and `MockSecretServiceTransport` protocol fake for deterministic testing without live keyring access; owner-only `0600` private file fallback with atomic replacement durability and explicit `doctor` diagnostic warning.
  - OpenIddict OAuth2 password grant authentication (`POST /connect/token`, `client_id=Tracker_App`, scope `Tracker offline_access`, the install's `device_id`; no tenant parameter and no workspace header).
  - Outbox store v2 (migrated from v1 in one transaction): captures in their own table, signed rows (HMAC over the payload and capture digests) verified before upload, parking of refused rows with park-and-continue sync, Windows queue limits, `open_or_recover`, 0700/0600 files; the read-only inspector reads v1 and v2 stores and reports `parked_count` and `capture_bytes`.
  - Pre-expiry token refresh buffer (60s early renewal) and bounded 401 retry / re-authentication.
  - Per-device 32-byte HMAC-SHA256 payload signing matching Windows byte semantics (`URL_SAFE_NO_PAD` base64url tag).
  - Concrete `HttpDeliveryTransport` implementing `DeliveryTransport` contract with exact ack deletion, transient failure retention and retry counter incrementation, auth-required halt, and mismatched ack halt.
  - Strict transport identity binding: `HttpDeliveryTransport` is bound to its authenticated account and workspace (`OutboxScope`) under exact canonicalization rules (`account_id: trim().to_lowercase()`, `workspace_id: trim().to_string()`), failing closed if unscoped or empty across all constructors (`new`, `from_credentials`, `from_store`), and rejecting mismatched queued scopes before any authentication, token refresh, or network transmission, preserving the queued row and retry metadata in SQLite without accidental acknowledgment or cross-tenant exposure.
  - Production transport initialization from persisted credentials (`HttpDeliveryTransport::from_store` / `from_credentials`), proving production upload reuses credentials, tokens, and signing key saved by CLI `login`.
  - Heartbeat presence primitive (`POST /api/app/presence/heartbeat`) on 60s cadence.
  - The foundation's `login`/`logout`/`status` commands, replaced by the daemon-backed CLI (section 4).
  - *Evidence & Boundaries:* Verified by `tests/cli_identity_transport_signing_tests.rs`: `dosi-tracker login` (password on stdin, through the daemon) against a stateful fake backend, then a v2 transport built from the credentials it saved (`HttpDeliveryTransport::new_shared`): signed delivery and ack deletion, HTTP 503 and TCP socket interruption keeping the row. Independent native QA remains pending.
- **Phase 2 (Native Consent-Gated Capture Adapters - FIRST BOUNDED SLICE & DEEP CORRECTIONS DELIVERED)**:
  - **Delivered Bounded Slice & Deep Corrections**:
    - Native consent-gated capture adapters with fakeable traits matching Windows (`AcquisitionProvider`, `ActiveWindowProvider`, `RunningAppsProvider`, `ConsentGatedTracker`, `FakeAcquisitionProvider`).
    - Real unprivileged X11 property transport via pure-Rust `x11rb v0.14.0` (`SysX11Transport`, `SysX11PropertyReader`), lazy connection behind affirmative consent gate (zero display socket or window queries during provider construction or `doctor` diagnostics), wire-level reply parsing (`GetPropertyReply::try_parse`), bounded decoding of `_NET_ACTIVE_WINDOW` (normalizing root window focus `id == root` to `None` with zero subsequent property queries), UTF-8 `_NET_WM_NAME` priority decoding with `WM_NAME` fallback queried only if preferred is absent, empty, NUL-only, or whitespace-only, `WM_CLASS`, and `_NET_WM_PID` mapped to `/proc/[pid]/exe`, with graceful unfocused handling (`_NET_ACTIVE_WINDOW = 0` / None).
    - User-space `/proc` process inventory scanner (`ProcFsRunningAppsProvider`) with injectable `ProcFsReader` seam, resilient to vanished PIDs (ENOENT), extracting pure executable filenames (`file_name`), native unbracketed kernel-thread detection via `/proc/[pid]/stat` `PF_KTHREAD` (`0x0020_0000`) robustly indexing fields after `stat_line.rfind(')')`, preserving bracketed user applications (`[editor]`, `[custom-app]`, `[unstat-task]`) when flags are clear or thread status is unavailable, provenance-aware aggregation (`aggregate_running_programs`) with case-insensitive first-seen deduplication and 200 cap, and empty titles / null usage metrics.
    - Truthful, honest Wayland unsupported behavior (`WaylandActiveWindowProvider`) reporting architectural unprivileged limits (`TrackingError::UnsupportedSession`) without synthetic or fabricated telemetry.
    - Wired into capability architecture and `doctor` support reporting (`DiagnosticStatus::Supported` under X11/procfs; `DiagnosticStatus::Unsupported` under Wayland; `adapters.window_sampler` derived directly from `active_window.adapter_status`).
    - Deterministic test harness (`tracking_native_adapter_tests`): default deny, capability-specific consent, wrapper-level in-flight pause->resume and revoke->regrant ABA generation mismatch invalidation, unsupported sessions, parsing/aggregation, and error paths.
  - **Sensors delivered (L3; wired into the worker in L5)**:
    - XInput 2.2 counts-only listener with a 125-minute ring of per-second buckets (`InputLog`), the ported Windows activity %, minute timeline and per-app usage math (`tracking::input`, `tracking::usage`, 3 s `UsageSampler` with a synchronous stop).
    - X11 `GetImage` and Wayland Screenshot-portal screen sources with the ported Windows JPEG/thumbnail/blur encoder.
    - X11 MIT-SCREEN-SAVER and Wayland D-Bus idle sources; explicit display and bus injection everywhere (nothing reads `DISPLAY` or `DBUS_SESSION_BUS_ADDRESS`).
    - Tests: crafted XI2 wire events, capture encoding, fake portal / idle services on private buses, and `tests/x11_xvfb_it.rs` against the harness Xvfb (`xdotool` input is counted as injected only).
  - **Remaining Phase 2 Backlog (OPEN)**:
    - Separate opt-in V4L2 webcam (deferred to v1.1).
- **Phase 3 (Worker, Presence, Session & Configuration - DELIVERED)**:
  - Interval capture worker with ABA generation tickets, the background uploader, logind lock/sleep/shutdown handling, idle detection.
  - The daemon (single instance through `daemon.lock`, IPC protocol v1 over a same-uid Unix socket), XDG `config.toml` with `DOSI__*` overrides, the rootless systemd user unit.
- **Phase 4 (CLI Surface & Packaging - CLI DELIVERED, PACKAGING OPEN)**:
  - The full CLI with `--json` envelopes and stable exit codes (section 4), doctor v2.
  - Open: the tray and the GUI client, `.deb`/`.rpm`/tarball packaging and their CI jobs.

