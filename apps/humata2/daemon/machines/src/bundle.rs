//! The tools bundle (TS-2): `humatad2`, `humata2_holder`, `humata`, `stack` and
//! `humata2_fusefs` for every platform Humata ships, with a `manifest.json` that names each
//! file and its SHA-256. Setup and takeover pick a platform's files by the
//! machine's `uname -sm`; the desktop app installs the host's own from it
//! (TS-0). A file is read only when a machine of its platform needs it, and
//! is checked against the manifest before anything is sent.
//!
//! ```json
//! {"platforms": {"linux-x86_64": {"humatad2": {"path": "linux-x86_64/humatad2", "sha256": "…"}, …}, …}}
//! ```
//!
//! A release is told by its bundle id: the sha256 of its `bundle.json`
//! (UP-1, D281 a), the manifest in the canonical form [`manifest_json`]
//! writes. An install puts it in `~/.humata2/bin` beside the files, and a
//! new release is first staged there as verified `.<name>.next` files
//! ([`stage_next`], D281 f) that the daemon's Upgrade call swaps in.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::install::{self, Builds, BuildsError, InstalledFile, Payload};
use crate::platform::Platform;
use crate::stack_config::{StackConfig, StackConfigError};

/// The manifest's name inside a bundle.
pub const MANIFEST: &str = "manifest.json";

/// The release's manifest as installed in `~/.humata2/bin`; a daemon reads
/// it once at start to learn which release it is.
pub(crate) const BUNDLE_JSON: &str = "bundle.json";

/// The mode of a staged tool, and of a staged `bundle.json`.
const TOOL_MODE: u32 = 0o755;
const BUNDLE_MODE: u32 = 0o644;

/// How much of a file is read at a time while it is hashed.
const CHUNK_BYTES: usize = 64 << 10;

/// The name `name` waits under while it is staged: `.<name>.next`.
pub(crate) fn next_name(name: &str) -> String {
    format!(".{name}.next")
}

#[derive(Debug, Deserialize)]
struct Manifest {
    platforms: BTreeMap<String, BTreeMap<String, Entry>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Entry {
    path: PathBuf,
    sha256: String,
}

/// One file the bundle has: where it is and what its SHA-256 must be.
#[derive(Debug, Clone, PartialEq, Eq)]
struct File {
    path: PathBuf,
    /// None for a host bundle made from the files beside the daemon: it is
    /// whatever is there.
    sha256: Option<String>,
}

/// Humata's own builds, read from disk on demand.
#[derive(Debug)]
pub struct Bundle {
    files: BTreeMap<(PlatformKey, InstalledFile), File>,
    /// Payloads currently in use, keyed by platform and file, so concurrent
    /// callers share the same `Arc<[u8]>` while idle memory drops to zero.
    loaded: Mutex<HashMap<(PlatformKey, InstalledFile), Weak<[u8]>>>,
    /// The host's `stack.toml` machines get beside the tools (D411).
    stack_config: Option<PathBuf>,
    /// Platforms the app brought tools for that this daemon may not use
    /// until it is migrated (see `with_stale_platforms`).
    stale: Vec<Platform>,
    /// The host's `repos.toml`, sent the same way (RL-12).
    repos_config: Option<PathBuf>,
    /// The host's `agents.toml` (AGENTSET), sent the same way.
    agents_config: Option<PathBuf>,
    /// The host's `machine-scripts.toml`, sent the same way.
    machine_scripts_config: Option<PathBuf>,
}

/// [`Platform`] in a form that sorts.
type PlatformKey = &'static str;

impl Bundle {
    /// The bundle in `root`, from its `manifest.json`. Paths in the
    /// manifest are relative to `root`.
    pub fn open(root: &Path) -> Result<Self, BuildsError> {
        let path = root.join(MANIFEST);
        let text = std::fs::read(&path).map_err(|source| BuildsError::ManifestUnreadable {
            path: path.clone(),
            source: Arc::new(source),
        })?;
        let manifest: Manifest = serde_json::from_slice(&text).map_err(|source| {
            BuildsError::ManifestInvalid {
                path: path.clone(),
                source: Arc::new(source),
            }
        })?;
        let mut files = BTreeMap::new();
        for (name, entries) in manifest.platforms {
            // A platform this Humata doesn't know is someone else's.
            let Some(platform) = Platform::from_bundle_name(&name) else {
                continue;
            };
            for file in InstalledFile::ALL {
                if let Some(entry) = entries.get(file.file_name()) {
                    files.insert(
                        (platform.bundle_name(), file),
                        File {
                            path: root.join(&entry.path),
                            sha256: Some(entry.sha256.to_ascii_lowercase()),
                        },
                    );
                }
            }
        }
        Ok(Self {
            files,
            loaded: Mutex::new(HashMap::new()),
            stale: Vec::new(),
            stack_config: None,
            repos_config: None,
            agents_config: None,
            machine_scripts_config: None,
        })
    }

    /// This bundle, also giving every machine the host's `stack.toml` at
    /// `path` (secrets taken out) when the file is there.
    pub fn with_stack_config(mut self, path: PathBuf) -> Self {
        self.stack_config = Some(path);
        self
    }

    /// This bundle, also giving every machine the host's `repos.toml` at
    /// `path` (RL-12) when the file is there.
    pub fn with_repos_config(mut self, path: PathBuf) -> Self {
        self.repos_config = Some(path);
        self
    }

    /// This bundle, also giving every machine the host's `agents.toml`
    /// (AGENTSET) at `path` when the file is there.
    pub fn with_agents_config(mut self, path: PathBuf) -> Self {
        self.agents_config = Some(path);
        self
    }

    /// This bundle, also giving every machine the host's `machine-scripts.toml`
    /// at `path` when the file is there.
    pub fn with_machine_scripts_config(mut self, path: PathBuf) -> Self {
        self.machine_scripts_config = Some(path);
        self
    }

    /// A bundle of only `platform`, from the files named as they are
    /// installed in `dir`: what a development build has beside humatad2
    /// when no bundle was built.
    pub fn beside(dir: &Path, platform: Platform) -> Self {
        let files = InstalledFile::ALL
            .into_iter()
            .map(|file| {
                (
                    (platform.bundle_name(), file),
                    File {
                        path: dir.join(file.file_name()),
                        sha256: None,
                    },
                )
            })
            .collect();
        Self {
            files,
            loaded: Mutex::new(HashMap::new()),
            stale: Vec::new(),
            stack_config: None,
            repos_config: None,
            agents_config: None,
            machine_scripts_config: None,
        }
    }

    /// This bundle, knowing `platforms` are tools the app brought that this
    /// daemon may not use until it is migrated (the host's own release is
    /// older than the app's). Asking for one of them, when the bundle has no
    /// files for it, is [`BuildsError::HostNeedsMigrate`], not `NoBuild`.
    pub fn with_stale_platforms(mut self, platforms: Vec<Platform>) -> Self {
        self.stale = platforms;
        self
    }

    /// This bundle, also carrying every platform of `other` that this one has
    /// no file for: the host's own tools beside the daemon, plus the Linux ones
    /// the app installed in `~/.humata2/machine-tools` (TS-2). A platform this
    /// bundle has any file for is left as it is, so a file `other` has and this
    /// one lacks is still reported missing, never filled from the other
    /// source. The bundle id ([`Builds::bundle_json`]) then names every
    /// platform of both, so a host whose own `bundle.json` should agree with
    /// it must have been installed from that same manifest.
    pub fn with_platforms_from(mut self, other: Bundle) -> Self {
        let have: Vec<PlatformKey> = self.files.keys().map(|(platform, _)| *platform).collect();
        for ((platform, file), entry) in other.files {
            if !have.contains(&platform) {
                self.files.insert((platform, file), entry);
            }
        }
        self
    }

    /// The platforms the bundle has at least one file for.
    pub fn platforms(&self) -> Vec<Platform> {
        Platform::SHIPPED
            .into_iter()
            .filter(|platform| {
                InstalledFile::ALL
                    .iter()
                    .any(|file| self.files.contains_key(&(platform.bundle_name(), *file)))
            })
            .collect()
    }

    /// The canonical `bundle.json` of this whole bundle, every platform it
    /// has (not only one machine's). Two bundles with the same files have the
    /// same bytes, so the same bundle id, however their `manifest.json`
    /// files are spaced or ordered.
    fn bundle_bytes(&self) -> Result<Vec<u8>, BuildsError> {
        let mut entries = Vec::with_capacity(self.files.len());
        for ((name, file), entry) in &self.files {
            let Some(platform) = Platform::from_bundle_name(name) else {
                continue;
            };
            let sha256 = match &entry.sha256 {
                Some(sha256) => sha256.clone(),
                // A host bundle made from the files beside the daemon: stream-hash
                // each file in 64 KiB chunks instead of loading the whole binary.
                None => hash_stream(&entry.path).map_err(|source| BuildsError::FileUnreadable {
                    platform,
                    file: *file,
                    path: entry.path.clone(),
                    source: Arc::new(source),
                })?,
            };
            entries.push((platform, *file, sha256));
        }
        Ok(manifest_json(&entries))
    }

    /// This bundle as a release for a machine of `platform`: its files, read
    /// and checked, and the `bundle.json` whose id names the release.
    pub fn release(&self, platform: Platform) -> Result<StagedRelease, StageError> {
        let files = self
            .payloads(platform)
            .map_err(|source| StageError::Builds { source })?;
        let bundle_json = self
            .bundle_bytes()
            .map_err(|source| StageError::Builds { source })?;
        StagedRelease::new(platform, bundle_json, files)
    }

    fn shipped(&self) -> String {
        let names: Vec<String> = self.platforms().into_iter().map(|platform| platform.to_string()).collect();
        if names.is_empty() {
            "no platforms".to_owned()
        } else {
            names.join(", ")
        }
    }

    fn load(&self, platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        let key = platform.bundle_name();
        if !InstalledFile::ALL
            .iter()
            .any(|file| self.files.contains_key(&(key, *file)))
        {
            if self.stale.contains(&platform) {
                return Err(BuildsError::HostNeedsMigrate { platform });
            }
            return Err(BuildsError::NoBuild {
                platform,
                shipped: self.shipped(),
            });
        }
        let mut payloads = Vec::with_capacity(InstalledFile::ALL.len());
        for file in InstalledFile::ALL {
            let entry = self
                .files
                .get(&(key, file))
                .ok_or(BuildsError::FileMissing { platform, file })?;
            let cached = self
                .loaded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&(key, file))
                .and_then(Weak::upgrade);
            let bytes = match cached {
                Some(bytes) => bytes,
                None => {
                    // Read outside the lock: the files are large.
                    let raw = std::fs::read(&entry.path).map_err(|source| BuildsError::FileUnreadable {
                        platform,
                        file,
                        path: entry.path.clone(),
                        source: Arc::new(source),
                    })?;
                    if let Some(expected) = &entry.sha256 {
                        let found = install::hex(&Sha256::digest(&raw));
                        if &found != expected {
                            return Err(BuildsError::Damaged {
                                platform,
                                file,
                                path: entry.path.clone(),
                                expected: expected.clone(),
                                found,
                            });
                        }
                    }
                    let bytes: Arc<[u8]> = Arc::from(raw);
                    let mut cache = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(existing) = cache.get(&(key, file)).and_then(Weak::upgrade) {
                        existing
                    } else {
                        cache.insert((key, file), Arc::downgrade(&bytes));
                        bytes
                    }
                }
            };
            payloads.push(Payload::new(file, bytes));
        }
        Ok(payloads)
    }
}

impl Builds for Bundle {
    fn payloads(&self, platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        self.load(platform)
    }

    fn bundle_json(&self, _platform: Platform) -> Result<Option<Arc<[u8]>>, BuildsError> {
        self.bundle_bytes().map(|bytes| Some(Arc::from(bytes)))
    }

    fn stack_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        match &self.stack_config {
            Some(path) => StackConfig::read(path),
            None => Ok(None),
        }
    }

    fn repos_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        match &self.repos_config {
            Some(path) => StackConfig::read_repos(path),
            None => Ok(None),
        }
    }

    fn agents_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        match &self.agents_config {
            Some(path) => StackConfig::read_agents(path),
            None => Ok(None),
        }
    }

    fn machine_scripts_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        match &self.machine_scripts_config {
            Some(path) => StackConfig::read_machine_scripts(path),
            None => Ok(None),
        }
    }
}

/// The canonical `bundle.json` for these files: one entry per platform and
/// file, in a fixed order, so the same release always has the same bytes and
/// so the same bundle id. Each file's `path` is `<platform>/<name>`; a
/// daemon reads only the `sha256`.
pub fn manifest_json(files: &[(Platform, InstalledFile, String)]) -> Vec<u8> {
    let sorted: BTreeMap<(&'static str, &'static str), String> = files
        .iter()
        .map(|(platform, file, sha256)| {
            (
                (platform.bundle_name(), file.file_name()),
                sha256.to_ascii_lowercase(),
            )
        })
        .collect();
    let mut platforms = serde_json::Map::new();
    for ((platform, file), sha256) in sorted {
        let entries = platforms
            .entry(platform.to_owned())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let serde_json::Value::Object(entries) = entries {
            entries.insert(
                file.to_owned(),
                serde_json::json!({"path": format!("{platform}/{file}"), "sha256": sha256}),
            );
        }
    }
    let mut root = serde_json::Map::new();
    root.insert("platforms".to_owned(), serde_json::Value::Object(platforms));
    serde_json::Value::Object(root).to_string().into_bytes()
}

/// `manifest` (a tools bundle's `manifest.json`) in the canonical form of
/// [`manifest_json`], keeping the platforms and files Humata knows.
pub fn canonical_manifest(manifest: &[u8]) -> Result<Vec<u8>, StageError> {
    let manifest: Manifest =
        serde_json::from_slice(manifest).map_err(|source| StageError::ManifestInvalid { source })?;
    let mut entries = Vec::new();
    for (name, files) in &manifest.platforms {
        let Some(platform) = Platform::from_bundle_name(name) else {
            continue;
        };
        for file in InstalledFile::ALL {
            if let Some(entry) = files.get(file.file_name()) {
                entries.push((platform, file, entry.sha256.clone()));
            }
        }
    }
    Ok(manifest_json(&entries))
}

/// One release, ready to stage on a machine of one platform: that
/// platform's files and the `bundle.json` naming them. Every file is
/// checked against the manifest when this is made, so a release that
/// disagrees with itself is never staged.
#[derive(Debug, Clone)]
pub struct StagedRelease {
    platform: Platform,
    bundle_json: Arc<[u8]>,
    id: String,
    files: Vec<Payload>,
}

impl StagedRelease {
    /// The release `bundle_json` names, with `files` as `platform`'s
    /// tools. Each file must be the manifest's entry for `platform`.
    pub fn new(platform: Platform, bundle_json: Vec<u8>, files: Vec<Payload>) -> Result<Self, StageError> {
        let manifest: Manifest = serde_json::from_slice(&bundle_json)
            .map_err(|source| StageError::ManifestInvalid { source })?;
        let entries = manifest.platforms.get(platform.bundle_name());
        let mut ordered = Vec::with_capacity(InstalledFile::ALL.len());
        for file in InstalledFile::ALL {
            let payload = files
                .iter()
                .find(|payload| payload.file() == file)
                .ok_or(StageError::MissingFile { file })?;
            let entry = entries
                .and_then(|entries| entries.get(file.file_name()))
                .ok_or(StageError::MissingEntry { platform, file })?;
            if !entry.sha256.eq_ignore_ascii_case(payload.sha256()) {
                return Err(StageError::EntryDiffers {
                    platform,
                    file,
                    manifest: entry.sha256.to_ascii_lowercase(),
                    found: payload.sha256().to_owned(),
                });
            }
            ordered.push(payload.clone());
        }
        let id = install::hex(&Sha256::digest(&bundle_json));
        Ok(Self {
            platform,
            bundle_json: Arc::from(bundle_json),
            id,
            files: ordered,
        })
    }

    /// A release of only `platform`, whose manifest is made from `files`:
    /// what a development build with no tools bundle has.
    pub fn for_platform(platform: Platform, files: Vec<Payload>) -> Result<Self, StageError> {
        let entries: Vec<(Platform, InstalledFile, String)> = files
            .iter()
            .map(|payload| (platform, payload.file(), payload.sha256().to_owned()))
            .collect();
        Self::new(platform, manifest_json(&entries), files)
    }

    /// The bundle id: the sha256 of `bundle.json`.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The files, in [`InstalledFile::ALL`] order.
    pub fn files(&self) -> &[Payload] {
        &self.files
    }

    /// The `bundle.json` bytes, whose sha256 is the id.
    pub fn bundle_json(&self) -> &Arc<[u8]> {
        &self.bundle_json
    }
}

/// Why a release couldn't be made or staged. Each message says what to
/// do; the cause is its source.
#[derive(Debug, Error)]
pub enum StageError {
    #[error(
        "Humata's own bundle manifest isn't in the bundle format ({source}). Humata's install is damaged; reinstall Humata"
    )]
    ManifestInvalid {
        #[source]
        source: serde_json::Error,
    },
    #[error("Humata's own copies of the tools couldn't be read. {source}")]
    Builds {
        #[source]
        source: BuildsError,
    },
    #[error("this release has no {file}, so it can't be staged; reinstall Humata")]
    MissingFile { file: InstalledFile },
    #[error(
        "this release's manifest names no {file} for {platform}, so it can't be staged; reinstall Humata"
    )]
    MissingEntry { platform: Platform, file: InstalledFile },
    #[error(
        "this release's {file} for {platform} has SHA-256 {found}, not the {manifest} its manifest says, so nothing was staged. Humata's install is damaged; reinstall Humata from a release you trust rather than editing the manifest"
    )]
    EntryDiffers {
        platform: Platform,
        file: InstalledFile,
        manifest: String,
        found: String,
    },
    #[error(
        "couldn't create {}: {source}. Check that the folder's parent is yours and writable, then try again",
        .path.display()
    )]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't read {}: {source}. Check that you own it, or remove it so Humata can stage a fresh copy, then try again",
        .path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't write {}: {source}. Free some disk space or fix the folder's permissions, then try again; the installed files were not touched",
        .path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{} reads back with SHA-256 {found}, not {expected}, so it was removed and nothing of this release is usable yet. The disk may be failing; check it, then try again",
        .path.display()
    )]
    Verify {
        path: PathBuf,
        expected: String,
        found: String,
    },
    #[error(
        "couldn't move the staged copy into {}: {source}. Fix the folder's permissions, then try again; the installed files were not touched",
        .path.display()
    )]
    Rename {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Stages `release` in `bin` (`~/.humata2/bin`): each tool as
/// `.<name>.next` (0755) and then `.bundle.json.next` (0644), every one
/// written beside, flushed, read back against its sha256 and renamed in, so
/// a kill at any point leaves whole files or none, and the installed files
/// are never touched. The bundle goes last: when `.bundle.json.next` is
/// there, the files it names are too. Staging again is safe and rewrites
/// only what differs. Returns the bundle id.
///
/// This only blocks on the disk; callers in async code run it on a
/// blocking thread.
pub fn stage_next(bin: &Path, release: &StagedRelease) -> Result<String, StageError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(bin)
        .map_err(|source| StageError::CreateDir {
            path: bin.to_owned(),
            source,
        })?;
    remove_leftovers(bin)?;
    for payload in release.files() {
        put(
            bin,
            &next_name(payload.file().file_name()),
            payload.bytes(),
            payload.sha256(),
            TOOL_MODE,
        )?;
    }
    put(
        bin,
        &next_name(BUNDLE_JSON),
        release.bundle_json(),
        release.id(),
        BUNDLE_MODE,
    )?;
    // The renames are only as durable as their folder.
    fs::File::open(bin)
        .and_then(|dir| dir.sync_all())
        .map_err(|source| StageError::Write {
            path: bin.to_owned(),
            source,
        })?;
    Ok(release.id().to_owned())
}

/// Writes `bytes` as `bin/<dest>` unless it already holds them.
fn put(bin: &Path, dest: &str, bytes: &[u8], sha256: &str, mode: u32) -> Result<(), StageError> {
    let target = bin.join(dest);
    if hash_file(&target)?.as_deref() == Some(sha256) {
        return fs::set_permissions(&target, fs::Permissions::from_mode(mode)).map_err(|source| {
            StageError::Write {
                path: target,
                source,
            }
        });
    }
    let temp = bin.join(format!("{dest}.{}.new", install::nonce()));
    let write = || -> io::Result<()> {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)?;
        out.write_all(bytes)?;
        out.set_permissions(fs::Permissions::from_mode(mode))?;
        out.sync_all()
    };
    if let Err(source) = write() {
        let _ = fs::remove_file(&temp);
        return Err(StageError::Write { path: temp, source });
    }
    let found = hash_file(&temp)?.unwrap_or_default();
    if found != sha256 {
        let _ = fs::remove_file(&temp);
        return Err(StageError::Verify {
            path: target,
            expected: sha256.to_owned(),
            found,
        });
    }
    fs::rename(&temp, &target).map_err(|source| {
        let _ = fs::remove_file(&temp);
        StageError::Rename {
            path: target,
            source,
        }
    })
}

/// Removes the temporary files of a staging that was killed.
fn remove_leftovers(bin: &Path) -> Result<(), StageError> {
    let entries = fs::read_dir(bin).map_err(|source| StageError::Read {
        path: bin.to_owned(),
        source,
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && name.contains(".next.") && name.ends_with(".new") {
            match fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(StageError::Write {
                        path: entry.path(),
                        source,
                    });
                }
            }
        }
    }
    Ok(())
}

/// The SHA-256 of the file at `path`, streamed in [`CHUNK_BYTES`] chunks.
fn hash_stream(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0; CHUNK_BYTES];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => hasher.update(&chunk[..count]),
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => return Err(source),
        }
    }
    Ok(install::hex(&hasher.finalize()))
}

/// The SHA-256 of the file at `path`; none when there is none.
fn hash_file(path: &Path) -> Result<Option<String>, StageError> {
    match hash_stream(path) {
        Ok(digest) => Ok(Some(digest)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StageError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Arch, Os};

    const LINUX: Platform = Platform {
        os: Os::Linux,
        arch: Arch::X86_64,
    };

    fn write_bundle(root: &Path, files: &[(Platform, InstalledFile, &[u8])], damage: Option<InstalledFile>) {
        let mut platforms: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
        for (platform, file, bytes) in files {
            let relative = format!("{}/{}", platform.bundle_name(), file.file_name());
            std::fs::create_dir_all(root.join(platform.bundle_name())).unwrap();
            let written: &[u8] = if damage == Some(*file) { b"replaced" } else { bytes };
            std::fs::write(root.join(&relative), written).unwrap();
            platforms.entry(platform.bundle_name().to_owned()).or_default().insert(
                file.file_name().to_owned(),
                serde_json::json!({"path": relative, "sha256": install::hex(&Sha256::digest(bytes))}),
            );
        }
        let manifest = serde_json::json!({ "platforms": platforms });
        std::fs::write(root.join(MANIFEST), manifest.to_string()).unwrap();
    }

    fn all(platform: Platform) -> Vec<(Platform, InstalledFile, &'static [u8])> {
        vec![
            (platform, InstalledFile::Humatad, b"daemon"),
            (platform, InstalledFile::Holder, b"holder"),
            (platform, InstalledFile::Cli, b"cli"),
            (platform, InstalledFile::Stack, b"stack"),
            (platform, InstalledFile::FuseFs, b"fusefs"),
        ]
    }

    #[test]
    fn a_platform_s_files_come_out_checked() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), None);
        let bundle = Bundle::open(root.path()).unwrap();

        let payloads = bundle.payloads(LINUX).unwrap();

        let files: Vec<(InstalledFile, Vec<u8>)> = payloads
            .iter()
            .map(|payload| (payload.file(), payload.bytes().to_vec()))
            .collect();
        assert_eq!(
            files,
            [
                (InstalledFile::Humatad, b"daemon".to_vec()),
                (InstalledFile::Holder, b"holder".to_vec()),
                (InstalledFile::Cli, b"cli".to_vec()),
                (InstalledFile::Stack, b"stack".to_vec()),
                (InstalledFile::FuseFs, b"fusefs".to_vec()),
            ]
        );
        assert_eq!(bundle.platforms(), [LINUX]);
    }

    #[test]
    fn merging_adds_the_platforms_the_bundle_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), None);
        let mac = Platform { os: Os::Darwin, arch: Arch::Aarch64 };

        let merged = Bundle::beside(dir.path(), mac).with_platforms_from(Bundle::open(root.path()).unwrap());

        assert_eq!(merged.platforms(), [mac, LINUX]);
        assert_eq!(merged.payloads(LINUX).unwrap().len(), 5);
    }

    #[test]
    fn merging_keeps_a_platform_the_bundle_already_has() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        for file in InstalledFile::ALL {
            std::fs::write(dir.path().join(file.file_name()), b"mine").unwrap();
        }
        write_bundle(root.path(), &all(LINUX), None);

        let merged = Bundle::beside(dir.path(), LINUX).with_platforms_from(Bundle::open(root.path()).unwrap());

        assert_eq!(merged.platforms(), [LINUX]);
        assert_eq!(merged.payloads(LINUX).unwrap()[0].bytes().as_ref(), b"mine");
    }

    #[test]
    fn a_merged_platform_missing_a_file_stays_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX)[..2], None);
        let mac = Platform { os: Os::Darwin, arch: Arch::Aarch64 };

        let merged = Bundle::beside(dir.path(), mac).with_platforms_from(Bundle::open(root.path()).unwrap());

        assert!(matches!(
            merged.payloads(LINUX),
            Err(BuildsError::FileMissing { file: InstalledFile::Cli, .. })
        ));
    }

    #[test]
    fn a_merged_bundle_is_one_release_naming_every_platform() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        for file in InstalledFile::ALL {
            std::fs::write(dir.path().join(file.file_name()), file.file_name()).unwrap();
        }
        write_bundle(root.path(), &all(LINUX), None);
        let mac = Platform { os: Os::Darwin, arch: Arch::Aarch64 };

        let merged = Bundle::beside(dir.path(), mac).with_platforms_from(Bundle::open(root.path()).unwrap());
        let json = merged.bundle_json(LINUX).unwrap().unwrap();
        let manifest: Manifest = serde_json::from_slice(&json).unwrap();

        assert!(manifest.platforms.contains_key(mac.bundle_name()));
        assert!(manifest.platforms.contains_key(LINUX.bundle_name()));
        // What a machine of either platform is staged with names the same release.
        assert_eq!(merged.release(LINUX).unwrap().id(), merged.release(mac).unwrap().id());
    }

    #[test]
    fn a_stale_platform_asks_for_the_hosts_migrate_not_no_build() {
        let dir = tempfile::tempdir().unwrap();
        let mac = Platform { os: Os::Darwin, arch: Arch::Aarch64 };
        let other = Platform { os: Os::Linux, arch: Arch::Aarch64 };

        let bundle = Bundle::beside(dir.path(), mac).with_stale_platforms(vec![LINUX]);

        assert!(matches!(
            bundle.payloads(LINUX),
            Err(BuildsError::HostNeedsMigrate { platform }) if platform == LINUX
        ));
        // Only the stale platforms: another one without files is still no build.
        assert!(matches!(bundle.payloads(other), Err(BuildsError::NoBuild { .. })));
    }

    #[test]
    fn a_platform_the_bundle_has_is_not_made_stale() {
        let dir = tempfile::tempdir().unwrap();
        for file in InstalledFile::ALL {
            std::fs::write(dir.path().join(file.file_name()), b"x").unwrap();
        }

        let bundle = Bundle::beside(dir.path(), LINUX).with_stale_platforms(vec![LINUX]);

        assert_eq!(bundle.payloads(LINUX).unwrap().len(), 5);
    }

    #[test]
    fn a_platform_without_files_is_no_build() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), None);
        let bundle = Bundle::open(root.path()).unwrap();
        let mac = Platform {
            os: Os::Darwin,
            arch: Arch::Aarch64,
        };

        assert!(matches!(bundle.payloads(mac), Err(BuildsError::NoBuild { platform, .. }) if platform == mac));
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), Some(InstalledFile::Holder));
        let bundle = Bundle::open(root.path()).unwrap();

        assert!(matches!(
            bundle.payloads(LINUX),
            Err(BuildsError::Damaged { file: InstalledFile::Holder, .. })
        ));
    }

    #[test]
    fn a_missing_file_and_a_missing_manifest_are_reported() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            Bundle::open(root.path()),
            Err(BuildsError::ManifestUnreadable { .. })
        ));

        write_bundle(root.path(), &all(LINUX)[..2], None);
        let bundle = Bundle::open(root.path()).unwrap();
        assert!(matches!(
            bundle.payloads(LINUX),
            Err(BuildsError::FileMissing { file: InstalledFile::Cli, .. })
        ));
    }

    #[test]
    fn files_beside_the_daemon_make_a_host_bundle() {
        let dir = tempfile::tempdir().unwrap();
        for file in InstalledFile::ALL {
            std::fs::write(dir.path().join(file.file_name()), file.file_name()).unwrap();
        }
        let bundle = Bundle::beside(dir.path(), LINUX);

        let payloads = bundle.payloads(LINUX).unwrap();

        assert_eq!(payloads.len(), 5);
        assert_eq!(payloads[2].bytes().as_ref(), b"humata");
        assert_eq!(payloads[3].bytes().as_ref(), b"stack");
        assert_eq!(payloads[4].bytes().as_ref(), b"humata2_fusefs");
    }

    #[test]
    fn a_bundle_gives_the_hosts_stack_settings_only_when_it_has_a_file_to_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = StackConfig::host_path(dir.path());
        let bundle = Bundle::beside(dir.path(), LINUX).with_stack_config(path.clone());
        assert_eq!(bundle.stack_config().unwrap(), None);

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "prefix = \"a\"\ntoken = \"x\"\n").unwrap();

        assert_eq!(
            bundle.stack_config().unwrap(),
            Some(StackConfig::from_text("prefix = \"a\"\ntoken = \"x\"\n"))
        );
        assert_eq!(Bundle::beside(dir.path(), LINUX).stack_config().unwrap(), None);
    }

    #[test]
    fn a_bundle_gives_the_hosts_repo_settings_apart_from_the_stack_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = StackConfig::repos_host_path(dir.path());
        let bundle = Bundle::beside(dir.path(), LINUX).with_repos_config(path.clone());
        assert_eq!(bundle.repos_config().unwrap(), None);

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "owners = [\"acme\"]\ntoken = \"x\"\n").unwrap();

        let given = bundle.repos_config().unwrap().expect("the file is there");
        assert_eq!(given.dropped(), ["token"]);
        assert_eq!(bundle.stack_config().unwrap(), None);
    }

    #[test]
    fn a_bundle_gives_the_hosts_agent_switches_apart_from_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = StackConfig::agents_host_path(dir.path());
        let bundle = Bundle::beside(dir.path(), LINUX).with_agents_config(path.clone());
        assert_eq!(bundle.agents_config().unwrap(), None);

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[agents.codex]\nenabled = false\ntoken = \"x\"\n").unwrap();

        let given = bundle.agents_config().unwrap().expect("the file is there");
        assert_eq!(given.dropped(), ["token"]);
        assert_eq!(bundle.repos_config().unwrap(), None);
    }

    const MAC: Platform = Platform {
        os: Os::Darwin,
        arch: Arch::Aarch64,
    };

    fn sha(bytes: &[u8]) -> String {
        install::hex(&Sha256::digest(bytes))
    }

    fn payloads(tag: &str) -> Vec<Payload> {
        InstalledFile::ALL
            .iter()
            .map(|file| Payload::new(*file, Arc::from(format!("{file} {tag}").into_bytes())))
            .collect()
    }

    fn release(tag: &str) -> StagedRelease {
        StagedRelease::for_platform(LINUX, payloads(tag)).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_manifest_has_the_same_bytes_whatever_order_its_files_come_in() {
        let entries = vec![
            (MAC, InstalledFile::Cli, sha(b"c")),
            (LINUX, InstalledFile::Humatad, sha(b"a")),
            (MAC, InstalledFile::Humatad, sha(b"b")),
        ];
        let mut backwards = entries.clone();
        backwards.reverse();

        assert_eq!(manifest_json(&entries), manifest_json(&backwards));
        let mut other = entries.clone();
        other[0].2 = sha(b"changed");
        assert_ne!(manifest_json(&entries), manifest_json(&other));
    }

    #[test]
    fn a_manifest_is_what_a_bundle_opens_and_a_daemon_reads() {
        let root = tempfile::tempdir().unwrap();
        let entries: Vec<(Platform, InstalledFile, String)> = all(LINUX)
            .iter()
            .map(|(platform, file, bytes)| (*platform, *file, sha(bytes)))
            .collect();
        fs::write(root.path().join(MANIFEST), manifest_json(&entries)).unwrap();
        let bundle = Bundle::open(root.path()).unwrap();

        assert_eq!(bundle.platforms(), [LINUX]);
        // The shape the daemon's build identity reads: platform, file, sha256.
        let read: serde_json::Value = serde_json::from_slice(&manifest_json(&entries)).unwrap();
        assert_eq!(
            read["platforms"]["linux-x86_64"]["humatad2"]["sha256"],
            sha(b"daemon")
        );
        // The FUSEFS helper is listed under its own file name, like the rest.
        assert_eq!(
            read["platforms"]["linux-x86_64"]["humata2_fusefs"]["sha256"],
            sha(b"fusefs")
        );
    }

    #[test]
    fn a_release_is_named_by_the_hash_of_its_manifest_not_by_how_it_was_spaced() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), None);
        let compact = Bundle::open(root.path()).unwrap().release(LINUX).unwrap();

        // The same manifest, spaced out and with a platform Humata doesn't know.
        let text = fs::read_to_string(root.path().join(MANIFEST)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut spaced = value.clone();
        spaced["platforms"]["plan9-mips"] = value["platforms"]["linux-x86_64"].clone();
        fs::write(root.path().join(MANIFEST), serde_json::to_string_pretty(&spaced).unwrap()).unwrap();
        let again = Bundle::open(root.path()).unwrap().release(LINUX).unwrap();

        assert_eq!(compact.id(), again.id());
        assert_eq!(compact.id(), sha(&canonical_manifest(text.as_bytes()).unwrap()));
        let other = tempfile::tempdir().unwrap();
        write_bundle(
            other.path(),
            &[
                (LINUX, InstalledFile::Humatad, b"another daemon"),
                (LINUX, InstalledFile::Holder, b"holder"),
                (LINUX, InstalledFile::Cli, b"cli"),
                (LINUX, InstalledFile::Stack, b"stack"),
                (LINUX, InstalledFile::FuseFs, b"fusefs"),
            ],
            None,
        );
        assert_ne!(Bundle::open(other.path()).unwrap().release(LINUX).unwrap().id(), compact.id());
    }

    #[test]
    fn a_bundle_beside_the_daemon_is_a_release_of_that_platform_alone() {
        let dir = tempfile::tempdir().unwrap();
        for file in InstalledFile::ALL {
            fs::write(dir.path().join(file.file_name()), file.file_name()).unwrap();
        }
        let bundle = Bundle::beside(dir.path(), LINUX);

        let release = bundle.release(LINUX).unwrap();
        let json = bundle.bundle_json(LINUX).unwrap().unwrap();

        assert_eq!(release.id(), sha(&json));
        assert_eq!(release.files().len(), 5);
        assert_eq!(release.platform(), LINUX);
    }

    #[test]
    fn a_release_whose_files_are_not_the_manifests_is_refused() {
        let mut files = payloads("one");
        let manifest = release("one").bundle_json().to_vec();
        files[1] = Payload::new(InstalledFile::Holder, Arc::from(&b"not the holder"[..]));

        assert!(matches!(
            StagedRelease::new(LINUX, manifest.clone(), files),
            Err(StageError::EntryDiffers { file: InstalledFile::Holder, .. })
        ));
        assert!(matches!(
            StagedRelease::new(LINUX, manifest.clone(), payloads("one")[..2].to_vec()),
            Err(StageError::MissingFile { file: InstalledFile::Cli })
        ));
        assert!(matches!(
            StagedRelease::new(MAC, manifest, payloads("one")),
            Err(StageError::MissingEntry { .. })
        ));
        assert!(matches!(
            StagedRelease::new(LINUX, b"{\"platforms\": {".to_vec(), payloads("one")),
            Err(StageError::ManifestInvalid { .. })
        ));
    }

    #[test]
    fn staging_writes_every_file_and_the_bundle_beside_what_is_installed() {
        let bin = tempfile::tempdir().unwrap();
        fs::write(bin.path().join("humatad2"), b"old daemon").unwrap();
        fs::write(bin.path().join("bundle.json"), b"old bundle").unwrap();
        let release = release("two");

        let id = stage_next(bin.path(), &release).unwrap();

        assert_eq!(id, release.id());
        assert_eq!(id, sha(&fs::read(bin.path().join(".bundle.json.next")).unwrap()));
        for payload in release.files() {
            let staged = bin.path().join(format!(".{}.next", payload.file()));
            assert_eq!(fs::read(&staged).unwrap(), payload.bytes().to_vec());
            assert_eq!(mode(&staged), 0o755);
        }
        assert_eq!(mode(&bin.path().join(".bundle.json.next")), 0o644);
        // What runs is untouched, and nothing else is left behind.
        assert_eq!(fs::read(bin.path().join("humatad2")).unwrap(), b"old daemon");
        assert_eq!(fs::read(bin.path().join("bundle.json")).unwrap(), b"old bundle");
        assert_eq!(
            names(bin.path()),
            [
                ".bundle.json.next",
                ".humata.next",
                ".humata2_fusefs.next",
                ".humata2_holder.next",
                ".humatad2.next",
                ".stack.next",
                "bundle.json",
                "humatad2"
            ]
        );
    }

    #[test]
    fn staging_again_or_a_newer_release_leaves_whole_files_of_the_last() {
        let bin = tempfile::tempdir().unwrap();
        let first = release("one");
        stage_next(bin.path(), &first).unwrap();
        // A killed staging left a temporary file.
        fs::write(bin.path().join(".humatad2.next.feedfacefeedface.new"), b"half").unwrap();

        stage_next(bin.path(), &first).unwrap();
        assert_eq!(names(bin.path()).len(), 6);

        let second = release("two");
        assert_eq!(stage_next(bin.path(), &second).unwrap(), second.id());
        assert_eq!(
            fs::read(bin.path().join(".humatad2.next")).unwrap(),
            second.files()[0].bytes().to_vec()
        );
        assert_eq!(names(bin.path()).len(), 6);
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_reported_with_where() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bin"), b"a file, not a folder").unwrap();

        let result = stage_next(&dir.path().join("bin").join("inside"), &release("one"));

        assert!(matches!(result, Err(StageError::CreateDir { .. })), "{result:?}");
    }
}
