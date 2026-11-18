//! Movable session-bound panes, driven by the same in-memory prototype state.
use super::*;
use crate::panes::{Edge, Kind, Node};
use gpui_kit::component::resizable::v_resizable;

#[derive(Clone)]
pub(super) struct CommentState {
    input: Entity<TextareaState>,
    anchor: flows::Anchor,
    file: usize,
    open: bool,
    selection: Option<DiffSelection>,
}

#[derive(Clone)]
pub(super) enum PaneDrag {
    Existing(usize),
    Session(usize, String),
    Open(Kind, usize),
}
impl Render for PaneDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(ACCENT))
            .child(match self {
                Self::Existing(_) => "Move pane".to_owned(),
                Self::Session(_, title) => title.clone(),
                Self::Open(kind, _) => kind.label().into(),
            })
    }
}
impl Prototype {
    pub(super) fn current_pane_comment(&self) -> CommentState {
        CommentState {
            input: self.comment_input.clone(),
            anchor: self.comment_anchor.clone(),
            file: self.selected_file,
            open: self.comment_open,
            selection: self.diff_selection.clone(),
        }
    }
    fn restore_pane_comment(&mut self, state: CommentState) {
        self.comment_input = state.input;
        self.comment_anchor = state.anchor;
        self.selected_file = state.file;
        self.comment_open = state.open;
        self.diff_selection = state.selection;
    }
    pub(super) fn focus_pane(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(pane) = self.pane_layout.panes.get(id).cloned() else {
            return;
        };
        if self.pane_layout.focused != id {
            self.pane_comments
                .insert(self.pane_layout.focused, self.current_pane_comment());
            if let Some(state) = self.pane_comments.get(&id).cloned() {
                self.restore_pane_comment(state);
            } else {
                self.comment_open = false;
                self.diff_selection = None;
            }
        }
        self.pane_layout.focused = id;
        self.pane_menu = None;
        self.active = pane.session;
        self.channel = self.sessions[pane.session].channel;
        match pane.kind {
            Kind::Agent => {
                self.terminal_tab = "Agent";
                self.review = false;
            }
            Kind::Shell => {
                self.terminal_tab = "Shell";
                self.review = false;
            }
            Kind::Tool(kind) => {
                self.review_tab = kind;
                self.review = true;
            }
        }
        self.markdown_file = self.sessions[pane.session].view.document;
        cx.notify();
    }
    pub(super) fn open_pane(
        &mut self,
        kind: Kind,
        session: usize,
        cx: &mut Context<Self>,
    ) -> usize {
        if kind != Kind::Shell {
            if let Some(id) = self.pane_layout.find(session, kind) {
                if self.pane_layout.maximized != Some(id) {
                    self.pane_layout.maximized = None;
                }
                self.focus_pane(id, cx);
                return id;
            }
        }
        let target = self.pane_layout.focused;
        self.pane_comments
            .insert(target, self.current_pane_comment());
        let id = self.pane_layout.add(session, kind, target, Edge::Right);
        self.pane_layout.focused = target;
        self.focus_pane(id, cx);
        id
    }
    pub(super) fn close_pane(&mut self, id: usize, cx: &mut Context<Self>) {
        let previous = self.pane_layout.focused;
        self.pane_comments
            .insert(previous, self.current_pane_comment());
        self.pane_layout.close(id);
        self.pane_recovery_visible = true;
        self.review = false;
        self.pane_menu = None;
        if self.pane_layout.root.is_some() {
            let next = self.pane_layout.focused;
            self.pane_layout.focused = previous;
            self.focus_pane(next, cx);
        }
        cx.notify();
    }
    pub(super) fn split_pane(&mut self, id: usize, edge: Edge, cx: &mut Context<Self>) {
        let previous = self.pane_layout.focused;
        self.pane_comments
            .insert(self.pane_layout.focused, self.current_pane_comment());
        let session = self.pane_layout.panes[id].session;
        let incoming = self.pane_layout.add(session, Kind::Shell, id, edge);
        self.pane_layout.focused = previous;
        self.pane_menu = None;
        self.focus_pane(incoming, cx);
    }
    fn drop_pane(
        &mut self,
        drag: &PaneDrag,
        target: usize,
        edge: Edge,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.pane_layout.focused;
        let incoming = match drag {
            PaneDrag::Existing(id) => {
                self.pane_layout.move_to(*id, target, edge);
                *id
            }
            PaneDrag::Session(session, _) => {
                if let Some(id) = self.pane_layout.find(*session, Kind::Agent) {
                    self.pane_layout.move_to(id, target, edge);
                    id
                } else {
                    self.pane_layout.add(*session, Kind::Agent, target, edge)
                }
            }
            PaneDrag::Open(kind, session) => {
                if *kind == Kind::Tool("Document") {
                    self.ensure_document(self.sessions[*session].view.document, window, cx);
                }
                if let Some(id) = self
                    .pane_layout
                    .find(*session, *kind)
                    .filter(|_| *kind != Kind::Shell)
                {
                    self.pane_layout.move_to(id, target, edge);
                    id
                } else {
                    self.pane_layout.add(*session, *kind, target, edge)
                }
            }
        };
        self.pane_layout.focused = previous;
        self.focus_pane(incoming, cx);
    }
    pub(super) fn pane_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if self.review
            && self
                .pane_layout
                .find(self.active, Kind::Tool(self.review_tab))
                .is_none()
        {
            if self.review_tab == "Document" {
                self.ensure_document(self.markdown_file, window, cx);
            }
            self.open_pane(Kind::Tool(self.review_tab), self.active, cx);
        }
        if self.review_full {
            self.pane_layout.maximized = Some(self.pane_layout.focused);
            self.review_full = false;
        }
        for (id, pane) in self.pane_layout.panes.iter().enumerate() {
            if !self.pane_comments.contains_key(&id) {
                let state = CommentState {
                    input: cx
                        .new(|cx| TextareaState::new(window, cx).placeholder("Leave feedback…")),
                    anchor: flows::Anchor::File,
                    file: 0,
                    open: false,
                    selection: None,
                };
                if id == self.pane_layout.focused {
                    self.comment_input = state.input.clone();
                }
                self.pane_comments.insert(id, state);
            }
            if self.pane_terminals.contains_key(&id) {
                continue;
            }
            let session = &self.sessions[pane.session];
            let terminal = match pane.kind {
                Kind::Agent => self
                    .terminal_views
                    .get(pane.session)
                    .map(|(agent, _)| agent.clone()),
                Kind::Shell => Some(crate::terminal::open(
                    &session.title,
                    "zsh",
                    session.machine,
                    window,
                    cx,
                )),
                _ => None,
            };
            if let Some(terminal) = terminal {
                self.pane_terminals.insert(id, terminal);
            }
        }
        let tree = if let Some(id) = self.pane_layout.maximized {
            Some(Node::Pane(id))
        } else {
            self.pane_layout.root.clone()
        };
        let content = if let Some(tree) = tree {
            self.pane_node(&tree, cx)
        } else {
            col()
                .id("empty-workspace")
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child("Open a pane from the right rail")
                .child(muted("Or drag a session here from the sidebar."))
                .on_drop(cx.listener(|this, drag: &PaneDrag, window, cx| {
                    this.drop_pane(drag, 0, Edge::Right, window, cx)
                }))
                .test_support()
                .into_any_element()
        };
        row()
            .size_full()
            .items_stretch()
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(div().flex_1().min_h_0().child(content))
                    .when(
                        self.pane_recovery_visible && !self.pane_layout.closed.is_empty(),
                        |d| {
                            d.child(
                                row()
                                    .px_3()
                                    .py_1()
                                    .gap_2()
                                    .bg(rgb(SIDE))
                                    .child(
                                        muted("Pane closed · session kept running")
                                            .text_size(px(10.)),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("reopen-pane")
                                            .ghost()
                                            .xsmall()
                                            .label("Reopen")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let previous = this.pane_layout.focused;
                                                if let Some(id) = this.pane_layout.reopen() {
                                                    this.pane_layout.focused = previous;
                                                    this.focus_pane(id, cx);
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("dismiss-pane-recovery")
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::X)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.pane_recovery_visible = false;
                                                cx.notify();
                                            })),
                                    ),
                            )
                        },
                    ),
            )
            .child(self.tool_strip(cx))
    }
    fn pane_node(&mut self, node: &Node, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Pane(id) => self.pane_leaf(*id, cx).into_any_element(),
            Node::Split {
                id,
                vertical,
                first,
                second,
            } => {
                let state = self
                    .pane_splits
                    .entry(*id)
                    .or_insert_with(|| cx.new(|_| ResizableState::default()))
                    .clone();
                let first = self.pane_node(first, cx);
                let second = self.pane_node(second, cx);
                let group = if *vertical {
                    v_resizable(("pane-split", *id))
                } else {
                    h_resizable(("pane-split", *id))
                };
                group
                    .with_state(&state)
                    .child(
                        resizable_panel()
                            .size(px(500.))
                            .size_range(px(120.)..px(4000.))
                            .child(first),
                    )
                    .child(
                        resizable_panel()
                            .size(px(500.))
                            .size_range(px(120.)..px(4000.))
                            .child(second),
                    )
                    .into_any_element()
            }
        }
    }
    fn pane_leaf(&mut self, id: usize, cx: &mut Context<Self>) -> ObservedDiv {
        let pane = self.pane_layout.panes[id].clone();
        let focused = id == self.pane_layout.focused;
        // Each pane renders its bound session. Mouse capture establishes that same
        // context before any child control runs; keyboard focus stays in that pane.
        let prior = (
            self.active,
            self.channel,
            self.terminal_tab,
            self.review_tab,
            self.markdown_file,
            self.comment_open,
        );
        let prior_comment = self.current_pane_comment();
        self.active = pane.session;
        self.channel = self.sessions[pane.session].channel;
        self.markdown_file = self.sessions[pane.session].view.document;
        self.rendering_pane = Some(id);
        if !focused {
            if let Some(state) = self.pane_comments.get(&id).cloned() {
                self.restore_pane_comment(state);
            } else {
                self.comment_open = false;
            }
        }
        let body = match pane.kind {
            Kind::Agent | Kind::Shell => {
                self.terminal_tab = pane.kind.label();
                self.terminal(cx).into_any_element()
            }
            Kind::Tool(kind) => {
                self.review_tab = kind;
                self.review(cx).into_any_element()
            }
        };
        (
            self.active,
            self.channel,
            self.terminal_tab,
            self.review_tab,
            self.markdown_file,
            self.comment_open,
        ) = prior;
        self.restore_pane_comment(prior_comment);
        self.rendering_pane = None;
        let session = &self.sessions[pane.session];
        let title = format!("{} · {}", pane.kind.label(), session.title);
        let header = row()
            .id(("pane-header", id))
            .h(px(34.))
            .px_2()
            .gap_2()
            .flex_shrink_0()
            .bg(rgb(if focused { 0x202127 } else { SIDE }))
            .border_b_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .id(("pane-drag", id))
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.))
                    .text_ellipsis()
                    .child(title)
                    .cursor_move()
                    .on_drag(PaneDrag::Existing(id), |drag, _, _, cx| {
                        cx.new(|_| drag.clone())
                    })
                    .test_support(),
            )
            .when(session.status == "Disconnected", |d| {
                d.child(
                    Button::new(("pane-reconnect", id))
                        .ghost()
                        .xsmall()
                        .label("Reconnect")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reconnect_machine(this.sessions[pane.session].machine, cx);
                        })),
                )
            })
            .child(
                Button::new(("pane-menu", id))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ellipsis)
                    .tooltip("Pane and session actions")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pane_menu = (this.pane_menu != Some(id)).then_some(id);
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("pane-close", id))
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .tooltip("Close pane (⌘W); keep session running")
                    .on_click(cx.listener(move |this, _, _, cx| this.close_pane(id, cx))),
            );
        let mut leaf = div()
            .id(("pane", id))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
                if this.pane_layout.focused != id {
                    this.focus_pane(id, cx);
                }
            }))
            .child(header)
            .child(div().flex_1().min_h_0().child(body));
        if self.pane_menu == Some(id) {
            let mut menu = col()
                .absolute()
                .top(px(31.))
                .right(px(6.))
                .w(px(190.))
                .p_1()
                .gap_1()
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(LINE))
                .rounded_md()
                .occlude();
            for (key, label, edge) in [
                ("right", "Split right · new shell", Edge::Right),
                ("below", "Split below · new shell", Edge::Below),
            ] {
                menu = menu.child(
                    Button::new(SharedString::from(format!("pane-split-{key}-{id}")))
                        .ghost()
                        .small()
                        .justify_start()
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| this.split_pane(id, edge, cx))),
                );
            }
            menu = menu
                .child(
                    Button::new("widen-review")
                        .ghost()
                        .small()
                        .justify_start()
                        .label(if self.pane_layout.maximized.is_some() {
                            "Restore layout"
                        } else {
                            "Maximize pane"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pane_layout.maximized =
                                (this.pane_layout.maximized != Some(id)).then_some(id);
                            this.pane_menu = None;
                            cx.notify();
                        })),
                )
                .when(!self.pane_layout.closed.is_empty(), |d| {
                    d.child(
                        Button::new("menu-reopen-pane")
                            .ghost()
                            .small()
                            .justify_start()
                            .label("Reopen closed pane")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let previous = this.pane_layout.focused;
                                if let Some(id) = this.pane_layout.reopen() {
                                    this.pane_layout.focused = previous;
                                    this.focus_pane(id, cx);
                                }
                                this.pane_menu = None;
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("fork")
                        .ghost()
                        .small()
                        .justify_start()
                        .label("Fork session…")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.focus_pane(id, cx);
                            this.pane_menu = None;
                            this.modal = Some("Fork session");
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("session-menu")
                        .ghost()
                        .small()
                        .justify_start()
                        .label("Session actions…")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.focus_pane(id, cx);
                            this.pane_menu = None;
                            this.modal = Some("Session actions");
                            cx.notify();
                        })),
                )
                .child(
                    muted("Drag the pane title to an edge")
                        .px_2()
                        .py_2()
                        .text_size(px(10.)),
                );
            leaf = leaf.child(menu);
        }
        for (edge, key) in [
            (Edge::Left, "left"),
            (Edge::Right, "right"),
            (Edge::Above, "above"),
            (Edge::Below, "below"),
        ] {
            let zone = div()
                .id(SharedString::from(format!("pane-drop-{key}-{id}")))
                .absolute()
                .role(Role::Group)
                .aria_label(format!("Move pane {key}"))
                .drag_over::<PaneDrag>(|style, _, _, _| style.bg(rgba(0x9caeff55)))
                .on_drop(cx.listener(move |this, drag: &PaneDrag, window, cx| {
                    this.drop_pane(drag, id, edge, window, cx)
                }));
            let zone = match edge {
                Edge::Left => zone.left_0().top(px(34.)).bottom_0().w(px(32.)),
                Edge::Right => zone.right_0().top(px(34.)).bottom_0().w(px(32.)),
                Edge::Above => zone.top_0().left_0().right_0().h(px(12.)),
                Edge::Below => zone.bottom_0().left_0().right_0().h(px(22.)),
            };
            leaf = leaf.child(zone.test_support());
        }
        leaf.test_support()
    }
}
