//! File-locked hot `.pblog` segment writer, deterministic torn-tail recovery,
//! cold `.pblog.zst` archive reader/writer, and log verification (`ST-1`..`ST-5`, `ST-7`).

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lore_proto::{
    FrameError, LogEvent, LoreError, RECORD_HEADER_LEN, Result, SEGMENT_HEADER_LEN, TailStatus,
    decode_raw_record_frames, decode_segment_frames, encode_record_frame, encode_zstd_frame,
    verify_archive_header, verify_segment_header, write_archive_header, write_segment_header,
};
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};

/// Default rotation threshold for a single hot `.pblog` segment (16 MiB).
pub const DEFAULT_HOT_SEGMENT_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Target uncompressed size per seekable zstd frame inside a cold archive (512 KiB).
pub const ARCHIVE_FRAME_TARGET_BYTES: usize = 512 * 1024;

static ARCHIVE_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns `<root>/hot`.
pub fn hot_dir(root: &Path) -> PathBuf {
    root.join("hot")
}

/// Returns `<root>/archive`.
pub fn archive_dir(root: &Path) -> PathBuf {
    root.join("archive")
}

/// Exclusive advisory file lock on `<root>/write.lock` (`ST-2`).
pub struct WriteLock {
    file: File,
}

impl WriteLock {
    /// Acquires an exclusive blocking `flock` on `<root>/write.lock`.
    pub fn acquire(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|source| LoreError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let lock_path = root.join("write.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| LoreError::Io {
                path: lock_path.clone(),
                source,
            })?;
        flock(&file, FlockOperation::LockExclusive).map_err(|err| LoreError::Io {
            path: lock_path,
            source: io::Error::from_raw_os_error(err.raw_os_error()),
        })?;
        Ok(Self { file })
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = flock(&self.file, FlockOperation::Unlock);
    }
}

/// Summary of startup hot-segment recovery (`ST-4`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub hot_segments_scanned: usize,
    pub hot_events_found: usize,
    pub truncated_tail_bytes: u64,
    pub max_seq: u64,
}

/// Summary of full log & archive verification (`lore verify`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub hot_segments: usize,
    pub hot_events: usize,
    pub archive_segments: usize,
    pub archive_frames: usize,
    pub archive_events: usize,
    pub max_seq: u64,
}

/// Location of a record inside a cold `.pblog.zst` archive (`GC-2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveRecordLoc {
    pub archive_seg: String,
    pub frame_offset: u64,
    pub record_offset: u64,
    pub record_len: u64,
}

/// Lists hot `.pblog` segment paths sorted ascending by filename (`seg-<20d>.pblog`).
pub fn list_hot_segments(root: &Path) -> Result<Vec<PathBuf>> {
    list_sorted_with_ext(&hot_dir(root), ".pblog", "seg-")
}

/// Lists cold `.pblog.zst` archive paths sorted ascending by filename (`arch-<min>-<max>.pblog.zst`).
pub fn list_archive_segments(root: &Path) -> Result<Vec<PathBuf>> {
    list_sorted_with_ext(&archive_dir(root), ".pblog.zst", "arch-")
}

fn list_sorted_with_ext(dir: &Path, suffix: &str, prefix: &str) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(LoreError::Io {
                path: dir.to_path_buf(),
                source,
            });
        }
    };

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LoreError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with(prefix) && name_str.ends_with(suffix) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

/// Scans all hot segments, deterministically truncating a torn tail on the final
/// active hot segment (`ST-4`) and rejecting any mid-segment corruption (`ST-5`).
pub fn recover_hot_segments(root: &Path) -> Result<(RecoveryReport, Option<PathBuf>)> {
    let segments = list_hot_segments(root)?;
    let mut report = RecoveryReport {
        hot_segments_scanned: segments.len(),
        ..Default::default()
    };

    let last_idx = segments.len().saturating_sub(1);
    for (idx, seg_path) in segments.iter().enumerate() {
        let bytes = fs::read(seg_path).map_err(|source| LoreError::Io {
            path: seg_path.clone(),
            source,
        })?;

        let (records, tail) = decode_segment_frames(&bytes).map_err(|source| {
            LoreError::CorruptSegment {
                path: seg_path.clone(),
                offset: 0,
                seq: 0,
                source,
            }
        })?;

        match tail {
            TailStatus::Clean => {}
            TailStatus::TruncatedTail { valid_end_offset } => {
                if idx == last_idx {
                    let old_len = bytes.len() as u64;
                    let file = OpenOptions::new()
                        .write(true)
                        .open(seg_path)
                        .map_err(|source| LoreError::Io {
                            path: seg_path.clone(),
                            source,
                        })?;
                    file.set_len(valid_end_offset).map_err(|source| LoreError::Io {
                        path: seg_path.clone(),
                        source,
                    })?;
                    file.sync_all().map_err(|source| LoreError::Io {
                        path: seg_path.clone(),
                        source,
                    })?;
                    report.truncated_tail_bytes += old_len.saturating_sub(valid_end_offset);
                } else {
                    let seq = records.last().map(|(_, ev)| ev.seq).unwrap_or(0);
                    return Err(LoreError::CorruptSegment {
                        path: seg_path.clone(),
                        offset: valid_end_offset,
                        seq,
                        source: FrameError::TruncatedFrame {
                            expected: RECORD_HEADER_LEN,
                            actual: bytes.len().saturating_sub(valid_end_offset as usize),
                        },
                    });
                }
            }
            TailStatus::MidSegmentCorrupt { offset, seq } => {
                return Err(LoreError::CorruptSegment {
                    path: seg_path.clone(),
                    offset,
                    seq,
                    source: FrameError::RecordChecksumMismatch {
                        seq,
                        expected: 0,
                        actual: 0,
                    },
                });
            }
        }

        report.hot_events_found += records.len();
        if let Some((_, last_ev)) = records.last() {
            report.max_seq = report.max_seq.max(last_ev.seq);
        }
    }

    let active = segments.last().cloned();
    Ok((report, active))
}

/// Appends a single uncompressed [`LogEvent`] frame to the active hot `.pblog`
/// segment and calls `sync_all()` (`ST-2`).
///
/// Caller must hold [`WriteLock`].
pub fn append_hot_event(
    root: &Path,
    active_seg: &mut Option<PathBuf>,
    event: &LogEvent,
    max_segment_bytes: u64,
) -> Result<PathBuf> {
    append_hot_events_batch(root, active_seg, std::slice::from_ref(event), max_segment_bytes)
}

/// Appends one or more uncompressed [`LogEvent`] frames to the active hot `.pblog`
/// segment in a single `write_all()` + single `sync_all()` (`ST-2`, atomic supersession).
///
/// Caller must hold [`WriteLock`].
pub fn append_hot_events_batch(
    root: &Path,
    active_seg: &mut Option<PathBuf>,
    events: &[LogEvent],
    max_segment_bytes: u64,
) -> Result<PathBuf> {
    let dir = hot_dir(root);
    fs::create_dir_all(&dir).map_err(|source| LoreError::Io {
        path: dir.clone(),
        source,
    })?;

    let first_seq = events.first().map(|e| e.seq).unwrap_or(0);
    let mut batch_buf = Vec::new();
    for event in events {
        let frame = encode_record_frame(event).map_err(|source| LoreError::CorruptSegment {
            path: active_seg
                .clone()
                .unwrap_or_else(|| dir.join(format!("seg-{:020}.pblog", event.seq))),
            offset: 0,
            seq: event.seq,
            source,
        })?;
        batch_buf.extend_from_slice(&frame);
    }

    let need_new_segment = match active_seg.as_ref() {
        None => true,
        Some(path) => match fs::metadata(path) {
            Ok(meta) => meta.len() >= max_segment_bytes,
            Err(_) => true,
        },
    };

    let seg_path = if need_new_segment {
        let new_path = dir.join(format!("seg-{first_seq:020}.pblog"));
        let mut full = Vec::with_capacity(SEGMENT_HEADER_LEN + batch_buf.len());
        write_segment_header(&mut full).map_err(|source| LoreError::CorruptSegment {
            path: new_path.clone(),
            offset: 0,
            seq: first_seq,
            source,
        })?;
        full.extend_from_slice(&batch_buf);
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&new_path)
            .map_err(|source| LoreError::Io {
                path: new_path.clone(),
                source,
            })?;
        file.write_all(&full).map_err(|source| LoreError::Io {
            path: new_path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| LoreError::Io {
            path: new_path.clone(),
            source,
        })?;
        *active_seg = Some(new_path.clone());
        new_path
    } else {
        let path = active_seg.clone().expect("checked Some above");
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|source| LoreError::Io {
                path: path.clone(),
                source,
            })?;
        file.write_all(&batch_buf).map_err(|source| LoreError::Io {
            path: path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| LoreError::Io {
            path: path.clone(),
            source,
        })?;
        path
    };

    Ok(seg_path)
}

/// Decodes all independent zstd frames inside a cold `.pblog.zst` archive file.
///
/// Returns `(frame_offset_in_file, Vec<(record_offset_in_frame, record_len, LogEvent)>)`.
pub fn read_archive_file(
    path: &Path,
) -> Result<Vec<(u64, Vec<(u64, u64, LogEvent)>)>> {
    let bytes = fs::read(path).map_err(|source| LoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    verify_archive_header(&bytes).map_err(|source| LoreError::CorruptSegment {
        path: path.to_path_buf(),
        offset: 0,
        seq: 0,
        source,
    })?;

    let mut out = Vec::new();
    let mut cursor_pos = SEGMENT_HEADER_LEN;

    while cursor_pos < bytes.len() {
        let frame_offset = cursor_pos as u64;
        let slice = &bytes[cursor_pos..];
        let mut cursor = io::Cursor::new(slice);
        let mut decoder = zstd::stream::read::Decoder::new(&mut cursor)
            .map_err(|err| LoreError::CorruptSegment {
                path: path.to_path_buf(),
                offset: frame_offset,
                seq: 0,
                source: FrameError::Io(err),
            })?
            .single_frame();

        let mut raw_frame = Vec::new();
        decoder
            .read_to_end(&mut raw_frame)
            .map_err(|err| LoreError::CorruptSegment {
                path: path.to_path_buf(),
                offset: frame_offset,
                seq: 0,
                source: FrameError::Io(err),
            })?;

        let consumed = cursor.position() as usize;
        if consumed == 0 {
            return Err(LoreError::CorruptSegment {
                path: path.to_path_buf(),
                offset: frame_offset,
                seq: 0,
                source: FrameError::TruncatedFrame {
                    expected: 1,
                    actual: 0,
                },
            });
        }
        cursor_pos += consumed;

        let decoded = decode_raw_record_frames(&raw_frame).map_err(|source| {
            LoreError::CorruptSegment {
                path: path.to_path_buf(),
                offset: frame_offset,
                seq: 0,
                source,
            }
        })?;

        let mut recs = Vec::with_capacity(decoded.len());
        for (idx, (rec_off, ev)) in decoded.iter().enumerate() {
            let next_off = decoded
                .get(idx + 1)
                .map(|(o, _)| *o)
                .unwrap_or(raw_frame.len() as u64);
            recs.push((*rec_off, next_off.saturating_sub(*rec_off), ev.clone()));
        }
        out.push((frame_offset, recs));
    }

    Ok(out)
}

/// Hydrates a single [`LogEvent`] from a cold `.pblog.zst` archive by seeking
/// directly to `frame_offset` and decompressing only that zstd frame (`GC-3`).
pub fn hydrate_archive_event(
    root: &Path,
    claim_id: &str,
    archive_seg: &str,
    frame_offset: u64,
    record_offset: u64,
    record_len: u64,
) -> Result<LogEvent> {
    let path = archive_dir(root).join(archive_seg);
    let mut file = File::open(&path).map_err(|err| LoreError::ArchiveHydrationFailed {
        claim_id: claim_id.to_string(),
        segment: path.clone(),
        source: FrameError::Io(err),
    })?;

    file.seek(SeekFrom::Start(frame_offset))
        .map_err(|err| LoreError::ArchiveHydrationFailed {
            claim_id: claim_id.to_string(),
            segment: path.clone(),
            source: FrameError::Io(err),
        })?;

    let mut decoder = zstd::stream::read::Decoder::new(BufReader::new(file))
        .map_err(|err| LoreError::ArchiveHydrationFailed {
            claim_id: claim_id.to_string(),
            segment: path.clone(),
            source: FrameError::Io(err),
        })?
        .single_frame();

    let mut raw_frame = Vec::new();
    decoder
        .read_to_end(&mut raw_frame)
        .map_err(|err| LoreError::ArchiveHydrationFailed {
            claim_id: claim_id.to_string(),
            segment: path.clone(),
            source: FrameError::Io(err),
        })?;

    let start = record_offset as usize;
    let end = start.saturating_add(record_len as usize);
    if end > raw_frame.len() || start >= end {
        return Err(LoreError::ArchiveHydrationFailed {
            claim_id: claim_id.to_string(),
            segment: path,
            source: FrameError::TruncatedFrame {
                expected: end,
                actual: raw_frame.len(),
            },
        });
    }

    let mut records = decode_raw_record_frames(&raw_frame[start..end]).map_err(|source| {
        LoreError::ArchiveHydrationFailed {
            claim_id: claim_id.to_string(),
            segment: path.clone(),
            source,
        }
    })?;

    records
        .pop()
        .map(|(_, ev)| ev)
        .ok_or_else(|| LoreError::ArchiveHydrationFailed {
            claim_id: claim_id.to_string(),
            segment: path,
            source: FrameError::TruncatedFrame {
                expected: RECORD_HEADER_LEN,
                actual: 0,
            },
        })
}

/// Writes a batch of [`LogEvent`]s into a new cold `.pblog.zst` archive file
/// composed of seekable zstd frames (level 6, XXH64 checksum) (`GC-2`).
///
/// Returns `(archive_filename, Vec<(seq, ArchiveRecordLoc)>)`.
pub fn write_archive_segment(
    root: &Path,
    events: &[LogEvent],
) -> Result<(String, Vec<(u64, ArchiveRecordLoc)>)> {
    if events.is_empty() {
        return Err(LoreError::Io {
            path: archive_dir(root),
            source: io::Error::new(io::ErrorKind::InvalidInput, "cannot write empty archive"),
        });
    }

    let dir = archive_dir(root);
    fs::create_dir_all(&dir).map_err(|source| LoreError::Io {
        path: dir.clone(),
        source,
    })?;

    let min_seq = events.iter().map(|e| e.seq).min().unwrap_or(0);
    let max_seq = events.iter().map(|e| e.seq).max().unwrap_or(0);

    // Choose a unique filename if an archive with the exact same seq range already exists.
    let mut seg_name = format!("arch-{min_seq:020}-{max_seq:020}.pblog.zst");
    let mut target_path = dir.join(&seg_name);
    let mut suffix = 1u32;
    while target_path.exists() {
        seg_name = format!("arch-{min_seq:020}-{max_seq:020}-{suffix}.pblog.zst");
        target_path = dir.join(&seg_name);
        suffix += 1;
    }

    let mut file_buf = Vec::new();
    write_archive_header(&mut file_buf).map_err(|source| LoreError::CorruptSegment {
        path: target_path.clone(),
        offset: 0,
        seq: min_seq,
        source,
    })?;

    let mut locations = Vec::with_capacity(events.len());
    let mut chunk_raw = Vec::new();
    let mut chunk_entries: Vec<(u64, u64, u64)> = Vec::new(); // (seq, rec_off, rec_len)

    let flush_chunk = |file_buf: &mut Vec<u8>,
                       chunk_raw: &mut Vec<u8>,
                       chunk_entries: &mut Vec<(u64, u64, u64)>,
                       locations: &mut Vec<(u64, ArchiveRecordLoc)>|
     -> Result<()> {
        if chunk_raw.is_empty() {
            return Ok(());
        }
        let frame_offset = file_buf.len() as u64;
        let compressed = encode_zstd_frame(chunk_raw).map_err(|source| LoreError::CorruptSegment {
            path: target_path.clone(),
            offset: frame_offset,
            seq: chunk_entries.first().map(|e| e.0).unwrap_or(0),
            source,
        })?;
        file_buf.extend_from_slice(&compressed);
        for (seq, rec_off, rec_len) in chunk_entries.drain(..) {
            locations.push((
                seq,
                ArchiveRecordLoc {
                    archive_seg: seg_name.clone(),
                    frame_offset,
                    record_offset: rec_off,
                    record_len: rec_len,
                },
            ));
        }
        chunk_raw.clear();
        Ok(())
    };

    for ev in events {
        let encoded = encode_record_frame(ev).map_err(|source| LoreError::CorruptSegment {
            path: target_path.clone(),
            offset: 0,
            seq: ev.seq,
            source,
        })?;
        if !chunk_raw.is_empty() && chunk_raw.len() + encoded.len() > ARCHIVE_FRAME_TARGET_BYTES {
            flush_chunk(&mut file_buf, &mut chunk_raw, &mut chunk_entries, &mut locations)?;
        }
        let rec_off = chunk_raw.len() as u64;
        let rec_len = encoded.len() as u64;
        chunk_raw.extend_from_slice(&encoded);
        chunk_entries.push((ev.seq, rec_off, rec_len));
    }
    flush_chunk(&mut file_buf, &mut chunk_raw, &mut chunk_entries, &mut locations)?;

    atomic_write_bytes(&target_path, &file_buf)?;
    Ok((seg_name, locations))
}

/// Atomically rewrites a hot `.pblog` segment, keeping only events that pass `keep`.
///
/// If no events remain, removes the segment file. Returns `(kept_count, removed_count)`.
pub fn rewrite_hot_segment_filtered<F>(
    seg_path: &Path,
    mut keep: F,
) -> Result<(usize, usize)>
where
    F: FnMut(&LogEvent) -> bool,
{
    let bytes = fs::read(seg_path).map_err(|source| LoreError::Io {
        path: seg_path.to_path_buf(),
        source,
    })?;
    let (records, tail) = decode_segment_frames(&bytes).map_err(|source| LoreError::CorruptSegment {
        path: seg_path.to_path_buf(),
        offset: 0,
        seq: 0,
        source,
    })?;
    match tail {
        TailStatus::Clean => {}
        TailStatus::MidSegmentCorrupt { offset, seq } => {
            return Err(LoreError::CorruptSegment {
                path: seg_path.to_path_buf(),
                offset,
                seq,
                source: FrameError::RecordChecksumMismatch {
                    seq,
                    expected: 0,
                    actual: 0,
                },
            });
        }
        TailStatus::TruncatedTail { valid_end_offset } => {
            let seq = records.last().map(|(_, e)| e.seq).unwrap_or(0);
            return Err(LoreError::CorruptSegment {
                path: seg_path.to_path_buf(),
                offset: valid_end_offset,
                seq,
                source: FrameError::TruncatedFrame {
                    expected: RECORD_HEADER_LEN,
                    actual: bytes.len().saturating_sub(valid_end_offset as usize),
                },
            });
        }
    }

    let mut kept = Vec::new();
    let mut removed = 0usize;
    for (_, ev) in records {
        if keep(&ev) {
            kept.push(ev);
        } else {
            removed += 1;
        }
    }

    if removed == 0 {
        return Ok((kept.len(), 0));
    }

    if kept.is_empty() {
        fs::remove_file(seg_path).map_err(|source| LoreError::Io {
            path: seg_path.to_path_buf(),
            source,
        })?;
        return Ok((0, removed));
    }

    let mut buf = Vec::new();
    write_segment_header(&mut buf).map_err(|source| LoreError::CorruptSegment {
        path: seg_path.to_path_buf(),
        offset: 0,
        seq: 0,
        source,
    })?;
    for ev in &kept {
        let frame = encode_record_frame(ev).map_err(|source| LoreError::CorruptSegment {
            path: seg_path.to_path_buf(),
            offset: 0,
            seq: ev.seq,
            source,
        })?;
        buf.extend_from_slice(&frame);
    }

    atomic_write_bytes(seg_path, &buf)?;
    Ok((kept.len(), removed))
}

/// Atomically rewrites a cold `.pblog.zst` archive segment, keeping only events
/// that pass `keep`, and returns updated [`ArchiveRecordLoc`]s for all remaining
/// events in that archive.
///
/// If no events remain, removes the archive file.
pub fn rewrite_archive_segment_filtered<F>(
    arch_path: &Path,
    mut keep: F,
) -> Result<(usize, usize, Vec<(u64, ArchiveRecordLoc)>)>
where
    F: FnMut(&LogEvent) -> bool,
{
    let frames = read_archive_file(arch_path)?;
    let mut kept = Vec::new();
    let mut removed = 0usize;
    for (_, recs) in frames {
        for (_, _, ev) in recs {
            if keep(&ev) {
                kept.push(ev);
            } else {
                removed += 1;
            }
        }
    }

    if removed == 0 {
        return Ok((kept.len(), 0, Vec::new()));
    }

    if kept.is_empty() {
        fs::remove_file(arch_path).map_err(|source| LoreError::Io {
            path: arch_path.to_path_buf(),
            source,
        })?;
        return Ok((0, removed, Vec::new()));
    }

    let seg_name = arch_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("arch.pblog.zst")
        .to_string();

    let mut file_buf = Vec::new();
    write_archive_header(&mut file_buf).map_err(|source| LoreError::CorruptSegment {
        path: arch_path.to_path_buf(),
        offset: 0,
        seq: 0,
        source,
    })?;

    let mut locations = Vec::with_capacity(kept.len());
    let mut chunk_raw = Vec::new();
    let mut chunk_entries: Vec<(u64, u64, u64)> = Vec::new();

    let flush_chunk = |file_buf: &mut Vec<u8>,
                       chunk_raw: &mut Vec<u8>,
                       chunk_entries: &mut Vec<(u64, u64, u64)>,
                       locations: &mut Vec<(u64, ArchiveRecordLoc)>|
     -> Result<()> {
        if chunk_raw.is_empty() {
            return Ok(());
        }
        let frame_offset = file_buf.len() as u64;
        let compressed = encode_zstd_frame(chunk_raw).map_err(|source| LoreError::CorruptSegment {
            path: arch_path.to_path_buf(),
            offset: frame_offset,
            seq: chunk_entries.first().map(|e| e.0).unwrap_or(0),
            source,
        })?;
        file_buf.extend_from_slice(&compressed);
        for (seq, rec_off, rec_len) in chunk_entries.drain(..) {
            locations.push((
                seq,
                ArchiveRecordLoc {
                    archive_seg: seg_name.clone(),
                    frame_offset,
                    record_offset: rec_off,
                    record_len: rec_len,
                },
            ));
        }
        chunk_raw.clear();
        Ok(())
    };

    for ev in &kept {
        let encoded = encode_record_frame(ev).map_err(|source| LoreError::CorruptSegment {
            path: arch_path.to_path_buf(),
            offset: 0,
            seq: ev.seq,
            source,
        })?;
        if !chunk_raw.is_empty() && chunk_raw.len() + encoded.len() > ARCHIVE_FRAME_TARGET_BYTES {
            flush_chunk(&mut file_buf, &mut chunk_raw, &mut chunk_entries, &mut locations)?;
        }
        let rec_off = chunk_raw.len() as u64;
        let rec_len = encoded.len() as u64;
        chunk_raw.extend_from_slice(&encoded);
        chunk_entries.push((ev.seq, rec_off, rec_len));
    }
    flush_chunk(&mut file_buf, &mut chunk_raw, &mut chunk_entries, &mut locations)?;

    atomic_write_bytes(arch_path, &file_buf)?;
    Ok((kept.len(), removed, locations))
}

/// Streams all [`LogEvent`]s from cold `.pblog.zst` archives and hot `.pblog`
/// segments with `seq > since_seq`, sorted ascending by `seq` (`ST-7`).
pub fn stream_log_events(root: &Path, since_seq: u64) -> Result<Vec<LogEvent>> {
    let mut by_seq: BTreeMap<u64, LogEvent> = BTreeMap::new();

    for arch_path in list_archive_segments(root)? {
        for (_, recs) in read_archive_file(&arch_path)? {
            for (_, _, ev) in recs {
                if ev.seq > since_seq {
                    by_seq.insert(ev.seq, ev);
                }
            }
        }
    }

    let hot_segs = list_hot_segments(root)?;
    let last_idx = hot_segs.len().saturating_sub(1);
    for (idx, seg_path) in hot_segs.into_iter().enumerate() {
        let bytes = fs::read(&seg_path).map_err(|source| LoreError::Io {
            path: seg_path.clone(),
            source,
        })?;
        let (records, tail) = decode_segment_frames(&bytes).map_err(|source| {
            LoreError::CorruptSegment {
                path: seg_path.clone(),
                offset: 0,
                seq: 0,
                source,
            }
        })?;
        match tail {
            TailStatus::Clean => {}
            TailStatus::MidSegmentCorrupt { offset, seq } => {
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset,
                    seq,
                    source: FrameError::RecordChecksumMismatch {
                        seq,
                        expected: 0,
                        actual: 0,
                    },
                });
            }
            TailStatus::TruncatedTail { valid_end_offset } if idx < last_idx => {
                let seq = records.last().map(|(_, e)| e.seq).unwrap_or(0);
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset: valid_end_offset,
                    seq,
                    source: FrameError::TruncatedFrame {
                        expected: RECORD_HEADER_LEN,
                        actual: bytes.len().saturating_sub(valid_end_offset as usize),
                    },
                });
            }
            TailStatus::TruncatedTail { .. } => {}
        }
        for (_, ev) in records {
            if ev.seq > since_seq {
                by_seq.insert(ev.seq, ev);
            }
        }
    }

    Ok(by_seq.into_values().collect())
}

/// Streams only events currently stored in hot `.pblog` segments (used by Tier-1 archival
/// so already-archived cold records are never duplicated).
pub fn stream_hot_events(root: &Path) -> Result<Vec<LogEvent>> {
    let mut by_seq: BTreeMap<u64, LogEvent> = BTreeMap::new();
    let hot_segs = list_hot_segments(root)?;
    let last_idx = hot_segs.len().saturating_sub(1);

    for (idx, seg_path) in hot_segs.into_iter().enumerate() {
        let bytes = fs::read(&seg_path).map_err(|source| LoreError::Io {
            path: seg_path.clone(),
            source,
        })?;
        let (records, tail) = decode_segment_frames(&bytes).map_err(|source| {
            LoreError::CorruptSegment {
                path: seg_path.clone(),
                offset: 0,
                seq: 0,
                source,
            }
        })?;
        match tail {
            TailStatus::Clean => {}
            TailStatus::MidSegmentCorrupt { offset, seq } => {
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset,
                    seq,
                    source: FrameError::RecordChecksumMismatch {
                        seq,
                        expected: 0,
                        actual: 0,
                    },
                });
            }
            TailStatus::TruncatedTail { valid_end_offset } if idx < last_idx => {
                let seq = records.last().map(|(_, e)| e.seq).unwrap_or(0);
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset: valid_end_offset,
                    seq,
                    source: FrameError::TruncatedFrame {
                        expected: RECORD_HEADER_LEN,
                        actual: bytes.len().saturating_sub(valid_end_offset as usize),
                    },
                });
            }
            TailStatus::TruncatedTail { .. } => {}
        }
        for (_, ev) in records {
            by_seq.insert(ev.seq, ev);
        }
    }

    Ok(by_seq.into_values().collect())
}

/// Streams all events along with optional archive location for cold-archived events,
/// used by `reindex()` to reconstruct `index.db` deterministically (`ST-1`).
pub fn stream_events_with_archive_locs(
    root: &Path,
) -> Result<Vec<(LogEvent, Option<ArchiveRecordLoc>)>> {
    let mut by_seq: BTreeMap<u64, (LogEvent, Option<ArchiveRecordLoc>)> = BTreeMap::new();

    for arch_path in list_archive_segments(root)? {
        let seg_name = arch_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        for (frame_offset, recs) in read_archive_file(&arch_path)? {
            for (record_offset, record_len, ev) in recs {
                by_seq.insert(
                    ev.seq,
                    (
                        ev,
                        Some(ArchiveRecordLoc {
                            archive_seg: seg_name.clone(),
                            frame_offset,
                            record_offset,
                            record_len,
                        }),
                    ),
                );
            }
        }
    }

    let hot_segs = list_hot_segments(root)?;
    let last_idx = hot_segs.len().saturating_sub(1);
    for (idx, seg_path) in hot_segs.into_iter().enumerate() {
        let bytes = fs::read(&seg_path).map_err(|source| LoreError::Io {
            path: seg_path.clone(),
            source,
        })?;
        let (records, tail) = decode_segment_frames(&bytes).map_err(|source| {
            LoreError::CorruptSegment {
                path: seg_path.clone(),
                offset: 0,
                seq: 0,
                source,
            }
        })?;
        match tail {
            TailStatus::Clean => {}
            TailStatus::MidSegmentCorrupt { offset, seq } => {
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset,
                    seq,
                    source: FrameError::RecordChecksumMismatch {
                        seq,
                        expected: 0,
                        actual: 0,
                    },
                });
            }
            TailStatus::TruncatedTail { valid_end_offset } if idx < last_idx => {
                let seq = records.last().map(|(_, e)| e.seq).unwrap_or(0);
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset: valid_end_offset,
                    seq,
                    source: FrameError::TruncatedFrame {
                        expected: RECORD_HEADER_LEN,
                        actual: bytes.len().saturating_sub(valid_end_offset as usize),
                    },
                });
            }
            TailStatus::TruncatedTail { .. } => {}
        }
        for (_, ev) in records {
            // Hot record supersedes any cold duplicate for the same seq.
            by_seq.insert(ev.seq, (ev, None));
        }
    }

    Ok(by_seq.into_values().collect())
}

/// Verifies every hot `.pblog` segment and every cold `.pblog.zst` archive (`lore verify`).
pub fn verify_all_logs(root: &Path) -> Result<VerifyReport> {
    let mut report = VerifyReport::default();

    for seg_path in list_hot_segments(root)? {
        let bytes = fs::read(&seg_path).map_err(|source| LoreError::Io {
            path: seg_path.clone(),
            source,
        })?;
        let _hdr = verify_segment_header(&bytes).map_err(|source| LoreError::CorruptSegment {
            path: seg_path.clone(),
            offset: 0,
            seq: 0,
            source,
        })?;
        let (records, tail) = decode_segment_frames(&bytes).map_err(|source| {
            LoreError::CorruptSegment {
                path: seg_path.clone(),
                offset: 0,
                seq: 0,
                source,
            }
        })?;
        match tail {
            TailStatus::Clean => {}
            TailStatus::TruncatedTail { valid_end_offset } => {
                let seq = records.last().map(|(_, e)| e.seq).unwrap_or(0);
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset: valid_end_offset,
                    seq,
                    source: FrameError::TruncatedFrame {
                        expected: RECORD_HEADER_LEN,
                        actual: bytes.len().saturating_sub(valid_end_offset as usize),
                    },
                });
            }
            TailStatus::MidSegmentCorrupt { offset, seq } => {
                return Err(LoreError::CorruptSegment {
                    path: seg_path,
                    offset,
                    seq,
                    source: FrameError::RecordChecksumMismatch {
                        seq,
                        expected: 0,
                        actual: 0,
                    },
                });
            }
        }
        report.hot_segments += 1;
        report.hot_events += records.len();
        if let Some((_, last)) = records.last() {
            report.max_seq = report.max_seq.max(last.seq);
        }
    }

    for arch_path in list_archive_segments(root)? {
        let frames = read_archive_file(&arch_path)?;
        report.archive_segments += 1;
        report.archive_frames += frames.len();
        for (_, recs) in frames {
            report.archive_events += recs.len();
            if let Some((_, _, last)) = recs.last() {
                report.max_seq = report.max_seq.max(last.seq);
            }
        }
    }

    Ok(report)
}

/// Computes total bytes on disk across `hot/*.pblog`.
pub fn total_hot_bytes(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    for path in list_hot_segments(root)? {
        if let Ok(meta) = fs::metadata(&path) {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Computes total bytes on disk across `archive/*.pblog.zst`.
pub fn total_archive_bytes(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    for path in list_archive_segments(root)? {
        if let Ok(meta) = fs::metadata(&path) {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Collects all blob SHA-256 digests referenced by any `ClaimUpserted` event in
/// hot segments or cold archives (`GC-4` mark phase).
pub fn collect_referenced_blobs(root: &Path) -> Result<HashSet<String>> {
    let mut referenced = HashSet::new();
    for ev in stream_log_events(root, 0)? {
        if let Some(lore_proto::log_event::Payload::ClaimUpserted(up)) = ev.payload {
            if let Some(claim) = up.claim {
                for v in claim.verifiers {
                    if let Some(lore_proto::verifier::Check::BlobRef(b)) = v.check {
                        if !b.sha256.is_empty() {
                            referenced.insert(b.sha256.to_ascii_lowercase());
                        }
                    }
                }
            }
        }
    }
    Ok(referenced)
}

fn atomic_write_bytes(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| LoreError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let nonce = ARCHIVE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("segment");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{}.{nonce}", std::process::id()));

    let write_res = (|| -> io::Result<()> {
        let mut f = File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();

    if let Err(source) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(LoreError::Io {
            path: tmp_path,
            source,
        });
    }

    fs::rename(&tmp_path, target).map_err(|source| {
        let _ = fs::remove_file(&tmp_path);
        LoreError::Io {
            path: target.to_path_buf(),
            source,
        }
    })?;

    Ok(())
}
