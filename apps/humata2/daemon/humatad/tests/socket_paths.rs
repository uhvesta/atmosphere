//! Holder sockets live in the state folder, so a long `--state` path can
//! push them past the length Unix sockets allow. A daemon whose state
//! folder would do that is refused at start, and an ordinary one still
//! runs a terminal.

use humatad_test_common as common;

use std::os::unix::fs::PermissionsExt;

use common::{Attach, Daemon, open, serve_and_exit};
use humata2_grpc::humata::v1 as pb;
use tempfile::TempDir;

/// A folder in /tmp, private to this user, whose name is `padding` bytes
/// longer than a plain one's.
fn padded_folder(padding: usize) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix(&format!("h2d{}", "p".repeat(padding)))
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn a_state_folder_whose_holder_sockets_would_be_too_long_is_refused_at_start() {
    let dir = padded_folder(70);
    let state = dir.path().join("state");

    let (status, stderr) = serve_and_exit(dir.path(), &state).await;

    assert!(!status.success(), "the daemon started: {stderr}");
    // The daemon made the folder before it refused, and names it resolved.
    let resolved = std::fs::canonicalize(&state).unwrap();
    assert!(
        stderr.contains(&resolved.display().to_string()),
        "the refusal should name the state folder {}: {stderr}",
        resolved.display()
    );
    assert!(!dir.path().join("second.sock").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_starts_in_a_state_folder_of_ordinary_length() {
    let dir = padded_folder(40);
    let daemon = Daemon::start_in(dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    attach.type_text("echo hel''lo\n").await;

    attach.wait_for("hello").await;
}
