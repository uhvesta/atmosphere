//! A terminal session's pane: the view over a `ghostty_vt` terminal, the
//! find bar, the multi-line paste prompt and the offline banner.

use std::cell::RefCell;
use std::rc::Rc;

use ghostty_vt::{ClipboardTarget, Event, Grid as _, Scroll, Terminal, clipboard_read_reply};
use gpui_kit::Pixels;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    SharedString, Subscription, Window, div, px,
};
use terminal_view::{Mark, TerminalView, Theme};

use crate::find::{Finder, MATCH_LIMIT, Query};
use crate::host::{GhosttyHost, PaneEvent, PendingPaste, Pty, Shared, paste_text};

/// What the find bar shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindStatus {
    pub query: Query,
    /// Matches in the whole history, at most the limit.
    pub matches: usize,
    /// More matched than are shown.
    pub capped: bool,
    /// The match the bar is on, 0 being the oldest.
    pub current: Option<usize>,
    /// Why the query can't run, such as a broken regular expression.
    pub error: Option<String>,
}

struct FindBar {
    input: Entity<InputState>,
    query: Query,
    marks: Vec<Mark>,
    current: Option<usize>,
    error: Option<String>,
    _input_events: Subscription,
}

/// One terminal session on screen. Feed it the program's output with
/// [`TerminalPane::feed`]; what the user does goes to its [`Pty`].
pub struct TerminalPane {
    view: Entity<TerminalView<Terminal>>,
    shared: Rc<RefCell<Shared>>,
    find: Option<FindBar>,
    refind_scheduled: bool,
    /// Whether the last frame showed the paste prompt.
    prompt_shown: bool,
    /// Output held while the pane is hidden (NV-34), parsed when it is
    /// shown or when it grows past [`HIDDEN_LIMIT`]; `None` while shown.
    hidden: Option<Vec<u8>>,
    /// Whether the pane draws its own disconnected banner.
    offline_banner: bool,
    _view_changes: Subscription,
}

/// The most output a hidden pane holds unparsed. Past it the terminal
/// parses what it holds, still drawing nothing, so a busy hidden pane
/// costs memory for its scrollback only.
const HIDDEN_LIMIT: usize = 1 << 20;

impl EventEmitter<PaneEvent> for TerminalPane {}

impl TerminalPane {
    pub fn new(
        terminal: Terminal,
        pty: impl Pty + 'static,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let shared = Shared::new(Box::new(pty));
        let host = GhosttyHost {
            shared: shared.clone(),
        };
        let view = cx.new(|cx| TerminalView::new(terminal, host, theme, window, cx));
        // The view redraws after every call it makes to the host; that is
        // when a paste may be waiting or an event queued.
        let view_changes = cx.observe(&view, |pane: &mut Self, _, cx| pane.take_events(cx));
        TerminalPane {
            view,
            shared,
            find: None,
            refind_scheduled: false,
            prompt_shown: false,
            hidden: None,
            offline_banner: true,
            _view_changes: view_changes,
        }
    }

    /// The view, for its theme, cursor blink and stats.
    pub fn view(&self) -> &Entity<TerminalView<Terminal>> {
        &self.view
    }

    /// Output from the program. The terminal parses it; replies to the
    /// program's queries go straight back to the pty, and the rest
    /// becomes [`PaneEvent`]s. A hidden pane only holds it (NV-34).
    pub fn feed(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if let Some(held) = &mut self.hidden {
            held.extend_from_slice(bytes);
            if held.len() > HIDDEN_LIMIT {
                let held = std::mem::take(held);
                self.parse(&held, cx);
            }
            return;
        }
        self.parse(bytes, cx);
    }

    /// Shows or hides the pane. A hidden pane draws nothing and holds the
    /// program's output instead of parsing it, so panes out of sight cost
    /// nothing but their bytes (NV-34); showing it parses what it held and
    /// draws once. While hidden, replies to the program's queries wait
    /// too, until it is shown or holds more than 1 MiB.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        match (visible, self.hidden.take()) {
            (true, Some(held)) => {
                self.parse(&held, cx);
                cx.notify();
            }
            (true, None) => {}
            (false, held) => {
                self.hidden = Some(held.unwrap_or_default());
                cx.notify();
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        self.hidden.is_none()
    }

    fn parse(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let events = self.view.update(cx, |view, cx| {
            let terminal = view.grid_mut();
            terminal.feed(bytes);
            cx.notify();
            terminal.drain_events()
        });
        for event in events {
            match event {
                Event::Reply(bytes) => self.shared.borrow_mut().send(&bytes),
                Event::TitleChanged(title) => cx.emit(PaneEvent::TitleChanged(title)),
                Event::PwdChanged(pwd) => cx.emit(PaneEvent::PwdChanged(pwd)),
                Event::Bell => cx.emit(PaneEvent::Bell),
                Event::ClipboardWrite { target, text } => {
                    cx.emit(PaneEvent::ClipboardWrite { target, text });
                }
                Event::ClipboardReadRequested { target } => {
                    cx.emit(PaneEvent::ClipboardReadRequested { target });
                }
            }
        }
        if self.find.is_some() && !self.refind_scheduled {
            // New output can add or move matches. Search once for all the
            // output that arrives before the app is next idle.
            self.refind_scheduled = true;
            let pane = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = pane.update(cx, |pane, cx| {
                    pane.refind_scheduled = false;
                    pane.refind(cx);
                });
            });
        }
    }

    /// Lets the caller show its own notice instead of the built-in
    /// disconnected banner (on by default).
    pub fn set_offline_banner(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.offline_banner != show {
            self.offline_banner = show;
            cx.notify();
        }
    }

    /// Grid rows taken by the program's output from the top (see
    /// `rows_used`), at most all but the last two rows. 0 for a blank screen.
    pub fn content_rows(&self, cx: &App) -> usize {
        let grid = self.view.read(cx).grid();
        let cursor = grid.cursor();
        let rows = usize::from(grid.size().rows);
        rows_used(
            (0..grid.size().rows).map(|y| {
                grid.row(y)
                    .is_some_and(|row| row.cells.iter().any(|cell| !cell.text.trim().is_empty()))
            }),
            cursor.visible.then_some(usize::from(cursor.y)),
        )
        .min(rows.saturating_sub(2))
    }

    /// The height of one terminal row, as the theme in use sets it.
    pub fn line_height(&self, cx: &App) -> Pixels {
        px(self.view.read(cx).theme().line_height.max(1.0).round())
    }

    /// Whether the session's machine is reachable. While it isn't (SE-16)
    /// typing and pasting are dropped and the pane says so.
    pub fn set_connected(&mut self, connected: bool, cx: &mut Context<Self>) {
        let mut shared = self.shared.borrow_mut();
        if shared.connected != connected {
            shared.connected = connected;
            if !connected {
                shared.pending_paste = None;
            }
            cx.notify();
        }
    }

    pub fn is_connected(&self) -> bool {
        self.shared.borrow().connected
    }

    /// Pastes `text` as if the user had, such as the path of an image the
    /// app uploaded (D30). It may wait for the user like any paste.
    pub fn paste_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.paste(text, false, cx);
    }

    /// A paste waiting for the user to agree to it.
    pub fn pending_paste(&self) -> Option<PendingPaste> {
        self.shared.borrow().pending_paste.clone()
    }

    /// Sends the waiting paste after all.
    pub fn confirm_paste(&mut self, cx: &mut Context<Self>) {
        let pending = self.shared.borrow_mut().pending_paste.take();
        if let Some(pending) = pending {
            self.paste(pending.text, true, cx);
        }
    }

    /// Drops the waiting paste.
    pub fn cancel_paste(&mut self, cx: &mut Context<Self>) {
        if self.shared.borrow_mut().pending_paste.take().is_some() {
            cx.notify();
        }
    }

    /// Answers the program's clipboard read (OSC 52), with `text` once the
    /// user allowed it or `None` to refuse.
    pub fn answer_clipboard_read(&mut self, target: ClipboardTarget, text: Option<&str>) {
        self.shared
            .borrow_mut()
            .send(&clipboard_read_reply(target, text));
    }

    fn paste(&mut self, text: String, confirmed: bool, cx: &mut Context<Self>) {
        let shared = self.shared.clone();
        self.view.update(cx, |view, cx| {
            let mut shared = shared.borrow_mut();
            if shared.connected {
                paste_text(&mut shared, view.grid_mut(), text, confirmed);
            }
            cx.notify();
        });
    }

    fn take_events(&mut self, cx: &mut Context<Self>) {
        let (events, waiting) = {
            let mut shared = self.shared.borrow_mut();
            (
                std::mem::take(&mut shared.events),
                shared.pending_paste.is_some(),
            )
        };
        for event in events {
            cx.emit(event);
        }
        // Draw the pane again only when the paste prompt comes or goes; the
        // terminal redraws on its own, so streaming output leaves the rest
        // of the window as it was.
        if waiting != self.prompt_shown {
            cx.notify();
        }
    }

    /// Opens the find bar, or selects its text when it is open.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = &self.find {
            find.input.update(cx, |input, cx| {
                input.select_all(window, cx);
                input.focus(window, cx);
            });
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let input_events = cx.subscribe_in(
            &input,
            window,
            |pane: &mut Self, input, event, window, cx| {
                match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        pane.update_query(|query| query.text = text, cx);
                    }
                    InputEvent::PressEnter { shift, .. } => {
                        if *shift {
                            pane.find_newer(cx);
                        } else {
                            pane.find_older(cx);
                        }
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                }
                let _ = window;
            },
        );
        input.update(cx, |input, cx| input.focus(window, cx));
        self.find = Some(FindBar {
            input,
            query: Query::default(),
            marks: Vec::new(),
            current: None,
            error: None,
            _input_events: input_events,
        });
        cx.notify();
    }

    /// Closes the find bar, clears its highlights and gives the keyboard
    /// back to the terminal.
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            self.view
                .update(cx, |view, cx| view.set_marks(Vec::new(), cx));
        }
        let focus = self.view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Searches for `query`, as typing in the find bar and its toggles do.
    pub fn set_find_query(&mut self, query: Query, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.open_find(window, cx);
        }
        if let Some(find) = &self.find {
            let text = query.text.clone();
            find.input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.update_query(|current| *current = query, cx);
    }

    pub fn find_status(&self) -> Option<FindStatus> {
        self.find.as_ref().map(|find| FindStatus {
            query: find.query.clone(),
            matches: find.marks.len(),
            capped: find.marks.len() >= MATCH_LIMIT,
            current: find.current,
            error: find.error.clone(),
        })
    }

    /// Moves to the next match up, toward older output (Enter).
    pub fn find_older(&mut self, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    /// Moves to the next match down, toward newer output (Shift+Enter).
    pub fn find_newer(&mut self, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    fn step(&mut self, newer: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        let count = find.marks.len();
        if count == 0 {
            return;
        }
        find.current = Some(match find.current {
            None if newer => 0,
            None => count - 1,
            Some(at) if newer => (at + 1) % count,
            Some(at) => (at + count - 1) % count,
        });
        self.show_marks(true, cx);
    }

    fn update_query(&mut self, change: impl FnOnce(&mut Query), cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        let before = find.query.clone();
        change(&mut find.query);
        if find.query != before {
            find.current = None;
            self.refind(cx);
        }
    }

    /// Runs the query again over the whole history and keeps the bar on
    /// the same match when it is still there.
    fn refind(&mut self, cx: &mut Context<Self>) {
        let bottom = self.view_bottom(cx);
        let Some(find) = &mut self.find else {
            return;
        };
        let previous = find
            .current
            .and_then(|at| find.marks.get(at))
            .map(|mark| mark.start);
        let (marks, error) = match Finder::new(&find.query) {
            Ok(Some(finder)) => {
                let terminal = self.view.read(cx).grid();
                let total = terminal.total_rows() as u64;
                let marks = finder.search(total, |y| {
                    u32::try_from(y)
                        .ok()
                        .and_then(|y| terminal.screen_row(y).ok())
                });
                (marks, None)
            }
            Ok(None) => (Vec::new(), None),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        let first_search = find.current.is_none();
        find.current = match previous {
            Some(start) => marks.iter().position(|mark| mark.start >= start),
            None => None,
        };
        find.marks = marks;
        find.error = error;
        if find.current.is_none() && !find.marks.is_empty() {
            // A new query starts on the newest match in view or above it,
            // as a search back through the history does.
            find.current = Some(
                find.marks
                    .iter()
                    .rposition(|mark| mark.start.row <= bottom)
                    .unwrap_or(find.marks.len() - 1),
            );
        }
        self.show_marks(first_search, cx);
    }

    /// The last history row in view.
    fn view_bottom(&self, cx: &App) -> u64 {
        let terminal = self.view.read(cx).grid();
        match terminal.scrollbar() {
            Some(bar) => (bar.offset + bar.len).saturating_sub(1),
            None => (terminal.total_rows() as u64).saturating_sub(1),
        }
    }

    /// Highlights every match and, when `reveal`, scrolls the current one
    /// into view.
    fn show_marks(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let Some(find) = &self.find else {
            return;
        };
        let current = find.current;
        let marks: Vec<Mark> = find
            .marks
            .iter()
            .enumerate()
            .map(|(at, mark)| Mark {
                current: Some(at) == current,
                ..*mark
            })
            .collect();
        let target = current
            .and_then(|at| find.marks.get(at))
            .map(|mark| mark.start.row);
        self.view.update(cx, |view, cx| {
            if reveal
                && let Some(row) = target
                && let Some(bar) = view.grid().scrollbar()
                && !(bar.offset..bar.offset + bar.len).contains(&row)
            {
                let top = row.saturating_sub(bar.len / 2);
                let _ = view
                    .grid_mut()
                    .scroll_viewport(Scroll::Row(usize::try_from(top).unwrap_or(usize::MAX)));
                cx.notify();
            }
            view.set_marks(marks, cx);
        });
        cx.notify();
    }

    /// Puts the keyboard back in the find field after a click on one of
    /// the bar's buttons, so Enter and Escape keep working.
    fn refocus_find(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = &self.find {
            find.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn toggle_case(&mut self, cx: &mut Context<Self>) {
        self.update_query(|query| query.case_sensitive = !query.case_sensitive, cx);
    }

    fn toggle_regex(&mut self, cx: &mut Context<Self>) {
        self.update_query(|query| query.regex = !query.regex, cx);
    }

    /// The pane's own shortcuts, taken before the program sees the key.
    fn shortcut(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        let mac = cfg!(target_os = "macos");
        let (find, again) = if mac {
            (
                m.platform && !m.control && !m.alt && key == "f" && !m.shift,
                m.platform && !m.control && !m.alt && key == "g",
            )
        } else {
            (m.control && m.shift && !m.alt && key == "f", false)
        };
        // ⌘\ on a Mac; Ctrl+Shift+\ elsewhere, since Ctrl+\ is the
        // program's quit signal.
        let split = key == "\\"
            && !m.alt
            && if mac {
                m.platform && !m.control && !m.shift
            } else {
                m.control && m.shift && !m.platform
            };
        if split {
            cx.emit(PaneEvent::SplitRequested);
            cx.stop_propagation();
        } else if find {
            self.open_find(window, cx);
            cx.stop_propagation();
        } else if again && self.find.is_some() {
            if m.shift {
                self.find_newer(cx);
            } else {
                self.find_older(cx);
            }
            cx.stop_propagation();
        }
    }

    fn render_find_bar(&self, find: &FindBar, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let count: SharedString = if let Some(error) = &find.error {
            error.clone().into()
        } else if find.query.text.is_empty() {
            SharedString::default()
        } else if find.marks.is_empty() {
            "No matches".into()
        } else {
            let total = if find.marks.len() >= MATCH_LIMIT {
                format!("{}+", find.marks.len())
            } else {
                find.marks.len().to_string()
            };
            match find.current {
                Some(at) => format!("{} of {total}", at + 1).into(),
                None => format!("{total} matches").into(),
            }
        };
        let count_color = if find.error.is_some() {
            theme.danger
        } else {
            theme.muted_foreground
        };
        h_flex()
            .id("find-bar")
            .test_support()
            .absolute()
            .top(px(6.))
            .right(px(14.))
            .gap(px(4.))
            .p(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_md()
            .on_action(cx.listener(|pane, _: &Escape, window, cx| pane.close_find(window, cx)))
            .child(
                div()
                    .w(px(200.))
                    .child(Input::new(&find.input).id("find-input")),
            )
            .child(
                Button::new("find-case")
                    .label("Aa")
                    .compact()
                    .ghost()
                    .tooltip("Match case")
                    .toggled(find.query.case_sensitive)
                    .on_click(cx.listener(|pane, _, window, cx| {
                        pane.toggle_case(cx);
                        pane.refocus_find(window, cx);
                    })),
            )
            .child(
                Button::new("find-regex")
                    .label(".*")
                    .compact()
                    .ghost()
                    .tooltip("Regular expression")
                    .toggled(find.query.regex)
                    .on_click(cx.listener(|pane, _, window, cx| {
                        pane.toggle_regex(cx);
                        pane.refocus_find(window, cx);
                    })),
            )
            .child(
                div()
                    .id("find-count")
                    .test_support()
                    .min_w(px(72.))
                    .text_xs()
                    .text_color(count_color)
                    .child(count),
            )
            .child(
                Button::new("find-older")
                    .label("↑")
                    .compact()
                    .ghost()
                    .tooltip("Older match (Enter)")
                    .on_click(cx.listener(|pane, _, window, cx| {
                        pane.find_older(cx);
                        pane.refocus_find(window, cx);
                    })),
            )
            .child(
                Button::new("find-newer")
                    .label("↓")
                    .compact()
                    .ghost()
                    .tooltip("Newer match (Shift+Enter)")
                    .on_click(cx.listener(|pane, _, window, cx| {
                        pane.find_newer(cx);
                        pane.refocus_find(window, cx);
                    })),
            )
            .child(
                Button::new("find-close")
                    .label("✕")
                    .compact()
                    .ghost()
                    .tooltip("Close (Escape)")
                    .on_click(cx.listener(|pane, _, window, cx| pane.close_find(window, cx))),
            )
            .into_any_element()
    }

    fn render_paste_prompt(&self, pending: &PendingPaste, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let lines = pending.line_breaks + 1;
        h_flex()
            .id("paste-prompt")
            .test_support()
            .absolute()
            .bottom(px(8.))
            .left(px(8.))
            .right(px(8.))
            .gap(px(8.))
            .p(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.warning)
            .bg(theme.popover)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(format!(
                        "Paste {lines} lines? This program didn't ask for bracketed paste, so each \
                     line break runs the line before it as a command."
                    )),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(6.))
                    .child(
                        Button::new("paste-cancel")
                            .label("Cancel")
                            .compact()
                            .on_click(cx.listener(|pane, _, _, cx| pane.cancel_paste(cx))),
                    )
                    .child(
                        Button::new("paste-confirm")
                            .label("Paste")
                            .compact()
                            .primary()
                            .on_click(cx.listener(|pane, _, _, cx| pane.confirm_paste(cx))),
                    ),
            )
            .into_any_element()
    }
}

/// Rows through the end of the output the program wrote from the top: its
/// first block of text, which ends at a gap of `GAP` blank rows, or through
/// the cursor row when that is lower and inside the block. A footer the
/// program pins to the bottom (an agent's input box) is not output, so a
/// notice placed under the output doesn't cover it.
fn rows_used(blank_free: impl Iterator<Item = bool>, cursor: Option<usize>) -> usize {
    const GAP: usize = 3;
    let mut end = 0;
    let mut blank = 0;
    for (y, used) in blank_free.enumerate() {
        if used {
            end = y + 1;
            blank = 0;
        } else {
            blank += 1;
            if end > 0 && blank >= GAP {
                break;
            }
        }
    }
    match cursor {
        Some(y) if y + 1 > end && y < end + GAP => y + 1,
        _ => end,
    }
}

#[cfg(test)]
mod tests {
    use super::rows_used;

    #[test]
    fn blank_screen_uses_no_rows() {
        assert_eq!(rows_used([false; 5].into_iter(), None), 0);
    }

    #[test]
    fn rows_run_through_the_last_one_with_text() {
        assert_eq!(
            rows_used([true, true, true, false, false].into_iter(), None),
            3
        );
    }

    #[test]
    fn a_visible_cursor_below_the_text_counts() {
        assert_eq!(
            rows_used([true, false, false, false].into_iter(), Some(2)),
            3
        );
    }

    #[test]
    fn a_footer_pinned_to_the_bottom_is_not_output() {
        let rows = [true, false, true, false, false, false, false, true, true];
        assert_eq!(rows_used(rows.into_iter(), Some(7)), 3);
    }
}

impl Focusable for TerminalPane {
    /// The terminal's own focus, so focusing the pane types into it.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle(cx)
    }
}

impl Render for TerminalPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.hidden.is_some() {
            return div().id("terminal-pane").size_full().into_any_element();
        }
        let shared = self.shared.borrow();
        let connected = shared.connected;
        let pending = shared.pending_paste.clone();
        drop(shared);
        self.prompt_shown = pending.is_some();
        let find_bar = self
            .find
            .as_ref()
            .map(|find| self.render_find_bar(find, cx));
        let paste_prompt = pending
            .as_ref()
            .map(|pending| self.render_paste_prompt(pending, cx));
        let theme = cx.theme();
        let banner = (!connected && self.offline_banner).then(|| {
            div()
                .id("disconnected-banner")
                .test_support()
                .mx(px(14.))
                .mb(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(7.))
                .bg(theme.muted)
                .text_size(px(12.))
                .text_color(theme.foreground)
                .child(
                    Icon::new(IconName::WifiOff)
                        .size(px(12.))
                        .text_color(theme.muted_foreground),
                )
                .child(
                    "Lost the connection to this program's machine. The program keeps running \
                     there; this view catches up when it is back, and typing is off until then.",
                )
        });
        div()
            .id("terminal-pane")
            .test_support()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .capture_key_down(cx.listener(Self::shortcut))
            .child(div().flex_1().min_h_0().child(self.view.clone()))
            .children(find_bar)
            .children(paste_prompt)
            .children(banner)
            .into_any_element()
    }
}
