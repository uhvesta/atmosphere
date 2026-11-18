#[path = "document_pane.rs"]
mod document_pane;
#[path = "pane_ui.rs"]
mod pane_ui;
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
use crate::board::{Board, BoardError, Kind, Scope, Status};
use crate::flows::{self, Delivery, DiffComment, Terminals};

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
    document: usize,
}
impl Default for SessionView {
    fn default() -> Self {
        Self {
            review: false,
            tab: "Diff",
            terminal: "Agent",
            full: false,
            document: 2,
        }
    }
}
#[derive(Clone)]
struct DiffSelection {
    file: usize,
    start: usize,
    end: usize,
    rows: Vec<flows::DiffRow>,
    dragging: bool,
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
    comments: Vec<DiffComment>,
    comment_anchor: flows::Anchor,
    diff_selection: Option<DiffSelection>,
    terminals: std::collections::HashMap<usize, Terminals>,
    terminal_views: Vec<(crate::terminal::View, crate::terminal::View)>,
    comment_open: bool,
    modal: Option<&'static str>,
    content_document: crate::rich::Document,
    documents: std::collections::HashMap<(usize, usize), document_pane::MarkdownPane>,
    markdown_file: usize,
    journey: Option<crate::journeys::Journey>,
    name_input: Entity<InputState>,
    comment_input: Entity<TextareaState>,
    search_input: Entity<InputState>,
    terminal_lines: Vec<(usize, String)>,
    selected_file: usize,
    board: Board,
    board_columns: bool,
    collapsed_milestones: Vec<usize>,
    new_kind: Kind,
    new_parent: Option<usize>,
    new_scope: Scope,
    starting_item: Option<usize>,
    item_open: Option<usize>,
    description_source: bool,
    item_description: crate::rich::Document,
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
    pane_layout: crate::panes::Layout,
    pane_terminals: std::collections::HashMap<usize, crate::terminal::View>,
    pane_splits: std::collections::HashMap<usize, Entity<ResizableState>>,
    pane_menu: Option<usize>,
    rendering_pane: Option<usize>,
    pane_comments: std::collections::HashMap<usize, pane_ui::CommentState>,
    pane_recovery_visible: bool,
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
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("What are we working on?"));
        let comment_input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("Leave feedback for this session’s agent…")
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search work, sessions, channels or machines…")
        });
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
            (
                "Verify backfill safety",
                0,
                "Claude Code",
                "devbox-1",
                "Needs you",
                2,
            ),
            ("Test concurrent writes", 0, "Codex", "devbox-1", "Idle", 3),
            (
                "Reproduce lock contention",
                0,
                "Copilot",
                "This Mac",
                "Exited",
                4,
            ),
        ];
        let mut prototype = Self {
            sessions: data
                .iter()
                .copied()
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
                            9 => Some(1),
                            10 => Some(9),
                            11 => Some(10),
                            _ => None,
                        },
                        fork_kind: match i {
                            1 => "Code + context",
                            2 => "Context only",
                            6 | 10 => "Code only",
                            9 | 11 => "Code + context",
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
                    title: "Edit PAY-5: Add property test for balance",
                    reason: "You created this step in the channel’s ledger milestone.",
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
            comment_anchor: flows::Anchor::File,
            diff_selection: None,
            terminals: std::collections::HashMap::new(),
            terminal_views: data
                .iter()
                .map(|(title, _, agent, machine, _, _)| {
                    (
                        crate::terminal::open(title, agent, machine, window, cx),
                        crate::terminal::open(title, "zsh", machine, window, cx),
                    )
                })
                .collect(),
            comment_open: false,
            modal: None,
            content_document: crate::rich::Document::new(crate::rich::SAMPLE),
            documents: std::collections::HashMap::new(),
            markdown_file: 2,
            journey: None,
            name_input,
            comment_input,
            search_input,
            terminal_lines: vec![],
            selected_file: 0,
            board: Board::default(),
            board_columns: false,
            collapsed_milestones: vec![],
            new_kind: Kind::Task,
            new_parent: None,
            new_scope: Scope::Global,
            starting_item: None,
            item_open: None,
            description_source: false,
            item_description: crate::rich::Document::new(""),
            item_title,
            item_details,
            item_comment,
            blocker_input,
            blocking_item: None,
            draft_blockers: vec![],
            board_error: None,
            _subscriptions: vec![search],
            channel: 0,
            channel_tab: "Sessions",
            review_full: false,
            fork_kind: "Code + context",
            modal_agent: "Claude Code",
            modal_machine: "This Mac",
            show_forks: true,
            offered_view: "new",
            pane_layout: crate::panes::Layout::new(),
            pane_terminals: Default::default(),
            pane_splits: Default::default(),
            pane_menu: None,
            rendering_pane: None,
            pane_comments: Default::default(),
            pane_recovery_visible: false,
            splits: (0..12)
                .map(|i| (i, cx.new(|_| ResizableState::default())))
                .collect(),
        };
        prototype.ensure_document(2, window, cx);
        #[cfg(target_family = "wasm")]
        let prototype = {
            let mut prototype = prototype;
            match crate::startup_scene().as_str() {
                "content" => {
                    prototype.review = true;
                    prototype.review_tab = "Document";
                }
                "work" => prototype.page = "Board",
                "scenarios" => prototype.page = "Scenarios",
                "offline" => {
                    prototype.active = 7;
                    prototype.channel = 2;
                }
                "exited" => prototype.sessions[0].status = "Exited",
                "closed" => prototype.terminals.entry(0).or_default().agent_hidden = true,
                _ => {}
            }
            prototype
        };
        prototype
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.pane_comments.insert(self.pane_layout.focused, self.current_pane_comment());
        self.remember_view();
        self.comment_open = false;
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
        self.markdown_file = view.document;
        if let Some(id) = self.pane_layout.find(index, crate::panes::Kind::Agent) {
            self.pane_layout.focused = id;
        } else if self.pane_layout.root.is_some() {
            let id = self.pane_layout.focused;
            self.pane_layout.panes[id] = crate::panes::Pane { session: index, kind: crate::panes::Kind::Agent };
            self.pane_terminals.remove(&id);
            self.pane_comments.remove(&id);
        } else {
            self.pane_layout.add(index, crate::panes::Kind::Agent, 0, crate::panes::Edge::Right);
        }
        self.pane_layout.maximized = None;
        cx.notify();
    }

    fn remember_view(&mut self) {
        self.sessions[self.active].view = SessionView {
            review: self.review,
            tab: self.review_tab,
            terminal: self.terminal_tab,
            full: self.review_full,
            document: self.markdown_file,
        };
    }

    fn panel(&mut self, name: &'static str, cx: &mut Context<Self>) {
        self.review = true;
        self.review_tab = name;
        self.review_full = false;
        self.remember_view();
        self.open_pane(crate::panes::Kind::Tool(name), self.active, cx);
        cx.notify();
    }

    fn channel_select(&mut self, channel: usize, cx: &mut Context<Self>) {
        self.remember_view();
        self.channel = channel;
        self.page = "channel";
        self.channel_tab = "Sessions";
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
            .aria_label(if label == "Board" { "Work" } else { label })
            .h(px(30.))
            .w_full()
            .px_2()
            .gap_2()
            .rounded(px(5.))
            .bg(rgb(if self.page == page { 0x292d39 } else { SIDE }))
            .cursor_pointer()
            .hover(|d| d.bg(rgb(0x25282f)))
            .child(icon(name))
            .child(if label == "Board" { "Work" } else { label })
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
                for index in self.session_order().into_iter().filter(|i| {
                    self.sessions[*i].channel == channel
                        && !self.sessions[*i].archived
                        && !self.terminal_state(*i).deleted
                }) {
                    let session = &self.sessions[index];
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
                                )
                                .on_drag(pane_ui::PaneDrag::Session(index, session.title.clone()), |drag,_,_,cx| cx.new(|_| drag.clone())),
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
            ("Agent", IconName::Terminal),
            ("Shell", IconName::Plus),
            ("Inbox", IconName::Inbox),
            ("Diff", IconName::Columns2),
            ("Files", IconName::Files),
            ("Document", IconName::File),
            ("Board", IconName::LayoutDashboard),
            ("Journal", IconName::MessageSquare),
            ("Info", IconName::Circle),
        ] {
            strip = strip.child(
                col().id(SharedString::from(format!("pane-source-{name}")))
                    .on_drag(pane_ui::PaneDrag::Open(match name { "Agent" => crate::panes::Kind::Agent, "Shell" => crate::panes::Kind::Shell, _ => crate::panes::Kind::Tool(name) }, self.active), |drag,_,_,cx| cx.new(|_| drag.clone()))
                    .items_center()
                    .child(
                        Button::new(SharedString::from(format!("tool-{name}")))
                            .ghost()
                            .small()
                            .icon(image)
                            .selected(self.review && self.review_tab == name)
                            .tooltip(if name == "Board" { "Linked work" } else { name })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if name == "Agent" || name == "Shell" {
                                    this.open_pane(if name == "Agent" {crate::panes::Kind::Agent} else {crate::panes::Kind::Shell}, this.active, cx);
                                    return;
                                }
                                if name == "Document" {
                                    this.ensure_document(this.markdown_file, window, cx);
                                }
                                this.panel(name, cx);
                            })),
                    )
                    .child(
                        muted(if name == "Board" {
                            "Work"
                        } else if name == "Document" {
                            "Docs"
                        } else {
                            name
                        })
                        .text_size(px(8.)),
                    ),
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
        bar = bar.child(
            Button::new("close-terminal-tab")
                .ghost()
                .xsmall()
                .icon(IconName::X)
                .tooltip("Close this tab; keep its process running")
                .on_click(cx.listener(|this, _, _, cx| {
                    let shell = this.terminal_tab == "Shell";
                    let state = this.terminals.entry(this.active).or_default();
                    if shell {
                        state.shell_hidden = true;
                    } else {
                        state.agent_hidden = true;
                    }
                    cx.notify();
                })),
        );
        bar = bar
            .child(div().flex_1())
            .child(muted(s.machine).text_size(px(10.)));
        let state = self.terminal_state(self.active);
        let hidden = if self.terminal_tab == "Shell" {
            state.shell_hidden
        } else {
            state.agent_hidden
        };
        if hidden {
            return col().id("terminal-pane").role(Role::Group).aria_label("Terminal").size_full().when(self.rendering_pane.is_none(), |d| d.child(bar))
                .child(col().flex_1().p_6().gap_4().child("Terminal tab closed")
                    .child(muted("Closing a tab hides its view. The process, conversation and scrollback are kept."))
                    .child(row().child(Button::new("reopen-terminal").primary().label("Reopen terminal")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let shell = this.terminal_tab == "Shell";
                            let state = this.terminals.entry(this.active).or_default();
                            if shell { state.shell_hidden = false; } else { state.agent_hidden = false; }
                            cx.notify();
                        }))))).test_support();
        }
        let terminal = self.rendering_pane.and_then(|id| self.pane_terminals.get(&id)).or_else(|| self.terminal_views.get(self.active).map(|(agent, shell)| {
            if self.terminal_tab == "Shell" {
                shell
            } else {
                agent
            }
        }));
        if let Some(terminal) = terminal {
            let messages: Vec<_> = self
                .terminal_lines
                .iter()
                .filter(|(session, _)| *session == self.active && self.terminal_tab == "Agent")
                .map(|(_, text)| text)
                .collect();
            terminal.update(cx, |view, cx| {
                let grid = view.grid_mut();
                let enabled_changed = grid.set_enabled(
                    s.status != "Disconnected"
                        && (self.terminal_tab == "Shell" || s.status != "Exited"),
                );
                if enabled_changed {
                    cx.notify();
                }
                if messages.len() > grid.messages_seen {
                    for message in &messages[grid.messages_seen..] {
                        grid.output(&format!("\n{message}\n> "));
                    }
                    grid.messages_seen = messages.len();
                    cx.notify();
                }
            });
        }
        col()
            .id("terminal-pane")
            .role(Role::Group)
            .aria_label("Terminal")
            .size_full()
            .bg(rgb(0x151619))
            .when(self.rendering_pane.is_none(), |d| d.child(bar))
            .when(s.status == "Exited", |d| d.child(col().p_3().gap_2().bg(rgb(0x2a231c))
                .child("Agent exited · conversation and scrollback kept")
                .child(muted(if state.resume_available { "Resume reopens this conversation in the same folder." } else { "This agent cannot resume the original conversation. Fork with context to continue; the old transcript stays readable." }))
                .child(row().gap_2().child(Button::new("resume-agent").small().label("Resume agent").disabled(!state.resume_available)
                    .on_click(cx.listener(|this, _, _, cx| this.resume_agent(cx))))
                    .when(!state.resume_available, |d| d.child(Button::new("recover-with-fork").small().label("Fork with context")
                        .on_click(cx.listener(|this, _, _, cx| { this.fork_kind = "Code + context"; this.modal = Some("Fork session"); cx.notify(); })))))))
            .when(self.comments.iter().any(|c| c.session == self.active && c.delivery == Delivery::Queued), |d| d.child(
                col().p_3().gap_2().bg(rgb(0x222536)).child(format!("Review feedback queued for {} · {}", s.agent, s.title))
                    .child(muted(match s.status { "Disconnected" => "Waiting for this machine to reconnect.", "Exited" => "Waiting for the agent to resume.", "Working" => "Waiting for the agent’s turn to finish.", _ => "Waiting: turn end not certain. Send when you’re ready." }))
                    .child(row().gap_2().child(Button::new("send-queued-now").small().label("Send now").disabled(s.status == "Disconnected" || s.status == "Exited")
                        .on_click(cx.listener(|this, _, _, cx| this.deliver_feedback(cx))))
                        .when(s.status == "Working", |d| d.child(Button::new("finish-turn").small().label("Simulate turn finished")
                            .on_click(cx.listener(|this, _, _, cx| { this.sessions[this.active].status = "Idle"; this.deliver_feedback(cx); })))))))
            .child(
                div()
                    .id("terminal-output")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .child(if let Some(terminal) = terminal { terminal.clone().into_any_element() } else { div().child("New session terminal · ready for input").into_any_element() }).test_support(),
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
            .child(if self.review_tab == "Board" {
                "Linked work"
            } else {
                self.review_tab
            })
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
            "Document" => self.document_pane(cx).into_any_element(),
            "Board" => self.board(cx).into_any_element(),
            "Inbox" => self.inbox(cx).into_any_element(),
            "Info" => self.info().into_any_element(),
            "Journal" => self.journal(cx).into_any_element(),
            _ => self.diff(cx).into_any_element(),
        };
        col()
            .id("review-pane")
            .role(Role::Group)
            .aria_label(if self.review_tab == "Board" {
                "Linked work"
            } else {
                self.review_tab
            })
            .size_full()
            .bg(rgb(BG))
            .border_l_1()
            .border_color(rgb(LINE))
            .when(self.rendering_pane.is_none(), |d| d.child(tabs))
            .child(div().flex_1().min_h_0().child(body))
            .test_support()
    }

    fn select_diff_range(
        &mut self,
        file: usize,
        row: usize,
        extend: bool,
        rows: &[flows::DiffRow],
        cx: &mut Context<Self>,
    ) {
        let start = if extend {
            self.diff_selection
                .as_ref()
                .filter(|s| s.file == file)
                .map(|s| s.start)
                .unwrap_or(row)
        } else {
            row
        };
        self.diff_selection = Some(DiffSelection {
            file,
            start,
            end: row,
            rows: rows.to_vec(),
            dragging: true,
        });
        self.selected_file = file;
        if let Some(anchor) = flows::selected_rows(rows, start, row) {
            self.comment_anchor = anchor;
        }
        cx.notify();
    }

    fn review_rows(&self, file: usize, rows: Vec<flows::DiffRow>, cx: &mut Context<Self>) -> Div {
        use flows::RowKind;
        let rows = std::sync::Arc::new(rows);
        let mut source = col().min_w_full().when(!self.wrap, |d| d.min_w(px(920.)));
        for (index, line) in rows.iter().enumerate() {
            let selected = self.diff_selection.as_ref().is_some_and(|s| {
                s.file == file && index >= s.start.min(s.end) && index <= s.start.max(s.end)
            });
            let background = if selected {
                0x303a56
            } else {
                match line.kind {
                    RowKind::Added => 0x1a2923,
                    RowKind::Deleted => 0x302020,
                    RowKind::Hunk => 0x20222d,
                    _ => BG,
                }
            };
            let color = match line.kind {
                RowKind::Added => 0xb3d5bf,
                RowKind::Deleted => 0xe1aaa9,
                RowKind::Hunk => 0x969bb4,
                _ => 0xaeb0b9,
            };
            let start_rows = rows.clone();
            let gutter_rows = rows.clone();
            let body = div()
                .id((SharedString::from(format!("diff-line-{file}")), index))
                .role(Role::Button)
                .aria_label(format!(
                    "{} · {}",
                    line.old.map(|n| format!("Old {n}")).unwrap_or_default(),
                    line.new.map(|n| format!("New {n}")).unwrap_or_default()
                ))
                .flex_1()
                .min_w_0()
                .pr_2()
                .text_color(rgb(color))
                .cursor_text()
                .when(!self.wrap, |d| d.whitespace_nowrap())
                .child(line.text.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if start_rows[index].kind != RowKind::Hunk {
                            this.select_diff_range(
                                file,
                                index,
                                event.modifiers.shift,
                                &start_rows,
                                cx,
                            );
                        }
                    }),
                )
                .test_support();
            source = source.child(
                row()
                    .min_h(px(22.))
                    .items_start()
                    .bg(rgb(background))
                    .font_family("JetBrains Mono")
                    .text_size(px(10.5))
                    .child(
                        div()
                            .id((SharedString::from(format!("line-comment-{file}")), index))
                            .role(Role::Button)
                            .aria_label("Comment on selected lines")
                            .w(px(24.))
                            .min_h(px(22.))
                            .flex_shrink_0()
                            .cursor_pointer()
                            .child(icon(IconName::MessageSquare).size(px(12.)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    if gutter_rows[index].kind != RowKind::Hunk {
                                        let preserve = !event.modifiers.shift
                                            && this.diff_selection.as_ref().is_some_and(|s| {
                                                s.file == file
                                                    && index >= s.start.min(s.end)
                                                    && index <= s.start.max(s.end)
                                            });
                                        if !preserve {
                                            this.select_diff_range(
                                                file,
                                                index,
                                                event.modifiers.shift,
                                                &gutter_rows,
                                                cx,
                                            );
                                        }
                                        this.comment_open = true;
                                        cx.notify();
                                    }
                                }),
                            )
                            .test_support(),
                    )
                    .child(
                        muted(line.old.map(|n| n.to_string()).unwrap_or_default())
                            .w(px(36.))
                            .flex_shrink_0()
                            .text_right()
                            .pr_2(),
                    )
                    .child(
                        muted(line.new.map(|n| n.to_string()).unwrap_or_default())
                            .w(px(36.))
                            .flex_shrink_0()
                            .text_right()
                            .pr_2(),
                    )
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .text_color(rgb(color))
                            .child(match line.kind {
                                RowKind::Added => "+",
                                RowKind::Deleted => "−",
                                _ => " ",
                            }),
                    )
                    .child(body)
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        if let Some(selection) = &mut this.diff_selection {
                            if selection.file == file && selection.dragging {
                                selection.end = index;
                                if let Some(anchor) =
                                    flows::selected_rows(&selection.rows, selection.start, index)
                                {
                                    this.comment_anchor = anchor;
                                }
                                cx.notify();
                            }
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(selection) = &mut this.diff_selection {
                                if selection.file == file {
                                    selection.dragging = false;
                                    this.selected_file = file;
                                    this.comment_open = true;
                                    cx.notify();
                                }
                            }
                        }),
                    ),
            );
        }
        source
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
            file = file.child(
                div()
                    .id(SharedString::from(format!("diff-source-{index}")))
                    .overflow_x_scroll()
                    .child(self.review_rows(index, flows::diff_rows(index), cx)),
            );
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
                                this.comment_anchor = flows::Anchor::File;
                                this.diff_selection = None;
                                cx.notify();
                            })),
                    ),
            );
            if self.comment_open && self.selected_file == index {
                file = file.child(self.diff_composer(cx));
            }
            for (note_id, note) in self
                .comments
                .iter()
                .enumerate()
                .filter(|(_, note)| note.owner == self.active && note.file == index)
            {
                file = file.child(
                    col()
                        .p_3()
                        .gap_2()
                        .border_t_1()
                        .border_color(rgb(LINE))
                        .child(
                            muted(format!(
                                "You · {}{}",
                                match note.delivery {
                                    Delivery::Draft => "Draft",
                                    Delivery::Queued => "Queued for agent",
                                    Delivery::Sent => "Sent to agent",
                                },
                                format!(" · {}", note.anchor.label())
                            ))
                            .text_size(px(10.)),
                        )
                        .when(!note.anchor.quote().is_empty(), |d| {
                            d.child(
                                div()
                                    .p_2()
                                    .bg(rgb(0x242733))
                                    .font_family("JetBrains Mono")
                                    .text_size(px(10.))
                                    .child(note.anchor.quote().to_owned()),
                            )
                        })
                        .child(gpui_kit::component::text::TextView::markdown(
                            ("diff-comment-body", note_id),
                            note.body.clone(),
                        ))
                        .when(note.delivery == Delivery::Draft, |d| {
                            d.child(
                                row().child(
                                    Button::new(("delete-diff-comment", note_id))
                                        .ghost()
                                        .xsmall()
                                        .label("Discard draft")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.comments.remove(note_id);
                                            cx.notify();
                                        })),
                                ),
                            )
                        }),
                );
            }
            list = list.child(
                col()
                    .id(("diff-repository", index))
                    .role(Role::Group)
                    .aria_label(repo)
                    .gap_3()
                    .child(muted(repo).text_size(px(10.)).px_1())
                    .child(file)
                    .test_support(),
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
                    .child(list)
                    .test_support(),
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
                            self.comments
                                .iter()
                                .filter(|note| note.session == self.active
                                    && note.delivery == Delivery::Draft)
                                .count()
                        ))
                        .text_size(px(10.)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("send-comments")
                            .small()
                            .label(format!("Send to {}", self.sessions[self.active].agent))
                            .disabled(!self.comments.iter().any(|note| {
                                note.session == self.active && note.delivery == Delivery::Draft
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                for note in this.comments.iter_mut().filter(|note| {
                                    note.session == this.active && note.delivery == Delivery::Draft
                                }) {
                                    note.delivery = Delivery::Queued;
                                }
                                this.terminal_tab = "Agent";
                                this.review_full = false;
                                this.terminals.entry(this.active).or_default().agent_hidden = false;
                                if this.sessions[this.active].status == "Idle"
                                    && this.sessions[this.active].agent == "Claude Code"
                                {
                                    this.deliver_feedback(cx);
                                }
                                cx.notify();
                            })),
                    ),
            )
    }

    fn diff_composer(&self, cx: &mut Context<Self>) -> Div {
        col()
            .p_3()
            .gap_2()
            .border_t_1()
            .border_color(rgb(ACCENT))
            .child(muted(format!(
                "Draft for {} · {}{}",
                self.sessions[if self.review_tab == "Document" {
                    self.document_recipient(self.selected_file)
                } else {
                    self.active
                }]
                .agent,
                format!(
                    "{}/{}",
                    flows::FILES[self.selected_file].0,
                    flows::FILES[self.selected_file].1
                ),
                format!(" · {}", self.comment_anchor.label())
            )))
            .when(!self.comment_anchor.quote().is_empty(), |d| {
                d.child(
                    div()
                        .id("comment-selected-context")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .p_2()
                        .bg(rgb(0x242733))
                        .font_family("JetBrains Mono")
                        .text_size(px(10.))
                        .child(self.comment_anchor.quote().to_owned()),
                )
            })
            .child(
                div()
                    .id("diff-comment-input")
                    .child(Textarea::new(&self.comment_input).h(px(80.)))
                    .test_support(),
            )
            .child(
                row()
                    .gap_2()
                    .child(
                        Button::new("save-comment")
                            .small()
                            .label("Add draft")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let body = this.comment_input.read(cx).value().to_string();
                                if body.trim().is_empty() {
                                    return;
                                }
                                this.comments.push(DiffComment {
                                    session: if this.review_tab == "Document" {
                                        this.document_recipient(this.selected_file)
                                    } else {
                                        this.active
                                    },
                                    file: this.selected_file,
                                    anchor: this.comment_anchor.clone(),
                                    owner: this.active,
                                    body,
                                    delivery: Delivery::Draft,
                                });
                                this.comment_open = false;
                                this.comment_input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("cancel-comment")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.comment_open = false;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn terminal_state(&self, session: usize) -> Terminals {
        self.terminals.get(&session).cloned().unwrap_or_default()
    }

    fn deliver_feedback(&mut self, cx: &mut Context<Self>) {
        self.deliver_feedback_to(self.active, cx);
    }
    fn deliver_feedback_to(&mut self, target: usize, cx: &mut Context<Self>) {
        if matches!(self.sessions[target].status, "Disconnected" | "Exited") {
            return;
        }
        if self
            .comments
            .iter()
            .any(|note| note.session == target && note.delivery == Delivery::Queued)
        {
            self.terminal_lines
                .push((target, flows::feedback(&self.comments, target)));
            for note in self
                .comments
                .iter_mut()
                .filter(|note| note.session == target && note.delivery == Delivery::Queued)
            {
                note.delivery = Delivery::Sent;
            }
        }
        cx.notify();
    }

    fn resume_agent(&mut self, cx: &mut Context<Self>) {
        if !self.terminal_state(self.active).resume_available {
            return;
        }
        self.sessions[self.active].status = "Idle";
        self.terminals.entry(self.active).or_default().agent_hidden = false;
        self.terminal_tab = "Agent";
        self.terminal_lines.push((
            self.active,
            format!(
                "{} resumed with the original conversation. Files and scrollback kept.",
                self.sessions[self.active].agent
            ),
        ));
        cx.notify();
    }

    fn session_order(&self) -> Vec<usize> {
        fn visit(sessions: &[Session], index: usize, result: &mut Vec<usize>) {
            result.push(index);
            for child in (0..sessions.len()).filter(|i| sessions[*i].parent == Some(index)) {
                visit(sessions, child, result);
            }
        }
        let mut result = vec![];
        for root in (0..self.sessions.len()).filter(|i| self.sessions[*i].parent.is_none()) {
            visit(&self.sessions, root, &mut result);
        }
        result
    }

    fn recovery_list(&self, cx: &mut Context<Self>) -> Div {
        let trash = self.channel_tab == "Trash";
        let mut list = col().p_5().gap_3().child(muted(if trash { "Deleted sessions stay recoverable for 30 days. Restore keeps their work and conversation." } else { "Archived sessions keep their work and conversation. Restore to continue." }));
        let mut count = 0;
        for (index, session) in self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.channel == self.channel)
        {
            let state = self.terminal_state(index);
            if !(if trash {
                state.deleted
            } else {
                session.archived && !state.deleted
            }) {
                continue;
            }
            count += 1;
            list = list.child(
                row()
                    .p_3()
                    .gap_3()
                    .bg(rgb(PANEL))
                    .rounded_md()
                    .child(
                        col()
                            .flex_1()
                            .gap_1()
                            .child(session.title.clone())
                            .child(muted(format!(
                                "{} · {} · {}",
                                session.agent,
                                session.machine,
                                if trash {
                                    "29 days remaining"
                                } else {
                                    "Conversation and work kept"
                                }
                            ))),
                    )
                    .child(
                        Button::new(("restore-session", index))
                            .small()
                            .label("Restore session")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.sessions[index].archived = false;
                                this.terminals.entry(index).or_default().deleted = false;
                                this.select(index, cx);
                            })),
                    ),
            );
        }
        if count == 0 {
            list = list.child(muted("No sessions here."));
        }
        list
    }

    fn journey_button(
        &self,
        id: &'static str,
        label: &'static str,
        key: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .small()
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.journey = Some(crate::journeys::Journey::new(key));
                this.page = "Journey";
                cx.notify();
            }))
    }

    fn journey_view(&self, cx: &mut Context<Self>) -> Div {
        let Some(journey) = &self.journey else {
            return col().child("Choose a flow in Scenarios.");
        };
        let screen = journey.screens[journey.at];
        let last = journey.at + 1 == journey.screens.len();
        let mut details = col()
            .gap_4()
            .p_6()
            .max_w(px(960.))
            .w_full()
            .child(
                row()
                    .gap_3()
                    .child(pill(format!(
                        "{} / {}",
                        journey.at + 1,
                        journey.screens.len()
                    )))
                    .child(muted(journey.title)),
            )
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(screen.title),
            )
            .child(muted(screen.description));
        if journey.key == "spec" {
            details = details.child(self.content_document.view("journey-spec", cx));
        }
        let mut rows = col().rounded_md().border_1().border_color(rgb(LINE));
        for (index, label) in screen.rows.iter().enumerate() {
            rows = rows.child(
                row()
                    .px_4()
                    .min_h(px(48.))
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new(("journey-choice", index))
                            .ghost()
                            .xsmall()
                            .label(if journey.selected.contains(&index) {
                                "✓"
                            } else {
                                "○"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let journey = this.journey.as_mut().unwrap();
                                if journey.selected.contains(&index) {
                                    journey.selected.retain(|i| *i != index);
                                } else {
                                    journey.selected.push(index);
                                }
                                cx.notify();
                            })),
                    )
                    .child(*label),
            );
        }
        details = details.child(rows).when(journey.at == 0, |d| {
            d.child(
                div()
                    .id("journey-name")
                    .child(Input::new(&self.name_input).small())
                    .test_support(),
            )
        });
        if journey.error {
            details = details.child(col().p_4().gap_2().rounded_md().bg(rgb(0x302020)).child("The operation could not finish")
            .child(muted("The source work and your choices are kept. Retry this step or cancel without discarding them."))
            .child(row().child(Button::new("journey-retry").small().label("Retry").on_click(cx.listener(|this, _, _, cx| { this.journey.as_mut().unwrap().error = false; cx.notify(); })))));
        }
        details = details.child(
            row()
                .gap_2()
                .when(journey.at > 0, |d| {
                    d.child(Button::new("journey-back").small().label("Back").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.journey.as_mut().unwrap().at -= 1;
                            cx.notify();
                        }),
                    ))
                })
                .child(
                    Button::new("journey-cancel")
                        .ghost()
                        .small()
                        .label(if last { "Back to scenarios" } else { "Cancel" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.page = "Scenarios";
                            cx.notify();
                        })),
                )
                .child(div().flex_1())
                .when(!last, |d| {
                    d.child(
                        Button::new("journey-failure")
                            .ghost()
                            .small()
                            .label("Preview failure")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.journey.as_mut().unwrap().error = true;
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("journey-next")
                        .primary()
                        .small()
                        .label(screen.action)
                        .disabled(journey.error)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if last {
                                this.page = "Scenarios";
                            } else {
                                this.journey.as_mut().unwrap().next();
                            }
                            cx.notify();
                        })),
                ),
        );
        col()
            .size_full()
            .min_h_0()
            .child(
                row()
                    .h(px(48.))
                    .px_5()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(journey.title),
            )
            .child(
                div()
                    .id("journey-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(details)
                    .test_support(),
            )
    }

    fn scenarios(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut list = col().p_6().gap_4().child(div().text_size(px(20.)).child("Prototype scenarios"))
            .child(muted("Open a hardcoded starting state, then use the regular controls. No processes, machines or repositories are changed."));
        for (id, title, description) in [
            (
                "review",
                "Review an active agent",
                "Comment on a line, queue feedback, finish its turn, and see feedback arrive in the owning terminal.",
            ),
            (
                "fallback",
                "Review an agent without turn hooks",
                "Codex feedback waits for an explicit Send now; switching sessions keeps their drafts separate.",
            ),
            (
                "closed",
                "Accidentally close a terminal",
                "Reopen its view without stopping the agent or losing its output.",
            ),
            (
                "exited",
                "Stop and resume an agent",
                "The original conversation and files survive. Resume the agent in the same session.",
            ),
            (
                "no-resume",
                "Conversation cannot be resumed",
                "Keep the transcript readable and continue through a fork with context.",
            ),
            (
                "offline",
                "Machine disconnected",
                "Read cached output, keep feedback queued, then reconnect manually.",
            ),
            (
                "archived",
                "Recover an archived session",
                "Restore a session from the channel’s Archived tab.",
            ),
            (
                "trash",
                "Recover a deleted session",
                "Restore a session from Trash during its retention window.",
            ),
            (
                "nested",
                "Four levels of forks",
                "Navigate parent links and separate agent context, machine, work and review state.",
            ),
            (
                "work",
                "Milestones, tasks and steps",
                "Create scoped work, expand milestones, edit details, set blockers and inspect history.",
            ),
            (
                "content",
                "Markdown and diagrams",
                "Preview specifications, code, tables, task lists and Mermaid diagrams.",
            ),
        ] {
            list = list.child(
                row()
                    .p_4()
                    .gap_4()
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(LINE))
                    .rounded_md()
                    .child(
                        col()
                            .flex_1()
                            .gap_2()
                            .child(title)
                            .child(muted(description)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("scenario-{id}")))
                            .small()
                            .label("Open scenario")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                match id {
                                    "work" => this.page = "Board",
                                    "content" => this.open_document(2, window, cx),
                                    "archived" | "trash" => {
                                        this.sessions[4].archived = id == "archived";
                                        this.terminals.entry(4).or_default().deleted =
                                            id == "trash";
                                        this.channel = 0;
                                        this.page = "channel";
                                        this.channel_tab = if id == "archived" {
                                            "Archived"
                                        } else {
                                            "Trash"
                                        };
                                    }
                                    _ => {
                                        let index = match id {
                                            "offline" => 7,
                                            "nested" => 11,
                                            "fallback" => 2,
                                            _ => 0,
                                        };
                                        this.select(index, cx);
                                        this.terminals.entry(index).or_default().agent_hidden =
                                            id == "closed";
                                        this.terminals.entry(index).or_default().resume_available =
                                            id != "no-resume";
                                        this.sessions[index].status = match id {
                                            "exited" | "no-resume" => "Exited",
                                            "offline" => "Disconnected",
                                            "fallback" => "Idle",
                                            _ => "Working",
                                        };
                                        this.terminal_tab = "Agent";
                                        this.review = matches!(id, "review" | "fallback");
                                        this.review_tab = "Diff";
                                        this.review_full = false;
                                    }
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        list = list
            .child(
                div()
                    .mt_4()
                    .text_size(px(17.))
                    .child("Workspace flow studies"),
            )
            .child(muted(
                "Additional product surfaces and their success, cancellation and failure states.",
            ));
        for (key, title, description) in crate::journeys::CATALOG {
            let key = *key;
            list = list.child(
                row()
                    .p_4()
                    .gap_4()
                    .bg(rgb(PANEL))
                    .rounded_md()
                    .child(
                        col()
                            .flex_1()
                            .gap_2()
                            .child(*title)
                            .child(muted(*description)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("journey-{key}")))
                            .small()
                            .label("Open flow")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.journey = Some(crate::journeys::Journey::new(key));
                                this.page = "Journey";
                                cx.notify();
                            })),
                    ),
            );
        }
        div()
            .id("scenario-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(list)
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
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.answer_request(index, true, window, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("deny", index))
                                            .small()
                                            .label("Deny")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.answer_request(index, false, window, cx);
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

    fn answer_request(
        &mut self,
        index: usize,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        while self.terminal_views.len() < self.sessions.len() {
            let session = &self.sessions[self.terminal_views.len()];
            self.terminal_views.push((
                crate::terminal::open(&session.title, session.agent, session.machine, window, cx),
                crate::terminal::open(&session.title, "zsh", session.machine, window, cx),
            ));
        }
    }

    fn board(&self, cx: &mut Context<Self>) -> Div {
        let visible = self.visible_work();
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
                    .child(
                        row()
                            .gap_2()
                            .child(pill(item.kind.label()))
                            .child(muted(item.id.clone()).text_size(px(10.))),
                    )
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
                card = card.child(muted(self.scope_label(item.scope)).text_size(px(10.)));
                if let Some(parent) = item.parent {
                    card = card.child(
                        muted(format!("Milestone: {}", self.board.items[parent].title))
                            .text_size(px(10.)),
                    );
                }
                if status == Status::Blocked {
                    let ids: Vec<_> = item
                        .blocked_on
                        .iter()
                        .map(|i| self.board.items[*i].id.clone())
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
        let scope = self.work_scope_label();
        let mut header = row()
            .min_h(px(62.))
            .px_5()
            .gap_3()
            .border_b_1()
            .border_color(rgb(LINE))
            .child(
                col()
                    .gap_1()
                    .child(div().font_weight(FontWeight::MEDIUM).child(scope))
                    .child(
                        muted(if self.page == "session" {
                            "Work linked to this session · stays in its channel"
                        } else {
                            "Milestones contain steps · tasks stand alone"
                        })
                        .text_size(px(11.)),
                    ),
            )
            .child(div().flex_1());
        for (id, label, columns) in [("work-list", "List", false), ("work-board", "Board", true)] {
            header = header.child(
                Button::new(id)
                    .ghost()
                    .small()
                    .label(label)
                    .selected(self.board_columns == columns)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.board_columns = columns;
                        cx.notify();
                    })),
            );
        }
        if self.page != "session" {
            for (id, label, kind) in [
                ("new-milestone", "+ Milestone", Kind::Milestone),
                ("new-task", "+ Task", Kind::Task),
            ] {
                header = header.child(Button::new(id).small().label(label).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.begin_work(kind, None, window, cx);
                    },
                )));
            }
        }
        let mut content = col().gap_2().p_4();
        if visible.is_empty() {
            content = content.child(muted("No work here yet."));
        }
        for index in visible
            .iter()
            .copied()
            .filter(|i| self.board.items[*i].parent.is_none() || self.page == "session")
        {
            content = content.child(self.work_row(index, false, cx));
            if self.board.items[index].kind == Kind::Milestone
                && !self.collapsed_milestones.contains(&index)
            {
                for child in self.board.children(index) {
                    content = content.child(self.work_row(child, true, cx));
                }
                content = content.child(
                    row().pl_8().child(
                        Button::new(("add-step", index))
                            .ghost()
                            .small()
                            .label("+ Add step")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.begin_work(Kind::Step, Some(index), window, cx)
                            })),
                    ),
                );
            }
        }
        col().size_full().min_h_0().child(header).child(
            div()
                .id("board-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .overflow_x_scroll()
                .child(if self.board_columns { columns } else { content }),
        )
    }

    fn visible_work(&self) -> Vec<usize> {
        self.board
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| match self.page {
                "session" => item.session == Some(self.active),
                "channel" => item.scope == Scope::Channel(self.channel),
                _ => true,
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn work_scope_label(&self) -> String {
        match self.page {
            "session" => "Linked work".into(),
            "channel" => format!("#{} · Work", CHANNELS[self.channel]),
            _ => "All work · Global & channels".into(),
        }
    }

    fn scope_label(&self, scope: Scope) -> String {
        match scope {
            Scope::Global => "Global".into(),
            Scope::Channel(channel) => format!("#{}", CHANNELS[channel]),
        }
    }

    fn begin_work(
        &mut self,
        kind: Kind,
        parent: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_kind = kind;
        self.new_parent = parent;
        self.new_scope = if self.page == "channel" {
            Scope::Channel(self.channel)
        } else {
            Scope::Global
        };
        self.board_error = None;
        self.name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.modal = Some("New work");
        cx.notify();
    }

    fn work_row(&self, index: usize, nested: bool, cx: &mut Context<Self>) -> Div {
        let item = &self.board.items[index];
        let children = self.board.children(index);
        let done = children
            .iter()
            .filter(|i| self.board.items[**i].status == Status::Done)
            .count();
        let mut line = row()
            .flex_shrink_0()
            .min_h(px(64.))
            .px_3()
            .gap_3()
            .rounded_md()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(LINE));
        if item.kind == Kind::Milestone {
            line = line.child(
                Button::new(("fold-milestone", index))
                    .ghost()
                    .xsmall()
                    .label(if self.collapsed_milestones.contains(&index) {
                        "›"
                    } else {
                        "⌄"
                    })
                    .tooltip("Expand or collapse steps")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.collapsed_milestones.contains(&index) {
                            this.collapsed_milestones.retain(|i| *i != index);
                        } else {
                            this.collapsed_milestones.push(index);
                        }
                        cx.notify();
                    })),
            );
        }
        line =
            line.child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        row()
                            .gap_2()
                            .child(pill(item.kind.label()))
                            .child(muted(item.id.clone()).text_size(px(10.)))
                            .child(muted(self.scope_label(item.scope)).text_size(px(10.))),
                    )
                    .child(
                        div()
                            .id(("open-work", index))
                            .role(Role::Button)
                            .aria_label(item.title.clone())
                            .w_full()
                            .cursor_pointer()
                            .hover(|d| d.text_color(rgb(ACCENT)))
                            .child(item.title.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_item(index, window, cx)
                            }))
                            .test_support(),
                    ),
            );
        if item.kind == Kind::Milestone {
            line = line.child(muted(format!("{done}/{} steps done", children.len())));
        }
        if item.status == Status::Blocked {
            line = line.child(muted(format!(
                "Blocked on {}{}",
                item.blocked_on
                    .iter()
                    .map(|i| self.board.items[*i].id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                item.blocker_note
            )));
        }
        line = line.child(
            Button::new(("work-status", index))
                .small()
                .label(item.status.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.blocking_item = Some(index);
                    this.modal = Some("Move item");
                    cx.notify();
                })),
        );
        div().when(nested, |d| d.pl_8()).child(line)
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
        self.description_source = false;
        self.item_description = crate::rich::Document::new(&self.board.items[index].details);
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
        let mut body = col().p_5().gap_4().child(
            row()
                .gap_2()
                .child(pill(item.kind.label()))
                .child(muted(self.scope_label(item.scope))),
        );
        if let Some(parent) = item.parent {
            body = body.child(
                Button::new("parent-milestone")
                    .ghost()
                    .small()
                    .label(format!("Milestone: {}", self.board.items[parent].title))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_item(parent, window, cx)),
                    ),
            );
        }
        if let Some(session) = item.session {
            body = body.child(
                Button::new("linked-session")
                    .ghost()
                    .small()
                    .label(format!("Open session: {}", self.sessions[session].title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.item_open = None;
                        this.select(session, cx);
                        cx.notify();
                    })),
            );
        }
        if item.session.is_none() && item.kind != Kind::Milestone {
            body = body.child(
                row().child(
                    Button::new("start-work-session")
                        .small()
                        .label("Start a session for this work")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.starting_item = Some(index);
                            if let Scope::Channel(channel) = this.board.items[index].scope {
                                this.channel = channel;
                            }
                            let title = this.board.items[index].title.clone();
                            this.name_input
                                .update(cx, |input, cx| input.set_value(title, window, cx));
                            this.modal = Some("New session");
                            cx.notify();
                        })),
                ),
            );
        }
        body = body
            .child(muted("Title"))
            .child(
                div()
                    .id("item-title-input")
                    .child(Input::new(&self.item_title))
                    .test_support(),
            )
            .child(
                row()
                    .gap_2()
                    .child(muted("Description · Markdown"))
                    .child(div().flex_1())
                    .child(
                        Button::new("toggle-description-source")
                            .ghost()
                            .small()
                            .label(if self.description_source {
                                "Preview"
                            } else {
                                "Edit"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.description_source = !this.description_source;
                                cx.notify();
                            })),
                    ),
            )
            .child(if self.description_source || item.details.is_empty() {
                div()
                    .id("item-details-input")
                    .child(Textarea::new(&self.item_details).h(px(170.)))
                    .test_support()
                    .into_any_element()
            } else {
                self.item_description
                    .view("item-description-preview", cx)
                    .into_any_element()
            })
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
                        if this.board_error.is_none() {
                            this.item_description =
                                crate::rich::Document::new(&this.board.items[index].details);
                        }
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

        if item.kind == Kind::Milestone {
            body = body.child(div().font_weight(FontWeight::MEDIUM).child("Steps"));
            for child in self.board.children(index) {
                body = body.child(self.work_row(child, false, cx));
            }
            body = body.child(
                Button::new("drawer-add-step")
                    .small()
                    .label("+ Add step")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.begin_work(Kind::Step, Some(index), window, cx)
                    })),
            );
        }
        body = body.child(div().font_weight(FontWeight::MEDIUM).child("Comments"));
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
                        muted(format!("{} · Comment {}", comment.author, n + 1)).text_size(px(10.)),
                    )
                    .child(gpui_kit::component::text::TextView::markdown(
                        ("item-comment-body", n),
                        comment.body.clone(),
                    )),
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
                                this.item_comment
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.board_error = None;
                            }
                            Err(error) => this.board_error = Some(error),
                        }
                        cx.notify();
                    })),
            );

        body = body.child(div().font_weight(FontWeight::MEDIUM).child("History"));
        body = body.child(muted("Latest changes first"));
        for (n, entry) in item.history.iter().enumerate().rev() {
            body = body.child(
                col()
                    .gap_2()
                    .pb_4()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(muted(format!("{} · Change {}", entry.author, n + 1)).text_size(px(10.)))
                    .child(entry.change.summary()),
            );
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
                    .child(muted(item.id.clone()))
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
            .child(
                div()
                    .id("item-drawer-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body)
                    .test_support(),
            );
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x00000065))
            .child(panel)
    }

    fn files(&self, cx: &mut Context<Self>) -> Div {
        let mut list = col()
            .p_4()
            .gap_2()
            .child(muted("SESSION FILES").text_size(px(10.)))
            .child(row().child(self.journey_button(
                "open-file-manager",
                "Open file manager…",
                "transfer",
                cx,
            )));
        for (index, path) in [
            "migrations/004_ledger.sql",
            "src/billing/service.rs",
            "specs/billing-v2/SPEC.md",
            "documents/release-notes.md",
            "notes/rollout-research.md",
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if index >= 2 {
                            this.open_document(index, window, cx);
                            return;
                        }
                        this.selected_file = index;
                        cx.notify();
                    })),
            );
        }
        let preview = if self.selected_file == 2 {
            self.content_document
                .view("session-markdown", cx)
                .into_any_element()
        } else {
            div().font_family("JetBrains Mono").text_size(px(12.)).line_height(px(23.)).child(if self.selected_file == 1 {
                "pub async fn post_invoice(\n    tx: &mut Transaction,\n    invoice: &Invoice,\n) -> Result<(), PostingError> {\n    let entry = Entry::from(invoice);\n    ledger.post(tx, entry).await?;\n    save_invoice(tx, invoice).await?;\n    Ok(())\n}"
            } else { "CREATE TABLE ledger_entries (\n    id UUID PRIMARY KEY,\n    account_id UUID NOT NULL,\n    debit BIGINT DEFAULT 0,\n    credit BIGINT DEFAULT 0\n);" }).into_any_element()
        };
        col()
            .size_full()
            .min_h_0()
            .child(list)
            .when(self.selected_file == 2, |d| {
                d.child(
                    row().px_4().child(
                        Button::new("open-specification")
                            .small()
                            .label("Open specification")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_document(2, window, cx)
                                }),
                            ),
                    ),
                )
            })
            .child(
                div()
                    .id("file-preview-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .child(preview),
            )
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
            .child(muted("Your agents run here. The workspace follows you."))
            .child(
                row()
                    .gap_2()
                    .child(self.journey_button("add-machine", "Add machine…", "provider", cx))
                    .child(self.journey_button(
                        "machine-storage",
                        "Storage & retention",
                        "storage",
                        cx,
                    ))
                    .child(self.journey_button("machine-recovery", "Recovery options", "lost", cx)),
            );
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
            "Archived",
            "Trash",
            "Inbox",
            "Board",
            "Repos & machines",
            "Agent access",
            "Activity",
        ] {
            tabs = tabs.child(
                Button::new(if name == "Board" {
                    "channel-work"
                } else {
                    name
                })
                .ghost()
                .small()
                .label(if name == "Board" { "Work" } else { name })
                .selected(self.channel_tab == name)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.channel_tab = name;
                    cx.notify();
                })),
            );
        }
        let body = match self.channel_tab {
            "Archived" | "Trash" => self.recovery_list(cx).into_any_element(),
            "Board" => self.board(cx).into_any_element(), "Inbox" => self.inbox(cx).into_any_element(),
            "Repos & machines" => self.repos().into_any_element(),
            "Agent access" => col().p_5().gap_4().child("Agent access rules")
                .child(muted("Agents own their session and clones. Changes to milestones, tasks and steps follow this channel’s access rules."))
                .child(muted("Read and write its own clones — Allow"))
                .child(muted("Edit another creator’s task or step — Ask me"))
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
                for (index, s) in self.sessions.iter().enumerate().filter(|(i, s)| s.channel == self.channel && !s.archived && !self.terminal_state(*i).deleted) {
                    list = list.child(row().id(("channel-session", index)).h(px(42.)).px_2().gap_2().cursor_pointer()
                        .border_b_1().border_color(rgb(LINE)).hover(|d| d.bg(rgb(PANEL)))
                        .child(dot(status_color(s.status)))
                        .child(col().flex_1().min_w_0().pl(px(s.depth as f32 * 14.)).child(s.title.clone())
                            .when(!s.fork_kind.is_empty(), |d| d.child(muted(s.fork_kind).text_size(px(10.)))))
                        .child(div().w(px(110.)).child(muted(s.agent)))
                        .child(div().w(px(100.)).child(muted(s.machine)))
                        .child(div().w(px(95.)).child(muted(if s.archived { "Archived" } else { s.status })))
                        .on_click(cx.listener(move |this, _, _, cx| {
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
                row()
                    .gap_2()
                    .child(self.journey_button("create-channel", "New channel…", "channel", cx))
                    .child(self.journey_button(
                        "start-pr-review",
                        "Review pull requests…",
                        "pr",
                        cx,
                    )),
            )
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
            .id("dialog-scroll")
            .max_h(px(800.))
            .overflow_y_scroll()
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
                                this.starting_item = None;
                                cx.notify();
                            })),
                    ),
            );
        if name == "New work" {
            let scope = self
                .new_parent
                .map(|p| self.board.items[p].scope)
                .unwrap_or(self.new_scope);
            panel = panel
                .child(pill(self.new_kind.label()))
                .child(muted(format!("Scope: {}", self.scope_label(scope))));
            if self.new_parent.is_none() {
                let mut choices = row().flex_wrap().gap_2();
                for scope in
                    std::iter::once(Scope::Global).chain((0..CHANNELS.len()).map(Scope::Channel))
                {
                    choices = choices.child(
                        Button::new(SharedString::from(format!("new-work-scope-{scope:?}")))
                            .small()
                            .label(self.scope_label(scope))
                            .selected(self.new_scope == scope)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.new_scope = scope;
                                cx.notify();
                            })),
                    );
                }
                panel = panel.child(muted("Create in")).child(choices);
            }
            if let Some(parent) = self.new_parent {
                panel = panel.child(muted(format!(
                    "Milestone: {}",
                    self.board.items[parent].title
                )));
            }
            panel =
                panel
                    .child(
                        div()
                            .id("new-work-title")
                            .child(Input::new(&self.name_input))
                            .test_support(),
                    )
                    .child(
                        Button::new("create-work")
                            .primary()
                            .label(format!("Create {}", self.new_kind.label().to_lowercase()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                match this.board.create(
                                    this.new_kind,
                                    scope,
                                    this.new_parent,
                                    this.name_input.read(cx).value().to_string(),
                                ) {
                                    Ok(index) => {
                                        this.modal = None;
                                        this.open_item(index, window, cx);
                                    }
                                    Err(error) => {
                                        this.board_error = Some(error);
                                        cx.notify();
                                    }
                                }
                            })),
                    );
            if let Some(error) = &self.board_error {
                panel = panel.child(muted(error.to_string()));
            }
        } else if name == "Move item" {
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
            for (index, item) in self
                .board
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.title.to_lowercase().contains(&query))
                .take(5)
            {
                panel = panel.child(
                    Button::new(("search-work", index))
                        .ghost()
                        .justify_start()
                        .label(format!(
                            "{} · {} · {}",
                            item.kind.label(),
                            item.title,
                            self.scope_label(item.scope)
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.modal = None;
                            this.page = "Board";
                            this.open_item(index, window, cx);
                        })),
                );
            }
            for (channel, title) in CHANNELS
                .iter()
                .enumerate()
                .filter(|(_, title)| title.contains(&query))
            {
                panel = panel.child(
                    Button::new(("search-channel", channel))
                        .ghost()
                        .justify_start()
                        .label(format!("Channel · #{title}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.modal = None;
                            this.channel_select(channel, cx);
                        })),
                );
            }
            if "machines".contains(&query) || "devbox".contains(&query) {
                panel = panel.child(
                    Button::new("search-machines")
                        .ghost()
                        .justify_start()
                        .label("Machines · This Mac, devbox-1, devbox-2, gpu-box")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.modal = None;
                            this.page = "Machines";
                            cx.notify();
                        })),
                );
            }
        } else if name == "Stop agent" {
            panel = panel.child("Stop the agent process?").child(muted("The conversation, files and terminal output are kept. Resume agent starts it again; closing a tab does not stop it."))
                .child(Button::new("confirm-stop-agent").primary().label("Stop agent").on_click(cx.listener(|this, _, _, cx| {
                    this.sessions[this.active].status = "Exited";
                    this.terminal_lines.push((this.active, "[Agent stopped by you · process exited with code 0]".into()));
                    this.modal = None; cx.notify();
                })));
        } else if name == "Archive session" || name == "Delete session" {
            panel = panel.child(muted(if name == "Archive session" { "Unpushed changes detected. Archive keeps the work and conversation recoverable. Forks are kept as separate sessions." } else { "Move this session to Trash? Its conversation and work can be restored for 30 days. Forks are kept." }))
                .child(Button::new("confirm-lifecycle").primary().label(if name == "Archive session" { "Archive and keep work" } else { "Move to Trash" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if name == "Archive session" { this.sessions[this.active].archived = true; }
                        else { this.terminals.entry(this.active).or_default().deleted = true; }
                        this.channel = this.sessions[this.active].channel; this.page = "channel";
                        this.channel_tab = if name == "Archive session" { "Archived" } else { "Trash" };
                        this.modal = None; cx.notify();
                    })));
        } else if name == "Session actions" {
            panel = panel
                .child(
                    Button::new("stop-agent")
                        .ghost()
                        .label("Stop agent…")
                        .disabled(self.sessions[self.active].status == "Exited")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.modal = Some("Stop agent");
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("delete-session")
                        .ghost()
                        .label("Move session to Trash…")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.modal = Some("Delete session");
                            cx.notify();
                        })),
                )
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
                            this.modal = Some("Archive session");
                            cx.notify();
                        })),
                );
        } else {
            panel = panel
                .child(muted(if name == "Fork session" {
                    "Explore another approach with its own code and agent context.".to_owned()
                } else {
                    format!("A fresh terminal in #{}.", CHANNELS[self.channel])
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
                                if let Some(item) = this.starting_item.take() {
                                    this.board.items[item].session = Some(this.sessions.len() - 1);
                                    if matches!(
                                        this.board.items[item].status,
                                        Status::Backlog | Status::Todo
                                    ) {
                                        this.board.move_item(item, Status::InProgress);
                                    }
                                    this.item_open = None;
                                }
                                let session = this.sessions.last().unwrap();
                                this.terminal_views.push((
                                    crate::terminal::open(
                                        &session.title,
                                        session.agent,
                                        session.machine,
                                        window,
                                        cx,
                                    ),
                                    crate::terminal::open(
                                        &session.title,
                                        "zsh",
                                        session.machine,
                                        window,
                                        cx,
                                    ),
                                ));
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
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x000000a0))
            .child(panel)
    }
}

impl Render for Prototype {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page {
            "channel" => self.channel_page(cx).into_any_element(),
            "Inbox" => self.inbox(cx).into_any_element(),
            "Board" => self.board(cx).into_any_element(),
            "Machines" => self.machines(cx).into_any_element(),
            "Scenarios" => self.scenarios(cx).into_any_element(),
            "Journey" => self.journey_view(cx).into_any_element(),
            "Content" => self.document_pane(cx).into_any_element(),
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
            _ => self.pane_workspace(window, cx).into_any_element(),
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.page == "session" && this.modal.is_none() && this.item_open.is_none() && event.keystroke.modifiers.platform {
                    if event.keystroke.key == "w" {
                        this.close_pane(this.pane_layout.focused,cx);
                        cx.stop_propagation();
                        return;
                    }
                    if matches!(event.keystroke.key.as_str(), "\\" | "backslash") {
                        let edge = if event.keystroke.modifiers.shift { crate::panes::Edge::Below } else { crate::panes::Edge::Right };
                        this.split_pane(this.pane_layout.focused,edge,cx);
                        cx.stop_propagation();
                        return;
                    }
                }
                if (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                    && event.keystroke.key == "k"
                {
                    this.modal = Some("Search");
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
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
                    .child(
                        Button::new("scenarios")
                            .ghost()
                            .xsmall()
                            .label("Scenarios")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = "Scenarios";
                                cx.notify();
                            })),
                    )
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
                    .child(muted(if gpui_kit::GPUI_FAST {
                        if cfg!(debug_assertions) {
                            "Rust + GPUI Fast · debug"
                        } else {
                            "Rust + GPUI Fast · release"
                        }
                    } else {
                        "Rust + GPUI Kit"
                    })),
            )
            .when(self.item_open.is_some(), |d| d.child(self.item_drawer(cx)))
            .when(self.modal.is_some(), |d| d.child(self.modal(cx)))
    }
}

#[cfg(all(test, target_os = "macos", feature = "automation"))]
mod automation {
    use super::*;
    use ::std::prelude::v1::test;
    use gpui_kit::test::{TestQueryExt, TestWindowExt};
    use std::path::PathBuf;

    fn open() -> (HeadlessAppContext, AnyWindowHandle, Entity<Prototype>) {
        let mut headless =
            humata2_engine::headless(&crate::resources()).expect("the bundled fonts load");
        let (handle, prototype) = open_in(&mut headless);
        (headless, handle, prototype)
    }

    /// Opens one more prototype window, hidden, in `headless`.
    fn open_in(headless: &mut HeadlessAppContext) -> (AnyWindowHandle, Entity<Prototype>) {
        let (handle, prototype) = headless.update(|cx| {
            crate::apply_theme(cx);
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
        (handle, prototype)
    }

    fn settle(window: &mut Window, cx: &mut App) {
        for _ in 0..3 {
            window.render_frame(cx);
        }
    }

    fn pane_action(prototype: &Entity<Prototype>, window: &mut Window, action: &'static str, cx: &mut App) {
        let focused = prototype.read(cx).pane_layout.focused;
        window.click(("pane-menu", focused), cx);
        window.click(action, cx);
    }

    fn capture(window: &mut Window, cx: &mut App, name: &str) {
        settle(window, cx);
        // Under `bazel test` the captures go to the test's undeclared
        // outputs (bazel-testlogs/apps/humata2/ui-prototype/<test>/test.outputs).
        let output = std::env::var_os("HUMATA2_UI_CAPTURE_DIR")
            .or_else(|| std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR"))
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR")
                    .map(|out| PathBuf::from(out).join("ui-captures"))
            })
            .unwrap_or_else(|| std::env::temp_dir().join("humata2-ui-captures"));
        std::fs::create_dir_all(&output).expect("the capture directory is writable");
        let image = window.render_to_image().expect("Metal renders off screen");
        image
            .save(output.join(format!("{name}.png")))
            .expect("the PNG is saved");
        std::fs::write(
            output.join(format!("{name}.view.txt")),
            window
                .elements()
                .iter()
                .map(|element| format!("{:?} {:?}\n", element.role(), element.label()))
                .collect::<String>(),
        )
        .expect("the view snapshot is saved");
        std::fs::write(
            output.join(format!("{name}.layout.txt")),
            window
                .elements()
                .iter()
                .map(|element| {
                    format!(
                        "{:?} {:?} {:?}\n",
                        element.role(),
                        element.label(),
                        element.bounds()
                    )
                })
                .collect::<String>(),
        )
        .expect("the layout snapshot is saved");
        println!("capture: {}", output.join(format!("{name}.png")).display());
    }

    /// One step of real input in [`natural_frames_match_fresh_windows_through_the_main_flows`].
    #[derive(Clone, Copy, Debug)]
    enum Step {
        Click(&'static str),
        Drag(&'static str, &'static str),
        Scroll(&'static str, f32),
        Type(&'static str),
    }

    /// The main flows drawn as the desktop app draws them: real input
    /// events, natural frames only, and after every step the frame compared
    /// with a second prototype that is built from scratch after the same
    /// input. On GPUI Fast this checks that retained views and paint match a
    /// fresh frame; on ordinary GPUI that natural frames match refreshed
    /// ones (D58).
    #[test]
    fn natural_frames_match_fresh_windows_through_the_main_flows() {
        use humata2_engine::frame::{self, FrameError};
        use humata2_engine::parity::Pair;

        let (mut headless, subject, _) = open();
        let (reference, _) = open_in(&mut headless);
        let pair =
            Pair::new(&mut headless, subject, reference, false).expect("both prototypes draw");
        let steps = [
            Step::Click("session-1"),
            Step::Click("session-0"),
            Step::Click("terminal"),
            Step::Type("ls"),
            Step::Click("tool-Diff"),
            Step::Click("pane-menu-1"),
            Step::Click("widen-review"),
            Step::Scroll("diff-scroll", -330.),
            Step::Drag("diff-line-1-5", "diff-line-1-7"),
            Step::Click("pane-menu-1"),
            Step::Click("widen-review"),
            Step::Click("pane-close-1"),
            Step::Click("tool-Document"),
            Step::Scroll("markdown-scroll", -650.),
            Step::Click("pane-close-2"),
            Step::Click("Board"),
            Step::Click("work-board"),
            Step::Drag("board-card-0", "board-column-todo"),
            Step::Click("board-card-0"),
            Step::Click("close-item"),
        ];
        let center = |window: &Window, id: &str| {
            frame::find(window, id)
                .unwrap_or_else(|| panic!("{id} is in the last frame"))
                .bounds()
                .center()
        };
        for step in steps {
            let ran = match step {
                Step::Click(id) => pair.input(&mut headless, |window, cx| {
                    frame::click(window, center(window, id), MouseButton::Left, cx);
                    Ok(())
                }),
                Step::Drag(from, to) => pair.input(&mut headless, |window, cx| {
                    let (from, to) = (center(window, from), center(window, to));
                    frame::drag(window, from, to, 6, cx);
                    Ok(())
                }),
                Step::Scroll(id, dy) => pair.input(&mut headless, |window, cx| {
                    frame::scroll(window, center(window, id), point(px(0.), px(dy)), cx);
                    Ok(())
                }),
                // One key per update and the frame in the next one: GPUI hands
                // the field's change to its subscribers only when the update
                // that typed the key ends, and a running app draws after that.
                Step::Type(text) => text.chars().try_for_each(|character| {
                    pair.input(&mut headless, |window, cx| {
                        frame::type_character(window, character, cx)
                    })?;
                    pair.update(&mut headless, |window, cx| {
                        frame::natural_frame(window, cx);
                    })
                }),
            };
            ran.unwrap_or_else(|error: FrameError| panic!("{step:?} runs: {error}"));
            pair.settle(&mut headless)
                .unwrap_or_else(|error| panic!("the windows settle after {step:?}: {error}"));
            let report = pair
                .check(&mut headless, 2)
                .expect("both prototypes draw to pixels");
            assert!(report.matches(), "after {step:?}:\n{report}");
        }
        let counts = pair
            .subject_counts(&mut headless)
            .expect("the subject is open");
        if humata2_engine::Engine::CURRENT.retains_views() {
            let counts = counts.expect("GPUI Fast counts frames");
            assert!(counts.views_reused > 0, "no view was reused: {counts:?}");
            println!(
                "natural-frame flow on {}: {counts:?}",
                humata2_engine::Backend::current()
            );
        }
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
                    let split_id = match prototype.read(cx).pane_layout.root.as_ref().unwrap() { crate::panes::Node::Split { id, .. } => *id, _ => panic!("two panes are split") };
                    let split = prototype.read(cx).pane_splits[&split_id].clone();
                    split.update(cx, |state, cx| state.resize_panel(0, px(430.), window, cx));
                    settle(window, cx);
                    let before = window.find("terminal-pane").bounds();
                    assert!(before.size.width > px(300.));
                    for cycle in 0..3 {
                        pane_action(&prototype, window, "widen-review", cx);
                        settle(window, cx);
                        assert!(window.try_find("terminal-pane").is_none());
                        if cycle == 0 {
                            capture(window, cx, &format!("{}-expanded", panel.to_lowercase()));
                        }
                        pane_action(&prototype, window, "widen-review", cx);
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
                    window.click(("pane-close", prototype.read(cx).pane_layout.focused), cx);
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
                    "expand" | "restore" => pane_action(&prototype, window, "widen-review", cx),
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
                window.click("work-board", cx);
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
                window.scroll(
                    "item-drawer-scroll",
                    ScrollDelta::Pixels(point(px(0.), px(-400.))),
                    cx,
                );
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
                window.scroll(
                    "item-drawer-scroll",
                    ScrollDelta::Pixels(point(px(0.), px(-400.))),
                    cx,
                );
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
    fn work_list_expands_milestones_and_creates_scoped_work() {
        let (mut headless, handle, prototype) = open();
        headless
            .update_window(handle, |_, window, cx| {
                settle(window, cx);
                window.click("Board", cx);
                settle(window, cx);
                assert!(window.try_find("toggle-review").is_none());
                assert!(!prototype.read(cx).board_columns);
                capture(window, cx, "work-list");
                window.click(("fold-milestone", 5usize), cx);
                settle(window, cx);
                assert!(window.try_find(("open-work", 2usize)).is_none());
                window.click(("fold-milestone", 5usize), cx);
                settle(window, cx);
                assert!(window.try_find(("open-work", 2usize)).is_some());
                window.click("new-task", cx);
                settle(window, cx);
                window.click("new-work-title", cx);
                window.input("Review support tickets", cx);
                window.click("create-work", cx);
                settle(window, cx);
                let index = prototype.read(cx).item_open.unwrap();
                assert_eq!(prototype.read(cx).board.items[index].scope, Scope::Global);
                assert_eq!(prototype.read(cx).board.items[index].kind, Kind::Task);
                window.click("close-item", cx);
                window.click(("channel", 0usize), cx);
                settle(window, cx);
                window.click("channel-work", cx);
                settle(window, cx);
                assert!(
                    prototype.read(cx).visible_work().iter().all(|i| prototype
                        .read(cx)
                        .board
                        .items[*i]
                        .scope
                        == Scope::Channel(0))
                );
                window.click(("add-step", 5usize), cx);
                settle(window, cx);
                window.click("new-work-title", cx);
                window.input("Verify migration on staging", cx);
                window.click("create-work", cx);
                settle(window, cx);
                let index = prototype.read(cx).item_open.unwrap();
                assert_eq!(prototype.read(cx).board.items[index].kind, Kind::Step);
                assert_eq!(prototype.read(cx).board.items[index].parent, Some(5));
                assert_eq!(
                    prototype.read(cx).board.items[index].scope,
                    Scope::Channel(0)
                );
                capture(window, cx, "work-step-details");
                window.click("parent-milestone", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).item_open, Some(5));
                capture(window, cx, "work-milestone-details");
                window.click("close-item", cx);
                window.click(("open-work", index), cx);
                settle(window, cx);
                window.click("start-work-session", cx);
                settle(window, cx);
                window.click("create-session", cx);
                settle(window, cx);
                let active = prototype.read(cx).active;
                assert_eq!(prototype.read(cx).board.items[index].session, Some(active));
                assert_eq!(
                    prototype.read(cx).board.items[index].status,
                    Status::InProgress
                );
                assert_eq!(prototype.read(cx).sessions[active].channel, 0);
                assert_eq!(
                    prototype.read(cx).terminal_views.len(),
                    prototype.read(cx).sessions.len()
                );
            })
            .expect("the work hierarchy is usable headlessly");
    }

    #[test]
    fn terminal_is_the_only_input_and_diff_contains_all_repositories() {
        use terminal_grid::Grid;
        let (mut headless, handle, prototype) = open();
        headless
            .update_window(handle, |_, window, cx| {
                settle(window, cx);
                assert!(window.try_find("send-message").is_none());
                window.click("terminal", cx);
                window.input("review direct input", cx);
                settle(window, cx);
                let view = &prototype.read(cx).terminal_views[0].0;
                let grid = view.read(cx).grid();
                let screen = (0..grid.size().rows)
                    .filter_map(|y| grid.row(y))
                    .map(|row| {
                        row.cells
                            .iter()
                            .map(|cell| cell.text.as_str())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(screen.contains("review direct input"));
                window.click("tool-Diff", cx);
                settle(window, cx);
                for index in 0..3usize {
                    assert!(window.try_find(("diff-repository", index)).is_some());
                }
                capture(window, cx, "terminal-only-input-multi-repo-diff");
            })
            .expect("direct terminal input and multi-repository review work together");
    }

    #[test]
    fn review_feedback_is_queued_and_delivered_only_to_its_agent_session() {
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_, window, cx| {
            settle(window, cx);
            window.click("tool-Diff", cx);
            settle(window, cx);
            window.click(("line-comment-0", 5usize), cx);
            settle(window, cx);
            window.click("diff-comment-input", cx);
            window.input("Please reject an unbalanced posting.", cx);
            window.click("save-comment", cx);
            window.click("send-comments", cx);
            settle(window, cx);
            assert_eq!(prototype.read(cx).comments[0].delivery, Delivery::Queued);
            assert_eq!(prototype.read(cx).comments[0].session, 0);
            assert!(matches!(&prototype.read(cx).comments[0].anchor, flows::Anchor::Diff { new: Some(span), .. } if span.start == 5 && span.end == 5));
            capture(window, cx, "review-feedback-queued");
            window.click(("session", 1usize), cx);
            settle(window, cx);
            assert!(!prototype.read(cx).comments.iter().any(|c| c.session == 1));
            window.click(("session", 0usize), cx);
            settle(window, cx);
            window.click("finish-turn", cx);
            settle(window, cx);
            assert_eq!(prototype.read(cx).comments[0].delivery, Delivery::Sent);
            assert!(prototype.read(cx).terminal_lines.iter().any(|(session, text)| *session == 0 && text.contains("migrations/004_ledger.sql · New L5") && text.contains("unbalanced posting")));
            assert!(!prototype.read(cx).terminal_lines.iter().any(|(session, text)| *session == 1 && text.contains("unbalanced posting")));
            capture(window, cx, "review-feedback-delivered");
            window.click("tool-Shell", cx);
            settle(window, cx);
            assert_eq!(prototype.read(cx).terminal_views[0].1.read(cx).grid().messages_seen, 0);
        }).expect("review feedback stays attached to its session");
    }

    #[test]
    fn closing_stopping_archiving_and_deleting_have_separate_recovery_paths() {
        let (mut headless, handle, prototype) = open();
        headless
            .update_window(handle, |_, window, cx| {
                settle(window, cx);
                window.click(("pane-close", 0usize), cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).sessions[0].status, "Working");
                assert!(!prototype.read(cx).pane_layout.visible(0));
                capture(window, cx, "terminal-closed-recoverable");
                window.click("reopen-pane", cx);
                settle(window, cx);
                assert!(prototype.read(cx).pane_layout.visible(0));
                pane_action(&prototype, window, "session-menu", cx);
                window.click("stop-agent", cx);
                window.click("confirm-stop-agent", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).sessions[0].status, "Exited");
                capture(window, cx, "agent-exited-resumable");
                window.click("resume-agent", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).sessions[0].status, "Idle");
                pane_action(&prototype, window, "session-menu", cx);
                window.click("archive-session", cx);
                window.click("confirm-lifecycle", cx);
                settle(window, cx);
                assert!(prototype.read(cx).sessions[0].archived);
                assert_eq!(prototype.read(cx).channel_tab, "Archived");
                capture(window, cx, "session-archive-recovery");
                window.click(("restore-session", 0usize), cx);
                settle(window, cx);
                assert!(!prototype.read(cx).sessions[0].archived);
                pane_action(&prototype, window, "session-menu", cx);
                window.click("delete-session", cx);
                window.click("confirm-lifecycle", cx);
                settle(window, cx);
                assert!(prototype.read(cx).terminal_state(0).deleted);
                assert_eq!(prototype.read(cx).channel_tab, "Trash");
                capture(window, cx, "session-trash-recovery");
                window.click(("restore-session", 0usize), cx);
                settle(window, cx);
                assert!(!prototype.read(cx).terminal_state(0).deleted);
                assert_eq!(prototype.read(cx).sessions[1].parent, Some(0));
                assert_eq!(prototype.read(cx).sessions[11].depth, 4);
                capture(window, cx, "session-restored");
            })
            .expect("recoverable work survives lifecycle actions");
    }

    /// Headless frame costs of the rendered Markdown and Mermaid document:
    /// opening it, then natural frames scrolling through it 120 px at a
    /// time, down and back, with its parsed blocks and diagram layouts
    /// cached. Not presented-frame times: the GPU and the display take no
    /// part. Run with
    /// `bazel run -c opt //apps/humata2/ui-prototype:ui_test -- --ignored markdown_frame_costs --nocapture`
    /// (`:ui_gpui_test` for ordinary GPUI).
    #[test]
    #[ignore = "a measurement; run it in release"]
    fn markdown_frame_costs() {
        use humata2_engine::{frame, window as engine_window};
        use std::time::{Duration, Instant};

        let (mut headless, handle, _) = open();
        let ms = |duration: Duration| duration.as_secs_f64() * 1_000.;
        headless
            .update_window(handle, |_, window, cx| {
                let center = |window: &Window, id: &str| {
                    frame::find(window, id)
                        .unwrap_or_else(|| panic!("{id} is in the last frame"))
                        .bounds()
                        .center()
                };
                frame::natural_frame(window, cx);
                let started = Instant::now();
                frame::click(window, center(window, "tool-Document"), MouseButton::Left, cx);
                let opened = started.elapsed();
                for _ in 0..3 {
                    frame::natural_frame(window, cx);
                }
                let at = center(window, "markdown-scroll");
                for index in 0..20 {
                    let dy = if index % 2 == 0 { -120. } else { 120. };
                    frame::scroll(window, at, point(px(0.), px(dy)), cx);
                }
                engine_window::reset_frame_counts(window);
                let mut times: Vec<Duration> = (0..400)
                    .map(|index| {
                        let dy = if (index / 20) % 2 == 0 { -120. } else { 120. };
                        let started = Instant::now();
                        frame::scroll(window, at, point(px(0.), px(dy)), cx);
                        started.elapsed()
                    })
                    .collect();
                times.sort();
                let at = |q: f64| {
                    let last = times.len().saturating_sub(1);
                    times.get((last as f64 * q).round() as usize).copied().unwrap_or_default()
                };
                println!(
                    "{{\"bench\":\"markdown\",\"backend\":\"{}\",\"headless\":true,\"open_ms\":{:.3},\"frames\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"counts\":\"{:?}\"}}",
                    humata2_engine::Backend::current(),
                    ms(opened),
                    times.len(),
                    ms(at(0.5)),
                    ms(at(0.95)),
                    ms(at(0.99)),
                    ms(at(1.)),
                    engine_window::frame_counts(window),
                );
            })
            .expect("the prototype stays open");
    }

    #[test]
    fn markdown_mermaid_source_editing_and_all_flow_studies_are_exercised() {
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_, window, cx| {
            window.click("tool-Document", cx);
            settle(window, cx);
            capture(window, cx, "markdown-preview");
            window.scroll("markdown-scroll", ScrollDelta::Pixels(point(px(0.), px(-650.))), cx);
            settle(window, cx);
            capture(window, cx, "mermaid-diagrams");
            window.click("markdown-mode-Edit", cx);
            settle(window, cx);
            window.click("markdown-editor", cx);
            window.press("cmd-a", cx);
            window.input("# Updated spec\n\n**Review this.**\n\n```mermaid\nflowchart LR\nA[Draft] --> B[Review]\n```", cx);
            window.click("markdown-mode-Preview", cx);
            settle(window, cx);
            assert!(prototype.read(cx).documents.get(&(0,2)).unwrap().source.read(cx).value().contains("Updated spec"));
            capture(window, cx, "markdown-edited-preview");
            for (key, _, _) in crate::journeys::CATALOG {
                prototype.update(cx, |this, cx| { this.journey = Some(crate::journeys::Journey::new(key)); this.page = "Journey"; cx.notify(); });
                settle(window, cx);
                window.scroll("journey-scroll", ScrollDelta::Pixels(point(px(0.), px(-10000.))), cx);
                settle(window, cx);
                window.click("journey-failure", cx);
                settle(window, cx);
                assert!(prototype.read(cx).journey.as_ref().unwrap().error);
                window.scroll("journey-scroll", ScrollDelta::Pixels(point(px(0.), px(-10000.))), cx);
                settle(window, cx);
                window.click("journey-retry", cx);
                settle(window, cx);
                assert!(!prototype.read(cx).journey.as_ref().unwrap().error);
                while prototype.read(cx).journey.as_ref().unwrap().at + 1 < prototype.read(cx).journey.as_ref().unwrap().screens.len() {
                    // Long rendered specs share this same scrollable surface.
                    window.scroll("journey-scroll", ScrollDelta::Pixels(point(px(0.), px(-10000.))), cx);
                    settle(window, cx);
                    window.click("journey-next", cx);
                    settle(window, cx);
                }
                capture(window, cx, &format!("flow-{key}-complete"));
                window.click("journey-next", cx);
                settle(window, cx);
                assert_eq!(prototype.read(cx).page, "Scenarios");
            }
        }).expect("content and product flow studies remain interactive");
    }
    #[test]
    fn dragging_a_mixed_diff_range_preserves_both_revisions_and_context() {
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_, window, cx| {
            settle(window, cx);
            window.click("tool-Diff", cx);
            pane_action(&prototype, window, "widen-review", cx);
            settle(window, cx);
            window.scroll("diff-scroll", ScrollDelta::Pixels(point(px(0.), px(-330.))), cx);
            settle(window, cx);
            window.drag_to(("diff-line-1", 5usize), ("diff-line-1", 7usize), cx);
            settle(window, cx);
            assert!(matches!(&prototype.read(cx).comment_anchor, flows::Anchor::Diff { old: Some(span), new: Some(new), .. } if span.start == 86 && new.start == 86 && new.end == 87));
            assert!(prototype.read(cx).comment_anchor.quote().contains("-     save_invoice"));
            window.scroll("diff-scroll", ScrollDelta::Pixels(point(px(0.), px(-250.))), cx);
            settle(window, cx);
            window.click("diff-comment-input", cx);
            window.input("Keep the old transaction boundary across this replacement.", cx);
            window.click("save-comment", cx);
            settle(window, cx);
            assert_eq!(prototype.read(cx).comments[0].file, 1);
            assert!(matches!(&prototype.read(cx).comments[0].anchor, flows::Anchor::Diff { old: Some(_), new: Some(_), .. }));
            capture(window, cx, "mixed-range-diff-comment");
        }).expect("mixed added/deleted ranges can receive a single comment");
    }

    #[test]
    fn markdown_preview_and_edit_selections_can_message_a_different_agent() {
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_, window, cx| {
            settle(window, cx);
            window.click("tool-Document", cx);
            pane_action(&prototype, window, "widen-review", cx);
            settle(window, cx);
            window.click(("markdown-file", 3usize), cx);
            settle(window, cx);
            let paragraph = window.find(("markdown-block", 2usize)).bounds();
            window.drag(point(paragraph.origin.x + px(3.), paragraph.origin.y + px(10.)), point(paragraph.origin.x + px(330.), paragraph.origin.y + px(10.)), cx);
            settle(window, cx);
            assert!(prototype.read(cx).documents.get(&(0,3)).unwrap().selection.is_some());
            window.click("comment-markdown-selection", cx);
            settle(window, cx);
            capture(window, cx, "markdown-comment-composer");
            window.click("diff-comment-input", cx);
            window.input("Clarify this for customers, please.", cx);
            window.click("markdown-recipient-picker", cx);
            window.click(("markdown-recipient", 1usize), cx);
            settle(window, cx);
            window.click("save-comment", cx);
            settle(window, cx);
            assert_eq!(prototype.read(cx).comments[0].owner, 0);
            assert_eq!(prototype.read(cx).comments[0].session, 1);
            assert_eq!(prototype.read(cx).comments[0].file, 3);
            assert!(matches!(&prototype.read(cx).comments[0].anchor, flows::Anchor::Markdown { lines, quote, .. } if lines.start == 5 && !quote.is_empty()));
            assert!(prototype.read(cx).comments[0].anchor.quote().contains("**double-entry ledger**"));
            capture(window, cx, "markdown-inline-preview-comment");
            window.click(("send-document-comment", 0usize), cx);
            settle(window, cx);
            assert!(prototype.read(cx).terminal_lines.iter().any(|(target, text)| *target == 1 && text.contains("documents/release-notes.md") && text.contains("Clarify this")));
            assert!(prototype.read(cx).terminal_lines.iter().any(|(target, text)| *target == 1 && text.contains("**double-entry ledger**")));
            window.click("markdown-mode-Edit", cx);
            settle(window, cx);
            window.click("markdown-editor", cx);
            window.press("cmd-a", cx);
            settle(window, cx);
            assert!(prototype.read(cx).documents.get(&(0,3)).unwrap().selection.is_some());
            window.click("comment-markdown-selection", cx);
            settle(window, cx);
            window.scroll("markdown-scroll", ScrollDelta::Pixels(point(px(0.), px(-10000.))), cx);
            settle(window, cx);
            capture(window, cx, "markdown-edit-composer");
            window.click("diff-comment-input", cx);
            window.input("Check the entire draft for consistency.", cx);
            window.click("save-comment", cx);
            settle(window, cx);
            assert!(matches!(&prototype.read(cx).comments[1].anchor, flows::Anchor::Markdown { lines, .. } if lines.end > lines.start));
            capture(window, cx, "markdown-edit-range-comment");
            let revised = prototype.read(cx).documents[&(0, 3)].source.read(cx).value().to_string()
                .replace("Billing now uses", "Invoice posting now uses");
            window.click("markdown-editor", cx);
            window.press("cmd-a", cx);
            window.input(&revised, cx);
            settle(window, cx);
            assert!(prototype.read(cx).comments[0].anchor.outdated(&revised));
            window.click("markdown-mode-Changes", cx);
            settle(window, cx);
            capture(window, cx, "markdown-source-changes");
            window.click("markdown-changes-rendered", cx);
            settle(window, cx);
            capture(window, cx, "markdown-rendered-changes");
            window.click(("markdown-file", 4usize), cx);
            settle(window, cx);
            capture(window, cx, "markdown-sql-shell-highlight");
        }).expect("generic document feedback works in preview and source mode");
    }
    #[test]
    fn panes_split_move_and_reopen_without_losing_terminal_input() {
        use crate::panes::{Kind, Node};
        use terminal_grid::Grid;
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_, window, cx| {
            settle(window,cx);
            window.click("tool-Shell",cx);
            settle(window,cx);
            assert_eq!(prototype.read(cx).pane_layout.panes[1].kind,Kind::Shell);
            window.click_at(("pane",1usize),point(px(80.),px(90.)),cx);
            window.input("shell-one-retained",cx);
            settle(window,cx);
            window.click("tool-Shell",cx);
            settle(window,cx);
            let view = prototype.read(cx).pane_terminals[&1].clone();
            let screen = |view:&crate::terminal::View,cx:&App| {
                let grid = view.read(cx).grid();
                (0..grid.size().rows).filter_map(|y|grid.row(y)).flat_map(|row|row.cells.iter().map(|cell|cell.text.clone())).collect::<String>()
            };
            assert!(screen(&view,cx).contains("shell-one-retained"));
            assert!(!screen(&prototype.read(cx).pane_terminals[&2],cx).contains("shell-one-retained"));
            capture(window,cx,"two-shells-and-agent");
            window.drag_to(("session",1usize),"pane-drop-right-2",cx);
            settle(window,cx);
            assert_eq!(prototype.read(cx).pane_layout.panes[3].session,1);
            assert_eq!(prototype.read(cx).pane_layout.panes[3].kind,Kind::Agent);
            capture(window,cx,"two-sessions-and-shells");
            window.drag_to(("pane-drag",1usize),"pane-drop-below-3",cx);
            settle(window,cx);
            let one = window.find(("pane",1usize)).bounds();
            let three = window.find(("pane",3usize)).bounds();
            assert!(one.origin.y > three.origin.y);
            capture(window,cx,"session-and-moved-shell");
            window.click(("pane-close",1usize),cx);
            settle(window,cx);
            assert!(!prototype.read(cx).pane_layout.visible(1));
            assert_eq!(prototype.read(cx).sessions[0].status,"Working");
            window.click("reopen-pane",cx);
            settle(window,cx);
            assert!(prototype.read(cx).pane_layout.visible(1));
            assert!(screen(&prototype.read(cx).pane_terminals[&1],cx).contains("shell-one-retained"));
            pane_action(&prototype,window,"widen-review",cx);
            settle(window,cx);
            assert!(window.try_find(("pane",3usize)).is_none());
            pane_action(&prototype,window,"widen-review",cx);
            settle(window,cx);
            assert!(window.try_find(("pane",3usize)).is_some());
            assert!(matches!(prototype.read(cx).pane_layout.root,Some(Node::Split{..})));
            capture(window,cx,"pane-layout-restored");
            window.press("cmd-w",cx);
            settle(window,cx);
            assert!(!prototype.read(cx).pane_layout.visible(1));
            window.click("reopen-pane",cx);
            settle(window,cx);
            let count = prototype.read(cx).pane_layout.panes.len();
            window.press("cmd-\\",cx);
            settle(window,cx);
            assert_eq!(prototype.read(cx).pane_layout.panes.len(),count+1);
        }).expect("panes remain independent and recoverable");
    }

    #[test]
    fn unsent_feedback_stays_with_its_pane_when_focus_and_layout_change() {
        let (mut headless, handle, prototype) = open();
        headless.update_window(handle, |_,window,cx| {
            window.click("tool-Diff",cx);
            settle(window,cx);
            window.click(("line-comment-0",5usize),cx);
            window.click("diff-comment-input",cx);
            window.input("Keep this draft on the ledger diff",cx);
            window.click("tool-Shell",cx);
            settle(window,cx);
            assert!(!prototype.read(cx).comment_open);
            assert!(prototype.read(cx).comment_input.read(cx).value().is_empty());
            window.click(("pane-drag",1usize),cx);
            settle(window,cx);
            assert!(prototype.read(cx).comment_open);
            assert!(prototype.read(cx).comment_input.read(cx).value().contains("ledger diff"));
            window.click(("pane-close",1usize),cx);
            settle(window,cx);
            window.click("reopen-pane",cx);
            settle(window,cx);
            assert!(prototype.read(cx).comment_open);
            assert!(prototype.read(cx).comment_input.read(cx).value().contains("ledger diff"));
            assert_eq!(prototype.read(cx).active,0);
            window.click("save-comment",cx);
            settle(window,cx);
            assert_eq!(prototype.read(cx).comments[0].owner,0);
            assert_eq!(prototype.read(cx).comments[0].file,0);
        }).expect("feedback drafts remain scoped to a pane");
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
