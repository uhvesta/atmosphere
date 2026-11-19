//! Desk sessions (DK-2, D182): the part of starting one that needs no gRPC.
//! A Desk session is an ordinary session filed under the channel "desk",
//! with no repositories and no branch. Its folder is a scratch folder Humata
//! makes under `<channels>/desk/<name>` (D451: no new session starts in a
//! folder the person picked; an old picked session is only ever read, closed
//! and moved, never started here).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use humata2_revive::Env;
use humata2_store::{
    DeskFolderKind, DeskRow, SessionDetail, SessionRecord, StartStep, Store, StoreError,
};

use crate::doc_repo::{self, DocGit, DocRepoError};
use humata2_sessions_core::error::{GitError, StartError};
use humata2_sessions_core::git::{clone_main, git, remote_url};
use humata2_sessions_core::names::branch_slug;
use humata2_sessions_core::setup::unix_ms;
use humata2_sessions_core::start::Started;

/// The channel id, and the folder name under the channels folder, every Desk
/// session lives under (D182.1). No channel may take this name.
pub use humata2_sessions_core::names::DESK_CHANNEL;

/// How many names a scratch folder tries before giving up: `slug`, then
/// `slug-2`, `slug-3`, and so on (DK-2).
const MAX_ATTEMPTS: u32 = 1000;

/// The folder a Desk session starts in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeskFolder {
    /// A folder Humata makes under `<channels>/desk`, named from the title.
    Scratch,
}

/// A Desk session to start.
#[derive(Clone, Debug)]
pub struct DeskStart {
    /// The session's id, chosen by the caller.
    pub id: String,
    /// The title shown; the scratch folder's name comes from it.
    pub title: String,
    /// The agent as named in Settings › Agents.
    pub agent: String,
    pub model: Option<String>,
    pub first_message: Option<String>,
    /// The machine the session runs on: this one.
    pub machine: String,
    pub folder: DeskFolder,
    /// Repositories cloned into the folder, each as `<name>` (DK-3).
    pub repos: Vec<(String, String)>,
    /// DK-4: the two switches the session starts with.
    pub boards: bool,
    pub channels_and_machines: bool,
    /// The operation that started the session, if any.
    pub operation: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// What a Desk session's folder, or a Desk operation, can't do, said for the
/// person who has to fix it.
#[derive(Debug, thiserror::Error)]
pub enum DeskError {
    #[error("the folder {path} can't be found: {source}; pick a folder that exists")]
    Missing {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{path} is not a folder directly under {root}, so Humata won't delete it; leave it where it is, or remove it by hand"
    )]
    NotAScratchFolder { path: PathBuf, root: PathBuf },
    #[error(
        "couldn't make the Desk folder {path}: {source}; check that {parent} exists and can be written to, then start the session again"
    )]
    Create {
        path: PathBuf,
        parent: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "none of the {attempts} folder names for {slug} under {root} is free; remove some old Desk folders, then start the session again"
    )]
    NoFreeName {
        root: PathBuf,
        slug: String,
        attempts: u32,
    },
    #[error(
        "couldn't remove the folder {path} after its session failed to start: {source}; remove it by hand, then start the session again"
    )]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't {doing} for the Desk session: {source}; fix the cause the message names, then try again")]
    Store {
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error(
        "couldn't set up the versions repository in {path}: {source}; check that git is installed and that the folder can be written to, then start the session again"
    )]
    DocRepo {
        path: PathBuf,
        #[source]
        source: DocRepoError,
    },
    #[error("a background task for the Desk failed: {source}; try again, and restart humatad2 if it keeps failing")]
    Background {
        #[source]
        source: tokio::task::JoinError,
    },
    #[error("{name} isn't a plain folder name for a repository; name the repository by its own name, such as `api`, with no slashes and no leading dot")]
    BadRepoName { name: String },
    #[error("{path} is already in this session's folder, so nothing was cloned and it is left as it is; remove it first, or leave it")]
    RepoExists { path: PathBuf },
    #[error("there is no repository {name} in {folder}; add it to the session first")]
    NoSuchRepo { name: String, folder: PathBuf },
    #[error("couldn't clone {owner}/{name}: {source}; check that the repository exists and that this Mac can reach its remote, then try again")]
    CloneFailed {
        owner: String,
        name: String,
        #[source]
        source: GitError,
    },
    #[error("{path} isn't a folder of its own directly inside {folder}, so Humata won't touch it; leave it, or remove it by hand")]
    NotInFolder { path: PathBuf, folder: PathBuf },
    #[error("{path} isn't a file inside {folder}; give a path relative to the session's folder, with no .. and no leading /")]
    OutsideFolder { path: PathBuf, folder: PathBuf },
    #[error("there is no file at {path}; check the name and try again")]
    NoSuchFile { path: PathBuf },
    #[error("{path} isn't a plain file, so it can't be copied; copy a file, not a folder or a link")]
    NotARegularFile { path: PathBuf },
    #[error("{path} is already in the channel's shared folder, so nothing was copied and that file is left as it is; rename the Desk file, then copy again")]
    ChannelHasFile { path: PathBuf },
    #[error("couldn't copy to {path}: {source}; check that the folder can be written to, then try again")]
    CopyFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Start(#[from] StartError),
}

/// The folder every Desk scratch folder is made in: `<channels>/desk`.
pub fn desk_root(channels: &Path) -> PathBuf {
    channels.join(DESK_CHANNEL)
}

/// Claims a scratch folder named `slug` under `root`: `slug` itself, else
/// `slug-2`, `slug-3`, … The first name not in `taken` and not already a
/// folder is made with one `create_dir` at mode 0700, so two starts can't
/// both get it (DK-2, FC-14).
pub fn claim_scratch(root: &Path, slug: &str, taken: &HashSet<String>) -> Result<PathBuf, DeskError> {
    create_private(root)?;
    for attempt in 1..=MAX_ATTEMPTS {
        let name = if attempt == 1 {
            slug.to_owned()
        } else {
            format!("{slug}-{attempt}")
        };
        if taken.contains(&name) {
            continue;
        }
        let path = root.join(&name);
        match fs::create_dir(&path) {
            Ok(()) => {
                restrict(&path)?;
                // The folder's entry in `root` must survive a power cut too.
                humata2_durable::sync_dir(root).map_err(|source| DeskError::Create {
                    path: path.clone(),
                    parent: root.to_owned(),
                    source,
                })?;
                return Ok(path);
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(DeskError::Create {
                    path,
                    parent: root.to_owned(),
                    source,
                });
            }
        }
    }
    Err(DeskError::NoFreeName {
        root: root.to_owned(),
        slug: slug.to_owned(),
        attempts: MAX_ATTEMPTS,
    })
}

/// The folder of a scratch session, checked before anything is moved out of
/// it: it must be a direct child of `desk_root`, not a symlink. Returns it
/// canonicalised (D182.4). Whatever the row says is not trusted.
pub fn scratch_child(desk_root: &Path, folder: &Path) -> Result<PathBuf, DeskError> {
    let refused = || DeskError::NotAScratchFolder {
        path: folder.to_owned(),
        root: desk_root.to_owned(),
    };
    let meta = fs::symlink_metadata(folder).map_err(|source| DeskError::Missing {
        path: folder.to_owned(),
        source,
    })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(refused());
    }
    let real = folder.canonicalize().map_err(|source| DeskError::Missing {
        path: folder.to_owned(),
        source,
    })?;
    let root = desk_root.canonicalize().map_err(|source| DeskError::Missing {
        path: desk_root.to_owned(),
        source,
    })?;
    if real.parent() != Some(root.as_path()) {
        return Err(refused());
    }
    Ok(real)
}

/// The bytes a folder holds, counting files only; symlinks count nothing.
pub fn folder_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        meta.len()
    } else if meta.is_dir() {
        fs::read_dir(path)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| folder_bytes(&entry.path()))
                    .sum()
            })
            .unwrap_or(0)
    } else {
        0
    }
}

/// The git work under a scratch folder that deleting it would lose (LD-17,
/// D182.4): the folder itself when it is a repository Humata didn't make as a
/// doc's versions (the folder's own repo, DV-1, holds nothing that would be
/// lost: a commit there keeps the folder another way, DV-4), and each
/// repository directly under it, with uncommitted files or commits no remote has. Empty
/// when there is none, in words otherwise. A repository git can't read counts
/// as work, so nothing is deleted unseen.
pub fn held_work(folder: &Path) -> String {
    let mut repos: Vec<PathBuf> = Vec::new();
    if folder.join(".git").exists() && !doc_repo::is_doc_repo(folder) {
        repos.push(folder.to_owned());
    }
    if let Ok(entries) = fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
            if is_dir && path.join(".git").exists() {
                repos.push(path);
            }
        }
    }
    repos_work(&repos)
}

/// The git work in one repository clone that removing it would lose: its
/// uncommitted files and commits no remote has (DK-3). Empty when there is
/// none. Unlike [`held_work`], the clone's own `.git` folder is never taken
/// for a doc's versions repo.
pub fn clone_work(clone: &Path) -> String {
    repos_work(&[clone.to_owned()])
}

/// The work held in each of `repos`, in words.
fn repos_work(repos: &[PathBuf]) -> String {
    let mut words = Vec::new();
    for repo in repos {
        let name = repo
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match git(Some(repo), &["status", "--porcelain"]) {
            Ok(text) => {
                let files = text.lines().filter(|line| !line.trim().is_empty()).count();
                if files > 0 {
                    words.push(format!("{name} has {files} changed or untracked file(s)"));
                }
            }
            Err(error) => words.push(format!("{name}: couldn't read its git state ({error}), so it is kept")),
        }
        match git(Some(repo), &["rev-list", "--count", "--branches", "--not", "--remotes"]) {
            Ok(text) => match text.trim().parse::<u64>() {
                Ok(0) => {}
                Ok(commits) => words.push(format!("{name} has {commits} commit(s) no remote has")),
                Err(_) => words.push(format!("{name}: couldn't count its commits, so it is kept")),
            },
            Err(error) => words.push(format!("{name}: couldn't read its commits ({error}), so it is kept")),
        }
    }
    words.join("; ")
}

/// The Desk row a new session starts with: nothing closed or deleted, and
/// used now (DK-4, SE-26).
pub fn new_row(
    session: &str,
    kind: DeskFolderKind,
    boards: bool,
    channels_and_machines: bool,
    now: i64,
) -> DeskRow {
    DeskRow {
        session_id: session.to_owned(),
        kind,
        boards,
        channels_and_machines,
        last_used_unix_ms: now,
        closed_unix_ms: None,
        deleted_unix_ms: None,
        held_work: String::new(),
    }
}

impl crate::DeskOps<'_> {
    /// Starts a Desk session (DK-14, D182): claims or checks its folder,
    /// records the session under channel "desk" with its Desk row, and
    /// launches its agent the way any session's agent is launched. A failure
    /// before the launch takes back what this start made; from the launch on,
    /// the records stay and revive owns them, as for any session.
    pub async fn start_desk(
        &self,
        request: DeskStart,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, DeskError> {
        let agent_name = self
            .agents
            .get(&request.agent)
            .map_err(StartError::from)?
            .person_name();
        let (store, channels) = (self.store.clone(), self.layout.root());
        let (choice, slug) = (request.folder.clone(), branch_slug(&request.title));
        let (template, repos) = (self.remote_template.clone(), request.repos.clone());
        // The folder's versions are authored under the agent's person name.
        let folder = blocking(move || -> Result<PathBuf, DeskError> {
            for (_, name) in &repos {
                check_repo_name(name)?;
            }
            let folder = match choice {
                DeskFolder::Scratch => {
                    let taken = held_desk_names(&store)?;
                    claim_scratch(&desk_root(&channels), &slug, &taken)?
                }
            };
            // A scratch folder is a versions repository of its own, with its
            // clones left out of it (DV-1). A step that fails takes back the
            // folder whole.
            if let Err(error) = prepare_folder(&template, &repos, &folder, &agent_name) {
                let _ = remove_folder(&folder);
                return Err(error);
            }
            Ok(folder)
        })
        .await?;

        let session = SessionRecord {
            id: request.id.clone(),
            channel: DESK_CHANNEL.to_owned(),
            machine: request.machine.clone(),
            title: request.title.clone(),
            agent: request.agent.clone(),
            folder: folder.clone(),
            created_unix_ms: unix_ms(),
        };
        let detail = SessionDetail {
            session: request.id.clone(),
            repos: Vec::new(),
            model: request.model.clone(),
            first_message: request.first_message.clone(),
            start: StartStep::Recorded,
            operation: request.operation.clone(),
            revision: 1,
        };
        let row = new_row(
            &request.id,
            DeskFolderKind::Scratch { kept: false },
            request.boards,
            request.channels_and_machines,
            unix_ms(),
        );
        if let Err(error) = self.record_desk(&session, &detail, &row).await {
            // Best effort: the start's own error is the one worth reporting.
            let _ = self.undo_desk(&session).await;
            return Err(error);
        }
        progress("write_record");
        let (detail, agent) = self
            .launch_session(&session, detail, request.cols, request.rows, env, progress)
            .await?;
        Ok(Started {
            session,
            detail,
            agent,
        })
    }

    /// Saves the session, its start and its Desk row in one blocking step.
    async fn record_desk(
        &self,
        session: &SessionRecord,
        detail: &SessionDetail,
        row: &DeskRow,
    ) -> Result<(), DeskError> {
        let (store, session, detail, row) = (
            self.store.clone(),
            session.clone(),
            detail.clone(),
            row.clone(),
        );
        blocking(move || -> Result<(), DeskError> {
            store
                .put_session(&session)
                .map_err(store_error("save the session"))?;
            store
                .put_session_detail(&detail)
                .map_err(store_error("save the session's start"))?;
            store
                .put_desk(&row)
                .map_err(store_error("save the Desk row"))
        })
        .await
    }

    /// Takes back a Desk start that failed before its launch: the scratch
    /// folder it made and the session's records.
    pub(crate) async fn undo_desk(&self, session: &SessionRecord) -> Result<(), DeskError> {
        let (store, id, folder) = (
            self.store.clone(),
            session.id.clone(),
            session.folder.clone(),
        );
        blocking(move || -> Result<(), DeskError> {
            remove_folder(&folder)?;
            store
                .remove_session(&id)
                .map_err(store_error("remove the session"))
        })
        .await
    }
}

/// Makes a scratch folder Humata just claimed a versions repository, named
/// for `agent` (DV-1). A failure removes the folder, so no start leaves one
/// behind.
pub(crate) fn init_scratch(folder: &Path, agent: &str) -> Result<(), DeskError> {
    doc_repo::init(&DocGit::inherit(), folder, agent).map_err(|source| {
        let _ = remove_folder(folder);
        DeskError::DocRepo {
            path: folder.to_owned(),
            source,
        }
    })
}

/// The folder's start: it becomes a repository, then the clones are made, and
/// each is left out of the folder's own versions.
fn prepare_folder(
    template: &str,
    repos: &[(String, String)],
    folder: &Path,
    agent: &str,
) -> Result<(), DeskError> {
    doc_repo::init(&DocGit::inherit(), folder, agent).map_err(|source| DeskError::DocRepo {
        path: folder.to_owned(),
        source,
    })?;
    clone_repos(template, repos, folder)?;
    for (_, name) in repos {
        doc_repo::exclude_clone(&DocGit::inherit(), folder, name).map_err(|source| DeskError::DocRepo {
            path: folder.to_owned(),
            source,
        })?;
    }
    Ok(())
}

/// Checks a repository name is one plain folder name, so its clone goes
/// directly under the session's folder (DK-3).
pub fn check_repo_name(name: &str) -> Result<(), DeskError> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(_)), None) if !name.starts_with('.') => Ok(()),
        _ => Err(DeskError::BadRepoName {
            name: name.to_owned(),
        }),
    }
}

fn clone_error(owner: &str, name: &str, source: GitError) -> DeskError {
    DeskError::CloneFailed {
        owner: owner.to_owned(),
        name: name.to_owned(),
        source,
    }
}

/// Clones `owner/name`'s default branch into `folder/name` (DK-3). A name
/// already in the folder is refused and left as it is.
pub fn clone_repo(template: &str, owner: &str, name: &str, folder: &Path) -> Result<PathBuf, DeskError> {
    check_repo_name(name)?;
    let dest = folder.join(name);
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(DeskError::RepoExists { path: dest });
    }
    let url = remote_url(template, owner, name).map_err(|source| clone_error(owner, name, source))?;
    clone_main(&url, &dest).map_err(|source| clone_error(owner, name, source))?;
    Ok(dest)
}

/// Clones each repository into `folder`. When one fails, the clones this call
/// made are taken back and the error names the repository that failed.
pub fn clone_repos(template: &str, repos: &[(String, String)], folder: &Path) -> Result<(), DeskError> {
    let mut made = Vec::new();
    for (owner, name) in repos {
        match clone_repo(template, owner, name, folder) {
            Ok(path) => made.push(path),
            Err(error) => {
                for path in &made {
                    let _ = remove_folder(path);
                }
                return Err(error);
            }
        }
    }
    Ok(())
}

/// The clone of repository `name` directly under `folder`, checked to be a
/// folder of its own and not a link (DK-3). Returns it canonicalised.
pub fn repo_clone(folder: &Path, name: &str) -> Result<PathBuf, DeskError> {
    check_repo_name(name)?;
    let path = folder.join(name);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(DeskError::NoSuchRepo {
                name: name.to_owned(),
                folder: folder.to_owned(),
            });
        }
        Err(source) => return Err(DeskError::Missing { path, source }),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(DeskError::NotInFolder {
            path,
            folder: folder.to_owned(),
        });
    }
    let real = path.canonicalize().map_err(|source| DeskError::Missing {
        path: path.clone(),
        source,
    })?;
    let root = folder.canonicalize().map_err(|source| DeskError::Missing {
        path: folder.to_owned(),
        source,
    })?;
    if real.parent() != Some(root.as_path()) {
        return Err(DeskError::NotInFolder {
            path,
            folder: folder.to_owned(),
        });
    }
    Ok(real)
}

/// The repositories directly under `folder`, by the owner and name in their
/// origin, sorted (DK-3). Plain file reads, no git. A repository whose origin
/// can't be read is named by its folder, with no owner.
pub fn repos_under(folder: &Path) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut repos: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_dir = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
            if !is_dir || !path.join(".git").exists() {
                return None;
            }
            let origin = fs::read_to_string(path.join(".git").join("config"))
                .ok()
                .and_then(|config| origin_url(&config))
                .and_then(|url| owner_and_name(&url));
            Some(origin.unwrap_or_else(|| {
                (String::new(), entry.file_name().to_string_lossy().into_owned())
            }))
        })
        .collect();
    repos.sort();
    repos
}

/// The `url` of `[remote "origin"]` in a git config, if it has one.
pub(crate) fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
            continue;
        }
        if !in_origin {
            continue;
        }
        if let Some(value) = line
            .strip_prefix("url")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// The owner and name a remote URL names: `…/owner/name(.git)`, for https,
/// ssh and scp-like addresses alike.
pub(crate) fn owner_and_name(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => url.rsplit_once(':').map_or(url, |(_, path)| path),
    };
    let mut parts = path.rsplit('/');
    let name = parts.next()?;
    let owner = parts.next()?;
    (!owner.is_empty() && !name.is_empty()).then(|| (owner.to_owned(), name.to_owned()))
}

/// The Markdown files directly in `folder`, by name, sorted (DK-5).
pub fn documents_under(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| fs::symlink_metadata(entry.path()).is_ok_and(|meta| meta.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md"))
        .collect();
    names.sort();
    names
}

/// The most bytes read from one Markdown file to count its lines.
const LINE_COUNT_CAP: u64 = 8 * 1024 * 1024;

/// How many lines each Markdown file directly in `folder` has, by name
/// (CD-4). A last line with no newline counts. A file that can't be read, or
/// is over [`LINE_COUNT_CAP`], is left out; the rest are still counted.
pub fn document_lines_under(folder: &Path) -> BTreeMap<String, u32> {
    documents_under(folder)
        .into_iter()
        .filter_map(|name| {
            let lines = count_lines(&folder.join(&name))?;
            Some((name, lines))
        })
        .collect()
}

fn count_lines(path: &Path) -> Option<u32> {
    use std::io::Read;
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > LINE_COUNT_CAP {
        return None;
    }
    let mut reader = file.take(LINE_COUNT_CAP);
    let mut buf = [0_u8; 16 * 1024];
    let mut lines: u32 = 0;
    // An empty file has no lines, so start as if the last byte ended one.
    let mut last = b'\n';
    loop {
        let read = reader.read(&mut buf).ok()?;
        if read == 0 {
            break;
        }
        let newlines = buf[..read].iter().filter(|byte| **byte == b'\n').count();
        lines = lines.saturating_add(u32::try_from(newlines).unwrap_or(u32::MAX));
        last = buf[read - 1];
    }
    if last != b'\n' {
        lines = lines.saturating_add(1);
    }
    Some(lines)
}

/// Copies the file `relative`, a path inside `folder`, into `shared` under the
/// same name (DK-13). Never overwrites: a file of that name is left as it is.
/// Returns the name copied to.
pub fn copy_document(folder: &Path, relative: &str, shared: &Path) -> Result<String, DeskError> {
    let rel = Path::new(relative);
    let outside = || DeskError::OutsideFolder {
        path: rel.to_owned(),
        folder: folder.to_owned(),
    };
    if relative.is_empty() || !rel.components().all(|part| matches!(part, Component::Normal(_))) {
        return Err(outside());
    }
    let Some(name) = rel.file_name().map(|name| name.to_string_lossy().into_owned()) else {
        return Err(outside());
    };
    let source = folder.join(rel);
    let meta = match fs::symlink_metadata(&source) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(DeskError::NoSuchFile { path: source });
        }
        Err(error) => return Err(DeskError::Missing { path: source, source: error }),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(DeskError::NotARegularFile { path: source });
    }
    let real = source.canonicalize().map_err(|error| DeskError::Missing {
        path: source.clone(),
        source: error,
    })?;
    let root = folder.canonicalize().map_err(|error| DeskError::Missing {
        path: folder.to_owned(),
        source: error,
    })?;
    if !real.starts_with(&root) {
        return Err(outside());
    }
    create_private(shared)?;
    let dest = shared.join(&name);
    let mut from = fs::File::open(&real).map_err(|error| DeskError::CopyFailed {
        path: real.clone(),
        source: error,
    })?;
    let mut to = match fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(DeskError::ChannelHasFile { path: dest });
        }
        Err(error) => return Err(DeskError::CopyFailed { path: dest, source: error }),
    };
    let copied = io::copy(&mut from, &mut to)
        .and_then(|_| to.sync_all())
        .and_then(|()| humata2_durable::sync_dir(shared));
    if let Err(error) = copied {
        let _ = fs::remove_file(&dest);
        return Err(DeskError::CopyFailed {
            path: dest,
            source: error,
        });
    }
    Ok(name)
}

/// The folder names the Desk's sessions already hold, tombstoned ones
/// included, so a name is never handed out twice (DK-2).
fn held_desk_names(store: &Store) -> Result<HashSet<String>, DeskError> {
    Ok(store
        .sessions()
        .map_err(store_error("list the Desk's sessions"))?
        .into_iter()
        .filter(|session| session.channel == DESK_CHANNEL)
        .filter_map(|session| {
            session
                .folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect())
}

pub(crate) fn store_error(doing: &'static str) -> impl Fn(StoreError) -> DeskError {
    move |source| DeskError::Store { doing, source }
}

/// Runs `work` on a blocking thread: the Desk's folders and records are on disk.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DeskError> + Send + 'static,
) -> Result<T, DeskError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| DeskError::Background { source })?
}

/// Makes `path` (and any parent missing) readable only by this user.
fn create_private(path: &Path) -> Result<(), DeskError> {
    humata2_sessions_core::layout::make_dirs(path, 0o700).map_err(|source| DeskError::Create {
        path: path.to_owned(),
        parent: path.parent().map(Path::to_owned).unwrap_or_default(),
        source,
    })?;
    restrict(path)
}

/// Sets `path` to mode 0700.
fn restrict(path: &Path) -> Result<(), DeskError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            DeskError::Create {
                path: path.to_owned(),
                parent: path.parent().map(Path::to_owned).unwrap_or_default(),
                source,
            }
        })?;
    }
    Ok(())
}

/// Removes a scratch folder Humata made; a folder already gone is fine.
fn remove_folder(path: &Path) -> Result<(), DeskError> {
    match fs::remove_dir_all(path) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => Err(DeskError::Remove {
            path: path.to_owned(),
            source,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_scratch_folder_takes_the_slug_then_the_next_free_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("desk");
        let first = claim_scratch(&root, "plan-q4", &held(&[])).unwrap();
        assert_eq!(first.file_name().unwrap(), "plan-q4");
        let second = claim_scratch(&root, "plan-q4", &held(&["plan-q4"])).unwrap();
        assert_eq!(second.file_name().unwrap(), "plan-q4-2");
    }

    #[test]
    fn a_name_held_by_a_record_or_a_folder_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("desk");
        std::fs::create_dir_all(root.join("plan-q4-2")).unwrap();
        let claimed = claim_scratch(&root, "plan-q4", &held(&["plan-q4"])).unwrap();
        assert_eq!(claimed.file_name().unwrap(), "plan-q4-3");
    }

    #[test]
    fn a_scratch_folder_is_private() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let temp = tempfile::tempdir().unwrap();
            let claimed = claim_scratch(&temp.path().join("desk"), "notes", &held(&[])).unwrap();
            let mode = std::fs::metadata(&claimed).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn lines_are_counted_per_markdown_file_with_or_without_a_last_newline() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path();
        std::fs::write(folder.join("a.md"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(folder.join("b.md"), "one\ntwo").unwrap();
        std::fs::write(folder.join("empty.md"), "").unwrap();
        std::fs::write(folder.join("notes.txt"), "not\nmarkdown\n").unwrap();
        std::fs::create_dir(folder.join("dir.md")).unwrap();
        let counts = document_lines_under(folder);
        let expected: BTreeMap<String, u32> = [("a.md", 3), ("b.md", 2), ("empty.md", 0)]
            .into_iter()
            .map(|(name, lines)| (name.to_owned(), lines))
            .collect();
        assert_eq!(counts, expected);
    }

    #[test]
    fn a_folder_that_cannot_be_read_has_no_counts() {
        let temp = tempfile::tempdir().unwrap();
        assert!(document_lines_under(&temp.path().join("missing")).is_empty());
    }

    #[test]
    fn a_file_over_the_cap_is_left_out_and_the_others_are_counted() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path();
        std::fs::write(folder.join("small.md"), "a\n").unwrap();
        let big = std::fs::File::create(folder.join("big.md")).unwrap();
        big.set_len(LINE_COUNT_CAP + 1).unwrap();
        let counts = document_lines_under(folder);
        assert_eq!(counts.get("small.md"), Some(&1));
        assert!(!counts.contains_key("big.md"));
    }

    #[test]
    fn a_new_row_is_open_with_the_switches_it_was_given() {
        let row = new_row("se_1", DeskFolderKind::Scratch { kept: false }, true, false, 42);
        assert_eq!(row.kind, DeskFolderKind::Scratch { kept: false });
        assert!(row.boards && !row.channels_and_machines);
        assert_eq!(row.closed_unix_ms, None);
        assert_eq!(row.last_used_unix_ms, 42);
        let picked = new_row("se_2", DeskFolderKind::Picked, false, false, 7);
        assert_eq!(picked.kind, DeskFolderKind::Picked);
    }
}
