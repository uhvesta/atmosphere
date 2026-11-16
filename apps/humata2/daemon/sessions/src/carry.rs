//! Carrying a session to another machine and checking it arrived (MV-2,
//! MV-13, MV-14, D195).
//!
//! The old machine **packs**: it stages, in `<channel>/.carry/<operation>/stage`,
//! the session's records, the agent's own files (its conversation, and its
//! project-keyed folder when the agent keeps one per session folder) and a
//! **manifest** that names every file of the session's folder, the agent's
//! files and the records with its size and SHA-256. The session folder itself
//! is copied from where it stands. The new machine **unpacks**: it checks
//! every hash against the manifest before anything is used, rewrites the old
//! folder's path in the files that name it, places the agent's files and
//! brings the records in. The session's shell tabs travel as their saved
//! output: each comes back on the new machine as a new tab whose shell
//! starts in the session's folder with that output above it, once the
//! agent resumes there. **Release** names what the old machine puts in its
//! trash once the agent runs on the new one.
//!
//! Nothing here knows about gRPC, links or the trash; the daemon drives it.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use humata2_holder::saved_scrollback_path;
use humata2_store::{AgentState, StoreError, TabKind, TabRecord, Take};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agents::AgentKind;
use crate::error::AgentError;
use crate::layout::partial;
use crate::start::Starter;

#[path = "carry_fork.rs"]
mod staged_fork;
pub use staged_fork::{ForkPack, ForkPackError, ForkPacked};

/// The folder under a channel's own where an operation's staged files wait.
pub const CARRY_DIR: &str = ".carry";

pub(crate) const STAGE: &str = "stage";
const INCOMING: &str = "in";
pub(crate) const MANIFEST: &str = "manifest.json";
pub(crate) const RECORDS: &str = "records.sqlite";
pub(crate) const AGENT: &str = "agent";
pub(crate) const TABS: &str = "tabs";
const SAVED: &str = "scrollback";

/// The biggest file whose text is searched for the old folder's path.
const REWRITE_MAX: u64 = 16 << 20;

/// Why a session couldn't be packed, unpacked, aborted or released.
#[derive(Debug, thiserror::Error)]
pub enum CarryError {
    #[error("this machine has no session {session}; nothing was changed")]
    NoSession { session: String },

    #[error("this machine has no channel {channel}; add the machine to the channel, then try again")]
    NoChannel { channel: String },

    #[error(
        "the session's folder {} isn't directly in the channel's sessions folder {}, so Humata won't copy it; \
         nothing was changed",
        folder.display(),
        sessions.display()
    )]
    NotASessionFolder { folder: PathBuf, sessions: PathBuf },

    #[error("{} is not a folder name Humata made for a session; nothing was changed", path)]
    BadFolder { path: String },

    #[error("couldn't {doing} {}: {source}; check that the disk has free space and the folder is yours. Nothing was deleted", path.display())]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't read the move manifest {}; the copy is incomplete or damaged. Nothing was deleted", path.display())]
    Manifest {
        path: PathBuf,
        #[source]
        source: ManifestProblem,
    },

    #[error(
        "the session {session}'s agent runs on this machine, so its copy isn't taken back; nothing was deleted. \
         Stop the session here first if it should run on the machine it came from"
    )]
    Running { session: String },

    #[error("{item} didn't arrive: it is in the move manifest but not in the copy. Nothing was deleted; try the move again")]
    Missing { item: String },

    #[error("{item} arrived different from what was sent (its hash doesn't match the move manifest). Nothing was deleted; try the move again")]
    Changed { item: String },

    #[error("{item} is in the copy but not in the move manifest. Nothing was deleted; try the move again")]
    Unexpected { item: String },

    #[error(
        "{} already exists on this machine, so the session's {what} can't be placed there. Nothing was deleted; \
         move or remove it, then try the move again",
        path.display()
    )]
    Exists { what: &'static str, path: PathBuf },

    #[error("the agent {agent} isn't set up on this machine: {source}")]
    Agent {
        agent: String,
        #[source]
        source: AgentError,
    },

    #[error("couldn't {doing} in this machine's state: {source}")]
    Store {
        doing: &'static str,
        #[source]
        source: StoreError,
    },
}

/// What was wrong with a manifest file.
#[derive(Debug, thiserror::Error)]
pub enum ManifestProblem {
    #[error("it can't be read")]
    Read(#[source] std::io::Error),
    #[error("it isn't a move manifest")]
    Parse(#[source] serde_json::Error),
}

pub(crate) fn io(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> CarryError {
    let path = path.to_owned();
    move |source| CarryError::Io { doing, path, source }
}

fn store(doing: &'static str) -> impl FnOnce(StoreError) -> CarryError {
    move |source| CarryError::Store { doing, source }
}

/// Where a file of the manifest belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    /// In the session's folder.
    Folder,
    /// One of the agent's own files.
    Agent,
    /// The records' side file.
    Records,
    /// A shell tab's saved output, as `<tab>.scrollback`.
    Tabs,
}

/// One file (or link) of the move manifest (MV-14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub area: Area,
    /// `/`-joined, under the area's root.
    pub rel: String,
    pub size: u64,
    /// Hex SHA-256 of the contents; of the target's text for a link.
    pub sha256: String,
    /// Set for a symbolic link: where it points.
    pub link: Option<String>,
}

impl Item {
    pub(crate) fn name(&self) -> String {
        match self.area {
            Area::Folder => self.rel.clone(),
            Area::Agent => format!("the agent's file {}", self.rel),
            Area::Records => "the session's records".to_owned(),
            Area::Tabs => format!("the saved output of shell tab {}", self.rel),
        }
    }
}

/// Which of the agent's files travel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFiles {
    /// The agent had no conversation file yet.
    None,
    /// The conversation file alone, named so.
    File { name: String },
    /// Every file of the agent's per-session folder, e.g.
    /// `~/.claude/projects/<folder>/`.
    Project,
}

/// What the old machine sends along with the folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub operation: String,
    pub session: String,
    pub channel: String,
    /// The session's folder under the channel's, `sessions/<name>`.
    pub folder: String,
    /// Where the folder was on the old machine.
    pub old_folder: String,
    pub agent: String,
    pub conversation_id: String,
    pub agent_files: AgentFiles,
    pub items: Vec<Item>,
    /// The session's shell tabs, oldest first (MV-13).
    #[serde(default)]
    pub tabs: Vec<CarriedTab>,
}

/// A shell tab of the session. Its saved output, if its holder saved any,
/// is staged as `tabs/<id>.scrollback`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarriedTab {
    pub id: String,
    pub title: String,
    pub created_unix_ms: i64,
}

/// What a pack staged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    /// The session's folder under the channel's, `sessions/<name>`.
    pub folder: String,
    /// The staged files under the channel's folder.
    pub stage: String,
    /// A hash of every item, so two packs of an unchanged session agree.
    pub digest: String,
    pub files: u32,
    pub bytes: u64,
    /// The session's agent was running when it was packed.
    pub agent_running: bool,
}

/// What an unpack brought in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unpacked {
    pub files: u32,
    pub bytes: u64,
}

/// One thing the old machine puts in its trash once the move is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discard {
    pub path: PathBuf,
    pub what: String,
}

pub(crate) fn sha_of_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((size, hex(&hasher.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha_of_text(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Every file and link under `root`, as items of `area`, in name order.
/// Folders hold nothing of their own; a socket or other special file is
/// left out, as no copy could carry it.
pub(crate) fn items_under(area: Area, root: &Path) -> Result<Vec<Item>, CarryError> {
    let mut found = Vec::new();
    walk(area, root, "", &mut found)?;
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(found)
}

fn walk(area: Area, folder: &Path, prefix: &str, found: &mut Vec<Item>) -> Result<(), CarryError> {
    let entries = fs::read_dir(folder).map_err(io("read the folder", folder))?;
    for entry in entries {
        let entry = entry.map_err(io("read the folder", folder))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let meta = fs::symlink_metadata(&path).map_err(io("look at", &path))?;
        if meta.is_dir() {
            walk(area, &path, &rel, found)?;
        } else if meta.is_symlink() {
            let target = fs::read_link(&path).map_err(io("read the link", &path))?;
            let target = target.to_string_lossy().into_owned();
            found.push(Item { area, rel, size: 0, sha256: sha_of_text(&target), link: Some(target) });
        } else if meta.is_file() {
            let (size, sha256) = sha_of_file(&path).map_err(io("read", &path))?;
            found.push(Item { area, rel, size, sha256, link: None });
        }
    }
    Ok(())
}

fn digest_of(items: &[Item]) -> String {
    let mut hasher = Sha256::new();
    // The records change as the agent stops, and shells keep printing,
    // their output carried as it stood; what must not change meanwhile is
    // the folder and the agent's own files.
    for item in items.iter().filter(|item| !matches!(item.area, Area::Records | Area::Tabs)) {
        hasher.update(format!("{:?}\0{}\0{}\0{}\n", item.area, item.rel, item.size, item.sha256));
    }
    hex(&hasher.finalize())
}

/// Checks that what is under `root` for `area` is exactly `wanted`, naming
/// the first item that is missing, different or extra.
pub(crate) fn verify(area: Area, root: &Path, wanted: &[&Item]) -> Result<(), CarryError> {
    let actual = if wanted.is_empty() && !root.exists() { Vec::new() } else { items_under(area, root)? };
    let have: BTreeMap<&str, &Item> = actual.iter().map(|item| (item.rel.as_str(), item)).collect();
    for item in wanted {
        match have.get(item.rel.as_str()) {
            None => return Err(CarryError::Missing { item: item.name() }),
            Some(found) if found.sha256 != item.sha256 || found.size != item.size => {
                return Err(CarryError::Changed { item: item.name() });
            }
            Some(_) => {}
        }
    }
    if let Some(extra) = actual.iter().find(|item| !wanted.iter().any(|w| w.rel == item.rel)) {
        return Err(CarryError::Unexpected { item: extra.name() });
    }
    Ok(())
}

/// `rel` is `sessions/<name>` with one plain name.
fn folder_name(rel: &str) -> Result<&str, CarryError> {
    match rel.split_once('/') {
        Some(("sessions", name))
            if !name.is_empty() && !name.contains('/') && name != "." && name != ".." && !name.contains('\0') =>
        {
            Ok(name)
        }
        _ => Err(CarryError::BadFolder { path: rel.to_owned() }),
    }
}

fn dashed(path: &str) -> String {
    path.replace(['/', '.'], "-")
}

/// The texts to find and what each becomes: the path as it is and as JSON
/// writes it, and the path as an agent names a project folder.
fn replacements(old: &str, new: &str) -> Vec<(String, String)> {
    if old == new {
        return Vec::new();
    }
    let escaped = |text: &str| {
        let quoted = serde_json::to_string(text).unwrap_or_default();
        quoted.trim_matches('"').to_owned()
    };
    let mut pairs = vec![(old.to_owned(), new.to_owned())];
    if escaped(old) != old {
        pairs.push((escaped(old), escaped(new)));
    }
    pairs.push((dashed(old), dashed(new)));
    pairs
}

/// `bytes` with every old text replaced, when it is text and holds one.
fn rewritten(bytes: &[u8], pairs: &[(String, String)]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut now = text.to_owned();
    for (old, new) in pairs {
        now = now.replace(old.as_str(), new);
    }
    (now != text).then(|| now.into_bytes())
}

/// Rewrites `file` in place when it is text naming the old path. A file that
/// is too big, isn't text or names nothing is left as it is.
fn rewrite_file(file: &Path, pairs: &[(String, String)]) -> Result<(), CarryError> {
    let meta = fs::symlink_metadata(file).map_err(io("look at", file))?;
    if !meta.is_file() || meta.len() > REWRITE_MAX {
        return Ok(());
    }
    let bytes = fs::read(file).map_err(io("read", file))?;
    if let Some(now) = rewritten(&bytes, pairs) {
        let temporary = partial(file);
        fs::write(&temporary, now).map_err(io("write", &temporary))?;
        fs::set_permissions(&temporary, meta.permissions()).map_err(io("set the permissions of", &temporary))?;
        fs::rename(&temporary, file).map_err(io("replace", file))?;
    }
    Ok(())
}

/// Rewrites the files of a clone's `.git` folder that can name the clone's
/// own place (its config, hooks and the like), never its objects, and never
/// the working files, which are the person's.
fn rewrite_git(clone: &Path, pairs: &[(String, String)]) -> Result<(), CarryError> {
    let git = clone.join(".git");
    let Ok(entries) = fs::read_dir(&git) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(io("read the folder", &git))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        match name.as_str() {
            "objects" | "refs" | "index" | "logs" | "lfs" => {}
            _ if path.is_dir() => {
                let Ok(inner) = fs::read_dir(&path) else { continue };
                for inner in inner.flatten() {
                    rewrite_file(&inner.path(), pairs)?;
                }
            }
            _ => rewrite_file(&path, pairs)?,
        }
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), CarryError> {
    fs::create_dir_all(to).map_err(io("create", to))?;
    for entry in fs::read_dir(from).map_err(io("read the folder", from))? {
        let entry = entry.map_err(io("read the folder", from))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let meta = fs::symlink_metadata(&source).map_err(io("look at", &source))?;
        if meta.is_dir() {
            copy_tree(&source, &target)?;
        } else if meta.is_file() {
            fs::copy(&source, &target).map_err(io("copy", &source))?;
        }
    }
    Ok(())
}

/// Where the agent keeps files per session folder rather than one file.
fn project_keyed(kind: &AgentKind) -> bool {
    kind.conversation_file.contains("{folder")
}

impl Starter {
    fn carry_channel(&self, channel: &str) -> Result<humata2_store::ChannelRecord, CarryError> {
        self.store
            .channel(channel)
            .map_err(store("look up the channel"))?
            .ok_or_else(|| CarryError::NoChannel { channel: channel.to_owned() })
    }

    /// The folder an operation's files wait in, in `channel`'s folder.
    fn carry_dir(&self, channel: &humata2_store::ChannelRecord, operation: &str) -> PathBuf {
        self.layout.channel(&channel.folder).join(CARRY_DIR).join(operation)
    }

    /// Stages `session` for a move: its records, the agent's files and the
    /// manifest, under `<channel>/.carry/<operation>/stage`. The session's
    /// folder isn't copied or changed. Blocking; safe to repeat.
    pub fn pack(&self, operation: &str, session: &str) -> Result<Packed, CarryError> {
        let record = self
            .store
            .session(session)
            .map_err(store("read the session"))?
            .ok_or_else(|| CarryError::NoSession { session: session.to_owned() })?;
        let channel = self.carry_channel(&record.channel)?;
        let sessions = self.layout.sessions(&channel.folder);
        let name = match record.folder.parent() {
            Some(parent) if parent == sessions => record.folder.file_name().map(|name| name.to_string_lossy().into_owned()),
            _ => None,
        }
        .ok_or_else(|| CarryError::NotASessionFolder { folder: record.folder.clone(), sessions: sessions.clone() })?;
        folder_name(&format!("sessions/{name}"))?;

        let stage = self.carry_dir(&channel, operation).join(STAGE);
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(io("clear an earlier stage in", &stage))?;
        }
        fs::create_dir_all(&stage).map_err(io("create", &stage))?;
        self.store
            .export_session(session, &stage.join(RECORDS))
            .map_err(store("copy the session's records"))?;

        let agent = self.store.agent(session).map_err(store("read the session's agent"))?;
        let agent_running = agent
            .as_ref()
            .is_some_and(|agent| matches!(agent.state, AgentState::Running { .. }));
        let mut agent_files = AgentFiles::None;
        let mut conversation_id = String::new();
        if let Some(agent) = &agent {
            conversation_id = agent.launch.conversation_id.clone();
            let file = &agent.launch.conversation_file;
            if file.is_file() {
                let kind = self
                    .agents
                    .get(&record.agent)
                    .map_err(|source| CarryError::Agent { agent: record.agent.clone(), source })?;
                let target = stage.join(AGENT);
                if project_keyed(kind) {
                    if let Some(project) = file.parent() {
                        copy_tree(project, &target)?;
                        agent_files = AgentFiles::Project;
                    }
                } else if let Some(file_name) = file.file_name() {
                    fs::create_dir_all(&target).map_err(io("create", &target))?;
                    fs::copy(file, target.join(file_name)).map_err(io("copy", file))?;
                    agent_files = AgentFiles::File { name: file_name.to_string_lossy().into_owned() };
                }
            }
        }

        // A folder freed by an archive is gone; restoring on another machine carries the records and the conversation alone.
        let mut items = if record.folder.exists() { items_under(Area::Folder, &record.folder)? } else { Vec::new() };
        let agent_dir = stage.join(AGENT);
        if agent_dir.is_dir() {
            items.extend(items_under(Area::Agent, &agent_dir)?);
        }
        let tabs = self.stage_shell_tabs(session, agent.as_ref().map(|agent| agent.tab.as_str()), &stage)?;
        let tabs_dir = stage.join(TABS);
        if tabs_dir.is_dir() {
            items.extend(items_under(Area::Tabs, &tabs_dir)?);
        }
        let (size, sha256) = sha_of_file(&stage.join(RECORDS)).map_err(io("read", &stage.join(RECORDS)))?;
        items.push(Item { area: Area::Records, rel: RECORDS.to_owned(), size, sha256, link: None });

        let manifest = Manifest {
            operation: operation.to_owned(),
            session: session.to_owned(),
            channel: channel.id.clone(),
            folder: format!("sessions/{name}"),
            old_folder: record.folder.to_string_lossy().into_owned(),
            agent: record.agent.clone(),
            conversation_id,
            agent_files,
            items: items.clone(),
            tabs,
        };
        let path = stage.join(MANIFEST);
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|source| CarryError::Manifest {
            path: path.clone(),
            source: ManifestProblem::Parse(source),
        })?;
        fs::write(&path, bytes).map_err(io("write", &path))?;
        Ok(Packed {
            folder: manifest.folder.clone(),
            stage: format!("{CARRY_DIR}/{operation}/{STAGE}"),
            digest: digest_of(&items),
            files: u32::try_from(items.len()).unwrap_or(u32::MAX),
            bytes: items.iter().map(|item| item.size).sum(),
            agent_running,
        })
    }

    /// `session`'s shell tabs on this machine, oldest first; `agent_tab` is
    /// the agent's own, which isn't one.
    fn shell_tabs(&self, session: &str, agent_tab: Option<&str>) -> Result<Vec<TabRecord>, CarryError> {
        let mut tabs: Vec<TabRecord> = self
            .store
            .tabs()
            .map_err(store("list the session's tabs"))?
            .into_iter()
            .filter(|tab| {
                tab.kind == TabKind::Shell && tab.session.as_deref() == Some(session) && agent_tab != Some(tab.id.as_str())
            })
            .collect();
        tabs.sort_by_key(|tab| (tab.created_unix_ms, tab.id.clone()));
        Ok(tabs)
    }

    /// Copies each shell tab's saved output into `stage`'s `tabs` folder
    /// as it stands now; its shell goes on running here until the move is
    /// released.
    fn stage_shell_tabs(&self, session: &str, agent_tab: Option<&str>, stage: &Path) -> Result<Vec<CarriedTab>, CarryError> {
        let tabs = self.shell_tabs(session, agent_tab)?;
        let target = stage.join(TABS);
        let mut carried = Vec::new();
        for tab in tabs {
            plain_tab_id(&tab.id)?;
            let saved = saved_scrollback_path(&tab.holder_socket);
            if saved.is_file() {
                fs::create_dir_all(&target).map_err(io("create", &target))?;
                match fs::copy(&saved, target.join(format!("{}.{SAVED}", tab.id))) {
                    Ok(_) => {}
                    // Its holder was shut down meanwhile, taking the file.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(CarryError::Io { doing: "copy", path: saved, source }),
                }
            }
            carried.push(CarriedTab { id: tab.id, title: tab.title, created_unix_ms: tab.created_unix_ms });
        }
        Ok(carried)
    }

    /// The holder sockets of `session`'s shell tabs here, so the machine a
    /// session left can stop its shells before it forgets their tabs.
    pub fn shell_sockets(&self, session: &str) -> Result<Vec<PathBuf>, CarryError> {
        let agent = self.store.agent(session).map_err(store("read the session's agent"))?;
        Ok(self
            .shell_tabs(session, agent.as_ref().map(|agent| agent.tab.as_str()))?
            .into_iter()
            .map(|tab| tab.holder_socket)
            .collect())
    }

    /// Saves each carried shell tab as a new tab of `session` here, its
    /// staged output put where its holder will start from. The shell
    /// itself starts when the agent resumes. An unpack done again first
    /// drops what the earlier one brought.
    fn bring_tabs(&self, manifest: &Manifest, stage: &Path) -> Result<(), CarryError> {
        self.drop_shell_tabs(&manifest.session)?;
        for carried in &manifest.tabs {
            plain_tab_id(&carried.id)?;
            let id = format!("tb_{}", uuid::Uuid::new_v4().simple());
            let socket = self.holders.join(format!("{id}.sock"));
            let staged = stage.join(TABS).join(format!("{}.{SAVED}", carried.id));
            if staged.is_file() {
                fs::create_dir_all(&self.holders).map_err(io("create", &self.holders))?;
                fs::copy(&staged, saved_scrollback_path(&socket)).map_err(io("copy", &staged))?;
            }
            let tab = TabRecord {
                id,
                session: Some(manifest.session.clone()),
                kind: TabKind::Shell,
                title: carried.title.clone(),
                holder_socket: socket,
                holder_pid: 0,
                created_unix_ms: carried.created_unix_ms,
            };
            self.store.put_tab(&tab).map_err(store("save a shell tab of the session"))?;
        }
        Ok(())
    }

    /// Forgets `session`'s shell tabs here and their saved output. Only for
    /// a session whose shells don't run here.
    fn drop_shell_tabs(&self, session: &str) -> Result<(), CarryError> {
        let agent = self.store.agent(session).map_err(store("read the session's agent"))?;
        for tab in self.shell_tabs(session, agent.as_ref().map(|agent| agent.tab.as_str()))? {
            remove(&saved_scrollback_path(&tab.holder_socket))?;
            self.store.remove_tab(&tab.id).map_err(store("remove a shell tab of the session"))?;
        }
        Ok(())
    }

    /// Where the incoming files of `operation` are, in `channel`'s folder:
    /// the staged files, which arrive as `in/stage`.
    fn incoming(&self, channel: &humata2_store::ChannelRecord, operation: &str) -> PathBuf {
        self.carry_dir(channel, operation).join(INCOMING).join(STAGE)
    }

    /// Checks the copy of a session's folder and staged files against the
    /// manifest, then puts it to use on this machine: the old folder's path
    /// is rewritten where the copy names it, the agent's files are placed
    /// where this machine's agent keeps them, and the records are brought in
    /// (`take`) with the agent saved as stopped, ready to resume. Nothing of the old machine's
    /// is touched. Blocking.
    pub fn unpack(&self, operation: &str, session: &str, channel: &str, take: Take) -> Result<Unpacked, CarryError> {
        let channel = self.carry_channel(channel)?;
        let stage = self.incoming(&channel, operation);
        let path = stage.join(MANIFEST);
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(&path).map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Read(source) })?,
        )
        .map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Parse(source) })?;
        let name = folder_name(&manifest.folder)?;
        let folder = self.layout.sessions(&channel.folder).join(name);

        let of = |area: Area| -> Vec<&Item> { manifest.items.iter().filter(|item| item.area == area).collect() };
        verify(Area::Folder, &folder, &of(Area::Folder))?;
        let agent_dir = stage.join(AGENT);
        if manifest.agent_files == AgentFiles::None {
            if agent_dir.exists() {
                return Err(CarryError::Unexpected { item: "the agent's files".to_owned() });
            }
        } else {
            verify(Area::Agent, &agent_dir, &of(Area::Agent))?;
        }
        verify(Area::Tabs, &stage.join(TABS), &of(Area::Tabs))?;
        let records = stage.join(RECORDS);
        let wanted = of(Area::Records);
        match fs::symlink_metadata(&records) {
            Ok(_) => {
                let (size, sha256) = sha_of_file(&records).map_err(io("read", &records))?;
                match wanted.first() {
                    Some(item) if item.size == size && item.sha256 == sha256 => {}
                    _ => return Err(CarryError::Changed { item: "the session's records".to_owned() }),
                }
            }
            Err(_) => return Err(CarryError::Missing { item: "the session's records".to_owned() }),
        }

        // Everything arrived as sent; now it is made this machine's.
        let new_folder = folder.to_string_lossy().into_owned();
        let pairs = replacements(&manifest.old_folder, &new_folder);
        if !pairs.is_empty()
            && let Ok(entries) = fs::read_dir(&folder)
        {
            for entry in entries.flatten() {
                if entry.path().join(".git").is_dir() {
                    rewrite_git(&entry.path(), &pairs)?;
                }
            }
        }
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let plan = kind.launch(&self.home, &folder, &manifest.conversation_id);
        match &manifest.agent_files {
            AgentFiles::None => {}
            AgentFiles::File { name } => {
                place(&agent_dir.join(name), &plan.conversation_file, &pairs)?;
            }
            AgentFiles::Project => {
                let Some(project) = plan.conversation_file.parent() else {
                    return Err(CarryError::BadFolder { path: plan.conversation_file.display().to_string() });
                };
                let mut files = Vec::new();
                collect_files(&agent_dir, "", &mut files)?;
                for rel in files {
                    place(&agent_dir.join(&rel), &project.join(&rel), &pairs)?;
                }
            }
        }

        self.store
            .import_session(session, &records, take)
            .map_err(store("bring the session's records in"))?;
        self.make_ready(&manifest, &folder, take)?;
        self.bring_tabs(&manifest, &stage)?;
        Ok(Unpacked {
            files: u32::try_from(manifest.items.len()).unwrap_or(u32::MAX),
            bytes: manifest.items.iter().map(|item| item.size).sum(),
        })
    }

    /// Points the imported records at this machine's folders and leaves the
    /// agent saved as one that should run, in a new tab, so the resume starts
    /// it in its conversation.
    fn make_ready(&self, manifest: &Manifest, folder: &Path, take: Take) -> Result<(), CarryError> {
        let session = &manifest.session;
        if take == Take::Everything {
            if let Some(mut record) = self.store.session(session).map_err(store("read the session"))? {
                record.folder = folder.to_owned();
                record.machine = String::new();
                self.store.put_session(&record).map_err(store("save the session"))?;
            }
            if let Some(mut detail) = self.store.session_detail(session).map_err(store("read the session's start"))? {
                for repo in &mut detail.repos {
                    repo.path = folder.join(&repo.name);
                }
                self.store.put_session_detail(&detail).map_err(store("save the session's start"))?;
            }
        }
        let Some(mut agent) = self.store.agent(session).map_err(store("read the session's agent"))? else {
            return Ok(());
        };
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let plan = kind.launch(&self.home, folder, &manifest.conversation_id);
        let tab = format!("tb_{}", uuid::Uuid::new_v4().simple());
        agent.launch.folder = folder.to_owned();
        agent.launch.argv = if agent.launch.argv == agent.launch.resume_argv {
            plan.resume_argv.clone()
        } else {
            plan.argv
        };
        agent.launch.resume_argv = plan.resume_argv;
        agent.launch.conversation_file = plan.conversation_file;
        agent.launch.holder_socket = self.holders.join(format!("{tab}.sock"));
        agent.tab = tab;
        // The old agent's holder is on the old machine. It is saved as
        // stopped, as a stopped agent is, so the host's resume starts it in
        // its conversation like any other.
        agent.state = AgentState::Stopped;
        self.store.put_agent(&agent).map_err(store("save the session's agent"))
    }

    /// Takes back what [`Starter::unpack`] and the copy put on this machine
    /// for `operation`: the copied folder `folder` (`sessions/<name>`), the
    /// agent's files that were placed, the staged files and the records
    /// brought in. Only for a move that never switched. A part that isn't
    /// there is already gone. Blocking.
    pub fn abort_incoming(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        folder: &str,
        take: Take,
    ) -> Result<(), CarryError> {
        let channel = self.carry_channel(channel)?;
        let name = folder_name(folder)?;
        let stage = self.incoming(&channel, operation);
        let mut first: Option<CarryError> = None;
        let mut keep = |result: Result<(), CarryError>| {
            if let Err(error) = result {
                first.get_or_insert(error);
            }
        };
        let had_records = stage.join(RECORDS).exists();
        // A copy whose agent was started here is the session's live one.
        if had_records
            && self
                .store
                .agent(session)
                .map_err(store("read the incoming session's agent"))?
                .is_some_and(|agent| matches!(agent.state, AgentState::Running { .. }))
        {
            return Err(CarryError::Running { session: session.to_owned() });
        }
        if let Ok(bytes) = fs::read(stage.join(MANIFEST))
            && let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes)
            && let Ok(kind) = self.agents.get(&manifest.agent)
        {
            let new_folder = self.layout.sessions(&channel.folder).join(name);
            let plan = kind.launch(&self.home, &new_folder, &manifest.conversation_id);
            match manifest.agent_files {
                AgentFiles::None => {}
                AgentFiles::File { .. } => keep(remove(&plan.conversation_file)),
                AgentFiles::Project => {
                    if let Some(project) = plan.conversation_file.parent() {
                        keep(remove(project));
                    }
                }
            }
        }
        keep(remove(&self.layout.sessions(&channel.folder).join(name)));
        keep(remove(&self.carry_dir(&channel, operation)));
        if had_records {
            keep(self.drop_shell_tabs(session));
            keep(match take {
                Take::Everything => self.store.remove_session(session).map_err(store("remove the incoming session")),
                Take::AgentOnly => {
                    self.store.remove_agent_and_tabs(session).map_err(store("remove the incoming session's agent"))
                }
            });
        }
        first.map_or(Ok(()), Err)
    }

    /// What the old machine puts in its trash once the session runs on the
    /// new one: its folder, then the agent's files. A part that isn't there
    /// is left out.
    pub fn discards(&self, session: &str) -> Result<Vec<Discard>, CarryError> {
        let record = self
            .store
            .session(session)
            .map_err(store("read the session"))?
            .ok_or_else(|| CarryError::NoSession { session: session.to_owned() })?;
        let mut found = Vec::new();
        if record.folder.exists() {
            found.push(Discard { path: record.folder.clone(), what: format!("session folder of {}", record.title) });
        }
        if let Some(agent) = self.store.agent(session).map_err(store("read the session's agent"))? {
            let file = agent.launch.conversation_file;
            let project = self.agents.get(&record.agent).ok().is_some_and(project_keyed);
            let path = if project { file.parent().map(Path::to_owned) } else { Some(file) };
            if let Some(path) = path.filter(|path| path.exists()) {
                found.push(Discard { path, what: format!("conversation of {}", record.title) });
            }
        }
        Ok(found)
    }

    /// Forgets the session on the machine it left: its staged files, and its
    /// records as `take` says (a box drops the session, the host keeps its
    /// own record and drops the agent). Blocking.
    pub fn forget_moved(&self, operation: &str, session: &str, channel: &str, take: Take) -> Result<(), CarryError> {
        let channel = self.carry_channel(channel)?;
        remove(&self.carry_dir(&channel, operation))?;
        match take {
            Take::Everything => self.store.remove_session(session).map_err(store("forget the moved session")),
            Take::AgentOnly => self.store.remove_agent_and_tabs(session).map_err(store("forget the moved session's agent")),
        }
    }

    /// Removes the staged files of `operation` on the machine that packed.
    pub fn unstage(&self, operation: &str, channel: &str) -> Result<(), CarryError> {
        let channel = self.carry_channel(channel)?;
        remove(&self.carry_dir(&channel, operation))
    }
}

/// A tab id names files, so it must be one plain name.
fn plain_tab_id(id: &str) -> Result<(), CarryError> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(CarryError::Unexpected { item: format!("a shell tab named {id:?}") })
    }
}

fn remove(path: &Path) -> Result<(), CarryError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path).map_err(io("remove", path)),
        Ok(_) => fs::remove_file(path).map_err(io("remove", path)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io("look at", path)(source)),
    }
}

fn collect_files(root: &Path, prefix: &str, found: &mut Vec<String>) -> Result<(), CarryError> {
    for entry in fs::read_dir(root).map_err(io("read the folder", root))? {
        let entry = entry.map_err(io("read the folder", root))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, &rel, found)?;
        } else {
            found.push(rel);
        }
    }
    Ok(())
}

/// Copies a staged agent file to where this machine's agent keeps it,
/// rewriting the old folder's path in it, through a `.partial` so a cut
/// leaves no half file under the real name.
fn place(from: &Path, to: &Path, pairs: &[(String, String)]) -> Result<(), CarryError> {
    if to.exists() {
        return Err(CarryError::Exists { what: "conversation", path: to.to_owned() });
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(io("create", parent))?;
    }
    let temporary = partial(to);
    let bytes = fs::read(from).map_err(io("read", from))?;
    let now = rewritten(&bytes, pairs).unwrap_or(bytes);
    fs::write(&temporary, now).map_err(io("write", &temporary))?;
    fs::rename(&temporary, to).map_err(io("place", to))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use humata2_revive::{ReviveConfig, RestartReports};
    use humata2_store::{
        AgentLaunch, AgentRecord, ChannelRecord, Store, ChannelRepo, MainClone, SessionDetail, SessionRecord, SessionRepo,
        StartStep,
    };

    use super::*;
    use crate::agents::Agents;
    use crate::layout::Layout;

    /// A machine under `root`: a channel `payments`, and agents whose
    /// conversations are in `<root>/conv`, or in a project folder per
    /// session folder when `project` is set.
    fn machine(root: &Path, project: bool) -> Starter {
        let store = Arc::new(Store::open(root.join("state")).unwrap());
        store
            .put_channel(&ChannelRecord {
                id: "ch_1".to_owned(),
                name: "Payments".to_owned(),
                folder: "payments".to_owned(),
                branch_prefix: None,
                default_agent: "fake".to_owned(),
                repos: vec![ChannelRepo { owner: "acme".to_owned(), name: "api".to_owned(), main: MainClone::Ready }],
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 1,
            })
            .unwrap();
        let file = if project { "{home}/projects/{folder_dashed}/{conversation}.jsonl" } else { "{home}/conv/{conversation}.jsonl" };
        let agents = format!(
            r#"[{{"name":"fake","program":"/bin/sh","start_args":["--session-id","{{conversation}}"],"resume_args":["--resume","{{conversation}}"],"conversation_file":"{file}"}}]"#
        );
        Starter {
            store,
            layout: Layout { root: root.join("Humata") },
            remote_template: String::new(),
            agents: Agents::from_json(agents.as_bytes()).unwrap(),
            home: root.to_owned(),
            revive: ReviveConfig {
                contain: None,
                holder: PathBuf::from("/nonexistent"),
                logs: root.join("logs"),
                scrollback_bytes: 1 << 16,
                shell: PathBuf::from("/bin/sh"),
                shell_args: Vec::new(),
                home: root.to_owned(),
                resume_timeout: Duration::from_secs(1),
                settle: Duration::from_millis(10),
                reports: RestartReports {
                    folder: root.join("restarts"),
                    version: "0".to_owned(),
                    machine: "ma".to_owned(),
                    os: "test".to_owned(),
                    daemon_log: None,
                },
            },
            holders: root.join("h"),
        }
    }

    /// Session `se_1` on `starter`, with a clone holding a config that names
    /// the folder, a working file and the agent's conversation.
    fn session(starter: &Starter) -> PathBuf {
        let folder = starter.layout.session("payments", "fix");
        let clone = folder.join("api");
        fs::create_dir_all(clone.join(".git")).unwrap();
        fs::write(clone.join(".git/config"), format!("[remote]\n\turl = {}/x\n", folder.display())).unwrap();
        fs::write(clone.join("work.txt"), "dirty\n").unwrap();
        fs::write(folder.join("PLAN.md"), "# plan\n").unwrap();
        starter
            .store
            .put_session(&SessionRecord {
                id: "se_1".to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "Fix".to_owned(),
                agent: "fake".to_owned(),
                folder: folder.clone(),
                created_unix_ms: 1,
            })
            .unwrap();
        starter
            .store
            .put_session_detail(&SessionDetail {
                session: "se_1".to_owned(),
                repos: vec![SessionRepo {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                    branch: "fix".to_owned(),
                    path: clone,
                }],
                model: None,
                first_message: None,
                start: StartStep::Done,
                operation: None,
                revision: 1,
            })
            .unwrap();
        let plan = starter.agents.get("fake").unwrap().launch(&starter.home, &folder, "conv-1");
        fs::create_dir_all(plan.conversation_file.parent().unwrap()).unwrap();
        fs::write(&plan.conversation_file, format!("{{\"cwd\":\"{}\"}}\nhello\n", folder.display())).unwrap();
        starter
            .store
            .put_agent(&AgentRecord {
                session: "se_1".to_owned(),
                tab: "tb_old".to_owned(),
                launch: AgentLaunch {
                    agent: "fake".to_owned(),
                    folder: folder.clone(),
                    argv: plan.argv,
                    resume_argv: plan.resume_argv,
                    conversation_id: "conv-1".to_owned(),
                    conversation_file: plan.conversation_file,
                    holder_socket: starter.holders.join("tb_old.sock"),
                    ..AgentLaunch::default()
                },
                state: AgentState::Stopped,
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: 1,
            })
            .unwrap();
        folder
    }

    /// What the engine does between two machines: the session's folder and
    /// the staged files appear under the new machine's channel.
    fn copy_over(old: &Starter, new: &Starter, folder: &Path) {
        let target = new.layout.sessions("payments").join("fix");
        copy_tree(folder, &target).unwrap();
        let staged = old.layout.channel("payments").join(CARRY_DIR).join("op_1").join(STAGE);
        copy_tree(&staged, &new.incoming(&new.carry_channel("ch_1").unwrap(), "op_1")).unwrap();
    }

    #[test]
    fn a_packed_session_unpacks_on_another_machine_with_its_paths_made_its_own() {
        for project in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (old, new) = (machine(&dir.path().join("old"), project), machine(&dir.path().join("new"), project));
            let folder = session(&old);

            let packed = old.pack("op_1", "se_1").unwrap();
            copy_over(&old, &new, &folder);
            new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

            let moved = new.layout.session("payments", "fix");
            assert_eq!(packed.folder, "sessions/fix");
            let config = fs::read_to_string(moved.join("api/.git/config")).unwrap();
            assert!(config.contains(&moved.display().to_string()) && !config.contains("/old/"), "{config}");
            assert_eq!(fs::read_to_string(moved.join("api/work.txt")).unwrap(), "dirty\n", "working files are left as they are");
            let agent = new.store.agent("se_1").unwrap().unwrap();
            assert_eq!(agent.state, AgentState::Stopped);
            assert!(agent.launch.conversation_file.starts_with(dir.path().join("new")));
            let talk = fs::read_to_string(&agent.launch.conversation_file).unwrap();
            assert!(talk.contains(&moved.display().to_string()) && talk.ends_with("hello\n"), "{talk}");
            assert_eq!(new.store.session("se_1").unwrap().unwrap().folder, moved);
            assert_eq!(new.store.session_detail("se_1").unwrap().unwrap().repos[0].path, moved.join("api"));
        }
    }

    /// A shell tab of `se_1` on `starter` whose holder saved `output`, and
    /// the agent's own tab.
    fn shell_tab(starter: &Starter, id: &str, output: &[u8]) -> PathBuf {
        let socket = starter.holders.join(format!("{id}.sock"));
        fs::create_dir_all(&starter.holders).unwrap();
        fs::write(saved_scrollback_path(&socket), output).unwrap();
        for (tab, kind) in [(id, TabKind::Shell), ("tb_old", TabKind::Agent)] {
            starter
                .store
                .put_tab(&TabRecord {
                    id: tab.to_owned(),
                    session: Some("se_1".to_owned()),
                    kind,
                    title: format!("{tab} title"),
                    holder_socket: starter.holders.join(format!("{tab}.sock")),
                    holder_pid: 7,
                    created_unix_ms: 5,
                })
                .unwrap();
        }
        socket
    }

    fn shells_of(starter: &Starter) -> Vec<TabRecord> {
        starter.store.tabs().unwrap().into_iter().filter(|tab| tab.kind == TabKind::Shell).collect()
    }

    #[test]
    fn a_shell_tab_arrives_as_a_new_tab_of_the_session_starting_from_its_saved_output() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (machine(&dir.path().join("old"), false), machine(&dir.path().join("new"), false));
        let folder = session(&old);
        let socket = shell_tab(&old, "tb_sh", b"saved shell output");

        let packed = old.pack("op_1", "se_1").unwrap();
        fs::write(saved_scrollback_path(&socket), b"saved shell output, and more").unwrap();
        assert_eq!(old.pack("op_1", "se_1").unwrap().digest, packed.digest, "a shell printing doesn't count as a change");
        old.pack("op_1", "se_1").unwrap();
        copy_over(&old, &new, &folder);
        new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();

        let shells = shells_of(&new);
        assert_eq!(shells.len(), 1, "{shells:?}");
        let arrived = &shells[0];
        assert_eq!(arrived.session.as_deref(), Some("se_1"));
        assert_eq!(arrived.title, "tb_sh title");
        assert_ne!(arrived.id, "tb_sh", "the new machine names its own tab");
        assert!(arrived.holder_socket.starts_with(&new.holders));
        assert_eq!(fs::read(saved_scrollback_path(&arrived.holder_socket)).unwrap(), b"saved shell output, and more");
        assert_eq!(old.shell_sockets("se_1").unwrap(), vec![socket], "the old machine can stop its shell");
    }

    #[test]
    fn taking_an_incoming_copy_back_drops_the_shell_tabs_it_brought() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (machine(&dir.path().join("old"), false), machine(&dir.path().join("new"), false));
        let folder = session(&old);
        shell_tab(&old, "tb_sh", b"saved");
        old.pack("op_1", "se_1").unwrap();
        copy_over(&old, &new, &folder);
        new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();
        let saved = saved_scrollback_path(&shells_of(&new)[0].holder_socket);

        new.abort_incoming("op_1", "se_1", "ch_1", "sessions/fix", Take::Everything).unwrap();

        assert!(shells_of(&new).is_empty());
        assert!(!saved.exists());
        assert!(old.layout.session("payments", "fix").exists() && shells_of(&old).len() == 1, "the old machine keeps its own");
    }

    #[test]
    fn a_file_that_changed_on_the_way_is_named_and_nothing_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (machine(&dir.path().join("old"), false), machine(&dir.path().join("new"), false));
        let folder = session(&old);
        old.pack("op_1", "se_1").unwrap();
        copy_over(&old, &new, &folder);
        fs::write(new.layout.session("payments", "fix").join("PLAN.md"), "# another plan\n").unwrap();

        let refused = new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap_err();

        assert!(matches!(&refused, CarryError::Changed { item } if item == "PLAN.md"), "{refused:?}");
        assert!(new.store.session("se_1").unwrap().is_none(), "no record was made");
    }

    #[test]
    fn a_file_that_never_arrived_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (machine(&dir.path().join("old"), false), machine(&dir.path().join("new"), false));
        let folder = session(&old);
        old.pack("op_1", "se_1").unwrap();
        copy_over(&old, &new, &folder);
        fs::remove_file(new.layout.session("payments", "fix").join("api/work.txt")).unwrap();

        let refused = new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap_err();

        assert!(matches!(&refused, CarryError::Missing { item } if item == "api/work.txt"), "{refused:?}");
    }

    #[test]
    fn packing_an_unchanged_session_twice_gives_the_same_digest_and_a_change_gives_another() {
        let dir = tempfile::tempdir().unwrap();
        let old = machine(dir.path(), false);
        let folder = session(&old);

        let first = old.pack("op_1", "se_1").unwrap();
        let again = old.pack("op_1", "se_1").unwrap();
        fs::write(folder.join("PLAN.md"), "# changed\n").unwrap();
        let changed = old.pack("op_1", "se_1").unwrap();

        assert_eq!(first.digest, again.digest);
        assert_ne!(first.digest, changed.digest);
    }

    #[test]
    fn taking_an_incoming_copy_back_leaves_nothing_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (machine(&dir.path().join("old"), false), machine(&dir.path().join("new"), false));
        let folder = session(&old);
        old.pack("op_1", "se_1").unwrap();
        copy_over(&old, &new, &folder);
        new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();
        let conversation = new.store.agent("se_1").unwrap().unwrap().launch.conversation_file;

        new.abort_incoming("op_1", "se_1", "ch_1", "sessions/fix", Take::Everything).unwrap();

        assert!(!new.layout.session("payments", "fix").exists());
        assert!(!conversation.exists());
        assert!(!new.layout.channel("payments").join(CARRY_DIR).join("op_1").exists());
        assert!(new.store.session("se_1").unwrap().is_none());
        assert!(folder.exists(), "the old machine's copy is untouched");
    }

    #[test]
    fn an_incoming_copy_whose_agent_runs_here_is_not_taken_back() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (machine(&dir.path().join("old"), false), machine(&dir.path().join("new"), false));
        let folder = session(&old);
        old.pack("op_1", "se_1").unwrap();
        copy_over(&old, &new, &folder);
        new.unpack("op_1", "se_1", "ch_1", Take::Everything).unwrap();
        let mut agent = new.store.agent("se_1").unwrap().unwrap();
        agent.state = AgentState::Running { holder_pid: 7 };
        new.store.put_agent(&agent).unwrap();

        let refused = new.abort_incoming("op_1", "se_1", "ch_1", "sessions/fix", Take::Everything).unwrap_err();

        assert!(matches!(&refused, CarryError::Running { session } if session == "se_1"), "{refused:?}");
        assert!(new.layout.session("payments", "fix").exists(), "the running copy is kept");
        assert!(new.store.session("se_1").unwrap().is_some());
    }

    #[test]
    fn the_old_machine_names_its_folder_and_conversation_to_discard() {
        let dir = tempfile::tempdir().unwrap();
        let old = machine(dir.path(), false);
        let folder = session(&old);

        let found = old.discards("se_1").unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, folder);
        assert!(found[1].path.ends_with("conv-1.jsonl"));
    }

    #[test]
    fn a_folder_name_must_be_one_plain_name_under_sessions() {
        assert!(folder_name("sessions/fix").is_ok());
        for bad in ["fix", "sessions/", "sessions/a/b", "sessions/..", "other/fix", ""] {
            assert!(folder_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn replacements_cover_the_path_and_the_form_an_agent_names_a_project_folder_by() {
        let pairs = replacements("/old/a b", "/new/x");
        let text = rewritten(br#"{"cwd":"/old/a b","project":"-old-a b/c"}"#, &pairs).unwrap();
        assert_eq!(String::from_utf8(text).unwrap(), r#"{"cwd":"/new/x","project":"-new-x/c"}"#);
        assert!(rewritten(b"nothing here", &pairs).is_none());
        assert!(replacements("/same", "/same").is_empty());
    }
}
