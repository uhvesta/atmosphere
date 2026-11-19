//! The footer's Push & PR menu and the base menus are never open together,
//! and toggling the push menu twice returns to closed.
use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{Reducer, TestClock};

/// The review page with a session open and every menu closed.
fn page() -> ReviewState {
    ReviewState {
        session: Some(SessionId::new("ledger")),
        ..ReviewState::default()
    }
}

/// Sends `action` to the review reducer, as the store would.
fn send(state: &mut ReviewState, action: A) {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(Workspace::default(), clock.clone());
    let review = Review::new(Arc::new(daemon), clock);
    let _effect = review.reduce(state, action);
}

#[test]
fn toggling_the_push_menu_twice_returns_to_closed() {
    let mut state = page();
    send(&mut state, A::TogglePushMenu);
    assert!(state.push_menu);
    send(&mut state, A::TogglePushMenu);
    assert!(!state.push_menu);
}

#[test]
fn closing_the_push_menu_closes_it() {
    let mut state = page();
    send(&mut state, A::TogglePushMenu);
    send(&mut state, A::ClosePushMenu);
    assert!(!state.push_menu);
}

#[test]
fn opening_the_base_menu_closes_the_push_menu() {
    let mut state = page();
    send(&mut state, A::TogglePushMenu);
    send(&mut state, A::ToggleBaseMenu);
    assert!(state.base_menu);
    assert!(!state.push_menu);
}

#[test]
fn opening_the_push_menu_closes_the_base_menu() {
    let mut state = page();
    send(&mut state, A::ToggleBaseMenu);
    send(&mut state, A::TogglePushMenu);
    assert!(state.push_menu);
    assert!(!state.base_menu);
}

/// UXSIMPLE 3-3: the New agent tab menu opens alone, and a pick closes it
/// and is remembered as last used.
#[test]
fn the_new_agent_tab_menu_opens_alone_and_remembers_the_pick() {
    use humata2_clients::model::Agent;
    let mut state = page();
    send(&mut state, A::TogglePushMenu);
    send(&mut state, A::ToggleAgentTabMenu);
    assert!(state.agent_tab_menu);
    assert!(!state.push_menu);
    send(&mut state, A::ToggleBaseMenu);
    assert!(!state.agent_tab_menu, "another menu closes it");
    send(&mut state, A::ToggleAgentTabMenu);
    send(&mut state, A::AgentTabPicked(Agent::Copilot));
    assert!(!state.agent_tab_menu);
    assert_eq!(state.last_agent_tab, Some(Agent::Copilot));
    send(&mut state, A::ToggleAgentTabMenu);
    send(&mut state, A::ToggleAgentTabMenu);
    assert!(!state.agent_tab_menu);
}
