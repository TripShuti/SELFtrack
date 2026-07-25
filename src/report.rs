use crate::db::Database;

pub fn print_report(db: &Database, date: &str) {
    match db.get_day_summary(date) {
        Ok(summary) => {
            println!("=== SELFTrack report for {date} ===");
            println!("PC on:     {}", format_duration(summary.pc_on_ms));
            println!("Active:    {}", format_duration(summary.active_ms));
            println!("Idle:      {}", format_duration(summary.idle_ms));
            println!();

            match db.get_app_summary_for_date(date) {
                Ok(apps) => {
                    if apps.is_empty() {
                        println!("No tracking data for this day.");
                        return;
                    }
                    println!("{:<20} {:>12} {:>14}", "App", "Time", "%");
                    println!("{}", "-".repeat(48));
                    for app in &apps {
                        let pct = if summary.active_ms > 0 {
                            (app.total_ms as f64 / summary.active_ms as f64) * 100.0
                        } else {
                            0.0
                        };
                        println!(
                            "{:<20} {:>12} {:>13.1}%",
                            truncate(&app.app_class, 20),
                            format_duration(app.total_ms),
                            pct
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Error querying app summary: {e}");
                }
            }
        }
        Err(e) => {
            eprintln!("Error querying day summary: {e}");
        }
    }
}

fn format_duration(ms: i64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{secs:02}")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max - 1])
    } else {
        s.to_string()
    }
}
