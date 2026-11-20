//! Chunk-level reuse in a move (MV-17, D346): a file of a megabyte or more
//! is cut into content-defined chunks (FastCDC), each named by its BLAKE3
//! hash, and the new machine is sent only the chunks it doesn't hold. A file
//! that grew, or was edited in the middle, costs the chunks around the
//! change, not the file.
//!
//! - Each machine keeps one **chunk store** for every channel under its
//!   root, `<root>/.cas` (the folder that holds the channel's folder), the
//!   `fusefs_cas` store: one file per chunk, sharded by its hash, kept to
//!   [`STORE_CAP`] bytes by dropping the chunks used longest ago. It is
//!   filled from what the machine packs and from what it takes in.
//! - Packing lists the [`FileManifest`] of each big file in `chunks.json`
//!   beside the move's manifest. The host asks the new machine which chunks
//!   it holds (`TransferEndService.HaveChunks`), leaves those files out of
//!   the copy, sends the chunks that are missing into `<stage>/chunks/`,
//!   each compressed as the adaptive planner picks, and writes `dedupe.json`,
//!   the recipe of each file: its manifest.
//! - Unpacking rebuilds each file from the chunks it was sent and those its
//!   store holds, checks every chunk's hash and the file's size, and only
//!   then goes on to the manifest's checks. A chunk that has gone fails the
//!   move with nothing deleted, and the next try sends it.
//!
//! An empty `<channel>/.carry/have`, where an earlier version kept fixed-size
//! chunks, is removed when a channel's store is first used.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use fusefs_cas::cdc::DEFAULT_MAX_SIZE;
use fusefs_cas::{
    AdaptiveCodecPlanner, ChunkHash, ChunkRef, ChunkSlice, ChunkStore, ChunkStoreConfig, Codec, CodecError,
    CompressionContext, EncodedChunk, EvictionPriority, FastCdcConfig, FileManifest, StoreError, chunk_ranges,
    decode_chunk, hash_bytes,
};
use humata2_store::ChannelRecord;
use serde::{Deserialize, Serialize};

use super::{AGENT, Area, CARRY_DIR, CarryError, Item, Manifest, ManifestProblem, io};

/// The staged file that lists the manifest of each big file.
pub const CHUNKS_FILE: &str = "chunks.json";
/// Where, in the new machine's stage, the host puts the chunks it sends.
pub const CHUNKS_DIR: &str = "chunks";
/// The staged file the host writes with the recipe of each file the new
/// machine rebuilds from chunks.
pub const REUSE_FILE: &str = "dedupe.json";
/// A file of at least this many bytes travels as chunks.
pub const CHUNKED_FROM: u64 = 1 << 20;

/// The machine's chunk store, in the folder that holds the channel folders.
const STORE_DIR: &str = ".cas";
/// Where an earlier version kept its fixed-size chunks, under a channel's `.carry`.
const LEGACY_HAVE: &str = "have";
/// What the chunk store may hold; the chunks used longest ago go first.
const STORE_CAP: u64 = 2 << 30;
/// How much of a file is cut at a time, so a big file isn't held whole.
const WINDOW: usize = 16 << 20;

/// What a recipe's path is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum At {
    /// The session's folder.
    Folder,
    /// The staged files.
    Stage,
}

#[derive(Debug, Serialize)]
struct Listed {
    at: At,
    rel: String,
    manifest: FileManifest,
}

#[derive(Debug, Serialize)]
struct List {
    files: Vec<Listed>,
}

#[derive(Debug, Deserialize)]
struct Assembled {
    at: At,
    rel: String,
    manifest: FileManifest,
}

#[derive(Debug, Deserialize)]
struct Recipe {
    files: Vec<Assembled>,
}

/// Where the chunk store of the channel whose folder is `channel_dir` is:
/// beside the channel folders.
pub fn chunk_store_dir(channel_dir: &Path) -> PathBuf {
    channel_dir.parent().map_or_else(|| PathBuf::from(STORE_DIR), |root| root.join(STORE_DIR))
}

/// Which of `names` (BLAKE3 hashes in hex) the chunk store at `store` holds.
/// A name that isn't a hash, and a store that isn't there, hold nothing.
pub fn held_chunks(store: &Path, names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| ChunkHash::from_hex(name).is_ok_and(|hash| chunk_path(store, &hash).is_file()))
        .cloned()
        .collect()
}

fn chunk_path(store: &Path, hash: &ChunkHash) -> PathBuf {
    let hex = hash.to_hex();
    store.join(&hex[0..2]).join(&hex[2..4]).join(hex)
}

const TAG_NONE: u8 = 0;
const TAG_LZ4: u8 = 1;
const TAG_ZSTD: u8 = 2;

/// A chunk as it is sent: how the adaptive planner picked to compress it, its
/// length and its bytes.
pub fn frame_chunk(raw: &[u8]) -> Result<Vec<u8>, CodecError> {
    let encoded = AdaptiveCodecPlanner::default().encode(raw, &CompressionContext::default())?;
    let tag = match encoded.codec {
        Codec::None => TAG_NONE,
        Codec::Lz4 => TAG_LZ4,
        Codec::Zstd { .. } => TAG_ZSTD,
    };
    let mut out = Vec::with_capacity(encoded.data.len() + 5);
    out.push(tag);
    out.extend_from_slice(&encoded.raw_len.to_le_bytes());
    out.extend_from_slice(&encoded.data);
    Ok(out)
}

/// The chunk [`frame_chunk`] made `framed` from; none when it isn't one.
fn unframe_chunk(framed: &[u8]) -> Option<Vec<u8>> {
    let (tag, rest) = framed.split_first()?;
    let (len, data) = rest.split_first_chunk::<4>()?;
    let raw_len = u32::from_le_bytes(*len);
    if usize::try_from(raw_len).ok()? > DEFAULT_MAX_SIZE {
        return None;
    }
    let codec = match *tag {
        TAG_NONE => Codec::None,
        TAG_LZ4 => Codec::Lz4,
        TAG_ZSTD => Codec::Zstd { level: 0 },
        _ => return None,
    };
    decode_chunk(&EncodedChunk { codec, raw_len, data: data.to_vec() }).ok()
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

/// Cuts the file at `path` into chunks, puts each in `store`, pinned so this
/// operation's own writes can't push it out, and describes the file.
fn chunk_file(store: &mut ChunkStore, path: &Path) -> Result<FileManifest, CarryError> {
    let stored = |source: StoreError| CarryError::ChunkStore { file: path.display().to_string(), source };
    let config = FastCdcConfig::fusefs_default();
    let mut file = fs::File::open(path).map_err(io("read", path))?;
    let mut buffer = vec![0u8; WINDOW];
    let mut carried = 0usize;
    let mut base = 0u64;
    let mut slices = Vec::new();
    loop {
        let read = fill(&mut file, &mut buffer[carried..]).map_err(io("read", path))?;
        let end = carried + read;
        let last = end < buffer.len();
        let ranges = chunk_ranges(&buffer[..end], config);
        // Only the window's last chunk may have been cut short by the end of
        // the window, so it is cut again with what follows.
        let settled = if last { ranges.len() } else { ranges.len().saturating_sub(1) };
        let mut consumed = 0usize;
        for range in &ranges[..settled] {
            let bytes = &buffer[range.clone()];
            let hash = store.put(bytes, EvictionPriority::Bulk).map_err(stored)?;
            store.pin(&hash);
            let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            slices.push(ChunkSlice { offset: base + range.start as u64, len, hash });
            consumed = range.end;
        }
        if last {
            break;
        }
        buffer.copy_within(consumed..end, 0);
        base += consumed as u64;
        carried = end - consumed;
    }
    let size = slices.last().map_or(0, ChunkSlice::end);
    Ok(FileManifest::from_slices(size, 1, slices))
}

/// One chunk of a recipe, from what the host sent or from the store; none
/// when neither has it whole. What the host sent is kept in the store.
fn chunk_bytes_of(
    sent: &Path,
    store: &mut ChunkStore,
    chunk: &ChunkRef,
    file: &str,
) -> Result<Option<Vec<u8>>, CarryError> {
    let wanted = |bytes: &[u8]| usize::try_from(chunk.len).ok() == Some(bytes.len()) && hash_bytes(bytes) == chunk.hash;
    if usize::try_from(chunk.len).is_ok_and(|len| len > DEFAULT_MAX_SIZE) {
        return Ok(None);
    }
    if let Ok(framed) = fs::read(sent.join(chunk.hash.to_hex()))
        && let Some(bytes) = unframe_chunk(&framed)
        && wanted(&bytes)
    {
        store
            .put_verified(chunk.hash, &bytes, EvictionPriority::Bulk)
            .map_err(|source| CarryError::ChunkStore { file: file.to_owned(), source })?;
        store.pin(&chunk.hash);
        return Ok(Some(bytes));
    }
    let held = store.get(&chunk.hash).map_err(|source| CarryError::ChunkStore { file: file.to_owned(), source })?;
    match held {
        Some(bytes) if wanted(&bytes) => {
            store.pin(&chunk.hash);
            Ok(Some(bytes))
        }
        Some(_) => {
            // A chunk that isn't what its name says is no chunk: the host
            // then sees the machine lacks it and sends it.
            let _ = fs::remove_file(store.chunk_path(&chunk.hash));
            Ok(None)
        }
        None => Ok(None),
    }
}

impl crate::CarryOps<'_> {
    /// The machine's chunk store for `channel`'s folder.
    fn chunk_store(&self, channel: &ChannelRecord) -> Result<ChunkStore, CarryError> {
        let channel_dir = self.layout.channel(&channel.folder);
        remove_empty_legacy(&channel_dir);
        let dir = chunk_store_dir(&channel_dir);
        ChunkStore::open(&dir, ChunkStoreConfig { max_bytes: STORE_CAP })
            .map_err(|source| CarryError::ChunkStore { file: dir.display().to_string(), source })
    }

    /// Stores the chunks of each big file of `items` and lists their
    /// manifests in `stage`'s `chunks.json` for the host. `folder` is the
    /// session's folder; the agent's files are already in `stage`.
    pub(super) fn stage_chunks(
        &self,
        channel: &ChannelRecord,
        items: &[Item],
        folder: &Path,
        stage: &Path,
    ) -> Result<(), CarryError> {
        let big: Vec<&Item> = items.iter().filter(|item| item.link.is_none() && item.size >= CHUNKED_FROM).collect();
        if big.is_empty() {
            remove_empty_legacy(&self.layout.channel(&channel.folder));
            return Ok(());
        }
        let mut store = self.chunk_store(channel)?;
        let mut files = Vec::new();
        for item in big {
            let (at, rel, path) = match item.area {
                Area::Folder => (At::Folder, item.rel.clone(), folder.join(&item.rel)),
                Area::Agent => (At::Stage, format!("{AGENT}/{}", item.rel), stage.join(AGENT).join(&item.rel)),
                Area::Records | Area::Tabs => continue,
            };
            let manifest = chunk_file(&mut store, &path)?;
            // A file that changed since it was listed is copied whole.
            if manifest.size != item.size {
                continue;
            }
            files.push(Listed { at, rel, manifest });
        }
        if files.is_empty() {
            return Ok(());
        }
        let path = stage.join(CHUNKS_FILE);
        let bytes = serde_json::to_vec(&List { files })
            .map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Parse(source) })?;
        fs::write(&path, bytes).map_err(io("write", &path))
    }

    /// Rebuilds the files of the host's recipe that belong at `at`: each
    /// from the chunks it sent and those this machine's store holds, every
    /// chunk and the whole file's size checked. Nothing to do when the host
    /// sent no recipe.
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
        let files: Vec<&Assembled> = recipe.files.iter().filter(|file| file.at == at).collect();
        if files.is_empty() {
            return Ok(());
        }
        let mut store = self.chunk_store(channel)?;
        let sent = stage.join(CHUNKS_DIR);
        let root = match at {
            At::Folder => folder,
            At::Stage => stage,
        };
        for file in files {
            if !Path::new(&file.rel).components().all(|part| matches!(part, Component::Normal(_))) {
                return Err(CarryError::BadFolder { path: file.rel.clone() });
            }
            let target = root.join(&file.rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(io("create", parent))?;
            }
            let mut out = fs::File::create(&target).map_err(io("create", &target))?;
            let mut total = 0u64;
            for chunk in &file.manifest.chunks {
                if chunk.offset != total {
                    return Err(CarryError::Changed { item: file.rel.clone() });
                }
                let bytes = chunk_bytes_of(&sent, &mut store, chunk, &file.rel)?.ok_or_else(|| CarryError::Missing {
                    item: format!("a chunk of {}, which this machine was to take from its own copy", file.rel),
                })?;
                out.write_all(&bytes).map_err(io("write", &target))?;
                total += bytes.len() as u64;
            }
            out.sync_all().map_err(io("save", &target))?;
            if total != file.manifest.size {
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
        let mut paths = Vec::new();
        for item in manifest.items.iter().filter(|item| item.link.is_none() && item.size >= CHUNKED_FROM) {
            match item.area {
                Area::Folder => paths.push(folder.join(&item.rel)),
                Area::Agent => paths.push(stage.join(AGENT).join(&item.rel)),
                Area::Records | Area::Tabs => {}
            }
        }
        if let Some(path) = placed
            && fs::metadata(path).is_ok_and(|meta| meta.len() >= CHUNKED_FROM)
        {
            paths.push(path.to_owned());
        }
        self.hold_files(channel, &paths);
    }

    /// Stores the chunks of each big file under `dir` as it is, before the
    /// staged conversation is written for the machine it goes to: when the
    /// session comes back, this machine holds the chunks of the form it had
    /// here. Best effort.
    pub(super) fn hold_staged(&self, channel: &ChannelRecord, dir: &Path) {
        let mut files = Vec::new();
        if super::collect_files(dir, "", &mut files).is_err() {
            return;
        }
        let paths: Vec<PathBuf> = files
            .into_iter()
            .map(|rel| dir.join(rel))
            .filter(|path| fs::metadata(path).is_ok_and(|meta| meta.len() >= CHUNKED_FROM))
            .collect();
        self.hold_files(channel, &paths);
    }

    /// Chunks `paths` into the store. A file or a store that can't be used
    /// only costs a later move the chance to skip chunks.
    fn hold_files(&self, channel: &ChannelRecord, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let Ok(mut store) = self.chunk_store(channel) else { return };
        for path in paths {
            let _ = chunk_file(&mut store, path);
        }
    }
}

/// Removes `<channel>/.carry/have` when it is an empty folder, where an
/// earlier version kept its chunks. One that holds anything is left alone.
fn remove_empty_legacy(channel_dir: &Path) {
    let _ = fs::remove_dir(channel_dir.join(CARRY_DIR).join(LEGACY_HAVE));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes that don't repeat, so each chunk is its own.
    fn noise(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed;
        let mut out = Vec::with_capacity(len + 8);
        while out.len() < len {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            out.extend_from_slice(&(x >> 7).to_le_bytes());
        }
        out.truncate(len);
        out
    }

    fn open(root: &Path) -> ChunkStore {
        ChunkStore::open(root.join(".cas"), ChunkStoreConfig { max_bytes: STORE_CAP }).unwrap()
    }

    fn names(manifest: &FileManifest) -> Vec<String> {
        manifest.chunks.iter().map(|chunk| chunk.hash.to_hex()).collect()
    }

    /// The chunks of `manifest` that the store at `store` lacks.
    fn lacking(store: &Path, manifest: &FileManifest) -> Vec<String> {
        let asked = names(manifest);
        let held = held_chunks(store, &asked);
        asked.into_iter().filter(|name| !held.contains(name)).collect()
    }

    #[test]
    fn a_file_is_described_by_chunks_that_add_up_to_it_and_each_is_stored() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let data = noise(1, 5 << 20);
        let file = root.path().join("data.bin");
        fs::write(&file, &data).unwrap();

        let manifest = chunk_file(&mut store, &file).unwrap();

        assert_eq!(manifest.size, data.len() as u64);
        let mut end = 0;
        for chunk in &manifest.chunks {
            assert_eq!(chunk.offset, end);
            assert_eq!(hash_bytes(&data[chunk.offset as usize..chunk.end() as usize]), chunk.hash);
            end = chunk.end();
        }
        assert_eq!(end, data.len() as u64);
        assert!(lacking(&root.path().join(".cas"), &manifest).is_empty());
    }

    #[test]
    fn a_file_is_cut_the_same_whatever_the_window_it_is_read_in() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        // More than one window, so a cut spans the window's edge.
        let data = noise(2, WINDOW + (3 << 20));
        let file = root.path().join("big.bin");
        fs::write(&file, &data).unwrap();

        let manifest = chunk_file(&mut store, &file).unwrap();
        let whole: Vec<ChunkSlice> = fusefs_cas::chunk_bytes(&data, FastCdcConfig::fusefs_default());

        assert_eq!(manifest.chunks.len(), whole.len());
        for (chunk, slice) in manifest.chunks.iter().zip(&whole) {
            assert_eq!((chunk.offset, chunk.len, chunk.hash), (slice.offset, slice.len, slice.hash));
        }
    }

    #[test]
    fn a_second_carry_of_the_same_bytes_lacks_no_chunk() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let file = root.path().join("data.bin");
        fs::write(&file, noise(3, 4 << 20)).unwrap();
        let first = chunk_file(&mut store, &file).unwrap();
        assert!(lacking(&root.path().join(".cas"), &first).is_empty(), "the first carry stored what it chunked");

        let copy = root.path().join("copy.bin");
        fs::copy(&file, &copy).unwrap();
        let second = chunk_file(&mut store, &copy).unwrap();

        assert_eq!(first, second);
        assert!(lacking(&root.path().join(".cas"), &second).is_empty());
    }

    #[test]
    fn a_changed_file_lacks_only_the_chunks_around_the_change() {
        let mut data = noise(4, 6 << 20);
        let root = tempfile::tempdir().unwrap();
        let mut sender = open(root.path());
        let file = root.path().join("data.bin");
        fs::write(&file, &data).unwrap();
        let before = chunk_file(&mut sender, &file).unwrap();

        // What the receiver holds is what the first carry stored.
        let receiver = tempfile::tempdir().unwrap();
        let mut theirs = open(receiver.path());
        for chunk in &before.chunks {
            let bytes = &data[chunk.offset as usize..chunk.end() as usize];
            theirs.put_verified(chunk.hash, bytes, EvictionPriority::Bulk).unwrap();
        }

        let middle = data.len() / 2;
        data[middle..middle + 64].fill(0xA5);
        fs::write(&file, &data).unwrap();
        let after = chunk_file(&mut sender, &file).unwrap();

        let missing = lacking(&receiver.path().join(".cas"), &after);
        assert!(!missing.is_empty(), "the edit changed at least one chunk");
        assert!(
            missing.len() <= 3 && missing.len() < after.chunks.len() / 2,
            "{} of {} chunks lacking after a 64 byte edit",
            missing.len(),
            after.chunks.len()
        );
    }

    #[test]
    fn a_framed_chunk_reads_back_to_the_same_bytes_whatever_its_codec() {
        let compressible = vec![7u8; 100_000];
        let random = noise(5, 100_000);
        for raw in [compressible, random, vec![1u8; 10]] {
            let framed = frame_chunk(&raw).unwrap();
            assert_eq!(unframe_chunk(&framed), Some(raw));
        }
        assert_eq!(unframe_chunk(&[9, 0, 0]), None);
        assert_eq!(unframe_chunk(&[]), None);
    }

    #[test]
    fn a_chunk_that_isnt_what_its_name_says_is_not_held_and_goes() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let bytes = noise(6, 4000);
        let hash = store.put(&bytes, EvictionPriority::Bulk).unwrap();
        fs::write(store.chunk_path(&hash), &bytes[..100]).unwrap();
        let chunk = ChunkRef { offset: 0, len: 4000, hash };

        let found = chunk_bytes_of(&root.path().join("none"), &mut store, &chunk, "x").unwrap();

        assert_eq!(found, None);
        assert!(!store.chunk_path(&hash).exists());
    }

    #[test]
    fn a_chunk_sent_arrives_whole_and_is_kept_for_the_next_move() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let bytes = noise(7, 50_000);
        let hash = hash_bytes(&bytes);
        let sent = root.path().join("chunks");
        fs::create_dir_all(&sent).unwrap();
        fs::write(sent.join(hash.to_hex()), frame_chunk(&bytes).unwrap()).unwrap();
        let chunk = ChunkRef { offset: 0, len: 50_000, hash };

        let found = chunk_bytes_of(&sent, &mut store, &chunk, "x").unwrap();

        assert_eq!(found, Some(bytes.clone()));
        assert_eq!(held_chunks(&root.path().join(".cas"), &[hash.to_hex()]), vec![hash.to_hex()]);
        let later = chunk_bytes_of(&root.path().join("gone"), &mut store, &chunk, "x").unwrap();
        assert_eq!(later, Some(bytes));
    }

    #[test]
    fn an_empty_legacy_have_folder_is_removed_and_a_full_one_is_left() {
        let root = tempfile::tempdir().unwrap();
        let empty = root.path().join("payments");
        fs::create_dir_all(empty.join(CARRY_DIR).join(LEGACY_HAVE)).unwrap();
        remove_empty_legacy(&empty);
        assert!(!empty.join(CARRY_DIR).join(LEGACY_HAVE).exists());
        assert!(empty.join(CARRY_DIR).exists(), "only the legacy folder goes");

        let full = root.path().join("search");
        fs::create_dir_all(full.join(CARRY_DIR).join(LEGACY_HAVE)).unwrap();
        fs::write(full.join(CARRY_DIR).join(LEGACY_HAVE).join("a"), b"x").unwrap();
        remove_empty_legacy(&full);
        assert!(full.join(CARRY_DIR).join(LEGACY_HAVE).join("a").exists());
    }

    #[test]
    fn the_store_is_beside_the_channel_folders_and_names_that_arent_hashes_are_not_held() {
        assert_eq!(chunk_store_dir(Path::new("/r/Humata/payments")), PathBuf::from("/r/Humata/.cas"));
        let root = tempfile::tempdir().unwrap();
        assert!(held_chunks(&root.path().join("nothing"), &["zz".to_owned(), "a".repeat(64)]).is_empty());
    }
}
