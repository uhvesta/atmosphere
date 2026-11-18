//! What release this daemon is (UP-1, D281 a). `Release::read` runs once at
//! boot: it hashes the running binary and reads the bundle manifest the app
//! installs in `bin`, beside the state folder. The bundle id is the sha256
//! of that manifest's bytes, and it is named only when the running binary is
//! this platform's `humatad2` entry there. Nothing here fails: a problem
//! gives an empty bundle id and one log line that says what to fix.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_grpc::humata::v1 as pb;
use humata2_machines::{InstalledFile, Platform};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The metadata a CLI names its release in.
pub(crate) const BUNDLE_HEADER: &str = "humata-bundle";

/// The bundle manifest's name in `bin`, as the app installs it.
const MANIFEST: &str = "bundle.json";

/// How much of the running binary is read at a time while it is hashed.
const CHUNK_BYTES: usize = 64 << 10;

/// This daemon's release: what its ServerInfo and BeginRelay report.
#[derive(Clone)]
pub(crate) struct Release {
    info: pb::BuildInfo,
    /// The sha256 of the CLI and the holder in `bin` (D371). Every clone of
    /// this release shares it, so what an upgrade or the bin watcher finds
    /// is what the next Hello and ServerInfo report.
    installed: Arc<Mutex<Vec<pb::BuildFile>>>,
}

impl Release {
    /// The release of the binary at `own_exe`, which runs as `version` with
    /// store schema `store_schema`, and whose bundle manifest is
    /// `bin_dir/bundle.json`. Never fails: see the module docs.
    pub(crate) fn read(bin_dir: &Path, own_exe: &Path, version: &str, store_schema: u32) -> Self {
        let installed = Arc::new(Mutex::new(super::tools::installed(bin_dir)));
        let own_sha256 = match hash_file(own_exe) {
            Ok(sha256) => sha256,
            Err(error) => {
                report(&error);
                return Self::of(version, store_schema, String::new(), None, installed);
            }
        };
        let matched = match matching_bundle(bin_dir, &own_sha256) {
            Ok(matched) => matched,
            Err(error) => {
                report(&error);
                None
            }
        };
        Self::of(version, store_schema, own_sha256, matched, installed)
    }

    /// The same binary's release as `bin_dir/bundle.json` names it now. A
    /// rolled back or abandoned upgrade puts the previous bundle back after
    /// `read` ran at boot; the binary isn't hashed again.
    pub(crate) fn reread(&self, bin_dir: &Path) -> Self {
        let own_sha256 = self.info.own_sha256.clone();
        if own_sha256.is_empty() {
            return self.clone();
        }
        let matched = match matching_bundle(bin_dir, &own_sha256) {
            Ok(matched) => matched,
            Err(error) => {
                report(&error);
                None
            }
        };
        Self::of(&self.info.version, self.info.store_schema, own_sha256, matched, self.installed.clone())
    }

    /// Looks at the CLI and the holder in `bin_dir` again: what the next
    /// report of this release says is installed.
    pub(crate) fn rescan_installed(&self, bin_dir: &Path) {
        let now = super::tools::installed(bin_dir);
        *self.installed.lock().unwrap_or_else(PoisonError::into_inner) = now;
    }

    /// The bundle id this release is named by; "" when it names none.
    pub(crate) fn bundle(&self) -> &str {
        &self.info.bundle
    }

    /// What this release reports as its build.
    pub(crate) fn info(&self) -> pb::BuildInfo {
        pb::BuildInfo {
            installed: self.installed.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            ..self.info.clone()
        }
    }

    fn of(
        version: &str,
        store_schema: u32,
        own_sha256: String,
        matched: Option<Matched>,
        installed: Arc<Mutex<Vec<pb::BuildFile>>>,
    ) -> Self {
        let (bundle, files) = match matched {
            Some(matched) => (matched.bundle, matched.files),
            None => (String::new(), Vec::new()),
        };
        Self {
            info: pb::BuildInfo {
                version: version.to_owned(),
                bundle,
                own_sha256,
                store_schema,
                files,
                installed: Vec::new(),
            },
            installed,
        }
    }
}

/// The first seven digits of a bundle id, as a version line shows it. The
/// whole id when it is shorter.
pub(crate) fn short(bundle: &str) -> &str {
    bundle.get(..7).unwrap_or(bundle)
}

/// The files `bundle_json` names for this platform, each with its sha256 in
/// lowercase hex. `None` when Humata ships nothing for this platform or the
/// bundle has no entry for it. Used to check a staged release before it is
/// swapped in.
pub(super) fn host_files(
    bundle_json: &[u8],
) -> Result<Option<Vec<(InstalledFile, String)>>, serde_json::Error> {
    let Some(platform) = Platform::host() else {
        return Ok(None);
    };
    let manifest: Manifest = serde_json::from_slice(bundle_json)?;
    Ok(manifest.platforms.get(platform.bundle_name()).map(|entries| {
        InstalledFile::ALL
            .into_iter()
            .filter_map(|file| {
                entries
                    .get(file.file_name())
                    .map(|entry| (file, entry.sha256.to_ascii_lowercase()))
            })
            .collect()
    }))
}

/// Whether one platform's entry in `bundle_json` names exactly `files`,
/// each by its lowercase hex sha256. A release staged over SSH carries the
/// files of the platform the machine's `uname` named, which can be another
/// than the one this daemon was built for (one running translated, or a
/// test's fake machine); those are still that release, whole.
pub(super) fn names_whole_entry(
    bundle_json: &[u8],
    files: &[(InstalledFile, String)],
) -> Result<bool, serde_json::Error> {
    let manifest: Manifest = serde_json::from_slice(bundle_json)?;
    Ok(manifest.platforms.values().any(|entries| {
        InstalledFile::ALL.into_iter().all(|file| {
            let named = entries.get(file.file_name());
            let staged = files.iter().find(|(candidate, _)| *candidate == file);
            matches!((named, staged), (Some(entry), Some((_, sha256))) if entry.sha256.eq_ignore_ascii_case(sha256))
        })
    }))
}

/// Whether some platform's entry in `bundle_json` names `file` by the
/// lowercase hex sha256 `sha256`. A tool staged over SSH is the file of the
/// platform the machine's `uname` named (see [`names_whole_entry`]).
pub(super) fn names_file(
    bundle_json: &[u8],
    file: InstalledFile,
    sha256: &str,
) -> Result<bool, serde_json::Error> {
    let manifest: Manifest = serde_json::from_slice(bundle_json)?;
    Ok(manifest.platforms.values().any(|entries| {
        entries
            .get(file.file_name())
            .is_some_and(|entry| entry.sha256.eq_ignore_ascii_case(sha256))
    }))
}

/// What the bundle names for this binary: its id, and this platform's files.
struct Matched {
    bundle: String,
    files: Vec<pb::BuildFile>,
}

/// Why the release couldn't be read. Each message says what to do; the
/// cause is its source.
#[derive(Debug, thiserror::Error)]
enum ReadError {
    #[error(
        "this daemon couldn't read its own binary at {}, so it reports no bundle and posts no Migrate items; start humatad2 from the file the app installed",
        .path.display()
    )]
    Own {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't read the bundle manifest at {}, so this daemon reports no bundle and posts no Migrate items; make the file readable by your user, or install Humata again to put it back",
        .path.display()
    )]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the bundle manifest at {} isn't in the bundle format, so this daemon reports no bundle and posts no Migrate items; install Humata again so the app writes it back",
        .path.display()
    )]
    Invalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// The manifest's shape: the platforms, each naming its files by sha256.
#[derive(Deserialize)]
struct Manifest {
    platforms: BTreeMap<String, BTreeMap<String, Entry>>,
}

#[derive(Deserialize)]
struct Entry {
    sha256: String,
}

fn report(error: &ReadError) {
    crate::log::say!("humatad2: {}", crate::status::chain(error));
}

fn own_error(path: &Path, source: std::io::Error) -> ReadError {
    ReadError::Own {
        path: path.to_owned(),
        source,
    }
}

/// The lowercase hex sha256 of the file at `path`, read in chunks.
fn hash_file(path: &Path) -> Result<String, ReadError> {
    let mut file = File::open(path).map_err(|source| own_error(path, source))?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0; CHUNK_BYTES];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => hasher.update(&chunk[..read]),
            Err(source) if source.kind() == ErrorKind::Interrupted => {}
            Err(source) => return Err(own_error(path, source)),
        }
    }
    Ok(hex(hasher.finalize().as_slice()))
}

/// What `bin_dir`'s bundle names for the binary whose sha256 is
/// `own_sha256`: the bundle's id and this platform's files, when the binary
/// is this platform's `humatad2` there. None when there is no bundle, no
/// entry for this platform, or the entry names another binary.
fn matching_bundle(bin_dir: &Path, own_sha256: &str) -> Result<Option<Matched>, ReadError> {
    let Some(platform) = Platform::host() else {
        return Ok(None);
    };
    let path = bin_dir.join(MANIFEST);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ReadError::Unreadable { path, source }),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|source| ReadError::Invalid {
        path: path.clone(),
        source,
    })?;
    let Some(entries) = manifest.platforms.get(platform.bundle_name()) else {
        return Ok(None);
    };
    let names_this_binary = entries
        .get(InstalledFile::Humatad.file_name())
        .is_some_and(|entry| entry.sha256.eq_ignore_ascii_case(own_sha256));
    if !names_this_binary {
        return Ok(None);
    }
    let files = InstalledFile::ALL
        .into_iter()
        .filter_map(|file| {
            entries.get(file.file_name()).map(|entry| pb::BuildFile {
                name: file.file_name().to_owned(),
                sha256: entry.sha256.to_ascii_lowercase(),
            })
        })
        .collect();
    Ok(Some(Matched {
        bundle: hex(Sha256::digest(&bytes).as_slice()),
        files,
    }))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAEMON: &[u8] = b"this daemon's binary";

    fn sha(bytes: &[u8]) -> String {
        hex(Sha256::digest(bytes).as_slice())
    }

    fn host() -> &'static str {
        Platform::host()
            .expect("the tests run on a platform Humata ships")
            .bundle_name()
    }

    /// A bundle manifest naming `daemon_sha` as `platform`'s humatad2.
    /// `release` makes two manifests differ without changing the entries.
    fn manifest(platform: &str, daemon_sha: &str, release: &str) -> Vec<u8> {
        let entry = serde_json::json!({
            "humatad2": {"path": "humatad2", "sha256": daemon_sha},
            "humata2_holder": {"path": "humata2_holder", "sha256": sha(b"holder")},
            "humata": {"path": "humata", "sha256": sha(b"cli")},
            "stack": {"path": "stack", "sha256": sha(b"stack")},
        });
        let mut platforms = serde_json::Map::new();
        platforms.insert(platform.to_owned(), entry);
        serde_json::json!({"release": release, "platforms": platforms})
            .to_string()
            .into_bytes()
    }

    /// A folder with the daemon binary in it, and `bundle` as its
    /// `bin/bundle.json` when given. Returns the folder, the bin folder and
    /// the binary's path.
    fn install(bundle: Option<&[u8]>) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let own_exe = dir.path().join("humatad2");
        std::fs::write(&own_exe, DAEMON).expect("the binary is written");
        let bin = dir.path().join("bin");
        if let Some(bytes) = bundle {
            std::fs::create_dir_all(&bin).expect("the bin folder is made");
            std::fs::write(bin.join(MANIFEST), bytes).expect("the manifest is written");
        }
        (dir, bin, own_exe)
    }

    fn files(release: &Release) -> Vec<(String, String)> {
        release
            .info()
            .files
            .into_iter()
            .map(|file| (file.name, file.sha256))
            .collect()
    }

    #[test]
    fn a_binary_the_bundle_names_as_its_platforms_daemon_names_the_bundle() {
        let bytes = manifest(host(), &sha(DAEMON), "a");
        let (_dir, bin, own_exe) = install(Some(&bytes));

        let release = Release::read(&bin, &own_exe, "2.0.5", 7);
        let info = release.info();

        assert_eq!(release.bundle(), sha(&bytes));
        assert_eq!(info.own_sha256, sha(DAEMON));
        assert_eq!(info.version, "2.0.5");
        assert_eq!(info.store_schema, 7);
        assert_eq!(
            files(&release),
            vec![
                ("humatad2".to_owned(), sha(DAEMON)),
                ("humata2_holder".to_owned(), sha(b"holder")),
                ("humata".to_owned(), sha(b"cli")),
                ("stack".to_owned(), sha(b"stack")),
            ]
        );
    }

    #[test]
    fn a_binary_the_bundle_does_not_name_says_its_hash_and_names_no_bundle() {
        let bytes = manifest(host(), &sha(b"another daemon"), "a");
        let (_dir, bin, own_exe) = install(Some(&bytes));

        let release = Release::read(&bin, &own_exe, "2.0.5", 7);

        assert_eq!(release.bundle(), "");
        assert!(files(&release).is_empty());
        assert_eq!(release.info().own_sha256, sha(DAEMON));
    }

    #[test]
    fn without_a_bundle_file_the_release_names_no_bundle() {
        let (_dir, bin, own_exe) = install(None);

        let release = Release::read(&bin, &own_exe, "2.0.5", 7);

        assert_eq!(release.bundle(), "");
        assert_eq!(release.info().own_sha256, sha(DAEMON));
        assert_eq!(release.info().version, "2.0.5");
    }

    #[test]
    fn a_bundle_that_does_not_parse_names_no_bundle() {
        let (_dir, bin, own_exe) = install(Some(b"{\"platforms\": {"));

        let release = Release::read(&bin, &own_exe, "2.0.5", 7);

        assert_eq!(release.bundle(), "");
        assert_eq!(release.info().own_sha256, sha(DAEMON));
    }

    #[test]
    fn only_this_platforms_entry_counts() {
        // plan9-mips is no platform Humata ships, so it can't be this one.
        let bytes = manifest("plan9-mips", &sha(DAEMON), "a");
        let (_dir, bin, own_exe) = install(Some(&bytes));

        let release = Release::read(&bin, &own_exe, "2.0.5", 7);

        assert_eq!(release.bundle(), "");
        assert!(files(&release).is_empty());
    }

    #[test]
    fn a_binary_that_cannot_be_read_names_no_bundle_and_no_hash() {
        let bytes = manifest(host(), &sha(DAEMON), "a");
        let (dir, bin, _own_exe) = install(Some(&bytes));

        let release = Release::read(&bin, &dir.path().join("missing"), "2.0.5", 7);

        assert_eq!(release.bundle(), "");
        assert_eq!(release.info().own_sha256, "");
        assert_eq!(release.info().version, "2.0.5");
    }

    #[test]
    fn releases_whose_bundles_differ_have_different_ids() {
        let one = manifest(host(), &sha(DAEMON), "a");
        let two = manifest(host(), &sha(DAEMON), "b");
        let (_one_dir, one_bin, one_exe) = install(Some(&one));
        let (_two_dir, two_bin, two_exe) = install(Some(&two));

        let first = Release::read(&one_bin, &one_exe, "2.0.5", 7);
        let second = Release::read(&two_bin, &two_exe, "2.0.5", 7);

        assert!(!first.bundle().is_empty() && !second.bundle().is_empty());
        assert_ne!(first.bundle(), second.bundle());
    }

    #[test]
    fn short_is_the_first_seven_digits_or_the_whole_id() {
        let bundle = sha(b"a bundle");

        assert_eq!(short(&bundle), &bundle[..7]);
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }

    #[test]
    fn a_reread_names_the_bundle_installed_now() {
        let first = manifest(host(), &sha(DAEMON), "a");
        let (_dir, bin, own_exe) = install(Some(&first));
        let release = Release::read(&bin, &own_exe, "2.0.4", 3);
        let second = manifest(host(), &sha(DAEMON), "b");
        std::fs::write(bin.join(MANIFEST), &second).expect("the manifest is rewritten");

        let again = release.reread(&bin);

        assert_eq!(release.bundle(), sha(&first));
        assert_eq!(again.bundle(), sha(&second));
        assert_eq!(again.info().own_sha256, sha(DAEMON));
    }

    #[test]
    fn the_cli_and_holder_found_in_bin_are_installed_and_a_missing_one_is_left_out() {
        let bytes = manifest(host(), &sha(DAEMON), "a");
        let (_dir, bin, own_exe) = install(Some(&bytes));
        std::fs::write(bin.join("humata"), b"a cli").expect("the CLI is written");
        let release = Release::read(&bin, &own_exe, "2.0.5", 7);
        let listed = |release: &Release| -> Vec<(String, String)> {
            release
                .info()
                .installed
                .into_iter()
                .map(|file| (file.name, file.sha256))
                .collect()
        };

        assert_eq!(listed(&release), vec![("humata".to_owned(), sha(b"a cli"))]);

        std::fs::write(bin.join("humata2_holder"), b"a holder").expect("the holder is written");
        release.rescan_installed(&bin);
        assert_eq!(
            listed(&release),
            vec![
                ("humata".to_owned(), sha(b"a cli")),
                ("humata2_holder".to_owned(), sha(b"a holder")),
            ]
        );
    }

    #[test]
    fn a_whole_entry_of_any_platform_is_named_but_a_mix_is_not() {
        let bundle = manifest("linux-x86_64", &sha(b"daemon"), "a");
        let whole = vec![
            (InstalledFile::Humatad, sha(b"daemon")),
            (InstalledFile::Holder, sha(b"holder")),
            (InstalledFile::Cli, sha(b"cli")),
            (InstalledFile::Stack, sha(b"stack")),
        ];
        let mut mixed = whole.clone();
        mixed[2].1 = sha(b"another cli");

        assert!(names_whole_entry(&bundle, &whole).expect("a bundle"));
        assert!(!names_whole_entry(&bundle, &mixed).expect("a bundle"));
    }
}
