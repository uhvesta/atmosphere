use humata2_clients::DaemonError;
use humata2_clients::github::matching;
use humata2_clients::model::{ChannelId, ChannelSetup, MachineId, MachineKind, SetupState};
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
    /// The repos `gh` sees on the host, in the order it lists them; `None`
    /// while they are being listed or when they could not be listed.
    pub listed: Option<Vec<String>>,
    /// Why the repos could not be listed, and what to do about it; `None`
    /// when they are listed or still being listed.
    pub repos_error: Option<String>,
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

    /// The repos the list shows (CH-3): the ticked repos in tick order, shown
    /// whether or not they match, then the listed repos the query matches that
    /// are not ticked, at most 50 in all. Empty while the repos are not listed.
    #[must_use]
    pub fn shown_repos(&self) -> Vec<&str> {
        let Some(listed) = self.listed.as_deref() else {
            return Vec::new();
        };
        self.repos
            .iter()
            .map(String::as_str)
            .chain(self.unticked_matches().into_iter().filter_map(|repo| {
                listed
                    .iter()
                    .find(|found| **found == repo)
                    .map(String::as_str)
            }))
            .take(REPO_LIMIT)
            .collect()
    }

    /// How many matching repos the 50-repo cap leaves out of
    /// [`shown_repos`](Self::shown_repos), for an "and N more; type to narrow"
    /// line.
    #[must_use]
    pub fn more_repos(&self) -> usize {
        let room = REPO_LIMIT.saturating_sub(self.repos.len());
        self.unticked_matches().len().saturating_sub(room)
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

    /// The listed repos the query matches and that are not ticked, in the
    /// order `matching` ranks them.
    fn unticked_matches(&self) -> Vec<String> {
        let Some(listed) = self.listed.as_deref() else {
            return Vec::new();
        };
        matching(&self.query, listed)
            .into_iter()
            .filter(|repo| !self.repos.contains(repo))
            .collect()
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
    /// The daemon listed the repos, or could not.
    ReposListed(Result<Vec<String>, DaemonError>),
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
        self.list_repos()
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
            NewChannelAction::Query(query) => sheet.query = query,
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
                sheet.repos_error = None;
                sheet.listed = None;
                return self.list_repos();
            }
            NewChannelAction::ReposListed(Ok(repos)) => {
                sheet.listed = Some(repos);
                sheet.repos_error = None;
            }
            NewChannelAction::ReposListed(Err(error)) => {
                sheet.listed = None;
                sheet.repos_error = Some(error.to_string());
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

    /// Asks the daemon for the repos gh sees on the host (CH-3).
    fn list_repos(&self) -> Effect<AppAction> {
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            AppAction::NewChannel(NewChannelAction::ReposListed(
                daemon.github_repos("").await,
            ))
        })
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
