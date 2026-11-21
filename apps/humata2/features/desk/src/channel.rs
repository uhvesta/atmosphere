//! A channel's own Desk (CD-1 to CD-6): the New desk session dialog with its
//! six switches, Close with its confirmation, and the switches' changes. The
//! sessions themselves are [`humata2_clients::desk::ChannelDeskSession`]s in
//! the workspace; they are in no channel's Sessions tree and not on the plain
//! Desk page.

use std::collections::BTreeSet;

use humata2_clients::desk::{ChannelDeskSession, NewChannelDesk, Switch, Switches, blocked_by};
use humata2_clients::model::{Agent, ChannelId, MachineId, Session, SessionId};
use humata2_clients::Reply;
use tca::Effect;

use crate::{Change, Delegate, Desk, DeskAction, DeskState};

/// Status of ensuring a channel's persistent primary Desk session (CHANNELDESK).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelDeskStatus {
    /// Starting or reopening the channel's primary Desk session.
    Starting,
    /// Every agent is turned off in Settings › Agents.
    NoAgent,
    /// No online machine is available to run the channel's Desk.
    NoMachine,
    /// Explicitly closed by the user; waiting to reopen or start fresh.
    Closed,
    /// Starting or reopening failed.
    Failed(String),
}

/// A channel's Desk session with the session behind it.
#[derive(Clone, Copy, Debug)]
pub struct ChannelRow<'a> {
    /// The session: its title, agent, machine and status.
    pub session: &'a Session,
    /// What the channel's Desk knows about it. DOCV's version strip reads
    /// `has_versions`, `doc_head` and the `desk` part from here.
    pub desk: &'a ChannelDeskSession,
}

/// The New desk session dialog on a channel's Desk (CD-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelForm {
    /// The channel it looks after.
    pub channel: ChannelId,
    /// The title; blank gets one from the agent.
    pub title: String,
    /// What it runs; [`Agent::Shell`] is "Terminal".
    pub agent: Agent,
    /// "A session" is picked rather than "This channel".
    pub on_session: bool,
    /// The session it looks after, when `on_session`.
    pub session: Option<SessionId>,
    /// The session pop-up is open.
    pub sessions_open: bool,
    /// The machine it runs on.
    pub machine: Option<MachineId>,
    /// The machine pop-up is open.
    pub machines_open: bool,
    /// Its switches.
    pub switches: Switches,
    /// The start is in flight.
    pub starting: bool,
    /// Why it didn't start, to show on the form.
    pub error: Option<String>,
}

impl ChannelForm {
    /// The title the session starts with: what was typed, or "New <agent>
    /// session" when that is blank.
    #[must_use]
    pub fn start_title(&self) -> String {
        if self.title.trim().is_empty() {
            format!("New {} session", self.agent.name())
        } else {
            self.title.trim().to_owned()
        }
    }

    /// The session the new one looks after: `None` for the whole channel.
    #[must_use]
    pub fn target(&self) -> Option<&SessionId> {
        self.on_session.then_some(self.session.as_ref()).flatten()
    }
}

/// A change to the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelNewAction {
    /// The title was typed.
    Title(String),
    /// An agent, or Terminal, was picked.
    Agent(Agent),
    /// "This channel" (false) or "A session" (true) was picked.
    OnSession(bool),
    /// A session was picked as the target.
    Session(SessionId),
    /// The session pop-up was opened or closed.
    Sessions(bool),
    /// A machine was picked.
    Machine(MachineId),
    /// The machine pop-up was opened or closed.
    Machines(bool),
    /// A switch was turned on or off.
    Switch(Switch, bool),
    /// Start was clicked.
    Start,
}

/// Close session on a channel Desk session, asking first (CD-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelCloseSheet {
    /// The session to close.
    pub session: SessionId,
    /// The channel it belongs to and whether it is the channel's primary Desk
    /// (`target: None`), captured when the sheet opened so a close still knows
    /// its channel even if a daemon event removes the record before the RPC returns.
    pub channel: Option<(ChannelId, bool)>,
    /// Markdown files already copied to `shared/` from this sheet.
    pub copied: BTreeSet<String>,
    /// The close is in flight.
    pub closing: bool,
    /// Why something didn't happen, to show on the sheet.
    pub error: Option<String>,
}

impl DeskState {
    /// A channel's open Desk sessions, most recently used first (CD-2).
    #[must_use]
    pub fn channel_rows(&self, channel: &ChannelId) -> Vec<ChannelRow<'_>> {
        let mut rows: Vec<ChannelRow<'_>> = self
            .workspace
            .sessions
            .iter()
            .filter_map(|session| {
                let desk = self.workspace.desk.channel_sessions.get(&session.id)?;
                (desk.channel == *channel && desk.desk.closed.is_none())
                    .then_some(ChannelRow { session, desk })
            })
            .collect();
        rows.sort_by(|a, b| {
            b.desk
                .desk
                .last_used
                .order
                .cmp(&a.desk.desk.last_used.order)
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
        rows
    }

    /// A channel's persistent primary Desk session (`target: None`), if one is
    /// currently open (CHANNELDESK).
    #[must_use]
    pub fn primary_channel_desk(&self, channel: &ChannelId) -> Option<ChannelRow<'_>> {
        self.channel_rows(channel)
            .into_iter()
            .find(|row| row.desk.target.is_none())
    }

    /// A channel's most recently used closed-kept primary Desk session
    /// (`target: None`), if one exists (CHANNELDESK).
    #[must_use]
    pub fn closed_primary_channel_desk(&self, channel: &ChannelId) -> Option<ChannelRow<'_>> {
        self.channel_closed(channel)
            .into_iter()
            .find(|row| row.desk.target.is_none())
    }

    /// A channel's closed Desk sessions whose folder is kept (DV-13), most
    /// recently used first. A folder is kept once the person kept it or it
    /// holds a version.
    #[must_use]
    pub fn channel_closed(&self, channel: &ChannelId) -> Vec<ChannelRow<'_>> {
        let mut rows: Vec<ChannelRow<'_>> = self
            .workspace
            .sessions
            .iter()
            .filter_map(|session| {
                let desk = self.workspace.desk.channel_sessions.get(&session.id)?;
                (desk.channel == *channel && desk.desk.closed.is_some() && folder_kept(desk))
                    .then_some(ChannelRow { session, desk })
            })
            .collect();
        rows.sort_by(|a, b| {
            b.desk
                .desk
                .last_used
                .order
                .cmp(&a.desk.desk.last_used.order)
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
        rows
    }

    /// The Markdown files of a channel's open Desk sessions and of its closed
    /// kept ones (DV-13), each with its row: open rows first.
    #[must_use]
    pub fn channel_documents(&self, channel: &ChannelId) -> Vec<(&str, ChannelRow<'_>)> {
        let mut rows = self.channel_rows(channel);
        rows.extend(self.channel_closed(channel));
        rows.into_iter()
            .flat_map(|row| {
                row.desk
                    .documents
                    .iter()
                    .map(move |name| (name.as_str(), row))
            })
            .collect()
    }

    /// What the channels' Desks know about a session, when it is a channel
    /// Desk session.
    #[must_use]
    pub fn channel_record(&self, id: &SessionId) -> Option<&ChannelDeskSession> {
        self.workspace.desk.channel_sessions.get(id)
    }

    /// The open channel Desk sessions looking after `session` (CD-2), most
    /// recently used first. Read from the workspace, which the app refreshes
    /// after every start and change, so the Info panel follows them.
    #[must_use]
    pub fn targeting_rows(&self, session: &SessionId) -> Vec<ChannelRow<'_>> {
        let mut rows: Vec<ChannelRow<'_>> = self
            .workspace
            .sessions
            .iter()
            .filter_map(|found| {
                let desk = self.workspace.desk.channel_sessions.get(&found.id)?;
                (desk.target.as_ref() == Some(session) && desk.desk.closed.is_none())
                    .then_some(ChannelRow { session: found, desk })
            })
            .collect();
        rows.sort_by(|a, b| {
            b.desk
                .desk
                .last_used
                .order
                .cmp(&a.desk.desk.last_used.order)
                .then_with(|| a.session.id.cmp(&b.session.id))
        });
        rows
    }

    /// The sessions of a channel a Desk session can look after: its open
    /// ones, parents first. Desk sessions are not among them.
    #[must_use]
    pub fn channel_members(&self, channel: &ChannelId) -> Vec<&Session> {
        self.workspace
            .sessions
            .iter()
            .filter(|session| session.channel == *channel && !session.archived)
            .collect()
    }

    /// The sessions a channel Desk session can be pointed at: its channel's
    /// open ones except the one it already looks after. Empty for a session
    /// that is not a channel Desk session.
    #[must_use]
    pub fn target_choices(&self, session: &SessionId) -> Vec<&Session> {
        let Some(record) = self.channel_record(session) else {
            return Vec::new();
        };
        self.channel_members(&record.channel)
            .into_iter()
            .filter(|member| record.target.as_ref() != Some(&member.id))
            .collect()
    }

    /// The switches the channel's rules set to Never (CD-3): shown as
    /// blocked, on or off.
    #[must_use]
    pub fn blocked_for(&self, channel: &ChannelId) -> Vec<Switch> {
        blocked_by(
            self.workspace
                .rules
                .get(channel)
                .map_or(&[][..], Vec::as_slice),
        )
    }
}

/// A channel Desk session's folder is kept once it holds a version (DV-13),
/// the daemon's rule on Close; there is no Keep folder for one.
fn folder_kept(desk: &ChannelDeskSession) -> bool {
    desk.desk.version_count > 0
}

impl Desk {
    /// Opens the New desk session dialog for `channel`, looking after
    /// `target` or, with none, the whole channel. It starts on the target's
    /// machine, else the channel's first online one.
    pub(super) fn start_channel_new(
        state: &mut DeskState,
        channel: ChannelId,
        target: Option<SessionId>,
        agent: Agent,
    ) {
        if state.sheet_open() {
            return;
        }
        let workspace = state.workspace.clone();
        let online = |id: &MachineId| workspace.machine(id).is_some_and(|machine| machine.online);
        let machine = target
            .as_ref()
            .and_then(|id| workspace.session(id))
            .map(|session| session.machine.clone())
            .filter(|id| online(id))
            .or_else(|| {
                workspace
                    .channel(&channel)
                    .and_then(|found| found.machines.iter().find(|id| online(id)).cloned())
            })
            .or_else(|| {
                workspace
                    .machines
                    .iter()
                    .find(|machine| machine.online)
                    .map(|machine| machine.id.clone())
            });
        state.menu = None;
        state.channel_new = Some(ChannelForm {
            channel,
            title: String::new(),
            agent,
            on_session: target.is_some(),
            session: target,
            sessions_open: false,
            machine,
            machines_open: false,
            switches: Switches::default(),
            starting: false,
            error: None,
        });
    }

    pub(super) fn reduce_channel_new(
        &self,
        state: &mut DeskState,
        action: ChannelNewAction,
    ) -> Effect<DeskAction> {
        let Some(channel) = state.channel_new.as_ref().map(|form| form.channel.clone()) else {
            return Effect::none();
        };
        let members: Vec<SessionId> = state
            .channel_members(&channel)
            .into_iter()
            .map(|session| session.id.clone())
            .collect();
        let online: Vec<MachineId> = state
            .workspace
            .machines
            .iter()
            .filter(|machine| machine.online)
            .map(|machine| machine.id.clone())
            .collect();
        let Some(form) = state.channel_new.as_mut() else {
            return Effect::none();
        };
        if form.starting {
            return Effect::none();
        }
        match action {
            ChannelNewAction::Title(title) => form.title = title,
            ChannelNewAction::Agent(agent) => form.agent = agent,
            ChannelNewAction::OnSession(on) => {
                form.on_session = on;
                form.sessions_open = false;
                if on && form.session.is_none() {
                    form.session = members.first().cloned();
                }
            }
            ChannelNewAction::Session(id) => {
                if members.contains(&id) {
                    form.session = Some(id);
                }
                form.sessions_open = false;
            }
            ChannelNewAction::Sessions(open) => form.sessions_open = open,
            ChannelNewAction::Machine(machine) => {
                if online.contains(&machine) {
                    form.machine = Some(machine);
                }
                form.machines_open = false;
            }
            ChannelNewAction::Machines(open) => form.machines_open = open,
            ChannelNewAction::Switch(switch, on) => form.switches = form.switches.with(switch, on),
            ChannelNewAction::Start => return self.start_channel_session(form),
        }
        Effect::none()
    }

    fn start_channel_session(&self, form: &mut ChannelForm) -> Effect<DeskAction> {
        let Some(machine) = form.machine.clone() else {
            form.error = Some(
                "There is no online machine to run this on. Pick an online machine, or add one on the Hosts page."
                    .to_owned(),
            );
            return Effect::none();
        };
        if form.on_session && form.session.is_none() {
            form.error = Some(
                "This channel has no session to look after. Pick \"This channel\", or start a session first."
                    .to_owned(),
            );
            return Effect::none();
        }
        let request = NewChannelDesk {
            channel: form.channel.clone(),
            target: form.target().cloned(),
            title: form.start_title(),
            agent: form.agent,
            machine,
            switches: form.switches,
        };
        form.starting = true;
        form.error = None;
        let client = self.client.clone();
        Effect::future(async move { DeskAction::Started(client.start_channel(request).await) })
    }

    /// Starts a channel's persistent primary Desk session (`target: None`)
    /// without opening a dialog or navigating away from the channel (CHANNELDESK).
    pub(super) fn ensure_primary_channel(
        &self,
        state: &mut DeskState,
        channel: ChannelId,
        title: String,
        agent: Agent,
        machine: MachineId,
    ) -> Effect<DeskAction> {
        if state.primary_channel_desk(&channel).is_some()
            || state.channel_status.get(&channel) == Some(&ChannelDeskStatus::Starting)
        {
            return Effect::none();
        }
        state
            .channel_status
            .insert(channel.clone(), ChannelDeskStatus::Starting);
        let request = NewChannelDesk {
            channel: channel.clone(),
            target: None,
            title,
            agent,
            machine,
            switches: Switches::default(),
        };
        let client = self.client.clone();
        Effect::future(async move {
            DeskAction::PrimaryChannelStarted {
                channel,
                result: client.start_channel(request).await,
            }
        })
    }

    /// Reopens a channel's closed-kept primary Desk session in place without
    /// navigating away from the channel (CHANNELDESK).
    pub(super) fn reopen_primary_channel(
        &self,
        state: &mut DeskState,
        channel: ChannelId,
        session: SessionId,
    ) -> Effect<DeskAction> {
        if state.channel_status.get(&channel) == Some(&ChannelDeskStatus::Starting) {
            return Effect::none();
        }
        state
            .channel_status
            .insert(channel.clone(), ChannelDeskStatus::Starting);
        let client = self.client.clone();
        Effect::future(async move {
            let result = client.reopen(&session).await;
            DeskAction::PrimaryChannelReopened {
                channel,
                session,
                result,
            }
        })
    }

    /// Close session on a channel Desk session asks first (CD-4).
    pub(super) fn ask_close_channel(state: &mut DeskState, session: SessionId) {
        if state.sheet_open() {
            return;
        }
        let channel = state
            .workspace
            .desk
            .channel_sessions
            .get(&session)
            .map(|found| (found.channel.clone(), found.target.is_none()));
        state.channel_close = Some(ChannelCloseSheet {
            session,
            channel,
            copied: BTreeSet::new(),
            closing: false,
            error: None,
        });
    }

    pub(super) fn confirm_close_channel(&self, state: &mut DeskState) -> Effect<DeskAction> {
        let Some(session) = state
            .channel_close
            .as_ref()
            .filter(|sheet| !sheet.closing)
            .map(|sheet| sheet.session.clone())
        else {
            return Effect::none();
        };
        // SMB-012: do not mark the sheet `closing = true` when another change
        // for this session is already in flight, because `change()` would
        // refuse to send Close and leave the sheet permanently stuck.
        if state.pending.contains(&session) {
            if let Some(sheet) = state.channel_close.as_mut() {
                sheet.error =
                    Some("A change is still saving for this session. Try again in a moment.".to_owned());
            }
            return Effect::none();
        }
        if let Some(sheet) = state.channel_close.as_mut() {
            sheet.closing = true;
            sheet.error = None;
        }
        self.change(state, session, Change::Close, |client, id| client.close(id))
    }

    /// Copy to shared/ on the close sheet, for one Markdown file (CD-4,
    /// DK-13).
    pub(super) fn copy_to_shared(&self, state: &mut DeskState, path: String) -> Effect<DeskAction> {
        let Some(session) = state.channel_close.as_ref().map(|sheet| sheet.session.clone()) else {
            return Effect::none();
        };
        let Some(channel) = state.channel_record(&session).map(|found| found.channel.clone())
        else {
            return Effect::none();
        };
        let machine = state
            .workspace
            .session(&session)
            .map(|found| found.machine.clone());
        let document = humata2_clients::desk::Document {
            session,
            path: path.clone(),
        };
        let client = self.client.clone();
        Effect::future(async move {
            let result: Result<(), _> = client
                .copy_document(&document, &channel, machine.as_ref())
                .await;
            DeskAction::SharedCopied { path, result }
        })
    }

    pub(super) fn shared_copied(
        state: &mut DeskState,
        path: String,
        result: Result<(), humata2_clients::DaemonError>,
    ) -> Effect<DeskAction> {
        let Some(sheet) = state.channel_close.as_mut() else {
            return Effect::none();
        };
        match result {
            Ok(()) => {
                sheet.copied.insert(path.clone());
                Effect::send(DeskAction::Delegate(Delegate::Toast(format!(
                    "Copied {path} to shared/."
                ))))
            }
            Err(error) => {
                sheet.error = Some(error.to_string());
                Effect::none()
            }
        }
    }

    /// A close that came back: the sheet goes on success; on failure it
    /// stays, with what went wrong.
    pub(super) fn close_sheet_answered(
        state: &mut DeskState,
        session: &SessionId,
        result: &Result<(), humata2_clients::DaemonError>,
    ) {
        if !state
            .channel_close
            .as_ref()
            .is_some_and(|sheet| &sheet.session == session)
        {
            return;
        }
        match result {
            Ok(()) => state.channel_close = None,
            Err(error) => {
                if let Some(sheet) = state.channel_close.as_mut() {
                    sheet.closing = false;
                    sheet.error = Some(error.to_string());
                }
            }
        }
    }

    pub(super) fn set_switch(
        &self,
        state: &mut DeskState,
        session: SessionId,
        switch: Switch,
        on: bool,
    ) -> Effect<DeskAction> {
        let Some(current) = state.channel_record(&session).map(|found| found.switches) else {
            return Effect::none();
        };
        let next = current.with(switch, on);
        self.change(state, session, Change::Switches, move |client, id| {
            let reply: Reply<()> = client.set_switches(id, next);
            reply
        })
    }

    /// Picks a new target for a channel Desk session, or none (the whole
    /// channel) (CD-5). The pop-up closes. A session that isn't one of the
    /// channel's, or a closed Desk session, asks nothing of the daemon.
    pub(super) fn set_target(
        &self,
        state: &mut DeskState,
        session: SessionId,
        target: Option<SessionId>,
    ) -> Effect<DeskAction> {
        state.menu = None;
        let Some(record) = state.channel_record(&session) else {
            return Effect::none();
        };
        if record.desk.closed.is_some() {
            return Effect::none();
        }
        if let Some(wanted) = &target
            && !state
                .channel_members(&record.channel)
                .iter()
                .any(|member| &member.id == wanted)
        {
            return Effect::none();
        }
        self.change(state, session, Change::Target, move |client, id| {
            let reply: Reply<()> = client.set_target(id, target.as_ref());
            reply
        })
    }
}
