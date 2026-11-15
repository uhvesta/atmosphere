//! Search keeps clone ownership and includes the current session's commands.
use humata2_clients::{fixture, model::SessionId};
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState};
use std::sync::Arc;
use tca::TestStore;
#[test]
fn file_search_and_parent_hints_keep_their_session_context() {
    let state = PaletteState {
        workspace: Arc::new(fixture::design()),
        context: Some(SessionId::new("ledger")),
        in_session: true,
        open: true,
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette).non_exhaustive();
    store.send(A::Query("client.rs".into()), |s| {
        s.query = "client.rs".into()
    });
    let files: Vec<_> = store
        .state()
        .entries
        .iter()
        .filter_map(|e| match &e.command {
            Command::File {
                session,
                repo,
                path,
            } => Some((session, repo, path)),
            _ => None,
        })
        .collect();
    assert!(!files.is_empty());
    assert!(
        files
            .iter()
            .all(|(session, _, path)| session.as_str() == "ledger" && path.ends_with("client.rs"))
    );
    store.send(A::Query("ledg".into()), |s| s.query = "ledg".into());
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Session(SessionId::new("brin")))
    );
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::TerminalFullScreen)
    );
    assert!(matches!(
        store.state().entries.first().unwrap().command,
        Command::Session(_)
    ));
    store.send(A::Query("no-result-xyz".into()), |s| {
        s.query = "no-result-xyz".into();
        s.entries = Arc::new(Vec::new());
    });
    assert!(store.state().entries.is_empty());
    store.finish();
}
