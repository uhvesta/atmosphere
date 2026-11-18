//! Closing a terminal tab asks what happens to what runs in it (D784): Keep
//! running (hide, the default), Stop (resumable) or End for good. An agent's
//! tab stops through StopSession and ends through Archive; a shell's stops,
//! resumes and ends through the machine's running-holder calls.

use humata2_clients::model::{Agent, MachineId, MachineKind, Session, SessionId};
use humata2_clients::{DaemonError, HolderTab};
use tca::{Effect, Reducer};

use crate::{Delegate, SessionAction, SessionState, Sessions, Tab, open_session};

/// What the tab being closed runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseKind {
    /// The session's agent, tab 0: Stop stops the session, End for good
    /// archives it.
    Agent,
    /// A shell: the session's own when it runs a shell, or an extra shell
    /// tab.
    Shell,
    /// An extra tab running an agent. Stop and End for good act on a whole
    /// session, so only Keep running is offered.
    ExtraAgent,
}

/// One of the dialog's three ways to close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseChoice {
    /// Hide the tab; it keeps running. The default, on Return.
    KeepRunning,
    /// End it now, resumable later.
    Stop,
    /// End it and, for an agent, archive the session.
    EndForGood,
}

/// A key pressed while the dialog is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseKey {
    /// Picks Keep running.
    Return,
    /// Cancels.
    Escape,
}

/// The close dialog for a terminal tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseConfirm {
    /// The session whose tab it is.
    pub session: SessionId,
    /// The tab's group and its place there.
    pub group: usize,
    /// Its place in the group.
    pub index: usize,
    /// The terminal tab: 0 is the agent's.
    pub tab: usize,
    /// What the tab runs.
    pub kind: CloseKind,
    /// What to call it: the session's title, `None` for an untitled shell.
    pub title: Option<String>,
    /// The program, as people know it: "Claude Code", "Copilot", "Shell".
    pub agent: String,
    /// The machine, "This Mac" for this one.
    pub machine: String,
    /// The "Always keep running without asking" box.
    pub always_keep_running: bool,
    /// The choice on its way to the daemon.
    pub sending: Option<CloseChoice>,
    /// Why the last choice failed.
    pub error: Option<String>,
}

impl CloseConfirm {
    /// The choices the dialog offers, left to right of the default.
    #[must_use]
    pub fn choices(&self) -> Vec<CloseChoice> {
        match self.kind {
            CloseKind::ExtraAgent => vec![CloseChoice::KeepRunning],
            CloseKind::Agent | CloseKind::Shell => {
                vec![CloseChoice::KeepRunning, CloseChoice::Stop, CloseChoice::EndForGood]
            }
        }
    }

    /// The choice that has focus: always Keep running, never Stop or End.
    #[must_use]
    pub const fn focused(&self) -> CloseChoice {
        CloseChoice::KeepRunning
    }

    /// The title as a toast says it: "the shell" for an untitled shell.
    #[must_use]
    pub fn named(&self) -> String {
        self.title.clone().unwrap_or_else(|| "the shell".to_owned())
    }
}

/// What a close toast's button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseThen {
    /// Open: show the hidden tab again.
    Open {
        /// The session.
        session: SessionId,
        /// Its terminal tab.
        tab: usize,
    },
    /// Resume a stopped session's agent.
    ResumeAgent(SessionId),
    /// Resume a stopped shell tab.
    ResumeShell {
        /// The session.
        session: SessionId,
        /// Its terminal tab.
        tab: usize,
    },
    /// Undo End for good: restore the session as Stopped, never running.
    RestoreStopped(SessionId),
}

/// A toast the close flow asks the parent to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseNotice {
    /// What it says.
    pub text: String,
    /// Its button.
    pub then: CloseThen,
}

/// What happens in the close dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum CloseAction {
    /// The tab's close was clicked: ask, unless it isn't a terminal or the
    /// box was ticked before.
    Ask {
        /// The tab's group.
        group: usize,
        /// Its place in the group.
        index: usize,
    },
    /// A button.
    Choose(CloseChoice),
    /// A key.
    Key(CloseKey),
    /// Cancel: the tab stays.
    Cancel,
    /// The "Always keep running without asking" box.
    AlwaysKeepRunning(bool),
    /// The same preference from Settings, or as saved.
    SetAlwaysKeepRunning(bool),
    /// Stop came back.
    Stopped(Result<(), DaemonError>),
    /// End for good of a shell came back.
    Ended(Result<(), DaemonError>),
    /// A close toast's button.
    Then(CloseThen),
    /// A toast button's call came back.
    ThenDone(Result<(), DaemonError>),
}

/// The program's name in the dialog: "Claude Code" rather than "Claude".
fn agent_name(agent: Agent) -> String {
    match agent {
        Agent::Claude => "Claude Code".to_owned(),
        other => other.name().to_owned(),
    }
}

fn machine_name(state: &SessionState, machine: &MachineId) -> String {
    match state.workspace.machine(machine) {
        Some(found) if matches!(found.kind, MachineKind::ThisMac) => "This Mac".to_owned(),
        Some(found) => found.name.clone(),
        None => machine.as_str().to_owned(),
    }
}

/// The dialog for terminal tab `tab` of `session`.
fn confirm(state: &SessionState, session: &Session, group: usize, index: usize, tab: usize) -> CloseConfirm {
    let extra = (tab > 0).then(|| session.terminal_agents.get(&tab).copied()).flatten();
    let (kind, agent) = match (tab, extra) {
        (0, _) if session.agent == Agent::Shell => (CloseKind::Shell, Agent::Shell),
        (0, _) => (CloseKind::Agent, session.agent),
        (_, Some(agent)) if agent != Agent::Shell => (CloseKind::ExtraAgent, agent),
        _ => (CloseKind::Shell, Agent::Shell),
    };
    let title = match kind {
        CloseKind::Shell if tab > 0 => None,
        _ => Some(session.title.clone()).filter(|title| !title.is_empty()),
    };
    CloseConfirm {
        session: session.id.clone(),
        group,
        index,
        tab,
        kind,
        title,
        agent: agent_name(agent),
        machine: machine_name(state, &session.machine),
        always_keep_running: false,
        sending: None,
        error: None,
    }
}

impl Sessions {
    pub(crate) fn close(&self, state: &mut SessionState, action: CloseAction) -> Effect<SessionAction> {
        match action {
            CloseAction::Ask { group, index } => return self.ask_close(state, group, index),
            CloseAction::Key(CloseKey::Return) => {
                return self.close(state, CloseAction::Choose(CloseChoice::KeepRunning));
            }
            CloseAction::Key(CloseKey::Escape) | CloseAction::Cancel => {
                if state.closing.as_ref().is_some_and(|confirm| confirm.sending.is_none()) {
                    state.closing = None;
                }
            }
            CloseAction::AlwaysKeepRunning(on) => {
                if let Some(confirm) = &mut state.closing {
                    confirm.always_keep_running = on;
                }
            }
            CloseAction::SetAlwaysKeepRunning(on) => state.always_keep_running = on,
            CloseAction::Choose(choice) => return self.choose_close(state, choice),
            CloseAction::Stopped(result) => return self.close_finished(state, result, true),
            CloseAction::Ended(result) => return self.close_finished(state, result, false),
            CloseAction::Then(then) => return self.close_then(state, then),
            CloseAction::ThenDone(result) => {
                if let Err(error) = result {
                    state.fail(&error);
                }
            }
        }
        Effect::none()
    }

    fn ask_close(&self, state: &mut SessionState, group: usize, index: usize) -> Effect<SessionAction> {
        let tab = state.view().and_then(|view| view.groups.get(group)).and_then(|g| g.tabs.get(index)).cloned();
        let (Some(Tab::Terminal(tab)), Some(session)) = (tab, open_session(state)) else {
            return self.reduce(state, SessionAction::CloseTab { group, index });
        };
        let confirm = confirm(state, session, group, index, tab);
        if !state.always_keep_running {
            state.closing = Some(confirm);
            return Effect::none();
        }
        let closed = self.reduce(state, SessionAction::CloseTab { group, index });
        let notice = CloseNotice {
            text: format!("{} is still running on {}.", confirm.named(), confirm.machine),
            then: CloseThen::Open {
                session: confirm.session,
                tab,
            },
        };
        Effect::merge([closed, Effect::send(SessionAction::Delegate(Delegate::CloseNotice(notice)))])
    }

    fn choose_close(&self, state: &mut SessionState, choice: CloseChoice) -> Effect<SessionAction> {
        let Some(confirm) = state.closing.clone() else {
            return Effect::none();
        };
        if confirm.sending.is_some() || !confirm.choices().contains(&choice) {
            return Effect::none();
        }
        if choice == CloseChoice::KeepRunning {
            if confirm.always_keep_running {
                state.always_keep_running = true;
            }
            state.closing = None;
            return self.reduce(
                state,
                SessionAction::CloseTab {
                    group: confirm.group,
                    index: confirm.index,
                },
            );
        }
        let Some(session) = state.workspace.session(&confirm.session).cloned() else {
            return Effect::none();
        };
        // An offline machine is never asked: nothing is queued (D784).
        if state.workspace.machine(&session.machine).is_some_and(|machine| !machine.online) {
            if let Some(confirm) = &mut state.closing {
                confirm.error = Some(DaemonError::Offline { machine: session.machine.clone() }.to_string());
            }
            return Effect::none();
        }
        if (choice, confirm.kind) == (CloseChoice::EndForGood, CloseKind::Agent) {
            // End for good is Archive, frame 65's unpushed-work dialog
            // included; its toast's Undo restores the session as Stopped.
            state.closing = None;
            state.ending_for_good = Some(confirm.session.clone());
            state.open = Some(confirm.session);
            return self.reduce(state, SessionAction::Archive);
        }
        if let Some(open) = &mut state.closing {
            open.sending = Some(choice);
            open.error = None;
        }
        let daemon = self.clients.daemon.clone();
        let (id, machine, tab) = (confirm.session.clone(), session.machine.clone(), confirm.tab);
        let holder = HolderTab::Session { session: id.clone(), index: tab };
        match (choice, confirm.kind) {
            (CloseChoice::Stop, CloseKind::Agent) => {
                Effect::future(async move { SessionAction::Close(CloseAction::Stopped(daemon.stop_agent(&id).await)) })
            }
            (CloseChoice::Stop, _) => Effect::future(async move {
                SessionAction::Close(CloseAction::Stopped(daemon.stop_running_holder(&machine, &holder).await))
            }),
            _ => Effect::future(async move {
                SessionAction::Close(CloseAction::Ended(daemon.end_running_holder(&machine, &holder).await))
            }),
        }
    }

    fn close_finished(
        &self,
        state: &mut SessionState,
        result: Result<(), DaemonError>,
        stopped: bool,
    ) -> Effect<SessionAction> {
        let Some(confirm) = state.closing.clone() else {
            return Effect::none();
        };
        if let Err(error) = result {
            if let Some(open) = &mut state.closing {
                open.sending = None;
                open.error = Some(error.to_string());
            }
            return Effect::none();
        }
        state.closing = None;
        let closed = self.reduce(
            state,
            SessionAction::CloseTab {
                group: confirm.group,
                index: confirm.index,
            },
        );
        if !stopped {
            return closed;
        }
        let then = match confirm.kind {
            CloseKind::Agent => CloseThen::ResumeAgent(confirm.session.clone()),
            CloseKind::Shell | CloseKind::ExtraAgent => CloseThen::ResumeShell {
                session: confirm.session.clone(),
                tab: confirm.tab,
            },
        };
        let notice = CloseNotice {
            text: format!("Stopped {} on {}.", confirm.named(), confirm.machine),
            then,
        };
        Effect::merge([closed, Effect::send(SessionAction::Delegate(Delegate::CloseNotice(notice)))])
    }

    fn close_then(&self, state: &mut SessionState, then: CloseThen) -> Effect<SessionAction> {
        let daemon = self.clients.daemon.clone();
        match then {
            CloseThen::Open { session, tab } => {
                let shells = state.workspace.session(&session).map_or(0, |found| found.shells.len());
                let view = state.views.entry(session.clone()).or_default();
                view.ensure_groups(shells);
                view.show_tab(Tab::Terminal(tab), view.focused_group, false);
                Effect::send(SessionAction::Delegate(Delegate::OpenSession(session)))
            }
            CloseThen::ResumeAgent(session) => {
                Effect::future(async move { SessionAction::Close(CloseAction::ThenDone(daemon.resume(&session).await)) })
            }
            CloseThen::ResumeShell { session, tab } => {
                let Some(machine) = state.workspace.session(&session).map(|found| found.machine.clone()) else {
                    return Effect::none();
                };
                let holder = HolderTab::Session { session, index: tab };
                Effect::future(async move {
                    SessionAction::Close(CloseAction::ThenDone(daemon.resume_running_holder(&machine, &holder).await))
                })
            }
            CloseThen::RestoreStopped(session) => Effect::future(async move {
                SessionAction::Close(CloseAction::ThenDone(daemon.restore_session_stopped(&session).await))
            }),
        }
    }
}

/// The toast after End for good archived `title` on `machine`.
pub(crate) fn ended_text(title: &str, machine: &str) -> String {
    format!("Ended and archived {title} on {machine}.")
}

/// `session`'s agent and machine, as the dialog names them; empty when the
/// session isn't in the workspace.
pub(crate) fn agent_and_machine(state: &SessionState, session: &SessionId) -> (String, String) {
    state.workspace.session(session).map_or_else(Default::default, |found| {
        (agent_name(found.agent), machine_name(state, &found.machine))
    })
}

/// `session`'s machine, as the dialog names it.
pub(crate) fn machine_of(state: &SessionState, session: &SessionId) -> String {
    state
        .workspace
        .session(session)
        .map_or_else(String::new, |found| machine_name(state, &found.machine))
}
