//! The review pane: the GPUI view over one session's [`SessionReview`] at a
//! time. It mounts only the rows on screen, saves every change through the
//! [`ReviewStore`] and sends queued comments through the [`CommentSink`]
//! when the terminal's events say the agent can take them. In a PR review
//! the review chat's agent works the view through [`ReviewPane::agent_action`].

mod keys;
mod render;
mod theme;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext as _, Context, EventEmitter, FocusHandle, Focusable, ListAlignment, ListOffset,
    ListState, SharedString, UniformListScrollHandle, Window, px,
};

pub use theme::Theme;

use crate::error::{ActionError, DeliveryError, HistoryError, StoreError};
use crate::images::{self, ImageCache, ImageStats, Job};
use crate::markdown::{MarkdownRenderer, PlainBlocks, RenderedBlock, rendered_diff};
use crate::model::{
    AgentAction, AgentState, Base, BodyRow, CommentId, ComposerAt, Content, DeliveryId, DiffLine,
    FileKey, ImageMode, Kind, LineKind, LineRange, ListRow, NoteId, PreparedReview,
    RenderedLayout, RenderedSpan, Review, ReviewSnapshot, Reviewed, Selection, SessionId,
    SessionReview, Side, Spot, Switch, Typed, body_rows, list_rows,
};
use crate::seams::{CommentSink, ReviewHistory, ReviewStore, TypeRequest};

/// How long typing in the comment box waits before it's saved. PER-25 asks
/// for drafts at least once a second.
const DRAFT_SAVE: Duration = Duration::from_millis(300);

/// How long the user must stop scrolling, typing and selecting before an
/// agent's change that moves the view applies (PR-24).
const QUIET: Duration = Duration::from_millis(800);

/// What became of an agent's tool call handed to the pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    /// It's applied and listed with Undo.
    Applied,
    /// The user is scrolling, typing or selecting: it applies once they
    /// stop, and [`PaneEvent::AgentApplied`] or [`PaneEvent::AgentDenied`]
    /// says how it went.
    Waiting,
}

/// What the pane tells the window it's in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneEvent {
    /// The session's count of comments not yet with the agent changed: the
    /// Diff strip's badge (RV-8).
    Unsent { session: SessionId, count: usize },
    /// A message reached the agent, or a PR review's review chat: mark the
    /// session working (RV-5).
    Sent {
        session: SessionId,
        delivery: DeliveryId,
        comments: usize,
    },
    /// An agent's change that waited for the user to stop applied: the
    /// chat's line for it ("● review_focus refund.rs:140") gets its Undo.
    AgentApplied {
        session: SessionId,
        id: String,
        summary: String,
    },
    /// An agent's change that waited couldn't apply; `message` is the
    /// tool's answer, said to the agent.
    AgentDenied {
        session: SessionId,
        id: String,
        message: String,
    },
    /// The agent-prepared view was reverted: put back what the window had
    /// before it opened (route, tab, panel; VW-9). The pane restored its own.
    Reverted { session: SessionId, view: String },
    /// The agent-prepared view was kept: drop the window's bar (VW-9).
    Kept { session: SessionId, view: String },
    /// Something the user has to act on, said so they can.
    Problem { session: SessionId, message: String },
}

/// How much of the review was mounted and reused, for tests and measures.
/// Counts are cumulative; compare two reads around the frames of interest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MountStats {
    /// Body rows in the current session's review.
    pub rows: usize,
    /// Body rows rendered, counting each frame's.
    pub mounted: usize,
    /// Distinct body rows ever rendered for the current review.
    pub distinct: usize,
    /// File list rows rendered.
    pub list_mounted: usize,
    /// Diff lines prepared for display.
    pub lines_built: usize,
    /// Times a prepared diff line was drawn again without preparing it.
    pub lines_reused: usize,
    /// Markdown files rendered.
    pub markdown_rendered: usize,
    pub images: ImageStats,
}

/// A diff line, prepared for display once and reused while it's on screen.
pub(crate) struct LineModel {
    pub(crate) kind: LineKind,
    pub(crate) old: SharedString,
    pub(crate) new: SharedString,
    pub(crate) text: SharedString,
    /// The line's number on the side it's on: old if removed, else new.
    pub(crate) number: Option<(Side, u32)>,
}

/// The send preview: the exact message and the comments it carries.
#[derive(Clone, Debug)]
pub(crate) struct Preview {
    /// The comments and questions it carries.
    pub(crate) ids: Vec<CommentId>,
    pub(crate) text: SharedString,
}

/// A session's save state.
#[derive(Default)]
struct Saving {
    /// The newest revision known to be on disk.
    saved: u64,
    /// A draft save is waiting for typing to pause.
    scheduled: bool,
    /// The last save failed; nothing more is sent until one works.
    failed: Option<String>,
}

/// Work for a session that's still loading.
#[derive(Default)]
struct Pending {
    review: Option<Review>,
    agent: Option<AgentState>,
}

/// The review pane. Create it with [`ReviewPane::new`], show a session with
/// [`ReviewPane::show_session`], and feed it the session's changes and its
/// terminal's state as they come.
pub struct ReviewPane {
    sink: Arc<dyn CommentSink>,
    store: Arc<dyn ReviewStore>,
    history: Option<Arc<dyn ReviewHistory>>,
    renderer: Rc<dyn MarkdownRenderer>,
    pub(crate) theme: Theme,
    pub(crate) focus: FocusHandle,
    pub(crate) composer_focus: FocusHandle,

    sessions: HashMap<SessionId, SessionReview>,
    loading: HashMap<SessionId, Pending>,
    broken: HashMap<SessionId, String>,
    saving: HashMap<SessionId, Saving>,
    in_flight: HashSet<SessionId>,
    /// Each session's reviewed snapshots, newest first, once loaded.
    pub(crate) reviewed: HashMap<SessionId, Vec<Reviewed>>,
    pub(crate) base_menu: bool,
    pub(crate) current: Option<SessionId>,

    pub(crate) list: ListState,
    pub(crate) files_scroll: UniformListScrollHandle,
    pub(crate) rows: Vec<BodyRow>,
    pub(crate) row_index: HashMap<BodyRow, usize>,
    pub(crate) list_rows: Vec<ListRow>,
    pub(crate) rendered: HashMap<(FileKey, String), Arc<Vec<RenderedBlock>>>,
    lines: HashMap<(usize, usize, usize), Rc<LineModel>>,
    mounted_rows: HashSet<BodyRow>,
    pub(crate) images: ImageCache,
    pub(crate) image_bounds: HashMap<usize, Rc<std::cell::Cell<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    pub(crate) stats: MountStats,

    pub(crate) cursor: Option<BodyRow>,
    pub(crate) anchor: Option<BodyRow>,
    pub(crate) dragging: bool,
    pub(crate) preview: Option<Preview>,
    pub(crate) notice: Option<SharedString>,

    /// When the user last scrolled, typed or selected (PR-24).
    active_at: Option<Instant>,
    /// Agent changes that move the view, waiting for the user to stop.
    deferred: Vec<(SessionId, AgentAction)>,
    /// A timer runs to apply them once the user is still.
    quiet_armed: bool,
}

impl EventEmitter<PaneEvent> for ReviewPane {}

impl Focusable for ReviewPane {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ReviewPane {
    /// A pane that sends to `sink` and saves to `store`, rendering markdown
    /// with the built-in block renderer.
    pub fn new(
        sink: Arc<dyn CommentSink>,
        store: Arc<dyn ReviewStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            sink,
            store,
            history: None,
            renderer: Rc::new(PlainBlocks),
            theme: Theme::default(),
            focus: cx.focus_handle(),
            composer_focus: cx.focus_handle(),
            sessions: HashMap::new(),
            loading: HashMap::new(),
            broken: HashMap::new(),
            saving: HashMap::new(),
            in_flight: HashSet::new(),
            reviewed: HashMap::new(),
            base_menu: false,
            current: None,
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            files_scroll: UniformListScrollHandle::new(),
            rows: Vec::new(),
            row_index: HashMap::new(),
            list_rows: Vec::new(),
            rendered: HashMap::new(),
            lines: HashMap::new(),
            mounted_rows: HashSet::new(),
            images: ImageCache::default(),
            image_bounds: HashMap::new(),
            stats: MountStats::default(),
            cursor: None,
            anchor: None,
            dragging: false,
            preview: None,
            notice: None,
            active_at: None,
            deferred: Vec::new(),
            quiet_armed: false,
        }
    }

    /// Renders markdown with `renderer` instead, such as the markdown crate's.
    pub fn with_renderer(mut self, renderer: Rc<dyn MarkdownRenderer>) -> Self {
        self.renderer = renderer;
        self
    }

    /// Reads and adds reviewed snapshots through `history`, which turns on
    /// Mark reviewed and the "Since review N" base picker (RV-28–RV-30).
    pub fn with_history(mut self, history: Arc<dyn ReviewHistory>) -> Self {
        self.history = Some(history);
        self
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Shows `session`. Its saved review is loaded off the UI thread the
    /// first time; until then the pane says it's loading.
    pub fn show_session(&mut self, session: SessionId, cx: &mut Context<Self>) {
        self.close_transient();
        if self.current.as_ref() != Some(&session) {
            self.current = Some(session.clone());
            self.cursor = None;
            self.anchor = None;
            self.lines.clear();
            self.mounted_rows.clear();
        }
        if !self.sessions.contains_key(&session)
            && !self.loading.contains_key(&session)
            && !self.broken.contains_key(&session)
        {
            self.loading.insert(session.clone(), Pending::default());
            let store = self.store.clone();
            let id = session.clone();
            cx.spawn(async move |this, cx| {
                let loaded = cx.background_spawn(async move { store.load(&id) }).await;
                this.update(cx, |pane, cx| pane.loaded(session, loaded, cx))
                    .ok();
            })
            .detach();
        }
        if let Some(review) = self.current_review() {
            self.cursor = review
                .memory()
                .current
                .as_ref()
                .and_then(|key| review.review().index(key))
                .map(BodyRow::File);
        }
        self.rebuild(cx);
    }

    /// The session's changes as they are now, against the session's base,
    /// from the daemon. While the user compares with an earlier review, the
    /// pane loads the changes against it through the history instead.
    pub fn set_review(&mut self, session: SessionId, review: Review, cx: &mut Context<Self>) {
        if let Some(pending) = self.loading.get_mut(&session) {
            pending.review = Some(review);
            return;
        }
        let Some(state) = self.sessions.get(&session) else {
            return;
        };
        let base = state.memory().base;
        if base != Base::Session && self.history.is_some() {
            self.load_changes(&session, base, cx);
            return;
        }
        self.apply_review(session, review, cx);
    }

    fn apply_review(&mut self, session: SessionId, review: Review, cx: &mut Context<Self>) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        state.set_review(review);
        if self.current.as_ref() == Some(&session) {
            self.lines.clear();
            self.rebuild(cx);
        }
        self.changed(&session, cx);
    }

    /// What the session's agent terminal can take now, from its events
    /// (busy, idle, exited, offline). Queued comments go when it's idle.
    pub fn set_agent_state(&mut self, session: SessionId, state: AgentState, cx: &mut Context<Self>) {
        if let Some(pending) = self.loading.get_mut(&session) {
            pending.agent = Some(state);
            return;
        }
        let Some(review) = self.sessions.get_mut(&session) else {
            return;
        };
        if review.book().agent() == state {
            return;
        }
        review.change(|book| book.set_agent(state));
        self.changed(&session, cx);
    }

    /// Shows `view`, an agent-prepared review (RV-32), over `session`'s
    /// review, with a bar to revert or keep it (VW-8). It's saved, so a
    /// restart shows it still.
    pub fn open_prepared(&mut self, session: SessionId, view: PreparedReview, cx: &mut Context<Self>) {
        let Some(review) = self.sessions.get_mut(&session) else {
            self.notice = Some("The review is still loading; open the prepared view again in a moment.".into());
            cx.notify();
            return;
        };
        review.open_prepared(view);
        if self.current.as_ref() == Some(&session) {
            self.cursor = review
                .memory()
                .current
                .as_ref()
                .and_then(|key| review.review().index(key))
                .map(BodyRow::File);
        }
        self.changed(&session, cx);
        if let Some(row) = self.cursor.clone() {
            self.go_to(row, cx);
        }
    }

    /// Applies a tool call from `session`'s review chat agent (PR-23): at
    /// once, listed with Undo (PR-24), unless it moves the view while the
    /// user is scrolling, typing or selecting; then it waits until they
    /// stop. An error is the tool's answer, such as a switch the user
    /// turned off (PR-25).
    pub fn agent_action(
        &mut self,
        session: SessionId,
        action: AgentAction,
        cx: &mut Context<Self>,
    ) -> Result<ActionOutcome, ActionError> {
        let Some(review) = self.sessions.get(&session) else {
            return Err(ActionError::NotOpen { session });
        };
        let switch = action.switch();
        if !review.memory().agent.may.allows(switch) {
            return Err(ActionError::TurnedOff { switch });
        }
        let waits = action.moves_view()
            && self.current.as_ref() == Some(&session)
            && (!self.still(cx) || self.deferred.iter().any(|(id, _)| *id == session));
        if waits {
            self.deferred.push((session, action));
            self.arm_quiet(cx);
            return Ok(ActionOutcome::Waiting);
        }
        self.apply_action(&session, action, cx)?;
        Ok(ActionOutcome::Applied)
    }

    /// Takes back the agent's change `id` in `session` (PR-24's Undo).
    pub fn undo_agent(&mut self, session: SessionId, id: &str, cx: &mut Context<Self>) -> Result<(), ActionError> {
        let Some(review) = self.sessions.get_mut(&session) else {
            return Err(ActionError::NotOpen { session });
        };
        let spot = review.undo_agent(id)?;
        self.changed(&session, cx);
        if let Some(spot) = spot.filter(|_| self.current.as_ref() == Some(&session)) {
            self.go_spot(&spot, cx);
        }
        Ok(())
    }

    /// Turns one of `session`'s "Agent may" switches on or off (PR-25).
    pub fn set_agent_may(&mut self, session: SessionId, switch: Switch, on: bool, cx: &mut Context<Self>) {
        let Some(review) = self.sessions.get_mut(&session) else {
            return;
        };
        review.set_agent_may(switch, on);
        self.changed(&session, cx);
    }

    /// What the user is looking at in `session`, for `review_context`
    /// (PR-23): the file, and the selected lines.
    pub fn context(&self, session: &SessionId) -> Option<Spot> {
        let review = self.sessions.get(session)?;
        if self.current.as_ref() != Some(session) {
            return Some(Spot {
                current: review.memory().current.clone(),
                selection: review.memory().agent.selection.clone(),
            });
        }
        Some(self.spot())
    }

    fn apply_action(&mut self, session: &SessionId, action: AgentAction, cx: &mut Context<Self>) -> Result<(), ActionError> {
        let now = if self.current.as_ref() == Some(session) {
            self.spot()
        } else {
            self.context(session).unwrap_or_default()
        };
        let Some(review) = self.sessions.get_mut(session) else {
            return Err(ActionError::NotOpen {
                session: session.clone(),
            });
        };
        let spot = review.apply_agent(action, now)?;
        self.changed(session, cx);
        if let Some(spot) = spot.filter(|_| self.current.as_ref() == Some(session)) {
            self.go_spot(&spot, cx);
        }
        Ok(())
    }

    /// Where the user is: the file, and the lines selected.
    fn spot(&self) -> Spot {
        let current = self
            .cursor
            .as_ref()
            .and_then(|row| self.file_key(row.file()))
            .or_else(|| self.current_review().and_then(|r| r.memory().current.clone()));
        let selection = self.selected_lines().and_then(|(file, side, lines, _)| {
            Some(Selection {
                file: self.file_key(file)?,
                side,
                lines,
            })
        });
        Spot { current, selection }
    }

    /// Moves the view to `spot`: selects its lines, or goes to its file.
    fn go_spot(&mut self, spot: &Spot, cx: &mut Context<Self>) {
        if let Some(selection) = &spot.selection
            && let Some((first, last)) = self.line_rows(selection)
        {
            self.anchor = Some(first.clone());
            self.cursor = Some(last);
            if let Some(&index) = self.row_index.get(&first) {
                self.list.scroll_to(ListOffset {
                    item_ix: index.saturating_sub(2),
                    offset_in_item: px(0.),
                });
            }
            cx.notify();
            return;
        }
        self.anchor = None;
        let file = spot
            .current
            .as_ref()
            .and_then(|key| self.current_review()?.review().index(key));
        match file {
            Some(file) => self.go_to(BodyRow::File(file), cx),
            None => {
                self.cursor = None;
                cx.notify();
            }
        }
    }

    /// The first and last rows of `selection`'s lines, if they're shown.
    fn line_rows(&self, selection: &Selection) -> Option<(BodyRow, BodyRow)> {
        let review = self.current_review()?;
        let file = review.review().index(&selection.file)?;
        let diff = review.review().files().get(file)?.text()?;
        let mut rows = self.rows.iter().filter(|row| match **row {
            BodyRow::Line { file: f, hunk, line } if f == file => diff
                .hunks
                .get(hunk)
                .and_then(|h| h.lines.get(line))
                .and_then(|l| l.number(selection.side))
                .is_some_and(|n| selection.lines.contains(n)),
            _ => false,
        });
        let first = rows.next()?.clone();
        let last = rows.last().cloned().unwrap_or_else(|| first.clone());
        Some((first, last))
    }

    /// The user scrolled, typed or selected: the agent's view changes wait
    /// until they stop (PR-24).
    pub(crate) fn user_active(&mut self, cx: &mut Context<Self>) {
        self.active_at = Some(cx.background_executor().now());
    }

    /// Whether the user has been still for [`QUIET`].
    fn still(&self, cx: &Context<Self>) -> bool {
        !self.dragging
            && self
                .active_at
                .is_none_or(|at| cx.background_executor().now().saturating_duration_since(at) >= QUIET)
    }

    /// Runs one timer that applies the waiting changes once the user is
    /// still, re-arming itself while they keep going. No polling: it's only
    /// armed while something waits.
    fn arm_quiet(&mut self, cx: &mut Context<Self>) {
        if self.quiet_armed {
            return;
        }
        self.quiet_armed = true;
        let wait = self.active_at.map_or(Duration::ZERO, |at| {
            QUIET.saturating_sub(cx.background_executor().now().saturating_duration_since(at))
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait.max(Duration::from_millis(1))).await;
            this.update(cx, |pane, cx| {
                pane.quiet_armed = false;
                if pane.still(cx) {
                    pane.apply_deferred(cx);
                } else if !pane.deferred.is_empty() {
                    pane.arm_quiet(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Applies the changes that waited, in order, saying how each went.
    fn apply_deferred(&mut self, cx: &mut Context<Self>) {
        for (session, action) in std::mem::take(&mut self.deferred) {
            let id = action.id.clone();
            let summary = action.summary();
            match self.apply_action(&session, action, cx) {
                Ok(()) => cx.emit(PaneEvent::AgentApplied { session, id, summary }),
                Err(error) => cx.emit(PaneEvent::AgentDenied {
                    session,
                    id,
                    message: error.to_string(),
                }),
            }
        }
    }

    /// Back: where the user was before the agent last moved the view.
    pub(crate) fn back(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else {
            return;
        };
        let Some(spot) = self.sessions.get_mut(&id).and_then(SessionReview::back) else {
            self.notice = Some("The agent hasn't moved the view since you were last here.".into());
            cx.notify();
            return;
        };
        self.changed(&id, cx);
        self.go_spot(&spot, cx);
    }

    /// Undoes an agent change in the current session, from its chip.
    pub(crate) fn undo_current(&mut self, change: &str, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else {
            return;
        };
        if let Err(error) = self.undo_agent(id, change, cx) {
            self.notice = Some(error.to_string().into());
            cx.notify();
        }
    }

    /// The current session's "Agent may" switch, flipped from its chip.
    pub(crate) fn flip_agent_may(&mut self, switch: Switch, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else {
            return;
        };
        let on = self
            .sessions
            .get(&id)
            .is_some_and(|review| review.memory().agent.may.allows(switch));
        self.set_agent_may(id, switch, !on, cx);
    }

    /// Runs a change on one of the agent's notes: make it a draft, edit it
    /// or drop it.
    pub(crate) fn on_note(&mut self, note: NoteId, change: NoteChange, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let result = match change {
            NoteChange::Draft => review.note_to_draft(note).map(|_| ()),
            NoteChange::Edit => review.edit_note(note),
            NoteChange::Drop => review.drop_note(note),
        };
        match result {
            Ok(()) => {
                self.notice = None;
                if change == NoteChange::Edit {
                    window.focus(&self.composer_focus, cx);
                }
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    /// The session's review, once loaded.
    pub fn session(&self, session: &SessionId) -> Option<&SessionReview> {
        self.sessions.get(session)
    }

    /// The newest revision of `session` known to be saved.
    pub fn saved_revision(&self, session: &SessionId) -> u64 {
        self.saving.get(session).map_or(0, |saving| saving.saved)
    }

    pub fn stats(&self) -> MountStats {
        let mut stats = self.stats;
        stats.rows = self.rows.len();
        stats.distinct = self.mounted_rows.len();
        stats.images = self.images.stats;
        stats
    }

    pub(crate) fn current_review(&self) -> Option<&SessionReview> {
        self.current.as_ref().and_then(|id| self.sessions.get(id))
    }

    pub(crate) fn current_review_mut(&mut self) -> Option<&mut SessionReview> {
        let id = self.current.as_ref()?;
        self.sessions.get_mut(id)
    }

    pub(crate) fn loading_current(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|id| self.loading.contains_key(id))
    }

    pub(crate) fn broken_current(&self) -> Option<&str> {
        self.current
            .as_ref()
            .and_then(|id| self.broken.get(id))
            .map(String::as_str)
    }

    pub(crate) fn unsaved_current(&self) -> Option<bool> {
        let id = self.current.as_ref()?;
        let review = self.sessions.get(id)?;
        let saving = self.saving.get(id);
        Some(saving.is_none_or(|saving| saving.saved < review.revision()))
    }

    pub(crate) fn save_failure_current(&self) -> Option<&str> {
        let id = self.current.as_ref()?;
        self.saving.get(id)?.failed.as_deref()
    }

    fn loaded(
        &mut self,
        session: SessionId,
        loaded: Result<Option<ReviewSnapshot>, StoreError>,
        cx: &mut Context<Self>,
    ) {
        let pending = self.loading.remove(&session).unwrap_or_default();
        let mut review = match loaded {
            Ok(Some(snapshot)) => {
                let revision = snapshot.revision;
                self.saving.entry(session.clone()).or_default().saved = revision;
                SessionReview::restore(snapshot)
            }
            Ok(None) => SessionReview::new(session.clone()),
            Err(error) => {
                let message = error.to_string();
                self.broken.insert(session.clone(), message.clone());
                cx.emit(PaneEvent::Problem { session, message });
                cx.notify();
                return;
            }
        };
        if let Some(changes) = pending.review {
            review.set_review(changes);
        }
        if let Some(agent) = pending.agent {
            review.change(|book| book.set_agent(agent));
        }
        let base = review.memory().base;
        if self.current.as_ref() == Some(&session) {
            self.cursor = review
                .memory()
                .current
                .as_ref()
                .and_then(|key| review.review().index(key))
                .map(BodyRow::File);
        }
        self.sessions.insert(session.clone(), review);
        if self.current.as_ref() == Some(&session) {
            self.rebuild(cx);
        }
        self.load_history(&session, cx);
        if base != Base::Session {
            self.load_changes(&session, base, cx);
        }
        self.changed(&session, cx);
    }

    /// Loads the session's reviewed snapshots off the UI thread.
    fn load_history(&mut self, session: &SessionId, cx: &mut Context<Self>) {
        let Some(history) = self.history.clone() else {
            return;
        };
        let id = session.clone();
        cx.spawn(async move |this, cx| {
            let ask = id.clone();
            let result = cx.background_spawn(async move { history.reviewed(&ask).await }).await;
            this.update(cx, |pane, cx| match result {
                Ok(list) => {
                    pane.reviewed.insert(id, list);
                    cx.notify();
                }
                Err(error) => pane.problem(&id, &error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Loads the session's changes against `base` off the UI thread.
    fn load_changes(&mut self, session: &SessionId, base: Base, cx: &mut Context<Self>) {
        let Some(history) = self.history.clone() else {
            return;
        };
        let id = session.clone();
        cx.spawn(async move |this, cx| {
            let ask = id.clone();
            let result = cx
                .background_spawn(async move { history.changes(&ask, base).await })
                .await;
            this.update(cx, |pane, cx| {
                // Only if that's still the base: a later pick wins.
                let current = pane.sessions.get(&id).map(|review| review.memory().base);
                if current != Some(base) {
                    return;
                }
                match result {
                    Ok(review) => pane.apply_review(id, review, cx),
                    Err(error) => pane.problem(&id, &error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Compares the current session with `base` (RV-30).
    pub(crate) fn choose_base(&mut self, base: Base, cx: &mut Context<Self>) {
        self.base_menu = false;
        let Some(id) = self.current.clone() else {
            return;
        };
        let Some(review) = self.sessions.get_mut(&id) else {
            return;
        };
        if review.memory().base == base {
            cx.notify();
            return;
        }
        review.remember(|memory| memory.base = base);
        self.load_changes(&id, base, cx);
        self.changed(&id, cx);
    }

    /// Records what the user sees now as reviewed (RV-28).
    pub(crate) fn mark_reviewed(&mut self, cx: &mut Context<Self>) {
        let (Some(history), Some(id)) = (self.history.clone(), self.current.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let ask = id.clone();
            let result = cx
                .background_spawn(async move { history.mark_reviewed(&ask).await })
                .await;
            this.update(cx, |pane, cx| match result {
                Ok(reviewed) => {
                    let notice = format!("Marked reviewed as review {}.", reviewed.number);
                    let list = pane.reviewed.entry(id.clone()).or_default();
                    list.retain(|other| other.number != reviewed.number);
                    list.insert(0, reviewed);
                    if pane.current.as_ref() == Some(&id) {
                        pane.notice = Some(notice.into());
                    }
                    cx.notify();
                }
                Err(error) => pane.problem(&id, &error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Reverts or keeps the prepared view showing.
    pub(crate) fn end_prepared(&mut self, revert: bool, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else {
            return;
        };
        let Some(review) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(view) = review.memory().prepared.as_ref().map(|showing| showing.view.id.clone()) else {
            return;
        };
        if revert {
            let before = review.memory().base;
            review.remember(|memory| memory.revert_prepared());
            let base = review.memory().base;
            self.cursor = review
                .memory()
                .current
                .as_ref()
                .and_then(|key| review.review().index(key))
                .map(BodyRow::File);
            cx.emit(PaneEvent::Reverted {
                session: id.clone(),
                view,
            });
            if base != before {
                self.load_changes(&id, base, cx);
            }
        } else {
            review.remember(|memory| memory.keep_prepared());
            cx.emit(PaneEvent::Kept {
                session: id.clone(),
                view,
            });
        }
        self.changed(&id, cx);
    }

    fn problem(&mut self, session: &SessionId, error: &HistoryError, cx: &mut Context<Self>) {
        let message = error.to_string();
        if self.current.as_ref() == Some(session) {
            self.notice = Some(message.clone().into());
        }
        cx.emit(PaneEvent::Problem {
            session: session.clone(),
            message,
        });
        cx.notify();
    }

    /// After any change to a session: save it, send what can be sent, and
    /// tell the window.
    pub(crate) fn changed(&mut self, session: &SessionId, cx: &mut Context<Self>) {
        self.persist(session, cx);
        self.pump(session, cx);
        if let Some(review) = self.sessions.get(session) {
            cx.emit(PaneEvent::Unsent {
                session: session.clone(),
                count: review.book().unsent(),
            });
        }
        if self.current.as_ref() == Some(session) {
            self.rebuild(cx);
        }
        cx.notify();
    }

    /// Saves the session's newest state off the UI thread.
    fn persist(&mut self, session: &SessionId, cx: &mut Context<Self>) {
        let Some(review) = self.sessions.get(session) else {
            return;
        };
        let saving = self.saving.entry(session.clone()).or_default();
        if saving.saved >= review.revision() {
            return;
        }
        let snapshot = review.snapshot();
        let store = self.store.clone();
        let id = session.clone();
        cx.spawn(async move |this, cx| {
            let revision = snapshot.revision;
            let result = cx.background_spawn(async move { store.save(&snapshot) }).await;
            this.update(cx, |pane, cx| pane.saved(&id, revision, result, cx))
                .ok();
        })
        .detach();
    }

    /// Saves the session soon, for typing: at most one save waits.
    pub(crate) fn persist_soon(&mut self, session: &SessionId, cx: &mut Context<Self>) {
        let saving = self.saving.entry(session.clone()).or_default();
        if saving.scheduled {
            return;
        }
        saving.scheduled = true;
        let id = session.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DRAFT_SAVE).await;
            this.update(cx, |pane, cx| {
                if let Some(saving) = pane.saving.get_mut(&id) {
                    saving.scheduled = false;
                }
                pane.persist(&id, cx);
            })
            .ok();
        })
        .detach();
    }

    fn saved(
        &mut self,
        session: &SessionId,
        revision: u64,
        result: Result<(), StoreError>,
        cx: &mut Context<Self>,
    ) {
        let saving = self.saving.entry(session.clone()).or_default();
        match result {
            Ok(()) => {
                saving.saved = saving.saved.max(revision);
                let recovered = saving.failed.take().is_some();
                if recovered {
                    self.pump(session, cx);
                }
            }
            Err(error) => {
                let message = error.to_string();
                saving.failed = Some(message.clone());
                cx.emit(PaneEvent::Problem {
                    session: session.clone(),
                    message,
                });
            }
        }
        cx.notify();
    }

    /// Sends the session's next queued delivery if the agent can take it:
    /// first saves it as sending, then types it off the UI thread. Called
    /// on events (a change, a terminal state, a finished send), never on a
    /// timer.
    fn pump(&mut self, session: &SessionId, cx: &mut Context<Self>) {
        if self.in_flight.contains(session) {
            return;
        }
        if self
            .saving
            .get(session)
            .is_some_and(|saving| saving.failed.is_some())
        {
            return;
        }
        let Some(review) = self.sessions.get_mut(session) else {
            return;
        };
        let Some(dispatch) = review.change(|book| book.next_dispatch()) else {
            return;
        };
        let snapshot = review.snapshot();
        self.in_flight.insert(session.clone());
        let store = self.store.clone();
        let sink = self.sink.clone();
        let request = TypeRequest {
            session: session.clone(),
            key: dispatch.key.clone(),
            text: dispatch.text.clone(),
        };
        let id = session.clone();
        cx.spawn(async move |this, cx| {
            let revision = snapshot.revision;
            let outcome = cx
                .background_spawn(async move {
                    // Saved as sending before a byte is typed, so a restart
                    // resends under the same key rather than forgetting.
                    match store.save(&snapshot) {
                        Ok(()) => Sent::Typed(sink.type_text(request).await),
                        Err(error) => Sent::NotSaved(error),
                    }
                })
                .await;
            this.update(cx, |pane, cx| {
                pane.dispatched(&id, dispatch.delivery, dispatch.attempt, revision, outcome, cx);
            })
            .ok();
        })
        .detach();
    }

    fn dispatched(
        &mut self,
        session: &SessionId,
        delivery: DeliveryId,
        attempt: u32,
        revision: u64,
        outcome: Sent,
        cx: &mut Context<Self>,
    ) {
        self.in_flight.remove(session);
        let Some(review) = self.sessions.get_mut(session) else {
            return;
        };
        match outcome {
            Sent::NotSaved(error) => {
                review.change(|book| book.abandon(delivery, attempt));
                self.saved(session, revision, Err(error), cx);
            }
            Sent::Typed(result) => {
                let saving = self.saving.entry(session.clone()).or_default();
                saving.saved = saving.saved.max(revision);
                let ok = result.is_ok();
                if let Err(error) = &result
                    && !matches!(
                        error,
                        DeliveryError::Disconnected { .. } | DeliveryError::Interrupted { .. }
                    )
                {
                    cx.emit(PaneEvent::Problem {
                        session: session.clone(),
                        message: error.to_string(),
                    });
                }
                review.change(|book| book.finish(delivery, attempt, result));
                if ok {
                    let comments = review
                        .book()
                        .delivery(delivery)
                        .map_or(0, |d| d.comments.len());
                    cx.emit(PaneEvent::Sent {
                        session: session.clone(),
                        delivery,
                        comments,
                    });
                }
            }
        }
        self.changed(session, cx);
    }

    /// Rebuilds the current session's rows and splices the body list so
    /// rows that didn't change keep their measured sizes and the scroll
    /// position holds.
    pub(crate) fn rebuild(&mut self, cx: &mut Context<Self>) {
        let rendered = self.rendered_layout(cx);
        let (rows, list_rows) = match self.current_review() {
            Some(review) => {
                let composer = review.composer().map(|draft| &draft.at);
                (
                    body_rows(
                        review.review(),
                        review.memory(),
                        review.book(),
                        &rendered,
                        composer,
                    ),
                    list_rows(review.review(), review.memory(), review.book()),
                )
            }
            None => (Vec::new(), Vec::new()),
        };
        let prefix = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = self.rows[prefix..]
            .iter()
            .rev()
            .zip(rows[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if self.rows.len() != rows.len() || prefix != rows.len() {
            self.list
                .splice(prefix..self.rows.len() - suffix, rows.len() - suffix - prefix);
        }
        self.row_index = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.clone(), index))
            .collect();
        self.rows = rows;
        self.list_rows = list_rows;
        if self
            .cursor
            .as_ref()
            .is_some_and(|row| !self.row_index.contains_key(row))
        {
            self.cursor = self.cursor.as_ref().map(|row| BodyRow::File(row.file()));
            if self
                .cursor
                .as_ref()
                .is_some_and(|row| !self.row_index.contains_key(row))
            {
                self.cursor = None;
            }
        }
        if self
            .anchor
            .as_ref()
            .is_some_and(|row| !self.row_index.contains_key(row))
        {
            self.anchor = None;
        }
        cx.notify();
    }

    /// The rendered blocks of each markdown file shown rendered, rendering
    /// those not cached yet.
    fn rendered_layout(&mut self, _cx: &mut Context<Self>) -> RenderedLayout {
        let mut layout = RenderedLayout::new();
        let Some(review) = self.current.as_ref().and_then(|id| self.sessions.get(id)) else {
            return layout;
        };
        for (index, file) in review.review().files().iter().enumerate() {
            if !review.memory().rendered.contains(&file.key) {
                continue;
            }
            let Content::Markdown(diff) = &file.content else {
                continue;
            };
            let key = (file.key.clone(), file.blob.clone());
            let blocks = match self.rendered.get(&key) {
                Some(blocks) => blocks.clone(),
                None => {
                    let old = diff.old_text.as_deref().unwrap_or("");
                    let Some(new) = diff.new_text.as_deref() else {
                        continue;
                    };
                    let blocks = Arc::new(rendered_diff(self.renderer.as_ref(), old, new));
                    self.stats.markdown_rendered += 1;
                    self.rendered.insert(key, blocks.clone());
                    blocks
                }
            };
            layout.insert(index, blocks.iter().map(RenderedBlock::new_lines).collect());
        }
        layout
    }

    /// The prepared display of a diff line, reused while its file is the same.
    pub(crate) fn line_model(&mut self, file: usize, hunk: usize, line: &DiffLine, index: usize) -> Rc<LineModel> {
        if let Some(model) = self.lines.get(&(file, hunk, index)) {
            self.stats.lines_reused += 1;
            return model.clone();
        }
        self.stats.lines_built += 1;
        let number = |n: Option<u32>| n.map_or(SharedString::default(), |n| n.to_string().into());
        let model = Rc::new(LineModel {
            kind: line.kind,
            old: number(line.old),
            new: number(line.new),
            text: expand_tabs(&line.text).into(),
            number: match line.kind {
                LineKind::Removed => line.old.map(|n| (Side::Old, n)),
                _ => line.new.map(|n| (Side::New, n)),
            },
        });
        self.lines.insert((file, hunk, index), model.clone());
        model
    }

    pub(crate) fn note_mounted(&mut self, row: &BodyRow) {
        self.stats.mounted += 1;
        if !self.mounted_rows.contains(row) {
            self.mounted_rows.insert(row.clone());
        }
    }

    pub(crate) fn note_list_mounted(&mut self, count: usize) {
        self.stats.list_mounted += count;
    }

    /// Starts background work the cache asked for.
    pub(crate) fn run_jobs(&mut self, jobs: Vec<Job>, cx: &mut Context<Self>) {
        for job in jobs {
            match job {
                Job::Decode(blob) => {
                    cx.spawn(async move |this, cx| {
                        let sha = blob.sha.clone();
                        let outcome = cx
                            .background_spawn(async move { images::decode(&blob) })
                            .await;
                        this.update(cx, |pane, cx| {
                            pane.images.decoded(sha, outcome);
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                Job::Diff { key, old, new } => {
                    cx.spawn(async move |this, cx| {
                        let diff = cx
                            .background_spawn(async move { images::compare(&old, &new) })
                            .await;
                        this.update(cx, |pane, cx| {
                            pane.images.diffed(key, diff);
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
            }
        }
    }

    /// Closes what belongs to the session on screen and not to the session:
    /// the send preview and any notice.
    fn close_transient(&mut self) {
        self.preview = None;
        self.base_menu = false;
        self.notice = None;
        self.dragging = false;
    }

    // Actions, shared by keys, buttons and the command palette.

    /// Scrolls to and puts the cursor on a row.
    pub(crate) fn go_to(&mut self, row: BodyRow, cx: &mut Context<Self>) {
        if let Some(&index) = self.row_index.get(&row) {
            if matches!(row, BodyRow::File(_)) {
                // A file that starts a prepared group shows under its title.
                let group_above = index
                    .checked_sub(1)
                    .filter(|&above| matches!(self.rows.get(above), Some(BodyRow::Group(_))));
                self.list.scroll_to(ListOffset {
                    item_ix: group_above.unwrap_or(index),
                    offset_in_item: px(0.),
                });
            } else {
                self.list.scroll_to_reveal_item(index);
            }
            let file = row.file();
            self.cursor = Some(row);
            if let Some(review) = self.current_review_mut() {
                let key = review.review().files().get(file).map(|f| f.key.clone());
                if review.memory().current != key {
                    review.remember(|memory| memory.current = key);
                    if let Some(id) = self.current.clone() {
                        self.persist_soon(&id, cx);
                    }
                }
            }
            cx.notify();
        }
    }

    /// Selects a line or block, or extends the selection to it.
    pub(crate) fn select(&mut self, row: BodyRow, extend: bool, cx: &mut Context<Self>) {
        self.user_active(cx);
        let same_file = self
            .anchor
            .as_ref()
            .is_some_and(|anchor| anchor.file() == row.file() && same_kind(anchor, &row));
        if !(extend && same_file) {
            self.anchor = Some(row.clone());
        }
        self.go_to(row, cx);
    }

    /// The lines the selection covers, on one side: the new side, unless
    /// only removed lines are selected.
    pub(crate) fn selected_lines(&self) -> Option<(usize, Side, LineRange, Option<RenderedSpan>)> {
        let review = self.current_review()?;
        let head = self.cursor.as_ref()?;
        let anchor = self.anchor.as_ref().unwrap_or(head);
        if anchor.file() != head.file() || !same_kind(anchor, head) {
            return None;
        }
        let (a, b) = (*self.row_index.get(anchor)?, *self.row_index.get(head)?);
        let (from, to) = (a.min(b), a.max(b));
        let file = review.review().files().get(head.file())?;
        match head {
            BodyRow::Line { .. } => {
                let diff = file.text()?;
                let lines: Vec<&DiffLine> = self.rows[from..=to]
                    .iter()
                    .filter_map(|row| match *row {
                        BodyRow::Line { hunk, line, .. } => diff.hunks.get(hunk)?.lines.get(line),
                        _ => None,
                    })
                    .collect();
                let side = if lines.iter().any(|line| line.new.is_some()) {
                    Side::New
                } else {
                    Side::Old
                };
                let numbers: Vec<u32> = lines.iter().filter_map(|line| line.number(side)).collect();
                let (first, last) = (*numbers.iter().min()?, *numbers.iter().max()?);
                Some((head.file(), side, LineRange::new(first, last), None))
            }
            BodyRow::Block { .. } => {
                let blocks = self.rendered.get(&(file.key.clone(), file.blob.clone()))?;
                let chosen: Vec<&RenderedBlock> = self.rows[from..=to]
                    .iter()
                    .filter_map(|row| match *row {
                        BodyRow::Block { block, .. } => blocks.get(block),
                        _ => None,
                    })
                    .filter(|block| block.new_lines().is_some())
                    .collect();
                let first = chosen.iter().map(|b| b.block.lines.first).min()?;
                let last = chosen.iter().map(|b| b.block.lines.last).max()?;
                let text = chosen
                    .iter()
                    .map(|b| b.block.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                Some((
                    head.file(),
                    Side::New,
                    LineRange::new(first, last),
                    Some(RenderedSpan { text }),
                ))
            }
            _ => None,
        }
    }

    /// Opens the comment box on the selection (RV-4, SM-6).
    pub(crate) fn comment_on_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_on_selection(Kind::Comment, window, cx);
    }

    /// Opens the box on the selection for a question to the review chat;
    /// only in a PR review (PR-19).
    pub(crate) fn ask_on_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pr_review() {
            return;
        }
        self.compose_on_selection(Kind::Question, window, cx);
    }

    /// Whether the current review is a PR review: its lines offer Ask, its
    /// box can draft for GitHub, and its agent is the review chat.
    pub(crate) fn pr_review(&self) -> bool {
        self.current_review()
            .is_some_and(|review| !review.review().pulls().is_empty())
    }

    /// Opens the box for a question on a whole file, or on one hunk of it.
    pub(crate) fn ask_about(&mut self, file: usize, hunk: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let Some(change) = review.review().files().get(file) else {
            return;
        };
        let key = change.key.clone();
        let (side, lines) = match hunk.and_then(|h| change.text()?.hunks.get(h)) {
            Some(hunk) => hunk_lines(hunk),
            None => (Side::New, LineRange::FILE),
        };
        review.open_composer_as(Kind::Question, ComposerAt { file: key, side, lines }, None);
        self.notice = None;
        window.focus(&self.composer_focus, cx);
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    /// Makes the open box a comment, a question or a GitHub draft.
    pub(crate) fn set_composer_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if let Some(review) = self.current_review_mut() {
            review.set_composer_kind(kind);
        }
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    fn compose_on_selection(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let Some((file, side, lines, rendered)) = self.selected_lines() else {
            self.notice = Some("Select a line first: click its number, or move to it with j and k.".into());
            cx.notify();
            return;
        };
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let Some(key) = review.review().files().get(file).map(|f| f.key.clone()) else {
            return;
        };
        review.open_composer_as(kind, ComposerAt { file: key, side, lines }, rendered);
        self.notice = None;
        window.focus(&self.composer_focus, cx);
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    pub(crate) fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.current_review_mut() else {
            return;
        };
        match review.submit_composer() {
            Ok(_) => {
                self.notice = None;
                self.anchor = None;
                window.focus(&self.focus, cx);
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    pub(crate) fn cancel_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(review) = self.current_review_mut() {
            review.close_composer();
        }
        window.focus(&self.focus, cx);
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    pub(crate) fn type_in_composer(&mut self, text: &str, cx: &mut Context<Self>) {
        self.edit_composer(|draft| draft.push_str(text), cx);
    }

    pub(crate) fn erase_in_composer(&mut self, cx: &mut Context<Self>) {
        self.edit_composer(
            |draft| {
                draft.pop();
            },
            cx,
        );
    }

    fn edit_composer(&mut self, edit: impl FnOnce(&mut String), cx: &mut Context<Self>) {
        self.user_active(cx);
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let Some(mut text) = review.composer().map(|draft| draft.text.clone()) else {
            return;
        };
        edit(&mut text);
        review.set_composer_text(text);
        if let Some(id) = self.current.clone() {
            self.persist_soon(&id, cx);
        }
        cx.notify();
    }

    /// Opens the send preview for `ids`, or for every draft.
    pub(crate) fn open_preview(&mut self, ids: Option<Vec<CommentId>>, cx: &mut Context<Self>) {
        let Some(review) = self.current_review() else {
            return;
        };
        let book = review.book();
        let ids = ids.unwrap_or_else(|| book.drafts());
        match book.preview(&ids) {
            Some(text) => {
                self.preview = Some(Preview {
                    ids,
                    text: text.into(),
                });
                self.notice = None;
            }
            None => self.notice = Some(crate::ReviewError::NothingToSend.to_string().into()),
        }
        cx.notify();
    }

    /// Queues what the preview showed. If the comments changed since, the
    /// preview shows the new text instead, so nothing unseen is sent.
    pub(crate) fn confirm_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.clone() else {
            return;
        };
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let now = review.book().preview(&preview.ids);
        if now.as_deref() != Some(preview.text.as_ref()) {
            self.preview = now.map(|text| Preview {
                text: text.into(),
                ..preview
            });
            self.notice = Some("The comments changed; check the message again.".into());
            cx.notify();
            return;
        }
        let result = review.change(|book| book.queue(&preview.ids));
        self.preview = None;
        if let Err(error) = result {
            self.notice = Some(error.to_string().into());
        }
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    /// Runs a change on the current session's comments; errors become the
    /// notice.
    pub(crate) fn on_book(
        &mut self,
        change: impl FnOnce(&mut crate::model::CommentBook) -> Result<(), crate::ReviewError>,
        cx: &mut Context<Self>,
    ) {
        let Some(review) = self.current_review_mut() else {
            return;
        };
        match review.change(change) {
            Ok(()) => self.notice = None,
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    /// Runs a change on what the current session remembers.
    pub(crate) fn on_memory(
        &mut self,
        change: impl FnOnce(&mut crate::model::ReviewMemory),
        cx: &mut Context<Self>,
    ) {
        let Some(review) = self.current_review_mut() else {
            return;
        };
        review.remember(change);
        if let Some(id) = self.current.clone() {
            self.changed(&id, cx);
        }
    }

    pub(crate) fn file_key(&self, file: usize) -> Option<FileKey> {
        self.current_review()?
            .review()
            .files()
            .get(file)
            .map(|f| f.key.clone())
    }

    pub(crate) fn toggle_viewed(&mut self, file: usize, cx: &mut Context<Self>) {
        let Some(review) = self.current_review() else {
            return;
        };
        let Some(change) = review.review().files().get(file) else {
            return;
        };
        let (key, blob) = (change.key.clone(), change.blob.clone());
        let viewed = review.memory().viewed(&key, &blob) == crate::model::Viewed::Yes;
        self.on_memory(|memory| memory.set_viewed(&key, &blob, !viewed), cx);
    }

    pub(crate) fn set_image_mode(&mut self, file: usize, mode: Option<ImageMode>, highlight: Option<bool>, cx: &mut Context<Self>) {
        let Some(key) = self.file_key(file) else {
            return;
        };
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let now = review.memory().image(&key);
        let mut next = now;
        if let Some(mode) = mode {
            next.mode = mode;
        }
        if let Some(highlight) = highlight {
            next.highlight = highlight;
        }
        if next == now {
            return;
        }
        // A drag changes this every frame: no rows change, and it's saved
        // when the drag pauses, well inside PER-25's second.
        review.remember(|memory| memory.images.insert(key, next));
        if let Some(id) = self.current.clone() {
            self.persist_soon(&id, cx);
        }
        cx.notify();
    }

    /// Saves the current session again after a save failed, such as once
    /// the disk has room.
    pub(crate) fn save_again(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else {
            return;
        };
        if let Some(review) = self.sessions.get_mut(&id) {
            review.touch();
        }
        self.persist(&id, cx);
        cx.notify();
    }
}

/// What the user does with one of the agent's notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoteChange {
    /// Make it a GitHub draft (PR-23).
    Draft,
    /// Open it in the box as a GitHub draft, to change first.
    Edit,
    Drop,
}

/// What a dispatch's background half came back with.
enum Sent {
    NotSaved(StoreError),
    Typed(Result<Typed, DeliveryError>),
}

fn same_kind(a: &BodyRow, b: &BodyRow) -> bool {
    matches!(
        (a, b),
        (BodyRow::Line { .. }, BodyRow::Line { .. }) | (BodyRow::Block { .. }, BodyRow::Block { .. })
    )
}

/// The side and lines a hunk covers: its new lines, or its old ones if it
/// only removes.
fn hunk_lines(hunk: &crate::model::Hunk) -> (Side, LineRange) {
    let numbers = |side: Side| -> Vec<u32> { hunk.lines.iter().filter_map(|line| line.number(side)).collect() };
    let (side, numbers) = match numbers(Side::New) {
        new if !new.is_empty() => (Side::New, new),
        _ => (Side::Old, numbers(Side::Old)),
    };
    match (numbers.iter().min(), numbers.iter().max()) {
        (Some(&first), Some(&last)) => (side, LineRange::new(first, last)),
        _ => (Side::New, LineRange::FILE),
    }
}

fn expand_tabs(text: &str) -> String {
    if text.contains('\t') {
        text.replace('\t', "    ")
    } else {
        text.to_owned()
    }
}
