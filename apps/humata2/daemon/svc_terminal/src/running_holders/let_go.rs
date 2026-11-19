//! D784, D783: the host forgot or removed this machine, so this machine
//! lets go of the host's sessions and channels it still records. Each
//! named session's record goes, and its tabs stay with no session, as
//! shells, so every holder keeps running and is listed under "Not in a
//! channel" by the tab it already had: the user attaches, adopts or ends it.
//! A named channel goes once none of its sessions is left. Nothing here
//! ends a holder.

use std::collections::HashSet;
use std::sync::Arc;

use humata2_sessions::DESK_CHANNEL;
use humata2_store::{Store, StoreError, TabKind};

use super::RunningError;

/// Lets go of `sessions` and then of `channels`, as the module says. The
/// sessions let go of: those this machine still recorded. Safe to repeat.
pub async fn let_go(
    store: &Arc<Store>,
    sessions: Vec<String>,
    channels: Vec<String>,
) -> Result<Vec<String>, RunningError> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || let_go_now(&store, &sessions, &channels))
        .await
        .map_err(|_| RunningError::Task {
            doing: "letting go of the host's sessions",
        })?
}

fn let_go_now(store: &Store, sessions: &[String], channels: &[String]) -> Result<Vec<String>, RunningError> {
    let failed = |doing| move |source: StoreError| RunningError::Store { doing, source };
    let named: HashSet<&str> = sessions.iter().map(String::as_str).collect();
    let tabs = store.tabs().map_err(failed("list the tabs"))?;
    let mut let_go = Vec::new();
    for record in store.sessions().map_err(failed("list the sessions"))? {
        if !named.contains(record.id.as_str()) {
            continue;
        }
        let kept: Vec<_> = tabs
            .iter()
            .filter(|tab| tab.session.as_deref() == Some(record.id.as_str()))
            .cloned()
            .collect();
        // The tabs go with the session; a crash before they are put back
        // leaves the holders with no tab, which the next listing gives one.
        store
            .remove_session(&record.id)
            .map_err(failed("remove a session the host let go of"))?;
        for mut tab in kept {
            tab.session = None;
            tab.kind = TabKind::Shell;
            store
                .put_tab(&tab)
                .map_err(failed("keep a let-go session's tab, with no session"))?;
        }
        let_go.push(record.id);
    }
    let left: HashSet<String> = store
        .sessions()
        .map_err(failed("list the sessions"))?
        .into_iter()
        .map(|record| record.channel)
        .collect();
    for channel in channels {
        if channel == DESK_CHANNEL || left.contains(channel) {
            continue;
        }
        if store.channel(channel).map_err(failed("read a channel the host let go of"))?.is_some() {
            store
                .forget_channel(channel)
                .map_err(failed("remove a channel the host let go of"))?;
        }
    }
    Ok(let_go)
}
