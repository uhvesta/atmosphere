//! AddRepoToSession (CH-21, FC-11): a repository cloned into a running
//! session. The user may add any repo; one the channel lacks is added to the
//! channel first, through the channel's own add. A session's agent may add
//! only a repo its channel already has (a repo for the channel is proposed
//! with `channel_add_repo`, CH-26). The clone is the session's own, on a
//! branch named as FC-11; the repo library hears of the use (RL-1). The
//! agent is told nothing (D743): the person prompts it, and it can ask
//! Humata's tools for its session's repos.
//!
//! The answer is one `Operation` (kind AddRepo, target the session) with the
//! steps `add_to_channel` (only when the channel needs the repo) and
//! `clone:<name>`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use humata2_access::{Capability, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_repo_library::Place;
use humata2_sessions::StartError;
use humata2_store::{MainClone, StartStep, StoreError};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::fork::not_built_part;
use super::SessionSvc;
use crate::channel_service::{ChannelSvc, Gated, blocking, error_of, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

const ADD_REPO: &str = "humata.v1.SessionService.AddRepoToSession";
/// How long the channel's own clone of a repo is waited for.
const MAIN_CLONE_WAIT: Duration = Duration::from_secs(15 * 60);
const MAIN_CLONE_POLL: Duration = Duration::from_millis(150);

/// The shared handle to the channel service, set once the services are made.
pub(super) type Channels = Arc<OnceLock<ChannelSvc>>;

#[derive(Debug, thiserror::Error)]
enum Refused {
    #[error("{repo} isn't in channel {channel}, and a session's agent can't add a repo to its channel by adding it to its session")]
    NotInChannel { repo: String, channel: String },
    #[error("session {session} hasn't finished starting, so it takes no repo yet")]
    Starting { session: String },
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error("couldn't add {repo} to the channel")]
    Channel {
        repo: String,
        #[source]
        source: Status,
    },
    #[error("the channel has no way to add repos yet, so {repo} was not added to it")]
    NoChannels { repo: String },
    #[error("the channel's clone of {repo} failed: {reason}")]
    MainFailed { repo: String, reason: String },
    #[error("the channel's clone of {repo} didn't finish in time")]
    MainTimeout { repo: String },
    #[error("a background task stopped before it finished")]
    Background(#[source] Status),
    #[error("couldn't read the channel")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error(transparent)]
    Start(#[from] StartError),
}

/// Whether a part of `owner/name` can be a GitHub owner or repository name.
fn github_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn fix_of(error: &Failure) -> &'static str {
    match error {
        Failure::Channel { .. } | Failure::NoChannels { .. } | Failure::MainFailed { .. } => {
            "check the repo's owner and name and that git can reach it on this machine, then add it to the session again"
        }
        Failure::MainTimeout { .. } => "wait for the channel's clone to finish, then add the repo to the session again",
        Failure::Store { .. } | Failure::Start(_) | Failure::Background(_) => {
            "fix the cause the message names, then add the repo to the session again; the session is as it was"
        }
    }
}

impl SessionSvc {
    /// Gives AddRepoToSession the channel service. Once.
    pub(crate) fn use_channels(&self, channels: ChannelSvc) {
        let _ = self.channels.set(channels);
    }

    /// The body of AddRepoToSession; `approved` once the user said yes to it.
    pub(super) async fn add_repo_to_session_of(
        &self,
        caller: &Caller,
        request: pb::AddRepoToSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = super::archive::session_id(request.id.clone())?;
        let repo = request.repo.clone().ok_or_else(|| status::missing("repo"))?;
        for (field, value) in [("owner", &repo.owner), ("name", &repo.name)] {
            if !github_part(value) {
                return Err(status::invalid_field(
                    &format!("repo.{field}"),
                    Problem::BadFormat,
                    format!("repo has {field} {value:?}, which GitHub doesn't allow"),
                    "use the owner and name as GitHub shows them: letters, digits, dashes, underscores and dots",
                ));
            }
        }
        let loaded = self.load(&id).await?;
        crate::desk_service::refuse_channel_action(
            &self.starter.store,
            &self.machine,
            &id,
            "Add repo to this session",
        )
        .await?;
        if let Caller::Agent(agent) = caller
            && agent.session.id.as_str() != id
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::OwnSessionManage),
            }));
        }
        if self.lives_elsewhere(&loaded.record).is_some() {
            return Err(not_built_part(ADD_REPO, "a session on another machine"));
        }
        if loaded
            .detail
            .as_ref()
            .is_none_or(|detail| detail.start != StartStep::Done)
        {
            return Err(failed(
                Code::FailedPrecondition,
                &Refused::Starting { session: id.clone() },
                "wait for the session to start, then add the repo",
                None,
            ));
        }
        let channel = self.find_channel(&loaded.record.channel).await?;
        let have = channel
            .repos
            .iter()
            .find(|have| have.owner == repo.owner && have.name == repo.name);
        let shown = format!("{}/{}", repo.owner, repo.name);
        let is_user = matches!(caller, Caller::User(_));
        if have.is_none() && !is_user {
            return Err(failed(
                Code::FailedPrecondition,
                &Refused::NotInChannel { repo: shown, channel: channel.id.clone() },
                "propose the repo for the channel with channel_add_repo (it can add it to this session too), or ask the user to add it",
                None,
            ));
        }
        if let Some(asked) = self.gate(
            caller,
            Capability::OwnSessionManage,
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        let into_channel = is_user && have.is_none_or(|have| matches!(have.main, MainClone::Failed { .. }));
        let mut steps = Vec::new();
        if into_channel {
            steps.push(("add_to_channel".to_owned(), format!("Adding {shown} to the channel")));
        }
        steps.push((format!("clone:{}", repo.name), format!("Cloning {shown} into the session")));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
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
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let work = Work {
            svc: self.clone(),
            caller: caller.clone(),
            session: id,
            channel: channel.id.clone(),
            repo: (repo.owner, repo.name),
            into_channel,
        };
        tokio::spawn(work.run(progress, identity::actor(caller)));
        Ok(Gated::Done(operation))
    }
}

/// One AddRepoToSession running in the background.
struct Work {
    svc: SessionSvc,
    caller: Caller,
    session: String,
    channel: String,
    repo: (String, String),
    into_channel: bool,
}

impl Work {
    async fn run(self, progress: Progress, by: pb::Actor) {
        let mut step = String::new();
        let outcome = self.steps(&progress, &mut step).await;
        match outcome {
            Ok(()) => progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: self.session.clone(),
            }))),
            Err(error) => {
                progress.set_step(&step, pb::StepState::Failed, Some(status::chain(&error)));
                progress.fail(error_of(&error, fix_of(&error), None));
            }
        }
        self.svc
            .tell
            .session(&self.session, pb::Change::Updated, Some(by))
            .await;
    }

    async fn steps(&self, progress: &Progress, step: &mut String) -> Result<(), Failure> {
        let (owner, name) = &self.repo;
        let shown = format!("{owner}/{name}");
        if self.into_channel {
            "add_to_channel".clone_into(step);
            progress.set_step(step, pb::StepState::Running, None);
            self.add_to_channel(&shown).await?;
            progress.set_step(step, pb::StepState::Done, None);
        }
        format!("clone:{name}").clone_into(step);
        progress.set_step(step, pb::StepState::Running, None);
        // Waits for the channel's main clone: another call may be cloning it.
        self.wait_for_main(&shown).await?;
        let lock = self.svc.lifecycle.lock_of(&self.session);
        let guard = lock.lock_owned().await;
        let added = self.svc.starter.add_repo_to_session(&self.session, &self.repo).await;
        drop(guard);
        added?;
        progress.set_step(step, pb::StepState::Done, None);
        if let Some(channels) = self.svc.channels.get() {
            let (channels, channel, owner, name) =
                (channels.clone(), self.channel.clone(), owner.clone(), name.clone());
            let _ = blocking(move || channels.record_added(&owner, &name, &Place::Channel(channel))).await;
        }
        self.svc
            .tell
            .session(&self.session, pb::Change::Updated, Some(identity::actor(&self.caller)))
            .await;
        Ok(())
    }

    async fn add_to_channel(&self, shown: &str) -> Result<(), Failure> {
        let Some(channels) = self.svc.channels.get() else {
            return Err(Failure::NoChannels { repo: shown.to_owned() });
        };
        let request = pb::AddRepoRequest {
            idempotency_key: None,
            channel: Some(pb::ChannelId { value: self.channel.clone() }),
            repo: Some(pb::RepoRef { owner: self.repo.0.clone(), name: self.repo.1.clone() }),
        };
        channels
            .add_repo_to_channel(&self.caller, request, false)
            .await
            .map(drop)
            .map_err(|source| Failure::Channel { repo: shown.to_owned(), source })
    }

    /// Returns once the channel's main clone of the repo is ready here.
    async fn wait_for_main(&self, shown: &str) -> Result<(), Failure> {
        let deadline = tokio::time::Instant::now() + MAIN_CLONE_WAIT;
        loop {
            let (store, channel) = (self.svc.starter.store.clone(), self.channel.clone());
            let record = blocking(move || store.channel(&channel))
                .await
                .map_err(Failure::Background)?
                .map_err(|source| Failure::Store { source })?;
            let main = record.and_then(|record| {
                record
                    .repos
                    .into_iter()
                    .find(|have| have.owner == self.repo.0 && have.name == self.repo.1)
                    .map(|have| have.main)
            });
            match main {
                Some(MainClone::Ready) => return Ok(()),
                Some(MainClone::Failed { reason }) => {
                    return Err(Failure::MainFailed { repo: shown.to_owned(), reason });
                }
                // Not there yet: the clone step names it when it is read.
                None => return Ok(()),
                Some(MainClone::Cloning) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Failure::MainTimeout { repo: shown.to_owned() });
            }
            tokio::time::sleep(MAIN_CLONE_POLL).await;
        }
    }
}
