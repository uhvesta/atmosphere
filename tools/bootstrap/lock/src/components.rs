//! What each component in `versions.toml` downloads, per platform.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use bootstrap::download;
use bootstrap::{Http, Pin, Platform};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::github::{GitHub, Release};
use crate::uv::{Uv, Wheel};

/// Go's `GOOS-GOARCH` spelling, used by Go projects' release assets.
fn go(platform: Platform) -> &'static str {
    match platform {
        Platform::DarwinArm64 => "darwin-arm64",
        Platform::DarwinX86_64 => "darwin-amd64",
        Platform::LinuxArm64 => "linux-arm64",
        Platform::LinuxX86_64 => "linux-amd64",
    }
}

/// The target triple, used by Rust projects' release assets and by uv.
pub(crate) fn triple(platform: Platform) -> &'static str {
    match platform {
        Platform::DarwinArm64 => "aarch64-apple-darwin",
        Platform::DarwinX86_64 => "x86_64-apple-darwin",
        Platform::LinuxArm64 => "aarch64-unknown-linux-gnu",
        Platform::LinuxX86_64 => "x86_64-unknown-linux-gnu",
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

/// python-build-standalone republishes each Python version in several dated
/// releases; the newest one that has it for every platform wins.
pub(crate) fn python(github: &GitHub, version: &str) -> Result<Vec<Pin>, Error> {
    let asset = |tag: &str, p: Platform| {
        format!(
            "cpython-{version}+{tag}-{}-install_only_stripped.tar.gz",
            triple(p)
        )
    };
    // Each release lists about a thousand assets, so read a few at a time.
    for page in 1..=10 {
        let releases: Vec<Release> = github.api(&format!(
            "repos/astral-sh/python-build-standalone/releases?per_page=4&page={page}"
        ))?;
        if releases.is_empty() {
            break;
        }
        for release in releases.iter().filter(|r| !r.draft && !r.prerelease) {
            let has = |p| {
                release
                    .assets
                    .iter()
                    .any(|a| a.name == asset(&release.tag_name, p))
            };
            if Platform::ALL.into_iter().all(has) {
                return Platform::ALL
                    .iter()
                    .map(|&p| {
                        release.pin(github, "python", p, |name| {
                            name == asset(&release.tag_name, p)
                        })
                    })
                    .collect();
            }
        }
    }
    Err(Error::NoPythonBuild {
        version: version.to_owned(),
    })
}

/// GitHub Desktop's relocatable git. Rebuilds of one git version are tagged
/// `v2.56.0`, `v2.56.0-1`, ...; the highest wins. Asset names carry a build id.
pub(crate) fn git(github: &GitHub, version: &str) -> Result<Vec<Pin>, Error> {
    let releases: Vec<Release> = github.api("repos/desktop/dugite-native/releases?per_page=100")?;
    let tag = format!("v{version}");
    let rebuild = |release: &Release| -> Option<u32> {
        let rest = release.tag_name.strip_prefix(&tag)?;
        if rest.is_empty() {
            Some(0)
        } else {
            rest.strip_prefix('-')?.parse().ok()
        }
    };
    let release = releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| Some((rebuild(r)?, r)))
        .max_by_key(|(rebuild, _)| *rebuild)
        .map(|(_, release)| release)
        .ok_or_else(|| Error::NoGitBuild {
            version: version.to_owned(),
        })?;
    let prefix = format!("dugite-native-v{version}-");
    Platform::ALL
        .iter()
        .map(|&p| {
            let suffix = match p {
                Platform::DarwinArm64 => "-macOS-arm64.tar.gz",
                Platform::DarwinX86_64 => "-macOS-x64.tar.gz",
                Platform::LinuxArm64 => "-ubuntu-arm64.tar.gz",
                Platform::LinuxX86_64 => "-ubuntu-x64.tar.gz",
            };
            release.pin(github, "git", p, |name| {
                name.starts_with(&prefix) && name.ends_with(suffix)
            })
        })
        .collect()
}

/// The GitHub CLI: a zip on macOS, a tarball on Linux.
pub(crate) fn gh(github: &GitHub, version: &str) -> Result<Vec<Pin>, Error> {
    let release = github.release("cli/cli", &format!("v{version}"))?;
    Platform::ALL
        .iter()
        .map(|&p| {
            let asset = match p {
                Platform::DarwinArm64 => "macOS_arm64.zip",
                Platform::DarwinX86_64 => "macOS_amd64.zip",
                Platform::LinuxArm64 => "linux_arm64.tar.gz",
                Platform::LinuxX86_64 => "linux_amd64.tar.gz",
            };
            release.pin(github, "gh", p, |name| {
                name == format!("gh_{version}_{asset}")
            })
        })
        .collect()
}

/// uv publishes a tarball per target triple.
pub(crate) fn uv(github: &GitHub, version: &str) -> Result<Vec<Pin>, Error> {
    let release = github.release("astral-sh/uv", version)?;
    Platform::ALL
        .iter()
        .map(|&p| {
            release.pin(github, "uv", p, |name| {
                name == format!("uv-{}.tar.gz", triple(p))
            })
        })
        .collect()
}

/// revup at an upstream commit, plus the pure-Python wheels uv resolves for
/// its dependencies under the pinned Python. Wheels every platform needs are
/// pinned as `any`.
pub(crate) fn revup(http: &Http, uv: &Uv, commit: &str, python: &str) -> Result<Vec<Pin>, Error> {
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::RevupCommit {
            value: commit.to_owned(),
        });
    }
    let url = format!("https://github.com/Skydio/revup/archive/{commit}.tar.gz");
    let archive = download::bytes(http, &url)?;
    let mut pins = vec![Pin::new(
        "revup",
        "any",
        download::hex(&Sha256::digest(&archive)),
        url.clone(),
    )];

    let setup_cfg = read_from_tarball(&archive, &url, &format!("revup-{commit}/setup.cfg"))?;
    let requirements = install_requires(&setup_cfg)?;

    let mut wheels: BTreeMap<String, (Wheel, BTreeSet<Platform>)> = BTreeMap::new();
    for platform in Platform::ALL {
        for wheel in uv.resolve(&requirements, python, platform)? {
            // Keyed by file name, so the lock lists wheels by package.
            let file = wheel
                .url
                .rsplit('/')
                .next()
                .unwrap_or(&wheel.url)
                .to_owned();
            wheels
                .entry(file)
                .or_insert_with(|| (wheel, BTreeSet::new()))
                .1
                .insert(platform);
        }
    }
    for (wheel, platforms) in wheels.into_values() {
        let names: Vec<&'static str> = if platforms.len() == Platform::ALL.len() {
            vec!["any"]
        } else {
            platforms.iter().map(|p| p.name()).collect()
        };
        for platform in names {
            pins.push(Pin::new(
                "revup-wheel",
                platform,
                wheel.sha256.clone(),
                wheel.url.clone(),
            ));
        }
    }
    Ok(pins)
}

/// The requirements in `[options] install_requires` of a setup.cfg.
fn install_requires(setup_cfg: &str) -> Result<Vec<String>, Error> {
    let mut section = "";
    let mut in_key = false;
    let mut requirements = Vec::new();
    for line in setup_cfg.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name;
            in_key = false;
            continue;
        }
        if section != "options" || trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            let (key, value) = line.split_once('=').unwrap_or((line, ""));
            in_key = key.trim() == "install_requires";
            if in_key && !value.trim().is_empty() {
                requirements.push(value.trim().to_owned());
            }
        } else if in_key {
            requirements.push(trimmed.to_owned());
        }
    }
    if requirements.is_empty() {
        return Err(Error::MissingSetupCfg {
            path: "setup.cfg [options] install_requires".to_owned(),
        });
    }
    Ok(requirements)
}

fn read_from_tarball(archive: &[u8], url: &str, path: &str) -> Result<String, Error> {
    let corrupt = |source| Error::CorruptArchive {
        url: url.to_owned(),
        source,
    };
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(corrupt)? {
        let mut entry = entry.map_err(corrupt)?;
        if entry.path().map_err(corrupt)?.to_str() == Some(path) {
            let mut contents = String::new();
            entry.read_to_string(&mut contents).map_err(corrupt)?;
            return Ok(contents);
        }
    }
    Err(Error::MissingSetupCfg {
        path: path.to_owned(),
    })
}
