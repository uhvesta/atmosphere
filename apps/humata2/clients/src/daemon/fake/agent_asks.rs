use super::FakeDaemon;
use crate::daemon::{Operation, Reply};

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

pub(super) fn codex_needs_trust(daemon: &FakeDaemon) -> Reply<bool> {
    daemon.reply(None, "codex_needs_trust".to_owned(), |inner| {
        Ok(inner.codex_needs_trust)
    })
}
