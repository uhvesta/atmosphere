//! Chunk-level reuse in a move (MV-17, D346): a file of a megabyte or more
//! travels as fixed 1 MiB chunks, the size the transfer engine moves them
//! in, and the new machine is sent only the chunks it doesn't hold. A file
//! that grew at its end since a machine last held it costs its new tail.
//!
//! - Each machine keeps a **chunk store**, one file per chunk named by its
//!   SHA-256, in `<channel>/.carry/have`. It is filled from what the machine
//!   packs and from what it takes in, and is kept to [`HAVE_CAP`] bytes by
//!   dropping the chunks used longest ago. A chunk is written to a temporary
//!   file that is synced and renamed, so a power cut leaves it whole or not
//!   there; one whose size or hash no longer matches its name counts as not
//!   held and is removed.
//! - Packing lists the chunk hashes of each big file in `chunks.json` beside
//!   the manifest. The host asks the new machine which of them it holds
//!   (`TransferEndService.HaveChunks`), leaves those files out of the copy,
//!   sends the chunks that are missing into `<stage>/chunks/` and writes
//!   `dedupe.json`, the recipe of each file: its chunks in order.
//! - Unpacking rebuilds each file from the chunks it was sent and those its
//!   store holds, checks every chunk and the whole file's SHA-256, and only
//!   then goes on to the manifest's checks. A chunk that has gone fails the
//!   move with nothing deleted, and the next try sends it.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use humata2_store::ChannelRecord;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{AGENT, Area, CarryError, Item, Manifest, ManifestProblem, hex, io};
use crate::layout::{make_dirs, partial};
use crate::start::Starter;

/// The staged file that lists the chunk hashes of each big file.
pub const CHUNKS_FILE: &str = "chunks.json";
/// Where, in the new machine's stage, the host puts the chunks it sends.
pub const CHUNKS_DIR: &str = "chunks";
/// The staged file the host writes with the recipe of each file the new
/// machine rebuilds from chunks.
pub const REUSE_FILE: &str = "dedupe.json";
/// Where, under a channel's folder, a machine keeps its chunk store.
pub const HAVE_DIR: &str = ".carry/have";
/// The size of a chunk, which is the transfer engine's.
pub const CHUNK_SIZE: u64 = 1 << 20;

const CHUNK: usize = 1 << 20;
/// What a chunk store may hold; the chunks used longest ago go first.
const HAVE_CAP: u64 = 2 << 30;

/// What a recipe's path is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum At {
    /// The session's folder.
    Folder,
    /// The staged files.
    Stage,
}

#[derive(Debug, Deserialize)]
struct Piece {
    sha256: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct Assembled {
    at: At,
    rel: String,
    size: u64,
    sha256: String,
    chunks: Vec<Piece>,
}

#[derive(Debug, Deserialize)]
struct Recipe {
    files: Vec<Assembled>,
}

fn is_hex(sha256: &str) -> bool {
    sha256.len() == 64 && sha256.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Reads until `buffer` is full or the file ends; how much it holds.
fn fill(file: &mut fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

/// Marks a chunk as used now, for the store's trimming.
fn touch(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// A machine's chunk store: one file per chunk, flat, named by its SHA-256.
struct Held {
    dir: PathBuf,
}

impl Held {
    fn file(&self, sha256: &str) -> PathBuf {
        self.dir.join(sha256)
    }

    /// The chunk, if the store holds it whole: its size and hash must be
    /// what it was asked for. One that isn't is removed. Marks it used.
    fn read(&self, sha256: &str, size: u64) -> Option<Vec<u8>> {
        let path = self.file(sha256);
        let bytes = fs::read(&path).ok()?;
        if u64::try_from(bytes.len()).ok() != Some(size) || hex(&Sha256::digest(&bytes)) != sha256 {
            let _ = fs::remove_file(&path);
            return None;
        }
        touch(&path);
        Some(bytes)
    }

    /// Stores a chunk. Best effort: one that can't be written only costs a
    /// later move the chance to skip it.
    fn put(&self, sha256: &str, bytes: &[u8]) {
        let path = self.file(sha256);
        if path.is_file() {
            touch(&path);
            return;
        }
        let _ = self.write(&path, bytes);
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            make_dirs(parent, 0o700)?;
        }
        let temporary = partial(path);
        let written = (|| {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            humata2_durable::rename(&temporary, path)?;
            humata2_durable::sync_dir(&self.dir)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written
    }

    /// The hashes of `path`'s chunks in order, each stored.
    fn add_file(&self, path: &Path) -> std::io::Result<Vec<String>> {
        let mut file = fs::File::open(path)?;
        let mut buffer = vec![0u8; CHUNK];
        let mut hashes = Vec::new();
        loop {
            let len = fill(&mut file, &mut buffer)?;
            if len == 0 {
                break;
            }
            let sha256 = hex(&Sha256::digest(&buffer[..len]));
            self.put(&sha256, &buffer[..len]);
            hashes.push(sha256);
            if len < CHUNK {
                break;
            }
        }
        Ok(hashes)
    }

    /// Removes the chunks used longest ago until the store fits `cap` bytes,
    /// and what a write a power cut cut short left behind.
    fn trim(&self, cap: u64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut chunks = Vec::new();
        let mut total = 0u64;
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            if !is_hex(&entry.file_name().to_string_lossy()) {
                let _ = fs::remove_file(entry.path());
                continue;
            }
            total += meta.len();
            chunks.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), entry.path()));
        }
        chunks.sort();
        for (_, len, path) in chunks {
            if total <= cap {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}

/// One chunk of a recipe, from what the host sent or from the store; none
/// when neither has it whole.
fn piece_bytes(sent: &Path, held: &Held, piece: &Piece) -> Option<Vec<u8>> {
    if !is_hex(&piece.sha256) || piece.size > CHUNK_SIZE {
        return None;
    }
    if let Ok(bytes) = fs::read(sent.join(&piece.sha256))
        && u64::try_from(bytes.len()).ok() == Some(piece.size)
        && hex(&Sha256::digest(&bytes)) == piece.sha256
    {
        return Some(bytes);
    }
    held.read(&piece.sha256, piece.size)
}

impl Starter {
    fn held(&self, channel: &ChannelRecord) -> Held {
        Held { dir: self.layout.channel(&channel.folder).join(HAVE_DIR) }
    }

    /// Stores the chunks of each big file of `items` and lists their hashes
    /// in `stage`'s `chunks.json` for the host. `folder` is the session's
    /// folder; the agent's files are already in `stage`.
    pub(super) fn stage_chunks(
        &self,
        channel: &ChannelRecord,
        items: &[Item],
        folder: &Path,
        stage: &Path,
    ) -> Result<(), CarryError> {
        let held = self.held(channel);
        let mut listed = Vec::new();
        for item in items.iter().filter(|item| item.link.is_none() && item.size >= CHUNK_SIZE) {
            let (at, rel, path) = match item.area {
                Area::Folder => ("folder", item.rel.clone(), folder.join(&item.rel)),
                Area::Agent => ("stage", format!("{AGENT}/{}", item.rel), stage.join(AGENT).join(&item.rel)),
                Area::Records | Area::Tabs => continue,
            };
            let hashes = held.add_file(&path).map_err(io("read", &path))?;
            // A file that changed since it was listed is copied whole.
            if u64::try_from(hashes.len()).ok() != Some(item.size.div_ceil(CHUNK_SIZE)) {
                continue;
            }
            listed.push(json!({ "at": at, "rel": rel, "size": item.size, "sha256": item.sha256, "chunks": hashes }));
        }
        held.trim(HAVE_CAP);
        if listed.is_empty() {
            return Ok(());
        }
        let path = stage.join(CHUNKS_FILE);
        let bytes = serde_json::to_vec(&json!({ "files": listed })).map_err(|source| CarryError::Manifest {
            path: path.clone(),
            source: ManifestProblem::Parse(source),
        })?;
        fs::write(&path, bytes).map_err(io("write", &path))
    }

    /// Rebuilds the files of the host's recipe that belong at `at`: each
    /// from the chunks it sent and those this machine's store holds, every
    /// chunk and the whole file checked. Nothing to do when the host sent no
    /// recipe.
    pub(super) fn assemble(
        &self,
        stage: &Path,
        folder: &Path,
        channel: &ChannelRecord,
        at: At,
    ) -> Result<(), CarryError> {
        let path = stage.join(REUSE_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(io("read", &path)(source)),
        };
        let recipe: Recipe = serde_json::from_slice(&bytes).map_err(|source| CarryError::Manifest {
            path: path.clone(),
            source: ManifestProblem::Parse(source),
        })?;
        let held = self.held(channel);
        let sent = stage.join(CHUNKS_DIR);
        let root = match at {
            At::Folder => folder,
            At::Stage => stage,
        };
        for file in recipe.files.iter().filter(|file| file.at == at) {
            if !Path::new(&file.rel).components().all(|part| matches!(part, Component::Normal(_))) {
                return Err(CarryError::BadFolder { path: file.rel.clone() });
            }
            let target = root.join(&file.rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(io("create", parent))?;
            }
            let mut out = fs::File::create(&target).map_err(io("create", &target))?;
            let mut whole = Sha256::new();
            let mut total = 0u64;
            for piece in &file.chunks {
                let bytes = piece_bytes(&sent, &held, piece).ok_or_else(|| CarryError::Missing {
                    item: format!("a chunk of {}, which this machine was to take from its own copy", file.rel),
                })?;
                out.write_all(&bytes).map_err(io("write", &target))?;
                whole.update(&bytes);
                total += piece.size;
            }
            out.sync_all().map_err(io("save", &target))?;
            if total != file.size || hex(&whole.finalize()) != file.sha256 {
                return Err(CarryError::Changed { item: file.rel.clone() });
            }
        }
        Ok(())
    }

    /// Stores the chunks of each big file just taken in, so a later move of
    /// a file that grew sends only what is new. Best effort. `placed` is the
    /// conversation's transcript as it was placed here, which may differ from
    /// the staged one by the paths this machine put in.
    pub(super) fn hold_chunks(
        &self,
        channel: &ChannelRecord,
        folder: &Path,
        stage: &Path,
        manifest: &Manifest,
        placed: Option<&Path>,
    ) {
        let held = self.held(channel);
        for item in manifest.items.iter().filter(|item| item.link.is_none() && item.size >= CHUNK_SIZE) {
            let path = match item.area {
                Area::Folder => folder.join(&item.rel),
                Area::Agent => stage.join(AGENT).join(&item.rel),
                Area::Records | Area::Tabs => continue,
            };
            let _ = held.add_file(&path);
        }
        if let Some(path) = placed
            && fs::metadata(path).is_ok_and(|meta| meta.len() >= CHUNK_SIZE)
        {
            let _ = held.add_file(path);
        }
        held.trim(HAVE_CAP);
    }

    /// Stores the chunks of each big file under `dir` as it is, before the
    /// staged conversation is written for the machine it goes to: when the
    /// session comes back, this machine holds the chunks of the form it had
    /// here. Best effort.
    pub(super) fn hold_staged(&self, channel: &ChannelRecord, dir: &Path) {
        let held = self.held(channel);
        let mut files = Vec::new();
        if super::collect_files(dir, "", &mut files).is_err() {
            return;
        }
        for rel in files {
            let path = dir.join(&rel);
            if fs::metadata(&path).is_ok_and(|meta| meta.len() >= CHUNK_SIZE) {
                let _ = held.add_file(&path);
            }
        }
        held.trim(HAVE_CAP);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn store(root: &Path) -> Held {
        Held { dir: root.join("have") }
    }

    fn name(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn a_stored_chunk_reads_back_whole() {
        let root = tempfile::tempdir().unwrap();
        let held = store(root.path());
        let bytes = vec![7u8; 1000];
        held.put(&name(&bytes), &bytes);
        assert_eq!(held.read(&name(&bytes), 1000), Some(bytes));
    }

    #[test]
    fn a_chunk_that_no_longer_matches_its_name_is_not_held_and_goes() {
        let root = tempfile::tempdir().unwrap();
        let held = store(root.path());
        let bytes = vec![7u8; 1000];
        held.put(&name(&bytes), &bytes);
        fs::write(held.file(&name(&bytes)), &bytes[..400]).unwrap();
        assert_eq!(held.read(&name(&bytes), 1000), None);
        assert!(!held.file(&name(&bytes)).exists());
    }

    #[test]
    fn a_file_is_listed_as_its_chunks_and_each_is_stored() {
        let root = tempfile::tempdir().unwrap();
        let held = store(root.path());
        let mut data = vec![1u8; CHUNK];
        data.extend(vec![2u8; 100]);
        let file = root.path().join("data.bin");
        fs::write(&file, &data).unwrap();
        let hashes = held.add_file(&file).unwrap();
        assert_eq!(hashes, vec![name(&data[..CHUNK]), name(&data[CHUNK..])]);
        assert_eq!(held.read(&hashes[1], 100), Some(data[CHUNK..].to_vec()));
    }

    #[test]
    fn trimming_drops_the_chunks_used_longest_ago_and_keeps_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let held = store(root.path());
        let (old, new) = (vec![1u8; 600], vec![2u8; 600]);
        held.put(&name(&old), &old);
        held.put(&name(&new), &new);
        let then = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options().write(true).open(held.file(&name(&old))).unwrap().set_modified(then).unwrap();
        held.trim(700);
        assert_eq!(held.read(&name(&old), 600), None);
        assert_eq!(held.read(&name(&new), 600), Some(new));
    }

    #[test]
    fn a_write_a_power_cut_cut_short_is_swept_and_never_read_as_a_chunk() {
        let root = tempfile::tempdir().unwrap();
        let held = store(root.path());
        let bytes = vec![3u8; 500];
        held.put(&name(&bytes), &bytes);
        let torn = partial(&held.file(&name(&[9u8; 10])));
        fs::write(&torn, [9u8; 4]).unwrap();
        held.trim(u64::MAX);
        assert!(!torn.exists());
        assert_eq!(held.read(&name(&bytes), 500), Some(bytes));
    }
}
