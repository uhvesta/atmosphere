use humata2_clients::DaemonError;
// REPOS (RL-5): the picker is the one repo search.
use crate::repo_search::{self, PickRow, RepoPicker, RepoSearchAction, RepoSearchState};
use humata2_clients::model::{ChannelId, ChannelSetup, Machine, MachineId, MachineKind, SetupState};
use std::sync::Arc;
use tca::Effect;

use crate::{App, AppAction, AppState, Overlay, Route};

/// How many repos the sheet lists at once (CH-3).
const REPO_LIMIT: usize = 50;

/// The New channel sheet (CH-1 to CH-4, CH-10).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewChannel {
    /// Its name, as typed.
    pub name: String,
    /// The repo search as typed; the list shows the repos it matches.
    pub query: String,
    /// The one repo search (RL-5): the Library and GitHub groups, the scope
    /// chips and why GitHub could not be searched.
    pub search: RepoSearchState,
    /// The ticked repos, in the order they were ticked.
    pub repos: Vec<String>,
    /// The ticked machines, in the order they were ticked.
    pub machines: Vec<MachineId>,
    /// Waiting for the daemon.
    pub creating: bool,
    /// Why it couldn't be made.
    pub error: Option<String>,
    /// Its folders and clones while they run (CH-5); the sheet shows them in
    /// place of the form.
    pub setup: Option<ChannelSetup>,
}

impl NewChannel {
    /// The folder the name gives (CH-2): lowercase, every run of characters
    /// that are not ASCII letters or digits as one `-`, and none at either
    /// end. "  # Q4 / Billing!! " gives "q4-billing".
    #[must_use]
    pub fn folder(&self) -> String {
        let mut folder = String::new();
        let mut pending = false;
        for c in self.name.chars() {
            if c.is_ascii_alphanumeric() {
                if pending && !folder.is_empty() {
                    folder.push('-');
                }
                pending = false;
                folder.push(c.to_ascii_lowercase());
            } else {
                pending = true;
            }
        }
        folder
    }

    /// The names of the ticked machines in tick order, joined as
    /// "This Mac, devbox-1"; `None` when none is ticked or none is known.
    #[must_use]
    pub fn machines_line(&self, machines: &[Machine]) -> Option<String> {
        let names: Vec<&str> = self
            .machines
            .iter()
            .filter_map(|ticked| machines.iter().find(|machine| &machine.id == ticked))
            .map(|machine| machine.name.as_str())
            .collect();
        (!names.is_empty()).then(|| names.join(", "))
    }

    /// The repos the list shows (CH-3): the ticked repos in tick order, shown
    /// whether or not they match, then the listed repos the query matches that
    /// are not ticked, at most 50 in all. Empty while the repos are not listed.
    #[must_use]
    pub fn shown_repos(&self) -> Vec<&str> {
        self.search.names(&self.repos, REPO_LIMIT)
    }

    /// The rows the picker draws: the ticked repos first, then the Library
    /// group, then the GitHub group (RL-6), at most 50, and how many
    /// matches the cap leaves out.
    #[must_use]
    pub fn picker(&self) -> (Vec<PickRow>, usize) {
        self.search.rows(&self.repos, REPO_LIMIT)
    }

    /// How many matching repos the 50-repo cap leaves out of
    /// [`shown_repos`](Self::shown_repos), for an "and N more; type to narrow"
    /// line.
    #[must_use]
    pub fn more_repos(&self) -> usize {
        self.picker().1
    }

    /// The create button's label, which says how many repos it will clone
    /// (CH-3).
    #[must_use]
    pub fn create_label(&self) -> String {
        match self.repos.len() {
            0 => "Create channel".to_owned(),
            1 => "Create channel and clone 1 repo".to_owned(),
            count => format!("Create channel and clone {count} repos"),
        }
    }
}

/// A change to the New channel sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewChannelAction {
    /// Its name changed.
    Name(String),
    /// The repo search changed.
    Query(String),
    /// A repo was ticked, or unticked when it already was.
    ToggleRepo(String),
    /// A machine was ticked, or unticked when it already was. An offline
    /// machine cannot be ticked (CH-4).
    ToggleMachine(MachineId),
    /// List the repos again after they could not be listed.
    RetryRepos,
    /// Make it.
    Create,
    /// Stop the setup that is running (CH-5).
    CancelSetup,
    /// Run the failed clones again (CH-5).
    RetrySetup,
    /// The daemon stopped the setup, or didn't.
    SetupStopped(Result<(), DaemonError>),
    /// The daemon started the failed clones again, or didn't.
    SetupRetried(Result<ChannelId, DaemonError>),
}

impl App {
    /// Opens the sheet with This Mac ticked (or the first online machine when
    /// This Mac is offline, or none), and lists the repos gh sees.
    pub(super) fn start_new_channel(&self, state: &mut AppState) -> Effect<AppAction> {
        let machines = state
            .workspace
            .machines
            .iter()
            .find(|machine| machine.online && matches!(machine.kind, MachineKind::ThisMac))
            .or_else(|| state.workspace.machines.iter().find(|machine| machine.online))
            .map(|machine| vec![machine.id.clone()])
            .unwrap_or_default();
        state.overlays.push(Overlay::NewChannel(NewChannel {
            machines,
            ..NewChannel::default()
        }));
        // REPOS (RL-3): the picker opens on the library, then asks GitHub.
        self.repo_search(state, RepoPicker::NewChannel, RepoSearchAction::Open)
    }

    pub(super) fn change_new_channel(
        &self,
        state: &mut AppState,
        change: NewChannelAction,
    ) -> Effect<AppAction> {
        let Some(Overlay::NewChannel(sheet)) = state.overlays.last_mut() else {
            return Effect::none();
        };
        match change {
            NewChannelAction::Name(name) => sheet.name = name,
            NewChannelAction::Query(query) => {
                return self.query_new_channel_repos(sheet, query);
            }
            NewChannelAction::ToggleRepo(repo) => toggle(&mut sheet.repos, repo),
            NewChannelAction::ToggleMachine(machine) => {
                // CH-4: an offline machine is shown but cannot be chosen; one
                // that went offline after it was ticked can still be unticked.
                let online = state
                    .workspace
                    .machines
                    .iter()
                    .any(|found| found.id == machine && found.online);
                if online || sheet.machines.contains(&machine) {
                    toggle(&mut sheet.machines, machine);
                }
            }
            NewChannelAction::RetryRepos => {
                return repo_search::reduce(
                    &self.clients,
                    &mut sheet.search,
                    RepoSearchAction::Retry,
                    "new-channel",
                )
                .map(|action| AppAction::RepoSearch(RepoPicker::NewChannel, action));
            }
            NewChannelAction::Create => {
                let name = sheet.name.trim().trim_start_matches('#').to_owned();
                if name.is_empty() {
                    sheet.error = Some("Give the channel a name.".to_owned());
                    return Effect::none();
                }
                // DK-2: the Desk's folder name is never a channel's.
                if sheet.folder() == humata2_clients::desk::FOLDER {
                    sheet.error = Some(
                        "“desk” is kept for the Desk's folder. Give the channel another name."
                            .to_owned(),
                    );
                    return Effect::none();
                }
                if sheet.machines.is_empty() {
                    sheet.error =
                        Some("Pick at least one machine to make the channel on.".to_owned());
                    return Effect::none();
                }
                sheet.creating = true;
                sheet.error = None;
                let repos = sheet.repos.clone();
                let machines = sheet.machines.clone();
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    AppAction::ChannelCreated(daemon.create_channel(&name, repos, machines).await)
                });
            }
            NewChannelAction::CancelSetup => {
                let Some(setup) = sheet.setup.as_ref().map(|setup| setup.id.clone()) else {
                    // Nothing runs yet: Cancel just closes the sheet.
                    state.overlays.pop();
                    return Effect::none();
                };
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    AppAction::NewChannel(NewChannelAction::SetupStopped(
                        daemon.cancel_setup(&setup).await,
                    ))
                });
            }
            NewChannelAction::RetrySetup => {
                let Some(setup) = sheet.setup.as_ref().map(|setup| setup.id.clone()) else {
                    return Effect::none();
                };
                sheet.error = None;
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    AppAction::NewChannel(NewChannelAction::SetupRetried(
                        daemon.retry_setup(&setup).await,
                    ))
                });
            }
            NewChannelAction::SetupStopped(Ok(())) => {
                if let Some(setup) = sheet.setup.take() {
                    Arc::make_mut(&mut state.workspace)
                        .setups
                        .retain(|found| found.id != setup.id);
                }
                state.overlays.pop();
            }
            NewChannelAction::SetupStopped(Err(error))
            | NewChannelAction::SetupRetried(Err(error)) => {
                sheet.error = Some(error.to_string());
            }
            NewChannelAction::SetupRetried(Ok(id)) => return self.channel_created(state, Ok(id)),
        }
        Effect::none()
    }

    /// A channel setup moved on: keep it in the workspace and in the sheet;
    /// when every step is done the sheet closes and the channel opens.
    pub(super) fn heard_setup(
        &self,
        state: &mut AppState,
        setup: ChannelSetup,
    ) -> Effect<AppAction> {
        let workspace = Arc::make_mut(&mut state.workspace);
        let done = !setup.steps.is_empty()
            && setup
                .steps
                .iter()
                .all(|step| step.state == SetupState::Done);
        match workspace.setups.iter_mut().find(|found| found.id == setup.id) {
            Some(found) => *found = setup.clone(),
            None => workspace.setups.push(setup.clone()),
        }
        let sheet = state
            .overlays
            .iter_mut()
            .rev()
            .find_map(|overlay| match overlay {
                Overlay::NewChannel(sheet) => Some(sheet),
                _ => None,
            });
        let Some(sheet) = sheet else {
            return Effect::none();
        };
        // The sheet's own setup: the one it already shows, or the one named
        // as the sheet's channel.
        let mine = match &sheet.setup {
            Some(shown) => shown.id == setup.id,
            None => sheet.folder() == setup.name,
        };
        if !mine {
            return Effect::none();
        }
        // Done opens the channel once it is listed. A live channel's id isn't
        // its name, so a channel not listed yet is opened by the create's
        // reply (`channel_created`) instead.
        let listed = done
            .then(|| {
                state
                    .workspace
                    .channels
                    .iter()
                    .find(|channel| channel.name == setup.name)
                    .map(|channel| channel.id.clone())
            })
            .flatten();
        if let Some(id) = listed {
            Arc::make_mut(&mut state.workspace)
                .setups
                .retain(|found| found.id != setup.id);
            state
                .overlays
                .retain(|overlay| !matches!(overlay, Overlay::NewChannel(_)));
            return self.navigate(state, Route::Channel(id));
        }
        sheet.error = None;
        sheet.setup = Some(setup);
        Effect::none()
    }

    pub(super) fn channel_created(
        &self,
        state: &mut AppState,
        result: Result<ChannelId, DaemonError>,
    ) -> Effect<AppAction> {
        let sheet = state
            .overlays
            .iter_mut()
            .rev()
            .find_map(|overlay| match overlay {
                Overlay::NewChannel(sheet) => Some(sheet),
                _ => None,
            });
        match result {
            Ok(id) => {
                state
                    .overlays
                    .retain(|overlay| !matches!(overlay, Overlay::NewChannel(_)));
                self.navigate(state, Route::Channel(id))
            }
            Err(error) => {
                if let Some(sheet) = sheet {
                    sheet.creating = false;
                    // The failed row already says why and offers Retry.
                    if sheet.setup.is_none() {
                        sheet.error = Some(error.to_string());
                    }
                }
                Effect::none()
            }
        }
    }
}

/// Takes `item` out of `items` when it is there, and adds it at the end when
/// it is not.
fn toggle<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if let Some(at) = items.iter().position(|found| *found == item) {
        items.remove(at);
    } else {
        items.push(item);
    }
}
