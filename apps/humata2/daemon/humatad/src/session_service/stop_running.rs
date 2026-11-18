//! Stopping a running agent (SAFE): Stopped is saved only once its holder is
//! gone, so no second agent can start beside one that may still run.

use std::future::Future;

use tonic::{Code, Status};

use crate::channel_service::failed;
use crate::holder_end::HolderEndError;

#[derive(Debug, thiserror::Error)]
pub(super) enum StopRunningError {
    #[error("session {session}'s terminal couldn't be ended, so its agent stays running")]
    Holder {
        session: String,
        #[source]
        source: HolderEndError,
    },
}

/// Ends the holder of `session`'s running agent with `end`. Ok means it is
/// gone, so the agent may be saved stopped; an error means it may still run
/// and the record must stay as it is.
pub(super) async fn end_running_holder<F, Fut>(
    session: &str,
    end: F,
) -> Result<(), StopRunningError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), HolderEndError>>,
{
    end().await.map_err(|source| StopRunningError::Holder {
        session: session.to_owned(),
        source,
    })
}

/// The status a stop of a running agent answers with when its holder stayed.
pub(super) fn status_of(_session: &str, error: &StopRunningError) -> Status {
    match error {
        StopRunningError::Holder { .. } => failed(
            Code::FailedPrecondition,
            error,
            "stop it again; if that fails too, end the terminal's process by hand as the message says, then stop it again. It stays running until then, so nothing can start a second agent beside it",
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use humata2_holder::EndError;
    use tonic::Code;

    use super::{StopRunningError, end_running_holder, status_of};
    use crate::holder_end::HolderEndError;

    fn survived() -> HolderEndError {
        HolderEndError::EndByPid {
            owner: "session se_a's terminal".to_owned(),
            socket: PathBuf::from("/nowhere/h.sock"),
            pid: 1,
            source: EndError::Survived {
                socket: PathBuf::from("/nowhere/h.sock"),
                pid: 1,
            },
        }
    }

    #[tokio::test]
    async fn a_holder_that_ended_lets_the_agent_be_saved_stopped() {
        let ended = Cell::new(0);
        let outcome = end_running_holder("se_a", || async {
            ended.set(ended.get() + 1);
            Ok(())
        })
        .await;

        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(ended.get(), 1);
    }

    #[tokio::test]
    async fn a_holder_that_cannot_be_ended_is_an_error_so_the_agent_is_not_saved_stopped() {
        let outcome = end_running_holder("se_a", || async { Err(survived()) }).await;

        assert!(matches!(outcome, Err(StopRunningError::Holder { .. })), "{outcome:?}");
    }

    #[tokio::test]
    async fn the_failed_stop_is_a_failed_precondition() {
        let error = end_running_holder("se_a", || async { Err(survived()) })
            .await
            .expect_err("the holder survived");

        assert_eq!(status_of("se_a", &error).code(), Code::FailedPrecondition);
    }
}
