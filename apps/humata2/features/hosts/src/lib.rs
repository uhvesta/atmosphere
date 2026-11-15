//! Hosts (HO-1–13, PER-26): every machine, whether it answers, and Connect
//! for one that needs the person. After a reconnect each of its sessions
//! shows as reattached, resumed, or recoverable with what it needs.
//!
//! Which hosts were connected is saved; after a restart a host that was
//! connected but doesn't answer says so next to its Connect button.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::model::{MachineId, Recovery, SessionId};
use humata2_clients::{Clients, DaemonError, Workspace};
use tca::{Effect, Reducer};

/// The hosts page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostsState {
    /// Where hosts come from.
    pub workspace: Arc<Workspace>,
    /// The machine whose health is open.
    pub selected: Option<MachineId>,
    /// Hosts being connected.
    pub connecting: BTreeSet<MachineId>,
    /// What a reconnect found, per host.
    pub outcomes: BTreeMap<MachineId, Vec<(SessionId, Recovery)>>,
    /// Hosts that were connected when the app last ran.
    pub previously: BTreeSet<MachineId>,
    /// Why a connect failed, per host.
    pub errors: BTreeMap<MachineId, String>,
}

impl HostsState {
    /// The hosts that answer now, to save.
    #[must_use]
    pub fn connected(&self) -> BTreeSet<MachineId> {
        self.workspace
            .machines
            .iter()
            .filter(|machine| machine.online)
            .map(|machine| machine.id.clone())
            .collect()
    }
}

/// What the hosts page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open a session.
    OpenSession(SessionId),
    /// Say this in a toast.
    Toast(String),
}

/// What happens on the hosts page.
#[derive(Clone, Debug, PartialEq)]
pub enum HostsAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// The hosts connected at the last run came back at launch.
    Restore(BTreeSet<MachineId>),
    /// Inspect a particular machine.
    Select(MachineId),
    /// Connect was clicked.
    Connect(MachineId),
    /// The reconnect finished.
    Connected(MachineId, Result<Vec<(SessionId, Recovery)>, DaemonError>),
    /// For the parent.
    Delegate(Delegate),
}

/// The hosts page's reducer.
#[derive(Clone)]
pub struct Hosts {
    clients: Clients,
}

impl Hosts {
    /// A hosts page that reconnects with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }
}

impl Reducer for Hosts {
    type State = HostsState;
    type Action = HostsAction;

    fn reduce(&self, state: &mut HostsState, action: HostsAction) -> Effect<HostsAction> {
        match action {
            HostsAction::Show(workspace) => {
                if !state
                    .selected
                    .as_ref()
                    .is_some_and(|id| workspace.machine(id).is_some())
                {
                    state.selected = workspace.machines.first().map(|m| m.id.clone());
                }
                state.workspace = workspace;
            }
            HostsAction::Select(id) => {
                if state.workspace.machine(&id).is_some() {
                    state.selected = Some(id);
                }
            }
            HostsAction::Restore(previously) => state.previously = previously,
            HostsAction::Connect(machine) => {
                if !state.connecting.insert(machine.clone()) {
                    return Effect::none();
                }
                state.errors.remove(&machine);
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    let result = daemon.reconnect(&machine).await;
                    HostsAction::Connected(machine, result)
                });
            }
            HostsAction::Connected(machine, result) => {
                state.connecting.remove(&machine);
                let name = state
                    .workspace
                    .machine(&machine)
                    .map_or_else(|| machine.to_string(), |found| found.name.clone());
                match result {
                    Ok(outcomes) => {
                        let recoverable = outcomes
                            .iter()
                            .filter(|(_, outcome)| matches!(outcome, Recovery::Recoverable { .. }))
                            .count();
                        state.previously.remove(&machine);
                        state.outcomes.insert(machine, outcomes);
                        let toast = if recoverable == 0 {
                            format!("Connected to {name}.")
                        } else {
                            format!(
                                "Connected to {name}. {recoverable} session{} need{} you to recover.",
                                if recoverable == 1 { "" } else { "s" },
                                if recoverable == 1 { "s" } else { "" }
                            )
                        };
                        return Effect::send(HostsAction::Delegate(Delegate::Toast(toast)));
                    }
                    Err(error) => {
                        state.errors.insert(machine, error.to_string());
                    }
                }
            }
            HostsAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
