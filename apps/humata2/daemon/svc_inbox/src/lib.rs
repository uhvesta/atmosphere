//! `humata2_svc_inbox`: the daemon's Inbox service and screen-question watcher
//! (`inbox/`, `screen_ask.rs`), decoupled from `humatad` via [`ports`].

pub mod inbox;
pub mod ports;
pub mod screen_ask;

pub use inbox::{
    BoardAsk, Deferred, Inbox, MoveSwitchError, Registered, Relays, Run,
    permission::{self, Decision, Question, codex_trust},
};
pub use ports::{
    AgentGatePort, AskingGuardPort, EventScope, InboxFeedPort, InboxGuardPort,
    MigrateApproverPort, NoopAsking, ScreenHoldersPort,
};
pub use screen_ask::ScreenAsk;
