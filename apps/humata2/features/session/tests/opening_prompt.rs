//! D857: an agent picked from New agent tab opens as a new tab, which types
//! the agent's command and then one prompt once each has painted and gone
//! quiet, and never types over the person.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{Agent, SessionId};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_session::{SessionAction as A, SessionState, Sessions};
use tca::{TestClock, TestStore};

const PROMPT: &str = "Look at the changes in the Diff panel. Don't edit files. Leave what you find on the lines.";

struct Rig {
    store: TestStore<Sessions>,
    fakes: Fakes,
    clock: TestClock,
}

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn rig() -> Rig {
    let mut workspace = fixture::design();
    for s in &mut workspace.sessions {
        s.unread = 0;
        s.recovery = None;
    }
    let clock = TestClock::new();
    let (clients, fakes): (Clients, Fakes) =
        humata2_clients::fake_clients(workspace.clone(), Arc::new(clock.clone()));
    let state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(ledger()),
        ..Default::default()
    };
    let store = TestStore::new(state, Sessions::new(clients)).non_exhaustive();
    Rig { store, fakes, clock }
}

impl Rig {
    /// Picks `agent` and waits for its tab to open; returns the tab.
    fn pick(&mut self, agent: Agent) -> usize {
        self.store.send(
            A::OpenAgentTab {
                agent,
                prompt: PROMPT.to_owned(),
            },
            |_| {},
        );
        self.store
            .receive_matching("the tab opening", |a| matches!(a, A::TerminalOpened { .. }), |_| {});
        self.store.settle();
        self.store.state().view().map_or(0, |v| v.tab)
    }

    fn later(&mut self, by: Duration) {
        self.clock.advance(by);
        self.store.settle();
    }

    /// What was typed into tabs by the new-tab sequence, in order.
    fn typed(&self) -> Vec<String> {
        self.fakes
            .daemon
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("send_to_tab"))
            .collect()
    }
}

const QUIET: Duration = Duration::from_millis(500);

#[test]
fn a_picked_agent_is_started_and_then_given_the_prompt_once_each_is_quiet() {
    let mut rig = rig();
    let tab = rig.pick(Agent::Codex);
    assert!(tab > 0);
    assert!(rig.typed().is_empty(), "nothing is typed before the shell is quiet");

    rig.later(QUIET);
    let typed = rig.typed();
    assert_eq!(typed.len(), 1, "{typed:?}");
    assert!(typed[0].contains(&format!(" {tab} ")) && typed[0].contains("codex"), "{typed:?}");

    rig.later(QUIET);
    let typed = rig.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
    assert!(typed[1].contains(PROMPT), "{typed:?}");
    rig.store.finish();
}

#[test]
fn the_sessions_own_agent_still_opens_a_new_tab_with_the_command_then_the_prompt() {
    let mut rig = rig();
    let own = rig
        .store
        .state()
        .workspace
        .session(&ledger())
        .map(|s| (s.agent, s.shells.len()))
        .expect("the ledger session exists");
    let tab = rig.pick(own.0);
    assert_eq!(tab, own.1 + 1, "a new tab, not an attach to the agent's");
    rig.later(QUIET);
    rig.later(QUIET);
    let typed = rig.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
    assert!(typed[0].contains(own.0.command()), "{typed:?}");
    assert!(typed[1].contains(PROMPT), "{typed:?}");
}

#[test]
fn the_person_typing_first_means_nothing_is_typed() {
    let mut rig = rig();
    let tab = rig.pick(Agent::Codex);
    rig.store
        .send(A::PersonTyped { session: ledger(), tab }, |_| {});
    rig.later(QUIET);
    rig.later(QUIET);
    assert!(rig.typed().is_empty(), "{:?}", rig.typed());
    rig.store.finish();
}

#[test]
fn the_person_typing_after_the_command_stops_the_prompt() {
    let mut rig = rig();
    let tab = rig.pick(Agent::Codex);
    rig.later(QUIET);
    assert_eq!(rig.typed().len(), 1);
    rig.store
        .send(A::PersonTyped { session: ledger(), tab }, |_| {});
    rig.later(QUIET);
    rig.later(QUIET);
    assert_eq!(rig.typed().len(), 1, "{:?}", rig.typed());
}

#[test]
fn nothing_is_typed_once_the_deadline_has_passed() {
    let mut rig = rig();
    rig.pick(Agent::Codex);
    rig.later(Duration::from_secs(16));
    assert!(rig.typed().is_empty(), "{:?}", rig.typed());
}

#[test]
fn a_plain_terminal_never_gets_a_prompt() {
    let mut rig = rig();
    rig.store.send(
        A::OpenTerminal {
            agent: Agent::Shell,
            split: false,
        },
        |_| {},
    );
    rig.store
        .receive_matching("the tab opening", |a| matches!(a, A::TerminalOpened { .. }), |_| {});
    rig.store.send(A::NewTerminalTab, |_| {});
    rig.store
        .receive_matching("the tab opening", |a| matches!(a, A::TerminalOpened { .. }), |_| {});
    rig.later(QUIET);
    rig.later(QUIET);
    assert!(rig.typed().is_empty(), "{:?}", rig.typed());
}
