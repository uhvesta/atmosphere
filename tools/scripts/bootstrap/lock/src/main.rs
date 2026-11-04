//! Resolves `tools/scripts/bootstrap/versions.toml` into `versions.lock`.
//!
//! ```text
//! bazel run //tools/scripts/bootstrap/lock
//! ```
//!
//! `versions.toml` holds only versions. This tool finds every download they
//! imply, one per platform, and records its URL and SHA-256 in `versions.lock`,
//! which the bootstrap scripts read. The scripts never resolve anything
//! themselves, so an install is the same on every machine until the lock
//! changes.
//!
//! GitHub publishes a SHA-256 for each release asset, so most hashes come from
//! the API without a download. Set `GH_TOKEN` or `GITHUB_TOKEN` to raise the
//! API rate limit.

mod components;
mod download;
mod github;
mod http;
mod lockfile;
mod platform;

use std::io;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

use serde::Deserialize;

use crate::download::Wait;
use crate::github::GitHub;
use crate::http::Http;

const TARGET: &str = "//tools/scripts/bootstrap/lock";
const BOOTSTRAP_DIR: &str = "tools/scripts/bootstrap";

/// Why `versions.toml` could not be resolved. The fields record what went
/// wrong; the message tells the user what to do about it.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("this tool needs the checkout's location from Bazel; run it with `bazel run {TARGET}`")]
    NotUnderBazel,

    #[error("could not read {}; check that it exists and is readable", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not write {}; check its permissions and free disk space", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "{} is invalid; it needs exactly the key bazelisk, a quoted version string",
        path.display()
    )]
    InvalidVersions {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error(
        "the server is rate-limiting requests from this machine ({url}); wait {wait} before retrying, since retrying sooner can extend the limit"
    )]
    RateLimited { url: String, wait: Wait },

    #[error(
        "GitHub is rate-limiting anonymous API requests from this machine ({url}); wait {wait}, or raise the limit with a token: `GH_TOKEN=$(gh auth token) bazel run {TARGET}`"
    )]
    AnonymousRateLimit { url: String, wait: Wait },

    #[error("{url} does not exist; check the versions in versions.toml")]
    NotFound { url: String },

    #[error("{url} answered HTTP {status}; retry later, and check the URL if it keeps failing")]
    Status { url: String, status: u16 },

    #[error("could not download {url}; check your network connection and retry")]
    Download {
        url: String,
        #[source]
        source: ureq::Error,
    },

    #[error("the download of {url} was cut off; check your network connection and retry")]
    Interrupted {
        url: String,
        #[source]
        source: io::Error,
    },

    #[error(
        "{url} answered with data this tool does not understand; the service may have changed its API, so the tool needs updating"
    )]
    UnexpectedResponse {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "{component} {tag} has no download for {platform}; pick a version in versions.toml that supports every platform"
    )]
    MissingAsset {
        component: String,
        tag: String,
        platform: &'static str,
    },
}

/// The versions to pin. Every field is required, so a typo fails loudly.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Versions {
    bazelisk: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            eprintln!("  caused by: {cause}");
            source = cause.source();
        }
        process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let workspace = env::var_os("BUILD_WORKSPACE_DIRECTORY").ok_or(Error::NotUnderBazel)?;
    let dir = PathBuf::from(workspace).join(BOOTSTRAP_DIR);
    let versions = read_versions(&dir.join("versions.toml"))?;

    let http = Http::default();
    let github = GitHub::new(&http, github_token());
    let mut pins = Vec::new();
    pins.extend(components::bazelisk(&github, &versions.bazelisk)?);

    let path = lockfile::write_lock(&dir, &pins)?;
    println!("Pinned {} files in {}", pins.len(), path.display());
    Ok(())
}

fn read_versions(path: &Path) -> Result<Versions, Error> {
    let text = fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| Error::InvalidVersions {
        path: path.to_owned(),
        source,
    })
}

/// A token raises GitHub's API rate limit; every request works without one.
fn github_token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| env::var(name).ok().filter(|token| !token.is_empty()))
}
