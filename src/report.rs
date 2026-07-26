use crate::db::Database;

pub fn print_report(db: &Database, date: &str, app: Option<&str>) {
    if let Some(app_class) = app {
        print_app_report(db, date, app_class);
        return;
    }

    match db.get_day_summary(date) {
        Ok(summary) => {
            println!("=== SELFTrack report for {date} ===");
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

fn print_app_report(db: &Database, date: &str, app_class: &str) {
    let pages = match db.get_page_summary_for_app(date, date, app_class) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error querying page summary: {e}");
            return;
        }
    };

    if pages.is_empty() {
        println!("No data for app \"{app_class}\" on {date}.");
        return;
    }

    let total: i64 = pages.iter().map(|p| p.total_ms).sum();
    println!("=== Page breakdown for \"{app_class}\" on {date} ===");
    println!("Total: {}", format_duration(total));
    println!();
    println!("{:<50} {:>10} {:>8}", "Page", "Time", "%");
    println!("{}", "-".repeat(72));
    for p in &pages {
        let pct = if total > 0 {
            (p.total_ms as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let title = clean_page_title(&p.app_class);
        println!(
            "{:<50} {:>10} {:>7.1}%",
            truncate(&title, 50),
            format_duration(p.total_ms),
            pct
        );
    }
}

pub fn print_timeline(db: &Database, date: &str) {
    let sessions = match db.get_sessions_for_range(date, date) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error querying sessions: {e}");
            return;
        }
    };

    if sessions.is_empty() {
        println!("No data for {date}.");
        return;
    }

    println!("=== SELFTrack timeline for {date} ===");
    println!("{:<19} {:<20} {}", "Time", "App", "Title");
    println!("{}", "-".repeat(80));
    for s in &sessions {
        let start = format_time(s.start_ms);
        let end = format_time(s.end_ms);
        let app = if s.is_idle { "__idle__" } else { &s.app_class };
        let title = if s.is_idle {
            String::new()
        } else {
            clean_page_title(&s.app_title)
        };
        println!("{start} — {end}   {:<20} {}", truncate(app, 20), truncate(&title, 40));
    }
}

fn format_duration(ms: i64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{secs:02}")
}

fn format_time(ms: i64) -> String {
    let secs = ms / 1000;
    let hours = (secs / 3600) % 24;
    let minutes = (secs % 3600) / 60;
    let secs = secs % 60;
    format!("{hours:02}:{minutes:02}:{secs:02}")
}

fn clean_page_title(title: &str) -> String {
    let suffixes = [
        " — Mozilla Firefox",
        " - Mozilla Firefox",
        " — Chromium",
        " - Chromium",
        " — Google Chrome",
        " - Google Chrome",
        " — Brave",
        " - Brave",
        " — Vivaldi",
        " - Vivaldi",
        " — Opera",
        " - Opera",
    ];
    let t = title.trim();
    for s in &suffixes {
        if let Some(base) = t.strip_suffix(s) {
            return base.trim().to_string();
        }
    }
    t.to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub(1);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
