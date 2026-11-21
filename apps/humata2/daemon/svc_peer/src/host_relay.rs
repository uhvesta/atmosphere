//! The host's view of each machine's live peer link, for the services that
//! relay calls to a machine (D139, TR-2, TR-6).
//!
//! The link supervisor tells `Links` when a machine's link comes up (with
//! the link's gRPC channel) and goes down. Other services ask it for the
//! machine's channel, or for a typed client on it, and follow a pushed
//! up/down state (TR-7: nothing polls, nothing sleeps here). A call made
//! while the machine is down fails at once with the API's offline error
//! (HO-5); it never waits for a link (TR-6).
//!
//! Every call through a relayed client is a fresh request. The host is seen
//! on the machine as the user (D139), so callers must build the request from
//! the message alone and never copy the app's or the CLI's inbound metadata:
//! that would forward their credentials. The only header added is
//! `humata-host-assigned-id`, through [`assigned`].

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::pb;
use tokio::sync::{broadcast, watch};
use tonic::metadata::{MetadataValue, errors::InvalidMetadataValue};
use tonic::transport::Channel;
use tonic::{Code, Request, Status};

use humata2_rpc_kit::lock;
use crate::ports as status;

/// The header carrying the id the host minted for what it asks the machine
/// to create (D139: "the host writes its record first with an id it mints").
pub const HOST_ASSIGNED_ID: &str = "humata-host-assigned-id";

/// The header naming a channel's folder on a call that starts a channel's
/// Desk session on a machine that holds no record of the channel (MV-17).
pub const CHANNEL_FOLDER: &str = "humata-channel-folder";

/// How many state changes `watch_all` holds for a slow follower before it
/// resynchronises it from the current states (TR-8).
const FOLLOW_BACKLOG: usize = 256;

/// Whether a machine's link is up. `generation` grows with every new link, so
/// a holder of an old channel can tell it must re-open its streams (TR-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Down,
    Up { generation: u64 },
}

/// Why a relayed call can't be made.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error(
        "{machine} doesn't answer. Its sessions keep running; use Reconnect now on the machine's page once it is reachable"
    )]
    Offline { machine: String },
    #[error("the id {id:?} can't be sent as a header; ids are plain ASCII, so mint it again")]
    BadAssignedId {
        id: String,
        #[source]
        source: InvalidMetadataValue,
    },
}

impl From<RelayError> for Status {
    fn from(error: RelayError) -> Self {
        let message = error.to_string();
        match error {
            RelayError::Offline { machine } => status::failure(
                Code::Unavailable,
                status::error(
                    message,
                    "Sessions there keep running. Use Reconnect now on the machine's page, then try again.",
                    pb::error::Kind::MachineOffline(pb::MachineOffline {
                        machine: None,
                        machine_name: machine,
                        since: None,
                    }),
                ),
            ),
            RelayError::BadAssignedId { .. } => Status::internal(message),
        }
    }
}

struct Slot {
    state: watch::Sender<LinkState>,
    channel: Option<Channel>,
    /// The last generation handed out; kept across down so the next link is
    /// always newer.
    generation: u64,
}

struct Inner {
    machines: Mutex<BTreeMap<String, Slot>>,
    changes: broadcast::Sender<(String, LinkState)>,
}

/// The live link of every machine, keyed by machine name. Cheap to clone.
#[derive(Clone)]
pub struct Links {
    inner: Arc<Inner>,
}

impl Default for Links {
    fn default() -> Self {
        let (changes, _) = broadcast::channel(FOLLOW_BACKLOG);
        Links {
            inner: Arc::new(Inner {
                machines: Mutex::new(BTreeMap::new()),
                changes,
            }),
        }
    }
}

impl Links {
    /// `machine`'s link is up on `channel`. Returns the new generation.
    pub fn up(&self, machine: &str, channel: Channel) -> u64 {
        let mut machines = lock(&self.inner.machines);
        let slot = slot(&mut machines, machine);
        slot.generation += 1;
        slot.channel = Some(channel);
        let state = LinkState::Up {
            generation: slot.generation,
        };
        slot.state.send_replace(state);
        let _ = self.inner.changes.send((machine.to_owned(), state));
        slot.generation
    }

    /// `machine`'s link is gone. Calls fail as offline from now on.
    pub fn down(&self, machine: &str) {
        let mut machines = lock(&self.inner.machines);
        let slot = slot(&mut machines, machine);
        let was_up = slot.channel.take().is_some();
        if was_up {
            slot.state.send_replace(LinkState::Down);
            let _ = self.inner.changes.send((machine.to_owned(), LinkState::Down));
        }
    }

    /// `machine` was removed: its watchers end.
    pub fn forget(&self, machine: &str) {
        let removed = lock(&self.inner.machines).remove(machine);
        if removed.is_some_and(|slot| slot.channel.is_some()) {
            let _ = self.inner.changes.send((machine.to_owned(), LinkState::Down));
        }
    }

    /// The live link's channel, or the offline error. Never waits (TR-6).
    pub fn channel(&self, machine: &str) -> Result<Channel, RelayError> {
        self.current(machine).map(|(_, channel)| channel)
    }

    /// A typed client factory on the live link.
    pub fn relayed(&self, machine: &str) -> Result<Relayed, RelayError> {
        let (generation, channel) = self.current(machine)?;
        Ok(Relayed { channel, generation })
    }

    fn current(&self, machine: &str) -> Result<(u64, Channel), RelayError> {
        lock(&self.inner.machines)
            .get(machine)
            .and_then(|slot| slot.channel.clone().map(|channel| (slot.generation, channel)))
            .ok_or_else(|| RelayError::Offline {
                machine: machine.to_owned(),
            })
    }

    /// `machine`'s state, pushed on every change (TR-7). A machine never
    /// seen is `Down`. The receiver reports closed once the machine is
    /// forgotten.
    pub fn watch(&self, machine: &str) -> watch::Receiver<LinkState> {
        slot(&mut lock(&self.inner.machines), machine).state.subscribe()
    }

    /// Every machine's changes, for callers that follow them all. A follower
    /// that falls behind is brought up to date from the current states, so
    /// it sees the latest of each machine (TR-8).
    pub fn watch_all(&self) -> Follow {
        Follow {
            links: self.clone(),
            changes: self.inner.changes.subscribe(),
            catch_up: VecDeque::new(),
        }
    }

    /// Every known machine and its state now.
    pub fn states(&self) -> Vec<(String, LinkState)> {
        lock(&self.inner.machines)
            .iter()
            .map(|(name, slot)| (name.clone(), *slot.state.borrow()))
            .collect()
    }

    /// Runs `run(channel, generation)` once for each link `machine` has,
    /// dropping the previous run when the link drops or a newer link comes
    /// up, and returns when the machine is forgotten. An attached terminal
    /// or an event relay uses it to catch up after a reconnect (HO-6). A run
    /// that finishes by itself isn't started again for the same link.
    pub async fn reopen_on_new_link<F, Fut>(&self, machine: &str, mut run: F)
    where
        F: FnMut(Channel, u64) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut state = self.watch(machine);
        loop {
            let live = *state.borrow_and_update();
            match (live, self.current(machine)) {
                (LinkState::Up { .. }, Ok((generation, channel))) => {
                    let work = run(channel, generation);
                    tokio::pin!(work);
                    let mut finished = false;
                    loop {
                        tokio::select! {
                            () = &mut work, if !finished => finished = true,
                            changed = state.changed() => {
                                if changed.is_err() {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                }
                _ => {
                    if state.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn slot<'a>(machines: &'a mut BTreeMap<String, Slot>, machine: &str) -> &'a mut Slot {
    machines.entry(machine.to_owned()).or_insert_with(|| Slot {
        state: watch::channel(LinkState::Down).0,
        channel: None,
        generation: 0,
    })
}

/// The stream of every machine's state changes, from [`Links::watch_all`].
pub struct Follow {
    links: Links,
    changes: broadcast::Receiver<(String, LinkState)>,
    catch_up: VecDeque<(String, LinkState)>,
}

impl Follow {
    /// The next change; none once the registry is gone.
    pub async fn next(&mut self) -> Option<(String, LinkState)> {
        loop {
            if let Some(change) = self.catch_up.pop_front() {
                return Some(change);
            }
            match self.changes.recv().await {
                Ok(change) => return Some(change),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    self.catch_up = self.links.states().into();
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Typed clients on one machine's live link, for the services D139 allowlists.
/// A client made here belongs to `generation`: when the machine's state shows
/// a newer one, open it again (TR-4).
pub struct Relayed {
    pub channel: Channel,
    pub generation: u64,
}

impl Relayed {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn terminal(&self) -> pb::terminal_service_client::TerminalServiceClient<Channel> {
        pb::terminal_service_client::TerminalServiceClient::new(self.channel.clone())
    }

    pub fn file(&self) -> pb::file_service_client::FileServiceClient<Channel> {
        pb::file_service_client::FileServiceClient::new(self.channel.clone())
    }

    pub fn trash(&self) -> pb::trash_service_client::TrashServiceClient<Channel> {
        pb::trash_service_client::TrashServiceClient::new(self.channel.clone())
    }


    pub fn session(&self) -> pb::session_service_client::SessionServiceClient<Channel> {
        pb::session_service_client::SessionServiceClient::new(self.channel.clone())
    }

    pub fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.channel.clone())
    }

    pub fn channels(&self) -> pb::channel_service_client::ChannelServiceClient<Channel> {
        pb::channel_service_client::ChannelServiceClient::new(self.channel.clone())
    }

    pub fn diff(&self) -> pb::diff_service_client::DiffServiceClient<Channel> {
        pb::diff_service_client::DiffServiceClient::new(self.channel.clone())
    }

    pub fn carry(&self) -> pb::session_carry_service_client::SessionCarryServiceClient<Channel> {
        pb::session_carry_service_client::SessionCarryServiceClient::new(self.channel.clone())
    }


    pub fn marks(&self) -> pb::mark_service_client::MarkServiceClient<Channel> {
        pb::mark_service_client::MarkServiceClient::new(self.channel.clone())
    }

    pub fn operation(&self) -> pb::operation_service_client::OperationServiceClient<Channel> {
        pb::operation_service_client::OperationServiceClient::new(self.channel.clone())
    }

}

/// A fresh request for `message`, carrying `id` as the host-assigned id when
/// given. It copies nothing from any inbound request.
pub fn assigned<T>(message: T, id: Option<&str>) -> Result<Request<T>, RelayError> {
    let mut request = Request::new(message);
    if let Some(id) = id {
        let value = MetadataValue::try_from(id).map_err(|source| RelayError::BadAssignedId {
            id: id.to_owned(),
            source,
        })?;
        request.metadata_mut().insert(HOST_ASSIGNED_ID, value);
    }
    Ok(request)
}

/// As [`assigned`], also naming the channel's folder (see [`CHANNEL_FOLDER`]).
pub fn assigned_in<T>(message: T, id: &str, folder: &str) -> Result<Request<T>, RelayError> {
    let mut request = assigned(message, Some(id))?;
    let value = MetadataValue::try_from(folder).map_err(|source| RelayError::BadAssignedId {
        id: folder.to_owned(),
        source,
    })?;
    request.metadata_mut().insert(CHANNEL_FOLDER, value);
    Ok(request)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use hyper_util::rt::TokioIo;
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
    use tonic::transport::server::Connected;
    use tonic::transport::{Endpoint, Server, Uri};

    use super::*;

    /// The machine's end of an in-memory link.
    struct Served(DuplexStream);

    impl Connected for Served {
        type ConnectInfo = ();

        fn connect_info(&self) -> Self::ConnectInfo {}
    }

    impl AsyncRead for Served {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Served {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    /// A machine that answers `GetServerInfo` with its tag as the version
    /// and remembers the header names of each call.
    struct Tagged {
        tag: String,
        seen: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[tonic::async_trait]
    impl pb::daemon_service_server::DaemonService for Tagged {
        async fn get_server_info(
            &self,
            request: Request<pb::GetServerInfoRequest>,
        ) -> Result<tonic::Response<pb::ServerInfo>, Status> {
            let names = request
                .metadata()
                .keys()
                .map(|key| match key {
                    tonic::metadata::KeyRef::Ascii(key) => key.as_str().to_owned(),
                    tonic::metadata::KeyRef::Binary(key) => key.as_str().to_owned(),
                })
                .collect();
            lock(&self.seen).push(names);
            Ok(tonic::Response::new(pb::ServerInfo {
                daemon_version: self.tag.clone(),
                ..Default::default()
            }))
        }

        async fn who_am_i(
            &self,
            _request: Request<pb::WhoAmIRequest>,
        ) -> Result<tonic::Response<pb::WhoAmIResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn begin_relay(
            &self,
            _request: Request<pb::BeginRelayRequest>,
        ) -> Result<tonic::Response<pb::BeginRelayResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn complete_relay(
            &self,
            _request: Request<pb::CompleteRelayRequest>,
        ) -> Result<tonic::Response<pb::CompleteRelayResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn repair_restart(
            &self,
            _request: Request<pb::RepairRestartRequest>,
        ) -> Result<tonic::Response<pb::RepairRestartResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn stop_sessions(
            &self,
            _request: Request<pb::StopSessionsRequest>,
        ) -> Result<tonic::Response<pb::StopSessionsResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn start_sessions(
            &self,
            _request: Request<pb::StartSessionsRequest>,
        ) -> Result<tonic::Response<pb::StartSessionsResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn upgrade(
            &self,
            _request: Request<pb::UpgradeRequest>,
        ) -> Result<tonic::Response<pb::UpgradeResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        type PeerUplinkStream = tokio_stream::wrappers::ReceiverStream<Result<pb::MachineToHost, Status>>;

        async fn peer_uplink(
            &self,
            _request: Request<tonic::Streaming<pb::HostToMachine>>,
        ) -> Result<tonic::Response<Self::PeerUplinkStream>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }

    /// A channel to an in-memory machine tagged `tag`.
    async fn machine(tag: &str, seen: Arc<Mutex<Vec<Vec<String>>>>) -> Channel {
        let (host, machine) = tokio::io::duplex(64 * 1024);
        let service = pb::daemon_service_server::DaemonServiceServer::new(Tagged {
            tag: tag.to_owned(),
            seen,
        });
        tokio::spawn(
            Server::builder()
                .add_service(service)
                .serve_with_incoming(tokio_stream::once(Ok::<_, std::io::Error>(Served(machine)))),
        );
        let pipe = Arc::new(Mutex::new(Some(host)));
        let connected = Endpoint::from_static("http://relay.test")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let pipe = lock(&pipe).take();
                async move { pipe.map(TokioIo::new).ok_or_else(|| std::io::Error::other("used")) }
            }))
            .await;
        connected.expect("the in-memory link connects")
    }

    async fn version(channel: Channel) -> String {
        let mut client = pb::daemon_service_client::DaemonServiceClient::new(channel);
        client
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .expect("the machine answers")
            .into_inner()
            .daemon_version
    }

    fn seen() -> Arc<Mutex<Vec<Vec<String>>>> {
        Arc::default()
    }

    #[tokio::test(start_paused = true)]
    async fn a_machine_with_no_link_is_offline_and_never_waited_for() {
        let links = Links::default();

        let refused = tokio::time::timeout(Duration::from_secs(1), async { links.channel("devbox") })
            .await
            .expect("it answers at once");

        assert!(matches!(refused, Err(RelayError::Offline { .. })));
        let status = Status::from(RelayError::Offline { machine: "devbox".into() });
        assert_eq!(status.code(), Code::Unavailable);
    }

    #[tokio::test(start_paused = true)]
    async fn the_channel_reaches_the_machine_while_the_link_is_up_and_not_after() {
        let links = Links::default();
        links.up("devbox", machine("one", seen()).await);

        assert_eq!(version(links.channel("devbox").unwrap()).await, "one");

        links.down("devbox");
        assert!(matches!(links.channel("devbox"), Err(RelayError::Offline { .. })));
        assert!(links.relayed("devbox").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn watchers_see_every_change_in_order() {
        let links = Links::default();
        let mut one = links.watch("devbox");
        let mut all = links.watch_all();
        assert_eq!(*one.borrow(), LinkState::Down);
        assert_eq!(*links.watch("never-seen").borrow(), LinkState::Down);

        let first = machine("one", seen()).await;
        let second = machine("two", seen()).await;
        links.up("devbox", first);
        one.changed().await.unwrap();
        assert_eq!(*one.borrow_and_update(), LinkState::Up { generation: 1 });
        links.down("devbox");
        one.changed().await.unwrap();
        assert_eq!(*one.borrow_and_update(), LinkState::Down);
        links.up("devbox", second);
        one.changed().await.unwrap();
        assert_eq!(*one.borrow_and_update(), LinkState::Up { generation: 2 });

        let mut states = Vec::new();
        for _ in 0..3 {
            states.push(all.next().await.unwrap());
        }
        assert_eq!(
            states,
            vec![
                ("devbox".to_owned(), LinkState::Up { generation: 1 }),
                ("devbox".to_owned(), LinkState::Down),
                ("devbox".to_owned(), LinkState::Up { generation: 2 }),
            ]
        );
        // Nothing more is pending, and nothing fires on its own.
        assert!(tokio::time::timeout(Duration::from_secs(3600), all.next()).await.is_err());
        assert!(!one.has_changed().unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_follower_is_caught_up_to_the_latest_of_each_machine() {
        let links = Links::default();
        let mut all = links.watch_all();
        let channel = machine("one", seen()).await;
        for _ in 0..(FOLLOW_BACKLOG + 10) {
            links.up("devbox", channel.clone());
            links.down("devbox");
        }
        links.up("devbox", channel);

        let mut last = None;
        while let Ok(Some(change)) = tokio::time::timeout(Duration::from_millis(1), all.next()).await {
            last = Some(change);
        }

        assert_eq!(last.map(|(_, state)| state), Some(LinkState::Up { generation: (FOLLOW_BACKLOG as u64 + 10) + 1 }));
    }

    #[tokio::test(start_paused = true)]
    async fn forgetting_a_machine_ends_its_watchers() {
        let links = Links::default();
        let mut one = links.watch("devbox");

        links.forget("devbox");

        assert!(one.changed().await.is_err());
        assert!(links.states().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn reopen_runs_once_per_link_and_cancels_the_run_when_the_link_drops() {
        let links = Links::default();
        let runs = Arc::new(Mutex::new(Vec::<(u64, String)>::new()));
        let cancelled = Arc::new(Mutex::new(Vec::<u64>::new()));

        struct Cancelled(u64, Arc<Mutex<Vec<u64>>>);
        impl Drop for Cancelled {
            fn drop(&mut self) {
                lock(&self.1).push(self.0);
            }
        }

        let follower = {
            let links = links.clone();
            let runs = runs.clone();
            let cancelled = cancelled.clone();
            tokio::spawn(async move {
                links
                    .reopen_on_new_link("devbox", |channel, generation| {
                        let runs = runs.clone();
                        let guard = Cancelled(generation, cancelled.clone());
                        async move {
                            let _guard = guard;
                            let tag = version(channel).await;
                            lock(&runs).push((generation, tag));
                            std::future::pending::<()>().await;
                        }
                    })
                    .await;
            })
        };

        links.up("devbox", machine("one", seen()).await);
        settle().await;
        assert_eq!(*lock(&runs), vec![(1, "one".to_owned())]);
        assert!(lock(&cancelled).is_empty());

        links.down("devbox");
        settle().await;
        assert_eq!(*lock(&cancelled), vec![1]);

        links.up("devbox", machine("two", seen()).await);
        settle().await;
        assert_eq!(*lock(&runs), vec![(1, "one".to_owned()), (2, "two".to_owned())]);

        links.forget("devbox");
        follower.await.expect("the follower ends when the machine is forgotten");
        assert_eq!(*lock(&cancelled), vec![1, 2]);
    }

    /// Lets every ready task run, on the paused clock.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_assigned_id_is_added_to_a_relayed_call() {
        let calls = seen();
        let links = Links::default();
        links.up("devbox", machine("one", calls.clone()).await);
        let mut client = pb::daemon_service_client::DaemonServiceClient::new(links.channel("devbox").unwrap());

        client
            .get_server_info(assigned(pb::GetServerInfoRequest {}, Some("se_01")).unwrap())
            .await
            .unwrap();
        client
            .get_server_info(assigned(pb::GetServerInfoRequest {}, None).unwrap())
            .await
            .unwrap();

        let calls = lock(&calls);
        assert!(calls[0].contains(&HOST_ASSIGNED_ID.to_owned()));
        assert!(!calls[0].iter().any(|name| name.contains("authorization") || name.contains("cookie")));
        assert!(!calls[1].contains(&HOST_ASSIGNED_ID.to_owned()));
    }

    #[test]
    fn an_id_that_is_not_a_header_value_is_refused() {
        let refused = assigned(pb::GetServerInfoRequest {}, Some("bad\nid"));
        assert!(matches!(refused, Err(RelayError::BadAssignedId { .. })));
    }
}
