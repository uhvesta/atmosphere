//! AddRepo and AddChannelMachine (CH-6, CH-7, CH-18, CH-19, D157): both take
//! the path CreateChannel takes. The channel's record gets the repository or
//! the machine, the folders and main clones are made here, and every other
//! machine of the channel is asked to make the same (the host's
//! `create_channel_on` answers a known id by adding what it lacks). The
//! answer is one `Operation` with the same steps as a channel's setup. A
//! repository or machine the channel already has is not refused: the call
//! runs as a retry, so a failed or missing clone is cloned again.

use std::sync::Arc;

use humata2_access::{Capability, ChannelId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, MainClone};
use pb::invalid_field::Problem;
use tonic::Status;

use super::known::clone_one_at_a_time;
use super::{
    ChannelSvc, Gated, blocking, clone_title, error_of, github_part, rejoin_channel, setup_status,
    tell_channel,
};
use crate::identity::{self, Caller};
use crate::operations::Started;
use crate::remote_work::{HostOp, MachineOutcome, channel_steps};
use crate::status;

/// The step an AddRepo ends with when an agent's repo was in the channel
/// already: it names the outcome, so a reader doesn't match on a title.
const ALREADY_IN_CHANNEL: (&str, &str) = ("already_in_channel", "Already in the channel");
const ALREADY_ADDED: (&str, &str) = ("already_added", "Already added");

/// True when `part` names a place on this machine's disk rather than a
/// GitHub owner or repository name.
fn looks_like_path(part: &str) -> bool {
    part.contains(['/', '\\']) || part.starts_with('~')
}

/// The channel id a request names.
fn channel_id(channel: Option<&pb::ChannelId>) -> Result<String, Status> {
    channel
        .map(|id| id.value.clone())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("channel"))
}

impl ChannelSvc {
    /// The body of AddRepo; `approved` once the user said yes to it.
    pub(crate) async fn add_repo_to_channel(
        &self,
        caller: &Caller,
        request: pb::AddRepoRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = channel_id(request.channel.as_ref())?;
        let repo = request.repo.ok_or_else(|| status::missing("repo"))?;
        for (field, value) in [("owner", &repo.owner), ("name", &repo.name)] {
            if matches!(caller, Caller::Agent(_)) && looks_like_path(value) {
                return Err(status::invalid_field(
                    &format!("repo.{field}"),
                    Problem::BadFormat,
                    format!("repo has {field} {value:?}, which looks like a path on this machine, and an agent can't propose one"),
                    "propose a GitHub repository by its owner and name (for example acme/api); to use a folder on this machine, ask the user to add it from Humata",
                ));
            }
            if !github_part(value) {
                return Err(status::invalid_field(
                    &format!("repo.{field}"),
                    Problem::BadFormat,
                    format!("repo has {field} {value:?}, which GitHub doesn't allow"),
                    "use the owner and name as GitHub shows them: letters, digits, dashes, underscores and dots",
                ));
            }
        }
        // The user's own call is never gated; an agent's is checked against
        // the channel's "Add a repo to a channel" rule (CH-26, D304). A repo
        // the channel already has, once ready, is not cloned again below, so
        // an approval that comes after the user added it succeeds as it was.
        let asked = self.gate_repo_add(caller, &id, approved)?;
        // An agent's proposal for a repo the channel has needs no card: the
        // call is answered at once, or as "Already added" when the card was
        // answered after the repo came in some other way (CH-26).
        if matches!(caller, Caller::Agent(_)) {
            let record = self.find(&id).await?;
            if record
                .repos
                .iter()
                .any(|have| have.owner == repo.owner && have.name == repo.name)
            {
                let outcome = if approved {
                    ALREADY_ADDED
                } else {
                    ALREADY_IN_CHANNEL
                };
                return self
                    .answer_repo_present(caller, &record.id, outcome, request.idempotency_key)
                    .map(Gated::Done);
            }
        }
        if let Some(asked) = asked {
            return Ok(Gated::Ask(asked));
        }
        if let Some(op) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(op));
        }
        let channel = self.find(&id).await?;
        let mut wanted: Vec<(String, String)> = channel
            .repos
            .iter()
            .map(|repo| (repo.owner.clone(), repo.name.clone()))
            .collect();
        let new = (repo.owner, repo.name);
        if !wanted.contains(&new) {
            wanted.push(new);
        }
        let remote_machines = if self.remote.is_some() {
            channel
                .machines
                .iter()
                .filter(|machine| **machine != self.machine)
                // A machine removed from Humata stays in the record until
                // the channel drops it; it has nothing to clone (D164).
                .filter(|machine| self.known.as_ref().is_none_or(|list| super::known::is_known(list, machine)))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        self.run_setup(
            caller,
            pb::OperationKind::AddRepo,
            channel,
            wanted,
            None,
            remote_machines,
            request.idempotency_key,
        )
        .await
    }

    /// The body of AddMachine; `approved` once the user said yes to it.
    pub(super) async fn add_machine_to_channel(
        &self,
        caller: &Caller,
        request: pb::AddChannelMachineRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = channel_id(request.channel.as_ref())?;
        let machine = request
            .machine
            .filter(|machine| !machine.value.is_empty())
            .ok_or_else(|| status::missing("machine"))?;
        if let Some(asked) = self.gate_change(caller, &id, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(op) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(op));
        }
        if machine.value != self.machine
            && let Some(list) = self.known.clone()
        {
            let named = machine.value.clone();
            blocking(move || super::known::check_known(&list, "machine", &named)).await??;
        }
        let machines = self.machines(std::slice::from_ref(&machine))?;
        let channel = self.find(&id).await?;
        let wanted = channel
            .repos
            .iter()
            .map(|repo| (repo.owner.clone(), repo.name.clone()))
            .collect();
        let named = machines.into_iter().next().unwrap_or(machine.value);
        let remote_machines = if self.remote.is_some() && named != self.machine {
            vec![named.clone()]
        } else {
            Vec::new()
        };
        self.run_setup(
            caller,
            pb::OperationKind::AddMachine,
            channel,
            wanted,
            Some(named),
            remote_machines,
            request.idempotency_key,
        )
        .await
    }

    /// An AddRepo that succeeded with nothing to do: one step, `outcome`.
    fn answer_repo_present(
        &self,
        caller: &Caller,
        channel: &str,
        outcome: (&str, &str),
        key: Option<pb::IdempotencyKey>,
    ) -> Result<pb::Operation, Status> {
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.to_owned(),
            })),
        };
        let steps = [(outcome.0.to_owned(), outcome.1.to_owned())];
        match self.operations.start_steps(
            caller,
            pb::OperationKind::AddRepo,
            target,
            &steps,
            false,
            key,
        )? {
            Started::Existing(operation) => Ok(operation),
            Started::New(progress) => {
                progress.set_step(outcome.0, pb::StepState::Done, None);
                let shown = progress.watch();
                progress.succeed(None);
                let operation = shown.borrow().clone();
                Ok(operation)
            }
        }
    }

    fn gate_repo_add(
        &self,
        caller: &Caller,
        id: &str,
        approved: bool,
    ) -> Result<Option<Box<humata2_access::AccessRequest>>, Status> {
        self.gate(
            caller,
            Capability::ChannelRepoAdd,
            Target::Channel(ChannelId::new(id)),
            approved,
        )
    }

    fn gate_change(
        &self,
        caller: &Caller,
        id: &str,
        approved: bool,
    ) -> Result<Option<Box<humata2_access::AccessRequest>>, Status> {
        self.gate(
            caller,
            Capability::ChannelChange,
            Target::Channel(ChannelId::new(id)),
            approved,
        )
    }

    /// What both calls share: puts `add_machine` in the record, makes
    /// `wanted` the channel's repositories (new ones cloning, unready ones
    /// cloned again), starts the operation, and runs the local clones and
    /// every machine of `remote_machines` in the background.
    #[allow(clippy::too_many_arguments)]
    async fn run_setup(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        channel: ChannelRecord,
        wanted: Vec<(String, String)>,
        add_machine: Option<String>,
        remote_machines: Vec<String>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Gated<pb::Operation>, Status> {
        let starter = self.starter.clone();
        let channel = blocking(move || {
            let mut channel = channel;
            let before = channel.revision;
            let mut added = false;
            if let Some(machine) = add_machine
                && !channel.machines.contains(&machine)
            {
                channel.machines.push(machine);
                added = true;
            }
            let mut channel = rejoin_channel(&starter, channel, &wanted)?;
            // The machine is saved once: with the repositories if they changed
            // the record, on its own if not.
            if added && channel.revision == before {
                channel.revision += 1;
                starter
                    .store
                    .put_channel(&channel)
                    .map_err(|source| humata2_sessions::SetupError::Store {
                        doing: "save the channel",
                        source,
                    })?;
            }
            Ok(channel)
        })
        .await?
        .map_err(|error| setup_status(error, &self.machine))?;

        let repo_refs: Vec<pb::RepoRef> = channel
            .repos
            .iter()
            .map(|repo| pb::RepoRef {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
            })
            .collect();
        let mut steps = vec![("record".to_owned(), "Saving the channel".to_owned())];
        for repo in &channel.repos {
            let shown = format!("{}/{}", repo.owner, repo.name);
            steps.push((format!("clone_main:{shown}"), clone_title(&shown)));
        }
        for machine in &remote_machines {
            steps.extend(channel_steps(machine, &repo_refs));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self
            .operations
            .start_steps(caller, kind, target, &steps, false, key)?
        {
            Started::New(progress) => progress,
            Started::Existing(op) => return Ok(Gated::Done(op)),
        };
        progress.set_step("record", pb::StepState::Done, None);
        for repo in channel
            .repos
            .iter()
            .filter(|repo| repo.main == MainClone::Ready)
        {
            let shown = format!("{}/{}", repo.owner, repo.name);
            progress.set_step(&format!("clone_main:{shown}"), pb::StepState::Done, None);
        }
        let answer = progress.operation();
        let host = Arc::new(HostOp::new(progress));

        let (feed, store, id, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            channel.id.clone(),
            identity::actor(caller),
        );
        let _ = blocking(move || {
            tell_channel(&feed, &store, &id, pb::Change::Updated, Some(by));
        })
        .await;

        let fix = "restart humatad2 and add it to the channel again";
        // REPOS (RL-1): the repos this call made ready go to the repo library.
        let adds = (self.library.clone(), self.starter.clone(), super::library_adds::ready_keys(&channel));
        let (starter, record, local, locks) = (
            self.starter.clone(),
            channel.clone(),
            host.clone(),
            self.cloning.clone(),
        );
        let cloning = tokio::task::spawn_blocking(move || {
            clone_one_at_a_time(&starter, &record, &local, &locks)
        });
        let mut remotes = Vec::new();
        if let Some(work) = &self.remote {
            let remote_request = pb::CreateChannelRequest {
                idempotency_key: None,
                name: channel.name.clone(),
                repos: repo_refs,
                machines: Vec::new(),
            };
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
        // The operation ends once, when the local clones and every machine's
        // part have: the first failure fails it; a machine removed while its
        // setup waited counts as done.
        let (feed, store, id, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            channel.id.clone(),
            identity::actor(caller),
        );
        tokio::spawn(async move {
            let mut outcome = cloning
                .await
                .unwrap_or_else(|error| Err(error_of(&error, fix, None)));
            // REPOS (RL-1): what the local clones made ready is recorded, whatever the other machines do.
            let (library, starter, was_ready) = adds;
            let channel_id = id.clone();
            let _ = blocking(move || super::library_adds::record_new_ready(&library, &starter, &channel_id, &was_ready)).await;
            for remote in remotes {
                let ended = match remote.await {
                    Ok(MachineOutcome::Failed(error)) => Err(error),
                    Ok(MachineOutcome::Ready | MachineOutcome::Gone) => Ok(()),
                    Err(error) => Err(error_of(&error, fix, None)),
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
            let _ = blocking(move || {
                tell_channel(&feed, &store, &id, pb::Change::Updated, Some(by));
            })
            .await;
        });
        Ok(Gated::Done(answer))
    }
}
