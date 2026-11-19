//! The chat follower (PR-22 to PR-26, D59, D88, D286): the review agent moves
//! the view and leaves notes, and this applies each change to what the
//! person sees, but only once they have been quiet for a moment. Scrolling,
//! typing or selecting resets the wait, a drag blocks it until the drag ends,
//! and focus moves that queued up collapse to the latest one. Each applied
//! change can be undone, and Back returns to where the person was. A change
//! the code made (`diff`, D88) shows without Undo.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::comments::CommentId;
use humata2_clients::model::SessionId;
use humata2_clients::review_chat::{
    Highlight, Note, Place, ReviewChange, ReviewChangeKind, ReviewChatClient, ReviewContext,
};
use humata2_clients::views::PlanView;
use tca::{Clock, Effect, Reducer};

/// How long the person must be still before the agent's changes show.
pub const QUIET: Duration = Duration::from_millis(1000);

const WATCH: &str = "pr-review-follow";
const WAIT: &str = "pr-review-quiet";

/// What the person did that the follower waits out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// Scrolled.
    Scroll,
    /// Pressed a key.
    Key,
    /// Changed the selection.
    Selection,
}

/// What an applied change did, so Undo can put it back.
#[derive(Clone, Debug, PartialEq)]
enum Reverse {
    /// Nothing to put back.
    Nothing,
    /// The view was somewhere else, with other highlights.
    View {
        before: Option<Place>,
        focus: Option<Focus>,
        highlights: Vec<Highlight>,
    },
    /// A note appeared.
    RemoveNote(CommentId),
    /// A note was dropped.
    RestoreNote(Note),
    /// The plan was different.
    Plan(Option<PlanView>),
    /// The stack showed another layer (`None` is Whole stack).
    Layer(Option<String>),
}

/// Where the agent last focused the view, as long as the person hasn't moved
/// on themselves. The Diff panel marks its line and scrolls to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Focus {
    /// The place the agent focused.
    pub place: Place,
    /// The change that focused it: a new one scrolls again.
    pub seq: u64,
}

impl Focus {
    /// The line the focus is on: the selection's first, else the first on
    /// screen.
    #[must_use]
    pub fn line(&self) -> Option<u32> {
        self.place
            .selection
            .or(self.place.visible)
            .map(|span| span.first)
    }
}

/// A change shown to the person.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    /// The change, as the daemon sent it.
    pub change: ReviewChange,
    /// Whether Undo took it back.
    pub undone: bool,
    reverse: Reverse,
}

impl Applied {
    /// Whether the chat line offers Undo: the change has an undo id (a
    /// `diff` change doesn't, D88) and wasn't undone yet.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undone && !self.change.undo_id.is_empty()
    }
}

/// The follower's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FollowerState {
    /// The review being followed.
    pub session: Option<SessionId>,
    /// What is on screen: the person's place, or the agent's after a move.
    pub shown: Option<Place>,
    /// The agent's focus, while it stands.
    pub focus: Option<Focus>,
    /// What the agent highlighted.
    pub highlights: Vec<Highlight>,
    /// Where the person was before the agent moved the view; Back returns
    /// there. Empty once the person moves themselves.
    pub back: Option<Place>,
    /// The agent's notes as shown, outdated ones included.
    pub notes: Vec<Note>,
    /// The plan of how to review, once the agent prepared one.
    pub plan: Option<PlanView>,
    /// The stack layer the agent has the view on, by title; `None` is
    /// Whole stack (D800).
    pub layer: Option<String>,
    /// The changes shown, oldest first.
    pub applied: Vec<Applied>,
    /// The last thing that failed, with what to do.
    pub error: Option<DaemonError>,
    seen: u64,
    queued: Vec<ReviewChange>,
    dragging: bool,
    last_activity: Option<Duration>,
}

impl FollowerState {
    /// Changes that arrived and wait for the person to be still.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.queued.len()
    }

    /// How much longer the person must be still, at `now`; zero when they
    /// have been.
    fn remaining(&self, now: Duration) -> Duration {
        match self.last_activity {
            Some(at) => QUIET.saturating_sub(now.saturating_sub(at)),
            None => Duration::ZERO,
        }
    }
}

/// What happens to the follower.
#[derive(Clone, Debug, PartialEq)]
pub enum FollowerAction {
    /// Follow a review's changes, as they are pushed.
    Follow(SessionId),
    /// The agent changed the view.
    Changed(ReviewChange),
    /// The feed ended; it is followed again from where it stopped.
    WatchEnded,
    /// The feed couldn't be followed.
    WatchFailed(DaemonError),
    /// The person scrolled, typed or selected.
    Interacted(Interaction),
    /// A drag started: nothing applies until it ends.
    DragStarted,
    /// The drag ended.
    DragEnded,
    /// The person went to a place themselves.
    UserWentTo(Place),
    /// The wait for stillness ran out.
    Quiet,
    /// Undo the change with this undo id.
    Undo(String),
    /// The daemon answered Undo.
    Undone {
        /// The undo id that was asked for.
        undo_id: String,
        /// The change that took it back, or why it was refused.
        result: Result<ReviewChange, DaemonError>,
    },
    /// Return to where the person was before the agent moved the view.
    Back,
    /// The notes were read after the code moved lines.
    ContextRead(Result<ReviewContext, DaemonError>),
    /// Hide the error.
    DismissError,
    /// For the parent.
    Delegate(Delegate),
}

/// What the follower asks its parent to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Delegate {
    /// Scroll the review to this place.
    Show(Place),
    /// Show this stack layer in the review; `None` is Whole stack.
    ShowLayer(Option<String>),
}

/// The follower's reducer.
#[derive(Clone)]
pub struct Follower {
    client: Arc<dyn ReviewChatClient>,
    clock: Arc<dyn Clock>,
}

impl Follower {
    /// A follower that asks `client` and waits on `clock`.
    #[must_use]
    pub fn new(client: Arc<dyn ReviewChatClient>, clock: Arc<dyn Clock>) -> Self {
        Self { client, clock }
    }

    fn watch(&self, session: SessionId, after: u64) -> Effect<FollowerAction> {
        let client = self.client.clone();
        Effect::run(move |send| async move {
            match client.watch_changes(&session, after).await {
                Err(error) => send.send(FollowerAction::WatchFailed(error)),
                Ok(mut changes) => {
                    while let Some(change) = changes.recv().await {
                        send.ready().await;
                        send.send(FollowerAction::Changed(change));
                    }
                    send.send(FollowerAction::WatchEnded);
                }
            }
        })
        .cancellable(WATCH, true)
    }

    /// After something the person did, or a change arriving: applies what
    /// waits if they have been still long enough, else waits for the rest of
    /// the time, replacing any wait running.
    fn settle(&self, state: &mut FollowerState) -> Effect<FollowerAction> {
        if state.queued.is_empty() {
            return Effect::cancel(WAIT);
        }
        if state.dragging {
            return Effect::cancel(WAIT);
        }
        let remaining = state.remaining(self.clock.now());
        if remaining.is_zero() {
            return Effect::cancel(WAIT).and(self.apply_queue(state));
        }
        let clock = self.clock.clone();
        Effect::future(async move {
            clock.sleep(remaining).await;
            FollowerAction::Quiet
        })
        .cancellable(WAIT, true)
    }

    fn touch(&self, state: &mut FollowerState) {
        state.last_activity = Some(self.clock.now());
    }

    /// Applies everything queued, in order; only the latest focus move is
    /// applied, since the person would only see the view jump through the
    /// earlier ones.
    fn apply_queue(&self, state: &mut FollowerState) -> Effect<FollowerAction> {
        let queued = std::mem::take(&mut state.queued);
        let is_focus = |change: &ReviewChange| matches!(change.change, Some(ReviewChangeKind::Focus(_)));
        let last_focus = queued.iter().rposition(is_focus);
        let mut effects = Vec::new();
        for (index, change) in queued.into_iter().enumerate() {
            if is_focus(&change) && Some(index) != last_focus {
                continue;
            }
            effects.push(self.apply(state, change));
        }
        Effect::merge(effects)
    }

    fn apply(&self, state: &mut FollowerState, change: ReviewChange) -> Effect<FollowerAction> {
        if change.tool == "undo" {
            return self.revert(state, &change.undid);
        }
        let mut effect = Effect::none();
        let reverse = match change.change.clone() {
            Some(ReviewChangeKind::Focus(place)) => {
                let before = state.shown.replace(place.clone());
                if state.back.is_none() {
                    state.back = before.clone();
                }
                let focus = state.focus.replace(Focus {
                    place: place.clone(),
                    seq: change.seq,
                });
                let highlights = std::mem::replace(&mut state.highlights, change.highlights.clone());
                effect = Effect::send(FollowerAction::Delegate(Delegate::Show(place)));
                Reverse::View { before, focus, highlights }
            }
            Some(ReviewChangeKind::Note(note)) => {
                let id = note.id.clone();
                state.notes.retain(|held| held.id != id);
                state.notes.push(note);
                Reverse::RemoveNote(id)
            }
            Some(ReviewChangeKind::NoteDropped(id)) => {
                match state.notes.iter().position(|held| held.id == id) {
                    Some(index) => Reverse::RestoreNote(state.notes.remove(index)),
                    None => Reverse::Nothing,
                }
            }
            Some(ReviewChangeKind::Plan(plan)) => Reverse::Plan(state.plan.replace(plan)),
            Some(ReviewChangeKind::Layer(layer)) => {
                let before = std::mem::replace(&mut state.layer, layer.clone());
                effect = Effect::send(FollowerAction::Delegate(Delegate::ShowLayer(layer)));
                Reverse::Layer(before)
            }
            None => {
                if change.tool == "diff" {
                    // The code moved lines: the notes' places changed.
                    effect = self.read_context(state);
                }
                Reverse::Nothing
            }
        };
        state.applied.push(Applied {
            change,
            undone: false,
            reverse,
        });
        effect
    }

    fn read_context(&self, state: &FollowerState) -> Effect<FollowerAction> {
        let Some(session) = state.session.clone() else {
            return Effect::none();
        };
        let client = self.client.clone();
        Effect::future(async move { FollowerAction::ContextRead(client.context(&session).await) })
    }

    /// Puts back what the applied change `undo_id` did. Nothing happens for
    /// a change that is unknown or already undone.
    fn revert(&self, state: &mut FollowerState, undo_id: &str) -> Effect<FollowerAction> {
        if undo_id.is_empty() {
            return Effect::none();
        }
        let Some(entry) = state
            .applied
            .iter_mut()
            .find(|entry| entry.change.undo_id == undo_id && !entry.undone)
        else {
            return Effect::none();
        };
        entry.undone = true;
        let reverse = entry.reverse.clone();
        match reverse {
            Reverse::Nothing => Effect::none(),
            Reverse::View { before, focus, highlights } => {
                state.shown = before.clone();
                state.focus = focus;
                state.highlights = highlights;
                state.back = None;
                before.map_or_else(Effect::none, |place| {
                    Effect::send(FollowerAction::Delegate(Delegate::Show(place)))
                })
            }
            Reverse::RemoveNote(id) => {
                state.notes.retain(|note| note.id != id);
                Effect::none()
            }
            Reverse::RestoreNote(note) => {
                state.notes.push(note);
                Effect::none()
            }
            Reverse::Plan(plan) => {
                state.plan = plan;
                Effect::none()
            }
            Reverse::Layer(layer) => {
                state.layer.clone_from(&layer);
                Effect::send(FollowerAction::Delegate(Delegate::ShowLayer(layer)))
            }
        }
    }
}

impl Reducer for Follower {
    type State = FollowerState;
    type Action = FollowerAction;

    fn reduce(&self, state: &mut FollowerState, action: FollowerAction) -> Effect<FollowerAction> {
        match action {
            FollowerAction::Follow(session) => {
                *state = FollowerState {
                    session: Some(session.clone()),
                    ..FollowerState::default()
                };
                return self.watch(session, 0).and(Effect::cancel(WAIT));
            }
            FollowerAction::WatchEnded => {
                if let Some(session) = state.session.clone() {
                    return self.watch(session, state.seen);
                }
            }
            FollowerAction::WatchFailed(error) => state.error = Some(error),
            FollowerAction::Changed(change) => {
                // A change the feed replays after a restart is old news.
                if change.seq <= state.seen {
                    return Effect::none();
                }
                state.seen = change.seq;
                state.queued.push(change);
                return self.settle(state);
            }
            FollowerAction::Interacted(_) => {
                self.touch(state);
                return self.settle(state);
            }
            FollowerAction::DragStarted => {
                state.dragging = true;
                self.touch(state);
                return self.settle(state);
            }
            FollowerAction::DragEnded => {
                state.dragging = false;
                self.touch(state);
                return self.settle(state);
            }
            FollowerAction::UserWentTo(place) => {
                state.shown = Some(place);
                state.focus = None;
                state.back = None;
                self.touch(state);
                return self.settle(state);
            }
            FollowerAction::Quiet => return self.settle(state),
            FollowerAction::Undo(undo_id) => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                if !state
                    .applied
                    .iter()
                    .any(|entry| entry.change.undo_id == undo_id && entry.can_undo())
                {
                    return Effect::none();
                }
                let client = self.client.clone();
                return Effect::future(async move {
                    let result = client.undo(&session, &undo_id).await;
                    FollowerAction::Undone { undo_id, result }
                });
            }
            FollowerAction::Undone { undo_id, result } => match result {
                Ok(_) => return self.revert(state, &undo_id),
                Err(error) => state.error = Some(error),
            },
            FollowerAction::Back => {
                if let Some(place) = state.back.take() {
                    state.shown = Some(place.clone());
                    state.focus = None;
                    state.highlights.clear();
                    return Effect::send(FollowerAction::Delegate(Delegate::Show(place)));
                }
            }
            FollowerAction::ContextRead(result) => match result {
                Ok(context) => state.notes = context.notes,
                Err(error) => state.error = Some(error),
            },
            FollowerAction::DismissError => state.error = None,
            FollowerAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
