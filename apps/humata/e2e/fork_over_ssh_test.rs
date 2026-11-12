//! A session on a host reached over SSH: OpenSSH's `ssh`, as people run it,
//! to an SSH server on loopback that this test starts with its own keys.

use std::process::Command;

mod common;
mod fork;

/// Quotes `text` for the remote shell, which SSH runs the command with.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[test]
fn a_session_on_a_host_over_ssh_forks_at_an_earlier_event() {
    let scratch = common::scratch();
    let root = scratch.path();
    let keys = root.join("ssh");
    std::fs::create_dir_all(&keys).unwrap_or_else(|error| panic!("{error}"));
    let sshd = Command::new(common::binary("TEST_SSHD"))
        .arg(&keys)
        .spawn()
        .unwrap_or_else(|error| panic!("couldn't start test_sshd: {error}"));
    let _sshd = common::Running(sshd);
    let config = keys.join("ssh_config");
    common::wait_for(&config);

    let humatad = common::binary("HUMATAD");
    let remote = format!(
        "{} host --stdio --state {}",
        quoted(&humatad.display().to_string()),
        quoted(&root.join("remote").display().to_string()),
    );
    let command = vec![
        "ssh".to_owned(),
        "-F".to_owned(),
        config.display().to_string(),
        "humata-test".to_owned(),
        remote,
    ];
    fork::fork_on_remote(root, command);
}
