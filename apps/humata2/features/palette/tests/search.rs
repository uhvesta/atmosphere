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

#[test]
fn namespace_prefixes_filter_to_their_kind() {
    let state = PaletteState {
        workspace: Arc::new(fixture::design()),
        context: Some(SessionId::new("ledger")),
        in_session: true,
        open: true,
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette).non_exhaustive();

    // `>` filters exclusively to commands (stripping `>` and optional space).
    for query in [">", ">sidebar", "> new"] {
        store.send(A::Query(query.into()), |_| {});
        assert!(!store.state().entries.is_empty(), "{query:?} returns commands");
        assert!(
            store.state().entries.iter().all(|e| !matches!(
                e.command,
                Command::Session(_)
                    | Command::Terminal { .. }
                    | Command::File { .. }
                    | Command::Channel(_)
                    | Command::Machine(_)
            )),
            "{query:?} returns commands only: {:?}",
            store.state().entries
        );
    }
    store.send(A::Query(">sidebar".into()), |_| {});
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::ToggleSidebar),
        "`>sidebar` finds Toggle sidebar"
    );

    // `#` filters exclusively to channels, while `#412` still offers PR #412.
    store.send(A::Query("#pay".into()), |_| {});
    assert!(!store.state().entries.is_empty());
    assert!(
        store
            .state()
            .entries
            .iter()
            .all(|e| matches!(e.command, Command::Channel(_))),
        "`#pay` returns channels only"
    );
    store.send(A::Query("#412".into()), |_| {});
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Pull(412)),
        "`#412` keeps pull request lookup"
    );

    // `@` filters exclusively to sessions (and their terminals).
    store.send(A::Query("@ledg".into()), |_| {});
    assert!(!store.state().entries.is_empty());
    assert!(
        store
            .state()
            .entries
            .iter()
            .all(|e| matches!(e.command, Command::Session(_) | Command::Terminal { .. })),
        "`@ledg` returns sessions only"
    );

    // `/` filters exclusively to files.
    store.send(A::Query("/client.rs".into()), |_| {});
    assert!(!store.state().entries.is_empty());
    assert!(
        store
            .state()
            .entries
            .iter()
            .all(|e| matches!(e.command, Command::File { .. })),
        "`/client.rs` returns files only"
    );

    // `!` filters exclusively to machines.
    store.send(A::Query("!".into()), |_| {});
    assert!(!store.state().entries.is_empty());
    assert!(
        store
            .state()
            .entries
            .iter()
            .all(|e| matches!(e.command, Command::Machine(_))),
        "`!` returns machines only"
    );

    store.finish();
}

#[test]
fn archived_sessions_are_excluded_by_default_and_included_when_opted_in() {
    let mut workspace = fixture::design();
    let archived_id = SessionId::new("ledger");
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == archived_id)
        .expect("design fixture has ledger")
        .archived = true;

    let state = PaletteState {
        workspace: Arc::new(workspace),
        open: true,
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette).non_exhaustive();

    // Excluded by default.
    store.send(A::Query("ledger".into()), |_| {});
    assert!(
        !store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Session(archived_id.clone())),
        "archived session is excluded by default"
    );

    // Included when explicitly toggled on.
    store.send(A::ToggleArchived, |_| {});
    assert!(store.state().include_archived);
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Session(archived_id.clone())),
        "archived session appears when Include archived is enabled"
    );

    // Excluded again when toggled off, but `archived:` prefix opts in inline.
    store.send(A::ToggleArchived, |_| {});
    assert!(!store.state().include_archived);
    assert!(
        !store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Session(archived_id.clone()))
    );
    store.send(A::Query("archived:ledger".into()), |_| {});
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Session(archived_id.clone())),
        "`archived:` prefix includes archived sessions"
    );

    store.finish();
}
