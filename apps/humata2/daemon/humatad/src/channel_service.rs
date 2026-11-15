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
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use humata2_access::{Call, Capability, ChannelId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{GitError, NewChannel, SetupError, Starter, clone_mains, create_channel};
use humata2_store::{ChannelRecord, MainClone, Store, StoreError};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Response, Status};

use crate::guard::{Checked, Guard};
use crate::identity::{self, Caller};
use crate::inbox::{Deferred, Inbox};
use crate::operations::{Operations, Progress, Started};
use crate::status;

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
        let (counting, id) = (store.clone(), channel.id.clone());
        let live = blocking(move || live_sessions(&counting, &id))
            .await?
            .map_err(|error| status::store(error, machine, "count the channel's sessions"))?;
        channels.push(channel_pb(&channel, live));
    }
    Ok(pb::ListChannelsResponse {
        channels,
        page: None,
    })
}

/// The longest channel name taken.
const NAME_MAX: usize = 100;

pub(crate) struct ChannelSvc {
    guard: Arc<Guard>,
    operations: Arc<Operations>,
    starter: Starter,
    /// This machine's id: the only machine a channel can have for now.
    machine: String,
    default_agent: String,
    /// Where a call a rule sends to Ask waits for the user (D38.6).
    inbox: Arc<Inbox>,
}

impl ChannelSvc {
    pub(crate) fn new(
        guard: Arc<Guard>,
        operations: Arc<Operations>,
        starter: Starter,
        machine: String,
        default_agent: String,
        inbox: Arc<Inbox>,
    ) -> Self {
        Self {
            guard,
            operations,
            starter,
            machine,
            default_agent,
            inbox,
        }
    }

    fn require(
        &self,
        caller: &Caller,
        capability: Capability,
        target: Target,
    ) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        self.guard
            .require(caller, &Call::Use { capability, target })
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
        let id = channel.id.clone();
        let live = blocking(move || live_sessions(&store, &id))
            .await?
            .map_err(|error| status::store(error, &self.machine, "count the channel's sessions"))?;
        Ok(channel_pb(&channel, live))
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

    fn machines(&self, wanted: &[pb::MachineId]) -> Result<Vec<String>, Status> {
        let mut machines: Vec<String> = Vec::new();
        for (n, machine) in wanted.iter().enumerate() {
            if machine.value != self.machine {
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

fn channel_pb(channel: &ChannelRecord, live_sessions: u32) -> pb::Channel {
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
        branch_prefix: Some(channel.branch_prefix.clone()).filter(|prefix| !prefix.is_empty()),
        default_agent: Some(pb::AgentId {
            value: channel.default_agent.clone(),
        }),
        live_sessions,
        revision: Some(pb::Revision {
            value: channel.revision,
        }),
        created_at: Some(timestamp_ms(channel.created_unix_ms)),
        ..pb::Channel::default()
    }
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
        SetupError::Name(_) => failed(
            Code::InvalidArgument,
            &error,
            "use a name with at least one letter or digit",
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

/// Clones the channel's mains, telling `progress` about each, and ends the
/// operation. Blocking.
fn clone_in_background(starter: Starter, channel: ChannelRecord, progress: Progress) {
    let operation = progress
        .operation()
        .id
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
    let done = clone_mains(
        &starter.store,
        &starter.layout,
        &starter.remote_template,
        &channel.id,
        &mut on_step,
    );
    match done {
        Err(error) => progress.fail(error_of(
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
                Some((repo, reason)) => progress.fail(clone_failed(&operation, &repo, &reason)),
                None => progress.succeed(None),
            }
        }
    }
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
        let request = request.into_inner();
        self.require(&caller, Capability::ChannelCreate, Target::Global)?;
        if let Some(setup) = self
            .operations
            .existing(&caller, request.idempotency_key.as_ref())?
        {
            return Ok(Response::new(self.answer(setup).await?));
        }
        let new = NewChannel {
            name: clean_name(&request.name)?,
            repos: clean_repos(&request.repos)?,
            machines: self.machines(&request.machines)?,
            branch_prefix: String::new(),
            default_agent: self.default_agent.clone(),
        };
        let (store, layout) = (self.starter.store.clone(), self.starter.layout.clone());
        let channel = blocking(move || create_channel(&store, &layout, new))
            .await?
            .map_err(|error| setup_status(error, &self.machine))?;

        let mut steps = vec![("record".to_owned(), "Saving the channel".to_owned())];
        for repo in &channel.repos {
            let shown = format!("{}/{}", repo.owner, repo.name);
            steps.push((format!("clone_main:{shown}"), clone_title(&shown)));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            &caller,
            pb::OperationKind::AddRepo,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(setup) => return Ok(Response::new(self.answer(setup).await?)),
        };
        progress.set_step("record", pb::StepState::Done, None);
        let setup = progress.operation();
        let (starter, record) = (self.starter.clone(), channel.clone());
        tokio::task::spawn_blocking(move || clone_in_background(starter, record, progress));
        Ok(Response::new(pb::CreateChannelResponse {
            channel: Some(self.show(channel).await?),
            setup: Some(setup),
        }))
    }

    async fn get_channel(
        &self,
        request: Request<pb::GetChannelRequest>,
    ) -> Result<Response<pb::Channel>, Status> {
        let caller = identity::caller(&request)?;
        let id = request
            .into_inner()
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty());
        let id = id.ok_or_else(|| status::missing("id"))?;
        self.require(
            &caller,
            Capability::ChannelRead,
            Target::Channel(ChannelId::new(id.as_str())),
        )?;
        let channel = self.find(&id).await?;
        Ok(Response::new(self.show(channel).await?))
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
                let run: crate::inbox::Run = Box::new(move || {
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

    /// Changes the name shown; the folder keeps its name (FC-1).
    async fn rename_channel(
        &self,
        request: Request<pb::RenameChannelRequest>,
    ) -> Result<Response<pb::Channel>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = request.id.map(|id| id.value).filter(|id| !id.is_empty());
        let id = id.ok_or_else(|| status::missing("id"))?;
        let name = clean_name(&request.name)?;
        self.require(
            &caller,
            Capability::ChannelChange,
            Target::Channel(ChannelId::new(id.as_str())),
        )?;
        let expected = request.expected_revision.map(|revision| revision.value);
        let (store, renaming, shown) = (self.starter.store.clone(), id.clone(), name.clone());
        let renamed = blocking(move || store.rename_channel(&renaming, &shown, expected)).await?;
        let channel = match renamed {
            Ok(channel) => channel,
            Err(StoreError::NoChannel { .. } | StoreError::InvalidId { .. }) => {
                return Err(no_channel(&id));
            }
            Err(StoreError::StaleChannel {
                expected, actual, ..
            }) => {
                return Err(status::failure(
                    Code::Aborted,
                    pb::Error {
                        message: format!(
                            "the channel is at revision {actual}, not {expected} as the change was made against"
                        ),
                        fix: "read the channel again, check the name still needs changing, and send the change again".to_owned(),
                        retry: None,
                        kind: Some(pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                            kind: pb::EntityKind::Channel as i32,
                            id,
                            expected: Some(pb::Revision { value: expected }),
                            actual: Some(pb::Revision { value: actual }),
                        })),
                    },
                ));
            }
            Err(error) => {
                return Err(status::store(
                    error,
                    &self.machine,
                    "save the channel's new name",
                ));
            }
        };
        Ok(Response::new(self.show(channel).await?))
    }

    async fn search_git_hub_repos(
        &self,
        _request: Request<pb::SearchGitHubReposRequest>,
    ) -> Result<Response<pb::SearchGitHubReposResponse>, Status> {
        Err(status::not_built(
            "humata.v1.ChannelService.SearchGitHubRepos",
        ))
    }

    async fn add_repo(
        &self,
        _request: Request<pb::AddRepoRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.ChannelService.AddRepo"))
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

    async fn add_machine(
        &self,
        _request: Request<pb::AddChannelMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.ChannelService.AddMachine"))
    }

    async fn plan_remove_machine(
        &self,
        _request: Request<pb::PlanRemoveChannelMachineRequest>,
    ) -> Result<Response<pb::RemoveChannelMachinePlan>, Status> {
        Err(status::not_built(
            "humata.v1.ChannelService.PlanRemoveMachine",
        ))
    }

    async fn remove_machine(
        &self,
        _request: Request<pb::RemoveChannelMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.ChannelService.RemoveMachine"))
    }

    async fn get_grid(
        &self,
        _request: Request<pb::GetGridRequest>,
    ) -> Result<Response<pb::ChannelGrid>, Status> {
        Err(status::not_built("humata.v1.ChannelService.GetGrid"))
    }

    async fn plan_delete_channel(
        &self,
        _request: Request<pb::PlanDeleteChannelRequest>,
    ) -> Result<Response<pb::DeleteChannelPlan>, Status> {
        Err(status::not_built(
            "humata.v1.ChannelService.PlanDeleteChannel",
        ))
    }

    async fn delete_channel(
        &self,
        _request: Request<pb::DeleteChannelRequest>,
    ) -> Result<Response<pb::DeleteChannelResponse>, Status> {
        Err(status::not_built("humata.v1.ChannelService.DeleteChannel"))
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
            branch_prefix: String::new(),
            default_agent: "claude".to_owned(),
            repos: Vec::new(),
            machines: vec!["m1".to_owned()],
            revision: 3,
            created_unix_ms: 1_000,
        };
        let shown = channel_pb(&record, 2);
        assert_eq!(shown.branch_prefix, None);
        assert_eq!(shown.live_sessions, 2);
        assert_eq!(shown.revision.map(|r| r.value), Some(3));
        let with = channel_pb(
            &ChannelRecord {
                branch_prefix: "me/".to_owned(),
                ..record
            },
            0,
        );
        assert_eq!(with.branch_prefix.as_deref(), Some("me/"));
    }
}
