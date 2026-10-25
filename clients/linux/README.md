# Dosi-Tracker Linux Desktop Agent (`dosi-tracker`)

Lightweight, privacy-first, rootless desktop agent foundation for Linux written in Rust.

## 1. Current Status

| Attribute | State | Details |
|---|---|---|
| **Crate** | `dosi-tracker-linux` v0.1.0 | Located in [`clients/linux`](./); binaries `dosi-tracker` (CLI) and `dosi-tracker-gui` (feature `gui`, placeholder until the GUI lands) |
| **Release Tier** | `headless-foundation` | Diagnostic foundation, default-deny consent core, durable outbox, rootless installer |
| **Telemetry Adapters** | First Bounded Slice Delivered | X11 EWMH active window + `/proc` running-apps inventory wired with consent gating; Wayland honest unsupported (Phase 2 first slice) |
| **Privilege Model** | Strictly Rootless | Runs as normal user. Refuses `sudo`, `root`, `setuid`, `evdev`, and `uinput` |
| **Offline Storage** | Transactional SQLite | Durable outbox queue with raw identity bounding and read-only inspection |
| **Credential Store** | SecretService + opt-in 0600 file | One Freedesktop SecretService item per account (`org.freedesktop.secrets` over session D-Bus, `workspace_id` attribute empty); the password only in the keyring and only when remembered; a locked keyring is reported (`Locked`) and only unlocked on an explicit prompt; mock transport injection seam (`SecretServiceTransport`) for deterministic protocol testing; the owner-only 0600 file store never holds a password; `credentials::resolve_store` (`auto`) never falls back to it, while the foundation `login` still uses it without a session bus, with the explicit doctor warning |
| **Transport & Auth** | Real HTTP Delivery Transport | Reqwest rustls, OpenIddict password grant (`POST /connect/token` with the install's `device_id`, no tenant parameter, no workspace header), workspace discovery (`GET /api/app/workspaces/mine`), `X-Workspace-Id` on workspace-scoped calls, pre-expiry refresh with rotation listener, bounded 401 retry, Windows upload-outcome table plus DeviceRevoked/WorkspaceArchived/WorkspaceRequired, park-and-continue, 413 strip-and-resend, heartbeat; no default server, `http://` only to loopback unless `allow_insecure_http`, redirects never followed |
| **Payload Signing** | HMAC-SHA256 (32 bytes) | URL_SAFE_NO_PAD base64url tag over canonical JSON, mirroring Windows byte semantics and test vectors |
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

## 4. Implemented CLI Commands

### Authentication (`login` & `logout`)
Authenticate with the backend via OpenIddict password grant and manage stored credentials:

```bash
# Login and store scoped credentials with per-device signing key
cargo run --bin dosi-tracker -- login \
  --username "worker@example.com" \
  --password "secret" \
  --workspace-id "00000000-0000-0000-0000-000000000001" \
  --server "https://tracker.example.com"   # required: there is no default server

# Logout and securely remove credentials
cargo run --bin dosi-tracker -- logout \
  --account-id "worker@example.com" \
  --workspace-id "00000000-0000-0000-0000-000000000001"
```

### Status Inspection (`status`)
Non-mutating inspection of identity, token validity, signing key status, and optional queue depth:

```bash
# Human-readable status
cargo run --bin dosi-tracker -- status \
  --account-id "worker@example.com" \
  --workspace-id "00000000-0000-0000-0000-000000000001" \
  --db ~/.local/share/dosi-tracker/outbox.db

# Structured JSON status (pure inspection, zero secret leaks)
cargo run --bin dosi-tracker -- status \
  --account-id "worker@example.com" \
  --workspace-id "00000000-0000-0000-0000-000000000001" \
  --json
```

### Diagnostic Doctor (`doctor`)
Inspects session environment, resolved non-mutating XDG paths, capability support, adapter status, credential storage backend, and privacy invariants:

```bash
# Human-readable diagnostic output
cargo run --bin dosi-tracker -- doctor

# Structured JSON diagnostic output
cargo run --bin dosi-tracker -- doctor --json
```

### Outbox Status Inspection (`outbox status`)
Performs bounded, strictly read-only inspection of the offline SQLite outbox queue for an exact account and workspace scope. Opens the database with `SQLITE_OPEN_READ_ONLY`, enforces a finite execution work budget (`DEFAULT_MAX_SQLITE_WORK_STEPS = 25_000`), folds corruption checks into a bounded CTE, and fails closed without creating missing database files:

```bash
cargo run --bin dosi-tracker -- outbox status \
  --db ~/.local/share/dosi-tracker/outbox.db \
  --account-id "00000000-0000-0000-0000-000000000001" \
  --workspace-id "00000000-0000-0000-0000-000000000002" \
  --json
```

### Help & Version
```bash
cargo run --bin dosi-tracker -- --help
cargo run --bin dosi-tracker -- --version
```

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
| **Running Applications** | Own-user `/proc` processes that own a `_NET_CLIENT_LIST` window | Own-user `/proc` processes in a systemd app scope (`app.slice/app-*.scope` / `app-*.service`) | **Sensor delivered** (`ProcFsRunningAppsProvider` with a mandatory `ProcessScope`: own euid only, never the agent itself, 200 cap, deduplication, `PF_KTHREAD` filtering); worker wiring pending |
| **Screen Capture** | `GetImage` of the RandR monitor showing most of the active window (fallback: whole root), 24/32-bit visuals (`X11ScreenSource`) | `org.freedesktop.portal.Screenshot`, non-interactive; returned file deleted; refusal = Denied until an explicit retry; 30 s timeout = needs approval (`PortalScreenSource`) | **Sensor delivered**; shared encoder (`tracking::capture`: ≤1920 px JPEG ladder within 8 MB, 480 px thumbnail within 512 KB, unreadable blur); worker wiring pending |
| **Keyboard Counting** | XInput 2.2 raw key presses on the root for master devices; keycodes never read; auto-repeat dropped; XTEST counted as injected only (`X11InputListener` → `InputLog`) | **UNSUPPORTED** (reported as `null`) | **Sensor delivered**; worker wiring pending |
| **Mouse Counting** | XInput 2.2 raw buttons 1-3/8-9 as clicks; wheel and motion as activity only; XTEST counted as injected only | **UNSUPPORTED** (reported as `null`) | **Sensor delivered**; worker wiring pending |
| **Idle Detection (local only)** | Last physical XI2 input, else MIT-SCREEN-SAVER (`X11ScreenSaverIdleSource`) | Mutter IdleMonitor, `org.freedesktop.ScreenSaver`, logind `IdleHint` (`probe_idle_sources`); none = manual mode | **Sensor delivered**; worker wiring pending |
| **Webcam Photo** | Supported via V4L2 (`/dev/video*`) | Supported via V4L2 / PipeWire camera portal | Planned Phase 2 Backlog |
| **Credential Storage** | Freedesktop Secret Service (D-Bus) | Freedesktop Secret Service (D-Bus) | **Delivered** (account-level items; opt-in owner-only 0600 file store without the password, with a doctor warning) |
| **Backend Sync & Transport** | Real HTTP Delivery Transport | Real HTTP Delivery Transport | **Delivered in Phase 1** (Library transport `HttpDeliveryTransport::from_store` / `from_credentials` reusing persisted login credentials; background daemon loop is Phase 3, CLI queue flush command is Phase 4) |
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
  - Non-mutating CLI commands: `login`, `logout`, `status [--json]`.
  - *Evidence & Boundaries:* Verified via persisted-session library integration (`SyncPassCoordinator` + `HttpDeliveryTransport::from_store`) using credentials saved by CLI `login`, with deterministic mid-upload HTTP 502 response and TCP socket interruption tests. Automated background daemon loop belongs to Phase 3; CLI queue flush command belongs to Phase 4. Independent native QA remains pending.
- **Phase 2 (Native Consent-Gated Capture Adapters - FIRST BOUNDED SLICE & DEEP CORRECTIONS DELIVERED)**:
  - **Delivered Bounded Slice & Deep Corrections**:
    - Native consent-gated capture adapters with fakeable traits matching Windows (`AcquisitionProvider`, `ActiveWindowProvider`, `RunningAppsProvider`, `ConsentGatedTracker`, `FakeAcquisitionProvider`).
    - Real unprivileged X11 property transport via pure-Rust `x11rb v0.14.0` (`SysX11Transport`, `SysX11PropertyReader`), lazy connection behind affirmative consent gate (zero display socket or window queries during provider construction or `doctor` diagnostics), wire-level reply parsing (`GetPropertyReply::try_parse`), bounded decoding of `_NET_ACTIVE_WINDOW` (normalizing root window focus `id == root` to `None` with zero subsequent property queries), UTF-8 `_NET_WM_NAME` priority decoding with `WM_NAME` fallback queried only if preferred is absent, empty, NUL-only, or whitespace-only, `WM_CLASS`, and `_NET_WM_PID` mapped to `/proc/[pid]/exe`, with graceful unfocused handling (`_NET_ACTIVE_WINDOW = 0` / None).
    - User-space `/proc` process inventory scanner (`ProcFsRunningAppsProvider`) with injectable `ProcFsReader` seam, resilient to vanished PIDs (ENOENT), extracting pure executable filenames (`file_name`), native unbracketed kernel-thread detection via `/proc/[pid]/stat` `PF_KTHREAD` (`0x0020_0000`) robustly indexing fields after `stat_line.rfind(')')`, preserving bracketed user applications (`[editor]`, `[custom-app]`, `[unstat-task]`) when flags are clear or thread status is unavailable, provenance-aware aggregation (`aggregate_running_programs`) with case-insensitive first-seen deduplication and 200 cap, and empty titles / null usage metrics.
    - Truthful, honest Wayland unsupported behavior (`WaylandActiveWindowProvider`) reporting architectural unprivileged limits (`TrackingError::UnsupportedSession`) without synthetic or fabricated telemetry.
    - Wired into capability architecture and `doctor` support reporting (`DiagnosticStatus::Supported` under X11/procfs; `DiagnosticStatus::Unsupported` under Wayland; `adapters.window_sampler` derived directly from `active_window.adapter_status`).
    - Deterministic test harness (`tracking_native_adapter_tests`): default deny, capability-specific consent, wrapper-level in-flight pause->resume and revoke->regrant ABA generation mismatch invalidation, unsupported sessions, parsing/aggregation, and error paths.
  - **Sensors delivered (L3; not yet wired into the worker)**:
    - XInput 2.2 counts-only listener with a 125-minute ring of per-second buckets (`InputLog`), the ported Windows activity %, minute timeline and per-app usage math (`tracking::input`, `tracking::usage`, 3 s `UsageSampler` with a synchronous stop).
    - X11 `GetImage` and Wayland Screenshot-portal screen sources with the ported Windows JPEG/thumbnail/blur encoder.
    - X11 MIT-SCREEN-SAVER and Wayland D-Bus idle sources; explicit display and bus injection everywhere (nothing reads `DISPLAY` or `DBUS_SESSION_BUS_ADDRESS`).
    - Tests: crafted XI2 wire events, capture encoding, fake portal / idle services on private buses, and `tests/x11_xvfb_it.rs` against the harness Xvfb (`xdotool` input is counted as injected only).
  - **Remaining Phase 2 Backlog (OPEN)**:
    - Worker wiring of the sensors (blocks, consent-gated start/stop, capture moment).
    - Separate opt-in V4L2 webcam (deferred to v1.1).
- **Phase 3 (Worker, Presence, Session & Configuration - OPEN)**:
  - Interval capture worker with ABA generation tickets.
  - Decoupled background upload loop preventing UI or capture blocking.
  - `logind` lock/unlock and idle detection matching presence status transitions.
  - Single-instance enforcement via UNIX domain socket.
  - XDG `config.toml` configuration with environment variable overrides (`DOSI__API_BASE_URL`).
  - Rootless systemd user service (`dosi-tracker.service`).
- **Phase 4 (CLI Surface Expansion & Packaging - OPEN)**:
  - Noninteractive CLI commands (`consent`, `queue`, `config`).
  - Native packaging recipes (`.deb` via `cargo-deb`, `.rpm` via `cargo-generate-rpm`, AppImage).
  - Multi-platform packaging verification.

