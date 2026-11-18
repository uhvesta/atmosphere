//! The repo library and the one repo search on the fake daemon (RL-1 to
//! RL-12, D470 to D479).

use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Operation, Reply};
use crate::github::matching;
use crate::repos::{
    FoundRepo, LibraryRepo, RepoPlace, RepoScope, RepoSearch, RepoSearchProblem,
    RepoSearchProblemKind,
};

/// Fewest characters GitHub is asked about within owners (RL-9, D477).
const OWNERS_MIN: usize = 2;
/// Fewest characters GitHub is asked about across all of GitHub (RL-9, D477).
const ALL_MIN: usize = 3;
/// The branch a repo is cloned on when it first enters the library (RL-1).
const DEFAULT_BRANCH: &str = "main";

/// What the fake keeps for the repo library and search.
pub(super) struct State {
    /// Most recently used first.
    library: Vec<LibraryRepo>,
    scope: RepoScope,
    problem: Option<RepoSearchProblem>,
    unknown_owners: Vec<String>,
    login: String,
    /// The orgs the caller belongs to on GitHub.
    owners: Vec<String>,
    /// What `now_ms` says; `None` is the same tick the library uses.
    now_ms: Option<i64>,
    /// Stands in for the clock: each recorded use takes the next value (RL-1).
    tick: i64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            library: Vec::new(),
            scope: RepoScope::YourRepos,
            problem: None,
            unknown_owners: Vec::new(),
            login: "you".to_owned(),
            owners: Vec::new(),
            now_ms: None,
            tick: 0,
        }
    }
}

impl FakeDaemon {
    /// Replaces the repo library; it is kept most recently used first.
    pub fn set_repo_library(&self, mut repos: Vec<LibraryRepo>) {
        repos.sort_by_key(|repo| std::cmp::Reverse(repo.last_used_at_ms));
        self.inner().repos.library = repos;
    }

    /// Makes every GitHub search fail with `problem` (the library group is
    /// still answered), or work again with `None`.
    pub fn set_repo_search_problem(&self, problem: Option<RepoSearchProblem>) {
        self.inner().repos.problem = problem;
    }

    /// Says which GitHub owners GitHub does not know; a search whose scope
    /// lists them reports them back.
    pub fn set_unknown_owners(&self, owners: Vec<String>) {
        self.inner().repos.unknown_owners = owners;
    }

    /// Says who the caller is on GitHub (default "you"), for the scope's
    /// "and me".
    pub fn set_repo_login(&self, login: impl Into<String>) {
        self.inner().repos.login = login.into();
    }

    /// Says which orgs the caller belongs to on GitHub, for + Org's suggestions.
    pub fn set_repo_owners(&self, owners: Vec<String>) {
        self.inner().repos.owners = owners;
    }

    /// Fixes the time the repo picker works "used ... ago" out from.
    pub fn set_repo_now_ms(&self, now_ms: i64) {
        self.inner().repos.now_ms = Some(now_ms);
    }
}

fn owner_of(repo: &str) -> &str {
    repo.split_once('/').map_or(repo, |(owner, _)| owner)
}

fn lists(owners: &[String], owner: &str) -> bool {
    owners.iter().any(|listed| listed.eq_ignore_ascii_case(owner))
}

/// Notes that `repo` was used in `place` (RL-1). A repo not yet in the library
/// joins it; one already there keeps when it was added. Either way it moves to
/// the front, most recently used.
pub(super) fn record(inner: &mut Inner, repo: &str, place: RepoPlace) {
    let state = &mut inner.repos;
    state.tick += 1;
    let now = state.tick;
    let found = state
        .library
        .iter()
        .position(|held| held.repo.eq_ignore_ascii_case(repo));
    let mut entry = match found {
        Some(at) => state.library.remove(at),
        None => LibraryRepo {
            repo: repo.to_owned(),
            remote: format!("git@github.com:{repo}.git"),
            default_branch: DEFAULT_BRANCH.to_owned(),
            added_at_ms: now,
            last_used_at_ms: now,
            last_used_in: place.clone(),
        },
    };
    entry.last_used_at_ms = now;
    entry.last_used_in = place;
    state.library.insert(0, entry);
}

pub(super) fn search_repos(
    daemon: &FakeDaemon,
    query: &str,
    scope: Option<RepoScope>,
) -> Reply<RepoSearch> {
    let query = query.trim().to_owned();
    daemon.reply(Some(Operation::SearchRepos), format!("search repos {query:?}"), move |inner| {
        let scope = scope.unwrap_or_else(|| inner.repos.scope.clone());
        let state = &inner.repos;

        let names: Vec<String> = state.library.iter().map(|repo| repo.repo.clone()).collect();
        let library: Vec<LibraryRepo> = matching(&query, &names)
            .iter()
            .filter_map(|name| state.library.iter().find(|repo| &repo.repo == name).cloned())
            .collect();

        let needed = match scope {
            RepoScope::YourRepos => 0,
            RepoScope::Owners { .. } => OWNERS_MIN,
            RepoScope::AllOfGitHub => ALL_MIN,
        };
        let mut problem = None;
        let mut github = Vec::new();
        if query.chars().count() < needed {
            problem = Some(RepoSearchProblem {
                kind: RepoSearchProblemKind::TooShort,
                message: format!("Type at least {needed} characters to search GitHub."),
            });
        } else if let Some(injected) = &state.problem {
            problem = Some(injected.clone());
        } else {
            let in_scope: Vec<String> = inner
                .github_repos
                .iter()
                .filter(|repo| match &scope {
                    RepoScope::YourRepos | RepoScope::AllOfGitHub => true,
                    RepoScope::Owners { owners, include_me } => {
                        lists(owners, owner_of(repo))
                            || (*include_me && owner_of(repo).eq_ignore_ascii_case(&state.login))
                    }
                })
                .cloned()
                .collect();
            github = matching(&query, &in_scope)
                .into_iter()
                .filter(|repo| !library.iter().any(|held| held.repo.eq_ignore_ascii_case(repo)))
                .map(|repo| FoundRepo { repo, description: String::new(), is_private: false })
                .collect();
        }

        let unknown_owners = match &scope {
            RepoScope::Owners { owners, .. } => owners
                .iter()
                .filter(|owner| lists(&state.unknown_owners, owner))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        Ok(RepoSearch { library, github, scope, problem, unknown_owners })
    })
}

pub(super) fn repo_library(daemon: &FakeDaemon) -> Reply<Vec<LibraryRepo>> {
    daemon.reply(
        Some(Operation::RepoLibrary),
        "repo library".to_owned(),
        |inner| Ok(inner.repos.library.clone()),
    )
}

pub(super) fn remove_from_repo_library(daemon: &FakeDaemon, repo: &str) -> Reply<Vec<LibraryRepo>> {
    let repo = repo.to_owned();
    daemon.reply(
        Some(Operation::RemoveFromRepoLibrary),
        format!("remove from repo library {repo:?}"),
        move |inner| {
            inner.repos.library.retain(|held| !held.repo.eq_ignore_ascii_case(&repo));
            Ok(inner.repos.library.clone())
        },
    )
}

pub(super) fn forget_repo_library(daemon: &FakeDaemon) -> Reply<u32> {
    daemon.reply(Some(Operation::ForgetRepoLibrary), "forget repo library".to_owned(), |inner| {
        let count = u32::try_from(inner.repos.library.len()).unwrap_or(u32::MAX);
        inner.repos.library.clear();
        Ok(count)
    })
}

pub(super) fn repo_scope(daemon: &FakeDaemon) -> Reply<RepoScope> {
    daemon.reply(
        Some(Operation::RepoScope),
        "repo scope".to_owned(),
        |inner| Ok(inner.repos.scope.clone()),
    )
}

pub(super) fn repo_owners(daemon: &FakeDaemon) -> Reply<Vec<String>> {
    daemon.reply(None, "repo owners".to_owned(), |inner| Ok(inner.repos.owners.clone()))
}

pub(super) fn now_ms(daemon: &FakeDaemon) -> i64 {
    let inner = daemon.inner();
    inner.repos.now_ms.unwrap_or(inner.repos.tick)
}

pub(super) fn save_repo_scope(daemon: &FakeDaemon, scope: RepoScope) -> Reply<RepoScope> {
    daemon.reply(Some(Operation::SaveRepoScope), "save repo scope".to_owned(), move |inner| {
        let scope = match scope {
            RepoScope::AllOfGitHub => {
                return Err(DaemonError::Refused {
                    what: "Saving the repo scope".to_owned(),
                    reason: "All of GitHub is for one search only. Pick owners or Your repos to save as the default".to_owned(),
                });
            }
            RepoScope::Owners { owners, include_me } => {
                let mut kept: Vec<String> = Vec::new();
                for owner in owners.iter().map(|owner| owner.trim()).filter(|owner| !owner.is_empty()) {
                    if !lists(&kept, owner) {
                        kept.push(owner.to_owned());
                    }
                }
                RepoScope::Owners { owners: kept, include_me }
            }
            other => other,
        };
        inner.repos.scope = scope.clone();
        Ok(scope)
    })
}
