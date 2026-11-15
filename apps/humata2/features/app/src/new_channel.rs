use humata2_clients::DaemonError;
use humata2_clients::model::ChannelId;
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
        }
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
                    sheet.error = Some(error.to_string());
                }
                Effect::none()
            }
        }
    }
}
