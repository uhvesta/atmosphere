//! Installs revup (https://github.com/Skydio/revup) into `.tools/revup` with
//! its own Python, git and GitHub CLI, then writes the `.tools/bin/revup`
//! launcher.
//!
//! ```text
//! bazel run //tools/scripts/bootstrap/revup
//! ```
//!
//! Every download is pinned by URL and SHA-256 in `versions.lock`:
//! - Python from python-build-standalone.
//! - git from GitHub Desktop's relocatable dugite-native builds.
//! - The GitHub CLI, used only to read the token of your existing gh login.
//! - revup (an upstream commit) and its pure-Python wheels.
//! - uv, which installs those wheels.
//!
//! uv only installs the downloaded files; it never contacts a package index.

use std::fs::{self, File};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use bootstrap::download;
use bootstrap::{Http, Pin, Platform, Versions};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

const TARGET: &str = "//tools/scripts/bootstrap/revup";
const COMPONENTS: [&str; 6] = ["python", "git", "gh", "uv", "revup", "revup-wheel"];
const LAUNCHER: &str = include_str!("../launcher.sh");
/// Changes to this installer rebuild the installation.
const SOURCE: &str = include_str!("main.rs");

/// Why revup could not be installed. The fields record what went wrong; the
/// message tells the user what to do. A failed build never replaces a working
/// installation.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Bootstrap(#[from] bootstrap::Error),

    #[error(
        "versions.lock has {count} {component} entries for {platform}, not one; regenerate it with `bazel run //tools/scripts/bootstrap/lock`"
    )]
    Pins {
        component: &'static str,
        platform: &'static str,
        count: usize,
    },

    #[error("could not extract {}; check free disk space and retry", archive.display())]
    Extract {
        archive: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not read the zip archive {}; delete it so it downloads again, then retry", archive.display())]
    Zip {
        archive: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },

    #[error(
        "{} has no {entry}; the archive's layout changed, so revup/src/main.rs needs updating",
        archive.display()
    )]
    MissingEntry { archive: PathBuf, entry: String },

    #[error("could not update {}; check its permissions and free disk space", path.display())]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "could not start {}; the pinned download may not run on this machine",
        program.display()
    )]
    Spawn {
        program: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("{step} failed ({status}); its output above says why")]
    Step {
        step: &'static str,
        status: ExitStatus,
    },

    #[error(
        "the checkout path {} is not UTF-8, which git config cannot store; move the checkout to a UTF-8 path",
        path.display()
    )]
    NonUtf8Path { path: PathBuf },
}

/// Attaches the path to a filesystem error.
trait At<T> {
    fn at(self, path: &Path) -> Result<T, Error>;
}

impl<T> At<T> for io::Result<T> {
    fn at(self, path: &Path) -> Result<T, Error> {
        self.map_err(|source| Error::Filesystem {
            path: path.to_owned(),
            source,
        })
    }
}

fn main() {
    if let Err(error) = run() {
        bootstrap::exit_with(&error);
    }
}

fn run() -> Result<(), Error> {
    let workspace = bootstrap::workspace(TARGET)?;
    let dir = bootstrap::bootstrap_dir(&workspace);
    let versions = Versions::read(&dir)?;
    let platform = Platform::host()?;
    let pins: Vec<Pin> = bootstrap::read_lock(&dir)?
        .into_iter()
        .filter(|pin| COMPONENTS.contains(&pin.component.as_str()) && pin.applies_to(platform))
        .collect();

    let tools = workspace.join(".tools");
    let install = tools.join("revup");
    let git = Git::new(&install);

    let stamp = stamp(&pins);
    let status = if fs::read_to_string(install.join("stamp")).is_ok_and(|current| current == stamp)
    {
        "already installed"
    } else {
        let staging = Staging::new(tools.join(".revup.staging"))?;
        build(&pins, platform, &tools.join("cache"), &staging.0)?;
        fs::write(staging.0.join("stamp"), &stamp).at(&staging.0)?;
        if install.exists() {
            fs::remove_dir_all(&install).at(&install)?;
        }
        staging.finish(&install)?;
        "installed"
    };

    let launcher = tools.join("bin/revup");
    write_executable(&launcher, LAUNCHER.as_bytes())?;

    // revup turns commit messages into PR descriptions, so start every commit
    // in this clone from the repository's What/Why/Approach template. Its
    // headings start with #, so git cuts only below the scissors line instead
    // of dropping every # line.
    let template = workspace.join(".gitmessage");
    let template = template.to_str().ok_or_else(|| Error::NonUtf8Path {
        path: workspace.clone(),
    })?;
    git.run(
        "setting this clone's commit template",
        &workspace,
        &["config", "--local", "commit.template", template],
    )?;
    git.run(
        "setting this clone's commit cleanup mode",
        &workspace,
        &["config", "--local", "commit.cleanup", "scissors"],
    )?;

    println!(
        "revup {:.12} {status} at {} (Python {}, git {}, gh {}, uv {})",
        versions.revup,
        launcher.display(),
        versions.python,
        versions.git,
        versions.gh,
        versions.uv
    );
    Ok(())
}

/// Identifies what the installation was built from: this installer and the
/// pins for this platform.
fn stamp(pins: &[Pin]) -> String {
    let mut stamp = format!("installer {}\n", download::hex(&Sha256::digest(SOURCE)));
    for pin in pins {
        stamp.push_str(&format!("{} {} {}\n", pin.component, pin.sha256, pin.url));
    }
    stamp
}

fn build(pins: &[Pin], platform: Platform, cache: &Path, root: &Path) -> Result<(), Error> {
    let only = |component: &'static str| -> Result<&Pin, Error> {
        let found: Vec<&Pin> = pins
            .iter()
            .filter(|pin| pin.component == component)
            .collect();
        match found[..] {
            [pin] => Ok(pin),
            _ => Err(Error::Pins {
                component,
                platform: platform.name(),
                count: found.len(),
            }),
        }
    };
    let http = Http::default();
    let fetch = |component| -> Result<PathBuf, Error> {
        Ok(download::fetch(&http, only(component)?, cache)?)
    };

    // The archive holds python/.
    untar(&fetch("python")?, root)?;
    untar(&fetch("git")?, &root.join("git"))?;

    // Keep only the gh binary: gh_<version>_<platform>/bin/gh.
    let gh_archive = fetch("gh")?;
    let gh = root.join("gh/gh");
    fs::create_dir_all(root.join("gh")).at(root)?;
    if gh_archive.extension().is_some_and(|ext| ext == "zip") {
        unzip_one(&gh_archive, "/bin/gh", &gh)?;
    } else {
        untar_one(&gh_archive, "/bin/gh", &gh)?;
    }

    // Keep only the uv binary: uv-<triple>/uv.
    let uv = root.join("uv/uv");
    fs::create_dir_all(root.join("uv")).at(root)?;
    untar_one(&fetch("uv")?, "/uv", &uv)?;

    let python = root.join("python/bin/python3");
    let site_packages = output(
        "asking the pinned Python for its site-packages directory",
        Command::new(&python).args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ]),
    )?;

    // revup is pure Python with no build step, so copy its package out of the
    // source archive instead of running setuptools.
    untar_subdir(
        &fetch("revup")?,
        "revup",
        &Path::new(site_packages.trim()).join("revup"),
    )?;

    let wheels = pins
        .iter()
        .filter(|pin| pin.component == "revup-wheel")
        .map(|pin| download::fetch(&http, pin, cache))
        .collect::<Result<Vec<_>, _>>()?;
    let mut install = Command::new(&uv);
    install
        .args(["pip", "install", "--quiet", "--python"])
        .arg(&python)
        // Only the downloaded wheels, without config files or UV_* variables.
        .args([
            "--no-config",
            "--offline",
            "--no-index",
            "--no-deps",
            "--no-cache",
        ])
        .args(&wheels);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("UV_") {
            install.env_remove(name);
        }
    }
    check("installing revup's dependencies with uv", &mut install)?;

    check(
        "importing revup with the pinned Python",
        Command::new(&python).args(["-I", "-c", "import aiohttp, argcomplete, revup.revup, rich"]),
    )?;
    Git::new(root).run("running the pinned git", root, &["--version"])?;
    check("running the pinned gh", Command::new(&gh).arg("--version"))?;
    check("running the pinned uv", Command::new(&uv).arg("--version"))?;
    Ok(())
}

/// The pinned git, ignoring system config like the launcher does.
struct Git(PathBuf);

impl Git {
    fn new(root: &Path) -> Self {
        Git(root.join("git"))
    }

    fn run(&self, step: &'static str, dir: &Path, args: &[&str]) -> Result<(), Error> {
        check(
            step,
            Command::new(self.0.join("bin/git"))
                .current_dir(dir)
                .env("GIT_EXEC_PATH", self.0.join("libexec/git-core"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(args),
        )
    }
}

/// A build directory that is deleted unless the build finishes.
struct Staging(PathBuf);

impl Staging {
    fn new(path: PathBuf) -> Result<Self, Error> {
        if path.exists() {
            fs::remove_dir_all(&path).at(&path)?;
        }
        fs::create_dir_all(&path).at(&path)?;
        Ok(Staging(path))
    }

    fn finish(self, to: &Path) -> Result<(), Error> {
        fs::rename(&self.0, to).at(to)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tarball(path: &Path) -> Result<tar::Archive<GzDecoder<File>>, Error> {
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(path).at(path)?));
    archive.set_preserve_permissions(true);
    Ok(archive)
}

/// Turns an extraction error into one that names the archive.
fn extracting(archive: &Path) -> impl Fn(io::Error) -> Error + '_ {
    move |source| Error::Extract {
        archive: archive.to_owned(),
        source,
    }
}

fn untar(path: &Path, into: &Path) -> Result<(), Error> {
    fs::create_dir_all(into).at(into)?;
    tarball(path)?.unpack(into).map_err(extracting(path))
}

/// Extracts the one file whose path ends with `suffix`.
fn untar_one(path: &Path, suffix: &str, to: &Path) -> Result<(), Error> {
    let fail = extracting(path);
    for entry in tarball(path)?.entries().map_err(&fail)? {
        let mut entry = entry.map_err(&fail)?;
        if entry
            .path()
            .map_err(&fail)?
            .to_string_lossy()
            .ends_with(suffix)
        {
            entry.unpack(to).map_err(&fail)?;
            return Ok(());
        }
    }
    Err(Error::MissingEntry {
        archive: path.to_owned(),
        entry: format!("*{suffix}"),
    })
}

/// Extracts `<top>/<subdir>/...` into `to`, where `<top>` is the archive's
/// single top-level directory.
fn untar_subdir(path: &Path, subdir: &str, to: &Path) -> Result<(), Error> {
    let fail = extracting(path);
    let mut found = false;
    for entry in tarball(path)?.entries().map_err(&fail)? {
        let mut entry = entry.map_err(&fail)?;
        let name = entry.path().map_err(&fail)?.into_owned();
        let Ok(rest) = name
            .iter()
            .skip(1)
            .collect::<PathBuf>()
            .strip_prefix(subdir)
            .map(Path::to_path_buf)
        else {
            continue;
        };
        found = true;
        let target = to.join(rest);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        entry.unpack(&target).map_err(&fail)?;
    }
    if !found {
        return Err(Error::MissingEntry {
            archive: path.to_owned(),
            entry: format!("*/{subdir}/"),
        });
    }
    Ok(())
}

/// Extracts the one file whose path ends with `suffix`, keeping its mode.
fn unzip_one(path: &Path, suffix: &str, to: &Path) -> Result<(), Error> {
    let zip_error = |source| Error::Zip {
        archive: path.to_owned(),
        source,
    };
    let mut archive = zip::ZipArchive::new(File::open(path).at(path)?).map_err(zip_error)?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(zip_error)?;
        if file.is_file() && file.name().map_err(zip_error)?.ends_with(suffix) {
            let mode = file.unix_mode().unwrap_or(0o755);
            io::copy(&mut file, &mut File::create(to).at(to)?).map_err(extracting(path))?;
            fs::set_permissions(to, fs::Permissions::from_mode(mode)).at(to)?;
            return Ok(());
        }
    }
    Err(Error::MissingEntry {
        archive: path.to_owned(),
        entry: format!("*{suffix}"),
    })
}

/// Replaces `path` in one step, so a running launcher is never half written.
fn write_executable(path: &Path, contents: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).at(parent)?;
    }
    let partial = path.with_extension("part");
    fs::write(&partial, contents).at(&partial)?;
    fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).at(&partial)?;
    fs::rename(&partial, path).at(path)
}

/// Runs `command`, showing only its errors.
fn check(step: &'static str, command: &mut Command) -> Result<(), Error> {
    let status = command
        .stdout(Stdio::null())
        .status()
        .map_err(|source| spawn_error(command, source))?;
    if !status.success() {
        return Err(Error::Step { step, status });
    }
    Ok(())
}

/// Runs `command` and returns what it printed, showing its errors.
fn output(step: &'static str, command: &mut Command) -> Result<String, Error> {
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .map_err(|source| spawn_error(command, source))?;
    if !output.status.success() {
        return Err(Error::Step {
            step,
            status: output.status,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn spawn_error(command: &Command, source: io::Error) -> Error {
    Error::Spawn {
        program: PathBuf::from(command.get_program()),
        source,
    }
}
