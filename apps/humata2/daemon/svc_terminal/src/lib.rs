//! TerminalService, RemoteTerminal, RunningHolders, and kernel terminal-session
//! detection (`humata2_svc_terminal`).

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_machines::humata::v1::*;
    pub use humata2_grpc_terminal::humata::v1::*;
}

pub mod agent_tabs;
pub mod ports;
pub mod remote_terminal;
pub mod running_holders;
pub mod terminal_session;

pub use self::agent_tabs::{AgentTabError, AgentTabLaunch, ShellTabEntry};
pub use self::ports::{
    AttachStream, LinkState, RelayError, RelayLinks, START_COLS, START_ROWS,
    TerminalLifecycle, TerminalSessions,
};
pub use self::remote_terminal::RemoteTerminal;
pub use self::running_holders::{RunningError, adopt, let_go};
pub use self::terminal_session::{
    Process, Processes, TerminalSession, TerminalSessionError, terminal_session,
};
