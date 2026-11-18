//! ListViews with include_forks (VW-10): the Journal lists a session's
//! offered views with those of its forks and of their forks, newest first.
//! A fork's draft is only its own agent's (VW-14), and without include_forks
//! only the session's own views are listed.
//!
//! The forks are seeded in the store as the fork service records them. Each
//! session's agent gets its token from a shell tab in the session, as
//! tests/review.rs does, and a view is offered when its agent marks it ready.
//! Views are listed by the daemon's answer, not by waiting on anything.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ForkOrigin, SessionRecord, Store};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

const TOKEN_HEADER: &str = "humata-session-token";
const CHANNEL: &str = "ch_test";
const REPO: &str = "api";
const ROOT: &str = "se_root";
/// A fork of ROOT.
const FORK: &str = "se_fork";
/// A fork of FORK.
const GRAND: &str = "se_grand";
/// Another fork of ROOT.
const SIDE: &str = "se_side";
/// A session that is not a fork of ROOT.
const OTHER: &str = "se_other";

/// A session's agent: its token, and the shell tab that keeps it open.
struct Agent {
    token: String,
    _attach: Attach,
}

struct World {
    daemon: Daemon,
    root: Agent,
    fork: Agent,
    grand: Agent,
    side: Agent,
    other: Agent,
}

impl World {
    /// ROOT with forks FORK and SIDE, GRAND under FORK, and OTHER, which is
    /// no fork of ROOT. Every session's agent is started.
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for id in [ROOT, FORK, GRAND, SIDE, OTHER] {
            store.put_session(&session(id, dir.path())).unwrap();
        }
        for (fork, parent) in [(FORK, ROOT), (GRAND, FORK), (SIDE, ROOT)] {
            store
                .put_fork_origin(&ForkOrigin {
                    session_id: fork.to_owned(),
                    parent: parent.to_owned(),
                    kind: pb::ForkKind::CodeAndContext as i32,
                    before_turn: 0,
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let root = agent(&daemon, ROOT).await;
        let fork = agent(&daemon, FORK).await;
        let grand = agent(&daemon, GRAND).await;
        let side = agent(&daemon, SIDE).await;
        let other = agent(&daemon, OTHER).await;
        Self {
            daemon,
            root,
            fork,
            grand,
            side,
            other,
        }
    }
}

fn session(id: &str, folder: &Path) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: CHANNEL.to_owned(),
        machine: String::new(),
        title: id.to_owned(),
        agent: "zsh".to_owned(),
        folder: folder.to_owned(),
        created_unix_ms: 0,
    }
}

/// The session's agent, with the token a shell tab in the session is given.
async fn agent(daemon: &Daemon, session: &str) -> Agent {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    let path = daemon.dir.path().join(format!("token-{}", tab.value));
    attach
        .type_text(&format!("printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r", path.display()))
        .await;
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(token) = std::fs::read_to_string(&path)
            && !token.is_empty()
        {
            return Agent {
                token,
                _attach: attach,
            };
        }
        assert!(Instant::now() < deadline, "the shell never wrote its token");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn views_client(daemon: &Daemon) -> pb::view_service_client::ViewServiceClient<Channel> {
    pb::view_service_client::ViewServiceClient::new(daemon.channel().await)
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

/// The agent prepares a walkthrough titled `title`: a draft. Its id.
async fn draft(daemon: &Daemon, agent: &Agent, title: &str) -> String {
    let request = pb::PrepareViewRequest {
        idempotency_key: None,
        title: title.to_owned(),
        summary: "Read the refund first".to_owned(),
        content: Some(pb::prepare_view_request::Content::Review(pb::ReviewWalkthrough {
            steps: vec![pb::review_walkthrough::Step {
                file: Some(pb::RepoPath {
                    repo: Some(pb::RepoRef {
                        owner: "acme".to_owned(),
                        name: REPO.to_owned(),
                    }),
                    path: "refund.rs".to_owned(),
                }),
                lines: Some(pb::LineRange { first: 1, last: 3 }),
                note: "the refund starts here".to_owned(),
            }],
        })),
    };
    views_client(daemon)
        .await
        .prepare_view(as_agent(request, &agent.token))
        .await
        .expect("the agent prepares a view")
        .into_inner()
        .id
        .expect("a prepared view has an id")
        .value
}

/// The agent prepares a view titled `title` and marks it ready. Its id.
async fn offer(daemon: &Daemon, agent: &Agent, title: &str) -> String {
    let id = draft(daemon, agent, title).await;
    let ready = views_client(daemon)
        .await
        .mark_ready(as_agent(
            pb::MarkReadyRequest {
                idempotency_key: None,
                id: Some(pb::ViewId { value: id.clone() }),
            },
            &agent.token,
        ))
        .await
        .expect("the agent marks its view ready")
        .into_inner();
    assert_eq!(ready.state, pb::ViewState::Waiting as i32, "a view marked ready is offered");
    id
}

/// ListViews for `session`: as the user when `caller` is None, else as the
/// agent.
async fn listed(
    daemon: &Daemon,
    caller: Option<&Agent>,
    session: &str,
    include_forks: bool,
) -> Vec<pb::PreparedView> {
    let request = pb::ListViewsRequest {
        session: Some(pb::SessionId {
            value: session.to_owned(),
        }),
        include_forks,
        ..pb::ListViewsRequest::default()
    };
    let request = match caller {
        Some(agent) => as_agent(request, &agent.token),
        None => Request::new(request),
    };
    views_client(daemon)
        .await
        .list_views(request)
        .await
        .expect("the views are listed")
        .into_inner()
        .views
}

fn ids(views: &[pb::PreparedView]) -> Vec<String> {
    let mut ids: Vec<String> = views
        .iter()
        .filter_map(|view| view.id.as_ref().map(|id| id.value.clone()))
        .collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn prepared_at(view: &pb::PreparedView) -> (i64, i32) {
    view.prepared_at
        .as_ref()
        .map_or((0, 0), |at| (at.seconds, at.nanos))
}

/// Each view was prepared no later than the one before it. Views prepared
/// in the same instant may come in either order.
fn newest_first(views: &[pb::PreparedView]) -> bool {
    views
        .windows(2)
        .all(|pair| prepared_at(&pair[0]) >= prepared_at(&pair[1]))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_user_lists_a_sessions_forks_offered_views_newest_first_and_never_a_forks_draft() {
    let world = World::start().await;
    let daemon = &world.daemon;
    let root_view = offer(daemon, &world.root, "Root").await;
    let fork_view = offer(daemon, &world.fork, "Fork").await;
    draft(daemon, &world.fork, "Fork draft").await;
    let grand_view = offer(daemon, &world.grand, "Grand").await;
    let side_view = offer(daemon, &world.side, "Side").await;
    draft(daemon, &world.root, "Root draft").await;
    offer(daemon, &world.other, "Other").await;

    let views = listed(daemon, None, ROOT, true).await;

    assert_eq!(
        ids(&views),
        sorted(vec![root_view, fork_view, grand_view, side_view]),
        "the session's offered views and those of its forks and their forks, with no draft"
    );
    assert!(newest_first(&views), "the views are listed newest first");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_include_forks_a_user_lists_only_the_sessions_own_offered_views() {
    let world = World::start().await;
    let daemon = &world.daemon;
    let root_view = offer(daemon, &world.root, "Root").await;
    offer(daemon, &world.fork, "Fork").await;
    offer(daemon, &world.grand, "Grand").await;
    draft(daemon, &world.root, "Root draft").await;

    let views = listed(daemon, None, ROOT, false).await;

    assert_eq!(ids(&views), vec![root_view], "only the session's own offered view");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_sees_its_own_draft_with_its_forks_offered_views_but_not_a_forks_draft() {
    let world = World::start().await;
    let daemon = &world.daemon;
    let root_view = offer(daemon, &world.root, "Root").await;
    let root_draft = draft(daemon, &world.root, "Root draft").await;
    let fork_view = offer(daemon, &world.fork, "Fork").await;
    draft(daemon, &world.fork, "Fork draft").await;
    let grand_view = offer(daemon, &world.grand, "Grand").await;

    let views = listed(daemon, Some(&world.root), ROOT, true).await;

    assert_eq!(
        ids(&views),
        sorted(vec![root_view, root_draft, fork_view, grand_view]),
        "the agent's own drafts, and the offered views of its forks"
    );
}
