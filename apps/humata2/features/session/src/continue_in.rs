//! Continue in another channel… (CV-8): a new top-level session in the
//! chosen channel, set up as that channel's New session, whose agent resumes
//! a copy of the conversation, from now or before an earlier turn (CV-2). The
//! session it came from is left as it is.

use humata2_clients::model::ChannelId;
use humata2_clients::{DaemonError, PastTurn, desk};
use tca::Effect;

use crate::{Delegate, SessionAction, SessionState, Sessions, open_session};

/// The Continue in another channel dialog (CV-8).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContinueDraft {
    /// The channel picked to continue in.
    pub channel: Option<ChannelId>,
    /// Continue from before this turn; `None` is from now.
    pub before_turn: Option<u32>,
    /// The session's turns, once the "Continue from" list asked for them.
    pub turns: Option<Vec<PastTurn>>,
    /// The "Continue from" list is open.
    pub picking_turn: bool,
    /// The request is on its way to the daemon.
    pub sending: bool,
}

/// Whether `channel` can take the open session's conversation: one of the
/// workspace's channels, other than the session's own and the Desk's.
fn can_take(state: &SessionState, channel: &ChannelId) -> bool {
    channel.as_str() != desk::FOLDER
        && open_session(state).is_some_and(|session| &session.channel != channel)
        && state.workspace.channel(channel).is_some()
}

impl Sessions {
    /// The Continue in another channel dialog's actions.
    pub(super) fn continue_in(
        &self,
        state: &mut SessionState,
        action: SessionAction,
    ) -> Effect<SessionAction> {
        match action {
            SessionAction::StartContinue => {
                // The Desk isn't a channel, and an archived session's
                // conversation is restored before it goes on anywhere.
                if open_session(state).is_some_and(|session| !desk::is_desk(session) && !session.archived) {
                    state.continuing = Some(ContinueDraft::default());
                    state.error = None;
                }
            }
            SessionAction::ContinuePick(channel) => {
                if can_take(state, &channel)
                    && let Some(draft) = state.continuing.as_mut().filter(|draft| !draft.sending)
                {
                    draft.channel = Some(channel);
                }
            }
            SessionAction::ContinueTurnPicker(open) => {
                let Some(id) = state.open.clone() else {
                    return Effect::none();
                };
                let Some(draft) = state.continuing.as_mut().filter(|draft| !draft.sending) else {
                    return Effect::none();
                };
                draft.picking_turn = open;
                if open && draft.turns.is_none() {
                    let daemon = self.clients.daemon.clone();
                    return Effect::future(async move {
                        SessionAction::ContinueTurns(daemon.list_turns(&id).await)
                    });
                }
            }
            SessionAction::ContinueTurns(result) => {
                if let Some(draft) = &mut state.continuing {
                    match result {
                        Ok(turns) => draft.turns = Some(turns),
                        // A daemon that can't list turns continues from now only.
                        Err(DaemonError::NotServedYet { .. }) => draft.turns = Some(Vec::new()),
                        Err(error) => state.error = Some(error.to_string()),
                    }
                }
            }
            SessionAction::ContinueBeforeTurn(turn) => {
                // Nothing comes before the first turn.
                if turn.is_some_and(|turn| turn < 2) {
                    return Effect::none();
                }
                if let Some(draft) = state.continuing.as_mut().filter(|draft| !draft.sending) {
                    draft.before_turn = turn;
                    draft.picking_turn = false;
                }
            }
            SessionAction::ConfirmContinue => {
                let (Some(id), Some(draft)) = (state.open.clone(), &mut state.continuing) else {
                    return Effect::none();
                };
                let Some(channel) = draft.channel.clone() else {
                    return Effect::none();
                };
                if draft.sending {
                    return Effect::none();
                }
                draft.sending = true;
                draft.picking_turn = false;
                let before_turn = draft.before_turn;
                state.error = None;
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    SessionAction::Continued(
                        daemon.continue_in_channel(&id, &channel, before_turn).await,
                    )
                });
            }
            SessionAction::CancelContinue => {
                if state.continuing.as_ref().is_some_and(|draft| !draft.sending) {
                    state.continuing = None;
                }
            }
            SessionAction::Continued(result) => match result {
                Ok(id) => {
                    state.continuing = None;
                    return Effect::send(SessionAction::Delegate(Delegate::OpenSession(id)));
                }
                Err(error) => {
                    // Shown in the dialog, which stays for another try.
                    if let Some(draft) = &mut state.continuing {
                        draft.sending = false;
                    }
                    state.error = Some(error.to_string());
                }
            },
            _ => {}
        }
        Effect::none()
    }
}
