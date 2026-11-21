//! GPUI To-do board view for the browser (WEBTODO, D905).
//!
//! Uses `humata2_todo`'s exact ranking, grouping, filtering, due-chip, and
//! header rules so the browser shows the same ranked To-do view as the desktop
//! app, updated live via WebSocket push.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{prelude::*, *};
use humata2_todo::{
    ChannelFilter, Day, DueChip, Effort, Filter, Impact, Need, Row, SortBy, State, Todo, Tone,
    apply, chip_for, header, rank_with,
};
use humata2_web_todo_wire::{BoardSnapshot, ChannelInfo, SessionInfo};

const BG_APP: u32 = 0x17181b;
const BG_SURFACE: u32 = 0x1e2025;
const BG_ROW_HOVER: u32 = 0x262930;
const BG_ROW_SELECTED: u32 = 0x2a2f3a;
const BG_PILL: u32 = 0x282b33;
const BG_PILL_ACTIVE: u32 = 0x3b4861;
const BORDER: u32 = 0x2e313a;
const TEXT_PRIMARY: u32 = 0xe8e9ed;
const TEXT_MUTED: u32 = 0x989aa3;
const TEXT_FAINT: u32 = 0x6f727c;
const ACCENT: u32 = 0x60a5fa;
const GREEN: u32 = 0x4ade80;
const AMBER: u32 = 0xfbbf24;
const RED: u32 = 0xf87171;
const PURPLE: u32 = 0xc084fc;

const SORT_OPTIONS: [SortBy; 5] = [
    SortBy::Priority,
    SortBy::DueSoonest,
    SortBy::MostNeeded,
    SortBy::SmallestEffort,
    SortBy::LargestImpact,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Attention,
    InProgress,
    WatchingPr,
    Next,
    Waiting,
    Hidden,
    Closed,
}

impl Group {
    const fn bit(self) -> u32 {
        1 << (self as u32)
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::InProgress => "In progress",
            Self::WatchingPr => "Watching PR",
            Self::Next => "Next",
            Self::Waiting => "Waiting on another item",
            Self::Hidden => "Hidden until a date",
            Self::Closed => "Closed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    Connecting,
    Live,
    Reconnecting(String),
}

pub(crate) struct WebTodoView {
    today: Day,
    items: Vec<Todo>,
    channels: Vec<ChannelInfo>,
    channel_names: HashMap<String, String>,
    sessions: HashMap<String, SessionInfo>,
    revision: u64,
    connection: ConnectionState,
    reconnects: u32,
    error: Option<String>,
    selected_channel: Option<String>,
    show_closed: bool,
    sort: SortBy,
    folded_toggles: u32,
    selected_key: Option<String>,
    diagnostics: Rc<RefCell<String>>,
}

impl WebTodoView {
    pub(crate) fn new(diagnostics: Rc<RefCell<String>>) -> Self {
        let view = Self {
            today: Day::from_ymd(2026, 11, 20).expect("default day"),
            items: Vec::new(),
            channels: Vec::new(),
            channel_names: HashMap::new(),
            sessions: HashMap::new(),
            revision: 0,
            connection: ConnectionState::Connecting,
            reconnects: 0,
            error: None,
            selected_channel: None,
            show_closed: false,
            sort: SortBy::Priority,
            folded_toggles: 0,
            selected_key: None,
            diagnostics,
        };
        view.publish_diagnostics();
        view
    }

    pub(crate) fn set_connection(&mut self, state: ConnectionState, cx: &mut Context<Self>) {
        if matches!(state, ConnectionState::Reconnecting(_)) {
            self.reconnects = self.reconnects.saturating_add(1);
        }
        if matches!(state, ConnectionState::Live) {
            self.error = None;
        }
        self.connection = state;
        self.publish_diagnostics();
        cx.notify();
    }

    pub(crate) fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message);
        self.publish_diagnostics();
        cx.notify();
    }

    pub(crate) fn apply_snapshot(&mut self, snapshot: BoardSnapshot, cx: &mut Context<Self>) {
        self.today = snapshot.today;
        self.items = snapshot.items;
        self.channel_names = snapshot
            .channels
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        self.channels = snapshot.channels;
        self.sessions = snapshot
            .sessions
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect();
        self.revision = snapshot.revision;
        self.connection = ConnectionState::Live;
        self.error = None;
        if let Some(key) = &self.selected_key
            && !self.items.iter().any(|t| t.key == *key)
        {
            self.selected_key = None;
        }
        self.publish_diagnostics();
        cx.notify();
    }

    fn current_filter(&self) -> Filter {
        let mut filter = Filter::default();
        filter.include_terminal = self.show_closed;
        if let Some(ch) = &self.selected_channel {
            filter.channel = Some(ChannelFilter::Channel(ch.clone()));
        }
        filter
    }

    fn header_line(&self, shown: &[Todo]) -> String {
        if self.items.is_empty() {
            return "no items".to_owned();
        }
        let (open, channels) = header(shown, self.today);
        if let Some(ch) = &self.selected_channel {
            let name = self
                .channel_names
                .get(ch)
                .map(String::as_str)
                .unwrap_or(ch.as_str());
            format!("{open} open in {name}")
        } else {
            let unit = if channels == 1 { "channel" } else { "channels" };
            format!("{open} open across {channels} {unit}")
        }
    }

    fn is_folded(&self, group: Group, shown_len: usize) -> bool {
        let by_default = match group {
            Group::Waiting | Group::Hidden => self.selected_channel.is_none(),
            Group::Attention | Group::InProgress | Group::WatchingPr => shown_len > 40,
            Group::Next | Group::Closed => false,
        };
        let toggled = (self.folded_toggles & group.bit()) != 0;
        by_default != toggled
    }

    fn publish_diagnostics(&self) {
        let filter = self.current_filter();
        let shown = apply(&self.items, &filter, self.today);
        let ranked = rank_with(&shown, self.sort, self.today);
        let keys: Vec<&str> = self.items.iter().map(|t| t.key.as_str()).collect();
        let connected = matches!(self.connection, ConnectionState::Live);
        let phase = match &self.connection {
            ConnectionState::Connecting => "connecting",
            ConnectionState::Live => "live",
            ConnectionState::Reconnecting(_) => "reconnecting",
        };
        *self.diagnostics.borrow_mut() = serde_json::json!({
            "phase": phase,
            "connected": connected,
            "item_count": self.items.len(),
            "shown_count": shown.len(),
            "header": self.header_line(&shown),
            "do_next": ranked.do_next,
            "keys": keys,
            "revision": self.revision,
            "reconnects": self.reconnects,
            "error": self.error,
        })
        .to_string();
    }

    fn render_row(&self, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(todo) = self.items.iter().find(|t| t.key == row.key) else {
            return div().into_any_element();
        };
        let key = todo.key.clone();
        let selected = self.selected_key.as_deref() == Some(todo.key.as_str());
        let state_dot = match todo.state {
            State::InProgress => GREEN,
            State::WatchingPr => PURPLE,
            State::Closed | State::WontDo | State::Became => TEXT_FAINT,
            State::Open => ACCENT,
        };

        let due_chip: Option<DueChip> = chip_for(todo, self.today);
        let step_text = step_progress(todo);
        let channel_label = todo
            .channel
            .as_ref()
            .map(|id| self.channel_names.get(id).cloned().unwrap_or_else(|| id.clone()));
        let session_chip = todo
            .sessions
            .first()
            .and_then(|sid| self.sessions.get(sid))
            .cloned();

        let mut container = div()
            .id(ElementId::Name(format!("row-{}", todo.key).into()))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .bg(rgb(if selected { BG_ROW_SELECTED } else { BG_SURFACE }))
            .hover(|s| s.bg(rgb(BG_ROW_HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_key = Some(key.clone());
                cx.notify();
            }));

        // Left side: status dot, key, title, optional reason/ask
        let mut left = div().flex().flex_row().items_center().gap_2().min_w_0().flex_1();
        left = left
            .child(div().size(px(8.0)).rounded_full().bg(rgb(state_dot)))
            .child(
                div()
                    .text_xs()
                    .font_family("JetBrains Mono")
                    .text_color(rgb(TEXT_MUTED))
                    .child(todo.key.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(TEXT_PRIMARY))
                    .truncate()
                    .child(todo.title.clone()),
            );

        // Right side: pills (channel, session, steps, due, effort/impact/need)
        let mut right = div().flex().flex_row().items_center().gap_2();

        if let Some(ch) = channel_label {
            right = right.child(pill(ch, BG_PILL, TEXT_MUTED));
        }
        if let Some(sess) = session_chip {
            right = right.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(2.0))
                    .rounded_sm()
                    .bg(rgb(BG_PILL))
                    .child(div().size(px(6.0)).rounded_full().bg(rgb(sess.dot)))
                    .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child(sess.name)),
            );
        }
        if let Some(steps) = step_text {
            right = right.child(pill(steps, BG_PILL, TEXT_MUTED));
        }
        if let Some(need) = todo.need {
            let color = match need {
                Need::Critical => RED,
                Need::Expected => AMBER,
                Need::NiceToHave | Need::Idea => TEXT_MUTED,
            };
            right = right.child(pill(need.label().to_owned(), BG_PILL, color));
        }
        if let Some(effort) = todo.effort {
            let label = match effort {
                Effort::S => "S",
                Effort::M => "M",
                Effort::L => "L",
            };
            right = right.child(pill(label.to_owned(), BG_PILL, TEXT_MUTED));
        }
        if let Some(impact) = todo.impact {
            let label = match impact {
                Impact::Low => "low",
                Impact::Medium => "med",
                Impact::High => "high",
            };
            right = right.child(pill(label.to_owned(), BG_PILL, TEXT_MUTED));
        }
        if let Some(due) = due_chip {
            let fg = match due.tone {
                Tone::Overdue => RED,
                Tone::Soon => AMBER,
                Tone::Plain => TEXT_MUTED,
            };
            right = right.child(pill(due.text, BG_PILL, fg));
        }

        container = container.child(left).child(right);
        container.into_any_element()
    }

    fn render_drawer(&self, todo: &Todo, cx: &mut Context<Self>) -> impl IntoElement {
        let mut drawer = div()
            .w(px(340.0))
            .h_full()
            .border_l_1()
            .border_color(rgb(BORDER))
            .bg(rgb(BG_SURFACE))
            .p_4()
            .flex()
            .flex_col()
            .gap_3();

        // Top row: key + state + close button
        let top = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_family("JetBrains Mono")
                            .text_color(rgb(TEXT_MUTED))
                            .child(todo.key.clone()),
                    )
                    .child(pill(state_label(todo.state).to_owned(), BG_PILL, ACCENT)),
            )
            .child(
                div()
                    .id("drawer-close")
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_sm()
                    .text_color(rgb(TEXT_MUTED))
                    .hover(|s| s.bg(rgb(BG_ROW_HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected_key = None;
                        cx.notify();
                    }))
                    .child("×"),
            );

        drawer = drawer.child(top).child(
            div()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT_PRIMARY))
                .child(todo.title.clone()),
        );

        if !todo.description.trim().is_empty() {
            drawer = drawer.child(
                div()
                    .text_sm()
                    .text_color(rgb(TEXT_MUTED))
                    .child(todo.description.clone()),
            );
        }

        if let Some(owner) = &todo.owner {
            drawer = drawer.child(
                div()
                    .text_xs()
                    .text_color(rgb(TEXT_MUTED))
                    .child(format!("Owner: {}", owner.label)),
            );
        }

        for pr in &todo.watched_prs {
            drawer = drawer.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(BG_APP))
                    .text_xs()
                    .text_color(rgb(PURPLE))
                    .child(format!("PR {} · {}", pr.repo_pr, pr.title)),
            );
        }

        if !todo.steps.is_empty() {
            let mut list = div().flex().flex_col().gap_1();
            list = list.child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_MUTED))
                    .child("Steps"),
            );
            for step in todo.steps.iter().filter(|s| !s.dropped) {
                let mark = if step.done { "✓" } else { "○" };
                let color = if step.done { TEXT_FAINT } else { TEXT_PRIMARY };
                list = list.child(
                    div()
                        .text_xs()
                        .text_color(rgb(color))
                        .child(format!("{mark} {}", step.text)),
                );
            }
            drawer = drawer.child(list);
        }

        if !todo.depends_on.is_empty() {
            drawer = drawer.child(
                div()
                    .text_xs()
                    .text_color(rgb(TEXT_MUTED))
                    .child(format!("Depends on: {}", todo.depends_on.join(", "))),
            );
        }

        if !todo.tags.is_empty() {
            let mut tags = div().flex().flex_row().flex_wrap().gap_1();
            for tag in &todo.tags {
                tags = tags.child(pill(format!("#{tag}"), BG_PILL, TEXT_MUTED));
            }
            drawer = drawer.child(tags);
        }

        drawer
    }
}

fn pill(text: String, bg: u32, fg: u32) -> impl IntoElement {
    div()
        .px_2()
        .py(px(2.0))
        .rounded_sm()
        .bg(rgb(bg))
        .text_xs()
        .text_color(rgb(fg))
        .child(text)
}

fn step_progress(todo: &Todo) -> Option<String> {
    let active: Vec<_> = todo.steps.iter().filter(|s| !s.dropped).collect();
    if active.is_empty() {
        return None;
    }
    let done = active.iter().filter(|s| s.done).count();
    Some(format!("{done}/{}", active.len()))
}

fn state_label(state: State) -> &'static str {
    match state {
        State::Open => "Open",
        State::InProgress => "In progress",
        State::Closed => "Closed",
        State::WontDo => "Won't do",
        State::Became => "Became",
        State::WatchingPr => "Watching PR",
    }
}

impl Render for WebTodoView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.current_filter();
        let shown = apply(&self.items, &filter, self.today);
        let ranked = rank_with(&shown, self.sort, self.today);
        let header_str = self.header_line(&shown);
        let do_next_key = ranked.do_next.clone();
        let do_next_label = match &do_next_key {
            Some(key) => format!("Do next · {key}"),
            None => "Do next · nothing yet".to_owned(),
        };

        let (conn_dot, conn_text) = match &self.connection {
            ConnectionState::Live => (GREEN, "Live"),
            ConnectionState::Connecting => (AMBER, "Connecting…"),
            ConnectionState::Reconnecting(_) => (AMBER, "Reconnecting…"),
        };

        // Top header bar
        let header_bar = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_PRIMARY))
                            .child("To-do"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(TEXT_MUTED))
                            .child(header_str),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(2.0))
                            .rounded_full()
                            .bg(rgb(BG_PILL))
                            .child(div().size(px(7.0)).rounded_full().bg(rgb(conn_dot)))
                            .child(div().text_xs().text_color(rgb(TEXT_MUTED)).child(conn_text)),
                    ),
            )
            .child(
                div()
                    .id("do-next-btn")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(BG_PILL_ACTIVE))
                    .text_xs()
                    .text_color(rgb(TEXT_PRIMARY))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(key) = &do_next_key {
                            this.selected_key = Some(key.clone());
                            cx.notify();
                        }
                    }))
                    .child(do_next_label),
            );

        // Filter & sort bar
        let mut toolbar = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_5()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .gap_3();

        let mut channel_pills = div().flex().flex_row().items_center().gap_1().flex_wrap();
        channel_pills = channel_pills.child(
            div()
                .id("ch-all")
                .px_2()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(if self.selected_channel.is_none() {
                    BG_PILL_ACTIVE
                } else {
                    BG_PILL
                }))
                .text_xs()
                .text_color(rgb(TEXT_PRIMARY))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.selected_channel = None;
                    this.publish_diagnostics();
                    cx.notify();
                }))
                .child("All channels"),
        );

        for ch in &self.channels {
            let id = ch.id.clone();
            let active = self.selected_channel.as_deref() == Some(ch.id.as_str());
            channel_pills = channel_pills.child(
                div()
                    .id(ElementId::Name(format!("ch-{}", ch.id).into()))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(if active { BG_PILL_ACTIVE } else { BG_PILL }))
                    .text_xs()
                    .text_color(rgb(TEXT_PRIMARY))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_channel = Some(id.clone());
                        this.publish_diagnostics();
                        cx.notify();
                    }))
                    .child(ch.name.clone()),
            );
        }

        let next_sort = SORT_OPTIONS
            .iter()
            .position(|s| *s == self.sort)
            .map(|i| SORT_OPTIONS[(i + 1) % SORT_OPTIONS.len()])
            .unwrap_or(SortBy::Priority);

        let right_controls = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("toggle-closed")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(if self.show_closed {
                        BG_PILL_ACTIVE
                    } else {
                        BG_PILL
                    }))
                    .text_xs()
                    .text_color(rgb(TEXT_PRIMARY))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_closed = !this.show_closed;
                        this.publish_diagnostics();
                        cx.notify();
                    }))
                    .child(if self.show_closed {
                        "Showing closed"
                    } else {
                        "Show closed"
                    }),
            )
            .child(
                div()
                    .id("cycle-sort")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(BG_PILL))
                    .text_xs()
                    .text_color(rgb(TEXT_MUTED))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sort = next_sort;
                        this.publish_diagnostics();
                        cx.notify();
                    }))
                    .child(format!("Sort: {}", self.sort.label())),
            );

        toolbar = toolbar.child(channel_pills).child(right_controls);

        // Ranked sections
        let sections: [(Group, Vec<Row>); 7] = [
            (Group::Attention, ranked.needs_attention),
            (Group::InProgress, ranked.in_progress),
            (Group::WatchingPr, ranked.watching_pr),
            (Group::Next, ranked.next),
            (Group::Waiting, ranked.waiting),
            (Group::Hidden, ranked.hidden),
            (Group::Closed, ranked.closed),
        ];

        let mut list = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .overflow_hidden();

        if let Some(err) = &self.error {
            list = list.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x3a2024))
                    .text_xs()
                    .text_color(rgb(RED))
                    .child(err.clone()),
            );
        }

        let mut any_section = false;
        for (group, rows) in sections {
            if rows.is_empty() {
                continue;
            }
            any_section = true;
            let folded = self.is_folded(group, shown.len());
            let chevron = if folded { "▸" } else { "▾" };
            let mut section_div = div().flex().flex_col().gap_1();
            section_div = section_div.child(
                div()
                    .id(ElementId::Name(format!("group-{}", group.bit()).into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.folded_toggles ^= group.bit();
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_MUTED))
                            .child(format!("{chevron} {} ({})", group.title(), rows.len())),
                    ),
            );
            if !folded {
                for row in &rows {
                    section_div = section_div.child(self.render_row(row, cx));
                }
            }
            list = list.child(section_div);
        }

        if !any_section {
            list = list.child(
                div()
                    .py_8()
                    .text_sm()
                    .text_color(rgb(TEXT_FAINT))
                    .child("No To-do items yet."),
            );
        }

        // Body: list + optional detail drawer
        let mut body = div().flex_1().flex().flex_row().min_h_0();
        body = body.child(list);
        if let Some(selected) = self
            .selected_key
            .as_deref()
            .and_then(|key| self.items.iter().find(|t| t.key == key))
            .cloned()
        {
            body = body.child(self.render_drawer(&selected, cx));
        }

        div()
            .size_full()
            .bg(rgb(BG_APP))
            .text_color(rgb(TEXT_PRIMARY))
            .font_family("Inter")
            .flex()
            .flex_col()
            .child(header_bar)
            .child(toolbar)
            .child(body)
    }
}
