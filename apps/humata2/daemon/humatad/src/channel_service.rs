//! ChannelService (CH-1, CH-2, CH-5): the channels of this machine, made
//! and read from the store. The work (the record, the folders, the main
//! clones) is `humata2_sessions`'; this wires it to the API: the access
//! checks, an `Operation` whose steps change as the clones finish, and the
//! errors in the API's terms. Only this machine can be a channel's machine
//! until machines get channels (MACH); the rest of the service answers
//! "not built".
//!
//! A new channel's branch prefix is empty: the default is the `gh` login
//! (ST-13), and Settings isn't built yet. Its default agent is the one the
//! daemon was told (`--default-agent`).

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, Capability, ChannelId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{GitError, NameError, NewChannel, SetupError, Starter, clone_mains_with, create_channel};
use humata2_store::{ChannelRecord, ChannelRepo, MainClone, Store, StoreError};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Response, Status};

use crate::events::{Feed, Scope};
use crate::guard::{Checked, Guard};
use crate::identity::{self, Caller};
use crate::inbox::{Deferred, Inbox};
use crate::operations::{Operations, Started};
use crate::remote_work::{HostOp, MachineOutcome, RemoteWork, channel_steps};
use crate::status;

mod add;
mod delete;
// REPOS (RL-5): the repo search reuses CH-3's listing.
pub(crate) mod gh_repos;
pub(crate) use delete::{finish_restore, pick_place};
pub(crate) use move_folder::{MoveError, carry_conversations, rewrite_folders};
mod known;
mod library_adds;
mod move_folder;
mod remove_machine;
mod remove_machine_apply;
mod follow_rename;
// CH-9 (D764): the Repos and machines grid.
mod grid;
mod rename_ask;
pub(crate) mod rename_folder;

/// Every channel `caller` may read.
async fn readable_channels(
    guard: &Guard,
    store: &Arc<Store>,
    machine: &str,
    caller: &Caller,
) -> Result<pb::ListChannelsResponse, Status> {
    let listing = store.clone();
    let all = blocking(move || listing.channels())
        .await?
        .map_err(|error| status::store(error, machine, "list the channels"))?;
    let mut channels = Vec::new();
    for channel in all {
        let read = Call::Use {
            capability: Capability::ChannelRead,
            target: Target::Channel(ChannelId::new(channel.id.as_str())),
        };
        if !matches!(guard.check(caller, &read), Ok(Checked::Allowed)) {
            continue;
        }
        let (counting, record) = (store.clone(), channel.clone());
        let (live, agent) = blocking(move || -> Result<_, StoreError> {
            Ok((live_sessions(&counting, &record.id)?, effective_agent(&counting, &record)))
        })
        .await?
        .map_err(|error| status::store(error, machine, "count the channel's sessions"))?;
        channels.push(channel_pb(&channel, live, &agent));
    }
    Ok(pb::ListChannelsResponse {
        channels,
        page: None,
    })
}

/// Puts `channel` as the store holds it now on the feed, as GetChannel
/// shows it. Blocking. A channel that can't be read is not told:
/// publishing never fails the call that changed it.
pub(crate) fn tell_channel(
    feed: &Feed,
    store: &Store,
    channel: &str,
    change: pb::Change,
    by: Option<pb::Actor>,
) {
    let Ok(Some(record)) = store.channel(channel) else {
        return;
    };
    let Ok(live) = live_sessions(store, channel) else {
        return;
    };
    let agent = effective_agent(store, &record);
    feed.publish_in(
        change,
        pb::event::Payload::Channel(channel_pb(&record, live, &agent)),
        Scope {
            channel: channel.to_owned(),
            session: String::new(),
        },
        by,
    );
}

/// The longest channel name taken.
const NAME_MAX: usize = 100;

/// What a guarded handler body got to: the answer, or the rule that sent
/// the call to Ask me before it could run (D38.6).
pub(crate) enum Gated<T> {
    Done(T),
    Ask(Box<AccessRequest>),
}

/// Checks `call` for `caller`. `Ok(None)` means go on; `Ok(Some(..))` means
/// the rule says Ask. An `approved` call (the user said yes in the Inbox)
/// goes on without asking again.
pub(crate) fn gate(
    guard: &Guard,
    caller: &Caller,
    call: &Call,
    approved: bool,
) -> Result<Option<Box<AccessRequest>>, Status> {
    if approved {
        return Ok(None);
    }
    match guard.check(caller, call)? {
        Checked::Allowed => Ok(None),
        Checked::Ask(asked) => Ok(Some(Box::new(asked))),
    }
}

/// Answers a unary call whose body ran as `first`. A body that stopped at an
/// Ask puts the call in the Inbox, with `again` to run it once the user
/// approves it: `again` must do the whole call afresh, with `approved` set,
/// and own what it needs (TP-13). The response's full name is `type_name`.
pub(crate) async fn answer_gated<T, Fut>(
    inbox: &Inbox,
    caller: &Caller,
    key: Option<String>,
    type_name: &'static str,
    first: Result<Gated<T>, Status>,
    again: impl FnOnce() -> Fut + Send + 'static,
) -> Result<Response<T>, Status>
where
    T: Message,
    Fut: Future<Output = Result<Gated<T>, Status>> + Send + 'static,
{
    answer_gated_showing(inbox, caller, key, type_name, first, None, again).await
}

/// Answers a plan call whose body ran as `first`. A plan only reads, so it
/// never asks (D730, decided by Fable): a rule of Never still refuses
/// it, but one of Ask answers the plan as Allow does, and the call that acts
/// on it is the one that waits in the Inbox, so approving that runs the
/// action. `again` runs the body with `approved` set.
pub(crate) async fn answer_plan<T, Fut>(
    first: Result<Gated<T>, Status>,
    again: impl FnOnce() -> Fut,
) -> Result<Response<T>, Status>
where
    Fut: Future<Output = Result<Gated<T>, Status>>,
{
    let done = match first? {
        Gated::Done(done) => done,
        Gated::Ask(_) => match again().await? {
            Gated::Done(done) => done,
            Gated::Ask(_) => {
                return Err(status::internal(
                    "a plan the rule allows was sent to Ask me a second time",
                    "plan it again",
                ));
            }
        },
    };
    Ok(Response::new(done))
}

/// [`answer_gated`] whose card, when the rule says Ask, previews `card`
/// rather than the capability (IN-10).
pub(crate) async fn answer_gated_showing<T, Fut>(
    inbox: &Inbox,
    caller: &Caller,
    key: Option<String>,
    type_name: &'static str,
    first: Result<Gated<T>, Status>,
    card: Option<crate::inbox::BoardAsk>,
    again: impl FnOnce() -> Fut + Send + 'static,
) -> Result<Response<T>, Status>
where
    T: Message,
    Fut: Future<Output = Result<Gated<T>, Status>> + Send + 'static,
{
    match first? {
        Gated::Done(done) => Ok(Response::new(done)),
        Gated::Ask(asked) => {
            let run: crate::inbox::Run = Box::new(move |_| {
                Box::pin(async move {
                    match again().await? {
                        Gated::Done(done) => Ok(Deferred {
                            result: done.encode_to_vec(),
                            result_type: type_name.to_owned(),
                        }),
                        Gated::Ask(_) => Err(status::internal(
                            "an approved call was sent to Ask me a second time",
                            "repeat the call",
                        )),
                    }
                })
            });
            Err(match card {
                Some(card) => inbox.defer_board(caller, *asked, key, card, run).await,
                None => inbox.defer(caller, *asked, key, run).await,
            })
        }
    }
}

#[derive(Clone)]
pub(crate) struct ChannelSvc {
    guard: Arc<Guard>,
    operations: Arc<Operations>,
    starter: Starter,
    /// This machine's id: the only machine a channel can have for now.
    machine: String,
    default_agent: String,
    /// Where a call a rule sends to Ask waits for the user (D38.6).
    inbox: Arc<Inbox>,
    /// The host's way to other machines (D139); none on a machine's own
    /// daemon, which keeps only this machine.
    remote: Option<RemoteWork>,
    /// The host's machine list, so a channel names only machines it has
    /// (D164); none on a machine's own daemon.
    known: Option<Arc<crate::machine_service::Work>>,
    /// One clone at a time per channel (D164).
    cloning: Arc<known::CloneLocks>,
    /// RemoveMachine plans not yet taken (CH-8).
    plans: Arc<remove_machine::Plans>,
    /// DeleteChannel plans and the trash it sends folders to (CH-24).
    deletes: Arc<delete::State>,
    /// The host's SessionService, which moves and archives the sessions of a machine being removed (CH-8); set once.
    sessions: Arc<std::sync::OnceLock<crate::session_service::SessionSvc>>,
    /// The Desk service, which closes a deleted channel's Desk sessions
    /// (CH-24); set once.
    desk: Arc<std::sync::OnceLock<crate::desk_service::channel_desk_close::DeskHandle>>,
    /// The host's repo library, which hears of every repo whose add worked (RL-1); set once.
    library: library_adds::Slot,
}

impl ChannelSvc {
    pub(crate) fn new(
        guard: Arc<Guard>,
        operations: Arc<Operations>,
        starter: Starter,
        machine: String,
        default_agent: String,
        inbox: Arc<Inbox>,
        remote: Option<RemoteWork>,
        known: Option<Arc<crate::machine_service::Work>>,
    ) -> Self {
        Self {
            guard,
            operations,
            starter,
            machine,
            default_agent,
            inbox,
            remote,
            known,
            cloning: Arc::default(),
            plans: Arc::default(),
            deletes: Arc::default(),
            sessions: Arc::default(),
            desk: Arc::default(),
            library: Arc::default(),
        }
    }

    /// Gives RemoveMachine the SessionService that moves and archives the sessions of the machine it removes. Once.
    pub(crate) fn use_sessions(&self, sessions: crate::session_service::SessionSvc) {
        let _ = self.sessions.set(sessions);
    }

    /// Lets a delete close the channel's Desk sessions first (CH-24).
    pub(crate) fn use_desk(&self, desk: crate::desk_service::channel_desk_close::DeskHandle) {
        let _ = self.desk.set(desk);
    }

    fn gate(
        &self,
        caller: &Caller,
        capability: Capability,
        target: Target,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        gate(
            &self.guard,
            caller,
            &Call::Use { capability, target },
            approved,
        )
    }

    async fn find(&self, id: &str) -> Result<ChannelRecord, Status> {
        let store = self.starter.store.clone();
        let key = id.to_owned();
        match blocking(move || store.channel(&key)).await? {
            Ok(Some(channel)) => Ok(channel),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(no_channel(id)),
            Err(error) => Err(status::store(error, &self.machine, "look up the channel")),
        }
    }

    /// `channel` as the API shows it.
    async fn show(&self, channel: ChannelRecord) -> Result<pb::Channel, Status> {
        let store = self.starter.store.clone();
        let record = channel.clone();
        let (live, agent) = blocking(move || -> Result<_, StoreError> {
            Ok((live_sessions(&store, &record.id)?, effective_agent(&store, &record)))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "count the channel's sessions"))?;
        Ok(channel_pb(&channel, live, &agent))
    }

    /// What a repeated call with the same key answers: the channel the
    /// first one made, with its operation.
    async fn answer(&self, setup: pb::Operation) -> Result<pb::CreateChannelResponse, Status> {
        let id = match setup
            .target
            .as_ref()
            .and_then(|target| target.entity.as_ref())
        {
            Some(pb::entity_ref::Entity::Channel(id)) => id.value.clone(),
            _ => {
                return Err(status::internal(
                    "an operation for a new channel named no channel",
                    "",
                ));
            }
        };
        let channel = self.find(&id).await?;
        Ok(pb::CreateChannelResponse {
            channel: Some(self.show(channel).await?),
            setup: Some(setup),
        })
    }

    /// The body of CreateChannel; `approved` once the user said yes to it.
    async fn make_channel(
        &self,
        caller: &Caller,
        request: pb::CreateChannelRequest,
        assigned: Option<String>,
        approved: bool,
    ) -> Result<Gated<pb::CreateChannelResponse>, Status> {
        if let Some(asked) =
            self.gate(caller, Capability::ChannelCreate, Target::Global, approved)?
        {
            return Ok(Gated::Ask(asked));
        }
        if let Some(setup) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(self.answer(setup).await?));
        }
        let name = clean_name(&request.name)?;
        let repos = clean_repos(&request.repos)?;
        let machines = self.machines(&request.machines)?;
        // The machines other than this one get the same channel made by their own
        // daemon, with the id this one chose (D139, D150).
        let remote_machines: Vec<String> = machines
            .iter()
            .filter(|machine| **machine != self.machine)
            .cloned()
            .collect();
        let remote_request = request.clone();
        // A host asking again for a channel this machine already made under its id:
        // answer with it, adding the repositories it lacks and cloning again the
        // ones that aren't ready (CH-7, CH-19).
        let known = match &assigned {
            Some(id) => {
                let (store, id) = (self.starter.store.clone(), id.clone());
                blocking(move || store.channel(&id))
                    .await?
                    .map_err(|error| status::store(error, &self.machine, "look up the channel"))?
            }
            None => None,
        };
        let channel = match known {
            Some(existing) => {
                let starter = self.starter.clone();
                blocking(move || rejoin_channel(&starter, existing, &repos))
                    .await?
                    .map_err(|error| setup_status(error, &self.machine))?
            }
            None => {
                let new = NewChannel {
                    id: assigned,
                    name,
                    repos,
                    machines,
                    branch_prefix: String::new(),
                    default_agent: self.default_agent.clone(),
                };
                let (store, layout) = (self.starter.store.clone(), self.starter.layout.clone());
                blocking(move || create_channel(&store, &layout, new))
                    .await?
                    .map_err(|error| setup_status(error, &self.machine))?
            }
        };

        let mut steps = vec![("record".to_owned(), "Saving the channel".to_owned())];
        for repo in &channel.repos {
            let shown = format!("{}/{}", repo.owner, repo.name);
            steps.push((format!("clone_main:{shown}"), clone_title(&shown)));
        }
        for machine in &remote_machines {
            steps.extend(channel_steps(machine, &request.repos));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::AddRepo,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(setup) => return Ok(Gated::Done(self.answer(setup).await?)),
        };
        progress.set_step("record", pb::StepState::Done, None);
        // What a channel this machine already had has ready is done already.
        for repo in channel
            .repos
            .iter()
            .filter(|repo| repo.main == MainClone::Ready)
        {
            let shown = format!("{}/{}", repo.owner, repo.name);
            progress.set_step(&format!("clone_main:{shown}"), pb::StepState::Done, None);
        }
        let setup = progress.operation();
        let host = Arc::new(HostOp::new(progress));
        let answer = pb::CreateChannelResponse {
            channel: Some(self.show(channel.clone()).await?),
            setup: Some(setup),
        };
        let (feed, store, id, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            channel.id.clone(),
            identity::actor(caller),
        );
        let _ = blocking(move || {
            tell_channel(&feed, &store, &id, pb::Change::Created, Some(by));
        })
        .await;
        let (starter, record, local, locks) =
            (self.starter.clone(), channel.clone(), host.clone(), self.cloning.clone());
        let cloning = tokio::task::spawn_blocking(move || {
            known::clone_one_at_a_time(&starter, &record, &local, &locks)
        });
        let mut remotes = Vec::new();
        if let Some(work) = &self.remote {
            for machine in remote_machines {
                let (work, id, request, host) = (
                    work.clone(),
                    channel.id.clone(),
                    remote_request.clone(),
                    host.clone(),
                );
                remotes.push(tokio::spawn(async move {
                    work.create_channel_on(&machine, &id, request, host).await
                }));
            }
        }
        // REPOS (RL-1): the repos this call made ready go to the repo library.
        let adds = (self.library.clone(), self.starter.clone(), library_adds::ready_keys(&channel));
        // The operation ends once, when the local clones and every machine's setup
        // have: the first failure fails it; a machine that was removed while its
        // setup waited counts as done (D150).
        let (feed, store, id, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            channel.id.clone(),
            identity::actor(caller),
        );
        tokio::spawn(async move {
            let mut outcome = cloning.await.unwrap_or_else(|error| {
                Err(error_of(
                    &error,
                    "restart humatad2 and create the channel again",
                    None,
                ))
            });
            // REPOS (RL-1): what the local clones made ready is recorded, whatever the other machines do.
            let (library, starter, was_ready) = adds;
            let channel_id = id.clone();
            let _ = blocking(move || library_adds::record_new_ready(&library, &starter, &channel_id, &was_ready)).await;
            for remote in remotes {
                let ended = match remote.await {
                    Ok(MachineOutcome::Failed(error)) => Err(error),
                    Ok(MachineOutcome::Ready | MachineOutcome::Gone) => Ok(()),
                    Err(error) => Err(error_of(
                        &error,
                        "restart humatad2 and create the channel again",
                        None,
                    )),
                };
                if outcome.is_ok() {
                    outcome = ended;
                }
            }
            if let Some(progress) = host.take() {
                match outcome {
                    Ok(()) => progress.succeed(None),
                    Err(error) => progress.fail(error),
                }
            }
            // The clones' outcome is part of how the channel shows.
            let _ = blocking(move || {
                tell_channel(&feed, &store, &id, pb::Change::Updated, Some(by));
            })
            .await;
        });
        Ok(Gated::Done(answer))
    }

    /// The body of GetChannel; `approved` once the user said yes to it.
    async fn read_channel(
        &self,
        caller: &Caller,
        request: pb::GetChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::Channel>, Status> {
        let id = request.id.map(|id| id.value).filter(|id| !id.is_empty());
        let id = id.ok_or_else(|| status::missing("id"))?;
        if let Some(asked) = self.gate(
            caller,
            Capability::ChannelRead,
            Target::Channel(ChannelId::new(id.as_str())),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        let channel = self.find(&id).await?;
        Ok(Gated::Done(self.show(channel).await?))
    }

    /// The body of RenameChannel; `approved` once the user said yes to it.
    async fn rename(
        &self,
        caller: &Caller,
        request: pb::RenameChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::Channel>, Status> {
        let id = request.id.map(|id| id.value).filter(|id| !id.is_empty());
        let id = id.ok_or_else(|| status::missing("id"))?;
        let name = clean_name(&request.name)?;
        if let Some(asked) = self.gate(
            caller,
            Capability::ChannelChange,
            Target::Channel(ChannelId::new(id.as_str())),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        let expected = request.expected_revision.map(|revision| revision.value);
        let found = self.find(&id).await?;
        if let Some(folder) = self.folder_for(&found, &name)? {
            let confirmed = request.confirm_running || approved;
            let channel = self
                .rename_moving(caller, found, name, folder, expected, confirmed)
                .await?;
            self.rename_on_machines(&channel, confirmed).await?;
            return Ok(Gated::Done(self.show(channel).await?));
        }
        let (store, renaming, shown) = (self.starter.store.clone(), id.clone(), name.clone());
        let renamed = blocking(move || store.rename_channel(&renaming, &shown, expected)).await?;
        let channel = renamed.map_err(|error| self.rename_failed(error, &id))?;
        let (feed, store, named, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            channel.id.clone(),
            identity::actor(caller),
        );
        let _ = blocking(move || {
            tell_channel(&feed, &store, &named, pb::Change::Updated, Some(by));
        })
        .await;
        self.rename_on_machines(&channel, request.confirm_running || approved).await?;
        Ok(Gated::Done(self.show(channel).await?))
    }

    /// What a failed rename of channel `id` answers.
    fn rename_failed(&self, error: StoreError, id: &str) -> Status {
        match error {
            StoreError::NoChannel { .. } | StoreError::InvalidId { .. } => no_channel(id),
            StoreError::StaleChannel { expected, actual, .. } => status::failure(
                Code::Aborted,
                pb::Error {
                    message: format!(
                        "the channel is at revision {actual}, not {expected} as the change was made against"
                    ),
                    fix: "read the channel again, check the name still needs changing, and send the change again".to_owned(),
                    retry: None,
                    kind: Some(pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                        kind: pb::EntityKind::Channel as i32,
                        id: id.to_owned(),
                        expected: Some(pb::Revision { value: expected }),
                        actual: Some(pb::Revision { value: actual }),
                    })),
                },
            ),
            error => status::store(error, &self.machine, "save the channel's new name"),
        }
    }

    fn machines(&self, wanted: &[pb::MachineId]) -> Result<Vec<String>, Status> {
        let mut machines: Vec<String> = Vec::new();
        for (n, machine) in wanted.iter().enumerate() {
            if machine.value != self.machine && self.remote.is_none() {
                return Err(status::failure(
                    Code::Unimplemented,
                    pb::Error {
                        message: format!(
                            "machines[{n}] ({}) is not this machine; a channel can only have this machine until remote machines are built (MACH)",
                            machine.value
                        ),
                        fix: "leave machines empty, or name only this machine, and create the channel again".to_owned(),
                        retry: None,
                        kind: Some(pb::error::Kind::InvalidField(pb::InvalidField {
                            field: format!("machines[{n}]"),
                            problem: Problem::NotAllowedHere as i32,
                        })),
                    },
                ));
            }
            // A machine the host never linked would wait for ever (D164).
            if machine.value != self.machine
                && let Some(list) = &self.known
            {
                known::check_known(list, &format!("machines[{n}]"), &machine.value)?;
            }
            if !machines.contains(&machine.value) {
                machines.push(machine.value.clone());
            }
        }
        if machines.is_empty() {
            machines.push(self.machine.clone());
        }
        Ok(machines)
    }
}

/// Runs store and file work off the async threads.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))
}

fn no_channel(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Channel,
        id,
        format!("no channel {id} is on this machine"),
        "list the channels to see their ids",
    )
}

/// The sessions of channel `channel` whose agent should be running.
fn live_sessions(store: &humata2_store::Store, channel: &str) -> Result<u32, StoreError> {
    let in_channel: HashSet<String> = store
        .sessions()?
        .into_iter()
        .filter(|session| session.channel == channel)
        .map(|session| session.id)
        .collect();
    let live = store
        .agents()?
        .into_iter()
        .filter(|agent| in_channel.contains(&agent.session) && agent.state.should_run())
        .count();
    Ok(u32::try_from(live).unwrap_or(u32::MAX))
}

pub(crate) fn timestamp_ms(ms: i64) -> humata2_grpc::Timestamp {
    let since = UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0));
    let since = since.duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

///
/// `agent` is the one a start that names none runs here (D770, ST-15), not
/// the one the channel was made with: the app preselects it in its dialogs.
fn channel_pb(channel: &ChannelRecord, live_sessions: u32, agent: &str) -> pb::Channel {
    pb::Channel {
        id: Some(pb::ChannelId {
            value: channel.id.clone(),
        }),
        name: channel.name.clone(),
        folder: channel.folder.clone(),
        repos: channel
            .repos
            .iter()
            .map(|repo| pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            })
            .collect(),
        machines: channel
            .machines
            .iter()
            .map(|machine| pb::MachineId {
                value: machine.clone(),
            })
            .collect(),
        branch_prefix: channel.branch_prefix.clone(),
        default_agent: Some(pb::AgentId {
            value: agent.to_owned(),
        }),
        live_sessions,
        revision: Some(pb::Revision {
            value: channel.revision,
        }),
        created_at: Some(timestamp_ms(channel.created_unix_ms)),
        is_pr_reviews: channel.folder == crate::review::PR_REVIEWS_FOLDER,
        ..pb::Channel::default()
    }
}

/// The agent a start that names none runs in `channel`, as `agent_for` picks
/// it: the channel's own default, then Settings', then the one it was made
/// with. A store that can't answer leaves the one it was made with.
pub(crate) fn effective_agent(store: &Store, channel: &ChannelRecord) -> String {
    crate::settings_service::default_agent_for(store, &channel.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.default_agent.clone())
}

/// A channel name: not empty, not absurdly long.
fn clean_name(name: &str) -> Result<String, Status> {
    let name = name.trim();
    if name.is_empty() {
        return Err(status::missing("name"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(status::invalid_field(
            "name",
            Problem::TooLong,
            format!("the name is longer than {NAME_MAX} characters"),
            format!("use a shorter name, at most {NAME_MAX} characters"),
        ));
    }
    Ok(name.to_owned())
}

/// Whether `part` can be a GitHub owner or repository name.
fn github_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The repositories as `(owner, name)`, each once, in the order given.
fn clean_repos(repos: &[pb::RepoRef]) -> Result<Vec<(String, String)>, Status> {
    let mut clean: Vec<(String, String)> = Vec::new();
    for (n, repo) in repos.iter().enumerate() {
        for (field, value) in [("owner", &repo.owner), ("name", &repo.name)] {
            if !github_part(value) {
                return Err(status::invalid_field(
                    &format!("repos[{n}].{field}"),
                    Problem::BadFormat,
                    format!("repos[{n}] has {field} {value:?}, which GitHub doesn't allow"),
                    "use the owner and name as GitHub shows them: letters, digits, dashes, underscores and dots",
                ));
            }
        }
        let pair = (repo.owner.clone(), repo.name.clone());
        if clean.contains(&pair) {
            return Err(status::invalid_field(
                &format!("repos[{n}]"),
                Problem::NotAllowedHere,
                format!("{}/{} is listed twice", repo.owner, repo.name),
                "list each repository once",
            ));
        }
        clean.push(pair);
    }
    Ok(clean)
}

fn setup_status(error: SetupError, machine: &str) -> Status {
    if let SetupError::Store { doing, source } = error {
        return status::store(source, machine, doing);
    }
    match &error {
        SetupError::Name(name) => failed(
            Code::InvalidArgument,
            &error,
            match name {
                NameError::NoLettersOrDigits { .. } => "use a name with at least one letter or digit",
                NameError::Reserved { .. } => "use another name: Desk sessions use that folder",
            },
            Some(pb::error::Kind::InvalidField(pb::InvalidField {
                field: "name".to_owned(),
                problem: Problem::BadFormat as i32,
            })),
        ),
        SetupError::FolderTaken { folder, .. } => failed(
            Code::AlreadyExists,
            &error,
            "pick another name for this channel",
            Some(pb::error::Kind::NameTaken(pb::NameTaken {
                kind: pb::EntityKind::Channel as i32,
                name: folder.clone(),
                suggestion: format!("{folder}-2"),
            })),
        ),
        SetupError::NoChannel { id } => no_channel(id),
        SetupError::Layout(_) => failed(
            Code::Unavailable,
            &error,
            "check that the Humata folder is writable and that the disk has space, then create the channel again",
            None,
        ),
        SetupError::Store { .. } | SetupError::Background { .. } => failed(
            Code::Internal,
            &error,
            "restart humatad2 and try again",
            None,
        ),
    }
}

/// An error with its causes, said for the person who fixes it.
pub(crate) fn failed(
    code: Code,
    error: &dyn std::error::Error,
    fix: &str,
    kind: Option<pb::error::Kind>,
) -> Status {
    status::failure(code, error_of(error, fix, kind))
}

pub(crate) fn error_of(
    error: &dyn std::error::Error,
    fix: &str,
    kind: Option<pb::error::Kind>,
) -> pb::Error {
    pb::Error {
        message: status::chain(error),
        fix: fix.to_owned(),
        retry: None,
        kind,
    }
}

fn clone_title(repo: &str) -> String {
    format!("Cloning {repo}")
}

/// What an operation that failed cloning `repo` says.
fn clone_failed(operation: &str, repo: &str, reason: &str) -> pb::Error {
    pb::Error {
        message: format!("couldn't clone {repo}: {reason}"),
        fix: "the channel and its folders were kept, with that repository marked as failed; check the repository's name and that git can reach it on this machine, then create the channel again under another name".to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::StepFailed(Box::new(pb::StepFailed {
            operation: Some(pb::OperationId {
                value: operation.to_owned(),
            }),
            step: format!("clone_main:{repo}"),
            cause: Some(Box::new(pb::Error {
                message: reason.to_owned(),
                fix: String::new(),
                retry: None,
                kind: None,
            })),
            left_as_is: "the channel, its folders and the clones that worked".to_owned(),
        }))),
    }
}

/// Clones the channel's mains, telling `progress` about each. Blocking. What
/// became of them is the answer; the caller ends the operation, once every
/// machine's part is in.
fn clone_in_background(
    starter: &Starter,
    channel: &ChannelRecord,
    progress: &HostOp,
) -> Result<(), pb::Error> {
    let operation = progress
        .operation()
        .and_then(|operation| operation.id)
        .map(|id| id.value)
        .unwrap_or_default();
    let keys: Vec<String> = channel
        .repos
        .iter()
        .map(|r| format!("{}/{}", r.owner, r.name))
        .collect();
    if let Some(first) = keys.first() {
        progress.set_step(&format!("clone_main:{first}"), pb::StepState::Running, None);
    }
    let mut on_step = |shown: &str, result: Result<(), &GitError>| {
        match result {
            Ok(()) => progress.set_step(&format!("clone_main:{shown}"), pb::StepState::Done, None),
            Err(error) => progress.set_step(
                &format!("clone_main:{shown}"),
                pb::StepState::Failed,
                Some(status::chain(error)),
            ),
        }
        if let Some(next) = keys
            .iter()
            .position(|key| key == shown)
            .and_then(|n| keys.get(n + 1))
        {
            progress.set_step(&format!("clone_main:{next}"), pb::StepState::Running, None);
        }
    };
    let units = progress.with_progress(|p| p.units());
    let on_units = |shown: &str, done: u64, total: u64| {
        if let Some(units) = &units {
            units.set(&format!("clone_main:{shown}"), done, total);
        }
    };
    let done = clone_mains_with(
        &starter.store,
        &starter.layout,
        &starter.remote_template,
        &channel.id,
        &mut on_step,
        on_units,
    );
    match done {
        Err(error) => Err(error_of(
            &error,
            "check that the Humata state folder is writable and the disk has space, then create the channel again",
            None,
        )),
        Ok(()) => {
            let failed = starter.store.channel(&channel.id).ok().flatten().and_then(|channel| {
                channel.repos.into_iter().find_map(|repo| match repo.main {
                    MainClone::Failed { reason } => Some((format!("{}/{}", repo.owner, repo.name), reason)),
                    _ => None,
                })
            });
            match failed {
                Some((repo, reason)) => Err(clone_failed(&operation, &repo, &reason)),
                None => Ok(()),
            }
        }
    }
}

/// The channel `channel` as a host asks for it again (D150): the repositories
/// the request names that it lacks are added, and each one whose main clone
/// isn't ready is set to be cloned again. Blocking.
fn rejoin_channel(
    starter: &Starter,
    mut channel: ChannelRecord,
    repos: &[(String, String)],
) -> Result<ChannelRecord, SetupError> {
    let mut changed = false;
    for (owner, name) in repos {
        match channel
            .repos
            .iter_mut()
            .find(|repo| &repo.owner == owner && &repo.name == name)
        {
            Some(repo) if repo.main != MainClone::Ready && repo.main != MainClone::Cloning => {
                repo.main = MainClone::Cloning;
                changed = true;
            }
            Some(_) => {}
            None => {
                channel.repos.push(ChannelRepo {
                    owner: owner.clone(),
                    name: name.clone(),
                    main: MainClone::Cloning,
                });
                changed = true;
            }
        }
    }
    if changed {
        channel.revision += 1;
        starter
            .store
            .put_channel(&channel)
            .map_err(|source| SetupError::Store {
                doing: "save the channel",
                source,
            })?;
    }
    starter.layout.create_channel(&channel.folder)?;
    Ok(channel)
}

#[tonic::async_trait]
impl pb::channel_service_server::ChannelService for ChannelSvc {
    /// Saves the channel and makes its folders, then answers with it and
    /// the operation cloning its repositories (CH-5): the clones run on.
    async fn create_channel(
        &self,
        request: Request<pb::CreateChannelRequest>,
    ) -> Result<Response<pb::CreateChannelResponse>, Status> {
        let caller = identity::caller(&request)?;
        let assigned = identity::host_assigned_id(&request, &caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .make_channel(&caller, request.clone(), assigned.clone(), false)
            .await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.CreateChannelResponse",
            first,
            move || async move { me.make_channel(&who, request, assigned, true).await },
        )
        .await
    }

    async fn get_channel(
        &self,
        request: Request<pb::GetChannelRequest>,
    ) -> Result<Response<pb::Channel>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let first = self.read_channel(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            None,
            "humata.v1.Channel",
            first,
            move || async move { me.read_channel(&who, request, true).await },
        )
        .await
    }

    /// Every channel the caller may read: all for the user, an agent's own
    /// (or those it was granted).
    async fn list_channels(
        &self,
        request: Request<pb::ListChannelsRequest>,
    ) -> Result<Response<pb::ListChannelsResponse>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        // Listing reaches every channel, which is the global-read rule's
        // to judge; when it says Ask, the call waits in the Inbox.
        let call = Call::Use {
            capability: Capability::GlobalRead,
            target: Target::Global,
        };
        match self.guard.check(&caller, &call)? {
            Checked::Allowed => {
                let store = self.starter.store.clone();
                Ok(Response::new(
                    readable_channels(&self.guard, &store, &self.machine, &caller).await?,
                ))
            }
            Checked::Ask(asked) => {
                let (guard, store) = (self.guard.clone(), self.starter.store.clone());
                let (machine, who) = (self.machine.clone(), caller.clone());
                let run: crate::inbox::Run = Box::new(move |_| {
                    Box::pin(async move {
                        let listed = readable_channels(&guard, &store, &machine, &who).await?;
                        Ok(Deferred {
                            result: listed.encode_to_vec(),
                            result_type: "humata.v1.ListChannelsResponse".to_owned(),
                        })
                    })
                });
                // The request has no idempotency key field.
                Err(self.inbox.defer(&caller, asked, None, run).await)
            }
        }
    }

    /// Changes the name shown and moves the channel's folder to the new
    /// name's (MV-16, D314): its sessions stop, and resume in their new
    /// folders, after a rename the person confirmed if one is in a turn.
    async fn rename_channel(
        &self,
        request: Request<pb::RenameChannelRequest>,
    ) -> Result<Response<pb::Channel>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.rename(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Channel",
            first,
            move || async move { me.rename(&who, request, true).await },
        )
        .await
    }

    /// Every repo the host's `gh` can see that matches the query, one page
    /// at a time (CH-3). Listing reaches the repos of every account the host
    /// is signed in to, which is the global-read rule's to judge.
    async fn search_git_hub_repos(
        &self,
        request: Request<pb::SearchGitHubReposRequest>,
    ) -> Result<Response<pb::SearchGitHubReposResponse>, Status> {
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let request = request.into_inner();
        let call = Call::Use {
            capability: Capability::GlobalRead,
            target: Target::Global,
        };
        match self.guard.check(&caller, &call)? {
            Checked::Allowed => Ok(Response::new(gh_repos::search(&request).await?)),
            Checked::Ask(asked) => {
                let run: crate::inbox::Run = Box::new(move |_| {
                    Box::pin(async move {
                        let found = gh_repos::search(&request).await?;
                        Ok(Deferred {
                            result: found.encode_to_vec(),
                            result_type: "humata.v1.SearchGitHubReposResponse".to_owned(),
                        })
                    })
                });
                // The request has no idempotency key field.
                Err(self.inbox.defer(&caller, asked, None, run).await)
            }
        }
    }

    /// Adds a repository to the channel and clones it on every machine of
    /// the channel (CH-6, CH-18, CH-19, D163).
    async fn add_repo(
        &self,
        request: Request<pb::AddRepoRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.add_repo_to_channel(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.add_repo_to_channel(&who, request, true).await },
        )
        .await
    }

    async fn plan_remove_repo(
        &self,
        _request: Request<pb::PlanRemoveRepoRequest>,
    ) -> Result<Response<pb::RemoveRepoPlan>, Status> {
        Err(status::not_built("humata.v1.ChannelService.PlanRemoveRepo"))
    }

    async fn remove_repo(
        &self,
        _request: Request<pb::RemoveRepoRequest>,
    ) -> Result<Response<pb::Channel>, Status> {
        Err(status::not_built("humata.v1.ChannelService.RemoveRepo"))
    }

    /// Adds a machine to the channel and makes its folders and main clones
    /// there (CH-7, D163).
    async fn add_machine(
        &self,
        request: Request<pb::AddChannelMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.add_machine_to_channel(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.add_machine_to_channel(&who, request, true).await },
        )
        .await
    }

    /// What leaving the channel would do to a machine's sessions (CH-8):
    /// their unpushed work and a token for RemoveMachine. Read-only.
    async fn plan_remove_machine(
        &self,
        request: Request<pb::PlanRemoveChannelMachineRequest>,
    ) -> Result<Response<pb::RemoveChannelMachinePlan>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let first = self.plan_remove_machine_of(&caller, request.clone(), false).await;
        answer_plan(first, || self.plan_remove_machine_of(&caller, request, true)).await
    }

    /// Takes a machine out of a channel once each of its sessions was moved or archived (CH-8, LD-10).
    async fn remove_machine(
        &self,
        request: Request<pb::RemoveChannelMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.remove_machine_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.remove_machine_of(&who, request, true).await },
        )
        .await
    }

    async fn get_grid(
        &self,
        request: Request<pb::GetGridRequest>,
    ) -> Result<Response<pb::ChannelGrid>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let first = self.grid_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            None,
            "humata.v1.ChannelGrid",
            first,
            move || async move { me.grid_of(&who, request, true).await },
        )
        .await
    }

    async fn plan_delete_channel(
        &self,
        request: Request<pb::PlanDeleteChannelRequest>,
    ) -> Result<Response<pb::DeleteChannelPlan>, Status> {
        let caller = identity::caller(&request)?;
        self.plan_delete_channel_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn delete_channel(
        &self,
        request: Request<pb::DeleteChannelRequest>,
    ) -> Result<Response<pb::DeleteChannelResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.delete_channel_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> pb::RepoRef {
        pb::RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn repos_keep_their_order_and_reject_what_github_cannot_name() {
        let ok = clean_repos(&[repo("acme", "api"), repo("acme", "web.app")]).unwrap();
        assert_eq!(
            ok,
            [
                ("acme".to_owned(), "api".to_owned()),
                ("acme".to_owned(), "web.app".to_owned())
            ]
        );
        assert!(clean_repos(&[repo("acme", "api"), repo("acme", "api")]).is_err());
        assert!(clean_repos(&[repo("acme", "../x")]).is_err());
        assert!(clean_repos(&[repo("", "api")]).is_err());
        assert!(clean_repos(&[repo("a/b", "api")]).is_err());
    }

    #[test]
    fn a_name_is_trimmed_and_cannot_be_empty_or_huge() {
        assert_eq!(clean_name("  Payments ").unwrap(), "Payments");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"x".repeat(NAME_MAX + 1)).is_err());
    }

    #[test]
    fn an_unset_prefix_shows_as_unset() {
        let record = ChannelRecord {
            id: "ch_1".to_owned(),
            name: "Payments".to_owned(),
            folder: "payments".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: Vec::new(),
            machines: vec!["m1".to_owned()],
            revision: 3,
            created_unix_ms: 1_000,
        };
        let shown = channel_pb(&record, 2, "claude");
        assert_eq!(shown.branch_prefix, None);
        assert_eq!(shown.live_sessions, 2);
        assert_eq!(shown.revision.map(|r| r.value), Some(3));
        let with = channel_pb(
            &ChannelRecord {
                branch_prefix: Some("me/".to_owned()),
                ..record
            },
            0,
            "claude",
        );
        assert_eq!(with.branch_prefix.as_deref(), Some("me/"));
    }
}
