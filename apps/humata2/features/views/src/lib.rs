//! Prepared views (VW-1–14): pages an agent offers to make for the person,
//! such as a status table. Nothing an agent prepares shows until the person
//! opts in with Open it; Later keeps the offer, Not now drops it, and an
//! opted-in view can be reverted (VW-9).

use std::collections::BTreeSet;
use std::sync::Arc;

use humata2_clients::{Workspace, model::SessionId};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// What the person decided about each offer, saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decisions {
    /// Offers opened, by title.
    pub accepted: BTreeSet<String>,
    /// Offers dropped, by title.
    pub dismissed: BTreeSet<String>,
}

/// The views page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewsState {
    /// Where offers come from.
    pub workspace: Arc<Workspace>,
    /// What was decided.
    pub decisions: Decisions,
    /// Offers hidden until the next visit to their session.
    pub later: BTreeSet<String>,
    /// The view showing, by title.
    pub showing: Option<String>,
}

impl ViewsState {
    /// Offers still waiting for a decision.
    #[must_use]
    pub fn offers(&self) -> Vec<&humata2_clients::model::PreparedView> {
        self.workspace
            .views
            .iter()
            .filter(|view| {
                !view.accepted
                    && !self.later.contains(&view.title)
                    && !self.decisions.accepted.contains(&view.title)
                    && !self.decisions.dismissed.contains(&view.title)
            })
            .collect()
    }

    /// Views opted in to.
    #[must_use]
    pub fn accepted(&self) -> Vec<&humata2_clients::model::PreparedView> {
        self.workspace
            .views
            .iter()
            .filter(|view| view.accepted || self.decisions.accepted.contains(&view.title))
            .collect()
    }
}

/// What happens to prepared views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewsAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved decisions came back at launch.
    Restore(Decisions),
    /// Open it: opt in and show the view.
    Accept(String),
    /// Hide this offer for the current visit.
    Later(String),
    /// Return to a session: show offers deferred there again.
    Return(SessionId),
    /// Not now: drop the offer.
    Dismiss(String),
    /// Show an opted-in view.
    Open(String),
    /// Close the view showing.
    Close,
    /// Undo opting in.
    Revert(String),
}

/// The views page's reducer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Views;

impl Reducer for Views {
    type State = ViewsState;
    type Action = ViewsAction;

    fn reduce(&self, state: &mut ViewsState, action: ViewsAction) -> Effect<ViewsAction> {
        match action {
            ViewsAction::Show(workspace) => state.workspace = workspace,
            ViewsAction::Restore(decisions) => state.decisions = decisions,
            ViewsAction::Accept(title) => {
                state.decisions.accepted.insert(title.clone());
                state.showing = Some(title);
            }
            ViewsAction::Later(title) => {
                state.later.insert(title);
            }
            ViewsAction::Return(session) => {
                for view in state
                    .workspace
                    .views
                    .iter()
                    .filter(|v| v.session == session)
                {
                    state.later.remove(&view.title);
                }
            }
            ViewsAction::Dismiss(title) => {
                state.decisions.dismissed.insert(title);
            }
            ViewsAction::Open(title) => state.showing = Some(title),
            ViewsAction::Close => state.showing = None,
            ViewsAction::Revert(title) => {
                state.decisions.accepted.remove(&title);
                if state.showing.as_ref() == Some(&title) {
                    state.showing = None;
                }
            }
        }
        Effect::none()
    }
}
