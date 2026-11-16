use crate::{Error, error::disk};
use flate2::{Crc, Decompress, FlushDecompress, Status};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use terminal_codec::{Envelope, Limits, Registry};
use terminal_wire::{HistoryPolicy, RecoveryHint, SNAPSHOT_ENGINE};
const LEGACY_MAGIC: &[u8; 8] = b"H2VT\0\0\0\x01";
const MAGIC: &[u8; 8] = b"H2VT\0\0\0\x02";
pub(crate) const MAX_CHECKPOINT: usize = terminal_wire::MAX_RAW_CHECKPOINT_BYTES;
pub(crate) const MAX_ENCODED_CHECKPOINT: usize = terminal_wire::MAX_SNAPSHOT_BYTES;
const MAX_METADATA: u64 = 16 << 20;
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub profile: Option<String>,
    pub recovery: Option<RecoveryHint>,
    pub history: HistoryPolicy,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            profile: None,
            recovery: None,
            history: HistoryPolicy::default(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Header {
    version: u32,
    engine: String,
    offset: u64,
    cols: u16,
    rows: u16,
    exited: Option<i32>,
    #[serde(default)]
    resize_epoch: u64,
}
pub(crate) struct Checkpoint {
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub cols: u16,
    pub rows: u16,
    pub exited: Option<i32>,
    pub resize_epoch: u64,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct ReplyCursor {
    pub answered_through: u64,
    pub pending: bool,
}
#[derive(Clone)]
pub(crate) struct Recovery {
    root: PathBuf,
    resize_lock: Arc<Mutex<()>>,
    codecs: Arc<Registry>,
    codec: Vec<String>,
}
impl Recovery {
    pub(crate) fn open(root: PathBuf) -> Result<Self, Error> {
        Self::open_with_codecs(
            root,
            Arc::new(Registry::default()),
            vec![terminal_codec::LZ4.into(), terminal_codec::ZLIB.into()],
        )
    }
    pub(crate) fn open_with_compression(
        root: PathBuf,
        codecs: Arc<Registry>,
        codec: String,
    ) -> Result<Self, Error> {
        Self::open_with_codecs(root, codecs, vec![codec])
    }
    pub(crate) fn open_with_codecs(
        root: PathBuf,
        codecs: Arc<Registry>,
        codec: Vec<String>,
    ) -> Result<Self, Error> {
        // Validate every registered preference before starting any process.
        if codec.is_empty() {
            return Err(Error::Invalid {
                reason: "at least one checkpoint codec must be selected",
            });
        }
        for id in &codec {
            codecs.encode(id, &[], archive_limits(MAX_ENCODED_CHECKPOINT as u64))?;
        }
        if root.exists() {
            let m = fs::symlink_metadata(&root)
                .map_err(|e| disk("inspect recovery folder", &root, e))?;
            if !m.is_dir() || m.file_type().is_symlink() || m.permissions().mode() & 0o077 != 0 {
                return Err(Error::Invalid {
                    reason: "recovery folder must be a dedicated private 0700 directory",
                });
            }
        } else {
            fs::create_dir(&root)
                .and_then(|_| fs::set_permissions(&root, fs::Permissions::from_mode(0o700)))
                .map_err(|e| disk("create recovery folder", &root, e))?;
        }
        Ok(Self {
            root,
            resize_lock: Arc::new(Mutex::new(())),
            codecs,
            codec,
        })
    }
    pub(crate) fn settings(&self) -> Result<BTreeMap<u64, Settings>, Error> {
        let path = self.root.join("sessions.json");
        let Some(file) = self.regular_file(&path)? else {
            return Ok(BTreeMap::new());
        };
        let mut bytes = Vec::new();
        file.take(MAX_METADATA + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| disk("read recovery metadata", &path, e))?;
        if bytes.len() as u64 > MAX_METADATA {
            return Err(Error::Invalid {
                reason: "recovery metadata exceeds 16 MiB; preserve it and use a separate host folder",
            });
        }
        let settings: BTreeMap<u64, Settings> =
            serde_json::from_slice(&bytes).map_err(|source| Error::Decode { path, source })?;
        if settings.len() > 256 {
            return Err(Error::Invalid {
                reason: "recovery metadata exceeds 256 sessions; preserve it and use a separate host folder",
            });
        }
        validate_settings(&settings)?;
        Ok(settings)
    }
    pub(crate) fn save_settings(&self, settings: &BTreeMap<u64, Settings>) -> Result<(), Error> {
        validate_settings(settings)?;
        let path = self.root.join("sessions.json");
        let bytes = serde_json::to_vec_pretty(settings).map_err(|source| Error::Decode {
            path: path.clone(),
            source,
        })?;
        if settings.len() > 256 || bytes.len() as u64 > MAX_METADATA {
            return Err(Error::Invalid {
                reason: "recovery metadata exceeds its safe size or session limit; use a separate host folder",
            });
        }
        self.atomic(&path, &bytes)
    }
    pub(crate) fn load(&self, id: u64, limit: u64) -> Result<Option<Checkpoint>, Error> {
        let path = self.root.join(format!("s{id}.checkpoint"));
        let Some(file) = self.regular_file(&path)? else {
            return Ok(None);
        };
        if file
            .metadata()
            .map_err(|e| disk("inspect recovery checkpoint", &path, e))?
            .len()
            > limit
        {
            return Err(Error::CheckpointLimit { id, limit });
        }
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| disk("read recovery checkpoint", &path, e))?;
        if bytes.len() < 8 {
            return Err(Error::Checkpoint { id });
        }
        let mut decoded = if &bytes[..8] == LEGACY_MAGIC {
            decode_checkpoint(id, &bytes[8..])?
        } else if &bytes[..8] == MAGIC {
            let limits = archive_limits(limit);
            let envelope = Envelope::from_bytes(&bytes[8..], limits)?;
            self.codecs.decode(&envelope, limits)?
        } else {
            return Err(Error::Checkpoint { id });
        };
        if decoded.len() < 4 || decoded.len() > MAX_CHECKPOINT + 4100 {
            return Err(Error::CheckpointLimit {
                id,
                limit: MAX_CHECKPOINT as u64,
            });
        }
        let n = u32::from_le_bytes(
            decoded[..4]
                .try_into()
                .map_err(|_| Error::Checkpoint { id })?,
        ) as usize;
        if n > 4096 || n + 4 > decoded.len() {
            return Err(Error::Checkpoint { id });
        }
        let h: Header =
            serde_json::from_slice(&decoded[4..4 + n]).map_err(|_| Error::Checkpoint { id })?;
        if h.version != 1 || h.engine != SNAPSHOT_ENGINE || h.cols == 0 || h.rows == 0 {
            return Err(Error::Checkpoint { id });
        }
        let snapshot = decoded.split_off(4 + n);
        if snapshot.len() > MAX_CHECKPOINT {
            return Err(Error::CheckpointLimit {
                id,
                limit: MAX_CHECKPOINT as u64,
            });
        }
        Ok(Some(Checkpoint {
            bytes: snapshot,
            offset: h.offset,
            cols: h.cols,
            rows: h.rows,
            exited: h.exited,
            resize_epoch: h.resize_epoch,
        }))
    }
    pub(crate) fn save(&self, id: u64, checkpoint: Checkpoint, limit: u64) -> Result<(), Error> {
        if checkpoint.bytes.len() > MAX_CHECKPOINT {
            return Err(Error::CheckpointLimit {
                id,
                limit: MAX_CHECKPOINT as u64,
            });
        }
        let path = self.root.join(format!("s{id}.checkpoint"));
        let h = Header {
            version: 1,
            engine: SNAPSHOT_ENGINE.into(),
            offset: checkpoint.offset,
            cols: checkpoint.cols,
            rows: checkpoint.rows,
            exited: checkpoint.exited,
            resize_epoch: checkpoint.resize_epoch,
        };
        let json = serde_json::to_vec(&h).map_err(|source| Error::Decode {
            path: path.clone(),
            source,
        })?;
        let mut raw = Vec::with_capacity(4 + json.len() + checkpoint.bytes.len());
        raw.extend_from_slice(&(json.len() as u32).to_le_bytes());
        raw.extend_from_slice(&json);
        raw.extend_from_slice(&checkpoint.bytes);
        let limits = archive_limits(limit);
        let compressed = self
            .codecs
            .encode_preferred(
                &self.codec.iter().map(String::as_str).collect::<Vec<_>>(),
                &raw,
                limits,
            )?
            .to_bytes(limits)?;
        if compressed.len() + 8 > limit as usize {
            return Err(Error::CheckpointLimit { id, limit });
        }
        let mut bytes = Vec::with_capacity(compressed.len() + 8);
        bytes.extend_from_slice(MAGIC);
        bytes.extend(compressed);
        self.atomic(&path, &bytes)?;
        self.clear_resize(id, checkpoint.resize_epoch)
    }
    pub(crate) fn resize_epoch(&self, id: u64) -> Result<Option<u64>, Error> {
        let _guard = self.resize_lock.lock().map_err(|_| Error::Invalid {
            reason: "resize recovery locking failed; restart the host",
        })?;
        self.read_resize_epoch(id)
    }
    fn read_resize_epoch(&self, id: u64) -> Result<Option<u64>, Error> {
        let path = self.root.join(format!("s{id}.resize"));
        match self.read_record(id, &path, 8)? {
            Some(bytes) => Ok(Some(u64::from_le_bytes(
                bytes.try_into().map_err(|_| Error::Checkpoint { id })?,
            ))),
            None => Ok(None),
        }
    }
    fn regular_file(&self, path: &Path) -> Result<Option<File>, Error> {
        let info = match fs::symlink_metadata(path) {
            Ok(info) => info,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(disk("inspect recovery entry", path, e)),
        };
        if !info.is_file() || info.file_type().is_symlink() {
            return Err(Error::RecoveryEntry { path: path.into() });
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| disk("read recovery entry", path, e))?;
        if !file
            .metadata()
            .map_err(|e| disk("inspect recovery entry", path, e))?
            .is_file()
        {
            return Err(Error::RecoveryEntry { path: path.into() });
        }
        Ok(Some(file))
    }
    fn read_record(&self, id: u64, path: &Path, length: usize) -> Result<Option<Vec<u8>>, Error> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(disk("inspect recovery record", path, e)),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != length as u64
        {
            return Err(Error::Checkpoint { id });
        }
        let file = self.regular_file(path)?.ok_or(Error::Checkpoint { id })?;
        let mut bytes = Vec::with_capacity(length + 1);
        file.take(length as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| disk("read recovery record", path, e))?;
        if bytes.len() != length {
            return Err(Error::Checkpoint { id });
        }
        Ok(Some(bytes))
    }
    pub(crate) fn reply_cursor(&self, id: u64) -> Result<ReplyCursor, Error> {
        let path = self.root.join(format!("s{id}.replies"));
        let Some(bytes) = self.read_record(id, &path, 21)? else {
            return Ok(ReplyCursor::default());
        };
        if &bytes[..8] != b"H2RP\0\0\0\x01" || bytes[16] > 1 {
            return Err(Error::Checkpoint { id });
        }
        let mut checksum = Crc::new();
        checksum.update(&bytes[..17]);
        let expected = u32::from_le_bytes(
            bytes[17..21]
                .try_into()
                .map_err(|_| Error::Checkpoint { id })?,
        );
        if checksum.sum() != expected {
            return Err(Error::Checkpoint { id });
        }
        let answered_through = u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| Error::Checkpoint { id })?,
        );
        Ok(ReplyCursor {
            answered_through,
            pending: bytes[16] == 1,
        })
    }
    pub(crate) fn save_reply_cursor(&self, id: u64, cursor: ReplyCursor) -> Result<(), Error> {
        let mut bytes = Vec::with_capacity(21);
        bytes.extend_from_slice(b"H2RP\0\0\0\x01");
        bytes.extend_from_slice(&cursor.answered_through.to_le_bytes());
        bytes.push(u8::from(cursor.pending));
        let mut checksum = Crc::new();
        checksum.update(&bytes);
        bytes.extend_from_slice(&checksum.sum().to_le_bytes());
        self.atomic(&self.root.join(format!("s{id}.replies")), &bytes)
    }
    pub(crate) fn mark_resize(&self, id: u64, epoch: u64) -> Result<(), Error> {
        let _guard = self.resize_lock.lock().map_err(|_| Error::Invalid {
            reason: "resize recovery locking failed; restart the host",
        })?;
        self.atomic(
            &self.root.join(format!("s{id}.resize")),
            &epoch.to_le_bytes(),
        )
    }
    fn clear_resize(&self, id: u64, epoch: u64) -> Result<(), Error> {
        let _guard = self.resize_lock.lock().map_err(|_| Error::Invalid {
            reason: "resize recovery locking failed; restart the host",
        })?;
        if self.read_resize_epoch(id)? == Some(epoch) {
            let path = self.root.join(format!("s{id}.resize"));
            fs::remove_file(&path)
                .and_then(|_| File::open(&self.root)?.sync_all())
                .map_err(|e| disk("commit resize recovery", &path, e))?;
        }
        Ok(())
    }
    pub(crate) fn remove(&self, id: u64) -> Result<(), Error> {
        let mut failure = None;
        let mut changed = false;
        for name in [
            format!("s{id}.checkpoint"),
            format!("s{id}.checkpoint.tmp"),
            format!("s{id}.tmp"),
        ] {
            let path = self.root.join(name);
            match fs::remove_file(&path) {
                Ok(()) => changed = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    if failure.is_none() {
                        failure = Some(disk("disable screen persistence", &path, e));
                    }
                }
            }
        }
        if changed {
            if let Err(e) = File::open(&self.root).and_then(|file| file.sync_all()) {
                if failure.is_none() {
                    failure = Some(disk("sync disabled recovery", &self.root, e));
                }
            }
        }
        if let Some(error) = failure {
            Err(error)
        } else {
            Ok(())
        }
    }
    fn atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), Error> {
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let tmp = PathBuf::from(temporary);
        match fs::symlink_metadata(&tmp) {
            Ok(info) => {
                if !info.is_file() || info.file_type().is_symlink() || info.nlink() != 1 {
                    return Err(Error::RecoveryTemporary {
                        path: tmp,
                        regular: info.is_file() && !info.file_type().is_symlink(),
                        links: info.nlink(),
                    });
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(disk("inspect recovery temporary entry", &tmp, e)),
        }
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&tmp)
            .map_err(|e| disk("write recovery", &tmp, e))?;
        let info = f
            .metadata()
            .map_err(|e| disk("inspect recovery temporary entry", &tmp, e))?;
        if !info.is_file() || info.nlink() != 1 {
            return Err(Error::RecoveryTemporary {
                path: tmp,
                regular: info.is_file(),
                links: info.nlink(),
            });
        }
        f.set_permissions(fs::Permissions::from_mode(0o600))
            .and_then(|_| f.set_len(0))
            .map_err(|e| disk("prepare recovery temporary entry", &tmp, e))?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| disk("sync recovery", &tmp, e))?;
        fs::rename(&tmp, path)
            .and_then(|_| File::open(&self.root)?.sync_all())
            .map_err(|e| disk("commit recovery", path, e))
    }
}
fn validate_settings(settings: &BTreeMap<u64, Settings>) -> Result<(), Error> {
    if settings.len() > 256 {
        return Err(Error::Invalid {
            reason: "recovery metadata exceeds 256 sessions; preserve it and use a separate host folder",
        });
    }
    for (id, setting) in settings {
        if *id == 0 {
            return Err(Error::Invalid {
                reason: "recovery metadata contains an invalid zero session ID; preserve it and restore a matching backup",
            });
        }
        crate::validate_history(&setting.history)?;
        crate::validate_recovery(setting.recovery.as_ref())?;
        if let Some(profile) = &setting.profile {
            crate::validate_name(profile)?;
        }
    }
    Ok(())
}
fn archive_limits(limit: u64) -> Limits {
    Limits {
        max_decoded_bytes: MAX_CHECKPOINT + 4100,
        max_encoded_bytes: (limit.min(MAX_ENCODED_CHECKPOINT as u64) as usize)
            .saturating_sub(8 + 34 + 64),
    }
}
fn decode_checkpoint(id: u64, compressed: &[u8]) -> Result<Vec<u8>, Error> {
    let limit = MAX_CHECKPOINT + 4100;
    let mut decoded = Vec::new();
    let mut inflater = Decompress::new(true);
    let mut chunk = [0u8; 8192];
    loop {
        let before = (inflater.total_in(), inflater.total_out());
        let capacity = chunk.len().min(limit + 1 - decoded.len());
        let status = inflater
            .decompress(
                &compressed[before.0 as usize..],
                &mut chunk[..capacity],
                FlushDecompress::None,
            )
            .map_err(|source| Error::CheckpointInflate { id, source })?;
        let produced = (inflater.total_out() - before.1) as usize;
        decoded.extend_from_slice(&chunk[..produced]);
        if decoded.len() > limit {
            return Err(Error::CheckpointLimit {
                id,
                limit: MAX_CHECKPOINT as u64,
            });
        }
        if status == Status::StreamEnd {
            if inflater.total_in() as usize != compressed.len() {
                return Err(Error::Checkpoint { id });
            }
            return Ok(decoded);
        }
        if before == (inflater.total_in(), inflater.total_out()) {
            return Err(Error::Checkpoint { id });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Checkpoint {
        Checkpoint {
            bytes: vec![7; 20000],
            offset: 123,
            cols: 80,
            rows: 24,
            exited: None,
            resize_epoch: 0,
        }
    }
    #[test]
    fn legacy_zlib_archive_loads_then_upgrades_to_selected_codec() {
        use flate2::{Compression, write::ZlibEncoder};
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        let h = Header {
            version: 1,
            engine: SNAPSHOT_ENGINE.into(),
            offset: 123,
            cols: 80,
            rows: 24,
            exited: None,
            resize_epoch: 0,
        };
        let json = serde_json::to_vec(&h).unwrap();
        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        z.write_all(&(json.len() as u32).to_le_bytes()).unwrap();
        z.write_all(&json).unwrap();
        z.write_all(&sample().bytes).unwrap();
        let mut old = LEGACY_MAGIC.to_vec();
        old.extend(z.finish().unwrap());
        let path = r.root.join("s1.checkpoint");
        fs::write(&path, &old).unwrap();
        assert_eq!(r.load(1, 100000).unwrap().unwrap().bytes, sample().bytes);
        let mut trailing = old.clone();
        trailing.push(0);
        fs::write(&path, trailing).unwrap();
        assert!(r.load(1, 100000).is_err());
        fs::write(&path, &old[..old.len() - 1]).unwrap();
        assert!(r.load(1, 100000).is_err());
        fs::write(&path, old).unwrap();
        r.save(1, sample(), 100000).unwrap();
        let saved = fs::read(&path).unwrap();
        let envelope = Envelope::from_bytes(&saved[8..], archive_limits(100000)).unwrap();
        assert_eq!(envelope.codec, terminal_codec::LZ4);
        assert_eq!(r.load(1, 100000).unwrap().unwrap().offset, 123);
    }
    #[test]
    fn default_preference_falls_back_to_zlib_only_when_lz4_exceeds_archive_budget() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("recovery");
        let r = Recovery::open(root.clone()).unwrap();
        let payload = || {
            let mut seed = 0x1234abcd_u32;
            (0..200000)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as u8) & 15
                })
                .collect::<Vec<_>>()
        };
        let cp = || Checkpoint {
            bytes: payload(),
            offset: 99,
            cols: 80,
            rows: 24,
            exited: None,
            resize_epoch: 0,
        };
        r.save(1, cp(), 128 << 10).unwrap();
        let path = root.join("s1.checkpoint");
        let saved = fs::read(&path).unwrap();
        let envelope = Envelope::from_bytes(&saved[8..], archive_limits(128 << 10)).unwrap();
        assert_eq!(envelope.codec, terminal_codec::ZLIB);
        assert!(saved.len() <= 128 << 10);
        let strict = Recovery::open_with_compression(
            root,
            Arc::new(Registry::default()),
            terminal_codec::LZ4.into(),
        )
        .unwrap();
        assert!(strict.save(1, cp(), 128 << 10).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(strict.load(1, 128 << 10).unwrap().unwrap().bytes, payload());
    }
    #[test]
    fn custom_codec_requires_explicit_registry_on_reopen() {
        struct Identity;
        impl terminal_codec::Codec for Identity {
            fn id(&self) -> &str {
                "test-identity-v1"
            }
            fn max_encoded_len(&self, n: usize) -> Result<usize, terminal_codec::Error> {
                Ok(n)
            }
            fn encode_into(
                &self,
                raw: &[u8],
                out: &mut [u8],
            ) -> Result<usize, terminal_codec::Error> {
                out[..raw.len()].copy_from_slice(raw);
                Ok(raw.len())
            }
            fn decode_into(
                &self,
                raw: &[u8],
                out: &mut [u8],
            ) -> Result<terminal_codec::Progress, terminal_codec::Error> {
                out[..raw.len()].copy_from_slice(raw);
                Ok(terminal_codec::Progress {
                    consumed: raw.len(),
                    written: raw.len(),
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("recovery");
        let mut registry = Registry::default();
        registry.register(Box::new(Identity)).unwrap();
        let registry = Arc::new(registry);
        let r = Recovery::open_with_compression(
            root.clone(),
            registry.clone(),
            "test-identity-v1".into(),
        )
        .unwrap();
        r.save(1, sample(), 100000).unwrap();
        let saved = fs::read(root.join("s1.checkpoint")).unwrap();
        assert!(
            Recovery::open(root.clone())
                .unwrap()
                .load(1, 100000)
                .is_err()
        );
        let reopened =
            Recovery::open_with_compression(root.clone(), registry, terminal_codec::ZLIB.into())
                .unwrap();
        assert_eq!(
            reopened.load(1, 100000).unwrap().unwrap().bytes,
            sample().bytes
        );
        assert_eq!(fs::read(root.join("s1.checkpoint")).unwrap(), saved);
        reopened.save(1, sample(), 100000).unwrap();
        assert_eq!(
            Recovery::open(root)
                .unwrap()
                .load(1, 100000)
                .unwrap()
                .unwrap()
                .bytes,
            sample().bytes
        );
    }
    #[test]
    fn compressed_checkpoint_round_trips_and_opt_out_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        let bytes = vec![7u8; 20000];
        r.save(
            1,
            Checkpoint {
                bytes: bytes.clone(),
                offset: 123,
                cols: 80,
                rows: 24,
                exited: None,
                resize_epoch: 0,
            },
            100000,
        )
        .unwrap();
        let c = r.load(1, 100000).unwrap().unwrap();
        assert_eq!(c.bytes, bytes);
        assert_eq!(c.offset, 123);
        assert!(fs::metadata(r.root.join("s1.checkpoint")).unwrap().len() < 20000);
        r.remove(1).unwrap();
        assert!(r.load(1, 100000).unwrap().is_none());
    }
    #[test]
    fn older_checkpoints_cannot_clear_a_newer_resize_guard() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        r.mark_resize(1, 2).unwrap();
        let cp = |resize_epoch| Checkpoint {
            bytes: vec![1; 100],
            offset: 0,
            cols: 80,
            rows: 24,
            exited: None,
            resize_epoch,
        };
        r.save(1, cp(1), 4096).unwrap();
        assert_eq!(r.resize_epoch(1).unwrap(), Some(2));
        r.save(1, cp(2), 4096).unwrap();
        assert_eq!(r.resize_epoch(1).unwrap(), None);
    }
    #[test]
    fn oversized_or_excessive_recovery_metadata_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        let path = r.root.join("sessions.json");
        File::create(&path)
            .unwrap()
            .set_len(MAX_METADATA + 1)
            .unwrap();
        assert!(r.settings().is_err());
        let records: BTreeMap<_, _> = (0..257).map(|id| (id, Settings::default())).collect();
        fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
        assert!(r.settings().is_err());
    }

    #[test]
    fn concurrent_checkpoint_and_resize_writes_preserve_both_records() {
        let dir = tempfile::tempdir().unwrap();
        let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
        let checkpoint = recovery.root.join("s1.checkpoint");
        let resize = recovery.root.join("s1.resize");
        let barrier = std::sync::Barrier::new(2);
        let payload = vec![42; 1 << 20];
        let (a, b) = std::thread::scope(|scope| {
            let left = scope.spawn(|| {
                let mut okay = true;
                for _ in 0..32 {
                    barrier.wait();
                    okay &= recovery.atomic(&checkpoint, &payload).is_ok();
                }
                okay
            });
            let right = scope.spawn(|| {
                let mut okay = true;
                for _ in 0..32 {
                    barrier.wait();
                    okay &= recovery.atomic(&resize, &123u64.to_le_bytes()).is_ok();
                }
                okay
            });
            (left.join().unwrap(), right.join().unwrap())
        });
        assert!(
            a && b,
            "independent atomic records must not steal each other's temporary file"
        );
        assert_eq!(fs::read(checkpoint).unwrap(), payload);
        assert_eq!(fs::read(resize).unwrap(), 123u64.to_le_bytes());
    }
    #[test]
    fn reply_cursor_rejects_each_corrupted_byte_before_using_its_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        r.save_reply_cursor(
            1,
            ReplyCursor {
                answered_through: 123,
                pending: false,
            },
        )
        .unwrap();
        let path = r.root.join("s1.replies");
        let good = fs::read(&path).unwrap();
        for index in 0..good.len() {
            let mut corrupt = good.clone();
            corrupt[index] ^= 1;
            fs::write(&path, corrupt).unwrap();
            assert!(
                r.reply_cursor(1).is_err(),
                "corrupt record byte {index} was accepted"
            );
        }
        fs::write(path, good).unwrap();
        assert_eq!(r.reply_cursor(1).unwrap().answered_through, 123);
    }
    #[test]
    fn history_opt_out_removes_checkpoint_temporary_files_after_an_interrupted_write() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        let orphan = r.root.join("s1.checkpoint.tmp");
        let legacy = r.root.join("s1.tmp");
        fs::write(&orphan, b"private incomplete checkpoint").unwrap();
        fs::write(&legacy, b"private legacy incomplete checkpoint").unwrap();
        r.remove(1).unwrap();
        assert!(
            !orphan.exists() && !legacy.exists(),
            "history opt-out retained an interrupted screen checkpoint"
        );
    }
    #[test]
    fn raw_checkpoint_limit_applies_even_when_contents_compress_well() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        assert!(
            r.save(
                1,
                Checkpoint {
                    bytes: vec![0; MAX_CHECKPOINT + 1],
                    offset: 0,
                    cols: 80,
                    rows: 24,
                    exited: None,
                    resize_epoch: 0
                },
                MAX_ENCODED_CHECKPOINT as u64
            )
            .is_err()
        );
        assert!(!r.root.join("s1.checkpoint").exists());
    }
    #[test]
    fn checkpoint_archives_reject_trailing_payload_and_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        r.save(
            1,
            Checkpoint {
                bytes: vec![42; 20000],
                offset: 0,
                cols: 80,
                rows: 24,
                exited: None,
                resize_epoch: 0,
            },
            100000,
        )
        .unwrap();
        let path = r.root.join("s1.checkpoint");
        let good = fs::read(&path).unwrap();
        let mut extra = good.clone();
        extra.extend_from_slice(b"trailing-private-data");
        fs::write(&path, extra).unwrap();
        let rejects_trailing = r.load(1, 100000).is_err();
        fs::write(&path, &good[..good.len() - 1]).unwrap();
        let rejects_truncated = r.load(1, 100000).is_err();
        assert!(
            rejects_trailing && rejects_truncated,
            "archive decoder accepted trailing payload or an incomplete block"
        );
    }
    #[test]
    fn recovery_settings_validate_field_limits_before_returning_policy_or_hints() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        let path = r.root.join("sessions.json");
        let mut settings = Settings::default();
        settings.history = HistoryPolicy::Recent {
            max_lines: 0,
            max_bytes: 64 << 10,
            interval_seconds: 0,
        };
        let invalid = BTreeMap::from([(1, settings)]);
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(r.settings().is_err());
        let mut invalid = BTreeMap::from([(1, Settings::default())]);
        invalid.get_mut(&1).unwrap().recovery = Some(RecoveryHint {
            provider: "fixture".into(),
            session_id: "explicit".into(),
            working_directory: "/tmp".into(),
            state_paths: vec!["bad\0path".into()],
        });
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(r.settings().is_err());
        invalid.get_mut(&1).unwrap().recovery = None;
        r.save_settings(&invalid).unwrap();
        assert_eq!(r.settings().unwrap().len(), 1);
    }
    #[test]
    fn malformed_or_oversized_checkpoints_do_not_replace_a_good_one() {
        let dir = tempfile::tempdir().unwrap();
        let r = Recovery::open(dir.path().join("recovery")).unwrap();
        r.save(
            1,
            Checkpoint {
                bytes: vec![1; 100],
                offset: 1,
                cols: 80,
                rows: 24,
                exited: None,
                resize_epoch: 0,
            },
            1024,
        )
        .unwrap();
        let before = fs::read(r.root.join("s1.checkpoint")).unwrap();
        assert!(
            r.save(
                1,
                Checkpoint {
                    bytes: {
                        let mut seed = 0x1234abcd_u32;
                        (0..4096)
                            .map(|_| {
                                seed ^= seed << 13;
                                seed ^= seed >> 17;
                                seed ^= seed << 5;
                                seed as u8
                            })
                            .collect()
                    },
                    offset: 2,
                    cols: 80,
                    rows: 24,
                    exited: None,
                    resize_epoch: 0
                },
                1024
            )
            .is_err()
        );
        assert_eq!(fs::read(r.root.join("s1.checkpoint")).unwrap(), before);
        fs::write(r.root.join("s2.checkpoint"), b"bad").unwrap();
        assert!(r.load(2, 1024).is_err());
    }
}
