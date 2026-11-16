//! KP-4/PR-1 until the Review a pull request dialog exists (D-provisional,
//! FIND): a pasted pull request offers Review this PR only when a session
//! here already reviews it, since that is the only thing it can do. Other
//! pull requests offer nothing, so no entry leads to a dead end.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState, PullRef};
use tca::Reducer;

fn opened() -> PaletteState {
    let mut state = PaletteState {
        workspace: Arc::new(fixture::design()),
        ..Default::default()
    };
    let _ = Palette.reduce(&mut state, A::Open { in_session: false });
    state
}

fn reviews(state: &PaletteState) -> Vec<Command> {
    state
        .entries
        .iter()
        .filter(|e| matches!(e.command, Command::ReviewPull(_)))
        .map(|e| e.command.clone())
        .collect()
}

#[test]
fn a_pull_request_a_session_reviews_is_offered_first() {
    let workspace = fixture::design();
    let Some(pull) = workspace.pulls.iter().find(|p| p.session.is_some()) else {
        panic!("the design workspace has a pull request under review");
    };
    let (owner, repo) = pull
        .repo
        .split_once('/')
        .unwrap_or_else(|| panic!("{} is owner/repo", pull.repo));
    let expected = Command::ReviewPull(PullRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number: pull.number,
    });
    for pasted in [
        format!("https://github.com/{}/pull/{}", pull.repo, pull.number),
        format!("{}#{}", pull.repo, pull.number),
    ] {
        let mut state = opened();
        let _ = Palette.reduce(&mut state, A::Query(pasted.clone()));
        assert_eq!(
            state.entries.first().map(|e| e.command.clone()),
            Some(expected.clone()),
            "{pasted}"
        );
    }
}

#[test]
fn a_pull_request_no_session_reviews_offers_no_review() {
    for pasted in ["https://github.com/o/r/pull/123", "o/r#123"] {
        let mut state = opened();
        let _ = Palette.reduce(&mut state, A::Query(pasted.to_owned()));
        assert!(reviews(&state).is_empty(), "{pasted}: {:?}", reviews(&state));
    }
}
