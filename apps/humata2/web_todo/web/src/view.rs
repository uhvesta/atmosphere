//! GPUI Minimal Chrome Web UI for To-do and Reviews (WEBTODO, D905).
//!
//! Renders:
//! - `/todo` (and `/`): ranked To-do board with minimal 46px top bar
//! - `/reviews`: Reviews inbox (Local stacks + GitHub feedback, requests & watched PRs)
//! - `/reviews/<id>?layer=N`: stacked layer diff review with left-rail layer list,
//!   breadcrumb chain, Unified/Split toggle, and Desktop handoff.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{prelude::*, *};
use humata2_todo::{
    ChannelFilter, Day, DueChip, Effort, Filter, Impact, Need, Row, SortBy, State, Todo, Tone,
    apply, chip_for, header, rank_with,
};
use humata2_web_todo_wire::{
    BoardSnapshot, ChannelInfo, ClientMessage, LayerStatus, ReviewDetail, ReviewDiffFile,
    ReviewInboxItem, ReviewKind, ReviewLayer, SessionInfo, WatchedPrEntry,
};

const BG_APP: u32 = 0x17181b;
const BG_SURFACE: u32 = 0x1e2025;
const BG_ROW_HOVER: u32 = 0x262930;
const BG_ROW_SELECTED: u32 = 0x2a2f3a;
const BG_PILL: u32 = 0x282b33;
const BG_PILL_ACTIVE: u32 = 0x3b4861;
const BG_DIFF_ADD: u32 = 0x1a2e22;
const BG_DIFF_DEL: u32 = 0x341e22;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WebRoute {
    Todo,
    ReviewsInbox,
    ReviewDetail { id: String, layer: Option<u32> },
}

impl WebRoute {
    fn from_location(raw: &str) -> Self {
        let (path, query) = raw.split_once('?').unwrap_or((raw, ""));
        let trimmed = path.trim_end_matches('/');
        if trimmed == "/reviews" {
            return Self::ReviewsInbox;
        }
        if let Some(id) = trimmed.strip_prefix("/reviews/")
            && !id.is_empty()
        {
            let layer = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("layer="))
                .and_then(|val| val.parse::<u32>().ok());
            return Self::ReviewDetail {
                id: id.to_owned(),
                layer,
            };
        }
        Self::Todo
    }

    fn to_path(&self) -> String {
        match self {
            Self::Todo => "/todo".to_owned(),
            Self::ReviewsInbox => "/reviews".to_owned(),
            Self::ReviewDetail { id, layer: Some(n) } => format!("/reviews/{id}?layer={n}"),
            Self::ReviewDetail { id, layer: None } => format!("/reviews/{id}"),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::ReviewsInbox => "reviews",
            Self::ReviewDetail { .. } => "review_detail",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewFilter {
    All,
    Local,
    Github,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffScope {
    AllChanges,
    SinceLastReview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffLayout {
    Unified,
    Split,
}

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
    reviews: Vec<ReviewInboxItem>,
    watched_prs: Vec<WatchedPrEntry>,
    github_checked_ago_secs: Option<u64>,
    review_details: Vec<ReviewDetail>,
    revision: u64,
    connection: ConnectionState,
    reconnects: u32,
    error: Option<String>,
    route: WebRoute,
    selected_channel: Option<String>,
    review_filter: ReviewFilter,
    show_watch_bar: bool,
    watch_url_draft: String,
    show_details_card: bool,
    diff_scope: DiffScope,
    diff_layout: DiffLayout,
    selected_file_path: Option<String>,
    show_closed: bool,
    sort: SortBy,
    folded_toggles: u32,
    selected_key: Option<String>,
    status_notice: Option<String>,
    diagnostics: Rc<RefCell<String>>,
}

impl WebTodoView {
    pub(crate) fn new(diagnostics: Rc<RefCell<String>>) -> Self {
        let initial_route = WebRoute::from_location(&crate::current_location_path());
        let view = Self {
            today: Day::from_ymd(2026, 11, 20).expect("default day"),
            items: Vec::new(),
            channels: Vec::new(),
            channel_names: HashMap::new(),
            sessions: HashMap::new(),
            reviews: Vec::new(),
            watched_prs: Vec::new(),
            github_checked_ago_secs: None,
            review_details: Vec::new(),
            revision: 0,
            connection: ConnectionState::Connecting,
            reconnects: 0,
            error: None,
            route: initial_route,
            selected_channel: None,
            review_filter: ReviewFilter::All,
            show_watch_bar: false,
            watch_url_draft: "https://github.com/acme/payments-api/pull/412".to_owned(),
            show_details_card: false,
            diff_scope: DiffScope::AllChanges,
            diff_layout: DiffLayout::Unified,
            selected_file_path: None,
            show_closed: false,
            sort: SortBy::Priority,
            folded_toggles: 0,
            selected_key: None,
            status_notice: None,
            diagnostics,
        };
        view.publish_diagnostics();
        view
    }

    fn navigate(&mut self, route: WebRoute, cx: &mut Context<Self>) {
        crate::push_browser_route(&route.to_path());
        self.route = route;
        self.selected_file_path = None;
        self.status_notice = None;
        self.publish_diagnostics();
        cx.notify();
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
        self.reviews = snapshot.reviews;
        self.watched_prs = snapshot.watched_prs;
        self.github_checked_ago_secs = snapshot.github_checked_ago_secs;
        self.review_details = snapshot.review_details;
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
        let (active_review, active_layer) = match &self.route {
            WebRoute::ReviewDetail { id, layer } => (Some(id.as_str()), *layer),
            _ => (None, None),
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
            "route": self.route.label(),
            "reviews_count": self.reviews.len(),
            "active_review": active_review,
            "active_layer": active_layer,
        })
        .to_string();
    }

    fn render_top_bar(&self, open_todo_count: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let is_todo = matches!(self.route, WebRoute::Todo);
        let is_reviews = matches!(
            self.route,
            WebRoute::ReviewsInbox | WebRoute::ReviewDetail { .. }
        );
        let reviews_count = self.reviews.len();

        let mut left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_PRIMARY))
                    .child("Humata"),
            )
            .child(
                div()
                    .id("nav-todo")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(if is_todo { BG_PILL_ACTIVE } else { BG_APP }))
                    .text_xs()
                    .text_color(rgb(if is_todo { TEXT_PRIMARY } else { TEXT_MUTED }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate(WebRoute::Todo, cx);
                    }))
                    .child(format!("To-do {open_todo_count}")),
            )
            .child(
                div()
                    .id("nav-reviews")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(if is_reviews { BG_PILL_ACTIVE } else { BG_APP }))
                    .text_xs()
                    .text_color(rgb(if is_reviews { TEXT_PRIMARY } else { TEXT_MUTED }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate(WebRoute::ReviewsInbox, cx);
                    }))
                    .child(format!("Reviews {reviews_count}")),
            );

        // Inline filter pills in the 46px top bar
        if is_todo {
            let mut pills = div().flex().flex_row().items_center().gap_1();
            pills = pills.child(
                div()
                    .id("ch-all")
                    .px_2()
                    .py(px(2.0))
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
                    .child("All"),
            );
            for ch in &self.channels {
                let id = ch.id.clone();
                let active = self.selected_channel.as_deref() == Some(ch.id.as_str());
                pills = pills.child(
                    div()
                        .id(ElementId::Name(format!("ch-{}", ch.id).into()))
                        .px_2()
                        .py(px(2.0))
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
            left = left.child(pills);
        } else if matches!(self.route, WebRoute::ReviewsInbox) {
            let local_n = self
                .reviews
                .iter()
                .filter(|r| r.kind == ReviewKind::Local)
                .count();
            let gh_n = self.reviews.len().saturating_sub(local_n);
            let filters = [
                (ReviewFilter::All, "All".to_owned()),
                (ReviewFilter::Local, format!("Local · {local_n}")),
                (ReviewFilter::Github, format!("GitHub · {gh_n}")),
            ];
            let mut pills = div().flex().flex_row().items_center().gap_1();
            for (kind, label) in filters {
                let active = self.review_filter == kind;
                pills = pills.child(
                    div()
                        .id(ElementId::Name(format!("rf-{kind:?}").into()))
                        .px_2()
                        .py(px(2.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(if active { BG_PILL_ACTIVE } else { BG_PILL }))
                        .text_xs()
                        .text_color(rgb(TEXT_PRIMARY))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.review_filter = kind;
                            cx.notify();
                        }))
                        .child(label),
                );
            }
            left = left.child(pills);
        }

        // Right side of minimal 46px top bar
        let right = if is_todo {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(BG_PILL))
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .child("⌘K Search"),
                )
        } else {
            let checked_label = match self.github_checked_ago_secs {
                Some(secs) if secs < 60 => "GitHub checked just now".to_owned(),
                Some(secs) => format!("GitHub checked {}m ago", secs / 60),
                None => "GitHub not checked yet".to_owned(),
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("refresh-reviews")
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(BG_PILL))
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .hover(|s| s.bg(rgb(BG_ROW_HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            crate::send_client_message(&ClientMessage::RefreshReviews);
                            this.status_notice = Some("Refreshing GitHub reviews…".to_owned());
                            cx.notify();
                        }))
                        .child(format!("{checked_label} · Refresh")),
                )
                .child(
                    div()
                        .id("toggle-watch-pr")
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(BG_PILL_ACTIVE))
                        .text_xs()
                        .text_color(rgb(TEXT_PRIMARY))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_watch_bar = !this.show_watch_bar;
                            cx.notify();
                        }))
                        .child("+ Watch PR URL…"),
                )
        };

        div()
            .h(px(46.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(BG_APP))
            .child(left)
            .child(right)
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
        let channel_label = todo.channel.as_ref().map(|id| {
            self.channel_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.clone())
        });
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
            .bg(rgb(if selected {
                BG_ROW_SELECTED
            } else {
                BG_SURFACE
            }))
            .hover(|s| s.bg(rgb(BG_ROW_HOVER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_key = Some(key.clone());
                cx.notify();
            }));

        let mut left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .min_w_0()
            .flex_1();
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
        let linked_review_id = self
            .reviews
            .iter()
            .find(|r| r.id == todo.key)
            .map(|r| r.id.clone());

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

        // Read-only web handoff actions
        let mut actions = div().flex().flex_row().items_center().gap_2();
        if let Some(rid) = linked_review_id {
            actions = actions.child(
                div()
                    .id("drawer-review-diff")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(BG_PILL_ACTIVE))
                    .text_xs()
                    .text_color(rgb(TEXT_PRIMARY))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigate(
                            WebRoute::ReviewDetail {
                                id: rid.clone(),
                                layer: Some(1),
                            },
                            cx,
                        );
                    }))
                    .child("Review diff"),
            );
        }
        actions = actions.child(
            div()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(rgb(BG_PILL))
                .text_xs()
                .text_color(rgb(TEXT_MUTED))
                .child("Open in Desktop ↗"),
        );
        drawer = drawer.child(actions);

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

    fn render_todo_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.current_filter();
        let shown = apply(&self.items, &filter, self.today);
        let ranked = rank_with(&shown, self.sort, self.today);
        let header_str = self.header_line(&shown);
        let do_next_key = ranked.do_next.clone();
        let do_next_label = match &do_next_key {
            Some(key) => format!("Do next · {key}"),
            None => "Do next · nothing yet".to_owned(),
        };

        let next_sort = SORT_OPTIONS
            .iter()
            .position(|s| *s == self.sort)
            .map(|i| SORT_OPTIONS[(i + 1) % SORT_OPTIONS.len()])
            .unwrap_or(SortBy::Priority);

        let subbar = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_5()
            .py_2()
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
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(header_str),
                    )
                    .child(
                        div()
                            .id("do-next-btn")
                            .px_2()
                            .py(px(2.0))
                            .rounded_sm()
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
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("toggle-closed")
                            .px_2()
                            .py(px(2.0))
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
                            .py(px(2.0))
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
                    ),
            );

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
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .child(subbar)
            .child(body)
    }

    fn render_review_inbox_row(
        &self,
        item: &ReviewInboxItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = item.id.clone();
        let chip_fg = match item.chip_tone.as_str() {
            "amber" => AMBER,
            "green" => GREEN,
            "purple" => PURPLE,
            "red" => RED,
            "accent" => ACCENT,
            _ => TEXT_MUTED,
        };
        let chip_label = if item.chip_label.is_empty() {
            match item.kind {
                ReviewKind::Local => "New".to_owned(),
                ReviewKind::GithubFeedback => "New feedback".to_owned(),
                ReviewKind::ReviewRequested => "Review requested".to_owned(),
                ReviewKind::Watched => "Watched".to_owned(),
            }
        } else {
            item.chip_label.clone()
        };

        div()
            .id(ElementId::Name(format!("rev-row-{}", item.id).into()))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(BG_SURFACE))
            .hover(|s| s.bg(rgb(BG_ROW_HOVER)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .flex_1()
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
                                    .child(item.id.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .truncate()
                                    .child(item.title.clone()),
                            )
                            .child(pill(chip_label, BG_PILL, chip_fg)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_FAINT))
                            .child(item.subtitle.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .font_family("JetBrains Mono")
                            .text_color(rgb(GREEN))
                            .child(format!("+{}", item.added)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family("JetBrains Mono")
                            .text_color(rgb(RED))
                            .child(format!("-{}", item.removed)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_FAINT))
                            .child(item.age_label.clone()),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(format!("rev-open-{}", item.id).into()))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(BG_PILL_ACTIVE))
                            .text_xs()
                            .text_color(rgb(TEXT_PRIMARY))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.navigate(
                                    WebRoute::ReviewDetail {
                                        id: id.clone(),
                                        layer: Some(1),
                                    },
                                    cx,
                                );
                            }))
                            .child("Review"),
                    ),
            )
    }

    fn render_reviews_inbox(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut container = div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_between()
            .min_h_0();

        let mut content = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .overflow_hidden();

        if let Some(err) = &self.error {
            content = content.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x3a2024))
                    .text_xs()
                    .text_color(rgb(RED))
                    .child(err.clone()),
            );
        }

        if let Some(notice) = &self.status_notice {
            content = content.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(BG_SURFACE))
                    .text_xs()
                    .text_color(rgb(ACCENT))
                    .child(notice.clone()),
            );
        }

        if self.show_watch_bar {
            let draft = self.watch_url_draft.clone();
            content = content.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(BG_SURFACE))
                    .child(
                        div()
                            .text_xs()
                            .font_family("JetBrains Mono")
                            .text_color(rgb(TEXT_MUTED))
                            .child(draft.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("submit-watch-pr")
                                    .px_3()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(rgb(BG_PILL_ACTIVE))
                                    .text_xs()
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        crate::send_client_message(&ClientMessage::WatchPrUrl {
                                            url: draft.clone(),
                                        });
                                        this.show_watch_bar = false;
                                        this.status_notice =
                                            Some(format!("Watching PR {draft}"));
                                        cx.notify();
                                    }))
                                    .child("Watch PR"),
                            )
                            .child(
                                div()
                                    .id("cancel-watch-pr")
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(rgb(BG_PILL))
                                    .text_xs()
                                    .text_color(rgb(TEXT_MUTED))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_watch_bar = false;
                                        cx.notify();
                                    }))
                                    .child("Cancel"),
                            ),
                    ),
            );
        }

        let local_items: Vec<&ReviewInboxItem> = self
            .reviews
            .iter()
            .filter(|r| r.kind == ReviewKind::Local)
            .collect();
        let github_items: Vec<&ReviewInboxItem> = self
            .reviews
            .iter()
            .filter(|r| r.kind != ReviewKind::Local)
            .collect();

        if matches!(self.review_filter, ReviewFilter::All | ReviewFilter::Local) {
            let mut sec = div().flex().flex_col().gap_2();
            sec = sec.child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_MUTED))
                    .child(format!("LOCAL · READY TO PUSH ({})", local_items.len())),
            );
            if local_items.is_empty() {
                sec = sec.child(
                    div()
                        .text_xs()
                        .text_color(rgb(TEXT_FAINT))
                        .child("No local stacks ready to push."),
                );
            } else {
                for item in local_items {
                    sec = sec.child(self.render_review_inbox_row(item, cx));
                }
            }
            content = content.child(sec);
        }

        if matches!(self.review_filter, ReviewFilter::All | ReviewFilter::Github) {
            let mut sec = div().flex().flex_col().gap_2();
            sec = sec.child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_MUTED))
                    .child(format!(
                        "ON GITHUB · FEEDBACK, REQUESTS & WATCHED PRS ({})",
                        github_items.len() + self.watched_prs.len()
                    )),
            );
            if github_items.is_empty() && self.watched_prs.is_empty() {
                sec = sec.child(
                    div()
                        .text_xs()
                        .text_color(rgb(TEXT_FAINT))
                        .child("No GitHub feedback, review requests, or watched PRs."),
                );
            } else {
                for item in github_items {
                    sec = sec.child(self.render_review_inbox_row(item, cx));
                }
                for pr in &self.watched_prs {
                    sec = sec.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(BG_SURFACE))
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
                                            .child(format!("{}#{}", pr.repo, pr.number)),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(TEXT_PRIMARY))
                                            .child(pr.title.clone()),
                                    )
                                    .child(pill("Watched".to_owned(), BG_PILL, PURPLE)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(TEXT_FAINT))
                                    .child(pr.url.clone()),
                            ),
                    );
                }
            }
            content = content.child(sec);
        }

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .px_5()
            .py_2()
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(BG_PILL))
                    .text_xs()
                    .text_color(rgb(TEXT_MUTED))
                    .child("Open Reviews in Desktop ↗"),
            );

        container = container.child(content).child(footer);
        container
    }

    fn render_review_detail(
        &self,
        id: &str,
        layer_num: Option<u32>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(detail) = self.review_details.iter().find(|d| d.id == id) else {
            return div()
                .flex_1()
                .p_6()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .id("back-to-reviews-missing")
                        .cursor_pointer()
                        .text_xs()
                        .text_color(rgb(ACCENT))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.navigate(WebRoute::ReviewsInbox, cx);
                        }))
                        .child("← Reviews"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(TEXT_MUTED))
                        .child(format!("Review {id} not found in snapshot.")),
                )
                .into_any_element();
        };

        let active_layer_idx = layer_num
            .and_then(|n| detail.layers.iter().position(|l| l.number == n))
            .unwrap_or(0);
        let active_layer: Option<&ReviewLayer> = detail.layers.get(active_layer_idx);
        let layer_count = detail.layers.len();
        let review_id = detail.id.clone();

        // Sub-header for /reviews/<id>?layer=N
        let badge_text = if detail.is_local {
            format!("Local review · {layer_count} layers")
        } else {
            format!("GitHub review · {layer_count} layers")
        };

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(BG_SURFACE))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("back-to-reviews")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(BG_PILL))
                            .text_xs()
                            .text_color(rgb(TEXT_PRIMARY))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.navigate(WebRoute::ReviewsInbox, cx);
                            }))
                            .child("← Reviews"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_PRIMARY))
                            .child(format!("{} · {}", detail.id, detail.title)),
                    )
                    .child(pill(badge_text, BG_PILL, ACCENT)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("toggle-details-card")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(if self.show_details_card {
                                BG_PILL_ACTIVE
                            } else {
                                BG_PILL
                            }))
                            .text_xs()
                            .text_color(rgb(TEXT_PRIMARY))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_details_card = !this.show_details_card;
                                cx.notify();
                            }))
                            .child("Details ⤢"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_PILL))
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child("Open in Desktop (with Agent) ↗"),
                    ),
            );

        // Left rail: scope toggle, stack layers (with Rebased · diff unchanged), and files
        let mut rail = div()
            .w(px(270.0))
            .h_full()
            .border_r_1()
            .border_color(rgb(BORDER))
            .bg(rgb(BG_SURFACE))
            .p_3()
            .flex()
            .flex_col()
            .gap_3();

        rail = rail.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id("scope-all")
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(if self.diff_scope == DiffScope::AllChanges {
                            BG_PILL_ACTIVE
                        } else {
                            BG_PILL
                        }))
                        .text_xs()
                        .text_color(rgb(TEXT_PRIMARY))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.diff_scope = DiffScope::AllChanges;
                            cx.notify();
                        }))
                        .child("All changes"),
                )
                .child(
                    div()
                        .id("scope-since")
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(if self.diff_scope == DiffScope::SinceLastReview {
                            BG_PILL_ACTIVE
                        } else {
                            BG_PILL
                        }))
                        .text_xs()
                        .text_color(rgb(TEXT_PRIMARY))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.diff_scope = DiffScope::SinceLastReview;
                            cx.notify();
                        }))
                        .child("Since last review"),
                ),
        );

        rail = rail.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT_MUTED))
                        .child(format!("STACK · {layer_count} LAYERS")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(TEXT_FAINT))
                        .child("⌥⌘↑↓"),
                ),
        );

        // Whole stack row
        let whole_id = review_id.clone();
        rail = rail.child(
            div()
                .id("layer-whole-stack")
                .px_2()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(if layer_num.is_none() {
                    BG_ROW_SELECTED
                } else {
                    BG_APP
                }))
                .text_xs()
                .text_color(rgb(TEXT_PRIMARY))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(
                        WebRoute::ReviewDetail {
                            id: whole_id.clone(),
                            layer: None,
                        },
                        cx,
                    );
                }))
                .child("Whole stack"),
        );

        // Layer rows
        let mut layers_list = div().flex().flex_col().gap_1();
        for layer in &detail.layers {
            let is_active = active_layer.is_some_and(|a| a.number == layer.number);
            let (status_label, status_color) = match layer.status {
                LayerStatus::LookedAt => ("Looked at", GREEN),
                LayerStatus::FixesPushed => ("Fixes pushed", AMBER),
                LayerStatus::NotLookedAt => ("Not looked at", TEXT_FAINT),
            };
            let nav_id = review_id.clone();
            let nav_num = layer.number;
            let mut row = div()
                .id(ElementId::Name(format!("layer-row-{}", layer.number).into()))
                .px_2()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(if is_active {
                    BG_ROW_SELECTED
                } else {
                    BG_APP
                }))
                .hover(|s| s.bg(rgb(BG_ROW_HOVER)))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(
                        WebRoute::ReviewDetail {
                            id: nav_id.clone(),
                            layer: Some(nav_num),
                        },
                        cx,
                    );
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(TEXT_PRIMARY))
                                .child(format!("{} · {}", layer.number, layer.title)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(status_color))
                                .child(status_label),
                        ),
                );
            if layer.rebased_diff_unchanged {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(TEXT_FAINT))
                        .child("Rebased · diff unchanged"),
                );
            }
            layers_list = layers_list.child(row);
        }
        rail = rail.child(layers_list);

        // Per-repo file list for active layer
        if let Some(layer) = active_layer {
            let mut files_div = div().flex().flex_col().gap_1();
            files_div = files_div.child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT_MUTED))
                    .child(format!("FILES ({})", layer.files.len())),
            );
            for f in &layer.files {
                let path = f.path.clone();
                let selected = self.selected_file_path.as_deref() == Some(f.path.as_str());
                files_div = files_div.child(
                    div()
                        .id(ElementId::Name(format!("file-{}", f.path).into()))
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(if selected { BG_ROW_SELECTED } else { BG_APP }))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_file_path = Some(path.clone());
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_family("JetBrains Mono")
                                .text_color(rgb(TEXT_PRIMARY))
                                .truncate()
                                .child(f.path.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("JetBrains Mono")
                                .text_color(rgb(TEXT_FAINT))
                                .child(format!("+{} -{}", f.added, f.removed)),
                        ),
                );
            }
            rail = rail.child(files_div);
        }

        // Main pane: breadcrumb chain + optional details card + diff viewer + footer
        let mut main = div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_between()
            .min_w_0()
            .min_h_0();

        // Top layer breadcrumb chain
        let mut chain = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER));

        let mut crumbs = div().flex().flex_row().items_center().gap_2();
        for (idx, layer) in detail.layers.iter().enumerate() {
            if idx > 0 {
                crumbs = crumbs.child(div().text_xs().text_color(rgb(TEXT_FAINT)).child("→"));
            }
            let prefix = if layer.status == LayerStatus::LookedAt {
                "✓ "
            } else {
                ""
            };
            let is_cur = active_layer.is_some_and(|a| a.number == layer.number);
            let nav_id = review_id.clone();
            let nav_num = layer.number;
            crumbs = crumbs.child(
                div()
                    .id(ElementId::Name(format!("crumb-{}", layer.number).into()))
                    .px_2()
                    .py(px(2.0))
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(if is_cur { BG_PILL_ACTIVE } else { BG_PILL }))
                    .text_xs()
                    .text_color(rgb(if is_cur { TEXT_PRIMARY } else { TEXT_MUTED }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigate(
                            WebRoute::ReviewDetail {
                                id: nav_id.clone(),
                                layer: Some(nav_num),
                            },
                            cx,
                        );
                    }))
                    .child(format!("{prefix}{} · {}", layer.number, layer.title)),
            );
        }
        let base_branch = active_layer
            .map(|l| l.base_branch.as_str())
            .filter(|b| !b.is_empty())
            .unwrap_or("main");
        chain = chain.child(crumbs).child(
            div()
                .text_xs()
                .font_family("JetBrains Mono")
                .text_color(rgb(TEXT_FAINT))
                .child(format!("Base: {base_branch}")),
        );

        let mut diff_area = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .overflow_hidden();

        if self.show_details_card {
            diff_area = diff_area.child(
                div()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(BG_SURFACE))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT_PRIMARY))
                            .child(format!("What: {}", detail.summary_what)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(format!("Why: {}", detail.summary_why)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(TEXT_FAINT))
                            .child(detail.summary_details.clone()),
                    ),
            );
        }

        // Active file diff
        let active_file: Option<&ReviewDiffFile> = active_layer.and_then(|l| {
            self.selected_file_path
                .as_deref()
                .and_then(|p| l.files.iter().find(|f| f.path == p))
                .or_else(|| l.files.first())
        });

        if let Some(file) = active_file {
            let file_header = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px_3()
                .py_2()
                .rounded_t_md()
                .bg(rgb(BG_SURFACE))
                .border_1()
                .border_color(rgb(BORDER))
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
                                .text_color(rgb(TEXT_PRIMARY))
                                .child(file.path.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("JetBrains Mono")
                                .text_color(rgb(GREEN))
                                .child(format!("+{}", file.added)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("JetBrains Mono")
                                .text_color(rgb(RED))
                                .child(format!("-{}", file.removed)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("layout-unified")
                                .px_2()
                                .py(px(2.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(if self.diff_layout == DiffLayout::Unified {
                                    BG_PILL_ACTIVE
                                } else {
                                    BG_PILL
                                }))
                                .text_xs()
                                .text_color(rgb(TEXT_PRIMARY))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.diff_layout = DiffLayout::Unified;
                                    cx.notify();
                                }))
                                .child("Unified"),
                        )
                        .child(
                            div()
                                .id("layout-split")
                                .px_2()
                                .py(px(2.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(if self.diff_layout == DiffLayout::Split {
                                    BG_PILL_ACTIVE
                                } else {
                                    BG_PILL
                                }))
                                .text_xs()
                                .text_color(rgb(TEXT_PRIMARY))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.diff_layout = DiffLayout::Split;
                                    cx.notify();
                                }))
                                .child("Split"),
                        )
                        .child(
                            div()
                                .px_2()
                                .py(px(2.0))
                                .rounded_sm()
                                .bg(rgb(BG_PILL))
                                .text_xs()
                                .text_color(rgb(TEXT_MUTED))
                                .child("Open file ↗"),
                        ),
                );

            let mut lines_box = div()
                .flex()
                .flex_col()
                .border_x_1()
                .border_b_1()
                .border_color(rgb(BORDER))
                .rounded_b_md()
                .bg(rgb(BG_APP));

            if file.lines.is_empty() {
                lines_box = lines_box.child(
                    div()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(TEXT_FAINT))
                        .child("No diff hunks loaded for this file."),
                );
            } else {
                for line in &file.lines {
                    let (bg, fg) = match line.kind.as_str() {
                        "add" => (BG_DIFF_ADD, GREEN),
                        "del" => (BG_DIFF_DEL, RED),
                        _ => (BG_APP, TEXT_PRIMARY),
                    };
                    lines_box = lines_box.child(
                        div()
                            .px_3()
                            .py(px(2.0))
                            .bg(rgb(bg))
                            .text_xs()
                            .font_family("JetBrains Mono")
                            .text_color(rgb(fg))
                            .child(line.text.clone()),
                    );
                }
            }

            for comment in &file.comments {
                lines_box = lines_box.child(
                    div()
                        .m_2()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(BG_SURFACE))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(TEXT_PRIMARY))
                                        .child(comment.author.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(TEXT_FAINT))
                                        .child(comment.age.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(TEXT_MUTED))
                                .child(comment.body.clone()),
                        ),
                );
            }

            diff_area = diff_area.child(div().flex().flex_col().child(file_header).child(lines_box));
        }

        // Footer: Prev/Next layer + Request changes + Layer looks good + Push N PRs…
        let prev_layer = if active_layer_idx > 0 {
            detail.layers.get(active_layer_idx - 1).map(|l| l.number)
        } else {
            None
        };
        let next_layer = detail.layers.get(active_layer_idx + 1).map(|l| l.number);
        let prev_id = review_id.clone();
        let next_id = review_id;

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(rgb(BORDER))
            .bg(rgb(BG_SURFACE))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("prev-layer-btn")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(BG_PILL))
                            .text_xs()
                            .text_color(rgb(if prev_layer.is_some() {
                                TEXT_PRIMARY
                            } else {
                                TEXT_FAINT
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(n) = prev_layer {
                                    this.navigate(
                                        WebRoute::ReviewDetail {
                                            id: prev_id.clone(),
                                            layer: Some(n),
                                        },
                                        cx,
                                    );
                                }
                            }))
                            .child("‹ Prev layer"),
                    )
                    .child(
                        div()
                            .id("next-layer-btn")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgb(BG_PILL))
                            .text_xs()
                            .text_color(rgb(if next_layer.is_some() {
                                TEXT_PRIMARY
                            } else {
                                TEXT_FAINT
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(n) = next_layer {
                                    this.navigate(
                                        WebRoute::ReviewDetail {
                                            id: next_id.clone(),
                                            layer: Some(n),
                                        },
                                        cx,
                                    );
                                }
                            }))
                            .child("Next layer ›"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_PILL))
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child("Request changes…"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_PILL))
                            .text_xs()
                            .text_color(rgb(GREEN))
                            .child("Layer looks good"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_PILL_ACTIVE))
                            .text_xs()
                            .text_color(rgb(TEXT_PRIMARY))
                            .child(format!("Push {} PRs…", detail.pr_count.max(1))),
                    ),
            );

        main = main.child(chain).child(diff_area).child(footer);

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .child(header)
            .child(div().flex_1().flex().flex_row().min_h_0().child(rail).child(main))
            .into_any_element()
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
        let open_count = self
            .items
            .iter()
            .filter(|t| !matches!(t.state, State::Closed | State::WontDo | State::Became))
            .count();

        let top_bar = self.render_top_bar(open_count, cx);

        let body = match self.route.clone() {
            WebRoute::Todo => self.render_todo_body(cx).into_any_element(),
            WebRoute::ReviewsInbox => self.render_reviews_inbox(cx).into_any_element(),
            WebRoute::ReviewDetail { id, layer } => {
                self.render_review_detail(&id, layer, cx).into_any_element()
            }
        };

        div()
            .size_full()
            .bg(rgb(BG_APP))
            .text_color(rgb(TEXT_PRIMARY))
            .font_family("Inter")
            .flex()
            .flex_col()
            .child(top_bar)
            .child(body)
    }
}
