//! Swift releases from swift.org. swift.org publishes only the Static Linux
//! SDK's checksum, so the macOS `.pkg` and the Ubuntu tarballs are hashed by
//! streaming them; nothing is written to disk.

use std::path::Path;

use bootstrap::{Http, download};
use serde::{Deserialize, Serialize};

use crate::catalog::{self, Catalog};
use crate::version::Version;
use crate::{Error, Options, sha256_each};

const RELEASES_URL: &str = "https://www.swift.org/api/v1/install/releases.json";

/// One entry of `swift_versions.json`. Fields are in alphabetical order, which
/// is the order they are written in.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct SwiftRelease {
    linux_aarch64: String,
    linux_x86_64: String,
    macos: String,
    static_sdk: String,
    static_sdk_version: String,
    xcode: String,
}

/// A release in swift.org's list, reduced to what the catalog needs.
#[derive(Deserialize)]
struct Release {
    name: String,
    #[serde(default)]
    xcode: String,
    #[serde(default)]
    platforms: Vec<Platform>,
}

#[derive(Deserialize)]
struct Platform {
    #[serde(default)]
    name: String,
    #[serde(default)]
    platform: String,
    checksum: Option<String>,
    version: Option<String>,
}

/// A release to add, with what swift.org already says about it.
#[derive(Debug, PartialEq)]
struct NewRelease {
    version: Version,
    xcode: String,
    static_sdk: String,
    static_sdk_version: String,
}

pub(crate) fn update(http: &Http, path: &Path, options: &Options) -> Result<(), Error> {
    let mut known: Catalog<SwiftRelease> = catalog::read(path, options.refresh)?;
    let releases: Vec<Release> = download::json(http, RELEASES_URL, &[])?;
    let mut failed = vec![];

    for release in new_releases(releases, &known, &options.min_swift) {
        eprintln!("hashing Swift {}", release.version);
        let hashes = sha256_each(http, &artifact_urls(&release.version));
        let [Some(macos), Some(linux_aarch64), Some(linux_x86_64)] =
            <[_; 3]>::try_from(hashes).expect("one hash per artifact")
        else {
            failed.push(release.version.to_string());
            continue;
        };
        known.insert(
            release.version,
            SwiftRelease {
                linux_aarch64,
                linux_x86_64,
                macos,
                static_sdk: release.static_sdk,
                static_sdk_version: release.static_sdk_version,
                xcode: release.xcode,
            },
        );
        // Save after every release so an interrupted run keeps its progress.
        catalog::write(path, &known)?;
    }
    catalog::write(path, &known)?;
    eprintln!("{} Swift releases in {}", known.len(), path.display());
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::Unhashed {
            what: "Swift",
            versions: failed,
        })
    }
}

/// The 6.x releases from `min` on that aren't in `known` and have every
/// artifact the toolchain needs.
fn new_releases(
    releases: Vec<Release>,
    known: &Catalog<SwiftRelease>,
    min: &Version,
) -> Vec<NewRelease> {
    releases
        .into_iter()
        .filter_map(|release| {
            let version = Version::parse(&release.name)?;
            if version.major() != 6 || version < *min || known.contains_key(&version) {
                return None;
            }
            let static_sdk = release
                .platforms
                .iter()
                .find(|p| p.platform == "static-sdk");
            let has_ubuntu = release.platforms.iter().any(|p| p.name == "Ubuntu 22.04");
            match static_sdk.and_then(|p| Some((p.checksum.clone()?, p.version.clone()?))) {
                Some((static_sdk, static_sdk_version)) if has_ubuntu => Some(NewRelease {
                    version,
                    xcode: release.xcode,
                    static_sdk,
                    static_sdk_version,
                }),
                _ => {
                    eprintln!("skipping Swift {version}: no Ubuntu 22.04 or Static SDK artifact");
                    None
                }
            }
        })
        .collect()
}

/// The macOS, Linux aarch64 and Linux x86_64 downloads of release `v`. Must
/// match the URL templates in extensions.bzl.
fn artifact_urls(v: &Version) -> [String; 3] {
    let base = format!("https://download.swift.org/swift-{v}-release");
    [
        format!("{base}/xcode/swift-{v}-RELEASE/swift-{v}-RELEASE-osx.pkg"),
        format!(
            "{base}/ubuntu2204-aarch64/swift-{v}-RELEASE/swift-{v}-RELEASE-ubuntu22.04-aarch64.tar.gz"
        ),
        format!("{base}/ubuntu2204/swift-{v}-RELEASE/swift-{v}-RELEASE-ubuntu22.04.tar.gz"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(name: &str, platforms: &[(&str, &str)]) -> Release {
        Release {
            name: name.to_owned(),
            xcode: "Xcode 26".to_owned(),
            platforms: platforms
                .iter()
                .map(|&(name, platform)| Platform {
                    name: name.to_owned(),
                    platform: platform.to_owned(),
                    checksum: (platform == "static-sdk").then(|| "abc".to_owned()),
                    version: (platform == "static-sdk").then(|| "0.0.1".to_owned()),
                })
                .collect(),
        }
    }

    const COMPLETE: &[(&str, &str)] = &[("Ubuntu 22.04", "linux"), ("Static SDK", "static-sdk")];

    #[test]
    fn keeps_only_new_complete_releases_from_the_minimum_on() {
        let mut known = Catalog::new();
        known.insert(
            Version::parse("6.2").unwrap(),
            SwiftRelease {
                linux_aarch64: String::new(),
                linux_x86_64: String::new(),
                macos: String::new(),
                static_sdk: String::new(),
                static_sdk_version: String::new(),
                xcode: String::new(),
            },
        );
        let releases = vec![
            release("5.10", COMPLETE),
            release("6.1", COMPLETE),
            release("6.2", COMPLETE),
            release("6.2.1", COMPLETE),
            release("6.3", &[("Ubuntu 22.04", "linux")]),
            release("6.4", &[("Static SDK", "static-sdk")]),
        ];
        let new = new_releases(releases, &known, &Version::parse("6.2").unwrap());
        assert_eq!(
            new,
            [NewRelease {
                version: Version::parse("6.2.1").unwrap(),
                xcode: "Xcode 26".to_owned(),
                static_sdk: "abc".to_owned(),
                static_sdk_version: "0.0.1".to_owned(),
            }]
        );
    }
}
