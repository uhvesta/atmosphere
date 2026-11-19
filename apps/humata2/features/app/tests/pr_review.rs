//! PR-1, PR-10, PR-15 on the fakes: a pasted pull request that is Started
//! opens its session, the new-commits feed is followed from app start-up, and
//! a review session that opens has its chat followed (once, and only for
//! review sessions). Choosing Review this PR in the palette opens the session
//! that already reviews the pull request, or pastes it into the dialog.
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::fixture;
use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::pull_requests::{PullRequestInfo, PullRequestRef, PullRequestState};
use humata2_clients::{Clients, Workspace};
use humata2_palette::{Command, PaletteAction, PullRef};
use humata2_pr_review::dialog::{DialogAction, Fetch};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn info() -> PullRequestInfo {
    PullRequestInfo {
        pr: PullRequestRef::new("acme/api", 482),
        title: "Change 482".to_owned(),
        author: "mara".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: "change-482".to_owned(),
        head_sha: "abc123".to_owned(),
        files: 3,
        added: 40,
        removed: 8,
        state: PullRequestState::Open,
        checks: "3 passed".to_owned(),
    }
}

/// The design fixture with Ledger a pull request review.
fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        if session.id == ledger() {
            session.pull_request_review = true;
        }
    }
    workspace
}

/// The app on Home, after the workspace loaded.
fn rig() -> (TestStore<App>, humata2_clients::Fakes) {
    let workspace = workspace();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    fakes.pull_requests.add_pull_request(info());
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    act(&mut store, A::Loaded(Ok(workspace)));
    (store, fakes)
}

fn act(store: &mut TestStore<App>, action: A) {
    store.send(action, |_| {});
    for _ in 0..8 {
        store.skip_received();
    }
}

#[test]
fn a_pasted_pull_request_that_starts_opens_the_started_session() {
    let (mut store, _fakes) = rig();
    act(
        &mut store,
        A::PrDialog(DialogAction::Open {
            machine: Some(MachineId::new("this-mac")),
            agent: Agent::Claude,
        }),
    );
    store.send(
        A::PrDialog(DialogAction::Paste("https://github.com/acme/api/pull/482".to_owned())),
        |_| {},
    );
    store.receive_matching(
        "GitHub answered",
        |action| matches!(action, A::PrDialog(DialogAction::Resolved { .. })),
        |_| {},
    );
    store.send(A::PrDialog(DialogAction::Start), |_| {});
    store.receive_matching(
        "the session started",
        |action| matches!(action, A::PrDialog(DialogAction::Started(Ok(_)))),
        |_| {},
    );
    store.receive_matching(
        "the dialog asked to open it",
        |action| matches!(action, A::PrDialog(DialogAction::Delegate(_))),
        |_| {},
    );
    assert_eq!(store.state().route, Route::Session(SessionId::new("se_pr_1")));
}

#[test]
fn the_new_commits_feed_is_followed_once_the_app_has_loaded() {
    let (store, _fakes) = rig();
    assert!(store.state().pr_heads.watching);
}

#[test]
fn a_review_session_opening_makes_the_follower_follow_it() {
    let (mut store, _fakes) = rig();
    assert_eq!(store.state().pr_follower.session, None);
    act(&mut store, A::Navigate(Route::Session(ledger())));
    assert_eq!(store.state().pr_follower.session, Some(ledger()));
}

#[test]
fn a_session_that_becomes_a_review_after_its_page_opened_is_followed() {
    let (mut store, _fakes) = rig();
    let other = store
        .state()
        .workspace
        .sessions
        .iter()
        .find(|session| !session.pull_request_review)
        .cloned();
    let Some(mut other) = other else {
        panic!("the design fixture has a session that is not a review");
    };
    act(&mut store, A::Navigate(Route::Session(other.id.clone())));
    assert_eq!(store.state().pr_follower.session, None);
    other.pull_request_review = true;
    let id = other.id.clone();
    act(&mut store, A::Event(humata2_clients::Event::Session(other)));
    assert_eq!(store.state().pr_follower.session, Some(id));
}

#[test]
fn a_session_that_is_not_a_review_is_not_followed() {
    let (mut store, _fakes) = rig();
    let other = store
        .state()
        .workspace
        .sessions
        .iter()
        .find(|session| !session.pull_request_review)
        .map(|session| session.id.clone());
    let Some(other) = other else {
        panic!("the design fixture has a session that is not a review");
    };
    act(&mut store, A::Navigate(Route::Session(other)));
    assert_eq!(store.state().pr_follower.session, None);
}

#[test]
fn choosing_review_for_a_pull_request_a_session_reviews_opens_that_session() {
    let workspace = workspace();
    let Some((session, pull)) = workspace
        .pulls
        .iter()
        .find_map(|pull| pull.session.clone().map(|session| (session, pull)))
    else {
        panic!("the design fixture has a pull request under review");
    };
    let Some((owner, repo)) = pull.repo.split_once('/') else {
        panic!("{} is owner/repo", pull.repo);
    };
    let review = PullRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number: pull.number,
    };
    let (mut store, _fakes) = rig();
    act(&mut store, A::Palette(PaletteAction::Begin(Command::ReviewPull(review))));
    assert_eq!(store.state().route, Route::Session(session));
    assert!(!store.state().pr_dialog.open);
}

#[test]
fn choosing_review_for_a_pull_request_no_session_reviews_pastes_it_into_the_dialog() {
    let (mut store, fakes) = rig();
    // acme/api #482 has its review session in the design fixture; #777 has none.
    assert!(!store.state().workspace.pulls.iter().any(|p| p.repo == "acme/api" && p.number == 777));
    fakes.pull_requests.add_pull_request(PullRequestInfo {
        pr: PullRequestRef::new("acme/api", 777),
        ..info()
    });
    let pull = PullRef {
        owner: "acme".to_owned(),
        repo: "api".to_owned(),
        number: 777,
    };
    store.send(A::Palette(PaletteAction::Begin(Command::ReviewPull(pull))), |_| {});
    store.receive_matching(
        "GitHub answered for the pasted pull request",
        |action| matches!(action, A::PrDialog(DialogAction::Resolved { .. })),
        |_| {},
    );
    let dialog = &store.state().pr_dialog;
    assert!(dialog.open);
    assert!(dialog.machine.is_some(), "This Mac is picked, so Start waits only on GitHub");
    let Some(main) = dialog.main.as_ref() else {
        panic!("the dialog holds the pasted pull request");
    };
    assert_eq!(main.pr, PullRequestRef::new("acme/api", 777));
    assert!(matches!(main.fetch, Fetch::Found(_)));
}
