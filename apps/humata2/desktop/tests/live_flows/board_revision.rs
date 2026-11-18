//! A board item moved from the app's drawer while an agent moved the same
//! item (decision D383, reversed): the drawer holds the old copy, so its move
//! is refused as a conflict; the app shows why, refreshes to the agent's
//! column, keeps the drawer open, and a second move then lands.
//!
//! The agent is the world's running session, calling the daemon's
//! BoardService with the session token its tab was given, as its MCP call
//! does. It makes its own item (an agent's own items need no approval) and
//! later moves it; each call is awaited to its reply, so the test never
//! polls for it.
//!
//! The app's event feed goes through [`Held`], which the test closes before
//! the agent's move and opens after the refusal. While it is closed the app
//! can settle and be clicked without hearing of the move, so the drawer
//! still holds the old revision when the person picks Done.
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_clients::model::{
    Agent, Answer, Archive, Capability, ChannelId, DiffBase, GrantId, Item, ItemId, ItemStatus,
    MachineId, PushChoice, PushPlan, PushedPullRequest, Recovery, RepoBases, RepoDiff, RequestId,
    RuleSetting, Scope, SessionFile, SessionId, SessionOptions, SetupId, TransferId,
};
use humata2_clients::{
    ArchivePlan, Clients, Daemon, EVENT_QUEUE, Events, ForkOptions, PastTurn, Reply, Workspace,
};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};

use super::world::World;
use super::{App, running_session, runtime};

/// The header a session token travels in (identity.rs).
const TOKEN_HEADER: &str = "humata-session-token";
const FIRST_MESSAGE: &str = "hello agent";
const ITEM_TITLE: &str = "Agent item";
const SESSION: &str = "Ledger agent";

/// The live daemon, with its event feed behind a gate the test opens and
/// closes. Every call but `events` goes straight through, so a refresh the
/// app asks for reads the daemon as it is now.
struct Held {
    inner: Arc<dyn Daemon>,
    open: watch::Sender<bool>,
}

impl Held {
    fn new(inner: Arc<dyn Daemon>) -> Self {
        Self {
            inner,
            open: watch::Sender::new(true),
        }
    }

    /// Events the daemon sends from now on wait until [`Held::release`].
    fn hold(&self) {
        self.open.send_replace(false);
    }

    /// Delivers what waited, in order, and what comes after.
    fn release(&self) {
        self.open.send_replace(true);
    }
}

impl Daemon for Held {
    fn events(&self) -> Reply<Events> {
        let inner = self.inner.clone();
        let mut open = self.open.subscribe();
        Box::pin(async move {
            let mut from = inner.events().await?;
            let (to, events) = mpsc::channel(EVENT_QUEUE);
            tokio::spawn(async move {
                while let Some(event) = from.recv().await {
                    if open.wait_for(|open| *open).await.is_err() {
                        return;
                    }
                    if to.send(event).await.is_err() {
                        return;
                    }
                }
            });
            Ok(events)
        })
    }

    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String> {
        self.inner.read_file(session, repo, path)
    }
    fn diffs(&self, session: &SessionId, base: &DiffBase) -> Reply<Vec<RepoDiff>> {
        self.inner.diffs(session, base)
    }
    fn bases(&self, session: &SessionId, repo: &str) -> Reply<RepoBases> {
        self.inner.bases(session, repo)
    }
    fn find_files(&self, session: &SessionId) -> Reply<Vec<SessionFile>> {
        self.inner.find_files(session)
    }
    fn plan_push(&self, session: &SessionId) -> Reply<PushPlan> {
        self.inner.plan_push(session)
    }
    fn push(
        &self,
        session: &SessionId,
        token: &str,
        choices: Vec<PushChoice>,
    ) -> Reply<Vec<PushedPullRequest>> {
        self.inner.push(session, token, choices)
    }
    fn mark_reviewed(&self, session: &SessionId) -> Reply<()> {
        self.inner.mark_reviewed(session)
    }
    fn load(&self) -> Reply<Workspace> {
        self.inner.load()
    }
    fn send(&self, session: &SessionId, text: &str) -> Reply<()> {
        self.inner.send(session, text)
    }
    fn send_keyed(&self, session: &SessionId, text: &str, key: &str) -> Reply<()> {
        self.inner.send_keyed(session, text, key)
    }
    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()> {
        self.inner.answer(request, answer)
    }
    fn reconnect(&self, machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>> {
        self.inner.reconnect(machine)
    }
    fn resume(&self, session: &SessionId) -> Reply<()> {
        self.inner.resume(session)
    }
    fn start_fresh(&self, session: &SessionId) -> Reply<()> {
        self.inner.start_fresh(session)
    }
    fn fork(&self, session: &SessionId, options: ForkOptions) -> Reply<SessionId> {
        self.inner.fork(session, options)
    }
    fn list_turns(&self, session: &SessionId) -> Reply<Vec<PastTurn>> {
        self.inner.list_turns(session)
    }
    fn move_session(&self, session: &SessionId, to: &MachineId) -> Reply<()> {
        self.inner.move_session(session, to)
    }
    fn rename_session(&self, session: &SessionId, title: &str, rename_branch: bool) -> Reply<()> {
        self.inner.rename_session(session, title, rename_branch)
    }
    fn stop_agent(&self, session: &SessionId) -> Reply<()> {
        self.inner.stop_agent(session)
    }
    fn plan_archive(&self, session: &SessionId) -> Reply<ArchivePlan> {
        self.inner.plan_archive(session)
    }
    fn archive(&self, session: &SessionId, push_first: bool) -> Reply<Archive> {
        self.inner.archive(session, push_first)
    }
    fn add_machine(&self, address: &str) -> Reply<MachineId> {
        self.inner.add_machine(address)
    }
    fn restore_session(&self, session: &SessionId) -> Reply<()> {
        self.inner.restore_session(session)
    }
    fn set_pinned(&self, session: &SessionId, pinned: bool) -> Reply<()> {
        self.inner.set_pinned(session, pinned)
    }
    fn set_vip(&self, session: &SessionId, vip: bool) -> Reply<()> {
        self.inner.set_vip(session, vip)
    }
    fn mark_read(&self, session: &SessionId) -> Reply<()> {
        self.inner.mark_read(session)
    }
    fn view(&self, session: Option<&SessionId>) -> Reply<()> {
        self.inner.view(session)
    }
    fn create_session(
        &self,
        channel: &ChannelId,
        agent: Agent,
        machine: &MachineId,
        title: &str,
        options: SessionOptions,
    ) -> Reply<SessionId> {
        self.inner.create_session(channel, agent, machine, title, options)
    }
    fn create_channel(
        &self,
        name: &str,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Reply<ChannelId> {
        self.inner.create_channel(name, repos, machines)
    }
    fn github_repos(&self, query: &str) -> Reply<Vec<String>> {
        self.inner.github_repos(query)
    }
    fn cancel_setup(&self, setup: &SetupId) -> Reply<()> {
        self.inner.cancel_setup(setup)
    }
    fn retry_setup(&self, setup: &SetupId) -> Reply<ChannelId> {
        self.inner.retry_setup(setup)
    }
    fn move_item(&self, item: &ItemId, status: ItemStatus, seen: Option<u64>) -> Reply<()> {
        self.inner.move_item(item, status, seen)
    }
    fn create_item_in(&self, scope: Scope, title: &str, status: ItemStatus) -> Reply<ItemId> {
        self.inner.create_item_in(scope, title, status)
    }
    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        self.inner.add_dependency(item, on)
    }
    fn remove_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()> {
        self.inner.remove_dependency(item, on)
    }
    fn set_item_session(&self, item: &ItemId, session: &SessionId, working: bool) -> Reply<()> {
        self.inner.set_item_session(item, session, working)
    }
    fn item_history(&self, item: &ItemId) -> Reply<Vec<String>> {
        self.inner.item_history(item)
    }
    fn retry_transfer(&self, transfer: &TransferId) -> Reply<()> {
        self.inner.retry_transfer(transfer)
    }
    fn set_rule(
        &self,
        channel: &ChannelId,
        capability: &Capability,
        setting: RuleSetting,
    ) -> Reply<()> {
        self.inner.set_rule(channel, capability, setting)
    }
    fn remove_grant(&self, grant: &GrantId) -> Reply<()> {
        self.inner.remove_grant(grant)
    }

    fn set_default_agent(&self, agent: Agent) -> Reply<()> {
        self.inner.set_default_agent(agent)
    }

    fn codex_needs_trust(&self) -> Reply<bool> {
        self.inner.codex_needs_trust()
    }
    fn agent_switches(&self) -> Reply<Vec<humata2_clients::AgentSwitch>> {
        self.inner.agent_switches()
    }
    fn set_agent_switch(
        &self,
        kind: String,
        change: humata2_clients::SwitchChange,
    ) -> Reply<humata2_clients::AgentSwitch> {
        self.inner.set_agent_switch(kind, change)
    }
}

/// The session's agent as the daemon sees it: a caller with the session
/// token the agent's tab was started with.
struct AgentCalls {
    socket: PathBuf,
    token: String,
}

impl AgentCalls {
    /// The agent of the world's one session, which is running.
    fn of(world: &World) -> Self {
        Self {
            socket: world.socket(),
            token: world.agent_token(),
        }
    }

    async fn boards(&self) -> pb::board_service_client::BoardServiceClient<Channel> {
        let path = self.socket.clone();
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
            }))
            .await
            .expect("the agent connects to humatad2");
        pb::board_service_client::BoardServiceClient::new(channel)
    }

    fn signed<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(
            TOKEN_HEADER,
            MetadataValue::try_from(self.token.as_str()).expect("a session token is header text"),
        );
        request
    }

    /// Makes an item titled `title` in To do on `channel`'s board.
    async fn make(&self, channel: &ChannelId, title: &str) -> ItemId {
        let written = self
            .boards()
            .await
            .create_item(self.signed(pb::CreateItemRequest {
                idempotency_key: key(&format!("make-{title}")),
                board: Some(pb::BoardRef {
                    board: Some(pb::board_ref::Board::Channel(pb::ChannelId {
                        value: channel.as_str().to_owned(),
                    })),
                }),
                title: title.to_owned(),
                column: pb::BoardColumn::ToDo as i32,
                ..Default::default()
            }))
            .await
            .expect("the session's agent makes its own item")
            .into_inner();
        let id = written
            .item
            .and_then(|item| item.id)
            .expect("an applied write returns the item");
        ItemId::new(id.value)
    }

    /// Moves `item` to In review, as the agent last read it: no revision.
    async fn move_to_review(&self, item: &ItemId) {
        self.boards()
            .await
            .move_item(self.signed(pb::MoveItemRequest {
                idempotency_key: key(&format!("review-{}", item.as_str())),
                id: Some(pb::BoardItemId {
                    value: item.as_str().to_owned(),
                }),
                column: pb::BoardColumn::InReview as i32,
                ..Default::default()
            }))
            .await
            .expect("the session's agent moves its own item");
    }
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: value.to_owned(),
    })
}

/// How the view writes an element's line: its role, then its label.
fn heading(role: Role, label: &str) -> String {
    format!("{role:?} {label:?}")
}

fn shows(app: &mut App, role: Role, label: &str) -> bool {
    app.view().contains(&heading(role, label))
}

fn item_in_app(app: &App, id: &ItemId) -> Option<Item> {
    app.ui
        .read(|state| state.workspace.items.iter().find(|item| &item.id == id).cloned())
}

fn item_on_daemon(app: &App, id: &ItemId) -> Item {
    let daemon = app.daemon.clone();
    app.runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .items
        .into_iter()
        .find(|item| &item.id == id)
        .expect("the daemon lists the item")
}

/// Picks `to` from the open drawer's status menu, whose button names the
/// status the drawer shows now.
fn pick_status(app: &mut App, shown: ItemStatus, to: ItemStatus) {
    app.click(Role::Button, shown.name());
    app.wait_until("the status menu", |app| {
        app.ui.read(|state| state.board.status_menu)
    });
    app.click(Role::Button, to.name());
}

#[test]
fn a_drawer_move_after_an_agents_move_is_refused_refreshes_and_then_lands() {
    let runtime = runtime();
    let (world, clients, channel, agent) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let session = running_session(&world, &clients, SESSION, FIRST_MESSAGE).await;
        let channel = clients
            .daemon
            .load()
            .await
            .expect("the daemon loads")
            .sessions
            .into_iter()
            .find(|listed| listed.id == session)
            .expect("the daemon lists the session")
            .channel;
        let agent = AgentCalls::of(&world);
        (world, clients, channel, agent)
    });
    let held = Arc::new(Held::new(clients.daemon.clone()));
    let mut app = App::open(
        &runtime,
        Clients {
            daemon: held.clone(),
            ..clients
        },
    );

    // The agent makes its item; the app hears of it and lists it.
    let id = runtime.block_on(agent.make(&channel, ITEM_TITLE));
    app.wait_until("the agent's item to be listed", |app| {
        item_in_app(app, &id).is_some()
    });
    let item = item_in_app(&app, &id).expect("the app lists the agent's item");
    assert_eq!(item.scope, Scope::Channel(channel.clone()));

    // The channel's page is reached by the navigation a sidebar row sends.
    let page = channel.clone();
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Channel(page)), cx));
    app.settle();
    app.wait_until("the channel's tabs", |app| shows(app, Role::Tab, "Board"));
    // A channel's page opens on the tab it showed last. Already on Board,
    // the board's own view switch shows a Tab "Board" too.
    if !shows(&mut app, Role::Button, "New item") {
        app.click(Role::Tab, "Board");
    }
    let label = format!("{} {}", id.as_str(), ITEM_TITLE);
    app.wait_until("the item's card", |app| shows(app, Role::ListItem, &label));
    app.click(Role::ListItem, &label);
    app.wait_until("the item's drawer", |app| {
        app.ui.read(|state| state.board.drawer.as_ref() == Some(&id))
    });
    let seen = item_in_app(&app, &id).expect("the app lists the item");

    // The agent moves its item while the app hears nothing more.
    held.hold();
    runtime.block_on(agent.move_to_review(&id));
    let moved = item_on_daemon(&app, &id);
    assert_eq!(moved.status, ItemStatus::InReview, "the agent's move landed");
    assert!(moved.revision > seen.revision, "the move raised the item's revision");
    app.settle();
    assert_eq!(
        item_in_app(&app, &id).map(|item| item.revision),
        Some(seen.revision),
        "the drawer still holds the copy from before the agent's move"
    );

    // The person moves it to Done from the drawer's status menu.
    pick_status(&mut app, seen.status, ItemStatus::Done);
    app.wait_until("the drawer to show the refusal", |app| {
        app.ui.read(|state| state.board.error.is_some())
    });
    // The refusal reads the daemon again: the app holds the agent's column
    // though its event feed is still held, and the drawer stays open.
    app.wait_until("the app to hold the agent's column", |app| {
        item_in_app(app, &id).is_some_and(|item| item.status == ItemStatus::InReview)
    });
    assert!(
        app.ui
            .read(|state| state.board.drawer.as_ref() == Some(&id)),
        "the drawer stays open"
    );
    assert_eq!(
        item_on_daemon(&app, &id).status,
        ItemStatus::InReview,
        "the refused move did not overwrite the agent's"
    );

    // With the feed open again and the fresh copy shown, the same move lands.
    held.release();
    pick_status(&mut app, ItemStatus::InReview, ItemStatus::Done);
    app.wait_until("the app to hear the item is Done", |app| {
        item_in_app(app, &id).is_some_and(|item| item.status == ItemStatus::Done)
    });
    assert_eq!(item_on_daemon(&app, &id).status, ItemStatus::Done);
    assert!(
        app.ui.read(|state| state.board.error.is_none()),
        "the landed move clears the refusal"
    );
    drop(app);
    drop(world);
}
