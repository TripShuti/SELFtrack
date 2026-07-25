use crate::db;
use chrono::{Datelike, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table},
    Frame,
};
use std::collections::HashMap;
use std::io;

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Day,
    Week,
    Month,
    Calendar,
}

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
    mode: ViewMode,
    // shared
    date: NaiveDate,
    summary: Option<db::DaySummary>,
    app_summaries: Vec<db::AppSummary>,
    expanded_app: Option<usize>,
    expanded_pages: Vec<db::AppSummary>,
    selected: usize,
    // calendar
    cal_month: NaiveDate,
    cal_day: u32,
    day_totals: HashMap<String, DayTotal>,
    cal_focus: Focus,
    db: db::Database,
}

// ── App ──

impl App {
    fn new(db: db::Database) -> Self {
        let today = chrono::Local::now().naive_local().date();
        let mut app = Self {
            mode: ViewMode::Calendar,
            date: today,
            summary: None,
            app_summaries: vec![],
            expanded_app: None,
            expanded_pages: vec![],
            selected: 0,
            cal_month: today.with_day(1).unwrap(),
            cal_day: today.day(),
            day_totals: HashMap::new(),
            cal_focus: Focus::Calendar,
            db,
        };
        app.refresh_calendar();
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
        self.expanded_app = None;
        self.expanded_pages = vec![];
        self.selected = 0;
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
                    .or_insert(DayTotal {
                        active_ms: 0,
                        idle_ms: 0,
                    });
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
        if let Some(d) = self.cal_month.with_day(self.cal_day) {
            let ds = d.format("%Y-%m-%d").to_string();
            self.summary = self.db.get_summary_for_range(&ds, &ds).ok();
            self.app_summaries = self
                .db
                .get_app_summary_for_range(&ds, &ds)
                .unwrap_or_default();
            self.expanded_app = None;
            self.expanded_pages = vec![];
            self.selected = 0;
        }
    }

    fn selected_date(&self) -> NaiveDate {
        self.cal_month.with_day(self.cal_day).unwrap()
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
            let (from, to) = match self.mode {
                ViewMode::Calendar => {
                    let d = self.selected_date().format("%Y-%m-%d").to_string();
                    (d.clone(), d)
                }
                _ => self.date_range(),
            };
            let app_class = &self.app_summaries[app_idx].app_class;
            self.expanded_pages = self
                .db
                .get_page_summary_for_app(&from, &to, app_class)
                .unwrap_or_default();
            self.expanded_app = Some(app_idx);
        }
    }

    fn date_range(&self) -> (String, String) {
        match self.mode {
            ViewMode::Day | ViewMode::Calendar => {
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
                let last = last_day_of(self.date.year(), self.date.month());
                let last_date = NaiveDate::from_ymd_opt(self.date.year(), self.date.month(), last).unwrap();
                (first.format("%Y-%m-%d").to_string(), last_date.format("%Y-%m-%d").to_string())
            }
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

// ── colours ──



// ── drawing ──

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    if app.mode == ViewMode::Calendar {
        let [top_bar, cal_area, summary_area, table_area, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);

        draw_cal_header(frame, top_bar, app);
        draw_cal_grid(frame, cal_area, app);
        draw_cal_summary(frame, summary_area, app);
        draw_cal_table(frame, table_area, app);
        draw_cal_footer(frame, footer, app);
    } else {
        // day/week/month aggregate views
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

        draw_header(frame, chunks[0], app);
        draw_summary(frame, chunks[1], app);
        draw_table(frame, chunks[2], app);
        draw_footer(frame, chunks[3]);
    }
}

// ── calendar view ──

fn draw_cal_header(frame: &mut Frame, area: Rect, app: &App) {
    let halves: [Rect; 2] = Layout::horizontal([Constraint::Min(0), Constraint::Length(42)]).areas(area);
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
        halves[0],
    );
    let names = [" Day ", " Week ", " Month ", " Calendar "];
    let selected = 3;
    let tabs = names
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == selected {
                format!("\u{25b8}{}\u{25c2}", t.trim())
            } else {
                format!(" {} ", t.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    frame.render_widget(Paragraph::new(tabs), halves[1]);
}

fn draw_cal_grid(frame: &mut Frame, area: Rect, app: &App) {
    let days_in_month = last_day_of(app.cal_month.year(), app.cal_month.month());
    let first_weekday = app.cal_month.weekday().num_days_from_monday() as usize;
    let today = chrono::Local::now().naive_local().date();
    let focus_cal = app.cal_focus == Focus::Calendar;

    let header = " Mon Tue Wed Thu Fri Sat Sun ";

    let mut lines = vec![
        Line::from(""),
        Line::styled(header, Style::new().fg(Color::DarkGray)),
    ];

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
                    Style::new().fg(Color::DarkGray)
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

    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_cal_summary(frame: &mut Frame, area: Rect, app: &App) {
    let ds = app.selected_date().format("%Y-%m-%d %a").to_string();
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
        Paragraph::new(summary_text).block(Block::bordered().title(format!(" {ds} "))),
        area,
    );
}

fn draw_cal_table(frame: &mut Frame, area: Rect, app: &App) {
    let active_ms = app.summary.as_ref().map(|s| s.active_ms).unwrap_or(0);

    let mut rows: Vec<Row> = Vec::new();
    for (app_idx, a) in app.app_summaries.iter().enumerate() {
        let pct = if active_ms > 0 {
            (a.total_ms as f64 / active_ms as f64) * 100.0
        } else {
            0.0
        };
        let is_expanded = app.expanded_app == Some(app_idx);
        let prefix = if is_expanded { "▾ " } else { "▸ " };
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

    frame.render_widget(table, area);
}

fn draw_cal_footer(frame: &mut Frame, area: Rect, app: &App) {
    let hint = match app.cal_focus {
        Focus::Calendar => {
            " \u{2190}\u{2192}\u{2191}\u{2193} day  |  Enter detail  |  1 2 3 view  |  q quit"
        }
        Focus::Detail => {
            " \u{2191}\u{2193} select  |  Enter expand  |  Esc back to calendar  |  q quit"
        }
    };
    frame.render_widget(Paragraph::new(Line::from(hint)), area);
}

// ── aggregate views (Day / Week / Month) ──

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let top = Layout::horizontal([Constraint::Min(0), Constraint::Length(42)]).split(area);
    let (from, to) = app.date_range();
    let range_label = match app.mode {
        ViewMode::Day => from.clone(),
        ViewMode::Week => {
            let wn = app.date.iso_week().week();
            format!("W{wn} ({from} — {to})")
        }
        ViewMode::Month => app.date.format("%Y-%m").to_string(),
        _ => unreachable!(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(format!(" SELFTrack  {range_label} "))).bold(),
        top[0],
    );
    let names = [" Day ", " Week ", " Month ", " Calendar "];
    let selected = match app.mode {
        ViewMode::Day => 0,
        ViewMode::Week => 1,
        ViewMode::Month => 2,
        ViewMode::Calendar => 3,
    };
    let tabs = names
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == selected {
                format!("\u{25b8}{}\u{25c2}", t.trim())
            } else {
                format!(" {} ", t.trim())
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    frame.render_widget(Paragraph::new(tabs), top[1]);
}

fn draw_summary(frame: &mut Frame, area: Rect, app: &App) {
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
        let prefix = if is_expanded { "▾ " } else { "▸ " };
        let row_sel = app.selected == rows.len();

        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{prefix}{}", a.app_class),
                    if row_sel {
                        Style::new().fg(Color::White).bold()
                    } else {
                        Style::new().fg(Color::White)
                    },
                )),
                Cell::from(Span::styled(
                    format_duration(a.total_ms),
                    if row_sel {
                        Style::new().fg(Color::White).bold()
                    } else {
                        Style::new().fg(Color::White)
                    },
                )),
                Cell::from(Span::styled(
                    format!("{pct:5.1}%"),
                    Style::new(),
                )),
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
                        Cell::from(Span::styled(
                            format!("  └ {}", truncate(&clean_page_title(&p.app_class), 30)),
                            Style::new(),
                        )),
                        Cell::from(Span::styled(
                            format_duration(p.total_ms),
                            Style::new(),
                        )),
                        Cell::from(Span::styled(
                            format!("{page_pct:5.1}%"),
                            Style::new(),
                        )),
                    ])
                    .style(if row_sel { Style::new().add_modifier(Modifier::BOLD) } else { Style::new() }),
                );
            }
        }
    }

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

    frame.render_widget(table, area);
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    frame.render_widget(Paragraph::new(Line::from(
        " ← → period  |  Tab / 1 2 3 view  |  4 Calendar  |  ↑↓ select  |  Enter expand  |  q quit",
    )), area);
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

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if crossterm::event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,

                    // ── view switching ──
                    KeyCode::Tab => {
                        app.mode = match app.mode {
                            ViewMode::Day => ViewMode::Week,
                            ViewMode::Week => ViewMode::Month,
                            ViewMode::Month => ViewMode::Calendar,
                            ViewMode::Calendar => ViewMode::Day,
                        };
                        if app.mode == ViewMode::Calendar {
                            app.cal_focus = Focus::Calendar;
                        }
                        app.date = chrono::Local::now().naive_local().date();
                        app.refresh();
                    }
                    KeyCode::Char('1') => {
                        app.mode = ViewMode::Day;
                        app.date = chrono::Local::now().naive_local().date();
                        app.refresh();
                    }
                    KeyCode::Char('2') => {
                        app.mode = ViewMode::Week;
                        app.date = chrono::Local::now().naive_local().date();
                        app.refresh();
                    }
                    KeyCode::Char('3') => {
                        app.mode = ViewMode::Month;
                        app.date = chrono::Local::now().naive_local().date();
                        app.refresh();
                    }
                    KeyCode::Char('4') => {
                        app.mode = ViewMode::Calendar;
                        app.cal_focus = Focus::Calendar;
                        let today = chrono::Local::now().naive_local().date();
                        app.cal_month = today.with_day(1).unwrap();
                        app.cal_day = today.day();
                        app.refresh_calendar();
                    }

                    // ── Calendar mode navigation ──
                    _ if app.mode == ViewMode::Calendar => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('q') => break,

                            KeyCode::Left | KeyCode::Char('h') if app.cal_focus == Focus::Calendar => {
                                app.move_selection(-1);
                            }
                            KeyCode::Right | KeyCode::Char('l') if app.cal_focus == Focus::Calendar => {
                                app.move_selection(1);
                            }
                            KeyCode::Up | KeyCode::Char('k') if app.cal_focus == Focus::Calendar => {
                                app.move_selection(-7);
                            }
                            KeyCode::Down | KeyCode::Char('j') if app.cal_focus == Focus::Calendar => {
                                app.move_selection(7);
                            }

                            KeyCode::Enter => {
                                match app.cal_focus {
                                    Focus::Calendar => {
                                        app.cal_focus = Focus::Detail;
                                        app.selected = 0;
                                    }
                                    Focus::Detail => {
                                        if let Some(app_idx) = app.row_to_app_idx(app.selected) {
                                            if app_idx < app.app_summaries.len() {
                                                app.toggle_expand(app_idx);
                                            }
                                        }
                                    }
                                }
                            }

                            KeyCode::Esc => {
                                app.cal_focus = Focus::Calendar;
                            }
                            KeyCode::Up | KeyCode::Char('k') if app.cal_focus == Focus::Detail => {
                                if app.selected > 0 {
                                    app.selected -= 1;
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') if app.cal_focus == Focus::Detail => {
                                let max = app.row_count().saturating_sub(1);
                                if app.selected < max {
                                    app.selected += 1;
                                }
                            }

                            _ => {}
                        }
                    }

                    // ── aggregate view navigation (Day / Week / Month) ──
                    _ => {
                        match key.code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                if app.selected > 0 {
                                    app.selected -= 1;
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                let max = app.row_count().saturating_sub(1);
                                if app.selected < max {
                                    app.selected += 1;
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(app_idx) = app.row_to_app_idx(app.selected) {
                                    if app_idx < app.app_summaries.len() {
                                        let prev = app.expanded_app;
                                        app.toggle_expand(app_idx);
                                        if prev.is_some() && app.expanded_app.is_none() {
                                            if app.selected > app_idx
                                                && app.selected <= app_idx + app.expanded_pages.len()
                                            {
                                                app.selected = app_idx + 1;
                                            }
                                        }
                                    }
                                }
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
                                        let d = app.date.day().min(last_day_of(y, m));
                                        NaiveDate::from_ymd_opt(y, m, d).unwrap_or(app.date)
                                    }
                                    _ => unreachable!(),
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
                                        let d = app.date.day().min(last_day_of(y, m));
                                        NaiveDate::from_ymd_opt(y, m, d).unwrap_or(app.date)
                                    }
                                    _ => unreachable!(),
                                };
                                app.date = next;
                                app.refresh();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
