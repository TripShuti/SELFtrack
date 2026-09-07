use std::os::unix::net::UnixStream;
use std::thread;
use tokio::sync::mpsc;
use wayland_client::{
    delegate_noop,
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::Event as IdleEv,
    ext_idle_notification_v1::ExtIdleNotificationV1,
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

pub enum IdleStatus {
    BecameIdle { at_ms: u64 },
    BecameActive { at_ms: u64 },
}

struct WlState {
    tx: mpsc::Sender<IdleStatus>,
    seat: Option<wl_seat::WlSeat>,
    notifier: Option<ExtIdleNotifierV1>,
    notification: Option<ExtIdleNotificationV1>,
    timeout_ms: u32,
}

pub fn spawn_idle_poller(threshold_min: u64, tx: mpsc::Sender<IdleStatus>) {
    let timeout_ms = (threshold_min * 60 * 1000) as u32;

    thread::spawn(move || {
        // Цикл перепідключення: раніше перша ж помилка dispatch
        // вбивала тред назавжди. Тепер ретраїмо з бекоффом, а щоб не
        // спамити журнал (було по 2 рядки кожні 5с годинами) —
        // варнінг тільки на перших спробах і далі раз на ~3 хв.
        let mut failures: u64 = 0;
        loop {
            if tx.is_closed() {
                return;
            }
            if run_poller(timeout_ms, &tx) {
                return;
            }
            failures += 1;
            let sleep_s = match failures {
                1..=3 => 5,
                4..=6 => 15,
                _ => 30,
            };
            if failures <= 3 || failures % 6 == 1 {
                tracing::warn!(
                    "idle poller stopped (attempt {failures}), reconnecting in {sleep_s}s"
                );
            } else {
                tracing::debug!(
                    "idle poller stopped (attempt {failures}), reconnecting in {sleep_s}s"
                );
            }
            thread::sleep(std::time::Duration::from_secs(sleep_s));
        }
    });
}

fn connect_wayland() -> Result<Connection, String> {
    // Швидкий шлях — через env.
    if let Ok(c) = Connection::connect_to_env() {
        return Ok(c);
    }
    // Повільний шлях — скан /run/user/*/wayland-*.
    // Важливо для випадку раннього старту systemd-юніта, коли в env
    // процесу WAYLAND_DISPLAY ще нема (env заморожений на момент exec),
    // а також якщо композитор перестворив сокет.
    let mut last_err = "connect_to_env failed".to_string();
    for path in crate::session::wayland_socket_candidates() {
        match UnixStream::connect(&path) {
            Ok(stream) => match Connection::from_socket(stream) {
                Ok(c) => return Ok(c),
                Err(e) => {
                    last_err = format!("{}: backend error: {e}", path.display());
                }
            },
            Err(e) => {
                last_err = format!("{}: {e}", path.display());
            }
        }
    }
    Err(last_err)
}

// true — чисте завершення (канал закрито, виходимо), false — ретраїти
fn run_poller(timeout_ms: u32, tx: &mpsc::Sender<IdleStatus>) -> bool {
        let conn = match connect_wayland() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("wayland connection failed: {e}");
                return false;
            }
        };

        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();
        let display = conn.display();

        let mut state = WlState {
            tx: tx.clone(),
            seat: None,
            notifier: None,
            notification: None,
            timeout_ms,
        };

        display.get_registry(&qh, ());

        if event_queue.roundtrip(&mut state).is_err() {
            tracing::debug!("wayland roundtrip failed, retrying");
            return false;
        }

        // Композитор може не мати ext_idle_notifier_v1 (або seat ще не
        // прийшов). Без цього idle-подій не буде ніколи — мовчки висіти
        // в dispatch не можна, треба ретраїти.
        if state.seat.is_none() || state.notifier.is_none() {
            tracing::debug!(
                "wayland idle protocol unavailable (seat: {}, notifier: {}), retrying",
                state.seat.is_some(),
                state.notifier.is_some(),
            );
            return false;
        }

        tracing::info!("wayland idle notification active");

        loop {
            if tx.is_closed() {
                return true;
            }
            if event_queue.blocking_dispatch(&mut state).is_err() {
                tracing::debug!("wayland dispatch error, reconnecting");
                return false;
            }
        }
}

impl Dispatch<wl_registry::WlRegistry, ()> for WlState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            match interface.as_str() {
                "wl_seat" => {
                    let seat: wl_seat::WlSeat = registry.bind(name, 1, qh, ());
                    state.seat = Some(seat);
                    state.try_create_notification(qh);
                }
                "ext_idle_notifier_v1" => {
                    let notifier: ExtIdleNotifierV1 = registry.bind(name, 1, qh, ());
                    state.notifier = Some(notifier);
                    state.try_create_notification(qh);
                }
                _ => {}
            }
        }
    }
}

impl WlState {
    fn try_create_notification(&mut self, qh: &QueueHandle<Self>) {
        if self.notification.is_some() {
            return;
        }
        if let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) {
            let notification = notifier.get_idle_notification(self.timeout_ms, seat, qh, ());
            tracing::info!("idle notification created");
            self.notification = Some(notification);
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _notification: &ExtIdleNotificationV1,
        event: IdleEv,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let now = current_time_ms();
        match event {
            IdleEv::Idled => {
                tracing::info!("user became idle (wayland)");
                let _ = state.tx.try_send(IdleStatus::BecameIdle { at_ms: now });
            }
            IdleEv::Resumed => {
                tracing::info!("user became active (wayland)");
                let _ = state.tx.try_send(IdleStatus::BecameActive { at_ms: now });
            }
            _ => {}
        }
    }
}

delegate_noop!(WlState: ignore wl_seat::WlSeat);
delegate_noop!(WlState: ignore ExtIdleNotifierV1);

pub fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
