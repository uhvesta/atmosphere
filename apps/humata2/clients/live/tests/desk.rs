//! The Desk through the fake daemon and through `LiveDaemon` on a real
//! `humatad2` (admin build), with the same cases, so the fake can't drift
//! from what the daemon does (DK-2, DK-4, DK-11, D182.7).

// The shared support serves other cases this file doesn't run.
#[allow(dead_code)]
mod support {
    pub mod cases;
    pub mod fake;
    pub mod real;
}

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::desk::{
    self, Access, CleanUp, DeskClient, DeskSession, Folder, FolderChoice, NewDeskSession,
};
use humata2_clients::model::{MachineId, SessionId};
use humata2_clients::{Daemon, DaemonError, Workspace};
use support::cases::World;
use support::{fake, real};

const DEADLINE: Duration = Duration::from_secs(20);

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

async fn load(daemon: &Arc<dyn Daemon>) -> Workspace {
    within("load", daemon.load()).await.expect("load the workspace")
}

async fn this_mac(daemon: &Arc<dyn Daemon>) -> MachineId {
    load(daemon)
        .await
        .machines
        .first()
        .expect("the workspace has a machine")
        .id
        .clone()
}

/// Starts a scratch session titled `title` with the default switches.
async fn start(
    world: &dyn World,
    daemon: &Arc<dyn Daemon>,
    client: &Arc<dyn DeskClient>,
    title: &str,
) -> SessionId {
    let machine = this_mac(daemon).await;
    within(
        "desk start",
        client.start(NewDeskSession {
            title: title.to_owned(),
            agent: world.agent(),
            machine,
            folder: FolderChoice::Scratch {
                name: desk::folder_name(title),
            },
            repos: Vec::new(),
            access: Access::default(),
        }),
    )
    .await
    .expect("start a desk session")
}

async fn desk_session(daemon: &Arc<dyn Daemon>, id: &SessionId) -> DeskSession {
    load(daemon)
        .await
        .desk
        .sessions
        .get(id)
        .cloned()
        .unwrap_or_else(|| panic!("the workspace's desk lists {id}"))
}

async fn a_started_session_is_on_the_desk(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let id = start(world, &daemon, &client, "Plan Q4").await;
    let workspace = load(&daemon).await;
    let session = workspace.session(&id).expect("the session is listed");
    assert!(desk::is_desk(session), "{session:?}");
    let found = workspace.desk.sessions.get(&id).expect("the desk lists it");
    match &found.folder {
        Folder::Scratch { path, kept } => {
            assert!(path.ends_with("/desk/plan-q4"), "{path}");
            assert!(!kept);
        }
        other => panic!("a scratch folder, not {other:?}"),
    }
    assert_eq!(found.access, Access::default());
    assert_eq!(found.closed, None);
}

async fn closing_and_reopening(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let id = start(world, &daemon, &client, "Notes").await;
    within("close", client.close(&id)).await.expect("close");
    let closed = desk_session(&daemon, &id).await.closed.expect("it is closed");
    assert_eq!(closed.days_left, Some(7));
    within("reopen", client.reopen(&id)).await.expect("reopen");
    assert_eq!(desk_session(&daemon, &id).await.closed, None);
}

async fn a_kept_folder_has_no_days_left(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let id = start(world, &daemon, &client, "Keep me").await;
    within("close", client.close(&id)).await.expect("close");
    within("keep", client.keep(&id)).await.expect("keep");
    let found = desk_session(&daemon, &id).await;
    assert!(matches!(found.folder, Folder::Scratch { kept: true, .. }), "{found:?}");
    assert_eq!(found.closed.expect("still closed").days_left, None);
}

async fn the_access_switches_change(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let id = start(world, &daemon, &client, "Switches").await;
    let access = Access {
        boards: false,
        channels_and_machines: true,
    };
    within("set access", client.set_access(&id, access))
        .await
        .expect("set the switches");
    assert_eq!(desk_session(&daemon, &id).await.access, access);
}

async fn clean_up_deletes_only_the_ticked_session(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let first = start(world, &daemon, &client, "First").await;
    let second = start(world, &daemon, &client, "Second").await;
    for id in [&first, &second] {
        within("close", client.close(id)).await.expect("close");
    }
    let shown = within("clean up list", client.clean_up_list())
        .await
        .expect("list closed sessions");
    let listed: Vec<&SessionId> = shown.iter().map(|row| &row.session).collect();
    assert!(listed.contains(&&first) && listed.contains(&&second), "{shown:?}");
    let done = within("clean up", client.clean_up(shown, vec![first.clone()]))
        .await
        .expect("clean up");
    assert!(matches!(done, CleanUp::Deleted { sessions: 1, .. }), "{done:?}");
    let workspace = load(&daemon).await;
    assert!(workspace.session(&first).is_none());
    assert!(!workspace.desk.sessions.contains_key(&first));
    assert!(workspace.desk.sessions.contains_key(&second));
}

async fn a_stale_clean_up_deletes_nothing(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let first = start(world, &daemon, &client, "Old").await;
    within("close", client.close(&first)).await.expect("close");
    let shown = within("clean up list", client.clean_up_list())
        .await
        .expect("list closed sessions");
    let later = start(world, &daemon, &client, "Later").await;
    within("close", client.close(&later)).await.expect("close");
    let done = within("clean up", client.clean_up(shown, vec![first.clone()]))
        .await
        .expect("clean up");
    match done {
        CleanUp::Changed(now) => assert!(now.iter().any(|row| row.session == later), "{now:?}"),
        other => panic!("the changed list, not {other:?}"),
    }
    assert!(load(&daemon).await.desk.sessions.contains_key(&first));
}

async fn a_repo_not_written_owner_name_is_refused(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    client: Arc<dyn DeskClient>,
) {
    let id = start(world, &daemon, &client, "Repos").await;
    let before = desk_session(&daemon, &id).await;
    let refused = within("add repo", client.add_repo(&id, "not a repo")).await;
    assert!(matches!(refused, Err(DaemonError::Refused { .. })), "{refused:?}");
    assert_eq!(desk_session(&daemon, &id).await, before);
}

/// D382: a Desk session has no repository, so its document is read with an
/// empty repo, from the session's own folder. The real daemon only: the
/// fake keeps no files on a disk.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_sessions_document_is_read_with_no_repository() {
    let (world, daemon, client) = real::start_desk().await;
    let id = start(&world, &daemon, &client, "Draft").await;
    let Folder::Scratch { path, .. } = desk_session(&daemon, &id).await.folder else {
        panic!("a scratch folder");
    };
    let folder = std::path::Path::new(&path);
    std::fs::write(folder.join("rfc.md"), "the document").expect("write the document");

    let read = within("read", daemon.read_file(&id, "", "rfc.md")).await;
    let missing = within("read missing", daemon.read_file(&id, "", "nope.md")).await;
    let outside = within("read outside", daemon.read_file(&id, "", "../rfc.md")).await;

    assert_eq!(read.expect("the document is read"), "the document");
    assert!(matches!(missing, Err(DaemonError::NotFound { .. })), "{missing:?}");
    assert!(matches!(outside, Err(DaemonError::Refused { .. })), "{outside:?}");
}

macro_rules! parity {
    ($($case:ident => $fake:ident, $real:ident;)*) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $fake() {
            let (world, daemon, client) = fake::start_desk();
            $case(&world, daemon, client).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn $real() {
            let (world, daemon, client) = real::start_desk().await;
            $case(&world, daemon, client).await;
        }
    )*};
}

parity! {
    a_started_session_is_on_the_desk => a_started_session_is_on_the_desk_on_fake, a_started_session_is_on_the_desk_on_real;
    closing_and_reopening => closing_and_reopening_on_fake, closing_and_reopening_on_real;
    a_kept_folder_has_no_days_left => a_kept_folder_has_no_days_left_on_fake, a_kept_folder_has_no_days_left_on_real;
    the_access_switches_change => the_access_switches_change_on_fake, the_access_switches_change_on_real;
    clean_up_deletes_only_the_ticked_session => clean_up_deletes_only_the_ticked_session_on_fake, clean_up_deletes_only_the_ticked_session_on_real;
    a_stale_clean_up_deletes_nothing => a_stale_clean_up_deletes_nothing_on_fake, a_stale_clean_up_deletes_nothing_on_real;
    a_repo_not_written_owner_name_is_refused => a_repo_not_written_owner_name_is_refused_on_fake, a_repo_not_written_owner_name_is_refused_on_real;
}
