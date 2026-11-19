//! TerminalService, RemoteTerminal, RunningHolders, and kernel terminal-session
//! detection (`humata2_svc_terminal`).

pub mod ports;
pub mod remote_terminal;
pub mod running_holders;
pub mod terminal_session;

pub use self::ports::{
    AttachStream, LinkState, RelayError, RelayLinks, START_COLS, START_ROWS,
    TerminalLifecycle, TerminalSessions,
};
pub use self::remote_terminal::RemoteTerminal;
pub use self::running_holders::{RunningError, adopt, let_go};
pub use self::terminal_session::{
    Process, Processes, TerminalSession, TerminalSessionError, terminal_session,
};
