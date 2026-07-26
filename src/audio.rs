pub const VIDEO_PLAYERS: &[&str] = &[
    "mpv",
    "vlc",
    "smplayer",
    "mplayer",
    "kodi",
    "jellyfinmediaplayer",
    "jellyfin media player",
];

pub const BROWSERS: &[&str] = &[
    "firefox",
    "firefox-esr",
    "chromium",
    "chromium-browser",
    "google-chrome",
    "chrome",
    "brave-browser",
    "brave",
    "vivaldi",
    "opera",
    "microsoft-edge",
    "msedge",
];

pub fn is_video_player(class: &str) -> bool {
    VIDEO_PLAYERS.contains(&class)
}

pub fn is_browser(class: &str) -> bool {
    BROWSERS.contains(&class)
}

pub fn is_media_player(class: &str) -> bool {
    is_video_player(class) || is_browser(class)
}

pub async fn is_audio_playing() -> bool {
    let output = tokio::process::Command::new("pactl")
        .args(["list", "sink-inputs", "short"])
        .output()
        .await;
    match output {
        Ok(o) if o.status.success() => !o.stdout.is_empty(),
        _ => false,
    }
}
