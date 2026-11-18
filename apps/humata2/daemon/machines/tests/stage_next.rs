//! Staging a release on a machine over its SSH path (UP-1, D281 f): each
//! tool as `.<name>.next` and then `.bundle.json.next`, checked on the far
//! end, with nothing installed or running touched. A fake `ssh` runs the
//! commands in a home folder of the test's own; the real ~/.ssh is never
//! read.

#[path = "../testing/fake_machine.rs"]
mod fake_machine;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use fake_machine::{FakeMachine, HOST_KEY};
use humata2_machines::{
    stage_next_remote, InstalledFile, Link, MachineName, Payload, Platform, RemoteStageError, SshAccess, SshTarget,
    StagedRelease,
};

const LINUX: Platform = Platform {
    os: humata2_machines::Os::Linux,
    arch: humata2_machines::Arch::X86_64,
};

/// A link to `machine` that checks its host key against a file of the
/// test's own.
struct Reached {
    link: Link,
    _known_hosts: tempfile::TempDir,
}

fn reach() -> Reached {
    let name = MachineName::new("box-1").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let known_hosts: PathBuf = dir.path().join("known_hosts");
    std::fs::write(&known_hosts, format!("humata2.{name} {HOST_KEY}\n")).unwrap();
    let target = SshTarget {
        host: "box-1.internal".to_owned(),
        access: SshAccess::Config {
            ssh_config: "/dev/null".to_owned(),
        },
        host_key: None,
    };
    Reached {
        link: Link::ssh(name, target, &known_hosts),
        _known_hosts: dir,
    }
}

fn release(tag: &str) -> StagedRelease {
    let files = InstalledFile::ALL
        .iter()
        .map(|file| Payload::new(*file, Arc::from(format!("{file} {tag}").into_bytes())))
        .collect();
    StagedRelease::for_platform(LINUX, files).unwrap()
}

fn mode(machine: &FakeMachine, name: &str) -> u32 {
    std::fs::metadata(machine.bin().join(name)).unwrap().permissions().mode() & 0o777
}

fn sha(bytes: &[u8]) -> String {
    Payload::new(InstalledFile::Cli, Arc::from(bytes)).sha256().to_owned()
}

#[test]
fn a_release_is_staged_beside_what_is_installed_and_names_its_bundle() {
    let machine = FakeMachine::new();
    std::fs::create_dir_all(machine.bin()).unwrap();
    std::fs::write(machine.bin().join("humatad2"), b"old daemon").unwrap();
    std::fs::write(machine.bin().join("bundle.json"), b"old bundle").unwrap();
    let release = release("new");

    let id = stage_next_remote(&reach().link, &machine.runner(), &release).unwrap();

    assert_eq!(id, release.id());
    let bundle = machine.installed(".bundle.json.next").unwrap();
    assert_eq!(sha(&bundle), id);
    for file in InstalledFile::ALL {
        let staged = machine.installed(&format!(".{file}.next")).unwrap();
        assert_eq!(staged, format!("{file} new").into_bytes());
        assert_eq!(mode(&machine, &format!(".{file}.next")), 0o755);
    }
    assert_eq!(mode(&machine, ".bundle.json.next"), 0o644);
    // What is installed is untouched.
    assert_eq!(machine.installed("humatad2").as_deref(), Some(&b"old daemon"[..]));
    assert_eq!(machine.installed("bundle.json").as_deref(), Some(&b"old bundle"[..]));
}

#[test]
fn staging_again_with_another_release_replaces_the_files_whole() {
    let machine = FakeMachine::new();
    let reached = reach();
    stage_next_remote(&reached.link, &machine.runner(), &release("one")).unwrap();

    let second = release("two");
    stage_next_remote(&reached.link, &machine.runner(), &second).unwrap();

    assert_eq!(
        machine.installed(".humatad2.next").as_deref(),
        Some(&b"humatad2 two"[..])
    );
    assert_eq!(sha(&machine.installed(".bundle.json.next").unwrap()), second.id());
    let mut left: Vec<String> = std::fs::read_dir(machine.bin())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [".bundle.json.next", ".humata.next", ".humata2_holder.next", ".humatad2.next", ".stack.next"]
    );
}

#[test]
fn a_machine_that_cannot_be_reached_says_which_file_failed_and_installs_nothing() {
    let machine = FakeMachine::new();
    machine.set("down");

    let result = stage_next_remote(&reach().link, &machine.runner(), &release("new"));

    assert!(
        matches!(&result, Err(RemoteStageError::Stage { file, .. }) if file == ".humatad2.next"),
        "{result:?}"
    );
    assert_eq!(machine.installed(".humatad2.next"), None);
    assert_eq!(machine.installed(".bundle.json.next"), None);
}
