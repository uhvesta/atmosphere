//! D166: a document tab's mode and link scrolling are kept per file and per session.
use humata2_clients::{Clients, fixture, model::SessionId};
use humata2_session::{
    DocumentMode, DocumentView, SessionAction as A, SessionState, SessionView, Sessions, Tab,
    TabGroup, is_markdown,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> TestStore<Sessions> {
    let workspace = fixture::design();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        ..Default::default()
    };
    state.open = Some(SessionId::new("ledger"));
    state
        .views
        .insert(SessionId::new("ledger"), SessionView::default());
    state
        .views
        .insert(SessionId::new("other"), SessionView::default());
    TestStore::new(state, Sessions::new(clients))
}

fn mode(path: &str, mode: DocumentMode) -> A {
    A::DocumentMode {
        repo: "api".into(),
        path: path.into(),
        mode,
    }
}

#[test]
fn a_picked_mode_is_kept_for_that_file_and_session_only() {
    let mut store = store();
    let ledger = SessionId::new("ledger");
    store.send(mode("README.md", DocumentMode::SideBySide), |s| {
        s.views.get_mut(&ledger).unwrap().set_document(
            "api",
            "README.md",
            DocumentView {
                mode: DocumentMode::SideBySide,
                linked: true,
            },
        );
    });
    let state = store.state();
    let view = &state.views[&ledger];
    assert_eq!(
        view.document("api", "README.md").mode,
        DocumentMode::SideBySide
    );
    assert_eq!(
        view.document("api", "docs/GUIDE.md").mode,
        DocumentMode::Preview
    );
    assert_eq!(
        state.views[&SessionId::new("other")]
            .document("api", "README.md")
            .mode,
        DocumentMode::Preview
    );
}

#[test]
fn link_scrolling_toggles_and_keeps_the_mode() {
    let mut store = store();
    let id = SessionId::new("ledger");
    store.send(mode("README.md", DocumentMode::Source), |s| {
        s.views.get_mut(&id).unwrap().set_document(
            "api",
            "README.md",
            DocumentView {
                mode: DocumentMode::Source,
                linked: true,
            },
        );
    });
    let unlink = |linked| A::LinkScrolling {
        repo: "api".into(),
        path: "README.md".into(),
        linked,
    };
    store.send(unlink(false), |s| {
        s.views.get_mut(&id).unwrap().set_document(
            "api",
            "README.md",
            DocumentView {
                mode: DocumentMode::Source,
                linked: false,
            },
        );
    });
    assert!(!store.state().views[&id].document("api", "README.md").linked);
    store.send(unlink(true), |s| {
        s.views.get_mut(&id).unwrap().set_document(
            "api",
            "README.md",
            DocumentView {
                mode: DocumentMode::Source,
                linked: true,
            },
        );
    });
    let view = store.state().views[&id].document("api", "README.md");
    assert!(view.linked);
    assert_eq!(view.mode, DocumentMode::Source);
}

#[test]
fn picking_the_defaults_again_leaves_the_view_as_it_began() {
    let mut store = store();
    let id = SessionId::new("ledger");
    store.send(mode("README.md", DocumentMode::Source), |s| {
        s.views.get_mut(&id).unwrap().set_document(
            "api",
            "README.md",
            DocumentView {
                mode: DocumentMode::Source,
                linked: true,
            },
        );
    });
    store.send(mode("README.md", DocumentMode::Preview), |s| {
        s.views.insert(id.clone(), SessionView::default());
    });
}

#[test]
fn a_view_saved_before_documents_loads_with_defaults() {
    let saved = r#"{"tab":0,"panel":null,"widened":false,"draft":"","queue":[]}"#;
    let view: SessionView = serde_json::from_str(saved).unwrap();
    assert_eq!(view.document("api", "README.md"), DocumentView::default());
    assert_eq!(DocumentView::default().mode, DocumentMode::Preview);
    assert!(DocumentView::default().linked);
}

#[test]
fn settings_survive_saving_and_loading() {
    let mut view = SessionView::default();
    let picked = DocumentView {
        mode: DocumentMode::Source,
        linked: false,
    };
    view.set_document("api", "README.md", picked);
    let json = serde_json::to_string(&view).unwrap();
    let back: SessionView = serde_json::from_str(&json).unwrap();
    assert_eq!(back.document("api", "README.md"), picked);
}

#[test]
fn markdown_files_are_found_by_extension_in_any_case() {
    for path in ["a.md", "docs/B.MD", "c.markdown", "d.Markdown"] {
        assert!(is_markdown(path), "{path}");
    }
    for path in ["a.rs", "md", "notes.md.txt", "a.mdx", ""] {
        assert!(!is_markdown(path), "{path}");
    }
}

fn doc_tab() -> Tab {
    Tab::File {
        repo: "api".into(),
        path: "docs/RFC.md".into(),
    }
}

/// A session with both terminals and a document in one group, the document
/// selected.
fn one_group() -> TestStore<Sessions> {
    let workspace = fixture::design();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        ..Default::default()
    };
    state.open = Some(SessionId::new("ledger"));
    state.views.insert(
        SessionId::new("ledger"),
        SessionView {
            groups: vec![TabGroup {
                tabs: vec![Tab::Terminal(0), Tab::Terminal(1), doc_tab()],
                selected: 2,
            }],
            ..Default::default()
        },
    );
    TestStore::new(state, Sessions::new(clients))
}

#[test]
fn moving_the_document_beside_the_terminals_and_back() {
    let mut store = one_group();
    let id = SessionId::new("ledger");
    store.send(A::MoveTab { group: 0, index: 2 }, |s| {
        let view = s.views.get_mut(&id).unwrap();
        view.groups = vec![
            TabGroup {
                tabs: vec![Tab::Terminal(0), Tab::Terminal(1)],
                selected: 1,
            },
            TabGroup {
                tabs: vec![doc_tab()],
                selected: 0,
            },
        ];
        view.focused_group = 1;
    });
    let view = store.state().view().unwrap().clone();
    assert_eq!(view.selected(1), Some(&doc_tab()));
    assert_eq!(view.focused_group, 1);

    store.send(A::MoveTab { group: 1, index: 0 }, |s| {
        let view = s.views.get_mut(&id).unwrap();
        view.groups = vec![TabGroup {
            tabs: vec![Tab::Terminal(0), Tab::Terminal(1), doc_tab()],
            selected: 2,
        }];
        view.focused_group = 0;
    });
    store.finish();
}

#[test]
fn a_lone_tab_in_the_only_group_does_not_move() {
    let mut store = one_group();
    let id = SessionId::new("ledger");
    store.send(A::CloseTab { group: 0, index: 0 }, |s| {
        let view = s.views.get_mut(&id).unwrap();
        view.groups[0].tabs.remove(0);
        view.groups[0].selected = 1;
    });
    store.send(A::CloseTab { group: 0, index: 0 }, |s| {
        let view = s.views.get_mut(&id).unwrap();
        view.groups[0].tabs.remove(0);
        view.groups[0].selected = 0;
        view.tab = 0;
    });
    // Only the document is left in the one group: nothing to split from.
    store.send(A::MoveTab { group: 0, index: 0 }, |_| {});
    store.send(A::MoveTab { group: 0, index: 5 }, |_| {});
    store.finish();
}
