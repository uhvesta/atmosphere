//! Resolves `tools/bootstrap/versions.toml` into `versions.lock`.
//!
//! ```text
//! bazel run //tools/bootstrap/lock
//! ```
//!
//! `versions.toml` holds only versions. This tool finds every download they
//! imply, one per platform, and records its URL and SHA-256 in `versions.lock`,
//! which the bootstrap scripts read. The scripts never resolve anything
//! themselves, so an install is the same on every machine until the lock
//! changes.
//!
//! GitHub publishes a SHA-256 for each release asset, so most hashes come from
//! the API without a download. revup's Python dependencies are resolved by the
//! pinned uv, run with the pinned Python, so this tool runs only on the
//! platforms it pins. Set `GH_TOKEN` or `GITHUB_TOKEN` to raise the GitHub API
//! rate limit.

mod components;
mod github;
mod uv;

use crate::github::GitHub;

use std::path::PathBuf;
use std::process::ExitStatus;
use std::{env, io};

use bootstrap::download::Wait;
use bootstrap::{Http, Versions};

use crate::uv::Uv;

const TARGET: &str = "//tools/bootstrap/lock";
const PBS_RELEASES: &str = "https://github.com/astral-sh/python-build-standalone/releases";
const DUGITE_RELEASES: &str = "https://github.com/desktop/dugite-native/releases";

/// Why `versions.toml` could not be resolved. The fields record what went
/// wrong; the message tells the user what to change.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Bootstrap(#[from] bootstrap::Error),

    #[error(
        "GitHub is rate-limiting anonymous API requests from this machine ({url}); wait {wait}, or raise the limit with a token: `GH_TOKEN=$(gh auth token) bazel run {TARGET}`"
    )]
    AnonymousRateLimit { url: String, wait: Wait },

    #[error(
        "no recent python-build-standalone release has Python {version} for every platform; pick a version listed at {PBS_RELEASES}"
    )]
    NoPythonBuild { version: String },

    #[error(
        "dugite-native has no release of git {version}; pick a version listed at {DUGITE_RELEASES}"
    )]
    NoGitBuild { version: String },

    #[error(
        "{component} {tag} has no download for {platform}; pick a version in versions.toml that supports every platform"
    )]
    MissingAsset {
        component: String,
        tag: String,
        platform: &'static str,
    },

    #[error(
        "{component} {tag} has several downloads that could be for {platform} ({names}); its file naming changed, so update the matching in lock/src/components.rs"
    )]
    AmbiguousAsset {
        component: String,
        tag: String,
        platform: &'static str,
        names: String,
    },

    #[error(
        "revup = \"{value}\" in versions.toml must be a full 40-character commit SHA from https://github.com/Skydio/revup/commits"
    )]
    RevupCommit { value: String },

    #[error(
        "could not unpack the archive from {url}; check free disk space, then retry, since the download may be corrupted"
    )]
    CorruptArchive {
        url: String,
        #[source]
        source: io::Error,
    },

    #[error(
        "revup's source has no {path}, where this tool reads its dependencies; pick another revup commit or update lock/src/components.rs"
    )]
    MissingSetupCfg { path: String },

    #[error(
        "versions.lock has no {component} for this machine ({platform}), which resolving revup's dependencies needs; pick a {component} version in versions.toml that supports every platform"
    )]
    NoHostPin {
        component: &'static str,
        platform: &'static str,
    },

    #[error(
        "could not run the pinned uv at {}; delete .tools/cache so it is unpacked again, then retry",
        path.display()
    )]
    RunUv {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "uv could not resolve revup's Python dependencies for {platform} ({status}); its output above says why. Try another revup commit or python version in versions.toml"
    )]
    Resolve {
        platform: &'static str,
        status: ExitStatus,
    },

    #[error(
        "uv's pylock.toml output is not what this tool expects; uv's format may have changed, so lock/src/uv.rs needs updating"
    )]
    UnexpectedUvOutput {
        #[source]
        source: toml::de::Error,
    },

    #[error(
        "{package} {version} has no pure-Python wheel, which the revup installer needs; try another revup commit or python version in versions.toml"
    )]
    NoPureWheel { package: String, version: String },
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

    let http = Http::default();
    let github = GitHub::new(&http, github_token());
    let mut pins = Vec::new();
    pins.extend(components::bazelisk(&github, &versions.bazelisk)?);
    pins.extend(components::python(&github, &versions.python)?);
    pins.extend(components::git(&github, &versions.git)?);
    pins.extend(components::gh(&github, &versions.gh)?);
    pins.extend(components::uv(&github, &versions.uv)?);
    let uv = Uv::new(&http, &pins, &workspace.join(".tools/cache"))?;
    pins.extend(components::revup(
        &http,
        &uv,
        &versions.revup,
        &versions.python,
    )?);

    let path = bootstrap::write_lock(&dir, &pins)?;
    println!("Pinned {} files in {}", pins.len(), path.display());
    Ok(())
}

/// A token raises GitHub's API rate limit; every request works without one.
fn github_token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| env::var(name).ok().filter(|token| !token.is_empty()))
}
