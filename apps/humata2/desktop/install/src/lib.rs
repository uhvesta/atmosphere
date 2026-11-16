//! Installs the host's own tools on every launch of the desktop app.
//!
//! Copies `humatad2`, `humata2_holder` and `humata` from the app bundle into
//! `~/.humata2/bin` (atomic swap, so a live holder keeps running its old
//! inode: PER-6), registers a launchd login item for the daemon (PER-20)
//! and links `humata` into `~/.local/bin` (D51).

mod error;
mod launchd;
mod plist;

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

/// Where things go on this machine.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
    bin: PathBuf,
    agents: PathBuf,
    path_dir: PathBuf,
    socket: PathBuf,
    logs: PathBuf,
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
}

impl Bundle {
    pub fn new(humatad2: PathBuf, holder: PathBuf, humata: PathBuf) -> Self {
        Self { humatad2, holder, humata }
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
    pub agent: AgentChange,
    /// Set when `~/.local/bin` is not on PATH: what to tell the person.
    pub path_hint: Option<String>,
    /// Set when `~/.local/bin/humata` already exists and is not our link
    /// (a regular file, or a link elsewhere, e.g. Humata 1's). It was left
    /// as it is (D137).
    pub path_clash: Option<PathBuf>,
}

/// Brings the host's tools in line with the bundle. Safe to run on every
/// launch: unchanged files are left alone.
pub fn install(
    bundle: &Bundle,
    layout: &Layout,
    launchd: &dyn Launchd,
) -> Result<Installed, InstallError> {
    make_dir(&layout.root, 0o700)?;
    make_dir(&layout.bin, 0o700)?;
    make_dir(&layout.logs, 0o700)?;

    let daemon = install_binary(&bundle.humatad2, &layout.bin.join(DAEMON))?;
    let holder = install_binary(&bundle.holder, &layout.bin.join(HOLDER))?;
    let humata = install_binary(&bundle.humata, &layout.bin.join(CLI))?;

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

    Ok(Installed { humatad2: daemon, holder, humata, agent, path_hint, path_clash })
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
