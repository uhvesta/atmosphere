//! Grants and their targets between the proto and humata2_access: what a
//! grant looks like to clients, and the call an ask-ahead is checked as.

use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{
    AgentId, AnswerError, BoardAct, BoardRef, Call, Capability, Entry, Grant, GrantTarget,
    MachineAction, MachineActionKind, MachineId, NetworkTarget, PathScope, Place, RepoBranch,
    RepoPath, RepoRef, Root, SessionId, SessionRef, Target,
};
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::guard;
use crate::identity::{self, AgentCaller};
use crate::status;

/// The capability a proto value names, if it names one.
pub(crate) fn capability_of(value: i32) -> Option<Capability> {
    Capability::ALL
        .into_iter()
        .find(|capability| guard::capability(*capability) as i32 == value)
}

pub(crate) fn timestamp(at: SystemTime) -> Timestamp {
    let elapsed = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    Timestamp {
        seconds: i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(elapsed.subsec_nanos()).unwrap_or(0),
    }
}

/// A grant's id as clients see it: its number.
pub(crate) fn grant_id(grant: &Grant) -> pb::GrantId {
    pb::GrantId {
        value: grant.id.get().to_string(),
    }
}

/// The number in a grant's id as clients see it.
pub(crate) fn grant_number(grant: &pb::Grant) -> Option<u64> {
    grant.id.as_ref()?.value.parse().ok()
}

/// A grant as clients see it. `suspended` is true while its rule is Never.
pub(crate) fn grant(grant: &Grant, suspended: bool) -> pb::Grant {
    pb::Grant {
        id: Some(grant_id(grant)),
        channel: Some(pb::ChannelId {
            value: grant.channel.to_string(),
        }),
        capability: guard::capability(grant.capability) as i32,
        requester: Some(session_id(&grant.requester)),
        target: Some(target(&grant.target)),
        granted_from: Some(pb::RequestId {
            value: grant.granted_from.to_string(),
        }),
        granted_at: Some(timestamp(grant.granted_at)),
        // Not recorded yet.
        last_used_at: None,
        suspended,
    }
}

/// A grant target as clients see it.
pub(crate) fn target(target: &GrantTarget) -> pb::GrantTarget {
    use pb::grant_target::Target as T;
    let target = match target {
        GrantTarget::Session(session) => T::Session(session_id(session)),
        GrantTarget::Board(board) => T::Board(pb::BoardRef {
            board: Some(match board {
                BoardRef::Global => pb::board_ref::Board::Global(pb::GlobalBoard {}),
                BoardRef::Channel(channel) => pb::board_ref::Board::Channel(pb::ChannelId {
                    value: channel.to_string(),
                }),
                BoardRef::Session(session) => pb::board_ref::Board::Session(session_id(&session.id)),
            }),
        }),
        GrantTarget::Path(scope) => T::Path(path_scope(scope)),
        GrantTarget::Network(network) => T::Network(pb::NetworkTarget {
            host: network.host.clone(),
            port: u32::from(network.port),
            machine: Some(machine_id(&network.machine)),
        }),
        GrantTarget::AgentOnMachine { machine, agent } => T::AgentOnMachine(pb::AgentOnMachine {
            machine: Some(machine_id(machine)),
            agent: Some(pb::AgentId {
                value: agent.to_string(),
            }),
        }),
        GrantTarget::RepoBranch(branch) => T::RepoBranch(pb::RepoBranch {
            repo: Some(repo_ref(&branch.repo)),
            branch: branch.branch.clone(),
        }),
        GrantTarget::MachineAction(action) => {
            let creates = matches!(action.action, MachineActionKind::Add | MachineActionKind::Create);
            T::MachineAction(pb::MachineAction {
                machine: (!creates).then(|| machine_id(&action.machine)),
                new_machine: if creates {
                    action.machine.to_string()
                } else {
                    String::new()
                },
                action: machine_action_kind(action.action) as i32,
            })
        }
    };
    pb::GrantTarget {
        target: Some(target),
    }
}

/// A clone's paths start with the repo's owner and name: a session's clone
/// holds each of its repos at `<owner>/<name>`.
fn path_scope(scope: &PathScope) -> pb::PathScope {
    let session = match &scope.root {
        Root::Clone(owner) => Some(session_id(&owner.id)),
        Root::Shared(_)
        | Root::ChannelFolder(_)
        | Root::MainClone { .. }
        | Root::Home
        | Root::Outside => None,
    };
    let prefix = scope.prefix.as_str();
    let mut parts = prefix.splitn(3, '/');
    let prefix = match (&scope.root, parts.next(), parts.next()) {
        (Root::Clone(_), Some(owner), Some(name)) => pb::RepoPath {
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            path: parts.next().unwrap_or_default().to_owned(),
        },
        _ => pb::RepoPath {
            repo: None,
            path: prefix.to_owned(),
        },
    };
    pb::PathScope {
        machine: Some(machine_id(&scope.machine)),
        session,
        prefix: Some(prefix),
    }
}

/// The call an agent asking ahead for `capability` on `target` would make,
/// for the policy to check (IN-26). Sessions it names are looked up in
/// `store`; `here` is this machine.
pub(crate) fn ask_ahead_call(
    store: &Store,
    here: &str,
    agent: &AgentCaller,
    capability: Capability,
    target: pb::GrantTarget,
) -> Result<Call, Status> {
    use pb::grant_target::Target as T;
    let me = &agent.session;
    let not_for = |field: &str| {
        status::invalid_field(
            field,
            Problem::NotAllowedHere,
            format!(
                "\"{capability}\" isn't asked ahead for on this kind of target"
            ),
            "name the target the capability acts on: a session for notes and typing, a path for files, an agent on a machine for starting sessions, a host and port for network, a repo and branch for pushes, a machine action for machines",
        )
    };
    match target.target {
        None => Err(status::missing("target")),
        Some(T::Session(session)) => {
            let session = session_of(store, here, Some(session), "target.session")?;
            Ok(Call::Use {
                capability,
                target: Target::Session(session),
            })
        }
        Some(T::Path(scope)) => {
            let owner = session_of(store, here, scope.session, "target.path.session")?;
            if let Some(machine) = scope.machine.filter(|machine| !machine.value.is_empty())
                && machine.value != owner.machine.as_str()
            {
                return Err(status::invalid_field(
                    "target.path.machine",
                    Problem::NotAllowedHere,
                    format!(
                        "session {} runs on machine {}, not {}",
                        owner.id, owner.machine, machine.value
                    ),
                    "leave the machine out, or name the one the session runs on",
                ));
            }
            let prefix = clone_path(scope.prefix)?;
            let place = Place {
                machine: owner.machine.clone(),
                root: Root::Clone(owner),
                path: prefix,
                entry: Entry::Folder,
            };
            match capability {
                Capability::FilesReadOtherSession | Capability::FilesReadOtherMachine => {
                    Ok(Call::ReadFiles(place))
                }
                // Copied into the asker's own clone.
                Capability::FilesCopyIn => Ok(Call::CopyFiles {
                    from: place,
                    into: Place {
                        machine: me.machine.clone(),
                        root: Root::Clone(me.clone()),
                        path: RepoPath::root(),
                        entry: Entry::Folder,
                    },
                }),
                _ => Err(not_for("target.path")),
            }
        }
        Some(T::Network(network)) => {
            if network.host.trim().is_empty() {
                return Err(status::missing("target.network.host"));
            }
            let port = u16::try_from(network.port)
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    status::invalid_field(
                        "target.network.port",
                        Problem::OutOfRange,
                        format!("port {} isn't a TCP port", network.port),
                        "give a port from 1 to 65535",
                    )
                })?;
            Ok(Call::Use {
                capability,
                target: Target::Network(NetworkTarget {
                    // HO-11: a forward opens from the asker's own machine.
                    machine: machine_or(network.machine, &me.machine),
                    host: network.host,
                    port,
                }),
            })
        }
        Some(T::AgentOnMachine(start)) => {
            let agent = start
                .agent
                .map(|agent| agent.value)
                .filter(|agent| !agent.is_empty())
                .ok_or_else(|| status::missing("target.agent_on_machine.agent"))?;
            Ok(Call::Use {
                capability,
                target: Target::Start {
                    channel: me.channel.clone(),
                    machine: machine_or(start.machine, &me.machine),
                    agent: AgentId::new(agent),
                },
            })
        }
        Some(T::RepoBranch(branch)) => {
            let repo = branch
                .repo
                .filter(|repo| !repo.owner.is_empty() && !repo.name.is_empty())
                .ok_or_else(|| status::missing("target.repo_branch.repo"))?;
            if branch.branch.trim().is_empty() {
                return Err(status::missing("target.repo_branch.branch"));
            }
            Ok(Call::Use {
                capability,
                target: Target::Push(RepoBranch {
                    repo: RepoRef::new(format!("{}/{}", repo.owner, repo.name)),
                    branch: branch.branch,
                }),
            })
        }
        Some(T::MachineAction(action)) => {
            let kind = pb::MachineActionKind::try_from(action.action)
                .ok()
                .and_then(action_kind)
                .ok_or_else(|| status::missing("target.machine_action.action"))?;
            let creates = matches!(kind, MachineActionKind::Add | MachineActionKind::Create);
            let machine = if creates {
                Some(action.new_machine).filter(|name| !name.is_empty())
            } else {
                action
                    .machine
                    .map(|machine| machine.value)
                    .filter(|machine| !machine.is_empty())
            };
            let field = if creates {
                "target.machine_action.new_machine"
            } else {
                "target.machine_action.machine"
            };
            let machine = machine.ok_or_else(|| status::missing(field))?;
            Ok(Call::Use {
                capability,
                target: Target::Machine(MachineAction {
                    machine: MachineId::new(machine),
                    action: kind,
                }),
            })
        }
        Some(T::Board(board)) => {
            let board = match board.board {
                None => return Err(status::missing("target.board")),
                Some(pb::board_ref::Board::Global(_)) => BoardRef::Global,
                Some(pb::board_ref::Board::Channel(channel)) if !channel.value.is_empty() => {
                    BoardRef::Channel(humata2_access::ChannelId::new(channel.value))
                }
                Some(pb::board_ref::Board::Channel(_)) => {
                    return Err(status::missing("target.board.channel"));
                }
                Some(pb::board_ref::Board::Session(session)) => BoardRef::Session(session_of(
                    store,
                    here,
                    Some(session),
                    "target.board.session",
                )?),
            };
            let act = match capability {
                Capability::ChannelBoardAdd => BoardAct::Add,
                // Items someone else made.
                Capability::ChannelBoardEditOthers | Capability::GlobalBoardWrite => {
                    BoardAct::Edit { created_by: None }
                }
                _ => return Err(not_for("target.board")),
            };
            Ok(Call::Board { board, act })
        }
        // The deprecated kinds: IN-22 offers neither.
        Some(_) => Err(status::invalid_field(
            "target",
            Problem::NotAllowedHere,
            "grants on a whole machine or on one board item aren't offered (IN-22)",
            "ask for a path (a machine's whole folder is an empty prefix), an agent on a machine, or a board",
        )),
    }
}

/// Why approving failed, as the status to return. Nothing was approved.
pub(crate) fn answer_failed(error: &AnswerError) -> Status {
    let (code, kind) = match error {
        AnswerError::RuleIsNever { capability, .. } => (
            Code::PermissionDenied,
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::NeverRule as i32,
                capability: guard::capability(*capability) as i32,
                request: None,
                user_note: String::new(),
            }),
        ),
        AnswerError::NotARule { capability } => (
            Code::PermissionDenied,
            pb::error::Kind::UserOnly(pb::UserOnly {
                capability: guard::capability(*capability) as i32,
            }),
        ),
        AnswerError::PassphraseNeeded => (
            Code::PermissionDenied,
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: vec![pb::presence_required::Method::ApprovalPassphrase as i32],
                caller_in_session_terminal: false,
            }),
        ),
        AnswerError::NeedsHost => (
            Code::Unavailable,
            pb::error::Kind::HostUnreachable(pb::HostUnreachable::default()),
        ),
        AnswerError::NoStandingApproval | AnswerError::TargetNotOffered => {
            return status::invalid_field(
                "answer.always.target",
                Problem::NotAllowedHere,
                status::chain(error),
                "answer with the request's Always allow target exactly as offered, or allow it once",
            );
        }
        AnswerError::UnknownChannel { .. } => {
            return status::internal(
                format!("couldn't approve the request: {}", status::chain(error)),
                "",
            );
        }
    };
    status::failure(code, status::error(status::chain(error), fix_of(error), kind))
}

fn fix_of(error: &AnswerError) -> &'static str {
    match error {
        AnswerError::RuleIsNever { .. } => {
            "change the rule back from Never in Agent access first if it should run, or deny it"
        }
        AnswerError::NotARule { .. } => "deny it; only you can do this, yourself",
        AnswerError::PassphraseNeeded => {
            "run `humata inbox` with the approval passphrase, or answer in the app once the Mac is back"
        }
        AnswerError::NeedsHost => "answer it in the app once the Mac is reachable",
        AnswerError::NoStandingApproval
        | AnswerError::TargetNotOffered
        | AnswerError::UnknownChannel { .. } => "",
    }
}

/// The session `id` names, from the store. A session this machine doesn't
/// know is not found.
fn session_of(
    store: &Store,
    here: &str,
    id: Option<pb::SessionId>,
    field: &str,
) -> Result<SessionRef, Status> {
    let id = id
        .map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing(field))?;
    match store.session(&id) {
        Ok(Some(record)) => Ok(identity::session_ref(&record, here)),
        Ok(None) => Err(status::not_found(
            pb::EntityKind::Session,
            &id,
            format!("no session {id} on this machine"),
            "check the session id; sessions on other machines can't be named here yet",
        )),
        Err(error) => Err(status::store(error, here, "look up the session")),
    }
}

/// A path in a session's clone: the repo's `<owner>/<name>`, then the path
/// in it. No repo means the whole clone.
fn clone_path(prefix: Option<pb::RepoPath>) -> Result<RepoPath, Status> {
    let prefix = prefix.unwrap_or_default();
    let path = match prefix.repo {
        None if prefix.path.is_empty() => String::new(),
        None => {
            return Err(status::missing("target.path.prefix.repo"));
        }
        Some(repo) if repo.owner.is_empty() || repo.name.is_empty() => {
            return Err(status::missing("target.path.prefix.repo"));
        }
        Some(repo) if prefix.path.is_empty() => format!("{}/{}", repo.owner, repo.name),
        Some(repo) => format!("{}/{}/{}", repo.owner, repo.name, prefix.path),
    };
    RepoPath::new(path).map_err(|error| {
        status::invalid_field(
            "target.path.prefix.path",
            Problem::BadFormat,
            status::chain(&error),
            "give a clean path relative to the repo, like src/ledger",
        )
    })
}

fn machine_or(machine: Option<pb::MachineId>, own: &MachineId) -> MachineId {
    machine
        .map(|machine| machine.value)
        .filter(|machine| !machine.is_empty())
        .map_or_else(|| own.clone(), MachineId::new)
}

fn session_id(session: &SessionId) -> pb::SessionId {
    pb::SessionId {
        value: session.to_string(),
    }
}

fn machine_id(machine: &MachineId) -> pb::MachineId {
    pb::MachineId {
        value: machine.to_string(),
    }
}

fn repo_ref(repo: &RepoRef) -> pb::RepoRef {
    let (owner, name) = repo.as_str().split_once('/').unwrap_or(("", repo.as_str()));
    pb::RepoRef {
        owner: owner.to_owned(),
        name: name.to_owned(),
    }
}

fn machine_action_kind(kind: MachineActionKind) -> pb::MachineActionKind {
    use MachineActionKind as K;
    use pb::MachineActionKind as P;
    match kind {
        K::Add => P::Add,
        K::Create => P::Create,
        K::Start => P::Start,
        K::Stop => P::Stop,
        K::Reconnect => P::Reconnect,
        K::SyncSkills => P::SyncSkills,
        K::Remove => P::Remove,
        K::Forget => P::Forget,
        K::Delete => P::Delete,
    }
}

fn action_kind(kind: pb::MachineActionKind) -> Option<MachineActionKind> {
    use MachineActionKind as K;
    use pb::MachineActionKind as P;
    Some(match kind {
        P::Unspecified => return None,
        P::Add => K::Add,
        P::Create => K::Create,
        P::Start => K::Start,
        P::Stop => K::Stop,
        P::Reconnect => K::Reconnect,
        P::SyncSkills => K::SyncSkills,
        P::Remove => K::Remove,
        P::Forget => K::Forget,
        P::Delete => K::Delete,
    })
}
