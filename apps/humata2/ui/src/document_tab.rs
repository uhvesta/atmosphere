//! A markdown file open as a session tab (D166, flow 26.20, frames 39 and
//! 41 to 43): the rendered page, the source with comments on its lines, or
//! both side by side with the page following the source.
//!
//! The holder keeps what GPUI owns for each open document: the page's
//! `MarkdownView`, the source pane's scroll handle, and one text field for
//! the comment box. Everything else is read from the store on each draw.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::comments::{Anchor, Comment, CommentId, Side};
use humata2_clients::model::{Session, SessionId};
use humata2_markdown::{
    Anchor as PageAnchor, Comment as PageComment, CommentMarks, LineRange, MarkdownOptions, MarkdownView,
};
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

/// A comment as the page shows it: where it is and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mark {
    id: String,
    first: u32,
    last: u32,
    text: String,
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
}

/// What a sync needs from the store, copied out so no store borrow is held
/// while entities change.
#[derive(Default)]
struct Plan {
    sessions: Vec<SessionId>,
    documents: Vec<Want>,
    /// The open comment box: session, first line, last line, text.
    editing: Option<(SessionId, u32, u32, String)>,
}

struct Want {
    key: Key,
    source: String,
    marks: Vec<Mark>,
    follow: bool,
}

fn plan(state: &AppState) -> Plan {
    let mut plan = Plan {
        sessions: state
            .workspace
            .sessions
            .iter()
            .map(|session| session.id.clone())
            .collect(),
        ..Plan::default()
    };
    let (Some(id), Some(view)) = (state.session.open.as_ref(), state.session.view()) else {
        return plan;
    };
    plan.editing = state
        .comments
        .sessions
        .get(id)
        .and_then(|held| held.editing.as_ref())
        .map(|b| (id.clone(), b.anchor.first, b.anchor.last, b.text.clone()));
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
pub(crate) struct Documents {
    pages: HashMap<Key, Page>,
    comment_input: Entity<TextareaState>,
    /// The box the field was last set for: session, lines and text.
    seen: Option<(SessionId, u32, u32, String)>,
    /// The detached comment whose new line the next click picks.
    reattach: Rc<RefCell<Option<CommentId>>>,
    /// The Diff panel's code views, by `repo/path`.
    diff_views: RefCell<HashMap<String, DiffView>>,
    /// The editors of changed code files, by repository and path.
    editors: RefCell<HashMap<(String, String), EditorView>>,
    pub(crate) conversation: crate::conversation_tab::ConversationScroll,
    _typing: Subscription,
}

/// What the Diff panel remembers about one file's code view so a rewrap can
/// keep its top line in place.
#[derive(Clone)]
pub(crate) struct DiffView {
    pub(crate) handle: ScrollHandle,
    anchor: Rc<RefCell<Option<TopAnchor>>>,
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
    /// Called with the bounds of the code body's children on every frame.
    /// While the width stays, notes which child is at the top; when it
    /// changes under wrap, moves the offset so that child is at the top
    /// again and asks for one more frame.
    pub(crate) fn keep_top(&self, width: f32, wrap: bool, children: &[Bounds<Pixels>], cx: &mut App) {
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
    pub(crate) fn new(ui: &Rc<Ui>, window: &mut Window, cx: &mut App) -> Self {
        let comment_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Write a comment")
                .auto_grow(1, 12)
        });
        let ui = ui.clone();
        let typing = cx.subscribe(
            &comment_input,
            move |input: Entity<TextareaState>, event: &InputEvent, cx: &mut App| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                let Some(session) = ui.read(|state| state.session.open.clone()) else {
                    return;
                };
                let held = ui.read(|state| {
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
                    ui.send(
                        AppAction::Comments(CommentsAction::Typed(session, value)),
                        cx,
                    );
                }
            },
        );
        Self {
            pages: HashMap::new(),
            comment_input,
            seen: None,
            reattach: Rc::new(RefCell::new(None)),
            diff_views: RefCell::new(HashMap::new()),
            editors: RefCell::new(HashMap::new()),
            conversation: Default::default(),
            _typing: typing,
        }
    }

    /// The scroll state of the Diff panel's code view for `name`.
    pub(crate) fn diff_view(&self, name: &str) -> DiffView {
        self.diff_views
            .borrow_mut()
            .entry(name.to_owned())
            .or_insert_with(|| DiffView {
                handle: ScrollHandle::new(),
                anchor: Rc::new(RefCell::new(None)),
            })
            .clone()
    }

    /// The editor state of a changed code file, kept across frames.
    pub(crate) fn editor_view(&self, repo: &str, path: &str) -> EditorView {
        self.editors
            .borrow_mut()
            .entry((repo.to_owned(), path.to_owned()))
            .or_insert_with(EditorView::new)
            .clone()
    }

    /// The comment box's text field, shared by every file view.
    pub(crate) fn input(&self) -> Entity<TextareaState> {
        self.comment_input.clone()
    }

    /// The cell holding the detached comment whose new line the next click
    /// picks.
    pub(crate) fn reattach(&self) -> Rc<RefCell<Option<CommentId>>> {
        self.reattach.clone()
    }

    /// Brings the pages, the comment field and the link in step with the
    /// store. Called before each draw of the window.
    pub(crate) fn sync(&mut self, ui: &Rc<Ui>, window: &mut Window, cx: &mut App) {
        let plan = ui.read(plan);
        let mut following = HashSet::new();
        for want in plan.documents {
            if want.follow {
                following.insert(want.key.clone());
            }
            match self.pages.get_mut(&want.key) {
                None => {
                    let mut options = MarkdownOptions::new(want.key.2.clone());
                    // Comments are written on source lines; the page only
                    // shows where they are.
                    options.comments = false;
                    options.marks = CommentMarks::Counts;
                    // The page's own typography, and a byline ending in the
                    // file's name.
                    options.document = Some(file_name(&want.key.2).into());
                    let source = want.source.clone();
                    let view = cx.new(|cx| MarkdownView::new(&source, options, cx));
                    set_marks(&view, &want.marks, cx);
                    self.pages.insert(
                        want.key,
                        Page {
                            view,
                            source: want.source,
                            marks: want.marks,
                            scroll: ScrollHandle::new(),
                            child_lines: Rc::new(RefCell::new(Vec::new())),
                            last_top: None,
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
        self.pages
            .retain(|(session, _, _), _| plan.sessions.contains(session));

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

        // Linked scrolling: the block at the source's top line goes to the
        // top of the page.
        for (key, page) in &mut self.pages {
            if !following.contains(key) {
                page.last_top = None;
                continue;
            }
            let top = page
                .child_lines
                .borrow()
                .get(page.scroll.top_item())
                .copied();
            if let Some(top) = top
                && page.last_top != Some(top)
            {
                page.last_top = Some(top);
                page.view.update(cx, |view, cx| {
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
    pub(crate) fn body(
        &self,
        ui: &Rc<Ui>,
        state: &AppState,
        session: &Session,
        repo: &str,
        path: &str,
        look: Look,
    ) -> Option<AnyElement> {
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
        let preview = || {
            div()
                .id("doc-preview")
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
                        .child(page.view.clone()),
                )
                .test_support()
        };
        Some(match shown.mode {
            DocumentMode::Preview => preview().into_any_element(),
            DocumentMode::Source => self.source(ui, state, session, page, repo, path, text, look),
            DocumentMode::SideBySide => {
                let groups = state.session.view().map_or(1, |view| view.groups.len());
                let drafts = state.comments.sendable(&session.id).len();
                let boxed = state
                    .comments
                    .sessions
                    .get(&session.id)
                    .is_some_and(|held| held.editing.is_some());
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
                        if boxed { 0 } else { drafts },
                        look,
                    ))
                    .child(
                        row()
                            .flex_1()
                            .min_h_0()
                            .items_stretch()
                            .child(self.source(ui, state, session, page, repo, path, text, look))
                            .child(div().w(px(1.)).h_full().flex_shrink_0().bg(rgb(look.line)))
                            .child(preview()),
                    )
                    .into_any_element()
            }
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
        look: Look,
    ) -> AnyElement {
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let total = lines.len() as u32;
        let by_line = state.comments.on_file(&session.id, repo, path);
        let editing = state
            .comments
            .sessions
            .get(&session.id)
            .and_then(|held| held.editing.as_ref())
            .filter(|b| b.anchor.repo == repo && b.anchor.path == path);
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
            let click = target.clone();
            let gutter = div()
                .id(SharedString::from(format!("doc-line-{n}")))
                .role(Role::Button)
                .aria_label(format!("Comment on line {n}"))
                .flex()
                .flex_shrink_0()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    click.click(n, event.modifiers.shift, cx);
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pr(px(8.))
                            .text_color(rgb(tint))
                            .when(!wrap, |text| text.whitespace_nowrap())
                            .child(line.clone())
                            .test_text(line.clone()),
                    )
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
            for comment in under {
                if Some(&comment.id) == box_comment.as_ref() {
                    continue;
                }
                children.push(
                    comment_cards::card(
                        ui,
                        look,
                        comment,
                        agent,
                        reattaching.as_ref() == Some(&comment.id),
                        &start,
                    )
                    .into_any_element(),
                );
                child_lines.push(n);
            }
            if let (Some(b), Some((_, last))) = (editing, box_range)
                && (last == n || (last_line && last > total))
            {
                children.push(
                    comment_cards::open_box(ui, look, &session.id, &b.anchor, &self.comment_input, agent)
                        .into_any_element(),
                );
                child_lines.push(n);
            }
        }
        *page.child_lines.borrow_mut() = child_lines;

        div()
            .id("doc-source")
            .role(Role::Document)
            .aria_label(format!("{path} source"))
            .track_scroll(&page.scroll)
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .py(px(10.))
            .overflow_y_scroll()
            .when(!wrap, |pane| pane.overflow_x_scroll())
            .bg(rgb(look.term))
            .font_family("Menlo")
            .text_size(px(11.5))
            .text_color(rgb(look.term_text))
            .children(children)
            .test_support()
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
pub(crate) fn mode_switch(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    group: usize,
    tabs: &TabGroup,
) -> Option<AnyElement> {
    let Some(Tab::File { repo, path }) = tabs.tabs.get(tabs.selected) else {
        return None;
    };
    if !is_markdown(path) {
        return None;
    }
    let current = state
        .session
        .view()
        .map(|view| view.document(repo, path))
        .unwrap_or_default()
        .mode;
    let segments = [
        (DocumentMode::Preview, "Preview", "preview"),
        (DocumentMode::Source, "Source", "source"),
        (DocumentMode::SideBySide, "Side by side", "side-by-side"),
    ];
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
                let (repo, path) = (repo.clone(), path.clone());
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
