//! Content-addressed evidence blob storage (`ST-8`, `GC-4`).
//!
//! Blobs live at `<root>/blobs/<sha256>.blob` using `lore_proto::encode_blob_bytes`
//! (`LRB0` raw vs `LRBZ` zstd when `>= 1 KiB` and saving `>= 6.25%`).

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lore_proto::{
    LoreError, Result, blob_sha256_hex, decode_blob_bytes, encode_blob_bytes,
};

static BLOB_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns `<root>/blobs`.
pub fn blobs_dir(root: &Path) -> PathBuf {
    root.join("blobs")
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn blob_path(root: &Path, sha256: &str) -> Result<PathBuf> {
    let normalized = sha256.to_ascii_lowercase();
    if !is_valid_sha256_hex(&normalized) {
        return Err(LoreError::Io {
            path: blobs_dir(root).join(sha256),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid blob sha256 digest: {sha256}"),
            ),
        });
    }
    Ok(blobs_dir(root).join(format!("{normalized}.blob")))
}

/// Stores `raw` under its SHA-256 digest, returning the lowercase hex digest.
///
/// Deduplicated: if `<sha256>.blob` already exists, no rewrite occurs.
pub fn put_blob(root: &Path, raw: &[u8]) -> Result<String> {
    let dir = blobs_dir(root);
    fs::create_dir_all(&dir).map_err(|source| LoreError::Io {
        path: dir.clone(),
        source,
    })?;

    let sha256 = blob_sha256_hex(raw);
    let target = dir.join(format!("{sha256}.blob"));
    if target.exists() {
        return Ok(sha256);
    }

    let encoded = encode_blob_bytes(raw).map_err(|err| LoreError::Io {
        path: target.clone(),
        source: io::Error::other(err.to_string()),
    })?;

    let nonce = BLOB_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!(".{sha256}.tmp.{}.{nonce}", std::process::id()));

    let write_res = (|| -> io::Result<()> {
        let mut f = File::create(&tmp_path)?;
        f.write_all(&encoded)?;
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

    fs::rename(&tmp_path, &target).map_err(|source| {
        let _ = fs::remove_file(&tmp_path);
        LoreError::Io {
            path: target,
            source,
        }
    })?;

    Ok(sha256)
}

/// Loads and decodes a content-addressed blob by its SHA-256 digest.
pub fn get_blob(root: &Path, sha256: &str) -> Result<Vec<u8>> {
    let path = blob_path(root, sha256)?;
    let encoded = fs::read(&path).map_err(|source| LoreError::Io {
        path: path.clone(),
        source,
    })?;
    decode_blob_bytes(&encoded).map_err(|err| LoreError::Io {
        path,
        source: io::Error::new(io::ErrorKind::InvalidData, err.to_string()),
    })
}

/// Returns `true` if `<root>/blobs/<sha256>.blob` exists on disk.
pub fn blob_exists(root: &Path, sha256: &str) -> bool {
    blob_path(root, sha256).map(|p| p.exists()).unwrap_or(false)
}

/// Lists all valid `<sha256>.blob` digests present in `<root>/blobs`.
pub fn list_blob_hashes(root: &Path) -> Result<Vec<String>> {
    let dir = blobs_dir(root);
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(LoreError::Io { path: dir, source }),
    };

    let mut hashes = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LoreError::Io {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if let Some(stem) = name_str.strip_suffix(".blob") {
            if is_valid_sha256_hex(stem) {
                hashes.push(stem.to_ascii_lowercase());
            }
        }
    }
    hashes.sort();
    Ok(hashes)
}

/// Removes any `.blob` files whose SHA-256 digest is not in `referenced`.
///
/// Returns `(removed_count, freed_bytes)`.
pub fn sweep_unreferenced_blobs(
    root: &Path,
    referenced: &HashSet<String>,
    dry_run: bool,
) -> Result<(u64, u64)> {
    let dir = blobs_dir(root);
    let mut removed = 0u64;
    let mut freed_bytes = 0u64;

    for hash in list_blob_hashes(root)? {
        if referenced.contains(&hash) {
            continue;
        }
        let path = dir.join(format!("{hash}.blob"));
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if !dry_run {
            fs::remove_file(&path).map_err(|source| LoreError::Io {
                path: path.clone(),
                source,
            })?;
        }
        removed += 1;
        freed_bytes += len;
    }

    Ok((removed, freed_bytes))
}

/// Total bytes on disk across `<root>/blobs/*.blob`.
pub fn total_blob_bytes(root: &Path) -> Result<u64> {
    let dir = blobs_dir(root);
    let mut total = 0u64;
    for hash in list_blob_hashes(root)? {
        let path = dir.join(format!("{hash}.blob"));
        if let Ok(meta) = fs::metadata(&path) {
            total += meta.len();
        }
    }
    Ok(total)
}
