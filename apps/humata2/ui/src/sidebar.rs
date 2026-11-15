//! The sidebar (NV-1–5, NV-16–23): pinned sessions, then channels with
//! their sessions and forks nested under them. Only the rows in view are
//! built, so five thousand sessions scroll as smoothly as five. With focus,
//! the arrow keys move a cursor through the rows, ← and → fold and unfold,
//! and Enter opens the row.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, PaneAction};
use humata2_sidebar::{Move, Row, RowKey, RowKind, SidebarAction};

use crate::bridge::{Part, Ui};
use crate::keys;
use crate::theme::{self, color};
use crate::widgets::{self, row};

/// The sidebar's view.
pub struct Sidebar {
    ui: Rc<Ui>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    shown_cursor: Option<RowKey>,
}

impl Sidebar {
    /// The sidebar on `ui`, taking focus through `focus`.
    pub fn new(ui: Rc<Ui>, focus: FocusHandle, cx: &mut Context<Self>) -> Self {
        ui.attach(Part::Sidebar, cx.entity_id());
        Self {
            ui,
            focus,
            scroll: UniformListScrollHandle::new(),
            shown_cursor: None,
        }
    }

    fn step(&mut self, step: Move, cx: &mut Context<Self>) {
        self.ui.send(AppAction::Sidebar(SidebarAction::Move(step)), cx);
    }
}

fn label(row: &Row) -> String {
    match &row.kind {
        RowKind::Section(name) => (*name).to_owned(),
        RowKind::Channel { name, .. } => format!("#{name}"),
        RowKind::Session { title, .. } => title.clone(),
    }
}

fn row_id(row: &Row, index: usize) -> ElementId {
    match &row.key {
        Some(RowKey::Channel(id)) => ElementId::Name(format!("sidebar-channel-{}", id.0).into()),
        Some(RowKey::Session(id)) => ElementId::Name(format!("sidebar-session-{}", id.0).into()),
        Some(RowKey::Pinned(id)) => ElementId::Name(format!("sidebar-pinned-{}", id.0).into()),
        None => ElementId::Name(format!("sidebar-section-{index}").into()),
    }
}

fn draw_row(
    ui: &Rc<Ui>,
    focus: &FocusHandle,
    entry: &Row,
    index: usize,
    cursor: bool,
    current: bool,
) -> AnyElement {
    if let RowKind::Section(name) = &entry.kind {
        return div()
            .id(row_id(entry, index))
            .h(px(theme::ROW))
            .px_3()
            .pt_2()
            .child(widgets::heading(*name))
            .into_any_element();
    }
    let Some(key) = entry.key.clone() else {
        return div().into_any_element();
    };
    let indent = px(8. + f32::from(entry.depth) * 14.);
    let chevron = entry.expanded.map(|open| {
        let ui = ui.clone();
        let key = key.clone();
        Button::new(("sidebar-fold", index))
            .icon(if open { IconName::ChevronDown } else { IconName::ChevronRight })
            .ghost()
            .xsmall()
            .tab_index(-1)
            .accessibility_label(if open { "Fold" } else { "Unfold" })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                ui.send(AppAction::Sidebar(SidebarAction::Toggle(key.clone())), cx);
            })
    });
    let body = match &entry.kind {
        RowKind::Channel {
            name,
            sessions,
            needs_you,
        } => row()
            .flex_1()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color(theme::TEXT))
                    .child(format!("#{name}")),
            )
            .when(*needs_you > 0, |r| r.child(widgets::pill(needs_you.to_string(), theme::ORANGE)))
            .child(widgets::faint(sessions.to_string())),
        RowKind::Session {
            title,
            status,
            agent,
            unread,
            vip,
            recovered,
        } => row()
            .flex_1()
            .gap_2()
            .overflow_hidden()
            .child(widgets::dot(*status))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(color(if *unread > 0 { theme::TEXT } else { theme::DIM }))
                    .when(*unread > 0, |t| t.font_weight(FontWeight::SEMIBOLD))
                    .child(title.clone()),
            )
            .when(*vip, |r| r.child(widgets::pill("VIP", theme::YELLOW)))
            .when(*recovered, |r| r.child(widgets::pill("Back", theme::ACCENT2)))
            .when(*unread > 0, |r| r.child(widgets::pill(unread.to_string(), theme::ACCENT)))
            .child(widgets::faint(widgets::agent_tag(*agent))),
        RowKind::Section(_) => row(),
    };
    let ui = ui.clone();
    let focus = focus.clone();
    let mut item = widgets::list_item(row_id(entry, index), Role::TreeItem, label(entry), cursor)
        .h(px(theme::ROW))
        .pl(indent)
        .pr_2()
        .mx_1()
        .when(current, |r| r.border_l_2().border_color(color(theme::ACCENT)))
        .child(div().w(px(18.)).children(chevron))
        .child(body)
        .on_click(move |_, window, cx| {
            window.focus(&focus, cx);
            ui.send(AppAction::Sidebar(SidebarAction::Activate(key.clone())), cx);
        });
    if let Some(open) = entry.expanded {
        item = item.aria_expanded(open);
    }
    item.test_support().into_any_element()
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rows, cursor, current, collapsed, width) = self.ui.read(|state| {
            let (width, _) = state.layout.widths(
                f32::from(window.viewport_size().width) - theme::RAIL_WIDTH,
                state.panel().is_some(),
            );
            (
                state.sidebar.rows.clone(),
                state.sidebar.cursor.clone(),
                state.sidebar.current.clone(),
                state.layout.sidebar.collapsed,
                width,
            )
        });
        if collapsed {
            return div().id("sidebar").into_any_element();
        }
        if cursor != self.shown_cursor {
            if let Some(index) = cursor
                .as_ref()
                .and_then(|key| rows.iter().position(|row| row.key.as_ref() == Some(key)))
            {
                self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
            }
            self.shown_cursor.clone_from(&cursor);
        }
        let ui = self.ui.clone();
        let focus = self.focus.clone();
        let list = {
            let rows = rows.clone();
            uniform_list("sidebar-rows", rows.len(), move |range, _, _| {
                range
                    .filter_map(|index| {
                        let row = rows.get(index)?;
                        let at_cursor = row.key.is_some() && row.key == cursor;
                        let is_current = row.key.is_some() && row.key == current;
                        Some(draw_row(&ui, &focus, row, index, at_cursor, is_current))
                    })
                    .collect()
            })
            .track_scroll(&self.scroll)
            .flex_1()
        };
        let ui = &self.ui;
        widgets::col()
            .id("sidebar")
            .role(Role::Navigation)
            .aria_label("Sidebar")
            .test_support()
            .track_focus(&self.focus)
            .key_context(keys::SIDEBAR)
            .on_action(cx.listener(|this, _: &keys::Up, _, cx| this.step(Move::Up, cx)))
            .on_action(cx.listener(|this, _: &keys::Down, _, cx| this.step(Move::Down, cx)))
            .on_action(cx.listener(|this, _: &keys::Out, _, cx| this.step(Move::Out, cx)))
            .on_action(cx.listener(|this, _: &keys::In, _, cx| this.step(Move::In, cx)))
            .on_action(cx.listener(|this, _: &keys::First, _, cx| this.step(Move::First, cx)))
            .on_action(cx.listener(|this, _: &keys::Last, _, cx| this.step(Move::Last, cx)))
            .on_action(cx.listener(|this, _: &keys::Open, _, cx| {
                this.ui.send(AppAction::Sidebar(SidebarAction::OpenCursor), cx);
            }))
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(color(theme::SIDE))
            .border_r_1()
            .border_color(color(theme::LINE))
            .child(
                row()
                    .h(px(theme::TOP_BAR))
                    .px_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(color(theme::LINE))
                    .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Humata"))
                    .child(
                        widgets::ghost(ui, "sidebar-expand-all", "", AppAction::Sidebar(SidebarAction::ExpandAll))
                            .icon(IconName::ChevronDown)
                            .xsmall()
                            .accessibility_label("Expand all")
                            .tooltip("Expand all  ⌥⌘→"),
                    )
                    .child(
                        widgets::ghost(ui, "sidebar-collapse-all", "", AppAction::Sidebar(SidebarAction::CollapseAll))
                            .icon(IconName::ChevronRight)
                            .xsmall()
                            .accessibility_label("Collapse all")
                            .tooltip("Collapse all  ⌥⌘←"),
                    )
                    .child(
                        widgets::ghost(ui, "sidebar-new-channel", "", AppAction::StartNewChannel)
                            .icon(IconName::Plus)
                            .xsmall()
                            .accessibility_label("New channel")
                            .tooltip("New channel  ⇧⌘N"),
                    )
                    .child(
                        widgets::ghost(ui, "sidebar-hide", "", AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)))
                            .icon(IconName::PanelLeft)
                            .xsmall()
                            .accessibility_label("Hide sidebar")
                            .tooltip("Hide sidebar  ⌘0"),
                    ),
            )
            .child(div().flex_1().flex().flex_col().py_1().child(list))
            .child(
                div().p_2().border_t_1().border_color(color(theme::LINE)).child(
                    widgets::send(ui, "sidebar-new-session", "New session", AppAction::StartNewSession)
                        .icon(IconName::Plus)
                        .w_full()
                        .tooltip("New session  ⌘N"),
                ),
            )
            .into_any_element()
    }
}
