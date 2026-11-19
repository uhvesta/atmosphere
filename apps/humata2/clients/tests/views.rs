//! The scripted views keep what an agent offered and tell every watcher
//! about each change, like the daemon's event feed (VW-1 to VW-10, P7).

use humata2_clients::model::{
    ChannelId, RepoBase, RepoDiffSpec, Scope, SessionId, ViewId, ViewState,
};
use humata2_clients::views::{
    FakeViews, PlanGroup, PlanNote, PlanView, ViewContent, ViewEvents, ViewRecord, ViewsClient,
    WalkthroughStep,
};
use humata2_clients::{DaemonError, fixture};
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn session(id: &str) -> SessionId {
    SessionId::new(id)
}

fn listed(views: &FakeViews, of: &SessionId) -> Vec<ViewRecord> {
    match block_on(views.list(of)) {
        Ok(list) => list,
        Err(error) => panic!("list works: {error}"),
    }
}

fn watching(views: &FakeViews) -> ViewEvents {
    match block_on(views.watch()) {
        Ok(events) => events,
        Err(error) => panic!("watch works: {error}"),
    }
}

fn answer(views: &FakeViews, id: &ViewId, state: ViewState) -> ViewRecord {
    match block_on(views.set_state(id, state)) {
        Ok(record) => record,
        Err(error) => panic!("set_state works: {error}"),
    }
}

fn pending(events: &mut ViewEvents) -> Vec<ViewRecord> {
    let mut seen = Vec::new();
    while let Ok(record) = events.try_recv() {
        seen.push(record);
    }
    seen
}

#[test]
fn an_offered_view_is_listed_for_its_session_only() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mine = listed(&views, &session("s-1"));
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, id);
    assert_eq!(mine[0].state, ViewState::Waiting);
    assert!(!mine[0].view.accepted);
    assert!(listed(&views, &session("s-2")).is_empty());
}

#[test]
fn an_offer_reaches_a_watcher_on_the_feed() {
    let views = FakeViews::new();
    let mut events = watching(&views);
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let seen = pending(&mut events);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].id, id);
    assert_eq!(seen[0].state, ViewState::Waiting);
}

#[test]
fn accepting_updates_the_view_and_sends_one_event() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut events = watching(&views);
    let after = answer(&views, &id, ViewState::Opened);
    assert_eq!(after.state, ViewState::Opened);
    assert!(after.view.accepted);
    assert_eq!(listed(&views, &session("s-1"))[0], after);
    assert_eq!(pending(&mut events), vec![after]);
}

#[test]
fn repeating_a_key_answers_the_first_time_and_sends_no_second_event() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut events = watching(&views);
    let first = match views.set_state_with_key(&id, "key-1", ViewState::Later) {
        Ok(record) => record,
        Err(error) => panic!("first answer works: {error}"),
    };
    // The retry names a different state, but the key was already used.
    let again = match views.set_state_with_key(&id, "key-1", ViewState::Opened) {
        Ok(record) => record,
        Err(error) => panic!("the retry works: {error}"),
    };
    assert_eq!(again, first);
    assert_eq!(listed(&views, &session("s-1"))[0].state, ViewState::Later);
    assert_eq!(pending(&mut events).len(), 1);
}

#[test]
fn answering_a_view_that_does_not_exist_is_not_found() {
    let views = FakeViews::new();
    let result = block_on(views.set_state(&ViewId::new("vw_nope"), ViewState::Opened));
    assert!(matches!(result, Err(DaemonError::NotFound { .. })));
}

#[test]
fn every_clone_sees_the_same_views() {
    let views = FakeViews::new();
    let other = views.clone();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let mut events = watching(&other);
    answer(&views, &id, ViewState::NotNow);
    assert_eq!(listed(&other, &session("s-1"))[0].state, ViewState::NotNow);
    assert_eq!(pending(&mut events).len(), 1);
}

fn offered(views: &FakeViews) -> Vec<ViewRecord> {
    match block_on(views.list_offered()) {
        Ok(list) => list,
        Err(error) => panic!("list_offered works: {error}"),
    }
}

#[test]
fn the_seeded_fake_holds_every_view_the_workspace_has() {
    let workspace = fixture::design();
    let views = FakeViews::seeded(&workspace);
    let mut held: Vec<_> = offered(&views)
        .into_iter()
        .map(|record| {
            (
                record.view.title.clone(),
                record.view.session.clone(),
                record.view.note.clone(),
                record.view.prepared.clone(),
                record.state,
                record.content,
            )
        })
        .collect();
    let mut expected: Vec<_> = workspace
        .views
        .iter()
        .map(|view| {
            (
                view.title.clone(),
                view.session.clone(),
                view.note.clone(),
                view.prepared.clone(),
                if view.accepted {
                    ViewState::Kept
                } else {
                    ViewState::Waiting
                },
                ViewContent::Summary,
            )
        })
        .collect();
    held.sort_by(|a, b| a.0.cmp(&b.0));
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!expected.is_empty());
    assert_eq!(held, expected);
}

#[test]
fn an_offered_view_keeps_what_it_holds_and_who_prepared_it() {
    let views = FakeViews::new();
    let mut events = watching(&views);
    let contents = [
        ViewContent::Walkthrough(vec![
            WalkthroughStep {
                repo: "acme/ledger".to_owned(),
                path: "migrations/0042_accounts.sql".to_owned(),
                note: "Check the account key.".to_owned(),
            },
            WalkthroughStep {
                repo: "acme/api".to_owned(),
                path: "src/client.rs".to_owned(),
                note: "One transaction.".to_owned(),
            },
        ]),
        ViewContent::ReviewPlan(PlanView {
            importance: Vec::new(),
            stack: None,
            targets: vec![
                RepoDiffSpec {
                    repo: "acme/api".to_owned(),
                    base: RepoBase::Branch("release/2.4".to_owned()),
                },
                RepoDiffSpec {
                    repo: "acme/web".to_owned(),
                    base: RepoBase::Uncommitted,
                },
            ],
            order: vec![("acme/api".to_owned(), "src/client.rs".to_owned())],
            groups: vec![PlanGroup {
                title: "Client".to_owned(),
                note: "The risky part.".to_owned(),
                first: ("acme/api".to_owned(), "src/client.rs".to_owned()),
            }],
            notes: vec![PlanNote {
                repo: "acme/api".to_owned(),
                path: "src/client.rs".to_owned(),
                text: "One transaction.".to_owned(),
                lines: Some((12, 14)),
                outdated: true,
            }],
        }),
        ViewContent::Board(Scope::Channel(ChannelId::new("payments-rewrite"))),
    ];
    for content in contents {
        let id = views.offer_view(
            &session("s-1"),
            "A view",
            "What it shows",
            "Codex",
            content.clone(),
        );
        let told = pending(&mut events);
        assert_eq!(told.len(), 1);
        for record in [
            told[0].clone(),
            listed(&views, &session("s-1"))
                .into_iter()
                .find(|record| record.id == id)
                .unwrap_or_else(|| panic!("the view is listed")),
            offered(&views)
                .into_iter()
                .find(|record| record.id == id)
                .unwrap_or_else(|| panic!("the view is offered")),
        ] {
            assert_eq!(record.id, id);
            assert_eq!(record.content, content);
            assert_eq!(record.prepared_by, "Codex");
            assert_eq!(record.state, ViewState::Waiting);
        }
    }
}

#[test]
fn a_plain_offer_holds_only_its_summary_and_names_who_prepared_it() {
    let views = FakeViews::new();
    let id = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let record = &listed(&views, &session("s-1"))[0];
    assert_eq!(record.id, id);
    assert_eq!(record.content, ViewContent::Summary);
    assert!(!record.prepared_by.is_empty());
}

#[test]
fn views_are_offered_across_sessions_newest_first_without_drafts() {
    let views = FakeViews::new();
    let first = views.offer(&session("s-1"), "Walkthrough", "Three files");
    let draft = views.offer(&session("s-1"), "Not ready", "Still writing");
    let second = views.offer(&session("s-2"), "Backfill plan", "Batches");
    if let Err(error) = views.set_state_with_key(&draft, "draft-1", ViewState::Draft) {
        panic!("the fake takes the view back to a draft: {error}");
    }
    let ids: Vec<ViewId> = offered(&views)
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(ids, vec![second, first]);
}

#[test]
fn a_session_board_offer_keeps_its_session_and_forks_on_the_fake() {
    let views = FakeViews::new();
    let mut events = watching(&views);
    for with_forks in [true, false] {
        let content = ViewContent::SessionBoard {
            session: session("s-1"),
            with_forks,
        };
        let id = views.offer_view(
            &session("s-1"),
            "Session board",
            "This session's items",
            "Codex",
            content.clone(),
        );
        let told = pending(&mut events);
        assert_eq!(told.len(), 1);
        let in_list = listed(&views, &session("s-1"))
            .into_iter()
            .find(|record| record.id == id)
            .unwrap_or_else(|| panic!("the view is listed"));
        let in_offers = offered(&views)
            .into_iter()
            .find(|record| record.id == id)
            .unwrap_or_else(|| panic!("the view is offered"));
        for record in [told[0].clone(), in_list, in_offers] {
            assert_eq!(record.id, id);
            assert_eq!(record.content, content);
        }
        let after = answer(&views, &id, ViewState::Opened);
        assert_eq!(after.content, content);
        assert_eq!(pending(&mut events), vec![after]);
    }
}
