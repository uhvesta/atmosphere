//! Rename and Stop the agent from the palette, over the session it opened on
//! (SE-7, FC-17).
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::SessionId;
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState};
use tca::Reducer;

fn opened(session: &str) -> PaletteState {
    let mut state = PaletteState {
        workspace: Arc::new(fixture::design()),
        context: Some(SessionId::new(session)),
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: true });
    state
}

fn offers(state: &PaletteState, command: &Command) -> bool {
    state.entries.iter().any(|e| &e.command == command)
}

#[test]
fn a_running_session_offers_rename_and_stop() {
    let state = opened("ledger");
    assert!(offers(&state, &Command::RenameSession));
    assert!(offers(&state, &Command::StopAgent));
}

#[test]
fn an_exited_agent_is_not_offered_stop() {
    let state = opened("postgres");
    assert!(offers(&state, &Command::RenameSession));
    assert!(!offers(&state, &Command::StopAgent));
}

#[test]
fn outside_a_session_neither_is_offered() {
    let mut state = PaletteState {
        workspace: Arc::new(fixture::design()),
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: false });
    assert!(!offers(&state, &Command::RenameSession));
    assert!(!offers(&state, &Command::StopAgent));
}
