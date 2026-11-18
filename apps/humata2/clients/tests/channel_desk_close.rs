//! The fake daemon closes and reopens a channel's Desk sessions as the real
//! one does (DV-13, CD-4): a kept folder closes in place and can reopen; one
//! that is not kept is gone at once.

use std::sync::Arc;

use humata2_clients::desk::Folder;
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{DaemonError, DeskClient, FakeDaemon, Workspace, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn id() -> SessionId {
    SessionId::new("cdesk-triage")
}

fn daemon(change: impl FnOnce(&mut humata2_clients::desk::ChannelDeskSession)) -> FakeDaemon {
    let mut workspace: Workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let Some(record) = workspace.desk.channel_sessions.get_mut(&id()) else {
        panic!("the fixture has the triage session");
    };
    change(record);
    FakeDaemon::new(workspace, Arc::new(TestClock::new()))
}

#[test]
fn closing_a_folder_that_holds_a_version_closes_in_place_and_it_reopens() {
    let daemon = daemon(|record| record.desk.version_count = 2);
    block_on(daemon.close(&id())).expect("it closes");
    let workspace = daemon.workspace();
    let Some(record) = workspace.desk.channel_sessions.get(&id()) else {
        panic!("a kept folder's session stays");
    };
    let Some(closed) = &record.desk.closed else {
        panic!("it is closed");
    };
    assert_eq!(closed.days_left, None);
    assert!(workspace.session(&id()).is_some());

    block_on(daemon.reopen(&id())).expect("it reopens");
    let workspace = daemon.workspace();
    let Some(record) = workspace.desk.channel_sessions.get(&id()) else {
        panic!("the session is still there");
    };
    assert!(record.desk.closed.is_none());
    let Some(session) = workspace.session(&id()) else {
        panic!("the session row is there");
    };
    assert_eq!(session.status, Status::Idle);
}

#[test]
fn closing_a_scratch_folder_marked_kept_with_no_version_still_goes_at_once() {
    let daemon = daemon(|record| {
        let path = record.desk.folder.path().to_owned();
        record.desk.folder = Folder::Scratch { path, kept: true };
        record.desk.version_count = 0;
    });
    block_on(daemon.close(&id())).expect("it closes");
    assert!(!daemon.workspace().desk.channel_sessions.contains_key(&id()));
}

#[test]
fn closing_a_folder_that_is_not_kept_removes_the_session_at_once() {
    let daemon = daemon(|record| record.desk.version_count = 0);
    block_on(daemon.close(&id())).expect("it closes");
    let workspace = daemon.workspace();
    assert!(!workspace.desk.channel_sessions.contains_key(&id()));
    assert!(workspace.session(&id()).is_none());
    assert!(matches!(
        block_on(daemon.reopen(&id())),
        Err(DaemonError::NotFound { .. })
    ));
}

#[test]
fn reopening_an_open_channel_desk_session_is_refused() {
    let daemon = daemon(|_| {});
    assert!(matches!(
        block_on(daemon.reopen(&id())),
        Err(DaemonError::Refused { .. })
    ));
}

#[test]
fn a_reopened_session_is_the_most_recently_used_of_its_channel() {
    let daemon = daemon(|record| record.desk.version_count = 1);
    let before = daemon
        .workspace()
        .desk
        .channel_sessions
        .values()
        .map(|found| found.desk.last_used.order)
        .max()
        .unwrap_or(0);
    block_on(daemon.close(&id())).expect("it closes");
    block_on(daemon.reopen(&id())).expect("it reopens");
    let workspace = daemon.workspace();
    let Some(record) = workspace.desk.channel_sessions.get(&id()) else {
        panic!("the session is there");
    };
    assert!(record.desk.last_used.order > before);
}
