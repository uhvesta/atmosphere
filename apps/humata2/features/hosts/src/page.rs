//! A machine's page (HO-4, HO-5, HO-13, SK-5, SK-8, SK-11; frame 59): its
//! system, health, sessions, Humata folders and what its agents can reach,
//! with Sync skills and host variables.
//!
//! An offline machine keeps what was last loaded; only the link is down.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    ClashChoice, MachineDetail, Machines, SkillsPreview,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use tca::Effect;
use tca::Reducer;

const DETAIL: &str = "machine-page-detail";

/// The page's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// Sessions, folders, health and rules.
    #[default]
    Overview,
    /// What Humata keeps on the machine (its content is another milestone).
    Storage,
}

/// The skills preview and what is happening to it (SK-5, SK-11).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillsDialog {
    /// What a sync would do; none while it loads.
    pub preview: Option<SkillsPreview>,
    /// The sync is running.
    pub syncing: bool,
    /// Why it couldn't continue, as text.
    pub error: Option<String>,
}

/// A host variable being set (HO-13).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariableEdit {
    /// The variable's name.
    pub name: String,
    /// The value, as typed in the masked field. Dropped once sent.
    pub value: String,
    /// The value was sent and the answer is awaited.
    pub sending: bool,
    /// Why it wasn't stored, as text.
    pub error: Option<String>,
}

/// The page's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MachinePageState {
    /// The machine shown.
    pub machine: Option<MachineId>,
    /// The tab.
    pub tab: Tab,
    /// What was last loaded; kept when a reload fails (HO-5).
    pub detail: Option<MachineDetail>,
    /// Why the last load failed, as text.
    pub load_error: Option<String>,
    /// A load is in flight.
    pub loading: bool,
    /// The skills preview, when open.
    pub skills: Option<SkillsDialog>,
    /// The "…" menu, when open.
    pub menu_open: bool,
    /// The variable being set.
    pub variable: Option<VariableEdit>,
    /// Why something else (a removal, a clash pick) failed, as text.
    pub error: Option<String>,
}

/// What the page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Reconnect this machine now.
    Reconnect(MachineId),
    /// Open a shell on it.
    OpenShell(MachineId),
    /// Start a session on it.
    NewSession(MachineId),
    /// Open a session.
    OpenSession(SessionId),
    /// Open its folder for a channel in Files.
    OpenFolder {
        /// The machine.
        machine: MachineId,
        /// The channel whose folder it is.
        channel: ChannelId,
    },
    /// Open Agent access for this machine.
    ChangeAgentAccess(MachineId),
    /// Say this in a toast.
    Toast(String),
}

/// The items of the "…" menu that run an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPick {
    /// Sync skills.
    SyncSkills,
    /// Reconnect now.
    ReconnectNow,
}

/// What happens on the page.
#[derive(Clone, Debug, PartialEq)]
pub enum MachinePageAction {
    /// Show this machine and load it.
    Show(MachineId),
    /// Load it again.
    Reload,
    /// A load finished.
    Loaded(MachineId, Result<MachineDetail, DaemonError>),
    /// Pick a tab.
    SelectTab(Tab),
    /// Reconnect now.
    ReconnectNow,
    /// Open the "…" menu.
    OpenMenu,
    /// Close the "…" menu without picking.
    CloseMenu,
    /// Pick an item of the "…" menu: runs what its button ran, then closes it.
    PickMenu(MenuPick),
    /// Open a shell.
    OpenShell,
    /// New session on this machine.
    NewSession,
    /// Open one of its sessions.
    OpenSession(SessionId),
    /// Open a channel's folder in Files.
    OpenFolder(ChannelId),
    /// Change what its agents can reach.
    ChangeAgentAccess,
    /// Sync skills: load the preview first.
    SyncSkills,
    /// The preview arrived.
    Previewed(MachineId, Result<SkillsPreview, DaemonError>),
    /// Confirm the previewed sync.
    ConfirmSync,
    /// Close the preview without syncing.
    CancelSync,
    /// The sync finished.
    Synced(MachineId, Result<(), DaemonError>),
    /// Pick which skill wins a clash.
    ResolveClash(String, ClashChoice),
    /// The pick was stored.
    ClashResolved(String, Result<(), DaemonError>),
    /// Start setting a variable; the name may be filled in.
    BeginVariable(Option<String>),
    /// Edit the variable's name.
    VariableName(String),
    /// Edit the variable's value.
    VariableValue(String),
    /// Send the value.
    SendVariable,
    /// Close the variable form.
    CancelVariable,
    /// The value was stored or refused.
    VariableSent(Result<(), DaemonError>),
    /// Remove a variable.
    RemoveVariable(String),
    /// The variable was removed or refused.
    VariableRemoved(Result<(), DaemonError>),
    /// For the parent.
    Delegate(Delegate),
}

/// The page's reducer.
#[derive(Clone)]
pub struct MachinePage {
    machines: Arc<dyn Machines>,
}

impl MachinePage {
    /// A page that reads and changes machines through `machines`.
    #[must_use]
    pub fn new(machines: Arc<dyn Machines>) -> Self {
        Self { machines }
    }

    fn load(&self, state: &mut MachinePageState, machine: MachineId) -> Effect<MachinePageAction> {
        state.loading = true;
        let call = self.machines.detail(&machine);
        Effect::future(async move { MachinePageAction::Loaded(machine, call.await) })
            .cancellable(DETAIL, true)
    }
}

impl Reducer for MachinePage {
    type State = MachinePageState;
    type Action = MachinePageAction;

    #[allow(clippy::too_many_lines)]
    fn reduce(
        &self,
        state: &mut MachinePageState,
        action: MachinePageAction,
    ) -> Effect<MachinePageAction> {
        use MachinePageAction as A;
        let delegate = |delegate: Delegate| Effect::send(A::Delegate(delegate));
        match action {
            A::Show(machine) => {
                if state.machine.as_ref() != Some(&machine) {
                    *state = MachinePageState { machine: Some(machine.clone()), ..MachinePageState::default() };
                }
                return self.load(state, machine);
            }
            A::Reload => {
                if let Some(machine) = state.machine.clone() {
                    return self.load(state, machine);
                }
            }
            A::Loaded(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                state.loading = false;
                match result {
                    Ok(detail) => {
                        state.detail = Some(detail);
                        state.load_error = None;
                    }
                    // Keep the last detail: only the link is down (HO-5).
                    Err(error) => state.load_error = Some(error.to_string()),
                }
            }
            A::SelectTab(tab) => state.tab = tab,
            A::OpenMenu => state.menu_open = true,
            A::CloseMenu => state.menu_open = false,
            A::PickMenu(pick) => {
                state.menu_open = false;
                let action = match pick {
                    MenuPick::SyncSkills => A::SyncSkills,
                    MenuPick::ReconnectNow => A::ReconnectNow,
                };
                return self.reduce(state, action);
            }
            A::ReconnectNow => {
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::Reconnect(machine));
                }
            }
            A::OpenShell => {
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::OpenShell(machine));
                }
            }
            A::NewSession => {
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::NewSession(machine));
                }
            }
            A::OpenSession(session) => return delegate(Delegate::OpenSession(session)),
            A::OpenFolder(channel) => {
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::OpenFolder { machine, channel });
                }
            }
            A::ChangeAgentAccess => {
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::ChangeAgentAccess(machine));
                }
            }
            A::SyncSkills => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                if state.skills.is_some() {
                    return Effect::none();
                }
                state.skills = Some(SkillsDialog::default());
                let call = self.machines.skills_preview(&machine);
                return Effect::future(async move { A::Previewed(machine, call.await) });
            }
            A::Previewed(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                let Some(dialog) = &mut state.skills else {
                    return Effect::none();
                };
                match result {
                    Ok(preview) => dialog.preview = Some(preview),
                    Err(error) => dialog.error = Some(error.to_string()),
                }
            }
            A::ConfirmSync => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                let Some(dialog) = &mut state.skills else {
                    return Effect::none();
                };
                // A name clash must be settled first (SK-5).
                let ready = dialog.preview.as_ref().is_some_and(|p| p.clashes.is_empty());
                if !ready || dialog.syncing {
                    return Effect::none();
                }
                dialog.syncing = true;
                dialog.error = None;
                let call = self.machines.sync_skills(&machine);
                return Effect::future(async move { A::Synced(machine, call.await) });
            }
            A::CancelSync => {
                if state.skills.as_ref().is_some_and(|dialog| !dialog.syncing) {
                    state.skills = None;
                }
            }
            A::Synced(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                let Some(dialog) = &mut state.skills else {
                    return Effect::none();
                };
                dialog.syncing = false;
                match result {
                    Ok(()) => {
                        state.skills = None;
                        // Say it first; the reload's answer follows.
                        return Effect::concat([
                            delegate(Delegate::Toast("Skills synced.".to_owned())),
                            self.load(state, machine),
                        ]);
                    }
                    Err(error) => dialog.error = Some(error.to_string()),
                }
            }
            A::ResolveClash(name, choice) => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                if state.skills.is_none() {
                    return Effect::none();
                }
                let call = self.machines.resolve_clash(&machine, &name, choice);
                return Effect::future(async move { A::ClashResolved(name, call.await) });
            }
            A::ClashResolved(name, result) => {
                let Some(dialog) = &mut state.skills else {
                    return Effect::none();
                };
                match result {
                    Ok(()) => {
                        if let Some(preview) = &mut dialog.preview {
                            preview.clashes.retain(|clash| clash.name != name);
                        }
                    }
                    Err(error) => dialog.error = Some(error.to_string()),
                }
            }
            A::BeginVariable(name) => {
                if state.machine.is_some() {
                    state.variable = Some(VariableEdit {
                        name: name.unwrap_or_default(),
                        ..VariableEdit::default()
                    });
                }
            }
            A::VariableName(name) => {
                if let Some(edit) = &mut state.variable
                    && !edit.sending
                {
                    edit.name = name;
                }
            }
            A::VariableValue(value) => {
                if let Some(edit) = &mut state.variable
                    && !edit.sending
                {
                    edit.value = value;
                }
            }
            A::SendVariable => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                let Some(edit) = &mut state.variable else {
                    return Effect::none();
                };
                if edit.sending || edit.name.trim().is_empty() || edit.value.is_empty() {
                    return Effect::none();
                }
                edit.sending = true;
                edit.error = None;
                // The value leaves the state as it is sent (HO-13).
                let value = std::mem::take(&mut edit.value);
                let call = self.machines.set_variable(&machine, edit.name.trim(), &value);
                return Effect::future(async move { A::VariableSent(call.await) });
            }
            A::CancelVariable => {
                if state.variable.as_ref().is_some_and(|edit| !edit.sending) {
                    state.variable = None;
                }
            }
            A::VariableSent(result) => {
                let Some(edit) = &mut state.variable else {
                    return Effect::none();
                };
                edit.sending = false;
                match result {
                    Ok(()) => {
                        state.variable = None;
                        if let Some(machine) = state.machine.clone() {
                            return self.load(state, machine);
                        }
                    }
                    Err(error) => edit.error = Some(error.to_string()),
                }
            }
            A::RemoveVariable(name) => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                state.error = None;
                let call = self.machines.remove_variable(&machine, &name);
                return Effect::future(async move { A::VariableRemoved(call.await) });
            }
            A::VariableRemoved(result) => match result {
                Ok(()) => {
                    if let Some(machine) = state.machine.clone() {
                        return self.load(state, machine);
                    }
                }
                Err(error) => state.error = Some(error.to_string()),
            },
            A::Delegate(_) => {}
        }
        Effect::none()
    }
}
