use crate::db;
use chrono::{Datelike, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Cell, Paragraph, Row, Table},
    Frame,
};
use std::io;

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Day,
    Week,
    Month,
}

struct App {
    mode: ViewMode,
    date: NaiveDate,
    summary: Option<db::DaySummary>,
    app_summaries: Vec<db::AppSummary>,
    db: db::Database,
}

impl App {
    fn new(db: db::Database) -> Self {
        let date = chrono::Local::now().naive_local().date();
        let mut app = Self {
            mode: ViewMode::Day,
            date,
            summary: None,
            app_summaries: vec![],
            db,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let (from, to) = self.date_range();
        self.summary = self.db.get_summary_for_range(&from, &to).ok();
        self.app_summaries = self
            .db
            .get_app_summary_for_range(&from, &to)
            .unwrap_or_default();
    }

    fn date_range(&self) -> (String, String) {
        match self.mode {
            ViewMode::Day => {
                let d = self.date.format("%Y-%m-%d").to_string();
                (d.clone(), d)
            }
            ViewMode::Week => {
                let weekday = self.date.weekday().num_days_from_monday();
                let monday = self
                    .date
                    .checked_sub_days(chrono::Days::new(weekday as u64))
                    .unwrap();
                let sunday = monday
                    .checked_add_days(chrono::Days::new(6))
                    .unwrap();
                (
                    monday.format("%Y-%m-%d").to_string(),
                    sunday.format("%Y-%m-%d").to_string(),
                )
            }
            ViewMode::Month => {
                let first = self.date.with_day(1).unwrap();
                let last = if self.date.month() == 12 {
                    NaiveDate::from_ymd_opt(self.date.year() + 1, 1, 1)
                        .unwrap()
                        .pred_opt()
                        .unwrap()
                } else {
                    NaiveDate::from_ymd_opt(self.date.year(), self.date.month() + 1, 1)
                        .unwrap()
                        .pred_opt()
                        .unwrap()
                };
                (
                    first.format("%Y-%m-%d").to_string(),
                    last.format("%Y-%m-%d").to_string(),
                )
            }
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

fn mode_index(mode: ViewMode) -> usize {
    match mode {
        ViewMode::Day => 0,
        ViewMode::Week => 1,
        ViewMode::Month => 2,
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let (from, to) = app.date_range();
    let range_label = match app.mode {
        ViewMode::Day => from.clone(),
        ViewMode::Week => {
            let wn = app.date.iso_week().week();
            format!("W{wn} ({from} — {to})")
        }
        ViewMode::Month => app.date.format("%Y-%m").to_string(),
    };

    // top bar: title + tabs
    let top = Layout::horizontal([Constraint::Min(0), Constraint::Length(30)]).split(chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(format!(" SELFTrack  {range_label} "))).bold(),
        top[0],
    );
    let selected = mode_index(app.mode);
    let tabs = [" Day ", " Week ", " Month "]
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == selected {
                format!("▸{}◂", t.trim())
            } else {
                format!(" {} ", t.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    frame.render_widget(Paragraph::new(tabs), top[1]);

    // summary block
    let summary_text = match &app.summary {
        Some(s) => vec![
            Line::from(format!(" PC on:  {}", format_duration(s.pc_on_ms))),
            Line::raw(""),
            Line::from(format!(" Active: {}", format_duration(s.active_ms))),
            Line::from(format!(" Idle:   {}", format_duration(s.idle_ms))),
        ],
        None => vec![Line::from(" No data")],
    };
    frame.render_widget(
        Paragraph::new(summary_text).block(Block::bordered().title(" Summary ")),
        chunks[1],
    );

    // app table
    let active_ms = app.summary.as_ref().map(|s| s.active_ms).unwrap_or(0);
    let rows: Vec<Row> = app
        .app_summaries
        .iter()
        .map(|a| {
            let pct = if active_ms > 0 {
                (a.total_ms as f64 / active_ms as f64) * 100.0
            } else {
                0.0
            };
            Row::new(vec![
                Cell::from(a.app_class.as_str()),
                Cell::from(format_duration(a.total_ms)),
                Cell::from(format!("{pct:5.1}%")),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(55),
        Constraint::Percentage(30),
        Constraint::Percentage(15),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec!["App", "Time", "%"])
                .style(Style::new().add_modifier(Modifier::BOLD)),
        )
        .block(Block::bordered().title(" Applications "));

    frame.render_widget(table, chunks[2]);

    // footer
    frame.render_widget(
        Paragraph::new(Line::from(
            " ← → navigate period  |  Tab / 1 2 3 switch view  |  q quit ",
        )),
        chunks[3],
    );
}

pub fn run(db: db::Database) -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(db);

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if crossterm::event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('1') => {
                        app.mode = ViewMode::Day;
                        app.refresh();
                    }
                    KeyCode::Char('2') => {
                        app.mode = ViewMode::Week;
                        app.refresh();
                    }
                    KeyCode::Char('3') => {
                        app.mode = ViewMode::Month;
                        app.refresh();
                    }
                    KeyCode::Tab => {
                        app.mode = match app.mode {
                            ViewMode::Day => ViewMode::Week,
                            ViewMode::Week => ViewMode::Month,
                            ViewMode::Month => ViewMode::Day,
                        };
                        app.refresh();
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        let prev = match app.mode {
                            ViewMode::Day => app.date.pred_opt().unwrap_or(app.date),
                            ViewMode::Week => app
                                .date
                                .checked_sub_days(chrono::Days::new(7))
                                .unwrap_or(app.date),
                            ViewMode::Month => {
                                let m = if app.date.month() == 1 { 12 } else { app.date.month() - 1 };
                                let y = if app.date.month() == 1 {
                                    app.date.year() - 1
                                } else {
                                    app.date.year()
                                };
                                let d = app.date.day().min(last_day(y, m));
                                NaiveDate::from_ymd_opt(y, m, d).unwrap_or(app.date)
                            }
                        };
                        app.date = prev;
                        app.refresh();
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        let next = match app.mode {
                            ViewMode::Day => app.date.succ_opt().unwrap_or(app.date),
                            ViewMode::Week => app
                                .date
                                .checked_add_days(chrono::Days::new(7))
                                .unwrap_or(app.date),
                            ViewMode::Month => {
                                let m = if app.date.month() == 12 { 1 } else { app.date.month() + 1 };
                                let y = if app.date.month() == 12 {
                                    app.date.year() + 1
                                } else {
                                    app.date.year()
                                };
                                let d = app.date.day().min(last_day(y, m));
                                NaiveDate::from_ymd_opt(y, m, d).unwrap_or(app.date)
                            }
                        };
                        app.date = next;
                        app.refresh();
                    }
                    _ => {}
                }
            }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn last_day(year: i32, month: u32) -> u32 {
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
