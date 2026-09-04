use std::collections::HashMap;

pub async fn is_audio_playing() -> bool {
    !playing_streams().await.is_empty()
}

pub async fn playing_streams() -> Vec<String> {
    let output = tokio::process::Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .await;
    let Ok(o) = output else { return vec![] };
    if !o.status.success() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&o.stdout);
    audible_streams(&text)
}

// Шукає стріми що реально звучать. Стара перевірка ("short непорожній")
// давала false positive на:
// - віртуальні лінки обробки (наш SELFshell EQ висить завжди поки EQ увімкнено
//   і глушив idle-детект на 100% випадків);
// - поставлені на паузу плеєри (corked-стрім лишається в списку);
// - замучені стріми (нічого не чутно).
fn audible_streams(text: &str) -> Vec<String> {
    let mut names = vec![];
    let mut props: HashMap<&str, &str> = HashMap::new();
    let mut corked = "";
    let mut mute = "";

    let mut flush = |props: &HashMap<&str, &str>, corked: &str, mute: &str, out: &mut Vec<String>| {
        if corked.trim() != "no" || mute.trim() != "no" {
            return;
        }
        if props.get("node.virtual").is_some_and(|v| v.trim() == "\"true\"") {
            return;
        }
        let name = props
            .get("application.name")
            .or_else(|| props.get("media.name"))
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|| "unknown".into());
        out.push(name);
    };

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Sink Input #") {
            flush(&props, corked, mute, &mut names);
            props.clear();
            corked = "";
            mute = "";
        } else if let Some(v) = t.strip_prefix("Corked:") {
            corked = v;
        } else if let Some(v) = t.strip_prefix("Mute:") {
            mute = v;
        } else if let Some((k, v)) = t.split_once('=') {
            props.insert(k.trim(), v.trim());
        }
    }
    flush(&props, corked, mute, &mut names);
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const EQ_ONLY: &str = r#"Sink Input #37
	Driver: PipeWire
	Sink: 63
	Corked: no
	Mute: no
	Properties:
		node.virtual = "true"
		media.name = "SELFshell EQ"
		object.serial = "37"
"#;

    const PAUSED_PLAYER: &str = r#"Sink Input #80
	Driver: PipeWire
	Sink: 63
	Corked: yes
	Mute: no
	Properties:
		application.name = "Chromium"
		object.serial = "80"
"#;

    const MUTED_PLAYER: &str = r#"Sink Input #81
	Driver: PipeWire
	Sink: 63
	Corked: no
	Mute: yes
	Properties:
		application.name = "mpv"
		object.serial = "81"
"#;

    const PLAYING_PLAYER: &str = r#"Sink Input #76
	Driver: PipeWire
	Sink: 36
	Corked: no
	Mute: no
	Properties:
		application.name = "PipeWire ALSA [selfsonic]"
		object.serial = "76"
"#;

    #[test]
    fn eq_loopback_alone_is_not_playback() {
        assert!(audible_streams(EQ_ONLY).is_empty());
    }

    #[test]
    fn paused_player_is_not_playback() {
        assert!(audible_streams(PAUSED_PLAYER).is_empty());
    }

    #[test]
    fn muted_player_is_not_playback() {
        assert!(audible_streams(MUTED_PLAYER).is_empty());
    }

    #[test]
    fn running_player_is_playback() {
        assert_eq!(
            audible_streams(PLAYING_PLAYER),
            vec!["PipeWire ALSA [selfsonic]".to_string()]
        );
    }

    #[test]
    fn mixed_list_reports_only_audible() {
        let mixed = [EQ_ONLY, PAUSED_PLAYER, PLAYING_PLAYER].join("\n");
        assert_eq!(
            audible_streams(&mixed),
            vec!["PipeWire ALSA [selfsonic]".to_string()]
        );
    }

    #[test]
    fn empty_list_is_silence() {
        assert!(audible_streams("").is_empty());
    }
}
