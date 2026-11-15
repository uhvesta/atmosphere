use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    resizable::{ResizableState, h_resizable, resizable_panel},
};
use gpui_kit::{
    assets::IconName,
    base::{Disableable, Selectable, TestSupportExt},
};
use gpui_kit::{prelude::*, *};
type ObservedDiv = gpui_kit::base::ObservedElement<Stateful<Div>>;
use crate::board::{Board, BoardError, Status};

#[derive(Clone)]
struct BoardDrag {
    item: usize,
    title: String,
}
impl Render for BoardDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(180.))
            .p_3()
            .rounded_md()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(ACCENT))
            .text_color(rgb(TEXT))
            .text_size(px(12.))
            .child(self.title.clone())
    }
}

const BG: u32 = 0x17181b;
const SIDE: u32 = 0x131416;
const PANEL: u32 = 0x1c1d21;
const LINE: u32 = 0x2b2d32;
const TEXT: u32 = 0xdedfe3;
const DIM: u32 = 0x898c96;
const ACCENT: u32 = 0x9caeff;
const GREEN: u32 = 0x91c7a3;
const AMBER: u32 = 0xe3b87c;
const CHANNELS: [&str; 5] = [
    "payments-rewrite",
    "web-app",
    "infra",
    "model-training",
    "scratch",
];

struct Session {
    title: String,
    channel: usize,
    agent: &'static str,
    machine: &'static str,
    status: &'static str,
    depth: usize,
    parent: Option<usize>,
    fork_kind: &'static str,
    archived: bool,
    view: SessionView,
}

#[derive(Clone)]
struct SessionView {
    review: bool,
    tab: &'static str,
    terminal: &'static str,
    full: bool,
}
impl Default for SessionView {
    fn default() -> Self {
        Self {
            review: false,
            tab: "Diff",
            terminal: "Agent",
            full: false,
        }
    }
}
struct Request {
    session: usize,
    title: &'static str,
    reason: &'static str,
    change: &'static str,
    state: u8,
}

pub(crate) struct Prototype {
    sessions: Vec<Session>,
    active: usize,
    page: &'static str,
    review_tab: &'static str,
    terminal_tab: &'static str,
    folded: [bool; 5],
    sidebar: bool,
    review: bool,
    wrap: bool,
    requests: Vec<Request>,
    request_selected: usize,
    show_answered: bool,
    comments: Vec<String>,
    sent_comments: bool,
    comment_open: bool,
    modal: Option<&'static str>,
    terminal_input: Entity<InputState>,
    name_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    search_input: Entity<InputState>,
    terminal_lines: Vec<(usize, String)>,
    selected_file: usize,
    board: Board,
    item_open: Option<usize>,
    item_tab: &'static str,
    item_title: Entity<InputState>,
    item_details: Entity<TextareaState>,
    item_comment: Entity<TextareaState>,
    blocker_input: Entity<InputState>,
    blocking_item: Option<usize>,
    draft_blockers: Vec<usize>,
    board_error: Option<BoardError>,
    _subscriptions: Vec<Subscription>,
    channel: usize,
    channel_tab: &'static str,
    review_full: bool,
    fork_kind: &'static str,
    modal_agent: &'static str,
    modal_machine: &'static str,
    show_forks: bool,
    offered_view: &'static str,
    splits: std::collections::HashMap<usize, Entity<ResizableState>>,
}

fn row() -> Div {
    div().flex().items_center()
}
fn col() -> Div {
    div().flex().flex_col()
}
fn muted(text: impl Into<SharedString>) -> Div {
    div().text_color(rgb(DIM)).child(text.into())
}
fn icon(name: IconName) -> Icon {
    Icon::new(name).size(px(15.))
}
fn separator() -> Div {
    div().w(px(1.)).h(px(15.)).bg(rgb(LINE)).mx_2()
}
fn dot(color: u32) -> Div {
    div()
        .size(px(6.))
        .rounded_full()
        .bg(rgb(color))
        .flex_shrink_0()
}
fn status_color(status: &str) -> u32 {
    match status {
        "Working" => GREEN,
        "Needs you" => AMBER,
        "Disconnected" => 0xd68282,
        _ => 0x626570,
    }
}
fn pill(text: impl Into<SharedString>) -> Div {
    div()
        .px_2()
        .py(px(2.))
        .rounded(px(4.))
        .bg(rgb(0x24262b))
        .text_size(px(10.))
        .text_color(rgb(DIM))
        .child(text.into())
}
fn navigation(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> ObservedDiv {
    let label = label.into();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(29.))
        .w_full()
        .min_w_0()
        .px_2()
        .gap_2()
        .rounded(px(5.))
        .cursor_pointer()
        .bg(rgb(if selected { 0x292d39 } else { SIDE }))
        .text_color(rgb(if selected { 0xefeff4 } else { 0xa5a8b2 }))
        .hover(|d| d.bg(rgb(if selected { 0x303645 } else { 0x202227 })))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .test_support()
}

impl Prototype {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Message the agent…"));
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("What are we working on?"));
        let comment_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Leave a comment for the agent…"));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find a session…"));
        let item_title = cx.new(|cx| InputState::new(window, cx).placeholder("Item title"));
        let item_details = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe the work, acceptance criteria or context…")
        });
        let item_comment =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Add a comment…"));
        let blocker_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("e.g. Waiting for staging credentials")
        });
        let send = cx.subscribe_in(&terminal_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.send_message(window, cx);
            }
        });
        let search = cx.subscribe(&search_input, |_, _, _: &InputEvent, cx| cx.notify());
        let data = [
            (
                "Ledger schema migration",
                0,
                "Claude Code",
                "This Mac",
                "Working",
                0,
            ),
            (
                "Try Postgres partitioning",
                0,
                "Claude Code",
                "This Mac",
                "Needs you",
                1,
            ),
            ("Review the migration", 0, "Codex", "This Mac", "Idle", 1),
            ("API rate limiter", 0, "Codex", "devbox-1", "Working", 0),
            ("Flaky test hunt", 0, "Copilot", "This Mac", "Idle", 0),
            ("Dashboard cards", 1, "OpenCode", "This Mac", "Needs you", 0),
            (
                "Spike: card layout",
                1,
                "Claude Code",
                "This Mac",
                "Idle",
                1,
            ),
            (
                "Runner autoscaling",
                2,
                "Claude Code",
                "devbox-2",
                "Disconnected",
                0,
            ),
            (
                "Train ranking model v3",
                3,
                "Claude Code",
                "gpu-box",
                "Working",
                0,
            ),
        ];
        Self {
            sessions: data
                .into_iter()
                .enumerate()
                .map(
                    |(i, (title, channel, agent, machine, status, depth))| Session {
                        title: title.into(),
                        channel,
                        agent,
                        machine,
                        status,
                        depth,
                        parent: match i {
                            1 | 2 => Some(0),
                            6 => Some(5),
                            _ => None,
                        },
                        fork_kind: match i {
                            1 => "Code + context",
                            2 => "Context only",
                            6 => "Code only",
                            _ => "",
                        },
                        archived: false,
                        view: SessionView::default(),
                    },
                )
                .collect(),
            active: 0,
            page: "session",
            review_tab: "Diff",
            terminal_tab: "Agent",
            folded: [false, false, true, true, true],
            sidebar: true,
            review: false,
            wrap: true,
            requests: vec![
                Request {
                    session: 0,
                    title: "Move PAY-3: Design the ledger schema → Done",
                    reason: "PAY-3 was created by you, not by this agent.",
                    change: "Status: In review → Done. Migration 0042 is ready and all 14 ledger tests pass.",
                    state: 0,
                },
                Request {
                    session: 0,
                    title: "Edit S1-3: Add property test for balance",
                    reason: "You created the item on this agent’s session board.",
                    change: "Rename to: Add proptest: sum(postings) == 0",
                    state: 0,
                },
                Request {
                    session: 3,
                    title: "Read files of Ledger schema migration on This Mac",
                    reason: "Those files belong to another session, on another machine.",
                    change: "Read only, for this turn: ledger/migrations/0042_accounts.sql and api/src/ledger/client.rs",
                    state: 0,
                },
                Request {
                    session: 1,
                    title: "Create Postgres 17 partitioning guide on the Global board",
                    reason: "Agents need your approval to add to the Global board.",
                    change: "New global item · Backlog · From notes/partitioning.md",
                    state: 0,
                },
                Request {
                    session: 5,
                    title: "Type into the terminal of Spike: card layout",
                    reason: "Controlling another session’s terminal needs you.",
                    change: "Input: use the CardGrid from design-system v4, not v3. This cannot be unread once sent.",
                    state: 0,
                },
                Request {
                    session: 1,
                    title: "Start a code-only fork on devbox-1",
                    reason: "Starting sessions creates clones and uses a new agent.",
                    change: "Codex · devbox-1 · Copy code only · acme/api · About 300 MB",
                    state: 0,
                },
            ],
            request_selected: 0,
            show_answered: false,
            comments: vec![],
            sent_comments: false,
            comment_open: false,
            modal: None,
            terminal_input,
            name_input,
            comment_input,
            search_input,
            terminal_lines: vec![],
            selected_file: 0,
            board: Board::default(),
            item_open: None,
            item_tab: "Details",
            item_title,
            item_details,
            item_comment,
            blocker_input,
            blocking_item: None,
            draft_blockers: vec![],
            board_error: None,
            _subscriptions: vec![send, search],
            channel: 0,
            channel_tab: "Sessions",
            review_full: false,
            fork_kind: "Code + context",
            modal_agent: "Claude Code",
            modal_machine: "This Mac",
            show_forks: true,
            offered_view: "new",
            splits: (0..9)
                .map(|i| (i, cx.new(|_| ResizableState::default())))
                .collect(),
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.remember_view();
        self.active = index;
        self.channel = self.sessions[index].channel;
        self.page = "session";
        self.splits
            .entry(index)
            .or_insert_with(|| cx.new(|_| ResizableState::default()));
        let view = &self.sessions[index].view;
        self.review = view.review;
        self.review_tab = view.tab;
        self.terminal_tab = view.terminal;
        self.review_full = view.full;
        cx.notify();
    }

    fn remember_view(&mut self) {
        self.sessions[self.active].view = SessionView {
            review: self.review,
            tab: self.review_tab,
            terminal: self.terminal_tab,
            full: self.review_full,
        };
    }

    fn panel(&mut self, name: &'static str, cx: &mut Context<Self>) {
        self.review = !(self.review && self.review_tab == name);
        self.review_tab = name;
        self.review_full = false;
        self.remember_view();
        cx.notify();
    }

    fn channel_select(&mut self, channel: usize, cx: &mut Context<Self>) {
        self.remember_view();
        self.channel = channel;
        self.page = "channel";
        self.channel_tab = "Sessions";
        cx.notify();
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.terminal_input.read(cx).value().to_string();
        if value.trim().is_empty() {
            return;
        }
        self.terminal_lines.push((self.active, value));
        self.terminal_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn nav_button(
        &self,
        label: &'static str,
        name: IconName,
        page: &'static str,
        cx: &mut Context<Self>,
    ) -> ObservedDiv {
        row()
            .id(label)
            .role(Role::Button)
            .aria_label(label)
            .h(px(30.))
            .w_full()
            .px_2()
            .gap_2()
            .rounded(px(5.))
            .bg(rgb(if self.page == page { 0x292d39 } else { SIDE }))
            .cursor_pointer()
            .hover(|d| d.bg(rgb(0x25282f)))
            .child(icon(name))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.remember_view();
                this.page = page;
                cx.notify();
            }))
            .test_support()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let mut tree = col().gap(px(2.));
        for (channel, title) in CHANNELS.iter().enumerate() {
            let opened = !self.folded[channel];
            tree = tree.child(
                row()
                    .gap_1()
                    .child(
                        Button::new(("fold-channel", channel))
                            .ghost()
                            .xsmall()
                            .icon(if opened {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.folded[channel] = !this.folded[channel];
                                cx.notify();
                            })),
                    )
                    .child(
                        navigation(
                            ("channel", channel),
                            format!("# {title}"),
                            self.page == "channel" && self.channel == channel,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.channel_select(channel, cx)),
                        ),
                    ),
            );
            if opened {
                for (index, session) in self
                    .sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.channel == channel && !s.archived)
                {
                    tree = tree.child(
                        row()
                            .pl(px(19. + session.depth as f32 * 12.))
                            .pr_1()
                            .gap_2()
                            .child(dot(status_color(session.status)))
                            .child(
                                navigation(
                                    ("session", index),
                                    session.title.clone(),
                                    self.page == "session" && self.active == index,
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.select(index, cx)),
                                ),
                            ),
                    );
                }
            }
        }
        col()
            .size_full()
            .bg(rgb(SIDE))
            .border_r_1()
            .border_color(rgb(LINE))
            .child(
                row()
                    .h(px(49.))
                    .px_4()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Humata"))
                    .child(pill("acme"))
                    .child(div().flex_1())
                    .child(
                        Button::new("collapse-sidebar")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelLeft)
                            .tooltip("Hide sidebar")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                col()
                    .px_2()
                    .gap_1()
                    .child(self.nav_button("Home", IconName::House, "Home", cx))
                    .child(self.nav_button("Inbox", IconName::Inbox, "Inbox", cx))
                    .child(self.nav_button("Board", IconName::LayoutDashboard, "Board", cx)),
            )
            .child(
                row()
                    .px_4()
                    .pt_5()
                    .pb_2()
                    .text_size(px(10.))
                    .text_color(rgb(DIM))
                    .child("PINNED")
                    .child(div().flex_1())
                    .child(icon(IconName::Pin)),
            )
            .child(
                row().px_3().gap_2().child(dot(GREEN)).child(
                    navigation("pin-ledger", "Ledger schema migration", false)
                        .on_click(cx.listener(|this, _, _, cx| this.select(0, cx))),
                ),
            )
            .child(
                row()
                    .px_4()
                    .pt_5()
                    .pb_2()
                    .text_size(px(10.))
                    .text_color(rgb(DIM))
                    .child("CHANNELS")
                    .child(div().flex_1())
                    .child(
                        Button::new("fold-all")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronsUpDown)
                            .tooltip("Expand / collapse channels")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let fold = !this.folded.iter().all(|v| *v);
                                this.folded = [fold; 5];
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("add-session")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("New session")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = Some("New session");
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("sidebar-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .child(tree),
            )
            .child(
                col()
                    .px_3()
                    .pb_3()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(self.nav_button("Machines", IconName::Server, "Machines", cx))
                    .child(self.nav_button("Settings", IconName::Settings, "Settings", cx))
                    .child(
                        row()
                            .px_2()
                            .pt_3()
                            .gap_2()
                            .child(dot(GREEN))
                            .child(muted("Local UI study").text_size(px(10.))),
                    ),
            )
    }

    fn workspace(&self, cx: &mut Context<Self>) -> Div {
        let s = &self.sessions[self.active];
        // Full-width rendering never changes the split's two slots or sizes.
        let content = if !self.review {
            self.terminal(cx).into_any_element()
        } else if self.review_full {
            self.review(cx).into_any_element()
        } else {
            h_resizable(("terminal-review", self.active))
                .with_state(&self.splits[&self.active])
                .child(
                    resizable_panel()
                        .size(px(570.))
                        .size_range(px(200.)..px(2400.))
                        .child(self.terminal(cx)),
                )
                .child(
                    resizable_panel()
                        .size(px(570.))
                        .size_range(px(200.)..px(2400.))
                        .child(self.review(cx)),
                )
                .into_any_element()
        };
        col()
            .size_full()
            .child(
                row()
                    .px_5()
                    .h(px(58.))
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("session-channel")
                            .ghost()
                            .xsmall()
                            .label(format!("# {}", CHANNELS[s.channel]))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.channel_select(this.sessions[this.active].channel, cx)
                            })),
                    )
                    .when_some(s.parent, |d, parent| {
                        d.child(
                            Button::new("parent-session")
                                .ghost()
                                .xsmall()
                                .icon(IconName::GitFork)
                                .label(self.sessions[parent].title.clone())
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.select(parent, cx)),
                                ),
                        )
                    })
                    .child(
                        col().gap_1().child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(15.))
                                .child(s.title.clone()),
                        ),
                    )
                    .child(div().flex_1())
                    .child(dot(status_color(s.status)))
                    .child(muted(s.status).text_size(px(11.)))
                    .child(separator())
                    .child(
                        Button::new("fork")
                            .ghost()
                            .small()
                            .icon(IconName::GitFork)
                            .label("Fork")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = Some("Fork session");
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("toggle-review")
                            .ghost()
                            .small()
                            .icon(IconName::PanelRight)
                            .tooltip("Toggle review pane")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review = !this.review;
                                this.review_full = false;
                                this.remember_view();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("session-menu")
                            .ghost()
                            .small()
                            .icon(IconName::Ellipsis)
                            .tooltip("Session actions")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = Some("Session actions");
                                cx.notify();
                            })),
                    ),
            )
            .when(s.status == "Disconnected", |d| {
                d.child(
                    row()
                        .px_4()
                        .py_2()
                        .gap_3()
                        .bg(rgb(0x2a231c))
                        .child(muted(format!(
                            "{} is disconnected. Reconnect when you’re ready to try again.",
                            s.machine
                        )))
                        .child(div().flex_1())
                        .child(
                            Button::new("reconnect")
                                .small()
                                .label("Reconnect")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reconnect_machine(this.sessions[this.active].machine, cx);
                                })),
                        ),
                )
            })
            .child(
                row()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().child(content))
                    .child(self.tool_strip(cx)),
            )
    }

    fn tool_strip(&self, cx: &mut Context<Self>) -> Div {
        let mut strip = col()
            .w(px(48.))
            .flex_shrink_0()
            .py_2()
            .gap_1()
            .bg(rgb(SIDE))
            .border_l_1()
            .border_color(rgb(LINE));
        for (name, image) in [
            ("Inbox", IconName::Inbox),
            ("Diff", IconName::Columns2),
            ("Files", IconName::Files),
            ("Board", IconName::LayoutDashboard),
            ("Journal", IconName::MessageSquare),
            ("Info", IconName::Circle),
        ] {
            strip = strip.child(
                col()
                    .items_center()
                    .child(
                        Button::new(SharedString::from(format!("tool-{name}")))
                            .ghost()
                            .small()
                            .icon(image)
                            .selected(self.review && self.review_tab == name)
                            .tooltip(name)
                            .on_click(cx.listener(move |this, _, _, cx| this.panel(name, cx))),
                    )
                    .child(muted(name).text_size(px(8.))),
            );
        }
        strip
    }

    fn terminal(&self, cx: &mut Context<Self>) -> ObservedDiv {
        let s = &self.sessions[self.active];
        let mut bar = row()
            .h(px(37.))
            .px_3()
            .gap_1()
            .border_b_1()
            .border_color(rgb(LINE));
        for label in ["Agent", "Shell"] {
            bar = bar.child(
                Button::new(SharedString::from(format!("terminal-tab-{label}")))
                    .ghost()
                    .small()
                    .icon(IconName::Terminal)
                    .label(label)
                    .selected(self.terminal_tab == label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.terminal_tab = label;
                        this.remember_view();
                        cx.notify();
                    })),
            );
        }
        bar = bar
            .child(div().flex_1())
            .child(muted(s.machine).text_size(px(10.)));
        let mut transcript = col()
            .gap_4()
            .text_size(px(12.))
            .line_height(px(21.))
            .font_family("JetBrains Mono");
        if self.terminal_tab == "Shell" {
            transcript = transcript
                .child(muted("you@devbox ~/Humata/payments-rewrite"))
                .child("$ git status --short")
                .child(div().text_color(rgb(AMBER)).child(
                    " M migrations/004_ledger.sql\n M src/ledger.rs\n M specs/billing-v2/SPEC.md",
                ))
                .child(muted("Simulated shell. No commands are executed."));
        } else if self.active != 0 {
            let body = match self.active {
                1 => {
                    "› Try range-partitioning postings by month.\n\n● Writing ledger/migrations/0043_partition_postings.sql +22\n● humata.board.create PAY-9 → allowed (channel board)\n\nPartitioning by month makes the backfill 4× faster in the benchmark.\n\nContinue with the backfill script? (y/n)"
                }
                2 => {
                    "› Review what you just wrote as if a stranger wrote it.\n\n● Reading ledger/migrations/0042_accounts.sql\n\nTwo findings: postings has no index on account_id; currency is not checked against accounts.currency.\n\nIdle. Type to continue."
                }
                3 => {
                    "› Rate-limit the billing API (PAY-5).\n\n● Writing api/src/middleware/rate_limit.rs +96\n● humata.files.read This Mac: Ledger schema migration\n  Waiting for your approval in the Inbox.\n\nShould the limiter key on API key or on account id?"
                }
                4 => {
                    "› Find why ledger_test::concurrent_post flakes.\n\nRan the test 200 times: 3 failures, all on a deadlock between two postings.\n\n[process exited — its conversation and scrollback are kept]"
                }
                5 => {
                    "› Build the new dashboard cards (WEB-2).\n\n● Editing web/src/dashboard/Cards.tsx\n\nCard grid: 3 or 4 columns on desktop?"
                }
                6 => {
                    "› A code-only fork: fresh agent, copied files.\n\nTrying a different layout for the dashboard cards."
                }
                7 => {
                    "› Autoscale the CI runners.\n\n● Reading infra/runners.tf\n\n[devbox-2 disconnected. Choose Reconnect to try again.]"
                }
                8 => {
                    "› Train ranking model v3 (ML-1).\n\n● python -m train.ranking_v3 --config train/config.yaml\n  epoch 1  loss 0.301  ndcg@10 0.371\n  epoch 2  loss 0.247  ndcg@10 0.398\n  epoch 3  loss 0.214  ndcg@10 0.409\n\nWaiting for clicks-2026-10.parquet in shared/data/ …"
                }
                _ => "New session. The simulated agent is waiting for your first instruction.",
            };
            transcript = transcript
                .child(muted(format!("{} · {} · {}", s.agent, s.title, s.machine)))
                .when(s.parent.is_some(), |d| {
                    d.child(muted(format!(
                        "Fork: {}{}",
                        s.fork_kind,
                        if s.fork_kind == "Context only" {
                            " · parent’s files are read-only"
                        } else {
                            ""
                        }
                    )))
                })
                .child(body);
        } else {
            transcript = transcript.child(row().gap_2().child(div().text_color(rgb(AMBER)).child("✳"))
                    .child(div().font_weight(FontWeight::MEDIUM).text_color(rgb(TEXT)).child(s.agent)))
                .child(muted("~/Humata/payments-rewrite/sessions/ledger-schema"))
                .child(div().child(if self.active == 0 {
                    "Migrate billing onto the double-entry ledger. Keep the existing API compatible and add a rollback path."
                } else { "Explore this approach independently. Show me the changes before we move ahead." }))
                .child("I’ll add the ledger tables first, then wire the posting flow through the existing billing API.")
                .child(col().gap_1().child(muted("Read  src/billing/service.rs")).child(muted("Read  migrations/003_accounts.sql")).child(muted("Read  specs/billing-v2/SPEC.md")))
                .child("The current transaction boundary already gives us an atomic write. The migration can stay small and reversible.")
                .child(col().gap_1().child(div().text_color(rgb(ACCENT)).child("Edit  migrations/004_ledger.sql"))
                    .child(div().text_color(rgb(GREEN)).child("      + ledger_entries table"))
                    .child(div().text_color(rgb(GREEN)).child("      + debit / credit balance constraint"))
                    .child(div().text_color(rgb(GREEN)).child("      + rollback migration")))
                .child(muted("$ cargo test -p ledger --test posting"))
                .child(div().text_color(rgb(GREEN)).child("  ✓ balanced_entry_is_committed\n  ✓ unbalanced_entry_is_rejected\n  ✓ rollback_preserves_existing_invoices"))
                .child(muted("test result: ok. 3 passed; 0 failed"))
                .child(div().text_color(rgb(TEXT)).child("The migration and posting path are ready for review. I’m checking the compatibility edge cases next."));
        }
        for (_, line) in self
            .terminal_lines
            .iter()
            .filter(|(i, _)| *i == self.active)
        {
            transcript = transcript
                .child(div().child(format!("> {line}")))
                .child(muted(
                    "Message received in this prototype. No agent is connected.",
                ));
        }
        col()
            .id("terminal-pane")
            .role(Role::Group)
            .aria_label("Terminal")
            .size_full()
            .bg(rgb(0x151619))
            .child(bar)
            .child(
                div()
                    .id("terminal-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .child(transcript),
            )
            .child(
                col()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(
                        row()
                            .gap_2()
                            .child(div().font_family("JetBrains Mono").child(">"))
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.terminal_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .small(),
                                ),
                            )
                            .child(
                                Button::new("send-message")
                                    .small()
                                    .icon(IconName::ArrowUp)
                                    .tooltip("Send message · Enter")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_message(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        row()
                            .text_size(px(10.))
                            .child(muted("Simulated terminal"))
                            .child(div().flex_1())
                            .child(muted("Enter to send")),
                    ),
            )
            .test_support()
    }

    fn review(&self, cx: &mut Context<Self>) -> ObservedDiv {
        let tabs = row()
            .h(px(37.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(LINE))
            .child(self.review_tab)
            .child(div().flex_1())
            .child(
                Button::new("widen-review")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Maximize)
                    .tooltip("Widen / restore panel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.review_full = !this.review_full;
                        this.remember_view();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("close-review")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .tooltip("Close panel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.review = false;
                        this.review_full = false;
                        this.remember_view();
                        cx.notify();
                    })),
            );
        let body = match self.review_tab {
            "Files" => self.files(cx).into_any_element(),
            "Board" => self.board(cx).into_any_element(),
            "Inbox" => self.inbox(cx).into_any_element(),
            "Info" => self.info().into_any_element(),
            "Journal" => self.journal(cx).into_any_element(),
            _ => self.diff(cx).into_any_element(),
        };
        col()
            .id("review-pane")
            .role(Role::Group)
            .aria_label(self.review_tab)
            .size_full()
            .bg(rgb(BG))
            .border_l_1()
            .border_color(rgb(LINE))
            .child(tabs)
            .child(div().flex_1().min_h_0().child(body))
            .test_support()
    }

    fn diff(&self, cx: &mut Context<Self>) -> Div {
        let files = [
            ("acme/ledger", "migrations/004_ledger.sql", "+42", "−0"),
            ("acme/api", "src/billing/service.rs", "+18", "−6"),
            ("acme/docs", "specs/billing-v2/SPEC.md", "+12", "−2"),
        ];
        let mut list = col().gap_3();
        for (index, (repo, path, added, removed)) in files.into_iter().enumerate() {
            let mut file = col()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(LINE))
                .overflow_hidden()
                .child(
                    row()
                        .h(px(34.))
                        .px_3()
                        .bg(rgb(PANEL))
                        .gap_2()
                        .child(icon(IconName::File))
                        .child(div().text_size(px(11.)).child(path))
                        .child(div().flex_1())
                        .child(div().text_size(px(10.)).text_color(rgb(GREEN)).child(added))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(0xd68f91))
                                .child(removed),
                        ),
                );
            let code: Vec<(&str, &str)> = match index {
                0 => vec![
                    ("hunk", "@@ -0,0 +1,42 @@"),
                    ("add", "CREATE TABLE ledger_entries ("),
                    ("add", "    id          UUID PRIMARY KEY,"),
                    ("add", "    account_id  UUID NOT NULL,"),
                    ("add", "    debit       BIGINT DEFAULT 0,"),
                    ("add", "    credit      BIGINT DEFAULT 0,"),
                    ("add", "    created_at  TIMESTAMPTZ NOT NULL,"),
                    ("add", "    CONSTRAINT balanced_entry"),
                    ("add", "      CHECK (debit >= 0 AND credit >= 0)"),
                    ("add", ");"),
                    ("plain", ""),
                    ("add", "CREATE INDEX ledger_account_idx"),
                    ("add", "    ON ledger_entries (account_id);"),
                ],
                1 => vec![
                    ("hunk", "@@ -82,6 +82,18 @@"),
                    ("plain", "pub async fn post_invoice("),
                    ("plain", "    tx: &mut Transaction,"),
                    ("plain", "    invoice: &Invoice,"),
                    ("plain", ") -> Result<(), PostingError> {"),
                    ("del", "    save_invoice(tx, invoice).await?;"),
                    ("add", "    let entry = Entry::from(invoice);"),
                    ("add", "    ledger.post(tx, entry).await?;"),
                    ("add", "    save_invoice(tx, invoice).await?;"),
                    ("plain", "    Ok(())"),
                    ("plain", "}"),
                ],
                _ => vec![
                    ("hunk", "@@ -24,2 +24,12 @@"),
                    ("plain", "## Rollback"),
                    ("add", "Existing invoices remain readable."),
                    ("add", "Revert the posting path before dropping"),
                    ("add", "the new ledger tables."),
                ],
            };
            for (line, (kind, text)) in code.into_iter().enumerate() {
                let background = match kind {
                    "add" => 0x1a2923,
                    "del" => 0x302020,
                    "hunk" => 0x20222d,
                    _ => BG,
                };
                let color = match kind {
                    "add" => 0xb3d5bf,
                    "del" => 0xe1aaa9,
                    "hunk" => 0x969bb4,
                    _ => 0xaeb0b9,
                };
                file = file.child(
                    row()
                        .items_start()
                        .bg(rgb(background))
                        .min_h(px(20.))
                        .font_family("JetBrains Mono")
                        .text_size(px(10.5))
                        .child(
                            div()
                                .w(px(35.))
                                .flex_shrink_0()
                                .pr_2()
                                .text_right()
                                .text_color(rgb(0x606b65))
                                .child(format!("{}", line + 1)),
                        )
                        .child(
                            div()
                                .w(px(17.))
                                .flex_shrink_0()
                                .text_color(rgb(color))
                                .child(if kind == "add" {
                                    "+"
                                } else if kind == "del" {
                                    "−"
                                } else {
                                    " "
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pr_2()
                                .text_color(rgb(color))
                                .when(!self.wrap, |d| d.whitespace_nowrap())
                                .child(text),
                        ),
                );
            }
            file = file.child(
                row()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new(("comment", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::MessageSquare)
                            .label("Comment")
                            .text_size(px(10.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.comment_open = true;
                                this.selected_file = index;
                                cx.notify();
                            })),
                    ),
            );
            list = list
                .child(muted(repo).text_size(px(10.)).px_1())
                .child(file);
        }
        if self.comment_open {
            list = list.child(
                col()
                    .p_3()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0x46445a))
                    .child(
                        muted(format!("Comment on {}", files[self.selected_file].1))
                            .text_size(px(11.)),
                    )
                    .child(Input::new(&self.comment_input).small())
                    .child(
                        Button::new("save-comment")
                            .small()
                            .label("Add draft comment")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let text = this.comment_input.read(cx).value().to_string();
                                if !text.trim().is_empty() {
                                    this.comments.push(text);
                                    this.comment_open = false;
                                    this.sent_comments = false;
                                    this.comment_input
                                        .update(cx, |i, cx| i.set_value("", window, cx));
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        for comment in &self.comments {
            list = list.child(
                col()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(LINE))
                    .child(
                        muted(if self.sent_comments {
                            "You · Sent to terminal"
                        } else {
                            "You · Draft"
                        })
                        .text_size(px(10.)),
                    )
                    .child(comment.clone()),
            );
        }
        col()
            .size_full()
            .child(
                row()
                    .h(px(42.))
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(12.))
                            .child("3 files changed"),
                    )
                    .child(muted("in 3 repositories").text_size(px(10.)))
                    .child(div().flex_1())
                    .child(
                        Button::new("wrap")
                            .ghost()
                            .xsmall()
                            .label("Wrap")
                            .selected(self.wrap)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.wrap = !this.wrap;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("diff-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .child(list),
            )
            .child(
                row()
                    .h(px(44.))
                    .px_3()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .gap_2()
                    .child(
                        muted(format!(
                            "{} draft comments",
                            if self.sent_comments {
                                0
                            } else {
                                self.comments.len()
                            }
                        ))
                        .text_size(px(10.)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("send-comments")
                            .small()
                            .label("Send comments")
                            .disabled(self.comments.is_empty() || self.sent_comments)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.terminal_lines.push((
                                    this.active,
                                    format!("Review comments:\n{}", this.comments.join("\n")),
                                ));
                                this.sent_comments = true;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn inbox(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut content = col()
            .id("inbox-scroll")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(
                row()
                    .gap_2()
                    .child(div().font_weight(FontWeight::MEDIUM).child("Inbox"))
                    .child(div().flex_1())
                    .when(self.page == "session", |d| {
                        d.child(
                            Button::new("include-forks")
                                .ghost()
                                .xsmall()
                                .label("Include forks")
                                .selected(self.show_forks)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_forks = !this.show_forks;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("answered")
                            .ghost()
                            .xsmall()
                            .label("Show answered")
                            .selected(self.show_answered)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_answered = !this.show_answered;
                                cx.notify();
                            })),
                    ),
            );
        let visible: Vec<usize> = self
            .requests
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let in_scope = match self.page {
                    "session" => {
                        r.session == self.active
                            || (self.show_forks && self.is_descendant(r.session, self.active))
                    }
                    "channel" => self.sessions[r.session].channel == self.channel,
                    _ => true,
                };
                in_scope && (self.show_answered || r.state == 0)
            })
            .map(|(i, _)| i)
            .collect();
        if visible.is_empty() {
            return content.child(muted("Nothing waiting in this scope."));
        }
        let selected = if visible.contains(&self.request_selected) {
            self.request_selected
        } else {
            visible[0]
        };
        for index in visible {
            let request = &self.requests[index];
            content = content.child(
                navigation(("request", index), request.title, index == selected).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.request_selected = index;
                        cx.notify();
                    }),
                ),
            );
            if index == selected {
                let session = &self.sessions[request.session];
                let sid = request.session;
                content = content.child(
                    col()
                        .p_4()
                        .gap_3()
                        .border_1()
                        .border_color(rgb(LINE))
                        .rounded_md()
                        .child(
                            row()
                                .gap_2()
                                .child(dot(status_color(session.status)))
                                .child(session.title.clone()),
                        )
                        .child(
                            muted(format!(
                                "{} · {} · #{}",
                                session.agent, session.machine, CHANNELS[session.channel]
                            ))
                            .text_size(px(10.)),
                        )
                        .child(div().font_weight(FontWeight::MEDIUM).child(request.title))
                        .child(muted("WHAT CHANGES").text_size(px(10.)))
                        .child(request.change)
                        .child(muted("WHY IT NEEDS YOU").text_size(px(10.)))
                        .child(request.reason)
                        .child(
                            Button::new(("request-session", index))
                                .ghost()
                                .small()
                                .label("Open its session ↗")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(sid, cx);
                                    this.review = true;
                                    this.review_tab = "Inbox";
                                    this.request_selected = index;
                                    this.remember_view();
                                    cx.notify();
                                })),
                        )
                        .when(request.state == 0, |d| {
                            d.child(
                                row()
                                    .gap_2()
                                    .child(
                                        Button::new(("approve", index))
                                            .primary()
                                            .small()
                                            .label("Allow once")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.answer_request(index, true);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("deny", index))
                                            .small()
                                            .label("Deny")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.answer_request(index, false);
                                                cx.notify();
                                            })),
                                    ),
                            )
                        })
                        .when(request.state != 0, |d| {
                            d.child(pill(if request.state == 1 {
                                "Approved"
                            } else {
                                "Denied"
                            }))
                        }),
                );
            }
        }
        content
    }

    fn is_descendant(&self, mut index: usize, parent: usize) -> bool {
        while let Some(p) = self.sessions[index].parent {
            if p == parent {
                return true;
            }
            index = p;
        }
        false
    }

    fn answer_request(&mut self, index: usize, allow: bool) {
        self.requests[index].state = if allow { 1 } else { 2 };
        if !allow {
            return;
        }
        match index {
            0 => self.board.move_item(2, Status::Done),
            4 => self.terminal_lines.push((
                6,
                "From Dashboard cards: use the CardGrid from design-system v4, not v3".into(),
            )),
            5 => self.sessions.push(Session {
                title: "Try hash partitioning".into(),
                channel: 0,
                agent: "Codex",
                machine: "devbox-1",
                status: "Working",
                depth: 2,
                parent: Some(1),
                fork_kind: "Code only",
                archived: false,
                view: SessionView::default(),
            }),
            _ => {}
        }
    }

    fn board(&self, cx: &mut Context<Self>) -> Div {
        let visible: Vec<usize> = (0..5)
            .filter(|i| match self.page {
                "session" => self.active == 0 && *i >= 3,
                "channel" => self.channel == 0 && (*i == 1 || *i == 2),
                _ => *i < 3,
            })
            .collect();
        let mut columns = row().items_stretch().min_h_full().gap_3().p_4();
        for status in Status::COLUMNS {
            let color = match status {
                Status::Done => GREEN,
                Status::Blocked => 0xd68f91,
                Status::WontDo => DIM,
                Status::InReview => 0xb69fd6,
                Status::InProgress => AMBER,
                Status::Todo => ACCENT,
                Status::Backlog => 0x777985,
            };
            let mut column = col()
                .id(SharedString::from(format!("board-column-{}", status.key())))
                .role(Role::Group)
                .aria_label(status.label())
                .flex_1()
                .min_w(px(168.))
                .min_h(px(380.))
                .gap_3()
                .p_2()
                .rounded_md()
                .child(
                    row()
                        .gap_2()
                        .child(dot(color))
                        .child(status.label())
                        .child(pill(format!(
                            "{}",
                            visible
                                .iter()
                                .filter(|i| self.board.items[**i].status == status)
                                .count()
                        ))),
                )
                .drag_over::<BoardDrag>(|style, _, _, _| {
                    style.bg(rgb(0x232734)).border_1().border_color(rgb(ACCENT))
                })
                .on_drop(cx.listener(move |this, drag: &BoardDrag, window, cx| {
                    this.move_board_item(drag.item, status, window, cx)
                }));
            for index in visible
                .iter()
                .copied()
                .filter(|i| self.board.items[*i].status == status)
            {
                let item = &self.board.items[index];
                let mut card = col()
                    .id(("board-card", index))
                    .role(Role::Button)
                    .aria_label(format!("{} {}", item.id, item.title))
                    .p_3()
                    .gap_3()
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(|d| d.border_color(rgb(0x565b6b)))
                    .child(muted(item.id).text_size(px(10.)))
                    .child(item.title.clone())
                    .on_drag(
                        BoardDrag {
                            item: index,
                            title: item.title.clone(),
                        },
                        |drag, _, _, cx| cx.new(|_| drag.clone()),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_item(index, window, cx)),
                    );
                if status == Status::Blocked {
                    let ids: Vec<_> = item
                        .blocked_on
                        .iter()
                        .map(|i| self.board.items[*i].id)
                        .collect();
                    card = card.child(
                        col()
                            .gap_1()
                            .text_size(px(10.))
                            .text_color(rgb(AMBER))
                            .when(!ids.is_empty(), |d| {
                                d.child(format!("Blocked on {}", ids.join(", ")))
                            })
                            .when(!item.blocker_note.is_empty(), |d| {
                                d.child(item.blocker_note.clone())
                            }),
                    );
                }
                card = card.child(
                    row()
                        .child(pill(if index == 2 { "Claude Code" } else { "You" }))
                        .child(div().flex_1())
                        .child(
                            Button::new(("move-card", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Ellipsis)
                                .tooltip("Change status")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.blocking_item = Some(index);
                                    this.modal = Some("Move item");
                                    cx.notify();
                                })),
                        ),
                );
                column = column.child(card.test_support());
            }
            columns = columns.child(column.test_support());
        }
        col()
            .size_full()
            .child(
                row()
                    .h(px(52.))
                    .px_5()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(match self.page {
                                "session" => "Session board",
                                "channel" => "Channel board",
                                _ => "Global board",
                            }),
                    )
                    .child(div().flex_1())
                    .child(pill(format!("{} items", visible.len()))),
            )
            .child(
                div()
                    .id("board-scroll")
                    .flex_1()
                    .overflow_x_scroll()
                    .child(columns),
            )
    }

    fn open_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let item = &self.board.items[index];
        self.item_title.update(cx, |input, cx| {
            input.set_value(item.title.clone(), window, cx)
        });
        self.item_details.update(cx, |input, cx| {
            input.set_value(item.details.clone(), window, cx)
        });
        self.item_comment
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.item_open = Some(index);
        self.item_tab = "Details";
        self.board_error = None;
        cx.notify();
    }

    fn move_board_item(
        &mut self,
        index: usize,
        status: Status,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if status == Status::Blocked {
            self.blocking_item = Some(index);
            self.draft_blockers = self.board.items[index].blocked_on.clone();
            self.blocker_input.update(cx, |input, cx| {
                input.set_value(self.board.items[index].blocker_note.clone(), window, cx)
            });
            self.modal = Some("Blocked on");
            self.board_error = None;
        } else {
            self.board.move_item(index, status);
            self.modal = None;
        }
        cx.notify();
    }

    fn item_drawer(&self, cx: &mut Context<Self>) -> Div {
        let index = self.item_open.unwrap_or(0);
        let item = &self.board.items[index];
        let mut tabs = row()
            .gap_2()
            .px_5()
            .h(px(42.))
            .border_b_1()
            .border_color(rgb(LINE));
        for tab in ["Details", "Comments", "History"] {
            tabs = tabs.child(
                Button::new(tab)
                    .ghost()
                    .small()
                    .label(tab)
                    .selected(self.item_tab == tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.item_tab = tab;
                        this.board_error = None;
                        cx.notify();
                    })),
            );
        }
        let mut body = col().p_5().gap_4();
        match self.item_tab {
            "Comments" => {
                if item.comments.is_empty() {
                    body = body.child(muted("No comments yet."));
                }
                for (n, comment) in item.comments.iter().enumerate() {
                    body = body.child(
                        col()
                            .p_3()
                            .gap_2()
                            .rounded_md()
                            .bg(rgb(PANEL))
                            .child(
                                muted(format!("{} · Comment {}", comment.author, n + 1))
                                    .text_size(px(10.)),
                            )
                            .child(comment.body.clone()),
                    );
                }
                body = body
                    .child(
                        div()
                            .id("item-comment-input")
                            .child(Textarea::new(&self.item_comment).h(px(100.)))
                            .test_support(),
                    )
                    .child(
                        Button::new("add-item-comment")
                            .primary()
                            .small()
                            .label("Add comment")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let text = this.item_comment.read(cx).value().to_string();
                                match this.board.comment(index, text) {
                                    Ok(()) => {
                                        this.item_comment.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                        this.board_error = None;
                                    }
                                    Err(error) => this.board_error = Some(error),
                                }
                                cx.notify();
                            })),
                    );
            }
            "History" => {
                body = body.child(muted("Latest changes first"));
                for (n, entry) in item.history.iter().enumerate().rev() {
                    body = body.child(
                        col()
                            .gap_2()
                            .pb_4()
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .child(
                                muted(format!("{} · Change {}", entry.author, n + 1))
                                    .text_size(px(10.)),
                            )
                            .child(entry.change.summary()),
                    );
                }
            }
            _ => {
                body = body
                    .child(muted("Title"))
                    .child(
                        div()
                            .id("item-title-input")
                            .child(Input::new(&self.item_title))
                            .test_support(),
                    )
                    .child(muted("Description"))
                    .child(
                        div()
                            .id("item-details-input")
                            .child(Textarea::new(&self.item_details).h(px(170.)))
                            .test_support(),
                    )
                    .child(
                        Button::new("save-item-details")
                            .primary()
                            .small()
                            .label("Save details")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.board_error = this
                                    .board
                                    .edit(
                                        index,
                                        this.item_title.read(cx).value().to_string(),
                                        this.item_details.read(cx).value().to_string(),
                                    )
                                    .err();
                                cx.notify();
                            })),
                    );
                if item.status == Status::Blocked
                    || !item.blocked_on.is_empty()
                    || !item.blocker_note.is_empty()
                {
                    body = body.child(muted("Blocked on").text_size(px(11.)));
                    for prerequisite in &item.blocked_on {
                        let prerequisite = *prerequisite;
                        let other = &self.board.items[prerequisite];
                        body = body.child(
                            Button::new(("open-prerequisite", prerequisite))
                                .ghost()
                                .small()
                                .label(format!(
                                    "{} · {} · {}",
                                    other.id,
                                    other.title,
                                    other.status.label()
                                ))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_item(prerequisite, window, cx)
                                })),
                        );
                    }
                    if !item.blocker_note.is_empty() {
                        body = body.child(item.blocker_note.clone());
                    }
                    body = body.child(
                        Button::new("edit-blockers")
                            .small()
                            .label("Edit blockers")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.move_board_item(index, Status::Blocked, window, cx)
                            })),
                    );
                }
            }
        }
        if let Some(error) = &self.board_error {
            body = body.child(div().text_color(rgb(AMBER)).child(error.to_string()));
        }
        let panel = col()
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(480.))
            .bg(rgb(BG))
            .border_l_1()
            .border_color(rgb(LINE))
            .child(
                row()
                    .h(px(50.))
                    .px_5()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(muted(item.id))
                    .child(div().flex_1())
                    .child(
                        Button::new("close-item")
                            .ghost()
                            .small()
                            .icon(IconName::X)
                            .tooltip("Close item")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.item_open = None;
                                this.board_error = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                col()
                    .px_5()
                    .py_4()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(item.title.clone()),
                    )
                    .child(
                        row().gap_2().child(muted("Status")).child(
                            Button::new("item-status")
                                .small()
                                .label(item.status.label())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.blocking_item = Some(index);
                                    this.modal = Some("Move item");
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(tabs)
            .child(
                div()
                    .id("item-drawer-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            );
        div().absolute().inset_0().bg(rgba(0x00000065)).child(panel)
    }

    fn files(&self, cx: &mut Context<Self>) -> Div {
        let mut list = col()
            .p_4()
            .gap_2()
            .child(muted("SESSION FILES").text_size(px(10.)));
        for (index, path) in [
            "migrations/004_ledger.sql",
            "src/billing/service.rs",
            "specs/billing-v2/SPEC.md",
        ]
        .into_iter()
        .enumerate()
        {
            list = list.child(
                Button::new(("file", index))
                    .ghost()
                    .small()
                    .justify_start()
                    .icon(IconName::File)
                    .label(path)
                    .selected(self.selected_file == index)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_file = index;
                        cx.notify();
                    })),
            );
        }
        col().size_full().child(list).child(div().h(px(1.)).bg(rgb(LINE)))
            .child(div().p_5().font_family("JetBrains Mono").text_size(px(12.)).line_height(px(23.)).child(match self.selected_file {
                1 => "pub async fn post_invoice(\n    tx: &mut Transaction,\n    invoice: &Invoice,\n) -> Result<(), PostingError> {\n    let entry = Entry::from(invoice);\n    ledger.post(tx, entry).await?;\n    save_invoice(tx, invoice).await?;\n    Ok(())\n}",
                2 => "# Billing v2\n\n## Rollback\n\nExisting invoices remain readable.\nRevert the posting path before\ndropping the new ledger tables.",
                _ => "CREATE TABLE ledger_entries (\n    id          UUID PRIMARY KEY,\n    account_id  UUID NOT NULL,\n    debit       BIGINT DEFAULT 0,\n    credit      BIGINT DEFAULT 0,\n    created_at  TIMESTAMPTZ NOT NULL\n);",
            }))
    }

    fn info(&self) -> Div {
        let s = &self.sessions[self.active];
        col()
            .p_6()
            .gap_4()
            .child(div().text_size(px(17.)).child("Session details"))
            .child(muted("AGENT").text_size(px(10.)))
            .child(s.agent)
            .child(muted("MACHINE").text_size(px(10.)))
            .child(s.machine)
            .child(muted("BRANCH").text_size(px(10.)))
            .child("uhvesta/ledger-schema")
            .child(muted("REPOSITORIES").text_size(px(10.)))
            .child("acme/api · acme/ledger · acme/docs")
            .child(muted("Independent UI prototype · simulated data only"))
    }

    fn reconnect_machine(&mut self, machine: &'static str, cx: &mut Context<Self>) {
        for (index, session) in self
            .sessions
            .iter_mut()
            .enumerate()
            .filter(|(_, s)| s.machine == machine && s.status == "Disconnected")
        {
            session.status = "Working";
            self.terminal_lines.push((
                index,
                format!("— Reconnected to {machine}; terminal reattached —"),
            ));
        }
        cx.notify();
    }

    fn machines(&self, cx: &mut Context<Self>) -> Div {
        let mut content = col()
            .p_6()
            .gap_4()
            .child(div().text_size(px(20.)).child("Machines"))
            .child(muted("Your agents run here. The workspace follows you."));
        for (name, system) in [
            ("This Mac", "macOS · arm64 · Local"),
            ("devbox-1", "Ubuntu · x86_64 · SSH"),
            ("devbox-2", "Debian · arm64 · SSH"),
            ("gpu-box", "Ubuntu · 2 GPUs · SSH"),
        ] {
            let online = !self
                .sessions
                .iter()
                .any(|s| s.machine == name && s.status == "Disconnected");
            content = content.child(
                row()
                    .p_4()
                    .gap_4()
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .child(icon(IconName::Server))
                    .child(
                        col()
                            .gap_1()
                            .child(name)
                            .child(muted(system).text_size(px(11.))),
                    )
                    .child(div().flex_1())
                    .child(dot(if online { GREEN } else { AMBER }))
                    .child(muted(if online { "Connected" } else { "Disconnected" }))
                    .when(!online, |d| {
                        d.child(
                            Button::new(SharedString::from(format!("reconnect-{name}")))
                                .small()
                                .label("Reconnect")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reconnect_machine(name, cx)
                                })),
                        )
                    }),
            );
        }
        content
    }

    fn channel_page(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = row()
            .h(px(36.))
            .px_4()
            .gap_1()
            .border_b_1()
            .border_color(rgb(LINE));
        for name in [
            "Sessions",
            "Inbox",
            "Board",
            "Repos & machines",
            "Agent access",
            "Activity",
        ] {
            tabs = tabs.child(
                Button::new(name)
                    .ghost()
                    .small()
                    .label(name)
                    .selected(self.channel_tab == name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.channel_tab = name;
                        cx.notify();
                    })),
            );
        }
        let body = match self.channel_tab {
            "Board" => self.board(cx).into_any_element(), "Inbox" => self.inbox(cx).into_any_element(),
            "Repos & machines" => self.repos().into_any_element(),
            "Agent access" => col().p_5().gap_4().child("Agent access rules")
                .child(muted("Agents own their session, clones and board. Other actions follow this channel’s rules."))
                .child(muted("Read and write its own clones — Allow"))
                .child(muted("Edit another agent’s board item — Ask me"))
                .child(muted("Read another session’s files — Ask me"))
                .child(muted("Start or fork sessions — Ask me"))
                .child(muted("Change channel settings or access rules — Never")).into_any_element(),
            "Activity" => col().p_5().gap_4().child("Recent activity")
                .child(muted(format!("{} · {}", self.sessions[self.active].agent, self.sessions[self.active].title)))
                .child("Prepared changes for review.").into_any_element(),
            _ => {
                let mut list = col().p_5().gap_1().child(row().px_2().h(px(30.)).text_size(px(10.)).text_color(rgb(DIM))
                    .child(div().flex_1().child("SESSION")).child(div().w(px(110.)).child("AGENT"))
                    .child(div().w(px(100.)).child("MACHINE")).child(div().w(px(95.)).child("STATUS")));
                for (index, s) in self.sessions.iter().enumerate().filter(|(_, s)| s.channel == self.channel) {
                    list = list.child(row().id(("channel-session", index)).h(px(42.)).px_2().gap_2().cursor_pointer()
                        .border_b_1().border_color(rgb(LINE)).hover(|d| d.bg(rgb(PANEL)))
                        .child(dot(status_color(s.status)))
                        .child(col().flex_1().min_w_0().pl(px(s.depth as f32 * 14.)).child(s.title.clone())
                            .when(!s.fork_kind.is_empty(), |d| d.child(muted(s.fork_kind).text_size(px(10.)))))
                        .child(div().w(px(110.)).child(muted(s.agent)))
                        .child(div().w(px(100.)).child(muted(s.machine)))
                        .child(div().w(px(95.)).child(muted(if s.archived { "Archived" } else { s.status })))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.sessions[index].archived { this.sessions[index].archived = false; }
                            this.select(index, cx);
                        })));
                }
                list.into_any_element()
            }
        };
        col()
            .size_full()
            .child(
                row()
                    .h(px(55.))
                    .px_5()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("# {}", CHANNELS[self.channel])),
                    )
                    .child(pill(if self.channel == 3 {
                        "gpu-box"
                    } else {
                        "This Mac"
                    }))
                    .child(div().flex_1())
                    .child(
                        Button::new("channel-new-session")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label("New session")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = Some("New session");
                                cx.notify();
                            })),
                    ),
            )
            .child(tabs)
            .child(
                div()
                    .id("channel-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
    }

    fn repos(&self) -> Div {
        let mut content = col()
            .p_5()
            .gap_4()
            .child("Repos & machines")
            .child(muted(format!("~/Humata/{}", CHANNELS[self.channel])))
            .child(muted(
                "Each machine has main/, sessions/ and shared/. Each session gets its own clones.",
            ));
        for name in ["acme/api", "acme/ledger", "acme/docs"] {
            content = content.child(
                row()
                    .p_3()
                    .gap_5()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(icon(IconName::Folder))
                    .child(div().flex_1().child(name))
                    .child(muted("This Mac · main clone"))
                    .child(muted("devbox-1 · main clone")),
            );
        }
        content
    }

    fn journal(&self, cx: &mut Context<Self>) -> Div {
        col().p_5().gap_4().child("Prepared views")
            .child(muted("Views prepared by this session and its forks stay here. Nothing opens until you choose it."))
            .child(col().p_4().gap_3().border_1().border_color(rgb(LINE)).rounded_md().child("Ledger migration walkthrough")
                .child(muted("Claude Code · Review the schema, posting path and rollback together."))
                .child(Button::new("open-prepared-view").small().label("Open it").on_click(cx.listener(|this, _, _, cx| {
                    this.offered_view = "open"; this.review = true; this.review_tab = "Diff"; this.review_full = true; this.remember_view(); cx.notify();
                }))))
    }

    fn home(&self, cx: &mut Context<Self>) -> Div {
        let mut content = col()
            .p_6()
            .gap_5()
            .child(muted("YOUR WORKSPACE").text_size(px(10.)))
            .child(
                div()
                    .text_size(px(25.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Pick up where you left off."),
            )
            .child(muted(format!(
                "{} working · {} need you · {} disconnected",
                self.sessions
                    .iter()
                    .filter(|s| s.status == "Working")
                    .count(),
                self.sessions
                    .iter()
                    .filter(|s| s.status == "Needs you")
                    .count(),
                self.sessions
                    .iter()
                    .filter(|s| s.status == "Disconnected")
                    .count()
            )));
        for index in [0, 1, 3, 5] {
            let s = &self.sessions[index];
            content = content.child(
                row()
                    .gap_3()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(dot(status_color(s.status)))
                    .child(
                        Button::new(("recent", index))
                            .ghost()
                            .label(s.title.clone())
                            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx))),
                    )
                    .child(div().flex_1())
                    .child(muted(s.agent))
                    .child(pill(s.machine)),
            );
        }
        content
    }

    fn modal(&self, cx: &mut Context<Self>) -> Div {
        let name = self.modal.unwrap_or("New session");
        let mut panel = col()
            .w(px(480.))
            .p_5()
            .gap_4()
            .rounded_lg()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(0x3a3d46))
            .child(
                row()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("close-dialog")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = None;
                                cx.notify();
                            })),
                    ),
            );
        if name == "Move item" {
            let index = self.blocking_item.unwrap_or(0);
            panel = panel.child(muted(format!(
                "{} · {}",
                self.board.items[index].id, self.board.items[index].title
            )));
            for status in Status::COLUMNS {
                panel = panel.child(
                    Button::new(SharedString::from(format!("status-{}", status.key())))
                        .ghost()
                        .small()
                        .label(status.label())
                        .selected(self.board.items[index].status == status)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.move_board_item(index, status, window, cx)
                        })),
                );
            }
        } else if name == "Blocked on" {
            let index = self.blocking_item.unwrap_or(0);
            panel = panel
                .child(muted(format!(
                    "{} · {}",
                    self.board.items[index].id, self.board.items[index].title
                )))
                .child("What needs to happen before this can continue?");
            for (other, item) in self
                .board
                .items
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
            {
                panel = panel.child(
                    Button::new(("blocker-choice", other))
                        .ghost()
                        .small()
                        .icon(if self.draft_blockers.contains(&other) {
                            IconName::Check
                        } else {
                            IconName::Square
                        })
                        .label(format!("{} · {}", item.id, item.title))
                        .selected(self.draft_blockers.contains(&other))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.draft_blockers.contains(&other) {
                                this.draft_blockers.retain(|i| *i != other);
                            } else {
                                this.draft_blockers.push(other);
                            }
                            this.board_error = None;
                            cx.notify();
                        })),
                );
            }
            panel = panel
                .child(muted("External blocker or additional context"))
                .child(
                    div()
                        .id("blocker-note-input")
                        .child(Input::new(&self.blocker_input))
                        .test_support(),
                );
            if let Some(error) = &self.board_error {
                panel = panel.child(div().text_color(rgb(AMBER)).child(error.to_string()));
            }
            panel = panel.child(
                Button::new("save-blockers")
                    .primary()
                    .small()
                    .label("Save blocked state")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match this.board.block(
                            index,
                            this.draft_blockers.clone(),
                            this.blocker_input.read(cx).value().to_string(),
                        ) {
                            Ok(()) => {
                                this.modal = None;
                                this.board_error = None;
                            }
                            Err(error) => this.board_error = Some(error),
                        }
                        cx.notify();
                    })),
            );
        } else if name == "Search" {
            panel = panel.child(Input::new(&self.search_input));
            let query = self.search_input.read(cx).value().to_lowercase();
            for (index, s) in self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| s.title.to_lowercase().contains(&query))
            {
                panel = panel.child(
                    Button::new(("result", index))
                        .ghost()
                        .justify_start()
                        .icon(IconName::Terminal)
                        .label(s.title.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.modal = None;
                            this.select(index, cx);
                        })),
                );
            }
        } else if name == "Session actions" {
            panel = panel
                .child(
                    Button::new("move-session")
                        .ghost()
                        .label("Move to another machine…")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.modal = Some("Move session");
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("archive-session")
                        .ghost()
                        .label("Archive session")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sessions[this.active].archived = true;
                            this.modal = None;
                            this.page = "channel";
                            this.channel_tab = "Sessions";
                            cx.notify();
                        })),
                );
        } else {
            panel = panel
                .child(muted(if name == "Fork session" {
                    "Explore another approach with its own code and agent context."
                } else {
                    "A fresh terminal in # payments-rewrite."
                }))
                .child(Input::new(&self.name_input));
            if name == "Fork session" {
                let mut options = row().gap_2();
                for choice in ["Code + context", "Code only", "Context only"] {
                    options = options.child(
                        Button::new(choice)
                            .small()
                            .label(choice)
                            .selected(self.fork_kind == choice)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fork_kind = choice;
                                cx.notify();
                            })),
                    );
                }
                panel = panel.child(options).child(muted(match self.fork_kind {
                    "Code only" => "Copies the code; starts with a fresh agent conversation.",
                    "Context only" => "Copies the conversation; the parent’s files are read-only until you make an own copy.",
                    _ => "Copies the conversation and gives the fork its own clones and branch.",
                }));
            }
            let mut agents = row().gap_1();
            for agent in ["Claude Code", "Codex", "Copilot", "OpenCode", "Shell"] {
                agents = agents.child(
                    Button::new(agent)
                        .ghost()
                        .small()
                        .label(agent)
                        .selected(self.modal_agent == agent)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.modal_agent = agent;
                            cx.notify();
                        })),
                );
            }
            let mut machines = row().gap_1();
            for machine in ["This Mac", "devbox-1", "gpu-box"] {
                machines = machines.child(
                    Button::new(machine)
                        .small()
                        .label(machine)
                        .selected(self.modal_machine == machine)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.modal_machine = machine;
                            cx.notify();
                        })),
                );
            }
            panel = panel
                .child(muted("Agent").text_size(px(10.)))
                .child(agents)
                .child(muted("Machine").text_size(px(10.)))
                .child(machines)
                .child(
                    muted("Prototype only. No processes or repositories are created.")
                        .text_size(px(11.)),
                )
                .child(
                    row().justify_end().child(
                        Button::new("create-session")
                            .primary()
                            .label(if name == "Fork session" {
                                "Create fork"
                            } else if name == "Move session" {
                                "Move session"
                            } else {
                                "Start session"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.modal == Some("Move session") {
                                    this.sessions[this.active].machine = this.modal_machine;
                                    this.modal = None;
                                    cx.notify();
                                    return;
                                }
                                let mut title = this.name_input.read(cx).value().to_string();
                                let fork = this.modal == Some("Fork session");
                                if title.trim().is_empty() {
                                    if fork {
                                        title =
                                            format!("Fork of {}", this.sessions[this.active].title);
                                    } else {
                                        return;
                                    }
                                }
                                this.sessions.push(Session {
                                    title,
                                    channel: this.channel,
                                    agent: this.modal_agent,
                                    machine: this.modal_machine,
                                    status: "Idle",
                                    depth: if fork {
                                        this.sessions[this.active].depth + 1
                                    } else {
                                        0
                                    },
                                    parent: if fork { Some(this.active) } else { None },
                                    fork_kind: if fork { this.fork_kind } else { "" },
                                    archived: false,
                                    view: SessionView::default(),
                                });
                                this.modal = None;
                                this.name_input
                                    .update(cx, |i, cx| i.set_value("", window, cx));
                                this.select(this.sessions.len() - 1, cx);
                            })),
                    ),
                );
        }
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x000000a0))
            .child(panel)
    }
}

impl Render for Prototype {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            "channel" => self.channel_page(cx).into_any_element(),
            "Inbox" => self.inbox(cx).into_any_element(),
            "Board" => self.board(cx).into_any_element(),
            "Machines" => self.machines(cx).into_any_element(),
            "Home" => self.home(cx).into_any_element(),
            "Settings" => col()
                .p_6()
                .gap_4()
                .child(div().text_size(px(20.)).child("Appearance"))
                .child(muted("Humata 2 · UI study 01"))
                .child("Charcoal surfaces · Compact controls · Quiet borders")
                .child(muted("Preferences stay in memory. Restart to reset."))
                .child(
                    Button::new("settings-sidebar")
                        .small()
                        .label("Toggle sidebar")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar = !this.sidebar;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            _ => self.workspace(cx).into_any_element(),
        };
        let main = col()
            .size_full()
            .child(div().flex_1().min_h_0().child(page));
        let mut layout = h_resizable("app-layout");
        if self.sidebar {
            layout = layout.child(
                resizable_panel()
                    .size(px(254.))
                    .size_range(px(210.)..px(400.))
                    .fixed()
                    .child(self.sidebar(cx)),
            );
        }
        layout = layout.child(resizable_panel().child(main));
        col()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .font_family("Inter")
            .text_size(px(12.))
            .child(
                row()
                    .h(px(27.))
                    .px_3()
                    .bg(rgb(SIDE))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .text_size(px(10.))
                    .gap_2()
                    .when(!self.sidebar, |d| {
                        d.child(
                            Button::new("show-sidebar")
                                .ghost()
                                .xsmall()
                                .icon(IconName::PanelLeft)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sidebar = true;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("H₂"))
                    .child(muted("Humata"))
                    .child(div().flex_1())
                    .child(muted("UI PROTOTYPE  /  01"))
                    .child(div().flex_1())
                    .child(muted("Simulated data"))
                    .child(
                        Button::new("search")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Search)
                            .tooltip("Find a session")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal = Some("Search");
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(layout))
            .child(
                row()
                    .h(px(25.))
                    .px_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(SIDE))
                    .text_size(px(10.))
                    .child(icon(IconName::GitBranch).size(px(12.)))
                    .child(muted("uhvesta/ledger-schema"))
                    .child(separator())
                    .child(dot(GREEN))
                    .child(muted("This Mac"))
                    .child(div().flex_1())
                    .child(muted("3 repositories"))
                    .child(separator())
                    .child(muted("Rust + GPUI Kit")),
            )
            .when(self.item_open.is_some(), |d| d.child(self.item_drawer(cx)))
            .when(self.modal.is_some(), |d| d.child(self.modal(cx)))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod automation {
    use super::*;
    use ::std::prelude::v1::test;
    use gpui_kit::test::TestWindowExt;
    use humata2_snapshot::{layout, view};
    use std::path::PathBuf;

    fn open() -> (HeadlessAppContext, AnyWindowHandle, Entity<Prototype>) {
        let mut headless = HeadlessAppContext::with_platform(
            std::sync::Arc::new(gpui_apple::AppleTextSystem::new()),
            std::sync::Arc::new(crate::PrototypeAssets),
            gpui_kit::platform::current_headless_renderer,
        );
        let (handle, prototype) = headless.update(|cx| {
            gpui_kit::init(cx);
            cx.text_system()
                .add_fonts(vec![
                    std::borrow::Cow::Borrowed(include_bytes!("../assets/Inter-Regular.ttf")),
                    std::borrow::Cow::Borrowed(include_bytes!("../assets/IBMPlexSans-Regular.ttf")),
                    std::borrow::Cow::Borrowed(include_bytes!(
                        "../assets/JetBrainsMono-Regular.ttf"
                    )),
                ])
                .expect("bundled fonts load");
            gpui_kit::component::theme::Theme::change(
                gpui_kit::component::theme::ThemeMode::Dark,
                None,
                cx,
            );
            gpui_kit::component::theme::Theme::update(cx, |theme| {
                theme.font_family = "Inter".into();
                theme.font_size = px(13.);
                theme.mono_font_family = "JetBrains Mono".into();
                theme.focus_ring = false;
            });
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1512.), px(960.)),
                })),
                show: false,
                focus: false,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Prototype::new(window, cx))
            })
            .expect("the headless prototype opens")
        });
        headless.run_until_parked();
        (headless, handle, prototype)
    }

    fn settle(window: &mut Window, cx: &mut App) {
        for _ in 0..3 {
            window.render_frame(cx);
        }
    }

    fn capture(window: &mut Window, cx: &mut App, name: &str) {
        settle(window, cx);
        let output = std::env::var_os("HUMATA2_UI_CAPTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/ui-captures")
            });
        std::fs::create_dir_all(&output).expect("the capture directory is writable");
        let image = window.render_to_image().expect("Metal renders off screen");
        image
            .save(output.join(format!("{name}.png")))
            .expect("the PNG is saved");
        std::fs::write(
            output.join(format!("{name}.view.txt")),
            view::capture(window).expect("the semantic tree is captured"),
        )
        .expect("the view snapshot is saved");
        std::fs::write(
            output.join(format!("{name}.layout.txt")),
            layout::capture(window).to_string(),
        )
        .expect("the layout snapshot is saved");
        println!("capture: {}", output.join(format!("{name}.png")).display());
    }

    #[test]
    fn expand_restore_preserves_user_split_sizes() {
        let (mut headless, handle, prototype) = open();
        headless
            .update_window(handle, |_, window, cx| {
                settle(window, cx);
                assert!(
                    window.try_find("Files").is_none(),
                    "session files must not be global navigation"
                );
                capture(window, cx, "workspace");
                for panel in ["Diff", "Board", "Inbox"] {
                    window.click(SharedString::from(format!("tool-{panel}")), cx);
                    settle(window, cx);
                    let split = prototype.read(cx).splits[&0].clone();
                    split.update(cx, |state, cx| state.resize_panel(0, px(430.), window, cx));
                    settle(window, cx);
                    let before = window.find("terminal-pane").bounds();
                    assert!(before.size.width > px(300.));
                    for cycle in 0..3 {
                        window.click("widen-review", cx);
                        settle(window, cx);
                        assert!(window.try_find("terminal-pane").is_none());
                        if cycle == 0 {
                            capture(window, cx, &format!("{}-expanded", panel.to_lowercase()));
                        }
                        window.click("widen-review", cx);
                        settle(window, cx);
                        let after = window.find("terminal-pane").bounds();
                        assert!(
                            (after.size.width - before.size.width).abs() <= px(1.),
                            "{panel} changed terminal width from {:?} to {:?}",
                            before.size.width,
                            after.size.width
                        );
                        if cycle == 0 {
                            capture(window, cx, &format!("{}-restored", panel.to_lowercase()));
                        }
                    }
                    window.click("close-review", cx);
                    settle(window, cx);
                    assert!(window.try_find("review-pane").is_none());
                }
            })
            .expect("the headless window remains open");
    }

    /// A reusable controller: semicolon-separated named UI steps, with a PNG,
    /// semantic view tree and measured layout written after every step.
    #[test]
    fn capture_named_steps() {
        let steps = match std::env::var("HUMATA2_UI_STEPS") {
            Ok(steps) => steps,
            Err(_) => return,
        };
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_, window, cx| {
            capture(window, cx, "step-00");
            for (index, step) in steps.split(';').filter(|s| !s.is_empty()).enumerate() {
                let (kind, value) = step.split_once(':').unwrap_or((step, ""));
                match kind {
                    "drag" => {
                        let (id, status) = value.split_once("->").expect("drag:<item-id>-><status-key>");
                        let item = prototype.read(cx).board.items.iter().position(|item| item.id == id).expect("the board item exists");
                        window.drag_to(("board-card", item), SharedString::from(format!("board-column-{status}")), cx);
                    }
                    "click" => window.click(SharedString::from(value.to_owned()), cx),
                    "session" => window.click(("session", value.parse::<usize>().expect("session:<zero-based-index>")), cx),
                    "channel" => window.click(("channel", value.parse::<usize>().expect("channel:<zero-based-index>")), cx),
                    "panel" => window.click(SharedString::from(format!("tool-{value}")), cx),
                    "expand" | "restore" => window.click("widen-review", cx),
                    "type" => window.input(value, cx),
                    "key" => window.press(value, cx),
                    _ => panic!("Unknown UI step {step:?}. Use channel:<index>, session:<index>, panel:Diff|Board|Inbox|Files, expand, restore, click:<element-id>, type:<text>, or key:<keystroke>."),
                }
                capture(window, cx, &format!("step-{:02}", index + 1));
            }
        }).expect("the headless window remains open");
    }

    #[test]
    fn board_cards_drag_open_edit_comment_and_record_history() {
        let (mut headless, handle, prototype) = open();
        headless
            .update_window(handle, |_, window, cx| {
                settle(window, cx);
                window.click("Board", cx);
                settle(window, cx);
                window.drag_to(("board-card", 0usize), "board-column-todo", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).board.items[0].status, Status::Todo);
                window.drag_to(("board-card", 0usize), "board-column-blocked", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).modal, Some("Blocked on"));
                window.click(("blocker-choice", 1usize), cx);
                window.click("save-blockers", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).board.items[0].status, Status::Blocked);
                assert_eq!(prototype.read(cx).board.items[0].blocked_on, vec![1]);
                capture(window, cx, "board-blocked");
                window.click(("board-card", 0usize), cx);
                settle(window, cx);
                window.click("item-title-input", cx);
                window.press("cmd-a", cx);
                window.input("Audit refund edge cases", cx);
                window.click("save-item-details", cx);
                settle(window, cx);
                assert_eq!(
                    prototype.read(cx).board.items[0].title,
                    "Audit refund edge cases"
                );
                window.click("Comments", cx);
                settle(window, cx);
                window.click("item-comment-input", cx);
                window.input(
                    "Waiting for the partition experiment before continuing.",
                    cx,
                );
                window.click("add-item-comment", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).board.items[0].comments.len(), 1);
                capture(window, cx, "board-item-comments");
                window.click("History", cx);
                settle(window, cx);
                assert!(prototype.read(cx).board.items[0].history.len() >= 6);
                capture(window, cx, "board-item-history");
                window.click("close-item", cx);
                settle(window, cx);
                window.drag_to(("board-card", 0usize), "board-column-wont-do", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).board.items[0].status, Status::WontDo);
                capture(window, cx, "board-wont-do");
            })
            .expect("the board flow runs headlessly");
    }

    #[test]
    fn disconnected_machine_reconnects_only_on_request() {
        let (mut headless, handle, prototype) = open();
        headless
            .update_window(handle, |_, window, cx| {
                settle(window, cx);
                window.click("Machines", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).sessions[7].status, "Disconnected");
                capture(window, cx, "machine-disconnected");
                window.click("reconnect-devbox-2", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).sessions[7].status, "Working");
                assert!(window.try_find("reconnect-devbox-2").is_none());
                capture(window, cx, "machine-reconnected");
            })
            .expect("the machine flow runs headlessly");
    }
}
