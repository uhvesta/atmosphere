//! Prepared views (VW-1 to VW-14, P7): pages an agent offers to make for the
//! person. Nothing an agent prepares shows until the person chooses Open it;
//! Later hides the offer until they come back to its session, Not now drops
//! it, and a view opened can be reverted, kept or closed.
//!
//! The daemon keeps every view's state. This page holds a copy the daemon's
//! watch keeps current: nothing polls, and a watch that ends starts again.
//! Each answer changes the copy at once and sends one `set_state`; if the
//! daemon refuses it, the copy goes back to what it was.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::model::{SessionId, ViewId, ViewState};
use humata2_clients::views::{ViewContent, ViewRecord, ViewsClient};
use tca::{Effect, Reducer};

/// The view opened in this run, while its bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Showing {
    /// The view.
    pub id: ViewId,
}

/// The views page's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewsState {
    /// Each view as the daemon last said, or as the person just answered it.
    /// A record with a lower revision than the one held is ignored.
    pub records: BTreeMap<ViewId, ViewRecord>,
    /// Views put off with Later during this visit.
    pub deferred: BTreeSet<ViewId>,
    /// The view opened in this run, while its bar shows.
    pub showing: Option<Showing>,
    /// The last answer that failed: what failed and what to do.
    pub error: Option<String>,
    /// Whether the offers were asked for and the watch is open or opening.
    pub watching: bool,
    /// Ids, newest first.
    order: Vec<ViewId>,
}

impl ViewsState {
    /// The offers waiting in `session`: waiting, or put off but not during
    /// this visit. Newest first.
    #[must_use]
    pub fn offers(&self, session: &SessionId) -> Vec<&ViewRecord> {
        self.newest_first()
            .filter(|record| &record.view.session == session)
            .filter(|record| match record.state {
                ViewState::Waiting => true,
                ViewState::Later => !self.deferred.contains(&record.id),
                _ => false,
            })
            .collect()
    }

    /// Every view of `family`'s sessions in any state but Draft, newest
    /// first.
    #[must_use]
    pub fn journal(&self, family: &[SessionId]) -> Vec<&ViewRecord> {
        self.newest_first()
            .filter(|record| record.state != ViewState::Draft)
            .filter(|record| family.contains(&record.view.session))
            .collect()
    }

    /// The view `id`.
    #[must_use]
    pub fn record(&self, id: &ViewId) -> Option<&ViewRecord> {
        self.records.get(id)
    }

    /// Tells the parent to show the notes again when `changed` is the
    /// content of the view showing (RV-34), whichever answer brought it: the
    /// open's own answer, the watch or a list.
    fn plan_read(&self, id: &ViewId, changed: bool) -> Effect<ViewsAction> {
        if changed && self.showing.as_ref().is_some_and(|showing| &showing.id == id) {
            Effect::send(ViewsAction::Delegate(Delegate::PlanRead(id.clone())))
        } else {
            Effect::none()
        }
    }

    fn newest_first(&self) -> impl Iterator<Item = &ViewRecord> {
        self.order.iter().filter_map(|id| self.records.get(id))
    }

    /// Takes `record` unless one with a higher revision is held. At the
    /// same revision only its content is taken: the daemon reads a review
    /// plan's notes against the files each time it lists, so they change
    /// with no revision. Answers whether that changed the content held, at
    /// the same revision or a newer one. A
    /// view not known yet goes to the front when `new` is true (it was just
    /// offered), else to the back (it was read from a list).
    fn merge(&mut self, record: ViewRecord, new: bool) -> bool {
        match self.records.get_mut(&record.id) {
            Some(held) if held.revision == record.revision => {
                let changed = held.content != record.content;
                if changed {
                    held.content = record.content;
                }
                return changed;
            }
            Some(held) if held.revision > record.revision => {}
            Some(held) => {
                let changed = held.content != record.content;
                *held = record;
                return changed;
            }
            None => {
                if new {
                    self.order.insert(0, record.id.clone());
                } else {
                    self.order.push(record.id.clone());
                }
                self.records.insert(record.id.clone(), record);
            }
        }
        false
    }
}

/// What happens to prepared views.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewsAction {
    /// Read the offers, then follow the daemon's watch.
    Start,
    /// The offers, or a session's views, were read.
    Listed(Result<Vec<ViewRecord>, DaemonError>),
    /// A view was offered or answered, on the watch.
    Changed(ViewRecord),
    /// The watch ended: it starts again.
    WatchEnded,
    /// Open it: the view shows and the app is asked to open it.
    Open(ViewId),
    /// Put this offer off until the person comes back to its session.
    Later(ViewId),
    /// Not now: drop the offer.
    NotNow(ViewId),
    /// The person arrived at a session: its put-off offers show again, and
    /// its views are read, which includes those turned down.
    Return(SessionId),
    /// The view under the one just reverted, kept or closed shows again.
    /// Nothing goes to the daemon.
    Resume(ViewId),
    /// Put back what the view showing changed.
    Revert,
    /// Keep what the view showing changed.
    Keep,
    /// Close the view showing.
    Close,
    /// The walkthrough `id` goes to step `to` (the app owns the step).
    Step {
        /// The view.
        id: ViewId,
        /// The step wanted, counted from 0.
        to: usize,
    },
    /// The daemon answered a change of state.
    Answered {
        /// The view as it was before the answer.
        prior: ViewRecord,
        /// The view after the change, or why it was refused.
        result: Result<ViewRecord, DaemonError>,
    },
    /// Hide the error.
    DismissError,
    /// For the parent.
    Delegate(Delegate),
}

/// What the views page asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Show this view: take what the person has now, then open it.
    Opening(ViewId),
    /// Put back what the person had before this view opened.
    Reverting(ViewId),
    /// The review plan showing was read again and its notes changed (they
    /// are read against the files each time): show the new ones.
    PlanRead(ViewId),
}

/// The views page's reducer.
#[derive(Clone)]
pub struct Views {
    client: Arc<dyn ViewsClient>,
}

impl Views {
    /// A reducer that asks `client`.
    #[must_use]
    pub fn new(client: Arc<dyn ViewsClient>) -> Self {
        Self { client }
    }

    /// Opens the watch, reads the offers, then follows the watch. The
    /// stream is open before the read, so no change falls between them.
    fn start(&self) -> Effect<ViewsAction> {
        let client = self.client.clone();
        Effect::run(move |send| async move {
            let watch = client.watch().await;
            let listed = client.list_offered().await;
            send.send(ViewsAction::Listed(listed));
            match watch {
                Err(error) => send.send(ViewsAction::Listed(Err(error))),
                Ok(mut events) => {
                    while let Some(record) = events.recv().await {
                        send.ready().await;
                        send.send(ViewsAction::Changed(record));
                    }
                    send.send(ViewsAction::WatchEnded);
                }
            }
        })
        .cancellable("views-watch", true)
    }

    /// Moves `id` to `to` at once and asks the daemon to. `None` when the
    /// view is not known.
    fn answer(
        &self,
        state: &mut ViewsState,
        id: &ViewId,
        to: ViewState,
    ) -> Option<Effect<ViewsAction>> {
        let record = state.records.get_mut(id)?;
        let prior = record.clone();
        record.state = to;
        let client = self.client.clone();
        let id = id.clone();
        // Opening a review plan reads its session's views again once the
        // answer is in, for the notes as the files are now (RV-34).
        if to == ViewState::Opened && matches!(prior.content, ViewContent::ReviewPlan(_)) {
            let session = prior.view.session.clone();
            return Some(Effect::run(move |send| async move {
                let result = client.set_state(&id, to).await;
                send.send(ViewsAction::Answered { prior, result });
                send.send(ViewsAction::Listed(client.list(&session).await));
            }));
        }
        Some(Effect::future(async move {
            let result = client.set_state(&id, to).await;
            ViewsAction::Answered { prior, result }
        }))
    }
}

impl Reducer for Views {
    type State = ViewsState;
    type Action = ViewsAction;

    fn reduce(&self, state: &mut ViewsState, action: ViewsAction) -> Effect<ViewsAction> {
        match action {
            // A watch that ends starts again: push, never poll.
            ViewsAction::Start | ViewsAction::WatchEnded => {
                state.watching = true;
                return self.start();
            }
            ViewsAction::Listed(Ok(records)) => {
                let mut read = Vec::new();
                for record in records {
                    let id = record.id.clone();
                    let changed = state.merge(record, false);
                    read.push(state.plan_read(&id, changed));
                }
                return Effect::merge(read);
            }
            ViewsAction::Listed(Err(error)) => {
                state.watching = false;
                state.error = Some(format!(
                    "Humata could not read the views your agents prepared ({error}). Check that the daemon is running; they are read again when you come back to a session or reopen the app."
                ));
            }
            ViewsAction::Changed(record) => {
                let id = record.id.clone();
                let changed = state.merge(record, true);
                return state.plan_read(&id, changed);
            }
            ViewsAction::Open(id) => {
                state.deferred.remove(&id);
                let Some(answer) = self.answer(state, &id, ViewState::Opened) else {
                    return Effect::none();
                };
                state.showing = Some(Showing { id: id.clone() });
                return Effect::merge([
                    answer,
                    Effect::send(ViewsAction::Delegate(Delegate::Opening(id))),
                ]);
            }
            ViewsAction::Later(id) => {
                if let Some(answer) = self.answer(state, &id, ViewState::Later) {
                    state.deferred.insert(id);
                    return answer;
                }
            }
            ViewsAction::NotNow(id) => {
                if let Some(answer) = self.answer(state, &id, ViewState::NotNow) {
                    return answer;
                }
            }
            ViewsAction::Return(session) => {
                let records = &state.records;
                state.deferred.retain(|id| {
                    records
                        .get(id)
                        .is_none_or(|record| record.view.session != session)
                });
                let client = self.client.clone();
                return Effect::future(async move {
                    ViewsAction::Listed(client.list(&session).await)
                });
            }
            ViewsAction::Resume(id) => {
                if state.records.contains_key(&id) {
                    state.showing = Some(Showing { id });
                }
            }
            ViewsAction::Revert => {
                let Some(showing) = state.showing.take() else {
                    return Effect::none();
                };
                let answer = self.answer(state, &showing.id, ViewState::Reverted);
                let ask = Effect::send(ViewsAction::Delegate(Delegate::Reverting(showing.id)));
                return Effect::merge(answer.into_iter().chain([ask]));
            }
            ViewsAction::Keep => {
                if let Some(showing) = state.showing.take()
                    && let Some(answer) = self.answer(state, &showing.id, ViewState::Kept)
                {
                    return answer;
                }
            }
            ViewsAction::Close => {
                if let Some(showing) = state.showing.take()
                    && let Some(answer) = self.answer(state, &showing.id, ViewState::OpenedAndClosed)
                {
                    return answer;
                }
            }
            ViewsAction::Answered { prior, result } => match result {
                Ok(record) => {
                    let id = record.id.clone();
                    let changed = state.merge(record, false);
                    return state.plan_read(&id, changed);
                }
                Err(error) => {
                    let title = prior.view.title.clone();
                    if prior.state != ViewState::Later {
                        state.deferred.remove(&prior.id);
                    }
                    // A newer change that arrived meanwhile stands.
                    if state
                        .records
                        .get(&prior.id)
                        .is_none_or(|held| held.revision <= prior.revision)
                    {
                        state.records.insert(prior.id.clone(), prior);
                    }
                    state.error = Some(format!(
                        "The daemon did not take your answer to “{title}” ({error}), so the view is back as it was. Check that the daemon is running, then answer again."
                    ));
                }
            },
            ViewsAction::DismissError => state.error = None,
            ViewsAction::Step { .. } | ViewsAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
