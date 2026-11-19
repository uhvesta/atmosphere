//! The Diff panel's "Filter files" field (RV-6, DS-9): shown only above
//! [`ABOVE`] changed files, matching on a file's name and folder.

/// The field is drawn above this many changed files, all repositories
/// together.
pub const ABOVE: usize = 20;

/// Whether a list of `count` changed files gets the field.
#[must_use]
pub fn shown_for(count: usize) -> bool {
    count > ABOVE
}

/// Whether `path` (folder and name) matches what was typed: every
/// whitespace-separated word of `query` is in it, ignoring case. An empty
/// query matches everything.
#[must_use]
pub fn matches(path: &str, query: &str) -> bool {
    let path = path.to_lowercase();
    query
        .split_whitespace()
        .all(|word| path.contains(&word.to_lowercase()))
}
