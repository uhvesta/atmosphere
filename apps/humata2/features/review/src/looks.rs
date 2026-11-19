//! Looks good (D852): which files of a session's diff the person looked at,
//! each with a fingerprint of the diff it showed. A file stays looked only
//! while its fingerprint matches the diff now read.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use humata2_clients::model::{FileDiff, RepoDiff, SessionId};

use crate::ReviewState;

/// What a layer row says about the person's looks (D852).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerLook {
    /// Looks good was not pressed on the layer, or its looks were cleared.
    None,
    /// Every file the layer showed when Looks good was pressed is still
    /// looked.
    LooksGood,
    /// A file the layer showed has a different diff than when it was looked
    /// at.
    ChangedSinceLooked,
}

/// A stable fingerprint of one file's diff: its lines (kind and text), the
/// kind of change and its paths. Two reads of the same diff give the same
/// number; any edit to a line, or a rename, gives another.
#[must_use]
pub fn fingerprint(file: &FileDiff) -> u64 {
    let mut hasher = DefaultHasher::new();
    file.path.hash(&mut hasher);
    file.old_path.hash(&mut hasher);
    std::mem::discriminant(&file.change).hash(&mut hasher);
    file.binary.hash(&mut hasher);
    file.lines.len().hash(&mut hasher);
    for line in &file.lines {
        std::mem::discriminant(&line.kind).hash(&mut hasher);
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}

/// Whether `file` of `repo` is looked at: it was marked, and its diff is the
/// one that was shown then.
#[must_use]
pub fn is_looked(
    state: &ReviewState,
    session: &SessionId,
    repo: &str,
    path: &str,
    file: &FileDiff,
) -> bool {
    state
        .looked
        .get(session)
        .and_then(|files| files.get(&(repo.to_owned(), path.to_owned())))
        .is_some_and(|seen| *seen == fingerprint(file))
}

/// What a file row's circle says (D852).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileLook {
    /// The file was never looked at, or its look was cleared.
    Never,
    /// Looked at, and its diff is the one that was shown then.
    Looked,
    /// Looked at, but its diff has changed since.
    Changed,
}

/// Whether `file` of `repo` was never looked at, is looked at, or changed
/// after it was.
#[must_use]
pub fn file_look(
    state: &ReviewState,
    session: &SessionId,
    repo: &str,
    path: &str,
    file: &FileDiff,
) -> FileLook {
    match state
        .looked
        .get(session)
        .and_then(|files| files.get(&(repo.to_owned(), path.to_owned())))
    {
        None => FileLook::Never,
        Some(seen) if *seen == fingerprint(file) => FileLook::Looked,
        Some(_) => FileLook::Changed,
    }
}

/// Whether the person looked at anything of `session` that is still marked.
#[must_use]
pub fn any_look(state: &ReviewState, session: &SessionId) -> bool {
    state.looked.get(session).is_some_and(|files| !files.is_empty())
}

/// How many files of `diffs` are looked at.
#[must_use]
pub fn looked_count(state: &ReviewState, session: &SessionId, diffs: &[RepoDiff]) -> usize {
    diffs
        .iter()
        .flat_map(|diff| diff.files.iter().map(move |file| (&diff.repo, file)))
        .filter(|(repo, file)| is_looked(state, session, repo, &file.path, file))
        .count()
}

/// How many files `diffs` holds.
#[must_use]
pub fn file_count(diffs: &[RepoDiff]) -> usize {
    diffs.iter().map(|diff| diff.files.len()).sum()
}

/// What layer `title` of `session` reads: from the files it covered when
/// Looks good was pressed on it, against the diff last read. A layer whose
/// files are not in the last read keeps reading Looks good until they are.
#[must_use]
pub fn layer_look(state: &ReviewState, session: &SessionId, title: &str) -> LayerLook {
    let Some(covered) = state.layer_looks.get(&(session.clone(), title.to_owned())) else {
        return LayerLook::None;
    };
    let looked = state.looked.get(session);
    let diffs = state
        .fetch
        .get(session)
        .and_then(|fetch| fetch.diffs.as_deref())
        .unwrap_or_default();
    let mut changed = false;
    for (repo, path, seen) in covered {
        let kept = looked
            .and_then(|files| files.get(&(repo.clone(), path.clone())))
            .is_some();
        if !kept {
            return LayerLook::None;
        }
        let now = diffs
            .iter()
            .filter(|diff| &diff.repo == repo)
            .flat_map(|diff| diff.files.iter())
            .find(|file| &file.path == path);
        if now.is_some_and(|file| fingerprint(file) != *seen) {
            changed = true;
        }
    }
    if changed {
        LayerLook::ChangedSinceLooked
    } else {
        LayerLook::LooksGood
    }
}
