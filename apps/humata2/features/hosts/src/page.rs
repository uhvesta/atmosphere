//! A machine's page (HO-4, HO-5, HO-13, SK-5, SK-8, SK-11; frame 59): its
//! system, health, sessions, Humata folders and what its agents can reach,
//! with Sync skills and host variables.
//!
//! An offline machine keeps what was last loaded; only the link is down.
//!
//! Stop all sessions… (frames 72–76, D260–D265): the "…" menu's last item
//! opens a confirm while the sessions run; confirming stops them and the
//! page reloads to show Sessions stopped. Start sessions opens a lighter
//! confirm; a stop the machine never got is cancelled with Cancel stop, no
//! confirm. While stopped, Open a shell and New session are refused here
//! and say why.
//!
//! Upgrade (UP-1, D281; frames 80–82): while this machine's daemon or store
//! waits for a Migrate item, the page is read-only: Open a shell and New
//! session are refused and say why, and Open in Inbox goes to the item. The
//! parent feeds the page the Inbox's Migrate items.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    ClashChoice, MachineDetail, MachineScripts, Machines, SkillsPreview, Started,
};
use humata2_clients::model::{
    Answer, ChannelId, MachineId, MachineStop, MigrateFailure, MigratePhase, MigrateThing, Release, Request,
    RequestId, SessionId,
};
use crate::scripts::ScriptsState;
use tca::Effect;
use tca::Reducer;

const DETAIL: &str = "machine-page-detail";
const SCRIPTS: &str = "machine-page-scripts";

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

/// A confirm shown over the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// "Stop all sessions…": what ends, what is kept (frame 72).
    Stop,
    /// "Start sessions": the lighter one (frame 75).
    Start,
}

/// A change to the machine's sessions that was sent and is awaited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionsChange {
    /// Stopping them, after the Stop confirm.
    Stop,
    /// Starting them, after the Start confirm.
    Start,
    /// Cancelling a stop the machine never got (frame 76).
    CancelStop,
}

/// Why the page didn't do what was clicked, for the view to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The machine's sessions are stopped: "Start sessions first" (D264).
    SessionsStopped,
    /// The machine is behind this Humata: the Migrate item, when one is
    /// named, brings it up to date.
    MigrationNeeded(Option<RequestId>),
}

/// Where the machine's release is, from the Migrate items of its daemon and
/// store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Upgrade {
    /// Nothing is waiting.
    Current,
    /// A Migrate item waits: the page is read-only.
    ReadOnly {
        /// The item.
        request: RequestId,
        /// The release it brings.
        to: Release,
    },
    /// The item is migrating.
    Migrating {
        /// The item.
        request: RequestId,
        /// The release it brings.
        to: Release,
    },
    /// The last try failed.
    Failed {
        /// The item.
        request: RequestId,
        /// The release it brings.
        to: Release,
        /// Why, in the daemon's words.
        failure: MigrateFailure,
    },
    /// It was migrated.
    Migrated {
        /// The release it is at now.
        to: Release,
        /// When, as people read it.
        at: String,
    },
}

impl Upgrade {
    /// Whether New session and Open a shell are held back.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        matches!(self, Self::ReadOnly { .. } | Self::Migrating { .. } | Self::Failed { .. })
    }

    fn request(&self) -> Option<&RequestId> {
        match self {
            Self::ReadOnly { request, .. }
            | Self::Migrating { request, .. }
            | Self::Failed { request, .. } => Some(request),
            Self::Current | Self::Migrated { .. } => None,
        }
    }
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
    /// Why something else (a removal, a clash pick, a stop or start) failed, as text.
    pub error: Option<String>,
    /// The Stop or Start confirm, when open.
    pub confirm: Option<Confirm>,
    /// A stop, start or cancel that was sent, while its answer is awaited;
    /// the view shows it as progress.
    pub changing: Option<SessionsChange>,
    /// Why the last click was refused here, until the next one.
    pub refused: Option<Refused>,
    /// The Inbox's Migrate items, as the parent last sent them.
    pub migrations: Vec<Request>,
    /// The Scripts section (MACHINESCRIPTS, D686).
    pub scripts: ScriptsState,
}

impl MachinePageState {
    /// The machine's stop, from what was last loaded; `None` while its
    /// sessions run or before the first load.
    #[must_use]
    pub fn stop(&self) -> Option<&MachineStop> {
        self.detail.as_ref().and_then(|detail| detail.stop.as_ref())
    }

    /// This machine's Migrate items that hold the page: its daemon's and
    /// its store's. CLI, holders and host tools don't.
    fn items(&self) -> impl Iterator<Item = &Request> {
        let machine = self.machine.as_ref();
        self.migrations.iter().filter(move |request| {
            request.migrate().is_some_and(|card| {
                Some(&card.machine) == machine
                    && matches!(card.thing, MigrateThing::Daemon | MigrateThing::Store)
            })
        })
    }

    /// Where the machine's release is. A waiting item decides first, by its
    /// phase; otherwise an approved one says it was migrated.
    #[must_use]
    pub fn upgrade(&self) -> Upgrade {
        let waiting = self.items().filter(|request| request.answer.is_none()).find_map(|request| {
            let card = request.migrate()?;
            let (request, to) = (request.id.clone(), card.to.clone());
            Some(match &card.phase {
                MigratePhase::Waiting => Upgrade::ReadOnly { request, to },
                MigratePhase::Migrating => Upgrade::Migrating { request, to },
                MigratePhase::Failed(failure) => Upgrade::Failed { request, to, failure: failure.clone() },
            })
        });
        if let Some(waiting) = waiting {
            return waiting;
        }
        self.items()
            .find_map(|request| {
                let card = request.migrate()?;
                (request.answer == Some(Answer::Approved)).then(|| Upgrade::Migrated {
                    to: card.to.clone(),
                    at: card.migrated.clone().unwrap_or_default(),
                })
            })
            .unwrap_or(Upgrade::Current)
    }

    /// The Migrate item Open in Inbox goes to: the one a refusal named, else
    /// this machine's waiting item, else its latest.
    fn inbox_item(&self) -> Option<RequestId> {
        if let Some(Refused::MigrationNeeded(Some(request))) = &self.refused {
            return Some(request.clone());
        }
        self.upgrade()
            .request()
            .cloned()
            .or_else(|| self.items().next().map(|request| request.id.clone()))
    }

    /// Keeps what a migration-needed failure says, for the page to show.
    fn note_refusal(&mut self, error: &DaemonError) {
        if let DaemonError::MigrationNeeded { request, .. } = error {
            self.refused = Some(Refused::MigrationNeeded(request.clone()));
        }
    }
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
    /// Open the Inbox with this Migrate item selected.
    OpenInInbox(RequestId),
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
    /// Stop all sessions…: opens the Stop confirm. Offered only while the
    /// sessions run; picked otherwise it only closes the menu.
    StopSessions,
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
    /// Start sessions, from the Stopped banner: opens the Start confirm.
    /// Does nothing while the sessions run.
    StartSessions,
    /// Cancel stop, from the banner of a stop the machine never got: sent
    /// at once, with no confirm. Does nothing for a delivered stop.
    CancelStop,
    /// The confirm's red Stop all sessions, or its Start sessions.
    ConfirmSessions,
    /// The confirm's Cancel: closes it, sends nothing.
    DismissConfirm,
    /// A stop finished.
    SessionsStopped(MachineId, Result<MachineStop, DaemonError>),
    /// A start or a Cancel stop finished.
    SessionsStarted(MachineId, Result<Started, DaemonError>),
    /// The Inbox's Migrate items changed: the parent sends them all, the
    /// app's own included.
    Migrations(Vec<Request>),
    /// The banner's Open in Inbox, or a refusal's.
    OpenInInbox,
    /// The machine's scripts were read (MACHINESCRIPTS, D686).
    ScriptsLoaded(MachineId, Result<MachineScripts, DaemonError>),
    /// The host's feed says a script run on this machine changed: read the
    /// runs again.
    ScriptRunChanged(MachineId),
    /// Open or close a rule's steps and their output.
    ToggleScript(String),
    /// Run now: runs the rule here as a manual run.
    RunScript(String),
    /// A Run now ended, or was refused.
    ScriptRan(MachineId, String, Result<(), DaemonError>),
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

    fn load_scripts(&self, state: &mut MachinePageState, machine: MachineId) -> Effect<MachinePageAction> {
        state.scripts.loading = true;
        let call = self.machines.scripts(Some(&machine));
        Effect::future(async move { MachinePageAction::ScriptsLoaded(machine, call.await) })
            .cancellable(SCRIPTS, true)
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
                    *state = MachinePageState {
                        machine: Some(machine.clone()),
                        migrations: std::mem::take(&mut state.migrations),
                        ..MachinePageState::default()
                    };
                }
                // The rules and their last runs load beside the detail.
                let scripts = self.load_scripts(state, machine.clone());
                return Effect::merge([self.load(state, machine), scripts]);
            }
            A::Migrations(migrations) => state.migrations = migrations,
            A::OpenInInbox => {
                if let Some(request) = state.inbox_item() {
                    return delegate(Delegate::OpenInInbox(request));
                }
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
                    Err(error) => {
                        state.note_refusal(&error);
                        state.load_error = Some(error.to_string());
                    }
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
                    MenuPick::StopSessions => {
                        if state.detail.is_some() && state.stop().is_none() && state.changing.is_none() {
                            state.confirm = Some(Confirm::Stop);
                        }
                        return Effect::none();
                    }
                };
                return self.reduce(state, action);
            }
            A::ReconnectNow => {
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::Reconnect(machine));
                }
            }
            A::OpenShell => {
                if state.stop().is_some() {
                    state.refused = Some(Refused::SessionsStopped);
                    return Effect::none();
                }
                if let Some(request) = state.upgrade().request().cloned() {
                    state.refused = Some(Refused::MigrationNeeded(Some(request)));
                    return Effect::none();
                }
                state.refused = None;
                if let Some(machine) = state.machine.clone() {
                    return delegate(Delegate::OpenShell(machine));
                }
            }
            A::NewSession => {
                if state.stop().is_some() {
                    state.refused = Some(Refused::SessionsStopped);
                    return Effect::none();
                }
                if let Some(request) = state.upgrade().request().cloned() {
                    state.refused = Some(Refused::MigrationNeeded(Some(request)));
                    return Effect::none();
                }
                state.refused = None;
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
                if let Err(error) = &result {
                    state.note_refusal(error);
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
                if let Err(error) = &result {
                    state.note_refusal(error);
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
                if let Err(error) = &result {
                    state.note_refusal(error);
                }
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
                if let Err(error) = &result {
                    state.note_refusal(error);
                }
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
                Err(error) => {
                    state.note_refusal(&error);
                    state.error = Some(error.to_string());
                }
            },
            A::StartSessions => {
                if state.stop().is_some() && state.changing.is_none() {
                    state.confirm = Some(Confirm::Start);
                }
            }
            A::CancelStop => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                // Only a stop the machine never got is cancelled without a confirm.
                if state.changing.is_some() || !state.stop().is_some_and(|stop| !stop.delivered) {
                    return Effect::none();
                }
                state.confirm = None;
                state.error = None;
                state.changing = Some(SessionsChange::CancelStop);
                let call = self.machines.start_sessions(&machine);
                return Effect::future(async move { A::SessionsStarted(machine, call.await) });
            }
            A::ConfirmSessions => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                if state.changing.is_some() {
                    return Effect::none();
                }
                let Some(confirm) = state.confirm.take() else {
                    return Effect::none();
                };
                state.error = None;
                match confirm {
                    Confirm::Stop => {
                        state.changing = Some(SessionsChange::Stop);
                        let call = self.machines.stop_sessions(&machine);
                        return Effect::future(async move { A::SessionsStopped(machine, call.await) });
                    }
                    Confirm::Start => {
                        state.changing = Some(SessionsChange::Start);
                        let call = self.machines.start_sessions(&machine);
                        return Effect::future(async move { A::SessionsStarted(machine, call.await) });
                    }
                }
            }
            A::DismissConfirm => state.confirm = None,
            A::SessionsStopped(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                state.changing = None;
                match result {
                    Ok(_) => return self.load(state, machine),
                    Err(error) => {
                        state.note_refusal(&error);
                        state.error = Some(error.to_string());
                    }
                }
            }
            A::SessionsStarted(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                state.changing = None;
                match result {
                    Ok(_) => return self.load(state, machine),
                    Err(error) => {
                        state.note_refusal(&error);
                        state.error = Some(error.to_string());
                    }
                }
            }
            A::ScriptsLoaded(machine, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                state.scripts.loading = false;
                match result {
                    Ok(scripts) => {
                        state.scripts.loaded = Some(scripts);
                        state.scripts.error = None;
                    }
                    // Keep the rules last read: only this read failed.
                    Err(error) => {
                        state.note_refusal(&error);
                        state.scripts.error = Some(error.to_string());
                    }
                }
            }
            A::ScriptRunChanged(machine) => {
                if state.machine.as_ref() == Some(&machine) {
                    return self.load_scripts(state, machine);
                }
            }
            A::ToggleScript(rule) => {
                state.scripts.open = if state.scripts.open.as_deref() == Some(rule.as_str()) {
                    None
                } else {
                    Some(rule)
                };
            }
            A::RunScript(rule) => {
                let Some(machine) = state.machine.clone() else {
                    return Effect::none();
                };
                let known = state
                    .scripts
                    .loaded
                    .as_ref()
                    .is_some_and(|scripts| scripts.rules.iter().any(|candidate| candidate.name == rule));
                if !known || state.scripts.running.contains(&rule) {
                    return Effect::none();
                }
                // A machine waiting for its Migrate item runs nothing (UP-1).
                if let Some(request) = state.upgrade().request().cloned() {
                    state.refused = Some(Refused::MigrationNeeded(Some(request)));
                    return Effect::none();
                }
                state.refused = None;
                state.scripts.refused.remove(&rule);
                state.scripts.running.insert(rule.clone());
                let call = self.machines.run_script(&machine, &rule, None);
                return Effect::future(async move { A::ScriptRan(machine, rule, call.await) });
            }
            A::ScriptRan(machine, rule, result) => {
                if state.machine.as_ref() != Some(&machine) {
                    return Effect::none();
                }
                state.scripts.running.remove(&rule);
                if let Err(error) = result {
                    state.note_refusal(&error);
                    state.scripts.refused.insert(rule, error.to_string());
                }
                // The run, ended or refused, is read from the host's log.
                return self.load_scripts(state, machine);
            }
            A::Delegate(_) => {}
        }
        Effect::none()
    }
}
