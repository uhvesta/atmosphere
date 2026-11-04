//! Resolves Python requirements into pinned wheels with the pinned uv.
//!
//! uv picks the versions; this module only turns its answer into pins. uv
//! needs an interpreter even to resolve for another platform, so it runs with
//! the pinned Python for this machine.

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bootstrap::download;
use bootstrap::{Http, Pin, Platform};
use flate2::read::GzDecoder;
use serde::Deserialize;

use crate::Error;
use crate::components::triple;

/// The pinned uv and Python for this machine, unpacked into the cache.
pub(crate) struct Uv {
    uv: PathBuf,
    python: PathBuf,
    cache: PathBuf,
}

/// A wheel that one platform installs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Wheel {
    pub(crate) url: String,
    pub(crate) sha256: String,
}

impl Uv {
    /// Downloads and unpacks the `uv` and `python` pins for this machine.
    pub(crate) fn new(http: &Http, pins: &[Pin], cache: &Path) -> Result<Self, Error> {
        let host = Platform::host()?;
        let pin = |component: &'static str| {
            pins.iter()
                .find(|pin| pin.component == component && pin.applies_to(host))
                .ok_or(Error::NoHostPin {
                    component,
                    platform: host.name(),
                })
        };
        let uv = unpacked(http, pin("uv")?, cache)?.join(format!("uv-{}/uv", triple(host)));
        let python = unpacked(http, pin("python")?, cache)?.join("python/bin/python3");
        Ok(Uv {
            uv,
            python,
            cache: cache.join("uv"),
        })
    }

    /// The wheel of each package that `requirements` need on `platform`
    /// under CPython `python`.
    pub(crate) fn resolve(
        &self,
        requirements: &[String],
        python: &str,
        platform: Platform,
    ) -> Result<Vec<Wheel>, Error> {
        let mut command = Command::new(&self.uv);
        command
            .args(["pip", "compile", "-", "--quiet", "--no-header"])
            .args(["--format", "pylock.toml"])
            // Ignore uv config files and UV_* variables, so every machine
            // resolves against PyPI the same way.
            .args(["--no-config", "--no-python-downloads"])
            .args(["--default-index", "https://pypi.org/simple"])
            .args(["--only-binary", ":all:"])
            .arg("--python")
            .arg(&self.python)
            .args(["--python-version", python])
            .args(["--python-platform", triple(platform)])
            .env("UV_CACHE_DIR", &self.cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        for (name, _) in env::vars_os() {
            if name.to_string_lossy().starts_with("UV_") && name != "UV_CACHE_DIR" {
                command.env_remove(name);
            }
        }
        let spawn_error = |source| Error::RunUv {
            path: self.uv.clone(),
            source,
        };
        let mut child = command.spawn().map_err(spawn_error)?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(requirements.join("\n").as_bytes())
                .map_err(spawn_error)?;
        }
        let output = child.wait_with_output().map_err(spawn_error)?;
        if !output.status.success() {
            return Err(Error::Resolve {
                platform: platform.name(),
                status: output.status,
            });
        }
        pure_wheels(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Unpacks `pin` once into the cache, next to the download, and returns the
/// directory it is unpacked in.
fn unpacked(http: &Http, pin: &Pin, cache: &Path) -> Result<PathBuf, Error> {
    let archive = download::fetch(http, pin, cache)?;
    let dir = archive.with_file_name("unpacked");
    if dir.is_dir() {
        return Ok(dir);
    }
    let partial = archive.with_file_name("unpacked.part");
    let write_error = |path: &Path| {
        let path = path.to_owned();
        move |source| bootstrap::Error::Write { path, source }
    };
    if partial.exists() {
        fs::remove_dir_all(&partial).map_err(write_error(&partial))?;
    }
    let file = File::open(&archive).map_err(|source| bootstrap::Error::Read {
        path: archive.clone(),
        source,
    })?;
    tar::Archive::new(GzDecoder::new(file))
        .unpack(&partial)
        .map_err(|source| Error::CorruptArchive {
            url: pin.url.clone(),
            source,
        })?;
    fs::rename(&partial, &dir).map_err(write_error(&dir))?;
    Ok(dir)
}

/// uv's `pylock.toml` output, reduced to what pinning needs.
#[derive(Deserialize)]
struct Lock {
    #[serde(default)]
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    #[serde(default)]
    wheels: Vec<LockedFile>,
}

#[derive(Deserialize)]
struct LockedFile {
    url: String,
    hashes: Hashes,
}

#[derive(Deserialize)]
struct Hashes {
    sha256: String,
}

/// The pure-Python wheel of each package in a `pylock.toml`. The installer
/// installs only those, so one file serves every platform.
fn pure_wheels(pylock: &str) -> Result<Vec<Wheel>, Error> {
    let lock: Lock =
        toml::from_str(pylock).map_err(|source| Error::UnexpectedUvOutput { source })?;
    lock.packages
        .into_iter()
        .map(|package| {
            package
                .wheels
                .into_iter()
                .find(|wheel| is_pure_py3_wheel(wheel.url.rsplit('/').next().unwrap_or("")))
                .map(|wheel| Wheel {
                    url: wheel.url,
                    sha256: wheel.hashes.sha256,
                })
                .ok_or(Error::NoPureWheel {
                    package: package.name,
                    version: package.version,
                })
        })
        .collect()
}

/// `name-version[-build]-python-abi-platform.whl` with a `py3` python tag and
/// no ABI or platform.
fn is_pure_py3_wheel(filename: &str) -> bool {
    let Some(stem) = filename.strip_suffix(".whl") else {
        return false;
    };
    let mut tags = stem.rsplitn(4, '-');
    let (Some(platform), Some(abi), Some(python)) = (tags.next(), tags.next(), tags.next()) else {
        return false;
    };
    platform == "any" && abi == "none" && python.split('.').any(|tag| tag == "py3")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_pure_wheel_of_each_package() {
        let pylock = r#"
lock-version = "1.0"
created-by = "uv"

[[packages]]
name = "aiohttp"
version = "3.14.5"
wheels = [
    { url = "https://files.example/aiohttp-3.14.5-cp314-cp314-macosx_11_0_arm64.whl", hashes = { sha256 = "aa" } },
    { url = "https://files.example/aiohttp-3.14.5-py3-none-any.whl", hashes = { sha256 = "bb" } },
]

[[packages]]
name = "six"
version = "1.17.0"
wheels = [{ url = "https://files.example/six-1.17.0-py2.py3-none-any.whl", hashes = { sha256 = "cc" } }]
"#;
        assert_eq!(
            pure_wheels(pylock).unwrap(),
            [
                Wheel {
                    url: "https://files.example/aiohttp-3.14.5-py3-none-any.whl".to_owned(),
                    sha256: "bb".to_owned(),
                },
                Wheel {
                    url: "https://files.example/six-1.17.0-py2.py3-none-any.whl".to_owned(),
                    sha256: "cc".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn a_package_without_a_pure_wheel_fails() {
        let pylock = r#"
[[packages]]
name = "native"
version = "1.0"
wheels = [{ url = "https://files.example/native-1.0-cp314-cp314-manylinux_2_28_x86_64.whl", hashes = { sha256 = "aa" } }]
"#;
        assert!(matches!(
            pure_wheels(pylock),
            Err(Error::NoPureWheel { package, version }) if package == "native" && version == "1.0"
        ));
    }

    #[test]
    fn python2_only_and_abi_specific_wheels_are_not_pure() {
        assert!(!is_pure_py3_wheel("a-1.0-py2-none-any.whl"));
        assert!(!is_pure_py3_wheel("a-1.0-py3-abi3-any.whl"));
        assert!(!is_pure_py3_wheel("a-1.0.tar.gz"));
        assert!(is_pure_py3_wheel("a-1.0-1-py3-none-any.whl"));
    }
}
