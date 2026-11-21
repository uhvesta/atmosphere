//! `humata2_svc_inbox`: the daemon's Inbox service and screen-question watcher
//! (`inbox/`, `screen_ask.rs`), decoupled from `humatad` via [`ports`].

#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_events::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
}

pub mod inbox;
pub mod ports;
pub mod screen_ask;

pub use inbox::{
    BoardAsk, Deferred, Inbox, MoveSwitchError, Registered, Relays, Run,
    permission::{self, Decision, Question, codex_trust},
};
pub use ports::{
    AgentGatePort, AskingGuardPort, EventScope, HookAskPort, InboxFeedPort, InboxGuardPort,
    MigrateApproverPort, NoopAsking, NoopHookAsk, ScreenHoldersPort,
};
pub use screen_ask::ScreenAsk;
