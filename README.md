# SELFTrack

Focus-based time tracker for Hyprland. Logs active application time per day using Hyprland IPC, detects idle via Wayland `ext-idle-notify-v1`.

## Features

- Tracks focused window (class + title) via Hyprland socket
- Idle detection with configurable threshold (default 5 min)
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

# TUI dashboard (calendar + day/week/month summary + app list)
selftrack tui

# CLI reports
selftrack today
selftrack report --date 2026-07-20
selftrack report --date 2026-07-25 --app kitty   # per-page breakdown
selftrack timeline                                 # session timeline
selftrack timeline --date 2026-07-20
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

No migration needed — schema is stable.

## Build Dependencies

- Rust 1.75+ (edition 2024)
- Wayland development libraries (`libwayland-client`)
