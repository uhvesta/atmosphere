//! A failed or cut upgrade leaves one whole release running (UP-1, D281 g).
//!
//! The swap in `run.rs` keeps the previous copy of each installed file
//! (`<name>.previous`, a second name for the old file) before it changes
//! anything, then records the upgrade, then renames the staged files in.
//! Each start with that record present decides here, before the store is
//! opened:
//!
//! - the build runs the release the record names and this is its first
//!   start: the store is copied (SAFE's `copy_before_upgrade`) before it
//!   can change schema, and the start goes on;
//! - it runs that release and has already started once without reaching
//!   ready: the upgrade is rolled back;
//! - it runs another release (the swap was cut before the bundle moved, or
//!   an earlier rollback was cut before it ended the record): the upgrade
//!   is abandoned.
//!
//! Undoing is the same either way and every step can be run again after a
//! cut: the store copy goes back first (an older daemon must never meet a
//! newer schema), then the four kept files, the bundle before the daemon
//! last, then the record ends. A daemon that ended up running a file that
//! is no longer the installed one starts the installed one.
//!
//! The previous bundle is checked against the bundle id the record says
//! the upgrade started from, so files left by an older upgrade are never
//! put back as if they were this one's.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use humata2_machines::InstalledFile;
use humata2_store::{Store, StoreError};

use super::build::{Release, host_files};
use super::record::{EndKind, Record, RecordError, UpgradeFolder};
use super::run::hash_file;

/// A start that is the new build's second without reaching ready rolls the
/// upgrade back.
const ROLL_BACK_AT_BOOTS: u32 = 2;
const BUNDLE: &str = "bundle.json";

/// What the start does next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// Open the store and serve as this build.
    Proceed,
    /// Start the installed daemon at this path instead, in this process.
    Restart(PathBuf),
}

/// Why the start couldn't be made safe. Each message says what to do; the
/// cause is its source.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RollbackError {
    #[error(transparent)]
    Record(#[from] RecordError),

    #[error(
        "couldn't keep a copy of the state before upgrade {upgrade_id} changes its schema, so humatad2 didn't start it. Make sure there is disk room and the state folder is writable, then start humatad2 again: it goes back to the previous release if this build keeps failing"
    )]
    Copy {
        upgrade_id: String,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't put back the state copy kept for upgrade {upgrade_id}, so humatad2 didn't start. Don't delete the state folder: the copy is in its upgrade-copies folder. Fix what the cause says and start humatad2 again, and it tries again"
    )]
    Restore {
        upgrade_id: String,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't remove the state copy kept for upgrade {upgrade_id}; it only takes disk room, and the next start tries again"
    )]
    DropCopy {
        upgrade_id: String,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't put the previous {} back in {}, so the upgrade was not undone. Check that your user can write in that folder and start humatad2 again; don't swap the files by hand",
        .previous.display(),
        .installed.display()
    )]
    PutBack {
        previous: PathBuf,
        installed: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't read {} to check it is the release the upgrade started from; make it readable by your user and start humatad2 again",
        .path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the previous bundle.json kept in {} isn't the release (bundle {wanted}) the upgrade started from, so putting it back could install the wrong release and nothing was changed. Install the Humata release you want again from the app; don't copy files into ~/.humata2/bin by hand",
        .path.display()
    )]
    NotTheRelease { path: PathBuf, wanted: String },
}

/// What the start does about an upgrade under way, if there is one. Runs
/// after the start was counted and before the store is opened.
pub(crate) fn at_start(state: &Path, bin: &Path, release: &Release) -> Result<Start, RollbackError> {
    let folder = UpgradeFolder::new(state);
    let store = state.join("store");
    let Some(record) = folder.read()? else {
        drop_finished_copy(&folder, &store)?;
        return Ok(Start::Proceed);
    };
    let is_the_new_build = release.bundle() == record.to_bundle;
    if is_the_new_build && record.boots < ROLL_BACK_AT_BOOTS {
        keep_copy(&store, &record)?;
        return Ok(Start::Proceed);
    }
    let kind = if is_the_new_build {
        EndKind::RolledBack
    } else {
        EndKind::Abandoned
    };
    undo(&folder, &store, bin, release, &record, kind)
}

/// The upgrade settled: its store copy is no longer needed.
pub(crate) fn drop_copy(folder: &UpgradeFolder, upgrade_id: &str) -> Result<(), RollbackError> {
    Store::drop_upgrade_copy(folder.state().join("store"), upgrade_id).map_err(|source| RollbackError::DropCopy {
        upgrade_id: upgrade_id.to_owned(),
        source,
    })
}

/// A settled or undone upgrade may have left its copy if the daemon was cut
/// between ending the record and dropping it.
fn drop_finished_copy(folder: &UpgradeFolder, store: &Path) -> Result<(), RollbackError> {
    let Some(upgrade_id) = folder.last_id()? else {
        return Ok(());
    };
    Store::drop_upgrade_copy(store, &upgrade_id).map_err(|source| RollbackError::DropCopy { upgrade_id, source })
}

/// The first start of the new build: keep the store as it is before it can
/// be migrated. Taking it again after a cut gives the same copy.
fn keep_copy(store: &Path, record: &Record) -> Result<(), RollbackError> {
    Store::copy_before_upgrade(store, &record.upgrade_id).map_err(|source| RollbackError::Copy {
        upgrade_id: record.upgrade_id.clone(),
        source,
    })?;
    crate::fault::point("upgrade.store.copied");
    Ok(())
}

fn undo(
    folder: &UpgradeFolder,
    store: &Path,
    bin: &Path,
    release: &Release,
    record: &Record,
    kind: EndKind,
) -> Result<Start, RollbackError> {
    match Store::restore_upgrade_copy(store, &record.upgrade_id) {
        // Never taken: the store was already at this schema, or never
        // opened by the new build.
        Ok(_) | Err(StoreError::NoUpgradeCopy { .. }) => {}
        Err(source) => {
            return Err(RollbackError::Restore {
                upgrade_id: record.upgrade_id.clone(),
                source,
            });
        }
    }
    crate::fault::point("upgrade.rollback.store");
    put_files_back(bin, &record.from_bundle)?;
    crate::fault::point("upgrade.rollback.files");
    folder.end(kind)?;
    crate::fault::point("upgrade.rollback.ended");
    let installed = bin.join(InstalledFile::Humatad.file_name());
    let own = release.info().own_sha256;
    let differs = !own.is_empty() && installed_daemon(bin, &record.from_bundle).is_some_and(|installed| installed != own);
    Ok(if differs { Start::Restart(installed) } else { Start::Proceed })
}

/// The sha256 of the daemon installed in `bin` once the files went back.
/// The bundle put back names it (it was checked against `from_bundle`
/// first), which spares hashing the whole binary before the daemon listens;
/// with no bundle kept, the file itself is hashed.
fn installed_daemon(bin: &Path, from_bundle: &str) -> Option<String> {
    if !from_bundle.is_empty() {
        let named = std::fs::read(bin.join(BUNDLE))
            .ok()
            .and_then(|bytes| host_files(&bytes).ok().flatten())
            .and_then(|files| {
                files
                    .into_iter()
                    .find(|(file, _)| *file == InstalledFile::Humatad)
                    .map(|(_, sha256)| sha256)
            });
        if named.is_some() {
            return named;
        }
    }
    hash_file(&bin.join(InstalledFile::Humatad.file_name())).ok()
}

/// Puts each kept file back over the installed one: the holder and the CLI,
/// then `bundle.json`, then the daemon. A file with nothing kept (a first
/// install) is left alone. `from_bundle` is the release the kept bundle
/// must be; empty when there was none.
fn put_files_back(bin: &Path, from_bundle: &str) -> Result<(), RollbackError> {
    if !from_bundle.is_empty() {
        let kept = bin.join(kept_name(BUNDLE));
        match hash_file(&kept) {
            Ok(found) if found.eq_ignore_ascii_case(from_bundle) => {}
            Ok(_) => {
                return Err(RollbackError::NotTheRelease {
                    path: kept,
                    wanted: from_bundle.to_owned(),
                });
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Err(RollbackError::NotTheRelease {
                    path: kept,
                    wanted: from_bundle.to_owned(),
                });
            }
            Err(source) => return Err(RollbackError::Read { path: kept, source }),
        }
    }
    for name in files_in_put_back_order() {
        put_back(bin, name)?;
    }
    humata2_durable::sync_dir(bin).map_err(|source| RollbackError::PutBack {
        previous: bin.to_owned(),
        installed: bin.to_owned(),
        source,
    })
}

/// The files an upgrade replaces, and keeps a previous copy of.
pub(super) fn kept_files() -> Vec<&'static str> {
    InstalledFile::ALL
        .into_iter()
        .map(InstalledFile::file_name)
        .chain([BUNDLE])
        .collect()
}

/// Where the previous copy of installed file `name` is kept.
pub(super) fn kept_name(name: &str) -> String {
    format!("{name}.previous")
}

fn files_in_put_back_order() -> Vec<&'static str> {
    let daemon = InstalledFile::Humatad.file_name();
    let mut names: Vec<&'static str> = kept_files().into_iter().filter(|name| *name != daemon).collect();
    names.push(daemon);
    names
}

/// Makes `bin/<name>` the kept file again, keeping the kept one so this can
/// run again after a cut. A second name is made first and renamed over the
/// installed file, so a cut leaves one or the other whole.
fn put_back(bin: &Path, name: &str) -> Result<(), RollbackError> {
    let (previous, installed) = (bin.join(kept_name(name)), bin.join(name));
    let staging = bin.join(format!(".{name}.rollback"));
    let failed = |source| RollbackError::PutBack {
        previous: previous.clone(),
        installed: installed.clone(),
        source,
    };
    match std::fs::remove_file(&staging) {
        Ok(()) => {}
        Err(source) if source.kind() == ErrorKind::NotFound => {}
        Err(source) => return Err(failed(source)),
    }
    match std::fs::hard_link(&previous, &staging) {
        Ok(()) => {}
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(failed(source)),
    }
    humata2_durable::rename(&staging, &installed).map_err(failed)?;
    // Renaming a second name over the same file (a run again after a cut)
    // changes nothing and keeps both names: the second one goes.
    match std::fs::remove_file(&staging) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(failed(source)),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use humata2_machines::Platform;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::upgrade::run::hex;

    const OLD: [(&str, &[u8]); 4] = [
        ("humatad2", b"old daemon"),
        ("humata2_holder", b"old holder"),
        ("humata", b"old cli"),
        ("stack", b"old stack"),
    ];
    const NEW: [(&str, &[u8]); 4] = [
        ("humatad2", b"new daemon"),
        ("humata2_holder", b"new holder"),
        ("humata", b"new cli"),
        ("stack", b"new stack"),
    ];

    fn manifest(files: &[(&str, &[u8])]) -> Vec<u8> {
        let entries: serde_json::Map<String, serde_json::Value> = files
            .iter()
            .map(|(name, bytes)| {
                (
                    (*name).to_owned(),
                    serde_json::json!({"path": name, "sha256": hex(Sha256::digest(bytes).as_slice())}),
                )
            })
            .collect();
        let platform = Platform::host().expect("a platform Humata ships").bundle_name();
        serde_json::to_vec(&serde_json::json!({"platforms": {platform: entries}})).expect("serializes")
    }

    fn id_of(bytes: &[u8]) -> String {
        hex(Sha256::digest(bytes).as_slice())
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("a file is written");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("made runnable");
    }

    /// A machine in the middle of an upgrade from OLD to NEW: the new files
    /// installed, the old ones kept as `.previous`, the record written with
    /// `boots` starts counted, and the store having been at an older schema.
    struct Machine {
        dir: tempfile::TempDir,
    }

    impl Machine {
        fn swapped(boots: u32) -> Self {
            let dir = tempfile::tempdir().expect("a temp folder");
            let bin = dir.path().join("bin");
            std::fs::create_dir_all(&bin).expect("bin");
            for (name, bytes) in OLD {
                write(&bin.join(kept_name(name)), bytes);
            }
            write(&bin.join(kept_name(BUNDLE)), &manifest(&OLD));
            for (name, bytes) in NEW {
                write(&bin.join(name), bytes);
            }
            write(&bin.join(BUNDLE), &manifest(&NEW));
            let machine = Self { dir };
            let folder = machine.folder();
            folder
                .write_started(&Record {
                    upgrade_id: "up_1".to_owned(),
                    from_version: "2.0.4".to_owned(),
                    from_bundle: id_of(&manifest(&OLD)),
                    to_bundle: id_of(&manifest(&NEW)),
                    started_unix_ms: 1,
                    boots: 0,
                })
                .expect("the record is written");
            for _ in 0..boots {
                folder.count_boot().expect("counts");
            }
            machine
        }

        fn state(&self) -> PathBuf {
            self.dir.path().join("state")
        }

        fn bin(&self) -> PathBuf {
            self.dir.path().join("bin")
        }

        fn folder(&self) -> UpgradeFolder {
            UpgradeFolder::new(&self.state())
        }

        fn store(&self) -> PathBuf {
            self.state().join("store")
        }

        /// A daemon running the new release, for the bundle id and file
        /// the new files name.
        fn new_release(&self) -> Release {
            Release::read(&self.bin(), &self.bin().join("humatad2"), "2.0.5", 3)
        }

        fn read(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.bin().join(name)).unwrap_or_default()
        }

        fn has_old_release(&self) -> bool {
            OLD.iter().all(|(name, bytes)| self.read(name) == *bytes) && self.read(BUNDLE) == manifest(&OLD)
        }

        /// A store at an older schema holding `marker` in its user table.
        fn old_store(&self, marker: &str) {
            std::fs::create_dir_all(self.store()).expect("store folder");
            let connection = rusqlite::Connection::open(self.store().join("state.sqlite")).expect("opens");
            let older = humata2_store::SCHEMA_VERSION - 1;
            connection
                .execute_batch(&format!(
                    "CREATE TABLE IF NOT EXISTS marker (value TEXT); DELETE FROM marker; INSERT INTO marker VALUES ('{marker}'); PRAGMA user_version = {older};"
                ))
                .expect("writes");
        }

        fn marker(&self) -> Option<String> {
            let connection = rusqlite::Connection::open(self.store().join("state.sqlite")).ok()?;
            connection.query_row("SELECT value FROM marker", [], |row| row.get(0)).ok()
        }
    }

    #[test]
    fn the_first_start_of_the_new_build_keeps_a_copy_of_the_store_and_goes_on() {
        let machine = Machine::swapped(1);
        machine.old_store("before");

        let start = at_start(&machine.state(), &machine.bin(), &machine.new_release()).expect("starts");

        assert_eq!(start, Start::Proceed);
        assert!(machine.store().join("upgrade-copies/up_1.sqlite").exists(), "the copy is kept");
        assert!(machine.folder().read().expect("reads").is_some(), "the upgrade is still under way");
        assert_eq!(machine.read("humatad2"), b"new daemon");
    }

    #[test]
    fn a_second_start_without_ready_puts_back_the_store_the_files_and_ends_the_record() {
        let machine = Machine::swapped(1);
        machine.old_store("before");
        at_start(&machine.state(), &machine.bin(), &machine.new_release()).expect("the first start");
        // The new build migrated the store, then never got ready.
        machine.old_store("migrated");
        machine.folder().count_boot().expect("the second start is counted");

        let start = at_start(&machine.state(), &machine.bin(), &machine.new_release()).expect("rolls back");

        assert_eq!(start, Start::Restart(machine.bin().join("humatad2")));
        assert!(machine.has_old_release(), "the old daemon, holder, cli and bundle are back");
        assert_eq!(machine.marker().as_deref(), Some("before"), "the store is as it was");
        assert_eq!(machine.folder().read().expect("reads"), None);
        let told = machine.folder().unreported_end().expect("reads");
        assert_eq!(told.map(|(_, kind)| kind), Some(EndKind::RolledBack));
    }

    #[test]
    fn a_rollback_run_again_after_a_cut_at_any_step_finishes_with_the_same_result() {
        for already in 0..3 {
            let machine = Machine::swapped(2);
            machine.old_store("before");
            Store::copy_before_upgrade(machine.store(), "up_1").expect("copied");
            machine.old_store("migrated");
            // What a cut after step `already` left behind.
            if already >= 1 {
                Store::restore_upgrade_copy(machine.store(), "up_1").expect("restored");
            }
            if already >= 2 {
                put_files_back(&machine.bin(), &id_of(&manifest(&OLD))).expect("put back");
            }

            let start = at_start(&machine.state(), &machine.bin(), &machine.new_release());

            // After the files went back the running binary is no longer
            // the one installed, which the release of the new files can't
            // tell; either way the result on disk is the same.
            assert!(start.is_ok(), "cut after step {already}: {start:?}");
            assert!(machine.has_old_release(), "cut after step {already}");
            assert_eq!(machine.marker().as_deref(), Some("before"), "cut after step {already}");
            assert_eq!(machine.folder().read().expect("reads"), None, "cut after step {already}");
        }
    }

    #[test]
    fn a_swap_cut_before_the_bundle_moved_is_abandoned_with_the_old_release_put_whole() {
        let machine = Machine::swapped(1);
        // The new daemon was swapped in but the bundle still names the old
        // release: the running build names no release.
        write(&machine.bin().join("bundle.json"), &manifest(&OLD));
        let release = Release::read(&machine.bin(), &machine.bin().join("humatad2"), "2.0.5", 3);
        assert_eq!(release.bundle(), "");

        let start = at_start(&machine.state(), &machine.bin(), &release).expect("starts");

        assert_eq!(start, Start::Restart(machine.bin().join("humatad2")));
        assert!(machine.has_old_release());
        assert_eq!(machine.folder().read().expect("reads"), None);
        let told = machine.folder().unreported_end().expect("reads");
        assert_eq!(told.map(|(record, kind)| (record.upgrade_id, kind)), Some(("up_1".to_owned(), EndKind::Abandoned)));
    }

    #[test]
    fn an_upgrade_cut_before_anything_was_swapped_is_abandoned_and_the_running_daemon_stays() {
        let machine = Machine::swapped(0);
        // Nothing was swapped: the old files are installed, and the kept
        // ones are the same files.
        for (name, bytes) in OLD {
            write(&machine.bin().join(name), bytes);
        }
        write(&machine.bin().join(BUNDLE), &manifest(&OLD));
        machine.folder().count_boot().expect("counts");
        let release = Release::read(&machine.bin(), &machine.bin().join("humatad2"), "2.0.4", 3);

        let start = at_start(&machine.state(), &machine.bin(), &release).expect("starts");

        assert_eq!(start, Start::Proceed);
        assert!(machine.has_old_release());
        assert_eq!(machine.folder().read().expect("reads"), None);
    }

    #[test]
    fn putting_the_files_back_twice_leaves_nothing_but_the_release() {
        let machine = Machine::swapped(2);
        let from = id_of(&manifest(&OLD));

        put_files_back(&machine.bin(), &from).expect("put back");
        put_files_back(&machine.bin(), &from).expect("put back again, as after a cut");

        assert!(machine.has_old_release());
        let left: Vec<String> = std::fs::read_dir(machine.bin())
            .expect("bin reads")
            .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".rollback"))
            .collect();
        assert_eq!(left, Vec::<String>::new());
    }

    #[test]
    fn files_kept_by_another_upgrade_are_never_put_back() {
        let machine = Machine::swapped(2);
        write(&machine.bin().join(kept_name(BUNDLE)), &manifest(&[("humatad2", b"older still")]));

        let refused = at_start(&machine.state(), &machine.bin(), &machine.new_release());

        assert!(matches!(refused, Err(RollbackError::NotTheRelease { .. })), "{refused:?}");
        assert_eq!(machine.read("humatad2"), b"new daemon", "nothing was changed");
        assert!(machine.folder().read().expect("reads").is_some());
    }

    #[test]
    fn a_settled_upgrade_s_leftover_copy_is_dropped_at_the_next_start_and_no_record_means_proceed() {
        let machine = Machine::swapped(1);
        machine.old_store("before");
        at_start(&machine.state(), &machine.bin(), &machine.new_release()).expect("the first start");
        machine.folder().settle(&machine.new_release()).expect("settles");
        assert!(machine.store().join("upgrade-copies/up_1.sqlite").exists());

        let start = at_start(&machine.state(), &machine.bin(), &machine.new_release()).expect("starts");

        assert_eq!(start, Start::Proceed);
        assert!(!machine.store().join("upgrade-copies/up_1.sqlite").exists());
    }

    #[test]
    fn a_machine_that_never_upgraded_just_starts() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let release = Release::read(dir.path(), &dir.path().join("humatad2"), "2.0.4", 3);

        let start = at_start(&dir.path().join("state"), dir.path(), &release).expect("starts");

        assert_eq!(start, Start::Proceed);
    }
}
