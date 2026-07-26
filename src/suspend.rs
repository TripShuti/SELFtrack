use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::Connection;

#[derive(Debug)]
pub enum SuspendEvent {
    GoingToSleep { at_ms: u64 },
    Resumed { at_ms: u64 },
}

pub fn spawn_suspend_listener(tx: mpsc::Sender<SuspendEvent>) {
    tokio::spawn(async move {
        let conn = match Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("D-Bus connection failed: {e}, suspend detection disabled");
                return;
            }
        };

        let proxy = match LoginManagerProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("D-Bus login1 proxy failed: {e}, suspend detection disabled");
                return;
            }
        };

        tracing::info!("D-Bus suspend listener active");

        let mut stream = match proxy.receive_prepare_for_sleep().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("D-Bus signal subscribe failed: {e}, suspend detection disabled");
                return;
            }
        };

        while let Some(signal) = stream.next().await {
            let args = match signal.args() {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("D-Bus signal parse error: {e}");
                    continue;
                }
            };
            let now = crate::idle::current_time_ms();
            if args.start {
                let _ = tx.send(SuspendEvent::GoingToSleep { at_ms: now }).await;
            } else {
                let _ = tx.send(SuspendEvent::Resumed { at_ms: now }).await;
            }
        }
    });
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}
