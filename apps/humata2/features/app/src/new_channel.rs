use humata2_clients::DaemonError;
// REPOS (RL-5): the picker is the one repo search.
use crate::repo_search::{self, PickRow, RepoPicker, RepoSearchAction, RepoSearchState};
use humata2_clients::model::{
    Channel, ChannelId, ChannelSetup, Machine, MachineId, MachineKind, SetupState,
};
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
        slug(&self.name)
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

    /// A channel setup moved on: keep it in the workspace while running and in
    /// the sheet until the channel is listed; once the channel is in the
    /// workspace the sheet closes right away and the channel opens while any
    /// remaining clone steps carry on in `workspace.setups`.
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
        if done {
            workspace.setups.retain(|found| found.id != setup.id);
        } else {
            match workspace.setups.iter_mut().find(|found| found.id == setup.id) {
                Some(found) => *found = setup.clone(),
                None => workspace.setups.push(setup.clone()),
            }
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
        // as the sheet's channel (matching either slug or display name).
        let mine = match &sheet.setup {
            Some(shown) => shown.id == setup.id,
            None => matches_sheet(sheet, &setup.name),
        };
        if !mine {
            return Effect::none();
        }
        // Once the channel is listed in the workspace, close the sheet and
        // open it immediately without waiting for clone steps to reach Done.
        // A live channel's id isn't its name, so a channel not listed yet is
        // opened by the create's reply (`channel_created`) instead.
        let listed = state
            .workspace
            .channels
            .iter()
            .find(|channel| channel_matches_name(channel, &setup.name))
            .map(|channel| channel.id.clone());
        if let Some(id) = listed {
            state
                .overlays
                .retain(|overlay| !matches!(overlay, Overlay::NewChannel(_)));
            return self.navigate(state, Route::Channel(id));
        }
        sheet.error = None;
        sheet.setup = Some(setup);
        Effect::none()
    }

    /// After a workspace reload, if a New channel sheet is creating a channel
    /// that is now listed in the workspace, close the sheet and open it right
    /// away without waiting for clone steps to reach `Done`.
    pub(super) fn loaded_new_channel(&self, state: &mut AppState) -> Effect<AppAction> {
        let listed = state
            .overlays
            .iter()
            .rev()
            .find_map(|overlay| match overlay {
                Overlay::NewChannel(sheet) if sheet.creating || sheet.setup.is_some() => Some(sheet),
                _ => None,
            })
            .and_then(|sheet| {
                let setup_name = sheet.setup.as_ref().map(|setup| setup.name.as_str());
                state
                    .workspace
                    .channels
                    .iter()
                    .find(|channel| {
                        matches_sheet(sheet, &channel.name)
                            || setup_name.is_some_and(|name| channel_matches_name(channel, name))
                    })
                    .map(|channel| channel.id.clone())
            });
        let Some(id) = listed else {
            return Effect::none();
        };
        state
            .overlays
            .retain(|overlay| !matches!(overlay, Overlay::NewChannel(_)));
        self.navigate(state, Route::Channel(id))
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
                // The reply can come after the setup's last step already
                // closed the sheet and opened the channel, and after the
                // person opened a fresh New channel sheet: that one asked
                // nothing (neither creating nor showing a setup), so it
                // stays, and the page isn't changed under it.
                if !sheet.is_some_and(|sheet| sheet.creating || sheet.setup.is_some()) {
                    return Effect::none();
                }
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
                    Effect::none()
                } else {
                    // SMB-004: surface late create failure when sheet was dismissed.
                    let clock = self.clients.clock.clone();
                    state.toast(error.to_string(), &clock)
                }
            }
        }
    }
}

/// The folder slug for a channel name (CH-2): lowercase, every run of
/// characters that are not ASCII letters or digits as one `-`, and none at
/// either end. "  # Q4 / Billing!! " gives "q4-billing".
fn slug(name: &str) -> String {
    let mut folder = String::new();
    let mut pending = false;
    for c in name.chars() {
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

/// Whether `name` (either a display name like "Q4 Billing" or a slug like
/// "q4-billing") matches the channel the sheet is creating.
fn matches_sheet(sheet: &NewChannel, name: &str) -> bool {
    let folder = sheet.folder();
    let trimmed = sheet.name.trim().trim_start_matches('#');
    let setup_slug = slug(name);
    (!folder.is_empty() && folder == setup_slug)
        || (!trimmed.is_empty() && trimmed == name.trim())
}

/// Whether `channel` matches a setup's `name` (either as typed or as slug).
fn channel_matches_name(channel: &Channel, name: &str) -> bool {
    let setup_slug = slug(name);
    channel.name == name
        || (!setup_slug.is_empty()
            && (channel.name == setup_slug || slug(&channel.name) == setup_slug))
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
