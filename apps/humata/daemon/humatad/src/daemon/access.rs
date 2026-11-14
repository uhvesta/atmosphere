//! Agent Access: what the `humata` command line and `humata mcp` may ask
//! of the daemon, as the person set it in Settings › Agent Access. Each of
//! the two has a main switch, a switch for each group of actions (Read,
//! Change, Destroy) and one for each action; an action runs only while all
//! three are on. The app is never asked here.
//!
//! This stops agents and scripts that go through `humata`; it is no lock
//! against a program running as the person, which could change Humata's
//! files itself.

use std::collections::BTreeSet;

use humata_wire::pb;
use serde_json::{Value, json};

use super::Daemon;
use crate::error::{AccessError, DaemonError};

/// A client the settings gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Client {
    /// An agent's MCP server, `humata mcp`.
    Agent,
    /// The person's own `humata` at the command line.
    Terminal,
}

impl Client {
    /// The settings key its switches are kept under.
    fn key(self) -> &'static str {
        match self {
            Self::Agent => "agent_access.agents",
            Self::Terminal => "agent_access.terminal",
        }
    }

    /// Its section of Settings › Agent Access.
    fn section(self) -> &'static str {
        match self {
            Self::Agent => "Agents",
            Self::Terminal => "Terminal",
        }
    }

    fn named(self) -> &'static str {
        match self {
            Self::Agent => "agents using humata mcp",
            Self::Terminal => "the humata command line",
        }
    }
}

/// What an action does, which is the group its switch is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Read,
    Change,
    Destroy,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Self::Read => "Read",
            Self::Change => "Change",
            Self::Destroy => "Destroy",
        }
    }
}

impl From<Group> for pb::AccessGroup {
    fn from(group: Group) -> Self {
        match group {
            Group::Read => Self::Read,
            Group::Change => Self::Change,
            Group::Destroy => Self::Destroy,
        }
    }
}

/// The actions, each with the id its switch is kept and sent under, which
/// is its MCP tool's name where it has one, its switch's title in
/// Settings, and its group.
macro_rules! actions {
    ($($variant:ident: $id:literal, $title:literal, $group:ident;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Action {
            $($variant,)*
        }

        impl Action {
            /// Every action, in the order Settings lists them.
            const ALL: &'static [Self] = &[$(Self::$variant,)*];

            fn id(self) -> &'static str {
                match self {
                    $(Self::$variant => $id,)*
                }
            }

            fn title(self) -> &'static str {
                match self {
                    $(Self::$variant => $title,)*
                }
            }

            fn group(self) -> Group {
                match self {
                    $(Self::$variant => Group::$group,)*
                }
            }
        }
    };
}

actions! {
    ListProjects: "list_projects", "List projects", Read;
    ListSessions: "list_sessions", "List sessions, their models and recovery copies", Read;
    ReadHistory: "read_history", "Read and follow sessions' chats", Read;
    ReadChanges: "read_changes", "Read sessions' changes, files, reviews and pull requests", Read;
    ListInbox: "list_inbox", "Read the inbox", Read;
    Board: "board", "Read boards, specs and proposals", Read;
    Search: "search", "Search sessions", Read;
    ListHosts: "list_hosts", "List hosts, terminals and forwarded ports", Read;
    HostStatus: "host_status", "Check hosts", Read;
    ReadSettings: "read_settings", "Read skills, plugins, storage and saved repositories", Read;
    // Opens a link with the Mac's `open`, so no request of the daemon's
    // stands for it: `humata mcp` goes by this switch itself.
    Show: "show", "Open sessions and boards in Humata", Read;
    // The humata tools of sessions' own agents, which reach the daemon
    // through their hosts, not a client's connection: for agents only.
    AgentReadSession: "agent_read_session", "Sessions' own agents: read other sessions", Read;

    CreateProject: "create_project", "Add and change projects", Change;
    StartSession: "start_session", "Start, import and restore sessions", Change;
    SendMessage: "send_message", "Send messages and edit queued ones", Change;
    Approve: "approve", "Approve what sessions ask, and set what they may do unasked", Change;
    StopTurn: "stop_turn", "Stop turns", Change;
    RewindSession: "rewind_session", "Rewind sessions", Change;
    UndoRewind: "undo_rewind", "Undo rewinds", Change;
    CompactSession: "compact_session", "Compact sessions", Change;
    ForkSession: "fork_session", "Fork sessions", Change;
    UpdateSession: "update_session", "Rename, move, mark and unarchive sessions, and change their model", Change;
    AnswerRequest: "answer_request", "Answer agents' questions and move requests", Change;
    Propose: "propose", "Propose tasks and spec edits", Change;
    CreateItem: "create_item", "Add and change tasks, with your approval", Change;
    ApplyProposal: "apply_proposal", "Apply and decline proposals, with your approval", Change;
    Review: "review", "Comment on changes and send reviews", Change;
    EditChanges: "edit_changes", "Stage and unstage changes, and restore a file's version", Change;
    LandSession: "land_session", "Commit, push, open pull requests and update from base", Change;
    ManageHosts: "manage_hosts", "Add, change and install hosts", Change;
    Terminals: "terminals", "Open and type in terminals", Change;
    ForwardPorts: "forward_ports", "Forward ports", Change;
    ChangeSettings: "change_settings", "Change skills, plugins, defaults and saved repositories", Change;
    AgentStartSession: "agent_start_session", "Sessions' own agents: start sessions", Change;
    AgentSendMessage: "agent_send_message", "Sessions' own agents: write to other sessions", Change;

    DeleteProject: "delete_project", "Delete projects", Destroy;
    ArchiveSession: "archive_session", "Archive sessions", Destroy;
    DeleteSession: "delete_session", "Delete sessions", Destroy;
    DeleteItem: "delete_item", "Delete tasks, with your approval", Destroy;
    RevertChange: "revert_change", "Undo board changes, with your approval", Destroy;
    DiscardChanges: "discard_changes", "Throw away a session's uncommitted changes", Destroy;
    RemoveHost: "remove_host", "Remove hosts and sessions that can't be opened", Destroy;
}

impl Action {
    /// Whether it is one of sessions' own agents' tools, which only the
    /// agents' switches hold.
    fn of_sessions_agents(self) -> bool {
        matches!(
            self,
            Self::AgentReadSession | Self::AgentStartSession | Self::AgentSendMessage
        )
    }

    /// The actions `client` has switches for.
    fn of(client: Client) -> impl Iterator<Item = Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(move |action| client == Client::Agent || !action.of_sessions_agents())
    }
}

/// The action whose switch `request` goes by; None for one any client may
/// send whatever the switches say: reading these settings and following
/// an approval, without which the command line couldn't say why it waits,
/// and requests refused for every client anyway.
///
/// Every request is named, with no catch-all, so a new one doesn't build
/// until it is put under a switch here.
fn action_of(request: &pb::request::Kind) -> Option<Action> {
    use pb::request::Kind as Request;
    Some(match request {
        Request::ListProjects(_) => Action::ListProjects,
        Request::ListSessions(_)
        | Request::ListModels(_)
        | Request::GetChatCommands(_)
        | Request::GetSessionDefaults(_)
        | Request::ListImportable(_)
        | Request::ListSkipped(_)
        | Request::ListRecoveries(_) => Action::ListSessions,
        Request::ReadHistory(_) | Request::WatchSession(_) | Request::ReadImage(_) => {
            Action::ReadHistory
        }
        Request::ReadChanges(_)
        | Request::ListFiles(_)
        | Request::ReadFile(_)
        | Request::ListFolder(_)
        | Request::ListFileVersions(_)
        | Request::ListThreads(_)
        | Request::ListPullRequests(_) => Action::ReadChanges,
        Request::ListInbox(_) => Action::ListInbox,
        Request::ListItems(_)
        | Request::ReadBoardLog(_)
        | Request::ListProposals(_)
        | Request::ReadSpec(_)
        | Request::ReadSkill(_) => Action::Board,
        Request::Search(_) => Action::Search,
        Request::ListHosts(_)
        | Request::ListSshHosts(_)
        | Request::ListForwards(_)
        | Request::ListTerminals(_) => Action::ListHosts,
        Request::CheckHost(_) => Action::HostStatus,
        Request::ListSkills(_)
        | Request::ListPlugins(_)
        | Request::GetStorage(_)
        | Request::GetPullRefresh(_)
        | Request::ListSavedRepositories(_)
        | Request::ListRepositoryOwners(_) => Action::ReadSettings,

        Request::CreateProject(_) | Request::UpdateProject(_) => Action::CreateProject,
        Request::StartSession(_)
        | Request::ImportSession(_)
        | Request::RestoreSession(_)
        | Request::RestoreHost(_)
        | Request::AgentStartSession(_) => Action::StartSession,
        Request::SendMessage(_)
        | Request::Unqueue(_)
        | Request::EditQueued(_)
        | Request::AgentSendMessage(_) => Action::SendMessage,
        Request::Approve(_) | Request::SetApprovalMode(_) => Action::Approve,
        Request::AbortTurn(_) => Action::StopTurn,
        Request::RewindSession(_) => Action::RewindSession,
        Request::UndoRewind(_) => Action::UndoRewind,
        Request::CompactSession(_) => Action::CompactSession,
        Request::ForkSession(_) => Action::ForkSession,
        // Archiving removes the session's copy of the code.
        Request::UpdateSession(update) if update.archived == Some(true) => Action::ArchiveSession,
        Request::UpdateSession(_) | Request::MarkSeen(_) => Action::UpdateSession,
        Request::AnswerRequest(_) => Action::AnswerRequest,
        Request::Propose(_) | Request::AskPerson(_) | Request::RequestMove(_) => Action::Propose,
        Request::CreateItem(_) | Request::UpdateItem(_) | Request::ImportIssues(_) => {
            Action::CreateItem
        }
        Request::ApplyProposal(_) | Request::DeclineProposal(_) => Action::ApplyProposal,
        Request::MarkViewed(_)
        | Request::CreateThread(_)
        | Request::ReplyToThread(_)
        | Request::ResolveThread(_)
        | Request::DeleteComment(_)
        | Request::SendReview(_) => Action::Review,
        // Reverting throws the copy's unstaged changes away.
        Request::EditChanges(edit) if edit.action() == pb::ChangeAction::Revert => {
            Action::DiscardChanges
        }
        Request::EditChanges(_) | Request::RestoreFileVersion(_) => Action::EditChanges,
        Request::LandSession(_) | Request::UpdateFromBase(_) => Action::LandSession,
        Request::AddHost(_) | Request::UpdateHost(_) | Request::InstallHost(_) => {
            Action::ManageHosts
        }
        Request::OpenTerminal(_)
        | Request::AttachTerminal(_)
        | Request::TerminalInput(_)
        | Request::ResizeTerminal(_)
        | Request::CloseTerminal(_) => Action::Terminals,
        Request::ForwardPort(_) | Request::StopForward(_) => Action::ForwardPorts,
        Request::PickSkill(_)
        | Request::SetProjectPlugin(_)
        | Request::SetSessionDefaults(_)
        | Request::LinkCommands(_)
        | Request::SetModelServer(_)
        | Request::RenameSavedRepository(_)
        | Request::RemoveSavedRepository(_)
        | Request::AddRepositoryOwner(_)
        | Request::RemoveRepositoryOwner(_)
        | Request::RefreshRepositoryOwners(_) => Action::ChangeSettings,

        Request::DeleteProject(_) => Action::DeleteProject,
        Request::DeleteSession(_) => Action::DeleteSession,
        Request::DeleteItem(_) => Action::DeleteItem,
        Request::RevertBoardChange(_) => Action::RevertChange,
        Request::RemoveHost(_) | Request::RemoveSkipped(_) => Action::RemoveHost,

        // Any client's: these settings, to leave out what's off, and the
        // approvals an action of its own waits on.
        Request::GetAgentAccess(_) | Request::WaitForApproval(_) | Request::ListApprovals(_) => {
            return None;
        }
        // The app's only, refused for any client before the switches are
        // looked at.
        Request::SetAgentAccess(_)
        | Request::DecideApprovals(_)
        | Request::RevokeGrant(_)
        | Request::SetStorageSettings(_)
        | Request::CleanUpStorage(_)
        | Request::SetPullRefresh(_) => return None,
        // For hosts only: the daemon refuses them whoever asks.
        Request::ConfigureHost(_)
        | Request::ReadHostStatus(_)
        | Request::CloneRepositories(_)
        | Request::ScanSkills(_)
        | Request::ScanPlugins(_)
        | Request::WatchToolCalls(_)
        | Request::AnswerToolCall(_)
        | Request::StopHost(_)
        | Request::BackupSession(_)
        | Request::RestoreBackup(_)
        | Request::ConnectPort(_)
        | Request::WritePort(_)
        | Request::ClosePort(_)
        | Request::SetSpecs(_)
        | Request::DeleteBranches(_) => return None,
    })
}

/// One client's switches. An action's own switch is on unless its id is in
/// `off`, so an action added later starts on, inside its group.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Switches {
    enabled: bool,
    read: bool,
    change: bool,
    destroy: bool,
    off: BTreeSet<String>,
}

impl Default for Switches {
    /// Reads and changes on, destroys off.
    fn default() -> Self {
        Self {
            enabled: true,
            read: true,
            change: true,
            destroy: false,
            off: BTreeSet::new(),
        }
    }
}

impl Switches {
    /// As kept in the settings; what can't be read is as the defaults, and
    /// a switch not kept is as its default.
    fn parse(text: &str) -> Self {
        let kept: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let defaults = Self::default();
        let switch = |name: &str, default: bool| kept[name].as_bool().unwrap_or(default);
        Self {
            enabled: switch("enabled", defaults.enabled),
            read: switch("read", defaults.read),
            change: switch("change", defaults.change),
            destroy: switch("destroy", defaults.destroy),
            off: kept["off"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        }
    }

    fn text(&self) -> String {
        json!({
            "enabled": self.enabled,
            "read": self.read,
            "change": self.change,
            "destroy": self.destroy,
            "off": self.off,
        })
        .to_string()
    }

    fn group(&self, group: Group) -> bool {
        match group {
            Group::Read => self.read,
            Group::Change => self.change,
            Group::Destroy => self.destroy,
        }
    }

    /// The switch that keeps `action` from running, the outermost one that
    /// is off; None while it may run.
    fn stopped_by(&self, action: Action) -> Option<String> {
        if !self.enabled {
            Some("the main switch".to_owned())
        } else if !self.group(action.group()) {
            Some(format!("the {} switch", action.group().name()))
        } else if self.off.contains(action.id()) {
            Some(format!("the \"{}\" switch", action.title()))
        } else {
            None
        }
    }

    /// These switches with what `set` says: an action it doesn't name
    /// keeps its switch, and an id that is no action's is left out.
    fn with(&self, set: &pb::ClientAccess) -> Self {
        let mut off = self.off.clone();
        for action in Action::ALL {
            if let Some(sent) = set.actions.iter().find(|sent| sent.id == action.id()) {
                if sent.enabled {
                    off.remove(action.id());
                } else {
                    off.insert(action.id().to_owned());
                }
            }
        }
        off.retain(|id| Action::ALL.iter().any(|action| action.id() == id));
        Self {
            enabled: set.enabled,
            read: set.read,
            change: set.change,
            destroy: set.destroy,
            off,
        }
    }

    fn shown(&self, client: Client) -> pb::ClientAccess {
        pb::ClientAccess {
            enabled: self.enabled,
            read: self.read,
            change: self.change,
            destroy: self.destroy,
            actions: Action::of(client)
                .map(|action| pb::ActionAccess {
                    id: action.id().to_owned(),
                    title: action.title().to_owned(),
                    group: pb::AccessGroup::from(action.group()).into(),
                    enabled: !self.off.contains(action.id()),
                })
                .collect(),
        }
    }
}

impl Daemon {
    fn switches(&self, client: Client) -> Result<Switches, DaemonError> {
        Ok(self
            .store
            .setting(client.key())?
            .map(|kept| Switches::parse(&kept))
            .unwrap_or_default())
    }

    /// The switches of both clients, with every action.
    pub(super) fn agent_access(&self) -> Result<pb::AgentAccess, DaemonError> {
        Ok(pb::AgentAccess {
            agents: Some(self.switches(Client::Agent)?.shown(Client::Agent)),
            terminal: Some(self.switches(Client::Terminal)?.shown(Client::Terminal)),
            refused: Vec::new(),
        })
    }

    /// Keeps the switches `access` gives; a client it leaves out keeps
    /// its own.
    pub(super) fn set_agent_access(&self, access: &pb::AgentAccess) -> Result<(), DaemonError> {
        for (client, set) in [
            (Client::Agent, &access.agents),
            (Client::Terminal, &access.terminal),
        ] {
            if let Some(set) = set {
                let switches = self.switches(client)?.with(set);
                self.store.set_setting(client.key(), &switches.text())?;
            }
        }
        Ok(())
    }

    /// Refuses `request` from `client` while a switch its action goes by
    /// is off.
    pub(super) fn allows(
        &self,
        client: Client,
        request: &pb::request::Kind,
    ) -> Result<(), DaemonError> {
        let Some(action) = action_of(request) else {
            return Ok(());
        };
        self.allows_action(client, action, client.named())
    }

    fn allows_action(
        &self,
        client: Client,
        action: Action,
        named: &'static str,
    ) -> Result<(), DaemonError> {
        match self.switches(client)?.stopped_by(action) {
            Some(switch) => Err(AccessError::Off {
                action: action.title(),
                client: named,
                switch,
                section: client.section(),
            }
            .into()),
            None => Ok(()),
        }
    }

    /// Refuses `request`, a call to the humata tools from a session's own
    /// agent, while the agents' switch for starting or writing to sessions
    /// is off. Its other tools read and ask the person, and go by the
    /// project's settings alone; read_session asks [`Daemon::agents_access`]
    /// itself, since what it asks is what other tools ask too.
    pub(super) fn allows_tool(&self, request: &pb::request::Kind) -> Result<(), DaemonError> {
        let action = match request {
            pb::request::Kind::AgentStartSession(_) => Action::AgentStartSession,
            pb::request::Kind::AgentSendMessage(_) => Action::AgentSendMessage,
            _ => return Ok(()),
        };
        self.allows_action(Client::Agent, action, "sessions' own agents")
    }

    /// The switches as agents are told them, with what is off for them.
    pub(super) fn agents_access(&self) -> Result<pb::AgentAccess, DaemonError> {
        let mut access = self.agent_access()?;
        access.refused = self.refused(Client::Agent)?;
        Ok(access)
    }

    /// The ids of the actions that are off for `client`, for `humata mcp`
    /// to leave out of its tools.
    pub(super) fn refused(&self, client: Client) -> Result<Vec<String>, DaemonError> {
        let switches = self.switches(client)?;
        Ok(Action::of(client)
            .filter(|action| switches.stopped_by(*action).is_some())
            .map(|action| action.id().to_owned())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_kept_is_reads_and_changes_on_and_destroys_off() {
        for kept in ["", "not json", "{}", "[1, 2]"] {
            let switches = Switches::parse(kept);
            assert_eq!(switches.stopped_by(Action::ReadHistory), None, "{kept:?}");
            assert_eq!(switches.stopped_by(Action::SendMessage), None, "{kept:?}");
            assert!(
                switches.stopped_by(Action::DeleteProject).is_some(),
                "{kept:?}"
            );
        }
    }

    #[test]
    fn switches_read_back_as_kept() {
        let switches = Switches {
            enabled: true,
            read: false,
            change: true,
            destroy: true,
            off: BTreeSet::from(["send_message".to_owned()]),
        };
        assert_eq!(Switches::parse(&switches.text()), switches);
    }

    #[test]
    fn an_action_runs_only_while_its_three_switches_are_on() {
        let on = Switches {
            destroy: true,
            ..Switches::default()
        };
        assert_eq!(on.stopped_by(Action::ArchiveSession), None);
        let main_off = Switches {
            enabled: false,
            ..on.clone()
        };
        let group_off = Switches {
            destroy: false,
            ..on.clone()
        };
        let own_off = Switches {
            off: BTreeSet::from([Action::ArchiveSession.id().to_owned()]),
            ..on.clone()
        };
        for off in [&main_off, &group_off, &own_off] {
            assert!(off.stopped_by(Action::ArchiveSession).is_some(), "{off:?}");
        }
        // Another group's, and another action's, still run.
        assert_eq!(group_off.stopped_by(Action::SendMessage), None);
        assert_eq!(own_off.stopped_by(Action::DeleteProject), None);
    }

    #[test]
    fn setting_keeps_the_switch_of_an_action_left_out() {
        let kept = Switches {
            off: BTreeSet::from(["search".to_owned()]),
            ..Switches::default()
        };
        let set = pb::ClientAccess {
            enabled: true,
            read: true,
            change: true,
            destroy: true,
            actions: vec![
                pb::ActionAccess {
                    id: "send_message".to_owned(),
                    enabled: false,
                    ..Default::default()
                },
                pb::ActionAccess {
                    id: "no_such_action".to_owned(),
                    enabled: false,
                    ..Default::default()
                },
            ],
        };
        let now = kept.with(&set);
        assert!(now.destroy);
        assert_eq!(
            now.off,
            BTreeSet::from(["search".to_owned(), "send_message".to_owned()])
        );
    }

    #[test]
    fn every_action_has_its_own_id() {
        let ids: BTreeSet<&str> = Action::ALL.iter().map(|action| action.id()).collect();
        assert_eq!(ids.len(), Action::ALL.len());
    }

    #[test]
    fn archiving_and_reverting_go_by_destroy_switches() {
        use pb::request::Kind as Request;
        let archive = Request::UpdateSession(pb::UpdateSession {
            archived: Some(true),
            ..Default::default()
        });
        let rename = Request::UpdateSession(pb::UpdateSession {
            name: Some("new".to_owned()),
            ..Default::default()
        });
        let revert = Request::EditChanges(pb::EditChanges {
            action: pb::ChangeAction::Revert.into(),
            ..Default::default()
        });
        let stage = Request::EditChanges(pb::EditChanges {
            action: pb::ChangeAction::Stage.into(),
            ..Default::default()
        });
        let group = |request: &Request| action_of(request).map(Action::group);
        assert_eq!(group(&archive), Some(Group::Destroy));
        assert_eq!(group(&rename), Some(Group::Change));
        assert_eq!(group(&revert), Some(Group::Destroy));
        assert_eq!(group(&stage), Some(Group::Change));
    }
}
