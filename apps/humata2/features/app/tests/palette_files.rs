//! The palette's Files group fills lazily: nothing is asked when the
//! workspace loads, each session's files are fetched the first time the
//! palette opens, and file-change events keep the lists current after that.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState};
use humata2_clients::model::{SessionFile, SessionId};
use humata2_clients::{Clients, Event, FileChange, Fakes, fixture};
use humata2_palette::Command;
use tca::{TestClock, TestStore};

/// The app loaded from a daemon whose workspace carries no file lists, as a
/// real host's does.
fn loaded() -> (TestStore<App>, Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut workspace = fakes.daemon.workspace();
    workspace.session_files.clear();
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(workspace)), |_| {});
    store.settle();
    (store, fakes)
}

fn file_requests(fakes: &Fakes) -> Vec<String> {
    fakes
        .daemon
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("find_files"))
        .collect()
}

fn live_sessions(fakes: &Fakes) -> Vec<SessionId> {
    fakes
        .daemon
        .workspace()
        .sessions
        .iter()
        .filter(|session| !session.archived)
        .map(|session| session.id.clone())
        .collect()
}

/// The files the palette lists, as (session, path).
fn palette_files(store: &TestStore<App>) -> Vec<(SessionId, String)> {
    store
        .state()
        .palette
        .entries
        .iter()
        .filter_map(|entry| match &entry.command {
            Command::File { session, path, .. } => Some((session.clone(), path.clone())),
            _ => None,
        })
        .collect()
}

fn open_palette(store: &mut TestStore<App>, sessions: usize) {
    store.send(A::OpenPalette, |_| {});
    for _ in 0..sessions {
        store.receive_matching(
            "a session's files",
            |action| matches!(action, A::SessionFiles { .. }),
            |_| {},
        );
    }
}

#[test]
fn loading_the_workspace_asks_for_no_files() {
    let (mut store, fakes) = loaded();
    store.skip_received();
    assert_eq!(file_requests(&fakes), Vec::<String>::new());
    assert!(store.state().workspace.session_files.is_empty());
    store.skip_in_flight();
}

#[test]
fn opening_the_palette_fetches_each_sessions_files_once() {
    let (mut store, fakes) = loaded();
    let sessions = live_sessions(&fakes);
    open_palette(&mut store, sessions.len());
    let mut asked = file_requests(&fakes);
    asked.sort();
    let mut expected: Vec<String> = sessions.iter().map(|id| format!("find_files {id}")).collect();
    expected.sort();
    assert_eq!(asked, expected, "one request per session, no more");
    let listed = palette_files(&store);
    for (session, files) in &fakes.daemon.workspace().session_files {
        for file in files.iter().filter(|file| !file.folder) {
            assert!(
                listed.contains(&(session.clone(), file.path.clone())),
                "the palette lists {} of {session}",
                file.path
            );
        }
    }
    store.skip_in_flight();
}

#[test]
fn opening_the_palette_again_fetches_nothing() {
    let (mut store, fakes) = loaded();
    open_palette(&mut store, live_sessions(&fakes).len());
    let before = file_requests(&fakes).len();
    store.send(A::Escape, |_| {});
    store.send(A::OpenPalette, |_| {});
    store.skip_received();
    assert_eq!(file_requests(&fakes).len(), before);
    store.skip_in_flight();
}

#[test]
fn a_created_file_joins_the_palette_of_a_loaded_session() {
    let (mut store, fakes) = loaded();
    let sessions = live_sessions(&fakes);
    open_palette(&mut store, sessions.len());
    let session = sessions[0].clone();
    let repo = fakes
        .daemon
        .workspace()
        .session(&session)
        .map(|s| s.repo.clone())
        .unwrap_or_else(|| panic!("the design workspace has {session}"));
    let file = SessionFile {
        repo,
        path: "src/brand_new_file.rs".to_owned(),
        folder: false,
    };
    store.send(
        A::Event(Event::Files {
            session: session.clone(),
            change: FileChange::Created(file),
        }),
        |_| {},
    );
    assert!(
        palette_files(&store).contains(&(session, "src/brand_new_file.rs".to_owned())),
        "the new file is in the Files group"
    );
    store.skip_in_flight();
}

#[test]
fn a_deleted_file_leaves_the_palette_and_an_unloaded_session_ignores_events() {
    let (mut store, fakes) = loaded();
    let sessions = live_sessions(&fakes);
    let workspace = fakes.daemon.workspace();
    let session = sessions
        .iter()
        .find(|id| {
            workspace
                .session_files
                .get(*id)
                .is_some_and(|files| files.iter().any(|file| !file.folder))
        })
        .cloned()
        .unwrap_or_else(|| panic!("the design workspace has a session with files"));
    let file = workspace.session_files[&session]
        .iter()
        .find(|file| !file.folder)
        .cloned()
        .unwrap_or_else(|| panic!("a file"));
    // Before the palette ever opened, the list isn't loaded: the event is ignored.
    store.send(
        A::Event(Event::Files {
            session: session.clone(),
            change: FileChange::Created(file.clone()),
        }),
        |_| {},
    );
    assert!(store.state().workspace.session_files.is_empty());
    open_palette(&mut store, sessions.len());
    assert!(palette_files(&store).contains(&(session.clone(), file.path.clone())));
    store.send(
        A::Event(Event::Files {
            session: session.clone(),
            change: FileChange::Deleted(file.clone()),
        }),
        |_| {},
    );
    assert!(!palette_files(&store).contains(&(session, file.path)));
    store.skip_in_flight();
}
