//! The machine page's running holders (D784, frame 59's "Sessions on it"):
//! every terminal of ours on the machine, its sessions' rows first and the
//! ones no channel has under "Not in a channel", with each row's ⋯ menu.
//!
//! A running row offers Open, Stop and End for good… (one confirm); a
//! stopped one Resume and Archive…; one no channel has Open, Adopt… and End
//! for good…. Stop asks nothing. An offline machine offers none of them:
//! its page keeps only Reconnect, and nothing is queued.

use std::sync::Arc;

use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::{Daemon, DaemonError, HolderTab, RunningHolder};
use tca::{Effect, Reducer};

const LIST: &str = "machine-page-running";

/// What a row's ⋯ menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    /// Open it in a tab.
    Open,
    /// End it now, resumable later; asks nothing.
    Stop,
    /// End it for good, after one confirm: a session is archived, a shell
    /// ends.
    EndForGood,
    /// Start a stopped one again.
    Resume,
    /// Archive a stopped session (its own confirm, frame 65's included).
    Archive,
    /// Give a terminal no channel has a channel or the Desk.
    Adopt,
}

/// The running list's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunningState {
    /// The machine listed.
    pub machine: Option<MachineId>,
    /// What it last listed, in the daemon's order.
    pub holders: Vec<RunningHolder>,
    /// A list is on its way.
    pub loading: bool,
    /// The machine doesn't answer: rows offer nothing until it reconnects.
    pub offline: bool,
    /// The End for good… confirm, for this row.
    pub confirm_end: Option<RunningHolder>,
    /// The Adopt… picker, for this row.
    pub adopting: Option<RunningHolder>,
    /// A row whose call is on its way.
    pub busy: Option<RunningHolder>,
    /// Why the last list or row action failed.
    pub error: Option<String>,
}

impl RunningState {
    /// The rows of its sessions, running then stopped.
    #[must_use]
    pub fn sessions(&self) -> Vec<&RunningHolder> {
        self.holders.iter().filter(|holder| !holder.is_orphan()).collect()
    }

    /// The "Not in a channel" group: `None` when there are none, so the
    /// group isn't drawn.
    #[must_use]
    pub fn not_in_a_channel(&self) -> Option<Vec<&RunningHolder>> {
        let orphans: Vec<_> = self.holders.iter().filter(|holder| holder.is_orphan()).collect();
        (!orphans.is_empty()).then_some(orphans)
    }

    /// What `holder`'s ⋯ menu offers; nothing while the machine is offline.
    #[must_use]
    pub fn row_actions(&self, holder: &RunningHolder) -> Vec<RowAction> {
        if self.offline {
            return Vec::new();
        }
        match (holder.is_orphan(), holder.stopped) {
            (false, false) => vec![RowAction::Open, RowAction::Stop, RowAction::EndForGood],
            (false, true) if is_shell(holder) => vec![RowAction::Resume, RowAction::EndForGood],
            (false, true) => vec![RowAction::Resume, RowAction::Archive],
            (true, false) => vec![RowAction::Open, RowAction::Adopt, RowAction::EndForGood],
            (true, true) => vec![RowAction::Resume, RowAction::EndForGood],
        }
    }
}

/// A shell's row goes through the running-holder calls by its tab; an
/// agent's through its session.
fn is_shell(holder: &RunningHolder) -> bool {
    holder.is_orphan() || (holder.agent == Agent::Shell && holder.tab.is_some())
}

/// Where Adopt… puts a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdoptTarget {
    /// A channel.
    Channel(humata2_clients::model::ChannelId),
    /// The Desk.
    Desk,
}

/// What the running list asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunningDelegate {
    /// Open this one in a tab: its session when it has one, else attach by
    /// its tab.
    Open {
        /// The machine.
        machine: MachineId,
        /// Its session, if one names it.
        session: Option<SessionId>,
        /// Its tab, as the daemon listed it.
        tab: Option<String>,
    },
    /// Archive this session, through the session page's own Archive (frame
    /// 65's unpushed-work dialog included).
    ArchiveSession(SessionId),
}

/// What happens in the running list.
#[derive(Clone, Debug, PartialEq)]
pub enum RunningAction {
    /// List this machine's holders.
    Load(MachineId),
    /// A list came back.
    Loaded(MachineId, Result<Vec<RunningHolder>, DaemonError>),
    /// A row's ⋯ menu pick.
    Row(RunningHolder, RowAction),
    /// Confirm End for good….
    ConfirmEnd,
    /// Cancel End for good….
    CancelEnd,
    /// Adopt… picked a place.
    ConfirmAdopt(AdoptTarget),
    /// Cancel Adopt….
    CancelAdopt,
    /// A row's call came back.
    Done(Result<(), DaemonError>),
    /// For the parent.
    Delegate(RunningDelegate),
}

/// The running list's reducer.
#[derive(Clone)]
pub struct Running {
    daemon: Arc<dyn Daemon>,
}

impl Running {
    /// A list that reads and changes holders through `daemon`.
    #[must_use]
    pub fn new(daemon: Arc<dyn Daemon>) -> Self {
        Self { daemon }
    }

    fn load(&self, state: &mut RunningState, machine: MachineId) -> Effect<RunningAction> {
        state.loading = true;
        let call = self.daemon.running_holders(&machine);
        Effect::future(async move { RunningAction::Loaded(machine, call.await) }).cancellable(LIST, true)
    }

    fn call(&self, state: &mut RunningState, holder: RunningHolder, call: humata2_clients::Reply<()>) -> Effect<RunningAction> {
        state.busy = Some(holder);
        state.error = None;
        Effect::future(async move { RunningAction::Done(call.await) })
    }

    fn row(&self, state: &mut RunningState, holder: RunningHolder, action: RowAction) -> Effect<RunningAction> {
        if state.busy.is_some() || !state.row_actions(&holder).contains(&action) {
            return Effect::none();
        }
        let machine = holder.machine.clone();
        let tab = holder.tab.clone().map(HolderTab::Listed);
        match (action, &holder.session, tab) {
            (RowAction::Open, session, _) => Effect::send(RunningAction::Delegate(RunningDelegate::Open {
                machine,
                session: session.clone(),
                tab: holder.tab.clone(),
            })),
            (RowAction::Stop, _, Some(tab)) if is_shell(&holder) => {
                let call = self.daemon.stop_running_holder(&machine, &tab);
                self.call(state, holder, call)
            }
            (RowAction::Stop, Some(session), _) => {
                let call = self.daemon.stop_agent(session);
                self.call(state, holder, call)
            }
            (RowAction::Resume, _, Some(tab)) if is_shell(&holder) => {
                let call = self.daemon.resume_running_holder(&machine, &tab);
                self.call(state, holder, call)
            }
            (RowAction::Resume, Some(session), _) => {
                let call = self.daemon.resume(session);
                self.call(state, holder, call)
            }
            (RowAction::Archive, Some(session), _) => {
                Effect::send(RunningAction::Delegate(RunningDelegate::ArchiveSession(session.clone())))
            }
            (RowAction::EndForGood, _, _) => {
                state.confirm_end = Some(holder);
                Effect::none()
            }
            (RowAction::Adopt, _, _) => {
                state.adopting = Some(holder);
                Effect::none()
            }
            _ => Effect::none(),
        }
    }
}

impl Reducer for Running {
    type State = RunningState;
    type Action = RunningAction;

    fn reduce(&self, state: &mut RunningState, action: RunningAction) -> Effect<RunningAction> {
        use RunningAction as A;
        match action {
            A::Load(machine) => {
                if state.machine.as_ref() != Some(&machine) {
                    *state = RunningState {
                        machine: Some(machine.clone()),
                        ..RunningState::default()
                    };
                }
                return self.load(state, machine);
            }
            A::Loaded(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                state.loading = false;
                match result {
                    Ok(holders) => {
                        state.holders = holders;
                        state.offline = false;
                        state.error = None;
                    }
                    // What was last listed stays; only the link is down.
                    Err(DaemonError::Offline { .. }) => state.offline = true,
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            A::Row(holder, action) => return self.row(state, holder, action),
            A::ConfirmEnd => {
                let Some(holder) = state.confirm_end.take() else {
                    return Effect::none();
                };
                if state.offline {
                    return Effect::none();
                }
                return match (&holder.session, holder.tab.clone()) {
                    (_, Some(tab)) if is_shell(&holder) => {
                        let call = self.daemon.end_running_holder(&holder.machine, &HolderTab::Listed(tab));
                        self.call(state, holder, call)
                    }
                    (Some(session), _) => {
                        Effect::send(A::Delegate(RunningDelegate::ArchiveSession(session.clone())))
                    }
                    _ => Effect::none(),
                };
            }
            A::CancelEnd => state.confirm_end = None,
            A::ConfirmAdopt(target) => {
                let Some(holder) = state.adopting.take() else {
                    return Effect::none();
                };
                let Some(tab) = holder.tab.clone() else {
                    return Effect::none();
                };
                let channel = match &target {
                    AdoptTarget::Channel(channel) => Some(channel),
                    AdoptTarget::Desk => None,
                };
                let adopt = self.daemon.adopt_running_holder(&holder.machine, &tab, channel);
                let call: humata2_clients::Reply<()> = Box::pin(async move { adopt.await.map(|_| ()) });
                return self.call(state, holder, call);
            }
            A::CancelAdopt => state.adopting = None,
            A::Done(result) => {
                state.busy = None;
                match result {
                    Ok(()) => {
                        if let Some(machine) = state.machine.clone() {
                            return self.load(state, machine);
                        }
                    }
                    Err(DaemonError::Offline { machine }) => {
                        state.offline = true;
                        state.error = Some(DaemonError::Offline { machine }.to_string());
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            A::Delegate(_) => {}
        }
        Effect::none()
    }
}
