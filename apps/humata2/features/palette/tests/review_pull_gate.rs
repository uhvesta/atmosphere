//! KP-4/PR-1 (D197 reversed): a pasted pull request, as a link or as
//! `owner/repo#n`, offers Review this PR at once, built from the parsed text
//! with no daemon call, whether or not a session reviews it. Choosing it
//! hands the pull request to the app, which opens the session that reviews
//! it or the Review a pull request dialog. Text that isn't a pull request
//! offers no review.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::fixture;
use humata2_palette::{Command, Delegate, Palette, PaletteAction as A, PaletteState, PullRef};
use tca::TestStore;

fn store() -> TestStore<Palette> {
    let state = PaletteState {
        workspace: Arc::new(fixture::design()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette)
        .non_exhaustive()
        .with_timeout(Duration::from_millis(100));
    store.send(A::Open { in_session: false }, |s| s.open = true);
    store
}

fn typed(store: &mut TestStore<Palette>, query: &str) {
    store.send(A::Query(query.to_owned()), |s| s.query = query.to_owned());
}

fn reviews(store: &TestStore<Palette>) -> Vec<Command> {
    store
        .state()
        .entries
        .iter()
        .filter(|e| matches!(e.command, Command::ReviewPull(_)))
        .map(|e| e.command.clone())
        .collect()
}

/// The two ways a pull request is pasted: its link and `owner/repo#n`.
fn pasted_forms(pull: &PullRef) -> [String; 2] {
    [
        format!("https://github.com/{}/{}/pull/{}", pull.owner, pull.repo, pull.number),
        format!("{}/{}#{}", pull.owner, pull.repo, pull.number),
    ]
}

/// A pull request the design fixture does not list.
fn unlisted() -> PullRef {
    PullRef {
        owner: "octo".to_owned(),
        repo: "unlisted".to_owned(),
        number: 7,
    }
}

/// A pull request a session in the design fixture reviews.
fn reviewed() -> PullRef {
    let workspace = fixture::design();
    let Some(pull) = workspace.pulls.iter().find(|p| p.session.is_some()) else {
        panic!("the design fixture has a pull request under review");
    };
    let Some((owner, repo)) = pull.repo.split_once('/') else {
        panic!("{} is owner/repo", pull.repo);
    };
    PullRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number: pull.number,
    }
}

#[test]
fn a_pasted_pull_request_offers_review_at_once() {
    for pull in [unlisted(), reviewed()] {
        for pasted in pasted_forms(&pull) {
            let mut store = store();
            typed(&mut store, &pasted);
            assert_eq!(
                reviews(&store),
                vec![Command::ReviewPull(pull.clone())],
                "{pasted}"
            );
        }
    }
}

#[test]
fn choosing_a_pasted_pull_request_hands_it_to_the_app() {
    let pull = unlisted();
    let mut store = store();
    typed(&mut store, &pasted_forms(&pull)[0]);
    store.send(A::Run, |s| s.open = false);
    store.receive(A::Delegate(Delegate::Closed), |_| {});
    store.receive(A::Delegate(Delegate::Run(Command::ReviewPull(pull))), |_| {});
}

#[test]
fn text_that_is_not_a_pull_request_offers_no_review() {
    for text in [
        "https://github.com/octo/unlisted",
        "https://github.com/octo/unlisted/issues/7",
        "https://github.com/octo/unlisted/pull/seven",
        "https://example.com/octo/unlisted/pull/7",
        "octo/unlisted",
        "octo/unlisted#seven",
    ] {
        let mut store = store();
        typed(&mut store, text);
        assert!(reviews(&store).is_empty(), "{text}");
    }
}
