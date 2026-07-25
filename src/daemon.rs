use std::sync::Arc;

use crate::db::{Database, Session};
use crate::hypr::{self, HyprEvent};
use crate::idle::{self, IdleStatus};
use tokio::sync::mpsc;

struct TrackerState {
    current_class: String,
    current_title: String,
    session_start_ms: u64,
}

pub async fn run(db: Arc<Database>, idle_threshold_min: u64) {
    let (hypr_tx, mut hypr_rx) = mpsc::channel::<HyprEvent>(64);
    let (idle_tx, mut idle_rx) = mpsc::channel::<IdleStatus>(64);

    idle::spawn_idle_poller(idle_threshold_min, idle_tx);
    hypr::spawn_event_listener(hypr_tx);

    let mut state = get_initial_state();
    let mut is_idle = false;

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
                        let now = idle::current_time_ms();
                        finalize_session(&db, &state, now, is_idle);
                        state = TrackerState {
                            current_class: class,
                            current_title: title,
                            session_start_ms: now,
                        };
                        if is_idle {
                            is_idle = false;
                        }
                    }
                    HyprEvent::Other(_) => {}
                }
            }
            Some(ev) = idle_rx.recv() => {
                match ev {
                    IdleStatus::BecameIdle { at_ms } => {
                        if !is_idle {
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
                        }
                    }
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
