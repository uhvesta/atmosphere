//! A carry to another logical place (DK-15..DK-17, MV-16, D305, D315): a
//! Desk session moved into a channel's `sessions/<name>`, on the machine it
//! is on. It is a carry whose landing is not where the session lives now,
//! so the folder, the conversation and every row that names the folder
//! follow it, and the old place keeps nothing once the move is released.
//! The move itself (Moving, a read-only terminal, finished or undone after
//! a power cut) is the Mover's, as for a move to another machine (MV-4).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use humata2_conversation::{FileRole, Format, adapter};
use humata2_store::{AgentState, Take};

use super::desk_part::Home;
use super::{
    AgentFiles, CarryError, ConversationError, Discard, MANIFEST, Manifest, ManifestProblem, Packed, RECORDS, STAGE,
    Unpacked, copy_tree, folder_name, io, project_keyed, remove, remove_staging, store, write_durably,
};
use crate::layout::partial;
use crate::start::Starter;
use crate::tree_copy::{Rebase, copy_file};

/// Under a landing's stage, the Codex rollout as it was before the unpack
/// rewrote it where it is, at its path under the Codex dir.
const BEFORE: &str = "before";

/// Beside the kept rollout, the git fields its thread row held before the
/// unpack (`git_sha`, `git_branch`), as JSON.
const THREAD_GIT: &str = "thread_git.json";

/// Where a moved session lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Landing {
    /// The channel's id.
    pub channel: String,
    /// The session's folder under the channel's folder, `sessions/<name>`.
    pub rel: String,
}

impl Starter {
    /// Stages `session` for a move to `landing`, as [`Starter::pack`] stages
    /// a move to another machine, under the landing channel's folder: the
    /// packed `folder` and `stage` are relative to it. A Desk folder the
    /// user picked (before D451) is packed as well, and the caller leaves
    /// the original in place (D461). An agent whose conversation no
    /// adapter carries is refused before anything is staged, as is a
    /// landing that is not one plain `sessions/<name>`.
    pub fn pack_landing(&self, operation: &str, session: &str, landing: &Landing) -> Result<Packed, CarryError> {
        let record = self.carry_session(session)?;
        let channel = self.carry_channel(&landing.channel)?;
        // A landing that is not `sessions/<name>` is refused here, before the
        // adapter or the disk is touched.
        folder_name(&landing.rel)?;
        let target = self.carry_target(&landing.channel, &landing.rel)?;
        // The session is not looked up by where it lives now: a Desk folder,
        // a picked one too, is packed from wherever its record says.
        let home = Home { channel, rel: landing.rel.clone(), rows: None };
        self.pack_home(operation, session, &record, home, Some(&target))
    }

    /// Refuses a move of `session` whose agent keeps its conversation where
    /// no adapter carries it, or holds records its adapter's strict read
    /// doesn't know (CV-3, CV-7), before anything is staged, stopped, added
    /// or copied. Reads only.
    pub fn check_landing(&self, session: &str) -> Result<(), CarryError> {
        let record = self.carry_session(session)?;
        let Some(agent) = self.store.agent(session).map_err(store("read the session's agent"))? else {
            return Ok(());
        };
        // The strict read the pack does later, so records Humata doesn't know
        // refuse the move here, before a repo is added or anything copied.
        let read = self.locate_conversation(&record, &agent).and_then(|found| {
            found.map_or(Ok(()), |found| adapter(found.format).check(&found, agent.launch.cli_version.as_deref()))
        });
        match read {
            Err(source @ ConversationError::Unsupported { .. }) => {
                Err(CarryError::Conversation { agent: record.agent.clone(), source })
            }
            _ => Ok(()),
        }
    }

    /// What the old place still holds of the session that `operation` moved
    /// to `channel`, once it landed: its old folder and its old
    /// conversation's own files, read from the move's manifest. Never the
    /// folder or the conversation it works in now, not even a conversation
    /// the move rewrote where it is.
    pub fn left_behind(&self, operation: &str, channel: &str) -> Result<Vec<Discard>, CarryError> {
        let channel = self.carry_channel(channel)?;
        let manifest = read_manifest(&self.carry_dir(&channel, operation).join(STAGE).join(MANIFEST))?;
        let record = self.carry_session(&manifest.session)?;
        let old = PathBuf::from(&manifest.old_folder);
        let new = self.carried_folder(&channel, &manifest.folder)?;
        let mut found = Vec::new();
        if old != new && fs::symlink_metadata(&old).is_ok() {
            found.push(Discard { path: old.clone(), what: format!("session folder of {}", record.title) });
        }
        let Some(agent) = self.store.agent(&manifest.session).map_err(store("read the session's agent"))? else {
            return Ok(found);
        };
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;

        // The conversation as it was at the old folder, and what the
        // session works in now, which stays.
        let mut was = record.clone();
        was.folder = old.clone();
        let mut now = record.clone();
        now.folder = new;
        let mut before = agent.clone();
        before.launch.folder = old.clone();
        before.launch.conversation_file = kind.launch(&self.home, &old, &manifest.conversation_id).conversation_file;
        let mut keep: Vec<PathBuf> = vec![agent.launch.conversation_file.clone()];
        let mut ids = vec![manifest.conversation_id.clone()];
        ids.extend(
            self.store
                .previous_conversations(&record.id)
                .map_err(store("read the session's earlier conversations"))?
                .into_iter()
                .map(|one| one.conversation_id),
        );
        for id in &ids {
            if let Some(current) = self.locate_by_id(&now, id) {
                keep.extend(current.files.into_iter().map(|file| file.path));
            }
        }
        let old_files = match self.conversation_discards(&was, &before)? {
            Some(files) => files,
            None => {
                let file = before.launch.conversation_file.clone();
                let path = if project_keyed(kind) { file.parent().map(Path::to_owned) } else { Some(file) };
                path.filter(|path| path.exists())
                    .map(|path| Discard { path, what: format!("conversation of {}", record.title) })
                    .into_iter()
                    .collect()
            }
        };
        found.extend(old_files.into_iter().filter(|discard| !keep.iter().any(|kept| kept.starts_with(&discard.path))));
        Ok(found)
    }

    /// Copies `session`'s folder and what `packed` staged to the landing in
    /// `channel`, on this machine, where [`Starter::unpack_landed`] takes
    /// them up. The session's own folder is only read. A folder already at
    /// the landing is refused and left as it is. Answers the landed folder.
    /// Blocking.
    pub fn land_copy(&self, operation: &str, session: &str, channel: &str, packed: &Packed) -> Result<PathBuf, CarryError> {
        let record = self.carry_session(session)?;
        let channel = self.carry_channel(channel)?;
        let folder = self.carried_folder(&channel, &packed.folder)?;
        if fs::symlink_metadata(&folder).is_ok() {
            return Err(CarryError::Exists { what: "the session folder", path: folder });
        }
        copy_tree(&record.folder, &folder)?;
        let root = self.layout.channel(&channel.folder);
        copy_tree(&root.join(&packed.stage), &self.incoming(&channel, operation))?;
        Ok(folder)
    }

    /// Brings the landing of `operation` up to date with what `session`
    /// wrote after it was packed and copied, once its agent and shells
    /// stopped, in one pass: the session is packed again, so its records,
    /// its conversation as it grew and the manifest are as they are now; the
    /// landed folder is synced with the session's (a file whose size, time,
    /// mode or bytes differ is copied again, a link made again, and what the
    /// session's folder no longer holds is removed); and the stage is copied
    /// again. The session's own folder is only read. Blocking.
    pub fn resync_landing(&self, operation: &str, session: &str, landing: &Landing) -> Result<(), CarryError> {
        let record = self.carry_session(session)?;
        let packed = self.pack_landing(operation, session, landing)?;
        let channel = self.carry_channel(&landing.channel)?;
        let folder = self.carried_folder(&channel, &packed.folder)?;
        if record.folder.exists() {
            sync_tree(&record.folder, &folder, &Rebase { from: &record.folder, to: &folder })?;
        }
        let incoming = self.incoming(&channel, operation);
        remove(&incoming)?;
        copy_tree(&self.layout.channel(&channel.folder).join(&packed.stage), &incoming)
    }

    /// Takes up a landed copy as [`Starter::unpack`] does, with every record,
    /// and with its clones as they were copied: a clone is never made again
    /// from the channel's main clone. A Codex rollout the unpack rewrites
    /// where it is is kept first as it was, so a move taken back puts it
    /// back. Blocking.
    pub fn unpack_landed(&self, operation: &str, session: &str, channel: &str) -> Result<Unpacked, CarryError> {
        self.keep_rollout(operation, channel)?;
        self.unpack_with(operation, session, channel, Take::Everything, false)
    }

    /// The Codex conversation of `manifest` when the unpack for `folder`
    /// rewrites its rollout where it is: this machine's Codex dir is the one
    /// it was packed from.
    fn rewritten_in_place<'m>(&self, manifest: &'m Manifest, folder: &Path) -> Option<&'m humata2_conversation::Packed> {
        let AgentFiles::Conversation { packed, old_home, old_cli_home, .. } = &manifest.agent_files else {
            return None;
        };
        if packed.format != Format::CodexRollout {
            return None;
        }
        let at = self.agents.get(&manifest.agent).ok()?.placement(&self.home, folder)?;
        (at.home == *old_home && at.cli_home == *old_cli_home).then_some(packed)
    }

    /// Keeps the rollout the unpack of `operation` is about to rewrite where
    /// it is, byte for byte, under the stage's `before`, whole or not at
    /// all. One kept already, by an unpack cut short, is the rollout from
    /// before and stays.
    fn keep_rollout(&self, operation: &str, channel: &str) -> Result<(), CarryError> {
        let channel = self.carry_channel(channel)?;
        let manifest = read_manifest(&self.incoming(&channel, operation).join(MANIFEST))?;
        let folder = self.carried_folder(&channel, &manifest.folder)?;
        let Some(packed) = self.rewritten_in_place(&manifest, &folder) else {
            return Ok(());
        };
        let kept = self.carry_dir(&channel, operation).join(STAGE).join(BEFORE);
        if kept.is_dir() {
            return Ok(());
        }
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let Some(was) = kind.placement(&self.home, Path::new(&manifest.old_folder)) else {
            return Ok(());
        };
        let found = match adapter(packed.format).locate(&was, &packed.id) {
            Ok(found) => found,
            // Nothing is there to rewrite, so nothing to keep.
            Err(ConversationError::NotFound { .. }) => return Ok(()),
            Err(source) => return Err(CarryError::Conversation { agent: manifest.agent.clone(), source }),
        };
        let writing = partial(&kept);
        remove(&writing)?;
        let transcripts = |role: &FileRole| *role == FileRole::Transcript;
        let staged = packed.files.iter().filter(|file| transcripts(&file.role));
        for (file, staged) in found.files.iter().filter(|file| transcripts(&file.role)).zip(staged) {
            let bytes = fs::read(&file.path).map_err(io("read", &file.path))?;
            let permissions = fs::metadata(&file.path).map_err(io("look at", &file.path))?.permissions();
            let to = writing.join(&staged.rel);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(io("create", parent))?;
            }
            write_durably(&to, &bytes, permissions).map_err(io("keep", &to))?;
        }
        // The thread row's git fields too: the unpack may change them.
        let git = adapter(packed.format)
            .thread_git(&was, &packed.id)
            .map_err(|source| CarryError::Conversation { agent: manifest.agent.clone(), source })?;
        if let Some(git) = git {
            use std::os::unix::fs::PermissionsExt;
            let file = writing.join(THREAD_GIT);
            let bytes = serde_json::to_vec(&git).map_err(std::io::Error::other).map_err(io("keep", &file))?;
            fs::create_dir_all(&writing).map_err(io("create", &writing))?;
            write_durably(&file, &bytes, fs::Permissions::from_mode(0o600)).map_err(io("keep", &file))?;
        }
        humata2_durable::rename(&writing, &kept).map_err(io("keep", &kept))?;
        match kept.parent() {
            Some(stage) => humata2_durable::sync_dir(stage).map_err(io("save", stage)),
            None => Ok(()),
        }
    }

    /// Puts back the rollout [`Self::keep_rollout`] kept, as it was before
    /// the unpack for `folder` rewrote it, and the CLI's thread row names the
    /// old folder again. Nothing is kept when the rollout lives in another
    /// Codex dir or the unpack never got that far.
    fn put_rollout_back(&self, manifest: &Manifest, stage: &Path, folder: &Path) -> Result<(), CarryError> {
        let kept = stage.join(BEFORE);
        let Some(packed) = self.rewritten_in_place(manifest, folder) else {
            return Ok(());
        };
        if !kept.is_dir() {
            return Ok(());
        }
        let refused = |source: ConversationError| CarryError::Conversation { agent: manifest.agent.clone(), source };
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let (Some(now), Some(was)) =
            (kind.placement(&self.home, folder), kind.placement(&self.home, Path::new(&manifest.old_folder)))
        else {
            return Ok(());
        };
        let files: Vec<_> = packed
            .files
            .iter()
            .filter(|file| file.role == FileRole::Transcript && kept.join(&file.rel).is_file())
            .cloned()
            .collect();
        if files.is_empty() {
            return Ok(());
        }
        let git_file = kept.join(THREAD_GIT);
        let staged = humata2_conversation::Packed { stage: kept, files, ..packed.clone() };
        let codex = adapter(packed.format);
        // The kept bytes name the old folder only, so placing them for it
        // from the new one writes them back as they were.
        codex.place(&staged, &now, &was).map(drop).map_err(refused)?;
        // The place put the row's cwd and rollout back; its git fields too.
        let bytes = match fs::read(&git_file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io("read", &git_file)(error)),
        };
        let git: humata2_conversation::ThreadGit = serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
            .map_err(io("read", &git_file))?;
        codex.restore_thread_git(&was, &packed.id, &git).map_err(refused)
    }

    /// Takes back a landing of `operation` that never switched: the landed
    /// folder and its staging, and once it was `unpacked` the conversation
    /// placed for the new folder, with the session's records put back as they
    /// were packed, a Codex rollout rewritten where it is put back as it was,
    /// and its agent saved as stopped, ready to resume where it was. The old
    /// folder is never touched. A part that isn't there is already gone.
    /// Blocking.
    pub fn take_landing_back(&self, operation: &str, session: &str, channel: &str, unpacked: bool) -> Result<(), CarryError> {
        let channel = self.carry_channel(channel)?;
        let dir = self.carry_dir(&channel, operation);
        let stage = dir.join(STAGE);
        let mut first: Option<CarryError> = None;
        let mut keep = |result: Result<(), CarryError>| {
            if let Err(error) = result {
                first.get_or_insert(error);
            }
        };
        // A pack again cut short leaves no stage; the copy of the first one
        // still names the landing.
        let manifest = [stage.join(MANIFEST), self.incoming(&channel, operation).join(MANIFEST)]
            .iter()
            .find_map(|path| read_manifest(path).ok());
        if let Some(manifest) = manifest {
            let folder = self.carried_folder(&channel, &manifest.folder)?;
            if unpacked {
                keep(self.drop_shell_tabs(session));
                keep(
                    self.store
                        .import_moved_session(session, &stage.join(RECORDS), Take::Everything, &[])
                        .map_err(store("put the session's records back as they were")),
                );
                keep(self.stopped_where_it_was(session));
                keep(self.put_rollout_back(&manifest, &stage, &folder));
                keep(self.remove_landed_conversation(&manifest, &folder));
            }
            keep(remove(&folder));
            // A copy cut short leaves the transfer engine's staging folder
            // beside the landed one.
            if let Some(beside) = folder.parent() {
                keep(remove_staging(beside, operation));
            }
        }
        keep(remove(&dir));
        first.map_or(Ok(()), Err)
    }

    /// Saves `session`'s agent as stopped: its holder was ended for the move.
    fn stopped_where_it_was(&self, session: &str) -> Result<(), CarryError> {
        let Some(mut agent) = self.store.agent(session).map_err(store("read the session's agent"))? else {
            return Ok(());
        };
        agent.state = AgentState::Stopped;
        self.store.put_agent(&agent).map_err(store("save the session's agent"))
    }

    /// Removes the conversation the unpack placed for the landed `folder`,
    /// never one that is the old folder's own.
    fn remove_landed_conversation(&self, manifest: &Manifest, folder: &Path) -> Result<(), CarryError> {
        let kind = self
            .agents
            .get(&manifest.agent)
            .map_err(|source| CarryError::Agent { agent: manifest.agent.clone(), source })?;
        let placed = kind.launch(&self.home, folder, &manifest.conversation_id).conversation_file;
        let was = kind
            .launch(&self.home, Path::new(&manifest.old_folder), &manifest.conversation_id)
            .conversation_file;
        match &manifest.agent_files {
            AgentFiles::None => Ok(()),
            AgentFiles::File { .. } if placed == was => Ok(()),
            AgentFiles::File { .. } => remove(&placed),
            AgentFiles::Conversation { .. } | AgentFiles::Earlier { .. } => {
                self.remove_placed(kind, &manifest.agent_files, folder)
            }
            AgentFiles::Project => match (placed.parent(), was.parent()) {
                (Some(project), Some(old)) if project != old => remove(project),
                _ => Ok(()),
            },
        }
    }
}

/// The move manifest at `path`.
fn read_manifest(path: &Path) -> Result<Manifest, CarryError> {
    let bytes =
        fs::read(path).map_err(|source| CarryError::Manifest { path: path.to_owned(), source: ManifestProblem::Read(source) })?;
    serde_json::from_slice(&bytes)
        .map_err(|source| CarryError::Manifest { path: path.to_owned(), source: ManifestProblem::Parse(source) })
}

/// Makes `to` hold what `from` holds, in one pass over both: a file whose
/// size, mode, modified time or bytes differ is copied again, a link whose
/// target differs is made again, a folder is synced in turn, and what `from`
/// no longer holds is removed from `to`. Links point as [`Rebase::target`]
/// says. A socket or other special file is left out, as a copy leaves it.
fn sync_tree(from: &Path, to: &Path, rebase: &Rebase<'_>) -> Result<(), CarryError> {
    if fs::symlink_metadata(to).is_ok_and(|meta| !meta.is_dir()) {
        remove(to)?;
    }
    fs::create_dir_all(to).map_err(io("create", to))?;
    let mut carried: BTreeSet<OsString> = BTreeSet::new();
    for entry in fs::read_dir(from).map_err(io("read the folder", from))? {
        let entry = entry.map_err(io("read the folder", from))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let meta = fs::symlink_metadata(&source).map_err(io("look at", &source))?;
        let have = fs::symlink_metadata(&target).ok();
        if meta.is_dir() {
            sync_tree(&source, &target, rebase)?;
        } else if meta.is_symlink() {
            let points = rebase.target(&fs::read_link(&source).map_err(io("read the link", &source))?);
            let same = have.as_ref().is_some_and(fs::Metadata::is_symlink)
                && fs::read_link(&target).ok().as_deref() == Some(points.as_path());
            if !same {
                remove(&target)?;
                std::os::unix::fs::symlink(&points, &target).map_err(io("make the link", &target))?;
            }
        } else if meta.is_file() {
            let same = match &have {
                Some(have) if have.is_file() => same_file(&source, &meta, &target, have)?,
                _ => false,
            };
            if !same {
                remove(&target)?;
                copy_file(&source, &target, &meta)?;
            }
        } else {
            continue;
        }
        carried.insert(entry.file_name());
    }
    for entry in fs::read_dir(to).map_err(io("read the folder", to))? {
        let entry = entry.map_err(io("read the folder", to))?;
        if !carried.contains(&entry.file_name()) {
            remove(&entry.path())?;
        }
    }
    Ok(())
}

/// Whether the copy `target` (its metadata `have`) still matches `source`
/// (`meta`): the same size, mode and modified time, and the same bytes.
fn same_file(source: &Path, meta: &fs::Metadata, target: &Path, have: &fs::Metadata) -> Result<bool, CarryError> {
    let modified = |meta: &fs::Metadata| meta.modified().ok();
    if meta.len() != have.len() || meta.permissions() != have.permissions() || modified(meta) != modified(have) {
        return Ok(false);
    }
    let mut ours = fs::File::open(source).map_err(io("read", source))?;
    let mut theirs = fs::File::open(target).map_err(io("read", target))?;
    let (mut a, mut b) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let read = fill(&mut ours, &mut a).map_err(io("read", source))?;
        let other = fill(&mut theirs, &mut b).map_err(io("read", target))?;
        if read != other || a.get(..read) != b.get(..other) {
            return Ok(false);
        }
        if read == 0 {
            return Ok(true);
        }
    }
}

/// Reads into `buffer` until it is full or `file` ends; the count read.
fn fill(file: &mut fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let Some(rest) = buffer.get_mut(filled..) else {
            break;
        };
        match file.read(rest) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}
