//! Stopping an agent saved as launching (SAFE, after the end-holder fix): a
//! record left launching by a daemon that stopped, whose old holder may
//! still run the agent. Stopped is saved only once that holder is gone, so
//! no second agent can run beside it.

use std::future::Future;

use tonic::{Code, Status};

use crate::channel_service::failed;
use crate::holder_end::HolderEndError;

#[derive(Debug, thiserror::Error)]
pub(super) enum StopLaunchingError {
    /// This daemon is starting the agent right now.
    #[error("session {session}'s agent is being started right now")]
    Live { session: String },

    #[error("session {session} was left starting and its old terminal couldn't be ended, so it stays starting")]
    Holder {
        session: String,
        #[source]
        source: HolderEndError,
    },
}

/// Ends the old holder of `session`'s launching agent with `end`, unless
/// this daemon's own start of it is `live`: that one finishes or fails by
/// itself, and ending its holder under it would race it. Ok means the holder
/// is gone (or none was found), so the agent may be saved stopped; an error
/// means it may still run and the record must stay as it is.
pub(super) async fn end_old_holder<F, Fut>(
    session: &str,
    live: bool,
    end: F,
) -> Result<(), StopLaunchingError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), HolderEndError>>,
{
    if live {
        return Err(StopLaunchingError::Live {
            session: session.to_owned(),
        });
    }
    end().await.map_err(|source| StopLaunchingError::Holder {
        session: session.to_owned(),
        source,
    })
}

/// The status a refused or failed stop of a launching agent answers with.
pub(super) fn status_of(session: &str, error: &StopLaunchingError) -> Status {
    match error {
        StopLaunchingError::Live { .. } => super::busy(session, "still starting"),
        StopLaunchingError::Holder { .. } => failed(
            Code::FailedPrecondition,
            error,
            "stop it again; if that fails too, end the terminal's process by hand as the message says, then stop it again. It stays starting until then, so nothing can start a second agent beside it",
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use humata2_holder::EndError;

    use super::{StopLaunchingError, end_old_holder};
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
    async fn a_launching_agent_with_no_live_start_has_its_holder_ended_and_may_then_be_stopped() {
        let ended = Cell::new(0);
        let outcome = end_old_holder("se_a", false, || async {
            ended.set(ended.get() + 1);
            Ok(())
        })
        .await;

        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(ended.get(), 1);
    }

    #[tokio::test]
    async fn a_holder_that_cannot_be_ended_is_an_error_so_the_agent_is_not_saved_stopped() {
        let outcome = end_old_holder("se_a", false, || async { Err(survived()) }).await;

        assert!(matches!(outcome, Err(StopLaunchingError::Holder { .. })), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_live_start_is_refused_and_its_holder_is_left_alone() {
        let ended = Cell::new(0);
        let outcome = end_old_holder("se_a", true, || async {
            ended.set(ended.get() + 1);
            Ok(())
        })
        .await;

        assert!(matches!(outcome, Err(StopLaunchingError::Live { .. })), "{outcome:?}");
        assert_eq!(ended.get(), 0, "no end was tried");
    }
}
