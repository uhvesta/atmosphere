//! One clone on this machine, and the two trees a spec compares.
//!
//! The new side is always a tree object. For anything beyond commits it is
//! written from a private index: a copy of the user's index (so git's stat
//! cache spares re-reading unchanged files) in a temporary folder inside
//! the git dir, updated with `git add -u` and the untracked files the spec
//! names, then `git write-tree`. The user's index is never written (D66).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::DiffError;
use crate::git::{Git, trimmed};
use crate::model::{RepoDiff, Trees};
use crate::spec::{Base, BaseContext, Compare, PathFilter, RepoSpec, Untracked, Upto};

/// A clone of one repo on this machine.
#[derive(Clone, Debug)]
pub struct Checkout {
    pub(crate) git: Git,
    git_dir: PathBuf,
    index: PathBuf,
}

/// The files to watch for one clone (TR-12): HEAD, the index, the current
/// branch's ref and `packed-refs`. The working tree is read on demand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchSet {
    pub paths: Vec<PathBuf>,
}

/// The watched files' sizes and modification times. Two equal stamps mean
/// HEAD, the branch and the index haven't moved; working-tree edits don't
/// show here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp(Vec<Option<(u64, SystemTime)>>);

impl Checkout {
    /// Opens the clone whose working tree contains `path`, with `git` from
    /// PATH.
    ///
    /// # Errors
    ///
    /// [`DiffError::NotARepository`] when `path` isn't inside a working
    /// tree; [`DiffError::GitMissing`] when git can't be run.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DiffError> {
        Self::open_with_git(path, "git")
    }

    /// As [`Self::open`], running `program` as git.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn open_with_git(
        path: impl AsRef<Path>,
        program: impl Into<PathBuf>,
    ) -> Result<Self, DiffError> {
        let path = path.as_ref();
        let probe = Git::new(program.into(), path.to_owned());
        let output = probe.output(
            [
                "rev-parse",
                "--show-toplevel",
                "--absolute-git-dir",
                "--git-path",
                "index",
            ],
            None,
        )?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        let (true, [top, git_dir, index]) = (output.status.success(), lines.as_slice()) else {
            return Err(DiffError::NotARepository {
                path: path.to_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        };
        let top = PathBuf::from(top);
        let index = absolute(path, index);
        Ok(Self {
            git: Git::new(probe.program().to_owned(), top),
            git_dir: PathBuf::from(git_dir),
            index,
        })
    }

    /// The working tree's root.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.git.workdir()
    }

    /// The trees `spec` compares, as of now.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when a base doesn't resolve or git fails.
    pub fn trees(&self, spec: &RepoSpec, context: &BaseContext) -> Result<Trees, DiffError> {
        Ok(Trees {
            old: self.base_tree(&spec.base, context)?,
            new: self.new_tree(&spec.compare, &spec.paths)?,
        })
    }

    /// The branch HEAD is on, or `None` when detached.
    pub(crate) fn branch(&self) -> Result<Option<String>, DiffError> {
        let output = self.git.output(["symbolic-ref", "-q", "--short", "HEAD"], None)?;
        Ok(output
            .status
            .success()
            .then(|| trimmed(&output.stdout))
            .filter(|branch| !branch.is_empty()))
    }

    pub(crate) fn head(&self) -> Result<Option<String>, DiffError> {
        self.git.resolve("HEAD^{commit}")
    }

    fn base_tree(&self, base: &Base, context: &BaseContext) -> Result<String, DiffError> {
        match base {
            Base::MergeBase { with } => {
                let head = self.head()?.ok_or_else(|| DiffError::NoCommits {
                    repo: self.path().to_owned(),
                })?;
                let other = self.commit(with)?;
                let output = self.git.output(["merge-base", &head, &other], None)?;
                let base = trimmed(&output.stdout);
                if !output.status.success() || base.is_empty() {
                    return Err(DiffError::NoMergeBase {
                        repo: self.path().to_owned(),
                        branch: with.clone(),
                    });
                }
                self.tree_of(&base)
            }
            Base::Rev { rev } => {
                if rev == "HEAD" && self.head()?.is_none() {
                    return self.empty_tree();
                }
                self.tree_of(rev)
            }
            Base::Snapshot { n: None } if self.never_reviewed(context)? => {
                self.base_tree(&self.unreviewed_base()?, context)
            }
            Base::Snapshot { n } => {
                let snapshot = self.snapshot_ref(&context.session, *n)?;
                self.tree_of(&snapshot)
            }
            Base::ForkPoint => {
                let commit = context.fork_point.as_deref().ok_or(DiffError::NotAFork)?;
                self.tree_of(commit)
            }
            Base::Index => {
                let scratch = self.scratch_index(Seed::UserIndex)?;
                scratch.write_tree()
            }
        }
    }

    fn new_tree(&self, compare: &Compare, paths: &PathFilter) -> Result<String, DiffError> {
        if compare.head.is_some() || !compare.worktree_paths.is_empty() {
            return self.pinned_tree(compare);
        }
        if compare.upto == Upto::Commits && compare.untracked == Untracked::None {
            return match self.head()? {
                Some(head) => self.tree_of(&head),
                None => self.empty_tree(),
            };
        }
        // Clean-tree fast path: when HEAD exists and the working tree / index
        // has no changes relevant to `compare`, skip creating a scratch index.
        // Unborn HEAD (`self.head()? == None`) falls through to the normal path.
        if let Some(head) = self.head()?
            && self.is_clean_for(compare)?
        {
            return self.tree_of(&head);
        }
        let seed = if compare.upto == Upto::Commits {
            Seed::Head
        } else {
            Seed::UserIndex
        };
        let scratch = self.scratch_index(seed)?;
        if compare.upto == Upto::Unstaged {
            // Every tracked file: a pathspec that matches none of them makes
            // `git add` fail, and the listing filters paths anyway. The
            // stat cache keeps unchanged files cheap.
            scratch.git.run(["add", "-u"])?;
        }
        let untracked_specs = match &compare.untracked {
            Untracked::None => None,
            Untracked::All => Some(paths.pathspecs()),
            Untracked::Paths(only) => Some(only.clone()),
        };
        if let Some(specs) = untracked_specs {
            let mut args: Vec<String> = ["ls-files", "-z", "--others", "--exclude-standard"]
                .map(str::to_owned)
                .to_vec();
            if !specs.is_empty() {
                args.push("--".to_owned());
                args.extend(specs);
            }
            // Untracked means not in the user's index, whatever the new
            // side starts from: a staged file isn't untracked.
            let listed = self.git.run(&args)?;
            if !listed.is_empty() {
                // `--remove` skips a listed path that vanished since the
                // listing (a file deleted mid-snapshot), not fails on it.
                scratch.git.run_with_input(
                    ["update-index", "--add", "--remove", "-z", "--stdin"],
                    &listed,
                )?;
            }
        }
        scratch.write_tree()
    }

    /// Whether the repo has no changes that `compare` would include beyond `HEAD`.
    fn is_clean_for(&self, compare: &Compare) -> Result<bool, DiffError> {
        let untracked_flag = match compare.untracked {
            Untracked::None => "--untracked-files=no",
            Untracked::All | Untracked::Paths(_) => "--untracked-files=normal",
        };
        let output = self
            .git
            .output(["status", "--porcelain=v1", "-z", untracked_flag], None)?;
        if !output.status.success() {
            return Ok(false);
        }
        if output.stdout.is_empty() {
            return Ok(true);
        }
        if compare.upto == Upto::Staged && compare.untracked == Untracked::None {
            // Only staged changes (X != ' ') affect Upto::Staged with Untracked::None.
            // Unstaged-only entries have X == ' ' and a single NUL-terminated path.
            let mut entries = output.stdout.split(|&b| b == 0).filter(|s| !s.is_empty());
            return Ok(entries.all(|entry| entry.len() >= 3 && entry[0] == b' ' && entry[2] == b' '));
        }
        Ok(false)
    }

    /// The new side for a compare with a head or worktree paths: the head's
    /// tree (HEAD's without one), with the working-tree state under the
    /// paths laid over it in a private index.
    fn pinned_tree(&self, compare: &Compare) -> Result<String, DiffError> {
        if compare.upto != Upto::Commits || compare.untracked != Untracked::None {
            return Err(DiffError::ConflictingCompare {
                repo: self.path().to_owned(),
                reason: format!(
                    "upto {:?}, untracked {:?}",
                    compare.upto, compare.untracked
                ),
            });
        }
        let start = match &compare.head {
            Some(head) => self.tree_of(head)?,
            None => match self.head()? {
                Some(head) => self.tree_of(&head)?,
                None => self.empty_tree()?,
            },
        };
        if compare.worktree_paths.is_empty() {
            return Ok(start);
        }
        let scratch = self.scratch_index(Seed::Tree(start.clone()))?;
        let mut specs = vec!["--".to_owned()];
        specs.extend(compare.worktree_paths.iter().cloned());
        let with = |prefix: &[&str]| -> Vec<String> {
            prefix
                .iter()
                .map(|arg| (*arg).to_owned())
                .chain(specs.iter().cloned())
                .collect()
        };
        // Every file that is tracked (by the user's index or the start
        // tree) or untracked under the paths: `update-index --remove`
        // takes a missing file out and `--add` puts the on-disk content in.
        let mut names = self.git.run(with(&["ls-files", "-z"]))?;
        names.extend(self.git.run(with(&[
            "ls-files",
            "-z",
            "--others",
            "--exclude-standard",
        ]))?);
        let mut in_tree: Vec<String> = ["ls-tree", "-r", "-z", "--name-only"]
            .map(str::to_owned)
            .to_vec();
        in_tree.push(start);
        in_tree.extend(specs.iter().cloned());
        names.extend(self.git.run(&in_tree)?);
        if !names.is_empty() {
            scratch.git.run_with_input(
                ["update-index", "--add", "--remove", "-z", "--stdin"],
                &names,
            )?;
        }
        scratch.write_tree()
    }

    /// `rev` as a commit id.
    pub(crate) fn commit(&self, rev: &str) -> Result<String, DiffError> {
        self.git
            .resolve(&format!("{rev}^{{commit}}"))?
            .ok_or_else(|| DiffError::UnknownRevision {
                repo: self.path().to_owned(),
                rev: rev.to_owned(),
            })
    }

    /// `rev`'s tree id.
    pub(crate) fn tree_of(&self, rev: &str) -> Result<String, DiffError> {
        self.git
            .resolve(&format!("{rev}^{{tree}}"))?
            .ok_or_else(|| DiffError::UnknownRevision {
                repo: self.path().to_owned(),
                rev: rev.to_owned(),
            })
    }

    fn empty_tree(&self) -> Result<String, DiffError> {
        Ok(trimmed(&self.git.run_with_input(["mktree"], b"")?))
    }

    /// A private index in a temporary folder inside the git dir, removed
    /// when dropped.
    fn scratch_index(&self, seed: Seed) -> Result<Scratch, DiffError> {
        let dir = tempfile::Builder::new()
            .prefix("humata-review-")
            .tempdir_in(&self.git_dir)
            .map_err(|source| DiffError::io("make a private index", &self.git_dir, source))?;
        let index = dir.path().join("index");
        let git = self.git.with_index(&index);
        match seed {
            Seed::UserIndex if self.index.exists() => {
                std::fs::copy(&self.index, &index)
                    .map_err(|source| DiffError::io("copy the index", &self.index, source))?;
            }
            Seed::UserIndex => {}
            Seed::Tree(tree) => {
                git.run(["read-tree", tree.as_str()])?;
            }
            Seed::Head => {
                if self.head()?.is_some() {
                    git.run(["read-tree", "HEAD"])?;
                }
            }
        }
        Ok(Scratch { git, _dir: dir })
    }

    /// The files to watch for this clone (TR-12). Ask again after HEAD
    /// changes: a new branch has a new ref file.
    ///
    /// # Errors
    ///
    /// [`DiffError`] when git fails.
    pub fn watch_set(&self) -> Result<WatchSet, DiffError> {
        let mut args = vec![
            "rev-parse".to_owned(),
            "--git-path".to_owned(),
            "HEAD".to_owned(),
            "--git-path".to_owned(),
            "index".to_owned(),
            "--git-path".to_owned(),
            "packed-refs".to_owned(),
        ];
        let symbolic = self.git.output(["symbolic-ref", "-q", "HEAD"], None)?;
        let branch_ref = trimmed(&symbolic.stdout);
        if symbolic.status.success() && !branch_ref.is_empty() {
            args.push("--git-path".to_owned());
            args.push(branch_ref);
        }
        let text = String::from_utf8_lossy(&self.git.run(&args)?).into_owned();
        Ok(WatchSet {
            paths: text.lines().map(|line| absolute(self.path(), line)).collect(),
        })
    }

    /// The watched files' state now, to compare with an earlier stamp.
    #[must_use]
    pub fn stamp(watch: &WatchSet) -> Stamp {
        Stamp(
            watch
                .paths
                .iter()
                .map(|path| {
                    std::fs::metadata(path)
                        .ok()
                        .and_then(|meta| Some((meta.len(), meta.modified().ok()?)))
                })
                .collect(),
        )
    }

    /// A repo's diff summary from trees already computed.
    pub(crate) fn summary(
        &self,
        spec: &RepoSpec,
        context: &BaseContext,
        trees: Trees,
        files: Vec<crate::model::FileSummary>,
    ) -> Result<RepoDiff, DiffError> {
        let label = if spec.base == (Base::Snapshot { n: None }) && self.never_reviewed(context)? {
            let since = RepoSpec {
                base: self.unreviewed_base()?,
                ..spec.clone()
            };
            format!("not reviewed yet: {}", crate::diff::label(&since))
        } else {
            crate::diff::label(spec)
        };
        Ok(RepoDiff {
            repo: spec.repo.clone(),
            branch: self.branch()?,
            head: self.head()?,
            label,
            trees,
            files,
        })
    }
}

enum Seed {
    /// A copy of the user's index, with its stat cache.
    UserIndex,
    /// HEAD's tree, for commits plus untracked files.
    Head,
    /// An arbitrary tree.
    Tree(String),
}

struct Scratch {
    git: Git,
    _dir: tempfile::TempDir,
}

impl Scratch {
    fn write_tree(&self) -> Result<String, DiffError> {
        Ok(trimmed(&self.git.run(["write-tree"])?))
    }
}

pub use humata2_git_head::origin_head;

fn origin_head_in(git: &Git) -> Option<String> {
    humata2_git_head::origin_head_with(git.program(), git.workdir())
}

impl Checkout {
    /// This clone's default branch, which the default preset compares with
    /// (B5): `origin/HEAD`'s branch, or `origin/main` when `origin/HEAD` isn't
    /// set and `origin/main` exists.
    ///
    /// # Errors
    ///
    /// [`DiffError::NoDefaultBranch`] when neither is set; git's errors.
    pub(crate) fn default_base_blocking(&self) -> Result<String, DiffError> {
        if let Some(head) = origin_head_in(&self.git) {
            return Ok(head);
        }
        if self.git.resolve("origin/main^{commit}")?.is_some() {
            return Ok("origin/main".to_owned());
        }
        Err(DiffError::NoDefaultBranch {
            repo: self.path().to_owned(),
        })
    }
}

fn absolute(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

impl Checkout {
    /// Where "Since I last reviewed" starts in a repo never reviewed (D198,
    /// B5): the repo's default branch.
    fn unreviewed_base(&self) -> Result<Base, DiffError> {
        Ok(Base::MergeBase {
            with: self.default_base_blocking()?,
        })
    }

    /// Whether this session has no snapshot of the repo yet and the repo has
    /// a default branch, so "Since I last reviewed" shows everything since
    /// that branch (D198). Without one the refusal stands.
    fn never_reviewed(&self, context: &BaseContext) -> Result<bool, DiffError> {
        if !self.snapshots_blocking(&context.session)?.is_empty() {
            return Ok(false);
        }
        match self.default_base_blocking() {
            Ok(_) => Ok(true),
            Err(DiffError::NoDefaultBranch { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }
}
