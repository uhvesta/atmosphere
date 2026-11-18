//! D748: a session's shell tab on the real daemon finds Humata's own tools
//! first on its PATH, beside its session token and socket.

mod common;

use std::path::{Path, PathBuf};

use common::{Attach, Daemon, folder, open, state_folder, wait_for_path};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};

const SESSION: &str = "se_tools";
const CHANNEL: &str = "ch_tools";

#[tokio::test(flavor = "multi_thread")]
async fn a_session_shell_finds_humatas_tools_first_on_its_path() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: CHANNEL.to_owned(),
            machine: String::new(),
            title: "tools".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId { value: SESSION.to_owned() }),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    let file = daemon.dir.path().join("tab-path");
    let partial = daemon.dir.path().join("tab-path.part");
    attach
        .type_text(&format!(
            "printf '%s\\n%s\\n' \"$PATH\" \"${{HUMATA_SESSION_TOKEN:+set}}\" > '{}'; mv '{}' '{}'\r",
            partial.display(),
            partial.display(),
            file.display()
        ))
        .await;
    wait_for_path(&file).await;
    let text = std::fs::read_to_string(&file).unwrap();
    let mut lines = text.lines();
    let path = lines.next().unwrap_or_default();
    assert_eq!(lines.next(), Some("set"), "the tab has its session token: {text:?}");
    // With no `--humata`, the daemon's `humata` is the one beside it,
    // named by its real path when that resolves,
    // or the daemon's own folder.
    let daemon_binary = PathBuf::from(std::env::var_os("HUMATAD2").expect("HUMATAD2"));
    let beside = daemon_binary.parent().unwrap_or(Path::new(".")).to_owned();
    let mut expected = vec![beside.clone()];
    expected.extend(std::fs::canonicalize(&beside));
    // The rig runs the daemon from its real path, not the runfiles link.
    expected.extend(
        std::fs::canonicalize(&daemon_binary)
            .ok()
            .and_then(|binary| binary.parent().map(Path::to_owned)),
    );
    expected.extend(
        std::fs::canonicalize(beside.join("humata"))
            .ok()
            .and_then(|humata| humata.parent().map(Path::to_owned)),
    );
    let expected: Vec<PathBuf> = expected.iter().filter_map(|folder| std::fs::canonicalize(folder).ok()).collect();
    let first = path.split(':').next().unwrap_or_default();
    assert!(
        std::fs::canonicalize(first).is_ok_and(|first| expected.contains(&first)),
        "PATH starts with Humata's tools folder: {path:?}, expected one of {expected:?}"
    );
    assert_eq!(path.split(':').filter(|entry| *entry == first).count(), 1, "once: {path:?}");
}
