//! Revision pins: every upload pushes each changed pull request's head to
//! `refs/stack/<prefix>/<topic>/r<N>` and its base to `.../r<N>-base`.
//! They stay out of refs/heads, so they never show as branches, and they keep
//! old revisions fetchable after a force-push, for `stack diff`.

use std::collections::BTreeMap;

use crate::git::{Git, GitError};

/// `refs/stack/<prefix>/<topic>/` (with the trailing slash).
pub(crate) fn folder(prefix: &str, topic: &str) -> String {
    format!("refs/stack/{prefix}/{topic}/")
}

/// The head pin of revision `rev`.
pub(crate) fn head_ref(prefix: &str, topic: &str, rev: u32) -> String {
    format!("{}r{rev}", folder(prefix, topic))
}

/// The base pin of revision `rev`.
pub(crate) fn base_ref(prefix: &str, topic: &str, rev: u32) -> String {
    format!("{}r{rev}-base", folder(prefix, topic))
}

/// The revision a pin ref names, when it is a head pin under `folder`.
fn rev_of(folder: &str, reference: &str) -> Option<u32> {
    reference.strip_prefix(folder)?.strip_prefix('r')?.parse().ok()
}

/// The pinned revisions of a topic on the remote: revision to head sha.
pub(crate) fn on_remote(git: &Git, remote: &str, prefix: &str, topic: &str) -> Result<BTreeMap<u32, String>, GitError> {
    let folder = folder(prefix, topic);
    let refs = git.remote_refs(remote, &format!("{folder}*"))?;
    Ok(refs
        .into_iter()
        .filter_map(|(reference, sha)| rev_of(&folder, &reference).map(|rev| (rev, sha)))
        .collect())
}

/// The prefixes with pins for `topic` on the remote.
pub(crate) fn prefixes_with(git: &Git, remote: &str, topic: &str) -> Result<Vec<String>, GitError> {
    let refs = git.remote_refs(remote, "refs/stack/*")?;
    let mut prefixes: Vec<String> = refs
        .keys()
        .filter_map(|reference| {
            let rest = reference.strip_prefix("refs/stack/")?;
            let (prefix, rest) = rest.split_once('/')?;
            let (name, _) = rest.split_once('/')?;
            (name == topic).then(|| prefix.to_owned())
        })
        .collect();
    prefixes.dedup();
    Ok(prefixes)
}

/// The ref updates that pin revision `rev` and, when `keep` is more than 0,
/// delete the pins of revisions `keep` or more older than it.
pub(crate) fn updates(
    prefix: &str,
    topic: &str,
    rev: u32,
    head: &str,
    base: &str,
    keep: u32,
    pinned: &BTreeMap<u32, String>,
) -> Vec<(String, Option<String>)> {
    let mut refs = vec![
        (head_ref(prefix, topic, rev), Some(head.to_owned())),
        (base_ref(prefix, topic, rev), Some(base.to_owned())),
    ];
    if keep > 0 {
        for old in pinned.keys().filter(|old| rev.saturating_sub(**old) >= keep) {
            refs.push((head_ref(prefix, topic, *old), None));
            refs.push((base_ref(prefix, topic, *old), None));
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{head_ref, rev_of, updates};

    #[test]
    fn a_pin_names_its_revision_and_nothing_else_does() {
        let folder = "refs/stack/alice/codec/";
        assert_eq!(rev_of(folder, &head_ref("alice", "codec", 3)), Some(3));
        assert_eq!(rev_of(folder, "refs/stack/alice/codec/r3-base"), None);
        assert_eq!(rev_of(folder, "refs/stack/alice/other/r3"), None);
    }

    #[test]
    fn keep_deletes_only_the_revisions_past_the_limit() {
        let pinned: BTreeMap<u32, String> = (1..=4).map(|rev| (rev, format!("sha{rev}"))).collect();
        let all = updates("p", "t", 5, "h", "b", 0, &pinned);
        assert!(all.iter().all(|(_, sha)| sha.is_some()));
        let kept = updates("p", "t", 5, "h", "b", 2, &pinned);
        let deleted: Vec<&str> = kept
            .iter()
            .filter(|(_, sha)| sha.is_none())
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            deleted,
            ["refs/stack/p/t/r1", "refs/stack/p/t/r1-base", "refs/stack/p/t/r2", "refs/stack/p/t/r2-base", "refs/stack/p/t/r3", "refs/stack/p/t/r3-base"]
        );
    }
}
