//! Add a GitHub repo to a channel on the fake daemon (CH-6, CH-18, CH-19).

use super::FakeDaemon;
use super::repos as library;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::github::parse_repo_text;
use crate::model::ChannelId;
use crate::repos::RepoPlace;

const WHAT: &str = "Adding the repo to the channel";

pub(super) fn add_repo_to_channel(daemon: &FakeDaemon, channel: &ChannelId, repo: &str) -> Reply<()> {
    let (channel, repo) = (channel.clone(), repo.trim().to_owned());
    daemon.reply(
        Some(Operation::AddRepoToChannel),
        format!("add repo {repo:?} to channel {channel}"),
        move |inner| {
            // Only `owner/name`: an address or a path is not what the daemon takes.
            let name = match parse_repo_text(&repo) {
                Ok(name) if name == repo => name,
                _ => {
                    return Err(DaemonError::Refused {
                        what: WHAT.to_owned(),
                        reason: format!(
                            "{repo:?} is not a GitHub repository. Write it as owner/name, such as acme/api"
                        ),
                    });
                }
            };
            let Some(found) = inner
                .workspace
                .channels
                .iter_mut()
                .find(|found| found.id == channel)
            else {
                return Err(DaemonError::NotFound {
                    what: format!("The channel {channel}"),
                });
            };
            if !found
                .repos
                .iter()
                .any(|held| held.eq_ignore_ascii_case(&name))
            {
                found.repos.push(name.clone());
            }
            library::record(inner, &name, RepoPlace::Channel(channel.to_string()));
            inner.emit(Event::Resync);
            Ok(())
        },
    )
}
