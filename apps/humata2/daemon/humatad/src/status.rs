//! Failures as gRPC statuses: a code for generic handling and one
//! `humata.v1.Error` in the details, which clients branch on (GRPC.md §10).

use humata2_access::{Capability, Fix, PresenceMethod, Refusal, SessionId};
use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use prost::Message;
use tonic::codegen::Bytes;
use tonic::{Code, Status};

use crate::cli_login::LoginUnusable;
use crate::guard;

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
        fix: "update Humata on this machine; until then, do this from the app".to_owned(),
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
        text.push_str(": ");
        text.push_str(&cause.to_string());
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
            }),
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
        Refusal::NotItsFolder { .. } | Refusal::ReviewSessionPush { .. } => (
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

/// A call the channel's rules send to the Inbox, which this daemon can't
/// make for it yet.
pub(crate) fn needs_approval(capability: Capability) -> Status {
    let message = format!(
        "\"{capability}\" needs the user's approval here, and asking for it on this call isn't built into this humatad2 yet"
    );
    failure(
        Code::Unimplemented,
        pb::Error {
            message,
            fix: "ask the user with inbox_ask to do it themselves in the app; don't retry the call, since it is asked again each time".to_owned(),
            retry: None,
            kind: None,
        },
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
        Fix::Nothing => "nothing the caller can change makes this work".to_owned(),
    }
}
