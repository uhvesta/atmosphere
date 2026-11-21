use super::FakeDaemon;
use crate::daemon::Reply;
use crate::model::Agent;

pub(super) fn set_default_agent(daemon: &FakeDaemon, agent: Agent) -> Reply<()> {
    daemon.reply(None, format!("set_default_agent {}", agent.command()), move |inner| {
        inner.full_settings.default_agent.agent = agent.command().to_owned();
        Ok(())
    })
}

pub(super) fn codex_needs_trust(daemon: &FakeDaemon) -> Reply<bool> {
    daemon.reply(None, "codex_needs_trust".to_owned(), |inner| {
        Ok(inner.codex_needs_trust)
    })
}
