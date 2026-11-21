//! The History strip of a Markdown tab in a Desk folder (DV-3 to DV-8) and
//! the compare view that Compare with current opens in the Changes panel
//! (DV-7).
//!
//! The strip sits under the tab row and above the page. It is drawn only for
//! a file that has a history ([`humata2_session::SessionState::history`]).
//! While an old version is shown, a second bar names it and offers Compare
//! with current, Restore as new version and Back to current; the tab shows
//! that version's text and recorded comments, and nothing can be edited.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, ObservedElement, TestSupportExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::comments::{Comment, Status};
use humata2_clients::docs::{DocVersion, VersionText};
use humata2_clients::model::{DiffLine, LineKind, SessionId};
use humata2_markdown::{CommentMarks, MarkdownOptions, MarkdownView, MermaidCache};
use humata2_session::{DocHistory, HistoryAction, SessionAction, Tab, is_markdown};

use crate::bridge::Ui;
use crate::chrome;
use crate::document_tab::{LruMap, MAX_CACHED_VIEWS};
use crate::theme::Look;
use crate::widgets::{self, col, row};

/// A viewed version the page needs a read-only view of.
struct Viewed {
    key: (SessionId, String),
    id: String,
    text: String,
}

/// What the holder needs from the store to keep the strip's parts in step,
/// copied out so no store borrow is held while entities change.
#[derive(Default)]
pub struct Wanted {
    viewed: Vec<Viewed>,
    /// The selected history tab's path and the text in its Save version box.
    message: Option<(String, String)>,
}

/// What the strip needs from the store: the old versions being shown, and
/// the Save version box of the selected Markdown tab.
pub fn wanted(state: &AppState) -> Wanted {
    let mut out = Wanted::default();
    let (Some(id), Some(view)) = (state.session.open.as_ref(), state.session.view()) else {
        return out;
    };
    for group in &view.groups {
        if out.message.is_none()
            && let Some(Tab::File { repo, path }) = group.tabs.get(group.selected)
            && repo.is_empty()
            && (is_markdown(path) || humata2_intent_spec::is_intent_spec(path))
            && let Some(history) = state.session.history(id, path)
        {
            out.message = Some((path.clone(), history.message.clone()));
        }
        for tab in &group.tabs {
            let Tab::File { repo, path } = tab else {
                continue;
            };
            if !repo.is_empty() || !is_markdown(path) {
                continue;
            }
            let Some(history) = state.session.history(id, path) else {
                continue;
            };
            let key = (id.clone(), path.clone());
            if let Some(read) = history.viewed.as_ref().filter(|_| history.viewing().is_some())
                && !out.viewed.iter().any(|seen| seen.key == key)
            {
                out.viewed.push(Viewed {
                    key,
                    id: read.version.id.clone(),
                    text: read.text.clone(),
                });
            }
        }
    }
    out
}

/// The Save version box's placeholder.
const PLACEHOLDER: &str = "What changed? (optional)";

/// What the History strip keeps that GPUI owns: the Save version box, the
/// open flag of the History list, and a read-only page for each old version
/// being shown.
pub struct HistoryUi {
    message: Entity<InputState>,
    /// The file whose Save version box the field is, set by each sync.
    path: Rc<RefCell<Option<String>>>,
    menu: Rc<Cell<bool>>,
    /// Whether the Save version box has focus, read at each sync. Unfocused
    /// and empty, the box shows its own placeholder, cut with an ellipsis.
    focused: Cell<bool>,
    viewed: LruMap<(SessionId, String), (String, Entity<MarkdownView>)>,
    diagrams: MermaidCache,
    _typing: Subscription,
}

impl HistoryUi {
    pub fn new(ui: &Rc<Ui>, diagrams: MermaidCache, window: &mut Window, cx: &mut App) -> Self {
        let message =
            cx.new(|cx| InputState::new(window, cx).placeholder(PLACEHOLDER));
        let path: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let (ui, shown) = (ui.clone(), path.clone());
        let typing = cx.subscribe(
            &message,
            move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let Some(path) = shown.borrow().clone() else {
                    return;
                };
                let value = input.read(cx).value().to_string();
                let held = ui.read(|state| {
                    state
                        .session
                        .open
                        .as_ref()
                        .and_then(|session| state.session.history(session, &path))
                        .map(|history| history.message.clone())
                });
                // The field also changes when the store sets it; only the
                // person's typing is news.
                if held.is_some_and(|held| held != value) {
                    ui.send(
                        AppAction::Session(SessionAction::History(HistoryAction::Message {
                            path,
                            text: value,
                        })),
                        cx,
                    );
                }
            },
        );
        Self {
            message,
            path,
            menu: Rc::new(Cell::new(false)),
            focused: Cell::new(false),
            viewed: LruMap::new(MAX_CACHED_VIEWS),
            diagrams,
            _typing: typing,
        }
    }

    /// Brings the Save version box and the old versions' pages in step with
    /// the store. Called before each draw of the window.
    pub fn sync(&mut self, wanted: &Wanted, window: &mut Window, cx: &mut App) {
        self.focused
            .set(self.message.read(cx).focus_handle(cx).is_focused(window));
        *self.path.borrow_mut() = wanted.message.as_ref().map(|(path, _)| path.clone());
        if let Some((_, message)) = &wanted.message
            && self.message.read(cx).value().to_string() != *message
        {
            let message = message.clone();
            self.message
                .update(cx, |input, cx| input.set_value(message, window, cx));
        }
        for want in &wanted.viewed {
            match self.viewed.get_mut(&want.key) {
                Some((id, _)) if *id == want.id => {}
                Some((id, view)) => {
                    let text = want.text.clone();
                    view.update(cx, |view, cx| view.set_source(&text, cx));
                    id.clone_from(&want.id);
                }
                None => {
                    let mut options = MarkdownOptions::new(want.key.1.clone());
                    options.comments = false;
                    options.marks = CommentMarks::Counts;
                    options.diagrams = self.diagrams.clone();
                    let name = want.key.1.rsplit('/').next().unwrap_or(&want.key.1);
                    options.document = Some(name.to_owned().into());
                    let text = want.text.clone();
                    let view = cx.new(|cx| MarkdownView::new(&text, options, cx));
                    self.viewed.insert(want.key.clone(), (want.id.clone(), view));
                }
            }
        }
        self.viewed
            .retain(|key, _| wanted.viewed.iter().any(|want| &want.key == key));
    }

    /// The read-only page of version `id` of the file, once it is made.
    pub fn viewed_page(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Option<Entity<MarkdownView>> {
        self.viewed
            .get(&(session.clone(), path.to_owned()))
            .filter(|(held, _)| held == id)
            .map(|(_, view)| view.clone())
    }

    /// The strip, and under it the bar naming the shown old version.
    /// `read` is that version's text, once read.
    pub fn strip(
        &self,
        ui: &Rc<Ui>,
        history: &DocHistory,
        path: &str,
        read: Option<&VersionText>,
        look: Look,
    ) -> Div {
        let act = |action: HistoryAction| AppAction::Session(SessionAction::History(action));
        let back = widgets::ghost_in(
            look,
            ui,
            "doc-history-back",
            "",
            act(HistoryAction::Back { path: path.to_owned() }),
        )
        .icon(IconName::ChevronLeft)
        // The frames' step buttons are 26 pt square-ish, not label-wide.
        .w(px(26.))
        .px_0()
        .accessibility_label("Older version")
        .disabled(!history.can_back());
        let forward = widgets::ghost_in(
            look,
            ui,
            "doc-history-forward",
            "",
            act(HistoryAction::Forward { path: path.to_owned() }),
        )
        .icon(IconName::ChevronRight)
        // The frames' step buttons are 26 pt square-ish, not label-wide.
        .w(px(26.))
        .px_0()
        .accessibility_label("Newer version")
        .disabled(!history.can_forward());

        let viewing = history.viewing();
        let label = match (viewing, history.newest()) {
            (Some(version), newest) => format!("v{} of {}", version.number, newest.unwrap_or(version.number)),
            (None, _) if !history.loaded => "Current".to_owned(),
            (None, None) => "No versions yet".to_owned(),
            (None, Some(_)) if history.unsaved => "Current · unsaved changes".to_owned(),
            (None, Some(_)) => "Current".to_owned(),
        };

        let menu = {
            let open = self.menu.clone();
            widgets::push_quiet_shape("doc-history-menu", "History ▾", look)
                .disabled(history.versions.is_empty())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    open.set(!open.get());
                    cx.refresh_windows();
                })
        };
        let list = (self.menu.get() && !history.versions.is_empty()).then(|| {
            let close = self.menu.clone();
            let scrim = div()
                .id("doc-history-scrim")
                .absolute()
                .top(px(-10_000.))
                .left(px(-10_000.))
                .w(px(20_000.))
                .h(px(20_000.))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    close.set(false);
                    cx.refresh_windows();
                });
            let card = crate::action_menu::card("doc-history-list", 400., look)
                .top(px(28.))
                .left(px(0.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .children(history.versions.iter().map(|version| {
                    let (ui, close) = (ui.clone(), self.menu.clone());
                    let (path, id) = (path.to_owned(), version.id.clone());
                    let on = viewing.is_some_and(|shown| shown.id == version.id);
                    version_row(version, on, look)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            close.set(false);
                            ui.send(
                                AppAction::Session(SessionAction::History(HistoryAction::Pick {
                                    path: path.clone(),
                                    id: id.clone(),
                                })),
                                cx,
                            );
                        })
                }));
            (scrim, card)
        });

        let mut bar = row()
            .id("doc-history-strip")
            .role(Role::Region)
            .aria_label("Versions")
            .test_support()
            .h(px(30.))
            .px(px(12.))
            .gap(px(6.))
            .flex_shrink_0()
            .bg(rgb(look.panel))
            .border_b_1()
            .border_color(rgb(look.line))
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(back)
            .child(div().id("doc-history-label").role(Role::Region).aria_label(label.clone()).child(label.clone()).test_support())
            .child(forward)
            .child(
                div().relative().flex_shrink_0().child(menu).children(
                    list.map(|(scrim, card)| {
                        div().child(deferred(scrim)).child(deferred(card))
                    }),
                ),
            )
            .child(div().flex_1());
        if !history.read_only() {
            let reason = (history.loaded && !history.can_save() && !history.working).then(|| {
                match history.newest() {
                    Some(newest) => format!("No changes since v{newest}"),
                    None => "No changes to save".to_owned(),
                }
            });
            bar = bar
                .children(reason.map(|reason| {
                    div()
                        .id("doc-save-reason")
                        .text_color(rgb(look.tertiary))
                        .role(Role::Region)
                        .aria_label(reason.clone())
                        .child(reason)
                        .test_support()
                }))
                .child(
                    div()
                        .id("doc-version-message")
                        .relative()
                        .w(px(210.))
                        .min_w_0()
                        .flex_shrink(1.)
                        .child(
                            Input::new(&self.message)
                                .small()
                                .min_h(px(24.))
                                .rounded(px(6.))
                                .bg(rgb(look.card))
                                .border_color(rgb(look.line))
                                .text_size(px(12.)),
                        )
                        // The input cuts a long placeholder mid-word in a
                        // narrow pane; over it, unfocused and empty, the same
                        // words end in an ellipsis (frame 41). The input keeps
                        // its placeholder for screen readers.
                        .when(history.message.is_empty() && !self.focused.get(), |field| {
                            field.child(
                                div()
                                    .absolute()
                                    .top(px(1.))
                                    .bottom(px(1.))
                                    .left(px(1.))
                                    .right(px(1.))
                                    .flex()
                                    .items_center()
                                    .px(px(8.))
                                    .rounded(px(5.))
                                    .bg(rgb(look.card))
                                    .text_size(px(12.))
                                    .text_color(rgb(look.tertiary))
                                    .child(div().min_w_0().truncate().child(PLACEHOLDER)),
                            )
                        })
                        .test_support(),
                )
                .child(
                    widgets::push_primary(
                        ui,
                        "doc-save-version",
                        "Save version",
                        act(HistoryAction::SaveVersion { path: path.to_owned() }),
                        look,
                    )
                    .disabled(!history.can_save()),
                );
        }

        let mut out = col().flex_shrink_0().child(bar);
        if let Some(version) = viewing {
            out = out.child(version_bar(ui, history, version, path, read, look));
        }
        out
    }
}

/// One row of the History list: number, time, who, message, +added −removed.
fn version_row(version: &DocVersion, on: bool, look: Look) -> ObservedElement<Stateful<Div>> {
    row()
        .id(SharedString::from(format!("doc-history-version-{}", version.number)))
        .role(Role::Button)
        .aria_label(format!("v{}", version.number))
        .test_support()
        .gap(px(8.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(rgb(look.text))
        .when(on, |row| row.bg(rgb(look.fill)))
        .child(div().w(px(28.)).font_weight(FontWeight::SEMIBOLD).child(format!("v{}", version.number)))
        .child(div().text_color(rgb(look.secondary)).child(when(version.at_ms)))
        .child(div().text_color(rgb(look.secondary)).child(version.who.clone()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(version.message.clone()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(0x30d158))
                .child(format!("+{}", version.added)),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(0xff453a))
                .child(format!("−{}", version.removed)),
        )
}

/// The bar under the strip while an old version is shown: which one, and
/// what can be done with it.
fn version_bar(
    ui: &Rc<Ui>,
    history: &DocHistory,
    version: &DocVersion,
    path: &str,
    read: Option<&VersionText>,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let act = |action: HistoryAction| AppAction::Session(SessionAction::History(action));
    let named = format!(
        "v{} · {} · {} · {}",
        version.number,
        when(version.at_ms),
        version.who,
        version.message
    );
    let ready = read.is_some() && !history.working;
    col()
        .id("doc-version-bar")
        .role(Role::Region)
        .aria_label("Old version")
        .test_support()
        .flex_shrink_0()
        .bg(rgb(look.fill))
        .border_b_1()
        .border_color(rgb(look.line))
        .text_size(px(11.5))
        .child(
            row()
                .h(px(30.))
                .px(px(12.))
                .gap(px(8.))
                .text_color(rgb(look.text))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(named))
                .child(
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .text_color(rgb(look.secondary))
                        .child("read-only"),
                )
                .child(div().flex_1())
                .child(
                    widgets::push_quiet(
                        ui,
                        "doc-compare-current",
                        "Compare with current",
                        act(HistoryAction::CompareWithCurrent { path: path.to_owned() }),
                        look,
                    )
                    .disabled(!ready),
                )
                .child(
                    widgets::push_primary(
                        ui,
                        "doc-restore-version",
                        "Restore as new version",
                        act(HistoryAction::Restore { path: path.to_owned() }),
                        look,
                    )
                    .disabled(!ready),
                )
                .child(widgets::push_quiet(
                    ui,
                    "doc-back-to-current",
                    "Back to current",
                    act(HistoryAction::BackToCurrent { path: path.to_owned() }),
                    look,
                )),
        )
        .children(read.filter(|read| read.comments.is_none()).map(|_| {
            div()
                .id("doc-comments-not-recorded")
                .px(px(12.))
                .pb(px(6.))
                .text_color(rgb(look.secondary))
                .role(Role::Region)
                .child("Comments weren't recorded for this version")
                .test_support()
        }))
}

/// A comment recorded with an old version, grouped under the line it ends
/// on (at least 1), for the read-only source.
pub fn recorded_by_line(read: &VersionText) -> std::collections::BTreeMap<u32, Vec<&Comment>> {
    let mut rows: std::collections::BTreeMap<u32, Vec<&Comment>> = std::collections::BTreeMap::new();
    for comment in read.comments.iter().flatten().filter(|comment| !comment.archived) {
        rows.entry(comment.anchor.last.max(1)).or_default().push(comment);
    }
    rows
}

/// A recorded comment as the old version had it: who, where, what it said,
/// and nothing to press.
pub fn recorded_card(look: Look, comment: &Comment) -> ObservedElement<Stateful<Div>> {
    let lines = crate::comment_cards::lines_label(&comment.anchor);
    let status = match comment.status {
        Status::Draft => "Draft",
        Status::Waiting => "Waiting to send",
        Status::Sent => "Sent",
        Status::Addressed => "Addressed",
        Status::Resolved => "Resolved",
        Status::Posted => "Posted",
    };
    col()
        .id(SharedString::from(format!("doc-recorded-comment-{}", comment.id)))
        .role(Role::Region)
        .test_support()
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .gap(px(4.))
        .max_w(px(640.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .text_color(rgb(look.text))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("You · {lines} · {status}")),
        )
        .child(comment.text.clone())
}

/// What a tab says while it reads an old version's text.
pub fn reading(look: Look) -> AnyElement {
    div()
        .id("doc-version-reading")
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(look.content))
        .text_size(px(12.))
        .text_color(rgb(look.secondary))
        .role(Role::Region)
        .child("Reading this version")
        .test_support()
        .into_any_element()
}

/// `2026-11-15 12:52` (UTC) for a time in milliseconds since the Unix epoch.
fn when(at_ms: i64) -> String {
    let seconds = at_ms.div_euclid(1000);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days since 1970-01-01 to a calendar date (Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

/// The Changes panel's body while a version is being compared with the
/// current file (DV-7): the daemon's diff of the version against the file as
/// it is, or a note while it is read or if it failed. `None` when nothing is
/// being compared in the open session.
pub fn compare_body(ui: &Rc<Ui>, state: &AppState, look: Look) -> Option<Div> {
    let compare = state.session.compare.as_ref()?;
    if state.session.open.as_ref() != Some(&compare.session) {
        return None;
    }
    let history = state.session.history(&compare.session, &compare.path)?;
    let version = history.versions.iter().find(|version| version.id == compare.version)?;
    let title = format!("{} · v{} against the current file", compare.path, version.number);
    let head = row()
        .h(px(30.))
        .px(px(10.))
        .gap(px(6.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .child(chrome::icon(IconName::FileDiff, look.secondary, 12.))
        .child(div().font_weight(FontWeight::MEDIUM).child(title))
        .child(div().flex_1())
        .child(widgets::push_quiet(
            ui,
            "doc-compare-close",
            "Back to current",
            AppAction::Session(SessionAction::History(HistoryAction::BackToCurrent {
                path: compare.path.clone(),
            })),
            look,
        ));
    let note = |id: &'static str, text: String| {
        div()
            .id(id)
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .px(px(16.))
            .bg(rgb(look.content))
            .text_size(px(12.))
            .text_color(rgb(look.secondary))
            .role(Role::Region)
            .child(text)
            .test_support()
            .into_any_element()
    };
    let body = match &compare.diff {
        humata2_session::CompareDiff::Reading => reading(look),
        humata2_session::CompareDiff::Failed(message) => note("doc-compare-failed", message.clone()),
        humata2_session::CompareDiff::Loaded(lines) if lines.is_empty() => {
            note("doc-compare-same", "This version reads the same as the current file".to_owned())
        }
        humata2_session::CompareDiff::Loaded(lines) => {
            let rows: Vec<AnyElement> = lines
                .iter()
                .map(|line: &DiffLine| {
                    if line.kind == LineKind::Hunk {
                        return div()
                            .flex_shrink_0()
                            .h(px(18.))
                            .px(px(10.))
                            .text_color(rgb(look.term_dim))
                            .child(line.text.clone())
                            .into_any_element();
                    }
                    let (sign, tint) = match line.kind {
                        LineKind::Added => ("+", Some(look.add)),
                        LineKind::Removed => ("−", Some(look.del)),
                        _ => (" ", None),
                    };
                    let number = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
                    row()
                        .flex_shrink_0()
                        .h(px(18.))
                        .when_some(tint, |row, tint| row.bg(rgb(tint)))
                        .child(div().w(px(36.)).pr(px(6.)).flex().justify_end().text_color(rgb(look.term_dim)).child(number(line.old)))
                        .child(div().w(px(36.)).pr(px(6.)).flex().justify_end().text_color(rgb(look.term_dim)).child(number(line.new)))
                        .child(div().w(px(14.)).child(sign))
                        .child(div().whitespace_nowrap().child(line.text.clone()))
                        .into_any_element()
                })
                .collect();
            div()
                .id("doc-compare")
                .role(Role::Code)
                .aria_label("Changes since this version")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(6.))
                .bg(rgb(look.term))
                .font_family("Menlo")
                .text_size(px(11.5))
                .text_color(rgb(look.term_text))
                .child(col().children(rows))
                .into_any_element()
        }
    };
    Some(col().flex_1().min_w_0().min_h_0().child(head).child(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    // The glob above brings GPUI's `test` macro; `#[test]` is the standard one.
    use ::core::prelude::v1::test;

    #[test]
    fn a_time_reads_as_a_utc_day_and_minute() {
        assert_eq!(when(0), "1970-01-01 00:00");
        assert_eq!(when(1_795_000_000_000), "2026-11-18 11:06");
    }
}
