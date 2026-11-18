//! SPEC flow 26.12 (an agent offers a view; the person opens it, steps
//! through it and Reverts or Keeps; the Journal lists it) on a real daemon,
//! for a session on This Mac. The session's agent prepares a walkthrough
//! and marks it ready over the daemon's ViewService with the session token
//! its tab was given, as an agent's MCP call does; the app hears of it on
//! its views feed and shows it as an offer. Nothing opens until the person
//! chooses Open it, Revert puts the session page back exactly as it was and
//! leaves their comments alone, and what they chose outlives the app.
//!
//! The remote case (a box session through the host) is `views_remote.rs`.
use std::path::PathBuf;

use humata2_app::layout::FullScreen;
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::comments::{Anchor, Side};
use humata2_clients::model::{DiffTargets, RepoBase, Scope, SessionId, ViewId, ViewState};
use humata2_clients::views::ViewsClient;
use humata2_grpc::humata::v1 as pb;
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{Panel, SessionAction, Tab, TabGroup};
use humata2_views::ViewsAction;
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::UnixStream;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};

use super::review::{clone_of, numbered, write};
use super::world::World;
use super::{App, running_session, runtime};

/// The header a session token travels in (identity.rs).
const TOKEN_HEADER: &str = "humata-session-token";
const TITLE: &str = "PR review: ledger schema";
const MIGRATION: &str = "migrations/0042_accounts.sql";
const CLIENT: &str = "src/client.rs";
/// The ledger repo as the app keys it.
const LEDGER: &str = "owner/ledger";

/// The session's agent as the daemon sees it: a caller with the session
/// token the agent's tab was started with.
pub(super) struct AgentCalls {
    pub(super) socket: PathBuf,
    pub(super) token: String,
}

impl AgentCalls {
    /// The agent of the world's one session, which is running.
    fn of(world: &World) -> Self {
        Self {
            socket: world.socket(),
            token: world.agent_token(),
        }
    }

    async fn views(&self) -> pb::view_service_client::ViewServiceClient<Channel> {
        let path = self.socket.clone();
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
            }))
            .await
            .expect("the agent connects to humatad2");
        pb::view_service_client::ViewServiceClient::new(channel)
    }

    fn signed<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(
            TOKEN_HEADER,
            MetadataValue::try_from(self.token.as_str()).expect("a session token is header text"),
        );
        request
    }

    /// Prepares a two-step walkthrough of the ledger repo titled `title`:
    /// a draft. Its id.
    pub(super) async fn prepare(&self, title: &str) -> pb::ViewId {
        let step = |path: &str, note: &str| pb::review_walkthrough::Step {
            file: Some(pb::RepoPath {
                repo: Some(pb::RepoRef {
                    owner: "owner".to_owned(),
                    name: "ledger".to_owned(),
                }),
                path: path.to_owned(),
            }),
            lines: Some(pb::LineRange { first: 1, last: 3 }),
            note: note.to_owned(),
        };
        let request = pb::PrepareViewRequest {
            idempotency_key: key(&format!("prepare-{title}")),
            title: title.to_owned(),
            summary: "Two files, the risky one first".to_owned(),
            content: Some(pb::prepare_view_request::Content::Review(pb::ReviewWalkthrough {
                steps: vec![
                    step(MIGRATION, "Check the account key."),
                    step(CLIENT, "Keep the dual write in one transaction."),
                ],
            })),
        };
        let draft = self
            .views()
            .await
            .prepare_view(self.signed(request))
            .await
            .expect("the session's agent prepares a view")
            .into_inner();
        draft.id.expect("a prepared view has an id")
    }

    /// Prepares a walkthrough titled `title` and marks it ready: an offer.
    pub(super) async fn offer(&self, title: &str) -> ViewId {
        let id = self.prepare(title).await;
        self.views()
            .await
            .mark_ready(self.signed(pb::MarkReadyRequest {
                idempotency_key: key(&format!("ready-{title}")),
                id: Some(id.clone()),
            }))
            .await
            .expect("the session's agent marks its view ready");
        ViewId::new(id.value)
    }
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: value.to_owned(),
    })
}

pub(super) fn send(app: &mut App, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// A running session on This Mac, the ledger clone holding the files the
/// walkthrough steps through, and the session's agent.
fn started(runtime: &tokio::runtime::Runtime) -> (World, humata2_clients::Clients, SessionId, AgentCalls) {
    let (world, clients, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session = running_session(&world, &clients, "Ledger schema migration", "hello agent").await;
        (world, clients, session)
    });
    let channels = world
        .conversations()
        .parent()
        .expect("the conversations folder has a parent")
        .join("ch");
    let ledger = clone_of(&channels, "ledger");
    write(&ledger, MIGRATION, &numbered(12));
    write(&ledger, CLIENT, &numbered(45));
    let agent = AgentCalls::of(&world);
    (world, clients, session, agent)
}

/// The app on the session's page, with the person's own setup: the Diff
/// panel open and the ledger compared with its main branch.
fn on_own_page(app: &mut App, session: &SessionId) {
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(session).is_some())
    });
    send(app, AppAction::Navigate(Route::Session(session.clone())));
    send(app, AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))));
    send(
        app,
        AppAction::Review(ReviewAction::PickRepoBase {
            session: session.clone(),
            repo: LEDGER.to_owned(),
            base: RepoBase::Branch("main".to_owned()),
        }),
    );
}

pub(super) fn offered(state: &AppState, session: &SessionId) -> Vec<ViewId> {
    state
        .views
        .offers(session)
        .into_iter()
        .map(|record| record.id.clone())
        .collect()
}

pub(super) fn in_journal(state: &AppState, session: &SessionId, id: &ViewId) -> Vec<ViewState> {
    state
        .views
        .journal(std::slice::from_ref(session))
        .into_iter()
        .filter(|record| &record.id == id)
        .map(|record| record.state)
        .collect()
}

fn groups(state: &AppState, session: &SessionId) -> Vec<TabGroup> {
    state
        .session
        .views
        .get(session)
        .map(|view| view.groups.clone())
        .unwrap_or_default()
}

/// Whether `id`'s tab is on the session's page, and whether it is the
/// selected tab of its group.
pub(super) fn view_tab(state: &AppState, session: &SessionId, id: &ViewId) -> (bool, bool) {
    let tab = Tab::View(id.to_string());
    let groups = groups(state, session);
    (
        groups.iter().any(|group| group.tabs.contains(&tab)),
        groups
            .iter()
            .any(|group| group.tabs.get(group.selected) == Some(&tab)),
    )
}

pub(super) fn showing(state: &AppState) -> Option<ViewId> {
    state.views.showing.as_ref().map(|showing| showing.id.clone())
}

pub(super) fn app_state(state: &AppState, id: &ViewId) -> Option<ViewState> {
    state.views.record(id).map(|record| record.state)
}

/// What the daemon holds for `id`, read through the app's own views client.
pub(super) fn daemon_state(app: &App, views: &Arc<dyn ViewsClient>, session: &SessionId, id: &ViewId) -> Option<ViewState> {
    app.runtime
        .block_on(views.list(session))
        .expect("the daemon lists the session's views")
        .into_iter()
        .find(|record| &record.id == id)
        .map(|record| record.state)
}

/// Waits until the daemon holds `id` in `state`.
pub(super) fn wait_for_daemon(app: &mut App, views: &Arc<dyn ViewsClient>, session: &SessionId, id: &ViewId, state: ViewState) {
    app.wait_until(&format!("the daemon to hold the view {state:?}"), |app| {
        daemon_state(app, views, session, id) == Some(state)
    });
}

/// Waits until the app offers `id` on the session's page.
pub(super) fn wait_for_offer(app: &mut App, session: &SessionId, id: &ViewId) {
    app.wait_until("the agent's view to be offered", |app| {
        app.ui.read(|state| offered(state, session).contains(id))
    });
}

/// Everything Revert must put back, as the person had it.
#[derive(Debug, PartialEq)]
pub(super) struct Seen {
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

pub(super) fn seen(app: &App, session: &SessionId) -> Seen {
    app.ui.read(|state| {
        let view = state.session.views.get(session).cloned().unwrap_or_default();
        Seen {
            route: state.route.clone(),
            trail: state.trail.clone(),
            groups: view.groups,
            focused_group: view.focused_group,
            terminal_tab: view.tab,
            panel: state.panel(),
            full_screen: state.full_screen(),
            full_screens: state.full_screens.get(session).copied(),
            collapsed: state.layout.panel.collapsed,
            board: state.board.scope.clone(),
            targets: state.review.targets.get(session).cloned(),
        }
    })
}

/// The session's comments in the app, each as its id, text and status.
fn comments(app: &App, session: &SessionId) -> Vec<String> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(session)
            .map(|held| {
                held.comments
                    .iter()
                    .map(|comment| format!("{}|{}|{:?}", comment.id, comment.text, comment.status))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Leaves a draft comment on line `line` of the ledger's `path`, and waits
/// until the app holds it.
fn draft(app: &mut App, session: &SessionId, path: &str, line: u32, body: &str) {
    let before = comments(app, session).len();
    let anchor = Anchor {
        repo: LEDGER.to_owned(),
        path: path.to_owned(),
        side: Side::New,
        first: line,
        last: line,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    };
    send(app, AppAction::Comments(CommentsAction::OpenBox(session.clone(), anchor)));
    send(app, AppAction::Comments(CommentsAction::Typed(session.clone(), body.to_owned())));
    send(app, AppAction::Comments(CommentsAction::Close(session.clone())));
    app.wait_until("the draft comment to be held", |app| {
        comments(app, session).len() == before + 1
    });
}

/// 26.12 step 1: the view the agent marks ready is offered on its
/// session's page and nothing else changes; a view it only drafted is not
/// offered and not in the Journal.
#[test]
fn a_view_the_agent_marks_ready_is_offered_and_its_draft_is_not() {
    let runtime = runtime();
    let (_world, clients, session, agent) = started(&runtime);
    let mut app = App::open(&runtime, clients);
    on_own_page(&mut app, &session);
    let before = seen(&app, &session);

    let draft = ViewId::new(runtime.block_on(agent.prepare("Still drafting")).value);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);

    app.ui.read(|state| {
        assert_eq!(offered(state, &session).first(), Some(&id), "the newest offer is first");
        assert_eq!(app_state(state, &id), Some(ViewState::Waiting));
        assert_eq!(
            state.views.record(&id).map(|record| record.view.session.clone()),
            Some(session.clone()),
            "the offer is the session's"
        );
        assert!(!offered(state, &session).contains(&draft), "a draft is never offered");
        assert!(in_journal(state, &session, &draft).is_empty(), "a draft is not in the Journal");
        assert_eq!(showing(state), None, "nothing opens until the person chooses to");
        assert_eq!(view_tab(state, &session, &id), (false, false));
    });
    assert_eq!(seen(&app, &session), before, "an offer leaves the page as it was");
}

/// 26.12 steps 2 to 5: Open it shows the view on the session's page, Next
/// opens step 2's file beside it, a comment is written there, and Revert
/// puts back exactly what the person had, keeps the comment, and the
/// Journal says Reverted; the view can be opened again.
#[test]
fn reverting_an_opened_walkthrough_puts_the_page_back_and_keeps_the_comment() {
    let runtime = runtime();
    let (_world, clients, session, agent) = started(&runtime);
    let views = clients.views.clone();
    let mut app = App::open(&runtime, clients);
    on_own_page(&mut app, &session);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);
    let before = seen(&app, &session);

    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    app.ui.read(|state| {
        assert_eq!(state.route, Route::Session(session.clone()));
        assert_eq!(showing(state), Some(id.clone()));
        assert_eq!(view_tab(state, &session, &id), (true, true));
        assert_eq!(app_state(state, &id), Some(ViewState::Opened));
        assert_eq!(state.view_step.get(&id).copied().unwrap_or(0), 0, "it opens on step 1");
        assert!(!offered(state, &session).contains(&id), "an opened view is no longer offered");
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Opened);

    send(
        &mut app,
        AppAction::Views(ViewsAction::Step {
            id: id.clone(),
            to: 1,
        }),
    );
    let step_file = Tab::File {
        repo: LEDGER.to_owned(),
        path: CLIENT.to_owned(),
    };
    app.ui.read(|state| assert_eq!(state.view_step.get(&id), Some(&1)));
    // The session's file list is fetched from the host the first time a
    // walkthrough needs it, and the step's file opens once it arrives.
    app.wait_until("step 2's file to open beside the view", |app| {
        app.ui.read(|state| {
            groups(state, &session).iter().any(|group| group.tabs.contains(&step_file))
        })
    });
    draft(&mut app, &session, CLIENT, 2, "Keep the two writes atomic.");
    let written = comments(&app, &session);

    send(&mut app, AppAction::Views(ViewsAction::Revert));
    assert_eq!(seen(&app, &session), before, "Revert puts back exactly what was there");
    assert_eq!(comments(&app, &session), written, "Revert leaves the comments alone");
    app.ui.read(|state| {
        assert!(!view_tab(state, &session, &id).0);
        assert_eq!(showing(state), None);
        assert_eq!(app_state(state, &id), Some(ViewState::Reverted));
        assert_eq!(in_journal(state, &session, &id), vec![ViewState::Reverted]);
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Reverted);

    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    app.ui.read(|state| {
        assert_eq!(showing(state), Some(id.clone()), "the Journal's view opens again");
        assert_eq!(view_tab(state, &session, &id), (true, true));
        assert_eq!(app_state(state, &id), Some(ViewState::Opened));
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Opened);
}

/// Keep ends the showing and leaves the view open where it is; the daemon
/// and the Journal say Kept.
#[test]
fn keep_leaves_the_view_open_and_the_journal_says_kept() {
    let runtime = runtime();
    let (_world, clients, session, agent) = started(&runtime);
    let views = clients.views.clone();
    let mut app = App::open(&runtime, clients);
    on_own_page(&mut app, &session);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);
    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    let opened = seen(&app, &session);

    send(&mut app, AppAction::Views(ViewsAction::Keep));
    assert_eq!(seen(&app, &session), opened, "Keep changes nothing on the page");
    app.ui.read(|state| {
        assert_eq!(showing(state), None);
        assert!(view_tab(state, &session, &id).0, "the view's tab stays");
        assert_eq!(app_state(state, &id), Some(ViewState::Kept));
        assert_eq!(in_journal(state, &session, &id), vec![ViewState::Kept]);
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Kept);
}

/// What the person chose outlives the app: started again, it lists the
/// reverted view in the Journal and not as an offer, still offers the view
/// nobody answered, and opens nothing by itself.
#[test]
fn a_restarted_app_has_the_answered_view_in_the_journal_and_the_other_still_offered() {
    let runtime = runtime();
    let (world, clients, session, agent) = started(&runtime);
    let views = clients.views.clone();
    let mut app = App::open(&runtime, clients);
    on_own_page(&mut app, &session);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);
    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    send(&mut app, AppAction::Views(ViewsAction::Revert));
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Reverted);
    let waiting = runtime.block_on(agent.offer("Second look"));
    wait_for_offer(&mut app, &session, &waiting);
    drop(app);

    let clients = runtime.block_on(world.clients()).expect("connect again");
    let mut app = App::open(&runtime, clients);
    app.wait_until("the restarted app to list both views", |app| {
        app.ui.read(|state| {
            app_state(state, &id) == Some(ViewState::Reverted)
                && app_state(state, &waiting) == Some(ViewState::Waiting)
        })
    });
    send(&mut app, AppAction::Navigate(Route::Session(session.clone())));
    app.ui.read(|state| {
        assert_eq!(in_journal(state, &session, &id), vec![ViewState::Reverted]);
        assert_eq!(offered(state, &session), vec![waiting.clone()], "only the unanswered view is offered");
        assert_eq!(showing(state), None, "a restart opens no view");
        assert!(!view_tab(state, &session, &id).0);
        assert!(!view_tab(state, &session, &waiting).0);
    });
}
