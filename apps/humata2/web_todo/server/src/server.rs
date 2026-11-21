//! Axum loopback HTTP + WebSocket server for the To-do web UI (WEBTODO, D905).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use humata2_web_todo_wire::{BoardSnapshot, ClientMessage, ServerMessage};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};

/// Static assets served for the GPUI Wasm To-do application, including
/// `wasm-bindgen` inline JS `snippets/` modules.
#[derive(Clone, Debug)]
pub struct Assets {
    pub index_html: Arc<[u8]>,
    pub js: Arc<[u8]>,
    pub wasm: Arc<[u8]>,
    pub extra: Arc<HashMap<String, (Arc<[u8]>, &'static str)>>,
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut HashMap<String, (Arc<[u8]>, &'static str)>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            let key = rel.to_string_lossy().replace('\\', "/");
            let ct = content_type_for(&key);
            let bytes: Arc<[u8]> = std::fs::read(&path)?.into();
            out.insert(key, (bytes, ct));
        }
    }
    Ok(())
}

impl Assets {
    /// Loads `index.html`, `web_todo.js`, `web_todo_bg.wasm`, and any nested
    /// `snippets/` files from a built `wasm_bundle` output directory.
    pub fn from_dir(dir: &Path) -> Result<Self, std::io::Error> {
        let index_html = std::fs::read(dir.join("index.html"))?.into();
        let js = std::fs::read(dir.join("web_todo.js"))?.into();
        let wasm = std::fs::read(dir.join("web_todo_bg.wasm"))?.into();
        let mut extra = HashMap::new();
        collect_files(dir, dir, &mut extra)?;
        Ok(Self {
            index_html,
            js,
            wasm,
            extra: Arc::new(extra),
        })
    }

    /// Minimal fallback assets used when a bundle directory is not present on
    /// disk (e.g., unit tests that only exercise the HTTP/WS server layer).
    #[must_use]
    pub fn fallback_placeholder() -> Self {
        let html = br#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Humata To-do</title></head><body><div id="loading">Humata To-do Wasm bundle not installed alongside humatad2.</div></body></html>"#;
        Self {
            index_html: Arc::from(html.as_slice()),
            js: Arc::from(b"export default async function(){};export function run(){};export function shutdown(){return false};export function inspect(){return '{}'};".as_slice()),
            wasm: Arc::from(b"\0asm\x01\0\0\0".as_slice()),
            extra: Arc::new(HashMap::new()),
        }
    }
}

/// Supplies live To-do snapshots and push notifications to the web server.
pub trait BoardSource: Send + Sync + 'static {
    /// Returns the current To-do board snapshot.
    fn snapshot(&self) -> BoardSnapshot;
    /// Subscribes to push notifications fired whenever the board changes.
    fn subscribe(&self) -> broadcast::Receiver<()>;
    /// Watches a remote GitHub PR URL in the Reviews inbox.
    fn watch_pr_url(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }
    /// Triggers an immediate refresh of GitHub reviews.
    fn refresh_reviews(&self) {}
}

/// Errors from starting the loopback web server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("web To-do server only binds loopback addresses, refused {addr}")]
    NonLoopback { addr: SocketAddr },
    #[error("couldn't bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Clone)]
struct AppState {
    assets: Assets,
    source: Arc<dyn BoardSource>,
}

/// Handle to a running loopback To-do web server. Dropping or calling
/// [`Self::stop`] shuts the listener and active WebSockets down cleanly.
pub struct RunningServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl RunningServer {
    /// Bound loopback socket address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Local HTTP URL (`http://127.0.0.1:<port>/` or `http://[::1]:<port>/`).
    #[must_use]
    pub fn url(&self) -> String {
        if self.addr.is_ipv6() {
            format!("http://[{}]:{}/", self.addr.ip(), self.addr.port())
        } else {
            format!("http://{}:{}/", self.addr.ip(), self.addr.port())
        }
    }

    /// Signals graceful shutdown and waits for the server task to finish.
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// Triggers shutdown without awaiting completion (for synchronous drop /
    /// controller stop paths).
    pub(crate) fn stop_now(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop_now();
    }
}

/// Starts the To-do web server bound strictly to `addr` (must be loopback).
pub async fn serve_loopback(
    addr: SocketAddr,
    assets: Assets,
    source: Arc<dyn BoardSource>,
) -> Result<RunningServer, ServerError> {
    if !addr.ip().is_loopback() {
        return Err(ServerError::NonLoopback { addr });
    }
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| ServerError::Bind { addr, source })?;
    let bound = listener
        .local_addr()
        .map_err(|source| ServerError::Bind { addr, source })?;

    let state = AppState { assets, source };
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/todo", get(index_handler))
        .route("/reviews", get(index_handler))
        .route("/reviews/{id}", get(index_handler))
        .route("/web_todo.js", get(js_handler))
        .route("/web_todo_bg.wasm", get(wasm_handler))
        .route("/health", get(health_handler))
        .route("/ws", get(ws_handler))
        .route("/{*path}", get(extra_asset_handler))
        .with_state(state);

    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });

    Ok(RunningServer {
        addr: bound,
        shutdown: Some(tx),
        task: Some(task),
    })
}

fn asset_response(body: Arc<[u8]>, content_type: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
        ],
        body.to_vec(),
    )
        .into_response()
}

async fn index_handler(State(state): State<AppState>) -> Response {
    asset_response(state.assets.index_html, "text/html; charset=utf-8")
}

async fn js_handler(State(state): State<AppState>) -> Response {
    asset_response(state.assets.js, "application/javascript; charset=utf-8")
}

async fn wasm_handler(State(state): State<AppState>) -> Response {
    asset_response(state.assets.wasm, "application/wasm")
}

async fn extra_asset_handler(
    AxumPath(path): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    if path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let Some((body, content_type)) = state.assets.extra.get(&path) {
        asset_response(Arc::clone(body), content_type)
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn health_handler() -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        r#"{"ok":true}"#,
    )
        .into_response()
}

/// Checks whether a WebSocket `Origin` header is allowed: absent (non-browser
/// client) or a loopback HTTP/HTTPS origin (`127.0.0.1`, `localhost`, `[::1]`).
pub(crate) fn is_loopback_origin(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(host_port) = rest else {
        return false;
    };
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

async fn ws_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !is_loopback_origin(origin) {
        return (StatusCode::FORBIDDEN, "cross-origin WebSocket refused").into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state.source))
}

async fn handle_socket(socket: WebSocket, source: Arc<dyn BoardSource>) {
    let (mut sender, mut receiver) = socket.split();
    // Subscribe BEFORE taking the initial snapshot so no event between snapshot
    // and live forwarding can be missed.
    let mut updates = source.subscribe();

    if send_snapshot(&mut sender, &source).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            changed = updates.recv() => {
                match changed {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        if send_snapshot(&mut sender, &source).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Ping) => {
                                if send_snapshot(&mut sender, &source).await.is_err() {
                                    break;
                                }
                            }
                            Ok(ClientMessage::WatchPrUrl { url }) => {
                                match source.watch_pr_url(&url) {
                                    Ok(()) => {
                                        if send_snapshot(&mut sender, &source).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(message) => {
                                        let payload = serde_json::to_string(&ServerMessage::Error {
                                            message,
                                        })
                                        .unwrap_or_else(|_| {
                                            r#"{"kind":"error","message":"watch_pr_url failed"}"#
                                                .to_owned()
                                        });
                                        if sender.send(Message::Text(payload.into())).await.is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                            }
                            Ok(ClientMessage::RefreshReviews) => {
                                source.refresh_reviews();
                                if send_snapshot(&mut sender, &source).await.is_err() {
                                    break;
                                }
                            }
                            Ok(ClientMessage::Hello { .. }) | Err(_) => {}
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

async fn send_snapshot(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    source: &Arc<dyn BoardSource>,
) -> Result<(), axum::Error> {
    let snapshot = source.snapshot();
    let payload = serde_json::to_string(&ServerMessage::Snapshot(snapshot))
        .unwrap_or_else(|_| r#"{"kind":"error","message":"failed to encode snapshot"}"#.to_owned());
    sender.send(Message::Text(payload.into())).await
}
