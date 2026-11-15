//! SE-8/9/13 and D76: file/terminal tabs, two groups, no duplicate agent.
use humata2_clients::{
    Clients, fixture,
    model::{Agent, SessionId},
};
use humata2_session::{SessionAction as A, SessionState, SessionView, Sessions, Tab, TabGroup};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> (TestStore<Sessions>, Clients) {
    let mut workspace = fixture::design();
    for s in &mut workspace.sessions {
        s.unread = 0;
        s.recovery = None;
    }
    let (clients, _) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        ..Default::default()
    };
    state.open = Some(SessionId::new("ledger"));
    state.views.insert(
        SessionId::new("ledger"),
        SessionView {
            groups: vec![TabGroup {
                tabs: vec![Tab::Terminal(0), Tab::Terminal(1)],
                selected: 0,
            }],
            ..Default::default()
        },
    );
    (
        TestStore::new(state, Sessions::new(clients.clone())),
        clients,
    )
}

#[test]
fn a_file_opens_once_and_closing_it_returns_to_the_terminal() {
    let (mut store, _) = store();
    let id = SessionId::new("ledger");
    store.send(
        A::SelectFile {
            repo: "api".into(),
            path: "src/client.rs".into(),
        },
        |s| {
            s.views.get_mut(&id).unwrap().file = Some(("api".into(), "src/client.rs".into()));
        },
    );
    let file = Tab::File {
        repo: "api".into(),
        path: "src/client.rs".into(),
    };
    store.send(A::OpenFile, |s| {
        let g = &mut s.views.get_mut(&id).unwrap().groups[0];
        g.tabs.push(file.clone());
        g.selected = 2;
    });
    let text = store.state().workspace.file_contents
        [&(id.clone(), "api".into(), "src/client.rs".into())]
        .clone();
    let loaded = A::FileLoaded {
        session: id.clone(),
        repo: "api".into(),
        path: "src/client.rs".into(),
        result: Ok(text.clone()),
    };
    store.receive(loaded.clone(), |s| {
        s.file_contents
            .insert((id.clone(), "api".into(), "src/client.rs".into()), Ok(text));
    });
    store.send(A::OpenFile, |_| {});
    store.receive(loaded, |_| {});
    assert_eq!(store.state().view().unwrap().groups[0].tabs.len(), 3);
    store.send(A::CloseTab { group: 0, index: 2 }, |s| {
        let g = &mut s.views.get_mut(&id).unwrap().groups[0];
        g.tabs.pop();
        g.selected = 1;
        s.views.get_mut(&id).unwrap().tab = 1;
    });
    assert_eq!(
        store.state().view().unwrap().selected(0),
        Some(&Tab::Terminal(1))
    );
    store.finish();
}

#[test]
fn split_is_limited_to_two_groups_and_attaches_an_existing_agent() {
    let (mut store, clients) = store();
    let id = SessionId::new("ledger");
    let mut snapshot = store.state().workspace.session(&id).unwrap().clone();
    snapshot.shells.push("codex".into());
    snapshot.terminal_agents.insert(2, Agent::Codex);
    store.send(
        A::OpenTerminal {
            agent: Agent::Codex,
            split: true,
        },
        |s| s.opening_terminal = true,
    );
    let opened = A::TerminalOpened {
        session: id.clone(),
        group: 0,
        split: true,
        result: Ok((2, snapshot.clone())),
    };
    store.receive(opened, |s| {
        s.opening_terminal = false;
        *Arc::make_mut(&mut s.workspace)
            .sessions
            .iter_mut()
            .find(|v| v.id == id)
            .unwrap() = snapshot.clone();
        let v = s.views.get_mut(&id).unwrap();
        v.groups.push(TabGroup {
            tabs: vec![Tab::Terminal(2)],
            selected: 0,
        });
        v.focused_group = 1;
        v.tab = 2;
    });
    store.send(
        A::OpenTerminal {
            agent: Agent::Codex,
            split: true,
        },
        |s| s.opening_terminal = true,
    );
    store.receive(
        A::TerminalOpened {
            session: id.clone(),
            group: 1,
            split: true,
            result: Ok((2, snapshot)),
        },
        |s| s.opening_terminal = false,
    );
    assert_eq!(store.state().view().unwrap().groups.len(), 2);
    assert_eq!(
        store.state().workspace.session(&id).unwrap().shells.len(),
        2
    );
    store.send(A::CloseGroup(1), |s| {
        let v = s.views.get_mut(&id).unwrap();
        let group = v.groups.pop().unwrap();
        v.groups[0].tabs.extend(group.tabs);
        v.focused_group = 0;
        v.tab = 0;
    });
    let saved = store.state().views.clone();
    store.send(A::Restore(saved), |_| {});
    assert_eq!(store.state().view().unwrap().groups.len(), 1);
    assert_eq!(store.state().view().unwrap().groups[0].tabs.len(), 3);
    drop(clients);
    store.finish();
}

#[test]
fn closing_the_only_file_in_a_split_removes_that_group() {
    let (base, clients) = store();
    let mut state = base.state().clone();
    base.finish();
    let id = SessionId::new("ledger");
    let v = state.views.get_mut(&id).unwrap();
    v.groups.push(TabGroup {
        tabs: vec![Tab::File {
            repo: "api".into(),
            path: "src/client.rs".into(),
        }],
        selected: 0,
    });
    v.focused_group = 1;
    let mut store = TestStore::new(state, Sessions::new(clients));
    store.send(A::CloseTab { group: 1, index: 0 }, |s| {
        let v = s.views.get_mut(&id).unwrap();
        v.groups.pop();
        v.focused_group = 0;
    });
    assert_eq!(
        store.state().view().unwrap().selected(0),
        Some(&Tab::Terminal(0))
    );
    store.finish();
}

/// SE-13: clicking a visible terminal focuses its group without selecting hidden tabs.
#[test]
fn terminal_focus_tracks_only_the_visible_tab_in_its_own_session() {
    let (base, clients) = store();
    let mut state = base.state().clone();
    base.finish();
    let id = SessionId::new("ledger");
    let view = state.views.get_mut(&id).unwrap();
    view.groups = vec![
        TabGroup {
            tabs: vec![Tab::Terminal(0), Tab::Terminal(1)],
            selected: 0,
        },
        TabGroup {
            tabs: vec![Tab::Terminal(2)],
            selected: 0,
        },
    ];
    view.focused_group = 0;
    let mut store = TestStore::new(state, Sessions::new(clients));
    store.send(
        A::FocusTerminal {
            session: SessionId::new("another"),
            tab: 2,
        },
        |_| {},
    );
    store.send(
        A::FocusTerminal {
            session: id.clone(),
            tab: 1,
        },
        |_| {},
    );
    store.send(
        A::FocusTerminal {
            session: id.clone(),
            tab: 2,
        },
        |s| {
            let view = s.views.get_mut(&id).unwrap();
            view.focused_group = 1;
            view.tab = 2;
        },
    );
    store.send(
        A::FocusTerminal {
            session: id.clone(),
            tab: 0,
        },
        |s| {
            let view = s.views.get_mut(&id).unwrap();
            view.focused_group = 0;
            view.tab = 0;
        },
    );
    assert_eq!(store.state().view().unwrap().groups[0].selected, 0);
    store.finish();
}
