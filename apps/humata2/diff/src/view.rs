//! The diff view: gpui-component's diff over a [`Review`], with a header
//! per repo and group, the review's notes, folds that ask for the file's
//! text, viewed files and the spec editor (D67).
//!
//! gpui-component's [`Diff`] owns the rows: it virtualizes them, wraps or
//! scrolls long lines, highlights off the UI thread, selects and copies
//! lines, and folds unchanged lines when a file's patch holds them. This
//! view turns the review into its files (one patch per file, named
//! `repo/path`, and an empty file per repo or group heading), into
//! annotations (comments, notes, folds, the spec editor) and into header
//! content, and turns the diff's events back into the review's terms.
//!
//! A file's patch holds only its hunks until the user opens one of its
//! folds or picks Full file: then it holds the whole file, and the diff
//! folds and unfolds it itself. When a file's text isn't in the review, a
//! fold asks for it with [`DiffEvent::ContextNeeded`].

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::rc::Rc;

use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::diff::{
    Diff, DiffAnnotation, DiffEvent as StateEvent, DiffFile, DiffHunkSeparator, DiffLinePosition,
    DiffLineRange, DiffMode as Columns, DiffSide, DiffState,
};
use gpui_kit::prelude::FluentBuilder as _;
// GPUI renamed the scroll wheel event to `ScrollEvent`; GPUI Fast still
// has only the old name, so the view keeps it on both engines.
#[allow(deprecated)]
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, Role,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Toggled, WeakEntity, Window, actions, div, px,
};

use crate::anchor::{CommentAnchor, LineRange, LineTarget, Side};
use crate::guide::NoteTarget;
use crate::model::{FileDiff, FileStatus, Review, Viewed};
use crate::order::{self, Annotation, AnnotationId, DiffMode, Entry, Place};
use crate::patch;
use crate::spec::{Base, DiffSpec, Include};

actions!(
    diff_view,
    [
        /// Go to the next change, wrapping to the first after the last.
        NextChange,
        /// Go to the previous change.
        PreviousChange,
        /// Go to the next file's header.
        NextFile,
        /// Go to the previous file's header.
        PreviousFile,
        /// Ask for a comment on the selected lines.
        CommentOnSelection,
        /// Clear the selection.
        ClearSelection,
    ]
);

/// The key context the view's bindings are in.
pub const KEY_CONTEXT: &str = "DiffView";

/// The view's key bindings, for the embedder to register with
/// `cx.bind_keys`: ⌃⌘↓ and ⌃⌘↑ for the next and previous change, ⌥⌘↓ and
/// ⌥⌘↑ for files, Return to comment on the selection and Escape to clear
/// it. Arrows, page keys, ⌘C and ⌘A are the diff's own.
#[must_use]
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("ctrl-cmd-down", NextChange, context),
        KeyBinding::new("ctrl-cmd-up", PreviousChange, context),
        KeyBinding::new("alt-cmd-down", NextFile, context),
        KeyBinding::new("alt-cmd-up", PreviousFile, context),
        KeyBinding::new("enter", CommentOnSelection, context),
        KeyBinding::new("escape", ClearSelection, context),
    ]
}

/// What the view does with lines wider than the pane (ST-7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LongLines {
    /// Break them at the column's width.
    #[default]
    Wrap,
    /// Keep each on one line; the code scrolls sideways.
    Scroll,
}

impl LongLines {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Wrap => "Wrap",
            Self::Scroll => "Scroll",
        }
    }
}

/// What the view tells its embedder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffEvent {
    /// The user asked to comment on lines: with the add button beside a
    /// line or a selection, or Return on a selection (RV-4).
    CommentRequested(CommentAnchor),
    /// The selected lines changed, or the selection was cleared.
    SelectionChanged(Option<LineTarget>),
    ModeChanged(DiffMode),
    LongLinesChanged(LongLines),
    /// The user asked to open a file in a big view (RV-3).
    OpenFile { repo: String, path: String },
    /// The user edited what a repo's diff compares (RV-31). Compute the
    /// diff for `spec` and hand it back with [`DiffView::set_review`]; the
    /// header shows `spec` until then.
    SpecChanged { repo: String, spec: DiffSpec },
    /// The user marked a file viewed or not (RV-29). `content_id` is the
    /// file's [`FileDiff::content_id`] as shown, so what is recorded is
    /// what the user saw. The view folds or unfolds the file at once and
    /// keeps the mark until the review agrees or the file's content
    /// changes.
    ViewedChanged {
        repo: String,
        path: String,
        viewed: bool,
        content_id: Option<String>,
    },
    /// The user opened a fold of a file whose text the review doesn't
    /// hold (RV-18). Hand back a review whose file has
    /// [`FileDiff::new_lines`]; the fold opens when it arrives.
    ContextNeeded { repo: String, path: String },
}

/// Draws an annotation, such as a comment thread or a draft, under its
/// lines. It may be any height and change height freely: the diff measures
/// visible annotations every frame.
pub trait AnnotationRenderer: 'static {
    fn render(&self, id: AnnotationId, window: &mut Window, cx: &mut App) -> AnyElement;
}

impl<F> AnnotationRenderer for F
where
    F: Fn(AnnotationId, &mut Window, &mut App) -> AnyElement + 'static,
{
    fn render(&self, id: AnnotationId, window: &mut Window, cx: &mut App) -> AnyElement {
        self(id, window, cx)
    }
}

/// A file by repo name and path.
type Key = (String, String);

/// How much of a file its patch holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Variant {
    /// Its hunks; the view draws the folds between them.
    Hunks,
    /// The whole file with its changes; the diff folds it.
    Full,
    /// The file as it is now, without changes (Full file, changes hidden).
    Text,
    /// A repo's or group's heading: an empty file.
    Heading,
}

/// A file as the diff holds it, kept while its content and variant stay.
struct Cached {
    hash: u64,
    variant: Variant,
    file: DiffFile,
    /// Why the file's changes aren't shown, when its patch didn't parse.
    notice: Option<String>,
}

/// What a header shows.
enum Header {
    Repo {
        name: String,
        detail: String,
        comparing: String,
        pending: bool,
        editing: bool,
        expanded: bool,
    },
    Group {
        title: String,
        count: String,
    },
    File {
        repo: String,
        path: String,
        status: String,
        counts: String,
        viewed: bool,
        changed_since: bool,
    },
}

/// What an annotation draws.
enum Item {
    Comment(AnnotationId),
    Note {
        label: String,
        text: String,
        author: String,
        outdated: bool,
    },
    Gap {
        key: Key,
        new_start: u32,
        len: u32,
        waiting: bool,
    },
    Spec {
        repo: String,
        spec: DiffSpec,
        bases: Vec<Base>,
    },
    Notice(String),
}

/// What the diff shows besides its files, rebuilt with them.
#[derive(Default)]
struct Layout {
    headers: HashMap<SharedString, Header>,
    /// Each file's repo and path, by its path in the diff.
    files: HashMap<SharedString, Key>,
    /// The files' paths in the diff, in order, without headings.
    order: Vec<SharedString>,
    annotations: Vec<DiffAnnotation>,
    items: HashMap<ElementId, Item>,
}

/// A virtualized view of a review's changes across repos (RV-17–RV-24).
pub struct DiffView {
    review: Rc<Review>,
    /// Each file's content hash, by repo and file index.
    hashes: Vec<Vec<u64>>,
    state: Entity<DiffState>,
    mode: DiffMode,
    long_lines: LongLines,
    show_changes: bool,
    toolbar: bool,
    annotations: Vec<Annotation>,
    renderer: Option<Rc<dyn AnnotationRenderer>>,
    collapsed_repos: HashSet<String>,
    /// Files the user collapsed or expanded, over their viewed state.
    folds: HashMap<Key, bool>,
    /// Viewed marks the user set that the review doesn't show yet.
    viewed: HashMap<Key, bool>,
    pending_specs: HashMap<String, DiffSpec>,
    editing_specs: HashSet<String>,
    /// Files shown whole because the user opened one of their folds.
    whole: HashSet<Key>,
    /// Folds waiting for their file's text: the line to open.
    waiting: HashMap<Key, u32>,
    cache: HashMap<String, Cached>,
    /// What the diff was last given: each file's path, variant and hash.
    given: Vec<(String, Variant, u64)>,
    /// The collapsed state last asked of the diff, by path.
    applied: HashMap<SharedString, bool>,
    /// The file navigation was last on, for when the top shows no line.
    last_file: Option<usize>,
    layout: Rc<Layout>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DiffEvent> for DiffView {}

impl Focusable for DiffView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.focus_handle(cx)
    }
}

impl DiffView {
    /// A view of `review`, inline, wrapping long lines.
    pub fn new(review: Review, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| DiffState::new([], cx).with_context_lines(Some(3)));
        let subscriptions = vec![
            cx.subscribe(&state, |view: &mut Self, _, event: &StateEvent, cx| {
                view.state_event(event, cx);
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        let mut view = Self {
            hashes: hashes(&review),
            review: Rc::new(review),
            state,
            mode: DiffMode::Inline,
            long_lines: LongLines::Wrap,
            show_changes: true,
            toolbar: true,
            annotations: Vec::new(),
            renderer: None,
            collapsed_repos: HashSet::new(),
            folds: HashMap::new(),
            viewed: HashMap::new(),
            pending_specs: HashMap::new(),
            editing_specs: HashSet::new(),
            whole: HashSet::new(),
            waiting: HashMap::new(),
            cache: HashMap::new(),
            given: Vec::new(),
            applied: HashMap::new(),
            last_file: None,
            layout: Rc::default(),
            _subscriptions: subscriptions,
        };
        view.sync(cx);
        view
    }

    /// Shows another version of the review, keeping the line at the top,
    /// collapsed files, opened folds and the selection where they still
    /// apply. Only files whose content changed are laid out again.
    ///
    /// Viewed marks the user set stay until the review agrees or the
    /// file's content changes; a spec the user picked shows until the
    /// repo's spec changes. A fold waiting for its file's text opens once
    /// the text is here.
    pub fn set_review(&mut self, review: Review, cx: &mut Context<Self>) {
        let old = self.review.clone();
        self.viewed.retain(|(repo, path), viewed| {
            let file = |review: &Review| {
                find(review, repo, path).map(|f| (f.content_id.clone(), f.viewed == Viewed::Yes))
            };
            match (file(&old), file(&review)) {
                (Some((before, _)), Some((after, shown))) => before == after && shown != *viewed,
                _ => false,
            }
        });
        self.pending_specs.retain(|repo, pending| {
            let spec = |review: &Review| review.repos.iter().find(|r| r.name == *repo).map(|r| r.spec.clone());
            let now = spec(&review);
            now.is_some() && now == spec(&old) && now.as_ref() != Some(pending)
        });
        self.whole.retain(|(repo, path)| find(&review, repo, path).is_some());
        let mut opened = Vec::new();
        self.waiting.retain(|key, line| match find(&review, &key.0, &key.1) {
            Some(file) if file.new_lines.is_some() => {
                opened.push((key.clone(), *line));
                false
            }
            Some(_) => true,
            None => false,
        });
        for (key, _) in &opened {
            self.whole.insert(key.clone());
        }
        self.hashes = hashes(&review);
        self.review = Rc::new(review);
        self.sync(cx);
        for (key, line) in opened {
            self.open_fold(&key, line, cx);
        }
    }

    #[must_use]
    pub fn review(&self) -> &Review {
        &self.review
    }

    pub fn set_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.sync(cx);
        cx.emit(DiffEvent::ModeChanged(mode));
    }

    #[must_use]
    pub fn mode(&self) -> DiffMode {
        self.mode
    }

    pub fn set_long_lines(&mut self, long_lines: LongLines, cx: &mut Context<Self>) {
        if self.long_lines == long_lines {
            return;
        }
        self.long_lines = long_lines;
        cx.emit(DiffEvent::LongLinesChanged(long_lines));
        cx.notify();
    }

    #[must_use]
    pub fn long_lines(&self) -> LongLines {
        self.long_lines
    }

    /// Whether Full file mode marks the changes (RV-22); without them it
    /// shows each file as it is now. Files whose text the review lacks
    /// show their changes either way.
    pub fn set_show_changes(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_changes == show {
            return;
        }
        self.show_changes = show;
        self.sync(cx);
    }

    #[must_use]
    pub fn show_changes(&self) -> bool {
        self.show_changes
    }

    /// Replaces the annotations, such as comments and drafts. Each is drawn
    /// under the last line of its target, or under its file's header when
    /// that line isn't shown.
    pub fn set_annotations(&mut self, annotations: Vec<Annotation>, cx: &mut Context<Self>) {
        self.annotations = annotations;
        self.sync(cx);
    }

    #[must_use]
    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    /// Redraws an annotation whose content changed.
    pub fn annotation_changed(&mut self, _id: AnnotationId, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub fn set_annotation_renderer(&mut self, renderer: impl AnnotationRenderer, cx: &mut Context<Self>) {
        self.renderer = Some(Rc::new(renderer));
        cx.notify();
    }

    /// Shows or hides the toolbar (modes, long lines, change navigation).
    pub fn show_toolbar(&mut self, show: bool, cx: &mut Context<Self>) {
        self.toolbar = show;
        cx.notify();
    }

    /// Collapses a repo to its header, or expands it.
    pub fn set_repo_collapsed(&mut self, repo: &str, collapsed: bool, cx: &mut Context<Self>) {
        let changed = if collapsed {
            self.collapsed_repos.insert(repo.to_owned())
        } else {
            self.collapsed_repos.remove(repo)
        };
        if changed {
            self.sync(cx);
        }
    }

    /// Collapses a file to its header, or expands it, whatever its viewed
    /// state.
    pub fn set_file_collapsed(&mut self, repo: &str, path: &str, collapsed: bool, cx: &mut Context<Self>) {
        self.folds.insert((repo.to_owned(), path.to_owned()), collapsed);
        let at = SharedString::from(patch::key(repo, path));
        self.applied.insert(at.clone(), collapsed);
        self.state.update(cx, |state, cx| state.set_file_collapsed(&at, collapsed, cx));
    }

    /// Whether a file shows only its header.
    #[must_use]
    pub fn is_file_collapsed(&self, repo: &str, path: &str, cx: &App) -> bool {
        self.state.read(cx).is_file_collapsed(&patch::key(repo, path))
    }

    /// Marks a file viewed or not (RV-29): a viewed file folds to its
    /// header. Sends [`DiffEvent::ViewedChanged`]. Returns whether the file
    /// is in the review.
    pub fn set_viewed(&mut self, repo: &str, path: &str, viewed: bool, cx: &mut Context<Self>) -> bool {
        let Some(file) = find(&self.review, repo, path) else {
            return false;
        };
        if self.is_viewed(repo, path) == viewed {
            return true;
        }
        let content_id = file.content_id.clone();
        let key = (repo.to_owned(), path.to_owned());
        self.viewed.insert(key.clone(), viewed);
        self.folds.remove(&key);
        self.applied.remove(patch::key(repo, path).as_str());
        self.sync(cx);
        cx.emit(DiffEvent::ViewedChanged {
            repo: repo.to_owned(),
            path: path.to_owned(),
            viewed,
            content_id,
        });
        true
    }

    /// Whether the file counts as viewed, with the user's latest mark.
    #[must_use]
    pub fn is_viewed(&self, repo: &str, path: &str) -> bool {
        let key = (repo.to_owned(), path.to_owned());
        self.viewed
            .get(&key)
            .copied()
            .unwrap_or_else(|| find(&self.review, repo, path).is_some_and(|file| file.viewed == Viewed::Yes))
    }

    /// The spec a repo's header shows: the one the user picked, until the
    /// review catches up, or the review's.
    #[must_use]
    pub fn spec(&self, repo: &str) -> Option<&DiffSpec> {
        self.pending_specs
            .get(repo)
            .or_else(|| self.review.repos.iter().find(|r| r.name == repo).map(|r| &r.spec))
    }

    /// Changes what a repo's diff compares, as if the user edited it: the
    /// header shows `spec` and [`DiffEvent::SpecChanged`] asks for its diff.
    pub fn set_spec(&mut self, repo: &str, spec: DiffSpec, cx: &mut Context<Self>) {
        if self.spec(repo) == Some(&spec) {
            return;
        }
        let shown = self.review.repos.iter().find(|r| r.name == repo).map(|r| &r.spec);
        if shown == Some(&spec) {
            self.pending_specs.remove(repo);
        } else {
            self.pending_specs.insert(repo.to_owned(), spec.clone());
        }
        self.sync(cx);
        cx.emit(DiffEvent::SpecChanged {
            repo: repo.to_owned(),
            spec,
        });
    }

    /// Opens or closes the spec editor under a repo's header.
    pub fn edit_spec(&mut self, repo: &str, open: bool, cx: &mut Context<Self>) {
        let changed = if open {
            self.editing_specs.insert(repo.to_owned())
        } else {
            self.editing_specs.remove(repo)
        };
        if changed {
            self.sync(cx);
        }
    }

    /// The selected lines, if any.
    #[must_use]
    pub fn selection(&self, cx: &App) -> Option<LineTarget> {
        self.state.read(cx).selected_lines().and_then(|range| self.target(&range))
    }

    /// Selects `target`'s lines and scrolls to them. Returns whether the
    /// lines are shown.
    pub fn select(&mut self, target: &LineTarget, cx: &mut Context<Self>) -> bool {
        if !self.scroll_to_line(target, cx) {
            return false;
        }
        let range = DiffLineRange::new(
            patch::key(&target.repo, &target.path),
            diff_side(target.side),
            target.lines.start as usize,
            target.lines.end as usize,
        );
        self.state.update(cx, |state, cx| state.set_selected_lines(Some(range), cx));
        cx.emit(DiffEvent::SelectionChanged(Some(target.clone())));
        true
    }

    /// Scrolls to `target`'s first line, opening its repo, its file and
    /// the fold around it. Returns whether the line is shown: a line in a
    /// fold of a file whose text the review lacks isn't.
    pub fn scroll_to_line(&mut self, target: &LineTarget, cx: &mut Context<Self>) -> bool {
        let key = (target.repo.clone(), target.path.clone());
        let Some(file) = find(&self.review, &target.repo, &target.path) else {
            return false;
        };
        let has_text = file.new_lines.is_some();
        let in_hunks = order::shown(file, false, target.side, target.lines.start);
        let shown = in_hunks || (has_text && order::shown(file, true, target.side, target.lines.start));
        if !shown {
            return false;
        }
        let mut changed = self.collapsed_repos.remove(&target.repo);
        changed |= !in_hunks && self.whole.insert(key.clone());
        changed |= self.folds.insert(key, false) != Some(false);
        if changed {
            self.sync(cx);
        }
        let position = DiffLinePosition::new(
            patch::key(&target.repo, &target.path),
            diff_side(target.side),
            target.lines.start as usize,
        );
        self.state.update(cx, |state, cx| state.scroll_to_line(position, cx));
        true
    }

    /// The line at the top of the viewport, or the first line under the
    /// header there.
    #[must_use]
    pub fn top_line(&self, cx: &App) -> Option<LineTarget> {
        let position = self.state.read(cx).top_line()?;
        let (repo, path) = self.layout.files.get(position.path())?.clone();
        Some(LineTarget {
            repo,
            path,
            side: side(position.side()),
            lines: LineRange::single(u32::try_from(position.line()).ok()?),
        })
    }

    /// Which change is at the top, counted from 1 (`None` above the first),
    /// and how many there are: each run of changed lines is one.
    #[must_use]
    pub fn change_position(&self, cx: &App) -> (Option<usize>, usize) {
        let (at, total) = self.state.read(cx).change_position();
        (at.map(|at| at + 1), total)
    }

    pub fn next_change(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.next_change(cx));
    }

    pub fn previous_change(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.previous_change(cx));
    }

    pub fn next_file(&mut self, cx: &mut Context<Self>) {
        self.go_to_file(true, cx);
    }

    pub fn previous_file(&mut self, cx: &mut Context<Self>) {
        self.go_to_file(false, cx);
    }

    /// The diff's state, for timing the view against the diff alone.
    #[doc(hidden)]
    #[must_use]
    pub fn diff_state(&self) -> &Entity<DiffState> {
        &self.state
    }

    fn go_to_file(&mut self, forward: bool, cx: &mut Context<Self>) {
        let order = &self.layout.order;
        if order.is_empty() {
            return;
        }
        let top = self.state.read(cx).top_line();
        let current = top
            .and_then(|position| order.iter().position(|path| path == position.path()))
            .or(self.last_file);
        let next = match (current, forward) {
            (None, _) => 0,
            (Some(at), true) => (at + 1).min(order.len() - 1),
            (Some(at), false) => at.saturating_sub(1),
        };
        self.last_file = Some(next);
        let path = order[next].clone();
        self.state.update(cx, |state, cx| state.scroll_to_file(&path, cx));
    }

    fn state_event(&mut self, event: &StateEvent, cx: &mut Context<Self>) {
        match event {
            StateEvent::FileCollapsed(path) | StateEvent::FileExpanded(path) => {
                let collapsed = matches!(event, StateEvent::FileCollapsed(_));
                self.applied.insert(path.clone(), collapsed);
                if let Some(key) = self.layout.files.get(path) {
                    self.folds.insert(key.clone(), collapsed);
                }
            }
            StateEvent::SelectionChanged(range) => {
                let target = range.as_ref().and_then(|range| self.target(range));
                cx.emit(DiffEvent::SelectionChanged(target));
            }
            _ => {}
        }
    }

    /// The lines a range of the diff covers: on the side it ends on, all of
    /// them when it starts there too, otherwise its last line.
    fn target(&self, range: &DiffLineRange) -> Option<LineTarget> {
        let (repo, path) = self.layout.files.get(range.path())?.clone();
        let end = u32::try_from(range.end()).ok()?;
        let lines = if range.side() == range.end_side() {
            LineRange::between(u32::try_from(range.start()).ok()?, end)
        } else {
            LineRange::single(end)
        };
        Some(LineTarget {
            repo,
            path,
            side: side(range.end_side()),
            lines,
        })
    }

    fn anchor(&self, target: LineTarget) -> Option<CommentAnchor> {
        let repo = self.review.repos.iter().find(|r| r.name == target.repo)?;
        let file = repo.files.iter().find(|f| f.path == target.path)?;
        let quoted: Vec<&str> = (target.lines.start..=target.lines.end)
            .filter_map(|line| order::line_text(file, target.side, line))
            .collect();
        Some(CommentAnchor {
            spec: repo.spec.clone(),
            base_commit: repo.base_commit.clone(),
            quoted: quoted.join("\n"),
            target,
        })
    }

    fn request_comment(&mut self, range: &DiffLineRange, cx: &mut Context<Self>) {
        if let Some(anchor) = self.target(range).and_then(|target| self.anchor(target)) {
            cx.emit(DiffEvent::CommentRequested(anchor));
        }
    }

    fn comment_on_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(range) = self.state.read(cx).selected_lines() {
            self.request_comment(&range, cx);
        }
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.state.read(cx).selected_lines().is_none() {
            return;
        }
        self.state.update(cx, |state, cx| state.set_selected_lines(None, cx));
        cx.emit(DiffEvent::SelectionChanged(None));
    }

    /// Opens a fold the view drew: with the file's text, the file is shown
    /// whole and the diff's own fold there opens; without it, the text is
    /// asked for.
    fn show_gap(&mut self, key: Key, new_start: u32, len: u32, cx: &mut Context<Self>) {
        let Some(file) = find(&self.review, &key.0, &key.1) else {
            return;
        };
        let middle = new_start + len / 2;
        if file.new_lines.is_some() {
            self.whole.insert(key.clone());
            self.sync(cx);
            self.open_fold(&key, middle, cx);
        } else {
            self.waiting.insert(key.clone(), middle);
            self.sync(cx);
            cx.emit(DiffEvent::ContextNeeded { repo: key.0, path: key.1 });
        }
    }

    fn open_fold(&mut self, key: &Key, line: u32, cx: &mut Context<Self>) {
        let position = DiffLinePosition::new(patch::key(&key.0, &key.1), DiffSide::Modified, line as usize);
        self.state.update(cx, |state, cx| {
            state.expand_unchanged_at(&position, cx);
        });
    }

    fn variant(&self, key: &Key, file: &FileDiff) -> Variant {
        let has_text = file.new_lines.is_some();
        match self.mode {
            DiffMode::FullFile if !self.show_changes && has_text => Variant::Text,
            DiffMode::FullFile => Variant::Full,
            _ if has_text && self.whole.contains(key) => Variant::Full,
            _ => Variant::Hunks,
        }
    }

    /// Hands the diff the review's files as they should show now, and
    /// rebuilds the headers and annotations. Files the diff has already
    /// keep their rows.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let review = self.review.clone();
        let entries = order::entries(&review, &self.collapsed_repos);
        let mut layout = Layout::default();
        let mut files = Vec::with_capacity(entries.len());
        let mut given = Vec::with_capacity(entries.len());
        let mut collapse: Vec<(SharedString, bool)> = Vec::new();
        let mut comments: HashMap<(&str, &str), Vec<&Annotation>> = HashMap::new();
        for annotation in &self.annotations {
            let target = &annotation.target;
            comments.entry((target.repo.as_str(), target.path.as_str())).or_default().push(annotation);
        }
        let mut gaps = Vec::new();
        for entry in &entries {
            let Some(path) = entry.path(&review) else {
                continue;
            };
            let at = SharedString::from(path.clone());
            let (hash, variant) = match *entry {
                Entry::Repo { repo } => {
                    let Some(repo) = review.repos.get(repo) else {
                        continue;
                    };
                    self.repo_header(repo, &at, &mut layout);
                    (0, Variant::Heading)
                }
                Entry::Group { group, files } => {
                    let title = review
                        .guide
                        .as_ref()
                        .and_then(|guide| guide.groups.get(group))
                        .map_or_else(|| "Other changes".to_owned(), |group| group.title.clone());
                    layout.headers.insert(at.clone(), Header::Group { title, count: count(files, "file") });
                    (0, Variant::Heading)
                }
                Entry::File { repo: repo_ix, file: file_ix } => {
                    let (Some(repo), Some(hash)) = (review.repos.get(repo_ix), self.hashes.get(repo_ix).and_then(|h| h.get(file_ix)))
                    else {
                        continue;
                    };
                    let Some(file) = repo.files.get(file_ix) else {
                        continue;
                    };
                    let key = (repo.name.clone(), file.path.clone());
                    let variant = self.variant(&key, file);
                    let viewed = self.is_viewed(&repo.name, &file.path);
                    let collapsed = self.folds.get(&key).copied().unwrap_or(viewed);
                    if self.applied.get(&at) != Some(&collapsed) {
                        collapse.push((at.clone(), collapsed));
                    }
                    let totals = file.totals();
                    layout.headers.insert(
                        at.clone(),
                        Header::File {
                            repo: repo.name.clone(),
                            path: file.path.clone(),
                            status: match &file.status {
                                FileStatus::Renamed { from } => format!("renamed from {from}"),
                                status => status.label().to_owned(),
                            },
                            counts: format!("+{} −{}", totals.added, totals.removed),
                            viewed,
                            changed_since: file.viewed == Viewed::ChangedSince && !viewed,
                        },
                    );
                    layout.files.insert(at.clone(), key.clone());
                    layout.order.push(at.clone());
                    let whole = variant != Variant::Hunks && file.new_lines.is_some();
                    for annotation in comments.get(&(repo.name.as_str(), file.path.as_str())).into_iter().flatten() {
                        let id = ElementId::NamedInteger("comment".into(), annotation.id.0);
                        let target = &annotation.target;
                        let place = place(order::place_lines(file, whole, target), variant);
                        layout.annotations.push(diff_annotation(id.clone(), &at, place));
                        layout.items.insert(id, Item::Comment(annotation.id));
                    }
                    if let Some(guide) = &review.guide {
                        for (ix, note) in guide.notes.iter().enumerate() {
                            if note.target.file() != (repo.name.as_str(), file.path.as_str()) {
                                continue;
                            }
                            let (spot, outdated) = order::place_note(file, whole, &note.target);
                            let id = ElementId::NamedInteger("note".into(), ix as u64);
                            layout.annotations.push(diff_annotation(id.clone(), &at, place(spot, variant)));
                            layout.items.insert(
                                id,
                                Item::Note {
                                    label: note_label(&note.target, outdated),
                                    text: note.text.clone(),
                                    author: guide.author.clone(),
                                    outdated,
                                },
                            );
                        }
                    }
                    if !whole {
                        for gap in order::gaps(file) {
                            let id = ElementId::NamedInteger(format!("gap {at}").into(), u64::from(gap.new_start));
                            gaps.push(diff_annotation(id.clone(), &at, gap.place));
                            layout.items.insert(
                                id,
                                Item::Gap {
                                    waiting: self.waiting.contains_key(&key),
                                    key: key.clone(),
                                    new_start: gap.new_start,
                                    len: gap.len,
                                },
                            );
                        }
                    }
                    (*hash, variant)
                }
            };
            let cached = self.cache.get(&path).filter(|c| c.hash == hash && c.variant == variant);
            if cached.is_none() {
                let built = match *entry {
                    Entry::File { repo, file } => review
                        .repos
                        .get(repo)
                        .and_then(|r| r.files.get(file).map(|f| build(&r.name, f, variant))),
                    _ => None,
                };
                let (file, notice) = built.unwrap_or_else(|| (DiffFile::unchanged(at.clone(), ""), None));
                self.cache.insert(path.clone(), Cached { hash, variant, file, notice });
            }
            let Some(cached) = self.cache.get(&path) else {
                continue;
            };
            if let Some(notice) = &cached.notice {
                let id = ElementId::NamedInteger(format!("notice {at}").into(), 0);
                layout.annotations.push(DiffAnnotation::file(id.clone(), at.clone()));
                layout.items.insert(id, Item::Notice(notice.clone()));
            }
            if variant == Variant::Heading && self.applied.get(&at) != Some(&true) {
                collapse.push((at.clone(), true));
            }
            files.push(cached.file.clone());
            given.push((path, variant, hash));
        }
        layout.annotations.extend(gaps);
        let live: HashSet<&str> = given.iter().map(|(path, _, _)| path.as_str()).collect();
        self.cache.retain(|path, _| live.contains(path.as_str()));
        self.applied.retain(|path, _| live.contains(path.as_ref()));
        let (columns, context) = match self.mode {
            DiffMode::Inline => (Columns::Unified, Some(3)),
            DiffMode::SideBySide => (Columns::Split, Some(3)),
            DiffMode::FullFile => (Columns::Unified, None),
        };
        let changed = given != self.given;
        self.given = given;
        self.state.update(cx, |state, cx| {
            if changed {
                state.update_files(files, cx);
            }
            if state.mode() != columns {
                state.set_mode(columns, cx);
            }
            if state.context_lines() != context {
                state.set_context_lines(context, cx);
            }
            if !collapse.is_empty() {
                state.set_files_collapsed(collapse.iter().map(|(path, collapsed)| (path.as_ref(), *collapsed)), cx);
            }
        });
        for (path, collapsed) in collapse {
            self.applied.insert(path, collapsed);
        }
        self.layout = Rc::new(layout);
        cx.notify();
    }

    fn repo_header(&self, repo: &crate::model::RepoDiff, at: &SharedString, layout: &mut Layout) {
        let name = repo.name.clone();
        let totals = repo.totals();
        let editing = self.editing_specs.contains(&name);
        let spec = self.spec(&name).cloned().unwrap_or_default();
        layout.headers.insert(
            at.clone(),
            Header::Repo {
                detail: format!(
                    "{} · {} · +{} −{}",
                    repo.branch,
                    count(repo.files.len(), "file"),
                    totals.added,
                    totals.removed
                ),
                comparing: format!("Comparing: {spec}"),
                pending: self.pending_specs.contains_key(&name),
                editing,
                expanded: !self.collapsed_repos.contains(&name),
                name: name.clone(),
            },
        );
        if editing {
            let mut bases = vec![spec.base.clone(), repo.spec.base.clone()];
            for base in &repo.bases {
                if !bases.contains(base) {
                    bases.push(base.clone());
                }
            }
            bases.dedup();
            let id = ElementId::NamedInteger(format!("spec {name}").into(), 0);
            layout.annotations.push(DiffAnnotation::file(id.clone(), at.clone()));
            layout.items.insert(id, Item::Spec { repo: name, spec, bases });
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Colours::of(cx);
        let button = move |id: &'static str, label: SharedString, selected: Option<bool>| {
            div()
                .id(id)
                .test_support()
                .role(Role::Button)
                .aria_label(label.clone())
                .when_some(selected, |button, selected| button.aria_selected(selected))
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded_sm()
                .cursor_pointer()
                .when(selected == Some(true), |button| button.bg(theme.accent))
                .child(label.clone())
                .test_text(label)
        };
        let modes = DiffMode::ALL.map(|mode| {
            let id = match mode {
                DiffMode::Inline => "mode-inline",
                DiffMode::SideBySide => "mode-side-by-side",
                DiffMode::FullFile => "mode-full-file",
            };
            button(id, mode.label().into(), Some(self.mode == mode))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_mode(mode, cx)))
        });
        let wrapping = [LongLines::Wrap, LongLines::Scroll].map(|long_lines| {
            let id = match long_lines {
                LongLines::Wrap => "long-wrap",
                LongLines::Scroll => "long-scroll",
            };
            button(id, long_lines.label().into(), Some(self.long_lines == long_lines))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_long_lines(long_lines, cx)))
        });
        let show = self.show_changes;
        let show_changes = (self.mode == DiffMode::FullFile).then(|| {
            button("show-changes", "Show changes".into(), Some(show))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| view.set_show_changes(!show, cx)))
        });
        let counter = match self.change_position(cx) {
            (_, 0) => "No changes".to_owned(),
            (Some(current), total) => format!("{current} of {total}"),
            (None, total) => format!("– of {total}"),
        };
        let credit = self.review.guide.as_ref().map(|guide| {
            if guide.author.is_empty() {
                "Prepared review".to_owned()
            } else {
                format!("Prepared by {}", guide.author)
            }
        });
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .h(px(30.))
            .bg(theme.secondary)
            .border_b_1()
            .border_color(theme.border)
            .children(modes)
            .child(div().w(px(12.)))
            .children(wrapping)
            .children(show_changes)
            .child(div().flex_1())
            .when_some(credit, |toolbar, credit| {
                toolbar.child(div().px(px(6.)).text_color(theme.muted).child(credit.clone()).test_text(credit))
            })
            .child(
                button("previous-file", "Previous file".into(), None)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.previous_file(cx))),
            )
            .child(
                button("next-file", "Next file".into(), None)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.next_file(cx))),
            )
            .child(
                button("previous-change", "Previous change".into(), None)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.previous_change(cx))),
            )
            .child(
                div()
                    .id("change-counter")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Change")
                    .px(px(4.))
                    .text_color(theme.muted)
                    .child(counter.clone())
                    .test_text(counter),
            )
            .child(
                button("next-change", "Next change".into(), None)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.next_change(cx))),
            )
            .into_any_element()
    }
}

impl Render for DiffView {
    #[allow(deprecated)]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Colours::of(cx);
        let view = cx.entity().downgrade();
        let headers = {
            let (layout, view) = (self.layout.clone(), view.clone());
            move |file: &DiffFile, _: &mut Window, cx: &mut App| header(&layout, &view, file, cx)
        };
        let items = {
            let (layout, view, renderer) = (self.layout.clone(), view.clone(), self.renderer.clone());
            move |annotation: &DiffAnnotation, window: &mut Window, cx: &mut App| {
                item(&layout, &view, renderer.as_deref(), annotation, window, cx)
            }
        };
        let comment = {
            let view = view.clone();
            move |range: &DiffLineRange, _: &mut Window, cx: &mut App| {
                view.update(cx, |view, cx| view.request_comment(range, cx)).ok();
            }
        };
        let diff = Diff::new(&self.state)
            .soft_wrap(self.long_lines == LongLines::Wrap)
            .hunk_separator(DiffHunkSeparator::Simple)
            .header_collapsible(|file| !order::is_heading(file.path()))
            .annotations(self.layout.annotations.iter().cloned())
            .render_header(headers)
            .render_annotation(items)
            .on_add_annotation(comment)
            .size_full();
        div()
            .id("diff-view")
            .test_support()
            .role(Role::Group)
            .aria_label("Changes")
            .key_context(KEY_CONTEXT)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.text)
            .on_action(cx.listener(|view, _: &NextChange, _, cx| view.next_change(cx)))
            .on_action(cx.listener(|view, _: &PreviousChange, _, cx| view.previous_change(cx)))
            .on_action(cx.listener(|view, _: &NextFile, _, cx| view.next_file(cx)))
            .on_action(cx.listener(|view, _: &PreviousFile, _, cx| view.previous_file(cx)))
            .on_action(cx.listener(|view, _: &CommentOnSelection, _, cx| view.comment_on_selection(cx)))
            .on_action(cx.listener(|view, _: &ClearSelection, _, cx| view.clear_selection(cx)))
            // The counter reads the scroll position, which the diff changes
            // without notifying anyone.
            .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| cx.notify()))
            .when(self.toolbar, |view| view.child(self.toolbar(cx)))
            .child(div().flex_1().min_h(px(0.)).w_full().child(diff))
    }
}

/// The theme's colours the view uses.
#[derive(Clone, Copy)]
struct Colours {
    background: Hsla,
    text: Hsla,
    muted: Hsla,
    border: Hsla,
    accent: Hsla,
    secondary: Hsla,
    warning: Hsla,
    added: Hsla,
    note: Hsla,
}

impl Colours {
    fn of(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            background: theme.background,
            text: theme.foreground,
            muted: theme.muted_foreground,
            border: theme.border,
            accent: theme.accent,
            secondary: theme.secondary,
            warning: theme.warning,
            added: theme.success,
            note: theme.muted,
        }
    }
}

fn header(layout: &Layout, view: &WeakEntity<DiffView>, file: &DiffFile, cx: &mut App) -> AnyElement {
    let theme = Colours::of(cx);
    let Some(header) = layout.headers.get(file.path()) else {
        return div().into_any_element();
    };
    let row = || {
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .whitespace_nowrap()
            .overflow_hidden()
    };
    let text = |text: String, colour: Hsla| div().text_color(colour).child(text.clone()).test_text(text);
    match header {
        Header::Repo {
            name,
            detail,
            comparing,
            pending,
            editing,
            expanded,
        } => {
            let (toggle, edit) = (view.clone(), view.clone());
            let (toggle_name, edit_name) = (name.clone(), name.clone());
            let (expanded, editing) = (*expanded, *editing);
            let shown = format!(
                "{comparing}{} {}",
                if *pending { " (updating)" } else { "" },
                if editing { "▴" } else { "▾" }
            );
            row()
                .child(
                    div()
                        .id(SharedString::from(format!("repo {name}")))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(name.clone())
                        .aria_expanded(expanded)
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            toggle.update(cx, |view, cx| view.set_repo_collapsed(&toggle_name, expanded, cx)).ok();
                        })
                        .child(div().child(if expanded { "▾" } else { "▸" }))
                        .child(text(name.clone(), theme.text))
                        .child(text(detail.clone(), theme.muted)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("spec {name}")))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(comparing.clone())
                        .aria_expanded(editing)
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_color(if *pending { theme.warning } else { theme.muted })
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            edit.update(cx, |view, cx| view.edit_spec(&edit_name, !editing, cx)).ok();
                        })
                        .child(shown.clone())
                        .test_text(shown),
                )
                .into_any_element()
        }
        Header::Group { title, count } => row()
            .id(SharedString::from(format!("group {}", file.path())))
            .test_support()
            .role(Role::Heading)
            .aria_label(title.clone())
            .child(text(title.clone(), theme.text))
            .child(text(count.clone(), theme.muted))
            .into_any_element(),
        Header::File {
            repo,
            path,
            status,
            counts,
            viewed,
            changed_since,
        } => {
            let viewed = *viewed;
            let (mark, open) = (view.clone(), view.clone());
            let (mark_repo, mark_path) = (repo.clone(), path.clone());
            let (open_repo, open_path) = (repo.clone(), path.clone());
            row()
                .child(
                    div()
                        .px(px(4.))
                        .rounded_sm()
                        .bg(theme.secondary)
                        .text_color(theme.muted)
                        .child(repo.clone())
                        .test_text(repo.clone()),
                )
                .child(div().min_w(px(0.)).overflow_hidden().child(path.clone()).test_text(path.clone()))
                .child(text(status.clone(), theme.muted))
                .child(text(counts.clone(), theme.added))
                .when(*changed_since, |row| row.child(text("changed since viewed".to_owned(), theme.warning)))
                .child(div().flex_1())
                .child(
                    div()
                        .id(SharedString::from(format!("viewed {repo}/{path}")))
                        .test_support()
                        .role(Role::CheckBox)
                        .aria_label(format!("Viewed: {repo}/{path}"))
                        .aria_toggled(if viewed { Toggled::True } else { Toggled::False })
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .text_color(if viewed { theme.text } else { theme.muted })
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            mark.update(cx, |view, cx| view.set_viewed(&mark_repo, &mark_path, !viewed, cx)).ok();
                        })
                        .child(
                            div()
                                .size(px(10.))
                                .rounded_sm()
                                .border_1()
                                .border_color(theme.muted)
                                .when(viewed, |checkbox| checkbox.bg(theme.accent)),
                        )
                        .child("Viewed")
                        .test_text("Viewed"),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("open {repo}/{path}")))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(format!("Open big: {repo}/{path}"))
                        .flex_none()
                        .text_color(theme.muted)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            open.update(cx, |_, cx| {
                                cx.emit(DiffEvent::OpenFile {
                                    repo: open_repo.clone(),
                                    path: open_path.clone(),
                                });
                            })
                            .ok();
                        })
                        .child("Open big")
                        .test_text("Open big"),
                )
                .into_any_element()
        }
    }
}

fn item(
    layout: &Layout,
    view: &WeakEntity<DiffView>,
    renderer: Option<&dyn AnnotationRenderer>,
    annotation: &DiffAnnotation,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = Colours::of(cx);
    let Some(item) = layout.items.get(annotation.id()) else {
        return div().into_any_element();
    };
    match item {
        Item::Comment(id) => match renderer {
            Some(renderer) => renderer.render(*id, window, cx),
            None => div().into_any_element(),
        },
        Item::Note {
            label,
            text,
            author,
            outdated,
        } => {
            let paragraphs = text.lines().map(|line| div().child(line.to_owned()).test_text(line.to_owned()));
            div()
                .id(annotation.id().clone())
                .test_support()
                .role(Role::Note)
                .aria_label(label.clone())
                .m(px(4.))
                .px(px(8.))
                .py(px(4.))
                .rounded_md()
                .bg(theme.note)
                .border_l_2()
                .border_color(if *outdated { theme.warning } else { theme.accent })
                .flex()
                .flex_col()
                .when(*outdated, |note| {
                    let outdated = "Outdated: the code changed since this note";
                    note.child(div().text_color(theme.warning).child(outdated).test_text(outdated))
                })
                .children(paragraphs)
                .when(!author.is_empty(), |note| {
                    let credit = format!("— {author}");
                    note.child(div().text_color(theme.muted).child(credit.clone()).test_text(credit))
                })
                .into_any_element()
        }
        Item::Gap {
            key,
            new_start,
            len,
            waiting,
        } => {
            let lines = if *len == 1 { "line" } else { "lines" };
            let shown = if *waiting {
                format!("⋯ {len} unchanged {lines} — loading")
            } else {
                format!("⋯ {len} unchanged {lines} — show")
            };
            let label = format!(
                "Show {len} unchanged {lines}, {}/{} {new_start}–{}",
                key.0,
                key.1,
                new_start + len - 1
            );
            let (view, key, new_start, len) = (view.clone(), key.clone(), *new_start, *len);
            div()
                .id(annotation.id().clone())
                .test_support()
                .role(Role::Button)
                .aria_label(label)
                .w_full()
                .px(px(12.))
                .py(px(2.))
                .bg(theme.secondary)
                .text_color(theme.muted)
                .whitespace_nowrap()
                .overflow_hidden()
                .when(!*waiting, |gap| {
                    gap.cursor_pointer().on_click(move |_, _, cx| {
                        view.update(cx, |view, cx| view.show_gap(key.clone(), new_start, len, cx)).ok();
                    })
                })
                .child(shown.clone())
                .test_text(shown)
                .into_any_element()
        }
        Item::Spec { repo, spec, bases } => spec_editor(view, repo, spec, bases, theme),
        Item::Notice(notice) => div()
            .id(annotation.id().clone())
            .test_support()
            .role(Role::Status)
            .aria_label(notice.clone())
            .px(px(12.))
            .py(px(4.))
            .text_color(theme.warning)
            .child(notice.clone())
            .test_text(notice.clone())
            .into_any_element(),
    }
}

/// The spec editor under a repo's header (RV-31): the base, how much of
/// the working tree, and whether whitespace and renames count. Every pick
/// sends [`DiffEvent::SpecChanged`].
fn spec_editor(view: &WeakEntity<DiffView>, name: &str, spec: &DiffSpec, bases: &[Base], theme: Colours) -> AnyElement {
    let choice = |id: String, role: Role, label: String, shown: String, on: bool, next: Option<DiffSpec>| {
        let (view, name) = (view.clone(), name.to_owned());
        div()
            .id(SharedString::from(id))
            .test_support()
            .role(role)
            .aria_label(label)
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .px(px(6.))
            .h(px(20.))
            .flex()
            .items_center()
            .rounded_sm()
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .when(on, |choice| choice.bg(theme.accent))
            .when_some(next, |choice, next| {
                choice.on_click(move |_, _, cx| {
                    view.update(cx, |view, cx| view.set_spec(&name, next.clone(), cx)).ok();
                })
            })
            .child(shown.clone())
            .test_text(shown)
    };
    let base_choices = bases.iter().enumerate().map(|(at, base)| {
        let next = DiffSpec {
            base: base.clone(),
            ..spec.clone()
        };
        choice(
            format!("spec {name} base {at}"),
            Role::RadioButton,
            format!("Base: {base}"),
            base.to_string(),
            *base == spec.base,
            Some(next),
        )
    });
    let include_choices = Include::LEVELS.into_iter().enumerate().map(|(at, level)| {
        let on = spec.include.same_level(&level);
        let next = (!on).then(|| DiffSpec {
            include: level.clone(),
            ..spec.clone()
        });
        choice(
            format!("spec {name} include {at}"),
            Role::RadioButton,
            level.label().to_owned(),
            level.label().to_owned(),
            on,
            next,
        )
    });
    let whitespace = DiffSpec {
        ignore_whitespace: !spec.ignore_whitespace,
        ..spec.clone()
    };
    let renames = DiffSpec {
        detect_renames: !spec.detect_renames,
        ..spec.clone()
    };
    let toggles = [
        ("whitespace", "Ignore whitespace", spec.ignore_whitespace, whitespace),
        ("renames", "Detect renames", spec.detect_renames, renames),
    ]
    .map(|(id, label, on, next)| {
        choice(format!("spec {name} {id}"), Role::CheckBox, label.to_owned(), label.to_owned(), on, Some(next))
    });
    let paths = if spec.paths.is_empty() {
        "Paths: all".to_owned()
    } else {
        format!("Paths: {}", spec.paths.join(", "))
    };
    let line = |label: &'static str| {
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .child(div().w(px(64.)).text_color(theme.muted).child(label).test_text(label))
    };
    let (done_view, done_name) = (view.clone(), name.to_owned());
    div()
        .id(SharedString::from(format!("spec editor {name}")))
        .test_support()
        .role(Role::Group)
        .aria_label(format!("What {name} compares"))
        .w_full()
        .p(px(8.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .bg(theme.note)
        .child(line("Base").children(base_choices))
        .child(line("Compare").children(include_choices))
        .child(
            line("Count")
                .children(toggles)
                .child(div().px(px(6.)).text_color(theme.muted).child(paths.clone()).test_text(paths)),
        )
        .child(
            div().flex().flex_row().child(
                div()
                    .id(SharedString::from(format!("spec {name} done")))
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Done")
                    .px(px(8.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .rounded_sm()
                    .bg(theme.accent)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        done_view.update(cx, |view, cx| view.edit_spec(&done_name, false, cx)).ok();
                    })
                    .child("Done")
                    .test_text("Done"),
            ),
        )
        .into_any_element()
}

/// The file's patch as the diff holds it, and why its changes aren't
/// shown when the patch doesn't parse.
fn build(repo: &str, file: &FileDiff, variant: Variant) -> (DiffFile, Option<String>) {
    let path = patch::key(repo, &file.path);
    if variant == Variant::Text
        && let Some(text) = patch::file_text(file)
    {
        return (DiffFile::unchanged(path, &text), None);
    }
    let context = if variant == Variant::Hunks { patch::Context::Hunks } else { patch::Context::Full };
    match DiffFile::parse(&patch::file_patch(repo, file, context)) {
        Ok(mut files) if !files.is_empty() => (files.swap_remove(0), None),
        Ok(_) => (DiffFile::unchanged(path, ""), None),
        Err(error) => {
            let notice = format!(
                "The changes to {path} can't be shown: their patch doesn't parse ({error:?}). Refresh the review; if this keeps happening, open the file big to read it whole."
            );
            (DiffFile::unchanged(path, ""), Some(notice))
        }
    }
}

/// Where an annotation goes in the diff: a file shown as it is now has
/// only its new side, so what is on the old side goes under the header.
fn place(place: Place, variant: Variant) -> Place {
    match place {
        Place::Line { side: Side::Old, .. } if variant == Variant::Text => Place::File,
        place => place,
    }
}

fn diff_annotation(id: ElementId, path: &SharedString, place: Place) -> DiffAnnotation {
    match place {
        Place::File => DiffAnnotation::file(id, path.clone()),
        Place::Line { side, line } => {
            DiffAnnotation::line(id, DiffLinePosition::new(path.clone(), diff_side(side), line as usize))
        }
    }
}

fn note_label(target: &NoteTarget, outdated: bool) -> String {
    let place = match target {
        NoteTarget::File(file) => format!("{}/{}", file.repo, file.path),
        NoteTarget::Hunk { file, line, side } => {
            let side = if *side == Side::Old { " (old)" } else { "" };
            format!("{}/{} hunk at {line}{side}", file.repo, file.path)
        }
        NoteTarget::Lines { target, .. } => target.to_string(),
    };
    if outdated {
        format!("Note on {place}, outdated")
    } else {
        format!("Note on {place}")
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

fn diff_side(side: Side) -> DiffSide {
    match side {
        Side::Old => DiffSide::Original,
        Side::New => DiffSide::Modified,
    }
}

fn side(side: DiffSide) -> Side {
    match side {
        DiffSide::Original => Side::Old,
        DiffSide::Modified => Side::New,
    }
}

fn find<'a>(review: &'a Review, repo: &str, path: &str) -> Option<&'a FileDiff> {
    review.repos.iter().find(|r| r.name == repo)?.files.iter().find(|f| f.path == path)
}

/// What a file's patch is made of, hashed, so a new review rebuilds only
/// the files that changed.
fn hashes(review: &Review) -> Vec<Vec<u64>> {
    review
        .repos
        .iter()
        .map(|repo| {
            repo.files
                .iter()
                .map(|file| {
                    let mut hasher = DefaultHasher::new();
                    repo.name.hash(&mut hasher);
                    file.path.hash(&mut hasher);
                    file.status.label().hash(&mut hasher);
                    if let FileStatus::Renamed { from } = &file.status {
                        from.hash(&mut hasher);
                    }
                    for hunk in &file.hunks {
                        (hunk.old_start, hunk.new_start).hash(&mut hasher);
                        for line in &hunk.lines {
                            line.kind.hash(&mut hasher);
                            line.text.hash(&mut hasher);
                        }
                    }
                    file.new_lines.hash(&mut hasher);
                    hasher.finish()
                })
                .collect()
        })
        .collect()
}
