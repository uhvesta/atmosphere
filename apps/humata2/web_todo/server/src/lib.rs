//! Host daemon loopback HTTP + WebSocket server for Humata's To-do UI (WEBTODO, D905).
//!
//! - Binds loopback (`127.0.0.1` / `::1`) only; refuses non-loopback addresses.
//! - Serves the GPUI Wasm bundle (`/`, `/web_todo.js`, `/web_todo_bg.wasm`) and `/health`.
//! - Pushes [`humata2_web_todo_wire::ServerMessage::Snapshot`] over `/ws` on connect
//!   and on every change notification (push, never poll).
//! - Managed by [`WebTodoController`] so `humatad2` can turn it on/off live from
//!   Settings and restore it on daemon restart.

mod controller;
mod server;

pub use controller::{DEFAULT_PORT, WebTodoController, WebTodoStatus};
pub use server::{Assets, BoardSource, RunningServer, ServerError, serve_loopback};
