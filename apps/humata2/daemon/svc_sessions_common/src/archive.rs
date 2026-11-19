//! Shared archive helpers (`PlanKind`, `failure_of`, `session_id`).

use crate::pb;
use humata2_rpc_kit::{missing, status_error_of};
use tonic::Status;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanKind {
    Archive,
    Delete,
    DropRepo,
    Push,
}

pub fn session_id(id: Option<pb::SessionId>) -> Result<String, Status> {
    id.map(|s| s.value)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| missing("id"))
}

pub fn failure_of(status: &Status) -> pb::Error {
    status_error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}
