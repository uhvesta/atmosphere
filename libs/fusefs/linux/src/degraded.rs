//! Explicit degraded native-symlink skeleton writer for environments without `/dev/fuse`.
//!
//! When `/dev/fuse` is missing or inaccessible (e.g., unprivileged containers),
//! [`DegradedSkeletonWriter`] materializes a clean local directory tree of native
//! symlinks alongside a `.degraded.json` notice marker. Refreshes are staged in a
//! sibling directory and swapped atomically, and cleanup verifies the marker before
//! removing the skeleton tree.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Marker file placed at the root of every degraded symlink skeleton.
pub const DEGRADED_MARKER_FILENAME: &str = ".degraded.json";

static STAGE_SEQ: AtomicU64 = AtomicU64::new(1);

/// Errors returned by [`DegradedSkeletonWriter`].
#[derive(Debug, Error)]
pub enum DegradedSkeletonError {
    #[error("invalid skeleton root path '{0}'")]
    InvalidRoot(PathBuf),

    #[error("invalid relative entry path '{0}': {1}")]
    InvalidEntryPath(PathBuf, &'static str),

    #[error("refusing to clean up '{0}': missing '{DEGRADED_MARKER_FILENAME}' marker")]
    MissingMarker(PathBuf),

    #[error("failed to serialize degraded notice: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("filesystem I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A single symlink entry to materialize inside the degraded skeleton tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkeletonEntry {
    /// Relative path inside the skeleton root (must not be absolute or contain `..`).
    pub relative_path: PathBuf,
    /// Native symlink target path.
    pub target: PathBuf,
}

impl SkeletonEntry {
    pub fn new(relative_path: impl Into<PathBuf>, target: impl Into<PathBuf>) -> Self {
        Self {
            relative_path: relative_path.into(),
            target: target.into(),
        }
    }
}

/// JSON marker written to `.degraded.json` at the skeleton root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegradedNotice {
    pub schema_version: u32,
    pub mode: String,
    pub reason: String,
    pub read_only: bool,
    pub entry_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl DegradedNotice {
    pub fn new(reason: impl Into<String>, entry_count: usize, hint: Option<String>) -> Self {
        Self {
            schema_version: 1,
            mode: "degraded_symlink_skeleton".to_string(),
            reason: reason.into(),
            read_only: true,
            entry_count,
            hint,
        }
    }
}

/// Summary returned after materializing a degraded symlink skeleton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradedMaterialization {
    pub root: PathBuf,
    pub marker_path: PathBuf,
    pub notice: DegradedNotice,
    pub materialized_links: Vec<PathBuf>,
}

/// Materializes, atomically refreshes, and cleans up a degraded native-symlink skeleton.
#[derive(Debug, Clone)]
pub struct DegradedSkeletonWriter {
    root: PathBuf,
    reason: String,
    hint: Option<String>,
}

impl DegradedSkeletonWriter {
    pub fn new(root: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            reason: reason.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn marker_path(&self) -> PathBuf {
        self.root.join(DEGRADED_MARKER_FILENAME)
    }

    /// Materializes `entries` into a staging sibling directory and atomically swaps it into `root`.
    pub fn materialize(
        &self,
        entries: &[SkeletonEntry],
    ) -> Result<DegradedMaterialization, DegradedSkeletonError> {
        let parent = self
            .root
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| DegradedSkeletonError::InvalidRoot(self.root.clone()))?;
        let root_name = self
            .root
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| DegradedSkeletonError::InvalidRoot(self.root.clone()))?;

        fs::create_dir_all(parent)?;

        // Validate all entries up front before touching disk.
        let mut normalized_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let rel = validate_relative_path(&entry.relative_path)?;
            normalized_entries.push((rel, &entry.target));
        }

        let seq = STAGE_SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let stage_dir = parent.join(format!(".{root_name}.stage-{pid}-{seq}"));
        let backup_dir = parent.join(format!(".{root_name}.old-{pid}-{seq}"));

        // Clean any stale staging path if present, then populate fresh staging tree.
        if stage_dir.exists() {
            fs::remove_dir_all(&stage_dir)?;
        }
        fs::create_dir_all(&stage_dir)?;

        let build_result: Result<Vec<PathBuf>, DegradedSkeletonError> = (|| {
            let mut created = Vec::with_capacity(normalized_entries.len());
            for (rel, target) in &normalized_entries {
                let link_path = stage_dir.join(rel);
                if let Some(link_parent) = link_path.parent() {
                    fs::create_dir_all(link_parent)?;
                }
                create_native_symlink(target, &link_path)?;
                created.push(self.root.join(rel));
            }

            let notice = DegradedNotice::new(
                self.reason.clone(),
                normalized_entries.len(),
                self.hint.clone(),
            );
            let mut notice_bytes = serde_json::to_vec_pretty(&notice)?;
            notice_bytes.push(b'\n');
            fs::write(stage_dir.join(DEGRADED_MARKER_FILENAME), notice_bytes)?;

            Ok(created)
        })();

        let materialized_links = match build_result {
            Ok(links) => links,
            Err(err) => {
                let _ = fs::remove_dir_all(&stage_dir);
                return Err(err);
            }
        };

        // Atomic directory swap: move previous root aside (if any), promote stage_dir, drop backup.
        let had_existing = self.root.exists() || self.root.symlink_metadata().is_ok();
        if had_existing {
            if backup_dir.exists() {
                let _ = fs::remove_dir_all(&backup_dir);
            }
            fs::rename(&self.root, &backup_dir)?;
        }

        if let Err(err) = fs::rename(&stage_dir, &self.root) {
            if had_existing {
                let _ = fs::rename(&backup_dir, &self.root);
            }
            let _ = fs::remove_dir_all(&stage_dir);
            return Err(err.into());
        }

        if had_existing {
            let _ = fs::remove_dir_all(&backup_dir);
        }

        let notice = DegradedNotice::new(
            self.reason.clone(),
            normalized_entries.len(),
            self.hint.clone(),
        );
        Ok(DegradedMaterialization {
            root: self.root.clone(),
            marker_path: self.marker_path(),
            notice,
            materialized_links,
        })
    }

    /// Safely removes the degraded skeleton directory after verifying `.degraded.json` is present.
    pub fn cleanup(&self) -> Result<(), DegradedSkeletonError> {
        if !self.root.exists() && self.root.symlink_metadata().is_err() {
            return Ok(());
        }
        let marker = self.marker_path();
        if !marker.is_file() {
            return Err(DegradedSkeletonError::MissingMarker(self.root.clone()));
        }
        fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

fn validate_relative_path(path: &Path) -> Result<PathBuf, DegradedSkeletonError> {
    if path.as_os_str().is_empty() {
        return Err(DegradedSkeletonError::InvalidEntryPath(
            path.to_path_buf(),
            "path must not be empty",
        ));
    }
    if path.is_absolute() {
        return Err(DegradedSkeletonError::InvalidEntryPath(
            path.to_path_buf(),
            "path must be relative",
        ));
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(seg) => clean.push(seg),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(DegradedSkeletonError::InvalidEntryPath(
                    path.to_path_buf(),
                    "parent traversal ('..') is forbidden",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(DegradedSkeletonError::InvalidEntryPath(
                    path.to_path_buf(),
                    "path must be relative",
                ));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err(DegradedSkeletonError::InvalidEntryPath(
            path.to_path_buf(),
            "path resolves to empty",
        ));
    }
    if clean == Path::new(DEGRADED_MARKER_FILENAME) {
        return Err(DegradedSkeletonError::InvalidEntryPath(
            path.to_path_buf(),
            "cannot overwrite reserved .degraded.json marker",
        ));
    }

    Ok(clean)
}

#[cfg(unix)]
fn create_native_symlink(target: &Path, link_path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link_path)
}

#[cfg(not(unix))]
fn create_native_symlink(target: &Path, link_path: &Path) -> std::io::Result<()> {
    let _ = (target, link_path);
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "native symlink creation requires Unix",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn materialize_refresh_and_cleanup_degraded_skeleton() {
        let tmp = TempDir::new().unwrap();
        let target_a = tmp.path().join("real/a.txt");
        let target_b = tmp.path().join("real/b.txt");
        fs::create_dir_all(target_a.parent().unwrap()).unwrap();
        fs::write(&target_a, b"alpha").unwrap();
        fs::write(&target_b, b"beta").unwrap();

        let mount_root = tmp.path().join("mnt-degraded");
        let writer = DegradedSkeletonWriter::new(&mount_root, "/dev/fuse unavailable")
            .with_hint("Run container with --device /dev/fuse for full FUSEFS mount");

        let mat = writer
            .materialize(&[
                SkeletonEntry::new("docs/a.txt", &target_a),
                SkeletonEntry::new("b.txt", &target_b),
            ])
            .unwrap();

        assert_eq!(mat.notice.mode, "degraded_symlink_skeleton");
        assert!(mat.notice.read_only);
        assert_eq!(mat.notice.entry_count, 2);

        // Verify .degraded.json marker parses cleanly.
        let marker_raw = fs::read(&mat.marker_path).unwrap();
        let parsed: DegradedNotice = serde_json::from_slice(&marker_raw).unwrap();
        assert_eq!(parsed, mat.notice);

        // Verify native symlink targets and readable content.
        let link_a = mount_root.join("docs/a.txt");
        let link_b = mount_root.join("b.txt");
        assert_eq!(fs::read_link(&link_a).unwrap(), target_a);
        assert_eq!(fs::read_link(&link_b).unwrap(), target_b);
        assert_eq!(fs::read(&link_a).unwrap(), b"alpha");

        // Atomic refresh replaces old entries cleanly.
        let refreshed = writer
            .materialize(&[SkeletonEntry::new("only-b.txt", &target_b)])
            .unwrap();
        assert_eq!(refreshed.notice.entry_count, 1);
        assert!(!mount_root.join("docs/a.txt").exists());
        assert_eq!(
            fs::read_link(mount_root.join("only-b.txt")).unwrap(),
            target_b
        );

        // Cleanup removes the skeleton directory cleanly.
        writer.cleanup().unwrap();
        assert!(!mount_root.exists());
        // Idempotent cleanup on already-removed root succeeds.
        writer.cleanup().unwrap();
    }

    #[test]
    fn rejects_escaping_or_reserved_paths_and_refuses_unmarked_cleanup() {
        let tmp = TempDir::new().unwrap();
        let mount_root = tmp.path().join("mnt");
        let writer = DegradedSkeletonWriter::new(&mount_root, "no /dev/fuse");

        let err = writer
            .materialize(&[SkeletonEntry::new("../escape.txt", "/tmp/x")])
            .unwrap_err();
        assert!(matches!(err, DegradedSkeletonError::InvalidEntryPath(_, _)));

        let err_marker = writer
            .materialize(&[SkeletonEntry::new(DEGRADED_MARKER_FILENAME, "/tmp/x")])
            .unwrap_err();
        assert!(matches!(
            err_marker,
            DegradedSkeletonError::InvalidEntryPath(_, _)
        ));

        // Create an unrelated directory without .degraded.json -> cleanup must refuse.
        fs::create_dir_all(&mount_root).unwrap();
        let cleanup_err = writer.cleanup().unwrap_err();
        assert!(matches!(
            cleanup_err,
            DegradedSkeletonError::MissingMarker(_)
        ));
        assert!(mount_root.exists());
    }
}
