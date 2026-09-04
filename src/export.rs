use crate::db::Database;
use chrono::{Datelike, NaiveDate};
use serde::Serialize;

#[derive(Serialize)]
struct SummaryJson {
    active_ms: i64,
    idle_ms: i64,
    pc_on_ms: i64,
}

#[derive(Serialize)]
struct RangeJson {
    from: String,
    to: String,
    #[serde(flatten)]
    summary: SummaryJson,
}

#[derive(Serialize)]
struct AppJson {
    app: String,
    ms: i64,
    pct: f64,
}

#[derive(Serialize)]
struct SessionJson {
    app: String,
    title: String,
    start_ms: i64,
    end_ms: i64,
    idle: bool,
}

#[derive(Serialize)]
struct ExportJson {
    date: String,
    day: SummaryJson,
    week: RangeJson,
    month: RangeJson,
    apps: Vec<AppJson>,
    sessions: Vec<SessionJson>,
    pages: Vec<AppJson>,
    page_app: Option<String>,
}

fn to_summary(s: &crate::db::DaySummary) -> SummaryJson {
    SummaryJson {
        active_ms: s.active_ms,
        idle_ms: s.idle_ms,
        pc_on_ms: s.pc_on_ms,
    }
}

fn last_day_of(year: i32, month: u32) -> u32 {
    NaiveDate::from_ymd_opt(
        if month == 12 { year + 1 } else { year },
        if month == 12 { 1 } else { month + 1 },
        1,
    )
    .unwrap()
    .pred_opt()
    .unwrap()
    .day()
}

fn parse_date(date: &str) -> NaiveDate {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().naive_local().date())
}

pub fn run(db: &Database, date: &str, app: Option<&str>) {
    let d = parse_date(date);
    let ds = d.format("%Y-%m-%d").to_string();

    let day = db
        .get_summary_for_range(&ds, &ds)
        .map(|s| to_summary(&s))
        .unwrap_or(SummaryJson {
            active_ms: 0,
            idle_ms: 0,
            pc_on_ms: 0,
        });

    let weekday = d.weekday().num_days_from_monday();
    let monday = d.checked_sub_days(chrono::Days::new(weekday as u64)).unwrap();
    let sunday = monday.checked_add_days(chrono::Days::new(6)).unwrap();
    let wf = monday.format("%Y-%m-%d").to_string();
    let wt = sunday.format("%Y-%m-%d").to_string();
    let week_summary = db
        .get_summary_for_range(&wf, &wt)
        .map(|s| to_summary(&s))
        .unwrap_or(SummaryJson {
            active_ms: 0,
            idle_ms: 0,
            pc_on_ms: 0,
        });

    let mf = d.with_day(1).unwrap();
    let ml = mf.with_day(last_day_of(d.year(), d.month())).unwrap();
    let mfs = mf.format("%Y-%m-%d").to_string();
    let mls = ml.format("%Y-%m-%d").to_string();
    let month_summary = db
        .get_summary_for_range(&mfs, &mls)
        .map(|s| to_summary(&s))
        .unwrap_or(SummaryJson {
            active_ms: 0,
            idle_ms: 0,
            pc_on_ms: 0,
        });

    let app_rows = db.get_app_summary_for_range(&ds, &ds).unwrap_or_default();
    let active = day.active_ms.max(0) as f64;
    let apps: Vec<AppJson> = app_rows
        .iter()
        .map(|a| AppJson {
            app: a.app_class.clone(),
            ms: a.total_ms,
            pct: if active > 0.0 {
                (a.total_ms as f64 / active) * 100.0
            } else {
                0.0
            },
        })
        .collect();

    let sessions: Vec<SessionJson> = db
        .get_sessions_for_range(&ds, &ds)
        .unwrap_or_default()
        .iter()
        .map(|s| SessionJson {
            app: s.app_class.clone(),
            title: s.app_title.clone(),
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            idle: s.is_idle,
        })
        .collect();

    let mut pages: Vec<AppJson> = vec![];
    if let Some(app_class) = app {
        let rows = db
            .get_page_summary_for_app(&ds, &ds, app_class)
            .unwrap_or_default();
        let total: f64 = rows.iter().map(|r| r.total_ms).sum::<i64>() as f64;
        pages = rows
            .iter()
            .map(|p| AppJson {
                app: p.app_class.clone(),
                ms: p.total_ms,
                pct: if total > 0.0 {
                    (p.total_ms as f64 / total) * 100.0
                } else {
                    0.0
                },
            })
            .collect();
    }

    let out = ExportJson {
        date: ds,
        day,
        week: RangeJson {
            from: wf,
            to: wt,
            summary: week_summary,
        },
        month: RangeJson {
            from: mfs,
            to: mls,
            summary: month_summary,
        },
        apps,
        sessions,
        pages,
        page_app: app.map(|s| s.to_string()),
    };

    match serde_json::to_string(&out) {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("failed to serialize export: {e}");
            std::process::exit(1);
        }
    }
}
