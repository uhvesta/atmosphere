//! DS-65, DS-67: skim files fold to one row at the end of their repository,
//! and a moved repository asks the agent, once, to send its stack again.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{SessionId, Status};
use humata2_clients::FakeDaemon;
use humata2_review::stack::{Tier, fold_skims, skim_label};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

#[test]
fn skim_files_move_to_the_end_and_the_rest_keep_their_order() {
    let files = ["Cargo.lock", "src/a.rs", "gen/b.rs", "src/c.rs"];
    let folded = fold_skims(&files, |path| match *path {
        "Cargo.lock" | "gen/b.rs" => Tier::Skim,
        "src/c.rs" => Tier::Key,
        _ => Tier::Normal,
    });
    assert_eq!(folded.shown, vec![&"src/a.rs", &"src/c.rs"]);
    assert_eq!(folded.skims, vec![&"Cargo.lock", &"gen/b.rs"]);
}

#[test]
fn with_nothing_to_skim_every_file_shows() {
    let files = ["a", "b"];
    let folded = fold_skims(&files, |_| Tier::Normal);
    assert_eq!(folded.shown.len(), 2);
    assert!(folded.skims.is_empty());
}

#[test]
fn the_skim_row_counts_every_file_and_names_a_few() {
    let one = skim_label(&["web/Cargo.lock"]);
    assert!(one.starts_with('1'));
    assert!(one.contains("Cargo.lock"));
    assert!(!one.contains("web/"), "file names, not paths: {one}");

    let many = skim_label(&["Cargo.lock", "a/b.rs", "c.rs", "d.rs"]);
    assert!(many.starts_with('4'));
    assert!(many.contains("Cargo.lock") && many.contains("b.rs"));
    assert!(!many.contains("d.rs"), "only the first names: {many}");
    assert!(skim_label(&[]).is_empty());
}

fn rig(status: Status, open: Option<SessionId>) -> (FakeDaemon, TestStore<Review>) {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = status;
        }
    }
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace.clone(), clock.clone());
    let state = ReviewState {
        workspace: Arc::new(workspace),
        session: open,
        ..ReviewState::default()
    };
    let store = TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    (daemon, store)
}

fn sends(daemon: &FakeDaemon) -> Vec<String> {
    daemon.calls().into_iter().filter(|call| call.starts_with("send ")).collect()
}

fn ask(store: &mut TestStore<Review>) {
    store.send(
        A::AskAgain { layer: "client".to_owned(), repo: "acme/web".to_owned() },
        |_| {},
    );
    store.receive_matching("the terminal answered", |action| matches!(action, A::AskedAgain { .. }), |_| {});
}

#[test]
fn asking_again_types_one_message_naming_the_layer_and_repository() {
    let (daemon, mut store) = rig(Status::Idle, Some(session()));
    ask(&mut store);
    let sent = sends(&daemon);
    assert_eq!(sent.len(), 1, "one message: {sent:?}");
    assert!(sent[0].contains("acme/web"));
    assert!(sent[0].contains("client"));
    assert!(store.state().ask_again_error.is_empty());
}

#[test]
fn an_ask_the_terminal_refuses_is_kept_for_the_panel() {
    let (_, mut store) = rig(Status::Stopped, Some(session()));
    ask(&mut store);
    assert!(store.state().ask_again_error.contains_key(&session()));
}

#[test]
fn without_an_open_session_asking_types_nothing() {
    let (daemon, mut store) = rig(Status::Idle, None);
    store.send(
        A::AskAgain { layer: "client".to_owned(), repo: "acme/web".to_owned() },
        |_| {},
    );
    assert!(sends(&daemon).is_empty());
}
