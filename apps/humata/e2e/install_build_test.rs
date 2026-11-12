//! Installing on a host of another platform than this Mac: the host gets
//! the build for its platform from the ones bundled with humatad, linked
//! into its `~/.local/bin`, and a platform with no build is refused with
//! what was there left alone. The host's platform is what a `uname` first
//! on the test sshd's PATH says. Connecting after the install fails, as a
//! Linux build can't run on this Mac, so only what's installed is checked.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use common::{Humata, Running};
use ssh_host::{Sshd, succeeds};

mod common;
mod ssh_host;

/// The test's sshd, whose commands see `platform` as `uname -sm` and
/// `home` as their home folder.
fn sshd_as(root: &Path, platform: &str, home: &Path) -> Sshd {
    let fake = root.join("fake-bin");
    std::fs::create_dir_all(&fake).unwrap_or_else(|error| panic!("{error}"));
    let uname = fake.join("uname");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"-sm\" ]; then echo '{platform}'; else exec /usr/bin/uname \"$@\"; fi\n"
    );
    std::fs::write(&uname, script).unwrap_or_else(|error| panic!("{error}"));
    std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("{error}"));
    let path = format!(
        "{}:{}",
        fake.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let keys = root.join("ssh");
    std::fs::create_dir_all(&keys).unwrap_or_else(|error| panic!("{error}"));
    let running = Command::new(common::binary("TEST_SSHD"))
        .arg(&keys)
        .env("PATH", path)
        .env("HOME", home)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start test_sshd: {error}"));
    let running = Running(running);
    let config = keys.join("ssh_config");
    common::wait_for(&config);
    Sshd { running, config }
}

fn add_host(humata: &Humata, sshd: &Sshd, folder: &Path) {
    humata.run(&[
        "host",
        "add",
        "box",
        "humata-test",
        "--ssh-config",
        &sshd.config.display().to_string(),
        "--folder",
        &folder.display().to_string(),
    ]);
}

fn starts_with(path: &Path, magic: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|bytes| bytes.starts_with(magic))
}

const ELF: &[u8] = b"\x7fELF";

#[test]
fn a_linux_host_gets_the_linux_build_linked_onto_its_path() {
    let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap_or_else(|error| panic!("{error}"));
    let sshd = sshd_as(root.path(), "Linux x86_64", &home);
    let hosts = root.path().join("hosts.json");
    let (_daemon, humata) = common::start(root.path(), &[OsStr::new("--hosts"), hosts.as_os_str()]);
    let folder = root.path().join("box");
    add_host(&humata, &sshd, &folder);

    // The connection after it fails: the build can't run here.
    let _ = succeeds(&humata, &["host", "install", "box"]);

    let installed = folder.join("bin/humatad");
    assert!(
        starts_with(&installed, ELF),
        "the host didn't get a Linux build"
    );
    let link = home.join(".local/bin/humatad");
    assert_eq!(
        std::fs::read_link(&link).ok(),
        Some(installed.clone()),
        "humatad isn't linked onto the host's PATH"
    );
}

#[test]
fn a_platform_with_no_build_is_refused_and_keeps_what_it_had() {
    let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let home = root.path().join("home");
    let mine = home.join(".local/bin/humatad");
    std::fs::create_dir_all(mine.parent().unwrap_or(&home))
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::write(&mine, b"#!/bin/sh\n").unwrap_or_else(|error| panic!("{error}"));
    let sshd = sshd_as(root.path(), "Plan9 mips", &home);
    let hosts = root.path().join("hosts.json");
    let (_daemon, humata) = common::start(root.path(), &[OsStr::new("--hosts"), hosts.as_os_str()]);
    let folder = root.path().join("box");
    add_host(&humata, &sshd, &folder);

    assert!(!succeeds(&humata, &["host", "install", "box"]));
    assert!(!folder.join("bin/humatad").exists());
    assert_eq!(std::fs::read(&mine).ok(), Some(b"#!/bin/sh\n".to_vec()));
}

#[test]
fn a_link_never_replaces_a_humatad_installed_some_other_way() {
    let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let home = root.path().join("home");
    let mine = home.join(".local/bin/humatad");
    std::fs::create_dir_all(mine.parent().unwrap_or(&home))
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::write(&mine, b"#!/bin/sh\n").unwrap_or_else(|error| panic!("{error}"));
    let sshd = sshd_as(root.path(), "Linux aarch64", &home);
    let hosts = root.path().join("hosts.json");
    let (_daemon, humata) = common::start(root.path(), &[OsStr::new("--hosts"), hosts.as_os_str()]);
    let folder = root.path().join("box");
    add_host(&humata, &sshd, &folder);

    // The connection after it fails: the build can't run here.
    let _ = succeeds(&humata, &["host", "install", "box"]);

    assert!(starts_with(&folder.join("bin/humatad"), ELF));
    assert_eq!(std::fs::read(&mine).ok(), Some(b"#!/bin/sh\n".to_vec()));
}
