//! Shared remote-session helpers (`elsewhere`).

use crate::pb;
use humata2_rpc_kit::failure;
use tonic::{Code, Status};

pub fn elsewhere(machine: &str, what: &str) -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!("{what}: session runs on {machine}, which is not this machine"),
            fix: "run this call against the host or the session's machine".to_owned(),
            retry: None,
            kind: None,
        },
    )
}
