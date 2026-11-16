//! Join tab groups is offered only when the open session has two groups.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::SessionId;
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState};
use tca::Reducer;

fn opened(split: bool) -> PaletteState {
    let mut state = PaletteState {
        workspace: Arc::new(fixture::design()),
        context: Some(SessionId::new("ledger")),
        split,
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: true });
    state
}

fn offered(state: &PaletteState) -> bool {
    state.entries.iter().any(|e| e.command == Command::JoinTabGroups)
}

#[test]
fn two_groups_offer_joining_them() {
    assert!(offered(&opened(true)));
}

#[test]
fn one_group_does_not() {
    assert!(!offered(&opened(false)));
}
