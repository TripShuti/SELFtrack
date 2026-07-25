use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub date: String,
    pub app_class: String,
    pub app_title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_idle: bool,
}

#[derive(Debug, Clone)]
pub struct AppSummary {
    pub app_class: String,
    pub total_ms: i64,
}

#[derive(Debug, Clone)]
pub struct DaySummary {
    pub date: String,
    pub pc_on_ms: i64,
    pub active_ms: i64,
    pub idle_ms: i64,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        let db = Database { conn: Mutex::new(conn) };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                date        TEXT    NOT NULL,
                app_class   TEXT    NOT NULL,
                app_title   TEXT    NOT NULL DEFAULT '',
                start_ms    INTEGER NOT NULL,
                end_ms      INTEGER NOT NULL,
                is_idle     INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_date ON sessions(date);
            CREATE INDEX IF NOT EXISTS idx_sessions_app ON sessions(date, app_class);"
        )?;
        Ok(())
    }

    pub fn insert_session(&self, s: &Session) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (date, app_class, app_title, start_ms, end_ms, is_idle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![s.date, s.app_class, s.app_title, s.start_ms, s.end_ms, s.is_idle as i32],
        )?;
        Ok(())
    }

    pub fn get_sessions_for_date(&self, date: &str) -> Result<Vec<Session>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT date, app_class, app_title, start_ms, end_ms, is_idle
             FROM sessions WHERE date = ?1 ORDER BY start_ms"
        )?;
        let rows = stmt.query_map(params![date], |row| {
            Ok(Session {
                date: row.get(0)?,
                app_class: row.get(1)?,
                app_title: row.get(2)?,
                start_ms: row.get(3)?,
                end_ms: row.get(4)?,
                is_idle: row.get::<_, i32>(5)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn get_app_summary_for_date(&self, date: &str) -> Result<Vec<AppSummary>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT app_class, SUM(end_ms - start_ms) as total_ms
             FROM sessions WHERE date = ?1 AND is_idle = 0
             GROUP BY app_class ORDER BY total_ms DESC"
        )?;
        let rows = stmt.query_map(params![date], |row| {
            Ok(AppSummary {
                app_class: row.get(0)?,
                total_ms: row.get(1)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_day_summary(&self, date: &str) -> Result<DaySummary, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let active_ms: i64 = conn.query_row(
            "SELECT COALESCE(SUM(end_ms - start_ms), 0) FROM sessions
             WHERE date = ?1 AND is_idle = 0",
            params![date],
            |row| row.get(0),
        )?;
        let idle_ms: i64 = conn.query_row(
            "SELECT COALESCE(SUM(end_ms - start_ms), 0) FROM sessions
             WHERE date = ?1 AND is_idle = 1",
            params![date],
            |row| row.get(0),
        )?;
        let pc_on_ms = active_ms + idle_ms;
        Ok(DaySummary {
            date: date.to_string(),
            pc_on_ms,
            active_ms,
            idle_ms,
        })
    }

    pub fn get_last_session_for_date(&self, date: &str) -> Result<Option<Session>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT date, app_class, app_title, start_ms, end_ms, is_idle
             FROM sessions WHERE date = ?1 ORDER BY start_ms DESC LIMIT 1"
        )?;
        let mut rows = stmt.query_map(params![date], |row| {
            Ok(Session {
                date: row.get(0)?,
                app_class: row.get(1)?,
                app_title: row.get(2)?,
                start_ms: row.get(3)?,
                end_ms: row.get(4)?,
                is_idle: row.get::<_, i32>(5)? != 0,
            })
        })?;
        match rows.next() {
            Some(Ok(s)) => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    pub fn get_summary_for_range(&self, from: &str, to: &str) -> Result<DaySummary, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let active_ms: i64 = conn.query_row(
            "SELECT COALESCE(SUM(end_ms - start_ms), 0) FROM sessions
             WHERE date >= ?1 AND date <= ?2 AND is_idle = 0",
            params![from, to],
            |row| row.get(0),
        )?;
        let idle_ms: i64 = conn.query_row(
            "SELECT COALESCE(SUM(end_ms - start_ms), 0) FROM sessions
             WHERE date >= ?1 AND date <= ?2 AND is_idle = 1",
            params![from, to],
            |row| row.get(0),
        )?;
        let pc_on_ms = active_ms + idle_ms;
        Ok(DaySummary {
            date: format!("{} — {}", from, to),
            pc_on_ms,
            active_ms,
            idle_ms,
        })
    }

    pub fn get_app_summary_for_range(&self, from: &str, to: &str) -> Result<Vec<AppSummary>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT app_class, SUM(end_ms - start_ms) as total_ms
             FROM sessions WHERE date >= ?1 AND date <= ?2 AND is_idle = 0
             GROUP BY app_class ORDER BY total_ms DESC"
        )?;
        let rows = stmt.query_map(params![from, to], |row| {
            Ok(AppSummary {
                app_class: row.get(0)?,
                total_ms: row.get(1)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_sessions_for_date(&self, date: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE date = ?1", params![date])?;
        Ok(())
    }
}
