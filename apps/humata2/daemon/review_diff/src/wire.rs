//! Conversions between this crate's types and `humata.v1` (review.proto),
//! for humatad's DiffService, ViewService, CommentService and
//! ReviewChatService.
//!
//! A repo is named here by its key: `owner/name`, or `name` when the
//! owner is empty ([`repo_key`], [`repo_ref`]); key the session's
//! [`crate::Clones`] the same way.

use std::collections::{BTreeMap, BTreeSet};

#[allow(unused_imports, ambiguous_glob_reexports)]
mod pb {
    pub use humata2_proto_core::humata::v1::*;
    pub use humata2_proto_review::humata::v1::*;
}
use humata2_proto_core::Timestamp;

use crate::chat::{
    AgentSwitches, Change, ChangeKind, ChangeTool, ChatError, Context, Highlight, Lines, Place,
    UserNote, ViewState, ViewedFile,
};
use crate::model::{
    Budget, Cursor, DiffLine, FileChunk, FileStatus, FileSummary, Hunk, LineKind, RepoDiff, Side,
    Snapshot, Trees, Viewed,
};
use crate::from_stack::{FromStack, TopicOverride};
use crate::plan::{Anchor, FileRef, Group, Importance, Note, NoteTarget, ReviewPlan, Tier};
use crate::spec::{
    Base, Compare, DiffSpec, PathFilter, RepoSpec, Untracked, Upto, Whitespace, default_repo_spec,
};
use crate::stack::{CheckedStack, Drift, DriftKind, Layer, LayerRepo, Part, Resolved, Stack};

/// A message that can't be turned into a diff, plan or view: what is
/// missing or wrong, for the caller to fix.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("`{field}` is missing; set it and send the request again")]
    Missing { field: &'static str },

    #[error("`{field}` is {value:?}, which isn't allowed: {expected}")]
    Invalid {
        field: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error(
        "the diff spec names {repo:?}, which isn't a repo of this session (its repos are {known:?}); remove it from the spec, or name one of the session's repos"
    )]
    UnknownRepo { repo: String, known: Vec<String> },

    #[error(
        "the diff spec names {repo:?} more than once; give each repo one entry, with the base you want for it"
    )]
    RepeatedRepo { repo: String },
}

fn missing(field: &'static str) -> WireError {
    WireError::Missing { field }
}

/// The key this crate names a repo by.
#[must_use]
pub fn repo_key(repo: &pb::RepoRef) -> String {
    if repo.owner.is_empty() {
        repo.name.clone()
    } else {
        format!("{}/{}", repo.owner, repo.name)
    }
}

/// The `RepoRef` for a key made by [`repo_key`].
#[must_use]
pub fn repo_ref(key: &str) -> pb::RepoRef {
    match key.split_once('/') {
        Some((owner, name)) => pb::RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        },
        None => pb::RepoRef {
            owner: String::new(),
            name: key.to_owned(),
        },
    }
}

fn file_ref(path: Option<&pb::RepoPath>, field: &'static str) -> Result<FileRef, WireError> {
    let path = path.ok_or_else(|| missing(field))?;
    let repo = path.repo.as_ref().ok_or_else(|| missing(field))?;
    if path.path.is_empty() {
        return Err(missing(field));
    }
    Ok(FileRef {
        repo: repo_key(repo),
        path: path.path.clone(),
    })
}

fn repo_path(file: &FileRef) -> pb::RepoPath {
    pb::RepoPath {
        repo: Some(repo_ref(&file.repo)),
        path: file.path.clone(),
    }
}

fn side_from(side: i32) -> Side {
    if side == pb::DiffSide::Old as i32 {
        Side::Old
    } else {
        Side::New
    }
}

fn side_to(side: Side) -> i32 {
    match side {
        Side::Old => pb::DiffSide::Old as i32,
        Side::New => pb::DiffSide::New as i32,
    }
}

fn lines_from(range: Option<&pb::LineRange>) -> Option<Lines> {
    range.map(|range| Lines {
        first: range.first.min(range.last),
        last: range.first.max(range.last),
    })
}

fn lines_to(lines: Lines) -> pb::LineRange {
    pb::LineRange {
        first: lines.first,
        last: lines.last,
    }
}

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp { seconds, nanos: 0 }
}

// ---------------------------------------------------------------------------
// Diff specs (RV-31)

/// A spec from the wire.
///
/// # Errors
///
/// [`WireError`] for a repo without a name or base.
pub fn spec_from_pb(spec: &pb::DiffSpec) -> Result<DiffSpec, WireError> {
    Ok(DiffSpec {
        repos: spec
            .repos
            .iter()
            .map(repo_spec_from_pb)
            .collect::<Result<_, _>>()?,
    })
}

fn repo_spec_from_pb(spec: &pb::RepoDiffSpec) -> Result<RepoSpec, WireError> {
    use pb::diff_spec_base::Base as B;
    use pb::diff_spec_compare::{Untracked as U, Upto as P};
    let repo = spec.repo.as_ref().ok_or_else(|| missing("repos.repo"))?;
    let base = match spec.base.as_ref().and_then(|base| base.base.as_ref()) {
        None => return Err(missing("repos.base")),
        Some(B::MergeBaseWith(with)) if !with.is_empty() => Base::MergeBase { with: with.clone() },
        Some(B::Rev(rev)) if !rev.is_empty() => Base::Rev { rev: rev.clone() },
        Some(B::MergeBaseWith(_) | B::Rev(_)) => return Err(missing("repos.base")),
        Some(B::ReviewedSnapshot(n)) => Base::Snapshot {
            n: (*n != 0).then_some(*n),
        },
        Some(B::ForkPoint(_)) => Base::ForkPoint,
        Some(B::Index(_)) => Base::Index,
        // Only a review's base can resolve it (PR-10): a plan, a push or a
        // saved spec has no pull request to ask.
        Some(B::PullRequestBase(_)) => {
            return Err(WireError::Invalid {
                field: "repos.base.pull_request_base",
                value: repo_key(repo),
                expected: "a base of a pull request review's Diff; use merge_base_with with the pull request's base branch here",
            });
        }
    };
    let compare = spec.compare.clone().unwrap_or_default();
    // A head or worktree paths pin the compare to commits (D800); unset
    // upto and untracked then mean commits only and no untracked files.
    let pinned = !compare.head.is_empty() || !compare.worktree_paths.is_empty();
    let upto = match compare.upto() {
        P::Commits => Upto::Commits,
        P::Staged => Upto::Staged,
        P::Unspecified if pinned => Upto::Commits,
        P::Unspecified | P::Unstaged => Upto::Unstaged,
    };
    let untracked = match compare.untracked() {
        U::None => Untracked::None,
        U::Unspecified if pinned => Untracked::None,
        U::Unspecified | U::All => Untracked::All,
        U::Paths if compare.untracked_paths.is_empty() => {
            return Err(WireError::Invalid {
                field: "repos.compare.untracked_paths",
                value: String::new(),
                expected: "UNTRACKED_PATHS needs at least one path; use UNTRACKED_NONE for none",
            });
        }
        U::Paths => Untracked::Paths(compare.untracked_paths.clone()),
    };
    let whitespace = match spec.whitespace() {
        pb::DiffWhitespace::Unspecified => Whitespace::Counts,
        pb::DiffWhitespace::IgnoreChanges => Whitespace::IgnoreChanges,
        pb::DiffWhitespace::IgnoreAll => Whitespace::IgnoreAll,
    };
    Ok(RepoSpec {
        repo: repo_key(repo),
        base,
        compare: Compare {
            upto,
            untracked,
            head: (!compare.head.is_empty()).then(|| compare.head.clone()),
            worktree_paths: compare.worktree_paths.clone(),
        },
        paths: PathFilter {
            include: spec.include_paths.clone(),
            exclude: spec.exclude_paths.clone(),
        },
        whitespace,
        renames: !spec.no_renames,
    })
}

/// A spec for the wire.
#[must_use]
pub fn spec_to_pb(spec: &DiffSpec) -> pb::DiffSpec {
    pb::DiffSpec {
        repos: spec.repos.iter().map(repo_spec_to_pb).collect(),
    }
}

fn repo_spec_to_pb(spec: &RepoSpec) -> pb::RepoDiffSpec {
    use pb::diff_spec_base::Base as B;
    use pb::diff_spec_compare::{Untracked as U, Upto as P};
    let base = match &spec.base {
        Base::MergeBase { with } => B::MergeBaseWith(with.clone()),
        Base::Rev { rev } => B::Rev(rev.clone()),
        Base::Snapshot { n } => B::ReviewedSnapshot(n.unwrap_or(0)),
        Base::ForkPoint => B::ForkPoint(pb::ForkPointBase {}),
        Base::Index => B::Index(pb::IndexBase {}),
    };
    let (untracked, untracked_paths) = match &spec.compare.untracked {
        Untracked::None => (U::None, Vec::new()),
        Untracked::All => (U::All, Vec::new()),
        Untracked::Paths(paths) => (U::Paths, paths.clone()),
    };
    let upto = match spec.compare.upto {
        Upto::Commits => P::Commits,
        Upto::Staged => P::Staged,
        Upto::Unstaged => P::Unstaged,
    };
    pb::RepoDiffSpec {
        repo: Some(repo_ref(&spec.repo)),
        base: Some(pb::DiffSpecBase { base: Some(base) }),
        compare: Some(pb::DiffSpecCompare {
            upto: upto as i32,
            untracked: untracked as i32,
            untracked_paths,
            head: spec.compare.head.clone().unwrap_or_default(),
            worktree_paths: spec.compare.worktree_paths.clone(),
        }),
        include_paths: spec.paths.include.clone(),
        exclude_paths: spec.paths.exclude.clone(),
        whitespace: match spec.whitespace {
            Whitespace::Counts => pb::DiffWhitespace::Unspecified,
            Whitespace::IgnoreChanges => pb::DiffWhitespace::IgnoreChanges,
            Whitespace::IgnoreAll => pb::DiffWhitespace::IgnoreAll,
        } as i32,
        no_renames: !spec.renames,
    }
}

/// The repos among `repos` whose default base [`spec_for_base`] needs: every
/// one for `OriginMain`, and those a `Spec` leaves out. Looked up before the
/// diff, so a repo with a spec of its own needs no default branch.
#[must_use]
pub fn repos_needing_default(base: &pb::DiffBase, repos: &[String]) -> Vec<String> {
    use pb::diff_base::Base as B;
    match base.base.as_ref() {
        Some(B::OriginMain(_)) => repos.to_vec(),
        Some(B::Spec(spec)) => repos
            .iter()
            .filter(|repo| {
                !spec
                    .repos
                    .iter()
                    .any(|own| own.repo.as_ref().map(repo_key).as_deref() == Some(repo.as_str()))
            })
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

/// The spec for one of RV-1's bases on `repos` (keys) of a session whose repos
/// are `session`. `defaults` holds the default base of each repo that
/// [`repos_needing_default`] names (B5). `pull_request_base` gives a repo's PR
/// base branch (PR-10), such as `origin/main`.
///
/// # Errors
///
/// [`WireError`] for an unset base, a `spec` that doesn't parse, a repo of a
/// `spec` that the session doesn't have or names twice, a PR base on a repo
/// without a PR, a repo with no entry in `defaults`, or `other_session`, which
/// the caller resolves to that session's snapshot itself.
pub fn spec_for_base(
    base: &pb::DiffBase,
    repos: &[String],
    session: &[String],
    defaults: &BTreeMap<String, String>,
    pull_request_base: impl Fn(&str) -> Option<String>,
) -> Result<DiffSpec, WireError> {
    use pb::diff_base::Base as B;
    let each = |make: &dyn Fn(&str) -> RepoSpec| DiffSpec {
        repos: repos.iter().map(|repo| make(repo)).collect(),
    };
    match base.base.as_ref().ok_or_else(|| missing("base"))? {
        B::OriginMain(_) => repos
            .iter()
            .map(|repo| default_of(defaults, repo).map(|base| default_repo_spec(repo, base)))
            .collect::<Result<Vec<RepoSpec>, WireError>>()
            .map(|repos| DiffSpec { repos }),
        B::UncommittedOnly(_) => Ok(each(&|repo| RepoSpec::uncommitted(repo))),
        B::SinceLastReviewed(_) => Ok(each(&|repo| RepoSpec::since_last_review(repo))),
        B::SinceFork(_) => Ok(each(&|repo| RepoSpec::since_fork(repo))),
        B::PullRequestBase(_) => repos
            .iter()
            .map(|repo| {
                pull_request_base(repo)
                    .map(|branch| RepoSpec::since_branch(repo.clone(), branch))
                    .ok_or(WireError::Invalid {
                        field: "base.pull_request_base",
                        value: repo.clone(),
                        expected: "a repo of a PR review session; pick another base for this repo",
                    })
            })
            .collect::<Result<_, _>>()
            .map(|repos| DiffSpec { repos }),
        B::OtherSession(_) => Err(WireError::Invalid {
            field: "base.other_session",
            value: String::new(),
            expected: "resolved by the daemon to the other session's snapshot, not here",
        }),
        B::Spec(spec) => {
            let resolved = resolve_pull_request_bases(spec, &pull_request_base)?;
            spec_for_repos(&resolved, session, repos, defaults)
        }
    }
}

/// `spec` for `repos`: each repo's own spec where `spec` names it, else its
/// default base (B5). A repo `session` doesn't have, or one `spec` names twice,
/// is refused, so a spec can't quietly drop or double a repo.
fn spec_for_repos(
    spec: &pb::DiffSpec,
    session: &[String],
    repos: &[String],
    defaults: &BTreeMap<String, String>,
) -> Result<DiffSpec, WireError> {
    let named = spec_from_pb(spec)?;
    let mut seen = BTreeSet::new();
    for own in &named.repos {
        if !session.contains(&own.repo) {
            return Err(WireError::UnknownRepo {
                repo: own.repo.clone(),
                known: session.to_vec(),
            });
        }
        if !seen.insert(own.repo.as_str()) {
            return Err(WireError::RepeatedRepo {
                repo: own.repo.clone(),
            });
        }
    }
    let repos = repos
        .iter()
        .map(|repo| match named.repos.iter().find(|own| &own.repo == repo) {
            Some(own) => Ok(own.clone()),
            None => default_of(defaults, repo).map(|base| default_repo_spec(repo, base)),
        })
        .collect::<Result<Vec<RepoSpec>, WireError>>()?;
    Ok(DiffSpec { repos })
}

/// `base` with every pull request base (PR-10) written out as that base's
/// merge-base form, so a machine that keeps no pull request records can
/// compute it: `PullRequestBase` becomes a spec with each of `repos` (keys)
/// on its base, and a spec's repos on theirs. Any other base is `base` as it
/// is.
///
/// # Errors
///
/// [`WireError`] for a repo with no pull request, as [`spec_for_base`].
pub fn with_pull_request_bases(
    base: &pb::DiffBase,
    repos: &[String],
    pull_request_base: impl Fn(&str) -> Option<String>,
) -> Result<pb::DiffBase, WireError> {
    use pb::diff_base::Base as B;
    let spec = match base.base.as_ref() {
        Some(B::PullRequestBase(_)) => pb::DiffSpec {
            repos: repos
                .iter()
                .map(|repo| {
                    let branch = pull_request_base(repo).ok_or_else(|| WireError::Invalid {
                        field: "base.pull_request_base",
                        value: repo.clone(),
                        expected: "a repo of a PR review session; pick another base for this repo",
                    })?;
                    Ok(repo_spec_to_pb(&RepoSpec::since_branch(repo.clone(), branch)))
                })
                .collect::<Result<_, WireError>>()?,
        },
        Some(B::Spec(spec)) => resolve_pull_request_bases(spec, &pull_request_base)?,
        _ => return Ok(base.clone()),
    };
    Ok(pb::DiffBase {
        base: Some(B::Spec(spec)),
    })
}

/// `spec` with each repo on its pull request's base (PR-10) as that base's
/// merge-base form.
fn resolve_pull_request_bases(
    spec: &pb::DiffSpec,
    pull_request_base: &impl Fn(&str) -> Option<String>,
) -> Result<pb::DiffSpec, WireError> {
    use pb::diff_spec_base::Base as B;
    let mut resolved = spec.clone();
    for own in &mut resolved.repos {
        let on_pull_request = matches!(
            own.base.as_ref().and_then(|base| base.base.as_ref()),
            Some(B::PullRequestBase(_))
        );
        if !on_pull_request {
            continue;
        }
        let key = own.repo.as_ref().map(repo_key).ok_or_else(|| missing("repos.repo"))?;
        let branch = pull_request_base(&key).ok_or_else(|| WireError::Invalid {
            field: "base.spec.repos.base.pull_request_base",
            value: key,
            expected: "a repo of a PR review session; pick another base for this repo",
        })?;
        own.base = Some(pb::DiffSpecBase {
            base: Some(B::MergeBaseWith(branch)),
        });
    }
    Ok(resolved)
}

/// The default base of `repo`, which the caller looked up before the diff.
fn default_of<'a>(defaults: &'a BTreeMap<String, String>, repo: &str) -> Result<&'a str, WireError> {
    defaults.get(repo).map(String::as_str).ok_or_else(|| WireError::Invalid {
        field: "repos",
        value: repo.to_owned(),
        expected: "a repo the session has, with its default branch looked up before the diff; this is a bug in Humata, please report it",
    })
}

// ---------------------------------------------------------------------------
// Diffs

fn change_kind(status: &FileStatus) -> pb::ChangeKind {
    match status {
        FileStatus::Added => pb::ChangeKind::Added,
        FileStatus::Modified => pb::ChangeKind::Modified,
        FileStatus::Deleted => pb::ChangeKind::Deleted,
        FileStatus::Renamed { .. } => pb::ChangeKind::Renamed,
    }
}

/// A file's summary for the wire, with no tier worked out (unspecified,
/// which readers take as normal).
#[must_use]
pub fn file_summary_to_pb(file: &FileSummary) -> pb::FileSummary {
    summary_to_pb(file, pb::review_importance::Tier::Unspecified)
}

/// A file's summary with the tier the daemon worked out for it (D802).
#[must_use]
pub fn file_summary_to_pb_with(file: &FileSummary, tier: Tier) -> pb::FileSummary {
    summary_to_pb(file, tier_to_pb(tier))
}

fn tier_to_pb(tier: Tier) -> pb::review_importance::Tier {
    match tier {
        Tier::Key => pb::review_importance::Tier::Key,
        Tier::Normal => pb::review_importance::Tier::Normal,
        Tier::Skim => pb::review_importance::Tier::Skim,
    }
}

/// An unspecified or unknown tier is normal.
fn tier_from_pb(tier: i32) -> Tier {
    if tier == pb::review_importance::Tier::Key as i32 {
        Tier::Key
    } else if tier == pb::review_importance::Tier::Skim as i32 {
        Tier::Skim
    } else {
        Tier::Normal
    }
}

fn importance_from_pb(list: &[pb::ReviewImportance], field: &'static str) -> Result<Vec<Importance>, WireError> {
    list.iter()
        .map(|entry| {
            Ok(Importance {
                file: file_ref(entry.file.as_ref(), field)?,
                tier: tier_from_pb(entry.tier),
            })
        })
        .collect()
}

fn importance_to_pb(list: &[Importance]) -> Vec<pb::ReviewImportance> {
    list.iter()
        .map(|entry| pb::ReviewImportance {
            file: Some(repo_path(&entry.file)),
            tier: tier_to_pb(entry.tier) as i32,
        })
        .collect()
}

fn summary_to_pb(file: &FileSummary, tier: pb::review_importance::Tier) -> pb::FileSummary {
    pb::FileSummary {
        path: file.path.clone(),
        old_path: match &file.status {
            FileStatus::Renamed { from, .. } => from.clone(),
            _ => String::new(),
        },
        change: change_kind(&file.status) as i32,
        added: file.added,
        removed: file.removed,
        binary: file.binary,
        old_blob_sha: file.old_blob.clone().unwrap_or_default(),
        new_blob_sha: file.new_blob.clone().unwrap_or_default(),
        similarity: match file.status {
            FileStatus::Renamed { similarity, .. } => u32::from(similarity),
            _ => 0,
        },
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        tier: tier as i32,
    }
}

/// A repo's diff for GetDiffResponse. The PR, if any, is the caller's.
#[must_use]
pub fn repo_diff_to_pb(diff: &RepoDiff) -> pb::RepoDiff {
    repo_diff_with(diff, file_summary_to_pb)
}

/// As [`repo_diff_to_pb`], each file carrying its tier (D802); `tiers` is
/// by path and a missing path is normal.
#[must_use]
pub fn repo_diff_to_pb_with_tiers(diff: &RepoDiff, tiers: &std::collections::HashMap<String, Tier>) -> pb::RepoDiff {
    repo_diff_with(diff, |file| {
        file_summary_to_pb_with(file, tiers.get(&file.path).copied().unwrap_or_default())
    })
}

fn repo_diff_with(diff: &RepoDiff, summary: impl Fn(&FileSummary) -> pb::FileSummary) -> pb::RepoDiff {
    let (added, removed) = diff.totals();
    pb::RepoDiff {
        repo: Some(repo_ref(&diff.repo)),
        branch: diff.branch.clone().unwrap_or_default(),
        pull_request: None,
        added: u32::try_from(added).unwrap_or(u32::MAX),
        removed: u32::try_from(removed).unwrap_or(u32::MAX),
        files: diff.files.iter().map(summary).collect(),
        label: diff.label.clone(),
        trees: Some(trees_to_pb(&diff.trees)),
        head: diff.head.clone().unwrap_or_default(),
    }
}

/// A repo's diff back from [`repo_diff_to_pb`], as a saved review keeps
/// what its diff showed. `None` for one without its trees.
#[must_use]
pub fn repo_diff_from_pb(diff: &pb::RepoDiff) -> Option<RepoDiff> {
    let some = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    Some(RepoDiff {
        repo: repo_key(diff.repo.as_ref()?),
        branch: some(&diff.branch),
        head: some(&diff.head),
        label: diff.label.clone(),
        trees: trees_from_pb(diff.trees.as_ref())?,
        files: diff
            .files
            .iter()
            .map(|file| FileSummary {
                path: file.path.clone(),
                status: match pb::ChangeKind::try_from(file.change) {
                    Ok(pb::ChangeKind::Added) => FileStatus::Added,
                    Ok(pb::ChangeKind::Deleted) => FileStatus::Deleted,
                    Ok(pb::ChangeKind::Renamed) => FileStatus::Renamed {
                        from: file.old_path.clone(),
                        similarity: u8::try_from(file.similarity).unwrap_or(100),
                    },
                    _ => FileStatus::Modified,
                },
                added: file.added,
                removed: file.removed,
                binary: file.binary,
                old_blob: some(&file.old_blob_sha),
                new_blob: some(&file.new_blob_sha),
                old_mode: file.old_mode,
                new_mode: file.new_mode,
            })
            .collect(),
    })
}

#[must_use]
pub fn trees_to_pb(trees: &Trees) -> pb::DiffTrees {
    pb::DiffTrees {
        old_tree: trees.old.clone(),
        new_tree: trees.new.clone(),
    }
}

/// The trees a GetFileDiff names, if it names both.
#[must_use]
pub fn trees_from_pb(trees: Option<&pb::DiffTrees>) -> Option<Trees> {
    trees
        .filter(|trees| !trees.old_tree.is_empty() && !trees.new_tree.is_empty())
        .map(|trees| Trees {
            old: trees.old_tree.clone(),
            new: trees.new_tree.clone(),
        })
}

/// Where GetFileDiff starts, and how much it may return.
#[must_use]
pub fn chunk_request(request: &pb::GetFileDiffRequest) -> (Cursor, Budget) {
    let from = request.from.map_or_else(Cursor::default, |cursor| Cursor {
        hunk: cursor.hunk,
        line: cursor.line,
    });
    let mut budget = Budget::default();
    if request.max_lines != 0 {
        budget.max_lines = usize::try_from(request.max_lines).unwrap_or(budget.max_lines);
    }
    (from, budget)
}

fn hunk_to_pb(hunk: &Hunk) -> pb::Hunk {
    pb::Hunk {
        header: format!("@@ -{} +{} @@ {}", hunk.old_start, hunk.new_start, hunk.section)
            .trim_end()
            .to_owned(),
        lines: hunk.lines.iter().map(line_to_pb).collect(),
        index: hunk.index,
        old_start: hunk.old_start,
        new_start: hunk.new_start,
        section: hunk.section.clone(),
        continued: hunk.continued,
    }
}

fn line_to_pb(line: &DiffLine) -> pb::DiffLine {
    pb::DiffLine {
        kind: match line.kind {
            LineKind::Context => pb::diff_line::Kind::Context,
            LineKind::Added => pb::diff_line::Kind::Added,
            LineKind::Removed => pb::diff_line::Kind::Removed,
        } as i32,
        old_line: line.old_line.unwrap_or(0),
        new_line: line.new_line.unwrap_or(0),
        text: line.text.clone(),
        no_newline: line.no_newline,
    }
}

/// GetFileDiff's answer: a chunk of a file's hunks.
#[must_use]
pub fn file_diff_to_pb(repo: &str, file: &FileSummary, chunk: &FileChunk) -> pb::FileDiff {
    pb::FileDiff {
        file: Some(repo_path(&FileRef {
            repo: repo.to_owned(),
            path: file.path.clone(),
        })),
        summary: Some(file_summary_to_pb(file)),
        hunks: chunk.hunks.iter().map(hunk_to_pb).collect(),
        old_blob_sha: file.old_blob.clone().unwrap_or_default(),
        new_blob_sha: file.new_blob.clone().unwrap_or_default(),
        next: chunk.next.map(|next| pb::DiffCursor {
            hunk: next.hunk,
            line: next.line,
        }),
        viewed: pb::ViewedResult::Unspecified as i32,
    }
}

/// GetFileDiff's answer for a file marked Viewed (RV-29). For
/// [`Viewed::Forgotten`], `whole` is the file's whole diff chunk.
#[must_use]
pub fn viewed_file_diff_to_pb(
    repo: &str,
    file: &FileSummary,
    viewed: &Viewed,
    whole: Option<&FileChunk>,
) -> pb::FileDiff {
    let empty = FileChunk::default();
    let (chunk, result) = match viewed {
        Viewed::Unchanged => (&empty, pb::ViewedResult::Unchanged),
        Viewed::Changed(chunk) => (chunk, pb::ViewedResult::Changed),
        Viewed::Forgotten => (whole.unwrap_or(&empty), pb::ViewedResult::Forgotten),
    };
    let mut answer = file_diff_to_pb(repo, file, chunk);
    answer.viewed = result as i32;
    answer
}

/// A reviewed snapshot for the wire.
#[must_use]
pub fn snapshot_to_pb(repo: &str, snapshot: &Snapshot) -> pb::ReviewedSnapshot {
    pb::ReviewedSnapshot {
        repo: Some(repo_ref(repo)),
        n: snapshot.n,
        commit: snapshot.commit.clone(),
        tree: snapshot.tree.clone(),
        head: snapshot.head.clone().unwrap_or_default(),
        taken_at: Some(timestamp(snapshot.taken_at)),
        pinned: snapshot.pinned,
    }
}

// ---------------------------------------------------------------------------
// Review plans (RV-32, RV-34)

/// `review_prepare`'s plan from the wire. Anchors come from the daemon,
/// not the agent: they are dropped here and set with
/// [`crate::Checkout::anchor_notes`].
///
/// # Errors
///
/// [`WireError`] for a missing spec, file or line range.
pub fn plan_from_pb(plan: &pb::ReviewPlan) -> Result<ReviewPlan, WireError> {
    Ok(ReviewPlan {
        spec: spec_from_pb(plan.spec.as_ref().ok_or_else(|| missing("spec"))?)?,
        order: plan
            .order
            .iter()
            .map(|path| file_ref(Some(path), "order"))
            .collect::<Result<_, _>>()?,
        groups: plan
            .groups
            .iter()
            .map(group_from_pb)
            .collect::<Result<_, _>>()?,
        notes: plan
            .notes
            .iter()
            .map(note_from_pb)
            .collect::<Result<_, _>>()?,
        stack: plan.stack.as_ref().map(stack_from_pb).transpose()?,
        from_stack: plan.from_stack.as_ref().map(from_stack_from_pb).transpose()?,
        importance: importance_from_pb(&plan.importance, "importance.file")?,
    })
}

fn group_from_pb(group: &pb::ReviewGroup) -> Result<Group, WireError> {
    Ok(Group {
        title: group.title.clone(),
        note: group.note.clone(),
        first: file_ref(group.first.as_ref(), "groups.first")?,
    })
}

fn group_to_pb(group: &Group) -> pb::ReviewGroup {
    pb::ReviewGroup {
        title: group.title.clone(),
        note: group.note.clone(),
        first: Some(repo_path(&group.first)),
    }
}

fn note_from_pb(note: &pb::ReviewNote) -> Result<Note, WireError> {
    use pb::review_note::Target as T;
    let side = side_from(note.side);
    let lines = lines_from(note.lines.as_ref());
    let target = match (note.target(), lines) {
        (T::Unspecified | T::File, _) => NoteTarget::File,
        (T::Hunk, Some(lines)) => NoteTarget::Hunk {
            line: lines.first,
            side,
        },
        (T::Lines, Some(lines)) => NoteTarget::Lines {
            first: lines.first,
            last: lines.last,
            side,
        },
        (T::Hunk | T::Lines, None) => return Err(missing("notes.lines")),
    };
    Ok(Note {
        file: file_ref(note.file.as_ref(), "notes.file")?,
        target,
        text: note.text.clone(),
        anchor: None,
        layer: layer_from_wire(&note.layer),
    })
}

/// A layer title off the wire: empty is the whole stack.
fn layer_from_wire(layer: &str) -> Option<String> {
    (!layer.is_empty()).then(|| layer.to_owned())
}

fn note_to_pb(note: &Note, outdated: bool) -> pb::ReviewNote {
    use pb::review_note::Target as T;
    let (target, lines, side) = match note.target {
        NoteTarget::File => (T::File, None, Side::New),
        NoteTarget::Hunk { line, side } => (T::Hunk, Some(Lines { first: line, last: line }), side),
        NoteTarget::Lines { first, last, side } => (T::Lines, Some(Lines { first, last }), side),
    };
    let anchor = note.anchor.as_ref();
    pb::ReviewNote {
        file: Some(repo_path(&note.file)),
        target: target as i32,
        lines: lines.map(lines_to),
        side: side_to(side),
        text: note.text.clone(),
        blob_sha: anchor.map(|anchor| anchor.blob.clone()).unwrap_or_default(),
        anchored: anchor.map(|anchor| pb::LineRange {
            first: anchor.first,
            last: anchor.last,
        }),
        quoted: anchor.map(|anchor| anchor.quoted.clone()).unwrap_or_default(),
        outdated,
        layer: note.layer.clone().unwrap_or_default(),
    }
}

/// A plan for the wire; `outdated` says per note whether it went outdated
/// (RV-34), in the plan's note order. With `resolved`, any stack layers carry
/// their resolved commits and file counts.
#[must_use]
pub fn plan_to_pb(
    plan: &ReviewPlan,
    outdated: &[bool],
    resolved: Option<&CheckedStack>,
) -> pb::ReviewPlan {
    pb::ReviewPlan {
        spec: Some(spec_to_pb(&plan.spec)),
        order: plan.order.iter().map(repo_path).collect(),
        groups: plan.groups.iter().map(group_to_pb).collect(),
        notes: plan
            .notes
            .iter()
            .enumerate()
            .map(|(index, note)| note_to_pb(note, outdated.get(index).copied().unwrap_or(false)))
            .collect(),
        stack: plan.stack.as_ref().map(|stack| stack_to_pb(stack, resolved)),
        from_stack: plan.from_stack.as_ref().map(from_stack_to_pb),
        importance: importance_to_pb(&plan.importance),
    }
}

/// A stored plan back from the wire, anchors included.
///
/// # Errors
///
/// As [`plan_from_pb`].
pub fn stored_plan_from_pb(plan: &pb::ReviewPlan) -> Result<ReviewPlan, WireError> {
    let mut parsed = plan_from_pb(plan)?;
    for (note, wire) in parsed.notes.iter_mut().zip(&plan.notes) {
        note.anchor = anchor_from(&wire.blob_sha, wire.anchored.as_ref(), wire.quoted.clone());
    }
    Ok(parsed)
}

fn anchor_from(blob: &str, lines: Option<&pb::LineRange>, quoted: Vec<String>) -> Option<Anchor> {
    let lines = lines_from(lines)?;
    (!blob.is_empty()).then(|| Anchor {
        blob: blob.to_owned(),
        first: lines.first,
        last: lines.last,
        quoted,
    })
}

// ---------------------------------------------------------------------------
// The review chat (PR-22–PR-26)

#[must_use]
pub fn switches_to_pb(switches: AgentSwitches) -> pb::AgentSwitches {
    pb::AgentSwitches {
        focus: switches.focus,
        notes: switches.notes,
        order: switches.order,
    }
}

/// Switches from the wire; unset means the default, all on.
#[must_use]
pub fn switches_from_pb(switches: Option<&pb::AgentSwitches>) -> AgentSwitches {
    switches.map_or_else(AgentSwitches::default, |switches| AgentSwitches {
        focus: switches.focus,
        notes: switches.notes,
        order: switches.order,
    })
}

fn place_from_pb(place: &pb::ReviewPlace) -> Result<Place, WireError> {
    Ok(Place {
        file: file_ref(place.file.as_ref(), "open.file")?,
        side: side_from(place.side),
        visible: lines_from(place.visible.as_ref()),
        selection: lines_from(place.selection.as_ref()),
    })
}

fn place_to_pb(place: &Place) -> pb::ReviewPlace {
    pb::ReviewPlace {
        file: Some(repo_path(&place.file)),
        visible: place.visible.map(lines_to),
        selection: place.selection.map(lines_to),
        side: side_to(place.side),
    }
}

fn highlight_from_pb(highlight: &pb::Highlight) -> Result<Highlight, WireError> {
    Ok(Highlight {
        file: file_ref(highlight.file.as_ref(), "highlights.file")?,
        side: side_from(highlight.side),
        lines: lines_from(highlight.lines.as_ref()).ok_or_else(|| missing("highlights.lines"))?,
        label: highlight.label.clone(),
    })
}

fn highlight_to_pb(highlight: &Highlight) -> pb::Highlight {
    pb::Highlight {
        file: Some(repo_path(&highlight.file)),
        lines: Some(lines_to(highlight.lines)),
        side: side_to(highlight.side),
        label: highlight.label.clone(),
    }
}

/// The view the app reports (ReportReviewView), or a persisted one.
///
/// # Errors
///
/// [`WireError`] for a file or range that is missing.
pub fn view_from_pb(view: &pb::ReviewViewState) -> Result<ViewState, WireError> {
    Ok(ViewState {
        spec: view
            .spec
            .as_ref()
            .map(spec_from_pb)
            .transpose()?
            .unwrap_or_default(),
        open: view.open.as_ref().map(place_from_pb).transpose()?,
        order: view
            .order
            .iter()
            .map(|path| file_ref(Some(path), "order"))
            .collect::<Result<_, _>>()?,
        groups: view
            .groups
            .iter()
            .map(group_from_pb)
            .collect::<Result<_, _>>()?,
        viewed: view
            .viewed
            .iter()
            .map(|viewed| {
                Ok(ViewedFile {
                    file: file_ref(viewed.file.as_ref(), "viewed.file")?,
                    blob: viewed.blob_sha.clone(),
                })
            })
            .collect::<Result<_, WireError>>()?,
        highlights: view
            .highlights
            .iter()
            .map(highlight_from_pb)
            .collect::<Result<_, _>>()?,
        stack: view.stack.as_ref().map(stack_from_pb).transpose()?,
        layer: layer_from_wire(&view.layer),
        importance: importance_from_pb(&view.importance, "importance")?,
    })
}

/// A view for the wire, and for the store to persist (PR-26). With `resolved`,
/// any stack layers carry their resolved commits and file counts.
#[must_use]
pub fn view_to_pb(view: &ViewState, resolved: Option<&CheckedStack>) -> pb::ReviewViewState {
    pb::ReviewViewState {
        spec: Some(spec_to_pb(&view.spec)),
        open: view.open.as_ref().map(place_to_pb),
        order: view.order.iter().map(repo_path).collect(),
        groups: view.groups.iter().map(group_to_pb).collect(),
        viewed: view
            .viewed
            .iter()
            .map(|viewed| pb::ViewedFile {
                file: Some(repo_path(&viewed.file)),
                blob_sha: viewed.blob.clone(),
            })
            .collect(),
        highlights: view.highlights.iter().map(highlight_to_pb).collect(),
        stack: view.stack.as_ref().map(|stack| stack_to_pb(stack, resolved)),
        layer: view.layer.clone().unwrap_or_default(),
        importance: importance_to_pb(&view.importance),
    }
}

/// `review_focus`'s request as a place and highlights.
///
/// # Errors
///
/// [`WireError`] for a missing file or highlight range.
pub fn focus_from_pb(request: &pb::FocusReviewRequest) -> Result<(Place, Vec<Highlight>), WireError> {
    let place = Place {
        file: file_ref(request.file.as_ref(), "file")?,
        side: side_from(request.side),
        visible: None,
        selection: lines_from(request.lines.as_ref()),
    };
    let highlights = request
        .highlights
        .iter()
        .map(highlight_from_pb)
        .collect::<Result<_, _>>()?;
    Ok((place, highlights))
}

/// `review_note`'s anchor (CreateComment with a `note` destination) as a
/// note: lines when the anchor has them, the file otherwise.
///
/// # Errors
///
/// [`WireError`] for a missing anchor or file.
pub fn note_from_anchor(anchor: Option<&pb::CommentAnchor>, text: &str) -> Result<Note, WireError> {
    let anchor = anchor.ok_or_else(|| missing("anchor"))?;
    let side = side_from(anchor.side);
    let target = match lines_from(anchor.lines.as_ref()) {
        Some(lines) => NoteTarget::Lines {
            first: lines.first,
            last: lines.last,
            side,
        },
        None => NoteTarget::File,
    };
    Ok(Note {
        file: file_ref(anchor.file.as_ref(), "anchor.file")?,
        target,
        text: text.to_owned(),
        anchor: None,
        layer: layer_from_wire(&anchor.layer),
    })
}

/// A note for the user as a Comment with the `note` destination. The
/// daemon fills in its id's session, author, revision and time.
#[must_use]
pub fn user_note_to_pb(note: &UserNote) -> pb::Comment {
    let (lines, side) = match note.note.target {
        NoteTarget::File => (None, Side::New),
        NoteTarget::Hunk { line, side } => (Some(Lines { first: line, last: line }), side),
        NoteTarget::Lines { first, last, side } => (Some(Lines { first, last }), side),
    };
    let anchor = note.note.anchor.as_ref();
    pb::Comment {
        id: Some(pb::CommentId {
            value: note.id.clone(),
        }),
        anchor: Some(pb::CommentAnchor {
            file: Some(repo_path(&note.note.file)),
            lines: lines.map(lines_to),
            side: side_to(side),
            quoted: anchor.map(|anchor| anchor.quoted.join("\n")).unwrap_or_default(),
            blob_sha: anchor.map(|anchor| anchor.blob.clone()).unwrap_or_default(),
            detached: false,
            outdated: note.outdated,
            layer: note.note.layer.clone().unwrap_or_default(),
            ..pb::CommentAnchor::default()
        }),
        text: note.note.text.clone(),
        destination: Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::Note(pb::UserNote {})),
        }),
        undo_id: note.undo_id.clone(),
        ..pb::Comment::default()
    }
}

/// A persisted note back, with the agent's name as the store keeps it.
///
/// # Errors
///
/// [`WireError`] for a comment without an id or anchor.
pub fn user_note_from_pb(comment: &pb::Comment, author: &str) -> Result<UserNote, WireError> {
    let mut note = note_from_anchor(comment.anchor.as_ref(), &comment.text)?;
    if let Some(anchor) = &comment.anchor {
        let quoted = anchor.quoted.lines().map(str::to_owned).collect();
        note.anchor = anchor_from(&anchor.blob_sha, anchor.lines.as_ref(), quoted);
    }
    Ok(UserNote {
        id: comment
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| missing("id"))?,
        author: author.to_owned(),
        note,
        undo_id: comment.undo_id.clone(),
        outdated: comment.anchor.as_ref().is_some_and(|anchor| anchor.outdated),
    })
}

/// `review_context`'s answer. `selected_text` is the selection's lines,
/// read by the caller from the open file.
#[must_use]
pub fn context_to_pb(
    context: &Context,
    selected_text: Vec<String>,
    resolved: Option<&CheckedStack>,
) -> pb::ReviewContext {
    pb::ReviewContext {
        view: Some(view_to_pb(&context.view, resolved)),
        selected_text,
        notes: context.notes.iter().map(user_note_to_pb).collect(),
        switches: Some(switches_to_pb(context.switches)),
    }
}

/// A change for the chat line and the app's feed.
#[must_use]
pub fn change_to_pb(
    change: &Change,
    at: i64,
    resolved: Option<&CheckedStack>,
) -> pb::ReviewChange {
    use pb::review_change::Change as C;
    let (tool, undid) = match change.tool {
        ChangeTool::Tool(tool) => (tool.tool_name().to_owned(), String::new()),
        ChangeTool::Undo { undid: 0 } => ("undo".to_owned(), String::new()),
        ChangeTool::Undo { undid } => ("undo".to_owned(), format!("change-{undid}")),
        ChangeTool::Diff => ("diff".to_owned(), String::new()),
    };
    let what = match &change.what {
        ChangeKind::Focus(place) => place.as_ref().map(|place| C::Focus(place_to_pb(place))),
        ChangeKind::Note(note) => Some(C::Note(user_note_to_pb(note))),
        ChangeKind::NoteDropped(id) => Some(C::NoteDropped(pb::CommentId { value: id.clone() })),
        ChangeKind::Order {
            order,
            groups,
            stack,
            importance,
            ..
        } => Some(C::Plan(pb::ReviewPlan {
            spec: None,
            order: order.iter().map(repo_path).collect(),
            groups: groups.iter().map(group_to_pb).collect(),
            notes: Vec::new(),
            stack: stack.as_ref().map(|stack| stack_to_pb(stack, resolved)),
            importance: importance_to_pb(importance),
            ..pb::ReviewPlan::default()
        })),
        ChangeKind::Layer(layer) => Some(C::Layer(layer.clone().unwrap_or_default())),
    };
    pb::ReviewChange {
        undo_id: change.undo_id.clone(),
        seq: change.seq,
        tool,
        summary: change.summary.clone(),
        change: what,
        highlights: change.highlights.iter().map(highlight_to_pb).collect(),
        undid,
        at: Some(timestamp(at)),
    }
}

/// The `Denied` a switched-off tool returns (PR-25), or `None` for any
/// other error. Its message is the error's own text.
#[must_use]
pub fn denied(error: &ChatError) -> Option<pb::Denied> {
    matches!(error, ChatError::SwitchOff { .. }).then(|| pb::Denied {
        source: pb::denied::Source::ReviewSwitchOff as i32,
        ..pb::Denied::default()
    })
}

// ---------------------------------------------------------------------------
// Stacks (D800)

/// A stack for the wire. With `resolved`, each layer repo carries the
/// commits and file counts it resolved to when the stack was checked
/// (only applied when `resolved` matches `stack`).
#[must_use]
pub fn stack_to_pb(stack: &Stack, resolved: Option<&CheckedStack>) -> pb::ReviewStack {
    let resolved = resolved.filter(|checked| checked.stack() == stack);
    pb::ReviewStack {
        layers: stack
            .layers
            .iter()
            .map(|layer| layer_to_pb(layer, resolved))
            .collect(),
    }
}

/// Applies `drift` findings onto a wire stack, marking drifted layer repos
/// and zeroing their `files` count (review.proto: `files` is 0 when drifted).
pub fn apply_drift_to_pb(stack: &mut pb::ReviewStack, drift: &[Drift]) {
    for found in drift {
        let kind = match found.kind {
            DriftKind::Moved => pb::review_layer_resolved::Drift::Moved,
            DriftKind::Missing => pb::review_layer_resolved::Drift::Missing,
            DriftKind::HeadMoved => pb::review_layer_resolved::Drift::HeadMoved,
        };
        let resolved = stack
            .layers
            .iter_mut()
            .filter(|layer| layer.title == found.layer)
            .flat_map(|layer| layer.repos.iter_mut())
            .filter(|part| {
                part.repo.as_ref().map(repo_key).as_deref() == Some(found.repo.as_str())
            })
            .filter_map(|part| part.resolved.as_mut());
        for resolved in resolved {
            if resolved.drift == pb::review_layer_resolved::Drift::None as i32
                || resolved.drift == pb::review_layer_resolved::Drift::Unspecified as i32
            {
                resolved.drift = kind as i32;
                resolved.files = 0;
            }
        }
    }
}

/// Reconstructs a `CheckedStack` from a stored wire stack's `resolved` fields
/// so drift can be checked without losing stored commits. Returns `None` if
/// the stack cannot be parsed or has no resolved entries.
#[must_use]
pub fn checked_stack_from_pb(wire: &pb::ReviewStack) -> Option<CheckedStack> {
    let stack = stack_from_pb(wire).ok()?;
    let mut any = false;
    let resolved: Vec<Vec<Resolved>> = wire
        .layers
        .iter()
        .map(|layer| {
            layer
                .repos
                .iter()
                .map(|part| match part.resolved.as_ref() {
                    Some(found) => {
                        any = true;
                        Resolved {
                            base_commit: found.base_commit.clone(),
                            head_commit: found.head_commit.clone(),
                            files: found.files,
                        }
                    }
                    None => Resolved {
                        base_commit: String::new(),
                        head_commit: String::new(),
                        files: 0,
                    },
                })
                .collect()
        })
        .collect();
    any.then(|| CheckedStack::from_resolved(stack, resolved))
        .flatten()
}

fn layer_to_pb(layer: &Layer, resolved: Option<&CheckedStack>) -> pb::ReviewLayer {
    pb::ReviewLayer {
        title: layer.title.clone(),
        purpose: layer.purpose.clone().unwrap_or_default(),
        repos: layer
            .repos
            .iter()
            .map(|repo| layer_repo_to_pb(&layer.title, repo, resolved))
            .collect(),
        topic: layer.topic.clone().unwrap_or_default(),
        order: layer.order.iter().map(repo_path).collect(),
        groups: layer.groups.iter().map(group_to_pb).collect(),
        importance: importance_to_pb(&layer.importance),
    }
}

fn layer_repo_to_pb(
    title: &str,
    repo: &LayerRepo,
    resolved: Option<&CheckedStack>,
) -> pb::ReviewLayerRepo {
    use pb::review_layer_repo::Part as P;
    let part = match &repo.part {
        Part::Range { base, head } => P::Range(pb::ReviewLayerRange {
            base: base.clone(),
            head: head.clone(),
        }),
        Part::Worktree { include_paths } => P::Worktree(pb::ReviewLayerWorktree {
            include_paths: include_paths.clone(),
        }),
    };
    pb::ReviewLayerRepo {
        repo: Some(repo_ref(&repo.repo)),
        part: Some(part),
        pull_request: repo.pull_request.clone().unwrap_or_default(),
        resolved: resolved
            .and_then(|checked| checked.resolved(title, &repo.repo))
            .map(|found| pb::ReviewLayerResolved {
                base_commit: found.base_commit.clone(),
                head_commit: found.head_commit.clone(),
                // Resolved and checked: no drift until the daemon finds some.
                drift: pb::review_layer_resolved::Drift::None as i32,
                files: found.files,
            }),
    }
}

/// A stack from the wire.
///
/// # Errors
///
/// [`WireError`] for a layer repo without a repo or a part.
pub fn stack_from_pb(stack: &pb::ReviewStack) -> Result<Stack, WireError> {
    let layers: Vec<Layer> = stack
        .layers
        .iter()
        .map(layer_from_pb)
        .collect::<Result<_, _>>()?;
    Ok(Stack::new(layers))
}

fn layer_from_pb(layer: &pb::ReviewLayer) -> Result<Layer, WireError> {
    let repos: Vec<LayerRepo> = layer
        .repos
        .iter()
        .map(layer_repo_from_pb)
        .collect::<Result<_, _>>()?;
    Ok(Layer {
        title: layer.title.clone(),
        purpose: (!layer.purpose.is_empty()).then(|| layer.purpose.clone()),
        repos,
        topic: (!layer.topic.is_empty()).then(|| layer.topic.clone()),
        order: layer
            .order
            .iter()
            .map(|path| file_ref(Some(path), "layers.order"))
            .collect::<Result<_, _>>()?,
        groups: layer
            .groups
            .iter()
            .map(group_from_pb)
            .collect::<Result<_, _>>()?,
        importance: importance_from_pb(&layer.importance, "layers.importance.file")?,
    })
}

fn layer_repo_from_pb(repo: &pb::ReviewLayerRepo) -> Result<LayerRepo, WireError> {
    use pb::review_layer_repo::Part as P;
    let key = repo_key(
        repo.repo
            .as_ref()
            .ok_or_else(|| missing("layers.repos.repo"))?,
    );
    let part = match &repo.part {
        Some(P::Range(range)) => Part::Range {
            base: range.base.clone(),
            head: range.head.clone(),
        },
        Some(P::Worktree(worktree)) => Part::Worktree {
            include_paths: worktree.include_paths.clone(),
        },
        None => {
            return Err(WireError::Invalid {
                field: "layers.repos.part",
                value: key,
                expected: "each layer repo needs a range (its base and head revs) or a worktree (its include_paths); set one",
            });
        }
    };
    Ok(LayerRepo {
        repo: key,
        part,
        pull_request: (!repo.pull_request.is_empty()).then(|| repo.pull_request.clone()),
    })
}

/// A `from_stack` request for the wire (D803).
#[must_use]
pub fn from_stack_to_pb(from: &FromStack) -> pb::ReviewFromStack {
    pb::ReviewFromStack {
        repo: Some(repo_ref(&from.repo)),
        tip: from.tip.clone().unwrap_or_default(),
        topics: from
            .topics
            .iter()
            .map(|(name, topic)| {
                (
                    name.clone(),
                    pb::ReviewTopicOverride {
                        title: topic.title.clone().unwrap_or_default(),
                        purpose: topic.purpose.clone().unwrap_or_default(),
                    },
                )
            })
            .collect(),
    }
}

/// A `from_stack` request from the wire (D803).
///
/// # Errors
///
/// [`WireError`] for a request without its repo.
pub fn from_stack_from_pb(from: &pb::ReviewFromStack) -> Result<FromStack, WireError> {
    let some = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    Ok(FromStack {
        repo: repo_key(from.repo.as_ref().ok_or_else(|| missing("from_stack.repo"))?),
        tip: some(&from.tip),
        topics: from
            .topics
            .iter()
            .map(|(name, topic)| {
                (
                    name.clone(),
                    TopicOverride {
                        title: some(&topic.title),
                        purpose: some(&topic.purpose),
                    },
                )
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shown_diff_round_trips_through_the_wire() {
        let diff = RepoDiff {
            repo: "acme/api".to_owned(),
            branch: Some("fix".to_owned()),
            head: None,
            label: "HEAD + unstaged".to_owned(),
            trees: Trees {
                old: "a".repeat(40),
                new: "b".repeat(40),
            },
            files: vec![
                FileSummary {
                    path: "new.rs".to_owned(),
                    status: FileStatus::Added,
                    added: 3,
                    removed: 0,
                    binary: false,
                    old_blob: None,
                    new_blob: Some("c".repeat(40)),
                    old_mode: 0,
                    new_mode: 100_644,
                },
                FileSummary {
                    path: "moved.rs".to_owned(),
                    status: FileStatus::Renamed {
                        from: "old.rs".to_owned(),
                        similarity: 87,
                    },
                    added: 1,
                    removed: 1,
                    binary: true,
                    old_blob: Some("d".repeat(40)),
                    new_blob: Some("e".repeat(40)),
                    old_mode: 100_644,
                    new_mode: 100_755,
                },
                FileSummary {
                    path: "gone.rs".to_owned(),
                    status: FileStatus::Deleted,
                    added: 0,
                    removed: 9,
                    binary: false,
                    old_blob: Some("f".repeat(40)),
                    new_blob: None,
                    old_mode: 100_644,
                    new_mode: 0,
                },
            ],
        };
        assert_eq!(repo_diff_from_pb(&repo_diff_to_pb(&diff)), Some(diff));
    }

    #[test]
    fn a_spec_round_trips_through_the_wire() {
        let spec = DiffSpec {
            repos: vec![
                RepoSpec {
                    repo: "acme/api".to_owned(),
                    base: Base::MergeBase {
                        with: "origin/main".to_owned(),
                    },
                    compare: Compare {
                        upto: Upto::Staged,
                        untracked: Untracked::Paths(vec!["migrations/".to_owned()]),
                        head: None,
                        worktree_paths: Vec::new(),
                    },
                    paths: PathFilter {
                        include: vec!["src/".to_owned()],
                        exclude: vec!["*.lock".to_owned()],
                    },
                    whitespace: Whitespace::IgnoreAll,
                    renames: false,
                },
                RepoSpec::since_last_review("ledger"),
            ],
        };
        assert_eq!(spec_from_pb(&spec_to_pb(&spec)), Ok(spec));
    }

    #[test]
    fn an_unset_compare_means_everything_on_disk() {
        let wire = pb::RepoDiffSpec {
            repo: Some(repo_ref("api")),
            base: Some(pb::DiffSpecBase {
                base: Some(pb::diff_spec_base::Base::Rev("HEAD".to_owned())),
            }),
            ..pb::RepoDiffSpec::default()
        };
        let parsed = spec_from_pb(&pb::DiffSpec { repos: vec![wire] });
        assert_eq!(
            parsed.map(|spec| spec.repos[0].compare.clone()),
            Ok(Compare::everything())
        );
    }

    #[test]
    fn a_spec_without_a_base_is_refused() {
        let wire = pb::RepoDiffSpec {
            repo: Some(repo_ref("api")),
            ..pb::RepoDiffSpec::default()
        };
        assert!(spec_from_pb(&pb::DiffSpec { repos: vec![wire] }).is_err());
    }

    #[test]
    fn a_pr_base_needs_each_repos_pr() {
        let base = pb::DiffBase {
            base: Some(pb::diff_base::Base::PullRequestBase(pb::PullRequestBase {})),
        };
        let repos = vec!["api".to_owned(), "ledger".to_owned()];
        let only_api = |repo: &str| (repo == "api").then(|| "origin/main".to_owned());
        let none = BTreeMap::new();
        assert!(spec_for_base(&base, &repos, &repos, &none, only_api).is_err());
        let both = |_: &str| Some("origin/main".to_owned());
        assert_eq!(
            spec_for_base(&base, &repos, &repos, &none, both).map(|spec| spec.repos.len()),
            Ok(2)
        );
    }

    #[test]
    fn a_spec_repo_on_its_pr_base_is_compared_with_that_base() {
        let on = |repo: &str, base: pb::diff_spec_base::Base| pb::RepoDiffSpec {
            repo: Some(repo_ref(repo)),
            base: Some(pb::DiffSpecBase { base: Some(base) }),
            ..pb::RepoDiffSpec::default()
        };
        let base = pb::DiffBase {
            base: Some(pb::diff_base::Base::Spec(pb::DiffSpec {
                repos: vec![
                    on("api", pb::diff_spec_base::Base::PullRequestBase(pb::PullRequestBaseRef {})),
                    on("ledger", pb::diff_spec_base::Base::Rev("v1".to_owned())),
                ],
            })),
        };
        let repos = vec!["api".to_owned(), "ledger".to_owned()];
        let none = BTreeMap::new();
        let release = |repo: &str| (repo == "api").then(|| "origin/release".to_owned());
        let spec = spec_for_base(&base, &repos, &repos, &none, release);
        assert_eq!(
            spec.map(|spec| spec.repos.into_iter().map(|each| each.base).collect::<Vec<_>>()),
            Ok(vec![
                Base::MergeBase { with: "origin/release".to_owned() },
                Base::Rev { rev: "v1".to_owned() },
            ])
        );
        let nobody = |_: &str| None;
        assert!(spec_for_base(&base, &repos, &repos, &none, nobody).is_err());
    }

    #[test]
    fn a_pr_base_written_out_needs_no_pull_requests_where_it_is_computed() {
        let pr = pb::DiffBase {
            base: Some(pb::diff_base::Base::PullRequestBase(pb::PullRequestBase {})),
        };
        let repos = vec!["acme/api".to_owned(), "acme/ledger".to_owned()];
        let none = BTreeMap::new();
        let bases = |repo: &str| Some(if repo == "acme/api" { "origin/main" } else { "origin/release" }.to_owned());
        let here = spec_for_base(&pr, &repos, &repos, &none, bases);
        let written = with_pull_request_bases(&pr, &repos, bases).unwrap();
        let nobody = |_: &str| None;
        assert_eq!(spec_for_base(&written, &repos, &repos, &none, nobody), here);

        let only_api = |repo: &str| (repo == "acme/api").then(|| "origin/main".to_owned());
        assert!(with_pull_request_bases(&pr, &repos, only_api).is_err());

        let uncommitted = pb::DiffBase {
            base: Some(pb::diff_base::Base::UncommittedOnly(pb::UncommittedOnly {})),
        };
        assert_eq!(with_pull_request_bases(&uncommitted, &repos, nobody), Ok(uncommitted.clone()));
    }

    #[test]
    fn a_user_note_round_trips_with_its_anchor() {
        let note = UserNote {
            id: "note-1".to_owned(),
            author: "Claude Code".to_owned(),
            note: Note {
                file: FileRef {
                    repo: "api".to_owned(),
                    path: "src/refund.rs".to_owned(),
                },
                target: NoteTarget::Lines {
                    first: 3,
                    last: 4,
                    side: Side::New,
                },
                text: "rounds half away from zero".to_owned(),
                anchor: Some(Anchor {
                    blob: "abc".to_owned(),
                    first: 3,
                    last: 4,
                    quoted: vec!["a".to_owned(), "b".to_owned()],
                }),
                layer: Some("codec".to_owned()),
            },
            undo_id: "change-2".to_owned(),
            outdated: true,
        };
        assert_eq!(
            user_note_from_pb(&user_note_to_pb(&note), "Claude Code"),
            Ok(note)
        );
    }

    #[test]
    fn only_a_switched_off_tool_is_a_denial() {
        let off = ChatError::SwitchOff {
            tool: "review_focus",
            switch: crate::chat::Switch::Focus,
        };
        assert!(denied(&off).is_some());
        assert!(denied(&ChatError::NothingBack).is_none());
    }

    // D800: a layer's compare and a stack on the wire.

    #[test]
    fn a_compare_with_a_head_and_worktree_paths_round_trips() {
        let spec = DiffSpec {
            repos: vec![
                RepoSpec::new(
                    "acme/api",
                    Base::Rev {
                        rev: "codec".to_owned(),
                    },
                    Compare::at("journal"),
                ),
                RepoSpec::new(
                    "acme/web",
                    Base::Rev {
                        rev: "HEAD".to_owned(),
                    },
                    Compare::worktree(vec!["src/".to_owned()]),
                ),
            ],
        };
        let back = spec_from_pb(&spec_to_pb(&spec)).expect("the spec reads back");
        assert_eq!(back, spec);
    }

    #[test]
    fn a_head_on_the_wire_with_upto_unset_means_commits_only() {
        let wire = pb::DiffSpec {
            repos: vec![pb::RepoDiffSpec {
                repo: Some(repo_ref("acme/api")),
                base: Some(pb::DiffSpecBase {
                    base: Some(pb::diff_spec_base::Base::Rev("codec".to_owned())),
                }),
                compare: Some(pb::DiffSpecCompare {
                    head: "journal".to_owned(),
                    ..pb::DiffSpecCompare::default()
                }),
                ..pb::RepoDiffSpec::default()
            }],
        };
        let spec = spec_from_pb(&wire).expect("a head alone is valid");
        assert_eq!(spec.repos[0].compare, Compare::at("journal"));
    }

    fn three_layer_stack() -> Stack {
        Stack::new(vec![
            Layer {
                title: "codec".to_owned(),
                purpose: Some("Adds the zstd codec".to_owned()),
                repos: vec![
                    LayerRepo {
                        repo: "acme/api".to_owned(),
                        part: Part::Range {
                            base: "main".to_owned(),
                            head: "codec".to_owned(),
                        },
                        pull_request: Some("acme/api#412".to_owned()),
                    },
                    LayerRepo {
                        repo: "acme/web".to_owned(),
                        part: Part::Range {
                            base: "main".to_owned(),
                            head: "codec".to_owned(),
                        },
                        pull_request: None,
                    },
                ],
                ..Layer::default()
            },
            Layer {
                title: "Not committed yet".to_owned(),
                purpose: None,
                repos: vec![LayerRepo {
                    repo: "acme/api".to_owned(),
                    part: Part::Worktree {
                        include_paths: vec!["src/journal/".to_owned()],
                    },
                    pull_request: None,
                }],
                ..Layer::default()
            },
        ])
    }

    #[test]
    fn a_stack_round_trips_through_the_wire() {
        let stack = three_layer_stack();
        let back = stack_from_pb(&stack_to_pb(&stack, None)).expect("the stack reads back");
        assert_eq!(back, stack);
    }

    #[test]
    fn a_layer_repo_with_neither_range_nor_worktree_is_refused() {
        let mut wire = stack_to_pb(&three_layer_stack(), None);
        wire.layers[0].repos[0].part = None;
        assert!(stack_from_pb(&wire).is_err());
    }

    #[test]
    fn resolved_stack_threads_through_plan_view_and_change_with_files() {
        let stack = three_layer_stack();
        let resolved = vec![
            vec![
                Resolved {
                    base_commit: "1111".to_owned(),
                    head_commit: "2222".to_owned(),
                    files: 3,
                },
                Resolved {
                    base_commit: "aaaa".to_owned(),
                    head_commit: "bbbb".to_owned(),
                    files: 1,
                },
            ],
            vec![Resolved {
                base_commit: "2222".to_owned(),
                head_commit: "2222".to_owned(),
                files: 2,
            }],
        ];
        let checked = CheckedStack::from_resolved(stack.clone(), resolved).expect("shape matches");

        let plan = ReviewPlan {
            spec: DiffSpec::default(),
            order: Vec::new(),
            groups: Vec::new(),
            notes: Vec::new(),
            stack: Some(stack.clone()),
            from_stack: None,
            importance: Vec::new(),
        };
        let pb_plan = plan_to_pb(&plan, &[], Some(&checked));
        let pb_stack = pb_plan.stack.as_ref().expect("stack present");
        let r0 = pb_stack.layers[0].repos[0]
            .resolved
            .as_ref()
            .expect("resolved present on plan");
        assert_eq!(r0.base_commit, "1111");
        assert_eq!(r0.head_commit, "2222");
        assert_eq!(r0.files, 3);
        assert_eq!(r0.drift, pb::review_layer_resolved::Drift::None as i32);

        let view = ViewState {
            stack: Some(stack.clone()),
            ..ViewState::default()
        };
        let pb_view = view_to_pb(&view, Some(&checked));
        let vr1 = pb_view.stack.as_ref().unwrap().layers[1].repos[0]
            .resolved
            .as_ref()
            .expect("resolved present on view");
        assert_eq!(vr1.files, 2);

        let change = Change {
            seq: 1,
            undo_id: "u1".to_owned(),
            tool: ChangeTool::Diff,
            summary: String::new(),
            highlights: Vec::new(),
            what: ChangeKind::Order {
                order: Vec::new(),
                groups: Vec::new(),
                stack: Some(stack.clone()),
                layer: None,
                importance: Vec::new(),
            },
        };
        let pb_change = change_to_pb(&change, 100, Some(&checked));
        let Some(pb::review_change::Change::Plan(cp)) = pb_change.change else {
            panic!("expected plan change");
        };
        let cr0 = cp.stack.as_ref().unwrap().layers[0].repos[1]
            .resolved
            .as_ref()
            .expect("resolved present on change");
        assert_eq!(cr0.files, 1);

        // Round-trip via checked_stack_from_pb and apply_drift_to_pb.
        let mut drifted_stack = pb_stack.clone();
        let rebuilt = checked_stack_from_pb(&drifted_stack).expect("rebuilt");
        assert_eq!(rebuilt.resolved("codec", "acme/api").unwrap().files, 3);
        apply_drift_to_pb(
            &mut drifted_stack,
            &[Drift {
                layer: "codec".to_owned(),
                repo: "acme/api".to_owned(),
                kind: DriftKind::Moved,
            }],
        );
        let after = drifted_stack.layers[0].repos[0].resolved.as_ref().unwrap();
        assert_eq!(after.drift, pb::review_layer_resolved::Drift::Moved as i32);
        assert_eq!(after.files, 0);
        // Untouched repo keeps its file count.
        assert_eq!(drifted_stack.layers[0].repos[1].resolved.as_ref().unwrap().files, 1);
    }
}
