//! The Info panel's review switches (PR-8, PR-25): whether the agent may push
//! to the pull request's branch, and which of Focus, Notes and Order it may do
//! to the view. A switch moves at once; when the daemon refuses, it goes back
//! and the panel says why.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::model::SessionId;
use humata2_clients::pull_requests::{PullRequestError, PullRequestsClient};
use humata2_clients::review_chat::{AgentSwitches, ReviewChatClient};
use tca::{Effect, Reducer};

/// One of the agent's three switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Agent may move the view to a file and its lines.
    Focus,
    /// Agent may leave, edit and drop notes.
    Notes,
    /// Agent may reorder the files.
    Order,
}

fn set(switches: &mut AgentSwitches, which: Switch, on: bool) {
    match which {
        Switch::Focus => switches.focus = on,
        Switch::Notes => switches.notes = on,
        Switch::Order => switches.order = on,
    }
}

/// Why a switch didn't change.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum InfoError {
    /// The daemon refused to change whether the agent may push.
    #[error("The agent's permission to push wasn't changed, so the switch went back: {source}")]
    Push {
        /// What the daemon said.
        #[source]
        source: PullRequestError,
    },
    /// The daemon refused to change what the agent may do to the view.
    #[error("What the agent may do wasn't changed, so the switch went back: {source}")]
    Switches {
        /// What the daemon said.
        #[source]
        source: DaemonError,
    },
}

/// The switches' state, for the review session shown.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoState {
    /// The review the switches belong to.
    pub session: Option<SessionId>,
    /// Whether the agent may push to the PR branch. No client reads this back,
    /// so it starts off and follows what the person sets here.
    pub push_allowed: bool,
    /// What the agent may do to the view; all on until the daemon says
    /// otherwise.
    pub switches: AgentSwitches,
    /// The last change that failed.
    pub error: Option<InfoError>,
}

impl Default for InfoState {
    fn default() -> Self {
        Self {
            session: None,
            push_allowed: false,
            switches: AgentSwitches::default(),
            error: None,
        }
    }
}

/// What happens to the switches.
#[derive(Clone, Debug, PartialEq)]
pub enum InfoAction {
    /// A review session opened: show whether it may push, as the session
    /// says, and read its switches.
    Open {
        /// The review.
        session: SessionId,
        /// Whether the session lets the agent push now.
        push_allowed: bool,
    },
    /// The daemon answered the read.
    Loaded {
        /// The review asked about.
        session: SessionId,
        /// The switches, or why not.
        result: Result<AgentSwitches, DaemonError>,
    },
    /// Let the agent push, or stop it.
    SetPush(bool),
    /// The daemon answered a push change.
    PushSet {
        /// The review.
        session: SessionId,
        /// The value before the change, put back on failure.
        previous: bool,
        /// Whether it worked, or why not.
        result: Result<(), PullRequestError>,
    },
    /// Turn one of the agent's switches on or off.
    SetSwitch(Switch, bool),
    /// The daemon answered a switch change.
    SwitchesSet {
        /// The review.
        session: SessionId,
        /// The switches before the change, put back on failure.
        previous: AgentSwitches,
        /// The switches now, or why not.
        result: Result<AgentSwitches, DaemonError>,
    },
    /// Hide the error.
    DismissError,
}

/// The switches' reducer.
#[derive(Clone)]
pub struct Info {
    pull_requests: Arc<dyn PullRequestsClient>,
    chat: Arc<dyn ReviewChatClient>,
}

impl Info {
    /// Switches that change the review through these clients.
    #[must_use]
    pub fn new(pull_requests: Arc<dyn PullRequestsClient>, chat: Arc<dyn ReviewChatClient>) -> Self {
        Self { pull_requests, chat }
    }
}

impl Reducer for Info {
    type State = InfoState;
    type Action = InfoAction;

    fn reduce(&self, state: &mut InfoState, action: InfoAction) -> Effect<InfoAction> {
        match action {
            InfoAction::Open {
                session,
                push_allowed,
            } => {
                if state.session.as_ref() != Some(&session) {
                    *state = InfoState {
                        session: Some(session.clone()),
                        ..InfoState::default()
                    };
                }
                state.push_allowed = push_allowed;
                let chat = self.chat.clone();
                return Effect::future(async move {
                    let result = chat.context(&session).await.map(|context| context.switches);
                    InfoAction::Loaded { session, result }
                });
            }
            InfoAction::Loaded { session, result } => {
                if state.session.as_ref() == Some(&session) {
                    match result {
                        Ok(switches) => state.switches = switches,
                        Err(source) => state.error = Some(InfoError::Switches { source }),
                    }
                }
            }
            InfoAction::SetPush(allowed) => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                let previous = std::mem::replace(&mut state.push_allowed, allowed);
                state.error = None;
                let client = self.pull_requests.clone();
                return Effect::future(async move {
                    let result = client.set_push_allowed(&session, allowed).await;
                    InfoAction::PushSet { session, previous, result }
                });
            }
            InfoAction::PushSet { session, previous, result } => {
                if state.session.as_ref() == Some(&session)
                    && let Err(source) = result
                {
                    state.push_allowed = previous;
                    state.error = Some(InfoError::Push { source });
                }
            }
            InfoAction::SetSwitch(which, on) => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                let previous = state.switches;
                set(&mut state.switches, which, on);
                state.error = None;
                let wanted = state.switches;
                let chat = self.chat.clone();
                return Effect::future(async move {
                    let result = chat.set_switches(&session, wanted).await;
                    InfoAction::SwitchesSet { session, previous, result }
                });
            }
            InfoAction::SwitchesSet { session, previous, result } => {
                if state.session.as_ref() == Some(&session) {
                    match result {
                        Ok(switches) => state.switches = switches,
                        Err(source) => {
                            state.switches = previous;
                            state.error = Some(InfoError::Switches { source });
                        }
                    }
                }
            }
            InfoAction::DismissError => state.error = None,
        }
        Effect::none()
    }
}
