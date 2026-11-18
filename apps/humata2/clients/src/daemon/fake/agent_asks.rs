use super::FakeDaemon;
use crate::daemon::{Operation, Reply};
use crate::model::Agent;

pub(super) fn asks_in_inbox(daemon: &FakeDaemon) -> Reply<bool> {
    daemon.reply(None, "asks_in_inbox".to_owned(), |inner| {
        Ok(inner.asks_in_inbox)
    })
}

pub(super) fn set_asks_in_inbox(daemon: &FakeDaemon, inbox: bool) -> Reply<bool> {
    daemon.reply(
        Some(Operation::SetAsksInInbox),
        format!("set_asks_in_inbox {inbox}"),
        move |inner| {
            inner.asks_in_inbox = inbox;
            Ok(inner.asks_in_inbox)
        },
    )
}

pub(super) fn set_default_agent(daemon: &FakeDaemon, agent: Agent) -> Reply<()> {
    daemon.reply(None, format!("set_default_agent {}", agent.command()), move |_| Ok(()))
}

pub(super) fn codex_needs_trust(daemon: &FakeDaemon) -> Reply<bool> {
    daemon.reply(None, "codex_needs_trust".to_owned(), |inner| {
        Ok(inner.codex_needs_trust)
    })
}
