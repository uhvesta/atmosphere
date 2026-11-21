//! Channel Desk To-do List-only split pane state tests (DESKTODO).
use std::sync::Arc;

use humata2_clients::model::ChannelId;
use humata2_clients::{FakeDaemon, Workspace, fixture};
use humata2_desk::{
    Desk, DeskAction as A, DeskState, TODO_PANE_DEFAULT_WIDTH, TODO_PANE_MAX_WIDTH,
    TODO_PANE_MIN_WIDTH, clamp_todo_pane_width,
};
use tca::{TestClock, TestStore};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn ops() -> ChannelId {
    ChannelId::new("ops-incidents")
}

fn store(workspace: Workspace) -> TestStore<Desk> {
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    TestStore::new(state, Desk::new(Arc::new(fake))).non_exhaustive()
}

#[test]
fn todo_pane_toggle_width_clamping_and_per_channel_memory() {
    let mut store = store(fixture::design());

    assert_eq!(clamp_todo_pane_width(100.0), TODO_PANE_MIN_WIDTH);
    assert_eq!(clamp_todo_pane_width(600.0), TODO_PANE_MAX_WIDTH);
    assert_eq!(clamp_todo_pane_width(400.0), 400.0);

    let default_pane = store.state().todo_pane(&payments());
    assert!(!default_pane.visible);
    assert_eq!(default_pane.width, TODO_PANE_DEFAULT_WIDTH);

    // Resize payments to 420, ops to 200 (clamped to 320).
    store.send(
        A::SetTodoPaneWidth {
            channel: payments(),
            width: 420.0,
        },
        |_| {},
    );
    store.send(
        A::SetTodoPaneWidth {
            channel: ops(),
            width: 200.0,
        },
        |_| {},
    );
    store.send(A::ToggleTodoPane(payments()), |_| {});

    assert!(store.state().todo_pane(&payments()).visible);
    assert_eq!(store.state().todo_pane(&payments()).width, 420.0);
    assert!(!store.state().todo_pane(&ops()).visible);
    assert_eq!(store.state().todo_pane(&ops()).width, TODO_PANE_MIN_WIDTH);

    // Search toggle clears query when closing; done toggle flips done_expanded.
    store.send(A::ToggleTodoPaneSearch(payments()), |_| {});
    store.send(
        A::SetTodoPaneSearchQuery {
            channel: payments(),
            query: "stripe".to_owned(),
        },
        |_| {},
    );
    assert!(store.state().todo_pane(&payments()).search_open);
    assert_eq!(store.state().todo_pane(&payments()).search_query, "stripe");
    store.send(A::ToggleTodoPaneSearch(payments()), |_| {});
    assert!(!store.state().todo_pane(&payments()).search_open);
    assert!(store.state().todo_pane(&payments()).search_query.is_empty());

    store.send(A::ToggleTodoPaneDone(payments()), |_| {});
    assert!(store.state().todo_pane(&payments()).done_expanded);
}

#[test]
fn unseen_agent_changes_increment_only_while_hidden_and_reset_on_open() {
    let mut store = store(fixture::design());

    // Hidden initially: agent board changes increment unseen count.
    store.send(
        A::RecordAgentBoardChange {
            channel: payments(),
            count: 2,
        },
        |_| {},
    );
    store.send(
        A::RecordAgentBoardChange {
            channel: payments(),
            count: 3,
        },
        |_| {},
    );
    assert_eq!(store.state().todo_pane(&payments()).unseen_agent_changes, 5);

    // Opening the pane resets unseen_agent_changes to 0.
    store.send(A::ToggleTodoPane(payments()), |_| {});
    assert!(store.state().todo_pane(&payments()).visible);
    assert_eq!(store.state().todo_pane(&payments()).unseen_agent_changes, 0);

    // While visible, agent board changes are a no-op.
    store.send(
        A::RecordAgentBoardChange {
            channel: payments(),
            count: 4,
        },
        |_| {},
    );
    assert_eq!(store.state().todo_pane(&payments()).unseen_agent_changes, 0);

    // Closing the pane allows unseen changes to accumulate again.
    store.send(A::ToggleTodoPane(payments()), |_| {});
    assert!(!store.state().todo_pane(&payments()).visible);
    store.send(
        A::RecordAgentBoardChange {
            channel: payments(),
            count: 1,
        },
        |_| {},
    );
    assert_eq!(store.state().todo_pane(&payments()).unseen_agent_changes, 1);
}
