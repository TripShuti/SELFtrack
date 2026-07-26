use crate::db;
use chrono::{Datelike, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table},
    Frame,
};
use std::collections::HashMap;
use std::io;

struct DayTotal {
    active_ms: i64,
    idle_ms: i64,
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Calendar,
    Detail,
}

struct App {
    cal_month: NaiveDate,
    cal_day: u32,
    day_totals: HashMap<String, DayTotal>,
    focus: Focus,

    summary: Option<db::DaySummary>,
    week_summary: Option<db::DaySummary>,
    month_summary: Option<db::DaySummary>,
    app_summaries: Vec<db::AppSummary>,
    expanded_app: Option<usize>,
    expanded_pages: Vec<db::AppSummary>,
    selected: usize,
    scroll: usize,

    db: db::Database,
}

// ── App ──

impl App {
    fn new(db: db::Database) -> Self {
        let today = chrono::Local::now().naive_local().date();
        let mut app = Self {
            cal_month: today.with_day(1).unwrap(),
            cal_day: today.day(),
            day_totals: HashMap::new(),
            focus: Focus::Calendar,

            summary: None,
            week_summary: None,
            month_summary: None,
            app_summaries: vec![],
            expanded_app: None,
            expanded_pages: vec![],
            selected: 0,
            scroll: 0,

            db,
        };
        app.refresh_calendar();
        app
    }

    fn selected_date(&self) -> NaiveDate {
        self.cal_month.with_day(self.cal_day).unwrap()
    }

    fn refresh_calendar(&mut self) {
        let first = self.cal_month;
        let last = last_day_of(first.year(), first.month());
        let last_date = first.with_day(last).unwrap();
        let from = first.format("%Y-%m-%d").to_string();
        let to = last_date.format("%Y-%m-%d").to_string();
        self.day_totals.clear();
        if let Ok(sessions) = self.db.get_sessions_for_range(&from, &to) {
            for s in &sessions {
                let dt = self
                    .day_totals
                    .entry(s.date.clone())
                    .or_insert(DayTotal { active_ms: 0, idle_ms: 0 });
                if s.is_idle {
                    dt.idle_ms += s.end_ms - s.start_ms;
                } else {
                    dt.active_ms += s.end_ms - s.start_ms;
                }
            }
        }
        self.refresh_day();
    }

    fn refresh_day(&mut self) {
        let date = self.selected_date();
        let ds = date.format("%Y-%m-%d").to_string();

        self.summary = self.db.get_summary_for_range(&ds, &ds).ok();
        self.app_summaries = self
            .db
            .get_app_summary_for_range(&ds, &ds)
            .unwrap_or_default();

        // week containing this day
        let weekday = date.weekday().num_days_from_monday();
        let monday = date.checked_sub_days(chrono::Days::new(weekday as u64)).unwrap();
        let sunday = monday.checked_add_days(chrono::Days::new(6)).unwrap();
        let wf = monday.format("%Y-%m-%d").to_string();
        let wt = sunday.format("%Y-%m-%d").to_string();
        self.week_summary = self.db.get_summary_for_range(&wf, &wt).ok();

        // month containing this day
        let mf = date.with_day(1).unwrap();
        let ml = mf.with_day(last_day_of(date.year(), date.month())).unwrap();
        let mfs = mf.format("%Y-%m-%d").to_string();
        let mls = ml.format("%Y-%m-%d").to_string();
        self.month_summary = self.db.get_summary_for_range(&mfs, &mls).ok();

        self.expanded_app = None;
        self.expanded_pages = vec![];
        self.selected = 0;
        self.scroll = 0;
    }

    fn move_selection(&mut self, delta_days: i64) {
        let current = self.selected_date();
        let new = if delta_days >= 0 {
            current.checked_add_days(chrono::Days::new(delta_days as u64))
        } else {
            current.checked_sub_days(chrono::Days::new((-delta_days) as u64))
        };
        if let Some(new) = new {
            let changed_month = new.month() != self.cal_month.month()
                || new.year() != self.cal_month.year();
            if changed_month {
                self.cal_month = NaiveDate::from_ymd_opt(new.year(), new.month(), 1).unwrap();
                self.cal_day = new.day();
                self.refresh_calendar();
            } else {
                self.cal_day = new.day();
                self.refresh_day();
            }
        }
    }

    fn row_count(&self) -> usize {
        let base = self.app_summaries.len();
        if self.expanded_app.is_some() {
            base + self.expanded_pages.len()
        } else {
            base
        }
    }

    fn row_to_app_idx(&self, row: usize) -> Option<usize> {
        if let Some(exp) = self.expanded_app {
            if row < exp + 1 {
                Some(row)
            } else if row < exp + 1 + self.expanded_pages.len() {
                None
            } else {
                Some(row - self.expanded_pages.len())
            }
        } else {
            Some(row)
        }
    }

    fn toggle_expand(&mut self, app_idx: usize) {
        if self.expanded_app == Some(app_idx) {
            self.expanded_app = None;
            self.expanded_pages = vec![];
        } else {
            let ds = self.selected_date().format("%Y-%m-%d").to_string();
            let app_class = &self.app_summaries[app_idx].app_class;
            self.expanded_pages = self
                .db
                .get_page_summary_for_app(&ds, &ds, app_class)
                .unwrap_or_default();
            self.expanded_app = Some(app_idx);
        }
    }
}

// ── helpers ──

fn format_duration(ms: i64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
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

// ── drawing ──

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let [top_bar, mid, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    let [cs_area, table_area] = Layout::vertical([
        Constraint::Length(9),
        Constraint::Min(0),
    ])
    .areas(mid);

    let [cal_area, sum_area] = Layout::horizontal([
        Constraint::Length(34),
        Constraint::Min(0),
    ])
    .areas(cs_area);

    draw_header(frame, top_bar, app);
    draw_cal_grid(frame, cal_area, app);
    draw_summary(frame, sum_area, app);
    draw_table(frame, table_area, app);
    draw_footer(frame, footer, app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" SELFTrack  "),
            Span::styled("\u{25c0} ", Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(
                app.cal_month.format("%Y/%m").to_string(),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" \u{25b6}", Style::new().add_modifier(Modifier::BOLD)),
        ])),
        area,
    );
}

fn draw_cal_grid(frame: &mut Frame, area: Rect, app: &App) {
    let days_in_month = last_day_of(app.cal_month.year(), app.cal_month.month());
    let first_weekday = app.cal_month.weekday().num_days_from_monday() as usize;
    let today = chrono::Local::now().naive_local().date();
    let focus_cal = app.focus == Focus::Calendar;

    let wd_header = " Mon Tue Wed Thu Fri Sat Sun ";

    let mut lines = vec![Line::styled(wd_header, Style::new().fg(ratatui::style::Color::DarkGray))];

    let mut day = 1i32;
    loop {
        let mut spans = Vec::new();
        for c in 0..7 {
            if (day == 1 && c < first_weekday) || day > days_in_month as i32 {
                spans.push(Span::raw("    "));
            } else {
                let d = day as u32;
                let current = app.cal_month.with_day(d).unwrap();
                let is_selected = current == app.selected_date();
                let is_today = current == today;
                let has_data = app.day_totals.contains_key(&current.format("%Y-%m-%d").to_string());

                let marker = if is_today && !is_selected { "*" } else { " " };
                let cell = format!("{:>2}{} ", d, marker);

                let style = if is_selected && focus_cal {
                    Style::new().add_modifier(Modifier::BOLD)
                } else if is_today {
                    Style::new().add_modifier(Modifier::BOLD)
                } else if has_data {
                    Style::new()
                } else {
                    Style::new().fg(ratatui::style::Color::DarkGray)
                };

                spans.push(Span::styled(cell, style));
                day += 1;
            }
        }
        lines.push(Line::from(spans));
        if day > days_in_month as i32 {
            break;
        }
    }

    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Calendar ")),
        area,
    );
}

fn draw_summary(frame: &mut Frame, area: Rect, app: &App) {
    let ds = app.selected_date().format("%Y-%m-%d %a").to_string();
    let day = &app.summary;
    let week = &app.week_summary;
    let month = &app.month_summary;

    let active_line = match day {
        Some(s) => format!(" Active {}", format_duration(s.active_ms)),
        None => String::new(),
    };
    let idle_line = match day {
        Some(s) => format!(" Idle   {}", format_duration(s.idle_ms)),
        None => String::new(),
    };
    let week_line = match week {
        Some(s) => {
            let wn = app.selected_date().iso_week().week();
            format!(" Week   {}  (W{wn})", format_duration(s.pc_on_ms))
        }
        None => " Week   —".into(),
    };
    let month_line = match month {
        Some(s) => {
            format!(" Month  {}  ({})", format_duration(s.pc_on_ms), app.selected_date().format("%Y-%m"))
        }
        None => " Month  —".into(),
    };

    let mut text = vec![];
    if !active_line.is_empty() {
        text.push(Line::from(active_line));
    }
    if !idle_line.is_empty() {
        text.push(Line::from(idle_line));
    }
    text.push(Line::from(week_line));
    text.push(Line::from(month_line));

    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title(format!(" {ds} "))),
        area,
    );
}

fn draw_table(frame: &mut Frame, area: Rect, app: &App) {
    let active_ms = app.summary.as_ref().map(|s| s.active_ms).unwrap_or(0);

    let mut rows: Vec<Row> = Vec::new();
    for (app_idx, a) in app.app_summaries.iter().enumerate() {
        let pct = if active_ms > 0 {
            (a.total_ms as f64 / active_ms as f64) * 100.0
        } else {
            0.0
        };
        let is_expanded = app.expanded_app == Some(app_idx);
        let prefix = if is_expanded { "\u{25be} " } else { "\u{25b8} " };
        let row_sel = app.selected == rows.len();

        rows.push(
            Row::new(vec![
                Cell::from(format!("{prefix}{}", a.app_class)),
                Cell::from(format_duration(a.total_ms)),
                Cell::from(format!("{pct:5.1}%")),
            ])
            .style(if row_sel { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() }),
        );

        if is_expanded {
            for p in &app.expanded_pages {
                let page_pct = if a.total_ms > 0 {
                    (p.total_ms as f64 / a.total_ms as f64) * 100.0
                } else {
                    0.0
                };
                let row_sel = app.selected == rows.len();
                rows.push(
                    Row::new(vec![
                        Cell::from(format!("  \u{2514} {}", truncate(&clean_page_title(&p.app_class), 30))),
                        Cell::from(format_duration(p.total_ms)),
                        Cell::from(format!("{page_pct:5.1}%")),
                    ])
                    .style(if row_sel { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() }),
                );
            }
        }
    }

    let total_rows = rows.len();
    let visible = area.height.saturating_sub(3) as usize;
    let max_scroll = total_rows.saturating_sub(visible);
    let scroll = app.scroll.min(max_scroll);
    let end = std::cmp::min(scroll + visible, total_rows);
    let displayed: Vec<Row> = rows.drain(scroll..end).collect();

    let widths = [
        Constraint::Percentage(55),
        Constraint::Percentage(30),
        Constraint::Percentage(15),
    ];
    let table = Table::new(displayed, widths)
        .header(
            Row::new(vec!["App", "Time", "%"])
                .style(Style::new().add_modifier(Modifier::BOLD)),
        )
        .block(Block::bordered().title(" Applications "));

    frame.render_widget(table, area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let hint = match app.focus {
        Focus::Calendar => {
            " \u{2190}\u{2192} day  \u{2191}\u{2193} week  |  Enter detail  |  q quit"
        }
        Focus::Detail => {
            " \u{2191}\u{2193} select  |  Enter expand  |  Esc calendar  |  q quit"
        }
    };
    frame.render_widget(Paragraph::new(Line::from(hint)), area);
}

// ── public entry point ──

pub fn run(db: db::Database) -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(db);

    let mut visible_rows = 1usize;

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if let Ok(size) = terminal.size() {
            visible_rows = (size.height.saturating_sub(14)).max(1) as usize;
        }

        if crossterm::event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => {
                        if app.focus == Focus::Detail {
                            app.focus = Focus::Calendar;
                        } else {
                            break;
                        }
                    }

                    KeyCode::Left | KeyCode::Char('h') if app.focus == Focus::Calendar => {
                        app.move_selection(-1);
                    }
                    KeyCode::Right | KeyCode::Char('l') if app.focus == Focus::Calendar => {
                        app.move_selection(1);
                    }
                    KeyCode::Up | KeyCode::Char('k') if app.focus == Focus::Calendar => {
                        app.move_selection(-7);
                    }
                    KeyCode::Down | KeyCode::Char('j') if app.focus == Focus::Calendar => {
                        app.move_selection(7);
                    }

                    KeyCode::Enter | KeyCode::Tab => match app.focus {
                        Focus::Calendar => {
                            app.focus = Focus::Detail;
                            app.selected = 0;
                        }
                        Focus::Detail => {
                            if let Some(app_idx) = app.row_to_app_idx(app.selected) {
                                if app_idx < app.app_summaries.len() {
                                    app.toggle_expand(app_idx);
                                }
                            }
                        }
                    },

                    KeyCode::Up | KeyCode::Char('k') if app.focus == Focus::Detail => {
                        if app.selected > 0 {
                            app.selected -= 1;
                            if app.selected < app.scroll {
                                app.scroll = app.scroll.saturating_sub(1);
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') if app.focus == Focus::Detail => {
                        let max = app.row_count().saturating_sub(1);
                        if app.selected < max {
                            app.selected += 1;
                            if app.selected >= app.scroll + visible_rows {
                                app.scroll += 1;
                            }
                        }
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
