//! A terminal tab keeps what its program says about itself (its title and
//! working folder) apart from the name the user gave the tab.

mod common;

use std::time::Duration;

use common::{Attach, Daemon, open};
use humata2_grpc::humata::v1 as pb;

async fn machine_shell(daemon: &Daemon) -> pb::TabId {
    daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap()
}

async fn tab_of(daemon: &Daemon, id: &pb::TabId) -> pb::Tab {
    daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .tabs
        .into_iter()
        .find(|tab| tab.id.as_ref() == Some(id))
        .expect("the tab is listed")
}

/// The tab once its program's title and folder are both reported.
async fn reported(daemon: &Daemon, id: &pb::TabId) -> pb::Tab {
    for _ in 0..200 {
        let tab = tab_of(daemon, id).await;
        if tab.program_title.is_some() && tab.pwd.is_some() {
            return tab;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the program's title and folder were never reported");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_program_title_and_folder_are_reported_beside_an_unchanged_tab_name_and_survive_a_restart() {
    let mut daemon = Daemon::start().await;
    let id = machine_shell(&daemon).await;
    let name = tab_of(&daemon, &id).await.title;
    let mut attach = Attach::start(&daemon, open(&id, 0)).await;
    attach
        .type_text("printf '\\033]2;build-title\\007\\033]7;file://localhost/tmp\\007'\n")
        .await;

    let tab = reported(&daemon, &id).await;
    assert_eq!(tab.program_title.as_deref(), Some("build-title"));
    assert_eq!(tab.pwd.as_deref(), Some("/tmp"));
    assert_eq!(tab.title, name);

    drop(attach);
    daemon.restart().await;
    let tab = tab_of(&daemon, &id).await;
    assert_eq!(tab.program_title.as_deref(), Some("build-title"));
    assert_eq!(tab.pwd.as_deref(), Some("/tmp"));
    assert_eq!(tab.title, name);
}
