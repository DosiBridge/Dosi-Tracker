# Dosi-Tracker — Windows Client (Rust)

A lightweight, background activity-tracking agent for Windows, written in Rust.

## Why Rust

- Tiny idle footprint (no runtime/GC) — a few MB of RAM.
- Event-driven input listening → ~0% CPU while idle.
- Small, self-contained binary (release profile is size/LTO optimized).

## What it does

Once per interval (5–60 min) it captures a snapshot for the active project:

- Screenshot (primary monitor)
- Webcam frame (optional)
- Active window (app + title)
- Running programs
- Keyboard / mouse **counts** (never keystroke content)

Snapshots are stored locally first (SQLite, offline-first) and then synced to
the backend REST API. Unsynced snapshots survive restarts and network outages.

## Project layout

```
src/
├── main.rs              # agent loop (interval capture + sync + Ctrl-C shutdown)
├── config.rs            # config.toml + DOSI_* env vars
├── model.rs             # shared data types (Project, Activity, ...)
├── api.rs               # backend REST client (login, projects, submit)
├── storage.rs           # local SQLite queue (offline-first)
└── tracking/
    ├── mod.rs           # Tracker: builds one Activity snapshot
    ├── input.rs         # event-driven keyboard/mouse counters
    ├── screenshot.rs    # primary-monitor PNG capture
    ├── webcam.rs        # single webcam JPEG frame
    └── active_window.rs # foreground window (Win32)
```

## Prerequisites

Rust is **not** installed on this machine yet. Install it via:

```powershell
winget install Rustlang.Rustup
# or download from https://rustup.rs
```

## Build & run

```powershell
cd clients/windows
copy config.example.toml config.toml   # then edit values

# credentials via env for now (a real UI would prompt)
$env:DOSI_USERNAME="you@example.com"
$env:DOSI_PASSWORD="..."

cargo run --release
```

## Notes

- This is a scaffold: capture modules are working skeletons meant as a
  starting point, not a finished product.
- `active_window::running_programs` currently reports the foreground window;
  extend with `EnumWindows` for a full process list.
- For true background operation, wrap the binary as a Windows Service
  (e.g. via `sc.exe create` or the `windows-service` crate) and add a small
  tray UI or let the shared frontend/desktop shell control it.
