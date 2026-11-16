//! Hears an agent's terminal exit while the daemon runs, records it and
//! tells the feed, so the app sees "Exited" without asking (T5b). Without
//! this the store keeps a gone agent as running until the next start's
//! reconcile.
//!
//! One connection per running agent, opened at the end of its output: the
//! holder sends output it prints too, which is read and dropped. A watch
//! ends when the holder says the agent exited or hangs up; a holder that
//! hangs up without an exit is left to the next start's reconcile.

use std::path::PathBuf;

use humata2_holder::{Attachment, Event};
use humata2_store::{AgentState, StoreError};
use tokio::task::spawn_blocking;

use super::{Tell, now_ms};
use crate::status;

/// Watches `session`'s agent if its record says it runs. Returns at once.
pub(super) fn agent(tell: Tell, session: String) {
    tokio::spawn(async move {
        let Some((holder_pid, socket)) = running(&tell, &session).await else {
            return;
        };
        let Some(code) = exit_of(&socket).await else {
            return;
        };
        if record_exit(&tell, &session, holder_pid, code).await {
            tell.session(&session, humata2_grpc::humata::v1::Change::Updated, None)
                .await;
        }
    });
}

/// After the start's reconcile: tells the feed of every session as it now
/// stands, since the app may have read it before it was brought back, and
/// watches each agent that runs.
pub(crate) async fn after_restart(tell: Tell) {
    let store = tell.store.clone();
    let agents = match spawn_blocking(move || store.agents()).await {
        Ok(Ok(agents)) => agents,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't read the agents after bringing them back, so the app hears of their state only when it reloads: {}",
                status::chain(&error)
            );
            return;
        }
        Err(_) => return,
    };
    for agent in agents {
        tell.session(
            &agent.session,
            humata2_grpc::humata::v1::Change::Updated,
            None,
        )
        .await;
        if matches!(agent.state, AgentState::Running { .. }) {
            self::agent(tell.clone(), agent.session);
        }
    }
}

/// The running agent's holder pid and its tab's holder socket.
async fn running(tell: &Tell, session: &str) -> Option<(u32, PathBuf)> {
    let (store, key) = (tell.store.clone(), session.to_owned());
    let read = spawn_blocking(move || -> Result<Option<(u32, PathBuf)>, StoreError> {
        let Some(agent) = store.agent(&key)? else {
            return Ok(None);
        };
        let AgentState::Running { holder_pid } = agent.state else {
            return Ok(None);
        };
        Ok(store
            .tab(&agent.tab)?
            .map(|tab| (holder_pid, tab.holder_socket)))
    })
    .await;
    match read {
        Ok(Ok(found)) => found,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't read session {session}'s agent to watch for its exit, so the app hears of it only when it reloads: {}",
                status::chain(&error)
            );
            None
        }
        Err(_) => None,
    }
}

/// The agent's exit code, once its holder says it exited. `None` when the
/// holder can't be reached or hangs up first.
async fn exit_of(socket: &std::path::Path) -> Option<i32> {
    let Attachment {
        welcome,
        mut events,
        control: _control,
    } = humata2_holder::attach(socket, u64::MAX).await.ok()?;
    if let Some(code) = welcome.exited {
        return Some(code);
    }
    loop {
        match events.next().await {
            Ok(Some(Event::Exited { code })) => return Some(code),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Saves the agent as exited, unless it was stopped or started again since
/// (its record no longer runs under `holder_pid`). Says whether it saved.
async fn record_exit(tell: &Tell, session: &str, holder_pid: u32, code: i32) -> bool {
    let (store, key) = (tell.store.clone(), session.to_owned());
    let saved = spawn_blocking(move || -> Result<bool, StoreError> {
        let Some(mut agent) = store.agent(&key)? else {
            return Ok(false);
        };
        if agent.state != (AgentState::Running { holder_pid }) {
            return Ok(false);
        }
        agent.state = AgentState::Exited { code };
        agent.updated_unix_ms = now_ms();
        store.put_agent(&agent)?;
        Ok(true)
    })
    .await;
    match saved {
        Ok(Ok(saved)) => saved,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: session {session}'s agent exited with {code}, but saving that failed, so it shows as running until humatad2 restarts: {}",
                status::chain(&error)
            );
            false
        }
        Err(_) => false,
    }
}
