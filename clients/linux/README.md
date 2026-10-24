# Dosi-Tracker Linux Desktop Agent (`dosi-tracker-linux`)

Lightweight, privacy-first, rootless desktop agent foundation for Linux written in Rust.

## 1. Current Status

| Attribute | State | Details |
|---|---|---|
| **Crate** | `dosi-tracker-linux` v0.1.0 | Located in [`clients/linux`](./) |
| **Release Tier** | `headless-foundation` | Diagnostic foundation, default-deny consent core, durable outbox, rootless installer |
| **Telemetry Adapters** | `NOT_IMPLEMENTED` | Truthfully diagnosed via `doctor [--json]`; no synthetic or fake telemetry (Phase 2) |
| **Privilege Model** | Strictly Rootless | Runs as normal user. Refuses `sudo`, `root`, `setuid`, `evdev`, and `uinput` |
| **Offline Storage** | Transactional SQLite | Durable outbox queue with raw identity bounding and read-only inspection |
| **Credential Store** | SecretService + 0600 Fallback | Scoped `(account, workspace)` store; SecretService primary, owner-only 0600 file fallback with explicit doctor warning |
| **Transport & Auth** | Real HTTP Delivery Transport | Reqwest rustls, OpenIddict password grant (`POST /connect/token`), pre-expiry refresh, bounded 401 retry, activity upload, heartbeat |
| **Payload Signing** | HMAC-SHA256 (32 bytes) | URL_SAFE_NO_PAD base64url tag over canonical JSON, mirroring Windows byte semantics and test vectors |
| **CI Gate** | Dedicated GitHub Action | `linux-agent` job in `.github/workflows/ci.yml` running on `ubuntu-latest` |

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
- Rust toolchain (1.80+ stable)
- SQLite 3 runtime libraries
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
~/.local/opt/dosi-tracker/bin/dosi-tracker-linux doctor
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
cargo run --bin dosi-tracker-linux -- login \
  --username "worker@example.com" \
  --password "secret" \
  --workspace-id "00000000-0000-0000-0000-000000000001" \
  --server "https://api.dosi.app"

# Logout and securely remove credentials
cargo run --bin dosi-tracker-linux -- logout \
  --account-id "worker@example.com" \
  --workspace-id "00000000-0000-0000-0000-000000000001"
```

### Status Inspection (`status`)
Non-mutating inspection of identity, token validity, signing key status, and optional queue depth:

```bash
# Human-readable status
cargo run --bin dosi-tracker-linux -- status \
  --account-id "worker@example.com" \
  --workspace-id "00000000-0000-0000-0000-000000000001" \
  --db ~/.local/share/dosi-tracker/outbox.db

# Structured JSON status (pure inspection, zero secret leaks)
cargo run --bin dosi-tracker-linux -- status \
  --account-id "worker@example.com" \
  --workspace-id "00000000-0000-0000-0000-000000000001" \
  --json
```

### Diagnostic Doctor (`doctor`)
Inspects session environment, resolved non-mutating XDG paths, capability support, adapter status, credential storage backend, and privacy invariants:

```bash
# Human-readable diagnostic output
cargo run --bin dosi-tracker-linux -- doctor

# Structured JSON diagnostic output
cargo run --bin dosi-tracker-linux -- doctor --json
```

### Outbox Status Inspection (`outbox status`)
Performs bounded, strictly read-only inspection of the offline SQLite outbox queue for an exact account and workspace scope. Opens the database with `SQLITE_OPEN_READ_ONLY`, enforces a finite execution work budget (`DEFAULT_MAX_SQLITE_WORK_STEPS = 25_000`), folds corruption checks into a bounded CTE, and fails closed without creating missing database files:

```bash
cargo run --bin dosi-tracker-linux -- outbox status \
  --db ~/.local/share/dosi-tracker/outbox.db \
  --account-id "00000000-0000-0000-0000-000000000001" \
  --workspace-id "00000000-0000-0000-0000-000000000002" \
  --json
```

### Help & Version
```bash
cargo run --bin dosi-tracker-linux -- --help
cargo run --bin dosi-tracker-linux -- --version
```

---

## 5. Platform Capability & Architecture Matrix

| Capability | X11 Protocol | Wayland Compositor | Implementation Status |
|---|---|---|---|
| **Screen Capture** | Supported via XShm / `xcap` | Supported via PipeWire / `xdg-desktop-portal` | Planned Phase 2 |
| **Keyboard Counting** | Supported via XInput2 raw ticks | **UNSUPPORTED** (reported as `null`) | Planned Phase 2 (X11 only; Wayland isolated) |
| **Mouse Counting** | Supported via XInput2 raw ticks | **UNSUPPORTED** (reported as `null`) | Planned Phase 2 (X11 only; Wayland isolated) |
| **Active Window** | Supported via `_NET_ACTIVE_WINDOW` | **UNSUPPORTED** (reported as `null`) | Planned Phase 2 (X11 only; Wayland isolated) |
| **Running Applications** | Supported via `/proc` enumeration | Supported via `/proc` enumeration | Planned Phase 2 |
| **Webcam Photo** | Supported via V4L2 (`/dev/video*`) | Supported via V4L2 / PipeWire camera portal | Planned Phase 2 |
| **Credential Storage** | Freedesktop Secret Service (D-Bus) | Freedesktop Secret Service (D-Bus) | **Delivered in Phase 1** (owner-only 0600 file fallback with warning) |
| **Backend Sync & Transport** | Real HTTP Delivery Transport | Real HTTP Delivery Transport | **Delivered in Phase 1** (Library transport `HttpDeliveryTransport::from_store` / `from_credentials` reusing persisted login credentials; background daemon loop is Phase 3, CLI queue flush command is Phase 4) |
| **Payload Signing** | HMAC-SHA256 (32 bytes) | HMAC-SHA256 (32 bytes) | **Delivered in Phase 1** (Windows byte semantics parity) |

---

## 6. Implementation Roadmap

- **Phase 0 (Completed Foundation & Hygiene)**:
  - Scoped GitHub Actions CI workflow (`linux-agent` in `.github/workflows/ci.yml`).
  - Strict formatting and clippy gates (`cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`).
  - Safe scoped SQLite progress handler wrapper (`ScopedProgressHandler`) with guaranteed RAII deregistration.
  - Graceful recoverable mutex poisoning handling in outbox (`OutboxError::LockPoisoned`).
  - Documentation and test count synchronization across repository.
- **Phase 1 (Delivered Foundation: Identity, Real Transport, Signed Queue)**:
  - Freedesktop Secret Service credential storage per account and workspace with owner-only `0600` private file fallback.
  - OpenIddict OAuth2 password grant authentication (`POST /connect/token`, `client_id=Tracker_App`, `scope=Tracker`, `__tenant` workspace parameter).
  - Pre-expiry token refresh buffer (60s early renewal) and bounded 401 retry / re-authentication.
  - Per-device 32-byte HMAC-SHA256 payload signing matching Windows byte semantics (`URL_SAFE_NO_PAD` base64url tag).
  - Concrete `HttpDeliveryTransport` implementing `DeliveryTransport` contract with exact ack deletion, transient failure retention and retry counter incrementation, auth-required halt, and mismatched ack halt.
  - Production transport initialization from persisted credentials (`HttpDeliveryTransport::from_store` / `from_credentials`), proving production upload reuses credentials, tokens, and signing key saved by CLI `login`.
  - Heartbeat presence primitive (`POST /api/app/presence/heartbeat`) on 60s cadence.
  - Non-mutating CLI commands: `login`, `logout`, `status [--json]`.
  - *Evidence & Boundaries:* Verified via persisted-session library integration (`SyncPassCoordinator` + `HttpDeliveryTransport::from_store`) using credentials saved by CLI `login`, with deterministic mid-upload HTTP 502 response and TCP socket interruption tests. Automated background daemon loop belongs to Phase 3; CLI queue flush command belongs to Phase 4. Independent native QA remains pending.
- **Phase 2 (Native Consent-Gated Capture Adapters - OPEN)**:
  - X11 active window detection (`_NET_ACTIVE_WINDOW`) and XInput2 activity counters.
  - Wayland portal ScreenCast via PipeWire (`ashpd`) with explicit user-prompted capture sessions.
  - V4L2 webcam capture adapter.
  - User-space `/proc` process inventory matching `AgentContextNormalizer.NormalizePlatform`.
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

