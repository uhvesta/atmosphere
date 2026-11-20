//! The GitHub repositories the New channel sheet offers (CH-3): the repos
//! `gh` sees on the host, narrowed by what is typed in the sheet's field.

/// The repositories (or owner names) that match `query`, best first:
/// exact match > repo-name prefix > `owner/repo` prefix > word-boundary match >
/// contiguous substring > fuzzy subsequence. Ties preserve the original order
/// of `repos`. An empty query keeps every entry in order.
#[must_use]
pub fn matching(query: &str, repos: &[String]) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return repos.to_vec();
    }
    let mut scored: Vec<(u8, &String)> = repos
        .iter()
        .filter_map(|repo| match_rank(&query, repo).map(|rank| (rank, repo)))
        .collect();
    scored.sort_by_key(|(rank, _)| *rank);
    scored.into_iter().map(|(_, repo)| repo.clone()).collect()
}

/// Rank of `repo` against a non-empty lowercased `query`; lower is better.
fn match_rank(query: &str, repo: &str) -> Option<u8> {
    let text = repo.to_lowercase();
    let repo_name = text.split_once('/').map(|(_, name)| name);
    if text == query {
        Some(0)
    } else if repo_name == Some(query) {
        Some(1)
    } else if repo_name.is_some_and(|name| name.starts_with(query)) {
        Some(2)
    } else if text.starts_with(query) {
        Some(3)
    } else if has_word_boundary_match(&text, query) {
        Some(4)
    } else if text.contains(query) {
        Some(5)
    } else if is_subsequence(query, &text) {
        Some(6)
    } else {
        None
    }
}

/// Whether `query` appears in `text` starting at a word boundary (`/` `-` `_` `.`).
fn has_word_boundary_match(text: &str, query: &str) -> bool {
    let mut offset = 0;
    while let Some(pos) = text[offset..].find(query) {
        let abs = offset + pos;
        let at_boundary = abs == 0
            || text[..abs]
                .chars()
                .next_back()
                .is_some_and(|prev| matches!(prev, '/' | '-' | '_' | '.'));
        if at_boundary {
            return true;
        }
        let step = text[abs..].chars().next().map_or(1, char::len_utf8);
        offset = abs + step;
    }
    false
}

/// Why typed or pasted text is not a GitHub repository.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RepoTextError {
    /// Nothing was typed.
    #[error("Type or paste a repository first, as owner/name or its github.com address")]
    Empty,
    /// The text is not `owner/name` or a GitHub address.
    #[error(
        "{text:?} is not a GitHub repository. Write it as owner/name, such as acme/api, or paste its https://github.com/owner/name address"
    )]
    NotARepo {
        /// What was typed.
        text: String,
    },
}

/// The ways a GitHub address begins, lowercase; the host is not case
/// sensitive.
const ADDRESS_STARTS: [&str; 7] = [
    "https://www.github.com/",
    "https://github.com/",
    "http://github.com/",
    "ssh://git@github.com/",
    "git://github.com/",
    "git@github.com:",
    "github.com/",
];

/// What follows the start of a GitHub address, if `text` is one.
fn after_address_start(text: &str) -> Option<&str> {
    ADDRESS_STARTS.iter().find_map(|start| {
        let head = text.get(..start.len())?;
        head.eq_ignore_ascii_case(start).then(|| &text[start.len()..])
    })
}

/// Whether `part` can be a GitHub owner or repository name: letters, digits,
/// dashes, underscores and dots, and not a path's `.` or `..`.
fn is_github_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The repository `text` names, as `owner/name`: written that way, or pasted
/// as a GitHub address (`https://github.com/owner/name`, with `.git`, a
/// trailing slash, a branch or a query after it, or `git@github.com:owner/name`).
/// A path on this machine, a name with a character GitHub does not allow, or
/// an address on another host is refused.
///
/// # Errors
///
/// [`RepoTextError::Empty`] when nothing was typed and
/// [`RepoTextError::NotARepo`] for text that names no GitHub repository.
pub fn parse_repo_text(text: &str) -> Result<String, RepoTextError> {
    let typed = text.trim();
    if typed.is_empty() {
        return Err(RepoTextError::Empty);
    }
    let refused = || RepoTextError::NotARepo {
        text: typed.to_owned(),
    };
    let (owner, name) = match after_address_start(typed) {
        Some(rest) => {
            let path = rest.split(['?', '#']).next().unwrap_or_default();
            let mut parts = path.split('/').filter(|part| !part.is_empty());
            (parts.next(), parts.next())
        }
        None => {
            let mut parts = typed.trim_end_matches('/').split('/');
            let pair = (parts.next(), parts.next());
            if parts.next().is_some() {
                return Err(refused());
            }
            pair
        }
    };
    let (Some(owner), Some(name)) = (owner, name) else {
        return Err(refused());
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    if is_github_part(owner) && is_github_part(name) {
        Ok(format!("{owner}/{name}"))
    } else {
        Err(refused())
    }
}

/// Whether the characters of `query` appear in `text` in order.
fn is_subsequence(query: &str, text: &str) -> bool {
    let mut rest = text.chars();
    query
        .chars()
        .all(|wanted| rest.any(|found| found == wanted))
}
