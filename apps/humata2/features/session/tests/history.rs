//! DV-3 to DV-8: a Markdown tab of a Desk file steps through its versions,
//! saves, restores and follows the agent's commits, on the fake daemon.
use std::sync::Arc;

use humata2_clients::docs::VersionBy;
use humata2_clients::model::LineKind;
use humata2_clients::{Clients, DaemonError, FakeDaemon, fixture, model::SessionId};
use humata2_session::{
    CompareDiff, DocumentMode, HistoryAction as H, Panel, Position, SessionAction as A, SessionState, Sessions,
};
use tca::{TestClock, TestStore};

const PATH: &str = "q4-plan.md";
const V1: &str = "# Plan\none\n";
const V2: &str = "# Plan\none\ntwo\n";

fn desk() -> SessionId {
    SessionId::new("desk-plan-q4")
}

fn store() -> (TestStore<Sessions>, FakeDaemon) {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (clients, fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
    let state = SessionState {
        workspace: Arc::new(fakes.daemon.workspace()),
        open: Some(desk()),
        ..Default::default()
    };
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes.daemon,
    )
}

/// Lets the effects the last actions started finish and reduces what they sent.
fn drain(store: &mut TestStore<Sessions>) {
    for _ in 0..4 {
        store.skip_received();
    }
}

fn act(store: &mut TestStore<Sessions>, action: H) {
    store.send(A::History(action), |_| {});
    drain(store);
}

fn load(store: &mut TestStore<Sessions>) {
    act(
        store,
        H::Load {
            session: desk(),
            path: PATH.into(),
        },
    );
}

fn path() -> String {
    PATH.to_owned()
}

/// A folder with two agent versions, listed.
fn two_versions() -> (TestStore<Sessions>, FakeDaemon) {
    let (mut store, daemon) = store();
    daemon.commit_as_agent(&desk(), PATH, V1, "First draft");
    daemon.commit_as_agent(&desk(), PATH, V2, "Add two");
    load(&mut store);
    (store, daemon)
}

fn shown(store: &TestStore<Sessions>) -> Option<u32> {
    store
        .state()
        .history(&desk(), PATH)
        .and_then(|history| history.viewing())
        .map(|version| version.number)
}

#[test]
fn stepping_back_and_forward_walks_the_versions_and_keeps_the_mode() {
    let (mut store, _) = two_versions();
    store.send(
        A::DocumentMode {
            repo: String::new(),
            path: path(),
            mode: DocumentMode::SideBySide,
        },
        |_| {},
    );
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert_eq!(history.versions.len(), 2);
    assert_eq!(history.newest(), Some(2));
    assert_eq!(history.position, Position::Current);
    assert!(!history.can_forward());

    act(&mut store, H::Back { path: path() });
    assert_eq!(shown(&store), Some(2));
    act(&mut store, H::Back { path: path() });
    assert_eq!(shown(&store), Some(1));
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert!(!history.can_back());
    assert_eq!(history.viewed.as_ref().map(|read| read.text.as_str()), Some(V1));
    // Nothing is older than v1.
    act(&mut store, H::Back { path: path() });
    assert_eq!(shown(&store), Some(1));

    act(&mut store, H::Forward { path: path() });
    assert_eq!(shown(&store), Some(2));
    act(&mut store, H::Forward { path: path() });
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert_eq!(history.position, Position::Current);
    assert!(history.viewed.is_none());

    let view = &store.state().views[&desk()];
    assert_eq!(view.document("", PATH).mode, DocumentMode::SideBySide);
}

#[test]
fn picking_a_version_in_the_list_shows_it() {
    let (mut store, _) = two_versions();
    let oldest = store
        .state()
        .history(&desk(), PATH)
        .and_then(|history| history.versions.last())
        .map(|version| version.id.clone())
        .expect("a version");
    act(&mut store, H::Pick { path: path(), id: oldest });
    assert_eq!(shown(&store), Some(1));
}

#[test]
fn save_version_is_off_without_a_change_and_on_after_one() {
    let (mut store, daemon) = two_versions();
    assert!(!store.state().history(&desk(), PATH).expect("a history").can_save());
    act(&mut store, H::SaveVersion { path: path() });
    assert_eq!(store.state().history(&desk(), PATH).expect("a history").versions.len(), 2);

    daemon.edit_document(&desk(), PATH, "# Plan\none\ntwo\nthree\n");
    load(&mut store);
    assert!(store.state().history(&desk(), PATH).expect("a history").can_save());

    act(
        &mut store,
        H::Message {
            path: path(),
            text: "Add three".into(),
        },
    );
    act(&mut store, H::SaveVersion { path: path() });
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert_eq!(history.versions.len(), 3);
    assert_eq!(history.versions[0].by, VersionBy::You);
    assert_eq!(history.versions[0].message, "Add three");
    assert!(!history.unsaved);
    assert!(history.message.is_empty());
    assert!(!history.can_save());
}

#[test]
fn an_old_version_is_read_only_until_back_to_current() {
    let (mut store, daemon) = two_versions();
    daemon.edit_document(&desk(), PATH, "# Plan\nunsaved\n");
    load(&mut store);
    assert!(!store.state().doc_read_only(&desk(), PATH));

    act(&mut store, H::Back { path: path() });
    assert!(store.state().doc_read_only(&desk(), PATH));
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert!(history.unsaved);
    assert!(!history.can_save());

    act(
        &mut store,
        H::Message {
            path: path(),
            text: "typed on an old version".into(),
        },
    );
    act(&mut store, H::SaveVersion { path: path() });
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert!(history.message.is_empty());
    assert_eq!(history.versions.len(), 2);

    act(&mut store, H::BackToCurrent { path: path() });
    assert!(!store.state().doc_read_only(&desk(), PATH));
}

#[test]
fn restoring_adds_a_version_and_lands_on_current() {
    let (mut store, _) = two_versions();
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::Restore { path: path() });

    let history = store.state().history(&desk(), PATH).expect("a history");
    assert_eq!(history.position, Position::Current);
    assert!(!history.working);
    assert_eq!(history.versions.len(), 3);
    assert_eq!(history.versions[0].by, VersionBy::You);
    assert!(!history.unsaved);
    let text = store
        .state()
        .file_contents
        .get(&(desk(), String::new(), PATH.to_owned()))
        .cloned();
    assert_eq!(text, Some(Ok(V1.to_owned())));
}

#[test]
fn restoring_with_unsaved_changes_saves_them_first() {
    let (mut store, daemon) = two_versions();
    daemon.edit_document(&desk(), PATH, "# Plan\nunsaved\n");
    load(&mut store);
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::Restore { path: path() });
    assert_eq!(store.state().history(&desk(), PATH).expect("a history").versions.len(), 4);
}

#[test]
fn restoring_the_text_the_file_already_has_changes_nothing() {
    let (mut store, _) = two_versions();
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::Restore { path: path() });
    let state = store.state();
    assert!(state.error.is_some());
    let history = state.history(&desk(), PATH).expect("a history");
    assert_eq!(history.versions.len(), 2);
    assert!(!history.working);
}

#[test]
fn an_agent_commit_moves_doc_head_and_the_versions_are_listed_again() {
    let (mut store, daemon) = two_versions();
    daemon.commit_as_agent(&desk(), PATH, "# Plan\nthree\n", "Rewrite");
    // The page doesn't know yet.
    assert_eq!(store.state().history(&desk(), PATH).expect("a history").versions.len(), 2);

    store.send(A::Show(Arc::new(daemon.workspace())), |_| {});
    drain(&mut store);
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert_eq!(history.versions.len(), 3);
    assert_eq!(history.versions[0].by, VersionBy::Agent);
}

#[test]
fn a_viewed_version_stays_when_the_agent_commits() {
    let (mut store, daemon) = two_versions();
    act(&mut store, H::Back { path: path() });
    daemon.commit_as_agent(&desk(), PATH, "# Plan\nthree\n", "Rewrite");
    store.send(A::Show(Arc::new(daemon.workspace())), |_| {});
    drain(&mut store);
    let history = store.state().history(&desk(), PATH).expect("a history");
    assert_eq!(history.versions.len(), 3);
    assert_eq!(shown(&store), Some(2));
}

#[test]
fn a_loaded_desk_document_gets_a_history_on_its_own() {
    let (mut store, _) = store();
    store.send(
        A::FileLoaded {
            session: desk(),
            repo: String::new(),
            path: path(),
            result: Ok(V1.to_owned()),
        },
        |_| {},
    );
    drain(&mut store);
    assert!(store.state().history(&desk(), PATH).is_some_and(|history| history.loaded));
}

#[test]
fn files_outside_a_versioned_desk_folder_have_no_history() {
    let (mut store, _) = store();
    // A clone's file in a channel session, a picked Desk folder, and a file
    // that isn't Markdown.
    let loaded = |session: &str, repo: &str, path: &str| A::FileLoaded {
        session: SessionId::new(session),
        repo: repo.to_owned(),
        path: path.to_owned(),
        result: Ok("text".to_owned()),
    };
    store.send(loaded("ledger", "api", "README.md"), |_| {});
    store.send(loaded("desk-tidy-infra", "", "notes.md"), |_| {});
    store.send(loaded("desk-plan-q4", "", "script.py"), |_| {});
    drain(&mut store);
    assert!(store.state().histories.is_empty());
}

#[test]
fn compare_with_current_opens_the_diff_view_against_the_shown_version() {
    let (mut store, _) = two_versions();
    act(&mut store, H::CompareWithCurrent { path: path() });
    assert!(store.state().compare.is_none());

    act(&mut store, H::Back { path: path() });
    act(&mut store, H::CompareWithCurrent { path: path() });
    let compare = store.state().compare.clone().expect("a comparison");
    let shown_id = store
        .state()
        .history(&desk(), PATH)
        .and_then(|history| history.viewing())
        .map(|version| version.id.clone());
    assert_eq!(Some(compare.version), shown_id);
    assert_eq!(
        store.state().view().and_then(|view| view.panel),
        Some(Panel::Changes)
    );
}

/// The first version shown, so the comparison is against the file's V2.
fn comparing_the_first_version() -> (TestStore<Sessions>, FakeDaemon) {
    let (mut store, daemon) = two_versions();
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::Back { path: path() });
    act(&mut store, H::CompareWithCurrent { path: path() });
    (store, daemon)
}

#[test]
fn compare_with_current_loads_the_daemons_diff_of_the_version_and_the_file() {
    let (store, _) = comparing_the_first_version();
    let compare = store.state().compare.clone().expect("a comparison");
    let CompareDiff::Loaded(lines) = compare.diff else {
        panic!("the diff is loaded, not {:?}", compare.diff);
    };
    let added: Vec<_> = lines
        .iter()
        .filter(|line| line.kind == LineKind::Added)
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(added, vec!["two"]);
    assert!(lines.iter().all(|line| line.kind != LineKind::Removed));
}

#[test]
fn a_failed_diff_is_kept_to_show_and_a_stale_answer_is_dropped() {
    let (mut store, _) = comparing_the_first_version();
    let compare = store.state().compare.clone().expect("a comparison");
    let refused = Err(DaemonError::Refused {
        what: "Comparing a version with the document".to_owned(),
        reason: "the version is gone".to_owned(),
    });
    act(
        &mut store,
        H::CompareLoaded {
            session: desk(),
            path: path(),
            version: "some-other-version".to_owned(),
            result: refused.clone(),
        },
    );
    assert!(matches!(
        store.state().compare.as_ref().map(|c| &c.diff),
        Some(CompareDiff::Loaded(_))
    ));
    act(
        &mut store,
        H::CompareLoaded {
            session: desk(),
            path: path(),
            version: compare.version,
            result: refused,
        },
    );
    assert!(matches!(
        store.state().compare.as_ref().map(|c| &c.diff),
        Some(CompareDiff::Failed(message)) if !message.is_empty()
    ));
}
