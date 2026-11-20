use super::FakeDaemon;
use super::repos as library;
use crate::daemon::{
    ChannelSessionWork, DaemonError, DeleteChannelPlan, DeleteChannelRequest,
    DeleteChannelResponse, Event, Operation, Reply,
};
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
                default_agent: None,
            });
            super::setup::finish(inner, &name);
            inner.emit(Event::Resync);
            Ok(id)
        },
    )
}

pub(super) fn plan_delete_channel(
    daemon: &FakeDaemon,
    channel: &ChannelId,
) -> Reply<DeleteChannelPlan> {
    let channel = channel.clone();
    daemon.reply(
        Some(Operation::PlanDeleteChannel),
        format!("plan delete channel {channel}"),
        move |inner| {
            if inner.workspace.channel(&channel).is_none() {
                return Err(DaemonError::NotFound {
                    what: format!("Channel {channel}"),
                });
            }
            let live_sessions: Vec<ChannelSessionWork> = inner
                .workspace
                .sessions
                .iter()
                .filter(|s| s.channel == channel && !s.archived)
                .map(|s| ChannelSessionWork {
                    session: s.id.clone(),
                    title: s.title.clone(),
                    dirty_files: 0,
                    unpushed_commits: 0,
                })
                .collect();
            let archived_sessions = u32::try_from(
                inner
                    .workspace
                    .sessions
                    .iter()
                    .filter(|s| s.channel == channel && s.archived)
                    .count(),
            )
            .unwrap_or(u32::MAX);
            let pending_requests = u32::try_from(
                inner
                    .workspace
                    .waiting()
                    .filter(|r| r.channel == channel)
                    .count(),
            )
            .unwrap_or(u32::MAX);
            let token = format!("pl_ch_{}", channel.as_str());
            Ok(DeleteChannelPlan {
                channel,
                live_sessions,
                archived_sessions,
                board_items: 0,
                pending_requests,
                token,
                moves_to_desk: Vec::new(),
                cannot_move_count: 0,
                offline_machines: Vec::new(),
            })
        },
    )
}

pub(super) fn delete_channel(
    daemon: &FakeDaemon,
    request: DeleteChannelRequest,
) -> Reply<DeleteChannelResponse> {
    daemon.reply(
        Some(Operation::DeleteChannel),
        format!("delete channel {}", request.channel),
        move |inner| {
            if inner.workspace.channel(&request.channel).is_none() {
                return Err(DaemonError::NotFound {
                    what: format!("Channel {}", request.channel),
                });
            }
            if request.plan_token.is_empty() {
                return Err(DaemonError::Refused {
                    what: format!("Deleting {}", request.channel),
                    reason: "a plan token is required. Plan the delete again".to_owned(),
                });
            }
            inner.workspace.channels.retain(|c| c.id != request.channel);
            inner.workspace.sessions.retain(|s| s.channel != request.channel);
            inner.workspace.requests.retain(|r| r.channel != request.channel);
            inner.emit(Event::Resync);
            Ok(DeleteChannelResponse {
                trash_item: format!("trash_{}", request.channel.as_str()),
            })
        },
    )
}
