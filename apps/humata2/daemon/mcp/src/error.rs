//! What can stop the server itself. Refusals of single calls are
//! [`crate::ToolError`]s, sent to the agent as tool results.

use std::io;

use thiserror::Error;

/// The session token couldn't be read (MCP.md §1).
///
/// The server still serves: `tools/list` works, and every call fails with
/// `identity_mismatch` carrying this message, so the agent can tell the
/// user.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TokenError {
    #[error(
        "{var} is not set, so this MCP server can't tell which session it works for. Humata sets it when it starts the agent: start the agent from Humata instead of by hand. Don't copy a token from another session; the daemon refuses it."
    )]
    Missing { var: &'static str },
    #[error(
        "{var} is empty, so this MCP server can't tell which session it works for. Humata sets it when it starts the agent: restart the session from Humata to get a new token."
    )]
    Empty { var: &'static str },
    #[error(
        "{var} is not valid text, so it isn't a token Humata made. Restart the session from Humata to get a new token."
    )]
    NotUnicode { var: &'static str },
}

/// The server stopped because its input or output failed.
#[derive(Debug, Error)]
pub enum ServeError {
    #[error(
        "couldn't read MCP messages from the agent: {source}. The agent's MCP client has likely exited; Humata starts a new server when the agent is next launched or resumed."
    )]
    Read {
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't write MCP replies to the agent: {source}. The agent's MCP client has likely exited; requests it was waiting on that never returned pending were withdrawn."
    )]
    Write {
        #[source]
        source: io::Error,
    },
}
