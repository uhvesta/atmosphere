use super::FakeDaemon;
use super::repos as library;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{Channel, ChannelId, MachineId};
use crate::repos::RepoPlace;

pub(super) fn github_repos(daemon: &FakeDaemon, query: &str) -> Reply<Vec<String>> {
    let query = query.to_owned();
    daemon.reply(
        Some(Operation::GitHubRepos),
        format!("github repos {query:?}"),
        move |inner| Ok(crate::github::matching(&query, &inner.github_repos)),
    )
}

pub(super) fn create_channel(
    daemon: &FakeDaemon,
    name: &str,
    repos: Vec<String>,
    machines: Vec<MachineId>,
) -> Reply<ChannelId> {
    let name = name.trim().to_lowercase().replace(' ', "-");
    daemon.reply(
        Some(Operation::CreateChannel),
        format!("create channel {name:?}"),
        move |inner| {
            if name.is_empty() {
                return Err(DaemonError::Refused {
                    what: "Creating a channel".to_owned(),
                    reason: "it needs a name. Type one and try again".to_owned(),
                });
            }
            if name == crate::desk::FOLDER {
                return Err(DaemonError::Refused {
                    what: format!("Creating #{name}"),
                    reason: "that name is kept for the Desk's folder. Pick another name".to_owned(),
                });
            }
            if inner
                .workspace
                .channels
                .iter()
                .any(|found| found.name == name)
            {
                return Err(DaemonError::Refused {
                    what: format!("Creating #{name}"),
                    reason: "a channel with that name exists. Pick another name".to_owned(),
                });
            }
            super::setup::start(inner, &name, &repos, &machines)?;
            let id = ChannelId::new(name.clone());
            for repo in &repos {
                library::record(inner, repo, RepoPlace::Channel(id.to_string()));
            }
            inner.workspace.channels.push(Channel {
                id: id.clone(),
                name: name.clone(),
                repos,
                machines,
                purpose: String::new(),
            });
            super::setup::finish(inner, &name);
            inner.emit(Event::Resync);
            Ok(id)
        },
    )
}
