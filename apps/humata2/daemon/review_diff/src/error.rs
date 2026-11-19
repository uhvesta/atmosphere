//! What computing a review diff can fail at.

use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

/// Why a diff, a snapshot or a file's hunks couldn't be produced.
#[derive(Debug, Error)]
pub enum DiffError {
    #[error(
        "couldn't run `{program}` to read the repository; install git (2.38 or newer) and make sure it is on humatad2's PATH"
    )]
    GitMissing {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{} is not a git repository (or git can't read it); check that the clone exists on this machine and belongs to you. If it was moved or deleted, recreate it from the session's Info panel",
        path.display()
    )]
    NotARepository { path: PathBuf, stderr: String },

    #[error(
        "`git {args}` failed in {} ({status}): {stderr}. Nothing in the repository was changed; fix what git reports and try again",
        repo.display()
    )]
    Git {
        repo: PathBuf,
        args: String,
        status: ExitStatus,
        stderr: String,
    },

    #[error(
        "`{rev}` doesn't name a commit in {}; check the spelling, or fetch it first (`git fetch origin`) if it is a remote branch",
        repo.display()
    )]
    UnknownRevision { repo: PathBuf, rev: String },

    #[error(
        "HEAD and `{branch}` share no history in {}, so there is no merge base to compare against; compare against `{branch}` directly instead",
        repo.display()
    )]
    NoMergeBase { repo: PathBuf, branch: String },

    #[error(
        "the comparison for {} mixes a head or worktree paths with other levels ({reason}); leave upto and untracked unset when sending a head or worktree paths",
        repo.display()
    )]
    ConflictingCompare { repo: PathBuf, reason: String },

    #[error(
        "the repository in {} has no commits yet, so there is nothing to compare commits against; compare uncommitted changes only, or make a first commit",
        repo.display()
    )]
    NoCommits { repo: PathBuf },

    #[error(
        "review {n} of session {session} isn't kept in {} any more (only the last 20 and pinned ones are); pick a newer review",
        repo.display()
    )]
    SnapshotMissing {
        repo: PathBuf,
        session: String,
        n: u32,
    },

    #[error(
        "session {session} has no review of {} yet, so there is nothing to compare since; press Mark reviewed first",
        repo.display()
    )]
    NoSnapshot { repo: PathBuf, session: String },

    #[error(
        "this session isn't a fork, so it has no fork point to compare against; pick another base"
    )]
    NotAFork,

    #[error(
        "{session:?} can't be used in a git ref name; session ids are letters, digits, `-`, `_` and `.`, and don't start with `.`"
    )]
    BadSession { session: String },

    #[error(
        "couldn't read the commit session {session}'s clone is on, in {} ({status}): {stderr}. Check that the session's clone of this repo exists and has a commit; its uncommitted changes aren't compared",
        from.display()
    )]
    OtherSession {
        session: String,
        from: PathBuf,
        status: ExitStatus,
        stderr: String,
    },

    #[error(
        "the saved looks of session {session} in {} can't be read ({what}); the file ticks are lost, press Looks good again to mark them. The repository is untouched",
        repo.display()
    )]
    BadLooks {
        repo: PathBuf,
        session: String,
        what: String,
    },

    #[error("the diff spec has no repo named {repo:?}; the session's repos are {known:?}")]
    UnknownRepo { repo: String, known: Vec<String> },

    #[error(
        "{} has no default branch to compare with: the clone doesn't know which branch its remote calls default, and there is no `origin/main`. Use Find the default branch, which asks the remote, or compare against a branch you pick",
        repo.display()
    )]
    NoDefaultBranch { repo: PathBuf },

    #[error(
        "couldn't {what} in {}; check that the folder is writable and the disk has free space. The repository's own index and refs are untouched",
        path.display()
    )]
    Io {
        what: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "git printed something in {} that couldn't be read ({what}); this is a bug in Humata, please report it with the repository's git version",
        repo.display()
    )]
    BadOutput { repo: PathBuf, what: String },

    #[error(
        "a blocking git task stopped before it finished; humatad2 is probably shutting down. Try again once it is running"
    )]
    Task {
        #[source]
        source: tokio::task::JoinError,
    },
}

impl DiffError {
    pub(crate) fn io(what: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            what,
            path: path.into(),
            source,
        }
    }
}

/// Why an agent's review plan (`review_prepare`) doesn't fit its diff.
///
/// Each says which entry is wrong and how the agent can fix it, since the
/// agent reads these as tool errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error(
        "{repo}/{path} is in the order but not in the diff the spec gives; drop it from the order, or widen the spec (base, untracked files, path filters) so it shows"
    )]
    NotInDiff { repo: String, path: String },

    #[error("{repo}/{path} is in the order twice; list each file once")]
    Duplicate { repo: String, path: String },

    #[error(
        "group {title:?} starts at {repo}/{path}, which isn't in the order; start each group at a file listed in `order`"
    )]
    GroupNotInOrder {
        title: String,
        repo: String,
        path: String,
    },

    #[error(
        "a note is on {repo}/{path}, which isn't in the diff the spec gives; put notes only on files the review shows"
    )]
    NoteNotInDiff { repo: String, path: String },

    #[error(
        "a note is on lines {first}–{last} of {repo}/{path}, but the {side} side has {lines} lines; use line numbers from the {side} side of the diff"
    )]
    NoteOutOfRange {
        repo: String,
        path: String,
        first: u32,
        last: u32,
        side: &'static str,
        lines: usize,
    },

    #[error(
        "a hunk note is on line {line} of {repo}/{path}, which no hunk of the diff covers; give a line inside the hunk, as the diff numbers it"
    )]
    NoHunkAt {
        repo: String,
        path: String,
        line: u32,
    },
}
