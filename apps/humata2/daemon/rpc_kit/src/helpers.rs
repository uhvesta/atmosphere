//! Zero-service gRPC status, blocking, lock, timestamp, and idempotency-answer
//! helpers relocated into `humata2_rpc_kit` (SPLIT_PHASE3_DAEMON §2.1 Step 6a).

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_grpc::humata::v1 as pb;
use prost::Message;
use tonic::codegen::Bytes;
use tonic::{Code, Status};

pub use humata2_daemon_ports::{API_MAJOR, API_MINOR, Gated, Role, SOCKET_PATH_MAX};

/// Finished answers kept for retries with the same idempotency key.
pub const REMEMBERED_MAX: usize = 1024;

const ERROR_TYPE_URL: &str = "type.googleapis.com/humata.v1.Error";

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

/// Wraps a [`pb::Error`] inside a gRPC [`Status`] detail.
pub fn failure(code: Code, error: pb::Error) -> Status {
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

/// Builds a [`pb::Error`] with a message, fix, and kind.
pub fn error(
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

pub fn not_found(
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

pub fn invalid_field(
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

pub fn missing(field: &str) -> Status {
    invalid_field(
        field,
        pb::invalid_field::Problem::Missing,
        format!("the request has no {field}"),
        format!("set {field} and send it again"),
    )
}

/// A bug, or a failure the person can't fix from the request. The cause
/// goes to the daemon's log under an incident id the message names.
pub fn internal(message: impl Into<String>, log_path: &str) -> Status {
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
pub fn not_built(method: &str) -> Status {
    let message = format!("{method} isn't built into this humatad2 yet");
    let details = pb::Error {
        message: message.clone(),
        fix: "do this from the Humata app on the host Mac; this release of humatad2 doesn't do it here, so updating helps only once a newer release builds it".to_owned(),
        retry: None,
        kind: None,
    };
    failure(Code::Unimplemented, details)
}

/// The `humata.v1.Error` a status carries, if it has one.
pub fn status_error_of(status: &Status) -> Option<pb::Error> {
    let details = RpcStatus::decode(status.details()).ok()?;
    details
        .details
        .into_iter()
        .find(|any| any.type_url == ERROR_TYPE_URL)
        .and_then(|any| pb::Error::decode(any.value.as_slice()).ok())
}

/// An error and its causes, on one line.
pub fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let said = cause.to_string();
        if !text.contains(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        source = cause.source();
    }
    text
}

/// Runs store and file work off the async threads.
pub async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| internal(format!("a background task failed: {error}"), ""))
}

/// An error with its causes, said for the person who fixes it.
pub fn failed(
    code: Code,
    error: &dyn std::error::Error,
    fix: &str,
    kind: Option<pb::error::Kind>,
) -> Status {
    failure(code, error_of(error, fix, kind))
}

pub fn error_of(
    error: &dyn std::error::Error,
    fix: &str,
    kind: Option<pb::error::Kind>,
) -> pb::Error {
    pb::Error {
        message: chain(error),
        fix: fix.to_owned(),
        retry: None,
        kind,
    }
}

pub fn timestamp_ms(ms: i64) -> humata2_grpc::Timestamp {
    let since = UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or(0));
    let since = since.duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Answers kept by idempotency key, oldest forgotten first.
pub struct Answers<T> {
    inner: Mutex<(HashMap<String, T>, VecDeque<String>)>,
}

impl<T: Clone> Default for Answers<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> Answers<T> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    pub fn get(&self, key: &str) -> Option<T> {
        lock(&self.inner).0.get(key).cloned()
    }

    pub fn put(&self, key: String, answer: T) {
        let mut inner = lock(&self.inner);
        if inner.0.insert(key.clone(), answer).is_none() {
            inner.1.push_back(key);
        }
        while inner.1.len() > REMEMBERED_MAX {
            if let Some(oldest) = inner.1.pop_front() {
                inner.0.remove(&oldest);
            }
        }
    }
}

/// What a deleter and the deleted thing were, kept with a trash item.
#[derive(Clone, PartialEq, Message)]
struct CallerData {
    #[prost(message, optional, tag = "1")]
    deleted_by: Option<pb::Actor>,
    #[prost(message, optional, tag = "2")]
    was: Option<pb::EntityRef>,
}

/// `deleted_by` and `was` as the string the trash keeps with an item.
pub fn caller_data(deleted_by: &pb::Actor, was: &pb::EntityRef) -> String {
    let data = CallerData {
        deleted_by: Some(deleted_by.clone()),
        was: Some(was.clone()),
    };
    data.encode_to_vec()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What [`caller_data`] kept; nothing for data it can't read.
pub fn decode_caller_data(text: &str) -> (Option<pb::Actor>, Option<pb::EntityRef>) {
    let bytes: Option<Vec<u8>> = (text.len() % 2 == 0)
        .then(|| {
            (0..text.len())
                .step_by(2)
                .map(|at| {
                    text.get(at..at + 2)
                        .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                })
                .collect()
        })
        .flatten();
    match bytes.and_then(|bytes| CallerData::decode(bytes.as_slice()).ok()) {
        Some(data) => (data.deleted_by, data.was),
        None => (None, None),
    }
}

/// Formats an idempotency key scoped to `call`, `actor`, and `what`.
pub fn answer_key_for_actor(
    call: &str,
    actor: &pb::Actor,
    key: Option<&pb::IdempotencyKey>,
    what: &str,
) -> Option<String> {
    let key = key
        .map(|key| key.value.as_str())
        .filter(|key| !key.is_empty())?;
    Some(format!("{call}\0{actor:?}\0{what}\0{key}"))
}
