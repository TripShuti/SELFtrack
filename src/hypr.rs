use std::path::PathBuf;
use std::process::Command;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

pub struct ActiveWindow {
    pub class: String,
    pub title: String,
}

fn socket_path() -> Result<PathBuf, String> {
    // Швидкий шлях — env. Повільний — скан інстансів.
    // Резолвиться ВСЕРЕДИНІ ретрай-лупа (див. нижче), бо сигнатура
    // змінюється при рестарті Hyprland, а env процесу заморожений.
    // env-шлях повертаємо тільки якщо він живий, інакше скануємо —
    // інакше закешований мертвий шлях ретраївся б вічно.
    if let (Ok(runtime), Ok(instance)) = (
        std::env::var("XDG_RUNTIME_DIR"),
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE"),
    ) {
        if !runtime.is_empty() && !instance.is_empty() {
            let p = PathBuf::from(runtime).join(format!("hypr/{instance}/.socket2.sock"));
            if std::os::unix::net::UnixStream::connect(&p).is_ok() {
                return Ok(p);
            }
            // env-шлях мертвий — падаємо на скан нижче, а не повертаємо його.
        }
    }
    crate::session::find_live_hypr_socket()
        .ok_or_else(|| "hypr socket not found (env missing, no live instance scanned)".to_string())
}

fn hyprctl_cmd() -> Command {
    let mut cmd = Command::new("hyprctl");
    // hyprctl теж читає env — підкладаємо знайдені значення,
    // якщо в процесі їх нема (ранній старт systemd-юніта).
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
        if let Some((sig, rt)) = crate::session::hypr_env() {
            cmd.env("HYPRLAND_INSTANCE_SIGNATURE", sig);
            cmd.env("XDG_RUNTIME_DIR", rt);
        }
    }
    cmd
}

fn hyprctl_output() -> Result<std::process::Output, String> {
    let output = hyprctl_cmd()
        .args(["activewindow", "-j"])
        .output()
        .map_err(|e| format!("failed to run hyprctl: {e}"))?;
    if output.status.success() {
        return Ok(output);
    }
    // Ретраї з відкритим сокетом, але мертвим hyprctl: env-сигнатура
    // застаріла (рестарт Hyprland), а в процесі лежить старе значення.
    // Пробуємо ще раз з висканеною живою сигнатурою.
    if let Some((sig, rt)) = crate::session::hypr_env() {
        let stale = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok();
        if stale.as_deref() != Some(sig.as_str()) {
            let retry = Command::new("hyprctl")
                .env("HYPRLAND_INSTANCE_SIGNATURE", &sig)
                .env("XDG_RUNTIME_DIR", &rt)
                .args(["activewindow", "-j"])
                .output()
                .map_err(|e| format!("failed to run hyprctl: {e}"))?;
            if retry.status.success() {
                return Ok(retry);
            }
        }
    }
    Err("hyprctl exited with non-zero status".into())
}

pub fn get_active_window() -> Result<ActiveWindow, String> {
    let output = hyprctl_output()?;

    let raw = String::from_utf8_lossy(&output.stdout);

    #[derive(serde::Deserialize)]
    struct HyprctlWindow {
        class: Option<String>,
        title: Option<String>,
    }

    let parsed: HyprctlWindow =
        serde_json::from_str(&raw).map_err(|e| format!("json parse error: {e}"))?;

    Ok(ActiveWindow {
        class: parsed.class.unwrap_or_else(|| "unknown".into()),
        title: parsed.title.unwrap_or_default(),
    })
}

#[derive(Debug, Clone)]
pub enum HyprEvent {
    ActiveWindow { class: String, title: String },
    Other(String),
}

pub fn spawn_event_listener(tx: mpsc::Sender<HyprEvent>) {
    tokio::spawn(async move {
        // Важливо: шлях резолвиться на КОЖНІЙ ітерації, а не один раз
        // до циклу. Інакше ранній старт (env ще нема) вбиває слухача
        // назавжди, а рестарт Hyprland з новою сигнатурою робить
        // закешований шлях мертвим.
        let mut failures: u64 = 0;
        loop {
            if tx.is_closed() {
                return;
            }
            let path = match socket_path() {
                Ok(p) => {
                    failures = 0;
                    p
                }
                Err(e) => {
                    failures += 1;
                    let sleep_s = match failures {
                        1..=3 => 5,
                        4..=6 => 15,
                        _ => 30,
                    };
                    if failures <= 3 || failures % 6 == 1 {
                        tracing::warn!(
                            "hypr socket unavailable: {e} (attempt {failures}), retrying in {sleep_s}s"
                        );
                    } else {
                        tracing::debug!(
                            "hypr socket unavailable: {e} (attempt {failures}), retrying in {sleep_s}s"
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(sleep_s)).await;
                    continue;
                }
            };

            let stream = match UnixStream::connect(&path).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("hypr socket connect failed: {e}, retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            tracing::info!("connected to hyprland socket");

            let reader = BufReader::new(stream);
            let mut lines = reader.lines();

            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if let Some(ev) = parse_event(&line) {
                            if tx.send(ev).await.is_err() {
                                return;
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::warn!("hypr socket closed, reconnecting...");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("hypr socket read error: {e}, reconnecting...");
                        break;
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

fn parse_event(line: &str) -> Option<HyprEvent> {
    let (tag, rest) = line.split_once(">>")?;
    match tag {
        "activewindow" => {
            let (cls, title) = rest.split_once(',').unwrap_or((rest, ""));
            let cls = if cls.is_empty() { "unknown" } else { cls };
            Some(HyprEvent::ActiveWindow {
                class: cls.to_string(),
                title: title.to_string(),
            })
        }
        _ => None,
    }
}
