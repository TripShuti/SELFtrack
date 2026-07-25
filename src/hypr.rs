use std::env;
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
    let runtime = env::var("XDG_RUNTIME_DIR").map_err(|_| "XDG_RUNTIME_DIR not set")?;
    let instance = env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .map_err(|_| "HYPRLAND_INSTANCE_SIGNATURE not set")?;
    Ok(PathBuf::from(runtime).join(format!("hypr/{instance}/.socket2.sock")))
}

pub fn get_active_window() -> Result<ActiveWindow, String> {
    let output = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .map_err(|e| format!("failed to run hyprctl: {e}"))?;

    if !output.status.success() {
        return Err("hyprctl exited with non-zero status".into());
    }

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
        let path = match socket_path() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("{e}");
                return;
            }
        };

        loop {
            let stream = match UnixStream::connect(&path).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("hypr socket connect failed: {e}, retrying in 5s");
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
            Some(HyprEvent::ActiveWindow {
                class: cls.to_string(),
                title: title.to_string(),
            })
        }
        _ => None,
    }
}
