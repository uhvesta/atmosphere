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
mod github;

use crate::github::GitHub;

use std::env;

use bootstrap::download::Wait;
use bootstrap::{Http, Versions};

const TARGET: &str = "//tools/scripts/bootstrap/lock";

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
        "{component} {tag} has no download for {platform}; pick a version in versions.toml that supports every platform"
    )]
    MissingAsset {
        component: String,
        tag: String,
        platform: &'static str,
    },
}

fn main() {
    if let Err(error) = run() {
        bootstrap::exit_with(&error);
    }
}

fn run() -> Result<(), Error> {
    let dir = bootstrap::bootstrap_dir(&bootstrap::workspace(TARGET)?);
    let versions = Versions::read(&dir)?;

    let http = Http::default();
    let github = GitHub::new(&http, github_token());
    let mut pins = Vec::new();
    pins.extend(components::bazelisk(&github, &versions.bazelisk)?);

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
