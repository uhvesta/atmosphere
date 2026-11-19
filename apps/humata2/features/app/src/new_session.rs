use humata2_clients::DaemonError;
use humata2_clients::model::{
    Agent, ChannelId, ItemId, MachineId, SessionId, SessionOptions,
};
use humata2_clients::progress::{OpKind, OpProgress, OpUpdate};
use tca::{Effect, Reducer};

use crate::{App, AppAction, AppState, Overlay, Route};

/// The New session sheet (SE-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSession {
    /// Its channel.
    pub channel: Option<ChannelId>,
    /// Its agent.
    pub agent: Agent,
    /// Its host.
    pub machine: Option<MachineId>,
    /// Its title.
    pub title: String,
    /// Typed once the agent starts.
    pub first_message: String,
    /// Repositories left out of this session.
    pub excluded_repos: std::collections::BTreeSet<String>,
    /// The board item this session starts for.
    pub item: Option<ItemId>,
    /// The picker currently open.
    pub picker: Option<NewSessionPicker>,
    /// Waiting for the daemon.
    pub creating: bool,
    /// The running step of the operation while `creating`.
    pub progress: Option<OpProgress>,
    /// The operation id `progress` follows; latched by the first update.
    pub op: Option<String>,
    /// Why it couldn't be made.
    pub error: Option<String>,
}

impl NewSession {
    /// The branch preview and the branch passed to the host.
    pub fn branch(&self) -> String {
        let slug = self
            .title
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            .join("-");
        if slug.is_empty() {
            "new-session".into()
        } else {
            slug
        }
    }
}

/// A New session picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewSessionPicker {
    Channel,
    Machine,
}

/// A change to the New session sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewSessionAction {
    /// Edit its first message.
    FirstMessage(String),
    /// Include or leave out one channel repository.
    ToggleRepo(String),
    /// Open/close a picker.
    Picker(Option<NewSessionPicker>),
    /// Pick its channel.
    Channel(ChannelId),
    /// Pick its agent.
    Agent(Agent),
    /// Pick its host.
    Machine(MachineId),
    /// Its title changed.
    Title(String),
    /// Start it.
    Create,
}

/// The agent a new session starts with. In a channel, the one the daemon
/// says a start takes there (D770, ST-15: the channel's own default, then
/// Settings', then the one it was made with); with no channel, or before the
/// daemon has said, the default agent picked in Settings. Either way, the
/// first agent that is on when that one has been turned off (AGENTSET).
pub(crate) fn default_agent(state: &AppState, channel: Option<&ChannelId>) -> Agent {
    let preferred = channel
        .and_then(|id| state.workspace.channels.iter().find(|channel| &channel.id == id))
        .and_then(|channel| channel.default_agent)
        .unwrap_or(state.settings.preferences.default_agent);
    let offered = state.settings.offered_agents();
    if preferred == Agent::Shell || offered.contains(&preferred) {
        return preferred;
    }
    offered.first().copied().unwrap_or(preferred)
}

impl App {
    pub(super) fn start_new_session(&self, state: &mut AppState) -> Effect<AppAction> {
        let channel = match &state.route {
            Route::Channel(id) => Some(id.clone()),
            Route::Session(id) if state.is_desk_session(id) => {
                state.workspace.channels.first().map(|c| c.id.clone())
            }
            Route::Session(id) => state.workspace.session(id).map(|s| s.channel.clone()),
            _ => state.workspace.channels.first().map(|c| c.id.clone()),
        };
        let machine = channel.as_ref().and_then(|id| {
            state
                .workspace
                .channel_machines(id)
                .find(|machine| machine.online)
                .map(|machine| machine.id.clone())
        });
        let agent = default_agent(state, channel.as_ref());
        state.overlays.push(Overlay::NewSession(NewSession {
            channel,
            agent,
            machine,
            title: String::new(),
            first_message: String::new(),
            excluded_repos: Default::default(),
            item: None,
            picker: None,
            creating: false,
            progress: None,
            op: None,
            error: None,
        }));
        Effect::none()
    }

    pub(super) fn start_new_for_item(&self, state: &mut AppState, id: ItemId) -> Effect<AppAction> {
        let Some(item) = state
            .workspace
            .items
            .iter()
            .find(|item| item.id == id)
            .cloned()
        else {
            return Effect::none();
        };
        let effect = self.reduce(state, AppAction::StartNewSession);
        if let Some(Overlay::NewSession(sheet)) = state.overlays.last_mut() {
            sheet.title = item.title;
            sheet.item = Some(id);
            if let Some(channel) = item.scope.channel().cloned() {
                sheet.machine = state
                    .workspace
                    .channel_machines(&channel)
                    .find(|m| m.online)
                    .map(|m| m.id.clone());
                sheet.channel = Some(channel);
            }
            if let Some(machine) = item
                .session
                .as_ref()
                .and_then(|id| state.workspace.session(id))
                .and_then(|s| state.workspace.machine(&s.machine))
                .filter(|m| m.online)
            {
                sheet.machine = Some(machine.id.clone());
            }
        }
        effect
    }

    pub(super) fn change_new_session(
        &self,
        state: &mut AppState,
        change: NewSessionAction,
    ) -> Effect<AppAction> {
        let Some(Overlay::NewSession(sheet)) = state.overlays.last_mut() else {
            return Effect::none();
        };
        if sheet.creating {
            return Effect::none();
        }
        match change {
            NewSessionAction::Channel(id) => {
                if !state
                    .workspace
                    .channel_machines(&id)
                    .any(|m| m.online && Some(&m.id) == sheet.machine.as_ref())
                {
                    sheet.machine = state
                        .workspace
                        .channel_machines(&id)
                        .find(|m| m.online)
                        .map(|m| m.id.clone());
                }
                sheet.channel = Some(id);
                sheet.excluded_repos.clear();
                sheet.picker = None;
            }
            NewSessionAction::Agent(agent) => {
                sheet.agent = agent;
            }
            NewSessionAction::Machine(id) => {
                sheet.machine = Some(id);
                sheet.picker = None;
            }
            NewSessionAction::FirstMessage(text) => sheet.first_message = text,
            NewSessionAction::Picker(picker) => sheet.picker = picker,
            NewSessionAction::ToggleRepo(repo) => {
                if !sheet.excluded_repos.remove(&repo) {
                    sheet.excluded_repos.insert(repo);
                }
            }
            NewSessionAction::Title(title) => sheet.title = title,
            NewSessionAction::Create => {
                let (Some(channel), Some(machine)) = (sheet.channel.clone(), sheet.machine.clone())
                else {
                    sheet.error = Some(
                        "Pick a channel and a host first. Add a host on the Hosts page if none is listed.".to_owned(),
                    );
                    return Effect::none();
                };
                let title = if sheet.title.trim().is_empty() {
                    format!("New {} session", sheet.agent.name())
                } else {
                    sheet.title.trim().to_owned()
                };
                sheet.creating = true;
                sheet.progress = None;
                sheet.op = None;
                sheet.error = None;
                if !state
                    .workspace
                    .channel_machines(&channel)
                    .any(|m| m.online && m.id == machine)
                {
                    sheet.creating = false;
                    sheet.progress = None;
                    sheet.op = None;
                    sheet.error=Some("Choose a machine connected to this channel, or add the machine to the channel first.".into());
                    return Effect::none();
                }
                let options = SessionOptions {
                    model: None,
                    repos: state
                        .workspace
                        .channel(&channel)
                        .map_or_else(Vec::new, |c| {
                            c.repos
                                .iter()
                                .filter(|repo| !sheet.excluded_repos.contains(*repo))
                                .cloned()
                                .collect()
                        }),
                    first_message: sheet.first_message.clone(),
                    item: sheet.item.clone(),
                    branch: sheet.branch(),
                };
                let agent = sheet.agent;
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    AppAction::SessionCreated(
                        daemon
                            .create_session(&channel, agent, &machine, &title, options)
                            .await,
                    )
                });
            }
        }
        Effect::none()
    }

    pub(super) fn session_created(
        &self,
        state: &mut AppState,
        result: Result<SessionId, DaemonError>,
    ) -> Effect<AppAction> {
        let sheet = state
            .overlays
            .iter_mut()
            .rev()
            .find_map(|overlay| match overlay {
                Overlay::NewSession(sheet) => Some(sheet),
                _ => None,
            });
        match result {
            Ok(id) => {
                state
                    .overlays
                    .retain(|overlay| !matches!(overlay, Overlay::NewSession(_)));
                self.navigate(state, Route::Session(id))
            }
            Err(error) => {
                if let Some(sheet) = sheet {
                    sheet.creating = false;
                    sheet.progress = None;
                    sheet.op = None;
                    sheet.error = Some(error.to_string());
                }
                Effect::none()
            }
        }
    }
}

impl App {
    /// A watched operation moved on: the waiting sheet keeps its progress.
    pub(super) fn new_session_operation(&self, state: &mut AppState, update: OpUpdate) {
        if update.kind != OpKind::StartSession {
            return;
        }
        let sheet = state.overlays.iter_mut().rev().find_map(|overlay| match overlay {
            Overlay::NewSession(sheet) => Some(sheet),
            _ => None,
        });
        if let Some(sheet) = sheet.filter(|sheet| sheet.creating) {
            humata2_session::follow_operation(&mut sheet.op, &mut sheet.progress, update);
        }
    }
}
