//! Lifecycle controller for `humatad2`'s loopback To-do web server (WEBTODO, D905).
//!
//! Mirrors `fusefs`'s controller pattern: `set_enabled(bool)` turns the server
//! on or off live from Settings, `status()` reports the current state and URL
//! for `SettingsService.GetSettings`, and `on_status_change` publishes Settings
//! events when the listener starts, stops, or fails.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;

use crate::server::{Assets, BoardSource, RunningServer, ServerError, serve_loopback};

/// Default loopback port for Humata's To-do web UI so open browser tabs
/// survive daemon restarts without changing URL. Falls back to an ephemeral
/// loopback port (`0`) if already occupied.
pub const DEFAULT_PORT: u16 = 17_420;

/// Live state of the To-do web server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WebTodoStatus {
    /// Disabled by Settings (the default).
    #[default]
    Off,
    /// Starting up and binding its loopback socket.
    Starting,
    /// Listening on `url` (e.g. `http://127.0.0.1:17420/`).
    Listening { url: String },
    /// Wanted, but failed to bind or start.
    Failed { reason: String },
}

struct Inner {
    enabled: bool,
    status: WebTodoStatus,
    running: Option<RunningServer>,
    on_change: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Manages the loopback To-do web server's start/stop lifecycle for `humatad2`.
pub struct WebTodoController {
    runtime: Handle,
    preferred_addr: SocketAddr,
    assets: Assets,
    source: Arc<dyn BoardSource>,
    inner: Mutex<Inner>,
}

impl WebTodoController {
    /// Creates a new controller (starts in [`WebTodoStatus::Off`]).
    #[must_use]
    pub fn new(
        runtime: Handle,
        preferred_addr: SocketAddr,
        assets: Assets,
        source: Arc<dyn BoardSource>,
    ) -> Self {
        Self {
            runtime,
            preferred_addr,
            assets,
            source,
            inner: Mutex::new(Inner {
                enabled: false,
                status: WebTodoStatus::Off,
                running: None,
                on_change: None,
            }),
        }
    }

    /// Default loopback bind address (`127.0.0.1:17420`).
    #[must_use]
    pub const fn default_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT)
    }

    /// Registers a callback fired whenever [`Self::status`] transitions.
    pub fn on_status_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.on_change = Some(callback);
    }

    /// Current live status of the server.
    #[must_use]
    pub fn status(&self) -> WebTodoStatus {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }

    /// Turns the loopback web server on or off. Blocks briefly on the tokio
    /// handle so the caller immediately sees `Listening { url }` or `Off`
    /// (safe to call from `spawn_blocking`, just like `fusefs.set_enabled`).
    pub fn set_enabled(&self, enabled: bool) {
        let notify = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.enabled == enabled
                && ((enabled && inner.running.is_some()) || (!enabled && inner.running.is_none()))
            {
                return;
            }
            inner.enabled = enabled;
            if !enabled {
                if let Some(mut running) = inner.running.take() {
                    running.stop_now();
                }
                inner.status = WebTodoStatus::Off;
                inner.on_change.clone()
            } else {
                inner.status = WebTodoStatus::Starting;
                None
            }
        };

        if let Some(cb) = notify {
            cb();
            return;
        }
        if !enabled {
            return;
        }

        let preferred = self.preferred_addr;
        let assets = self.assets.clone();
        let source = Arc::clone(&self.source);
        let started = self.runtime.block_on(async move {
            match serve_loopback(preferred, assets.clone(), Arc::clone(&source)).await {
                Ok(server) => Ok(server),
                Err(ServerError::Bind {
                    source: io_err, ..
                }) if io_err.kind() == std::io::ErrorKind::AddrInUse && preferred.port() != 0 => {
                    let fallback = SocketAddr::new(preferred.ip(), 0);
                    serve_loopback(fallback, assets, source).await
                }
                Err(err) => Err(err),
            }
        });

        let notify = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if !inner.enabled {
                // Turned off while binding.
                if let Ok(mut server) = started {
                    server.stop_now();
                }
                inner.status = WebTodoStatus::Off;
            } else {
                match started {
                    Ok(server) => {
                        let url = server.url();
                        inner.running = Some(server);
                        inner.status = WebTodoStatus::Listening { url };
                    }
                    Err(err) => {
                        inner.running = None;
                        inner.status = WebTodoStatus::Failed {
                            reason: err.to_string(),
                        };
                    }
                }
            }
            inner.on_change.clone()
        };

        if let Some(cb) = notify {
            cb();
        }
    }

    /// Stops the server cleanly on daemon shutdown.
    pub fn shutdown(&self) {
        let running = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.enabled = false;
            inner.status = WebTodoStatus::Off;
            inner.running.take()
        };
        if let Some(mut server) = running {
            server.stop_now();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use futures_util::StreamExt;
    use humata2_todo::{Day, State, Todo};
    use humata2_web_todo_wire::{BoardSnapshot, ChannelInfo, ServerMessage};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::broadcast;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    use super::*;
    use crate::server::is_loopback_origin;

    struct FakeSource {
        snapshot: Mutex<BoardSnapshot>,
        updates: broadcast::Sender<()>,
    }

    impl FakeSource {
        fn new() -> Self {
            let (updates, _) = broadcast::channel(16);
            let today = Day::from_ymd(2026, 11, 20).expect("day");
            Self {
                snapshot: Mutex::new(BoardSnapshot {
                    today,
                    items: vec![Todo::new("PAY-1", "Initial item", today)],
                    channels: vec![ChannelInfo {
                        id: "ch-1".to_owned(),
                        name: "payments".to_owned(),
                    }],
                    sessions: Vec::new(),
                    revision: 1,
                }),
                updates,
            }
        }

        fn push_item(&self, key: &str, title: &str) {
            let mut snap = self.snapshot.lock().unwrap();
            let mut todo = Todo::new(key, title, snap.today);
            todo.state = State::InProgress;
            snap.items.push(todo);
            snap.revision += 1;
            let _ = self.updates.send(());
        }
    }

    impl BoardSource for FakeSource {
        fn snapshot(&self) -> BoardSnapshot {
            self.snapshot.lock().unwrap().clone()
        }

        fn subscribe(&self) -> broadcast::Receiver<()> {
            self.updates.subscribe()
        }
    }

    async fn http_get(addr: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.expect("read");
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[tokio::test]
    async fn refuses_non_loopback_bind_address() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        let source = Arc::new(FakeSource::new());
        let err = serve_loopback(addr, Assets::fallback_placeholder(), source)
            .await
            .err()
            .expect("must refuse 0.0.0.0");
        assert!(matches!(err, ServerError::NonLoopback { .. }));
    }

    #[test]
    fn loopback_origin_policy() {
        assert!(is_loopback_origin(None));
        assert!(is_loopback_origin(Some("http://127.0.0.1:17420")));
        assert!(is_loopback_origin(Some("http://localhost:8080")));
        assert!(is_loopback_origin(Some("http://[::1]:17420")));
        assert!(!is_loopback_origin(Some("https://evil.example")));
        assert!(!is_loopback_origin(Some("http://127.0.0.1.evil.example")));
    }

    #[tokio::test]
    async fn serves_assets_and_pushes_live_board_updates_over_ws() {
        let source = Arc::new(FakeSource::new());
        let server = serve_loopback(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Assets::fallback_placeholder(),
            Arc::clone(&source) as Arc<dyn BoardSource>,
        )
        .await
        .expect("start");

        // HTTP static assets + security headers + health
        let index = http_get(server.addr(), "/").await;
        assert!(index.starts_with("HTTP/1.1 200 OK"), "{index}");
        assert!(index.contains("text/html"));
        assert!(index.contains("x-content-type-options: nosniff"));

        let js = http_get(server.addr(), "/web_todo.js").await;
        assert!(js.contains("application/javascript"));

        let wasm = http_get(server.addr(), "/web_todo_bg.wasm").await;
        assert!(wasm.contains("application/wasm"));

        let health = http_get(server.addr(), "/health").await;
        assert!(health.contains(r#"{"ok":true}"#));

        // Foreign Origin rejected on /ws
        let ws_url = format!("ws://{}/ws", server.addr());
        let mut bad_req = ws_url.clone().into_client_request().expect("req");
        bad_req.headers_mut().insert(
            "Origin",
            "https://evil.example".parse().expect("header"),
        );
        let rejected = tokio_tungstenite::connect_async(bad_req).await;
        assert!(rejected.is_err(), "foreign origin must be rejected");

        // Loopback WebSocket receives initial snapshot immediately and live push updates
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("ws connect");

        let first = ws.next().await.expect("first msg").expect("ok");
        let WsMessage::Text(text) = first else {
            panic!("expected text frame, got {first:?}");
        };
        let ServerMessage::Snapshot(snap1) = serde_json::from_str(&text).expect("decode") else {
            panic!("expected snapshot");
        };
        assert_eq!(snap1.items.len(), 1);
        assert_eq!(snap1.items[0].key, "PAY-1");

        // Trigger a live push (no polling!)
        source.push_item("PAY-2", "Live pushed item");

        let second = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
            .await
            .expect("push should arrive immediately")
            .expect("second msg")
            .expect("ok");
        let WsMessage::Text(text2) = second else {
            panic!("expected text frame");
        };
        let ServerMessage::Snapshot(snap2) = serde_json::from_str(&text2).expect("decode") else {
            panic!("expected snapshot");
        };
        assert_eq!(snap2.items.len(), 2);
        assert_eq!(snap2.items[1].key, "PAY-2");
        assert_eq!(snap2.revision, 2);

        server.stop().await;
    }

    #[test]
    fn controller_toggles_on_off_and_notifies_status_changes() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let source = Arc::new(FakeSource::new());
        let controller = WebTodoController::new(
            rt.handle().clone(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            Assets::fallback_placeholder(),
            source,
        );
        let transitions = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&transitions);
        controller.on_status_change(Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));

        assert_eq!(controller.status(), WebTodoStatus::Off);

        controller.set_enabled(true);
        let url = match controller.status() {
            WebTodoStatus::Listening { url } => url,
            other => panic!("expected Listening, got {other:?}"),
        };
        assert!(url.starts_with("http://127.0.0.1:"), "got {url}");
        assert_eq!(transitions.load(Ordering::Relaxed), 1);

        controller.set_enabled(false);
        assert_eq!(controller.status(), WebTodoStatus::Off);
        assert_eq!(transitions.load(Ordering::Relaxed), 2);
    }
}
