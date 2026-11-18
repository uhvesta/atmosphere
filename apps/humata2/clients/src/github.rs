//! The GitHub repositories the New channel sheet offers (CH-3): the repos
//! `gh` sees on the host, narrowed by what is typed in the sheet's field.

/// The repositories that match `query`, best first. Those that contain the
/// lowercased query as a contiguous substring come before those that match
/// it only as a subsequence (its letters in order, with gaps). Each group
/// keeps the order of `repos`. An empty query keeps every repository.
#[must_use]
pub fn matching(query: &str, repos: &[String]) -> Vec<String> {
    let query = query.to_lowercase();
    let mut contiguous = Vec::new();
    let mut scattered = Vec::new();
    for repo in repos {
        let text = repo.to_lowercase();
        if text.contains(&query) {
            contiguous.push(repo.clone());
        } else if is_subsequence(&query, &text) {
            scattered.push(repo.clone());
        }
    }
    contiguous.extend(scattered);
    contiguous
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
