//! ForkSession onto another machine (FK-8, FK-12, flow 26.5; D195). The
//! parent keeps running where it is. The machine it is on stages a copy
//! of it as a new session on a new branch (`Starter::pack_fork`: the code as
//! it is now, with committed, uncommitted and untracked files, and, for code
//! and context, the conversation under an id of its own), and the fork
//! travels as a moved session does: the folder and the staged files are
//! copied through the carry driver, the target checks every hash and takes
//! the fork in, the host records it, and its agent starts there.
//!
//! The steps, in the operation: **pack**, **copy**, **unpack**, **launch**
//! and **verify**. A fork that fails at any step takes back what it put on
//! the target and what it staged, and says which step failed; the parent
//! is never touched. A target that is offline is refused before anything is
//! made (FK-12).
//!
//! A code-only fork has no conversation to resume: its agent is started
//! fresh by the target's ResumeSession (`unlaunched`, `launch_unlaunched`).

pub mod context;

use humata2_access::{AgentId, Call, Capability, ChannelId, MachineId, Target};
use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{Gated, blocking, failed, failure, invalid_field, not_found, status_error_of};
use humata2_store::{ChannelRecord, SessionRecord, StartStep};
use humata2_svc_sessions_common::fork::{human_bytes, not_built_part};
use humata2_svc_sessions_common::{Loaded, clean_title, pb, start_error};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::Checked;

pub const FORK_SESSION: &str = "humata.v1.SessionService.ForkSession";

/// The steps, as `(key, title)`.
pub const STEPS: [(&str, &str); 5] = [
    ("pack", "Copying the code and conversation"),
    ("copy", "Sending them to the other machine"),
    ("unpack", "Checking every file arrived"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

#[derive(Debug, thiserror::Error)]
pub enum ForkElsewhereError {
    #[error("session {session} is archived, so it can't be forked")]
    ParentArchived { session: String },
    #[error("machine {machine} isn't one of channel {channel}'s machines, so session {session} can't be forked onto it")]
    NotInChannel {
        session: String,
        machine: String,
        channel: String,
    },
    #[error("{folder} already exists on {machine}, so the fork's folder can't be copied there")]
    FolderTaken { folder: String, machine: String },
    #[error("the parent's start wasn't recorded on this machine, so it can't be forked onto another")]
    NoDetail { session: String },
    #[error(
        "a context-only fork works in session {session}'s clones on {machine}, so it can only run on {machine}"
    )]
    ContextOnlyElsewhere { session: String, machine: String },
    #[error("the fork's agent didn't start on {machine}")]
    NotStarted { machine: String },
}

/// A fork elsewhere, checked and the access rule passed.
pub struct Elsewhere {
    pub parent: Loaded,
    pub from: String,
    pub to: String,
    pub channel: ChannelRecord,
    pub copy: pb::ForkKind,
    pub title: String,
}

/// The preview's words for a fork elsewhere (FK-4).
pub fn elsewhere_text(checked: &Elsewhere, before_turn: u32, plan: &pb::CarryPreviewForkResponse) -> String {
    let repos: Vec<&str> = plan.repos.iter().map(|repo| repo.name.as_str()).collect();
    let context = match (checked.copy, before_turn) {
        (pb::ForkKind::CodeOnly, _) => "starts a fresh conversation".to_owned(),
        (_, 0) => "carries on the conversation from a copy of it".to_owned(),
        (_, turn) => format!("carries on the conversation up to turn {turn} from a copy of it"),
    };
    format!(
        "Forks \u{201c}{}\u{201d} into a new session on {} on branch {} in {}. It copies the code as it is now on {}, with committed, uncommitted and untracked files ({}), and {context}.",
        checked.parent.record.title,
        checked.to,
        plan.branch,
        repos.join(", "),
        checked.from,
        human_bytes(plan.copy_bytes),
    )
}

/// The preview of a context-only fork on the parent's machine (D187): the
/// parent's clones on their branches, and nothing copied.
pub fn context_preview(checked: &Elsewhere) -> pb::ForkPreview {
    let repos = checked.parent.detail.repos.as_slice();
    let on = repos
        .iter()
        .map(|repo| format!("{} on {}", repo.name, repo.branch))
        .collect::<Vec<_>>()
        .join(", ");
    pb::ForkPreview {
        text: format!(
            "Forks \u{201c}{}\u{201d} into a new session on {} that works in the parent's files: its clones are the parent's, on their branches ({on}), so what it changes lands in them. It copies no code and carries on the conversation from a copy of it.",
            checked.parent.record.title, checked.to,
        ),
        branches: repos
            .iter()
            .map(|repo| pb::preview_branch_names_response::Name {
                repo: Some(pb::RepoRef {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                }),
                branch: repo.branch.clone(),
                suffix_reason: String::new(),
            })
            .collect(),
        copy_bytes: 0,
        seeded_with_summary: false,
    }
}

/// The machine `record` lives on, from this host's point of view.
pub fn machine_of(record: &SessionRecord, here: &str) -> String {
    if record.machine.is_empty() {
        here.to_owned()
    } else {
        record.machine.clone()
    }
}

/// An offline answer as a failed precondition: the fork is refused as it
/// stands, and nothing changes (FK-12).
pub fn offline(status: Status) -> Status {
    match status_error_of(&status) {
        Some(error) => failure(Code::FailedPrecondition, error),
        None => status,
    }
}

/// Whether `loaded` is a carried session that never had an agent: its first
/// ResumeSession starts one fresh.
pub fn unlaunched(loaded: &Loaded) -> bool {
    loaded.detail.start == StartStep::Cloned
}

impl SessionTransferSvc {
    /// A typed branch is refused when the fork is asked for, not when the
    /// parent's machine packs it: a name git doesn't accept always, and one
    /// that is taken when the parent's clones are on this machine. A parent
    /// on another machine has its taken names checked there, at pack.
    pub async fn check_typed_branch(&self, parent: &Loaded, from: &str, typed: &str) -> Result<(), Status> {
        if typed.is_empty() {
            return Ok(());
        }
        if !humata2_sessions::valid_branch_name(typed) {
            return Err(start_error(&humata2_sessions::StartError::BadBranch {
                branch: typed.to_owned(),
            }));
        }
        if !self.env.is_host(from) {
            return Ok(());
        }
        let (starter, id, title, typed) = (
            self.core.starter.clone(),
            parent.record.id.clone(),
            parent.record.title.clone(),
            typed.to_owned(),
        );
        blocking(move || starter.check_fork_branch(&id, &title, &typed))
            .await?
            .map_err(|error| start_error(&error))
    }

    /// The checks of a fork whose parent or target is on another machine
    /// than this host, and the access rule passed. None when the fork is a
    /// local one, which the caller goes on with.
    pub async fn check_elsewhere(
        &self,
        caller: &Caller,
        request: &pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Option<Gated<Elsewhere>>, Status> {
        let Some(parent_id) = request
            .parent
            .as_ref()
            .map(|parent| parent.value.clone())
            .filter(|parent| !parent.is_empty())
        else {
            return Ok(None);
        };
        let Ok(parent) = self.env.load_session(&self.core, &parent_id) else {
            return Ok(None);
        };
        let from = machine_of(&parent.record, &self.core.machine);
        let to = request
            .machine
            .as_ref()
            .map(|machine| machine.value.clone())
            .filter(|machine| !machine.is_empty())
            .unwrap_or_else(|| from.clone());
        if self.env.is_host(&from) && self.env.is_host(&to) {
            return Ok(None);
        }

        let copy = match pb::ForkKind::try_from(request.kind) {
            Ok(pb::ForkKind::CodeAndContext) => pb::ForkKind::CodeAndContext,
            Ok(pb::ForkKind::CodeOnly) => pb::ForkKind::CodeOnly,
            Ok(pb::ForkKind::ContextOnly) if from != to => {
                // D208: it works in the parent's clones, which are on `from`.
                return Err(failed(
                    Code::FailedPrecondition,
                    &ForkElsewhereError::ContextOnlyElsewhere {
                        session: parent.record.id.clone(),
                        machine: from.clone(),
                    },
                    "fork with code and context to work on another machine, or fork with context only onto the parent's machine",
                    None,
                ));
            }
            // D187: from and to are the same machine, a box; it forks there.
            Ok(pb::ForkKind::ContextOnly) => pb::ForkKind::ContextOnly,
            _ => {
                return Err(invalid_field(
                    "kind",
                    Problem::Missing,
                    "the request doesn't say what the fork carries",
                    "set kind to CODE_AND_CONTEXT, CODE_ONLY or CONTEXT_ONLY, and send it again",
                ));
            }
        };
        if copy == pb::ForkKind::CodeOnly && request.before_turn != 0 {
            return Err(invalid_field(
                "before_turn",
                Problem::NotAllowedHere,
                "a fork before a turn carries the conversation up to that turn, and a code-only fork carries none",
                "fork with CODE_AND_CONTEXT, or set before_turn to 0 to fork from now",
            ));
        }
        if copy == pb::ForkKind::ContextOnly && !request.branch.is_empty() {
            return Err(invalid_field(
                "branch",
                Problem::NotAllowedHere,
                "a fork that carries only the conversation works in its parent's clones, so it has no branch to name",
                "clear the branch, or fork with CODE_AND_CONTEXT or CODE_ONLY to get a branch of its own",
            ));
        }
        self.check_typed_branch(&parent, &from, &request.branch).await?;
        if request
            .agent
            .as_ref()
            .is_some_and(|agent| !agent.value.is_empty() && agent.value != parent.record.agent)
        {
            return Err(not_built_part(
                "a fork that runs another agent than its parent",
            ));
        }
        let (store, id) = (self.core.store.clone(), parent_id.clone());
        let archived = blocking(move || store.archive(&id))
            .await?
            .map_err(|error| failed(Code::Internal, &error, "look up the session", None))?;
        if archived.is_some() {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::ParentArchived { session: parent_id },
                "restore the session first, then fork it",
                None,
            ));
        }
        let (store, ch_id) = (self.core.store.clone(), parent.record.channel.clone());
        let channel = blocking(move || store.channel(&ch_id))
            .await?
            .map_err(|error| failed(Code::Internal, &error, "look up the channel", None))?
            .ok_or_else(|| {
                not_found(
                    pb::EntityKind::Channel,
                    &parent.record.channel,
                    "channel was not found",
                    "check the channel id and try again",
                )
            })?;
        if !channel.machines.iter().any(|machine| machine == &to) && !self.env.is_host(&to) {
            return Err(failed(
                Code::FailedPrecondition,
                &ForkElsewhereError::NotInChannel {
                    session: parent.record.id.clone(),
                    machine: to,
                    channel: channel.name.clone(),
                },
                "add the machine to the channel first, or pick one of the channel's machines",
                None,
            ));
        }
        if !approved {
            let capability = if self.env.is_host(&to) {
                Capability::SessionStartSameMachine
            } else {
                Capability::SessionStartOtherMachine
            };
            if let Checked::Ask(asked) = self.env.check_capability(
                caller,
                capability,
                Target::Start {
                    channel: ChannelId::new(channel.id.as_str()),
                    machine: MachineId::new(to.as_str()),
                    agent: AgentId::new(parent.record.agent.as_str()),
                    fork_of: Some(humata2_access::SessionId::new(parent.record.id.as_str())),
                },
            )? {
                return Ok(Some(Gated::Ask(Box::new(asked))));
            }
        }
        let title = if request.title.trim().is_empty() {
            clean_title(&format!("Fork of {}", parent.record.title))?
        } else {
            clean_title(&request.title)?
        };
        Ok(Some(Gated::Done(Elsewhere {
            parent,
            from,
            to,
            channel,
            copy,
            title,
        })))
    }

    /// PreviewFork when the fork or its parent is on another machine than
    /// this host. None when the fork is a local one.
    pub async fn preview_fork_elsewhere_of(
        &self,
        caller: &Caller,
        request: &pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Option<Gated<pb::ForkPreview>>, Status> {
        let checked = match self.check_elsewhere(caller, request, approved).await? {
            None => return Ok(None),
            Some(Gated::Ask(asked)) => return Ok(Some(Gated::Ask(asked))),
            Some(Gated::Done(checked)) => checked,
        };
        if checked.copy == pb::ForkKind::ContextOnly {
            return Ok(Some(Gated::Done(context_preview(&checked))));
        }
        Err(self.env.not_wired("SessionService.PreviewFork (remote)"))
    }

    /// ForkSession when the fork or its parent is on another machine than
    /// this host. None when the fork is a local one.
    pub async fn fork_elsewhere_of(
        &self,
        caller: &Caller,
        request: &pb::ForkSessionRequest,
        approved: bool,
    ) -> Result<Option<Gated<pb::Operation>>, Status> {
        let Elsewhere {
            parent,
            to,
            copy,
            title,
            ..
        } = match self.check_elsewhere(caller, request, approved).await? {
            None => return Ok(None),
            Some(Gated::Ask(asked)) => return Ok(Some(Gated::Ask(asked))),
            Some(Gated::Done(checked)) => checked,
        };
        if copy == pb::ForkKind::ContextOnly {
            let there = context::ContextThere {
                parent: parent.record,
                parent_detail: parent.detail,
                machine: to,
                title,
            };
            return self
                .fork_context_there(caller, request, there)
                .await
                .map(|operation| Some(Gated::Done(operation)));
        }
        Err(self.env.not_wired("SessionService.ForkSession (remote)"))
    }

    /// ResumeSession for a carried session that never had an agent: starts
    /// one fresh, as a launch does.
    pub async fn launch_unlaunched(
        &self,
        _caller: &Caller,
        _key: Option<pb::IdempotencyKey>,
        _loaded: Loaded,
    ) -> Result<Gated<pb::Operation>, Status> {
        Err(self.env.not_wired("SessionService.ResumeSession (launch_unlaunched)"))
    }
}
