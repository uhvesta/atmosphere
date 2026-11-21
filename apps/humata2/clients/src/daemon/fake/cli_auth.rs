//! Fake `CliAuthService` (`humata login` approvals, active CLI sessions, and script tokens).

use super::FakeDaemon;
use crate::daemon::{
    CliSession, DaemonError, Event, Operation, PendingCliLogin, Reply, ScriptToken,
};

impl FakeDaemon {
    /// Adds or updates a pending `humata login` and publishes `Event::PendingCliLogin`.
    pub fn push_pending_cli_login(&self, login: PendingCliLogin) {
        let mut inner = self.inner();
        if let Some(existing) = inner
            .pending_cli_logins
            .iter_mut()
            .find(|item| item.id == login.id)
        {
            *existing = login.clone();
        } else {
            inner.pending_cli_logins.push(login.clone());
        }
        inner.emit(Event::PendingCliLogin(login));
    }

    /// Removes a pending `humata login` (as when the terminal stops waiting) and
    /// publishes `Event::PendingCliLoginGone`.
    pub fn withdraw_pending_cli_login(&self, id: &str) {
        let mut inner = self.inner();
        let before = inner.pending_cli_logins.len();
        inner.pending_cli_logins.retain(|item| item.id != id);
        if inner.pending_cli_logins.len() != before {
            inner.emit(Event::PendingCliLoginGone(id.to_owned()));
        }
    }

    /// Pending CLI logins currently held by the fake daemon.
    #[must_use]
    pub fn pending_cli_logins(&self) -> Vec<PendingCliLogin> {
        self.inner().pending_cli_logins.clone()
    }

    /// Every `(id, allow)` answer sent to `answer_cli_login`, in order.
    #[must_use]
    pub fn cli_login_answers(&self) -> Vec<(String, bool)> {
        self.inner().cli_login_answers.clone()
    }
}

pub(super) fn list_pending_cli_logins(daemon: &FakeDaemon) -> Reply<Vec<PendingCliLogin>> {
    daemon.reply(
        Some(Operation::ListPendingCliLogins),
        "list_pending_cli_logins".to_owned(),
        |inner| Ok(inner.pending_cli_logins.clone()),
    )
}

pub(super) fn answer_cli_login(daemon: &FakeDaemon, id: String, allow: bool) -> Reply<()> {
    let call = format!("answer_cli_login {id} {allow}");
    daemon.reply(Some(Operation::AnswerCliLogin), call, move |inner| {
        inner.cli_login_answers.push((id.clone(), allow));
        let before = inner.pending_cli_logins.len();
        inner.pending_cli_logins.retain(|item| item.id != id);
        if inner.pending_cli_logins.len() == before {
            return Err(DaemonError::NotFound {
                what: format!("CLI login {id}"),
            });
        }
        inner.emit(Event::PendingCliLoginGone(id));
        Ok(())
    })
}

pub(super) fn list_cli_sessions(daemon: &FakeDaemon) -> Reply<Vec<CliSession>> {
    daemon.reply(
        Some(Operation::ListCliSessions),
        "list_cli_sessions".to_owned(),
        |inner| Ok(inner.cli_sessions.clone()),
    )
}

pub(super) fn end_cli_session(daemon: &FakeDaemon, id: String) -> Reply<()> {
    let call = format!("end_cli_session {id}");
    daemon.reply(Some(Operation::EndCliSession), call, move |inner| {
        let before = inner.cli_sessions.len();
        inner.cli_sessions.retain(|session| session.id != id);
        if inner.cli_sessions.len() == before {
            return Err(DaemonError::NotFound {
                what: format!("CLI session {id}"),
            });
        }
        Ok(())
    })
}

pub(super) fn list_script_tokens(daemon: &FakeDaemon) -> Reply<Vec<ScriptToken>> {
    daemon.reply(
        Some(Operation::ListScriptTokens),
        "list_script_tokens".to_owned(),
        |inner| Ok(inner.script_tokens.clone()),
    )
}

pub(super) fn revoke_script_token(daemon: &FakeDaemon, id: String) -> Reply<()> {
    let call = format!("revoke_script_token {id}");
    daemon.reply(Some(Operation::RevokeScriptToken), call, move |inner| {
        let before = inner.script_tokens.len();
        inner.script_tokens.retain(|token| token.id != id);
        if inner.script_tokens.len() == before {
            return Err(DaemonError::NotFound {
                what: format!("script token {id}"),
            });
        }
        Ok(())
    })
}
