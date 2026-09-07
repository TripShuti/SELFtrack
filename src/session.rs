//! Discovery of Wayland / Hyprland sockets with fallback scanning.
//!
//! Проблема: демон стартував через `WantedBy=default.target` раніше за
//! композитор, коли в `env` процесу ще не було `WAYLAND_DISPLAY` /
//! `HYPRLAND_INSTANCE_SIGNATURE`. `env` процесу заморожений на момент
//! `exec`, тому чистий ретрай `connect_to_env()` в тому ж процесі
//! ніколи не виліковується без рестарту. Плюс сигнатура Hyprland
//! змінюється при кожному рестарті композитора.
//!
//! Тому: спочатку пробуємо `env`, потім скануємо `/run/user/$UID/`
//! і повертаємо перший живий сокет. Резолв викликається всередині
//! циклів ретраю, а не один раз на старті.

use std::path::PathBuf;
use std::time::SystemTime;

pub fn runtime_dir() -> PathBuf {
    if let Ok(r) = std::env::var("XDG_RUNTIME_DIR") {
        if !r.is_empty() {
            return PathBuf::from(r);
        }
    }
    if let Some(p) = dirs::runtime_dir() {
        return p;
    }
    // Останній шанс: найсвіжіший /run/user/<uid>
    if let Ok(entries) = std::fs::read_dir("/run/user") {
        let mut best: Option<(SystemTime, PathBuf)> = None;
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let replace = match &best {
                Some((t, _)) => mtime > *t,
                None => true,
            };
            if replace {
                best = Some((mtime, p));
            }
        }
        if let Some((_, p)) = best {
            return p;
        }
    }
    PathBuf::from("/run/user/1000")
}

fn is_live_socket(p: &PathBuf) -> bool {
    std::os::unix::net::UnixStream::connect(p).is_ok()
}

/// Кандидати Wayland-сокета: спочатку env, потім скан рантайм-діра.
pub fn wayland_socket_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    if let Ok(display) = std::env::var("WAYLAND_DISPLAY") {
        if !display.is_empty() {
            let p = PathBuf::from(&display);
            if p.is_absolute() {
                out.push(p);
            } else {
                out.push(runtime_dir().join(display));
            }
        }
    }

    let rt = runtime_dir();
    // Явні типові імена — дешево і покриває 99% випадків.
    for name in ["wayland-1", "wayland-0"] {
        let p = rt.join(name);
        if !out.contains(&p) {
            out.push(p);
        }
    }

    // Повний скан: wayland-* без *.lock і службових *-*.sock
    // (напр. wayland-1-awww-daemon.sock — не композитор).
    if let Ok(entries) = std::fs::read_dir(&rt) {
        let mut scanned: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                (name.starts_with("wayland-") || name == "wayland-0")
                    && !name.ends_with(".lock")
                    && !name.contains("-awww-")
                    && !out.contains(p)
            })
            .collect();
        scanned.sort();
        out.extend(scanned);
    }

    out
}

/// Кандидати Hyprland socket2: спочатку env, потім скан інстансів.
pub fn hypr_socket_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let rt = runtime_dir();

    if let (Ok(sig), Ok(r)) = (
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE"),
        std::env::var("XDG_RUNTIME_DIR"),
    ) {
        if !sig.is_empty() && !r.is_empty() {
            out.push(PathBuf::from(r).join(format!("hypr/{sig}/.socket2.sock")));
        }
    } else if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        if !sig.is_empty() {
            out.push(rt.join(format!("hypr/{sig}/.socket2.sock")));
        }
    }

    // Скан усіх інстансів, найсвіжіший першим (рестарт Hyprland
    // змінює сигнатуру, старий шлях назавжди мертвий).
    let hypr_dir = rt.join("hypr");
    if let Ok(entries) = std::fs::read_dir(&hypr_dir) {
        let mut scanned: Vec<(SystemTime, PathBuf)> = entries
            .flatten()
            .map(|e| e.path())
            .map(|d| d.join(".socket2.sock"))
            .filter(|p| !out.contains(p))
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                (mtime, p)
            })
            .collect();
        scanned.sort_by(|a, b| b.0.cmp(&a.0));
        out.extend(scanned.into_iter().map(|(_, p)| p));
    }

    out
}

/// Перший живий Hyprland socket2.
pub fn find_live_hypr_socket() -> Option<PathBuf> {
    hypr_socket_candidates()
        .into_iter()
        .find(|p| is_live_socket(p))
}

/// Сигнатура + рантайм для `hyprctl` (він теж читає env).
/// Повертає `(signature, runtime)` якщо вдалось визначити.
/// env-значення довіряємо тільки якщо сокет живий, інакше скануємо
/// (сигнатура протухає при рестарті Hyprland).
pub fn hypr_env() -> Option<(String, PathBuf)> {
    if let (Ok(sig), Ok(r)) = (
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE"),
        std::env::var("XDG_RUNTIME_DIR"),
    ) {
        if !sig.is_empty() && !r.is_empty() {
            let p = PathBuf::from(&r).join(format!("hypr/{sig}/.socket2.sock"));
            if is_live_socket(&p) {
                return Some((sig, PathBuf::from(r)));
            }
        }
    }
    let rt = runtime_dir();
    let hypr_dir = rt.join("hypr");
    let entries = std::fs::read_dir(&hypr_dir).ok()?;
    let mut best: Option<(SystemTime, String)> = None;
    for e in entries.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        if !dir.join(".socket2.sock").exists() {
            continue;
        }
        let sig = e.file_name().to_string_lossy().into_owned();
        let mtime = std::fs::metadata(dir.join(".socket2.sock"))
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let replace = match &best {
            Some((t, _)) => mtime > *t,
            None => true,
        };
        if replace {
            best = Some((mtime, sig));
        }
    }
    best.map(|(_, sig)| (sig, rt))
}
