//! Transfer a review batch into the session queue in one parent reduction.
use crate::{App, AppAction, AppState};
use humata2_clients::{
    DaemonError,
    model::{Comment, SessionId},
};
use humata2_review::ReviewAction;
use humata2_session::SessionAction;
use tca::Effect;

impl App {
    pub(super) fn queue_comments(
        &self,
        state: &mut AppState,
        session: SessionId,
        comments: Vec<Comment>,
    ) -> Effect<AppAction> {
        if state.session.workspace.session(&session).is_none() {
            return self.child(
                state,
                AppAction::Review(ReviewAction::QueueFailed(DaemonError::NotFound {
                    what: format!("Session {session}; reopen its channel and try sending again"),
                })),
            );
        }
        let enqueue = self.child(
            state,
            AppAction::Session(SessionAction::QueueComments {
                session: session.clone(),
                comments: comments.clone(),
            }),
        );
        let retained = self.child(
            state,
            AppAction::Review(ReviewAction::Queued(session, comments)),
        );
        Effect::merge([enqueue, retained])
    }
}
