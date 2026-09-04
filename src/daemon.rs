use std::sync::Arc;

use crate::audio;
use crate::db::{Database, Session};
use crate::hypr::{self, HyprEvent};
use crate::idle::{self, IdleStatus};
use crate::suspend::SuspendEvent;
use tokio::sync::mpsc;

struct TrackerState {
    current_class: String,
    current_title: String,
    session_start_ms: u64,
}

pub async fn run(db: Arc<Database>, idle_threshold_min: u64, retention_days: u64) {
    let (hypr_tx, mut hypr_rx) = mpsc::channel::<HyprEvent>(64);
    let (idle_tx, mut idle_rx) = mpsc::channel::<IdleStatus>(64);
    let (suspend_tx, mut suspend_rx) = mpsc::channel::<SuspendEvent>(64);

    idle::spawn_idle_poller(idle_threshold_min, idle_tx);
    hypr::spawn_event_listener(hypr_tx);
    crate::suspend::spawn_suspend_listener(suspend_tx);

    let mut state = get_initial_state();
    let mut is_idle = false;
    let mut is_sleeping = false;

    let mut suppress_active = false;
    let mut recheck = tokio::time::interval(std::time::Duration::from_secs(30));
    let mut flush = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut last_prune_date = today();
    prune(&db, retention_days);

    tracing::info!(
        "tracking started, initial app: {} / {}",
        state.current_class,
        state.current_title
    );

    loop {
        tokio::select! {
            Some(ev) = hypr_rx.recv() => {
                match ev {
                    HyprEvent::ActiveWindow { class, title } => {
                        if (is_idle || is_sleeping) && !suppress_active {
                            continue;
                        }
                        suppress_active = false;
                        if is_idle || is_sleeping {
                            continue;
                        }
                        tracing::info!("activewindow: {class} / {title}");
                        let now = idle::current_time_ms();
                        finalize_session(&db, &state, now, false);
                        state = TrackerState {
                            current_class: class,
                            current_title: title,
                            session_start_ms: now,
                        };
                    }
                    HyprEvent::Other(_) => {}
                }
            }
            Some(ev) = idle_rx.recv() => {
                if is_sleeping {
                    continue;
                }
                match ev {
                    IdleStatus::BecameIdle { at_ms } => {
                        if !is_idle {
                            if audio::is_audio_playing().await {
                                suppress_active = true;
                                recheck.reset();
                                tracing::info!(
                                    "suppressing idle — {} is playing audio",
                                    state.current_class
                                );
                                continue;
                            }
                            tracing::info!("user became idle");
                            finalize_session(&db, &state, at_ms, false);
                            is_idle = true;
                            state.session_start_ms = at_ms;
                        }
                    }
                    IdleStatus::BecameActive { at_ms } => {
                        if is_idle {
                            tracing::info!("user became active");
                            let idle_session = Session {
                                date: today(),
                                app_class: "__idle__".into(),
                                app_title: String::new(),
                                start_ms: state.session_start_ms as i64,
                                end_ms: at_ms as i64,
                                is_idle: true,
                            };
                            let _ = db.insert_session(&idle_session);
                            is_idle = false;
                            match hypr::get_active_window() {
                                Ok(w) => {
                                    state = TrackerState {
                                        current_class: w.class,
                                        current_title: w.title,
                                        session_start_ms: at_ms,
                                    };
                                }
                                Err(e) => {
                                    tracing::warn!("could not get active window: {e}");
                                    state.session_start_ms = at_ms;
                                }
                            }
                        } else if suppress_active {
                            tracing::info!("user became active, clearing suppress");
                            suppress_active = false;
                        }
                    }
                }
            }
            Some(ev) = suspend_rx.recv() => {
                match ev {
                    SuspendEvent::GoingToSleep { at_ms } => {
                        tracing::info!("system going to sleep");
                        suppress_active = false;
                        if !is_idle {
                            finalize_session(&db, &state, at_ms, false);
                            state.session_start_ms = at_ms;
                        }
                        is_sleeping = true;
                        is_idle = false;
                    }
                    SuspendEvent::Resumed { at_ms } => {
                        if is_sleeping {
                            tracing::info!("system resumed from sleep");
                            is_sleeping = false;
                            match hypr::get_active_window() {
                                Ok(w) => {
                                    state = TrackerState {
                                        current_class: w.class,
                                        current_title: w.title,
                                        session_start_ms: at_ms,
                                    };
                                }
                                Err(e) => {
                                    tracing::warn!("could not get active window: {e}");
                                    state.session_start_ms = at_ms;
                                }
                            }
                        }
                    }
                }
            }
            _ = recheck.tick() => {
                if suppress_active && !audio::is_audio_playing().await {
                    let now = idle::current_time_ms();
                    tracing::info!("ending suppressed idle — audio stopped");
                    finalize_session(&db, &state, now, false);
                    is_idle = true;
                    state.session_start_ms = now;
                    suppress_active = false;
                }
            }
            _ = flush.tick() => {
                // Живий флашу: без цього відкрита сесія лежить тільки в RAM
                // і віджет/export бачать заморожені дані до перемикання вікна.
                // Спліт — фіналізуємо шматок і одразу починаємо новий з тими
                // ж class/title. Один INSERT/хв навантаження не дає.
                if !is_idle && !is_sleeping {
                    let now = idle::current_time_ms();
                    finalize_session(&db, &state, now, false);
                    state.session_start_ms = now;
                }
                // Демон живе тижнями без рестартів, тому автопрун —
                // не частіше разу на добу, а не тільки на старті
                let day = today();
                if day != last_prune_date {
                    last_prune_date = day;
                    prune(&db, retention_days);
                }
            }
        }
    }
}

fn get_initial_state() -> TrackerState {
    match hypr::get_active_window() {
        Ok(w) => TrackerState {
            current_class: w.class,
            current_title: w.title,
            session_start_ms: idle::current_time_ms(),
        },
        Err(e) => {
            tracing::warn!("initial active window query failed: {e}");
            TrackerState {
                current_class: "unknown".into(),
                current_title: String::new(),
                session_start_ms: idle::current_time_ms(),
            }
        }
    }
}

fn finalize_session(db: &Database, state: &TrackerState, end_ms: u64, was_idle: bool) {
    let duration_ms = end_ms.saturating_sub(state.session_start_ms);
    if duration_ms < 1000 {
        return;
    }
    let session = Session {
        date: today(),
        app_class: if was_idle {
            "__idle__".into()
        } else {
            state.current_class.clone()
        },
        app_title: state.current_title.clone(),
        start_ms: state.session_start_ms as i64,
        end_ms: end_ms as i64,
        is_idle: was_idle,
    };
    if let Err(e) = db.insert_session(&session) {
        tracing::error!("failed to insert session: {e}");
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn prune(db: &Database, retention_days: u64) {
    if retention_days == 0 {
        return;
    }
    let cutoff = chrono::Local::now()
        .naive_local()
        .date()
        .checked_sub_days(chrono::Days::new(retention_days))
        .map(|d| d.format("%Y-%m-%d").to_string());
    let Some(cutoff) = cutoff else {
        return;
    };
    match db.delete_sessions_older_than(&cutoff) {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!("pruned {n} sessions older than {cutoff}");
            if let Err(e) = db.vacuum() {
                tracing::warn!("vacuum failed: {e}");
            }
        }
        Err(e) => tracing::warn!("prune failed: {e}"),
    }
}
