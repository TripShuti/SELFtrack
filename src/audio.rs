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
