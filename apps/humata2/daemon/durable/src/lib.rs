//! File writes that say when they are on disk, so a test can show what a
//! power cut would leave.
//!
//! The store and the holder write their files through [`File`],
//! [`rename`], [`remove_file`], [`create_dir_all`] and [`sync_dir`]. In a
//! normal build these are the `std` calls and nothing more.
//!
//! Built with the `faults` feature (Bazel's testonly `durable_faults`),
//! and with `HUMATA2_POWER_CUT_ROOTS` naming folders, every write under
//! those folders is remembered until it is fsynced: the bytes it replaced
//! and the file's length before, a new folder entry until its folder is
//! fsynced, and a renamed-over or removed file kept aside (hard-linked
//! under `HUMATA2_POWER_CUT_SPARE`, on the same file system) until then.
//! SQLite's files are covered too, by a VFS that sits under the real one
//! ([`install_sqlite`]). [`cut`] puts back everything not yet fsynced, in
//! reverse order, and kills the process with SIGKILL: what is left on disk
//! is what a power cut at that moment would leave if the drive kept
//! nothing it wasn't told to.

#[cfg(feature = "faults")]
mod ledger;
#[cfg(feature = "faults")]
mod sqlite;

use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
#[cfg(not(feature = "faults"))]
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

#[cfg(feature = "faults")]
pub use ledger::revert_unsynced;
#[cfg(feature = "faults")]
pub use sqlite::install as install_sqlite;

/// A file whose writes are remembered until they're synced, when a power
/// cut is being simulated.
#[derive(Debug)]
pub struct File {
    file: fs::File,
    #[cfg(feature = "faults")]
    tracked: Option<ledger::Id>,
}

impl File {
    /// Opens `path` with `options`. A file this makes is a new entry in
    /// its folder, lost to a power cut until the folder is synced.
    pub fn open(options: &OpenOptions, path: &Path) -> io::Result<Self> {
        #[cfg(feature = "faults")]
        {
            ledger::open(options, path).map(|(file, tracked)| Self { file, tracked })
        }
        #[cfg(not(feature = "faults"))]
        {
            options.open(path).map(|file| Self { file })
        }
    }

    /// Writes all of `bytes` at the end of the file.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let at = self.file.metadata()?.len();
        self.write_all_at(bytes, at)
    }

    /// Writes all of `bytes` at `offset`.
    pub fn write_all_at(&self, bytes: &[u8], offset: u64) -> io::Result<()> {
        #[cfg(feature = "faults")]
        if let Some(id) = self.tracked {
            return ledger::write(&self.file, id, bytes, offset);
        }
        self.file.write_all_at(bytes, offset)
    }

    pub fn set_len(&self, len: u64) -> io::Result<()> {
        #[cfg(feature = "faults")]
        if let Some(id) = self.tracked {
            return ledger::set_len(&self.file, id, len);
        }
        self.file.set_len(len)
    }

    /// Puts the file's bytes on disk, as `fdatasync` does.
    pub fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()?;
        #[cfg(feature = "faults")]
        if let Some(id) = self.tracked {
            ledger::synced(id);
        }
        Ok(())
    }

    /// Puts the file's bytes and metadata on disk, as `fsync` does.
    pub fn sync_all(&self) -> io::Result<()> {
        self.file.sync_all()?;
        #[cfg(feature = "faults")]
        if let Some(id) = self.tracked {
            ledger::synced(id);
        }
        Ok(())
    }

    /// Another handle on the same file, whose syncs count for this one.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            #[cfg(feature = "faults")]
            tracked: self.tracked,
        })
    }

    /// The file, to read or ask about. Writes through it aren't tracked.
    pub fn as_std(&self) -> &fs::File {
        &self.file
    }
}

/// Renames `from` to `to`, replacing `to`. Lost to a power cut until the
/// folders are synced.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(feature = "faults")]
    {
        ledger::rename(from, to)
    }
    #[cfg(not(feature = "faults"))]
    {
        fs::rename(from, to)
    }
}

/// Removes the file at `path`. Undone by a power cut until its folder is
/// synced.
pub fn remove_file(path: &Path) -> io::Result<()> {
    #[cfg(feature = "faults")]
    {
        ledger::remove_file(path)
    }
    #[cfg(not(feature = "faults"))]
    {
        fs::remove_file(path)
    }
}

/// Makes `path` and its missing parents with `mode`. Each one made is
/// lost to a power cut until its parent is synced.
pub fn create_dir_all(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(feature = "faults")]
    {
        ledger::create_dir_all(path, mode)
    }
    #[cfg(not(feature = "faults"))]
    {
        fs::DirBuilder::new().recursive(true).mode(mode).create(path)
    }
}

/// Puts the folder's entries on disk: files made, renamed and removed in
/// it.
pub fn sync_dir(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()?;
    #[cfg(feature = "faults")]
    ledger::dir_synced(path);
    Ok(())
}

/// Stops the process here as a power cut would: with a power cut being
/// simulated, everything not yet synced is put back first; then SIGKILL,
/// so nothing is flushed or unwound. `name` says where, if it can't.
#[cfg(feature = "faults")]
pub fn cut(name: &str) -> ! {
    ledger::cut(name)
}
