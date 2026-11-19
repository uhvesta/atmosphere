//! Read marks on the real daemon (CH-16, CH-17; D392): the session the app
//! shows is held viewed through MarkService.ViewSession, so its output
//! marks nothing and viewing it clears its mark. One view at a time: a new
//! one replaces the last, and viewing nothing ends it.

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::mark_service_client::MarkServiceClient;
use tokio::sync::oneshot;
use tonic::{Code, Status};

use crate::connection::Shared;

const WHAT: &str = "Marking the session read";

/// The view this client holds open. Dropping it ends the stream, and with
/// it the view on the daemon.
pub(crate) struct Viewing {
    session: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Viewing {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Shared {
    /// Views `session` on the daemon, or nothing. Returns once the daemon
    /// has cleared its mark. A session the daemon doesn't know has nothing
    /// to clear.
    pub(crate) async fn view(&self, session: Option<&SessionId>) -> Result<(), DaemonError> {
        let Some(session) = session else {
            let ended = self.state().viewing.take();
            drop(ended);
            return Ok(());
        };
        let id = session.as_str().to_owned();
        let held = self
            .state()
            .viewing
            .as_ref()
            .is_some_and(|viewing| viewing.session == id && !viewing.task.is_finished());
        if held {
            return Ok(());
        }
        let (opened, open) = oneshot::channel();
        let mut client = MarkServiceClient::new(self.wire.clone());
        let request = pb::ViewSessionRequest {
            session: Some(pb::SessionId { value: id.clone() }),
        };
        let task = self.spawn(async move {
            let mut stream = match client.view_session(request).await {
                Ok(response) => response.into_inner(),
                Err(status) => {
                    let _ = opened.send(Err(status));
                    return;
                }
            };
            match stream.message().await {
                Ok(Some(_)) => {
                    let _ = opened.send(Ok(()));
                }
                Ok(None) => {
                    let _ = opened.send(Err(Status::unavailable(
                        "the daemon ended the view before confirming it",
                    )));
                    return;
                }
                Err(status) => {
                    let _ = opened.send(Err(status));
                    return;
                }
            }
            // Held until the view is dropped: the daemon counts the session
            // viewed while the stream is open.
            while let Ok(Some(_)) = stream.message().await {}
        });
        let replaced = self.state().viewing.replace(Viewing { session: id, task });
        drop(replaced);
        match open.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(status)) if status.code() == Code::NotFound => Ok(()),
            Ok(Err(status)) => Err(self.error(&status, WHAT)),
            // A later view replaced this one before it was confirmed.
            Err(_) => Ok(()),
        }
    }
}
