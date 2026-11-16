//! Browser entry point: GPUI renders the actual terminal; JS only adapts WebSocket APIs.
use gpui_kit::{prelude::*, *};
use std::{cell::RefCell, rc::Rc};
use terminal_client::{Options, RemoteWorkspace, SendError, Transport, TransportEvent, resources};
use wasm_bindgen::prelude::*;
mod lifecycle;

#[wasm_bindgen(inline_js = r#"
let nextSocket = 1;
const sockets = new Map();
// Retained UTF-16 string storage plus conservative object/array overhead.
// Legitimate PTY bursts may contain thousands of tiny frames inside one ACK window.
export class IncomingQueue {
  constructor(limit=64*1024*1024) { this.limit=limit;this.items=[];this.cost=0; }
  push(event) {
    const cost=512+2*(event.kind==='text'?event.text.length:(event.reason?.length||0));
    if(this.cost+cost>this.limit)return false;
    this.items.push({event,cost});this.cost+=cost;return true;
  }
  drain(max=256) {
    const batch=this.items.splice(0,max);
    for(const item of batch)this.cost-=item.cost;
    return batch.map(item=>item.event);
  }
  clear() {this.items=[];this.cost=0;}
}
export function terminal_panic(message) { console.error(message); }
export function terminal_configuration() {
  const query = new URL(location.href).searchParams;
  const fragment = new URLSearchParams(location.hash.replace(/^#/, ''));
  return JSON.stringify({url: query.get('ws') || `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/ws`, token: fragment.get('token') || '', backend:query.get('backend') || 'auto',memory_saver:query.get('memory')==='1'});
}
function connect(state) {
  try {
    const socket = new WebSocket(state.url);
    state.socket = socket;
    socket.onopen = () => { if (state.socket === socket) state.events.push({kind:'opened'}); };
    socket.onmessage = event => {
      if (state.socket !== socket || state.failed) return;
      if (typeof event.data !== 'string' || event.data.length > 32*1024*1024 || !state.events.push({kind:'text',text:event.data})) {
        state.failed=true;state.events.clear();
        state.events.push({kind:'failed', reason:'The viewer output queue exceeded its memory budget; reconnect for a fresh snapshot. The host process continues running.'});
        socket.close(4009, 'viewer output queue limit');
        return;
      }
    };
    socket.onclose = event => { if (state.socket === socket) state.events.push({kind:'closed', reason:event.reason || `WebSocket closed (${event.code})`}); };
    socket.onerror = () => { if (state.socket === socket) state.events.push({kind:'failed', reason:'The WebSocket connection failed. Check the host address, TLS and network.'}); };
  } catch (error) { state.events.push({kind:'failed', reason:String(error)}); }
}
export function terminal_socket_open(url) {
  const id = nextSocket++;
  const state = {url, socket:null, events:new IncomingQueue(), failed:false};
  sockets.set(id,state); connect(state); return id;
}
export function terminal_socket_drain(id) {
  const state=sockets.get(id); if(!state) return '[]';
  return JSON.stringify(state.events.drain(256));
}
export function terminal_socket_send(id,text) {
  const state=sockets.get(id); if(!state || !state.socket || state.socket.readyState!==WebSocket.OPEN) return 1;
  if(state.socket.bufferedAmount>1024*1024) return 2;
  state.socket.send(text); return 0;
}
export function terminal_socket_idle(id) {
  const state=sockets.get(id);return !!state?.socket && state.socket.readyState===WebSocket.OPEN && state.socket.bufferedAmount===0;
}
export function terminal_socket_close_all() {
  for(const id of Array.from(sockets.keys()))terminal_socket_close(id);
}
export function terminal_socket_close(id) {
  const state=sockets.get(id); if(!state) return;
  const socket=state.socket;state.socket=null;state.events.clear();
  if(socket){socket.onopen=null;socket.onmessage=null;socket.onclose=null;socket.onerror=null;socket.close();}
  sockets.delete(id);
}
"#)]
extern "C" {
    fn terminal_configuration() -> String;
    fn terminal_panic(message: &str);
    fn terminal_socket_open(url: &str) -> u32;
    fn terminal_socket_drain(id: u32) -> String;
    fn terminal_socket_send(id: u32, text: &str) -> u32;
    fn terminal_socket_idle(id: u32) -> bool;
    fn terminal_socket_close(id: u32);
    fn terminal_socket_close_all();
}
#[derive(serde::Deserialize)]
struct Configuration {
    url: String,
    token: String,
    #[serde(default)]
    backend: Backend,
    #[serde(default)]
    memory_saver: bool,
}
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Backend {
    #[default]
    Auto,
    Webgpu,
    Webgl,
}
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BrowserEvent {
    Opened,
    Text { text: String },
    Closed { reason: String },
    Failed { reason: String },
}
// The first ready application pump starts networking, after window/GPU setup.
struct BrowserSocket {
    url: String,
    id: Option<u32>,
}
impl Drop for BrowserSocket {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            terminal_socket_close(id);
        }
    }
}
impl Transport for BrowserSocket {
    fn drain(&mut self) -> Vec<TransportEvent> {
        let id = match self.id {
            Some(id) => id,
            None => {
                let id = terminal_socket_open(&self.url);
                self.id = Some(id);
                id
            }
        };
        match serde_json::from_str::<Vec<BrowserEvent>>(&terminal_socket_drain(id)) {
            Ok(events) => events
                .into_iter()
                .map(|event| match event {
                    BrowserEvent::Opened => TransportEvent::Opened,
                    BrowserEvent::Text { text } => TransportEvent::Text(text),
                    BrowserEvent::Closed { reason } => TransportEvent::Closed { reason },
                    BrowserEvent::Failed { reason } => TransportEvent::Failed {
                        operation: "use the browser connection",
                        reason,
                    },
                })
                .collect(),
            Err(error) => vec![TransportEvent::Failed {
                operation: "decode browser connection events",
                reason: error.to_string(),
            }],
        }
    }
    fn send(&mut self, text: String) -> Result<(), SendError> {
        let Some(id) = self.id else {
            return Err(SendError::Closed);
        };
        match terminal_socket_send(id, &text) {
            0 => Ok(()),
            2 => Err(SendError::Full),
            _ => Err(SendError::Closed),
        }
    }
    fn reconnect(&mut self) {
        if let Some(id) = self.id.take() {
            terminal_socket_close(id);
        }
    }
    fn is_idle(&self) -> bool {
        self.id.is_some_and(terminal_socket_idle)
    }
}
thread_local! {
    static APPLICATION:RefCell<Option<ApplicationHandle>>=const{RefCell::new(None)};
    static LIFECYCLE:RefCell<lifecycle::Lifecycle>=RefCell::new(lifecycle::Lifecycle::default());
    static DIAGNOSTICS:Rc<RefCell<String>>=Rc::new(RefCell::new("{\"connected\":false,\"phase\":\"starting\"}".into()));
}
/// Read-only diagnostics for headless cross-platform acceptance. Rendering remains GPUI.
#[wasm_bindgen]
pub fn inspect() -> String {
    DIAGNOSTICS.with(|d| d.borrow().clone())
}
/// Releases this view's graphics, event loops and socket; host processes continue.
#[wasm_bindgen]
pub fn shutdown() -> bool {
    LIFECYCLE.with(|lifecycle| lifecycle.borrow_mut().stop());
    terminal_socket_close_all();
    let application = APPLICATION.with(|slot| slot.borrow_mut().take());
    let existed = application.is_some();
    if let Some(application) = application {
        // Public embedded-app lifecycle API; unlike WebPlatform::quit this
        // clears windows and runs WebWindow's renderer/RAF/observer cleanup.
        application.update(|cx| cx.shutdown());
        drop(application);
    }
    DIAGNOSTICS.with(|d| *d.borrow_mut() = "{\"connected\":false,\"phase\":\"stopped\"}".into());
    existed
}
#[wasm_bindgen]
pub fn run() {
    let Some(launch) = LIFECYCLE.with(|lifecycle| lifecycle.borrow_mut().start()) else {
        return;
    };
    use gpui_kit::web::{CanvasFontFallback, WebBackendPreference, WebPlatform};
    std::panic::set_hook(Box::new(|panic| terminal_panic(&panic.to_string())));
    gpui_kit::platform::web_init();
    // Application logging policy: keep warnings/errors without shader debug spam.
    log::set_max_level(log::LevelFilter::Info);
    let config = match serde_json::from_str::<Configuration>(&terminal_configuration()) {
        Ok(c) => c,
        Err(error) => {
            DIAGNOSTICS.with(|d| {
                *d.borrow_mut() =
                    serde_json::json!({"error":format!("Invalid viewer configuration: {error}")})
                        .to_string()
            });
            return;
        }
    };
    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        match config.backend {
            Backend::Auto => WebBackendPreference::Auto,
            Backend::Webgpu => WebBackendPreference::WebGpu,
            Backend::Webgl => WebBackendPreference::WebGl,
        },
        CanvasFontFallback::EmojiAndCjk,
    ));
    let application = Application::with_platform(platform.clone())
        .with_http_client(std::sync::Arc::new(platform.fetch_http_client()))
        .with_assets(resources().asset_source());
    APPLICATION.with(|slot| {
        *slot.borrow_mut() = Some(application.run_embedded(move |cx| {
            if !launch.is_current() {return;}
            // Browser font databases have no system family. Select the bundled
            // families before component initialization probes its defaults.
            let mut startup_theme = gpui_kit::component::theme::Theme::default();
            startup_theme.font_family = "Inter".into();
            startup_theme.mono_font_family = "JetBrains Mono".into();
            cx.set_global(startup_theme);
            let resources = resources();
            if let Err(error) = humata2_engine::init(cx, &resources) {
                DIAGNOSTICS.with(|d| *d.borrow_mut() = serde_json::json!({"error":format!("Could not initialize GPUI: {error}")}).to_string());
                return;
            }
            let registry = gpui_kit::component::theme::ThemeRegistry::global(cx);
            let light = registry.default_light_theme().clone();
            let dark = registry.default_dark_theme().clone();
            let theme = gpui_kit::component::theme::Theme::global_mut(cx);
            theme.light_theme = light;
            theme.dark_theme = dark;
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(800.)), cx))),
                titlebar: None,
                ..Default::default()
            };
            let diagnostics = DIAGNOSTICS.with(Clone::clone);
            let socket = BrowserSocket{url:config.url.clone(),id:None};
            if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| {let mut workspace=RemoteWorkspace::new(socket, Options {
                token: config.token,
                title: "Humata · Browser terminal".into(),
                diagnostics: Some(diagnostics),
            }, window, cx);if config.memory_saver {workspace.set_memory_saver(true,std::time::Duration::from_secs(30),cx);}workspace})) {
                DIAGNOSTICS.with(|d| *d.borrow_mut() = serde_json::json!({"error":format!("Could not open the GPUI browser window: {error}")}).to_string());
            }
        }));
    });
}
