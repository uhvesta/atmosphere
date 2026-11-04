//! Shared by the bootstrap tools and the Swift catalog tool: `versions.toml`,
//! `versions.lock`, the supported platforms, and downloads.

pub mod download;
mod error;
mod http;
mod lockfile;
mod platform;
mod versions;

use std::fs;
use std::path::{Path, PathBuf};

pub use error::{Error, exit_with, print_error};
pub use http::Http;
pub use lockfile::{Pin, read_lock, write_lock};
pub use platform::Platform;
pub use versions::Versions;

const BOOTSTRAP_DIR: &str = "tools/bootstrap";

/// The repository root, from `bazel run`. `target` is the label to suggest
/// when the tool runs some other way.
pub fn workspace(target: &'static str) -> Result<PathBuf, Error> {
    std::env::var_os("BUILD_WORKSPACE_DIRECTORY")
        .map(PathBuf::from)
        .ok_or(Error::NotUnderBazel { target })
}

/// `tools/bootstrap`, which holds `versions.toml` and `versions.lock`.
pub fn bootstrap_dir(workspace: &Path) -> PathBuf {
    workspace.join(BOOTSTRAP_DIR)
}

fn read_to_string(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_owned(),
        source,
    })
}
