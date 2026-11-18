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
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use humata2_conversation::{Conversation, ConversationError, GitHead, Placement, adapter};
use humata2_holder::saved_scrollback_path;
use humata2_store::{AgentState, StoreError, TabKind, TabRecord, Take};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agents::AgentKind;
use crate::error::AgentError;
use crate::layout::{make_dirs, partial};
use crate::start::Starter;
use crate::tree_copy::{self, Dirs, Rebase, TreeCopyError};

#[path = "carry_fork.rs"]
mod staged_fork;
pub use staged_fork::{ForkPack, ForkPackError, ForkPacked};

#[cfg(test)]
#[path = "carry_roam.rs"]
mod roam_tests;

#[cfg(test)]
#[path = "carry_reid_tests.rs"]
mod reid_tests;

#[path = "carry_cheap.rs"]
mod cheap;
#[path = "carry_desk.rs"]
mod desk_part;
#[path = "carry_landing.rs"]
mod landing;
#[cfg(test)]
#[path = "carry_landing_tests.rs"]
mod landing_tests;
pub use cheap::CLONES_FILE;

#[path = "carry_chunks.rs"]
mod chunks;
pub use chunks::{CHUNKS_DIR, CHUNKS_FILE, CHUNK_SIZE, HAVE_DIR, REUSE_FILE};

#[path = "carry_placed.rs"]
mod placed;
pub use desk_part::CHANNELS_ROOT;
pub use landing::Landing;

/// The folder under a channel's own where an operation's staged files wait.
pub const CARRY_DIR: &str = ".carry";

pub(crate) const STAGE: &str = "stage";
const INCOMING: &str = "in";
pub(crate) const MANIFEST: &str = "manifest.json";
pub(crate) const RECORDS: &str = "records.sqlite";
pub(crate) const AGENT: &str = "agent";
pub(crate) const TABS: &str = "tabs";
const SAVED: &str = "scrollback";

/// Where, under the agent folder of a stage, an earlier conversation is
/// staged: `previous-<n>`.
const PREVIOUS: &str = "previous-";

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

    #[error(
        "the session's folder {} isn't a scratch folder of the Desk at {}, so Humata won't move it; \
         nothing was changed. A folder you picked stays where it is",
        folder.display(),
        desk.display()
    )]
    NotADeskFolder { folder: PathBuf, desk: PathBuf },

    #[error(
        "couldn't rebuild the clone {name} on this machine: {source}. Nothing was deleted; check that this machine \
         can reach the repository's remote, that its main clone of the channel is whole and that the session's \
         unpushed commits came along, then try the move again"
    )]
    Rebuild {
        name: String,
        #[source]
        source: crate::error::GitError,
    },

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

    /// The agent's conversation adapter refused it (MV-16, D336), e.g. a
    /// record or CLI version it doesn't know, before anything was staged or
    /// placed.
    #[error("couldn't carry the session's {agent} conversation: {source}. Nothing was copied or deleted")]
    Conversation {
        agent: String,
        #[source]
        source: humata2_conversation::ConversationError,
    },

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
    /// The conversation's own files, as its adapter packed them (MV-16):
    /// staged under `agent/`, placed on the new machine by the adapter.
    /// `old_home` and `old_cli_home` are where the old machine's CLI ran.
    Conversation {
        packed: humata2_conversation::Packed,
        old_home: PathBuf,
        old_cli_home: Option<PathBuf>,
        /// The conversations the session left behind (PER-15), each staged
        /// under `agent/previous-<n>`.
        #[serde(default)]
        previous: Vec<PreviousPacked>,
        /// The folder the staged conversation was already written for (MV-17):
        /// the new machine places it from there, not from the old folder.
        /// Absent in a manifest from before: placed from the old folder.
        #[serde(default)]
        placed_for: Option<PathBuf>,
    },
    /// The agent's current conversation has no files, but conversations it
    /// left behind do: they travel, each under `agent/previous-<n>`.
    Earlier { old_home: PathBuf, old_cli_home: Option<PathBuf>, previous: Vec<PreviousPacked> },
}

/// An earlier conversation of a carried session, as packed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousPacked {
    pub packed: humata2_conversation::Packed,
    /// The path the old machine's records name it by.
    pub old_path: PathBuf,
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
    items_skipping(area, root, &[])
}

/// As [`items_under`], leaving out what `skip` names, `/`-joined under
/// `root`: a clone's `.git`, which a move rebuilds instead of copying.
pub(crate) fn items_skipping(area: Area, root: &Path, skip: &[String]) -> Result<Vec<Item>, CarryError> {
    let mut found = Vec::new();
    walk(area, root, "", skip, &mut found)?;
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(found)
}

fn walk(area: Area, folder: &Path, prefix: &str, skip: &[String], found: &mut Vec<Item>) -> Result<(), CarryError> {
    let entries = fs::read_dir(folder).map_err(io("read the folder", folder))?;
    for entry in entries {
        let entry = entry.map_err(io("read the folder", folder))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let meta = fs::symlink_metadata(&path).map_err(io("look at", &path))?;
        if skip.contains(&rel) {
            continue;
        }
        if meta.is_dir() {
            walk(area, &path, &rel, skip, found)?;
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
    verify_skipping(area, root, wanted, &[])
}

/// As [`verify`], leaving out what `skip` names.
pub(crate) fn verify_skipping(area: Area, root: &Path, wanted: &[&Item], skip: &[String]) -> Result<(), CarryError> {
    let actual = if wanted.is_empty() && !root.exists() { Vec::new() } else { items_skipping(area, root, skip)? };
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

/// `path` as Claude Code names its per-folder record.
fn dashed(path: &str) -> String {
    humata2_conversation::claude_project_name(Path::new(path))
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
        write_durably(file, &now, meta.permissions()).map_err(io("replace", file))?;
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
            // A linked worktree's own folder (`gitdir`, `HEAD`, ...) names the
            // worktree's place.
            "worktrees" => {
                let Ok(worktrees) = fs::read_dir(&path) else { continue };
                for worktree in worktrees.flatten() {
                    let Ok(files) = fs::read_dir(worktree.path()) else { continue };
                    for file in files.flatten() {
                        rewrite_file(&file.path(), pairs)?;
                    }
                }
            }
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

/// Copies `from` to `to`: its folders, its files with their modified
/// times, and its links as links (see [`Rebase::target`]). A socket or other
/// special file is left out, as no copy could carry it.
fn copy_tree(from: &Path, to: &Path) -> Result<(), CarryError> {
    tree_copy::copy_tree(from, to, to, Dirs::Default).map_err(CarryError::from)
}

impl From<TreeCopyError> for CarryError {
    fn from(error: TreeCopyError) -> Self {
        CarryError::Io { doing: error.doing, path: error.path, source: error.source }
    }
}

/// `wanted`, the folder's items as packed, as the landed `folder` holds
/// them: a link into the old folder `old` that the copy pointed into
/// `folder` instead is checked as so pointing.
fn as_landed(wanted: &[&Item], old: &Path, folder: &Path) -> Vec<Item> {
    let rebase = Rebase { from: old, to: folder };
    wanted
        .iter()
        .map(|item| {
            let Some(link) = &item.link else {
                return (*item).clone();
            };
            let moved = rebase.target(Path::new(link));
            let landed = fs::read_link(folder.join(&item.rel)).ok();
            if moved.as_path() != Path::new(link) && landed.as_deref() == Some(moved.as_path()) {
                let text = moved.to_string_lossy().into_owned();
                Item { sha256: sha_of_text(&text), link: Some(text), ..(*item).clone() }
            } else {
                (*item).clone()
            }
        })
        .collect()
}

/// Where the agent keeps files per session folder rather than one file.
fn project_keyed(kind: &AgentKind) -> bool {
    kind.conversation_file.contains("{folder")
}

impl Starter {
    fn carry_channel(&self, channel: &str) -> Result<humata2_store::ChannelRecord, CarryError> {
        if channel == CHANNELS_ROOT {
            return Ok(self.channels_root_record());
        }
        self.store
            .channel(channel)
            .map_err(store("look up the channel"))?
            .ok_or_else(|| CarryError::NoChannel { channel: channel.to_owned() })
    }

    /// The folder an operation's files wait in, in `channel`'s folder.
    fn carry_dir(&self, channel: &humata2_store::ChannelRecord, operation: &str) -> PathBuf {
        self.layout.channel(&channel.folder).join(CARRY_DIR).join(operation)
    }

    /// The conversation of `agent` as its adapter finds it on this machine.
    /// `None` for a kind no adapter knows, and for a conversation whose file
    /// isn't there yet. A conversation the adapter can't read is its refusal.
    fn locate_conversation(
        &self,
        record: &humata2_store::SessionRecord,
        agent: &humata2_store::AgentRecord,
    ) -> Result<Option<Conversation>, ConversationError> {
        let Ok(kind) = self.agents.get(&record.agent) else {
            return Ok(None);
        };
        let Some(format) = kind.conversation_format.adapter_format() else {
            return Ok(None);
        };
        let Some(at) = kind.placement(&self.home, &record.folder) else {
            return Ok(None);
        };
        match adapter(format).locate(&at, &agent.launch.conversation_id) {
            Ok(found) => Ok(Some(found)),
            Err(error @ ConversationError::Unsupported { .. }) => Err(error),
            Err(_) if !agent.launch.conversation_file.is_file() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// What the old machine discards of `agent`'s conversation, file by
    /// file (a folder named for the conversation as one), never a file of
    /// another conversation. `None` when the adapter can't say, so the
    /// agent's file is discarded as it was.
    fn conversation_discards(
        &self,
        record: &humata2_store::SessionRecord,
        agent: &humata2_store::AgentRecord,
    ) -> Result<Option<Vec<Discard>>, CarryError> {
        let formatted = self
            .agents
            .get(&record.agent)
            .is_ok_and(|kind| kind.conversation_format.adapter_format().is_some());
        if !formatted {
            return Ok(None);
        }
        let current = match self.locate_conversation(record, agent) {
            Ok(found) => found,
            Err(ConversationError::Unsupported { .. }) => return Ok(None),
            Err(ConversationError::NotFound { .. }) => None,
            Err(source) => return Err(CarryError::Conversation { agent: record.agent.clone(), source }),
        };
        let mut all: Vec<(Conversation, String)> =
            current.into_iter().map(|found| (found, agent.launch.conversation_id.clone())).collect();
        let earlier = self
            .store
            .previous_conversations(&record.id)
            .map_err(store("read the session's earlier conversations"))?;
        for one in earlier {
            if let Some(found) = self.locate_by_id(record, &one.conversation_id) {
                all.push((found, one.conversation_id));
            }
        }
        let mut paths = std::collections::BTreeSet::new();
        for (found, id) in &all {
            for file in &found.files {
                let grouped =
                    file.path.ancestors().skip(1).find(|up| up.file_name().is_some_and(|name| name == id.as_str()));
                paths.insert(grouped.unwrap_or(file.path.as_path()).to_owned());
            }
        }
        Ok(Some(
            paths
                .into_iter()
                .filter(|path| path.exists())
                .map(|path| Discard { path, what: format!("conversation of {}", record.title) })
                .collect(),
        ))
    }

    /// Places the conversation an adapter packed, where the CLI at `folder`
    /// looks for it, on the head of the rebuilt clone when there is one.
    /// The path of its transcript; `None` for another kind of agent files.
    fn place_conversation(
        &self,
        manifest: &Manifest,
        folder: &Path,
        agent_dir: &Path,
        head: Option<&GitHead>,
    ) -> Result<Option<PathBuf>, CarryError> {
        let AgentFiles::Conversation { packed, old_home, old_cli_home, placed_for, .. } = &manifest.agent_files else {
            return Ok(None);
        };
        let refused = |source: ConversationError| CarryError::Conversation { agent: manifest.agent.clone(), source };
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let Some(to) = kind.placement(&self.home, folder) else {
            return Err(refused(ConversationError::Unsupported { format: packed.format, version: None }));
        };
        // A conversation staged already written for this folder is placed from
        // it, so none of the folder's paths is rewritten twice (MV-17).
        let from = Placement {
            home: old_home.clone(),
            cli_home: old_cli_home.clone(),
            folder: placed_for.clone().unwrap_or_else(|| PathBuf::from(&manifest.old_folder)),
        };
        let mut packed = packed.clone();
        packed.stage = agent_dir.to_owned();
        let placed = adapter(packed.format).place_on_head(&packed, &from, &to, head).map_err(refused)?;
        Ok(placed.files.first().map(|file| file.path.clone()))
    }

    /// The conversation `id` of `record`'s agent as its adapter finds it
    /// here; `None` when it isn't there.
    fn locate_by_id(&self, record: &humata2_store::SessionRecord, id: &str) -> Option<Conversation> {
        let kind = self.agents.get(&record.agent).ok()?;
        let format = kind.conversation_format.adapter_format()?;
        let at = kind.placement(&self.home, &record.folder)?;
        adapter(format).locate(&at, id).ok()
    }

    /// Stages the conversations `record`'s session left behind that are
    /// still on this machine, each under `agent/previous-<n>`, and names
    /// them in `files`. Per-folder files (Claude's memory) travel with the
    /// current conversation only.
    fn stage_previous(
        &self,
        record: &humata2_store::SessionRecord,
        stage: &Path,
        files: AgentFiles,
    ) -> Result<AgentFiles, CarryError> {
        let placed_for = match &files {
            AgentFiles::Conversation { placed_for, .. } => placed_for.clone(),
            _ => None,
        };
        let (current, old_home, old_cli_home, mut previous) = match files {
            AgentFiles::Conversation { packed, old_home, old_cli_home, previous, .. } => {
                (Some(packed), old_home, old_cli_home, previous)
            }
            AgentFiles::None => {
                let Ok(kind) = self.agents.get(&record.agent) else {
                    return Ok(AgentFiles::None);
                };
                let Some(at) = kind.placement(&self.home, &record.folder) else {
                    return Ok(AgentFiles::None);
                };
                (None, at.home, at.cli_home, Vec::new())
            }
            other => return Ok(other),
        };
        let earlier = self
            .store
            .previous_conversations(&record.id)
            .map_err(store("read the session's earlier conversations"))?;
        for (n, one) in earlier.iter().enumerate() {
            if current.as_ref().is_some_and(|packed| packed.id == one.conversation_id) {
                continue;
            }
            let Some(found) = self.locate_by_id(record, &one.conversation_id) else {
                continue;
            };
            let target = stage.join(AGENT).join(format!("{PREVIOUS}{n}"));
            let mut staged = adapter(found.format).pack(&found, &target).map_err(|source| {
                let _ = fs::remove_dir_all(stage);
                CarryError::Conversation { agent: record.agent.clone(), source }
            })?;
            staged.files.retain(|file| file.role != humata2_conversation::FileRole::Project);
            previous.push(PreviousPacked { packed: staged, old_path: one.path.clone() });
        }
        Ok(match current {
            Some(packed) => AgentFiles::Conversation { packed, old_home, old_cli_home, previous, placed_for },
            None if previous.is_empty() => AgentFiles::None,
            None => AgentFiles::Earlier { old_home, old_cli_home, previous },
        })
    }

    /// Places the earlier conversations of a carried session where the CLI
    /// at `folder` looks for them. Each is the exact move of the path the
    /// records named to the file placed, so the store rebases them without
    /// guessing how the CLI renamed the project folder.
    fn place_previous(
        &self,
        manifest: &Manifest,
        folder: &Path,
        agent_dir: &Path,
    ) -> Result<Vec<(PathBuf, PathBuf)>, CarryError> {
        let (AgentFiles::Conversation { old_home, old_cli_home, previous, .. }
        | AgentFiles::Earlier { old_home, old_cli_home, previous }) = &manifest.agent_files
        else {
            return Ok(Vec::new());
        };
        if previous.is_empty() {
            return Ok(Vec::new());
        }
        let refused = |source: ConversationError| CarryError::Conversation { agent: manifest.agent.clone(), source };
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let Some(to) = kind.placement(&self.home, folder) else {
            return Ok(Vec::new());
        };
        let from =
            Placement { home: old_home.clone(), cli_home: old_cli_home.clone(), folder: PathBuf::from(&manifest.old_folder) };
        let mut moves = Vec::new();
        for (n, one) in previous.iter().enumerate() {
            let mut packed = one.packed.clone();
            packed.stage = agent_dir.join(format!("{PREVIOUS}{n}"));
            let placed = adapter(packed.format).place(&packed, &from, &to).map_err(refused)?;
            if let Some(file) = placed.files.first() {
                moves.push((one.old_path.clone(), file.path.clone()));
            }
        }
        Ok(moves)
    }

    /// Takes back the files [`Starter::place_conversation`] put under
    /// `folder`. A Codex conversation rewritten in place on the same home is
    /// the old machine's own and stays.
    fn remove_placed(&self, kind: &AgentKind, files: &AgentFiles, folder: &Path) -> Result<(), CarryError> {
        let (current, old_home, old_cli_home, previous) = match files {
            AgentFiles::Conversation { packed, old_home, old_cli_home, previous, .. } => {
                (Some(packed), old_home, old_cli_home, previous)
            }
            AgentFiles::Earlier { old_home, old_cli_home, previous } => (None, old_home, old_cli_home, previous),
            _ => return Ok(()),
        };
        let Some(packed) = current.or(previous.first().map(|one| &one.packed)) else {
            return Ok(());
        };
        let Some(at) = kind.placement(&self.home, folder) else {
            return Ok(());
        };
        if packed.format == humata2_conversation::Format::CodexRollout
            && at.home == *old_home
            && at.cli_home == *old_cli_home
        {
            return Ok(());
        }
        let ids = std::iter::once(&packed.id).chain(previous.iter().map(|one| &one.packed.id));
        for id in ids {
            if let Ok(found) = adapter(packed.format).locate(&at, id) {
                for file in &found.files {
                    remove(&file.path)?;
                }
            }
        }
        let plan = kind.launch(&self.home, folder, &packed.id);
        if project_keyed(kind)
            && let Some(project) = plan.conversation_file.parent()
        {
            remove(project)?;
        }
        Ok(())
    }

    /// Stages `session` for a move: its records, the agent's files and the
    /// manifest, under `<channel>/.carry/<operation>/stage`. The session's
    /// folder isn't copied or changed. Blocking; safe to repeat.
    pub fn pack(&self, operation: &str, session: &str) -> Result<Packed, CarryError> {
        self.pack_for(operation, session, None)
    }

    /// As [`Starter::pack`]. With `target`, the folder the session will have
    /// on the machine it goes to ([`Starter::carry_target`] there), a Claude
    /// conversation is staged already written for it, so the new machine's
    /// chunks of an earlier copy match what is chunked here (MV-17).
    pub fn pack_for(&self, operation: &str, session: &str, target: Option<&Path>) -> Result<Packed, CarryError> {
        let record = self.carry_session(session)?;
        let home = self.home_of(&record)?;
        self.pack_home(operation, session, &record, home, target)
    }

    /// `session`'s record on this machine.
    fn carry_session(&self, session: &str) -> Result<humata2_store::SessionRecord, CarryError> {
        self.store
            .session(session)
            .map_err(store("read the session"))?
            .ok_or_else(|| CarryError::NoSession { session: session.to_owned() })
    }

    /// The staging of [`Starter::pack_for`] and [`Starter::pack_landing`]:
    /// `home` says which channel's folder it is staged under and the folder
    /// the session has there.
    fn pack_home(
        &self,
        operation: &str,
        session: &str,
        record: &humata2_store::SessionRecord,
        home: desk_part::Home,
        target: Option<&Path>,
    ) -> Result<Packed, CarryError> {
        let channel = home.channel.clone();

        // The adapter refuses a conversation it can't carry before anything is staged.
        let agent = self.store.agent(session).map_err(store("read the session's agent"))?;
        let located = match &agent {
            Some(agent) => self
                .locate_conversation(&record, agent)
                .map_err(|source| CarryError::Conversation { agent: record.agent.clone(), source })?,
            None => None,
        };

        let stage = self.carry_dir(&channel, operation).join(STAGE);
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(io("clear an earlier stage in", &stage))?;
        }
        fs::create_dir_all(&stage).map_err(io("create", &stage))?;
        self.store
            .export_session(session, &stage.join(RECORDS))
            .map_err(store("copy the session's records"))?;

        let agent_running = agent
            .as_ref()
            .is_some_and(|agent| matches!(agent.state, AgentState::Running { .. }));
        let mut agent_files = AgentFiles::None;
        let mut conversation_id = String::new();
        if let Some(agent) = &agent {
            conversation_id = agent.launch.conversation_id.clone();
            let file = &agent.launch.conversation_file;
            if let Some(found) = &located {
                agent_files = stage_conversation(found, &record.agent, &stage)?;
                agent_files = self.stage_previous(record, &stage, agent_files)?;
                if let Some(target) = target {
                    // What this machine holds of the conversation as it is
                    // here, so the session that comes back finds it.
                    self.hold_staged(&channel, &stage.join(AGENT));
                    let placing = placed::Placing {
                        agent: &record.agent,
                        from_folder: &record.folder,
                        target,
                        stage: &stage,
                        scratch: self.carry_dir(&channel, operation).join(placed::SCRATCH),
                    };
                    agent_files = placed::place_staged(&placing, agent_files).inspect_err(|_| {
                        let _ = fs::remove_dir_all(&stage);
                    })?;
                }
            } else if file.is_file() {
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

        // Earlier conversations travel even when the current one has no files.
        if agent_files == AgentFiles::None {
            agent_files = self.stage_previous(record, &stage, agent_files)?;
        }

        // A folder freed by an archive is gone; restoring on another machine carries the records and the conversation alone.
        let clones = self.stage_clones(record, &stage)?;
        let mut items = if record.folder.exists() {
            items_skipping(Area::Folder, &record.folder, &clones.git_dirs())?
        } else {
            Vec::new()
        };
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

        self.stage_rows(&home, &stage)?;
        self.stage_chunks(&channel, &items, &record.folder, &stage)?;
        let manifest = Manifest {
            operation: operation.to_owned(),
            session: session.to_owned(),
            channel: channel.id.clone(),
            folder: home.rel.clone(),
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
            digest: clones.digest(digest_of(&items)),
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

    /// The holder sockets and pids of `session`'s shell tabs here, so the
    /// machine a session left can stop its shells before it forgets their
    /// tabs. A tab with pid 0 has no holder started here yet (a carried tab
    /// waits for its agent to resume), so it is left out.
    pub fn shell_holders(&self, session: &str) -> Result<Vec<(PathBuf, u32)>, CarryError> {
        let agent = self.store.agent(session).map_err(store("read the session's agent"))?;
        Ok(self
            .shell_tabs(session, agent.as_ref().map(|agent| agent.tab.as_str()))?
            .into_iter()
            .filter(|tab| tab.holder_pid != 0)
            .map(|tab| (tab.holder_socket, tab.holder_pid))
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
                make_dirs(&self.holders, 0o700).map_err(io("create", &self.holders))?;
                let scrollback = fs::read(&staged).map_err(io("read", &staged))?;
                let permissions = fs::metadata(&staged).map_err(io("look at", &staged))?.permissions();
                let target = saved_scrollback_path(&socket);
                write_durably(&target, &scrollback, permissions).map_err(io("copy", &staged))?;
            }
            let tab = TabRecord {
                id,
                session: Some(manifest.session.clone()),
                kind: TabKind::Shell,
                title: carried.title.clone(),
                holder_socket: socket,
                holder_pid: 0,
                created_unix_ms: carried.created_unix_ms,
                holder_generation: 0,
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
        self.unpack_with(operation, session, channel, take, true)
    }

    /// [`Self::unpack`], with the clones rebuilt from their remotes only when
    /// `rebuild`: a landing on the same machine carries them whole.
    fn unpack_with(
        &self,
        operation: &str,
        session: &str,
        channel: &str,
        take: Take,
        rebuild: bool,
    ) -> Result<Unpacked, CarryError> {
        let channel = self.carry_channel(channel)?;
        let stage = self.incoming(&channel, operation);
        let path = stage.join(MANIFEST);
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(&path).map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Read(source) })?,
        )
        .map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Parse(source) })?;
        let folder = self.carried_folder(&channel, &manifest.folder)?;
        let recipes = self.read_clones(&stage)?;

        let of = |area: Area| -> Vec<&Item> { manifest.items.iter().filter(|item| item.area == area).collect() };
        let agent_dir = stage.join(AGENT);
        // The agent's big files the host sent as chunks are rebuilt first, so
        // the check that follows sees them whole (MV-17).
        self.assemble(&stage, &folder, &channel, chunks::At::Stage)?;
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

        // The clones are rebuilt from their remote and the files this machine
        // holds already are copied in; then every file is checked (MV-17).
        if rebuild {
            self.rebuild_clones(&recipes, &folder, &channel, &stage)?;
        }
        self.assemble(&stage, &folder, &channel, chunks::At::Folder)?;
        let landed = as_landed(&of(Area::Folder), Path::new(&manifest.old_folder), &folder);
        verify_skipping(Area::Folder, &folder, &landed.iter().collect::<Vec<_>>(), &recipes.git_dirs())?;

        // Everything arrived as sent; now it is made this machine's.
        let new_folder = folder.to_string_lossy().into_owned();
        let pairs = replacements(&manifest.old_folder, &new_folder);
        if !pairs.is_empty()
            && let Ok(entries) = fs::read_dir(&folder)
        {
            for entry in entries.flatten() {
                let git = entry.path().join(".git");
                if git.is_dir() {
                    rewrite_git(&entry.path(), &pairs)?;
                } else if git.is_file() {
                    // A linked worktree: its `.git` file names the clone it belongs to.
                    rewrite_file(&git, &pairs)?;
                }
            }
        }
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let plan = kind.launch(&self.home, &folder, &manifest.conversation_id);
        let mut placed = None;
        match &manifest.agent_files {
            AgentFiles::None => {}
            AgentFiles::File { name } => {
                // A landing on this machine finds a conversation file that
                // doesn't depend on the folder where it already is: it stays
                // as it is, and taking the landing back leaves it alone too.
                let was = kind
                    .launch(
                        &self.home,
                        Path::new(&manifest.old_folder),
                        &manifest.conversation_id,
                    )
                    .conversation_file;
                let in_place =
                    !rebuild && was == plan.conversation_file && plan.conversation_file.is_file();
                if !in_place {
                    place(&agent_dir.join(name), &plan.conversation_file, &pairs)?;
                }
            }
            AgentFiles::Conversation { .. } => {
                placed = self.place_conversation(&manifest, &folder, &agent_dir, head_of(&recipes).as_ref())?;
            }
            AgentFiles::Earlier { .. } => {}
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

        // The earlier conversations are named exactly, and first so they win.
        let mut moves = self.place_previous(&manifest, &folder, &agent_dir)?;
        if !manifest.old_folder.is_empty() {
            moves.push((PathBuf::from(&manifest.old_folder), folder.clone()));
        }
        self.store
            .import_moved_session(session, &records, take, &moves)
            .map_err(store("bring the session's records in"))?;
        self.make_ready(&manifest, &folder, take, placed.as_deref())?;
        self.bring_tabs(&manifest, &stage)?;
        self.bring_rows(&manifest, &stage, take)?;
        self.hold_chunks(&channel, &folder, &stage, &manifest, placed.as_deref());
        Ok(Unpacked {
            files: u32::try_from(manifest.items.len()).unwrap_or(u32::MAX),
            bytes: manifest.items.iter().map(|item| item.size).sum(),
        })
    }

    /// Points the imported records at this machine's folders and leaves the
    /// agent saved as one that should run, in a new tab, so the resume starts
    /// it in its conversation.
    fn make_ready(
        &self,
        manifest: &Manifest,
        folder: &Path,
        take: Take,
        conversation_file: Option<&Path>,
    ) -> Result<(), CarryError> {
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
        // A moved session keeps the model it was started on.
        let model = self.store.session_detail(session).map_err(store("read the session's start"))?.and_then(|detail| detail.model);
        let plan = kind.launch_with_model(&self.home, folder, &manifest.conversation_id, model.as_deref());
        let tab = format!("tb_{}", uuid::Uuid::new_v4().simple());
        agent.launch.folder = folder.to_owned();
        agent.launch.argv = if agent.launch.argv == agent.launch.resume_argv {
            plan.resume_argv.clone()
        } else {
            plan.argv
        };
        agent.launch.resume_argv = plan.resume_argv;
        // Where the adapter placed the transcript, when it did.
        agent.launch.conversation_file = conversation_file.map_or(plan.conversation_file, Path::to_owned);
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
        let target_folder = self.carried_folder(&channel, folder)?;
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
            let new_folder = target_folder.clone();
            let plan = kind.launch(&self.home, &new_folder, &manifest.conversation_id);
            match &manifest.agent_files {
                AgentFiles::None => {}
                AgentFiles::File { .. } => keep(remove(&plan.conversation_file)),
                AgentFiles::Conversation { .. } => keep(self.remove_placed(kind, &manifest.agent_files, &new_folder)),
                AgentFiles::Earlier { .. } => keep(self.remove_placed(kind, &manifest.agent_files, &new_folder)),
                AgentFiles::Project => {
                    if let Some(project) = plan.conversation_file.parent() {
                        keep(remove(project));
                    }
                }
            }
        }
        keep(remove(&target_folder));
        // A copy cut short leaves the transfer engine's staging folder
        // beside the session's folder.
        if let Some(beside) = target_folder.parent() {
            keep(remove_staging(beside, operation));
        }
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
        if let Some(folder) = self.folder_to_release(&record)?.filter(|folder| folder.exists()) {
            found.push(Discard { path: folder, what: format!("session folder of {}", record.title) });
        }
        if let Some(agent) = self.store.agent(session).map_err(store("read the session's agent"))? {
            if let Some(files) = self.conversation_discards(&record, &agent)? {
                found.extend(files);
                return Ok(found);
            }
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

/// Copies the located conversation into `stage`'s `agent` folder through its
/// adapter, which stages only that conversation's files. A refusal leaves
/// nothing staged.
fn stage_conversation(found: &Conversation, agent: &str, stage: &Path) -> Result<AgentFiles, CarryError> {
    let packed = adapter(found.format).pack(found, &stage.join(AGENT)).map_err(|source| {
        let _ = fs::remove_dir_all(stage);
        CarryError::Conversation { agent: agent.to_owned(), source }
    })?;
    Ok(AgentFiles::Conversation {
        packed,
        old_home: found.placement.home.clone(),
        old_cli_home: found.placement.cli_home.clone(),
        previous: Vec::new(),
        placed_for: None,
    })
}

/// The commit the one clone of a carried session is on, which its
/// conversation then names; none when there isn't exactly one clone.
fn head_of(recipes: &cheap::Clones) -> Option<GitHead> {
    match recipes.clones.as_slice() {
        [only] if !only.head.is_empty() => Some(GitHead {
            branch: (!only.branch.is_empty()).then(|| only.branch.clone()),
            sha: only.head.clone(),
        }),
        _ => None,
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

/// Removes `operation`'s transfer staging folders in `folder`: a carry's
/// copies are named `<operation>_…`.
fn remove_staging(folder: &Path, operation: &str) -> Result<(), CarryError> {
    let lead = format!(".humata-transfer-{operation}_");
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io("read the folder", folder)(source)),
    };
    for entry in entries {
        let entry = entry.map_err(io("read the folder", folder))?;
        if entry.file_name().to_string_lossy().starts_with(&lead) {
            remove(&entry.path())?;
        }
    }
    Ok(())
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
        make_dirs(parent, 0o700).map_err(io("create", parent))?;
    }
    let bytes = fs::read(from).map_err(io("read", from))?;
    let permissions = fs::metadata(from).map_err(io("look at", from))?.permissions();
    let now = rewritten(&bytes, pairs).unwrap_or(bytes);
    write_durably(to, &now, permissions).map_err(io("place", to))
}

/// Writes `bytes` to `path` with `permissions`: to a `.partial` beside it,
/// fsynced, renamed over `path`, and the folder fsynced, so a power cut
/// leaves the old file or the whole new one.
fn write_durably(path: &Path, bytes: &[u8], permissions: fs::Permissions) -> std::io::Result<()> {
    let temporary = partial(path);
    let written = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        drop(file);
        humata2_durable::rename(&temporary, path)?;
        match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => humata2_durable::sync_dir(parent),
            _ => Ok(()),
        }
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
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
            layout: Layout::new(root.join("Humata")),
            remote_template: String::new(),
            agents: Agents::from_json(agents.as_bytes()).unwrap(),
            home: root.to_owned(),
            revive: ReviveConfig {
                contain: None,
                builds: None,
                versions: None,
                gate: None,
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
            first_message: Default::default(),
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
        for (tab, kind, pid) in [(id, TabKind::Shell, 8), ("tb_old", TabKind::Agent, 7)] {
            starter
                .store
                .put_tab(&TabRecord {
                    id: tab.to_owned(),
                    session: Some("se_1".to_owned()),
                    kind,
                    title: format!("{tab} title"),
                    holder_socket: starter.holders.join(format!("{tab}.sock")),
                    holder_pid: pid,
                    created_unix_ms: 5,
                    holder_generation: 0,
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
        assert_eq!(
            old.shell_holders("se_1").unwrap(),
            vec![(socket, 8)],
            "the old machine can stop its shell by its holder's socket and pid, and not the agent's tab"
        );
    }

    #[test]
    fn taking_an_incoming_copy_back_drops_the_staging_a_cut_copy_left() {
        let dir = tempfile::tempdir().unwrap();
        let new = machine(&dir.path().join("new"), false);
        let sessions = new.layout.sessions("payments");
        let ours = sessions.join(".humata-transfer-op_1_0_folder");
        let theirs = sessions.join(".humata-transfer-op_2_0_folder");
        fs::create_dir_all(ours.join("fix")).unwrap();
        fs::create_dir_all(&theirs).unwrap();

        new.abort_incoming("op_1", "se_1", "ch_1", "sessions/fix", Take::Everything).unwrap();

        assert!(!ours.exists(), "the cut copy's staging is gone");
        assert!(theirs.exists(), "another operation's staging stays");
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
        let text = rewritten(br#"{"cwd":"/old/a b","project":"-old-a-b/c"}"#, &pairs).unwrap();
        assert_eq!(String::from_utf8(text).unwrap(), r#"{"cwd":"/new/x","project":"-new-x/c"}"#);
        assert!(rewritten(b"nothing here", &pairs).is_none());
        assert!(replacements("/same", "/same").is_empty());
    }
}
