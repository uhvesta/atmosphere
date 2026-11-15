//! Humata 2's MCP server (api/MCP.md).
//!
//! Agents reach Humata only through MCP. This crate speaks the protocol on
//! stdio: `initialize`, `tools/list` with each tool's schema, `tools/call`,
//! progress while a call waits for the user, and cancellation. Every tool is
//! one call on the daemon, behind the [`Daemon`] trait; the daemon makes the
//! capability check (humata2_access) and the MCP server passes its answer
//! on. It never widens what the daemon allowed, and decides nothing itself
//! except that user-only operations have no tool (MCP.md §7).
//!
//! humatad implements [`Daemon`] over gRPC on its socket and runs
//! [`Server::serve_stdio`] as `humatad mcp`, with the session token from
//! [`SessionToken::from_env`].

mod daemon;
mod error;
mod jsonrpc;
mod outcome;
mod schema;
mod server;
mod session;
mod time;
mod tools;
mod user_only;

pub use daemon::{
    Call, Daemon, ErrorKind, IdempotencyKey, JournalPosition, Reply, RequestId, Retry, ToolError,
    WaitEvent,
};
pub use error::{ServeError, TokenError};
pub use server::Server;
pub use session::{SessionToken, TOKEN_VAR};
pub use tools::Operation;
