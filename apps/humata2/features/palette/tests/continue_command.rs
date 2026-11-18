//! "Continue in another channel…" in the palette (CV-8): offered over a
//! channel session, not over a Desk session, an archived one, or no session.
use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Workspace, fixture};
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState};
use tca::Reducer;

fn opened(workspace: Workspace, session: Option<&str>) -> PaletteState {
    let mut state = PaletteState {
        workspace: Arc::new(workspace),
        context: session.map(SessionId::new),
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: session.is_some() });
    state
}

fn offers(state: &PaletteState) -> bool {
    state
        .entries
        .iter()
        .any(|e| e.command == Command::ContinueInChannel)
}

#[test]
fn a_channel_session_offers_continue_in_another_channel() {
    let mut state = opened(fixture::design(), Some("ledger"));
    assert!(offers(&state));
    let _ = Palette.reduce(&mut state, A::Query("continue".to_owned()));
    assert_eq!(
        state.entries.first().map(|e| e.command.clone()),
        Some(Command::ContinueInChannel),
        "typing its name puts it first"
    );
}

#[test]
fn a_desk_session_is_not_offered_continue() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    assert!(!offers(&opened(workspace, Some("desk-plan-q4"))));
}

#[test]
fn an_archived_session_is_not_offered_continue() {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        if session.id == SessionId::new("ledger") {
            session.archived = true;
        }
    }
    assert!(!offers(&opened(workspace, Some("ledger"))));
}

#[test]
fn outside_a_session_it_is_not_offered() {
    assert!(!offers(&opened(fixture::design(), None)));
}
