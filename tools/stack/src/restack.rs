//! `stack restack`: stop at an earlier topic or commit to amend it, then
//! replay the rest of the stack onto the result. It drives `git rebase -i`,
//! so git owns the conflict state, crash safety and resuming; stack only
//! writes the todo list and a small state file next to it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::cli::{EditArgs, ReplayArgs, RestackCommand};
use crate::config::{Config, ConfigError};
use crate::git::{Git, GitError};
use crate::plan::{Plan, PlanError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum RestackError {
    #[error(transparent)]
    Git(#[from] GitError),

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Plan(#[from] PlanError),

    #[error(
        "the working tree has uncommitted changes to tracked files, and a restack rewrites \
         commits under them. Commit or stash them first; nothing was changed. Next: `git status`"
    )]
    Dirty,

    #[error(
        "a git rebase that is not a stack restack is in progress. Finish or cancel it first; \
         nothing was changed. Next: `git rebase --continue` (or `git rebase --abort`)"
    )]
    GitRebaseInProgress,

    #[error(
        "a restack is already in progress (stopped at {sha} \"{subject}\"). Finish it before \
         starting another; nothing was changed. Next: `stack restack continue` (or `stack \
         restack abort`)"
    )]
    AlreadyInProgress { sha: String, subject: String },

    #[error(
        "`stack restack {verb}` needs a restack in progress, and none is. Nothing was changed. \
         Next: `stack restack edit --topic <topic>`"
    )]
    NoRestack { verb: &'static str },

    #[error(
        "there is no commit with a Topic between the trunk and HEAD, so there is nothing to \
         restack. Next: `stack check`"
    )]
    EmptyStack,

    #[error(
        "`{target}` is not in this stack; the topics are: {topics}. Name one of them, or a commit \
         of the stack. Next: `stack check`"
    )]
    UnknownTarget { target: String, topics: String },

    #[error("`stack restack edit` needs exactly one of --topic or --commit. Next: `stack restack edit --help`")]
    NoTarget,

    #[error(
        "--exec commands must be one line, because each becomes one line of the rebase todo list. \
         Put the steps in a script file and pass that. Next: `stack restack edit --help`"
    )]
    BadExec,

    #[error(
        "unresolved conflicts remain in: {files}. Resolve each file and `git add` it. Next: `git \
         add {files}` then `stack restack continue`"
    )]
    Unmerged { files: String },

    #[error(
        "tracked files have changes that are not staged, and continuing would lose or refuse \
         them. Commit them into the stopped commit, or stage them. Next: `git commit --amend -a` \
         (or `git add` the files), then `stack restack continue`"
    )]
    Unstaged,

    #[error(
        "`stack restack skip` drops a commit that conflicts, and the restack is not stopped on a \
         conflict (it is stopped at {sha} \"{subject}\"). Next: `stack restack continue`"
    )]
    SkipWithoutConflict { sha: String, subject: String },

    #[error(
        "restack stopped on a merge conflict in topic {topic}, at commit {sha} \"{subject}\". \
         Conflicting files: {files}. Resolve them and `git add` each one; to drop this commit run \
         `stack restack skip`, to cancel everything `stack restack abort`. Next: `stack restack \
         continue`"
    )]
    Conflict {
        topic: String,
        sha: String,
        subject: String,
        files: String,
    },

    #[error(
        "--exec command failed at topic {topic}: {command}. The restack is stopped after {sha} \
         \"{subject}\"; later topics are not replayed yet. Fix it and commit any change it \
         needs. Next: `stack restack continue`"
    )]
    ExecFailed {
        topic: String,
        command: String,
        sha: String,
        subject: String,
    },

    #[error(
        "cannot upload while a restack is in progress (stopped at {sha} \"{subject}\" in topic \
         {topic}); uploading now would push partly replayed branches. Next: `stack restack \
         continue` (or `stack restack abort`)"
    )]
    InProgress {
        sha: String,
        subject: String,
        topic: String,
    },

    #[error(
        "`git rebase {args}` failed: {stderr}. The rebase state is left as git has it. Next: `git \
         status`"
    )]
    RebaseFailed { args: String, stderr: String },

    #[error(
        "could not {action} {path}. Check that the git folder is writable. Next: `git status`"
    )]
    StateIo {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the restack state file {path} is unreadable ({detail}). Cancel the restack and start \
         again. Next: `stack restack abort`"
    )]
    StateCorrupt { path: PathBuf, detail: String },
}

impl RestackError {
    /// 3 for a conflict, 5 for a failed --exec, 1 when git or the disk
    /// failed, otherwise 2 (something the person can fix and retry).
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            RestackError::Conflict { .. } => 3,
            RestackError::ExecFailed { .. } => 5,
            RestackError::Git(_) | RestackError::RebaseFailed { .. } | RestackError::StateIo { .. } => 1,
            RestackError::Config(error) => error.exit_code(),
            _ => 2,
        }
    }

    /// Just the command the message ends with.
    pub(crate) fn next_command(&self) -> String {
        match self {
            RestackError::Conflict { .. }
            | RestackError::ExecFailed { .. }
            | RestackError::Unmerged { .. }
            | RestackError::SkipWithoutConflict { .. } => "stack restack continue".to_owned(),
            RestackError::Unstaged => "git commit --amend -a".to_owned(),
            RestackError::AlreadyInProgress { .. } | RestackError::InProgress { .. } => {
                "stack restack continue".to_owned()
            }
            RestackError::NoRestack { .. } => "stack restack edit --topic <topic>".to_owned(),
            RestackError::EmptyStack | RestackError::UnknownTarget { .. } => "stack check".to_owned(),
            RestackError::NoTarget | RestackError::BadExec => "stack restack edit --help".to_owned(),
            RestackError::GitRebaseInProgress => "git rebase --continue".to_owned(),
            RestackError::StateCorrupt { .. } => "stack restack abort".to_owned(),
            RestackError::Plan(_) => "stack check".to_owned(),
            RestackError::Config(_) => "stack config show".to_owned(),
            RestackError::Dirty
            | RestackError::RebaseFailed { .. }
            | RestackError::StateIo { .. }
            | RestackError::Git(_) => "git status".to_owned(),
        }
    }

    /// Whether the message already ends with `Next: …`; the wrapped git,
    /// config and plan errors don't.
    pub(crate) fn names_next(&self) -> bool {
        !matches!(self, RestackError::Git(_) | RestackError::Config(_) | RestackError::Plan(_))
    }
}

/// Why a restack is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// At the commit you asked to edit.
    Edit,
    /// After a topic, because of --step.
    Step,
    /// A commit did not apply.
    Conflict,
    /// The --exec command failed.
    Exec,
}

impl Why {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Why::Edit => "edit",
            Why::Step => "step",
            Why::Conflict => "conflict",
            Why::Exec => "exec",
        }
    }
}

/// A restack that is stopped, as `stack status` shows it.
#[derive(Debug, Clone)]
pub(crate) struct Progress {
    /// The topic being edited, when the restack was started with --topic or
    /// the commit has one.
    pub(crate) target_topic: Option<String>,
    /// The topic of the commit it stopped at.
    pub(crate) topic: Option<String>,
    /// The short sha it stopped at (the original commit's, on a conflict).
    pub(crate) sha: String,
    pub(crate) subject: String,
    pub(crate) why: Why,
    /// Commits still to replay.
    pub(crate) left: usize,
    /// The command that moves on.
    pub(crate) next: String,
    topics_left: usize,
    files: Vec<String>,
    command: Option<String>,
}

impl Progress {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "target_topic": self.target_topic,
            "topic": self.topic,
            "sha": self.sha,
            "subject": self.subject,
            "why": self.why.as_str(),
            "commits_left": self.left,
            "next": self.next,
        })
    }

    /// One line for `stack status`.
    pub(crate) fn line(&self) -> String {
        let topic = self
            .topic
            .as_deref()
            .map(|topic| format!(" in topic {topic}"))
            .unwrap_or_default();
        format!(
            "restack in progress: stopped ({}) at {} \"{}\"{topic}; {} commits left. Next: `{}`",
            self.why.as_str(),
            self.sha,
            self.subject,
            self.left,
            self.next
        )
    }
}

/// One commit the restack replays.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Replayed {
    sha: String,
    topic: Option<String>,
    subject: String,
}

/// What `stack restack edit` recorded in the git folder.
#[derive(Debug, Clone)]
struct State {
    original: String,
    branch: Option<String>,
    target_topic: Option<String>,
    target_sha: String,
    step: bool,
    exec: Option<String>,
    commits: Vec<Replayed>,
}

impl State {
    fn to_json(&self) -> Value {
        let commits: Vec<Value> = self
            .commits
            .iter()
            .map(|commit| json!({"sha": commit.sha, "topic": commit.topic, "subject": commit.subject}))
            .collect();
        json!({
            "original": self.original,
            "branch": self.branch,
            "target": {"topic": self.target_topic, "sha": self.target_sha},
            "step": self.step,
            "exec": self.exec,
            "commits": commits,
        })
    }

    fn from_json(value: &Value) -> Option<State> {
        let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let target = value.get("target")?;
        let mut commits = Vec::new();
        for commit in value.get("commits")?.as_array()? {
            commits.push(Replayed {
                sha: text(commit, "sha")?,
                topic: text(commit, "topic"),
                subject: text(commit, "subject")?,
            });
        }
        Some(State {
            original: text(value, "original")?,
            branch: text(value, "branch"),
            target_topic: text(target, "topic"),
            target_sha: text(target, "sha")?,
            step: value.get("step")?.as_bool()?,
            exec: text(value, "exec"),
            commits,
        })
    }

    /// The index of the commit a todo-list sha (abbreviated or full) names.
    fn find(&self, sha: &str) -> Option<usize> {
        if sha.is_empty() {
            return None;
        }
        self.commits.iter().position(|commit| commit.sha.starts_with(sha))
    }
}

/// Runs a `stack restack` subcommand.
pub(crate) fn run(command: &RestackCommand, dir: &Path) -> Result<(), RestackError> {
    let git = Git::new(dir);
    match command {
        RestackCommand::Edit(args) => edit(&git, args),
        RestackCommand::Continue(args) => replay(&git, args, "continue"),
        RestackCommand::Skip(args) => replay(&git, args, "skip"),
        RestackCommand::Abort => abort(&git),
    }
}

/// The stopped restack, when there is one: its state file exists and git's
/// rebase is still in progress.
pub(crate) fn in_progress(git: &Git) -> Result<Option<Progress>, RestackError> {
    match read_state(git)? {
        Some(state) => inspect(git, &state),
        None => Ok(None),
    }
}

/// `RestackError::InProgress` when a restack is stopped; upload calls this
/// before it pushes anything.
pub(crate) fn refuse_if_in_progress(git: &Git) -> Result<(), RestackError> {
    match in_progress(git)? {
        Some(progress) => Err(RestackError::InProgress {
            sha: progress.sha,
            subject: progress.subject,
            topic: progress.topic.unwrap_or_else(|| "(none)".to_owned()),
        }),
        None => Ok(()),
    }
}

fn edit(git: &Git, args: &EditArgs) -> Result<(), RestackError> {
    let config = Config::load()?;
    let remote = config.remote(&args.target);
    let trunk = config.local_trunk(&args.target, git, &remote);
    if let Some(exec) = &args.replay.exec
        && exec.contains('\n')
    {
        return Err(RestackError::BadExec);
    }
    // A stale or unreadable state file of a finished restack is overwritten
    // below, so only a live restack blocks.
    if let Ok(Some(state)) = read_state(git)
        && let Some(progress) = inspect(git, &state)?
    {
        return Err(RestackError::AlreadyInProgress {
            sha: progress.sha,
            subject: progress.subject,
        });
    }
    if git.rebase_in_progress()? {
        return Err(RestackError::GitRebaseInProgress);
    }
    if git.has_uncommitted_changes()? {
        return Err(RestackError::Dirty);
    }

    let merge_base = git.merge_base("HEAD", &format!("{remote}/{trunk}"))?;
    let commits = git.commits(&merge_base, "HEAD")?;
    let plan = Plan::build(commits.clone())?;
    if plan.topics.is_empty() {
        return Err(RestackError::EmptyStack);
    }
    let mut topic_of: BTreeMap<&str, &str> = BTreeMap::new();
    for topic in &plan.topics {
        for commit in &topic.commits {
            topic_of.insert(commit.sha.as_str(), topic.name.as_str());
        }
    }
    let known = || {
        plan.topics
            .iter()
            .map(|topic| topic.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let index = match (&args.edit.topic, &args.edit.commit) {
        (Some(name), None) => {
            let last = plan
                .topics
                .iter()
                .find(|topic| &topic.name == name)
                .and_then(|topic| topic.commits.last());
            let Some(last) = last else {
                return Err(RestackError::UnknownTarget {
                    target: name.clone(),
                    topics: known(),
                });
            };
            commits.iter().position(|commit| commit.sha == last.sha)
        }
        (None, Some(rev)) => match git.rev_parse(rev) {
            Ok(sha) => commits.iter().position(|commit| commit.sha == sha),
            Err(_) => None,
        },
        _ => return Err(RestackError::NoTarget),
    };
    let Some(index) = index else {
        return Err(RestackError::UnknownTarget {
            target: args.edit.commit.clone().unwrap_or_default(),
            topics: known(),
        });
    };

    let replayed: Vec<Replayed> = commits[index..]
        .iter()
        .map(|commit| Replayed {
            sha: commit.sha.clone(),
            topic: topic_of.get(commit.sha.as_str()).map(|topic| (*topic).to_owned()),
            subject: commit.message.subject.clone(),
        })
        .collect();
    let target = replayed[0].clone();
    let state = State {
        original: git.rev_parse("HEAD")?,
        branch: git.current_branch()?,
        target_topic: target.topic.clone(),
        target_sha: target.sha.clone(),
        step: args.replay.step,
        exec: args.replay.exec.clone(),
        commits: replayed,
    };

    let todo = edit_todo(&state.commits, state.step, state.exec.as_deref());
    let todo_path = git.git_path("stack/restack-todo")?;
    write_file(&todo_path, &todo)?;
    write_state(git, &state)?;
    let editor = format!("cp {}", shell_quote(&todo_path.to_string_lossy()));
    let upstream = format!("{}^", target.sha);
    let output = git.rebase(&["-i", "--no-autosquash", "--no-update-refs", upstream.as_str()], Some(&editor))?;
    let result = report(git, &state, &output, &["-i", upstream.as_str()]);
    // The todo file was only an input for git's editor.
    let _ = fs::remove_file(&todo_path);
    result
}

/// `continue` and `skip`.
fn replay(git: &Git, args: &ReplayArgs, verb: &'static str) -> Result<(), RestackError> {
    if let Some(exec) = &args.exec
        && exec.contains('\n')
    {
        return Err(RestackError::BadExec);
    }
    let Some(mut state) = read_state(git)? else {
        return Err(RestackError::NoRestack { verb });
    };
    let Some(progress) = inspect(git, &state)? else {
        remove_state(git)?;
        return Err(RestackError::NoRestack { verb });
    };
    if verb == "skip" {
        if progress.why != Why::Conflict {
            return Err(RestackError::SkipWithoutConflict {
                sha: progress.sha,
                subject: progress.subject,
            });
        }
    } else {
        let unmerged = git.unmerged_paths()?;
        if !unmerged.is_empty() {
            return Err(RestackError::Unmerged {
                files: unmerged.join(" "),
            });
        }
        if git.has_unstaged_changes()? {
            return Err(RestackError::Unstaged);
        }
    }

    let old_exec = state.exec.clone();
    state.step = args.step;
    if args.exec.is_some() {
        state.exec = args.exec.clone();
    }
    let todo_path = git.git_path("rebase-merge/git-rebase-todo")?;
    let todo = match fs::read_to_string(&todo_path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(RestackError::StateIo {
                action: "read",
                path: todo_path,
                source,
            });
        }
    };
    let rewritten = rewrite_todo(&todo, &state.commits, old_exec.as_deref(), state.step, state.exec.as_deref());
    write_state(git, &state)?;
    if rewritten != todo {
        write_file(&todo_path, &rewritten)?;
    }

    let flag = if verb == "skip" { "--skip" } else { "--continue" };
    let output = git.rebase(&[flag], None)?;
    report(git, &state, &output, &[flag])
}

fn abort(git: &Git) -> Result<(), RestackError> {
    let state = read_state(git).ok().flatten();
    let rebasing = git.git_path("rebase-merge")?.is_dir();
    let Some(state) = state.filter(|_| rebasing) else {
        if !rebasing {
            let _ = remove_state(git);
        }
        return Err(RestackError::NoRestack { verb: "abort" });
    };
    let output = git.rebase(&["--abort"], None)?;
    if !output.status.success() {
        return Err(rebase_failed(&["--abort"], &output));
    }
    remove_state(git)?;
    let original: String = state.original.chars().take(8).collect();
    let on = state
        .branch
        .map(|branch| format!("branch {branch}"))
        .unwrap_or_else(|| "HEAD".to_owned());
    println!("Restack cancelled: {on} is back at its original commit {original}.");
    Ok(())
}

/// Classifies what a `git rebase` run left behind, prints the outcome, and
/// turns a conflict or a failed exec into its error.
fn report(git: &Git, state: &State, output: &Output, args: &[&str]) -> Result<(), RestackError> {
    let success = output.status.success();
    let Some(progress) = inspect(git, state)? else {
        if !success {
            let _ = remove_state(git);
            return Err(rebase_failed(args, output));
        }
        remove_state(git)?;
        println!(
            "Restack finished: {} commits replayed.\nNext: stack upload",
            state.commits.len()
        );
        return Ok(());
    };
    match progress.why {
        Why::Conflict => Err(RestackError::Conflict {
            topic: progress.topic.clone().unwrap_or_else(|| "(none)".to_owned()),
            sha: progress.sha.clone(),
            subject: progress.subject.clone(),
            files: if progress.files.is_empty() {
                "(none left unmerged; run `git add` on the resolved files)".to_owned()
            } else {
                progress.files.join(", ")
            },
        }),
        Why::Exec if !success => Err(RestackError::ExecFailed {
            topic: progress.topic.clone().unwrap_or_else(|| "(none)".to_owned()),
            command: progress.command.clone().unwrap_or_default(),
            sha: progress.sha.clone(),
            subject: progress.subject.clone(),
        }),
        _ if !success => Err(rebase_failed(args, output)),
        Why::Step => {
            let topic = progress.topic.as_deref().unwrap_or("(none)");
            println!(
                "Replayed topic {topic}; stopped at {} \"{}\". {} later topics are left.",
                progress.sha, progress.subject, progress.topics_left
            );
            println!(
                "Next: stack restack continue --step (or `stack restack continue` to run to the end)"
            );
            Ok(())
        }
        Why::Edit | Why::Exec => {
            println!("Stopped at {} \"{}\".", progress.sha, progress.subject);
            println!("Next: git commit --amend, then stack restack continue");
            Ok(())
        }
    }
}

/// Reads git's rebase folder and the state into a `Progress`; None when no
/// rebase is in progress.
fn inspect(git: &Git, state: &State) -> Result<Option<Progress>, RestackError> {
    let dir = git.git_path("rebase-merge")?;
    if !dir.is_dir() {
        return Ok(None);
    }
    let files = git.unmerged_paths()?;
    let rebase_head = git.rev_parse("REBASE_HEAD").ok();
    let done = read_lines(&dir.join("done"))?;
    let todo = read_lines(&dir.join("git-rebase-todo"))?;

    let last = done.last().map(String::as_str).unwrap_or("");
    let last_word = last.split_whitespace().next().unwrap_or("");
    let why = if !files.is_empty() || (rebase_head.is_some() && last_word == "pick") {
        Why::Conflict
    } else {
        match last_word {
            "break" => Why::Step,
            "exec" => Why::Exec,
            _ => Why::Edit,
        }
    };

    let last_pick = done
        .iter()
        .rev()
        .filter(|line| matches!(line.split_whitespace().next(), Some("pick" | "edit")))
        .find_map(|line| line.split_whitespace().nth(1).and_then(|sha| state.find(sha)));
    let replayed_index = match (&why, &rebase_head) {
        (Why::Conflict, Some(head)) => state.find(head).or(last_pick),
        _ => last_pick,
    };
    let replayed = replayed_index.and_then(|index| state.commits.get(index));

    let sha_source = match (&why, &rebase_head) {
        (Why::Conflict, Some(head)) => head.clone(),
        _ => git.rev_parse("HEAD")?,
    };
    let sha: String = sha_source.chars().take(8).collect();

    let mut left = 0;
    let mut topics_left: BTreeSet<&str> = BTreeSet::new();
    for line in &todo {
        let mut words = line.split_whitespace();
        if !matches!(words.next(), Some("pick" | "edit")) {
            continue;
        }
        left += 1;
        let topic = words
            .next()
            .and_then(|sha| state.find(sha))
            .and_then(|index| state.commits.get(index))
            .and_then(|commit| commit.topic.as_deref());
        if let Some(topic) = topic {
            topics_left.insert(topic);
        }
    }

    let next = match why {
        Why::Step => "stack restack continue --step",
        _ => "stack restack continue",
    }
    .to_owned();
    let command = (why == Why::Exec)
        .then(|| last.strip_prefix("exec").map(|rest| rest.trim().to_owned()))
        .flatten();
    Ok(Some(Progress {
        target_topic: state.target_topic.clone(),
        topic: replayed.and_then(|commit| commit.topic.clone()),
        sha,
        subject: replayed.map(|commit| commit.subject.clone()).unwrap_or_default(),
        why,
        left,
        next,
        topics_left: topics_left.len(),
        files,
        command,
    }))
}

/// The indices of the commits that end a topic among the replayed commits,
/// except the first (you are already stopped there).
fn topic_ends(commits: &[Replayed]) -> BTreeSet<usize> {
    let mut ends = BTreeSet::new();
    for (index, commit) in commits.iter().enumerate().skip(1) {
        let Some(topic) = &commit.topic else { continue };
        if !commits[index + 1..].iter().any(|later| later.topic.as_ref() == Some(topic)) {
            ends.insert(index);
        }
    }
    ends
}

/// The todo lines that go after a topic's last commit.
fn extra_lines(step: bool, exec: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(exec) = exec {
        lines.push(format!("exec {exec}"));
    }
    if step {
        lines.push("break".to_owned());
    }
    lines
}

/// The whole todo list for `git rebase -i <target>^`.
fn edit_todo(commits: &[Replayed], step: bool, exec: Option<&str>) -> String {
    let ends = topic_ends(commits);
    let mut todo = String::new();
    for (index, commit) in commits.iter().enumerate() {
        let word = if index == 0 { "edit" } else { "pick" };
        todo.push_str(&format!("{word} {}\n", commit.sha));
        if ends.contains(&index) {
            for line in extra_lines(step, exec) {
                todo.push_str(&line);
                todo.push('\n');
            }
        }
    }
    todo
}

/// The remaining todo list with stack's `break` and `exec` lines replaced by
/// the ones the new mode wants.
fn rewrite_todo(
    todo: &str,
    commits: &[Replayed],
    old_exec: Option<&str>,
    step: bool,
    exec: Option<&str>,
) -> String {
    let ends = topic_ends(commits);
    let old_line = old_exec.map(|exec| format!("exec {exec}"));
    let mut out = String::new();
    for line in todo.lines() {
        let trimmed = line.trim();
        if trimmed == "break" || old_line.as_deref() == Some(trimmed) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        let mut words = trimmed.split_whitespace();
        if !matches!(words.next(), Some("pick" | "edit")) {
            continue;
        }
        let end = words
            .next()
            .and_then(|sha| commits.iter().position(|commit| !sha.is_empty() && commit.sha.starts_with(sha)))
            .is_some_and(|index| ends.contains(&index));
        if end {
            for extra in extra_lines(step, exec) {
                out.push_str(&extra);
                out.push('\n');
            }
        }
    }
    out
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn rebase_failed(args: &[&str], output: &Output) -> RestackError {
    RestackError::RebaseFailed {
        args: args.join(" "),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn read_lines(path: &Path) -> Result<Vec<String>, RestackError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(RestackError::StateIo {
            action: "read",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), RestackError> {
    let io_error = |source: io::Error| RestackError::StateIo {
        action: "write",
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, text).map_err(io_error)?;
    fs::rename(&temp, path).map_err(io_error)
}

fn state_path(git: &Git) -> Result<PathBuf, RestackError> {
    Ok(git.git_path("stack/restack.json")?)
}

fn write_state(git: &Git, state: &State) -> Result<(), RestackError> {
    let path = state_path(git)?;
    let text = serde_json::to_string_pretty(&state.to_json()).unwrap_or_default();
    write_file(&path, &text)
}

fn read_state(git: &Git) -> Result<Option<State>, RestackError> {
    let path = state_path(git)?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RestackError::StateIo {
                action: "read",
                path,
                source,
            });
        }
    };
    let value: Value = serde_json::from_str(&text).map_err(|error| RestackError::StateCorrupt {
        path: path.clone(),
        detail: error.to_string(),
    })?;
    State::from_json(&value)
        .map(Some)
        .ok_or_else(|| RestackError::StateCorrupt {
            path,
            detail: "a field is missing".to_owned(),
        })
}

fn remove_state(git: &Git) -> Result<(), RestackError> {
    let path = state_path(git)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RestackError::StateIo {
            action: "remove",
            path,
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, topic: Option<&str>) -> Replayed {
        Replayed {
            sha: sha.to_owned(),
            topic: topic.map(str::to_owned),
            subject: "s".to_owned(),
        }
    }

    #[test]
    fn a_new_mode_replaces_the_old_break_and_exec_lines() {
        let commits = vec![
            commit("aaaa1111", Some("a")),
            commit("bbbb2222", Some("b")),
            commit("cccc3333", Some("c")),
        ];
        let todo = "pick bbbb222 s\nexec old\nbreak\npick cccc333 s\nexec old\n";
        let out = rewrite_todo(todo, &commits, Some("old"), false, Some("new"));
        assert_eq!(out, "pick bbbb222 s\nexec new\npick cccc333 s\nexec new\n");
        let stepped = rewrite_todo(&out, &commits, Some("new"), true, None);
        assert_eq!(stepped, "pick bbbb222 s\nbreak\npick cccc333 s\nbreak\n");
    }

    #[test]
    fn only_the_last_commit_of_each_later_topic_gets_a_stop() {
        let commits = vec![
            commit("a1", Some("a")),
            commit("b1", Some("b")),
            commit("x1", None),
            commit("b2", Some("b")),
        ];
        assert_eq!(edit_todo(&commits, true, None), "edit a1\npick b1\npick x1\npick b2\nbreak\n");
    }
}
