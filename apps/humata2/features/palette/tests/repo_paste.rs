//! RL-5, KP-4: pasting a repo (owner/name or a GitHub address) into the
//! palette offers to add it to the channel on screen, and lists the sessions
//! that already work in it. A pull request address is not a repo here.
use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::fixture;
use humata2_clients::model::{ChannelId, SessionId};
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState};
use tca::Reducer;

fn opened(workspace: Workspace, channel: Option<ChannelId>) -> PaletteState {
    let mut state = PaletteState {
        workspace: Arc::new(workspace),
        channel,
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: false });
    state
}

fn commands(state: &PaletteState) -> Vec<Command> {
    state.entries.iter().map(|entry| entry.command.clone()).collect()
}

fn adds(state: &PaletteState) -> Vec<(ChannelId, String)> {
    commands(state)
        .into_iter()
        .filter_map(|command| match command {
            Command::AddRepoToChannel { channel, repo } => Some((channel, repo)),
            _ => None,
        })
        .collect()
}

fn channel() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

#[test]
fn a_pasted_repo_offers_to_add_it_to_the_channel_on_screen() {
    for pasted in [
        "acme/ledger",
        "https://github.com/acme/ledger",
        "https://github.com/acme/ledger.git",
    ] {
        let mut state = opened(fixture::design(), Some(channel()));
        let _ = Palette.reduce(&mut state, A::Query(pasted.to_owned()));
        assert_eq!(
            adds(&state),
            vec![(channel(), "acme/ledger".to_owned())],
            "{pasted}"
        );
        assert_eq!(
            commands(&state).first(),
            Some(&Command::AddRepoToChannel {
                channel: channel(),
                repo: "acme/ledger".to_owned()
            }),
            "{pasted}: it ranks first"
        );
    }
}

#[test]
fn without_a_channel_on_screen_nothing_is_offered() {
    let mut state = opened(fixture::design(), None);
    let _ = Palette.reduce(&mut state, A::Query("acme/ledger".to_owned()));
    assert!(adds(&state).is_empty());
}

#[test]
fn text_that_is_not_a_repo_and_pull_requests_offer_no_add() {
    for typed in ["settings", "move", "https://github.com/acme/ledger/pull/12", "acme/ledger#12"] {
        let mut state = opened(fixture::design(), Some(channel()));
        let _ = Palette.reduce(&mut state, A::Query(typed.to_owned()));
        assert!(adds(&state).is_empty(), "{typed}: {:?}", adds(&state));
    }
}

#[test]
fn sessions_already_in_the_repo_are_offered_to_open() {
    let mut workspace = fixture::design();
    let id = SessionId::new("refund");
    for session in workspace.sessions.iter_mut().filter(|s| s.id == id) {
        "Acme/Ledger".clone_into(&mut session.repo);
    }
    let mut state = opened(workspace, Some(channel()));
    let _ = Palette.reduce(&mut state, A::Query("acme/ledger".to_owned()));
    assert!(commands(&state).contains(&Command::Session(id)));
}
