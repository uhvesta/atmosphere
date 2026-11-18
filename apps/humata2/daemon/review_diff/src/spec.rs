//! The diff spec (RV-31): per repo, what a review shows. Every base of the
//! Diff panel's picker (RV-1) is one, `review_prepare` carries one, and the
//! skills write one, so an agent's review is data the user can see and
//! edit (D55).

/// What a review shows, per repo of the session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffSpec {
    pub repos: Vec<RepoSpec>,
}

/// One repo's part of a [`DiffSpec`]: the old side ([`Base`]) and the new
/// side ([`Compare`]), which paths, and how lines are matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSpec {
    /// The repo's name as the session knows it, such as `api`.
    pub repo: String,
    pub base: Base,
    pub compare: Compare,
    pub paths: PathFilter,
    pub whitespace: Whitespace,
    /// Whether a moved file shows as one rename (git's `-M`) or as a
    /// deletion and an addition.
    pub renames: bool,
}

/// The old side of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base {
    /// The merge base of HEAD and a branch, as GitHub shows a pull request
    /// (`origin/main...HEAD`).
    MergeBase { with: String },
    /// A branch, tag or commit as it is (`git diff <rev>`).
    Rev { rev: String },
    /// A reviewed snapshot of this session (RV-28): `Some(n)` for review
    /// `n`, `None` for the latest.
    Snapshot { n: Option<u32> },
    /// Where this session was forked from its parent; the caller supplies
    /// the commit in [`BaseContext::fork_point`].
    ForkPoint,
    /// The index (what is staged), for "only my unstaged changes".
    Index,
}

/// The new side of a diff: how much of the work in progress counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compare {
    pub upto: Upto,
    pub untracked: Untracked,
}

/// Commits only, plus what is staged, or plus unstaged changes too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Upto {
    /// HEAD's commit.
    Commits,
    /// HEAD plus what is staged: the index.
    Staged,
    /// Every tracked file as it is on disk.
    Unstaged,
}

/// Which files git doesn't track yet count as added. Ignored files never
/// do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Untracked {
    None,
    All,
    /// Only those under these git pathspecs (`migrations/`, `*.sql`).
    Paths(Vec<String>),
}

/// Which paths the review shows, as git pathspecs relative to the repo
/// root. Empty `include` means every path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathFilter {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Whether whitespace changes count as changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Whitespace {
    #[default]
    Counts,
    /// Changes in the amount of whitespace don't count (`-b`).
    IgnoreChanges,
    /// No whitespace counts (`-w`).
    IgnoreAll,
}

/// What the daemon knows about the session that a spec refers to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseContext {
    /// The session's id, which names its reviewed snapshots.
    pub session: String,
    /// The commit this repo's clone was forked from, for a fork.
    pub fork_point: Option<String>,
}

impl RepoSpec {
    /// A spec with git's defaults: renames detected, whitespace counts,
    /// every path.
    #[must_use]
    pub fn new(repo: impl Into<String>, base: Base, compare: Compare) -> Self {
        Self {
            repo: repo.into(),
            base,
            compare,
            paths: PathFilter::default(),
            whitespace: Whitespace::Counts,
            renames: true,
        }
    }

    /// RV-1 "origin/main": everything since the branch left `branch`,
    /// commits, uncommitted and new files, as a pull request would show it.
    #[must_use]
    pub fn since_branch(repo: impl Into<String>, branch: impl Into<String>) -> Self {
        Self::new(
            repo,
            Base::MergeBase {
                with: branch.into(),
            },
            Compare::everything(),
        )
    }

    /// RV-1 "Uncommitted only": staged, unstaged and new files against HEAD.
    #[must_use]
    pub fn uncommitted(repo: impl Into<String>) -> Self {
        Self::new(
            repo,
            Base::Rev {
                rev: "HEAD".to_owned(),
            },
            Compare::everything(),
        )
    }

    /// RV-1 "Since I last reviewed": against the latest snapshot.
    #[must_use]
    pub fn since_last_review(repo: impl Into<String>) -> Self {
        Self::new(repo, Base::Snapshot { n: None }, Compare::everything())
    }

    /// RV-1 "Since the fork from its parent".
    #[must_use]
    pub fn since_fork(repo: impl Into<String>) -> Self {
        Self::new(repo, Base::ForkPoint, Compare::everything())
    }
}

impl Compare {
    /// Commits, staged, unstaged and every untracked file.
    #[must_use]
    pub fn everything() -> Self {
        Self {
            upto: Upto::Unstaged,
            untracked: Untracked::All,
        }
    }

    /// Committed work only.
    #[must_use]
    pub fn commits() -> Self {
        Self {
            upto: Upto::Commits,
            untracked: Untracked::None,
        }
    }
}

impl PathFilter {
    /// The pathspecs git takes after `--`: the includes, then each exclude
    /// as `:(exclude)`. Empty when every path counts.
    pub(crate) fn pathspecs(&self) -> Vec<String> {
        let mut specs = self.include.clone();
        if specs.is_empty() && !self.exclude.is_empty() {
            specs.push(".".to_owned());
        }
        specs.extend(self.exclude.iter().map(|path| format!(":(exclude){path}")));
        specs
    }
}

/// The default preset's part for one repo (RV-1 "origin/main", B5): everything
/// since the repo's default branch, `default_base`, as a pull request would
/// show it. Every spec that means "the default" for a repo is built here.
pub(crate) fn default_repo_spec(repo: &str, default_base: &str) -> RepoSpec {
    RepoSpec::since_branch(repo, default_base)
}

/// Whether `session` is safe as one component of a ref name.
pub(crate) fn valid_session(session: &str) -> bool {
    !session.is_empty()
        && !session.starts_with('.')
        && !session.ends_with(".lock")
        && !session.contains("..")
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
