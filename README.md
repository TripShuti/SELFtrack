# SELFTrack

Focus-based time tracker for Hyprland. Logs active application time per day using Hyprland IPC, detects idle via Wayland `ext-idle-notify-v1`.

## Features

- Tracks focused window (class + title) via Hyprland socket
- Idle detection with configurable threshold (default 5 min)
- Stores sessions in SQLite (`~/.local/share/selftrack/track.db`)
- CLI report: `selftrack today`, `selftrack report --date 2026-07-25`
- TUI with Day / Week / Month views

## Installation

```bash
git clone https://github.com/YOUR_USER/SELFtrack
cd SELFtrack
cargo build --release
cargo install --path .
```

Or directly:
```bash
cargo install --git https://github.com/YOUR_USER/SELFtrack
```

## Usage

```bash
# Start the daemon (background tracker)
selftrack daemon

# CLI reports
selftrack today
selftrack report --date 2026-07-20

# TUI dashboard
selftrack tui
```

### TUI Controls

| Key | Action |
|---|---|
| `←` / `→` | Previous / next period |
| `Tab` | Cycle Day → Week → Month |
| `1` / `2` / `3` | Switch to Day / Week / Month |
| `q` / `Esc` | Quit |

## Autostart with systemd

```bash
cp scripts/selftrack-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now selftrack-daemon
```

Edit the service file to update the binary path if you installed via `cargo install` (binary at `~/.cargo/bin/selftrack`).

## Data

All data is stored in `~/.local/share/selftrack/track.db`. Each session records:
- date, app class, app title
- start and end timestamps (ms)
- idle flag (1 if idle session)

## Build Dependencies

- Rust 1.75+ (edition 2024)
- Wayland development libraries (`libwayland-client`)
