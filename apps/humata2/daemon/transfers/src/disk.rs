//! This machine's file system as an [`End`].

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use humata2_durable::{File, create_dir_all, sync_dir};
use sha2::{Digest, Sha256};

use crate::end::{End, Entry, Kind, Sink};
use crate::error::EndError;
use crate::fsutil::{remove_all, rename_keeping_mode};
use crate::trash::{NewItem, Trash};

pub struct Disk {
    label: String,
    trash: Arc<Trash>,
}

impl Disk {
    pub fn new(label: impl Into<String>, trash: Arc<Trash>) -> Self {
        Self {
            label: label.into(),
            trash,
        }
    }
}

fn fail(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> EndError {
    let path = path.to_owned();
    move |source| match source.kind() {
        ErrorKind::NotFound => EndError::NotFound { path, source },
        ErrorKind::AlreadyExists => EndError::Exists { path },
        _ => EndError::Io { doing, path, source },
    }
}

fn modified_ms(meta: &fs::Metadata) -> i64 {
    meta.mtime()
        .saturating_mul(1000)
        .saturating_add(meta.mtime_nsec() / 1_000_000)
}

fn walk(path: &Path, rel: &str, out: &mut Vec<Entry>) -> Result<(), EndError> {
    let meta = fs::symlink_metadata(path).map_err(fail("look at", path))?;
    let kind = meta.file_type();
    let mut entry = Entry {
        rel: rel.to_owned(),
        kind: Kind::File,
        size: 0,
        mode: meta.mode() & 0o7777,
        modified_unix_ms: modified_ms(&meta),
        link_target: None,
    };
    if kind.is_symlink() {
        entry.kind = Kind::Link;
        entry.link_target = Some(fs::read_link(path).map_err(fail("read the link", path))?);
        out.push(entry);
    } else if kind.is_file() {
        entry.size = meta.len();
        out.push(entry);
    } else if kind.is_dir() {
        entry.kind = Kind::Folder;
        out.push(entry);
        let mut names: Vec<String> = fs::read_dir(path)
            .map_err(fail("read the folder", path))?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()
            .map_err(fail("read the folder", path))?;
        names.sort();
        for name in names {
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            walk(&path.join(&name), &child, out)?;
        }
    } else {
        use std::os::unix::fs::FileTypeExt;
        let what = if kind.is_socket() {
            "socket"
        } else if kind.is_fifo() {
            "named pipe"
        } else {
            "device"
        };
        return Err(EndError::Unsupported {
            path: path.to_owned(),
            what,
        });
    }
    Ok(())
}

struct DiskSink {
    file: File,
    pos: u64,
    path: PathBuf,
}

impl Sink for DiskSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), EndError> {
        self.file
            .write_all_at(bytes, self.pos)
            .map_err(fail("write", &self.path))?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn sync(&mut self) -> Result<(), EndError> {
        self.file.sync_all().map_err(fail("save", &self.path))
    }
}

impl End for Disk {
    fn label(&self) -> &str {
        &self.label
    }

    fn same_disk(&self, other: &dyn End, from: &Path, to_folder: &Path) -> bool {
        if other.label() != self.label {
            return false;
        }
        match (fs::symlink_metadata(from), fs::metadata(to_folder)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => false,
        }
    }

    fn plan(&self, source: &Path) -> Result<Vec<Entry>, EndError> {
        let mut out = Vec::new();
        walk(source, "", &mut out)?;
        Ok(out)
    }

    fn open_read(&self, path: &Path, offset: u64) -> Result<Box<dyn Read + Send>, EndError> {
        let mut file = fs::File::open(path).map_err(fail("open", path))?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))
                .map_err(fail("read", path))?;
        }
        Ok(Box::new(file))
    }

    fn free_bytes(&self, folder: &Path) -> Result<u64, EndError> {
        let stat = rustix::fs::statvfs(folder).map_err(|errno| EndError::Io {
            doing: "measure the free space of",
            path: folder.to_owned(),
            source: errno.into(),
        })?;
        Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
    }

    fn exists(&self, path: &Path) -> Result<bool, EndError> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(fail("look at", path)(error)),
        }
    }

    fn make_folder(&self, path: &Path, mode: u32) -> Result<(), EndError> {
        if fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
            // Made by an earlier run, which may have died before saving it.
            if let Some(parent) = path.parent() {
                sync_dir(parent).map_err(fail("save", parent))?;
            }
            return Ok(());
        }
        create_dir_all(path, mode | 0o700).map_err(fail("create the folder", path))?;
        fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700))
            .map_err(fail("set the permissions of", path))?;
        if let Some(parent) = path.parent() {
            sync_dir(parent).map_err(fail("save", parent))?;
        }
        Ok(())
    }

    fn make_link(&self, path: &Path, target: &Path) -> Result<(), EndError> {
        if self.exists(path)? {
            // Made by an earlier run, which may have died before saving it.
            if let Some(parent) = path.parent() {
                sync_dir(parent).map_err(fail("save", parent))?;
            }
            return Ok(());
        }
        std::os::unix::fs::symlink(target, path).map_err(fail("create the link", path))?;
        // The journal soon says this entry is done; its folder must hold it.
        if let Some(parent) = path.parent() {
            sync_dir(parent).map_err(fail("save", parent))?;
        }
        Ok(())
    }

    fn open_write(&self, path: &Path, offset: u64) -> Result<Box<dyn Sink>, EndError> {
        let existed = fs::symlink_metadata(path).is_ok();
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false).mode(0o600);
        let file = match File::open(&options, path) {
            Err(error) if error.kind() == ErrorKind::PermissionDenied && existed => {
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                    .map_err(fail("make writable", path))?;
                File::open(&options, path)
            }
            other => other,
        }
        .map_err(fail("open for writing", path))?;
        let len = file.as_std().metadata().map_err(fail("measure", path))?.len();
        if len < offset {
            return Err(EndError::Io {
                doing: "continue writing",
                path: path.to_owned(),
                source: std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "the file is shorter than the confirmed offset",
                ),
            });
        }
        if len > offset {
            file.set_len(offset).map_err(fail("cut", path))?;
        }
        // Also when the file was there: an earlier run may have made it
        // and died before saving its folder.
        if let Some(parent) = path.parent() {
            sync_dir(parent).map_err(fail("save", parent))?;
        }
        Ok(Box::new(DiskSink {
            file,
            pos: offset,
            path: path.to_owned(),
        }))
    }

    fn hash(&self, path: &Path, len: u64) -> Result<[u8; 32], EndError> {
        let file = fs::File::open(path).map_err(fail("open", path))?;
        let mut reader = file.take(len);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut seen = 0u64;
        loop {
            let n = match reader.read(&mut buf) {
                Ok(n) => n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(fail("read", path)(error)),
            };
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            seen += n as u64;
        }
        if seen < len {
            return Err(EndError::Io {
                doing: "read all of",
                path: path.to_owned(),
                source: std::io::Error::new(ErrorKind::UnexpectedEof, "the file is shorter"),
            });
        }
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        for (o, b) in out.iter_mut().zip(digest.iter()) {
            *o = *b;
        }
        Ok(out)
    }

    fn finish_file(&self, path: &Path, mode: u32, modified_unix_ms: i64) -> Result<(), EndError> {
        let folder = fs::symlink_metadata(path)
            .map_err(fail("look at", path))?
            .is_dir();
        // A folder can't be opened for writing; its time and mode are set
        // through a read-only handle just the same.
        let file = if folder {
            fs::File::open(path)
        } else {
            OpenOptions::new().write(true).open(path)
        }
        .map_err(fail("open", path))?;
        let ms = u64::try_from(modified_unix_ms).unwrap_or(0);
        file.set_modified(UNIX_EPOCH + Duration::from_millis(ms))
            .map_err(fail("set the time of", path))?;
        file.set_permissions(fs::Permissions::from_mode(mode & 0o7777))
            .map_err(fail("set the permissions of", path))?;
        file.sync_all().map_err(fail("save", path))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), EndError> {
        if fs::symlink_metadata(to).is_ok() {
            return Err(EndError::Exists { path: to.to_owned() });
        }
        rename_keeping_mode(from, to).map_err(fail("move", from))
    }

    fn sync_folder(&self, folder: &Path) -> Result<(), EndError> {
        sync_dir(folder).map_err(fail("save", folder))
    }

    fn remove_tree(&self, path: &Path) -> Result<(), EndError> {
        remove_all(path).map_err(fail("remove", path))
    }

    fn trash(&self, path: &Path, description: &str, now_unix_ms: i64) -> Result<(), EndError> {
        self.trash
            .put(
                path,
                NewItem {
                    description: description.to_owned(),
                    caller_data: String::new(),
                    now_unix_ms,
                },
            )
            .map(|_| ())
            .map_err(|source| EndError::Trash {
                path: path.to_owned(),
                source,
            })
    }
}
