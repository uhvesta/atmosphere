//! Browser entry point for Humata's To-do UI (WEBTODO, D905).
//!
//! GPUI renders the real To-do UI on a browser canvas; JS only adapts the
//! browser `WebSocket` API with push-driven callbacks (zero polling) and
//! automatic reconnect across daemon restarts.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{prelude::*, *};
use humata2_web_todo_wire::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use wasm_bindgen::prelude::*;

mod view;

use view::{ConnectionState, WebTodoView};

#[wasm_bindgen(inline_js = r#"
let socket = null;
let reconnectTimer = 0;
let reconnectDelay = 200;
let stopped = false;
let currentUrl = '';
let wakeup = null;
let queue = [];

function notify() {
  if (wakeup) {
    try { wakeup(); } catch (e) { console.error('web_todo wakeup failed', e); }
  }
}

function scheduleReconnect() {
  if (stopped || reconnectTimer) return;
  const delay = reconnectDelay;
  reconnectDelay = Math.min(reconnectDelay * 2, 3000);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = 0;
    if (!stopped) openSocket();
  }, delay);
}

function openSocket() {
  if (stopped) return;
  try {
    const ws = new WebSocket(currentUrl);
    socket = ws;
    ws.onopen = () => {
      if (socket !== ws || stopped) return;
      reconnectDelay = 200;
      queue.push({ kind: 'opened' });
      try { ws.send(JSON.stringify({ kind: 'hello', protocol: 1 })); } catch (_) {}
      notify();
    };
    ws.onmessage = (event) => {
      if (socket !== ws || stopped) return;
      if (typeof event.data === 'string') {
        queue.push({ kind: 'text', text: event.data });
        notify();
      }
    };
    ws.onclose = (event) => {
      if (socket !== ws) return;
      socket = null;
      if (!stopped) {
        queue.push({ kind: 'closed', reason: event.reason || `closed (${event.code})` });
        notify();
        scheduleReconnect();
      }
    };
    ws.onerror = () => {
      if (socket !== ws || stopped) return;
      queue.push({ kind: 'closed', reason: 'connection error' });
      notify();
    };
  } catch (error) {
    queue.push({ kind: 'closed', reason: String(error) });
    notify();
    scheduleReconnect();
  }
}

export function web_todo_panic(message) { console.error(message); }

export function web_todo_ws_url() {
  const query = new URL(location.href).searchParams;
  return query.get('ws') || `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/ws`;
}

export function web_todo_backend() {
  return new URL(location.href).searchParams.get('backend') || 'auto';
}

export function web_todo_location_path() {
  return `${location.pathname || '/'}${location.search || ''}`;
}

export function web_todo_push_route(path) {
  try {
    const cur = `${location.pathname || '/'}${location.search || ''}`;
    if (cur !== path) {
      history.pushState(null, '', path);
    }
  } catch (_) {}
}

export function web_todo_send_ws(payload) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    try {
      socket.send(payload);
      return true;
    } catch (_) {}
  }
  return false;
}

export function web_todo_connect(url, onEvent) {
  stopped = false;
  currentUrl = url;
  wakeup = onEvent;
  reconnectDelay = 200;
  queue = [];
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = 0; }
  openSocket();
}

export function web_todo_drain() {
  if (queue.length === 0) return '[]';
  const out = queue;
  queue = [];
  return JSON.stringify(out);
}

export function web_todo_disconnect() {
  stopped = true;
  wakeup = null;
  queue = [];
  if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = 0; }
  if (socket) {
    const ws = socket;
    socket = null;
    ws.onopen = null;
    ws.onmessage = null;
    ws.onclose = null;
    ws.onerror = null;
    try { ws.close(); } catch (_) {}
  }
}
"#)]
extern "C" {
    fn web_todo_panic(message: &str);
    fn web_todo_ws_url() -> String;
    fn web_todo_backend() -> String;
    fn web_todo_location_path() -> String;
    fn web_todo_push_route(path: &str);
    fn web_todo_send_ws(payload: &str) -> bool;
    fn web_todo_connect(url: &str, on_event: &JsClosure);
    fn web_todo_drain() -> String;
    fn web_todo_disconnect();
}

pub(crate) fn current_location_path() -> String {
    web_todo_location_path()
}

pub(crate) fn push_browser_route(path: &str) {
    web_todo_push_route(path);
}

pub(crate) fn send_client_message(msg: &ClientMessage) -> bool {
    match serde_json::to_string(msg) {
        Ok(json) => web_todo_send_ws(&json),
        Err(_) => false,
    }
}

type JsClosure = Closure<dyn FnMut()>;

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BrowserWsEvent {
    Opened,
    Text { text: String },
    Closed { reason: String },
}

pub(crate) struct Assets;
impl AssetSource for Assets {
    fn load(&self, _path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(None)
    }
    fn list(&self, _path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

fn resources() -> humata2_engine::Resources {
    humata2_engine::Resources::new(Assets)
        .font(include_bytes!(
            "../../../terminal/client/assets/Inter-Regular.ttf"
        ))
        .font(include_bytes!(
            "../../../terminal/client/assets/JetBrainsMono-Regular.ttf"
        ))
}

thread_local! {
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
    static VIEW: RefCell<Option<WeakEntity<WebTodoView>>> = const { RefCell::new(None) };
    static WAKEUP: RefCell<Option<Closure<dyn FnMut()>>> = const { RefCell::new(None) };
    static DIAGNOSTICS: Rc<RefCell<String>> = Rc::new(RefCell::new(
        r#"{"phase":"starting","connected":false,"item_count":0}"#.to_owned()
    ));
}

/// Read-only diagnostics for headless browser acceptance tests.
#[wasm_bindgen]
pub fn inspect() -> String {
    DIAGNOSTICS.with(|d| d.borrow().clone())
}

/// Releases graphics, event loops, and the WebSocket connection cleanly.
#[wasm_bindgen]
pub fn shutdown() -> bool {
    web_todo_disconnect();
    WAKEUP.with(|slot| {
        slot.borrow_mut().take();
    });
    VIEW.with(|slot| {
        slot.borrow_mut().take();
    });
    let app = APPLICATION.with(|slot| slot.borrow_mut().take());
    let existed = app.is_some();
    if let Some(app) = app {
        app.update(|cx| cx.shutdown());
        drop(app);
    }
    DIAGNOSTICS.with(|d| {
        *d.borrow_mut() = r#"{"phase":"stopped","connected":false,"item_count":0}"#.to_owned();
    });
    existed
}

fn process_ws_events() {
    let raw = web_todo_drain();
    let events: Vec<BrowserWsEvent> = match serde_json::from_str(&raw) {
        Ok(events) => events,
        Err(error) => {
            log::error!("web_todo: failed to decode ws events: {error}");
            return;
        }
    };
    if events.is_empty() {
        return;
    }

    let weak = VIEW.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    APPLICATION.with(|slot| {
        let borrow = slot.borrow();
        let Some(application) = borrow.as_ref() else {
            return;
        };
        application.update(move |cx| {
            let _ = weak.update(cx, |view, cx| {
                for event in events {
                    match event {
                        BrowserWsEvent::Opened => {
                            view.set_connection(ConnectionState::Live, cx);
                        }
                        BrowserWsEvent::Closed { reason } => {
                            view.set_connection(ConnectionState::Reconnecting(reason), cx);
                        }
                        BrowserWsEvent::Text { text } => {
                            match serde_json::from_str::<ServerMessage>(&text) {
                                Ok(ServerMessage::Snapshot(snapshot)) => {
                                    view.apply_snapshot(snapshot, cx);
                                }
                                Ok(ServerMessage::Error { message }) => {
                                    view.set_error(message, cx);
                                }
                                Err(error) => {
                                    view.set_error(format!("Invalid board message: {error}"), cx);
                                }
                            }
                        }
                    }
                }
            });
        });
    });
}

/// Starts the GPUI Web To-do UI and connects to the host daemon's push `/ws`.
#[wasm_bindgen]
pub fn run() {
    use gpui_kit::web::{CanvasFontFallback, WebBackendPreference, WebPlatform};
    let _ = ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
    };

    std::panic::set_hook(Box::new(|panic| web_todo_panic(&panic.to_string())));
    gpui_kit::platform::web_init();
    log::set_max_level(log::LevelFilter::Info);

    let backend = match web_todo_backend().as_str() {
        "webgpu" => WebBackendPreference::WebGpu,
        "webgl" => WebBackendPreference::WebGl,
        _ => WebBackendPreference::Auto,
    };

    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        backend,
        CanvasFontFallback::EmojiAndCjk,
    ));
    let application = Application::with_platform(platform.clone())
        .with_http_client(std::sync::Arc::new(platform.fetch_http_client()))
        .with_assets(resources().asset_source());

    let diagnostics = DIAGNOSTICS.with(Rc::clone);

    APPLICATION.with(|slot| {
        *slot.borrow_mut() = Some(application.run_embedded(move |cx| {
            let mut startup_theme = gpui_kit::component::theme::Theme::default();
            startup_theme.font_family = "Inter".into();
            startup_theme.mono_font_family = "JetBrains Mono".into();
            cx.set_global(startup_theme);

            let res = resources();
            if let Err(error) = humata2_engine::init(cx, &res) {
                *diagnostics.borrow_mut() =
                    serde_json::json!({"phase": "failed", "error": format!("GPUI init failed: {error}")})
                        .to_string();
                return;
            }
            let registry = gpui_kit::component::theme::ThemeRegistry::global(cx);
            let light = registry.default_light_theme().clone();
            let dark = registry.default_dark_theme().clone();
            let theme = gpui_kit::component::theme::Theme::global_mut(cx);
            theme.light_theme = light;
            theme.dark_theme = dark;
            gpui_kit::component::theme::Theme::change(
                gpui_kit::component::theme::ThemeMode::Dark,
                None,
                cx,
            );

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                titlebar: None,
                ..Default::default()
            };
            let diag = Rc::clone(&diagnostics);
            match gpui_kit::open_window(options, cx, move |_window, cx| {
                let entity = cx.new(|_cx| WebTodoView::new(diag));
                VIEW.with(|v| *v.borrow_mut() = Some(entity.downgrade()));
                entity
            }) {
                Ok(_) => {
                    let wakeup = Closure::<dyn FnMut()>::new(process_ws_events);
                    let url = web_todo_ws_url();
                    web_todo_connect(&url, &wakeup);
                    WAKEUP.with(|w| *w.borrow_mut() = Some(wakeup));
                }
                Err(error) => {
                    *diagnostics.borrow_mut() = serde_json::json!({
                        "phase": "failed",
                        "error": format!("Could not open GPUI browser window: {error}")
                    })
                    .to_string();
                }
            }
        }));
    });
}
