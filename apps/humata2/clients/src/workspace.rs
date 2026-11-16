//! Everything the app shows at once, as one value the daemon client loads.

use std::collections::BTreeMap;

use crate::model::{
    AccessRule, Channel, ChannelId, FileEntry, Item, JournalEntry, Machine, MachineId, PreparedView,
    PullRequest, RepoDiff, Request, Session, SessionFile, SessionId, Skill, Spec, Transfer,
    Trashed,
};

/// What every daemon the app talks to holds, merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The Desk: sessions outside any channel, and their documents (DK-1).
    pub desk: crate::desk::DeskData,
    /// The hosts.
    pub machines: Vec<Machine>,
    /// Health reported by each host.
    pub machine_health: BTreeMap<MachineId, crate::model::MachineHealth>,
    /// What each host found when it last restarted, newest first (frame 47).
    pub restarts: Vec<crate::model::Restart>,
    /// Channels being set up (frame 45).
    pub setups: Vec<crate::model::ChannelSetup>,
    /// The channels, in sidebar order.
    pub channels: Vec<Channel>,
    /// Every session, parents before their forks.
    pub sessions: Vec<Session>,
    /// Explicit launch choices, indexed by session.
    pub session_options: BTreeMap<SessionId, crate::model::SessionOptions>,
    /// Fake PTY output, in byte order, including ANSI control sequences.
    pub terminal_bytes: BTreeMap<(SessionId, usize), Vec<u8>>,
    /// Last terminal size reported by each attached view (columns, rows).
    pub terminal_sizes: BTreeMap<(SessionId, usize), (u16, u16)>,
    /// Other attached writers per terminal, excluding this app and read-only observers.
    pub terminal_peers: BTreeMap<(SessionId, usize), usize>,
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
    /// Each channel's access rules, in the order IN-17 lists them.
    pub rules: BTreeMap<ChannelId, Vec<AccessRule>>,
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

    /// Hosts declared by the channel or already holding one of its sessions.
    pub fn channel_machines<'a>(
        &'a self,
        channel: &'a ChannelId,
    ) -> impl Iterator<Item = &'a Machine> + 'a {
        self.machines.iter().filter(move |machine| {
            self.channel(channel)
                .is_some_and(|c| c.machines.contains(&machine.id))
                || self
                    .sessions
                    .iter()
                    .any(|s| &s.channel == channel && s.machine == machine.id)
        })
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
