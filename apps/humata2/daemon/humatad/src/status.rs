//! Failures as gRPC statuses: a code for generic handling and one
//! `humata.v1.Error` in the details, which clients branch on (GRPC.md §10).

use humata2_access::{Capability, Fix, PresenceMethod, Refusal, RepoRef, SessionId};
use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use prost::Message;
use tonic::codegen::Bytes;
use tonic::{Code, Status};

use crate::cli_login::LoginUnusable;
use crate::guard;
use crate::script_tokens::ScriptTokenUnusable;

/// `google.rpc.Status`, the wrapper the details travel in.
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

/// `google.protobuf.Any`.
#[derive(Clone, PartialEq, prost::Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

const ERROR_TYPE_URL: &str = "type.googleapis.com/humata.v1.Error";

pub(crate) fn failure(code: Code, error: pb::Error) -> Status {
    let message = error.message.clone();
    let details = RpcStatus {
        code: code as i32,
        message: message.clone(),
        details: vec![Any {
            type_url: ERROR_TYPE_URL.to_owned(),
            value: error.encode_to_vec(),
        }],
    };
    Status::with_details(code, message, Bytes::from(details.encode_to_vec()))
}

pub(crate) fn error(
    message: impl Into<String>,
    fix: impl Into<String>,
    kind: pb::error::Kind,
) -> pb::Error {
    pb::Error {
        message: message.into(),
        fix: fix.into(),
        retry: None,
        kind: Some(kind),
    }
}

pub(crate) fn not_found(
    kind: pb::EntityKind,
    id: &str,
    message: impl Into<String>,
    fix: impl Into<String>,
) -> Status {
    failure(
        Code::NotFound,
        error(
            message,
            fix,
            pb::error::Kind::NotFound(pb::NotFound {
                kind: kind as i32,
                id: id.to_owned(),
                in_trash: false,
            }),
        ),
    )
}

pub(crate) fn invalid_field(
    field: &str,
    problem: pb::invalid_field::Problem,
    message: impl Into<String>,
    fix: impl Into<String>,
) -> Status {
    failure(
        Code::InvalidArgument,
        error(
            message,
            fix,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: problem as i32,
            }),
        ),
    )
}

pub(crate) fn missing(field: &str) -> Status {
    invalid_field(
        field,
        pb::invalid_field::Problem::Missing,
        format!("the request has no {field}"),
        format!("set {field} and send it again"),
    )
}

/// A bug, or a failure the person can't fix from the request. The cause
/// goes to the daemon's log under an incident id the message names.
pub(crate) fn internal(message: impl Into<String>, log_path: &str) -> Status {
    let incident = format!("in_{}", uuid::Uuid::new_v4().simple());
    let message = message.into();
    crate::log::say!("humatad2: incident {incident}: {message}");
    failure(
        Code::Internal,
        error(
            format!("{message} (incident {incident})"),
            "",
            pb::error::Kind::Internal(pb::Internal {
                incident_id: incident,
                log_path: log_path.to_owned(),
            }),
        ),
    )
}

/// The service or method isn't part of this build of humatad2.
pub(crate) fn not_built(method: &str) -> Status {
    let message = format!("{method} isn't built into this humatad2 yet");
    let details = pb::Error {
        message: message.clone(),
        // D768: said by the daemon that lacks it, so it is about this build,
        // whatever the caller runs: updating helps only once a release
        // builds it, and the CLI and daemon may already be the same one.
        fix: "do this from the Humata app on the host Mac; this release of humatad2 doesn't do it here, so updating helps only once a newer release builds it".to_owned(),
        retry: None,
        kind: None,
    };
    failure(Code::Unimplemented, details)
}

/// A store failure, said in the API's terms: a full disk is `disk_full`,
/// a session that's gone is `not_found`, the rest are `internal`.
pub(crate) fn store(error: StoreError, machine: &str, doing: &str) -> Status {
    if error.is_disk_full() {
        let path = match &error {
            StoreError::Write { path, .. } => path.display().to_string(),
            _ => String::new(),
        };
        return failure(
            Code::ResourceExhausted,
            pb::Error {
                message: format!("couldn't {doing}: the disk is full"),
                fix: "free space on this machine and try again. Deleting a session's clones loses the work in them that isn't pushed".to_owned(),
                retry: None,
                kind: Some(pb::error::Kind::DiskFull(pb::DiskFull {
                    machine: Some(pb::MachineId {
                        value: machine.to_owned(),
                    }),
                    path,
                    needed_bytes: 0,
                    free_bytes: 0,
                })),
            },
        );
    }
    match error {
        StoreError::NoSession { id } => not_found(
            pb::EntityKind::Session,
            &id,
            format!("couldn't {doing}: no session {id} is on this machine"),
            "check the session id; it may have been removed or be on another machine",
        ),
        other => internal(format!("couldn't {doing}: {}", chain(&other)), ""),
    }
}

/// An error and its causes, on one line.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        // Most messages already name their cause (`could not fork …:
        // {source}`); add a cause only when it isn't said yet, so no reason
        // is printed twice.
        let said = cause.to_string();
        if !text.contains(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        source = cause.source();
    }
    text
}

/// A session token this daemon didn't give out, or one whose tab has
/// closed (MCP.md §1).
pub(crate) fn identity_mismatch() -> Status {
    failure(
        Code::Unauthenticated,
        error(
            "the session token isn't one this daemon gave out, or its tab has closed",
            "start the agent from a Humata session tab, which sets HUMATA_SESSION_TOKEN; don't copy a token from another tab, since the daemon refuses it",
            pb::error::Kind::IdentityMismatch(pb::IdentityMismatch {
                claimed: None,
                claimed_machine: None,
            }),
        ),
    )
}

/// A call with no credential: no session token, script token, CLI login or
/// app key (SEC-9). The proto has no kind for it; `presence_required` with
/// the CLI login is the closest, since that is how a person's shell gets in.
pub(crate) fn unauthenticated() -> Status {
    failure(
        Code::Unauthenticated,
        error(
            "humatad2 refused the call: it carried no credential, so the daemon can't tell a person from an agent or a script",
            "run `humata login` in this terminal and allow it in the Humata app, or use the app itself. A command in a Humata session tab acts as that session's agent. If you are developing Humata, run a test daemon built as humatad2_admin with `serve --admin`; never use it as your real daemon, since it lets anything on your account act as you",
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: vec![pb::presence_required::Method::CliLogin as i32],
                caller_in_session_terminal: false,
                tries_left: 0,
            }),
        ),
    )
}

/// A call whose CLI login doesn't work here (SEC-10, SEC-11).
pub(crate) fn login_unusable(unusable: &LoginUnusable) -> Status {
    let fix = match unusable {
        LoginUnusable::Unknown | LoginUnusable::Expired => {
            "run `humata login` in this terminal again and allow it in the Humata app"
        }
        LoginUnusable::OtherTerminal { .. } => {
            "run `humata login` in this terminal. Don't copy a login to another terminal or run a command with setsid to keep one: a login covers only the terminal session it was made in, and the daemon refuses it anywhere else"
        }
        LoginUnusable::NoProcess { .. } => {
            "run the command on this machine against the daemon's own socket; through a bridge or proxy the daemon can't see which terminal the call came from, so a CLI login can't be used there"
        }
    };
    failure(
        Code::Unauthenticated,
        error(
            format!("humatad2 refused the call: {}", chain(unusable)),
            fix,
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: vec![pb::presence_required::Method::CliLogin as i32],
                caller_in_session_terminal: false,
                tries_left: 0,
            }),
        ),
    )
}

/// A call whose script token doesn't work here (SEC-14).
pub(crate) fn script_token_unusable(unusable: &ScriptTokenUnusable) -> Status {
    let fix = match unusable {
        ScriptTokenUnusable::Expired { .. } => {
            "make a new token with `humata token create`; a token can't be renewed once it has expired"
        }
        ScriptTokenUnusable::Unknown => {
            "the token was revoked or made by another daemon; list this daemon's tokens with `humata token ls` and make a new one with `humata token create`"
        }
    };
    failure(
        Code::Unauthenticated,
        error(
            format!("humatad2 refused the call: {}", chain(unusable)),
            fix,
            pb::error::Kind::IdentityMismatch(pb::IdentityMismatch {
                claimed: None,
                claimed_machine: None,
            }),
        ),
    )
}

/// A script asking for the event feed, which is for the app and sessions.
pub(crate) fn script_has_no_event_feed() -> Status {
    failure(
        Code::PermissionDenied,
        error(
            "the event feed is for the app and for sessions; a script's token doesn't get it",
            "run `humata` commands from the script and read their output instead of subscribing to events",
            pb::error::Kind::UserOnly(pb::UserOnly { capability: 0 }),
        ),
    )
}

/// `humata login`, or a presence prompt, from inside a session's terminal
/// (SEC-12): the session's agent could answer it.
pub(crate) fn in_session_terminal() -> Status {
    failure(
        Code::PermissionDenied,
        error(
            "humata login can't run inside a Humata session's terminal: that session's agent could answer the prompt or use the login",
            "run `humata login` in a terminal outside Humata. Inside a session, ask the user with inbox_ask instead",
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: Vec::new(),
                caller_in_session_terminal: true,
                tries_left: 0,
            }),
        ),
    )
}

/// `humata login` from a process with no terminal session to bind.
pub(crate) fn login_needs_terminal(cause: &dyn std::error::Error) -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!(
                "humata login needs a terminal to bind, and the process that ran it has none: {}",
                chain(cause)
            ),
            fix: "run `humata login` in an interactive terminal; a login covers only that terminal's session. For a script, use a script token instead".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// The user chose Don't Allow for a login.
pub(crate) fn login_denied() -> Status {
    failure(
        Code::PermissionDenied,
        error(
            "the login was turned down in the Humata app (Don't Allow)",
            "if that was a slip, run `humata login` again and choose Allow. If you didn't start this login, something in that terminal tried to act as you: close the terminal rather than allow it",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::UserDenied as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// The passphrase login was tried while the host is reachable.
pub(crate) fn login_host_reachable() -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: "logging in with the approval passphrase works only while the Mac is away, and this machine can reach it now".to_owned(),
            fix: "log in from the Mac, or wait until it is away".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// The passphrase login was tried on a machine that holds no passphrase.
pub(crate) fn login_no_passphrase() -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: "this machine holds no approval passphrase, so it can't log anyone in while the Mac is away".to_owned(),
            fix: "set one in Settings > Security on the Mac, then connect this machine once so it receives it".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// A wrong approval passphrase (SEC-13): refused as any missing presence
/// is, with how many more wrong tries the machine allows before it locks.
pub(crate) fn passphrase_wrong(tries_left: u32) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            format!(
                "that approval passphrase is wrong; {tries_left} more wrong tries and this machine's passphrase prompt locks for 15 minutes"
            ),
            "type the approval passphrase from Settings › Security on the Mac exactly as it shows there, or answer from the app on the Mac. Every wrong try counts toward the lock, so don't guess",
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: vec![pb::presence_required::Method::ApprovalPassphrase as i32],
                caller_in_session_terminal: false,
                tries_left,
            }),
        ),
    )
}

/// Five wrong passphrase tries locked the prompt until `until`.
pub(crate) fn passphrase_locked(machine: &str, until: std::time::SystemTime) -> Status {
    passphrase_locked_at(machine, until, std::time::SystemTime::now())
}

/// [`passphrase_locked`] at the time `now`, which is named so the fix's
/// "in N minutes" can be tested.
fn passphrase_locked_at(
    machine: &str,
    until: std::time::SystemTime,
    now: std::time::SystemTime,
) -> Status {
    let since = until
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let at = humata2_grpc::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    };
    failure(
        Code::ResourceExhausted,
        error(
            "too many wrong approval passphrase tries: this machine's passphrase prompt is locked",
            format!(
                "wait {} (until {:02}:{:02} UTC), or answer from the app on the Mac. Trying again early does not shorten the lock",
                lock_wait(until.duration_since(now).map_or(0, |left| left.as_secs())),
                (since.as_secs() % 86_400) / 3_600,
                (since.as_secs() % 3_600) / 60,
            ),
            pb::error::Kind::PassphraseLocked(pb::PassphraseLocked {
                machine: Some(pb::MachineId {
                    value: machine.to_owned(),
                }),
                until: Some(at),
            }),
        ),
    )
}

/// How long a lock has left, in words: whole minutes, rounded up, or
/// "about a minute" for anything under it.
fn lock_wait(seconds: u64) -> String {
    match seconds.div_ceil(60) {
        0 | 1 => "about a minute".to_owned(),
        minutes => format!("about {minutes} minutes"),
    }
}

/// No one answered a login in the app in time.
pub(crate) fn login_unanswered(minutes: u64) -> Status {
    failure(
        Code::DeadlineExceeded,
        error(
            format!("no one answered the login in the Humata app within {minutes} minutes"),
            "open the Humata app on this Mac and run `humata login` again; the app asks for it there",
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: vec![pb::presence_required::Method::ConfirmInApp as i32],
                caller_in_session_terminal: false,
                tries_left: 0,
            }),
        ),
    )
}

/// A call only the app may make, such as answering a login.
pub(crate) fn app_only(what: &str) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            format!(
                "only the Humata app can {what}: a terminal can't, even a logged-in one, or anything able to type there could log in other terminals"
            ),
            "do it in the Humata app",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// The peer link refused a call: its two ends haven't proved they hold
/// this machine's enrollment key (GRPC.md §8).
pub(crate) fn peer_unproven(
    reason: pb::peer_unproven::Reason,
    machine: &str,
    message: impl Into<String>,
    fix: impl Into<String>,
) -> Status {
    failure(
        Code::Unauthenticated,
        error(
            message,
            fix,
            pb::error::Kind::PeerUnproven(pb::PeerUnproven {
                reason: reason as i32,
                machine: Some(pb::MachineId {
                    value: machine.to_owned(),
                }),
            }),
        ),
    )
}

/// A call the policy refused, with the proto kind and fix it names.
/// `refused`, with the machines a refusal names read as people read them:
/// the host as "This Mac" and a machine by its name, not its id (D755).
pub(crate) fn refused_named(refusal: &Refusal, names: &crate::upgrade::items::Names) -> Status {
    match refusal {
        Refusal::SharedOnOtherMachine { machine, here } => refused(&Refusal::SharedOnOtherMachine {
            machine: humata2_access::MachineId::new(names.of(machine.as_str())),
            here: humata2_access::MachineId::new(names.of(here.as_str())),
        }),
        other => refused(other),
    }
}

pub(crate) fn refused(refusal: &Refusal) -> Status {
    use pb::denied::Source as Denied;
    let denied = |source: Denied, capability: Option<Capability>| {
        pb::error::Kind::Denied(pb::Denied {
            source: source as i32,
            capability: capability.map_or(0, |capability| guard::capability(capability) as i32),
            request: None,
            user_note: String::new(),
        })
    };
    let session = |id: &SessionId| {
        Some(pb::SessionId {
            value: id.to_string(),
        })
    };
    let (code, kind, retry) = match refusal {
        Refusal::UserOnly { capability } => (
            Code::PermissionDenied,
            pb::error::Kind::UserOnly(pb::UserOnly {
                capability: capability.map_or(0, |capability| guard::capability(capability) as i32),
            }),
            false,
        ),
        Refusal::NeverRule { capability, .. } => (
            Code::PermissionDenied,
            denied(Denied::NeverRule, Some(*capability)),
            false,
        ),
        Refusal::ParentReadOnly { fork, parent } => (
            Code::PermissionDenied,
            pb::error::Kind::ParentReadOnly(pb::ParentReadOnly {
                fork: session(fork),
                parent: session(parent),
            }),
            false,
        ),
        Refusal::MainCloneReadOnly { repo, machine } => {
            let (owner, name) = repo.as_str().split_once('/').unwrap_or(("", repo.as_str()));
            (
                Code::PermissionDenied,
                pb::error::Kind::MainCloneReadOnly(pb::MainCloneReadOnly {
                    repo: Some(pb::RepoRef {
                        owner: owner.to_owned(),
                        name: name.to_owned(),
                    }),
                    machine: Some(pb::MachineId {
                        value: machine.to_string(),
                    }),
                }),
                false,
            )
        }
        Refusal::OutsideHumataFolders { .. } => (
            Code::PermissionDenied,
            pb::error::Kind::OutsideHumataFolders(pb::OutsideHumataFolders {
                path: String::new(),
            }),
            false,
        ),
        // PR-7: the repo and branch are filled in by the call that knows them
        // (`with_push_target`).
        Refusal::ReviewSessionPush { .. } => (
            Code::PermissionDenied,
            pb::error::Kind::PushRefused(pb::PushRefused {
                reason: pb::push_refused::Reason::ReviewReadOnly as i32,
                repo: None,
                branch: String::new(),
            }),
            false,
        ),
        Refusal::NotItsFolder { .. }
        | Refusal::SharedOnOtherMachine { .. }
        | Refusal::MachineStopped { .. } => (
            Code::PermissionDenied,
            denied(Denied::HardLimit, None),
            false,
        ),
        Refusal::OtherChannel { .. }
        | Refusal::ScriptTokenHasNoSession
        | Refusal::ScriptTokenOtherChannel { .. }
        | Refusal::PeerCannotCall => (
            Code::PermissionDenied,
            denied(Denied::Unspecified, None),
            false,
        ),
        Refusal::ScriptTokenLacks { capability }
        | Refusal::ScriptTokenCannotAsk { capability }
        | Refusal::PeerOnly { capability } => (
            Code::PermissionDenied,
            denied(Denied::Unspecified, Some(*capability)),
            false,
        ),
        Refusal::PresenceRequired { accepted } => (
            Code::PermissionDenied,
            pb::error::Kind::PresenceRequired(pb::PresenceRequired {
                accepted: accepted
                    .iter()
                    .map(|method| presence_method(*method) as i32)
                    .collect(),
                caller_in_session_terminal: false,
                tries_left: 0,
            }),
            false,
        ),
        Refusal::UserAway { channel } => (
            Code::Unavailable,
            pb::error::Kind::UserAway(pb::UserAway {
                channel: Some(pb::ChannelId {
                    value: channel.to_string(),
                }),
            }),
            true,
        ),
        Refusal::HostUnreachable => (
            Code::Unavailable,
            pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
            true,
        ),
        Refusal::NothingToAskAhead { .. } => (
            Code::InvalidArgument,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "capability".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
            false,
        ),
    };
    failure(
        code,
        pb::Error {
            message: refusal.to_string(),
            fix: fix_text(&refusal.fix()),
            retry: retry.then_some(pb::RetryAdvice {
                after: None,
                same_key: true,
            }),
            kind: Some(kind),
        },
    )
}

/// A review session's push refusal, with the repository and branch it was
/// refused for; any other failure as it was.
pub(crate) fn with_push_target(refused: Status, repo: &RepoRef, branch: &str) -> Status {
    let Some(mut error) = error_of(&refused) else {
        return refused;
    };
    let Some(pb::error::Kind::PushRefused(push)) = &mut error.kind else {
        return refused;
    };
    if push.reason != pb::push_refused::Reason::ReviewReadOnly as i32 {
        return refused;
    }
    let (owner, name) = repo.as_str().split_once('/').unwrap_or(("", repo.as_str()));
    push.repo = Some(pb::RepoRef {
        owner: owner.to_owned(),
        name: name.to_owned(),
    });
    push.branch = branch.to_owned();
    failure(refused.code(), error)
}

/// A call the channel's rules send to the Inbox, which this daemon can't
/// make for it yet.
///
/// It is `denied`, not a missing method (D768): an Unimplemented status
/// with no kind reads as `peer_version_unsupported` to an agent, which sent
/// agents to update a machine that was current.
pub(crate) fn needs_approval(capability: Capability) -> Status {
    let message = format!(
        "\"{capability}\" needs the user's approval here, and this call can't wait for it in the Inbox, so nothing was done"
    );
    failure(
        Code::PermissionDenied,
        error(
            message,
            "ask the user with inbox_ask to do it themselves in the app, or to let agents do it in the channel's rules; don't retry the call, since it is asked again each time",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: guard::capability(capability) as i32,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// A call the channel's rule sent to the Inbox (D38.6): it runs once the
/// user approves it, and its result comes from waiting on `request`.
pub(crate) fn approval_pending(request: &pb::RequestId) -> Status {
    failure(
        Code::FailedPrecondition,
        error(
            format!(
                "this call needs the user's approval and is waiting for their answer in the Inbox as request {}",
                request.value
            ),
            "call inbox_wait (WaitForAnswer) with this request id to get the result once the user approves; don't repeat the call without the same idempotency key, which would put a second request in the Inbox",
            pb::error::Kind::ApprovalPending(pb::ApprovalPending {
                request: Some(request.clone()),
            }),
        ),
    )
}

/// What an approved asked read keeps as its result type (D768): approving
/// it runs nothing, so the waiter is told to repeat the read, which then goes
/// through. Not a proto message: no bytes come with it.
pub(crate) const READ_APPROVED: &str = "humata2.ReadApproved";

/// An agent's file read the channel's rule sent to the Inbox (D768). Unlike
/// [`approval_pending`], approving it runs nothing: the agent repeats the
/// read once the user approves, and a repeat before then meets this request.
pub(crate) fn read_approval_pending(request: &pb::RequestId) -> Status {
    failure(
        Code::FailedPrecondition,
        error(
            format!(
                "this read needs the user's approval and is waiting for their answer in the Inbox as request {}",
                request.value
            ),
            "call inbox_wait (WaitForAnswer) with this request id; once the user approves, repeat the same read to get its result. Repeating it before then shows this request again and doesn't add another",
            pb::error::Kind::ApprovalPending(pb::ApprovalPending {
                request: Some(request.clone()),
            }),
        ),
    )
}

/// A call only the user can make, at the app: an agent or script that makes
/// it is told what was refused and the fix it can act on (MP-14, MP-17).
/// `what` names the call ("logging in to a machine by hand").
pub(crate) fn user_only_call(capability: Capability, what: &str, fix: &str) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            format!("{what} is the user's to do, at the app; an agent or a script can't do it"),
            fix,
            pb::error::Kind::UserOnly(pb::UserOnly {
                capability: guard::capability(capability) as i32,
            }),
        ),
    )
}

/// The `humata.v1.Error` a status carries, if it has one.
pub(crate) fn error_of(status: &Status) -> Option<pb::Error> {
    let details = RpcStatus::decode(status.details()).ok()?;
    details
        .details
        .into_iter()
        .find(|any| any.type_url == ERROR_TYPE_URL)
        .and_then(|any| pb::Error::decode(any.value.as_slice()).ok())
}

fn presence_method(method: PresenceMethod) -> pb::presence_required::Method {
    match method {
        PresenceMethod::ConfirmInApp => pb::presence_required::Method::ConfirmInApp,
        PresenceMethod::CliLogin => pb::presence_required::Method::CliLogin,
        PresenceMethod::ApprovalPassphrase => pb::presence_required::Method::ApprovalPassphrase,
    }
}

fn fix_text(fix: &Fix) -> String {
    match fix {
        Fix::AskTheUser => {
            "ask the user with inbox_ask; they can do it themselves in the app".to_owned()
        }
        Fix::UserChangesRule {
            channel,
            capability,
        } => format!(
            "only the user can change channel {channel}'s rule for \"{capability}\", in Agent access; asking again is refused the same way"
        ),
        Fix::UseOwnFolders => {
            "work in the session's own clone or the channel's shared/ folder".to_owned()
        }
        Fix::PutInSharedHereForPull { machine } => format!(
            "put the file in shared/ here; a session on {machine} can copy it in from there, or the user can send it with `humata cp`"
        ),
        Fix::SessionCloneOrRemoveRepo { repo } => format!(
            "make the change in a session's clone of {repo}; if the main clone itself should go, remove {repo} from the channel, which deletes its main clones"
        ),
        Fix::UserAllowsReviewPush { session } => format!(
            "the user can turn on \"Let the agent push to the PR branch\" in session {session}'s Info panel"
        ),
        Fix::ShowPresence { accepted } if accepted.is_empty() => {
            "run it from the app or a terminal outside Humata".to_owned()
        }
        Fix::ShowPresence { .. } => {
            "show the user is there in one of the ways the message names, then try again".to_owned()
        }
        Fix::WaitForUser => "try again when the user is back".to_owned(),
        Fix::WaitForHost => "try again with the same key when the Mac can be reached".to_owned(),
        Fix::TokenWithCapability { capability } => format!(
            "make a new token that lists \"{capability}\" with `humata token create`; a token can't be widened"
        ),
        Fix::TokenForChannel { channel } => {
            format!("make a token for channel {channel} with `humata token create`")
        }
        Fix::NotFromAScript => "do it from the app or an agent session instead".to_owned(),
        Fix::MakeTheCall => "make the call itself; the user is asked then".to_owned(),
        Fix::UserStartsMachine { machine } => format!(
            "ask the user to start the sessions on {machine} from the app or with `humata start`, then make the call again; while they are stopped no agent or script is answered, so an agent can't ask with inbox_ask until then"
        ),
        Fix::Nothing => "nothing the caller can change makes this work".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn error_of(status: &Status) -> pb::Error {
        let rpc = RpcStatus::decode(status.details()).unwrap_or_else(|e| panic!("details: {e}"));
        let any = rpc
            .details
            .first()
            .unwrap_or_else(|| panic!("the status carries no details"));
        pb::Error::decode(any.value.as_slice()).unwrap_or_else(|e| panic!("error: {e}"))
    }

    /// D755's pull hint names the host as "This Mac", never by its id.
    #[test]
    fn the_pull_hint_names_the_host_as_this_mac_and_not_by_id() {
        let refusal = Refusal::SharedOnOtherMachine {
            machine: humata2_access::MachineId::new("ma_host"),
            here: humata2_access::MachineId::new("vm-a"),
        };
        let names = crate::upgrade::items::Names::of_host("ma_host");

        let error = error_of(&refused_named(&refusal, &names));

        assert!(error.message.contains("This Mac"), "{}", error.message);
        assert!(error.fix.contains("This Mac"), "{}", error.fix);
        assert!(!error.message.contains("ma_host"), "{}", error.message);
        assert!(!error.fix.contains("ma_host"), "{}", error.fix);
        assert!(error.message.contains("vm-a"), "{}", error.message);
    }

    /// The lock names how long is left and when it ends, not seconds since
    /// 1970, and still reports the exact end for clients.
    #[test]
    fn a_passphrase_lock_says_how_long_is_left_and_not_unix_seconds() {
        let until = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let now = until - Duration::from_secs(15 * 60);

        let error = error_of(&passphrase_locked_at("mac", until, now));

        assert!(error.fix.contains("15 minutes"), "{}", error.fix);
        assert!(!error.fix.contains("1700000000"), "{}", error.fix);
        assert!(!error.fix.contains("epoch"), "{}", error.fix);
        match error.kind {
            Some(pb::error::Kind::PassphraseLocked(locked)) => {
                assert_eq!(locked.until.map(|at| at.seconds), Some(1_700_000_000));
            }
            other => panic!("expected passphrase_locked, got {other:?}"),
        }
    }

    /// A cause the message already names, and one it doesn't.
    #[derive(Debug, thiserror::Error)]
    enum Told {
        #[error("could not fork the conversation: {source}; fork with the code only")]
        Names {
            #[source]
            source: std::io::Error,
        },
        #[error("could not fork the conversation")]
        Silent {
            #[source]
            source: std::io::Error,
        },
    }

    /// Each cause reads once in the chain, whether the message names it or not.
    #[test]
    fn a_chain_says_each_cause_once() {
        let cause = || std::io::Error::other("the disk is full");

        for told in [Told::Names { source: cause() }, Told::Silent { source: cause() }] {
            let text = chain(&told);
            assert_eq!(text.matches("the disk is full").count(), 1, "{text}");
        }
    }

    /// A lock that has just ended is not "0 minutes".
    #[test]
    fn a_passphrase_lock_that_has_ended_does_not_say_zero_minutes() {
        let until = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let error = error_of(&passphrase_locked_at("mac", until, until + Duration::from_secs(5)));

        assert!(!error.fix.contains(" 0 "), "{}", error.fix);
    }
}
