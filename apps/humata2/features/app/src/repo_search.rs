//! The one repo search (RL-5 to RL-10, D470 to D479), as every picker uses
//! it: New channel, the Desk's Add repo, and anything later. The state holds
//! what was typed, the scope of this search (None is the saved default), the
//! last answer and what to say when GitHub could not be searched. The
//! library group is answered at once from the last library; GitHub is asked
//! after typing pauses.

use humata2_clients::github::matching;
use humata2_clients::repos::{
    FoundRepo, LibraryRepo, RepoPlace, RepoScope, RepoSearch, RepoSearchProblemKind,
};
use humata2_clients::{Clients, DaemonError, Workspace};
use std::time::Duration;
use tca::Effect;

use crate::{App, AppAction, AppState, NewChannel, Overlay};

/// How long typing pauses before GitHub is asked (RL-9, D477).
pub const PAUSE: Duration = Duration::from_millis(300);

/// Which picker a repo search belongs to. Every action and every effect
/// carries it, so a late answer for a sheet that closed finds no search and
/// is dropped instead of landing in another one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoPicker {
    /// The New channel sheet's Repos field (CH-3).
    NewChannel,
    /// A Desk session's Add repo sheet (DK-3).
    DeskAddRepo,
    /// Add a GitHub repo… on a channel (CH-18).
    ChannelAddRepo,
    /// The New desk session form's repo field (DK-14).
    NewDeskSession,
    /// A running session's Add repo to this session sheet (CH-21).
    SessionAddRepo,
}

impl RepoPicker {
    /// The picker on screen, if one is: the New channel sheet when it is
    /// on top, else a Desk session's Add repo.
    #[must_use]
    pub fn on_screen(state: &AppState) -> Option<Self> {
        match state.overlays.last() {
            Some(Overlay::NewChannel(_)) => Some(Self::NewChannel),
            Some(Overlay::SessionAddRepo(_)) => Some(Self::SessionAddRepo),
            Some(Overlay::ChannelAddRepo(_)) => Some(Self::ChannelAddRepo),
            Some(_) => None,
            None if state.desk.new.as_ref().is_some_and(|form| form.adding_repo) => {
                Some(Self::NewDeskSession)
            }
            None => matches!(&state.desk.menu, Some(humata2_desk::Menu::AddRepo(_)))
                .then_some(Self::DeskAddRepo),
        }
    }
}

/// A chip in the scope row (RL-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoChip {
    /// One GitHub org or user.
    Owner(String),
    /// The user's own repos, the "you" chip.
    Me,
    /// The repos `gh` lists as owner, collaborator or member.
    YourRepos,
    /// Every public repo; this search only.
    AllOfGitHub,
}

/// A chip as the field draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipView {
    /// Which chip it is, and what its ✕ removes.
    pub chip: RepoChip,
    /// What it says.
    pub label: String,
    /// GitHub does not know this owner (RL-10).
    pub unknown: bool,
}

/// Which group a picker row is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoGroup {
    /// The repos the user added before (RL-6).
    Library,
    /// What `gh` found within the scope.
    GitHub,
}

/// One row of the picker list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRow {
    /// `owner/name`.
    pub repo: String,
    /// Its group; `None` for a ticked repo, which stays first whatever
    /// matched (D277).
    pub group: Option<RepoGroup>,
    /// The library entry, for "used … ago · where".
    pub library: Option<LibraryRepo>,
    /// GitHub's description; empty when it has none or it is a library row.
    pub description: String,
    /// Whether GitHub says it is private.
    pub private: bool,
    /// Whether it is ticked.
    pub ticked: bool,
}

/// What a repo search shows and remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoSearchState {
    /// What was typed.
    pub query: String,
    /// This search's scope; `None` is the saved default (RL-8).
    pub scope: Option<RepoScope>,
    /// The saved default scope, as the last answer told it.
    pub saved: RepoScope,
    /// The last answer.
    pub result: Option<RepoSearch>,
    /// The whole library, as the last search with nothing typed told it.
    pub library: Vec<LibraryRepo>,
    /// A search is out or about to go out.
    pub loading: bool,
    /// The daemon could not be asked at all, and what to do.
    pub failed: Option<String>,
    /// The owner being typed after **+ Org**; `None` when it is closed.
    pub adding_owner: Option<String>,
    /// Save as default is being written.
    pub saving: bool,
    /// The saved default could not be written, and why.
    pub save_error: Option<String>,
    /// The orgs the user's GitHub login belongs to, once **+ Org** asked.
    pub github_owners: Vec<String>,
    /// The time the daemon gave when this search last answered, in
    /// milliseconds since the epoch; "used ... ago" is worked out from it.
    pub now_ms: i64,
    owners_asked: bool,
    seq: u64,
    asked: String,
}

/// A change to a repo search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoSearchAction {
    /// The picker opened: the library and the saved scope, at once.
    Open,
    /// Search again after GitHub or the daemon could not be asked.
    Retry,
    /// The text changed; GitHub is asked once typing pauses.
    QueryChanged(String),
    /// The pause after typing `0` ended.
    Paused(u64),
    /// A chip's ✕: the search narrows (RL-8).
    ScopeChipRemoved(RepoChip),
    /// **+ Org** opened its field.
    AddOwnerOpened,
    /// The daemon listed the user's orgs, or could not.
    OwnersListed(Result<Vec<String>, DaemonError>),
    /// **+ Org** was dismissed.
    AddOwnerClosed,
    /// The owner being typed changed.
    OwnerTextChanged(String),
    /// Return in the owner field: add what was typed.
    OwnerEntered,
    /// An owner joins this search's scope.
    OwnerAdded(String),
    /// **All of GitHub**, for this search only.
    AllOfGitHubPicked,
    /// Back to the saved scope.
    ScopeReset,
    /// **Save as default**.
    SaveScopeAsDefault,
    /// The daemon saved the scope, or refused.
    ScopeSaved(Result<RepoScope, DaemonError>),
    /// The daemon answered search number `seq`.
    Results {
        /// Which search it answers.
        seq: u64,
        /// The answer.
        result: RepoSearch,
    },
    /// The daemon could not be asked for search number `seq`.
    Failed {
        /// Which search failed.
        seq: u64,
        /// Why.
        error: DaemonError,
    },
}

impl RepoSearchState {
    /// The scope this search runs in.
    #[must_use]
    pub fn scope(&self) -> &RepoScope {
        self.scope.as_ref().unwrap_or(&self.saved)
    }

    /// Whether **Save as default** is offered: the scope differs from the
    /// saved one, and it is a scope that can be saved (All of GitHub is one
    /// search only, RL-8).
    #[must_use]
    pub fn can_save_default(&self) -> bool {
        match &self.scope {
            Some(RepoScope::AllOfGitHub) | None => false,
            Some(scope) => *scope != self.saved,
        }
    }

    /// Whether the scope was changed for this search.
    #[must_use]
    pub fn widened(&self) -> bool {
        self.scope.as_ref().is_some_and(|scope| *scope != self.saved)
    }

    /// The chips before the text, with owners GitHub does not know marked
    /// (RL-8, RL-10).
    #[must_use]
    pub fn chips(&self) -> Vec<ChipView> {
        let unknown = |owner: &str| {
            self.result.as_ref().is_some_and(|result| {
                result
                    .unknown_owners
                    .iter()
                    .any(|found| found.eq_ignore_ascii_case(owner))
            })
        };
        match self.scope() {
            RepoScope::YourRepos => vec![ChipView {
                chip: RepoChip::YourRepos,
                label: "Your repos".to_owned(),
                unknown: false,
            }],
            RepoScope::AllOfGitHub => vec![ChipView {
                chip: RepoChip::AllOfGitHub,
                label: "All of GitHub".to_owned(),
                unknown: false,
            }],
            RepoScope::Owners { owners, include_me } => owners
                .iter()
                .map(|owner| ChipView {
                    chip: RepoChip::Owner(owner.clone()),
                    label: owner.clone(),
                    unknown: unknown(owner),
                })
                .chain(include_me.then(|| ChipView {
                    chip: RepoChip::Me,
                    label: "you".to_owned(),
                    unknown: false,
                }))
                .collect(),
        }
    }

    /// The GitHub group's heading, naming the scope ("GitHub · bazelbuild,
    /// you").
    #[must_use]
    pub fn github_heading(&self) -> String {
        match self.scope() {
            RepoScope::YourRepos => "GitHub · your repos".to_owned(),
            RepoScope::AllOfGitHub => "GitHub · all of GitHub".to_owned(),
            RepoScope::Owners { owners, include_me } => {
                let names: Vec<&str> = owners
                    .iter()
                    .map(String::as_str)
                    .chain(include_me.then_some("you"))
                    .collect();
                if names.is_empty() {
                    "GitHub".to_owned()
                } else {
                    format!("GitHub · {}", names.join(", "))
                }
            }
        }
    }

    /// Owners worth suggesting after **+ Org**: the library's owners that
    /// the scope does not list yet (RL-8).
    #[must_use]
    pub fn owner_suggestions(&self) -> Vec<String> {
        let listed: &[String] = match self.scope() {
            RepoScope::Owners { owners, .. } => owners,
            _ => &[],
        };
        humata2_clients::repos::suggest_owners(&self.library, &self.github_owners, listed)
    }

    /// Why GitHub's group is missing, when it is and it is a failure to
    /// report: the call failing, or the answer's problem (RL-10). Too short
    /// is a hint, not a failure.
    #[must_use]
    pub fn problem(&self) -> Option<&str> {
        if let Some(failed) = &self.failed {
            return Some(failed);
        }
        let problem = self.result.as_ref()?.problem.as_ref()?;
        (problem.kind != RepoSearchProblemKind::TooShort).then_some(problem.message.as_str())
    }

    /// The hint to type more, when the query is too short for GitHub.
    #[must_use]
    pub fn too_short(&self) -> Option<&str> {
        let problem = self.result.as_ref()?.problem.as_ref()?;
        (problem.kind == RepoSearchProblemKind::TooShort).then_some(problem.message.as_str())
    }

    /// Whether nothing has answered yet and nothing has failed.
    #[must_use]
    pub fn listing(&self) -> bool {
        self.result.is_none() && self.failed.is_none()
    }

    /// The picker list: the ticked repos in tick order (D277), then the
    /// library group, then the GitHub group, at most `limit` rows, and how
    /// many matches the limit left out. The library group is narrowed from
    /// the last library at once; GitHub's group is the last answer narrowed
    /// the same way until the next answer comes (RL-6, D476).
    #[must_use]
    pub fn rows(&self, ticked: &[String], limit: usize) -> (Vec<PickRow>, usize) {
        let library_names: Vec<String> = self.library.iter().map(|repo| repo.repo.clone()).collect();
        let library: Vec<&LibraryRepo> = matching(&self.query, &library_names)
            .iter()
            .filter_map(|name| self.library.iter().find(|repo| &repo.repo == name))
            .collect();
        let found: &[FoundRepo] = self
            .result
            .as_ref()
            .map(|result| result.github.as_slice())
            .unwrap_or_default();
        let found_names: Vec<String> = found.iter().map(|repo| repo.repo.clone()).collect();
        let github: Vec<&FoundRepo> = matching(&self.query, &found_names)
            .iter()
            .filter_map(|name| found.iter().find(|repo| &repo.repo == name))
            .filter(|repo| {
                !self
                    .library
                    .iter()
                    .any(|held| held.repo.eq_ignore_ascii_case(&repo.repo))
            })
            .collect();

        let is_ticked = |repo: &str| ticked.iter().any(|picked| picked == repo);
        let mut rows: Vec<PickRow> = ticked
            .iter()
            .map(|repo| {
                let held = self.library.iter().find(|held| &held.repo == repo);
                let seen = found.iter().find(|seen| &seen.repo == repo);
                PickRow {
                    repo: repo.clone(),
                    group: None,
                    library: held.cloned(),
                    description: seen.map(|seen| seen.description.clone()).unwrap_or_default(),
                    private: seen.is_some_and(|seen| seen.is_private),
                    ticked: true,
                }
            })
            .collect();
        rows.extend(library.into_iter().filter(|repo| !is_ticked(&repo.repo)).map(|repo| PickRow {
            repo: repo.repo.clone(),
            group: Some(RepoGroup::Library),
            library: Some(repo.clone()),
            description: String::new(),
            private: false,
            ticked: false,
        }));
        rows.extend(github.into_iter().filter(|repo| !is_ticked(&repo.repo)).map(|repo| PickRow {
            repo: repo.repo.clone(),
            group: Some(RepoGroup::GitHub),
            library: None,
            description: repo.description.clone(),
            private: repo.is_private,
            ticked: false,
        }));
        let more = rows.len().saturating_sub(limit.max(ticked.len()));
        rows.truncate(limit.max(ticked.len()));
        (rows, more)
    }

    /// The names `rows` lists, borrowed, in the same order.
    #[must_use]
    pub fn names<'a>(&'a self, ticked: &'a [String], limit: usize) -> Vec<&'a str> {
        let (rows, _) = self.rows(ticked, limit);
        rows.iter()
            .filter_map(|row| {
                ticked
                    .iter()
                    .map(String::as_str)
                    .chain(self.library.iter().map(|held| held.repo.as_str()))
                    .chain(
                        self.result
                            .iter()
                            .flat_map(|result| result.github.iter().map(|found| found.repo.as_str())),
                    )
                    .find(|name| *name == row.repo)
            })
            .collect()
    }

    /// Sends the query to the daemon now; an older search still out is
    /// dropped.
    fn search(&mut self, clients: &Clients, key: &str) -> Effect<RepoSearchAction> {
        self.seq += 1;
        self.loading = true;
        self.asked = self.query.clone();
        let seq = self.seq;
        let daemon = clients.daemon.clone();
        let query = self.query.clone();
        let scope = self.scope.clone();
        Effect::future(async move {
            match daemon.search_repos(&query, scope).await {
                Ok(result) => RepoSearchAction::Results { seq, result },
                Err(error) => RepoSearchAction::Failed { seq, error },
            }
        })
        .cancellable(format!("repo-search-{key}"), true)
    }

    /// Changes the scope of this search and searches again at once.
    fn rescope(&mut self, scope: RepoScope, clients: &Clients, key: &str) -> Effect<RepoSearchAction> {
        self.scope = (scope != self.saved).then_some(scope);
        self.adding_owner = None;
        self.save_error = None;
        self.search(clients, key)
    }
}

/// Applies `action`. `key` tells two searches on screen apart for their
/// pause and their in-flight call.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut RepoSearchState,
    action: RepoSearchAction,
    key: &str,
) -> Effect<RepoSearchAction> {
    use RepoSearchAction as A;
    match action {
        A::Open => {
            *state = RepoSearchState::default();
            state.now_ms = clients.daemon.now_ms();
            state.search(clients, key)
        }
        A::Retry => {
            state.failed = None;
            state.search(clients, key)
        }
        A::QueryChanged(query) => {
            if state.query == query {
                return Effect::none();
            }
            state.query = query;
            state.seq += 1;
            state.loading = true;
            if state.query.trim().is_empty() {
                // Nothing typed: the library answers at once, no pause.
                return Effect::cancel(format!("repo-pause-{key}")).and(state.search(clients, key));
            }
            let seq = state.seq;
            let sleep = clients.clock.sleep(PAUSE);
            Effect::future(async move {
                sleep.await;
                A::Paused(seq)
            })
            .cancellable(format!("repo-pause-{key}"), true)
        }
        A::Paused(seq) => {
            if seq == state.seq {
                state.search(clients, key)
            } else {
                Effect::none()
            }
        }
        A::ScopeChipRemoved(chip) => {
            let narrowed = match (state.scope().clone(), chip) {
                (RepoScope::AllOfGitHub, _) => return reduce(clients, state, A::ScopeReset, key),
                (RepoScope::YourRepos, _) => RepoScope::Owners {
                    owners: Vec::new(),
                    include_me: false,
                },
                (RepoScope::Owners { mut owners, include_me }, RepoChip::Owner(owner)) => {
                    owners.retain(|listed| !listed.eq_ignore_ascii_case(&owner));
                    RepoScope::Owners { owners, include_me }
                }
                (RepoScope::Owners { owners, .. }, RepoChip::Me) => RepoScope::Owners {
                    owners,
                    include_me: false,
                },
                (scope, _) => scope,
            };
            state.rescope(narrowed, clients, key)
        }
        A::AddOwnerOpened => {
            state.adding_owner = Some(String::new());
            if state.owners_asked {
                return Effect::none();
            }
            state.owners_asked = true;
            let daemon = clients.daemon.clone();
            Effect::future(async move { A::OwnersListed(daemon.repo_owners().await) })
        }
        A::OwnersListed(Ok(owners)) => {
            state.github_owners = owners;
            Effect::none()
        }
        // Suggestions are optional; the next + Org asks again.
        A::OwnersListed(Err(_)) => {
            state.owners_asked = false;
            Effect::none()
        }
        A::AddOwnerClosed => {
            state.adding_owner = None;
            Effect::none()
        }
        A::OwnerTextChanged(text) => {
            if state.adding_owner.is_some() {
                state.adding_owner = Some(text);
            }
            Effect::none()
        }
        A::OwnerEntered => {
            let typed = state.adding_owner.clone().unwrap_or_default();
            reduce(clients, state, A::OwnerAdded(typed), key)
        }
        A::OwnerAdded(owner) => {
            let owner = owner.trim().trim_start_matches('@').to_owned();
            if owner.is_empty() {
                state.adding_owner = None;
                return Effect::none();
            }
            let widened = match state.scope().clone() {
                RepoScope::YourRepos | RepoScope::AllOfGitHub => RepoScope::Owners {
                    owners: vec![owner],
                    include_me: true,
                },
                RepoScope::Owners { mut owners, include_me } => {
                    if !owners.iter().any(|listed| listed.eq_ignore_ascii_case(&owner)) {
                        owners.push(owner);
                    }
                    RepoScope::Owners { owners, include_me }
                }
            };
            state.rescope(widened, clients, key)
        }
        A::AllOfGitHubPicked => state.rescope(RepoScope::AllOfGitHub, clients, key),
        A::ScopeReset => {
            state.scope = None;
            state.adding_owner = None;
            state.save_error = None;
            state.search(clients, key)
        }
        A::SaveScopeAsDefault => {
            if !state.can_save_default() || state.saving {
                return Effect::none();
            }
            let Some(scope) = state.scope.clone() else {
                return Effect::none();
            };
            state.saving = true;
            state.save_error = None;
            let daemon = clients.daemon.clone();
            Effect::future(async move { A::ScopeSaved(daemon.save_repo_scope(scope).await) })
        }
        A::ScopeSaved(Ok(saved)) => {
            state.saving = false;
            state.saved = saved;
            state.scope = None;
            state.search(clients, key)
        }
        A::ScopeSaved(Err(error)) => {
            state.saving = false;
            state.save_error = Some(error.to_string());
            Effect::none()
        }
        A::Results { seq, result } => {
            if seq != state.seq {
                return Effect::none();
            }
            state.loading = false;
            state.failed = None;
            state.now_ms = clients.daemon.now_ms();
            if state.asked.trim().is_empty() {
                state.library = result.library.clone();
            }
            if state.scope.is_none() {
                state.saved = result.scope.clone();
            }
            state.result = Some(result);
            Effect::none()
        }
        A::Failed { seq, error } => {
            if seq == state.seq {
                state.loading = false;
                state.failed = Some(error.to_string());
            }
            Effect::none()
        }
    }
}

/// "used 2 days ago · #deps" for a library row: when it was last used and
/// where (RL-6). `now_ms` is milliseconds since the epoch.
#[must_use]
pub fn used_line(repo: &LibraryRepo, now_ms: i64, workspace: &Workspace) -> String {
    let place = match &repo.last_used_in {
        RepoPlace::Desk => "Desk".to_owned(),
        RepoPlace::Channel(id) => workspace
            .channels
            .iter()
            .find(|channel| channel.id.to_string() == *id)
            .map_or_else(|| id.clone(), |channel| format!("#{}", channel.name)),
    };
    format!("used {} · {place}", ago(now_ms.saturating_sub(repo.last_used_at_ms)))
}

/// How long ago, for an age in milliseconds: "just now", "5 minutes ago",
/// "3 hours ago", "yesterday", "2 days ago", "last week", "3 weeks ago".
#[must_use]
pub fn ago(age_ms: i64) -> String {
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let plural = |count: i64, unit: &str| {
        if count == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{count} {unit}s ago")
        }
    };
    match age_ms.max(0) {
        age if age < MINUTE => "just now".to_owned(),
        age if age < HOUR => plural(age / MINUTE, "minute"),
        age if age < DAY => plural(age / HOUR, "hour"),
        age if age < 2 * DAY => "yesterday".to_owned(),
        age if age < 7 * DAY => plural(age / DAY, "day"),
        age if age < 14 * DAY => "last week".to_owned(),
        age if age < 60 * DAY => plural(age / (7 * DAY), "week"),
        age => plural(age / (30 * DAY), "month"),
    }
}

impl App {
    /// Applies `action` to `picker`'s search. A picker that is not open has
    /// no search: the action is dropped.
    pub(super) fn repo_search(
        &self,
        state: &mut AppState,
        picker: RepoPicker,
        action: RepoSearchAction,
    ) -> Effect<AppAction> {
        let (search, key) = match picker {
            RepoPicker::SessionAddRepo => return self.session_repo_search(state, action),
            RepoPicker::NewChannel => {
                let sheet = state.overlays.iter_mut().rev().find_map(|overlay| match overlay {
                    Overlay::NewChannel(NewChannel { search, .. }) => Some(search),
                    _ => None,
                });
                match sheet {
                    Some(search) => (search, "new-channel"),
                    None => return Effect::none(),
                }
            }
            RepoPicker::NewDeskSession => {
                if !state.desk.new.as_ref().is_some_and(|form| form.adding_repo) {
                    return Effect::none();
                }
                (&mut state.new_desk_repos, "new-desk-session")
            }
            RepoPicker::DeskAddRepo => {
                if !matches!(&state.desk.menu, Some(humata2_desk::Menu::AddRepo(_))) {
                    return Effect::none();
                }
                (&mut state.desk_repos, "desk-add-repo")
            }
            RepoPicker::ChannelAddRepo => {
                let sheet = state.overlays.iter_mut().rev().find_map(|overlay| match overlay {
                    Overlay::ChannelAddRepo(crate::ChannelAddRepo { search, .. }) => Some(search),
                    _ => None,
                });
                match sheet {
                    Some(search) => (search, "channel-add-repo"),
                    None => return Effect::none(),
                }
            }
        };
        reduce(&self.clients, search, action, key)
            .map(move |action| AppAction::RepoSearch(picker, action))
    }

    /// The New channel sheet's search, as a field's Query sends it.
    pub(super) fn query_new_channel_repos(
        &self,
        sheet: &mut NewChannel,
        query: String,
    ) -> Effect<AppAction> {
        sheet.query.clone_from(&query);
        reduce(
            &self.clients,
            &mut sheet.search,
            RepoSearchAction::QueryChanged(query),
            "new-channel",
        )
        .map(|action| AppAction::RepoSearch(RepoPicker::NewChannel, action))
    }

    /// Starts the New desk session form's repo search with the library showing.
    pub(super) fn open_new_desk_repo_search(&self, state: &mut AppState) -> Effect<AppAction> {
        reduce(
            &self.clients,
            &mut state.new_desk_repos,
            RepoSearchAction::Open,
            "new-desk-session",
        )
        .map(|action| AppAction::RepoSearch(RepoPicker::NewDeskSession, action))
    }

    /// Starts the Desk's Add repo search with the library showing.
    pub(super) fn open_desk_repo_search(&self, state: &mut AppState) -> Effect<AppAction> {
        reduce(&self.clients, &mut state.desk_repos, RepoSearchAction::Open, "desk-add-repo")
            .map(|action| AppAction::RepoSearch(RepoPicker::DeskAddRepo, action))
    }
}
