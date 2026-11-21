//! The "Add a GitHub repo…" sheet of a channel (CH-6, CH-18, CH-19, frame
//! 20). It opens from the channel's Repos and machines tab or its empty
//! Sessions tab, embeds the one repo search (RL-5), and picking a repo
//! clones it into the channel. A GitHub address pasted into the field is
//! read as `owner/name`. The sheet shows no path and offers no local folder:
//! the daemon places the clone by the channel's root rules.

use humata2_clients::DaemonError;
use humata2_clients::github::parse_repo_text;
use humata2_clients::model::ChannelId;
use tca::Effect;

use crate::repo_search::{self, RepoPicker, RepoSearchAction, RepoSearchState};
use crate::{App, AppAction, AppState, Overlay};

/// Ids and in-flight calls of this sheet's search.
const SEARCH_KEY: &str = "channel-add-repo";

/// The sheet's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelAddRepo {
    /// The channel the repo is added to.
    pub channel: ChannelId,
    /// What is typed or pasted in the field.
    pub query: String,
    /// The one repo search the field shows.
    pub search: RepoSearchState,
    /// The repo being added, while the daemon works on it.
    pub adding: Option<String>,
    /// Why the last add did not happen, and what to do; shown inline.
    pub error: Option<String>,
}

/// A change to the sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelAddRepoAction {
    /// Open the sheet for this channel.
    Open(ChannelId),
    /// The field's text changed.
    Query(String),
    /// A row was clicked: add this repo.
    Pick(String),
    /// Return, or the Add button: add what is typed or pasted.
    Submit,
    /// The inline error was dismissed.
    DismissError,
    /// The daemon cloned the repo into the channel, or refused.
    Added(Result<(), DaemonError>),
}

fn sheet(state: &mut AppState) -> Option<&mut ChannelAddRepo> {
    state
        .overlays
        .iter_mut()
        .rev()
        .find_map(|overlay| match overlay {
            Overlay::ChannelAddRepo(sheet) => Some(sheet),
            _ => None,
        })
}

impl App {
    pub(super) fn change_channel_add_repo(
        &self,
        state: &mut AppState,
        action: ChannelAddRepoAction,
    ) -> Effect<AppAction> {
        match action {
            ChannelAddRepoAction::Open(channel) => {
                if matches!(state.overlays.last(), Some(Overlay::ChannelAddRepo(_))) {
                    return Effect::none();
                }
                state.overlays.push(Overlay::ChannelAddRepo(ChannelAddRepo {
                    channel,
                    query: String::new(),
                    search: RepoSearchState::default(),
                    adding: None,
                    error: None,
                }));
                // The picker opens on the library, then asks GitHub (RL-3).
                self.repo_search(state, RepoPicker::ChannelAddRepo, RepoSearchAction::Open)
            }
            ChannelAddRepoAction::Query(text) => {
                let Some(sheet) = sheet(state) else {
                    return Effect::none();
                };
                sheet.query.clone_from(&text);
                sheet.error = None;
                // A pasted address searches as the repo it names.
                let searched = parse_repo_text(&text).unwrap_or(text);
                repo_search::reduce(
                    &self.clients,
                    &mut sheet.search,
                    RepoSearchAction::QueryChanged(searched),
                    SEARCH_KEY,
                )
                .map(|action| AppAction::RepoSearch(RepoPicker::ChannelAddRepo, action))
            }
            ChannelAddRepoAction::Submit => {
                let Some(typed) = sheet(state).map(|sheet| sheet.query.clone()) else {
                    return Effect::none();
                };
                self.add_repo_from_sheet(state, &typed)
            }
            ChannelAddRepoAction::Pick(repo) => self.add_repo_from_sheet(state, &repo),
            ChannelAddRepoAction::DismissError => {
                if let Some(sheet) = sheet(state) {
                    sheet.error = None;
                }
                Effect::none()
            }
            ChannelAddRepoAction::Added(Ok(())) => {
                state
                    .overlays
                    .retain(|overlay| !matches!(overlay, Overlay::ChannelAddRepo(_)));
                // The channel lists the new repo once the workspace is read again.
                self.load()
            }
            ChannelAddRepoAction::Added(Err(error)) => {
                if let Some(sheet) = sheet(state) {
                    sheet.adding = None;
                    sheet.error = Some(error.to_string());
                    Effect::none()
                } else {
                    // SMB-004: surface late add-repo failure when sheet was dismissed.
                    let clock = self.clients.clock.clone();
                    state.toast(error.to_string(), &clock)
                }
            }
        }
    }

    /// Adds the repo `text` names to the sheet's channel.
    fn add_repo_from_sheet(&self, state: &mut AppState, text: &str) -> Effect<AppAction> {
        let Some((channel, adding)) = sheet(state).map(|sheet| (sheet.channel.clone(), sheet.adding.is_some()))
        else {
            return Effect::none();
        };
        if adding {
            return Effect::none();
        }
        let named = parse_repo_text(text).map_err(|error| error.to_string()).and_then(|repo| {
            match state.workspace.channel(&channel) {
                Some(found) if found.repos.iter().any(|held| held.eq_ignore_ascii_case(&repo)) => Err(format!(
                    "#{} already has {repo}. Pick another repo to add",
                    found.name
                )),
                _ => Ok(repo),
            }
        });
        let Some(sheet) = sheet(state) else {
            return Effect::none();
        };
        let repo = match named {
            Ok(repo) => repo,
            Err(error) => {
                sheet.error = Some(error);
                return Effect::none();
            }
        };
        sheet.adding = Some(repo.clone());
        sheet.error = None;
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            AppAction::ChannelAddRepo(ChannelAddRepoAction::Added(
                daemon.add_repo_to_channel(&channel, &repo).await,
            ))
        })
    }
}
