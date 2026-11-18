//! The host's stack settings on a box (D411): `config/stack.toml` in the
//! host's humata home goes to `~/.humata2/config/stack.toml` on every
//! linked machine, with anything that looks like a secret left out, checked
//! by size and SHA-256 before it replaces what the box has. The `stack`
//! CLI itself goes with the other tools. A host with no such file leaves a
//! box alone.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use remote_rig::{LONG, Rig};
use sha2::{Digest, Sha256};

const BOX: &str = "box-1";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where the box keeps its copy.
fn on_box(rig: &Rig) -> PathBuf {
    rig.the(BOX).home().join(".humata2/config/stack.toml")
}

/// Waits until the file at `path` holds exactly `want`. The box is written
/// by a send the host starts when a link comes up, which has no event of
/// its own to wait on.
async fn holds(path: &Path, want: &[u8]) {
    let deadline = Instant::now() + LONG;
    loop {
        let now = std::fs::read(path).unwrap_or_default();
        if now == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} never held {:?}; it holds {:?}",
            path.display(),
            String::from_utf8_lossy(want),
            String::from_utf8_lossy(&now)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_gets_the_stack_cli_and_the_hosts_settings_without_their_secrets() {
    let rig = Rig::start_with_stack_settings(
        &[BOX],
        &[],
        "prefix = \"alice\"\ngh_token = \"ghp_abc\"\npin_revisions = false\n",
    )
    .await;

    let stack = rig.the(BOX).home().join(".humata2/bin/stack");
    let installed = std::fs::read(&stack).expect("the stack CLI is installed on the box");
    assert_eq!(sha256_hex(&installed), rig.manifest_sha("stack"));

    let settings = std::fs::read(on_box(&rig)).expect("the host's stack settings are on the box");
    let text = String::from_utf8(settings.clone()).expect("the settings are text");
    assert_eq!(sha256_hex(&settings), sha256_hex(b"prefix = \"alice\"\npin_revisions = false\n"));
    assert!(text.contains("prefix") && text.contains("pin_revisions"), "{text}");
    assert!(!text.contains("gh_token") && !text.contains("ghp_"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_host_file_reaches_a_box_when_its_link_comes_back() {
    let rig = Rig::start_with_stack_settings(&[BOX], &[], "prefix = \"alice\"\n").await;
    assert_eq!(std::fs::read(on_box(&rig)).unwrap(), b"prefix = \"alice\"\n");

    rig.link_lost(BOX).await;
    rig.set_host_stack_settings("prefix = \"bob\"\ngh_token = \"ghp_def\"\n");
    rig.link_back(BOX).await;

    holds(&on_box(&rig), b"prefix = \"bob\"\n").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_with_no_stack_settings_sends_a_box_nothing() {
    let rig = Rig::start(&[BOX], &[]).await;

    assert!(!rig.host_stack_settings().exists());
    assert!(!on_box(&rig).exists(), "the box was given settings the host doesn't have");
    assert!(rig.the(BOX).home().join(".humata2/bin/stack").is_file());
}
