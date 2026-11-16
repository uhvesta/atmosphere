//! An inbox of access requests. Each row answers its own request: the
//! answer is sent after a short window in which it can be undone. The
//! rows live in an `IdentifiedVec` and run through `for_each`, which keeps
//! each row's cancel ids apart, so answering one row never cancels
//! another's. Once a row's answer is sent, it tells the inbox through a
//! delegate action, and the inbox takes it off the list.
//!
//! Patterns: `IdentifiedVec` and `for_each`; per-row effects with one cancel
//! id that don't cancel each other; removing a row cancels its effect;
//! delegate actions from a row.

use std::sync::Arc;
use std::time::Duration;

use tca::{Clock, Effect, Identified, IdentifiedVec, Reducer, TestClock, TestStore, for_each};

/// How long an answer can be undone before it's sent.
const UNDO_WINDOW: Duration = Duration::from_secs(3);

/// Every row names its sending effect with this same id. That's safe only
/// because `for_each` keeps each row's ids within the row.
const SENDING: &str = "sending";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Approve,
    Decline,
}

#[derive(Clone, Debug, PartialEq)]
struct Request {
    id: RequestId,
    from: String,
    wants: String,
    /// The answer waiting out its undo window, if one was given.
    answering: Option<Answer>,
}

impl Identified for Request {
    type Id = RequestId;

    fn id(&self) -> RequestId {
        self.id
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RowAction {
    Answer(Answer),
    Undo,
    Sent(Answer),
    Delegate(RowDelegate),
}

#[derive(Clone, Debug, PartialEq)]
enum RowDelegate {
    /// The answer went out; the row is done.
    Answered(Answer),
}

/// One row. It knows nothing of the other rows or of the list.
struct Answering {
    clock: Arc<dyn Clock>,
}

impl Reducer for Answering {
    type State = Request;
    type Action = RowAction;

    fn reduce(&self, request: &mut Request, action: RowAction) -> Effect<RowAction> {
        match action {
            RowAction::Answer(answer) => {
                request.answering = Some(answer);
                let clock = self.clock.clone();
                // Cancelling in flight: changing the answer restarts the
                // window instead of sending both.
                Effect::future(async move {
                    clock.sleep(UNDO_WINDOW).await;
                    RowAction::Sent(answer)
                })
                .cancellable(SENDING, true)
            }
            RowAction::Undo => {
                request.answering = None;
                Effect::cancel(SENDING)
            }
            RowAction::Sent(answer) => {
                Effect::send(RowAction::Delegate(RowDelegate::Answered(answer)))
            }
            RowAction::Delegate(_) => Effect::none(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Inbox {
    requests: IdentifiedVec<Request>,
    /// Who was let in, for the "recently approved" line.
    approved: Vec<String>,
    declined: u32,
}

#[derive(Debug, PartialEq)]
enum InboxAction {
    Row(RequestId, RowAction),
    /// Swiped away without an answer.
    Remove(RequestId),
}

struct Inboxing {
    row: Answering,
}

/// The scope `for_each` keeps a row's cancel ids in: the row's id as
/// `Debug` shows it. The inbox needs it to cancel a row's effect from
/// outside the row, as when the row is removed.
fn row_scope(id: RequestId) -> String {
    format!("{id:?}")
}

impl Reducer for Inboxing {
    type State = Inbox;
    type Action = InboxAction;

    fn reduce(&self, inbox: &mut Inbox, action: InboxAction) -> Effect<InboxAction> {
        match action {
            InboxAction::Row(id, action) => {
                // Look at the delegate before handing the action over, then
                // run the row first and handle it after, so the row has had
                // its say before it's taken away.
                let answered = match &action {
                    RowAction::Delegate(RowDelegate::Answered(answer)) => Some(*answer),
                    _ => None,
                };
                let effect = for_each(&self.row, &mut inbox.requests, &id, action)
                    .map(move |action| InboxAction::Row(id, action));
                if let Some(answer) = answered {
                    if let Some(request) = inbox.requests.remove(&id) {
                        match answer {
                            Answer::Approve => inbox.approved.push(request.from),
                            Answer::Decline => inbox.declined += 1,
                        }
                    }
                }
                effect
            }
            InboxAction::Remove(id) => {
                inbox.requests.remove(&id);
                // The row is gone, but its effect isn't: cancel it by the
                // row's id within the row's scope, as `for_each` named it.
                // Without this the answer would still be sent.
                Effect::cancel(SENDING).scoped(&row_scope(id))
            }
        }
    }
}

fn request(id: u32, from: &str, wants: &str) -> Request {
    Request {
        id: RequestId(id),
        from: from.into(),
        wants: wants.into(),
        answering: None,
    }
}

fn inbox() -> (TestStore<Inboxing>, TestClock) {
    let clock = TestClock::new();
    let state = Inbox {
        requests: [
            request(1, "ana", "edit the roadmap"),
            request(2, "bo", "join the design channel"),
            request(3, "cy", "read the budget"),
        ]
        .into_iter()
        .collect(),
        ..Inbox::default()
    };
    let reducer = Inboxing {
        row: Answering {
            clock: Arc::new(clock.clone()),
        },
    };
    (TestStore::new(state, reducer), clock)
}

fn answering(store_state: &mut Inbox, id: u32, answer: Option<Answer>) {
    store_state
        .requests
        .update(&RequestId(id), |request| request.answering = answer);
}

#[test]
fn two_rows_answered_at_once_are_both_sent_and_both_leave_the_list() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::Row(RequestId(1), RowAction::Answer(Answer::Approve)), |s| {
        answering(s, 1, Some(Answer::Approve));
    });
    // Same cancel id, cancelling in flight, but another row: row 1's
    // answer keeps running.
    store.send(InboxAction::Row(RequestId(2), RowAction::Answer(Answer::Decline)), |s| {
        answering(s, 2, Some(Answer::Decline));
    });
    clock.advance(UNDO_WINDOW);
    store.receive(InboxAction::Row(RequestId(1), RowAction::Sent(Answer::Approve)), |_| {});
    store.receive(InboxAction::Row(RequestId(2), RowAction::Sent(Answer::Decline)), |_| {});
    store.receive(
        InboxAction::Row(RequestId(1), RowAction::Delegate(RowDelegate::Answered(Answer::Approve))),
        |s| {
            s.requests.remove(&RequestId(1));
            s.approved.push("ana".into());
        },
    );
    store.receive(
        InboxAction::Row(RequestId(2), RowAction::Delegate(RowDelegate::Answered(Answer::Decline))),
        |s| {
            s.requests.remove(&RequestId(2));
            s.declined = 1;
        },
    );
    assert_eq!(store.state().requests.ids().collect::<Vec<_>>(), [RequestId(3)]);
}

#[test]
fn undoing_one_row_leaves_the_others_answer_running() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::Row(RequestId(1), RowAction::Answer(Answer::Approve)), |s| {
        answering(s, 1, Some(Answer::Approve));
    });
    store.send(InboxAction::Row(RequestId(3), RowAction::Answer(Answer::Approve)), |s| {
        answering(s, 3, Some(Answer::Approve));
    });
    store.send(InboxAction::Row(RequestId(1), RowAction::Undo), |s| {
        answering(s, 1, None);
    });
    clock.advance(UNDO_WINDOW);
    store.receive(InboxAction::Row(RequestId(3), RowAction::Sent(Answer::Approve)), |_| {});
    store.receive(
        InboxAction::Row(RequestId(3), RowAction::Delegate(RowDelegate::Answered(Answer::Approve))),
        |s| {
            s.requests.remove(&RequestId(3));
            s.approved.push("cy".into());
        },
    );
    store.finish();
}

#[test]
fn changing_an_answer_restarts_its_window_and_sends_only_the_last() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::Row(RequestId(2), RowAction::Answer(Answer::Approve)), |s| {
        answering(s, 2, Some(Answer::Approve));
    });
    clock.advance(Duration::from_secs(2));
    store.send(InboxAction::Row(RequestId(2), RowAction::Answer(Answer::Decline)), |s| {
        answering(s, 2, Some(Answer::Decline));
    });
    // The first answer's window would have ended here.
    clock.advance(Duration::from_secs(1));
    store.send(InboxAction::Row(RequestId(1), RowAction::Undo), |_| {});
    clock.advance(Duration::from_secs(2));
    store.receive(InboxAction::Row(RequestId(2), RowAction::Sent(Answer::Decline)), |_| {});
    store.receive(
        InboxAction::Row(RequestId(2), RowAction::Delegate(RowDelegate::Answered(Answer::Decline))),
        |s| {
            s.requests.remove(&RequestId(2));
            s.declined = 1;
        },
    );
}

#[test]
fn removing_a_row_cancels_the_answer_it_was_about_to_send() {
    let (mut store, clock) = inbox();
    store.send(InboxAction::Row(RequestId(1), RowAction::Answer(Answer::Approve)), |s| {
        answering(s, 1, Some(Answer::Approve));
    });
    store.send(InboxAction::Row(RequestId(2), RowAction::Answer(Answer::Approve)), |s| {
        answering(s, 2, Some(Answer::Approve));
    });
    store.send(InboxAction::Remove(RequestId(1)), |s| {
        s.requests.remove(&RequestId(1));
    });
    clock.advance(UNDO_WINDOW);
    // Only row 2's answer goes out; nobody approved ana.
    store.receive(InboxAction::Row(RequestId(2), RowAction::Sent(Answer::Approve)), |_| {});
    store.receive(
        InboxAction::Row(RequestId(2), RowAction::Delegate(RowDelegate::Answered(Answer::Approve))),
        |s| {
            s.requests.remove(&RequestId(2));
            s.approved.push("bo".into());
        },
    );
    store.finish();
}

#[test]
fn an_action_for_a_row_no_longer_there_does_nothing() {
    let (mut store, _clock) = inbox();
    store.send(InboxAction::Remove(RequestId(3)), |s| {
        s.requests.remove(&RequestId(3));
    });
    store.send(InboxAction::Row(RequestId(3), RowAction::Answer(Answer::Approve)), |_| {});
    store.finish();
}
