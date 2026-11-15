//! Humata 2's review diffs (SPEC RV-28–RV-34, PR-22–PR-26, D66 in
//! DECISIONS.md).
//!
//! - A [`DiffSpec`] says per repo what a review shows (RV-31); a
//!   [`Checkout`] computes it with the git CLI into a [`RepoDiff`]: files
//!   with counts and content hashes, and per file [`FileChunk`]s of hunks
//!   asked for lazily, at most a [`Budget`] each (TR-11).
//! - The new side is a tree written from a private copy of the index, so
//!   git's stat cache is used and the user's index is never written.
//! - Reviewed snapshots (RV-28, RV-30) are commits under
//!   `refs/humata/reviewed/<session>/<n>`; per-file Viewed (RV-29) is a
//!   blob id, and [`Checkout::since_viewed`] shows the change since.
//! - A [`ReviewPlan`] is what `review_prepare` carries (RV-32); its notes
//!   are anchored to blobs and go outdated when their lines change (RV-34).
//! - [`ReviewChat`] is the review chat's view state (PR-22–PR-26): focus,
//!   notes for the user, order, Undo and Back, gated by [`AgentSwitches`].
//! - [`wire`] converts all of it to and from `humata.v1`.
//!
//! Every git call blocks, so each `*_blocking` method has an async twin
//! that runs it on Tokio's blocking pool.
//!
//! # Mapping to `humata2_diff_view`
//!
//! The Diff panel's model (apps/humata2/diff/src/model.rs) is built from
//! this one as: `RepoDiff { name: repo, branch, base: label, files }`; per
//! file `FileDiff { path, status, hunks, new_lines }` with `FileStatus`
//! from [`FileStatus`] (dropping the rename score); each [`Hunk`] becomes
//! `Hunk { old_start, new_start, section, lines }`, a `continued` hunk's
//! lines appended to the previous one; each [`DiffLine`] keeps its `kind`
//! and `text`.

mod chat;
mod checkout;
mod diff;
mod error;
mod git;
mod model;
mod plan;
mod snapshot;
mod spec;
pub mod wire;

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use chat::{
    AgentSwitches, Change, ChangeKind, ChangeTool, ChatError, ChatTool, Context, Highlight, Lines,
    Place, ReviewChat, Switch, UserNote, ViewState, ViewedFile,
};
pub use checkout::{Checkout, Stamp, WatchSet};
pub use diff::Matching;
pub use error::{DiffError, PlanError};
pub use model::{
    Budget, Cursor, DiffLine, FileChunk, FileStatus, FileSummary, Hunk, LineKind, RepoDiff, Side,
    Snapshot, Trees, Viewed,
};
pub use plan::{
    Anchor, AnchorError, Arrangement, FileRef, Group, Note, NoteState, NoteTarget, ReviewPlan,
    Section,
};
pub use snapshot::KEPT_SNAPSHOTS;
pub use spec::{
    Base, BaseContext, Compare, DiffSpec, PathFilter, RepoSpec, Untracked, Upto, Whitespace,
};

async fn blocking<T, F>(work: F) -> Result<T, DiffError>
where
    F: FnOnce() -> Result<T, DiffError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| DiffError::Task { source })?
}

impl Checkout {
    /// [`Self::diff_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::diff_blocking`].
    pub async fn diff(&self, spec: RepoSpec, context: BaseContext) -> Result<RepoDiff, DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.diff_blocking(&spec, &context)).await
    }

    /// [`Self::file_chunk_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::file_chunk_blocking`].
    pub async fn file_chunk(
        &self,
        trees: Trees,
        file: FileSummary,
        matching: Matching,
        from: Cursor,
        budget: Budget,
    ) -> Result<FileChunk, DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.file_chunk_blocking(&trees, &file, matching, from, budget)).await
    }

    /// [`Self::since_viewed_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::since_viewed_blocking`].
    pub async fn since_viewed(
        &self,
        viewed_blob: String,
        now: FileSummary,
        matching: Matching,
        from: Cursor,
        budget: Budget,
    ) -> Result<Viewed, DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.since_viewed_blocking(&viewed_blob, &now, matching, from, budget))
            .await
    }

    /// [`Self::mark_reviewed_blocking`] on the blocking pool. `shown_tree`
    /// is the new tree of the diff the user was shown; `None` snapshots the
    /// working tree as it is now (staged, unstaged and untracked).
    ///
    /// # Errors
    ///
    /// As [`Self::mark_reviewed_blocking`].
    pub async fn mark_reviewed(
        &self,
        session: String,
        shown_tree: Option<String>,
    ) -> Result<Snapshot, DiffError> {
        let checkout = self.clone();
        blocking(move || {
            let tree = match shown_tree {
                Some(tree) => tree,
                None => {
                    let spec = RepoSpec::new(
                        String::new(),
                        Base::Rev {
                            rev: "HEAD".to_owned(),
                        },
                        Compare::everything(),
                    );
                    checkout.trees(&spec, &BaseContext::default())?.new
                }
            };
            checkout.mark_reviewed_blocking(&session, &tree)
        })
        .await
    }

    /// [`Self::snapshots_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::snapshots_blocking`].
    pub async fn snapshots(&self, session: String) -> Result<Vec<Snapshot>, DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.snapshots_blocking(&session)).await
    }

    /// [`Self::fetch_session_head_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::fetch_session_head_blocking`].
    pub async fn fetch_session_head(&self, from: PathBuf, session: String) -> Result<String, DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.fetch_session_head_blocking(&from, &session)).await
    }

    /// [`Self::pin_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::pin_blocking`].
    pub async fn pin(&self, session: String, n: u32, pinned: bool) -> Result<(), DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.pin_blocking(&session, n, pinned)).await
    }

    /// [`Self::note_states_blocking`] on the blocking pool.
    ///
    /// # Errors
    ///
    /// As [`Self::note_states_blocking`].
    pub async fn note_states(
        &self,
        diff: RepoDiff,
        notes: Vec<Note>,
    ) -> Result<Vec<Option<NoteState>>, DiffError> {
        let checkout = self.clone();
        blocking(move || checkout.note_states_blocking(&diff, &notes)).await
    }

    /// Anchors `notes` against `diff` (RV-34) on the blocking pool, reading
    /// hunks for hunk notes with the default budget.
    ///
    /// # Errors
    ///
    /// As [`Self::anchor_notes_blocking`].
    pub async fn anchor_notes(
        &self,
        diff: RepoDiff,
        matching: Matching,
        mut notes: Vec<Note>,
    ) -> Result<Vec<Note>, AnchorError> {
        let checkout = self.clone();
        tokio::task::spawn_blocking(move || {
            let trees = diff.trees.clone();
            checkout.anchor_notes_blocking(&diff, &mut notes, |file| {
                checkout.file_chunk_blocking(
                    &trees,
                    file,
                    matching,
                    Cursor::default(),
                    Budget::default(),
                )
            })?;
            Ok(notes)
        })
        .await
        .map_err(|source| AnchorError::Diff(DiffError::Task { source }))?
    }
}

/// The session's clones by repo name, for specs over several repos.
#[derive(Clone, Debug, Default)]
pub struct Clones {
    by_repo: BTreeMap<String, Checkout>,
}

impl Clones {
    #[must_use]
    pub fn new(by_repo: BTreeMap<String, Checkout>) -> Self {
        Self { by_repo }
    }

    /// The clone of `repo`.
    ///
    /// # Errors
    ///
    /// [`DiffError::UnknownRepo`] naming the repos there are.
    pub fn get(&self, repo: &str) -> Result<&Checkout, DiffError> {
        self.by_repo.get(repo).ok_or_else(|| DiffError::UnknownRepo {
            repo: repo.to_owned(),
            known: self.by_repo.keys().cloned().collect(),
        })
    }

    /// Every repo of `spec`, computed at once, in the spec's order.
    ///
    /// # Errors
    ///
    /// The first repo's [`DiffError`].
    pub async fn diff(
        &self,
        spec: &DiffSpec,
        context: &BaseContext,
    ) -> Result<Vec<RepoDiff>, DiffError> {
        let mut tasks = tokio::task::JoinSet::new();
        for (index, repo) in spec.repos.iter().enumerate() {
            let checkout = self.get(&repo.repo)?.clone();
            let (repo, context) = (repo.clone(), context.clone());
            tasks.spawn_blocking(move || (index, checkout.diff_blocking(&repo, &context)));
        }
        let mut diffs: Vec<Option<RepoDiff>> = vec![None; spec.repos.len()];
        while let Some(joined) = tasks.join_next().await {
            let (index, diff) = joined.map_err(|source| DiffError::Task { source })?;
            if let Some(slot) = diffs.get_mut(index) {
                *slot = Some(diff?);
            }
        }
        Ok(diffs.into_iter().flatten().collect())
    }
}
