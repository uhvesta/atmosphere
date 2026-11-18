//! Runs git in the user's checkout. Every call goes through `Git::run` or
//! `Git::run_with_input`, so failures carry the command and its stderr.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::plan::Commit;
use crate::trailers::Message;

pub(crate) struct Git {
    dir: PathBuf,
}

/// One branch to push: `new` replaces `old` (None: the branch is new).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PushUpdate {
    pub(crate) branch: String,
    pub(crate) new: String,
    pub(crate) old: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GitError {
    #[error("could not run `git {args}`. Install git and make sure it is on PATH")]
    Spawn {
        args: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`git {args}` failed ({status}): {stderr}")]
    Failed {
        args: String,
        status: String,
        stderr: String,
    },

    #[error("could not read the output of `git {args}`: {output}")]
    Parse { args: String, output: String },

    #[error(
        "commit {commit} ({subject}) does not apply onto {onto}: conflicts in {files}. It probably \
         depends on a commit of another topic: add `Relative: <that topic>` to its topic, or \
         reorder the commits (git rebase -i) so it applies on its own"
    )]
    Conflict {
        commit: String,
        subject: String,
        onto: String,
        files: String,
    },

    #[error(
        "commit {commit} is a merge commit; a stack needs a linear branch. Rebase onto the trunk \
         (git rebase <remote>/<trunk>) and run again"
    )]
    MergeCommit { commit: String },

    #[error(
        "git push was refused: {stderr}. If another upload changed these branches since, run \
         upload again; don't force-push the stack branches by hand"
    )]
    Push { stderr: String },
}

impl Git {
    pub(crate) fn new(dir: &Path) -> Git {
        Git {
            dir: dir.to_path_buf(),
        }
    }

    /// `git <args>` in the checkout; its stdout, trimmed of the final newline.
    /// A non-zero exit is `GitError::Failed` with stderr trimmed.
    fn run(&self, args: &[&str]) -> Result<String, GitError> {
        let output = self.output(args, None, &[])?;
        stdout_of(args, output)
    }

    /// `run`, with `input` on stdin and extra environment variables.
    fn run_with_input(&self, args: &[&str], input: &str, env: &[(&str, &str)]) -> Result<String, GitError> {
        let output = self.output(args, Some(input), env)?;
        stdout_of(args, output)
    }

    /// Spawns `git <args>` in the checkout with `LC_ALL=C` and waits for it.
    /// Only a spawn or I/O failure is an error here; the exit status is left
    /// to the caller.
    fn output(&self, args: &[&str], input: Option<&str>, env: &[(&str, &str)]) -> Result<Output, GitError> {
        let spawn_error = |source: std::io::Error| GitError::Spawn {
            args: args.join(" "),
            source,
        };
        let mut command = Command::new("git");
        command
            .current_dir(&self.dir)
            .args(args)
            .env("LC_ALL", "C")
            .envs(env.iter().copied())
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(spawn_error)?;
        if let Some(input) = input
            && let Some(mut stdin) = child.stdin.take()
        {
            match stdin.write_all(input.as_bytes()) {
                Ok(()) => {}
                // git exits without reading its input when it fails early;
                // its exit status says why, so that is what gets reported.
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(error) => return Err(spawn_error(error)),
            }
        }
        child.wait_with_output().map_err(spawn_error)
    }

    /// `git rev-parse --show-toplevel`.
    pub(crate) fn toplevel(&self) -> Result<PathBuf, GitError> {
        self.run(&["rev-parse", "--show-toplevel"]).map(PathBuf::from)
    }

    /// `git rev-parse --verify <rev>^{commit}`: the full sha.
    pub(crate) fn rev_parse(&self, rev: &str) -> Result<String, GitError> {
        let spec = format!("{rev}^{{commit}}");
        let sha = self.run(&["rev-parse", "--verify", "--quiet", spec.as_str()])?;
        Ok(sha.trim().to_string())
    }

    /// Whether git accepts `branch` as a branch name:
    /// `git check-ref-format --branch <branch>` exits 0.
    pub(crate) fn is_valid_branch(&self, branch: &str) -> Result<bool, GitError> {
        let args = ["check-ref-format", "--branch", branch];
        Ok(self.output(&args, None, &[])?.status.success())
    }

    /// `git fetch --quiet <remote> <refspecs...>`.
    pub(crate) fn fetch(&self, remote: &str, refspecs: &[String]) -> Result<(), GitError> {
        let mut args = vec!["fetch", "--quiet", remote];
        args.extend(refspecs.iter().map(String::as_str));
        self.run(&args).map(|_| ())
    }

    /// The trunk name from `git symbolic-ref --short refs/remotes/<remote>/HEAD`
    /// (`origin/main` gives `main`); None when that ref doesn't exist.
    pub(crate) fn remote_default_branch(&self, remote: &str) -> Option<String> {
        let head = format!("refs/remotes/{remote}/HEAD");
        let short = self.run(&["symbolic-ref", "--short", head.as_str()]).ok()?;
        let prefix = format!("{remote}/");
        let branch = short.strip_prefix(prefix.as_str())?;
        (!branch.is_empty()).then(|| branch.to_owned())
    }

    /// `git merge-base <a> <b>`.
    pub(crate) fn merge_base(&self, a: &str, b: &str) -> Result<String, GitError> {
        let base = self.run(&["merge-base", a, b])?;
        Ok(base.trim().to_string())
    }

    /// The commits in `base..head`, oldest first, each with its parent and
    /// parsed message. A merge commit in the range is `GitError::MergeCommit`.
    pub(crate) fn commits(&self, base: &str, head: &str) -> Result<Vec<Commit>, GitError> {
        let range = format!("{base}..{head}");
        let args = ["log", "--reverse", "--format=%H%x00%P%x00%B%x1e", range.as_str()];
        let stdout = self.run(&args)?;
        let mut commits = Vec::new();
        for record in stdout.split('\x1e') {
            if record.trim().is_empty() {
                continue;
            }
            let record = record.trim_start_matches('\n');
            let mut fields = record.splitn(3, '\0');
            let (Some(sha), Some(parents), Some(message)) = (fields.next(), fields.next(), fields.next())
            else {
                return Err(parse_error(&args, record));
            };
            let parents: Vec<&str> = parents.split_whitespace().collect();
            match parents.as_slice() {
                [parent] => commits.push(Commit {
                    sha: sha.to_string(),
                    parent: parent.to_string(),
                    message: Message::parse(message),
                }),
                [_, _, ..] => {
                    return Err(GitError::MergeCommit {
                        commit: short_sha(sha),
                    });
                }
                [] => return Err(parse_error(&args, record)),
            }
        }
        Ok(commits)
    }

    /// Copies `commit` onto `onto` without touching the work tree: the tree
    /// from `git merge-tree --write-tree --merge-base=<commit's parent> <onto> <commit>`,
    /// then `git commit-tree` with the commit's message, author and committer
    /// (name, email and date), so the same input always gives the same sha.
    /// A conflict is `GitError::Conflict` naming the files.
    pub(crate) fn copy_onto(&self, commit: &Commit, onto: &str) -> Result<String, GitError> {
        if onto == commit.parent {
            return Ok(commit.sha.clone());
        }
        let merge_base = format!("--merge-base={}", commit.parent);
        let args = [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            merge_base.as_str(),
            onto,
            commit.sha.as_str(),
        ];
        let merged = self.output(&args, None, &[])?;
        let text = String::from_utf8_lossy(&merged.stdout);
        let mut lines = text.lines();
        let tree = lines.next().unwrap_or_default();
        match merged.status.code() {
            Some(0) if !tree.is_empty() => {}
            Some(1) => {
                let files: Vec<&str> = lines.filter(|line| !line.is_empty()).collect();
                return Err(GitError::Conflict {
                    commit: short_sha(&commit.sha),
                    subject: commit.message.subject.clone(),
                    onto: short_sha(onto),
                    files: files.join(", "),
                });
            }
            Some(0) => return Err(parse_error(&args, &text)),
            _ => return Err(failed(&args, &merged)),
        }

        let log_meta = [
            "log",
            "-1",
            "--date=raw",
            "--format=%an%x00%ae%x00%ad%x00%cn%x00%ce%x00%cd",
            commit.sha.as_str(),
        ];
        let meta = self.run(&log_meta)?;
        let fields: Vec<&str> = meta.split('\0').collect();
        let &[
            author_name,
            author_email,
            author_date,
            committer_name,
            committer_email,
            committer_date,
        ] = fields.as_slice()
        else {
            return Err(parse_error(&log_meta, &meta));
        };
        let message = self.run(&["log", "-1", "--format=%B", commit.sha.as_str()])?;
        let env = [
            ("GIT_AUTHOR_NAME", author_name),
            ("GIT_AUTHOR_EMAIL", author_email),
            ("GIT_AUTHOR_DATE", author_date),
            ("GIT_COMMITTER_NAME", committer_name),
            ("GIT_COMMITTER_EMAIL", committer_email),
            ("GIT_COMMITTER_DATE", committer_date),
        ];
        let sha = self.run_with_input(
            &["commit-tree", "--no-gpg-sign", tree, "-p", onto],
            &message,
            &env,
        )?;
        Ok(sha.trim().to_string())
    }

    /// The branches under `refs/heads/<prefix>` on the remote and their shas,
    /// from `git ls-remote <remote> refs/heads/<prefix>*`. Keys are branch
    /// names without `refs/heads/`.
    pub(crate) fn remote_heads(&self, remote: &str, prefix: &str) -> Result<BTreeMap<String, String>, GitError> {
        let pattern = format!("refs/heads/{prefix}*");
        let args = ["ls-remote", remote, pattern.as_str()];
        let stdout = self.run(&args)?;
        let mut heads = BTreeMap::new();
        for line in stdout.lines().filter(|line| !line.is_empty()) {
            let Some((sha, reference)) = line.split_once('\t') else {
                return Err(parse_error(&args, line));
            };
            let Some(branch) = reference.strip_prefix("refs/heads/") else {
                return Err(parse_error(&args, line));
            };
            if sha.is_empty() || branch.is_empty() {
                return Err(parse_error(&args, line));
            }
            heads.insert(branch.to_string(), sha.to_string());
        }
        Ok(heads)
    }

    /// One `git push --atomic --quiet <remote>` of every update, each with
    /// `--force-with-lease=refs/heads/<branch>:<old or empty>` and refspec
    /// `<new>:refs/heads/<branch>`. A refusal is `GitError::Push`.
    pub(crate) fn push(&self, remote: &str, updates: &[PushUpdate]) -> Result<(), GitError> {
        if updates.is_empty() {
            return Ok(());
        }
        let leases: Vec<String> = updates
            .iter()
            .map(|update| {
                format!(
                    "--force-with-lease=refs/heads/{}:{}",
                    update.branch,
                    update.old.as_deref().unwrap_or("")
                )
            })
            .collect();
        let refspecs: Vec<String> = updates
            .iter()
            .map(|update| format!("{}:refs/heads/{}", update.new, update.branch))
            .collect();
        let mut args: Vec<&str> = vec!["push", "--atomic", "--quiet", remote];
        args.extend(leases.iter().map(String::as_str));
        args.extend(refspecs.iter().map(String::as_str));
        match self.run(&args) {
            Ok(_) => Ok(()),
            Err(GitError::Failed { stderr, .. }) => Err(GitError::Push { stderr }),
            Err(error) => Err(error),
        }
    }

    /// `git diff --shortstat <from> <to>`, trimmed; `0 files changed` when empty.
    pub(crate) fn shortstat(&self, from: &str, to: &str) -> Result<String, GitError> {
        let stat = self.run(&["diff", "--shortstat", from, to])?;
        let stat = stat.trim();
        if stat.is_empty() {
            Ok("0 files changed".to_string())
        } else {
            Ok(stat.to_string())
        }
    }

    /// The stable patch id of `git diff <from> <to>` (`git patch-id --stable`
    /// on that diff); None when the diff is empty.
    pub(crate) fn patch_id(&self, from: &str, to: &str) -> Result<Option<String>, GitError> {
        let diff = self.run(&["diff", from, to])?;
        if diff.is_empty() {
            return Ok(None);
        }
        let input = diff + "\n";
        let stdout = self.run_with_input(&["patch-id", "--stable"], &input, &[])?;
        Ok(stdout.split_whitespace().next().map(str::to_string))
    }

    /// Whether the commit object exists locally (`git cat-file -e <sha>^{commit}`).
    pub(crate) fn has_commit(&self, sha: &str) -> bool {
        let spec = format!("{sha}^{{commit}}");
        self.output(&["cat-file", "-e", spec.as_str()], None, &[])
            .is_ok_and(|output| output.status.success())
    }

    /// Where git keeps `name` inside the git folder (`git rev-parse
    /// --git-path <name>`), as an absolute path.
    pub(crate) fn git_path(&self, name: &str) -> Result<PathBuf, GitError> {
        let path = PathBuf::from(self.run(&["rev-parse", "--git-path", name])?);
        Ok(if path.is_absolute() { path } else { self.dir.join(path) })
    }

    /// The files with unresolved merge conflicts, one path per entry.
    pub(crate) fn unmerged_paths(&self) -> Result<Vec<String>, GitError> {
        let out = self.run(&["diff", "--name-only", "--diff-filter=U"])?;
        Ok(out.lines().filter(|line| !line.is_empty()).map(str::to_owned).collect())
    }

    /// Whether a tracked file differs between the work tree and the index.
    pub(crate) fn has_unstaged_changes(&self) -> Result<bool, GitError> {
        let args = ["diff", "--quiet"];
        let output = self.output(&args, None, &[])?;
        match output.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(failed(&args, &output)),
        }
    }

    /// Whether any tracked file is changed, staged or not (untracked files
    /// don't count).
    pub(crate) fn has_uncommitted_changes(&self) -> Result<bool, GitError> {
        let out = self.run(&["status", "--porcelain", "--untracked-files=no"])?;
        Ok(!out.trim().is_empty())
    }

    /// The checked-out branch, or None when HEAD is detached.
    pub(crate) fn current_branch(&self) -> Result<Option<String>, GitError> {
        let args = ["symbolic-ref", "--quiet", "--short", "HEAD"];
        let output = self.output(&args, None, &[])?;
        match output.status.code() {
            Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())),
            Some(1) => Ok(None),
            _ => Err(failed(&args, &output)),
        }
    }

    /// Whether any rebase (interactive or not) is in progress.
    pub(crate) fn rebase_in_progress(&self) -> Result<bool, GitError> {
        Ok(self.git_path("rebase-merge")?.is_dir() || self.git_path("rebase-apply")?.is_dir())
    }

    /// `git rebase <args>` with the user's git config out of the way: no
    /// editor opens, no autosquash, ref updates or abbreviation, and the
    /// todo list is not checked. `sequence_editor` is the shell command that
    /// replaces the todo list (`true` leaves it as it is). Only a spawn
    /// failure is an error; the exit status is left to the caller.
    pub(crate) fn rebase(&self, args: &[&str], sequence_editor: Option<&str>) -> Result<Output, GitError> {
        let editor = sequence_editor.unwrap_or("true");
        let sequence = format!("sequence.editor={editor}");
        let mut full: Vec<&str> = Vec::new();
        for setting in [
            "core.editor=true",
            "rebase.autoSquash=false",
            "rebase.autoStash=false",
            "rebase.updateRefs=false",
            "rebase.abbreviateCommands=false",
            "rebase.missingCommitsCheck=ignore",
            "rebase.rescheduleFailedExec=false",
        ] {
            full.push("-c");
            full.push(setting);
        }
        full.push("-c");
        full.push(sequence.as_str());
        full.push("rebase");
        full.extend(args.iter().copied());
        self.output(&full, None, &[("GIT_EDITOR", "true"), ("GIT_SEQUENCE_EDITOR", editor)])
    }

    /// The refs matching `pattern` (such as `refs/stack/alice/codec/*`) on
    /// the remote and their shas, from `git ls-remote`. Keys are full ref
    /// names.
    pub(crate) fn remote_refs(&self, remote: &str, pattern: &str) -> Result<BTreeMap<String, String>, GitError> {
        let args = ["ls-remote", remote, pattern];
        let stdout = self.run(&args)?;
        let mut refs = BTreeMap::new();
        for line in stdout.lines().filter(|line| !line.is_empty()) {
            let Some((sha, reference)) = line.split_once('\t') else {
                return Err(parse_error(&args, line));
            };
            refs.insert(reference.to_string(), sha.to_string());
        }
        Ok(refs)
    }

    /// One `git push --atomic --quiet <remote>` that sets each full ref to
    /// its sha, or deletes it when the sha is None. Pins are written only by
    /// stack and never move, so no lease is needed.
    pub(crate) fn push_refs(&self, remote: &str, refs: &[(String, Option<String>)]) -> Result<(), GitError> {
        if refs.is_empty() {
            return Ok(());
        }
        let refspecs: Vec<String> = refs
            .iter()
            .map(|(name, sha)| match sha {
                Some(sha) => format!("+{sha}:{name}"),
                None => format!(":{name}"),
            })
            .collect();
        let mut args: Vec<&str> = vec!["push", "--atomic", "--quiet", remote];
        args.extend(refspecs.iter().map(String::as_str));
        match self.run(&args) {
            Ok(_) => Ok(()),
            Err(GitError::Failed { stderr, .. }) => Err(GitError::Push { stderr }),
            Err(error) => Err(error),
        }
    }

    /// `git range-diff <old_base>..<old_head> <new_base>..<new_head>`, with
    /// no colour.
    pub(crate) fn range_diff(&self, old: (&str, &str), new: (&str, &str)) -> Result<String, GitError> {
        let old_range = format!("{}..{}", old.0, old.1);
        let new_range = format!("{}..{}", new.0, new.1);
        // Both ranges are one topic's commits, so pair them even when a commit
        // changed most of its patch: at the default creation factor (60%) a
        // small commit that was rewritten counts as one deleted and one new,
        // and range-diff prints no diff between them.
        self.run(&[
            "range-diff",
            "--no-color",
            "--creation-factor=1000",
            old_range.as_str(),
            new_range.as_str(),
        ])
    }
}

/// The stdout of a finished git call, without its final newline; `Failed`
/// when git exited non-zero.
fn stdout_of(args: &[&str], output: Output) -> Result<String, GitError> {
    if !output.status.success() {
        return Err(failed(args, &output));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text: &str = &stdout;
    Ok(text.strip_suffix('\n').unwrap_or(text).to_string())
}

/// `GitError::Failed` for a git call that exited non-zero.
fn failed(args: &[&str], output: &Output) -> GitError {
    GitError::Failed {
        args: args.join(" "),
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    }
}

/// `GitError::Parse` for output that doesn't have the expected shape.
fn parse_error(args: &[&str], output: &str) -> GitError {
    GitError::Parse {
        args: args.join(" "),
        output: output.to_string(),
    }
}

/// The first 8 characters of a sha, as used in user-facing messages.
fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;

    /// A work repo on `main` and a bare remote beside it, in a temp dir. Git
    /// runs here with no system or global config, so results don't depend on
    /// the machine running the tests.
    struct Fixture {
        root: tempfile::TempDir,
        repo: PathBuf,
        remote: PathBuf,
    }

    impl Fixture {
        fn new() -> Fixture {
            let root = tempfile::tempdir().expect("create a temp dir");
            let repo = root.path().join("work");
            let remote = root.path().join("remote.git");
            fs::create_dir(&repo).expect("create the work dir");
            fs::create_dir(&remote).expect("create the remote dir");
            fs::write(root.path().join("gitconfig"), "").expect("write an empty global config");
            let fixture = Fixture { root, repo, remote };
            fixture.git_in(&fixture.remote, &["init", "--quiet", "--bare"]);
            fixture.git(&["init", "--quiet", "--initial-branch=main"]);
            fixture.git(&["config", "user.name", "T"]);
            fixture.git(&["config", "user.email", "t@e"]);
            fixture.git(&["config", "commit.gpgsign", "false"]);
            fixture
        }

        /// Runs git in `dir` with a fixed identity and fixed dates, asserts it
        /// succeeded, and returns its stdout without the trailing newline.
        fn git_in(&self, dir: &Path, args: &[&str]) -> String {
            let output = Command::new("git")
                .current_dir(dir)
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.root.path().join("gitconfig"))
                .env("GIT_AUTHOR_NAME", "T")
                .env("GIT_AUTHOR_EMAIL", "t@e")
                .env("GIT_COMMITTER_NAME", "T")
                .env("GIT_COMMITTER_EMAIL", "t@e")
                .env("GIT_AUTHOR_DATE", "1700000000 +0000")
                .env("GIT_COMMITTER_DATE", "1700000000 +0000")
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim_end().to_string()
        }

        /// Runs git in the work repo.
        fn git(&self, args: &[&str]) -> String {
            self.git_in(&self.repo, args)
        }

        fn git_repo(&self) -> Git {
            Git::new(&self.repo)
        }

        /// Writes `name`, commits it with `subject`, and returns the new HEAD sha.
        fn commit_file(&self, name: &str, contents: &str, subject: &str) -> String {
            fs::write(self.repo.join(name), contents).expect("write the file");
            self.git(&["add", name]);
            self.git(&["commit", "--quiet", "-m", subject]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    /// The single commit in `commits`, or a panic naming the wrong count.
    fn only(mut commits: Vec<Commit>) -> Commit {
        assert_eq!(commits.len(), 1, "expected one commit");
        commits.remove(0)
    }

    #[test]
    fn commits_are_listed_oldest_first_with_parents_and_subjects() {
        let fixture = Fixture::new();
        let base = fixture.commit_file("a.txt", "one\n", "Add a");
        let first = fixture.commit_file("b.txt", "two\n", "Add b");
        let second = fixture.commit_file("c.txt", "three\n", "Add c");

        let commits = fixture.git_repo().commits(&base, &second).expect("list the commits");

        let summary: Vec<(&str, &str, &str)> = commits
            .iter()
            .map(|commit| {
                (
                    commit.sha.as_str(),
                    commit.parent.as_str(),
                    commit.message.subject.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (first.as_str(), base.as_str(), "Add b"),
                (second.as_str(), first.as_str(), "Add c"),
            ]
        );
    }

    #[test]
    fn commits_over_a_range_with_a_merge_commit_is_an_error() {
        let fixture = Fixture::new();
        let base = fixture.commit_file("a.txt", "one\n", "Add a");
        fixture.git(&["checkout", "--quiet", "-b", "feature"]);
        fixture.commit_file("b.txt", "two\n", "Add b");
        fixture.git(&["checkout", "--quiet", "main"]);
        fixture.commit_file("c.txt", "three\n", "Add c");
        fixture.git(&["merge", "--quiet", "--no-ff", "-m", "Merge feature", "feature"]);
        let head = fixture.git(&["rev-parse", "HEAD"]);

        let error = fixture
            .git_repo()
            .commits(&base, &head)
            .err()
            .expect("a merge commit is refused");

        assert!(matches!(error, GitError::MergeCommit { .. }));
    }

    #[test]
    fn copy_onto_applies_a_commit_on_another_branch_with_the_same_sha_each_time() {
        let fixture = Fixture::new();
        let base = fixture.commit_file("a.txt", "one\n", "Add a");
        fixture.git(&["checkout", "--quiet", "-b", "other"]);
        let onto = fixture.commit_file("c.txt", "three\n", "Add c");
        fixture.git(&["checkout", "--quiet", "-b", "topic", base.as_str()]);
        let topic = fixture.commit_file("b.txt", "two\n", "Add b");
        let repo = fixture.git_repo();
        let commit = only(repo.commits(&base, &topic).expect("list the commit"));

        let copied = repo.copy_onto(&commit, &onto).expect("copy the commit");
        let again = repo.copy_onto(&commit, &onto).expect("copy the commit again");

        assert_eq!(copied, again);
        assert_eq!(fixture.git(&["log", "-1", "--format=%P", copied.as_str()]), onto);
        assert_eq!(fixture.git(&["log", "-1", "--format=%s", copied.as_str()]), "Add b");
        let tree = fixture.git(&["ls-tree", "--name-only", copied.as_str()]);
        assert_eq!(tree.lines().collect::<Vec<_>>(), ["a.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn copy_onto_the_commits_own_parent_keeps_its_sha() {
        let fixture = Fixture::new();
        let base = fixture.commit_file("a.txt", "one\n", "Add a");
        let topic = fixture.commit_file("b.txt", "two\n", "Add b");
        let repo = fixture.git_repo();
        let commit = only(repo.commits(&base, &topic).expect("list the commit"));

        let copied = repo.copy_onto(&commit, &base).expect("copy onto its parent");

        assert_eq!(copied, topic);
    }

    #[test]
    fn copy_onto_a_commit_that_edits_a_line_another_branch_edited_names_the_file() {
        let fixture = Fixture::new();
        let base = fixture.commit_file("a.txt", "one\n", "Add a");
        fixture.git(&["checkout", "--quiet", "-b", "topic", base.as_str()]);
        let topic = fixture.commit_file("a.txt", "topic\n", "Edit a");
        fixture.git(&["checkout", "--quiet", "-b", "other", base.as_str()]);
        let onto = fixture.commit_file("a.txt", "other\n", "Edit a elsewhere");
        let repo = fixture.git_repo();
        let commit = only(repo.commits(&base, &topic).expect("list the commit"));

        let error = repo
            .copy_onto(&commit, &onto)
            .err()
            .expect("the copy conflicts");

        assert!(matches!(error, GitError::Conflict { ref files, .. } if files == "a.txt"));
    }

    #[test]
    fn push_creates_a_remote_branch_and_refuses_a_stale_lease() {
        let fixture = Fixture::new();
        let remote = fixture.remote.to_str().expect("the temp path is UTF-8");
        fixture.git(&["remote", "add", "origin", remote]);
        let first = fixture.commit_file("a.txt", "one\n", "Add a");
        let second = fixture.commit_file("b.txt", "two\n", "Add b");
        let repo = fixture.git_repo();
        let update = |new: &str, old: Option<&str>| PushUpdate {
            branch: "u/stack/one".to_string(),
            new: new.to_string(),
            old: old.map(str::to_string),
        };

        repo.push("origin", &[update(first.as_str(), None)])
            .expect("the first push creates the branch");
        let heads = repo.remote_heads("origin", "u/stack/").expect("list the remote heads");
        assert_eq!(heads.get("u/stack/one"), Some(&first));

        // The remote has `first`, so a lease that expects `second` is stale.
        let error = repo
            .push("origin", &[update(second.as_str(), Some(second.as_str()))])
            .err()
            .expect("a stale lease is refused");
        assert!(matches!(error, GitError::Push { .. }));
    }

    #[test]
    fn shortstat_and_patch_id_of_an_empty_diff_are_empty() {
        let fixture = Fixture::new();
        fixture.commit_file("a.txt", "one\n", "Add a");
        let head = fixture.commit_file("a.txt", "two\n", "Edit a");
        let repo = fixture.git_repo();

        assert_eq!(
            repo.shortstat(&head, &head).expect("shortstat"),
            "0 files changed"
        );
        assert_eq!(repo.patch_id(&head, &head).expect("patch id"), None);
    }

    #[test]
    fn shortstat_and_patch_id_of_a_changed_range_are_set() {
        let fixture = Fixture::new();
        let base = fixture.commit_file("a.txt", "one\n", "Add a");
        let head = fixture.commit_file("a.txt", "two\n", "Edit a");
        let repo = fixture.git_repo();

        assert_ne!(
            repo.shortstat(&base, &head).expect("shortstat"),
            "0 files changed"
        );
        assert!(repo.patch_id(&base, &head).expect("patch id").is_some());
    }

    #[test]
    fn has_commit_knows_the_head_and_not_the_null_sha() {
        let fixture = Fixture::new();
        let head = fixture.commit_file("a.txt", "one\n", "Add a");
        let repo = fixture.git_repo();

        assert!(repo.has_commit(&head));
        assert!(!repo.has_commit("0000000000000000000000000000000000000000"));
    }
}
