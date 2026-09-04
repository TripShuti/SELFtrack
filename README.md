# SELFTrack

Focus-based time tracker for Hyprland. Logs active application time per day using Hyprland IPC, detects idle via Wayland `ext-idle-notify-v1`.

## Features

- Tracks focused window (class + title) via Hyprland socket
- Idle detection with configurable threshold (default 5 min); idle is
  suppressed only while audio is actually playing (running, unmuted,
  non-virtual streams — paused players and the EQ loopback don't count)
- Per-page breakdown for browsers (tracks tab titles)
- Stores sessions in SQLite (`~/.local/share/selftrack/track.db`)
- TUI: calendar view with day/week/month totals + expandable app pages
- CLI: report, per-app page breakdown, timeline

## Installation

```bash
git clone https://github.com/TripShuti/SELFtrack
cd SELFtrack
cargo build --release
cargo install --path .
```

Or directly:
```bash
cargo install --git https://github.com/TripShuti/SELFtrack
```

## Usage

```bash
# Start the daemon (background tracker)
selftrack daemon
selftrack daemon --retention-days 60  # auto-prune sessions older than N days (0 = keep all)

# TUI dashboard (calendar + day/week/month summary + app list)
selftrack tui

# Machine-readable JSON for widgets/integrations
selftrack export --date 2026-07-20
selftrack export --date 2026-07-20 --app kitty  # adds per-page breakdown

# CLI reports
selftrack today
selftrack report --date 2026-07-20
selftrack report --date 2026-07-25 --app kitty   # per-page breakdown
selftrack timeline                                 # session timeline
selftrack timeline --date 2026-07-20
```

### `export` JSON schema

```jsonc
{
  "date": "2026-07-20",
  "day":   { "active_ms": 123, "idle_ms": 45, "pc_on_ms": 168 },
  "week":  { "from": "2026-07-14", "to": "2026-07-20", "active_ms": 1, "idle_ms": 2, "pc_on_ms": 3 },
  "month": { "from": "2026-07-01", "to": "2026-07-31", "active_ms": 1, "idle_ms": 2, "pc_on_ms": 3 },
  "apps":     [{ "app": "kitty", "ms": 123, "pct": 95.2 }],
  "sessions": [{ "app": "kitty", "title": "~", "start_ms": 1, "end_ms": 2, "idle": false }],
  "pages":    [{ "app": "page title", "ms": 12, "pct": 50.0 }],  // only with --app
  "page_app": "kitty"  // only with --app, echoes the requested app
}
```

### TUI Controls

| Key | Action |
|---|---|
| `←` / `→` | Previous / next day |
| `↑` / `↓` | Previous / next week |
| `Enter` / `Tab` | Toggle detail mode |
| `↑` / `↓` (detail) | Navigate app list |
| `Enter` (detail) | Expand/collapse app pages |
| `Esc` | Back to calendar |
| `q` | Quit |

## Autostart with systemd

```bash
cp scripts/selftrack-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now selftrack-daemon
```

Edit `ExecStart` in the service file if the binary path differs (default: `~/.cargo/bin/selftrack`).

## Data

Stored in `~/.local/share/selftrack/track.db` (SQLite). Each session records:
- date, app class, app title (browser page titles included)
- start and end timestamps (ms)
- idle flag (1 if idle session)

The daemon splits the open session every 60 seconds, so the database
(and `export`) is never more than a minute stale. Sessions older than
`--retention-days` (default 60) are auto-pruned on daemon start and once
a day, followed by `VACUUM`.

No migration needed — schema is stable.

## Build Dependencies

- Rust 1.75+ (edition 2024)
- Wayland development libraries (`libwayland-client`)
