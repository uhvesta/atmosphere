//! Looks good (D852): which files of a session's diff the person looked at,
//! each with a fingerprint of the diff it showed. A file stays looked only
//! while its fingerprint matches the diff now read.

use humata2_clients::model::{FileChange, FileDiff, LineKind, RepoDiff, SessionId};

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

/// A fingerprint of one file's diff: its lines (kind and text), the kind of
/// change and its paths. Two reads of the same diff give the same number
/// in any process, on any run (the hash is FNV-1a over fixed bytes, since
/// looks are saved by the host); any edit to a line, or a rename, gives
/// another.
#[must_use]
pub fn fingerprint(file: &FileDiff) -> u64 {
    let mut hash = Fnv::new();
    hash.text(&file.path);
    match &file.old_path {
        Some(old) => {
            hash.byte(1);
            hash.text(old);
        }
        None => hash.byte(0),
    }
    hash.byte(match file.change {
        FileChange::Added => 1,
        FileChange::Modified => 2,
        FileChange::Deleted => 3,
        FileChange::Renamed => 4,
    });
    hash.byte(u8::from(file.binary));
    hash.number(file.lines.len() as u64);
    for line in &file.lines {
        hash.byte(match line.kind {
            LineKind::Context => 1,
            LineKind::Added => 2,
            LineKind::Removed => 3,
            LineKind::Hunk => 4,
            LineKind::Note => 5,
        });
        hash.text(&line.text);
    }
    hash.0
}

/// 64-bit FNV-1a.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn number(&mut self, number: u64) {
        for byte in number.to_le_bytes() {
            self.byte(byte);
        }
    }

    /// Length first, so "ab" + "c" differs from "a" + "bc".
    fn text(&mut self, text: &str) {
        self.number(text.len() as u64);
        for byte in text.bytes() {
            self.byte(byte);
        }
    }
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

/// The `(repo, path)` of the files a stack row covers, when they are known:
/// the shown row's from the diff last read, another layer's from what it
/// covered when Looks good was pressed on it. `None` for Whole stack unless
/// it is shown, and for a layer whose files were not read yet. `title` is
/// `None` for Whole stack.
#[must_use]
pub fn layer_files(state: &ReviewState, session: &SessionId, title: Option<&str>) -> Option<Vec<(String, String)>> {
    if state.layer_of(session) == title {
        let diffs = state.fetch.get(session)?.diffs.as_deref()?;
        return Some(
            diffs
                .iter()
                .flat_map(|diff| diff.files.iter().map(|file| (diff.repo.clone(), file.path.clone())))
                .collect(),
        );
    }
    let covered = state.layer_looks.get(&(session.clone(), title?.to_owned()))?;
    Some(covered.iter().map(|(repo, path, _)| (repo.clone(), path.clone())).collect())
}

/// Rebuilds what a layer covered from the files its diff shows: every one
/// of them must have a look. `None` when any has none.
#[must_use]
pub fn covered_by_looks(
    state: &ReviewState,
    session: &SessionId,
    diffs: &[RepoDiff],
) -> Option<Vec<(String, String, u64)>> {
    let looked = state.looked.get(session)?;
    let mut covered = Vec::new();
    for diff in diffs {
        for file in &diff.files {
            let seen = looked.get(&(diff.repo.clone(), file.path.clone()))?;
            covered.push((diff.repo.clone(), file.path.clone(), *seen));
        }
    }
    (!covered.is_empty()).then_some(covered)
}
