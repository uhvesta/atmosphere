//! Failures as gRPC statuses: a code for generic handling and one
//! `humata.v1.Error` in the details, which clients branch on (GRPC.md §10).

use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use prost::Message;
use tonic::codegen::Bytes;
use tonic::{Code, Status};

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

fn failure(code: Code, error: pb::Error) -> Status {
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

fn error(message: impl Into<String>, fix: impl Into<String>, kind: pb::error::Kind) -> pb::Error {
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
    eprintln!("humatad2: incident {incident}: {message}");
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
