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

/// Whether the characters of `query` appear in `text` in order.
fn is_subsequence(query: &str, text: &str) -> bool {
    let mut rest = text.chars();
    query
        .chars()
        .all(|wanted| rest.any(|found| found == wanted))
}
