//! Repo names as the library keeps them: `owner/name`, as GitHub allows.

use std::fmt;
use std::hash::{Hash, Hasher};

/// A GitHub repo, as `owner/name`. The name keeps the case it was typed or
/// pasted in, so the library shows `Bazelbuild/rules_java` as written, but
/// two names that differ only in case are the same repo, as on GitHub.
#[derive(Clone, Debug)]
pub struct RepoName {
    owner: String,
    name: String,
}

impl RepoName {
    /// Reads a repo from what the user typed or pasted. Accepted forms are
    /// `owner/name`, `https://github.com/owner/name` and
    /// `git@github.com:owner/name.git`, each with or without a `.git`
    /// suffix, and with spaces around it. Anything else is refused.
    pub fn parse(text: &str) -> Result<RepoName, NameError> {
        let trimmed = text.trim();
        let path = trimmed
            .strip_prefix("https://github.com/")
            .or_else(|| trimmed.strip_prefix("git@github.com:"))
            .unwrap_or(trimmed);
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path.split_once('/').ok_or_else(|| refused(text))?;
        if is_owner(owner) && is_name(name) {
            Ok(RepoName {
                owner: owner.to_owned(),
                name: name.to_owned(),
            })
        } else {
            Err(refused(text))
        }
    }

    /// The owner (a GitHub org or user), as it was given.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The repo's name within its owner, as it was given.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The lowercased `owner/name`, which is how the library finds a repo.
    pub(crate) fn key(&self) -> String {
        format!("{}/{}", self.owner, self.name).to_ascii_lowercase()
    }

    /// Rebuilds a repo from the owner and name the library stored.
    pub(crate) fn from_parts(owner: String, name: String) -> RepoName {
        RepoName { owner, name }
    }
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl PartialEq for RepoName {
    fn eq(&self, other: &Self) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner)
            && self.name.eq_ignore_ascii_case(&other.name)
    }
}

impl Eq for RepoName {}

impl Hash for RepoName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// Text that isn't a repo name.
#[derive(Debug, thiserror::Error)]
#[error(
    "{text:?} isn't a repo name. Write it as owner/name, such as acme/api, or paste its GitHub address, such as https://github.com/acme/api. The owner is letters, digits and '-' (up to 39 characters, not starting with '-'), and the name is letters, digits, '-', '_' and '.'"
)]
pub struct NameError {
    text: String,
}

fn refused(text: &str) -> NameError {
    NameError {
        text: text.to_owned(),
    }
}

/// Whether `text` is a GitHub owner (an org or a user) name: 1 to 39
/// letters, digits or '-', not starting with '-'.
pub(crate) fn is_owner(text: &str) -> bool {
    (1..=39).contains(&text.len())
        && !text.starts_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Whether `text` is a repo name within its owner: letters, digits, '-',
/// '_' or '.', and not `.` or `..`.
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text != "."
        && text != ".."
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
