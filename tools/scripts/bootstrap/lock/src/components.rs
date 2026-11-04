//! What each component in `versions.toml` downloads, per platform.

use bootstrap::{Pin, Platform};

use crate::Error;
use crate::github::GitHub;

/// Go's `GOOS-GOARCH` spelling, used by Go projects' release assets.
fn go(platform: Platform) -> &'static str {
    match platform {
        Platform::DarwinArm64 => "darwin-arm64",
        Platform::DarwinX86_64 => "darwin-amd64",
        Platform::LinuxArm64 => "linux-arm64",
        Platform::LinuxX86_64 => "linux-amd64",
    }
}

/// Bazelisk publishes a bare binary per platform.
pub(crate) fn bazelisk(github: &GitHub, version: &str) -> Result<Vec<Pin>, Error> {
    let release = github.release("bazelbuild/bazelisk", &format!("v{version}"))?;
    Platform::ALL
        .iter()
        .map(|&p| {
            release.pin(github, "bazelisk", p, |name| {
                name == format!("bazelisk-{}", go(p))
            })
        })
        .collect()
}
