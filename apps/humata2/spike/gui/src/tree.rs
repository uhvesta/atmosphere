//! Screen 1: the sidebar tree. 200 channels of 24 sessions each, 5,000 rows
//! when all are open, drawn with a virtualised `uniform_list`. Which
//! channels are open and which row is selected live in a tca-rust reducer;
//! the view only sends it actions.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use tca::{Effect, Reducer};

use crate::store_bridge::{StoreBridge, StoreBridgeError};

/// How many channels the generated outline has.
pub const CHANNELS: usize = 200;
/// How many sessions each generated channel has.
pub const SESSIONS_PER_CHANNEL: usize = 24;

const ROW_HEIGHT: f32 = 24.;
const INDENT: f32 = 16.;

const TOPICS: [&str; 10] = [
    "build", "deploy", "review", "infra", "design", "billing", "search", "mobile", "docs", "ops",
];
const VERBS: [&str; 8] = [
    "fix", "plan", "refactor", "triage", "ship", "profile", "migrate", "audit",
];
const OBJECTS: [&str; 9] = [
    "cache", "login", "sync", "parser", "queue", "sidebar", "renderer", "index", "daemon",
];

/// One row of the tree, by what it shows rather than where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TreeRow {
    Channel(usize),
    Session { channel: usize, session: usize },
}

impl TreeRow {
    /// A number unique to the row, for its element id.
    pub fn key(self) -> usize {
        match self {
            TreeRow::Channel(channel) => channel << 16,
            TreeRow::Session { channel, session } => (channel << 16) | (session + 1),
        }
    }

    fn channel(self) -> usize {
        match self {
            TreeRow::Channel(channel) | TreeRow::Session { channel, .. } => channel,
        }
    }
}

/// The channels and sessions, generated the same way every run.
pub struct TreeOutline {
    channels: Vec<Channel>,
}

struct Channel {
    label: SharedString,
    sessions: Vec<SharedString>,
}

impl TreeOutline {
    /// The outline of [`CHANNELS`] channels of [`SESSIONS_PER_CHANNEL`] sessions.
    pub fn generate() -> Self {
        let channels = (0..CHANNELS)
            .map(|channel| Channel {
                label: format!("#{}-{channel:03}", TOPICS[channel % TOPICS.len()]).into(),
                sessions: (0..SESSIONS_PER_CHANNEL)
                    .map(|session| {
                        let seed = channel * 31 + session * 7;
                        format!(
                            "{} {} {channel:03}.{session:02}",
                            VERBS[seed % VERBS.len()],
                            OBJECTS[(seed / 3) % OBJECTS.len()],
                        )
                        .into()
                    })
                    .collect(),
            })
            .collect();
        Self { channels }
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Every row when every channel is open.
    pub fn row_count(&self) -> usize {
        self.channels
            .iter()
            .map(|channel| 1 + channel.sessions.len())
            .sum()
    }

    /// The row's label, or None for a row the outline doesn't have.
    pub fn label(&self, row: TreeRow) -> Option<&SharedString> {
        match row {
            TreeRow::Channel(channel) => self.channels.get(channel).map(|c| &c.label),
            TreeRow::Session { channel, session } => self
                .channels
                .get(channel)
                .and_then(|c| c.sessions.get(session)),
        }
    }

    fn sessions(&self, channel: usize) -> usize {
        self.channels.get(channel).map_or(0, |c| c.sessions.len())
    }
}

/// What the tree shows: which channels are open and which row is selected.
#[derive(Clone)]
pub struct TreeState {
    outline: Arc<TreeOutline>,
    expanded: BTreeSet<usize>,
    selected: Option<TreeRow>,
    /// The rows in view order, kept in step with `expanded`.
    visible: Arc<Vec<TreeRow>>,
}

impl TreeState {
    /// Every channel closed and nothing selected.
    pub fn new(outline: Arc<TreeOutline>) -> Self {
        let mut state = Self {
            outline,
            expanded: BTreeSet::new(),
            selected: None,
            visible: Arc::default(),
        };
        state.relayout();
        state
    }

    pub fn outline(&self) -> &TreeOutline {
        &self.outline
    }

    pub fn expanded(&self) -> &BTreeSet<usize> {
        &self.expanded
    }

    pub fn selected(&self) -> Option<TreeRow> {
        self.selected
    }

    /// The rows the list shows, top to bottom.
    pub fn visible_rows(&self) -> &[TreeRow] {
        &self.visible
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.visible.iter().position(|row| *row == selected)
    }

    fn relayout(&mut self) {
        let mut rows = Vec::with_capacity(self.outline.row_count());
        for channel in 0..self.outline.channel_count() {
            rows.push(TreeRow::Channel(channel));
            if self.expanded.contains(&channel) {
                rows.extend(
                    (0..self.outline.sessions(channel))
                        .map(|session| TreeRow::Session { channel, session }),
                );
            }
        }
        self.visible = Arc::new(rows);
    }

    /// Closing a channel moves a selection inside it onto the channel.
    fn collapse(&mut self, channel: usize) {
        if self.expanded.remove(&channel) {
            if let Some(TreeRow::Session { channel: inside, .. }) = self.selected
                && inside == channel
            {
                self.selected = Some(TreeRow::Channel(channel));
            }
            self.relayout();
        }
    }
}

/// What the view asks of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeAction {
    SelectNext,
    SelectPrevious,
    /// Opens a closed channel, or moves into an open one.
    ExpandSelected,
    /// Closes an open channel, or moves from a session to its channel.
    CollapseSelected,
    Select(TreeRow),
    Toggle(usize),
    ExpandAll,
    CollapseAll,
}

/// Changes a [`TreeState`] for a [`TreeAction`]. It has no effects.
pub struct TreeReducer;

impl Reducer for TreeReducer {
    type State = TreeState;
    type Action = TreeAction;

    fn reduce(&self, state: &mut TreeState, action: TreeAction) -> Effect<TreeAction> {
        match action {
            TreeAction::SelectNext => {
                let next = match state.selected_index() {
                    Some(index) => (index + 1).min(state.visible.len().saturating_sub(1)),
                    None => 0,
                };
                if let Some(row) = state.visible.get(next) {
                    state.selected = Some(*row);
                }
            }
            TreeAction::SelectPrevious => {
                let previous = match state.selected_index() {
                    Some(index) => index.saturating_sub(1),
                    None => 0,
                };
                if let Some(row) = state.visible.get(previous) {
                    state.selected = Some(*row);
                }
            }
            TreeAction::ExpandSelected => match state.selected {
                Some(TreeRow::Channel(channel)) if state.expanded.contains(&channel) => {
                    if state.outline.sessions(channel) > 0 {
                        state.selected = Some(TreeRow::Session {
                            channel,
                            session: 0,
                        });
                    }
                }
                Some(TreeRow::Channel(channel)) => {
                    state.expanded.insert(channel);
                    state.relayout();
                }
                Some(TreeRow::Session { .. }) | None => {}
            },
            TreeAction::CollapseSelected => match state.selected {
                Some(TreeRow::Channel(channel)) => state.collapse(channel),
                Some(TreeRow::Session { channel, .. }) => {
                    state.selected = Some(TreeRow::Channel(channel));
                }
                None => {}
            },
            TreeAction::Select(row) => {
                if state.outline.label(row).is_some() {
                    state.selected = Some(row);
                }
            }
            TreeAction::Toggle(channel) => {
                if state.expanded.contains(&channel) {
                    state.collapse(channel);
                } else if channel < state.outline.channel_count() {
                    state.expanded.insert(channel);
                    state.relayout();
                }
            }
            TreeAction::ExpandAll => {
                state.expanded = (0..state.outline.channel_count()).collect();
                state.relayout();
            }
            TreeAction::CollapseAll => {
                if let Some(row) = state.selected {
                    state.selected = Some(TreeRow::Channel(row.channel()));
                }
                state.expanded.clear();
                state.relayout();
            }
        }
        Effect::none()
    }
}

/// The tree's store.
pub struct TreeStore(StoreBridge<TreeReducer>);

impl TreeStore {
    /// A store over a freshly generated outline, every channel closed.
    ///
    /// # Errors
    ///
    /// When the store's effects runtime can't start.
    pub fn new() -> Result<Self, StoreBridgeError> {
        let state = TreeState::new(Arc::new(TreeOutline::generate()));
        StoreBridge::new(state, TreeReducer).map(Self)
    }
}

/// The sidebar tree screen.
pub struct TreeScreen {
    store: TreeStore,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl TreeScreen {
    /// The screen, focused so the arrow keys reach it.
    pub fn new(store: TreeStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            store,
            focus,
            scroll: UniformListScrollHandle::new(),
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// What `read` makes of the tree's state as it is now.
    pub fn with_state<T>(&self, read: impl FnOnce(&TreeState) -> T) -> T {
        self.store.0.with_state(read)
    }

    /// Opens every channel, as the Expand all button does.
    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.send(TreeAction::ExpandAll, cx);
    }

    /// Scrolls `step` further down, for the scroll benchmark. True once the
    /// list is at its end, or can't scroll at all.
    pub fn bench_step(&mut self, step: Pixels) -> bool {
        let handle = self.scroll.0.borrow().base_handle.clone();
        let max = handle.max_offset().y.as_f32();
        if max <= 0. {
            return true;
        }
        let next = (-handle.offset().y.as_f32() + step.as_f32()).min(max);
        handle.set_offset(point(px(0.), px(-next)));
        next >= max
    }

    fn send(&mut self, action: TreeAction, cx: &mut Context<Self>) {
        self.store.0.send(action, cx);
    }

    /// Sends a keyboard action and keeps the selection in view.
    fn send_from_keyboard(&mut self, action: TreeAction, cx: &mut Context<Self>) {
        self.send(action, cx);
        if let Some(index) = self.with_state(TreeState::selected_index) {
            self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let action = match event.keystroke.key.as_str() {
            "down" => TreeAction::SelectNext,
            "up" => TreeAction::SelectPrevious,
            "right" => TreeAction::ExpandSelected,
            "left" => TreeAction::CollapseSelected,
            _ => return,
        };
        self.send_from_keyboard(action, cx);
        cx.stop_propagation();
    }

    fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.with_state(|state| {
            range
                .filter_map(|index| {
                    let row = *state.visible.get(index)?;
                    let label = state.outline.label(row)?.clone();
                    Some(render_row(row, label, state, cx))
                })
                .collect()
        })
    }

    fn header_button(
        id: &'static str,
        label: &'static str,
        action: TreeAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(label)
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x2d3340))
            .hover(|style| style.bg(rgb(0x3a4252)))
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.send_from_keyboard(action, cx);
                let focus = this.focus.clone();
                window.focus(&focus, cx);
            }))
    }
}

fn render_row(
    row: TreeRow,
    label: SharedString,
    state: &TreeState,
    cx: &mut Context<TreeScreen>,
) -> AnyElement {
    let selected = state.selected == Some(row);
    let (depth, chevron, expanded) = match row {
        TreeRow::Channel(channel) => {
            let open = state.expanded.contains(&channel);
            (0., if open { "▾" } else { "▸" }, Some(open))
        }
        TreeRow::Session { .. } => (1., "", None),
    };
    let element = div()
        .id(("tree-row", row.key()))
        .test_support()
        .role(Role::TreeItem)
        .aria_label(label.clone())
        .aria_selected(selected);
    let element = match expanded {
        Some(open) => element.aria_expanded(open),
        None => element,
    };
    element
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .pl(px(8. + depth * INDENT))
        .text_color(if expanded.is_some() {
            rgb(0xe6e9ef)
        } else {
            rgb(0xb8bfcc)
        })
        .when(selected, |row| row.bg(rgb(0x2f4a72)))
        .when(!selected, |row| row.hover(|style| style.bg(rgb(0x252a33))))
        .child(div().w(px(12.)).flex_shrink_0().child(chevron))
        .child(div().flex_1().overflow_hidden().whitespace_nowrap().child(label))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.send(TreeAction::Select(row), cx);
            if let TreeRow::Channel(channel) = row {
                this.send(TreeAction::Toggle(channel), cx);
            }
            let focus = this.focus.clone();
            window.focus(&focus, cx);
        }))
        .into_any_element()
}

impl Render for TreeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (count, open, total) = self.with_state(|state| {
            (
                state.visible.len(),
                state.expanded.len(),
                state.outline.row_count(),
            )
        });
        div()
            .id("sidebar-tree-screen")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1f27))
            .text_color(rgb(0xe6e9ef))
            .text_size(px(13.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(0x2d3340))
                    .child(Self::header_button(
                        "expand-all",
                        "Expand all",
                        TreeAction::ExpandAll,
                        cx,
                    ))
                    .child(Self::header_button(
                        "collapse-all",
                        "Collapse all",
                        TreeAction::CollapseAll,
                        cx,
                    ))
                    .child(
                        div()
                            .text_color(rgb(0x8a93a3))
                            .child(format!("{count} of {total} rows · {open} channels open")),
                    ),
            )
            .child(
                div()
                    .id("sidebar-tree")
                    .test_support()
                    .track_focus(&self.focus)
                    .key_context("SidebarTree")
                    .flex_1()
                    .min_h(px(0.))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                        this.key_down(event, cx);
                    }))
                    .child(
                        uniform_list(
                            "sidebar-tree-rows",
                            count,
                            cx.processor(|this, range: Range<usize>, _window, cx| {
                                this.render_rows(range, cx)
                            }),
                        )
                        .size_full()
                        .track_scroll(&self.scroll),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> TreeState {
        TreeState::new(Arc::new(TreeOutline::generate()))
    }

    fn reduce(state: &mut TreeState, actions: &[TreeAction]) {
        for action in actions {
            let _ = TreeReducer.reduce(state, *action);
        }
    }

    #[::std::prelude::v1::test]
    fn the_outline_has_five_thousand_rows_with_every_channel_open() {
        let mut state = state();
        assert_eq!(state.visible_rows().len(), CHANNELS);
        reduce(&mut state, &[TreeAction::ExpandAll]);
        assert_eq!(state.visible_rows().len(), 5_000);
        assert_eq!(state.outline().row_count(), 5_000);
    }

    #[::std::prelude::v1::test]
    fn selection_moves_through_open_channels_and_stops_at_the_ends() {
        let mut state = state();
        reduce(&mut state, &[TreeAction::SelectPrevious]);
        assert_eq!(state.selected(), Some(TreeRow::Channel(0)));
        reduce(&mut state, &[TreeAction::ExpandSelected, TreeAction::SelectNext]);
        assert_eq!(
            state.selected(),
            Some(TreeRow::Session {
                channel: 0,
                session: 0
            })
        );
        reduce(&mut state, &[TreeAction::CollapseAll]);
        assert_eq!(state.selected(), Some(TreeRow::Channel(0)));
        for _ in 0..CHANNELS + 5 {
            reduce(&mut state, &[TreeAction::SelectNext]);
        }
        assert_eq!(state.selected(), Some(TreeRow::Channel(CHANNELS - 1)));
    }

    #[::std::prelude::v1::test]
    fn closing_a_channel_moves_a_selection_inside_it_onto_the_channel() {
        let mut state = state();
        reduce(
            &mut state,
            &[
                TreeAction::Toggle(3),
                TreeAction::Select(TreeRow::Session {
                    channel: 3,
                    session: 5,
                }),
                TreeAction::Toggle(3),
            ],
        );
        assert_eq!(state.selected(), Some(TreeRow::Channel(3)));
        assert!(state.expanded().is_empty());
    }

    #[::std::prelude::v1::test]
    fn rows_the_outline_lacks_are_not_selected() {
        let mut state = state();
        reduce(&mut state, &[TreeAction::Select(TreeRow::Channel(CHANNELS))]);
        assert_eq!(state.selected(), None);
    }
}
