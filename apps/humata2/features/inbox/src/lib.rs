//! The Inbox (IN-1–10): what sessions ask the person, newest first. Each
//! request shows why it is asked, what it would do and what the agent was
//! doing, and is answered with Approve, Always…, Deny, or Deny and reply.
//! Approve all and Deny all ask first when something can't be undone
//! (IN-9).

use std::collections::BTreeSet;
use std::sync::Arc;

use humata2_clients::model::{Answer, ChannelId, Request, RequestId, SessionId};
use humata2_clients::{Clients, DaemonError, Workspace};
use tca::{Effect, Reducer};

/// Which requests show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// Every channel's.
    #[default]
    All,
    /// One channel's.
    Channel(ChannelId),
    /// One session's, as its side panel shows them.
    Session(SessionId),
}

/// A bulk answer waiting for confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bulk {
    /// Approve every shown request.
    ApproveAll,
    /// Deny every shown request.
    DenyAll,
}

/// Which answers are included in the wide Inbox list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    Waiting,
    Answered,
    All,
}

/// The Inbox's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InboxState {
    /// Where the requests come from.
    pub workspace: Arc<Workspace>,
    /// Which show.
    pub filter: Filter,
    /// Whether answered requests show too.
    pub show_answered: bool,
    /// Limit the history to answered requests.
    pub answered_only: bool,
    /// The request whose details show.
    pub selected: Option<RequestId>,
    /// A reply being written to deny with, and to which request.
    pub reply: Option<(RequestId, String)>,
    /// Answers on their way.
    pub pending: BTreeSet<RequestId>,
    /// The requests whose "Allow this for the rest of the session" box is ticked (IN-14).
    pub always: BTreeSet<RequestId>,
    /// A bulk answer waiting for "Are you sure?".
    pub confirm: Option<Bulk>,
    /// What went wrong last.
    pub error: Option<String>,
}

/// How many requests wait for an answer. The sidebar's Inbox badge and Home's
/// "Needs you" both show this count, so they cannot disagree.
#[must_use]
pub fn waiting(workspace: &Workspace) -> usize {
    workspace
        .requests
        .iter()
        .filter(|request| request.answer.is_none())
        .count()
}

impl InboxState {
    /// The requests that show, newest first.
    #[must_use]
    pub fn shown(&self) -> Vec<&Request> {
        self.workspace
            .requests
            .iter()
            .filter(|request| self.show_answered || request.answer.is_none())
            .filter(|request| !self.answered_only || request.answer.is_some())
            .filter(|request| match &self.filter {
                Filter::All => true,
                Filter::Channel(channel) => &request.channel == channel,
                Filter::Session(session) => &request.session == session,
            })
            .collect()
    }

    /// The selected request.
    #[must_use]
    pub fn selection(&self) -> Option<&Request> {
        let id = self.selected.as_ref()?;
        self.workspace
            .requests
            .iter()
            .find(|request| &request.id == id)
    }

    fn select_visible(&mut self) {
        let shown = self.shown();
        if !shown.iter().any(|r| Some(&r.id) == self.selected.as_ref()) {
            self.selected = shown.first().map(|r| r.id.clone());
        }
    }
}

/// What the Inbox asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open the session that asked (IN-4).
    OpenSession(SessionId),
    /// Open the session that asked this request and select the request (IN-11).
    OpenRequest {
        /// The session that asked.
        session: SessionId,
        /// The request to show there.
        request: RequestId,
    },
    /// Say this in a toast.
    Toast(String),
}

/// What happens in the Inbox.
#[derive(Clone, Debug, PartialEq)]
pub enum InboxAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Show these requests.
    SetFilter(Filter),
    /// Show or hide answered requests.
    ToggleAnswered,
    /// Pick Waiting, Answered, or All.
    SetHistory(History),
    /// Show a request's details.
    Select(RequestId),
    /// Answer a request.
    Answer(RequestId, Answer),
    /// Approve and make the request's rule for next time.
    Always(RequestId),
    /// Start writing a reply to deny with.
    StartReply(RequestId),
    /// The reply's text changed.
    ReplyText(String),
    /// Deny with the reply.
    SendReply,
    /// Drop the reply.
    CancelReply,
    /// Approve all or Deny all was clicked.
    Bulk(Bulk),
    /// "Are you sure?" was answered yes.
    ConfirmBulk,
    /// "Are you sure?" was answered no.
    CancelBulk,
    /// The daemon took an answer, or didn't.
    Answered(RequestId, Result<(), DaemonError>),
    /// Open the session that asked the selected request.
    OpenRequester,
    /// Tick or untick "Allow this for the rest of the session" on a request.
    ToggleAlways(RequestId),
    /// Answer a question with one of its choices.
    Choose(RequestId, String),
    /// Open the session that asked this request, with the request selected.
    OpenRequest(RequestId),
    /// For the parent.
    Delegate(Delegate),
}

/// The Inbox's reducer.
#[derive(Clone)]
pub struct Inbox {
    clients: Clients,
}

impl Inbox {
    /// An Inbox that answers with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }

    fn answer(&self, state: &mut InboxState, id: RequestId, answer: Answer) -> Effect<InboxAction> {
        if !state.pending.insert(id.clone()) {
            return Effect::none();
        }
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.answer(&id, answer).await;
            InboxAction::Answered(id, result)
        })
    }

    fn bulk(&self, state: &mut InboxState, bulk: Bulk) -> Effect<InboxAction> {
        let ids: Vec<_> = state
            .shown()
            .into_iter()
            .filter(|request| request.answer.is_none())
            .map(|request| request.id.clone())
            .collect();
        let answer = match bulk {
            Bulk::ApproveAll => Answer::Approved,
            Bulk::DenyAll => Answer::Denied { reply: None },
        };
        Effect::merge(
            ids.into_iter()
                .map(|id| self.answer(state, id, answer.clone()))
                .collect::<Vec<_>>(),
        )
    }
}

impl Reducer for Inbox {
    type State = InboxState;
    type Action = InboxAction;

    fn reduce(&self, state: &mut InboxState, action: InboxAction) -> Effect<InboxAction> {
        match action {
            InboxAction::Show(workspace) => {
                state.workspace = workspace;
                state.select_visible();
            }
            InboxAction::SetFilter(filter) => {
                state.filter = filter;
                state.select_visible();
            }
            InboxAction::ToggleAnswered => {
                state.show_answered = !state.show_answered;
                state.answered_only = false;
                state.select_visible();
            }
            InboxAction::SetHistory(history) => {
                state.show_answered = history != History::Waiting;
                state.answered_only = history == History::Answered;
                state.select_visible();
            }
            InboxAction::Select(id) => state.selected = Some(id),
            InboxAction::Answer(id, answer) => {
                let answer = match answer {
                    Answer::Approved if state.always.contains(&id) => {
                        match state.workspace.requests.iter().find(|r| r.id == id) {
                            Some(request) if !request.rule.is_empty() => Answer::Always {
                                rule: request.rule.clone(),
                            },
                            _ => Answer::Approved,
                        }
                    }
                    other => other,
                };
                return self.answer(state, id, answer);
            }
            InboxAction::ToggleAlways(id) => {
                let grantable = state
                    .workspace
                    .requests
                    .iter()
                    .any(|r| r.id == id && !r.rule.is_empty() && r.answer.is_none());
                if grantable && !state.always.remove(&id) {
                    state.always.insert(id);
                }
            }
            InboxAction::Choose(id, text) => {
                return self.answer(state, id, Answer::Replied { text });
            }
            InboxAction::OpenRequest(id) => {
                if let Some(request) = state.workspace.requests.iter().find(|r| r.id == id) {
                    return Effect::send(InboxAction::Delegate(Delegate::OpenRequest {
                        session: request.session.clone(),
                        request: id,
                    }));
                }
            }
            InboxAction::Always(id) => {
                let rule = state
                    .workspace
                    .requests
                    .iter()
                    .find(|request| request.id == id)
                    .map(|request| request.rule.clone())
                    .unwrap_or_default();
                return self.answer(state, id, Answer::Always { rule });
            }
            InboxAction::StartReply(id) => state.reply = Some((id, String::new())),
            InboxAction::ReplyText(text) => {
                if let Some(reply) = &mut state.reply {
                    reply.1 = text;
                }
            }
            InboxAction::SendReply => {
                if let Some((id, text)) = state.reply.take() {
                    let text = text.trim().to_owned();
                    let question = state.workspace.requests.iter().any(|request| {
                        request.id == id
                            && request.kind == humata2_clients::model::RequestKind::Question
                    });
                    let answer = if question {
                        Answer::Replied { text }
                    } else {
                        Answer::Denied {
                            reply: (!text.is_empty()).then_some(text),
                        }
                    };
                    return self.answer(state, id, answer);
                }
            }
            InboxAction::CancelReply => state.reply = None,
            InboxAction::Bulk(bulk) => {
                let risky = state
                    .shown()
                    .iter()
                    .any(|request| request.answer.is_none() && !request.undoable);
                if bulk == Bulk::ApproveAll && risky {
                    state.confirm = Some(bulk);
                } else {
                    return self.bulk(state, bulk);
                }
            }
            InboxAction::ConfirmBulk => {
                if let Some(bulk) = state.confirm.take() {
                    return self.bulk(state, bulk);
                }
            }
            InboxAction::CancelBulk => state.confirm = None,
            InboxAction::Answered(id, result) => {
                state.pending.remove(&id);
                state.always.remove(&id);
                if let Err(error) = result {
                    state.error = Some(error.to_string());
                } else if state.selected.as_ref() == Some(&id) && !state.show_answered {
                    state.selected = None;
                }
            }
            InboxAction::OpenRequester => {
                if let Some(request) = state.selection() {
                    return Effect::send(InboxAction::Delegate(Delegate::OpenSession(
                        request.session.clone(),
                    )));
                }
            }
            InboxAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
