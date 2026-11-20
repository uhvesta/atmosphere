//! Settings › Sync (FUSEFS): which parts of the host's agent config sessions
//! see, per machine where it differs, and whether the cross-machine view is
//! mounted. A change shows at once and is saved; a refusal puts it back.

use humata2_clients::daemon::{
    MachineSyncSettings, SyncChange, SyncSettings, SyncSkillsMode, SyncSnapshot,
};
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// A kind of agent config that can be synced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncCategory {
    /// Skills.
    Skills,
    /// Instruction files.
    Instructions,
    /// MCP server definitions.
    Mcp,
    /// Agent settings files.
    Settings,
}

/// A column of the machines table: the choices one machine can depart from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncColumn {
    /// The machine's own master switch.
    Sync,
    /// Skills on this machine.
    Skills,
    /// Instruction files on this machine.
    Instructions,
    /// MCP servers on this machine.
    Mcp,
    /// Agent settings on this machine.
    Settings,
}

/// What the daemon last said about Sync, with changes shown at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncState {
    /// The open Default / On / Off menu: its machine's id and column.
    pub machine_menu: Option<(String, SyncColumn)>,
    /// `None` until the daemon has said.
    pub snapshot: Option<SyncSnapshot>,
}

/// What happens on the Sync section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncAction {
    /// Read the settings from the daemon.
    Load,
    /// The daemon gave the settings, or couldn't.
    Loaded(Result<SyncSnapshot, DaemonError>),
    /// Turn the master switch on or off. The categories keep their values.
    ToggleEnabled,
    /// Turn one category on or off.
    ToggleCategory(SyncCategory),
    /// Pick how synced skills meet a session's own.
    SetSkillsMode(SyncSkillsMode),
    /// Set one machine's departures from the global values (by machine id);
    /// all `None` makes it follow the global values again.
    SetMachine(String, MachineSyncSettings),
    /// Turn the cross-machine mount on or off.
    ToggleFuseFs,
    /// Open the pick menu of one machine's column, or close it if open.
    OpenMachineMenu(String, SyncColumn),
    /// Close the open pick menu.
    CloseMachineMenu,
    /// Pick Default (`None`), On or Off for one machine's column and close
    /// the menu. Only that field changes.
    PickMachine(String, SyncColumn, Option<bool>),
    /// The daemon saved a change, or refused.
    Saved {
        /// What was shown before the change, to put back on a refusal.
        before: SyncSnapshot,
        /// What the daemon holds now, or why it refused.
        result: Result<SyncSnapshot, DaemonError>,
    },
}

fn flip(sync: &mut SyncSettings, category: SyncCategory) {
    let field = match category {
        SyncCategory::Skills => &mut sync.skills,
        SyncCategory::Instructions => &mut sync.instructions,
        SyncCategory::Mcp => &mut sync.mcp,
        SyncCategory::Settings => &mut sync.settings,
    };
    *field = !*field;
}

/// Applies `action`, calling the daemon through `clients`. `error` is the
/// settings page's error line.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut SyncState,
    error: &mut Option<String>,
    action: SyncAction,
) -> Effect<SyncAction> {
    match action {
        SyncAction::Load => {
            let daemon = clients.daemon.clone();
            Effect::future(async move { SyncAction::Loaded(daemon.sync_settings().await) })
        }
        SyncAction::Loaded(result) => {
            match result {
                Ok(snapshot) => state.snapshot = Some(snapshot),
                Err(failure) => *error = Some(failure.to_string()),
            }
            Effect::none()
        }
        SyncAction::ToggleEnabled => change_section(clients, state, |sync| {
            sync.sync_enabled = !sync.sync_enabled;
        }),
        SyncAction::ToggleCategory(category) => {
            change_section(clients, state, |sync| flip(sync, category))
        }
        SyncAction::SetSkillsMode(mode) => {
            change_section(clients, state, |sync| sync.skills_mode = mode)
        }
        SyncAction::SetMachine(machine, departures) => change_section(clients, state, |sync| {
            if departures == MachineSyncSettings::default() {
                sync.machines.remove(&machine);
            } else {
                sync.machines.insert(machine, departures);
            }
        }),
        SyncAction::OpenMachineMenu(machine, column) => {
            let open = (machine, column);
            state.machine_menu = if state.machine_menu.as_ref() == Some(&open) { None } else { Some(open) };
            Effect::none()
        }
        SyncAction::CloseMachineMenu => {
            state.machine_menu = None;
            Effect::none()
        }
        SyncAction::PickMachine(machine, column, value) => {
            state.machine_menu = None;
            change_section(clients, state, |sync| {
                let mut departures = sync.machines.get(&machine).copied().unwrap_or_default();
                match column {
                    SyncColumn::Sync => departures.enabled = value,
                    SyncColumn::Skills => departures.skills = value,
                    SyncColumn::Instructions => departures.instructions = value,
                    SyncColumn::Mcp => departures.mcp = value,
                    SyncColumn::Settings => departures.settings = value,
                }
                if departures == MachineSyncSettings::default() {
                    sync.machines.remove(&machine);
                } else {
                    sync.machines.insert(machine, departures);
                }
            })
        }
        SyncAction::ToggleFuseFs => {
            let Some(snapshot) = state.snapshot.as_mut() else {
                return Effect::none();
            };
            let before = snapshot.clone();
            snapshot.fusefs_enabled = !snapshot.fusefs_enabled;
            let change = SyncChange { sync: None, fusefs_enabled: Some(snapshot.fusefs_enabled) };
            save(clients, before, change)
        }
        SyncAction::Saved { before, result } => {
            match result {
                Ok(saved) => {
                    // The mount state is the daemon's to say (D908).
                    if let Some(snapshot) = state.snapshot.as_mut() {
                        snapshot.fusefs_status = saved.fusefs_status;
                    }
                }
                Err(failure) => {
                    state.snapshot = Some(before);
                    *error = Some(failure.to_string());
                }
            }
            Effect::none()
        }
    }
}

/// Applies `edit` to the Sync section at once and saves the whole section.
fn change_section(
    clients: &Clients,
    state: &mut SyncState,
    edit: impl FnOnce(&mut SyncSettings),
) -> Effect<SyncAction> {
    let Some(snapshot) = state.snapshot.as_mut() else {
        return Effect::none();
    };
    let before = snapshot.clone();
    edit(&mut snapshot.sync);
    let change = SyncChange { sync: Some(snapshot.sync.clone()), fusefs_enabled: None };
    save(clients, before, change)
}

fn save(clients: &Clients, before: SyncSnapshot, change: SyncChange) -> Effect<SyncAction> {
    let daemon = clients.daemon.clone();
    Effect::future(async move {
        SyncAction::Saved { before, result: daemon.set_sync_settings(change).await }
    })
}
