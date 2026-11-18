//! Add a repo to a running session on the fake daemon (CH-21): the session
//! gets a clone on its own branch, the channel gets the repo when it lacks
//! it, and the repo library hears of the use (RL-1).

use super::FakeDaemon;
use super::repos as library;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{SessionClone, SessionId};
use crate::repos::RepoPlace;

const WHAT: &str = "Adding the repo to the session";

pub(super) fn add_repo_to_session(daemon: &FakeDaemon, session: &SessionId, repo: &str) -> Reply<()> {
    let (session, repo) = (session.clone(), repo.trim().to_owned());
    daemon.reply(
        Some(Operation::AddRepoToSession),
        format!("add repo {repo:?} to session {session}"),
        move |inner| {
            // Only `owner/name`: an address or a path is not what the daemon takes.
            let Some(name) = owner_and_name(&repo) else {
                return Err(DaemonError::Refused {
                    what: WHAT.to_owned(),
                    reason: format!("{repo:?} is not a GitHub repository. Write it as owner/name, such as acme/api"),
                });
            };
            let found = inner.session_mut(&session)?;
            let channel = found.channel.clone();
            let short = name.rsplit_once('/').map_or(name.as_str(), |(_, short)| short).to_owned();
            if !found.clones.iter().any(|clone| clone.repo.eq_ignore_ascii_case(&name)) {
                let path = if found.folder.is_empty() {
                    short
                } else {
                    format!("{}/{short}", found.folder)
                };
                found.clones.push(SessionClone {
                    repo: name.clone(),
                    branch: found.branch.clone(),
                    path,
                });
            }
            let changed = found.clone();
            if let Some(held) = inner
                .workspace
                .channels
                .iter_mut()
                .find(|held| held.id == channel)
                && !held.repos.iter().any(|have| have.eq_ignore_ascii_case(&name))
            {
                held.repos.push(name.clone());
            }
            library::record(inner, &name, RepoPlace::Channel(channel.to_string()));
            inner.emit(Event::Session(changed));
            Ok(())
        },
    )
}

/// `text` when it is `owner/name` with the characters GitHub allows.
fn owner_and_name(text: &str) -> Option<String> {
    let (owner, name) = text.split_once('/')?;
    let part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (part(owner) && part(name)).then(|| text.to_owned())
}
