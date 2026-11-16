//! The daemon's messages as the app's model, and back. Pure: no calls.

use std::collections::BTreeMap;

use humata2_clients::model::{
    Agent, Channel, ChannelId, ForkKind, Machine, MachineId, MachineKind, Recovery, Session,
    SessionId, Status, Turn,
};
use humata2_grpc::humata::v1 as pb;

/// Which daemon agent name each app agent runs under. By default the
/// agent's command name ("claude", "zsh", ...).
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentNames {
    overrides: BTreeMap<String, Agent>,
    names: BTreeMap<String, String>,
}

impl AgentNames {
    pub(crate) fn set(&mut self, agent: Agent, name: String) {
        self.overrides.insert(name.clone(), agent);
        self.names.insert(agent.command().to_owned(), name);
    }

    /// The daemon's name for `agent`.
    pub(crate) fn name(&self, agent: Agent) -> String {
        self.names
            .get(agent.command())
            .cloned()
            .unwrap_or_else(|| agent.command().to_owned())
    }

    /// The app's agent for a daemon name; a name nobody knows is a shell.
    pub(crate) fn agent(&self, name: &str) -> Agent {
        if let Some(agent) = self.overrides.get(name) {
            return *agent;
        }
        Agent::ALL
            .into_iter()
            .find(|agent| agent.command() == name)
            .unwrap_or(Agent::Shell)
    }
}

/// `owner/name` as a repository reference; `None` unless it has exactly
/// one `/` with text on both sides.
pub(crate) fn parse_repo(text: &str) -> Option<pb::RepoRef> {
    let (owner, name) = text.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(pb::RepoRef {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// A repository reference written `owner/name`.
pub(crate) fn repo_text(repo: &pb::RepoRef) -> String {
    format!("{}/{}", repo.owner, repo.name)
}

pub(crate) fn machine_id(id: Option<&pb::MachineId>) -> MachineId {
    MachineId::new(id.map(|id| id.value.clone()).unwrap_or_default())
}

pub(crate) fn session_id(id: Option<&pb::SessionId>) -> SessionId {
    SessionId::new(id.map(|id| id.value.clone()).unwrap_or_default())
}

pub(crate) fn channel(channel: &pb::Channel) -> Channel {
    Channel {
        id: ChannelId::new(
            channel
                .id
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        name: channel.name.clone(),
        repos: channel.repos.iter().map(repo_text).collect(),
        machines: channel
            .machines
            .iter()
            .map(|machine| MachineId::new(machine.value.clone()))
            .collect(),
        purpose: String::new(),
    }
}

/// This Mac, as the app shows it.
pub(crate) fn this_machine(id: MachineId) -> Machine {
    Machine {
        id,
        name: "This Mac".to_owned(),
        kind: MachineKind::ThisMac,
        online: true,
        detail: String::new(),
        last_seen: String::new(),
        lost_at: None,
    }
}

pub(crate) fn machine(machine: &pb::Machine) -> Machine {
    let kind = if machine.is_this_mac {
        MachineKind::ThisMac
    } else if let Some(provider) = &machine.provider {
        MachineKind::Provider {
            provider: provider.value.clone(),
        }
    } else {
        MachineKind::Ssh {
            address: machine.address.clone(),
        }
    };
    Machine {
        id: machine_id(machine.id.as_ref()),
        name: machine.name.clone(),
        kind,
        online: machine.reachability == pb::Reachability::Online as i32,
        detail: [machine.os.as_str(), machine.arch.as_str()]
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
        last_seen: String::new(),
        lost_at: None,
    }
}

/// The tabs in the app's order: the agent's first, then the shells as the
/// daemon lists them.
pub(crate) fn order_tabs(tabs: Vec<pb::Tab>) -> Vec<pb::Tab> {
    let (agents, shells): (Vec<_>, Vec<_>) = tabs
        .into_iter()
        .partition(|tab| tab.kind == pb::tab::Kind::Agent as i32);
    agents.into_iter().take(1).chain(shells).collect()
}

/// A session's status and, when it needs one, the recovery note.
fn status(session: &pb::Session) -> (Status, Option<Recovery>) {
    use pb::SessionStatus as S;
    let exited = Status::Exited {
        code: session.exit_code,
    };
    match S::try_from(session.status).unwrap_or(S::Unspecified) {
        S::Starting | S::Working => (Status::Working, None),
        S::NeedsInput => (Status::NeedsYou, None),
        S::ProbablyIdle => (Status::ProbablyIdle, None),
        S::Exited => (exited, None),
        S::Offline => (Status::Offline, None),
        S::ResumeFailed => (
            exited,
            Some(Recovery::Recoverable {
                needs: "Humata couldn't resume this session's agent. Try Resume again".to_owned(),
            }),
        ),
        _ => (Status::Idle, None),
    }
}

/// A session; `tabs` are the session's tabs in [`order_tabs`] order.
pub(crate) fn session(session: &pb::Session, tabs: &[pb::Tab], agents: &AgentNames) -> Session {
    let (status, recovery) = status(session);
    let first = session.repos.first();
    let fork = session.fork_origin.as_ref();
    Session {
        id: session_id(session.id.as_ref()),
        channel: ChannelId::new(
            session
                .channel
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        parent: fork.and_then(|origin| origin.parent.as_ref().map(|id| SessionId::new(id.value.clone()))),
        fork: fork.and_then(|origin| match pb::ForkKind::try_from(origin.kind) {
            Ok(pb::ForkKind::CodeAndContext) => Some(ForkKind::Worktree),
            Ok(pb::ForkKind::CodeOnly) => Some(ForkKind::Shared),
            Ok(pb::ForkKind::ContextOnly) => Some(ForkKind::Conversation),
            _ => None,
        }),
        title: session.title.clone(),
        agent: agents.agent(
            session
                .agent
                .as_ref()
                .map(|agent| agent.value.as_str())
                .unwrap_or_default(),
        ),
        machine: machine_id(session.machine.as_ref()),
        turn: match status {
            Status::Working => Turn::Running,
            _ => Turn::Ended,
        },
        status,
        pinned: session.pinned,
        vip: session.vip,
        unread: u32::from(session.unread),
        repo: first
            .and_then(|repo| repo.repo.as_ref())
            .map(repo_text)
            .unwrap_or_default(),
        branch: first.map(|repo| repo.branch.clone()).unwrap_or_default(),
        started: String::new(),
        active: String::new(),
        activity: String::new(),
        transcript: Vec::new(),
        shells: tabs
            .iter()
            .filter(|tab| tab.kind == pb::tab::Kind::Shell as i32)
            .map(|tab| tab.title.clone())
            .collect(),
        terminal_agents: BTreeMap::new(),
        archived: session.status == pb::SessionStatus::Archived as i32,
        recovery,
        model: (!session.model.is_empty()).then(|| session.model.clone()),
        restart_detail: None,
        permissions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb_session(status: pb::SessionStatus) -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId {
                value: "se_1".to_owned(),
            }),
            status: status as i32,
            exit_code: 3,
            agent: Some(pb::AgentId {
                value: "claude".to_owned(),
            }),
            repos: vec![pb::SessionRepo {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "app".to_owned(),
                }),
                branch: "feature".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn tab(kind: pb::tab::Kind, id: &str) -> pb::Tab {
        pb::Tab {
            id: Some(pb::TabId {
                value: id.to_owned(),
            }),
            kind: kind as i32,
            title: id.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn statuses_map_to_the_apps_statuses() {
        let agents = AgentNames::default();
        let status_of = |status| session(&pb_session(status), &[], &agents).status;
        assert_eq!(status_of(pb::SessionStatus::Starting), Status::Working);
        assert_eq!(status_of(pb::SessionStatus::Working), Status::Working);
        assert_eq!(status_of(pb::SessionStatus::NeedsInput), Status::NeedsYou);
        assert_eq!(status_of(pb::SessionStatus::Idle), Status::Idle);
        assert_eq!(status_of(pb::SessionStatus::Offline), Status::Offline);
        assert_eq!(status_of(pb::SessionStatus::Moving), Status::Idle);
        assert_eq!(
            status_of(pb::SessionStatus::Exited),
            Status::Exited { code: 3 }
        );
    }

    #[test]
    fn a_failed_resume_is_exited_and_recoverable() {
        let mapped = session(
            &pb_session(pb::SessionStatus::ResumeFailed),
            &[],
            &AgentNames::default(),
        );
        assert_eq!(mapped.status, Status::Exited { code: 3 });
        assert!(matches!(
            mapped.recovery,
            Some(Recovery::Recoverable { .. })
        ));
    }

    #[test]
    fn a_session_takes_its_branch_repo_and_agent() {
        let mut agents = AgentNames::default();
        let plain = session(&pb_session(pb::SessionStatus::Idle), &[], &agents);
        assert_eq!(plain.branch, "feature");
        assert_eq!(plain.repo, "acme/app");
        assert_eq!(plain.agent, Agent::Claude);
        assert_eq!(plain.recovery, None);

        agents.set(Agent::Codex, "claude".to_owned());
        let renamed = session(&pb_session(pb::SessionStatus::Idle), &[], &agents);
        assert_eq!(renamed.agent, Agent::Codex);
        assert_eq!(agents.name(Agent::Codex), "claude");
        assert_eq!(agents.name(Agent::Shell), "zsh");
        assert_eq!(agents.agent("nobody-knows"), Agent::Shell);
    }

    #[test]
    fn shells_are_the_shell_tabs_after_the_agent() {
        let tabs = order_tabs(vec![
            tab(pb::tab::Kind::Shell, "sh1"),
            tab(pb::tab::Kind::Agent, "ag"),
            tab(pb::tab::Kind::Shell, "sh2"),
        ]);
        let ids: Vec<_> = tabs
            .iter()
            .filter_map(|tab| tab.id.as_ref().map(|id| id.value.as_str()))
            .collect();
        assert_eq!(ids, ["ag", "sh1", "sh2"]);
        let mapped = session(
            &pb_session(pb::SessionStatus::Idle),
            &tabs,
            &AgentNames::default(),
        );
        assert_eq!(mapped.shells, ["sh1", "sh2"]);
    }

    #[test]
    fn repos_parse_as_owner_and_name() {
        assert_eq!(
            parse_repo("acme/app"),
            Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "app".to_owned()
            })
        );
        assert_eq!(parse_repo("app"), None);
        assert_eq!(parse_repo("a/b/c"), None);
        assert_eq!(parse_repo("/app"), None);
        assert_eq!(parse_repo("acme/"), None);
    }
}
