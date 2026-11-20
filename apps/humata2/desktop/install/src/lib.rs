//! Installs the host's own tools on every launch of the desktop app.
//!
//! The first install copies `humatad2`, `humata2_holder`, `humata` and `stack`
//! from the app bundle into `~/.humata2/bin` (atomic, so a live holder keeps
//! running its old inode: PER-6) with the release's `bundle.json` beside
//! them, registers a launchd login item for the daemon (PER-20) and links
//! `humata` into `~/.local/bin` (D51).
//!
//! Once a daemon is installed, a launch never swaps it directly: a bundle that
//! differs from the installed one is staged as verified `.<name>.next` files
//! and `.bundle.json.next` (UP-1, D281 f). The running daemon watches its bin
//! folder and automatically verifies, swaps, and restarts into the staged
//! release with two-boot rollback protection (or `migrate_old_daemon` moves
//! daemons too old to watch `.next` themselves). The same bundle changes nothing.

mod error;
mod launchd;
mod plist;

use humata2_machines::{
    canonical_manifest, stage_next, Builds, Bundle as ToolsBundle, InstalledFile, Payload, Platform,
    StageError, StagedRelease, MANIFEST,
};

pub use error::{InstallError, LaunchdError};
pub use launchd::{Launchd, SystemLaunchd};

use std::ffi::OsString;
use std::fs::{self, DirBuilder, File};
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The launchd label of the daemon's login item.
pub const AGENT_LABEL: &str = "app.humata2.humatad2";

const DAEMON: &str = "humatad2";
const HOLDER: &str = "humata2_holder";
const CLI: &str = "humata";
const STACK: &str = "stack";
/// The release's manifest, installed beside the tools; the sha256 of its
/// bytes is the bundle id.
const BUNDLE_JSON: &str = "bundle.json";
/// A release waiting for Migrate: its bundle, written last of its files.
const STAGED_BUNDLE_JSON: &str = ".bundle.json.next";

/// Where things go on this machine.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
    bin: PathBuf,
    agents: PathBuf,
    path_dir: PathBuf,
    socket: PathBuf,
    logs: PathBuf,
    /// The Linux tools Add machine installs from (TS-2): `~/.humata2/machine-tools`.
    machine_tools: PathBuf,
    path_env: Option<OsString>,
}

impl Layout {
    /// For this user: `~/.humata2/bin`, `~/Library/LaunchAgents`,
    /// `~/.local/bin`, `~/.humata2/humatad.sock`, `~/.humata2/logs`. The
    /// PATH check reads the process's `PATH`; see [`Layout::with_path`].
    pub fn for_home(home: &Path) -> Self {
        let root = home.join(".humata2");
        Self {
            bin: root.join("bin"),
            agents: home.join("Library/LaunchAgents"),
            path_dir: home.join(".local/bin"),
            socket: root.join("humatad.sock"),
            logs: root.join("logs"),
            machine_tools: root.join("machine-tools"),
            root,
            path_env: std::env::var_os("PATH"),
        }
    }

    /// Use this value instead of the process's `PATH` when deciding whether
    /// `~/.local/bin` needs a hint.
    pub fn with_path(mut self, path: Option<OsString>) -> Self {
        self.path_env = path;
        self
    }

    /// The daemon's socket, for callers that wait for it after install.
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

/// The app's own copies of the tools.
#[derive(Debug, Clone)]
pub struct Bundle {
    humatad2: PathBuf,
    holder: PathBuf,
    humata: PathBuf,
    stack: PathBuf,
    manifest: Option<PathBuf>,
    machine_tools: Option<PathBuf>,
}

impl Bundle {
    pub fn new(humatad2: PathBuf, holder: PathBuf, humata: PathBuf, stack: PathBuf) -> Self {
        Self { humatad2, holder, humata, stack, manifest: None, machine_tools: None }
    }

    /// The app's tools manifest (`manifest.json`, every platform), which
    /// names the release. Without it the release is made from the four
    /// files alone, for this platform only: what a development build has.
    pub fn with_manifest(mut self, manifest: PathBuf) -> Self {
        self.manifest = Some(manifest);
        self
    }

    /// The app's folder of Linux tools (a tools bundle: `manifest.json` and
    /// `<platform>/<file>`), which [`install_machine_tools`] copies to
    /// `~/.humata2/machine-tools`. It needs [`Bundle::with_manifest`] too: the
    /// release it belongs to is that manifest's.
    pub fn with_machine_tools(mut self, folder: PathBuf) -> Self {
        self.machine_tools = Some(folder);
        self
    }
}

/// What happened to one installed binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryChange {
    Installed,
    Updated,
    Unchanged,
}

/// What happened to the launchd agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentChange {
    /// Plist written or daemon replaced; the agent was reloaded.
    Reloaded,
    /// It was not loaded, so it was loaded.
    Loaded,
    /// Loaded and unchanged.
    Unchanged,
}

/// What an install changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub humatad2: BinaryChange,
    pub holder: BinaryChange,
    pub humata: BinaryChange,
    pub stack: BinaryChange,
    pub agent: AgentChange,
    /// Set when `~/.local/bin` is not on PATH: what to tell the person.
    pub path_hint: Option<String>,
    /// Set when `~/.local/bin/humata` already exists and is not our link
    /// (a regular file, or a link elsewhere, e.g. Humata 1's). It was left
    /// as it is (D137).
    pub path_clash: Option<PathBuf>,
}

/// The bundle id of the release staged in `layout`'s `bin`, waiting for
/// Migrate (the sha256 of `.bundle.json.next`); none when nothing is
/// staged. The app posts "Migrate the daemon on This Mac" for it.
pub fn staged_release(layout: &Layout) -> Result<Option<String>, InstallError> {
    let path = layout.bin.join(STAGED_BUNDLE_JSON);
    match fs::symlink_metadata(&path) {
        Ok(_) => sha256(&path)
            .map(|sha| Some(hex(&sha)))
            .map_err(|source| InstallError::Hash { path, source }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(InstallError::Hash { path, source }),
    }
}

/// Moves a daemon too old to hold its own Migrate item to the release staged
/// beside it (D374, D281 b). Every staged file is checked against
/// `.bundle.json.next`'s entry for this platform before anything changes;
/// then the files are renamed over the installed ones with the bundle last
/// (running holders and CLIs keep their old inode, PER-6), the folder is
/// synced, and the login item is kickstarted once so the daemon starts at the
/// new release. With nothing staged it changes nothing.
pub fn migrate_old_daemon(layout: &Layout, launchd: &dyn Launchd) -> Result<(), InstallError> {
    let staged_bundle = layout.bin.join(STAGED_BUNDLE_JSON);
    let bundle_json = match fs::read(&staged_bundle) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(InstallError::ReadStaged { path: staged_bundle, source }),
    };
    let platform = Platform::host().ok_or(InstallError::UnsupportedHost)?;
    let mut files = Vec::with_capacity(InstalledFile::ALL.len());
    for (file, name) in [
        (InstalledFile::Humatad, DAEMON),
        (InstalledFile::Holder, HOLDER),
        (InstalledFile::Cli, CLI),
        (InstalledFile::Stack, STACK),
    ] {
        let path = layout.bin.join(format!(".{name}.next"));
        let bytes = fs::read(&path).map_err(|source| InstallError::ReadStaged { path, source })?;
        files.push(Payload::new(file, bytes.into()));
    }
    StagedRelease::new(platform, bundle_json, files)
        .map_err(|source| InstallError::StagedMismatch { bin: layout.bin.clone(), source })?;

    // The daemon's files first, the bundle that names them last: a cut
    // between leaves a daemon its bundle doesn't name, which reports no release.
    for name in [DAEMON, HOLDER, CLI, STACK] {
        let from = layout.bin.join(format!(".{name}.next"));
        let to = layout.bin.join(name);
        fs::rename(&from, &to).map_err(|source| InstallError::Rename { from, to, source })?;
    }
    let to = layout.bin.join(BUNDLE_JSON);
    fs::rename(&staged_bundle, &to)
        .map_err(|source| InstallError::Rename { from: staged_bundle, to, source })?;
    File::open(&layout.bin)
        .and_then(|dir| dir.sync_all())
        .map_err(|source| InstallError::ReadStaged { path: layout.bin.clone(), source })?;

    let plist = layout.agents.join(format!("{AGENT_LABEL}.plist"));
    launchd
        .kickstart(AGENT_LABEL)
        .map_err(|source| InstallError::Launchd { plist, source })
}

/// Brings the host's tools in line with the bundle. Safe to run on every
/// launch: with no daemon installed it installs the tools and the release's
/// `bundle.json`; with one installed it leaves it, its holder and the CLIs
/// alone and, when the bundle is another release, stages it for Migrate (see
/// [`staged_release`]).
pub fn install(
    bundle: &Bundle,
    layout: &Layout,
    launchd: &dyn Launchd,
) -> Result<Installed, InstallError> {
    make_dir(&layout.root, 0o700)?;
    make_dir(&layout.bin, 0o700)?;
    make_dir(&layout.logs, 0o700)?;

    let release = release_of(bundle)?;
    let first = !layout.bin.join(DAEMON).exists();
    let (mut daemon, mut holder, mut humata, mut stack) = (
        BinaryChange::Unchanged,
        BinaryChange::Unchanged,
        BinaryChange::Unchanged,
        BinaryChange::Unchanged,
    );
    if first {
        daemon = install_binary(&bundle.humatad2, &layout.bin.join(DAEMON))?;
        holder = install_binary(&bundle.holder, &layout.bin.join(HOLDER))?;
        humata = install_binary(&bundle.humata, &layout.bin.join(CLI))?;
        stack = install_binary(&bundle.stack, &layout.bin.join(STACK))?;
        write_bundle_json(&layout.bin.join(BUNDLE_JSON), release.bundle_json())?;
    } else if !is_installed(&layout.bin, &release)? {
        stage_next(&layout.bin, &release).map_err(|source| InstallError::Release { source })?;
    }

    make_dir(&layout.agents, 0o755)?;
    let plist_path = layout.agents.join(format!("{AGENT_LABEL}.plist"));
    let plist_changed = write_if_changed(
        &plist_path,
        plist::render(AGENT_LABEL, &layout.bin.join(DAEMON), &layout.logs.join("launchd.log"))
            .as_bytes(),
    )?;

    // Restarting the daemon is safe at any time: holders outlive it (PER-5),
    // so no live session is lost; the "next safe moment" is not needed.
    let load_err = |source| InstallError::Launchd { plist: plist_path.clone(), source };
    let agent = if plist_changed || daemon != BinaryChange::Unchanged {
        launchd.bootout(AGENT_LABEL).map_err(load_err)?;
        launchd.bootstrap(&plist_path).map_err(load_err)?;
        AgentChange::Reloaded
    } else if !launchd.is_loaded(AGENT_LABEL).map_err(load_err)? {
        launchd.bootstrap(&plist_path).map_err(load_err)?;
        AgentChange::Loaded
    } else {
        AgentChange::Unchanged
    };

    let path_clash = link_cli(&layout.path_dir, &layout.bin)?;
    let on_path = layout
        .path_env
        .as_ref()
        .is_some_and(|p| std::env::split_paths(p).any(|d| d == layout.path_dir));
    let path_hint = (!on_path).then(|| {
        format!(
            "To use `humata` in a terminal, add {} to your PATH. Put this line in your shell \
             profile (for example ~/.zshrc) and open a new terminal:\n\
             export PATH=\"$HOME/.local/bin:$PATH\"",
            layout.path_dir.display()
        )
    });

    install_machine_tools(bundle, layout)?;

    Ok(Installed { humatad2: daemon, holder, humata, stack, agent, path_hint, path_clash })
}

/// Puts the app's Linux tools in `~/.humata2/machine-tools` (TS-1, TS-2), the
/// folder the host daemon sets Linux machines up from, with a `bundle.json`
/// that is the release's (the one the host's own `bin/bundle.json` has, so a
/// machine set up from them and the host name one release). Every file is
/// checked against the app's manifest first; the folder is built aside and
/// renamed into place, and nothing changes when it already holds exactly this.
/// A bundle with no machine tools, or no manifest to name their release,
/// installs nothing.
pub fn install_machine_tools(bundle: &Bundle, layout: &Layout) -> Result<BinaryChange, InstallError> {
    let (Some(from), Some(manifest)) = (&bundle.machine_tools, &bundle.manifest) else {
        return Ok(BinaryChange::Unchanged);
    };
    let tools_manifest = from.join(MANIFEST);
    let raw_tools = fs::read(&tools_manifest)
        .map_err(|source| InstallError::ReadBundle { path: tools_manifest.clone(), source })?;
    let raw_release = fs::read(manifest)
        .map_err(|source| InstallError::ReadBundle { path: manifest.clone(), source })?;
    let release = canonical_manifest(&raw_release).map_err(|source| InstallError::Release { source })?;
    let tools = ToolsBundle::open(from)
        .map_err(|source| InstallError::MachineTools { path: from.clone(), source })?;
    let mut platforms: Vec<(&'static str, Vec<Payload>)> = Vec::new();
    for platform in tools.platforms() {
        let payloads = tools
            .payloads(platform)
            .map_err(|source| InstallError::MachineTools { path: from.clone(), source })?;
        platforms.push((platform.bundle_name(), payloads));
    }

    let dest = &layout.machine_tools;
    if machine_tools_current(dest, &raw_tools, &release, &platforms)? {
        return Ok(BinaryChange::Unchanged);
    }

    let stage = layout.root.join(".machine-tools.partial");
    let old = layout.root.join(".machine-tools.old");
    remove_tree(&stage)?;
    remove_tree(&old)?;
    make_dir(&stage, 0o755)?;
    for (name, payloads) in &platforms {
        let folder = stage.join(name);
        make_dir(&folder, 0o755)?;
        for payload in payloads {
            let bytes: &[u8] = payload.bytes().as_ref();
            write_machine_file(&folder.join(payload.file().file_name()), bytes, 0o755)?;
        }
    }
    write_machine_file(&stage.join(MANIFEST), &raw_tools, 0o644)?;
    write_machine_file(&stage.join(BUNDLE_JSON), &release, 0o644)?;

    let had = match fs::rename(dest, &old) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => return Err(InstallError::Rename { from: dest.clone(), to: old, source }),
    };
    if let Err(source) = fs::rename(&stage, dest) {
        if had {
            let _ = fs::rename(&old, dest);
        }
        return Err(InstallError::Rename { from: stage, to: dest.clone(), source });
    }
    if had {
        // Best effort: a leftover is removed by the next install.
        let _ = fs::remove_dir_all(&old);
    }
    Ok(if had { BinaryChange::Updated } else { BinaryChange::Installed })
}

/// Whether `dest` already holds these manifests and exactly these files.
fn machine_tools_current(
    dest: &Path,
    tools_manifest: &[u8],
    release: &[u8],
    platforms: &[(&'static str, Vec<Payload>)],
) -> Result<bool, InstallError> {
    for (name, want) in [(MANIFEST, tools_manifest), (BUNDLE_JSON, release)] {
        let path = dest.join(name);
        match fs::read(&path) {
            Ok(found) if found[..] == *want => {}
            Ok(_) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(InstallError::Hash { path, source }),
        }
    }
    for (name, payloads) in platforms {
        for payload in payloads {
            let path = dest.join(name).join(payload.file().file_name());
            match sha256(&path) {
                Ok(sha) if hex(&sha) == payload.sha256() => {}
                Ok(_) => return Ok(false),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(source) => return Err(InstallError::Hash { path, source }),
            }
        }
    }
    Ok(true)
}

/// Deletes `path` and what is under it; nothing there is fine.
fn remove_tree(path: &Path) -> Result<(), InstallError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(InstallError::WriteMachineTools { path: path.to_path_buf(), source }),
    }
}

/// Writes a new file with `mode`, synced.
fn write_machine_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), InstallError> {
    let err = |source| InstallError::WriteMachineTools { path: path.to_path_buf(), source };
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(path).map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.set_permissions(fs::Permissions::from_mode(mode)).map_err(err)?;
    file.sync_all().map_err(err)
}

/// The release `bundle` carries, its files read and checked against its
/// manifest.
fn release_of(bundle: &Bundle) -> Result<StagedRelease, InstallError> {
    let platform = Platform::host().ok_or(InstallError::UnsupportedHost)?;
    let mut files = Vec::with_capacity(InstalledFile::ALL.len());
    for (file, path) in [
        (InstalledFile::Humatad, &bundle.humatad2),
        (InstalledFile::Holder, &bundle.holder),
        (InstalledFile::Cli, &bundle.humata),
        (InstalledFile::Stack, &bundle.stack),
    ] {
        let bytes = fs::read(path)
            .map_err(|source| InstallError::ReadBundle { path: path.clone(), source })?;
        files.push(Payload::new(file, bytes.into()));
    }
    let release = match &bundle.manifest {
        Some(manifest) => {
            let raw = fs::read(manifest)
                .map_err(|source| InstallError::ReadBundle { path: manifest.clone(), source })?;
            canonical_manifest(&raw)
                .and_then(|json| StagedRelease::new(platform, json, files))
        }
        None => StagedRelease::for_platform(platform, files),
    };
    release.map_err(|source: StageError| InstallError::Release { source })
}

/// Whether `bin` already holds exactly `release`: its `bundle.json` and all
/// four tools.
fn is_installed(bin: &Path, release: &StagedRelease) -> Result<bool, InstallError> {
    let path = bin.join(BUNDLE_JSON);
    match fs::read(&path) {
        Ok(installed) if installed[..] == release.bundle_json()[..] => {}
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(InstallError::Hash { path, source }),
    }
    for payload in release.files() {
        let path = bin.join(payload.file().file_name());
        match sha256(&path) {
            Ok(sha) if hex(&sha) == payload.sha256() => {}
            Ok(_) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(InstallError::Hash { path, source }),
        }
    }
    Ok(true)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Writes the release's manifest atomically, readable by all.
fn write_bundle_json(path: &Path, bytes: &[u8]) -> Result<(), InstallError> {
    let err = |source| InstallError::WriteBundle { path: path.to_path_buf(), source };
    let tmp = path.with_extension("json.partial");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&tmp)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.set_permissions(fs::Permissions::from_mode(0o644)).map_err(err)?;
    file.sync_all().map_err(err)?;
    drop(file);
    fs::rename(&tmp, path).map_err(err)
}

fn make_dir(path: &Path, mode: u32) -> Result<(), InstallError> {
    DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(path)
        .map_err(|source| InstallError::CreateFolder { path: path.to_path_buf(), source })
}

fn sha256(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(hasher.finalize().into())
}

fn install_binary(from: &Path, to: &Path) -> Result<BinaryChange, InstallError> {
    let bundled = sha256(from).map_err(|source| InstallError::ReadBundle {
        path: from.to_path_buf(),
        source,
    })?;
    let existed = to.exists();
    if existed {
        let installed = sha256(to)
            .map_err(|source| InstallError::Hash { path: to.to_path_buf(), source })?;
        if installed == bundled {
            return Ok(BinaryChange::Unchanged);
        }
    }

    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let partial = to.with_file_name(format!(".{name}.partial"));
    // A leftover from an interrupted install must not block this one.
    match fs::remove_file(&partial) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(InstallError::Copy { from: from.to_path_buf(), to: partial, source }),
    }
    let copy_err = |source| InstallError::Copy {
        from: from.to_path_buf(),
        to: partial.clone(),
        source,
    };
    let mut src = File::open(from).map_err(|source| InstallError::ReadBundle {
        path: from.to_path_buf(),
        source,
    })?;
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(&partial)
        .map_err(copy_err)?;
    std::io::copy(&mut src, &mut dst).map_err(copy_err)?;
    dst.set_permissions(fs::Permissions::from_mode(0o755)).map_err(copy_err)?;
    dst.sync_all().map_err(copy_err)?;
    drop(dst);
    // Atomic swap: a running process keeps the old inode (PER-6).
    fs::rename(&partial, to).map_err(|source| InstallError::Rename {
        from: partial.clone(),
        to: to.to_path_buf(),
        source,
    })?;
    Ok(if existed { BinaryChange::Updated } else { BinaryChange::Installed })
}

/// Writes `contents` atomically unless the file already holds them.
fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool, InstallError> {
    if fs::read(path).is_ok_and(|old| old == contents) {
        return Ok(false);
    }
    let err = |source| InstallError::WritePlist { path: path.to_path_buf(), source };
    let tmp = path.with_extension("plist.partial");
    let mut file = File::create(&tmp).map_err(err)?;
    file.write_all(contents).map_err(err)?;
    file.sync_all().map_err(err)?;
    drop(file);
    fs::rename(&tmp, path).map_err(err)?;
    Ok(true)
}

/// Links `path_dir/humata` to our binary. Anything there that is not ours
/// is left alone and returned (D137); only our own link, or a dangling link
/// that pointed into `bin`, is replaced.
fn link_cli(path_dir: &Path, bin: &Path) -> Result<Option<PathBuf>, InstallError> {
    let target = bin.join(CLI);
    let link = path_dir.join(CLI);
    let err = |source| InstallError::Symlink {
        path: link.clone(),
        target: target.clone(),
        source,
    };
    make_dir(path_dir, 0o755)?;
    match fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => match fs::read_link(&link) {
            Ok(t) if t == target => return Ok(None),
            Ok(t) if t.starts_with(bin) && !link.exists() => {}
            _ => return Ok(Some(link)),
        },
        Ok(_) => return Ok(Some(link)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(err(source)),
    }
    // Replace atomically: build the new link beside it, then rename over.
    let tmp = path_dir.join(".humata.partial");
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(err(source)),
    }
    symlink(&target, &tmp).map_err(err)?;
    fs::rename(&tmp, &link).map_err(err)?;
    Ok(None)
}
