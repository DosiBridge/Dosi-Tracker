# Dosi-Tracker Linux Desktop Agent (`dosi-tracker-linux`)

Lightweight, privacy-first, rootless desktop agent foundation for Linux written in Rust.

## 1. Current Status

| Attribute | State | Details |
|---|---|---|
| **Crate** | `dosi-tracker-linux` v0.1.0 | Located in [`clients/linux`](./) |
| **Release Tier** | `headless-foundation` | Diagnostic foundation, default-deny consent core, durable outbox, rootless installer |
| **Telemetry Adapters** | `NOT_IMPLEMENTED` | Truthfully diagnosed via `doctor [--json]`; no synthetic or fake telemetry |
| **Privilege Model** | Strictly Rootless | Runs as normal user. Refuses `sudo`, `root`, `setuid`, `evdev`, and `uinput` |
| **Offline Storage** | Transactional SQLite | Durable outbox queue with raw identity bounding and read-only inspection |
| **CI Gate** | Dedicated GitHub Action | `linux-agent` job in `.github/workflows/ci.yml` running on `ubuntu-latest` |

---

## 2. Privacy & Security Invariants

1. **Strictly Rootless & Unprivileged**:
   The agent runs entirely in user space. Escalation to `root` or `sudo`, access to `/dev/input/event*` (evdev), and `/dev/uinput` are strictly prohibited. Any solution requiring elevated privileges is rejected.
2. **Anti-Keylogger Policy**:
   Only keyboard event **counts** and active time percentages are recorded. Keycodes, keystroke sequences, and text content are never captured or reconstructed.
3. **Default-Deny Consent Core**:
   All capabilities are disabled by default. Widening requires explicit user action and successful disk persistence (`ConsentStore`). Failures during persistence immediately revoke memory permissions to prevent unauthorized collection. ABA lifecycle tickets invalidate stale authorization across permission cycles.
4. **Truthful Diagnostics**:
   The diagnostic CLI (`doctor`) reports unimplemented adapters honestly as `NOT_IMPLEMENTED` with architectural rationales (e.g., `NO_DISPLAY_SERVER`, `PORTAL_REQUIRED`). Missing databases, configurations, or credentials are never created by diagnostic or inspection commands.

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

### Diagnostic Doctor (`doctor`)
Inspects session environment, resolved non-mutating XDG paths, capability support, adapter status, and privacy invariants:

```bash
# Human-readable diagnostic output
cargo run --bin dosi-tracker-linux -- doctor

# Structured JSON diagnostic output
cargo run --bin dosi-tracker-linux -- doctor --json
```

*Example JSON output snippet:*
```json
{
  "client": {
    "name": "dosi-tracker-linux",
    "version": "0.1.0",
    "release_tier": "headless-foundation"
  },
  "session": "x11",
  "capabilities": {
    "screen_capture": {
      "adapter_status": "NOT_IMPLEMENTED",
      "architectural_support": "SUPPORTED_X11"
    }
  },
  "privacy_invariants": {
    "unprivileged_process": true,
    "anti_keylogger_policy": true,
    "default_deny_consent": true
  }
}
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

| Capability | X11 Protocol | Wayland Compositor | Implementation Plan |
|---|---|---|---|
| **Screen Capture** | Supported via XShm / `xcap` | Supported via PipeWire / `xdg-desktop-portal` | User-consented screenshot captures; portal dialog ensures informed consent on Wayland |
| **Keyboard Counting** | Supported via XInput2 raw ticks | **UNSUPPORTED** (reported as `null`) | Event counters only; Wayland security isolation strictly respected without `evdev`/root bypass |
| **Mouse Counting** | Supported via XInput2 raw ticks | **UNSUPPORTED** (reported as `null`) | Click and movement activity counters; Wayland security isolation strictly respected |
| **Active Window** | Supported via `_NET_ACTIVE_WINDOW` | **UNSUPPORTED** (reported as `null`) | Window title and application WM_CLASS sanitized under active privacy policy |
| **Running Applications** | Supported via `/proc` enumeration | Supported via `/proc` enumeration | Scans user-owned process list for matching task programs |
| **Webcam Photo** | Supported via V4L2 (`/dev/video*`) | Supported via V4L2 / PipeWire camera portal | Discrete capture interval; opt-in camera permission |
| **Credential Storage** | Freedesktop Secret Service (D-Bus) | Freedesktop Secret Service (D-Bus) | Secure keyring storage with owner-only `0600` encrypted file fallback |

---

## 6. Implementation Roadmap

- **Phase 0 (Completed Foundation & Hygiene)**:
  - Scoped GitHub Actions CI workflow (`linux-agent` in `.github/workflows/ci.yml`).
  - Strict formatting and clippy gates (`cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`).
  - Safe scoped SQLite progress handler wrapper (`ScopedProgressHandler`) with guaranteed RAII deregistration.
  - Graceful recoverable mutex poisoning handling in outbox (`OutboxError::LockPoisoned`).
  - Documentation and test count synchronization across repository.
- **Phase 1 (Identity, Real Transport, Signed Queue)**:
  - Freedesktop Secret Service credential storage per account and workspace.
  - OpenIddict OAuth2 password grant authentication (`POST /connect/token`, `client_id=Tracker_App`, `scope=Tracker`, `__tenant` workspace header).
  - HMAC-SHA256 request signing matching backend middleware expectations.
  - Real `DeliveryTransport` implementation with chronological oldest-first batching to `POST /api/app/activity` with `client_activity_id` idempotency.
  - Background heartbeat daemon (`POST /api/app/presence/heartbeat`) on a 60s cadence.
- **Phase 2 (Native Consent-Gated Capture Adapters)**:
  - X11 active window detection (`_NET_ACTIVE_WINDOW`) and XInput2 activity counters.
  - Wayland portal ScreenCast via PipeWire (`ashpd`) with explicit user-prompted capture sessions.
  - V4L2 webcam capture adapter.
  - User-space `/proc` process inventory matching `AgentContextNormalizer.NormalizePlatform`.
- **Phase 3 (Worker, Presence, Session & Configuration)**:
  - Interval capture worker with ABA generation tickets.
  - Decoupled background upload loop preventing UI or capture blocking.
  - `logind` lock/unlock and idle detection matching presence status transitions.
  - Single-instance enforcement via UNIX domain socket.
  - XDG `config.toml` configuration with environment variable overrides (`DOSI__API_BASE_URL`).
  - Rootless systemd user service (`dosi-tracker.service`).
- **Phase 4 (CLI Surface Expansion & Packaging)**:
  - Noninteractive CLI commands (`consent`, `queue`, `config`, `logout`).
  - Native packaging recipes (`.deb` via `cargo-deb`, `.rpm` via `cargo-generate-rpm`, AppImage).
  - Multi-platform packaging verification.
