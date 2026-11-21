//! A markdown file open as a session tab (D166, flow 26.20, frames 39 and
//! 41 to 43): the rendered page, the source with comments on its lines, or
//! both side by side with the page following the source.
//!
//! The holder keeps what GPUI owns for each open document: the page's
//! `MarkdownView`, the source pane's scroll handle, and one text field for
//! the comment box. Everything else is read from the store on each draw.

#[path = "intent_tab.rs"]
pub mod intent_tab;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::comments::{Anchor, Comment, CommentId, Side};
use humata2_clients::docs::VersionText;
use humata2_clients::model::{Session, SessionId};
use humata2_markdown::{
    Anchor as PageAnchor, Comment as PageComment, CommentMarks, LineRange, MarkdownEvent,
    MarkdownOptions, MarkdownView, MermaidCache,
};
use humata2_review::DiffMarkdownMode;
use humata2_review::comments::CommentsAction;
use humata2_session::{DocumentMode, SessionAction, Tab, TabGroup, is_markdown};
use unicode_width::UnicodeWidthStr;

use crate::bridge::Ui;
use crate::chrome;
use crate::code_editor::EditorView;
use crate::comment_cards::{self, StartReattach};
use crate::theme::Look;
use crate::widgets::{self, col, row};

type Key = (SessionId, String, String);

/// Maximum number of retained views/editors per cache in [`Documents`].
pub(crate) const MAX_CACHED_VIEWS: usize = 8;

/// Small LRU-bounded map that evicts the least recently used entry past `cap`.
pub(crate) struct LruMap<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
    cap: usize,
}

impl<K: Eq + std::hash::Hash + Clone, V> LruMap<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.clone());
    }

    fn evict_overflow(&mut self) {
        while self.order.len() > self.cap {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&oldest);
        }
    }

    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if self.map.contains_key(key) {
            self.touch(key);
            self.map.get_mut(key)
        } else {
            None
        }
    }

    pub(crate) fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> &mut V {
        if !self.map.contains_key(&key) {
            let value = make();
            self.map.insert(key.clone(), value);
        }
        self.touch(&key);
        self.evict_overflow();
        self.map.get_mut(&key).expect("just inserted")
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        self.map.insert(key.clone(), value);
        self.touch(&key);
        self.evict_overflow();
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        self.map.retain(&mut keep);
        self.order.retain(|k| self.map.contains_key(k));
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.map.iter_mut()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

/// A comment as the page shows it: where it is and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mark {
    id: String,
    first: u32,
    last: u32,
    text: String,
}

/// Opens an external link activated inside a Markdown view.
pub(crate) fn open_markdown_link(url: &str, cx: &mut App) {
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("mailto:")
    {
        cx.open_url(url);
    }
}

struct Page {
    view: Entity<MarkdownView>,
    source: String,
    marks: Vec<Mark>,
    /// The source pane's scroll position.
    scroll: ScrollHandle,
    /// For each child of the source pane, the source line it is on; the
    /// pane's draw writes it and the next sync reads it.
    child_lines: Rc<RefCell<Vec<u32>>>,
    /// The line the page was last brought to while linked.
    last_top: Option<u32>,
    /// The old version the page was last linked to, when one is shown.
    last_old: Option<String>,
    _subscription: Subscription,
}

/// Rendered Markdown page for a file shown in the full-screen Diff view, with
/// block commenting wired directly into the review draft comment flow.
struct DiffPage {
    view: Entity<MarkdownView>,
    source: String,
    marks: Vec<Mark>,
    line_map: Rc<RefCell<Option<Vec<u32>>>>,
    lines: Rc<RefCell<Vec<String>>>,
    _subscription: Subscription,
}

/// What a sync needs from the store, copied out so no store borrow is held
/// while entities change.
#[derive(Default)]
struct Plan {
    sessions: Vec<SessionId>,
    active_session: Option<SessionId>,
    documents: Vec<Want>,
    /// The full-screen Diff panel's active Markdown file when in Split or
    /// Rendered mode.
    diff_markdown: Option<WantDiff>,
    /// The open comment box: session, first line, last line, text.
    editing: Option<(SessionId, u32, u32, String)>,
    /// The History strip's parts: old versions shown, the Save version box.
    history: crate::doc_history::Wanted,
}

struct WantDiff {
    key: Key,
    source: String,
    line_map: Option<Vec<u32>>,
    marks: Vec<Mark>,
}

struct Want {
    key: Key,
    source: String,
    marks: Vec<Mark>,
    follow: bool,
    /// The id of the old version the tab shows, once it is read: the page
    /// that follows the source is that version's.
    old: Option<String>,
}

fn plan(state: &AppState) -> Plan {
    let mut plan = Plan {
        sessions: state
            .workspace
            .sessions
            .iter()
            .filter(|session| !session.archived)
            .map(|session| session.id.clone())
            .collect(),
        active_session: state.session.open.clone(),
        ..Plan::default()
    };
    plan.history = crate::doc_history::wanted(state);
    let Some(id) = state.session.open.as_ref() else {
        return plan;
    };
    plan.editing = state
        .comments
        .sessions
        .get(id)
        .and_then(|held| held.editing.as_ref())
        .map(|b| (id.clone(), b.anchor.first, b.anchor.last, b.text.clone()));

    // Full-screen Diff Markdown renderer (Split or Rendered mode).
    if state.review.markdown_mode(id) != DiffMarkdownMode::Diff {
        let diffs: Vec<_> = state
            .workspace
            .diffs
            .iter()
            .filter(|diff| &diff.session == id)
            .collect();
        if let Some((diff, file)) = crate::chrome::selected(state, &diffs)
            && crate::panel_changes::markdown_view::is_diff_markdown(&file.path)
        {
            let (source, line_map) =
                crate::panel_changes::markdown_view::diff_markdown_source(state, diff, file);
            let mut marks: Vec<Mark> = state
                .comments
                .on_file(id, &diff.repo, &file.path)
                .into_values()
                .flatten()
                .filter(|comment| !comment.anchor.detached)
                .filter_map(|comment| {
                    let (first, last) =
                        crate::panel_changes::markdown_view::map_real_to_rendered_lines(
                            comment.anchor.first,
                            comment.anchor.last,
                            line_map.as_deref(),
                        )?;
                    Some(Mark {
                        id: comment.id.to_string(),
                        first,
                        last,
                        text: comment.text.clone(),
                    })
                })
                .collect();
            if let Some(b) = state
                .comments
                .sessions
                .get(id)
                .and_then(|held| held.editing.as_ref())
                .filter(|b| {
                    b.comment.is_none()
                        && b.anchor.repo == diff.repo
                        && b.anchor.path == file.path
                })
                && let Some((first, last)) =
                    crate::panel_changes::markdown_view::map_real_to_rendered_lines(
                        b.anchor.first,
                        b.anchor.last,
                        line_map.as_deref(),
                    )
            {
                marks.push(Mark {
                    id: "box".to_owned(),
                    first,
                    last,
                    text: b.text.clone(),
                });
            }
            plan.diff_markdown = Some(WantDiff {
                key: (id.clone(), diff.repo.clone(), file.path.clone()),
                source,
                line_map,
                marks,
            });
        }
    }

    let Some(view) = state.session.view() else {
        return plan;
    };
    let mut seen = HashSet::new();
    for group in &view.groups {
        for tab in &group.tabs {
            let Tab::File { repo, path } = tab else {
                continue;
            };
            let key = (id.clone(), repo.clone(), path.clone());
            if !is_markdown(path) || !seen.insert(key.clone()) {
                continue;
            }
            let Some(Ok(source)) = state.session.file_contents.get(&key) else {
                continue;
            };
            // The page lights drafts and the box being written; a sent
            // comment shows on its source line only.
            let mut marks: Vec<Mark> = state
                .comments
                .on_file(id, repo, path)
                .into_values()
                .flatten()
                .filter(|comment| comment.counts_as_draft() && !comment.anchor.detached)
                .map(|comment| Mark {
                    id: comment.id.to_string(),
                    first: comment.anchor.first,
                    last: comment.anchor.last,
                    text: comment.text.clone(),
                })
                .collect();
            // A box with no text yet has no comment to mark, so its lines are.
            if let Some(b) = state
                .comments
                .sessions
                .get(id)
                .and_then(|held| held.editing.as_ref())
                .filter(|b| b.comment.is_none() && b.anchor.repo == *repo && b.anchor.path == *path)
            {
                marks.push(Mark {
                    id: "box".to_owned(),
                    first: b.anchor.first,
                    last: b.anchor.last,
                    text: b.text.clone(),
                });
            }
            let shown = view.document(repo, path);
            plan.documents.push(Want {
                key,
                source: source.clone(),
                marks,
                follow: shown.mode == DocumentMode::SideBySide && shown.linked,
                old: if repo.is_empty() {
                    state
                        .session
                        .history(id, path)
                        .filter(|history| history.read_only())
                        .and_then(|history| history.viewed.as_ref())
                        .map(|read| read.version.id.clone())
                } else {
                    None
                },
            });
        }
    }
    plan
}

/// The last part of a file's path.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The page's comments: each mark's lines of the page's own source.
fn page_comments(view: &MarkdownView, marks: &[Mark]) -> Vec<PageComment> {
    let document = view.document();
    marks
        .iter()
        .filter_map(|mark| {
            let bytes = document.line_bytes(LineRange {
                start: mark.first as usize,
                end: mark.last as usize,
            });
            let quote = document.source().get(bytes.clone())?.to_owned();
            let selection = document.selection(bytes, quote)?;
            Some(PageComment {
                id: mark.id.clone(),
                anchor: PageAnchor::new(document, &selection),
                body: mark.text.clone(),
            })
        })
        .collect()
}

fn set_marks(view: &Entity<MarkdownView>, marks: &[Mark], cx: &mut App) {
    view.update(cx, |view, cx| {
        let comments = page_comments(view, marks);
        view.set_comments(comments, cx);
    });
}

/// Everything a click on a source line needs.
struct Target {
    ui: Rc<Ui>,
    session: SessionId,
    repo: String,
    path: String,
    lines: Vec<String>,
    reattach: Rc<RefCell<Option<CommentId>>>,
}

impl Target {
    fn anchor(&self, first: u32, last: u32) -> Anchor {
        let quoted = self
            .lines
            .get(first.saturating_sub(1) as usize..last as usize)
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        Anchor {
            repo: self.repo.clone(),
            path: self.path.clone(),
            side: Side::New,
            first,
            last,
            quoted,
            base: None,
            detached: false,
            outdated: false,
        }
    }

    /// A line was clicked: it takes a comment that is being re-attached, or
    /// opens the box on it, or with `extend` stretches the open box to it.
    fn click(&self, line: u32, extend: bool, cx: &mut App) {
        let pending = self.reattach.borrow_mut().take();
        if let Some(id) = pending {
            self.ui.send(
                AppAction::Comments(CommentsAction::Reattach(
                    self.session.clone(),
                    id,
                    self.anchor(line, line),
                )),
                cx,
            );
            return;
        }
        let range = extend
            .then(|| {
                self.ui.read(|state| {
                    state
                        .comments
                        .sessions
                        .get(&self.session)
                        .and_then(|held| held.editing.as_ref())
                        .filter(|b| b.anchor.repo == self.repo && b.anchor.path == self.path)
                        .map(|b| (b.anchor.first.min(line), b.anchor.last.max(line)))
                })
            })
            .flatten()
            .unwrap_or((line, line));
        self.ui.send(
            AppAction::Comments(CommentsAction::OpenBox(
                self.session.clone(),
                self.anchor(range.0, range.1),
            )),
            cx,
        );
    }
}

/// The open documents of the shown session.
pub struct Documents {
    pages: LruMap<Key, Page>,
    diff_pages: LruMap<Key, DiffPage>,
    /// Shared bounded Mermaid diagram cache across all Markdown views owned by
    /// this holder, preventing each view from keeping its own 96 MB cache.
    diagrams: MermaidCache,
    comment_input: Entity<TextareaState>,
    /// The box the field was last set for: session, lines and text.
    seen: Option<(SessionId, u32, u32, String)>,
    /// The detached comment whose new line the next click picks.
    unfolded: comment_cards::Unfolded,
    reattach: Rc<RefCell<Option<CommentId>>>,
    /// The Diff panel's code views, by `session\x1frepo/path` (or `repo/path`).
    diff_views: RefCell<LruMap<String, DiffView>>,
    /// The editors of changed code files, by session, repository and path.
    editors: RefCell<LruMap<Key, EditorView>>,
    pub conversation: crate::bridge::ConversationScroll,
    /// The History strip's Save version box, list and old versions' pages.
    history: crate::doc_history::HistoryUi,
    _typing: Subscription,
    /// The filter field of a repository's base menu in the Diff panel
    /// (RV-31), and the repository it was last set for.
    base_filter: Entity<InputState>,
    base_filter_for: Option<String>,
    _base_filtering: Subscription,
    /// The Diff panel's "Filter files" field (RV-6), and the session + text
    /// last synced from the store.
    file_filter: Entity<InputState>,
    file_filter_for: Option<(SessionId, String)>,
    _file_filtering: Subscription,
    /// The Diff panel's overall ("as a whole") comment/request input, and the
    /// session + text last synced from the store.
    overall_input: Entity<TextareaState>,
    overall_input_for: Option<(SessionId, String)>,
    _overall_typing: Subscription,
    /// The Diff's file list: while it has this focus its arrows and Enter move
    /// the cursor, open a file or fold a repository (DS-84).
    pub diff_list: FocusHandle,
    /// A stack's layer list (D804): a click in it gives the Diff panel the
    /// focus, so the layer keys step from the layer just picked.
    pub diff_stack: FocusHandle,
    /// The Diff panel's code (DS-40): a click on a line gives it the focus,
    /// so ⌥Z flips Wrap there and no longer types into a comment box.
    pub diff_code: FocusHandle,
    /// Active drag of the full-screen Diff file list splitter: `(start_x, start_width)`.
    pub diff_list_drag: Rc<Cell<Option<(f32, f32)>>>,
}

/// What the Diff panel remembers about one file's code view so a rewrap can
/// keep its top line in place: a short file's scroll offset and top row, a
/// long file's virtual list (DV-5), how far Scroll mode has moved the code
/// sideways, and the facts that take a pass over every line.
#[derive(Clone)]
pub struct DiffView {
    pub handle: ScrollHandle,
    anchor: Rc<RefCell<Option<TopAnchor>>>,
    list: Rc<RefCell<Option<VirtualRows>>>,
    shift: Rc<Cell<f32>>,
    facts: Rc<RefCell<Option<(LinesKey, LineFacts)>>>,
}

/// How far past the view a long file's list lays out lines, so a short
/// scroll never shows a gap.
const OVERDRAW: f32 = 240.;

/// Which read of a file a cached fact is about: where its lines are held and
/// how many there are. A new read holds them somewhere else.
#[derive(Clone, Copy, PartialEq, Eq)]
struct LinesKey {
    at: usize,
    len: usize,
}

impl LinesKey {
    fn of<T>(lines: &[T]) -> Self {
        Self {
            at: lines.as_ptr() as usize,
            len: lines.len(),
        }
    }
}

/// A file's widest line number in digits and its widest line in cells.
#[derive(Clone, Copy)]
pub struct LineFacts {
    pub digits: usize,
    pub widest: usize,
}

/// A long file's list, one item per line, and what it was last measured for.
struct VirtualRows {
    state: ListState,
    lines: LinesKey,
    wrap: bool,
    width: f32,
    /// The list width the lines not drawn last got a height guess at.
    guessed_at: Option<Pixels>,
}

/// The row at the top of the view: its child index, how far into it the view
/// starts as a fraction of its height, and the panel width it was seen at.
#[derive(Clone, Copy)]
struct TopAnchor {
    width: f32,
    child: usize,
    fraction: f32,
}

impl DiffView {
    /// The file's line facts, counted once per read of the file rather than
    /// on every frame.
    pub fn facts<T>(&self, lines: &[T], count: impl FnOnce(&[T]) -> LineFacts) -> LineFacts {
        let key = LinesKey::of(lines);
        let mut held = self.facts.borrow_mut();
        match *held {
            Some((at, facts)) if at == key => facts,
            _ => {
                let facts = count(lines);
                *held = Some((key, facts));
                facts
            }
        }
    }

    /// A long file's virtual list, one item per line. It is made once; a
    /// new read of the file, a wrap switch or a new panel width measures the
    /// lines again, and the line at the top stays there, as far into it as
    /// before relative to its new height. A read with a different number of
    /// lines keeps the top line's index.
    pub fn rows<T>(&self, lines: &[T], wrap: bool, width: f32, line_height: f32) -> ListState {
        let key = LinesKey::of(lines);
        let guess = px(line_height);
        let mut held = self.list.borrow_mut();
        let Some(rows) = held.as_mut() else {
            let state = ListState::new(key.len, ListAlignment::Top, px(OVERDRAW))
                .with_uniform_item_height(guess);
            *held = Some(VirtualRows {
                state: state.clone(),
                lines: key,
                wrap,
                width,
                guessed_at: None,
            });
            return state;
        };
        if rows.lines.len != key.len {
            let top = rows.state.logical_scroll_top();
            rows.state.reset_with_uniform_height(key.len, guess);
            rows.state.scroll_to(top);
        } else if rows.lines != key || rows.wrap != wrap || (rows.width - width).abs() > 0.5 {
            rows.state.remeasure();
        }
        rows.lines = key;
        rows.wrap = wrap;
        rows.width = width;
        rows.state.clone()
    }

    /// Called once the list is laid out. A new list width drops the list's
    /// height guesses, so the lines not drawn get one line's height again and
    /// the list keeps its whole length to scroll through.
    pub fn guess_heights(&self, line_height: f32) {
        let mut held = self.list.borrow_mut();
        let Some(rows) = held.as_mut() else {
            return;
        };
        let width = rows.state.viewport_bounds().size.width;
        if rows.guessed_at != Some(width) {
            let _ = rows.state.clone().with_uniform_item_height(px(line_height));
            rows.guessed_at = Some(width);
        }
    }

    /// How far Scroll mode has moved the code left, in pixels, held within
    /// `max` (the widest line less the code column).
    pub fn shift(&self, max: f32) -> f32 {
        let shift = self.shift.get().clamp(0., max.max(0.));
        self.shift.set(shift);
        shift
    }

    /// Moves the code sideways by a wheel's horizontal `delta`, within
    /// `max`; true when it moved.
    pub fn scroll_sideways(&self, delta: f32, max: f32) -> bool {
        let before = self.shift.get();
        let after = (before - delta).clamp(0., max.max(0.));
        self.shift.set(after);
        (after - before).abs() > f32::EPSILON
    }

    /// Called with the bounds of the code body's children on every frame.
    /// While the width stays, notes which child is at the top; when it
    /// changes under wrap, moves the offset so that child is at the top
    /// again and asks for one more frame.
    pub fn keep_top(&self, width: f32, wrap: bool, children: &[Bounds<Pixels>], cx: &mut App) {
        let view_top = self.handle.bounds().origin.y;
        let scroll_top = -self.handle.offset().y;
        let held = *self.anchor.borrow();
        if let Some(held) = held.filter(|held| wrap && (held.width - width).abs() > 0.5) {
            if let Some(child) = children.get(held.child) {
                // Bounds are as drawn, offset included.
                let content = child.origin.y - view_top + scroll_top;
                let target = content + child.size.height * held.fraction;
                let mut offset = self.handle.offset();
                offset.y = -target.max(px(0.));
                self.handle.set_offset(offset);
                *self.anchor.borrow_mut() = Some(TopAnchor { width, ..held });
                cx.refresh_windows();
                return;
            }
        }
        let found = children.iter().enumerate().rev().find(|(_, child)| child.origin.y <= view_top);
        *self.anchor.borrow_mut() = found.map(|(child, bounds)| {
            let height = f32::from(bounds.size.height);
            let into = f32::from(view_top - bounds.origin.y);
            TopAnchor {
                width,
                child,
                fraction: if height > 0. { (into / height).clamp(0., 1.) } else { 0. },
            }
        });
    }
}

impl Documents {
    pub fn new(ui: &Rc<Ui>, window: &mut Window, cx: &mut App) -> Self {
        let comment_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Write a comment")
                .auto_grow(1, 12)
        });
        let diagrams = MermaidCache::default();
        let history = crate::doc_history::HistoryUi::new(ui, diagrams.clone(), window, cx);
        let typing_ui = ui.clone();
        let typing = cx.subscribe(
            &comment_input,
            move |input: Entity<TextareaState>, event: &InputEvent, cx: &mut App| {
                match event {
                    InputEvent::PressEnter { secondary: true, .. } => {
                        let Some(session) = typing_ui.read(|state| state.session.open.clone()) else {
                            return;
                        };
                        let can_send = typing_ui.read(|state| {
                            state
                                .comments
                                .sessions
                                .get(&session)
                                .and_then(|held| held.editing.as_ref())
                                .is_some_and(|b| !b.text.trim().is_empty())
                        });
                        if can_send {
                            typing_ui.send(
                                AppAction::Comments(CommentsAction::Comment(session)),
                                cx,
                            );
                        }
                        return;
                    }
                    InputEvent::Change => {}
                    _ => return,
                }
                let value = input.read(cx).value().to_string();
                let Some(session) = typing_ui.read(|state| state.session.open.clone()) else {
                    return;
                };
                let held = typing_ui.read(|state| {
                    state
                        .comments
                        .sessions
                        .get(&session)
                        .and_then(|held| held.editing.as_ref())
                        .map(|b| b.text.clone())
                });
                // The field also changes when the store sets it; only the
                // person's typing is news.
                if held.is_some_and(|held| held != value) {
                    typing_ui.send(
                        AppAction::Comments(CommentsAction::Typed(session, value)),
                        cx,
                    );
                }
            },
        );
        let base_filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter branches, or type a branch or commit")
        });
        let filtering = {
            let ui = ui.clone();
            cx.subscribe(
                &base_filter,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let value = input.read(cx).value().to_string();
                    let held = ui.read(|state| {
                        state.review.repo_picker.as_ref().map(|picker| picker.filter.clone())
                    });
                    // Only the person's typing is news, not the store's own set.
                    if held.is_some_and(|held| held != value) {
                        ui.send(
                            AppAction::Review(humata2_review::ReviewAction::FilterRepoBases(value)),
                            cx,
                        );
                    }
                },
            )
        };
        let file_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter files"));
        let file_filtering = {
            let ui = ui.clone();
            cx.subscribe(
                &file_filter,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let value = input.read(cx).value().to_string();
                    let held = ui.read(|state| {
                        state.session.open.clone().map(|session| {
                            let held = state.review.file_filter(&session).to_owned();
                            (session, held)
                        })
                    });
                    // Only the person's typing is news, not the store's own set.
                    if let Some((session, held)) = held.filter(|(_, held)| *held != value) {
                        let _ = held;
                        ui.send(
                            AppAction::Review(humata2_review::ReviewAction::FilterFiles(
                                session, value,
                            )),
                            cx,
                        );
                    }
                },
            )
        };
        let overall_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Comment or request on the whole change…")
                .auto_grow(1, 4)
        });
        let overall_typing = {
            let ui = ui.clone();
            cx.subscribe(
                &overall_input,
                move |input: Entity<TextareaState>, event: &InputEvent, cx: &mut App| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        let held = ui.read(|state| {
                            state.session.open.clone().map(|session| {
                                let held = state.comments.overall_draft(&session).to_owned();
                                (session, held)
                            })
                        });
                        if let Some((session, held)) = held.filter(|(_, held)| *held != value) {
                            let _ = held;
                            ui.send(
                                AppAction::Comments(CommentsAction::SetOverallDraft(session, value)),
                                cx,
                            );
                        }
                    }
                    InputEvent::PressEnter { secondary: true, .. } => {
                        if let Some(session) = ui.read(|state| state.session.open.clone()) {
                            ui.send(AppAction::Comments(CommentsAction::SendAll(session)), cx);
                        }
                    }
                    _ => {}
                },
            )
        };
        Self {
            pages: LruMap::new(MAX_CACHED_VIEWS),
            diff_pages: LruMap::new(MAX_CACHED_VIEWS),
            diagrams,
            comment_input,
            seen: None,
            reattach: Rc::new(RefCell::new(None)),
            unfolded: Default::default(),
            diff_views: RefCell::new(LruMap::new(MAX_CACHED_VIEWS)),
            editors: RefCell::new(LruMap::new(MAX_CACHED_VIEWS)),
            conversation: Default::default(),
            history,
            _typing: typing,
            base_filter,
            base_filter_for: None,
            _base_filtering: filtering,
            file_filter,
            file_filter_for: None,
            _file_filtering: file_filtering,
            overall_input,
            overall_input_for: None,
            _overall_typing: overall_typing,
            diff_list: cx.focus_handle(),
            diff_stack: cx.focus_handle(),
            diff_code: cx.focus_handle(),
            diff_list_drag: Rc::new(Cell::new(None)),
        }
    }

    /// The rendered Markdown view for a file in the full-screen Diff panel.
    pub fn diff_markdown_view(
        &self,
        session: &SessionId,
        repo: &str,
        path: &str,
    ) -> Option<Entity<MarkdownView>> {
        self.diff_pages
            .get(&(session.clone(), repo.to_owned(), path.to_owned()))
            .map(|page| page.view.clone())
    }

    /// The Diff panel's "Filter files" field.
    pub fn file_filter(&self) -> Entity<InputState> {
        self.file_filter.clone()
    }

    /// The Diff panel's overall ("as a whole") comment/request field.
    pub fn overall_input(&self) -> Entity<TextareaState> {
        self.overall_input.clone()
    }

    /// The filter field of the open repository base menu.
    pub fn base_filter(&self) -> Entity<InputState> {
        self.base_filter.clone()
    }

    /// The scroll state of the Diff panel's code view for `name`.
    pub fn diff_view(&self, name: &str) -> DiffView {
        self.diff_views
            .borrow_mut()
            .get_or_insert_with(name.to_owned(), || DiffView {
                handle: ScrollHandle::new(),
                anchor: Rc::new(RefCell::new(None)),
                list: Rc::new(RefCell::new(None)),
                shift: Rc::new(Cell::new(0.)),
                facts: Rc::new(RefCell::new(None)),
            })
            .clone()
    }

    /// The scroll state of the Diff panel's code view for `name` in
    /// `session` (DS-73). Two sessions on one file keep their own scroll; the
    /// unit separator keeps a session's key apart from any `repo/path` key.
    pub fn session_diff_view(&self, session: &SessionId, name: &str) -> DiffView {
        self.diff_view(&format!("{}\u{1f}{name}", session.as_str()))
    }

    /// The editor state of a changed code file, kept across frames.
    pub fn editor_view(&self, session: &SessionId, repo: &str, path: &str) -> EditorView {
        self.editors
            .borrow_mut()
            .get_or_insert_with(
                (session.clone(), repo.to_owned(), path.to_owned()),
                EditorView::new,
            )
            .clone()
    }

    /// The comment box's text field, shared by every file view.
    pub fn input(&self) -> Entity<TextareaState> {
        self.comment_input.clone()
    }

    /// The cell holding the detached comment whose new line the next click
    /// picks.
    pub fn reattach(&self) -> Rc<RefCell<Option<CommentId>>> {
        self.reattach.clone()
    }

    /// The files whose done comments are unfolded, as `repo/path`.
    pub fn unfolded(&self) -> comment_cards::Unfolded {
        self.unfolded.clone()
    }

    /// Brings the pages, the comment field and the link in step with the
    /// store. Called before each draw of the window.
    pub fn sync(&mut self, ui: &Rc<Ui>, window: &mut Window, cx: &mut App) {
        let plan = ui.read(plan);
        self.history.sync(&plan.history, window, cx);
        let mut following = HashSet::new();
        let mut olds = HashMap::new();
        let mut wanted_pages = HashSet::new();
        for want in plan.documents {
            wanted_pages.insert(want.key.clone());
            if want.follow {
                following.insert(want.key.clone());
            }
            olds.insert(want.key.clone(), want.old.clone());
            match self.pages.get_mut(&want.key) {
                None => {
                    let mut options = MarkdownOptions::new(want.key.2.clone());
                    // Comments are written on source lines; the page only
                    // shows where they are.
                    options.comments = false;
                    options.marks = CommentMarks::Counts;
                    options.diagrams = self.diagrams.clone();
                    // The page's own typography, and a byline ending in the
                    // file's name.
                    options.document = Some(file_name(&want.key.2).into());
                    let source = want.source.clone();
                    let view = cx.new(|cx| MarkdownView::new(&source, options, cx));
                    set_marks(&view, &want.marks, cx);
                    let subscription = cx.subscribe(
                        &view,
                        |_view, event: &MarkdownEvent, cx: &mut App| {
                            if let MarkdownEvent::LinkActivated { url } = event {
                                open_markdown_link(url, cx);
                            }
                        },
                    );
                    self.pages.insert(
                        want.key,
                        Page {
                            view,
                            source: want.source,
                            marks: want.marks,
                            scroll: ScrollHandle::new(),
                            child_lines: Rc::new(RefCell::new(Vec::new())),
                            last_top: None,
                            last_old: None,
                            _subscription: subscription,
                        },
                    );
                }
                Some(page) => {
                    let text_changed = page.source != want.source;
                    if text_changed {
                        let source = want.source.clone();
                        page.view
                            .update(cx, |view, cx| view.set_source(&source, cx));
                        page.source = want.source;
                    }
                    if text_changed || page.marks != want.marks {
                        set_marks(&page.view, &want.marks, cx);
                        page.marks = want.marks;
                    }
                }
            }
        }
        self.pages.retain(|key, _| {
            plan.sessions.contains(&key.0)
                && (plan.active_session.as_ref() != Some(&key.0) || wanted_pages.contains(key))
        });

        if let Some(want) = plan.diff_markdown {
            match self.diff_pages.get_mut(&want.key) {
                None => {
                    let mut options = MarkdownOptions::new(want.key.2.clone());
                    options.comments = true;
                    options.marks = CommentMarks::Counts;
                    options.diagrams = self.diagrams.clone();
                    options.document = Some(file_name(&want.key.2).into());
                    let source = want.source.clone();
                    let view = cx.new(|cx| MarkdownView::new(&source, options, cx));
                    set_marks(&view, &want.marks, cx);
                    let line_map = Rc::new(RefCell::new(want.line_map));
                    let lines = Rc::new(RefCell::new(
                        source.lines().map(str::to_owned).collect::<Vec<_>>(),
                    ));
                    let sub_ui = ui.clone();
                    let (sub_session, sub_repo, sub_path) = want.key.clone();
                    let sub_line_map = line_map.clone();
                    let sub_lines = lines.clone();
                    let subscription = cx.subscribe(
                        &view,
                        move |_view, event: &MarkdownEvent, cx: &mut App| match event {
                            MarkdownEvent::CommentRequested(selection) => {
                                let (first, last) =
                                    crate::panel_changes::markdown_view::map_rendered_lines(
                                        selection.lines.start,
                                        selection.lines.end,
                                        sub_line_map.borrow().as_deref(),
                                    );
                                let quoted = {
                                    let held = sub_lines.borrow();
                                    let start = selection.lines.start.saturating_sub(1);
                                    let end = selection.lines.end.min(held.len());
                                    if start < end {
                                        held[start..end].to_vec()
                                    } else {
                                        selection.source.lines().map(str::to_owned).collect()
                                    }
                                };
                                let anchor = Anchor {
                                    repo: sub_repo.clone(),
                                    path: sub_path.clone(),
                                    side: Side::New,
                                    first,
                                    last,
                                    quoted,
                                    base: None,
                                    detached: false,
                                    outdated: false,
                                };
                                sub_ui.send(
                                    AppAction::Comments(CommentsAction::OpenBox(
                                        sub_session.clone(),
                                        anchor,
                                    )),
                                    cx,
                                );
                            }
                            MarkdownEvent::CommentActivated { id } if id != "box" => {
                                let draft_id = sub_ui.read(|state| {
                                    state.comments.sessions.get(&sub_session).and_then(|held| {
                                        held.comments
                                            .iter()
                                            .find(|c| {
                                                c.id.to_string() == *id && c.counts_as_draft()
                                            })
                                            .map(|c| c.id.clone())
                                    })
                                });
                                if let Some(comment_id) = draft_id {
                                    sub_ui.send(
                                        AppAction::Comments(CommentsAction::EditDraft(
                                            sub_session.clone(),
                                            comment_id,
                                        )),
                                        cx,
                                    );
                                }
                            }
                            MarkdownEvent::LinkActivated { url } => {
                                open_markdown_link(url, cx);
                            }
                            _ => {}
                        },
                    );
                    self.diff_pages.insert(
                        want.key,
                        DiffPage {
                            view,
                            source: want.source,
                            marks: want.marks,
                            line_map,
                            lines,
                            _subscription: subscription,
                        },
                    );
                }
                Some(page) => {
                    let text_changed = page.source != want.source;
                    if text_changed {
                        let source = want.source.clone();
                        page.view
                            .update(cx, |view, cx| view.set_source(&source, cx));
                        *page.lines.borrow_mut() =
                            source.lines().map(str::to_owned).collect();
                        page.source = want.source;
                    }
                    *page.line_map.borrow_mut() = want.line_map;
                    if text_changed || page.marks != want.marks {
                        set_marks(&page.view, &want.marks, cx);
                        page.marks = want.marks;
                    }
                }
            }
        }
        self.diff_pages
            .retain(|(session, _, _), _| plan.sessions.contains(session));
        self.editors
            .borrow_mut()
            .retain(|(session, _, _), _| plan.sessions.contains(session));
        self.diff_views.borrow_mut().retain(|key, _| {
            match key.split_once('\u{1f}') {
                Some((session_id, _)) => plan.sessions.iter().any(|s| s.as_str() == session_id),
                None => true,
            }
        });

        // The comment box's field follows the store: set when a box opens or
        // a draft is loaded into it, cleared when it closes. Only a value
        // the store changed is written, so typing is never overwritten.
        match plan.editing {
            None => {
                if self.seen.take().is_some() {
                    self.set_input(String::new(), window, cx);
                }
            }
            Some(target) => {
                if self.seen.as_ref() != Some(&target) {
                    let opened = self
                        .seen
                        .as_ref()
                        .is_none_or(|seen| seen.0 != target.0 || seen.1 != target.1 || seen.2 != target.2);
                    if self.comment_input.read(cx).value().as_ref() != target.3 {
                        self.set_input(target.3.clone(), window, cx);
                    }
                    if opened {
                        let focus = self.comment_input.focus_handle(cx);
                        window.focus(&focus, cx);
                    }
                    self.seen = Some(target);
                }
            }
        }

        // The "Filter files" field shows the open session's own text.
        let open = ui.read(|state| {
            state.session.open.clone().map(|session| {
                let text = state.review.file_filter(&session).to_owned();
                (session, text)
            })
        });
        if let Some((session, text)) = open {
            // Only push into the input when the session or store value itself
            // changed, so background redraws never wipe in-flight keystrokes.
            let changed = self
                .file_filter_for
                .as_ref()
                .is_none_or(|(prev_session, prev_text)| *prev_session != session || *prev_text != text);
            if changed {
                if self.file_filter.read(cx).value().as_ref() != text.as_str() {
                    self.file_filter
                        .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                }
                self.file_filter_for = Some((session, text));
            }
        }

        // The Diff panel's overall comment/request field follows the open session.
        let open_overall = ui.read(|state| {
            state.session.open.clone().map(|session| {
                let text = state.comments.overall_draft(&session).to_owned();
                (session, text)
            })
        });
        if let Some((session, text)) = open_overall {
            let changed = self
                .overall_input_for
                .as_ref()
                .is_none_or(|(prev_session, prev_text)| *prev_session != session || *prev_text != text);
            if changed {
                if self.overall_input.read(cx).value().as_ref() != text.as_str() {
                    self.overall_input
                        .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                }
                self.overall_input_for = Some((session, text));
            }
        }

        // A repository's base menu opens with its filter empty and focused,
        // and the field empties when the menu closes.
        let picker = ui.read(|state| {
            state
                .review
                .repo_picker
                .as_ref()
                .map(|picker| (picker.repo.clone(), picker.filter.clone()))
        });
        match picker {
            Some((repo, filter)) if self.base_filter_for.as_ref() != Some(&repo) => {
                self.base_filter
                    .update(cx, |input, cx| input.set_value(filter, window, cx));
                self.base_filter
                    .update(cx, |input, cx| input.focus(window, cx));
                self.base_filter_for = Some(repo);
            }
            Some(_) => {}
            None => {
                if self.base_filter_for.take().is_some() {
                    self.base_filter
                        .update(cx, |input, cx| input.set_value(String::new(), window, cx));
                }
            }
        }

        // Linked scrolling: the block at the source's top line goes to the
        // top of the page.
        for (key, page) in self.pages.iter_mut() {
            if !following.contains(key) {
                page.last_top = None;
                continue;
            }
            // The page that follows is the one shown: the old version's
            // while one is read, and nothing to move until it is made.
            let old = olds.get(key).cloned().flatten();
            if page.last_old != old {
                page.last_old = old.clone();
                page.last_top = None;
            }
            let shown = match &old {
                Some(id) => self.history.viewed_page(&key.0, &key.2, id),
                None => Some(page.view.clone()),
            };
            let Some(shown) = shown else {
                continue;
            };
            let top = page
                .child_lines
                .borrow()
                .get(page.scroll.top_item())
                .copied();
            if let Some(top) = top
                && page.last_top != Some(top)
            {
                page.last_top = Some(top);
                shown.update(cx, |view, cx| {
                    let block = view
                        .document()
                        .blocks()
                        .partition_point(|block| block.lines.start <= top as usize)
                        .saturating_sub(1);
                    view.reveal_block(block, cx);
                });
            }
        }
    }

    fn set_input(&self, value: String, window: &mut Window, cx: &mut App) {
        self.comment_input
            .update(cx, |input, cx| input.set_value(value, window, cx));
    }

    /// The document for a selected file tab, or `None` while its text isn't
    /// here (the caller then draws the plain file).
    pub fn body(
        &self,
        ui: &Rc<Ui>,
        state: &AppState,
        session: &Session,
        repo: &str,
        path: &str,
        look: Look,
    ) -> Option<AnyElement> {
        if humata2_intent_spec::is_intent_spec(path) {
            return intent_tab::body(self, ui, state, session, repo, path, look);
        }
        let key = (session.id.clone(), repo.to_owned(), path.to_owned());
        let page = self.pages.get(&key)?;
        let Some(Ok(text)) = state.session.file_contents.get(&key) else {
            return None;
        };
        let shown = state
            .session
            .view()
            .map(|view| view.document(repo, path))
            .unwrap_or_default();
        // An old version of a Desk file is read-only (DV-6): its text, its
        // recorded comments, and no way to comment, send or resolve.
        let history = if repo.is_empty() {
            state.session.history(&session.id, path)
        } else {
            None
        };
        let old = history.filter(|history| history.read_only());
        let read = old.and_then(|history| history.viewed.as_ref());
        let viewed_view = read
            .and_then(|read| self.history.viewed_page(&session.id, path, &read.version.id));
        let reading = old.is_some() && viewed_view.is_none();
        let shown_view = viewed_view.unwrap_or_else(|| page.view.clone());
        // `unlink_on_scroll`: the page follows the source while scrolling is
        // linked, so turning the wheel over the page is scrolling it by
        // hand, and that unticks Link scrolling.
        let preview = |unlink_on_scroll: bool, view: Entity<MarkdownView>| {
            let (ui, repo, path) = (ui.clone(), repo.to_owned(), path.to_owned());
            let linked_path = path.clone();
            div()
                .id("doc-preview")
                .when(unlink_on_scroll, move |pane| {
                    pane.on_scroll(move |_, _, cx| {
                        ui.send(
                            AppAction::Session(SessionAction::LinkScrolling {
                                repo: repo.clone(),
                                path: linked_path.clone(),
                                linked: false,
                            }),
                            cx,
                        );
                    })
                })
                .role(Role::Document)
                .aria_label(format!("{path} preview"))
                .flex_1()
                .min_w_0()
                .min_h_0()
                .bg(rgb(look.content))
                .flex()
                .justify_center()
                // The padding is inside the pane, so beside the source the
                // two panes split the width evenly; the page is 560 at most.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(px(608.))
                        .px(px(24.))
                        .h_full()
                        .child(view),
                )
                .test_support()
        };
        let content = if reading {
            crate::doc_history::reading(look)
        } else {
            match shown.mode {
                DocumentMode::Preview => preview(false, shown_view).into_any_element(),
                DocumentMode::Source => {
                    self.source(ui, state, session, page, repo, path, text, read, look)
                }
                DocumentMode::SideBySide => {
                    let groups = state.session.view().map_or(1, |view| view.groups.len());
                    let sendable = state.comments.sendable(&session.id);
                    // While a box is open, Send shows only when another draft
                    // waits beside the box's own (frame 42 has none); it then
                    // saves the box's draft and sends all of them.
                    let in_box = state
                        .comments
                        .sessions
                        .get(&session.id)
                        .and_then(|held| held.editing.as_ref())
                        .map(|open| open.comment.clone());
                    let boxed = in_box.as_ref().is_some_and(|in_box| {
                        sendable.iter().all(|draft| Some(&draft.id) == in_box.as_ref())
                    });
                    let drafts = sendable.len();
                    col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(link_bar(
                            ui,
                            &session.id,
                            repo,
                            path,
                            shown.linked,
                            groups <= 1,
                            if boxed || old.is_some() { 0 } else { drafts },
                            look,
                        ))
                        .child(
                            row()
                                .flex_1()
                                .min_h_0()
                                .items_stretch()
                                .child(self.source(
                                    ui, state, session, page, repo, path, text, read, look,
                                ))
                                .child(div().w(px(1.)).h_full().flex_shrink_0().bg(rgb(look.line)))
                                .child(preview(shown.linked, shown_view)),
                        )
                        .into_any_element()
                }
            }
        };
        Some(match history {
            Some(history) => col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(self.history.strip(ui, history, path, read, look))
                .child(content)
                .into_any_element(),
            None => content,
        })
    }

    /// The source: numbered lines, comments under the line each ends on and
    /// the box under its last line.
    #[allow(clippy::too_many_arguments)]
    fn source(
        &self,
        ui: &Rc<Ui>,
        state: &AppState,
        session: &Session,
        page: &Page,
        repo: &str,
        path: &str,
        text: &str,
        read: Option<&VersionText>,
        look: Look,
    ) -> AnyElement {
        // An old version shows its own text, comments and no box (DV-6).
        let text = read.map_or(text, |read| read.text.as_str());
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let total = lines.len() as u32;
        let by_line = match read {
            Some(read) => crate::doc_history::recorded_by_line(read),
            None => state.comments.on_file(&session.id, repo, path),
        };
        let editing = state
            .comments
            .sessions
            .get(&session.id)
            .and_then(|held| held.editing.as_ref())
            .filter(|b| b.anchor.repo == repo && b.anchor.path == path)
            .filter(|_| read.is_none());
        let box_range = editing.map(|b| (b.anchor.first, b.anchor.last));
        let box_comment = editing.and_then(|b| b.comment.clone());
        let wrap = state.review.wrap;
        let reattaching = self.reattach.borrow().clone();
        let lit_color = if look.term == 0xffffff { 0xeaf2ff } else { 0x1c2a3f };
        let markers: BTreeSet<u32> = by_line
            .values()
            .flatten()
            .filter(|comment| !comment.anchor.detached)
            .map(|comment| comment.anchor.first)
            .chain(box_range.map(|(first, _)| first))
            .collect();
        let widest = lines.iter().map(|line| line.width()).max().unwrap_or(0);
        let min_width = widest as f32 * 7. + 70.;
        let agent = session.agent.command();
        let cell = self.reattach.clone();
        let start: StartReattach = Rc::new(move |id, cx| {
            *cell.borrow_mut() = Some(id.clone());
            cx.refresh_windows();
        });
        let target = Rc::new(Target {
            ui: ui.clone(),
            session: session.id.clone(),
            repo: repo.to_owned(),
            path: path.to_owned(),
            lines: lines.clone(),
            reattach: self.reattach.clone(),
        });

        let fold_key = format!("{repo}/{path}");
        let todo_ctx = comment_cards::todo_context(state, &session.id);
        let fold_count = comment_cards::all_done(by_line.values().flatten().copied()).filter(|_| read.is_none());
        let folded = fold_count.is_some() && !self.unfolded.borrow().contains(&fold_key);
        let fold_at = by_line.keys().next().copied().map(|n| n.min(total));
        let dark = ((look.term >> 8) & 0xff) < 0x80;
        let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let syntax = humata2_highlight::highlight_lines_cached(
            humata2_highlight::Language::Markdown,
            &line_refs,
            dark,
        );
        let mut children: Vec<AnyElement> = Vec::new();
        let mut child_lines: Vec<u32> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let n = index as u32 + 1;
            let lit = box_range.is_some_and(|(first, last)| first <= n && n <= last);
            let tint = if line.starts_with('#') {
                look.accent
            } else if line.starts_with("```") || line.starts_with('_') {
                look.term_dim
            } else {
                look.term_text
            };
            let line_spans = syntax.line(index);
            let click = target.clone();
            let gutter = div()
                .id(SharedString::from(if read.is_some() {
                    format!("doc-old-line-{n}")
                } else {
                    format!("doc-line-{n}")
                }))
                .flex()
                .flex_shrink_0()
                // Only the file as it is takes a comment (DV-6).
                .when(read.is_none(), move |gutter| {
                    gutter
                        .role(Role::Button)
                        .aria_label(format!("Comment on line {n}"))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            click.click(n, event.modifiers.shift, cx);
                        })
                })
                .when(read.is_some(), |gutter| {
                    gutter.role(Role::Region).aria_label(format!("Line {n}"))
                })
                .child(
                    div()
                        .w(px(30.))
                        .flex_shrink_0()
                        .pr(px(6.))
                        .flex()
                        .justify_end()
                        .text_color(rgb(look.term_dim))
                        .child(n.to_string()),
                )
                .child(
                    div()
                        .w(px(18.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(lit, |gutter| gutter.border_l_2().border_color(rgb(look.accent)))
                        .when(markers.contains(&n), |gutter| {
                            gutter.child(chrome::icon(IconName::MessageSquare, look.accent, 12.))
                        }),
                )
                .test_support();
            children.push(
                div()
                    .flex()
                    .flex_shrink_0()
                    .min_h(px(18.))
                    .line_height(px(18.))
                    .when(lit, |row| row.bg(rgb(lit_color)))
                    .when(!wrap, |row| row.min_w(px(min_width)))
                    .child(gutter)
                    .child({
                        let text_cell = div()
                            .flex_1()
                            .min_w_0()
                            .pr(px(8.))
                            .text_color(rgb(tint))
                            .when(!wrap, |text| text.whitespace_nowrap());
                        let text_cell = if line_spans.is_empty() {
                            text_cell.child(line.clone())
                        } else {
                            text_cell.child(
                                StyledText::new(line.clone()).with_highlights(line_spans.to_vec()),
                            )
                        };
                        text_cell.test_text(line.clone())
                    })
                    .into_any_element(),
            );
            child_lines.push(n);

            let last_line = n == total;
            let mut under: Vec<&Comment> = by_line.get(&n).cloned().unwrap_or_default();
            if last_line {
                // Comments whose lines are gone sit under the last line.
                for later in by_line.range(n + 1..).map(|(_, comments)| comments) {
                    under.extend(later.iter().copied());
                }
            }
            if fold_at == Some(n)
                && let Some(count) = fold_count
            {
                children.push(
                    comment_cards::fold_line(look, &fold_key, count, !folded, &self.unfolded)
                        .into_any_element(),
                );
                child_lines.push(n);
            }
            for comment in under {
                if Some(&comment.id) == box_comment.as_ref() {
                    continue;
                }
                if folded && comment_cards::is_done(comment) {
                    continue;
                }
                children.push(if read.is_some() {
                    crate::doc_history::recorded_card(look, comment).into_any_element()
                } else {
                    comment_cards::card_with_todo(
                        ui,
                        look,
                        comment,
                        agent,
                        session.pull_request_review,
                        reattaching.as_ref() == Some(&comment.id),
                        &start,
                        &todo_ctx,
                    )
                    .into_any_element()
                });
                child_lines.push(n);
            }
            if let (Some(b), Some((_, last))) = (editing, box_range)
                && (last == n || (last_line && last > total))
            {
                children.push(
                    comment_cards::open_box(
                        ui,
                        look,
                        &session.id,
                        &b.anchor,
                        &self.comment_input,
                        agent,
                        &b.text,
                        &comment_cards::offers(state, &session.id, &b.anchor.repo),
                    )
                    .into_any_element(),
                );
                child_lines.push(n);
            }
        }
        *page.child_lines.borrow_mut() = child_lines;

        div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .id("doc-source")
                    .role(Role::Document)
                    .aria_label(format!("{path} source"))
                    .track_scroll(&page.scroll)
                    .flex()
                    .flex_col()
                    .size_full()
                    .py(px(10.))
                    .overflow_y_scroll()
                    .when(!wrap, |pane| pane.overflow_x_scroll())
                    .bg(rgb(look.term))
                    .font_family("Menlo")
                    .text_size(px(11.5))
                    .text_color(rgb(look.term_text))
                    .children(children)
                    .test_support(),
            )
            .vertical_scrollbar(&page.scroll)
            .into_any_element()
    }
}

/// The bar above a side-by-side pair: what linking does, the checkbox, and
/// Send for the drafts while no box is open.
#[allow(clippy::too_many_arguments)]
fn link_bar(
    ui: &Rc<Ui>,
    session: &SessionId,
    repo: &str,
    path: &str,
    linked: bool,
    long: bool,
    drafts: usize,
    look: Look,
) -> Div {
    let words = match (linked, long) {
        (true, true) => {
            "Scrolling is linked: the page follows the source. Comment on source lines; the page shows where they are."
        }
        (true, false) => "Scrolling is linked",
        (false, true) => {
            "Scrolling is not linked: the page and the source scroll on their own. Comment on source lines; the page shows where they are."
        }
        (false, false) => "Scrolling is not linked",
    };
    let toggle = {
        let ui = ui.clone();
        let (repo, path) = (repo.to_owned(), path.to_owned());
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            ui.send(
                AppAction::Session(SessionAction::LinkScrolling {
                    repo: repo.clone(),
                    path: path.clone(),
                    linked: !linked,
                }),
                cx,
            );
        }
    };
    row()
        .h(px(26.))
        .px(px(12.))
        .gap(px(6.))
        .flex_shrink_0()
        .bg(rgb(look.panel))
        .border_b_1()
        .border_color(rgb(look.line))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(chrome::icon(IconName::Columns2, look.secondary, 12.))
        .child(div().min_w_0().child(words).test_text(words))
        .child(div().flex_1())
        .children((drafts > 0).then(|| {
            widgets::push_primary(
                ui,
                "doc-send-comments",
                format!("Send {drafts} comment{}", if drafts == 1 { "" } else { "s" }),
                AppAction::Comments(CommentsAction::SendAll(session.clone())),
                look,
            )
        }))
        .child(
            row()
                .id("doc-link-scrolling")
                .role(Role::CheckBox)
                .aria_label("Link scrolling")
                .aria_selected(linked)
                .test_support()
                .gap(px(7.))
                .flex_shrink_0()
                .text_size(px(12.5))
                .cursor_pointer()
                .on_click(toggle)
                .child(
                    row()
                        .size(px(14.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .justify_center()
                        .when(linked, |box_| {
                            box_.bg(rgb(look.accent))
                                .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                        })
                        .when(!linked, |box_| {
                            box_.border_1()
                                .border_color(rgb(look.tertiary))
                                .bg(rgb(look.card))
                        }),
                )
                .child("Link scrolling"),
        )
}

/// How a markdown tab shows its file, at the right end of its group's tab
/// row. `None` unless the group's selected tab is a markdown file.
pub fn mode_switch(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    group: usize,
    tabs: &TabGroup,
) -> Option<AnyElement> {
    let Some(Tab::File { repo, path }) = tabs.tabs.get(tabs.selected) else {
        return None;
    };
    let is_intent = humata2_intent_spec::is_intent_spec(path);
    if !is_markdown(path) && !is_intent {
        return None;
    }
    let current = state
        .session
        .view()
        .map(|view| view.document(repo, path))
        .unwrap_or_default()
        .mode;
    let segments = if is_intent {
        [
            (DocumentMode::Preview, "Structured", "structured"),
            (DocumentMode::Source, "YAML", "yaml"),
            (DocumentMode::SideBySide, "Diff", "diff"),
        ]
    } else {
        [
            (DocumentMode::Preview, "Preview", "preview"),
            (DocumentMode::Source, "Source", "source"),
            (DocumentMode::SideBySide, "Side by side", "side-by-side"),
        ]
    };
    Some(
        row()
            .id(SharedString::from(format!("doc-modes-{group}")))
            .role(Role::RadioGroup)
            .aria_label("Show the file as")
            .test_support()
            .mr(px(8.))
            .p(px(2.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .flex_shrink_0()
            .children(segments.into_iter().map(|(mode, label, slug)| {
                let on = mode == current;
                let ui = ui.clone();
                let (repo, path) = (repo.to_owned(), path.clone());
                div()
                    .id(SharedString::from(format!("doc-mode-{group}-{slug}")))
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(on)
                    .px(px(12.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .text_size(px(11.5))
                    .text_color(rgb(look.text))
                    .cursor_pointer()
                    .when(on, |segment| segment.bg(rgb(look.card)).shadow_sm())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        ui.send(
                            AppAction::Session(SessionAction::DocumentMode {
                                repo: repo.clone(),
                                path: path.clone(),
                                mode,
                            }),
                            cx,
                        );
                    })
                    .child(label)
                    .test_support()
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    #[test]
    fn lru_map_evicts_oldest_and_promotes_on_access() {
        let mut lru = LruMap::new(MAX_CACHED_VIEWS);
        for i in 0..MAX_CACHED_VIEWS {
            lru.insert(i, i * 10);
        }
        assert_eq!(lru.len(), MAX_CACHED_VIEWS);
        // Touch key 0 so key 1 becomes the LRU entry.
        assert_eq!(lru.get_mut(&0).copied(), Some(0));
        // Inserting one more evicts key 1 and retains key 0.
        lru.insert(MAX_CACHED_VIEWS, MAX_CACHED_VIEWS * 10);
        assert_eq!(lru.len(), MAX_CACHED_VIEWS);
        assert_eq!(lru.get(&0), Some(&0));
        assert_eq!(lru.get(&1), None);
        assert_eq!(lru.get(&MAX_CACHED_VIEWS), Some(&(MAX_CACHED_VIEWS * 10)));

        lru.retain(|k, _| *k % 2 == 0);
        assert!(lru.get(&0).is_some());
        assert!(lru.get(&3).is_none());
    }
}

