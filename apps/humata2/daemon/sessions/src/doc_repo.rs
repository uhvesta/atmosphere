//! A Desk folder's own git repository, the store of a doc's versions (DV-1,
//! DV-2, DV-5, DV-8, DV-11). A version is a commit that changes the file;
//! the person saves one with Save version, the agent with a plain `git
//! commit`, and nothing is saved on its own. Humata writes only the repo's
//! local `.git/config`, never the person's global git config, and its own
//! commits skip the person's hooks and signing.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::error::GitError;

/// The trailer on the commits Humata makes for the person (DV-5).
pub const SAVED_BY_TRAILER: &str = "Humata-Saved-By";

/// How git is run for a Desk folder's repo: the daemon's own environment, or
/// that plus a few variables (tests point HOME and the global config at a
/// temporary folder).
#[derive(Clone, Debug, Default)]
pub struct DocGit {
    envs: Vec<(OsString, OsString)>,
}

impl DocGit {
    /// Git with the daemon's environment as it is.
    #[must_use]
    pub fn inherit() -> Self {
        Self::default()
    }

    /// Git with these variables set on top of the daemon's environment.
    #[must_use]
    pub fn with_env(envs: Vec<(OsString, OsString)>) -> Self {
        Self { envs }
    }

    /// Runs git in `folder` and returns what it printed.
    fn run(&self, folder: &Path, args: &[&str]) -> Result<String, GitError> {
        let mut cmd = Command::new("git");
        cmd.current_dir(folder)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0");
        // Git must never find a repository above the folder, so a folder
        // that is not a repo of its own stays unrepresented.
        if let Some(parent) = folder.parent() {
            cmd.env("GIT_CEILING_DIRECTORIES", parent);
        }
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }
        let shown = args.join(" ");
        let out = cmd.output().map_err(|source| GitError::Spawn {
            args: shown.clone(),
            source,
        })?;
        if !out.status.success() {
            return Err(GitError::Failed {
                args: shown,
                status: out.status.to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Who saved a version (DV-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedBy {
    /// The person, through Save version or Restore.
    You,
    /// The session's agent, or anything else that committed.
    Agent,
}

/// One version of one file: a commit that changed it (DV-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// The commit.
    pub id: String,
    /// 1 for the file's oldest version, counting up.
    pub number: u32,
    /// When it was committed, in Unix milliseconds.
    pub unix_ms: i64,
    /// Who saved it.
    pub by: SavedBy,
    /// The commit's subject line.
    pub message: String,
    /// Lines of the file added in this version.
    pub added: u32,
    /// Lines of the file removed in this version.
    pub removed: u32,
}

/// What a restore committed (DV-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restored {
    /// The version that saved unsaved changes first, when there were any.
    pub saved_first: Option<String>,
    /// The new version holding the restored text.
    pub restored: String,
}

/// Why a Desk folder's repo couldn't do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum DocRepoError {
    #[error(
        "{folder} has no versions repository; reopen the session so Humata sets one up, or ask \
         Humata to migrate the Desk folders"
    )]
    NotARepo { folder: PathBuf },
    #[error(
        "{path} is not a file inside the session's folder; give a path relative to the folder, \
         without `..` and outside `.git`"
    )]
    BadPath { path: String },
    #[error("nothing in {folder} changed since the last version, so there is nothing to save")]
    NoChanges { folder: PathBuf },
    #[error(
        "{path} has no version {id}; list its versions again, the history may have been \
         rewritten outside Humata"
    )]
    NoSuchVersion { path: String, id: String },
    #[error("{path} already reads as version {number}, so restoring it would change nothing")]
    AlreadyCurrent { path: String, number: u32 },
    #[error("couldn't {doing} in the versions repository: {source}")]
    Git {
        doing: &'static str,
        #[source]
        source: GitError,
    },
    #[error("couldn't read or write {path}: {source}; check the folder is there and writable")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Maps a failed git step to the error that says what was being done.
fn git_failed(doing: &'static str) -> impl FnOnce(GitError) -> DocRepoError {
    move |source| DocRepoError::Git { doing, source }
}

/// Maps a failed file step to an error that names the path.
fn io_failed(path: &Path) -> impl FnOnce(io::Error) -> DocRepoError + '_ {
    move |source| DocRepoError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Refuses a folder that has no versions repository of its own.
fn require_repo(folder: &Path) -> Result<(), DocRepoError> {
    if is_doc_repo(folder) {
        Ok(())
    } else {
        Err(DocRepoError::NotARepo {
            folder: folder.to_path_buf(),
        })
    }
}

/// Refuses a path that is not a plain file path inside the folder (DV-6): no
/// empty path, no absolute path, no `.` or `..` parts, nothing under `.git`.
fn check_path(path: &str) -> Result<(), DocRepoError> {
    let relative = Path::new(path);
    let plain = !path.is_empty()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    let outside_git = !matches!(
        relative.components().next(),
        Some(Component::Normal(first)) if first == ".git"
    );
    if plain && outside_git {
        Ok(())
    } else {
        Err(DocRepoError::BadPath {
            path: path.to_owned(),
        })
    }
}

/// The identity a repo's agent commits under: the agent's name in lowercase
/// with every character that is not a letter or digit as a dash, or "agent"
/// when nothing is left.
fn identity_slug(agent: &str) -> String {
    let slug: String = agent
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "agent".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Adds `/<name>/` to the folder's `.git/info/exclude` unless it is there.
fn add_exclude(folder: &Path, name: &str) -> Result<(), DocRepoError> {
    let info = folder.join(".git").join("info");
    let exclude = info.join("exclude");
    let line = format!("/{name}/");
    let existing = match fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(DocRepoError::Io { path: exclude, source }),
    };
    if existing.lines().any(|known| known == line.as_str()) {
        return Ok(());
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&line);
    text.push('\n');
    fs::create_dir_all(&info).map_err(io_failed(&info))?;
    fs::write(&exclude, text).map_err(io_failed(&exclude))
}

/// Excludes every clone directly under the folder, so the folder's own
/// versions never take in a repository of its own (DV-1).
fn exclude_child_repos(folder: &Path) -> Result<(), DocRepoError> {
    for entry in fs::read_dir(folder).map_err(io_failed(folder))? {
        let entry = entry.map_err(io_failed(folder))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        // symlink_metadata does not follow links, so a symlink is never a clone.
        let meta = fs::symlink_metadata(&path).map_err(io_failed(&path))?;
        if meta.is_dir() && path.join(".git").exists() {
            add_exclude(folder, &name)?;
        }
    }
    Ok(())
}

/// Whether `file` exists and holds exactly `text`.
fn is_on_disk(file: &Path, text: &str) -> Result<bool, DocRepoError> {
    match fs::read(file) {
        Ok(bytes) => Ok(bytes == text.as_bytes()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(DocRepoError::Io {
            path: file.to_path_buf(),
            source,
        }),
    }
}

/// Writes `text` to `file`, making its folders first.
fn write_file(file: &Path, text: &str) -> Result<(), DocRepoError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(io_failed(parent))?;
    }
    fs::write(file, text).map_err(io_failed(file))
}

/// The commit `HEAD` names right after a commit was made.
fn named_head(git: &DocGit, folder: &Path) -> Result<String, DocRepoError> {
    git.run(folder, &["rev-parse", "--verify", "HEAD^{commit}"])
        .map(|out| out.trim().to_owned())
        .map_err(git_failed("read the new version"))
}

/// Commits what is staged as a version by the person (DV-5). The repo's own
/// identity makes the author; the commit skips the person's hooks and signing.
fn commit_as_you(git: &DocGit, folder: &Path, message: &str) -> Result<String, DocRepoError> {
    let trailer = format!("{SAVED_BY_TRAILER}: you");
    git.run(
        folder,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--no-verify",
            "-m",
            message,
            "-m",
            trailer.as_str(),
        ],
    )
    .map_err(git_failed("save the version"))?;
    named_head(git, folder)
}

/// One `git log --numstat` line (`added<TAB>removed<TAB>path`). A binary file
/// shows "-", which counts as none.
fn numstat(line: &str) -> (u32, u32) {
    let mut counts = line
        .split('\t')
        .map(|count| count.parse::<u32>().unwrap_or(0));
    (counts.next().unwrap_or(0), counts.next().unwrap_or(0))
}

/// One record of the versions log: the commit's header fields, then its
/// numstat line. A record that does not have the fields is skipped.
fn parse_record(record: &str) -> Option<Version> {
    let mut lines = record.lines();
    let mut fields = lines.next()?.splitn(4, '\x1f');
    let id = fields.next()?;
    let seconds: i64 = fields.next()?.trim().parse().ok()?;
    let message = fields.next()?;
    let saved_by = fields.next()?.trim();
    let (added, removed) = lines
        .find(|line| !line.trim().is_empty())
        .map_or((0, 0), numstat);
    Some(Version {
        id: id.to_owned(),
        number: 0,
        unix_ms: seconds.saturating_mul(1000),
        by: if saved_by == "you" {
            SavedBy::You
        } else {
            SavedBy::Agent
        },
        message: message.to_owned(),
        added,
        removed,
    })
}

/// The version of `path` that commit `id` is, or NoSuchVersion.
fn find_version(
    git: &DocGit,
    folder: &Path,
    path: &str,
    id: &str,
) -> Result<Version, DocRepoError> {
    versions(git, folder, path)?
        .into_iter()
        .find(|version| version.id == id)
        .ok_or_else(|| DocRepoError::NoSuchVersion {
            path: path.to_owned(),
            id: id.to_owned(),
        })
}

/// Makes `folder` a git repository with no commit, its identity in the
/// repo's local config named for `agent` (DV-1, DV-11). A folder that is a
/// repository already is left as it is.
pub fn init(git: &DocGit, folder: &Path, agent: &str) -> Result<(), DocRepoError> {
    if is_doc_repo(folder) {
        return Ok(());
    }
    fs::metadata(folder).map_err(io_failed(folder))?;
    git.run(folder, &["init", "-q", "-b", "main"])
        .map_err(git_failed("create the versions repository"))?;
    git.run(folder, &["config", "--local", "user.name", agent])
        .map_err(git_failed("set the versions repository's author name"))?;
    let email = format!("{}@agents.humata.local", identity_slug(agent));
    git.run(
        folder,
        &["config", "--local", "user.email", email.as_str()],
    )
    .map_err(git_failed("set the versions repository's author email"))?;
    Ok(())
}

/// Keeps the clone `name` directly under `folder` out of the folder's own
/// versions, through `.git/info/exclude` (DV-1). Adding it twice adds one line.
pub fn exclude_clone(_git: &DocGit, folder: &Path, name: &str) -> Result<(), DocRepoError> {
    require_repo(folder)?;
    add_exclude(folder, name)
}

/// Whether `folder` has a versions repository of its own.
#[must_use]
pub fn is_doc_repo(folder: &Path) -> bool {
    folder.join(".git").is_dir()
}

/// Whether the folder holds at least one version, which keeps it (DV-4).
pub fn has_version(git: &DocGit, folder: &Path) -> Result<bool, DocRepoError> {
    Ok(head(git, folder)?.is_some())
}

/// The newest commit, or `None` before the first version.
pub fn head(git: &DocGit, folder: &Path) -> Result<Option<String>, DocRepoError> {
    require_repo(folder)?;
    match git.run(folder, &["rev-parse", "--verify", "-q", "HEAD^{commit}"]) {
        Ok(out) => Ok(Some(out.trim().to_owned())),
        // Before the first commit there is no HEAD to name, which git
        // reports as a failed run.
        Err(GitError::Failed { .. }) => Ok(None),
        Err(source) => Err(DocRepoError::Git {
            doing: "read the newest version",
            source,
        }),
    }
}

/// How many versions the folder holds: 0 before the first.
pub fn version_count(git: &DocGit, folder: &Path) -> Result<u32, DocRepoError> {
    require_repo(folder)?;
    match git.run(folder, &["rev-list", "--count", "HEAD"]) {
        Ok(out) => Ok(out.trim().parse().unwrap_or(0)),
        // Before the first commit HEAD names nothing, which git reports as
        // a failed run.
        Err(GitError::Failed { .. }) => Ok(0),
        Err(source) => Err(DocRepoError::Git {
            doing: "count the versions",
            source,
        }),
    }
}

/// Whether anything outside the clones changed since the last version.
pub fn has_changes(git: &DocGit, folder: &Path) -> Result<bool, DocRepoError> {
    require_repo(folder)?;
    exclude_child_repos(folder)?;
    let status = git
        .run(folder, &["status", "--porcelain", "--untracked-files=all"])
        .map_err(git_failed("check for unsaved changes"))?;
    Ok(!status.trim().is_empty())
}

/// The versions of the file `path` (relative to the folder), newest first,
/// numbered from the oldest (DV-2).
pub fn versions(git: &DocGit, folder: &Path, path: &str) -> Result<Vec<Version>, DocRepoError> {
    require_repo(folder)?;
    check_path(path)?;
    if head(git, folder)?.is_none() {
        return Ok(Vec::new());
    }
    let format = format!(
        "--format=%x1e%H%x1f%ct%x1f%s%x1f%(trailers:key={SAVED_BY_TRAILER},valueonly,separator=%x2c)"
    );
    let log = git
        .run(
            folder,
            &["log", format.as_str(), "--numstat", "--", path],
        )
        .map_err(git_failed("list the versions of a file"))?;
    let mut list: Vec<Version> = log.split('\x1e').filter_map(parse_record).collect();
    // Git lists newest first, so the oldest version is the last one.
    let total = list.len();
    for (index, version) in list.iter_mut().enumerate() {
        version.number = u32::try_from(total - index).unwrap_or(u32::MAX);
    }
    Ok(list)
}

/// The text of `path` as version `id` has it (DV-6).
pub fn read(git: &DocGit, folder: &Path, path: &str, id: &str) -> Result<String, DocRepoError> {
    require_repo(folder)?;
    check_path(path)?;
    let version = find_version(git, folder, path, id)?;
    let spec = format!("{}:{path}", version.id);
    git.run(folder, &["show", spec.as_str()])
        .map_err(git_failed("read a version"))
}

/// Which side of a version diff a line is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// In both.
    Context,
    /// Only in the file now.
    Added,
    /// Only in the version.
    Removed,
}

/// One line of a version diff, numbered on the side(s) it is on (0 on the
/// other).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// Its side.
    pub change: Change,
    /// Its number in the version, 0 for an added line.
    pub old_line: u32,
    /// Its number in the file now, 0 for a removed line.
    pub new_line: u32,
    /// Its text.
    pub text: String,
}

/// A run of changes with its context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    /// git's `@@ -a,b +c,d @@ section` line.
    pub header: String,
    /// First line of the hunk in the version.
    pub old_start: u32,
    /// First line of the hunk in the file now.
    pub new_start: u32,
    /// The text after the second `@@`, if any.
    pub section: String,
    /// Its lines.
    pub lines: Vec<DiffLine>,
}

/// A version against the file as it is now (DV-7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionDiff {
    /// Empty when the file reads as the version does.
    pub hunks: Vec<DiffHunk>,
    /// Lines added since the version.
    pub added: u32,
    /// Lines removed since the version.
    pub removed: u32,
}

/// The change from version `id` of `path` to the file as it is on disk, with
/// three lines of context (DV-7). A file the version doesn't have, or an
/// unknown version, is NoSuchVersion; a file deleted since shows all removed.
pub fn diff(git: &DocGit, folder: &Path, path: &str, id: &str) -> Result<VersionDiff, DocRepoError> {
    require_repo(folder)?;
    check_path(path)?;
    let version = find_version(git, folder, path, id)?;
    let out = git
        .run(
            folder,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--unified=3",
                version.id.as_str(),
                "--",
                path,
            ],
        )
        .map_err(git_failed("compare a version with the file"))?;
    Ok(parse_diff(&out))
}

/// `@@ -a,b +c,d @@ section` to (a, c, section).
fn parse_hunk_header(line: &str) -> Option<(u32, u32, String)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let start = |range: &str| range.split(',').next()?.parse::<u32>().ok();
    Some((start(old)?, start(new)?, section.trim().to_owned()))
}

/// The hunks of one file's `git diff` output. Everything before the first
/// `@@` is the file header and is skipped; a binary file has no hunks.
fn parse_diff(text: &str) -> VersionDiff {
    let mut diff = VersionDiff::default();
    let (mut old_at, mut new_at) = (0_u32, 0_u32);
    for line in text.lines() {
        if let Some((old_start, new_start, section)) = parse_hunk_header(line) {
            (old_at, new_at) = (old_start, new_start);
            diff.hunks.push(DiffHunk {
                header: line.to_owned(),
                old_start,
                new_start,
                section,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else {
            continue;
        };
        let (change, body) = match line.as_bytes().first() {
            Some(b'+') => (Change::Added, &line[1..]),
            Some(b'-') => (Change::Removed, &line[1..]),
            Some(b' ') => (Change::Context, &line[1..]),
            None => (Change::Context, ""),
            // "\ No newline at end of file".
            Some(_) => continue,
        };
        let (old_line, new_line) = match change {
            Change::Added => {
                diff.added = diff.added.saturating_add(1);
                let at = new_at;
                new_at = new_at.saturating_add(1);
                (0, at)
            }
            Change::Removed => {
                diff.removed = diff.removed.saturating_add(1);
                let at = old_at;
                old_at = old_at.saturating_add(1);
                (at, 0)
            }
            Change::Context => {
                let at = (old_at, new_at);
                old_at = old_at.saturating_add(1);
                new_at = new_at.saturating_add(1);
                at
            }
        };
        hunk.lines.push(DiffLine {
            change,
            old_line,
            new_line,
            text: body.to_owned(),
        });
    }
    diff
}

/// Saves every change outside the clones as one version by the person
/// (DV-5): `message`, or "Saved by you" when it is blank. Returns the new
/// version. Refused when nothing changed.
pub fn save(git: &DocGit, folder: &Path, message: Option<&str>) -> Result<String, DocRepoError> {
    require_repo(folder)?;
    if !has_changes(git, folder)? {
        return Err(DocRepoError::NoChanges {
            folder: folder.to_path_buf(),
        });
    }
    git.run(folder, &["add", "-A"])
        .map_err(git_failed("stage the changes"))?;
    let text = message
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("Saved by you");
    commit_as_you(git, folder, text)
}

/// Restores `path` to version `id` as a new version (DV-8). Unsaved changes
/// are saved first, then only that file is written and committed as
/// "Restored vN". Every older version stays.
pub fn restore(git: &DocGit, folder: &Path, path: &str, id: &str) -> Result<Restored, DocRepoError> {
    require_repo(folder)?;
    check_path(path)?;
    let version = find_version(git, folder, path, id)?;
    let text = read(git, folder, path, id)?;
    let file = folder.join(path);
    let on_disk = is_on_disk(&file, &text)?;
    let unsaved = has_changes(git, folder)?;
    if on_disk && !unsaved {
        return Err(DocRepoError::AlreadyCurrent {
            path: path.to_owned(),
            number: version.number,
        });
    }
    let saved_first = if unsaved {
        let note = format!("Saved before restoring v{}", version.number);
        Some(save(git, folder, Some(note.as_str()))?)
    } else {
        None
    };
    if let (true, Some(saved)) = (on_disk, &saved_first) {
        // The save above already committed this text, so nothing is left to write.
        return Ok(Restored {
            saved_first: Some(saved.clone()),
            restored: saved.clone(),
        });
    }
    write_file(&file, &text)?;
    git.run(folder, &["add", "--", path])
        .map_err(git_failed("stage the restored file"))?;
    let note = format!("Restored v{}", version.number);
    let restored = commit_as_you(git, folder, &note)?;
    Ok(Restored {
        saved_first,
        restored,
    })
}

/// The files commit `id` changed, relative to the folder (DV-12).
pub fn changed_paths(git: &DocGit, folder: &Path, id: &str) -> Result<Vec<String>, DocRepoError> {
    require_repo(folder)?;
    let out = git
        .run(
            folder,
            &[
                "-c",
                "core.quotePath=false",
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                "--root",
                id,
            ],
        )
        .map_err(git_failed("list the files a version changed"))?;
    Ok(out
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// The commits after `old` up to and including `new`, oldest first (DV-12);
/// with no `old`, the newest 256 up to `new`.
pub fn commits_since(
    git: &DocGit,
    folder: &Path,
    old: Option<&str>,
    new: &str,
) -> Result<Vec<String>, DocRepoError> {
    require_repo(folder)?;
    let range = match old {
        Some(old) => format!("{old}..{new}"),
        None => new.to_owned(),
    };
    let out = git
        .run(
            folder,
            &["rev-list", "--reverse", "--max-count=256", range.as_str()],
        )
        .map_err(git_failed("list the new versions"))?;
    Ok(out
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOBAL: &str = "[user]\n\tname = Owner\n\temail = owner@example.com\n";

    /// A Desk folder and a git whose HOME and global config are a
    /// temporary folder's, so nothing here reads or writes the real ones.
    struct Sandbox {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        folder: PathBuf,
        git: DocGit,
    }

    impl Sandbox {
        fn new() -> Self {
            Self::with_global(GLOBAL)
        }

        fn with_global(global: &str) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let home = tmp.path().join("home");
            let folder = tmp.path().join("desk").join("plan-q4");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&folder).unwrap();
            fs::write(home.join(".gitconfig"), global).unwrap();
            let git = DocGit::with_env(vec![
                ("HOME".into(), home.clone().into()),
                ("XDG_CONFIG_HOME".into(), home.join(".config").into()),
                ("GIT_CONFIG_GLOBAL".into(), home.join(".gitconfig").into()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ]);
            Self {
                _tmp: tmp,
                home,
                folder,
                git,
            }
        }

        fn ready(self) -> Self {
            init(&self.git, &self.folder, "Claude Code").unwrap();
            self
        }

        fn write(&self, path: &str, text: &str) {
            let file = self.folder.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }

        fn text(&self, path: &str) -> String {
            fs::read_to_string(self.folder.join(path)).unwrap()
        }

        /// What the agent does: a plain `git add` and `git commit`, under
        /// the repo's own identity.
        fn agent_commit(&self, message: &str) -> String {
            self.git.run(&self.folder, &["add", "-A"]).unwrap();
            self.git
                .run(
                    &self.folder,
                    &["-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message],
                )
                .unwrap();
            head(&self.git, &self.folder).unwrap().unwrap()
        }

        fn tracked(&self) -> Vec<String> {
            self.git
                .run(&self.folder, &["ls-files"])
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        /// A clone of some repository directly under the folder.
        fn clone_at(&self, name: &str) {
            let clone = self.folder.join(name);
            fs::create_dir_all(&clone).unwrap();
            self.git.run(&clone, &["init", "-q"]).unwrap();
            fs::write(clone.join("README.md"), "clone\n").unwrap();
        }
    }

    #[test]
    fn init_makes_a_repo_with_no_version_and_leaves_the_global_config_alone() {
        let sandbox = Sandbox::new().ready();
        assert!(is_doc_repo(&sandbox.folder));
        assert!(!has_version(&sandbox.git, &sandbox.folder).unwrap());
        assert_eq!(head(&sandbox.git, &sandbox.folder).unwrap(), None);
        assert_eq!(
            fs::read_to_string(sandbox.home.join(".gitconfig")).unwrap(),
            GLOBAL,
            "the global config is byte for byte what it was"
        );
        let name = sandbox
            .git
            .run(&sandbox.folder, &["config", "--local", "user.name"])
            .unwrap();
        assert_eq!(name.trim(), "Claude Code", "the identity is the repo's own");
    }

    #[test]
    fn a_plain_folder_is_not_a_doc_repo() {
        let sandbox = Sandbox::new();
        assert!(!is_doc_repo(&sandbox.folder));
        assert!(matches!(
            has_version(&sandbox.git, &sandbox.folder),
            Err(DocRepoError::NotARepo { .. })
        ));
    }

    #[test]
    fn init_on_a_repo_changes_nothing() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "one\n");
        let first = sandbox.agent_commit("first");
        let config = fs::read(sandbox.folder.join(".git/config")).unwrap();
        init(&sandbox.git, &sandbox.folder, "Codex").unwrap();
        assert_eq!(fs::read(sandbox.folder.join(".git/config")).unwrap(), config);
        assert_eq!(head(&sandbox.git, &sandbox.folder).unwrap(), Some(first));
    }

    #[test]
    fn a_plain_git_commit_is_the_agents_version() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "one\ntwo\n");
        let id = sandbox.agent_commit("Draft the plan");
        assert!(has_version(&sandbox.git, &sandbox.folder).unwrap());
        let list = versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].number, 1);
        assert_eq!(list[0].by, SavedBy::Agent);
        assert_eq!((list[0].added, list[0].removed), (2, 0));
        assert!(list[0].unix_ms > 0);
    }

    #[test]
    fn save_commits_every_change_as_yours_and_never_the_clones() {
        let sandbox = Sandbox::new().ready();
        sandbox.clone_at("api");
        exclude_clone(&sandbox.git, &sandbox.folder, "api").unwrap();
        exclude_clone(&sandbox.git, &sandbox.folder, "api").unwrap();
        sandbox.clone_at("web");
        sandbox.write("plan.md", "one\n");
        sandbox.write("notes/ideas.md", "idea\n");
        assert!(has_changes(&sandbox.git, &sandbox.folder).unwrap());
        let id = save(&sandbox.git, &sandbox.folder, None).unwrap();
        assert_eq!(head(&sandbox.git, &sandbox.folder).unwrap(), Some(id));
        let mut tracked = sandbox.tracked();
        tracked.sort();
        assert_eq!(tracked, vec!["notes/ideas.md".to_owned(), "plan.md".to_owned()]);
        assert!(!has_changes(&sandbox.git, &sandbox.folder).unwrap());
        let list = versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap();
        assert_eq!(list[0].by, SavedBy::You);
        let exclude = fs::read_to_string(sandbox.folder.join(".git/info/exclude")).unwrap();
        assert_eq!(exclude.lines().filter(|line| line.contains("api")).count(), 1);
    }

    #[test]
    fn save_with_nothing_changed_is_refused() {
        let sandbox = Sandbox::new().ready();
        assert!(matches!(
            save(&sandbox.git, &sandbox.folder, Some("Nothing")),
            Err(DocRepoError::NoChanges { .. })
        ));
        sandbox.write("plan.md", "one\n");
        save(&sandbox.git, &sandbox.folder, Some("First")).unwrap();
        assert!(matches!(
            save(&sandbox.git, &sandbox.folder, None),
            Err(DocRepoError::NoChanges { .. })
        ));
    }

    #[test]
    fn versions_are_per_file_newest_first_and_numbered_from_the_oldest() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "a\nb\n");
        let one = sandbox.agent_commit("one");
        sandbox.write("other.md", "x\n");
        sandbox.agent_commit("only other");
        sandbox.write("plan.md", "a\nc\nd\n");
        let two = save(&sandbox.git, &sandbox.folder, Some("Tighten")).unwrap();
        let list = versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap();
        let ids: Vec<_> = list.iter().map(|version| version.id.clone()).collect();
        assert_eq!(ids, vec![two, one]);
        assert_eq!(list[0].number, 2);
        assert_eq!(list[1].number, 1);
        assert_eq!(list[0].message, "Tighten");
        assert_eq!((list[0].added, list[0].removed), (2, 1));
        assert!(versions(&sandbox.git, &sandbox.folder, "never.md").unwrap().is_empty());
    }

    #[test]
    fn read_returns_that_versions_text() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        let one = sandbox.agent_commit("one");
        sandbox.write("plan.md", "second\n");
        sandbox.agent_commit("two");
        assert_eq!(read(&sandbox.git, &sandbox.folder, "plan.md", &one).unwrap(), "first\n");
        assert_eq!(sandbox.text("plan.md"), "second\n", "reading changes nothing on disk");
    }

    #[test]
    fn an_unknown_version_is_an_error() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        sandbox.agent_commit("one");
        sandbox.write("other.md", "x\n");
        let other = sandbox.agent_commit("other only");
        for id in ["0000000000000000000000000000000000000000", other.as_str(), "not-a-commit"] {
            assert!(
                matches!(
                    read(&sandbox.git, &sandbox.folder, "plan.md", id),
                    Err(DocRepoError::NoSuchVersion { .. })
                ),
                "{id}"
            );
            assert!(matches!(
                restore(&sandbox.git, &sandbox.folder, "plan.md", id),
                Err(DocRepoError::NoSuchVersion { .. })
            ));
        }
    }

    #[test]
    fn restore_adds_a_version_and_keeps_every_old_one() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        let one = sandbox.agent_commit("one");
        sandbox.write("plan.md", "second\n");
        let two = sandbox.agent_commit("two");
        let restored = restore(&sandbox.git, &sandbox.folder, "plan.md", &one).unwrap();
        assert_eq!(restored.saved_first, None);
        assert_eq!(sandbox.text("plan.md"), "first\n");
        let list = versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap();
        let ids: Vec<_> = list.iter().map(|version| version.id.clone()).collect();
        assert_eq!(ids, vec![restored.restored, two, one]);
        assert_eq!(list[0].by, SavedBy::You);
        assert_eq!(list[0].number, 3);
        assert!(!has_changes(&sandbox.git, &sandbox.folder).unwrap());
    }

    #[test]
    fn restore_saves_unsaved_changes_first() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        let one = sandbox.agent_commit("one");
        sandbox.write("plan.md", "typed but not saved\n");
        let restored = restore(&sandbox.git, &sandbox.folder, "plan.md", &one).unwrap();
        let Some(saved) = restored.saved_first else {
            panic!("the unsaved text is saved before the restore");
        };
        assert_eq!(
            read(&sandbox.git, &sandbox.folder, "plan.md", &saved).unwrap(),
            "typed but not saved\n"
        );
        assert_eq!(sandbox.text("plan.md"), "first\n");
        assert_eq!(versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap().len(), 3);
    }

    #[test]
    fn restore_changes_only_that_file() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        sandbox.write("other.md", "other first\n");
        let one = sandbox.agent_commit("one");
        sandbox.write("plan.md", "second\n");
        sandbox.write("other.md", "other second\n");
        sandbox.agent_commit("two");
        let restored = restore(&sandbox.git, &sandbox.folder, "plan.md", &one).unwrap();
        assert_eq!(sandbox.text("other.md"), "other second\n");
        assert_eq!(
            changed_paths(&sandbox.git, &sandbox.folder, &restored.restored).unwrap(),
            vec!["plan.md".to_owned()]
        );
    }

    #[test]
    fn restoring_the_text_the_file_already_has_is_refused() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        let one = sandbox.agent_commit("one");
        assert!(matches!(
            restore(&sandbox.git, &sandbox.folder, "plan.md", &one),
            Err(DocRepoError::AlreadyCurrent { .. })
        ));
        assert_eq!(versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap().len(), 1);
    }

    #[test]
    fn a_path_outside_the_folder_is_refused() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        let one = sandbox.agent_commit("one");
        for path in ["../escape.md", "/etc/passwd", "", ".git/config", "notes/../../x.md"] {
            assert!(
                matches!(
                    versions(&sandbox.git, &sandbox.folder, path),
                    Err(DocRepoError::BadPath { .. })
                ),
                "{path:?}"
            );
            assert!(matches!(
                read(&sandbox.git, &sandbox.folder, path, &one),
                Err(DocRepoError::BadPath { .. })
            ));
            assert!(matches!(
                restore(&sandbox.git, &sandbox.folder, path, &one),
                Err(DocRepoError::BadPath { .. })
            ));
        }
    }

    #[test]
    fn humatas_own_commits_skip_the_persons_hooks_and_signing() {
        let hooks = tempfile::tempdir().unwrap();
        let hook = hooks.path().join("pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let global = format!(
            "{GLOBAL}[core]\n\thooksPath = {}\n[commit]\n\tgpgsign = true\n[gpg]\n\tprogram = /bin/false\n",
            hooks.path().display()
        );
        let sandbox = Sandbox::with_global(&global).ready();
        sandbox.write("plan.md", "first\n");
        let one = save(&sandbox.git, &sandbox.folder, Some("First")).unwrap();
        sandbox.write("plan.md", "second\n");
        save(&sandbox.git, &sandbox.folder, None).unwrap();
        restore(&sandbox.git, &sandbox.folder, "plan.md", &one).unwrap();
        assert_eq!(versions(&sandbox.git, &sandbox.folder, "plan.md").unwrap().len(), 3);
    }

    #[test]
    fn changed_paths_names_the_files_a_version_changed() {
        let sandbox = Sandbox::new().ready();
        sandbox.write("plan.md", "first\n");
        sandbox.write("notes/ideas.md", "idea\n");
        let one = sandbox.agent_commit("one");
        let mut paths = changed_paths(&sandbox.git, &sandbox.folder, &one).unwrap();
        paths.sort();
        assert_eq!(paths, vec!["notes/ideas.md".to_owned(), "plan.md".to_owned()]);
    }
}
