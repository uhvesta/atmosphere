//! The review chat through `LiveReviewChat` against a real `humatad2` (admin
//! build), in a review session the case makes: the view the app reports
//! comes back in the context, the switches are kept, and focus is refused,
//! with a fix, while the Focus switch is off (PR-22, PR-25, D59). The same
//! expectations hold for the fake client, which the features test against.

use std::time::Duration;

use humata2_clients::comments::Side;
use humata2_clients::model::{MachineId, RepoBase, RepoDiffSpec, SessionId, SessionOptions};
use humata2_clients::review_chat::{
    AgentSwitches, FakeReviewChat, FocusRequest, LineSpan, Place, ReviewChatClient, ReviewView,
};
use humata2_clients::{Daemon, DaemonError};
use humata2_clients_live_daemon::LiveDaemon;
use humata2_clients_live_review::LiveReviewChat;
use humata2_clients_live_test_support::cases::World;
use humata2_clients_live_test_support::real;

const DEADLINE: Duration = Duration::from_secs(60);

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

/// A real daemon, one repository and a session on it, which the case reviews.
async fn review_session() -> (real::RealWorld, LiveDaemon, SessionId, String) {
    let (world, live) = real::start_daemon().await;
    let workspace = within("load", live.load()).await.expect("load the workspace");
    let mac: MachineId = workspace
        .machines
        .first()
        .expect("the workspace has a machine")
        .id
        .clone();
    let repo = world.repo("owner", "ledger");
    let channel = within(
        "create_channel",
        live.create_channel("payments", vec![repo.clone()], vec![mac.clone()]),
    )
    .await
    .expect("create channel");
    let session = within(
        "create_session",
        live.create_session(
            &channel,
            world.agent(),
            &mac,
            "Review the ledger",
            SessionOptions {
                repos: vec![repo.clone()],
                first_message: String::new(),
                ..SessionOptions::default()
            },
        ),
    )
    .await
    .expect("create session");
    (world, live, session, repo)
}

/// What the app reports comes back in the context, from either client.
async fn the_view_the_app_reports_comes_back_in_the_context(
    chat: &dyn ReviewChatClient,
    session: &SessionId,
    repo: &str,
) {
    let view = ReviewView {
        targets: vec![RepoDiffSpec {
            repo: repo.to_owned(),
            base: RepoBase::DefaultBranch,
        }],
        open: Some(Place {
            repo: repo.to_owned(),
            path: "README.md".to_owned(),
            visible: Some(LineSpan { first: 1, last: 1 }),
            selection: None,
            side: Side::New,
        }),
        ..ReviewView::default()
    };
    within("report", chat.report_view(session, view.clone()))
        .await
        .expect("the view is reported");
    let context = within("context", chat.context(session))
        .await
        .expect("the context is read");
    assert_eq!(context.view.targets, view.targets);
    assert_eq!(context.view.open, view.open);
}

/// Switches are kept, and focus is refused while the Focus switch is off,
/// from either client.
async fn switches_are_kept_and_focus_is_refused_while_it_is_off(
    chat: &dyn ReviewChatClient,
    session: &SessionId,
    repo: &str,
) {
    let switches = AgentSwitches {
        focus: false,
        ..AgentSwitches::default()
    };
    let kept = within("set switches", chat.set_switches(session, switches))
        .await
        .expect("the switches are set");
    assert_eq!(kept, switches);
    let context = within("context", chat.context(session))
        .await
        .expect("the context is read");
    assert_eq!(context.switches, switches);
    let refused = within(
        "focus",
        chat.focus(
            session,
            FocusRequest {
                repo: repo.to_owned(),
                path: "README.md".to_owned(),
                lines: None,
                side: Side::New,
                highlights: Vec::new(),
                layer: None,
                whole_stack: false,
            },
        ),
    )
    .await;
    assert!(
        matches!(refused, Err(DaemonError::Refused { .. })),
        "focus with the switch off is refused: {refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_view_comes_back_on_the_fake() {
    let chat = FakeReviewChat::new();
    the_view_the_app_reports_comes_back_in_the_context(&chat, &SessionId::new("se_review"), "owner/ledger").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_view_comes_back_on_the_live_daemon() {
    let (_world, live, session, repo) = review_session().await;
    let chat = LiveReviewChat::new(&live);
    the_view_the_app_reports_comes_back_in_the_context(&chat, &session, &repo).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn switches_hold_on_the_fake() {
    let chat = FakeReviewChat::new();
    switches_are_kept_and_focus_is_refused_while_it_is_off(&chat, &SessionId::new("se_review"), "owner/ledger").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn switches_hold_on_the_live_daemon() {
    let (_world, live, session, repo) = review_session().await;
    let chat = LiveReviewChat::new(&live);
    switches_are_kept_and_focus_is_refused_while_it_is_off(&chat, &session, &repo).await;
}
