//! One repo search (RL-6, RL-9, D274, D476, D477): which GitHub search a
//! query and scope make, and how the library and GitHub groups merge.
//! Pure: it does not run `gh`, so the host does that with the query given.

use crate::library::Entry;
use crate::name::RepoName;
use crate::scope::Scope;

/// A repo GitHub search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The repo.
    pub repo: RepoName,
    /// The repo's description on GitHub, empty when it has none.
    pub description: String,
    /// Whether the repo is private.
    pub is_private: bool,
}

/// The two groups of one repo search, ready to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The library repos that match the query, whole matches first, most
    /// recently used first within each tier.
    pub library: Vec<Entry>,
    /// The GitHub hits, in GitHub's order, without the repos already shown
    /// under `library`.
    pub github: Vec<Hit>,
}

/// Merges one search's two groups. A library repo matches when the query's
/// letters appear in order in its `owner/name` (ignoring case). Repos where
/// the query appears as typed come first, the rest after; within each tier
/// the most recently used comes first. An empty query matches the whole
/// library. The GitHub hits are kept in order, but a hit for a repo already
/// shown under the library is dropped. The scope never filters the library.
pub fn merge(query: &str, library: &[Entry], github: &[Hit]) -> Found {
    let query = query.trim().to_ascii_lowercase();
    let mut whole = Vec::new();
    let mut scattered = Vec::new();
    for entry in library {
        let full = entry.repo.to_string().to_ascii_lowercase();
        if full.contains(query.as_str()) {
            whole.push(entry.clone());
        } else if letters_in_order(&query, &full) {
            scattered.push(entry.clone());
        }
    }
    whole.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
    scattered.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
    let shown: Vec<Entry> = whole.into_iter().chain(scattered).collect();
    let github_hits: Vec<Hit> = github
        .iter()
        .filter(|hit| !shown.iter().any(|entry| entry.repo == hit.repo))
        .cloned()
        .collect();
    Found {
        library: shown,
        github: github_hits,
    }
}

/// Whether the characters of `query` appear in `full`, in order, with any
/// characters between them.
fn letters_in_order(query: &str, full: &str) -> bool {
    let mut wanted = query.chars();
    let mut next = wanted.next();
    for c in full.chars() {
        if next == Some(c) {
            next = wanted.next();
        }
    }
    next.is_none()
}

/// What one repo search sends to GitHub, or why it sends nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubQuery {
    /// Your repos: the cached listing of your repos, matched locally. No
    /// search is sent.
    Listing,
    /// The GitHub repository search to run, with its qualifiers.
    Search(String),
    /// The query is too short to search. Type at least `needs` characters.
    TooShort {
        /// How many characters the search needs.
        needs: usize,
    },
    /// The scope names no owner and doesn't include you, so there is
    /// nothing to search.
    Nothing,
}

/// Builds the GitHub search for a query within a scope. Words with a ':' in
/// them are dropped, so a typed qualifier can't widen the scope. Within
/// owners, the search is qualified by each owner, then by your own `login`
/// when `include_me` is on and the login is known, and needs 2 characters.
/// All of GitHub is unqualified and needs 3.
pub fn github_query(query: &str, scope: &Scope, login: Option<&str>) -> GitHubQuery {
    let cleaned = query
        .split_whitespace()
        .filter(|word| !word.contains(':'))
        .collect::<Vec<_>>()
        .join(" ");
    let length = cleaned.chars().count();
    match scope {
        Scope::YourRepos => GitHubQuery::Listing,
        Scope::AllOfGitHub => {
            if length < 3 {
                GitHubQuery::TooShort { needs: 3 }
            } else {
                GitHubQuery::Search(cleaned)
            }
        }
        Scope::Owners {
            owners,
            include_me,
        } => {
            let mut qualifiers: Vec<String> = owners
                .iter()
                .map(|owner| format!("user:{owner}"))
                .collect();
            let me = if *include_me { login } else { None };
            if let Some(login) = me {
                qualifiers.push(format!("user:{login}"));
            }
            if qualifiers.is_empty() {
                return GitHubQuery::Nothing;
            }
            if length < 2 {
                return GitHubQuery::TooShort { needs: 2 };
            }
            let mut terms = vec![cleaned];
            terms.extend(qualifiers);
            GitHubQuery::Search(terms.join(" "))
        }
    }
}
