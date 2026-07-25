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
        let conn = match Connection::connect_to_env() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("wayland connection failed: {e}, idle disabled");
                return;
            }
        };

        let mut event_queue = conn.new_event_queue();
        let qh = event_queue.handle();
        let display = conn.display();

        let mut state = WlState {
            tx,
            seat: None,
            notifier: None,
            notification: None,
            timeout_ms,
        };

        display.get_registry(&qh, ());

        if event_queue.roundtrip(&mut state).is_err() {
            tracing::warn!("wayland roundtrip failed, idle disabled");
            return;
        }

        tracing::info!("wayland idle notification active");

        loop {
            if event_queue.blocking_dispatch(&mut state).is_err() {
                tracing::warn!("wayland dispatch error, idle stopped");
                break;
            }
        }
    });
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
