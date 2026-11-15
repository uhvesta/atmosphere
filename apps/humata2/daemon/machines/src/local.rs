//! Installing the host's own tools (TS-0): at every launch the desktop app
//! puts its bundled `humatad2`, `humata2_holder` and `humata` into
//! `~/.humata2/bin` by the same rules as a machine (TS-1), without SSH. A
//! file whose SHA-256 already matches is left alone; a new copy is written
//! beside the old one, flushed, checked and renamed over it, so a kill or a
//! power cut at any point leaves the old copy or the new one, never half of
//! either (PER-27). While a holder runs, the holder and `humata` wait as
//! `.<name>.staged`; the next call after no holder runs moves them in
//! without writing them again.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::install::{self, InstalledFile, Payload};

/// Whether a `humata2_holder` of this user runs.
pub trait Holders {
    fn running(&self) -> Result<bool, LocalInstallError>;
}

/// Asks `pgrep` for this user's processes named `humata2_holder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunningHolders;

impl Holders for RunningHolders {
    fn running(&self) -> Result<bool, LocalInstallError> {
        let uid = rustix::process::getuid().as_raw().to_string();
        let status = Command::new("/usr/bin/pgrep")
            .args(["-u", &uid, "-x", "humata2_holder"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|source| LocalInstallError::Holders { source })?;
        // 0: found; 1: none. Anything else is pgrep failing, and a file is
        // never swapped on a guess.
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(LocalInstallError::HoldersUnknown {
                status: status.to_string(),
            }),
        }
    }
}

/// What a local install did to each file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalReport {
    /// Already the bundle's copy.
    pub unchanged: Vec<InstalledFile>,
    /// The new copy is in place.
    pub swapped: Vec<InstalledFile>,
    /// The new copy waits as `.<name>.staged` while a holder runs.
    pub staged: Vec<InstalledFile>,
}

#[derive(Debug, Error)]
pub enum LocalInstallError {
    #[error(
        "couldn't create {path}: {source}. Check that your home folder is writable and has free space, then open Humata again"
    )]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't read {path}: {source}. Check its permissions (it should be yours), or remove it and open Humata again"
    )]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't write {file} to {path}: {source}. Free some disk space or fix the folder's permissions, then open Humata again; the copy already there was left as it was"
    )]
    Write {
        file: InstalledFile,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "the {file} written to {path} reads back with SHA-256 {found}, not {expected}, so it was removed and the old copy kept. The disk may be failing; check it (Disk Utility › First Aid) before opening Humata again"
    )]
    Verify {
        file: InstalledFile,
        path: PathBuf,
        expected: String,
        found: String,
    },
    #[error(
        "couldn't move the new {file} into {path}: {source}. Fix the folder's permissions, then open Humata again; the old copy is still in place"
    )]
    Rename {
        file: InstalledFile,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't run pgrep to see whether a terminal holder is running ({source}), so nothing a holder uses was replaced. Check that /usr/bin/pgrep exists, then open Humata again"
    )]
    Holders {
        #[source]
        source: io::Error,
    },
    #[error(
        "pgrep failed ({status}) while checking for running terminal holders, so nothing a holder uses was replaced; open Humata again"
    )]
    HoldersUnknown { status: String },
}

/// Puts `payloads` into `bin_dir` (`~/.humata2/bin`), each by the TS-1
/// rules: unchanged when its SHA-256 matches, moved in from `.staged` when
/// that matches and no holder runs, otherwise written beside and swapped in,
/// or staged while a holder runs. Leftovers of a killed earlier run are
/// removed first. Safe to call at every launch and again whenever holders
/// end.
pub fn install_local(
    bin_dir: &Path,
    payloads: &[Payload],
    holders: &dyn Holders,
) -> Result<LocalReport, LocalInstallError> {
    create_private_dir(bin_dir)?;
    remove_leftovers(bin_dir)?;
    let mut report = LocalReport::default();
    // Asked once, and only when something a holder uses would change.
    let mut running: Option<bool> = None;
    let mut holders_running = || -> Result<bool, LocalInstallError> {
        if let Some(running) = running {
            return Ok(running);
        }
        let answer = holders.running()?;
        running = Some(answer);
        Ok(answer)
    };
    for payload in payloads {
        let file = payload.file();
        let target = bin_dir.join(file.file_name());
        let staged = bin_dir.join(file.staged_name());
        if sha_of(&target)?.as_deref() == Some(payload.sha256()) {
            // A stale staged copy of something else has nothing to wait for.
            remove_if_there(&staged)?;
            report.unchanged.push(file);
            continue;
        }
        let waits = file.waits_for_holders() && holders_running()?;
        if sha_of(&staged)?.as_deref() == Some(payload.sha256()) {
            if waits {
                report.staged.push(file);
            } else {
                rename(file, &staged, &target, bin_dir)?;
                report.swapped.push(file);
            }
            continue;
        }
        let temp = bin_dir.join(format!(".{}.{}.new", file.file_name(), install::nonce()));
        write_verified(payload, &temp)?;
        if waits {
            rename(file, &temp, &staged, bin_dir)?;
            report.staged.push(file);
        } else {
            rename(file, &temp, &target, bin_dir)?;
            remove_if_there(&staged)?;
            report.swapped.push(file);
        }
    }
    Ok(report)
}

fn create_private_dir(dir: &Path) -> Result<(), LocalInstallError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|source| LocalInstallError::CreateDir {
            path: dir.to_owned(),
            source,
        })
}

/// Removes the temporary files of an install that was killed.
fn remove_leftovers(dir: &Path) -> Result<(), LocalInstallError> {
    let entries = fs::read_dir(dir).map_err(|source| LocalInstallError::Read {
        path: dir.to_owned(),
        source,
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ours = InstalledFile::ALL
            .iter()
            .any(|file| name.starts_with(&format!(".{}.", file.file_name())) && name.ends_with(".new"));
        if ours {
            remove_if_there(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_if_there(path: &Path) -> Result<(), LocalInstallError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LocalInstallError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// The SHA-256 of the file at `path`; none when there is none.
fn sha_of(path: &Path) -> Result<Option<String>, LocalInstallError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(LocalInstallError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = file.read(&mut buffer).map_err(|source| LocalInstallError::Read {
            path: path.to_owned(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Some(install::hex(&hasher.finalize())))
}

/// Writes `payload` to `temp` (0700), flushes it to disk and checks what
/// reads back; removes it on any failure.
fn write_verified(payload: &Payload, temp: &Path) -> Result<(), LocalInstallError> {
    let file = payload.file();
    let write = || -> io::Result<()> {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(temp)?;
        out.write_all(payload.bytes())?;
        out.sync_all()
    };
    if let Err(source) = write() {
        let _ = fs::remove_file(temp);
        return Err(LocalInstallError::Write {
            file,
            path: temp.to_owned(),
            source,
        });
    }
    let found = sha_of(temp)?;
    if found.as_deref() != Some(payload.sha256()) {
        let _ = fs::remove_file(temp);
        return Err(LocalInstallError::Verify {
            file,
            path: temp.to_owned(),
            expected: payload.sha256().to_owned(),
            found: found.unwrap_or_default(),
        });
    }
    Ok(())
}

/// Renames `from` over `to` and flushes the folder, so the rename itself
/// survives a power cut.
fn rename(file: InstalledFile, from: &Path, to: &Path, dir: &Path) -> Result<(), LocalInstallError> {
    let fail = |source| LocalInstallError::Rename {
        file,
        path: to.to_owned(),
        source,
    };
    fs::rename(from, to).map_err(fail)?;
    File::open(dir).and_then(|dir| dir.sync_all()).map_err(fail)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use super::*;

    struct FakeHolders(Cell<bool>);

    impl Holders for FakeHolders {
        fn running(&self) -> Result<bool, LocalInstallError> {
            Ok(self.0.get())
        }
    }

    fn build(tag: &str) -> Vec<Payload> {
        InstalledFile::ALL
            .into_iter()
            .map(|file| Payload::new(file, Arc::from(format!("{file} {tag}").as_bytes())))
            .collect()
    }

    fn read(dir: &Path, name: &str) -> Option<Vec<u8>> {
        fs::read(dir.join(name)).ok()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_first_launch_installs_all_three_private_and_executable() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".humata2/bin");

        let report = install_local(&bin, &build("v1"), &FakeHolders(Cell::new(false))).unwrap();

        assert_eq!(report.swapped, InstalledFile::ALL);
        for file in InstalledFile::ALL {
            assert_eq!(read(&bin, file.file_name()), Some(format!("{file} v1").into_bytes()));
            let mode = fs::metadata(bin.join(file.file_name())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert_eq!(fs::metadata(&bin).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn a_launch_with_the_same_build_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let holders = FakeHolders(Cell::new(false));
        install_local(&bin, &build("v1"), &holders).unwrap();
        holders.0.set(true);

        let report = install_local(&bin, &build("v1"), &holders).unwrap();

        assert_eq!(report.unchanged, InstalledFile::ALL);
        assert!(report.swapped.is_empty() && report.staged.is_empty());
    }

    #[test]
    fn a_new_build_waits_for_holders_then_goes_in() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let holders = FakeHolders(Cell::new(false));
        install_local(&bin, &build("v1"), &holders).unwrap();

        holders.0.set(true);
        let report = install_local(&bin, &build("v2"), &holders).unwrap();
        assert_eq!(report.swapped, [InstalledFile::Humatad]);
        assert_eq!(report.staged, [InstalledFile::Holder, InstalledFile::Cli]);
        assert_eq!(read(&bin, "humatad2"), Some(b"humatad2 v2".to_vec()));
        assert_eq!(read(&bin, "humata2_holder"), Some(b"humata2_holder v1".to_vec()));
        assert_eq!(read(&bin, "humata"), Some(b"humata v1".to_vec()));

        holders.0.set(false);
        let report = install_local(&bin, &build("v2"), &holders).unwrap();
        assert_eq!(report.swapped, [InstalledFile::Holder, InstalledFile::Cli]);
        assert_eq!(read(&bin, "humata2_holder"), Some(b"humata2_holder v2".to_vec()));
        assert_eq!(read(&bin, "humata"), Some(b"humata v2".to_vec()));
        assert_eq!(names(&bin), ["humata", "humata2_holder", "humatad2"]);
    }

    #[test]
    fn leftovers_of_a_killed_install_are_cleaned_and_it_finishes() {
        // A kill between writing the temporary file and renaming it, and a
        // kill after staging but before the holders ended.
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let holders = FakeHolders(Cell::new(false));
        install_local(&bin, &build("v1"), &holders).unwrap();
        fs::write(bin.join(".humatad2.0123456789abcdef.new"), b"humatad2 v").unwrap();
        fs::write(bin.join(".humata.staged"), b"humata v2").unwrap();

        let report = install_local(&bin, &build("v2"), &holders).unwrap();

        assert_eq!(report.swapped, InstalledFile::ALL);
        for file in InstalledFile::ALL {
            assert_eq!(read(&bin, file.file_name()), Some(format!("{file} v2").into_bytes()));
        }
        assert_eq!(names(&bin), ["humata", "humata2_holder", "humatad2"]);
    }

    #[test]
    fn a_damaged_staged_copy_is_written_again() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(".humata2_holder.staged"), b"half of it").unwrap();

        install_local(&bin, &build("v1"), &FakeHolders(Cell::new(false))).unwrap();

        assert_eq!(read(&bin, "humata2_holder"), Some(b"humata2_holder v1".to_vec()));
        assert_eq!(names(&bin), ["humata", "humata2_holder", "humatad2"]);
    }
}
