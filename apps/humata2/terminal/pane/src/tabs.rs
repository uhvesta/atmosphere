//! The tab strip over a session's terminal area (SE-8, SE-9): one tab per
//! terminal, file or review view, reordered by dragging, renamed by
//! double-clicking and switched from the keyboard.
//!
//! Closing a tab closes its view only. What runs in it keeps running in
//! `humatad` (PER-1); stopping it is a separate request, so the app can
//! ask before ending a process.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    AnyView, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent,
    SharedString, Subscription, Window, div, px,
};

/// A tab, for as long as the strip lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

/// What a tab shows.
pub struct TabSpec {
    pub title: SharedString,
    /// Whether the tab has a ✕. The agent's terminal and the session's
    /// shell have none; file and review tabs do (SE-8).
    pub closable: bool,
    pub content: AnyView,
    /// Focused when the tab is activated, such as a terminal pane's.
    pub focus: Option<FocusHandle>,
}

/// What the user did with the tabs. The strip has already done it to
/// itself; the app does the rest (open a shell, keep the order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabsEvent {
    /// The user asked for another shell tab (⌘T, or the + button).
    NewTab,
    Activated(TabId),
    /// The tab's view was closed. Its process, if any, keeps running.
    Closed(TabId),
    /// The user asked to stop the tab's process. The tab stays until the
    /// app closes it.
    StopRequested(TabId),
    Renamed { id: TabId, title: String },
    /// The tab now sits at `to`, counting from 0 on the left.
    Moved { id: TabId, to: usize },
}

/// The tabs and which one is active, without any drawing.
#[derive(Debug, Clone)]
pub struct TabStrip<T> {
    tabs: Vec<(TabId, T)>,
    active: Option<TabId>,
}

impl<T> Default for TabStrip<T> {
    fn default() -> Self {
        TabStrip {
            tabs: Vec::new(),
            active: None,
        }
    }
}

impl<T> TabStrip<T> {
    /// Adds a tab at the right end and makes it active.
    pub fn push(&mut self, id: TabId, tab: T) {
        self.tabs.push((id, tab));
        self.active = Some(id);
    }

    pub fn ids(&self) -> Vec<TabId> {
        self.tabs.iter().map(|(id, _)| *id).collect()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    pub fn get(&self, id: TabId) -> Option<&T> {
        self.tabs.iter().find(|(tab, _)| *tab == id).map(|(_, tab)| tab)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut T> {
        self.tabs.iter_mut().find(|(tab, _)| *tab == id).map(|(_, tab)| tab)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|(tab, _)| *tab == id)
    }

    /// Makes `id` active; `false` when there is no such tab.
    pub fn activate(&mut self, id: TabId) -> bool {
        let found = self.index_of(id).is_some();
        if found {
            self.active = Some(id);
        }
        found
    }

    /// The tab at `index`, or the last one when `index` is past the end.
    pub fn nth_or_last(&self, index: usize) -> Option<TabId> {
        self.tabs
            .get(index)
            .or_else(|| self.tabs.last())
            .map(|(id, _)| *id)
    }

    /// The tab `steps` to the right of the active one (left when
    /// negative), going round at either end.
    pub fn neighbour(&self, steps: isize) -> Option<TabId> {
        let len = self.tabs.len() as isize;
        let at = self.active.and_then(|id| self.index_of(id))? as isize;
        let to = (at + steps).rem_euclid(len.max(1));
        self.tabs.get(to as usize).map(|(id, _)| *id)
    }

    /// Removes a tab. When it was active, the tab to its right becomes
    /// active, or the one to its left when it was the last.
    pub fn remove(&mut self, id: TabId) -> Option<T> {
        let at = self.index_of(id)?;
        let (_, tab) = self.tabs.remove(at);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(at)
                .or_else(|| at.checked_sub(1).and_then(|left| self.tabs.get(left)))
                .map(|(id, _)| *id);
        }
        Some(tab)
    }

    /// Moves a tab to `to` (clamped to the strip). Returns where it ended
    /// up, or `None` when it didn't move.
    pub fn move_to(&mut self, id: TabId, to: usize) -> Option<usize> {
        let from = self.index_of(id)?;
        let to = to.min(self.tabs.len() - 1);
        if from == to {
            return None;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        Some(to)
    }
}

struct Tab {
    title: SharedString,
    closable: bool,
    content: AnyView,
    focus: Option<FocusHandle>,
}

struct Renaming {
    id: TabId,
    input: Entity<InputState>,
    _events: Subscription,
}

/// What a dragged tab carries, and its picture under the pointer.
#[derive(Clone)]
struct DraggedTab {
    id: TabId,
    title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded(px(4.))
            .bg(theme.tab_active)
            .text_color(theme.tab_active_foreground)
            .text_sm()
            .opacity(0.85)
            .child(self.title.clone())
    }
}

/// The tab strip and the active tab's content below it.
pub struct TerminalTabs {
    strip: TabStrip<Tab>,
    next_id: u64,
    renaming: Option<Renaming>,
    disconnected: bool,
}

impl EventEmitter<TabsEvent> for TerminalTabs {}

impl TerminalTabs {
    pub fn new() -> Self {
        TerminalTabs {
            strip: TabStrip::default(),
            next_id: 0,
            renaming: None,
            disconnected: false,
        }
    }

    /// Adds a tab at the right end, activates it and focuses it.
    pub fn add_tab(&mut self, spec: TabSpec, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        self.strip.push(id, Tab {
            title: spec.title,
            closable: spec.closable,
            content: spec.content,
            focus: spec.focus,
        });
        self.focus_active(window, cx);
        cx.notify();
        id
    }

    pub fn tabs(&self) -> Vec<TabId> {
        self.strip.ids()
    }

    pub fn active(&self) -> Option<TabId> {
        self.strip.active()
    }

    pub fn title(&self, id: TabId) -> Option<SharedString> {
        self.strip.get(id).map(|tab| tab.title.clone())
    }

    /// Sets a tab's title, such as from the program's own (OSC 2).
    pub fn set_title(&mut self, id: TabId, title: impl Into<SharedString>, cx: &mut Context<Self>) {
        if let Some(tab) = self.strip.get_mut(id) {
            tab.title = title.into();
            cx.notify();
        }
    }

    /// Whether the session's machine is offline: the strip says
    /// "disconnected" (SE-16).
    pub fn set_disconnected(&mut self, disconnected: bool, cx: &mut Context<Self>) {
        if self.disconnected != disconnected {
            self.disconnected = disconnected;
            cx.notify();
        }
    }

    pub fn activate(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.strip.active() != Some(id) && self.strip.activate(id) {
            self.focus_active(window, cx);
            cx.emit(TabsEvent::Activated(id));
            cx.notify();
        }
    }

    /// Closes a tab's view, if it has a ✕. Its process keeps running.
    pub fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.strip.get(id).is_some_and(|tab| tab.closable) {
            return;
        }
        let was_active = self.strip.active() == Some(id);
        self.strip.remove(id);
        if self.renaming.as_ref().is_some_and(|renaming| renaming.id == id) {
            self.renaming = None;
        }
        cx.emit(TabsEvent::Closed(id));
        if was_active && let Some(now) = self.strip.active() {
            self.focus_active(window, cx);
            cx.emit(TabsEvent::Activated(now));
        }
        cx.notify();
    }

    /// Asks the app to stop the tab's process.
    pub fn request_stop(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.strip.get(id).is_some() {
            cx.emit(TabsEvent::StopRequested(id));
        }
    }

    pub fn move_tab(&mut self, id: TabId, to: usize, cx: &mut Context<Self>) {
        if let Some(to) = self.strip.move_to(id, to) {
            cx.emit(TabsEvent::Moved { id, to });
            cx.notify();
        }
    }

    /// Turns the tab's title into a field; Enter keeps the new name and
    /// Escape the old one.
    pub fn start_rename(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.strip.get(id) else {
            return;
        };
        let title = tab.title.clone();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(title.to_string(), window, cx);
            input.select_all(window, cx);
            input
        });
        let events = cx.subscribe_in(&input, window, move |tabs: &mut Self, input, event, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    let title = input.read(cx).value().trim().to_owned();
                    tabs.finish_rename(id, Some(title), window, cx);
                }
                InputEvent::Blur => {
                    let title = input.read(cx).value().trim().to_owned();
                    tabs.finish_rename(id, Some(title), window, cx);
                }
                InputEvent::Change | InputEvent::Focus => {}
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.renaming = Some(Renaming {
            id,
            input,
            _events: events,
        });
        cx.notify();
    }

    pub fn is_renaming(&self) -> Option<TabId> {
        self.renaming.as_ref().map(|renaming| renaming.id)
    }

    fn finish_rename(
        &mut self,
        id: TabId,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.renaming.as_ref().is_none_or(|renaming| renaming.id != id) {
            return;
        }
        self.renaming = None;
        if let Some(title) = title.filter(|title| !title.is_empty())
            && let Some(tab) = self.strip.get_mut(id)
            && tab.title.as_ref() != title
        {
            tab.title = title.clone().into();
            cx.emit(TabsEvent::Renamed { id, title });
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(focus) = self
            .strip
            .active()
            .and_then(|id| self.strip.get(id))
            .and_then(|tab| tab.focus.as_ref())
        {
            window.focus(focus, cx);
        }
    }

    /// The tab shortcuts, taken before the tab's content sees the key.
    fn shortcut(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = command(&event.keystroke) else {
            return;
        };
        if self.renaming.is_some() {
            return;
        }
        match command {
            Command::New => cx.emit(TabsEvent::NewTab),
            Command::Close => match self.strip.active() {
                Some(id) if self.strip.get(id).is_some_and(|tab| tab.closable) => {
                    self.close_tab(id, window, cx);
                }
                // Not the strip's to close: leave it to the app.
                _ => return,
            },
            Command::Step(steps) => {
                if let Some(id) = self.strip.neighbour(steps) {
                    self.activate(id, window, cx);
                }
            }
            Command::Nth(index) => {
                if let Some(id) = self.strip.nth_or_last(index) {
                    self.activate(id, window, cx);
                }
            }
            Command::Last => {
                if let Some(id) = self.strip.nth_or_last(usize::MAX) {
                    self.activate(id, window, cx);
                }
            }
        }
        cx.stop_propagation();
    }

    fn render_tab(&self, index: usize, id: TabId, tab: &Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active = self.strip.active() == Some(id);
        let (bg, fg) = if active {
            (theme.tab_active, theme.tab_active_foreground)
        } else {
            (theme.tab, theme.tab_foreground)
        };
        let renaming = self.renaming.as_ref().filter(|renaming| renaming.id == id);
        let label = match renaming {
            Some(renaming) => div()
                .w(px(140.))
                .on_action(cx.listener(move |tabs, _: &Escape, window, cx| {
                    tabs.finish_rename(id, None, window, cx);
                }))
                .child(Input::new(&renaming.input).id(format!("tab-rename-{}", id.0)))
                .into_any_element(),
            None => div().child(tab.title.clone()).into_any_element(),
        };
        let dragged = DraggedTab {
            id,
            title: tab.title.clone(),
        };
        h_flex()
            .id(SharedString::from(format!("tab-{}", id.0)))
            .test_support()
            .gap(px(6.))
            .px(px(10.))
            .py(px(4.))
            .text_sm()
            .bg(bg)
            .text_color(fg)
            .border_r_1()
            .border_color(theme.border)
            .on_click(cx.listener(move |tabs, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    tabs.start_rename(id, window, cx);
                } else {
                    tabs.activate(id, window, cx);
                }
            }))
            .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            .drag_over::<DraggedTab>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(cx.listener(move |tabs, dragged: &DraggedTab, _, cx| {
                tabs.move_tab(dragged.id, index, cx);
            }))
            .child(label)
            .when(tab.closable, |tab| {
                tab.child(
                    Button::new(SharedString::from(format!("tab-close-{}", id.0)))
                        .label("✕")
                        .compact()
                        .ghost()
                        .tooltip("Close the tab (what runs in it keeps running)")
                        .on_click(cx.listener(move |tabs, _, window, cx| {
                            cx.stop_propagation();
                            tabs.close_tab(id, window, cx);
                        })),
                )
            })
    }
}

impl Default for TerminalTabs {
    fn default() -> Self {
        TerminalTabs::new()
    }
}

impl Render for TerminalTabs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<_> = self
            .strip
            .ids()
            .into_iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let tab = self.strip.get(id)?;
                Some(self.render_tab(index, id, tab, cx).into_any_element())
            })
            .collect();
        let content = self
            .strip
            .active()
            .and_then(|id| self.strip.get(id))
            .map(|tab| tab.content.clone());
        let theme = cx.theme();
        let disconnected = self.disconnected.then(|| {
            div()
                .id("tabs-disconnected")
                .test_support()
                .px(px(8.))
                .text_xs()
                .text_color(theme.warning)
                .child("disconnected")
        });
        v_flex()
            .id("terminal-tabs")
            .test_support()
            .size_full()
            .capture_key_down(cx.listener(Self::shortcut))
            .child(
                h_flex()
                    .id("tab-bar")
                    .test_support()
                    .w_full()
                    .bg(theme.tab_bar)
                    .border_b_1()
                    .border_color(theme.border)
                    .children(tabs)
                    .child(
                        Button::new("tab-new")
                            .label("+")
                            .compact()
                            .ghost()
                            .tooltip("New shell tab")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(TabsEvent::NewTab))),
                    )
                    .child(div().flex_1())
                    .children(disconnected),
            )
            .child(div().id("tab-content").flex_1().min_h(px(0.)).children(content))
    }
}

/// A tab shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    New,
    Close,
    Step(isize),
    /// The tab at this index, or the last when there are fewer.
    Nth(usize),
    Last,
}

/// The tab shortcut a key is, if any. On a Mac: ⌘T, ⌘W, ⌘⇧] and ⌘⇧[,
/// ⌃Tab and ⌃⇧Tab, ⌘1 to ⌘8 and ⌘9 for the last tab, as in Ghostty and
/// browsers. Elsewhere Ctrl+Shift+T and W, Ctrl+Tab, Ctrl+Page Up and Down
/// and Alt+1 to 9, which leave the program's own Ctrl keys alone.
fn command(keystroke: &gpui_kit::Keystroke) -> Option<Command> {
    let m = keystroke.modifiers;
    let key = keystroke.key.as_str();
    if m.control && !m.alt && !m.platform && key == "tab" {
        return Some(Command::Step(if m.shift { -1 } else { 1 }));
    }
    let digit = key
        .parse::<usize>()
        .ok()
        .filter(|digit| (1..=9).contains(digit));
    if cfg!(target_os = "macos") {
        if !m.platform || m.control || m.alt {
            return None;
        }
        return match (key, m.shift, digit) {
            ("t", false, _) => Some(Command::New),
            ("w", false, _) => Some(Command::Close),
            ("]" | "}", true, _) => Some(Command::Step(1)),
            ("[" | "{", true, _) => Some(Command::Step(-1)),
            (_, false, Some(9)) => Some(Command::Last),
            (_, false, Some(digit)) => Some(Command::Nth(digit - 1)),
            _ => None,
        };
    }
    if m.control && m.shift && !m.alt && !m.platform {
        return match key {
            "t" => Some(Command::New),
            "w" => Some(Command::Close),
            _ => None,
        };
    }
    if m.control && !m.shift && !m.alt && !m.platform {
        return match key {
            "pagedown" => Some(Command::Step(1)),
            "pageup" => Some(Command::Step(-1)),
            _ => None,
        };
    }
    if m.alt && !m.control && !m.shift && !m.platform {
        return match digit {
            Some(9) => Some(Command::Last),
            Some(digit) => Some(Command::Nth(digit - 1)),
            None => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{TabId, TabStrip};

    fn strip(n: u64) -> TabStrip<()> {
        let mut strip = TabStrip::default();
        for id in 0..n {
            strip.push(TabId(id), ());
        }
        strip
    }

    #[test]
    fn a_new_tab_goes_last_and_becomes_active() {
        let strip = strip(3);
        assert_eq!(strip.ids(), vec![TabId(0), TabId(1), TabId(2)]);
        assert_eq!(strip.active(), Some(TabId(2)));
    }

    #[test]
    fn closing_the_active_tab_activates_its_right_neighbour_or_else_its_left() {
        let mut strip = strip(3);
        strip.activate(TabId(1));
        strip.remove(TabId(1));
        assert_eq!(strip.active(), Some(TabId(2)));
        strip.remove(TabId(2));
        assert_eq!(strip.active(), Some(TabId(0)));
        strip.remove(TabId(0));
        assert_eq!(strip.active(), None);
    }

    #[test]
    fn closing_another_tab_keeps_the_active_one() {
        let mut strip = strip(3);
        strip.activate(TabId(0));
        strip.remove(TabId(2));
        assert_eq!(strip.active(), Some(TabId(0)));
    }

    #[test]
    fn stepping_goes_round_at_either_end() {
        let mut strip = strip(3);
        assert_eq!(strip.neighbour(1), Some(TabId(0)));
        strip.activate(TabId(0));
        assert_eq!(strip.neighbour(-1), Some(TabId(2)));
        assert_eq!(strip.neighbour(4), Some(TabId(1)));
    }

    #[test]
    fn a_number_past_the_end_picks_the_last_tab() {
        let strip = strip(2);
        assert_eq!(strip.nth_or_last(0), Some(TabId(0)));
        assert_eq!(strip.nth_or_last(7), Some(TabId(1)));
        assert_eq!(TabStrip::<()>::default().nth_or_last(0), None);
    }

    #[test]
    fn moving_a_tab_shifts_the_others_and_is_clamped() {
        let mut strip = strip(4);
        assert_eq!(strip.move_to(TabId(0), 2), Some(2));
        assert_eq!(strip.ids(), vec![TabId(1), TabId(2), TabId(0), TabId(3)]);
        assert_eq!(strip.move_to(TabId(1), 99), Some(3));
        assert_eq!(strip.ids(), vec![TabId(2), TabId(0), TabId(3), TabId(1)]);
        assert_eq!(strip.move_to(TabId(1), 3), None);
        // The active tab stays active wherever it goes.
        assert_eq!(strip.active(), Some(TabId(3)));
    }
}
