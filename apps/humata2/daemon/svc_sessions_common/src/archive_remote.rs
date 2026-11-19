//! Shared remote-archive / remote-operation helpers (`finished`,
//! `follow_to_end`, `machine_failed`, `next_link_up`).

use std::future::Future;

use crate::pb;
use humata2_rpc_kit::failure;
use tonic::{Code, Status};

pub fn finished(state: i32) -> bool {
    matches!(
        pb::OperationState::try_from(state),
        Ok(pb::OperationState::Succeeded
            | pb::OperationState::Failed
            | pb::OperationState::Cancelled)
    )
}

pub fn machine_failed(machine: &str, what: &str, detail: &str) -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!("{what} on {machine} failed: {detail}"),
            fix: "check the machine's connection and try again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

pub async fn follow_to_end<F, Fut>(mut poll: F) -> Result<pb::Operation, Status>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<pb::Operation, Status>>,
{
    loop {
        let op = poll().await?;
        if finished(op.state) {
            return Ok(op);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub async fn next_link_up<F, Fut>(wait: F) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = bool>,
{
    wait().await
}
