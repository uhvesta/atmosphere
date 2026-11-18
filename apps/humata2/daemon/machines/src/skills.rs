//! Skills sync, the engine (SK-1 to SK-11, PER-27): scan the host's skill
//! folders into a set, plan what a machine needs, and apply a set on the
//! machine so a reader sees the whole old set or the whole new one.
//!
//! Filesystem only: no network, no gRPC. The caller decides every real path.
//! On a machine, `<root>/.skills` is a symlink to `<root>/.skills-sets/<id>`,
//! and nothing else under `<root>` is ever touched (SK-4).
//!
//! Each agent reads its own skills folder (`~/.copilot/skills`,
//! `~/.claude/skills`, `~/.codex/skills`), so after a sync
//! [`Managed::link_agents`] puts one link per synced skill there,
//! `<folder>/<name>` → `<root>/.skills/<agent>/<name>`. The links go through
//! the stable `.skills` link, so an agent sees the old set or the new one,
//! never a mix (SK-9). A folder, file or link Humata didn't make is never
//! replaced or removed (SK-4); no home variable is set (D193, VF-5).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const SKILLS_LINK: &str = ".skills";
const SETS_DIR: &str = ".skills-sets";
const CHOICES_FILE: &str = ".skills-choices.json";
const MANIFEST_FILE: &str = "manifest.json";
const INCOMING_PREFIX: &str = ".incoming-";
const LINK_PREFIX: &str = ".skills.new-";
const FORMAT_VERSION: u32 = 1;

/// File names that look like secrets (SK-6). Matched, lowercased, against a
/// file's own name, whatever the user's exclude patterns say.
const SECRET_NAMES: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa*",
    "id_ed25519*",
    "id_ecdsa*",
    ".netrc",
    ".npmrc",
    "credentials",
    "credentials.*",
    "*.token",
    "token",
    ".token",
];

/// What went wrong scanning, planning or applying skills.
#[derive(Debug, Error)]
pub enum SkillsError {
    /// A filesystem step failed.
    #[error("couldn't {action} {}; check that the folder exists, is writable and the disk isn't full, then retry", .path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// An agent name that can't be a folder name.
    #[error("the agent name {agent:?} can't be used as a folder name; use a plain name such as `claude`")]
    BadAgent { agent: String },
    /// `<root>/.skills` exists and isn't Humata's link.
    #[error(
        "{} exists but isn't the link Humata manages; Humata won't delete it. Move it away (for example rename it to `skills-mine`) and sync again",
        .path.display()
    )]
    ForeignSkillsPath { path: PathBuf },
    /// A path in a manifest or a chunk that must not be written.
    #[error("the path {path:?} is not allowed ({reason}); the sender must use relative paths of plain names listed in the manifest")]
    BadPath { path: String, reason: &'static str },
    /// A chunk for a file the manifest doesn't list.
    #[error("{path:?} is not in the manifest, so it was not written; the sender must list every file first")]
    NotInManifest { path: String },
    /// A chunk that doesn't continue the file.
    #[error("a chunk for {path:?} starts at byte {got} but the file has {expected} bytes so far; send the chunks in order")]
    OutOfOrder { path: String, expected: u64, got: u64 },
    /// More bytes than the manifest says.
    #[error("{path:?} would grow past the {size} bytes the manifest lists; resend the set")]
    TooLarge { path: String, size: u64 },
    /// A listed file that is missing or short at the end.
    #[error("{path:?} has {found} of {expected} bytes at the end of the transfer; resend the set")]
    Incomplete { path: String, expected: u64, found: u64 },
    /// A file whose content doesn't match its listed hash.
    #[error("{path:?} doesn't match its SHA-256 in the manifest; the current set was kept, resend the set")]
    HashMismatch { path: String },
    /// A manifest that isn't valid JSON of the expected shape.
    #[error("the skills manifest can't be read as JSON; resend the set")]
    ManifestJson {
        #[source]
        source: serde_json::Error,
    },
    /// A manifest of a format this version doesn't know.
    #[error("the skills manifest has format {found}, which this Humata doesn't know; update Humata on both sides")]
    ManifestVersion { found: u32 },
    /// A manifest whose set id isn't the id of its entries.
    #[error("the skills manifest's set id doesn't match its files; resend the set")]
    ManifestId,
    /// The skills this binary carries can't be read: a fault in the build.
    #[error("the skills built into this Humata can't be read ({what}), so none were installed; this is a fault in this build of Humata, not in your files. Update Humata, and report it if the newest release says the same")]
    OwnBundle { what: &'static str },
    /// The clash choices file can't be read.
    #[error("{} isn't valid JSON; fix or delete it (your clash picks will be asked again)", .path.display())]
    ChoicesFile {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub(crate) fn io_err<'a>(action: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> SkillsError + 'a {
    move |source| SkillsError::Io { action, path: path.to_path_buf(), source }
}

/// A folder of skills for one agent on the host.
#[derive(Debug, Clone)]
pub struct Source {
    /// The agent's folder name in the set (`claude`, `copilot`).
    pub agent: String,
    /// The host folder whose top-level entries are skills.
    pub root: PathBuf,
}

/// One file of a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillEntry {
    /// `<agent>/<skill>/<path…>`.
    pub path: String,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
    /// Whether the file is executable.
    pub executable: bool,
    /// Where the host reads it from; empty in a manifest from a machine.
    #[serde(skip)]
    pub source: PathBuf,
}

/// A scanned set of skill files, sorted, with a stable id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSet {
    entries: Vec<SkillEntry>,
    excluded_files: u32,
    set_id: String,
}

impl SkillSet {
    /// The files, sorted by path.
    pub fn entries(&self) -> &[SkillEntry] {
        &self.entries
    }

    /// Files left out: secret-looking, excluded, symlinks (SK-6).
    pub fn excluded_files(&self) -> u32 {
        self.excluded_files
    }

    /// The same for the same content, whatever the order it was found in.
    pub fn set_id(&self) -> &str {
        &self.set_id
    }

    /// The manifest a machine stores with the set.
    pub fn manifest(&self) -> SkillsManifest {
        SkillsManifest {
            version: FORMAT_VERSION,
            set_id: self.set_id.clone(),
            entries: self.entries.clone(),
        }
    }
}

/// What a machine stores inside a synced set as `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsManifest {
    /// Format version.
    pub version: u32,
    /// The set's id.
    pub set_id: String,
    /// Every file of the set.
    pub entries: Vec<SkillEntry>,
}

impl SkillsManifest {
    /// The manifest as JSON.
    pub fn to_json(&self) -> Result<String, SkillsError> {
        serde_json::to_string_pretty(self).map_err(|source| SkillsError::ManifestJson { source })
    }

    /// Reads a manifest, checking its version and that its id fits its files.
    pub fn from_json(text: &str) -> Result<Self, SkillsError> {
        let manifest: Self =
            serde_json::from_str(text).map_err(|source| SkillsError::ManifestJson { source })?;
        if manifest.version != FORMAT_VERSION {
            return Err(SkillsError::ManifestVersion { found: manifest.version });
        }
        let mut entries = manifest.entries.clone();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        if set_id(&entries) != manifest.set_id {
            return Err(SkillsError::ManifestId);
        }
        Ok(manifest)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn set_id(sorted: &[SkillEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in sorted {
        hasher.update(entry.path.as_bytes());
        hasher.update([0]);
        hasher.update(entry.sha256.as_bytes());
        hasher.update([0]);
        hasher.update(if entry.executable { b"x" } else { b"-" });
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// Matches `*` (not `/`), `?` (one character, not `/`) and `**` (anything,
/// `/` included) against `text`.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob(&p, &t)
}

fn glob(p: &[char], t: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') if p.get(1) == Some(&'*') => {
            let rest = &p[2..];
            // `**/` also matches no folder at all.
            let after_slash = rest.first() == Some(&'/');
            if after_slash && glob(&rest[1..], t) {
                return true;
            }
            (0..=t.len()).any(|i| glob(rest, &t[i..]))
        }
        Some('*') => {
            let rest = &p[1..];
            let mut i = 0;
            loop {
                if glob(rest, &t[i..]) {
                    return true;
                }
                if i < t.len() && t[i] != '/' {
                    i += 1;
                } else {
                    return false;
                }
            }
        }
        Some('?') => !t.is_empty() && t[0] != '/' && glob(&p[1..], &t[1..]),
        Some(c) => t.first() == Some(c) && glob(&p[1..], &t[1..]),
    }
}

fn looks_secret(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECRET_NAMES.iter().any(|pattern| glob_matches(pattern, &lower))
}

fn excluded_by(patterns: &[String], relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    patterns.iter().any(|pattern| {
        glob_matches(pattern, relative) || (!pattern.contains('/') && glob_matches(pattern, name))
    })
}

fn plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// Scans the sources into a set (SK-1). Every top-level entry of a source is
/// a skill. Symlinks are never followed, secret-looking files are never
/// included (SK-6), and `excludes` are globs (`*`, `?`, `**`) matched against
/// the path under the source root, or a bare file name when the glob has no
/// `/`. A source folder that doesn't exist is an agent that isn't installed
/// and gives no skills. A skill Humata installed itself (its folder holds
/// Humata's mark, D790) is not part of the set and is not counted as
/// excluded: every machine's daemon installs its own.
pub fn scan(sources: &[Source], excludes: &[String]) -> Result<SkillSet, SkillsError> {
    let mut entries = Vec::new();
    let mut excluded = 0u32;
    for source in sources {
        if !plain_name(&source.agent) {
            return Err(SkillsError::BadAgent { agent: source.agent.clone() });
        }
        walk(source, &source.root, "", excludes, &mut entries, &mut excluded)?;
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries.dedup_by(|a, b| a.path == b.path);
    let set_id = set_id(&entries);
    Ok(SkillSet { entries, excluded_files: excluded, set_id })
}

fn walk(
    source: &Source,
    dir: &Path,
    relative: &str,
    excludes: &[String],
    entries: &mut Vec<SkillEntry>,
    excluded: &mut u32,
) -> Result<(), SkillsError> {
    let listing = match fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind() == io::ErrorKind::NotFound && relative.is_empty() => return Ok(()),
        Err(source) => return Err(SkillsError::Io { action: "read the folder", path: dir.to_path_buf(), source }),
    };
    let mut children = Vec::new();
    for child in listing {
        children.push(child.map_err(io_err("read the folder", dir))?);
    }
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            *excluded += 1;
            continue;
        };
        let rel = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
        let kind = child.file_type().map_err(io_err("inspect", &path))?;
        if kind.is_symlink() {
            *excluded += 1;
        } else if kind.is_dir() {
            if relative.is_empty() && path.join(crate::own_skills::MARK).is_file() {
                continue;
            }
            walk(source, &path, &rel, excludes, entries, excluded)?;
        } else if kind.is_file() {
            if looks_secret(&name) || excluded_by(excludes, &rel) {
                *excluded += 1;
                continue;
            }
            let (sha256, size, executable) = hash_file(&path)?;
            entries.push(SkillEntry {
                path: format!("{}/{rel}", source.agent),
                sha256,
                size,
                executable,
                source: path,
            });
        } else {
            *excluded += 1;
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<(String, u64, bool), SkillsError> {
    let mut file = File::open(path).map_err(io_err("open", path))?;
    let mode = file.metadata().map_err(io_err("inspect", path))?.permissions().mode();
    let (sha, size) = hash_reader(&mut file).map_err(io_err("read", path))?;
    Ok((sha, size, mode & 0o111 != 0))
}

fn hash_reader(reader: &mut impl Read) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((hex(&hasher.finalize()), size))
}

/// A skill's agent and name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SkillName {
    /// The agent folder.
    pub agent: String,
    /// The skill's name.
    pub skill: String,
}

/// What a sync does to one skill (SK-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// In the new set, not on the machine.
    Add,
    /// Any file of the skill differs, appeared or went.
    Update,
    /// On the machine, not in the new set.
    Remove,
}

/// One skill the sync changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The skill.
    pub name: SkillName,
    /// What happens to it.
    pub kind: ChangeKind,
}

/// The user's pick for a name clash (SK-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// The synced skill is used.
    Synced,
    /// The machine's own skill is used.
    Local,
}

/// The state of a clash in a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClashUse {
    /// Not picked yet: the machine-local one is used.
    Unpicked,
    /// The synced one is used.
    Synced,
    /// The machine-local one is used.
    Local,
}

/// A synced skill whose name a machine-local skill also has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    /// The skill.
    pub name: SkillName,
    /// Which one is used now.
    pub current: ClashUse,
    /// No saved choice yet; an automatic sync must ask (SK-11).
    pub is_new: bool,
}

/// What a sync would do to a machine (SK-5, SK-11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The machine has no synced set yet.
    pub first_sync: bool,
    /// Skills added, updated and removed, sorted; empty when nothing changed.
    pub changes: Vec<Change>,
    /// Name clashes with machine-local skills.
    pub clashes: Vec<Clash>,
    /// Files left out of the set (SK-6).
    pub excluded_files: u32,
}

impl Plan {
    /// Whether an automatic sync must ask the user first: a first sync
    /// (SK-5) or a clash with no saved choice (SK-11).
    pub fn needs_asking(&self) -> bool {
        self.first_sync || self.clashes.iter().any(|clash| clash.is_new)
    }
}

type SkillFiles<'a> = BTreeMap<SkillName, BTreeMap<&'a str, (&'a str, bool)>>;

fn by_skill(entries: &[SkillEntry]) -> SkillFiles<'_> {
    let mut skills: SkillFiles<'_> = BTreeMap::new();
    for entry in entries {
        let mut parts = entry.path.splitn(3, '/');
        let (Some(agent), Some(skill)) = (parts.next(), parts.next()) else { continue };
        skills
            .entry(SkillName { agent: agent.to_owned(), skill: skill.to_owned() })
            .or_default()
            .insert(entry.path.as_str(), (entry.sha256.as_str(), entry.executable));
    }
    skills
}

/// Works out what syncing `set` does to a machine that has `current` (its
/// stored manifest, if any), given the machine-local skill names per agent
/// and the saved clash `choices` (SK-5, SK-11).
pub fn plan(
    set: &SkillSet,
    current: Option<&SkillsManifest>,
    local: &BTreeMap<String, BTreeSet<String>>,
    choices: &BTreeMap<SkillName, Choice>,
) -> Plan {
    let new = by_skill(&set.entries);
    let old = current.map(|manifest| by_skill(&manifest.entries)).unwrap_or_default();
    let mut changes = Vec::new();
    for (name, files) in &new {
        match old.get(name) {
            None => changes.push(Change { name: name.clone(), kind: ChangeKind::Add }),
            Some(before) if before != files => {
                changes.push(Change { name: name.clone(), kind: ChangeKind::Update });
            }
            Some(_) => {}
        }
    }
    for name in old.keys().filter(|name| !new.contains_key(*name)) {
        changes.push(Change { name: name.clone(), kind: ChangeKind::Remove });
    }
    changes.sort_by(|a, b| a.name.cmp(&b.name));
    let clashes = new
        .keys()
        .filter(|name| local.get(&name.agent).is_some_and(|skills| skills.contains(&name.skill)))
        .map(|name| {
            let choice = choices.get(name);
            Clash {
                name: name.clone(),
                current: match choice {
                    None => ClashUse::Unpicked,
                    Some(Choice::Synced) => ClashUse::Synced,
                    Some(Choice::Local) => ClashUse::Local,
                },
                is_new: choice.is_none(),
            }
        })
        .collect();
    Plan { first_sync: current.is_none(), changes, clashes, excluded_files: set.excluded_files }
}

#[derive(Serialize, Deserialize)]
struct ChoiceRecord {
    agent: String,
    skill: String,
    choice: Choice,
}

/// A machine's Humata-managed folder (SK-4, SK-9): `<root>/.skills` points
/// at one set under `<root>/.skills-sets`.
#[derive(Debug, Clone)]
pub struct Managed {
    root: PathBuf,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn unique() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn sync_dir(dir: &Path) -> Result<(), SkillsError> {
    File::open(dir).and_then(|f| f.sync_all()).map_err(io_err("flush", dir))
}

fn sync_tree(dir: &Path) -> Result<(), SkillsError> {
    for child in fs::read_dir(dir).map_err(io_err("read the folder", dir))? {
        let child = child.map_err(io_err("read the folder", dir))?;
        let path = child.path();
        if child.file_type().map_err(io_err("inspect", &path))?.is_dir() {
            sync_tree(&path)?;
        }
    }
    sync_dir(dir)
}

fn check_path(path: &str) -> Result<(), SkillsError> {
    let bad = |reason| Err(SkillsError::BadPath { path: path.to_owned(), reason });
    if path.is_empty() || path.starts_with('/') {
        return bad("empty or absolute");
    }
    let mut count = 0;
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\0', '\\']) {
            return bad("empty, `.` or `..` component");
        }
        count += 1;
    }
    if count < 2 {
        return bad("needs an agent folder and a file");
    }
    Ok(())
}

impl Managed {
    /// Opens the managed folder at `root` (in production `~/Humata`),
    /// refusing when `<root>/.skills` is a real folder or file.
    pub fn open(root: &Path) -> Result<Self, SkillsError> {
        let managed = Self { root: root.to_path_buf() };
        managed.check_link()?;
        Ok(managed)
    }

    fn link(&self) -> PathBuf {
        self.root.join(SKILLS_LINK)
    }

    fn sets(&self) -> PathBuf {
        self.root.join(SETS_DIR)
    }

    /// The link's target set id, `None` when never synced.
    fn check_link(&self) -> Result<Option<String>, SkillsError> {
        let link = self.link();
        match fs::symlink_metadata(&link) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SkillsError::Io { action: "inspect", path: link, source }),
            Ok(meta) if !meta.file_type().is_symlink() => Err(SkillsError::ForeignSkillsPath { path: link }),
            Ok(_) => {
                let target = fs::read_link(&link).map_err(io_err("read the link", &link))?;
                let mut parts = target.components();
                let id = match (parts.next(), parts.next(), parts.next()) {
                    (
                        Some(std::path::Component::Normal(dir)),
                        Some(std::path::Component::Normal(id)),
                        None,
                    ) if dir == SETS_DIR => id.to_str().map(str::to_owned),
                    _ => None,
                };
                id.map(Some).ok_or(SkillsError::ForeignSkillsPath { path: link })
            }
        }
    }

    /// The live set's manifest; `None` when never synced.
    pub fn current_manifest(&self) -> Result<Option<SkillsManifest>, SkillsError> {
        let Some(id) = self.check_link()? else { return Ok(None) };
        let path = self.sets().join(id).join(MANIFEST_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => SkillsManifest::from_json(&text).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SkillsError::Io { action: "read", path, source }),
        }
    }

    /// Starts an incoming set in a temporary folder, first removing what
    /// earlier cut-short applies left behind (PER-27).
    pub fn begin(&self, manifest: SkillsManifest) -> Result<Incoming, SkillsError> {
        self.check_link()?;
        let mut files = BTreeMap::new();
        for entry in &manifest.entries {
            check_path(&entry.path)?;
            files.insert(entry.path.clone(), entry.clone());
        }
        self.clean_leftovers()?;
        let sets = self.sets();
        fs::create_dir_all(&sets).map_err(io_err("create", &sets))?;
        let temp = sets.join(format!("{INCOMING_PREFIX}{}", unique()));
        fs::create_dir(&temp).map_err(io_err("create", &temp))?;
        Ok(Incoming { managed: self.clone(), temp, manifest, files, written: BTreeMap::new(), done: false })
    }

    fn clean_leftovers(&self) -> Result<(), SkillsError> {
        for (dir, prefix) in [(self.sets(), INCOMING_PREFIX), (self.root.clone(), LINK_PREFIX)] {
            let Ok(listing) = fs::read_dir(&dir) else { continue };
            for child in listing.flatten() {
                if child.file_name().to_str().is_some_and(|n| n.starts_with(prefix)) {
                    let path = child.path();
                    let result = match child.file_type() {
                        Ok(kind) if kind.is_dir() => fs::remove_dir_all(&path),
                        _ => fs::remove_file(&path),
                    };
                    result.map_err(io_err("remove the leftover", &path))?;
                }
            }
        }
        Ok(())
    }

    /// The saved clash choices (SK-5); empty when none were saved.
    pub fn choices(&self) -> Result<BTreeMap<SkillName, Choice>, SkillsError> {
        let path = self.root.join(CHOICES_FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(source) => return Err(SkillsError::Io { action: "read", path, source }),
        };
        let records: Vec<ChoiceRecord> =
            serde_json::from_str(&text).map_err(|source| SkillsError::ChoicesFile { path, source })?;
        Ok(records
            .into_iter()
            .map(|r| (SkillName { agent: r.agent, skill: r.skill }, r.choice))
            .collect())
    }

    /// Whether `path` is a link Humata made into an agent's folder: a
    /// symlink pointing into this folder's `.skills`.
    pub fn owns_link(&self, path: &Path) -> bool {
        let is_link = fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
        is_link
            && fs::read_link(path).is_ok_and(|target| target.starts_with(self.absolute_link()))
    }

    /// `<root>/.skills` as an absolute path, which the agents' links hold.
    fn absolute_link(&self) -> PathBuf {
        std::path::absolute(self.link()).unwrap_or_else(|_| self.link())
    }

    /// Links each skill of the live set into its agent's own folder
    /// (`agents`, one per agent), and takes away Humata's links whose skill
    /// left the set (VF-5). A name the folder already holds as anything but
    /// Humata's link is the machine's own skill and stays (SK-4); a clash
    /// picked `local` gets no link (SK-5). An agent folder that isn't there
    /// is made only when the agent has a synced skill.
    pub fn link_agents(&self, agents: &[Source]) -> Result<(), SkillsError> {
        let manifest = self.current_manifest()?;
        let entries = manifest.as_ref().map_or(&[][..], |manifest| &manifest.entries[..]);
        let synced = by_skill(entries);
        let choices = self.choices()?;
        let base = self.absolute_link();
        for agent in agents {
            let wanted: BTreeSet<&str> = synced
                .keys()
                .filter(|name| name.agent == agent.agent)
                .filter(|name| choices.get(*name) != Some(&Choice::Local))
                .map(|name| name.skill.as_str())
                .collect();
            self.unlink_stale(&agent.root, &wanted)?;
            for skill in wanted {
                link_one(&agent.root.join(skill), &base.join(&agent.agent).join(skill), self)?;
            }
        }
        Ok(())
    }

    /// Removes Humata's links in `folder` whose skill isn't `wanted`.
    fn unlink_stale(&self, folder: &Path, wanted: &BTreeSet<&str>) -> Result<(), SkillsError> {
        let listing = match fs::read_dir(folder) {
            Ok(listing) => listing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(SkillsError::Io { action: "read the folder", path: folder.to_path_buf(), source }),
        };
        for child in listing {
            let child = child.map_err(io_err("read the folder", folder))?;
            let path = child.path();
            let keep = child.file_name().to_str().is_some_and(|name| wanted.contains(name));
            if !keep && self.owns_link(&path) {
                fs::remove_file(&path).map_err(io_err("remove Humata's link", &path))?;
            }
        }
        Ok(())
    }

    /// Saves one clash choice, atomically; it stays across syncs.
    pub fn set_choice(&self, name: &SkillName, choice: Choice) -> Result<(), SkillsError> {
        let mut all = self.choices()?;
        all.insert(name.clone(), choice);
        let records: Vec<ChoiceRecord> = all
            .into_iter()
            .map(|(n, choice)| ChoiceRecord { agent: n.agent, skill: n.skill, choice })
            .collect();
        let path = self.root.join(CHOICES_FILE);
        let temp = self.root.join(format!("{CHOICES_FILE}.new-{}", unique()));
        let text = serde_json::to_string_pretty(&records)
            .map_err(|source| SkillsError::ChoicesFile { path: path.clone(), source })?;
        fs::create_dir_all(&self.root).map_err(io_err("create", &self.root))?;
        let written = File::create(&temp).and_then(|mut f| {
            f.write_all(text.as_bytes())?;
            f.sync_all()
        });
        let result = written.and_then(|()| fs::rename(&temp, &path));
        if let Err(source) = result {
            let _ = fs::remove_file(&temp);
            return Err(SkillsError::Io { action: "write", path, source });
        }
        sync_dir(&self.root)
    }
}

/// Puts Humata's link at `slot` pointing at `target`: made when the name is
/// free, re-pointed (with one rename) when Humata's link points elsewhere,
/// and left alone when the name holds anything Humata didn't make.
fn link_one(slot: &Path, target: &Path, managed: &Managed) -> Result<(), SkillsError> {
    match fs::symlink_metadata(slot) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(folder) = slot.parent() {
                fs::create_dir_all(folder).map_err(io_err("create", folder))?;
            }
            match symlink(target, slot) {
                // Something took the name since it was looked at: it's the
                // machine's own now, so it stays.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                result => result.map_err(io_err("create the link", slot)),
            }
        }
        Err(source) => Err(SkillsError::Io { action: "inspect", path: slot.to_path_buf(), source }),
        Ok(_) if !managed.owns_link(slot) => Ok(()),
        Ok(_) if fs::read_link(slot).ok().as_deref() == Some(target) => Ok(()),
        Ok(_) => {
            let name = slot.file_name().and_then(|name| name.to_str()).unwrap_or("skill");
            let temp = slot.with_file_name(format!(".{name}{LINK_PREFIX}{}", unique()));
            symlink(target, &temp).map_err(io_err("create the link", &temp))?;
            fs::rename(&temp, slot).map_err(|source| {
                let _ = fs::remove_file(&temp);
                SkillsError::Io { action: "switch the link", path: slot.to_path_buf(), source }
            })
        }
    }
}

/// A set being received. Dropping it without `finish` leaves the current
/// set untouched and removes the partial files.
#[derive(Debug)]
pub struct Incoming {
    managed: Managed,
    temp: PathBuf,
    manifest: SkillsManifest,
    files: BTreeMap<String, SkillEntry>,
    written: BTreeMap<String, u64>,
    done: bool,
}

impl Incoming {
    /// Appends a chunk to a listed file. The path must be in the manifest
    /// and made of plain names; `offset` must be the bytes written so far.
    /// A refused chunk writes nothing.
    pub fn write(&mut self, path: &str, offset: u64, bytes: &[u8]) -> Result<(), SkillsError> {
        check_path(path)?;
        let entry = self.files.get(path).ok_or_else(|| SkillsError::NotInManifest { path: path.to_owned() })?;
        let so_far = self.written.get(path).copied().unwrap_or(0);
        if offset != so_far {
            return Err(SkillsError::OutOfOrder { path: path.to_owned(), expected: so_far, got: offset });
        }
        if so_far + bytes.len() as u64 > entry.size {
            return Err(SkillsError::TooLarge { path: path.to_owned(), size: entry.size });
        }
        let target = self.temp.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_err("create", parent))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)
            .map_err(io_err("open", &target))?;
        file.write_all(bytes).map_err(io_err("write", &target))?;
        self.written.insert(path.to_owned(), so_far + bytes.len() as u64);
        Ok(())
    }

    /// Checks every listed file (size, SHA-256), sets modes, flushes, then
    /// makes the set current with one atomic rename of the `.skills` link
    /// (SK-9), and removes sets that are no longer current. Applying the
    /// set that is already current succeeds and changes nothing.
    pub fn finish(mut self) -> Result<(), SkillsError> {
        let root = self.managed.root.clone();
        let id = self.manifest.set_id.clone();
        let sets = self.managed.sets();
        let final_dir = sets.join(&id);
        if final_dir.is_dir() {
            fs::remove_dir_all(&self.temp).map_err(io_err("remove", &self.temp))?;
        } else {
            self.verify()?;
            let json = self.manifest.to_json()?;
            let manifest_path = self.temp.join(MANIFEST_FILE);
            File::create(&manifest_path)
                .and_then(|mut f| {
                    f.write_all(json.as_bytes())?;
                    f.sync_all()
                })
                .map_err(io_err("write", &manifest_path))?;
            sync_tree(&self.temp)?;
            fs::rename(&self.temp, &final_dir).map_err(io_err("move into place", &final_dir))?;
            sync_dir(&sets)?;
        }
        self.done = true;
        self.managed.check_link()?;
        let wanted = PathBuf::from(SETS_DIR).join(&id);
        if fs::read_link(self.managed.link()).ok().as_deref() != Some(wanted.as_path()) {
            let temp_link = root.join(format!("{LINK_PREFIX}{}", unique()));
            symlink(&wanted, &temp_link).map_err(io_err("create the link", &temp_link))?;
            if let Err(source) = fs::rename(&temp_link, self.managed.link()) {
                let _ = fs::remove_file(&temp_link);
                return Err(SkillsError::Io { action: "switch", path: self.managed.link(), source });
            }
            sync_dir(&root)?;
        }
        for child in fs::read_dir(&sets).map_err(io_err("read the folder", &sets))? {
            let child = child.map_err(io_err("read the folder", &sets))?;
            if child.file_name() != id.as_str() {
                let path = child.path();
                fs::remove_dir_all(&path).map_err(io_err("remove the old set", &path))?;
            }
        }
        Ok(())
    }

    fn verify(&self) -> Result<(), SkillsError> {
        for (path, entry) in &self.files {
            let target = self.temp.join(path);
            if self.written.get(path).copied().unwrap_or(0) == 0 {
                if entry.size != 0 {
                    return Err(SkillsError::Incomplete { path: path.clone(), expected: entry.size, found: 0 });
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(io_err("create", parent))?;
                }
                File::create(&target).map_err(io_err("create", &target))?;
            }
            let mut file = File::open(&target).map_err(io_err("open", &target))?;
            let (sha, size) = hash_reader(&mut file).map_err(io_err("read", &target))?;
            if size != entry.size {
                return Err(SkillsError::Incomplete { path: path.clone(), expected: entry.size, found: size });
            }
            if sha != entry.sha256 {
                return Err(SkillsError::HashMismatch { path: path.clone() });
            }
            let mode = if entry.executable { 0o755 } else { 0o644 };
            fs::set_permissions(&target, fs::Permissions::from_mode(mode)).map_err(io_err("set the mode of", &target))?;
            file.sync_all().map_err(io_err("flush", &target))?;
        }
        Ok(())
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        if !self.done {
            let _ = fs::remove_dir_all(&self.temp);
        }
    }
}
