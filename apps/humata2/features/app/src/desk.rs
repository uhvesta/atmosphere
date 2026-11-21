//! The Desk in the app (DK-1, DK-5, DK-11, DK-14): New desk session from
//! anywhere, what the Desk feature asks the app to do, leaving a Desk
//! session's page once it is closed, and when the sidebar lists the open
//! Desk sessions under its Desk row.

use humata2_clients::desk;
use humata2_clients::{FileRoot, Folder};
use humata2_clients::model::{ChannelId, ForkKind, MachineId, SessionId};
use humata2_desk::{Change, ChannelDeskStatus, Delegate, DeskAction, DeskState, Sort};
use humata2_files::FilesAction;
use humata2_session::SessionAction;
use tca::Effect;

use crate::{App, AppAction, AppState, Route};

/// The Desk as the last run left it: only its sort is saved.
pub(super) fn restored(sort: Sort) -> DeskState {
    DeskState {
        sort,
        ..DeskState::default()
    }
}

impl AppState {
    /// Whether `id` is a Desk session.
    #[must_use]
    pub fn is_desk_session(&self, id: &SessionId) -> bool {
        self.workspace.session(id).is_some_and(desk::is_desk)
    }

    /// The primary open Channel Desk session currently shown inline on a
    /// channel's Desk tab (`CHANNELDESK`), if any.
    #[must_use]
    pub fn active_channel_desk(&self) -> Option<SessionId> {
        let Route::Channel(channel) = &self.route else {
            return None;
        };
        let tab = self
            .channel_pages
            .get(channel)
            .map(|view| view.tab)
            .unwrap_or_default();
        if tab != crate::channel::Tab::Desk {
            return None;
        }
        self.desk
            .primary_channel_desk(channel)
            .map(|row| row.session.id.clone())
    }
}

impl App {
    /// Ensures `channel` has a persistent primary Desk session (`target: None`)
    /// ready to talk to inline on `Tab::Desk` (`CHANNELDESK`).
    ///
    /// - Reuses an already-open primary Desk with 0 RPCs.
    /// - Deduplicates concurrent calls while startup is in flight.
    /// - When `force` is false, respects an explicit Close or a closed-kept
    ///   primary Desk so the UI can offer Reopen / Start fresh instead of
    ///   recreating immediately.
    pub(super) fn ensure_channel_desk(
        &self,
        state: &mut AppState,
        channel: &ChannelId,
        force: bool,
    ) -> Effect<AppAction> {
        let Some(found) = state.workspace.channel(channel).cloned() else {
            return Effect::none();
        };
        if !force {
            if state.desk.channel_status.get(channel) == Some(&ChannelDeskStatus::Closed) {
                return Effect::none();
            }
            if state.desk.closed_primary_channel_desk(channel).is_some()
                && state.desk.primary_channel_desk(channel).is_none()
            {
                return Effect::none();
            }
        }
        if state.desk.primary_channel_desk(channel).is_some() {
            state.desk.channel_status.remove(channel);
            return Effect::none();
        }
        if state.desk.channel_status.get(channel) == Some(&ChannelDeskStatus::Starting) {
            return Effect::none();
        }
        let Ok(agent) = state.default_agent(Some(channel)) else {
            state
                .desk
                .channel_status
                .insert(channel.clone(), ChannelDeskStatus::NoAgent);
            return Effect::none();
        };
        let online = |id: &MachineId| {
            state
                .workspace
                .machine(id)
                .is_some_and(|machine| machine.online)
        };
        let machine = found
            .machines
            .iter()
            .find(|id| online(id))
            .cloned()
            .or_else(|| {
                state
                    .workspace
                    .machines
                    .iter()
                    .find(|machine| machine.online)
                    .map(|machine| machine.id.clone())
            });
        let Some(machine) = machine else {
            state
                .desk
                .channel_status
                .insert(channel.clone(), ChannelDeskStatus::NoMachine);
            return Effect::none();
        };
        let title = format!("#{} Desk", found.name);
        self.child(
            state,
            AppAction::Desk(DeskAction::EnsurePrimaryChannel {
                channel: channel.clone(),
                title,
                agent,
                machine,
            }),
        )
    }

    /// New desk session, from Home, the Desk page or a command: the form
    /// starts on the agent Settings picked.
    pub(super) fn start_new_desk_session(&self, state: &mut AppState) -> Effect<AppAction> {
        let Ok(agent) = state.default_agent(None) else {
            let clock = self.clients.clock.clone();
            return state.toast_no_agent(&clock);
        };
        self.child(state, AppAction::Desk(DeskAction::StartNew { agent }))
    }

    /// New desk session for this session, from a session's menu (CD-6): the
    /// channel's dialog with that session as the target. A Desk session has
    /// no channel to look after, so it starts nothing there.
    pub(super) fn new_desk_session_for(
        &self,
        state: &mut AppState,
        id: &SessionId,
    ) -> Effect<AppAction> {
        if state.is_desk_session(id) {
            return Effect::none();
        }
        let Some(channel) = state
            .workspace
            .session(id)
            .map(|session| session.channel.clone())
        else {
            return Effect::none();
        };
        let Ok(agent) = state.default_agent(Some(&channel)) else {
            let clock = self.clients.clock.clone();
            return state.toast_no_agent(&clock);
        };
        let menu = self.child(state, AppAction::Session(SessionAction::Menu(false)));
        let dialog = self.child(
            state,
            AppAction::Desk(DeskAction::StartChannelNew {
                channel,
                target: Some(id.clone()),
                agent,
            }),
        );
        Effect::merge([menu, dialog])
    }

    /// Does what the Desk asked for, and leaves a Desk session's page for
    /// the Desk once that session is closed.
    pub(super) fn desk_delegate(
        &self,
        state: &mut AppState,
        action: &DeskAction,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            DeskAction::Delegate(Delegate::OpenSession(id)) => {
                self.navigate(state, Route::Session(id.clone()))
            }
            // D382: a Desk session has no repository, so its folder is
            // opened as the session's own, on the machine it runs on.
            DeskAction::Delegate(Delegate::OpenFolder(id)) => {
                let Some(machine) = state.workspace.session(id).map(|session| session.machine.clone())
                else {
                    return Effect::none();
                };
                let go = self.navigate(state, Route::Files);
                let side = state.files.active;
                let folder = Folder::root(machine, FileRoot::SessionFolder { session: id.clone() });
                let show = self.child(state, AppAction::Files(FilesAction::Navigate(side, folder)));
                Effect::merge([go, show])
            }
            DeskAction::Delegate(Delegate::Toast(text)) => state.toast(text.clone(), &clock),
            // AGENTOFF: a form opened on an agent that is off (the channel
            // tab sends Settings' pick) starts on one that is on, or not at
            // all when none is.
            // CD-4: closing a channel's Desk session from its own page goes
            // back to that channel's Desk tab without respawning it.
            DeskAction::Changed {
                session,
                change: Change::Close,
                result: Ok(()),
            } if self.channel_desk_close_info(state, session).is_some() => {
                self.finish_channel_desk_close(state, session)
            }
            DeskAction::Changed {
                session,
                change: Change::Close,
                result: Ok(()),
            } if state.route == Route::Session(session.clone()) => {
                let gone = Route::Session(session.clone());
                self.leave(state, &gone, Route::Desk)
            }
            // REPOS (RL-5, DK-14): New desk session's repo search opens on the library.
            DeskAction::New(humata2_desk::NewAction::AddingRepo(true)) => {
                self.open_new_desk_repo_search(state)
            }
            // REPOS (RL-3, RL-5): Add repo opens on the library.
            DeskAction::Menu(Some(humata2_desk::Menu::AddRepo(_))) => {
                self.open_desk_repo_search(state)
            }
            // The daemon's Desk events keep the Desk current; after the
            // app's own change it still reads again, which also brings the
            // session rows a start or a close changes.
            DeskAction::Started(Ok(_))
            | DeskAction::PrimaryChannelStarted { result: Ok(_), .. }
            | DeskAction::PrimaryChannelReopened { result: Ok(()), .. }
            | DeskAction::Changed { result: Ok(()), .. }
            | DeskAction::Copied(Ok(()))
            | DeskAction::CleanedUp(Ok(_)) => self.load(),
            // The Desk page shows its own error; anywhere else (a session's
            // folder bar) a failed change has only the toast to say so.
            DeskAction::Changed {
                result: Err(error), ..
            } if state.route != Route::Desk => state.toast(error.to_string(), &clock),
            _ => Effect::none(),
        }
    }
    /// Resolves `(channel, is_primary)` for a channel Desk session being closed,
    /// checking both the workspace and `channel_close` in case a daemon event
    /// removed the session from the workspace before the close RPC returned.
    fn channel_desk_close_info(
        &self,
        state: &AppState,
        session: &SessionId,
    ) -> Option<(ChannelId, bool)> {
        state
            .workspace
            .desk
            .channel_sessions
            .get(session)
            .map(|found| (found.channel.clone(), found.target.is_none()))
            .or_else(|| {
                state
                    .desk
                    .channel_close
                    .as_ref()
                    .filter(|sheet| &sheet.session == session)
                    .and_then(|sheet| sheet.channel.clone())
            })
    }

    /// Marks a closed channel Desk session closed in local state, navigates off
    /// its full-session route back to the channel's Desk tab without respawning
    /// it, and triggers a workspace reload.
    fn finish_channel_desk_close(
        &self,
        state: &mut AppState,
        session: &SessionId,
    ) -> Effect<AppAction> {
        let Some((channel, is_primary)) = self.channel_desk_close_info(state, session) else {
            return Effect::none();
        };
        if is_primary {
            state
                .desk
                .channel_status
                .insert(channel.clone(), ChannelDeskStatus::Closed);
        }
        // Immediately reflect the close in `state.workspace` so intervening
        // events or `arrive()` cannot see the closed session as an open primary
        // and wipe `ChannelDeskStatus::Closed` before `self.load()` returns.
        let workspace = std::sync::Arc::make_mut(&mut state.workspace);
        if let Some(record) = workspace.desk.channel_sessions.get_mut(session) {
            if record.desk.version_count > 0 {
                record.desk.closed = Some(humata2_clients::desk::Closed {
                    when: "just now".to_owned(),
                    days_left: None,
                });
            } else {
                workspace.desk.channel_sessions.remove(session);
            }
        }
        if state.route == Route::Session(session.clone()) {
            state.channel_pages.entry(channel.clone()).or_default().tab =
                crate::channel::Tab::Desk;
            let gone = Route::Session(session.clone());
            let go = self.leave(state, &gone, Route::Channel(channel));
            Effect::merge([go, self.load()])
        } else {
            self.load()
        }
    }

    /// Handles `SessionAction::DeskEnded` (e.g. closing a Desk session via the
    /// terminal tab's `End for good` action) so primary channel Desks stay
    /// closed and leave their full-session route cleanly.
    pub(super) fn desk_session_ended(
        &self,
        state: &mut AppState,
        session: &SessionId,
    ) -> Effect<AppAction> {
        if self.channel_desk_close_info(state, session).is_some() {
            return self.finish_channel_desk_close(state, session);
        }
        if state.route == Route::Session(session.clone()) {
            let gone = Route::Session(session.clone());
            let go = self.leave(state, &gone, Route::Desk);
            Effect::merge([go, self.load()])
        } else {
            self.load()
        }
    }
}

impl App {
    /// Whether `agent` can be started: the shell always, others when on.
    /// AGENTOFF (D915): a Desk form the Desk just opened on an agent that is
    /// off moves to the default agent that is on, or closes with a toast when
    /// none is. Runs after the Desk's reducer, once the form exists.
    pub(super) fn desk_form_agent(&self, state: &mut AppState, action: &DeskAction) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            DeskAction::StartNew { .. } => {
                let off = state
                    .desk
                    .new
                    .as_ref()
                    .is_some_and(|form| !Self::agent_is_on(state, form.agent));
                if !off {
                    return Effect::none();
                }
                match state.default_agent(None) {
                    Ok(agent) => {
                        if let Some(form) = state.desk.new.as_mut() {
                            form.agent = agent;
                        }
                        Effect::none()
                    }
                    Err(_) => {
                        state.desk.new = None;
                        state.toast_no_agent(&clock)
                    }
                }
            }
            DeskAction::StartChannelNew { channel, .. } => {
                let off = state
                    .desk
                    .channel_new
                    .as_ref()
                    .is_some_and(|form| !Self::agent_is_on(state, form.agent));
                if !off {
                    return Effect::none();
                }
                match state.default_agent(Some(channel)) {
                    Ok(agent) => {
                        if let Some(form) = state.desk.channel_new.as_mut() {
                            form.agent = agent;
                        }
                        Effect::none()
                    }
                    Err(_) => {
                        state.desk.channel_new = None;
                        state.toast_no_agent(&clock)
                    }
                }
            }
            _ => Effect::none(),
        }
    }

    /// AGENTOFF (D915 5): Continue in another channel restarts the session's
    /// own agent, so with that agent off its dialog closes and a toast points
    /// to Settings › Agents. It runs after every action, so it covers the
    /// dialog opening, the agent turned off while it is open, and the
    /// daemon's refusal once that answer is in.
    pub(super) fn continue_unless_agent_off(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.session.continuing.as_ref().is_none_or(|draft| draft.sending) {
            return Effect::none();
        }
        let Some(agent) = state
            .session
            .open
            .as_ref()
            .and_then(|id| state.workspace.session(id))
            .map(|session| session.agent)
        else {
            return Effect::none();
        };
        if Self::agent_is_on(state, agent) {
            return Effect::none();
        }
        state.session.continuing = None;
        state.session.error = None;
        let text = format!(
            "{} is off, so this session can't continue in another channel. Turn it on in Settings › Agents, then try again.",
            agent.name()
        );
        let clock = self.clients.clock.clone();
        state.toast_agent_off(text, &clock)
    }

    fn agent_is_on(state: &AppState, agent: humata2_clients::model::Agent) -> bool {
        agent == humata2_clients::model::Agent::Shell
            || state.settings.offered_agents().contains(&agent)
    }

    /// The open session, when it is a Desk session.
    fn open_desk_session(state: &AppState) -> Option<SessionId> {
        state
            .session
            .open
            .clone()
            .filter(|id| state.is_desk_session(id))
    }

    /// Archive, from the palette or a shortcut. On a Desk session it is
    /// Close session (DK-11): the folder stays and the session goes under
    /// Closed on the Desk.
    pub(super) fn archive_or_close(&self, state: &mut AppState) -> Effect<AppAction> {
        match Self::open_desk_session(state) {
            Some(id) => self.handle(state, AppAction::Desk(DeskAction::AskClose(id))),
            None => self.child(state, AppAction::Session(SessionAction::Archive)),
        }
    }

    /// Archives `id` (or asks to close it on the Desk when it is a Desk
    /// session) without navigating away from the current screen.
    pub(super) fn archive_session(
        &self,
        state: &mut AppState,
        id: SessionId,
    ) -> Effect<AppAction> {
        self.archive_session_with_end(state, id, false)
    }

    /// Like [`Self::archive_session`], and also ends the session's running
    /// agent and shells (D801 F) so its row leaves the Machine page.
    pub(super) fn archive_session_and_end(
        &self,
        state: &mut AppState,
        id: SessionId,
    ) -> Effect<AppAction> {
        self.archive_session_with_end(state, id, true)
    }

    fn archive_session_with_end(
        &self,
        state: &mut AppState,
        id: SessionId,
        end: bool,
    ) -> Effect<AppAction> {
        if state.is_desk_session(&id) {
            return self.handle(state, AppAction::Desk(DeskAction::AskClose(id)));
        }
        self.child(
            state,
            AppAction::Session(SessionAction::ArchiveSession { session: id, end }),
        )
    }

    /// Fork, from the palette or a shortcut. A Desk session has none
    /// (DK-12).
    pub(super) fn fork_unless_desk(
        &self,
        state: &mut AppState,
        kind: ForkKind,
    ) -> Effect<AppAction> {
        if Self::open_desk_session(state).is_some() {
            return Effect::none();
        }
        let forked = self.child(state, AppAction::Session(SessionAction::StartFork(kind)));
        // AGENTOFF: the fork keeps the parent's agent unless it is off; then
        // it opens on one that is on with More options showing it, or not at
        // all when none is.
        let parent_agent = state
            .session
            .open
            .as_ref()
            .and_then(|id| state.workspace.session(id))
            .map(|session| session.agent);
        if state.session.fork.is_some() && parent_agent.is_some_and(|agent| !Self::agent_is_on(state, agent)) {
            match humata2_clients::pick_agent(&state.settings.offered_agents(), &[]) {
                Ok(agent) => {
                    if let Some(fork) = state.session.fork.as_mut() {
                        fork.agent = Some(agent);
                        fork.more = true;
                    }
                }
                Err(_) => {
                    state.session.fork = None;
                    let clock = self.clients.clock.clone();
                    return Effect::merge([forked, state.toast_no_agent(&clock)]);
                }
            }
        }
        forked
    }
}
