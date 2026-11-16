use humata2_clients::DaemonError;
use humata2_clients::model::{ChannelId, ChannelSetup, SetupState};
use std::sync::Arc;
use tca::Effect;

use crate::{App, AppAction, AppState, Overlay, Route};

/// The New channel sheet (CH-1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewChannel {
    /// Its name.
    pub name: String,
    /// Its repositories, comma-separated as typed.
    pub repos: String,
    /// Waiting for the daemon.
    pub creating: bool,
    /// Why it couldn't be made.
    pub error: Option<String>,
    /// Its folders and clones while they run (CH-5); the sheet shows them in
    /// place of the form.
    pub setup: Option<ChannelSetup>,
}

/// A change to the New channel sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewChannelAction {
    /// Its name changed.
    Name(String),
    /// Its repositories changed.
    Repos(String),
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
    pub(super) fn start_new_channel(&self, state: &mut AppState) -> Effect<AppAction> {
        state
            .overlays
            .push(Overlay::NewChannel(NewChannel::default()));
        Effect::none()
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
            NewChannelAction::Repos(repos) => sheet.repos = repos,
            NewChannelAction::Create => {
                let name = sheet.name.trim().trim_start_matches('#').to_owned();
                if name.is_empty() {
                    sheet.error = Some("Give the channel a name.".to_owned());
                    return Effect::none();
                }
                // DK-2: the Desk's folder name is never a channel's.
                if name.to_lowercase().replace(' ', "-") == humata2_clients::desk::FOLDER {
                    sheet.error = Some(
                        "“desk” is kept for the Desk's folder. Give the channel another name."
                            .to_owned(),
                    );
                    return Effect::none();
                }
                let repos = sheet
                    .repos
                    .split(',')
                    .map(str::trim)
                    .filter(|repo| !repo.is_empty())
                    .map(str::to_owned)
                    .collect();
                sheet.creating = true;
                sheet.error = None;
                let machines = state
                    .workspace
                    .machines
                    .iter()
                    .filter(|machine| machine.online)
                    .map(|machine| machine.id.clone())
                    .take(1)
                    .collect();
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
            None => {
                sheet
                    .name
                    .trim()
                    .trim_start_matches('#')
                    .to_lowercase()
                    .replace(' ', "-")
                    == setup.name
            }
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
