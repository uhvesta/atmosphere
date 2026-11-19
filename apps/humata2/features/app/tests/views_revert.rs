//! Flow 26.12, VW-6 to VW-9, P8 on the fakes: an agent's view opens only when
//! the person chooses Open it, Revert puts back exactly what they had (the
//! route, the terminal tab, the panel, full screen, the board's scope and
//! each repository's diff base) and never touches their comments, Keep
//! leaves the view, a board view's Revert brings the board's scope back, and
//! a review plan opens the Diff panel with a base per repository. Views stack:
//! reverting the top one shows the one under it, Close of the top shows the
//! one under it, and opening the view showing again changes nothing. A
//! walkthrough step opens its file beside the view, and a session's board
//! view opens that session's board.
use humata2_app::layout::FullScreen;
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::comments::{Anchor, Comment, FakeComments, Side};
use humata2_clients::model::{
    ChannelId, DiffTargets, RepoBase, RepoDiffSpec, Scope, SessionId, ViewId, ViewState,
};
use humata2_clients::views::{
    FakeViews, LayerView, PlanGroup, PlanNote, PlanView, StackView, ViewContent, ViewsClient,
    WalkthroughStep,
};
use humata2_clients::{Clients, fixture};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{Panel, SessionAction, Tab, TabGroup};
use humata2_views::ViewsAction;
use std::future::Future;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

struct Rig {
    store: TestStore<App>,
    views: FakeViews,
    comments: FakeComments,
}

/// The app on the design fixture, on Ledger's page, after the workspace
/// loaded. `offer` runs on the fake views first, as an agent marking a view
/// ready before the app starts, and answers the view to open.
fn rig(offer: impl FnOnce(&FakeViews) -> ViewId) -> (Rig, ViewId) {
    let workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    fakes.comments.set_repos(&ledger(), &["api", "ledger"]);
    let id = offer(&fakes.views);
    let mut state = AppState::launch(&clients);
    state.route = Route::Session(ledger());
    state.trail = vec![Route::Home];
    state.session.open = Some(ledger());
    let mut rig = Rig {
        store: TestStore::new(state, App::new(clients)).non_exhaustive(),
        views: fakes.views,
        comments: fakes.comments,
    };
    rig.act(A::Loaded(Ok(workspace)));
    (rig, id)
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..8 {
            self.store.skip_received();
        }
    }

    fn state(&self) -> &AppState {
        self.store.state()
    }

    /// The tabs of Ledger's page, by group.
    fn groups(&self) -> Vec<TabGroup> {
        self.state()
            .session
            .views
            .get(&ledger())
            .map(|view| view.groups.clone())
            .unwrap_or_default()
    }

    /// Whether `id`'s tab is on Ledger's page, and whether it is selected
    /// in its group.
    fn view_tab(&self, id: &ViewId) -> (bool, bool) {
        let tab = Tab::View(id.to_string());
        let groups = self.groups();
        (
            groups.iter().any(|group| group.tabs.contains(&tab)),
            groups
                .iter()
                .any(|group| group.tabs.get(group.selected) == Some(&tab)),
        )
    }

    fn showing(&self) -> Option<ViewId> {
        self.state()
            .views
            .showing
            .as_ref()
            .map(|showing| showing.id.clone())
    }

    fn view_state(&self, id: &ViewId) -> Option<ViewState> {
        self.state().views.record(id).map(|record| record.state)
    }

    /// What the daemon holds for `id`.
    fn daemon_state(&self, id: &ViewId) -> Option<ViewState> {
        let listed = match block_on(self.views.list(&ledger())) {
            Ok(listed) => listed,
            Err(error) => panic!("the fake lists Ledger's views: {error}"),
        };
        listed
            .into_iter()
            .find(|record| &record.id == id)
            .map(|record| record.state)
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

/// Everything Revert must put back, as the person had it.
#[derive(Debug, PartialEq)]
struct Seen {
    route: Route,
    trail: Vec<Route>,
    groups: Vec<TabGroup>,
    focused_group: usize,
    terminal_tab: usize,
    panel: Option<Panel>,
    full_screen: Option<FullScreen>,
    full_screens: Option<FullScreen>,
    collapsed: bool,
    board: Scope,
    targets: Option<DiffTargets>,
}

fn seen(state: &AppState) -> Seen {
    let view = state
        .session
        .views
        .get(&ledger())
        .cloned()
        .unwrap_or_default();
    Seen {
        route: state.route.clone(),
        trail: state.trail.clone(),
        groups: view.groups,
        focused_group: view.focused_group,
        terminal_tab: view.tab,
        panel: state.panel(),
        full_screen: state.full_screen(),
        full_screens: state.full_screens.get(&ledger()).copied(),
        collapsed: state.layout.panel.collapsed,
        board: state.board.scope.clone(),
        targets: state.review.targets.get(&ledger()).cloned(),
    }
}

/// The person's own setup on Ledger's page: its shell tab, the Diff panel
/// full screen, a board scoped to the channel, and api on its own base.
fn set_up_own_page(rig: &mut Rig) {
    rig.act(A::Session(SessionAction::SelectTab(1)));
    rig.act(A::Session(SessionAction::SetPanel(Some(Panel::Changes))));
    rig.act(A::ToggleFullScreen(FullScreen::Panel));
    rig.act(A::Board(humata2_board::BoardAction::SetScope(
        Scope::Channel(payments()),
    )));
    rig.act(A::Review(ReviewAction::PickRepoBase {
        session: ledger(),
        repo: "acme/api".to_owned(),
        base: RepoBase::Branch("release/2.4".to_owned()),
    }));
}

fn walkthrough(views: &FakeViews) -> ViewId {
    views.offer_view(
        &ledger(),
        "PR review: ledger schema",
        "Two files, the risky one first",
        "Claude Code",
        ViewContent::Walkthrough(vec![
            WalkthroughStep {
                repo: "acme/ledger".to_owned(),
                path: "migrations/0042_accounts.sql".to_owned(),
                note: "Check the account key.".to_owned(),
            },
            WalkthroughStep {
                repo: "acme/api".to_owned(),
                path: "src/client.rs".to_owned(),
                note: "Keep the dual write in one transaction.".to_owned(),
            },
        ]),
    )
}

fn anchor() -> Anchor {
    Anchor {
        repo: "api".into(),
        path: "src/client.rs".into(),
        side: Side::New,
        first: 43,
        last: 43,
        quoted: vec!["        tx.insert_legacy(&entry).await?;".into()],
        base: None,
        detached: false,
        outdated: false,
    }
}

/// The comments Ledger holds, on the fake and in the app, each as its
/// id, text and status.
fn comments(rig: &Rig) -> (Vec<String>, Vec<String>) {
    let shown = |list: &[Comment]| {
        list.iter()
            .map(|comment| format!("{}|{}|{:?}", comment.id, comment.text, comment.status))
            .collect::<Vec<_>>()
    };
    let held = rig.comments.comments(&ledger());
    let app = rig
        .state()
        .comments
        .sessions
        .get(&ledger())
        .map(|session| session.comments.clone())
        .unwrap_or_default();
    (shown(&held), shown(&app))
}

#[test]
fn an_offered_view_waits_until_the_person_opens_it() {
    let (rig, id) = rig(walkthrough);
    let before = rig.groups();
    let offers: Vec<_> = rig
        .state()
        .views
        .offers(&ledger())
        .into_iter()
        .map(|record| record.id.clone())
        .collect();
    assert_eq!(offers.first(), Some(&id));
    assert_eq!(rig.showing(), None);
    assert_eq!(rig.view_tab(&id), (false, false));
    assert_eq!(rig.groups(), before);
    assert_eq!(rig.view_state(&id), Some(ViewState::Waiting));
}

#[test]
fn reverting_a_walkthrough_puts_back_exactly_what_was_there_and_keeps_the_comment() {
    let (mut rig, id) = rig(walkthrough);
    set_up_own_page(&mut rig);
    let before = seen(rig.state());

    // Open it: the view's tab opens on Ledger's page and shows.
    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.state().route, Route::Session(ledger()));
    assert_eq!(rig.showing(), Some(id.clone()));
    assert_eq!(rig.view_tab(&id), (true, true));
    assert_eq!(rig.view_state(&id), Some(ViewState::Opened));
    assert_eq!(rig.state().view_step.get(&id).copied().unwrap_or(0), 0);

    // Next › to step 2; stepping past the last step stays on it.
    rig.act(A::Views(ViewsAction::Step {
        id: id.clone(),
        to: 1,
    }));
    assert_eq!(rig.state().view_step.get(&id), Some(&1));
    rig.act(A::Views(ViewsAction::Step {
        id: id.clone(),
        to: 7,
    }));
    assert_eq!(rig.state().view_step.get(&id), Some(&1));

    // A comment on step 2's file.
    rig.act(A::Comments(CommentsAction::Open(ledger())));
    rig.act(A::Comments(CommentsAction::OpenBox(ledger(), anchor())));
    rig.act(A::Comments(CommentsAction::Typed(
        ledger(),
        "Keep the two writes atomic.".into(),
    )));
    rig.act(A::Comments(CommentsAction::Comment(ledger())));
    let written = comments(&rig);
    assert_eq!(written.0.len(), 1);

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert_eq!(rig.view_tab(&id).0, false);
    assert_eq!(rig.showing(), None);
    assert_eq!(comments(&rig), written);
    assert_eq!(rig.view_state(&id), Some(ViewState::Reverted));
    assert_eq!(rig.daemon_state(&id), Some(ViewState::Reverted));
    let journal: Vec<_> = rig
        .state()
        .views
        .journal(&[ledger()])
        .into_iter()
        .filter(|record| record.id == id)
        .map(|record| record.state)
        .collect();
    assert_eq!(journal, vec![ViewState::Reverted]);

    // It can be opened again.
    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.showing(), Some(id.clone()));
    assert_eq!(rig.view_tab(&id), (true, true));
    assert_eq!(rig.view_state(&id), Some(ViewState::Opened));
}

#[test]
fn keep_ends_the_showing_and_leaves_the_view_open() {
    let (mut rig, id) = rig(walkthrough);
    set_up_own_page(&mut rig);
    rig.act(A::Views(ViewsAction::Open(id.clone())));
    let opened = seen(rig.state());

    rig.act(A::Views(ViewsAction::Keep));
    assert_eq!(rig.showing(), None);
    assert_eq!(rig.view_tab(&id).0, true);
    assert_eq!(seen(rig.state()), opened);
    assert_eq!(rig.view_state(&id), Some(ViewState::Kept));
    assert_eq!(rig.daemon_state(&id), Some(ViewState::Kept));

    // Nothing shows, so a Revert now puts nothing back.
    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), opened);
    assert_eq!(rig.view_state(&id), Some(ViewState::Kept));
}

#[test]
fn reverting_a_board_view_brings_back_the_page_and_the_boards_scope() {
    let (mut rig, id) = rig(|views| {
        views.offer_view(
            &ledger(),
            "Payments board",
            "What is left before the cut-over",
            "Claude Code",
            ViewContent::Board(Scope::Channel(payments())),
        )
    });
    rig.act(A::Board(humata2_board::BoardAction::SetScope(
        Scope::Global,
    )));
    let before = seen(rig.state());

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.state().route, Route::Board(Scope::Channel(payments())));
    assert_eq!(rig.state().board.scope, Scope::Channel(payments()));
    assert_eq!(rig.showing(), Some(id.clone()));

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert_eq!(rig.view_state(&id), Some(ViewState::Reverted));
}

#[test]
fn a_review_plan_opens_the_diff_panel_with_a_base_per_repository() {
    let (mut rig, id) = rig(|views| {
        views.offer_view(
            &ledger(),
            "Review plan: ledger schema",
            "The migration first, then the client",
            "Claude Code",
            ViewContent::ReviewPlan(PlanView {
                importance: Vec::new(),
                targets: vec![
                    RepoDiffSpec {
                        repo: "acme/ledger".to_owned(),
                        base: RepoBase::Branch("release/2.4".to_owned()),
                    },
                    RepoDiffSpec {
                        repo: "acme/api".to_owned(),
                        base: RepoBase::Uncommitted,
                    },
                ],
                order: vec![
                    (
                        "acme/ledger".to_owned(),
                        "migrations/0042_accounts.sql".to_owned(),
                    ),
                    ("acme/api".to_owned(), "src/client.rs".to_owned()),
                ],
                groups: vec![PlanGroup {
                    title: "Schema".to_owned(),
                    note: "The risky part.".to_owned(),
                    first: (
                        "acme/ledger".to_owned(),
                        "migrations/0042_accounts.sql".to_owned(),
                    ),
                }],
                notes: vec![PlanNote {
                    repo: "acme/api".to_owned(),
                    path: "src/client.rs".to_owned(),
                    text: "One transaction.".to_owned(),
                    lines: None,
                    outdated: false,
                }],
                stack: None,
            }),
        )
    });
    // The person had api on its own base and no panel open.
    rig.act(A::Review(ReviewAction::PickRepoBase {
        session: ledger(),
        repo: "acme/api".to_owned(),
        base: RepoBase::DefaultBranch,
    }));
    rig.act(A::Session(SessionAction::SetPanel(None)));
    let before = seen(rig.state());
    assert_eq!(before.panel, None);

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.state().route, Route::Session(ledger()));
    assert_eq!(rig.state().panel(), Some(Panel::Changes));
    let targets = rig.state().review.targets_of(&ledger());
    assert!(targets.per_repo());
    assert_eq!(
        targets.of("acme/ledger"),
        RepoBase::Branch("release/2.4".to_owned())
    );
    assert_eq!(targets.of("acme/api"), RepoBase::Uncommitted);
    assert!(rig.state().review.plans.contains_key(&ledger()));

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert!(!rig.state().review.plans.contains_key(&ledger()));
}

impl Rig {
    /// The file tabs on Ledger's page other than `own`, each as its group,
    /// its path and whether it is selected in that group. Each names a file
    /// Ledger's clones hold.
    fn opened_files(&self, own: &[Tab]) -> Vec<(usize, String, bool)> {
        let held = self
            .state()
            .workspace
            .session_files
            .get(&ledger())
            .cloned()
            .unwrap_or_default();
        let mut found = Vec::new();
        for (index, group) in self.groups().iter().enumerate() {
            for (at, tab) in group.tabs.iter().enumerate() {
                if let Tab::File { repo, path } = tab
                    && !own.contains(tab)
                {
                    assert!(
                        held.iter()
                            .any(|file| !file.folder && &file.repo == repo && &file.path == path),
                        "the tab {repo}/{path} names a file of Ledger's"
                    );
                    found.push((index, path.clone(), group.selected == at));
                }
            }
        }
        found
    }

    /// The group holding `id`'s tab.
    fn view_group(&self, id: &ViewId) -> Option<usize> {
        let tab = Tab::View(id.to_string());
        self.groups()
            .iter()
            .position(|group| group.tabs.contains(&tab))
    }
}

/// The design walkthrough, offered first, and another view the agent
/// offers after it. Answers both ids, the walkthrough's first.
fn two(second: impl FnOnce(&FakeViews) -> ViewId) -> (Rig, ViewId, ViewId) {
    let mut top = None;
    let (rig, under) = rig(|views| {
        let under = walkthrough(views);
        top = Some(second(views));
        under
    });
    let Some(top) = top else {
        panic!("the second view was offered");
    };
    (rig, under, top)
}

fn board_view(views: &FakeViews) -> ViewId {
    views.offer_view(
        &ledger(),
        "Payments board",
        "What is left before the cut-over",
        "Claude Code",
        ViewContent::Board(Scope::Channel(payments())),
    )
}

#[test]
fn reverting_a_second_view_shows_the_first_again_and_a_second_revert_puts_back_the_page() {
    let (mut rig, under, top) = two(board_view);
    rig.act(A::Board(humata2_board::BoardAction::SetScope(
        Scope::Global,
    )));
    let before = seen(rig.state());

    rig.act(A::Views(ViewsAction::Open(under.clone())));
    let under_open = seen(rig.state());
    rig.act(A::Views(ViewsAction::Open(top.clone())));
    assert_eq!(rig.state().route, Route::Board(Scope::Channel(payments())));
    assert_eq!(rig.showing(), Some(top.clone()));

    // The board view goes; the walkthrough shows again, as it was.
    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), under_open);
    assert_eq!(rig.showing(), Some(under.clone()));
    assert_eq!(rig.view_tab(&under), (true, true));
    assert_eq!(rig.view_state(&top), Some(ViewState::Reverted));
    assert_eq!(rig.view_state(&under), Some(ViewState::Opened));
    assert_eq!(rig.daemon_state(&under), Some(ViewState::Opened));

    // Then the walkthrough goes, and the page is as the person had it.
    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert_eq!(rig.showing(), None);
    assert_eq!(rig.view_tab(&under).0, false);
    assert_eq!(rig.view_state(&under), Some(ViewState::Reverted));
    assert_eq!(rig.daemon_state(&under), Some(ViewState::Reverted));
}

#[test]
fn closing_the_top_view_shows_the_one_under_it_and_its_revert_still_puts_back_the_page() {
    let (mut rig, under, top) =
        two(|views| views.offer(&ledger(), "Backfill plan", "Batches of ten thousand"));
    let before = seen(rig.state());

    rig.act(A::Views(ViewsAction::Open(under.clone())));
    rig.act(A::Views(ViewsAction::Open(top.clone())));
    assert_eq!(rig.view_tab(&top), (true, true));

    rig.act(A::Views(ViewsAction::Close));
    assert_eq!(rig.view_tab(&top).0, false);
    assert_eq!(rig.showing(), Some(under.clone()));
    assert_eq!(rig.view_tab(&under).0, true);
    assert_eq!(rig.view_state(&top), Some(ViewState::OpenedAndClosed));
    assert_eq!(rig.view_state(&under), Some(ViewState::Opened));

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert_eq!(rig.showing(), None);
    assert_eq!(rig.view_state(&under), Some(ViewState::Reverted));
}

#[test]
fn opening_the_view_showing_again_changes_nothing_and_one_revert_puts_back_the_page() {
    let (mut rig, id) = rig(walkthrough);
    set_up_own_page(&mut rig);
    let before = seen(rig.state());

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    let opened = seen(rig.state());
    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(seen(rig.state()), opened);
    assert_eq!(rig.showing(), Some(id.clone()));

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert_eq!(rig.showing(), None);
}

#[test]
fn a_walkthrough_step_opens_its_file_beside_the_view_and_next_replaces_it() {
    let (mut rig, id) = rig(walkthrough);
    // A file the person opened themselves.
    rig.act(A::Session(SessionAction::SelectFile {
        repo: "api".to_owned(),
        path: "src/entry.rs".to_owned(),
    }));
    rig.act(A::Session(SessionAction::OpenFile));
    assert_eq!(rig.opened_files(&[]).len(), 1);
    let own: Vec<Tab> = rig
        .groups()
        .iter()
        .flat_map(|group| group.tabs.clone())
        .filter(|tab| matches!(tab, Tab::File { .. }))
        .collect();
    let before = seen(rig.state());

    // Open it: the first step's file opens in the other group.
    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.view_tab(&id), (true, true));
    assert_eq!(rig.groups().len(), 2);
    let Some(view_group) = rig.view_group(&id) else {
        panic!("the view's tab is on Ledger's page");
    };
    let opened = rig.opened_files(&own);
    assert_eq!(opened.len(), 1);
    assert_ne!(opened[0].0, view_group);
    assert_eq!(
        (opened[0].1.as_str(), opened[0].2),
        ("migrations/0042_accounts.sql", true)
    );

    // Next ›: step 2's file takes the place of step 1's; the person's own
    // file stays.
    rig.act(A::Views(ViewsAction::Step {
        id: id.clone(),
        to: 1,
    }));
    let stepped = rig.opened_files(&own);
    assert_eq!(stepped.len(), 1);
    assert_ne!(stepped[0].0, view_group);
    assert_eq!(
        (stepped[0].1.as_str(), stepped[0].2),
        ("src/client.rs", true)
    );
    let groups = rig.groups();
    assert!(own
        .iter()
        .all(|tab| groups.iter().any(|group| group.tabs.contains(tab))));

    // Revert takes the step's file away with the rest.
    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert!(rig.opened_files(&own).is_empty());
}

#[test]
fn a_step_in_a_repository_the_session_does_not_have_opens_no_file() {
    let (mut rig, id) = rig(|views| {
        views.offer_view(
            &ledger(),
            "PR review: the form",
            "A file elsewhere, then the client",
            "Claude Code",
            ViewContent::Walkthrough(vec![
                WalkthroughStep {
                    repo: "acme/web".to_owned(),
                    path: "src/form.tsx".to_owned(),
                    note: "Not in this session.".to_owned(),
                },
                WalkthroughStep {
                    repo: "acme/api".to_owned(),
                    path: "src/client.rs".to_owned(),
                    note: "Keep the dual write in one transaction.".to_owned(),
                },
            ]),
        )
    });

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.showing(), Some(id.clone()));
    assert_eq!(rig.view_tab(&id), (true, true));
    assert!(rig.opened_files(&[]).is_empty());

    // The next step's repository is the session's, so its file opens.
    rig.act(A::Views(ViewsAction::Step {
        id: id.clone(),
        to: 1,
    }));
    let opened: Vec<String> = rig
        .opened_files(&[])
        .into_iter()
        .map(|(_, path, _)| path)
        .collect();
    assert_eq!(opened, vec!["src/client.rs".to_owned()]);
}

#[test]
fn a_session_board_view_opens_the_sessions_board_and_revert_comes_back() {
    let (mut rig, id) = rig(|views| {
        views.offer_view(
            &ledger(),
            "Ledger's board",
            "This session's items and its forks'",
            "Claude Code",
            ViewContent::SessionBoard {
                session: ledger(),
                with_forks: true,
            },
        )
    });
    rig.act(A::Board(humata2_board::BoardAction::SetScope(
        Scope::Global,
    )));
    let before = seen(rig.state());

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    let scope = Scope::Session {
        channel: payments(),
        session: ledger(),
    };
    assert_eq!(rig.state().route, Route::Board(scope.clone()));
    assert_eq!(rig.state().board.scope, scope);
    assert_eq!(rig.showing(), Some(id.clone()));

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(seen(rig.state()), before);
    assert_eq!(rig.view_state(&id), Some(ViewState::Reverted));
}

#[test]
fn a_board_view_of_a_session_the_workspace_does_not_know_opens_as_a_summary() {
    let (mut rig, id) = rig(|views| {
        views.offer_view(
            &ledger(),
            "A gone session's board",
            "Its items",
            "Claude Code",
            ViewContent::SessionBoard {
                session: SessionId::new("gone"),
                with_forks: false,
            },
        )
    });

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.state().route, Route::Session(ledger()));
    assert_eq!(rig.showing(), Some(id.clone()));
    assert_eq!(rig.view_tab(&id), (true, true));
}

/// RV-34: opening a review plan shows its notes as the daemon reads them
/// then, not as they were when the view was offered.
#[test]
fn opening_a_review_plan_shows_its_notes_as_read_now() {
    let plan = |outdated: bool, lines: (u32, u32)| {
        ViewContent::ReviewPlan(PlanView {
            importance: Vec::new(),
            targets: Vec::new(),
            order: vec![("acme/api".to_owned(), "src/client.rs".to_owned())],
            groups: Vec::new(),
            notes: vec![PlanNote {
                repo: "acme/api".to_owned(),
                path: "src/client.rs".to_owned(),
                text: "One transaction.".to_owned(),
                lines: Some(lines),
                outdated,
            }],
            stack: None,
        })
    };
    let (mut rig, id) = rig(|views| {
        views.offer_view(&ledger(), "Review plan: client", "The client", "Claude Code", plan(false, (4, 4)))
    });
    // The files changed after the offer: the daemon now reads the note as
    // moved and outdated.
    rig.views.read_as(&id, plan(true, (9, 9)));

    rig.act(A::Views(ViewsAction::Open(id)));

    let shown = rig.state().review.plans.get(&ledger()).map(|plan| plan.notes.clone());
    assert_eq!(shown.as_deref().map(|notes| notes.iter().map(|note| (note.lines, note.outdated)).collect::<Vec<_>>()), Some(vec![(Some((9, 9)), true)]));
}

/// A real host's workspace carries no file lists: they are fetched lazily.
/// A walkthrough's step still opens its file beside the view, once the
/// session's files arrive, and the next step's file follows.
#[test]
fn a_walkthrough_step_opens_its_file_when_the_sessions_files_arrive_later() {
    let mut workspace = fixture::design();
    workspace.session_files.clear();
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    fakes.comments.set_repos(&ledger(), &["api", "ledger"]);
    let id = walkthrough(&fakes.views);
    // Home is shown while the workspace loads: a session page asks for its
    // files when it opens, so none are fetched before the view opens it.
    let mut state = AppState::launch(&clients);
    state.route = Route::Home;
    let mut rig = Rig {
        store: TestStore::new(state, App::new(clients)).non_exhaustive(),
        views: fakes.views,
        comments: fakes.comments,
    };
    rig.act(A::Loaded(Ok(workspace)));
    assert!(rig.state().workspace.session_files.get(&ledger()).is_none());

    rig.act(A::Views(ViewsAction::Open(id.clone())));
    assert_eq!(rig.showing(), Some(id.clone()));
    let opened: Vec<String> = rig
        .opened_files(&[])
        .into_iter()
        .map(|(_, path, _)| path)
        .collect();
    assert_eq!(opened, vec!["migrations/0042_accounts.sql".to_owned()]);

    rig.act(A::Views(ViewsAction::Step {
        id: id.clone(),
        to: 1,
    }));
    let stepped: Vec<String> = rig
        .opened_files(&[])
        .into_iter()
        .map(|(_, path, _)| path)
        .collect();
    assert_eq!(stepped, vec!["src/client.rs".to_owned()]);
}

/// A plan whose stack has these layers, bottom first.
fn stacked_plan(titles: &[&str]) -> ViewContent {
    ViewContent::ReviewPlan(PlanView {
        stack: Some(StackView {
            layers: titles
                .iter()
                .map(|title| LayerView {
                    title: (*title).to_owned(),
                    ..LayerView::default()
                })
                .collect(),
            whole: Vec::new(),
        }),
        ..PlanView::default()
    })
}

/// D800: opening a stack starts at Whole stack, and Revert puts back the
/// plan, its stack and the layer the person had as one unit.
#[test]
fn a_stack_opens_at_whole_stack_and_revert_restores_the_stack_and_layer_together() {
    let mut second = None;
    let (mut rig, first) = rig(|views| {
        second = Some(views.offer_view(
            &ledger(),
            "Stack: two",
            "Two layers",
            "Claude Code",
            stacked_plan(&["api", "web", "docs"]),
        ));
        views.offer_view(
            &ledger(),
            "Stack: one",
            "Two layers",
            "Claude Code",
            stacked_plan(&["api", "web"]),
        )
    });
    let Some(second) = second else {
        panic!("the second view was offered");
    };
    rig.act(A::Views(ViewsAction::Open(first)));
    rig.act(A::Review(ReviewAction::PickLayer(Some("web".to_owned()))));
    let titles = |rig: &Rig| -> Vec<String> {
        rig.state()
            .review
            .plans
            .get(&ledger())
            .and_then(|plan| plan.stack.as_ref())
            .map(|stack| stack.layers.iter().map(|layer| layer.title.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(rig.state().review.layer_of(&ledger()), Some("web"));
    assert_eq!(titles(&rig), vec!["api", "web"]);

    rig.act(A::Views(ViewsAction::Open(second)));
    assert_eq!(rig.state().review.layer_of(&ledger()), None);
    assert_eq!(titles(&rig), vec!["api", "web", "docs"]);

    rig.act(A::Views(ViewsAction::Revert));
    assert_eq!(rig.state().review.layer_of(&ledger()), Some("web"));
    assert_eq!(titles(&rig), vec!["api", "web"]);
}
