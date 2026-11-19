//! uxsimple 3-4: "N agents" counts the session's own agent and each tab
//! running one, those opened from New agent tab included, each once.
use humata2_clients::fixture;
use humata2_clients::model::{Agent, Session, SessionId};
use humata2_session::SessionState;

fn ledger() -> Session {
    let mut session = fixture::design()
        .session(&SessionId::new("ledger"))
        .cloned()
        .expect("the design fixture lists ledger");
    session.agent = Agent::Claude;
    session.terminal_agents.clear();
    session
}

#[test]
fn a_session_with_no_agent_tabs_runs_its_own_agent() {
    assert_eq!(SessionState::default().agent_count(&ledger()), 1);
}

#[test]
fn an_agent_tab_opened_from_new_agent_tab_is_counted() {
    let mut state = SessionState::default();
    state.agent_tabs.insert((SessionId::new("ledger"), 2), Agent::Copilot);
    assert_eq!(state.agent_count(&ledger()), 2);
}

#[test]
fn an_agent_tab_the_daemon_also_lists_is_counted_once() {
    let mut state = SessionState::default();
    state.agent_tabs.insert((SessionId::new("ledger"), 2), Agent::Copilot);
    let mut session = ledger();
    session.terminal_agents.insert(2, Agent::Copilot);
    session.terminal_agents.insert(3, Agent::Codex);
    assert_eq!(state.agent_count(&session), 3);
}

#[test]
fn another_sessions_agent_tab_is_not_counted() {
    let mut state = SessionState::default();
    state.agent_tabs.insert((SessionId::new("other"), 2), Agent::Copilot);
    assert_eq!(state.agent_count(&ledger()), 1);
}

#[test]
fn a_plain_shell_session_counts_only_its_agent_tabs() {
    let mut state = SessionState::default();
    state.agent_tabs.insert((SessionId::new("ledger"), 1), Agent::Codex);
    let mut session = ledger();
    session.agent = Agent::Shell;
    assert_eq!(state.agent_count(&session), 1);
}
