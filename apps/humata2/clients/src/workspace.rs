//! Everything the app shows at once, as one value the daemon client loads.

use std::collections::BTreeMap;

use crate::model::{
    Channel, ChannelId, FileEntry, Item, JournalEntry, Machine, MachineId, PreparedView,
    PullRequest, RepoDiff, Request, Session, SessionFile, SessionId, Skill, Spec, Transfer,
    Trashed,
};

/// What every daemon the app talks to holds, merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The hosts.
    pub machines: Vec<Machine>,
    /// The channels, in sidebar order.
    pub channels: Vec<Channel>,
    /// Every session, parents before their forks.
    pub sessions: Vec<Session>,
    /// Inbox requests, newest first, answered ones included.
    pub requests: Vec<Request>,
    /// Board items on every board.
    pub items: Vec<Item>,
    /// Descriptions, ownership, history and approvals for board items.
    pub item_details: BTreeMap<crate::model::ItemId, crate::model::ItemDetails>,
    /// Each session's changes.
    pub diffs: Vec<RepoDiff>,
    /// Files on the hosts.
    pub files: Vec<FileEntry>,
    /// Repository paths in each session’s own clones, including unchanged files.
    pub session_files: BTreeMap<SessionId, Vec<SessionFile>>,
    /// Text returned by the fake host's file reader, keyed by session/repo/path.
    pub file_contents: BTreeMap<(SessionId, String, String), String>,
    /// Copies between hosts.
    pub transfers: Vec<Transfer>,
    /// What can still be restored.
    pub trash: Vec<Trashed>,
    /// Open pull requests.
    pub pulls: Vec<PullRequest>,
    /// Specs.
    pub specs: Vec<Spec>,
    /// The journal, newest first.
    pub journal: Vec<JournalEntry>,
    /// Channel events, newest first.
    pub activity: Vec<crate::model::Activity>,
    /// Views agents prepared.
    pub views: Vec<PreparedView>,
    /// The skills the composer's `/` menu offers.
    pub skills: Vec<Skill>,
}

impl Workspace {
    /// The session with `id`.
    #[must_use]
    pub fn session(&self, id: &SessionId) -> Option<&Session> {
        self.sessions.iter().find(|session| &session.id == id)
    }

    /// The channel with `id`.
    #[must_use]
    pub fn channel(&self, id: &ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|channel| &channel.id == id)
    }

    /// The host with `id`.
    #[must_use]
    pub fn machine(&self, id: &MachineId) -> Option<&Machine> {
        self.machines.iter().find(|machine| &machine.id == id)
    }

    /// Requests still waiting for an answer.
    pub fn waiting(&self) -> impl Iterator<Item = &Request> {
        self.requests
            .iter()
            .filter(|request| request.answer.is_none())
    }

    /// The forks of `id`, in order.
    pub fn forks<'a>(&'a self, id: &'a SessionId) -> impl Iterator<Item = &'a Session> + 'a {
        self.sessions
            .iter()
            .filter(move |session| session.parent.as_ref() == Some(id) && !session.archived)
    }
}
