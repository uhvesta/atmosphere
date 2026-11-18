//! Hosts (HO-1–13, PER-26): every machine, whether it answers, and Connect
//! for one that needs the person. After a reconnect each of its sessions
//! shows as reattached, resumed, or recoverable with what it needs.
//!
//! Which hosts were connected is saved; after a restart a host that was
//! connected but doesn't answer says so next to its Connect button.
//!
//! A machine whose sessions are stopped reads "Sessions stopped" and its
//! row's one action is Start sessions (frame 74), which opens the same
//! Start confirm the machine's page has (frame 75). The confirm is this
//! page's own: one machine at a time, kept here rather than in the page.

pub mod add;
pub mod page;
pub mod running;
pub mod scripts;

pub use add::{AddMachine, AddMachineAction, AddMachineState};
pub use page::{MachinePage, MachinePageAction, MachinePageState};
pub use running::{AdoptTarget, RowAction, Running, RunningAction, RunningDelegate, RunningState};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::machines::{MachineDetail, Started};
use humata2_clients::model::{MachineId, Recovery, RootPlan, SessionId};
use humata2_clients::{Clients, DaemonError, Workspace};
use tca::{Effect, Reducer};

/// The sheet that changes a machine's Humata folder (PL-1, FC-10, ST-5),
/// opened by Change… in Settings › Machines. It holds what is typed, the plan
/// the daemon made for it, and the change while it runs. Nothing moves until
/// a choice is made on a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootChange {
    /// The machine.
    pub machine: MachineId,
    /// The folder as typed; it starts as the machine's current root.
    pub text: String,
    /// A plan is being made for the text.
    pub planning: bool,
    /// What the daemon found under the old root, for the text as it was
    /// when asked; dropped when the text changes.
    pub plan: Option<RootPlan>,
    /// The change was sent and is awaited.
    pub running: bool,
    /// Why the last plan or change failed, in the daemon's words.
    pub error: Option<String>,
}

impl RootChange {
    /// Whether Move them can be chosen: a plan is shown and no session runs
    /// in the channels it would move.
    #[must_use]
    pub fn can_move(&self) -> bool {
        !self.running && self.plan.as_ref().is_some_and(|plan| plan.running.is_empty())
    }
}

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
    /// What each machine's daemon last reported (HO-4); kept when a later read fails.
    pub health: BTreeMap<MachineId, MachineDetail>,
    /// The machine whose Start confirm is open (frame 75).
    pub start_confirm: Option<MachineId>,
    /// Machines whose start was sent and is awaited.
    pub starting: BTreeSet<MachineId>,
    /// The Change… sheet for a machine's Humata folder, when open.
    pub root_change: Option<RootChange>,
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
    /// A machine's Humata folder was changed to this one; the app's copy of
    /// the machine follows without waiting for the daemon's event.
    RootChanged(MachineId, String),
}

/// What happens on the hosts page.
#[derive(Clone, Debug, PartialEq)]
pub enum HostsAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// The hosts connected at the last run came back at launch.
    Restore(BTreeSet<MachineId>),
    /// Open the Add machine dialog (the app does it).
    StartAdd,
    /// Inspect a particular machine.
    Select(MachineId),
    /// Connect was clicked.
    Connect(MachineId),
    /// The reconnect finished.
    Connected(MachineId, Result<Vec<(SessionId, Recovery)>, DaemonError>),
    /// A machine's health was read.
    Health(MachineId, Result<MachineDetail, DaemonError>),
    /// A stopped machine's Start sessions: opens the Start confirm. Does
    /// nothing for a machine whose sessions run.
    StartSessions(MachineId),
    /// The confirm's Start sessions.
    ConfirmStart,
    /// The confirm's Cancel: closes it, sends nothing.
    DismissStart,
    /// A start finished; a failure shows on the machine's row as a connect
    /// failure does.
    SessionsStarted(MachineId, Result<Started, DaemonError>),
    /// Change… on a machine's Humata folder: opens the sheet with the
    /// current root typed in. Nothing is asked of the daemon yet.
    OpenRoot(MachineId),
    /// The folder field changed; a plan made for other text is dropped.
    RootText(String),
    /// Plan: asks what changing to the typed folder would reach.
    PlanRoot,
    /// A plan came back for the machine and the text that was asked.
    RootPlanned(MachineId, String, Result<RootPlan, DaemonError>),
    /// Move them (`move_channels`) or Use for new channels only: applies the
    /// shown plan. Moving is ignored while sessions run in the channels.
    ApplyRoot {
        /// Whether to move the plan's channels into the new root.
        move_channels: bool,
    },
    /// The change finished for the machine, to the root named.
    RootApplied(MachineId, String, Result<(), DaemonError>),
    /// Closes the sheet; ignored while a change runs.
    DismissRoot,
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

    /// Read `machine`'s health for its card (HO-4).
    fn read_health(&self, machine: MachineId) -> Effect<HostsAction> {
        let call = self.clients.machines.detail(&machine);
        Effect::future(async move { HostsAction::Health(machine, call.await) })
    }
}

impl Reducer for Hosts {
    type State = HostsState;
    type Action = HostsAction;

    fn reduce(&self, state: &mut HostsState, action: HostsAction) -> Effect<HostsAction> {
        match action {
            HostsAction::Show(workspace) => {
                let before = state.selected.clone();
                if !state
                    .selected
                    .as_ref()
                    .is_some_and(|id| workspace.machine(id).is_some())
                {
                    state.selected = workspace.machines.first().map(|m| m.id.clone());
                }
                state.workspace = workspace;
                // Only a new selection is read: Show comes on every change of the data.
                if state.selected != before
                    && let Some(id) = state.selected.clone()
                {
                    return self.read_health(id);
                }
            }
            HostsAction::Select(id) => {
                if state.workspace.machine(&id).is_some() {
                    state.selected = Some(id.clone());
                    return self.read_health(id);
                }
            }
            // A failed read keeps what was there; it is not a connect error.
            HostsAction::Health(machine, result) => {
                if let Ok(detail) = result {
                    state.health.insert(machine, detail);
                }
            }
            HostsAction::Restore(previously) => state.previously = previously,
            // The app opens the Add machine dialog.
            HostsAction::StartAdd => {}
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
            HostsAction::StartSessions(machine) => {
                let stopped = state.workspace.machine(&machine).is_some_and(|found| found.stop.is_some());
                if stopped && !state.starting.contains(&machine) {
                    state.start_confirm = Some(machine);
                }
            }
            HostsAction::DismissStart => state.start_confirm = None,
            HostsAction::ConfirmStart => {
                let Some(machine) = state.start_confirm.take() else {
                    return Effect::none();
                };
                if !state.starting.insert(machine.clone()) {
                    return Effect::none();
                }
                state.errors.remove(&machine);
                let call = self.clients.machines.start_sessions(&machine);
                return Effect::future(async move { HostsAction::SessionsStarted(machine, call.await) });
            }
            HostsAction::SessionsStarted(machine, result) => {
                state.starting.remove(&machine);
                match result {
                    Ok(_) => {
                        if let Some(detail) = state.health.get_mut(&machine) {
                            detail.stop = None;
                        }
                    }
                    Err(error) => {
                        state.errors.insert(machine, error.to_string());
                    }
                }
            }
            HostsAction::OpenRoot(machine) => {
                if let Some(found) = state.workspace.machine(&machine) {
                    state.root_change = Some(RootChange {
                        text: found.root.clone(),
                        machine,
                        planning: false,
                        plan: None,
                        running: false,
                        error: None,
                    });
                }
            }
            HostsAction::RootText(text) => {
                if let Some(sheet) = state.root_change.as_mut().filter(|sheet| !sheet.running) {
                    sheet.text = text;
                    sheet.plan = None;
                    sheet.error = None;
                }
            }
            HostsAction::PlanRoot => {
                let Some(sheet) = state.root_change.as_mut() else {
                    return Effect::none();
                };
                let root = sheet.text.trim().to_owned();
                if sheet.running || sheet.planning || root.is_empty() {
                    return Effect::none();
                }
                sheet.planning = true;
                sheet.plan = None;
                sheet.error = None;
                let machine = sheet.machine.clone();
                let call = self.clients.machines.plan_change_root(&machine, &root);
                return Effect::future(async move { HostsAction::RootPlanned(machine, root, call.await) });
            }
            HostsAction::RootPlanned(machine, root, result) => {
                if let Some(sheet) = state.root_change.as_mut().filter(|sheet| sheet.machine == machine) {
                    sheet.planning = false;
                    // A plan for text that has since changed is not the one to show.
                    if sheet.text.trim() == root {
                        match result {
                            Ok(plan) => sheet.plan = Some(plan),
                            Err(error) => sheet.error = Some(error.to_string()),
                        }
                    }
                }
            }
            HostsAction::ApplyRoot { move_channels } => {
                let Some(sheet) = state.root_change.as_mut() else {
                    return Effect::none();
                };
                if sheet.running || (move_channels && !sheet.can_move()) {
                    return Effect::none();
                }
                let Some(plan) = sheet.plan.clone() else {
                    return Effect::none();
                };
                sheet.running = true;
                sheet.error = None;
                let call = self.clients.machines.change_root(&plan, move_channels);
                return Effect::future(async move {
                    HostsAction::RootApplied(plan.machine, plan.new_root, call.await)
                });
            }
            HostsAction::RootApplied(machine, root, result) => {
                let Some(sheet) = state.root_change.as_mut().filter(|sheet| sheet.machine == machine) else {
                    return Effect::none();
                };
                match result {
                    Ok(()) => {
                        state.root_change = None;
                        let workspace = Arc::make_mut(&mut state.workspace);
                        if let Some(found) = workspace.machines.iter_mut().find(|found| found.id == machine) {
                            found.root.clone_from(&root);
                        }
                        return Effect::send(HostsAction::Delegate(Delegate::RootChanged(machine, root)));
                    }
                    Err(error) => {
                        sheet.running = false;
                        sheet.error = Some(error.to_string());
                    }
                }
            }
            HostsAction::DismissRoot => {
                if state.root_change.as_ref().is_some_and(|sheet| !sheet.running) {
                    state.root_change = None;
                }
            }
            HostsAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
