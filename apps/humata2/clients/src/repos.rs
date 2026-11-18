//! The repo library and the one repo search, as the app sees them (RL-1 to
//! RL-12, D470 to D479): plain types, no wire formats.

/// Where a repo is searched for beyond the library (RL-7, RL-8).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RepoScope {
    /// The repos `gh` lists as owner, collaborator or member.
    #[default]
    YourRepos,
    /// The repos of these GitHub owners, and of the caller when `include_me`.
    Owners {
        /// Orgs or users.
        owners: Vec<String>,
        /// Whether the caller's own repos are searched too.
        include_me: bool,
    },
    /// Every public repo on GitHub; one search only, never saved.
    AllOfGitHub,
}

/// The owners **+ Org** suggests (RL-8): the library's owners first, most
/// recently used first, then the orgs the user's GitHub login belongs to;
/// none twice (case aside) and none the scope already lists.
#[must_use]
pub fn suggest_owners(library: &[LibraryRepo], github: &[String], listed: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let from_library = library
        .iter()
        .map(|repo| repo.repo.split_once('/').map_or(repo.repo.as_str(), |(owner, _)| owner));
    for owner in from_library.chain(github.iter().map(String::as_str)) {
        let known = |list: &[String]| list.iter().any(|name| name.eq_ignore_ascii_case(owner));
        if !known(listed) && !known(&found) {
            found.push(owner.to_owned());
        }
    }
    found
}

/// Where a library repo was last used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoPlace {
    /// In this channel (its id).
    Channel(String),
    /// On the Desk.
    Desk,
}

/// A repo in the library (RL-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRepo {
    /// `owner/name`, as stored.
    pub repo: String,
    /// The remote it is cloned from.
    pub remote: String,
    /// The branch it is cloned on.
    pub default_branch: String,
    /// When it entered the library, in milliseconds since the epoch.
    pub added_at_ms: i64,
    /// When it was last used, in milliseconds since the epoch.
    pub last_used_at_ms: i64,
    /// Where it was last used.
    pub last_used_in: RepoPlace,
}

/// A repo GitHub found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundRepo {
    /// `owner/name`.
    pub repo: String,
    /// GitHub's description; empty when it has none.
    pub description: String,
    /// Whether the repo is private.
    pub is_private: bool,
}

/// Why the GitHub group is missing (RL-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoSearchProblemKind {
    /// `gh` is not installed on the host.
    GhMissing,
    /// `gh` is not signed in on the host.
    GhSignedOut,
    /// GitHub says to wait; `retry_at_ms` is when the limit lifts, if it says.
    RateLimited {
        /// Milliseconds since the epoch.
        retry_at_ms: Option<i64>,
    },
    /// GitHub did not answer.
    GitHubUnreachable,
    /// The query is shorter than the scope needs (RL-9); not an error.
    TooShort,
}

/// A problem with the GitHub group; the library group is still answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSearchProblem {
    /// What went wrong.
    pub kind: RepoSearchProblemKind,
    /// What to do about it, in the host's words.
    pub message: String,
}

/// The answer to one repo search (RL-5, RL-6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoSearch {
    /// The library group, most recently used first.
    pub library: Vec<LibraryRepo>,
    /// The GitHub group, without the repos already in the library group.
    pub github: Vec<FoundRepo>,
    /// The scope that was searched.
    pub scope: RepoScope,
    /// Why the GitHub group is missing, when it is.
    pub problem: Option<RepoSearchProblem>,
    /// Owners in the scope that GitHub does not know.
    pub unknown_owners: Vec<String>,
}
