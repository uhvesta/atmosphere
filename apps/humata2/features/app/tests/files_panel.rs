//! The session page's Files panel lists the session's files and reconciles
//! against the daemon whenever the panel is opened or refreshed.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{SessionFile, SessionId};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_session::{Panel, SessionAction};
use tca::{TestClock, TestStore};

fn file_requests(fakes: &Fakes) -> usize {
    fakes
        .daemon
        .calls()
        .iter()
        .filter(|call| call.starts_with("find_files"))
        .count()
}

/// The app on a session's page, its first file list read, and that
/// session: one the design workspace gives files.
fn on_a_session_page() -> (TestStore<App>, Fakes, SessionId) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let session = fakes
        .daemon
        .workspace()
        .session_files
        .iter()
        .find(|(_, files)| !files.is_empty())
        .map(|(id, _)| id.clone())
        .expect("the design workspace has a session with files");
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    store.send(A::Switch(Route::Session(session.clone())), |_| {});
    store.settle();
    (store, fakes, session)
}

fn listed(store: &TestStore<App>, session: &SessionId) -> usize {
    store
        .state()
        .workspace
        .session_files
        .get(session)
        .map_or(0, Vec::len)
}

#[test]
fn opening_files_reads_a_list_that_came_back_empty_again() {
    let (mut store, fakes, session) = on_a_session_page();
    store.send(
        A::SessionFiles {
            session: session.clone(),
            result: Ok(Vec::new()),
        },
        |_| {},
    );
    assert_eq!(listed(&store, &session), 0);
    let before = file_requests(&fakes);

    store.send(A::Session(SessionAction::SetPanel(Some(Panel::Files))), |_| {});
    store.receive_matching(
        "the session's files, read again",
        |a| matches!(a, A::SessionFiles { session: s, result: Ok(files) } if *s == session && !files.is_empty()),
        |_| {},
    );
    assert_eq!(file_requests(&fakes), before + 1, "the list is read again");
    assert!(listed(&store, &session) > 0, "the panel has the session's files");
    store.skip_in_flight();
}

#[test]
fn opening_files_reconciles_non_empty_cache_with_new_session_root_file() {
    let mut workspace = fixture::design();
    let session = workspace
        .session_files
        .iter()
        .find(|(_, files)| !files.is_empty())
        .map(|(id, _)| id.clone())
        .expect("the design workspace has a session with files");
    let repo_only = workspace.session_files[&session].clone();
    let root_doc = SessionFile {
        repo: String::new(),
        path: "CHANNEL_HOME_SPEC.md".to_owned(),
        folder: false,
    };
    workspace
        .session_files
        .entry(session.clone())
        .or_default()
        .push(root_doc.clone());

    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    store.send(A::Switch(Route::Session(session.clone())), |_| {});
    store.settle();

    // Simulate a stale non-empty cache that has repo files but missed the new session-root file.
    store.send(
        A::SessionFiles {
            session: session.clone(),
            result: Ok(repo_only),
        },
        |_| {},
    );
    assert!(listed(&store, &session) > 0);
    assert!(!store.state().workspace.session_files[&session].contains(&root_doc));
    let before = file_requests(&fakes);

    store.send(A::Session(SessionAction::SetPanel(Some(Panel::Files))), |_| {});
    store.receive_matching(
        "the session's files, reconciled on Panel::Files open",
        |a| matches!(a, A::SessionFiles { session: s, result: Ok(files) } if *s == session && files.contains(&root_doc)),
        |_| {},
    );
    assert_eq!(file_requests(&fakes), before + 1);
    assert!(store.state().workspace.session_files[&session].contains(&root_doc));

    // Explicit RefreshSessionFiles also reconciles even when non-empty.
    store.send(
        A::SessionFiles {
            session: session.clone(),
            result: Ok(vec![SessionFile {
                repo: "acme/api".to_owned(),
                path: "src/lib.rs".to_owned(),
                folder: false,
            }]),
        },
        |_| {},
    );
    let before_refresh = file_requests(&fakes);
    store.send(A::RefreshSessionFiles(session.clone()), |_| {});
    store.receive_matching(
        "the session's files, reconciled on explicit refresh",
        |a| matches!(a, A::SessionFiles { session: s, result: Ok(files) } if *s == session && files.contains(&root_doc)),
        |_| {},
    );
    assert_eq!(file_requests(&fakes), before_refresh + 1);
    assert!(store.state().workspace.session_files[&session].contains(&root_doc));
    store.skip_in_flight();
}

#[test]
fn a_big_first_repo_does_not_hide_the_sessions_other_repos() {
    let (mut store, _fakes, session) = on_a_session_page();
    let file = |repo: &str, i: usize| humata2_clients::model::SessionFile {
        repo: repo.to_owned(),
        path: format!("src/f{i}.rs"),
        folder: false,
    };
    let mut files: Vec<_> = (0..700).map(|i| file("acme/api", i)).collect();
    files.extend((0..3).map(|i| file("acme/web", i)));
    store.send(
        A::SessionFiles {
            session: session.clone(),
            result: Ok(files),
        },
        |_| {},
    );
    let listed = &store.state().workspace.session_files[&session];
    assert!(listed.iter().any(|f| f.repo == "acme/web"), "the second repo is listed");
    assert_eq!(listed.iter().filter(|f| f.repo == "acme/web").count(), 3);
    store.skip_in_flight();
}

#[test]
fn incremental_created_file_is_not_dropped_past_cap() {
    let (mut store, _fakes, session) = on_a_session_page();
    let files: Vec<_> = (0..humata2_clients::SESSION_FILES_CAP)
        .map(|i| humata2_clients::model::SessionFile {
            repo: "acme/api".to_owned(),
            path: format!("src/f{i}.rs"),
            folder: false,
        })
        .collect();
    store.send(
        A::SessionFiles {
            session: session.clone(),
            result: Ok(files),
        },
        |_| {},
    );
    assert_eq!(listed(&store, &session), humata2_clients::SESSION_FILES_CAP);

    let created = humata2_clients::model::SessionFile {
        repo: String::new(),
        path: "notes.md".to_owned(),
        folder: false,
    };
    store.send(
        A::Event(humata2_clients::Event::Files {
            session: session.clone(),
            change: humata2_clients::FileChange::Created(created.clone()),
        }),
        |_| {},
    );
    let after = &store.state().workspace.session_files[&session];
    assert!(
        after.contains(&created),
        "incremental FileChange::Created must not be dropped at SESSION_FILES_CAP"
    );
    store.skip_in_flight();
}
