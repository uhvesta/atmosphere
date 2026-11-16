//! The sidebar (NV-1–5, NV-16–23): the Pinned section, then channels with
//! their sessions and forks nested under them. Only the rows in view are
//! built, so five thousand sessions scroll as smoothly as five. With focus,
//! the arrow keys move a cursor through the rows, ← and → fold and unfold,
//! and Enter opens the row.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, AppState, PaneAction, Route};
use humata2_clients::model::{MachineKind, Scope, Status};
use humata2_sidebar::{Move, Row, RowKey, RowKind, SidebarAction};

use crate::bridge::{Part, Ui};
use crate::keys;
use crate::theme;
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
        self.ui
            .send(AppAction::Sidebar(SidebarAction::Move(step)), cx);
    }
}

/// One line of the channel list: a row of the tree, with its place in the
/// tree, or the note under an open channel that has no session yet.
#[derive(Clone)]
enum Line {
    Row(usize, Row),
    NoSessions,
}

fn label(row: &Row) -> String {
    match &row.kind {
        RowKind::Section(name) => (*name).to_owned(),
        RowKind::Channel { name, .. } => name.clone(),
        RowKind::Session { title, .. } => title.clone(),
    }
}

fn row_id(row: &Row, index: usize) -> ElementId {
    match &row.key {
        Some(RowKey::Channel(id)) => ElementId::Name(format!("sidebar-channel-{}", id.0).into()),
        Some(RowKey::Session(id)) => ElementId::Name(format!("sidebar-session-{}", id.0).into()),
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
    look: Look,
    width: f32,
) -> AnyElement {
    if let RowKind::Section(name) = &entry.kind {
        return chrome::section(look, name)
            .h(px(theme::ROW))
            .into_any_element();
    }
    let Some(key) = entry.key.clone() else {
        return div().into_any_element();
    };
    let selected = if current { 0xffffff } else { look.text };
    let faint = if current { 0xffffff } else { look.tertiary };
    let mut body = row().gap(px(7.)).flex_1().min_w_0();
    let indent;
    match &entry.kind {
        RowKind::Channel { name, sessions, .. } => {
            indent = 4.;
            // A channel with no session has nothing to fold in the tree, so
            // whether it is open comes from what the person folded.
            let open = entry.expanded.unwrap_or_else(|| {
                ui.read(|s| match &entry.key {
                    Some(RowKey::Channel(id)) => !s.sidebar.folding.channels.contains(id),
                    _ => false,
                })
            });
            let fold_ui = ui.clone();
            let key = key.clone();
            body = body
                .child(
                    div()
                        .id(("sidebar-fold", index))
                        .size(px(12.))
                        .flex_shrink_0()
                        .role(Role::Button)
                        .aria_label(if open { "Fold" } else { "Unfold" })
                        .child(chrome::icon(
                            if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            faint,
                            12.,
                        ))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            fold_ui.send(AppAction::Sidebar(SidebarAction::Toggle(key.clone())), cx)
                        })
                        .test_support(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .font_weight(
                            if ui.read(|s| {
                                s.workspace.sessions.iter().any(|x| {
                                    Some(RowKey::Channel(x.channel.clone())) == entry.key
                                        && x.unread > 0
                                })
                            }) {
                                FontWeight::BOLD
                            } else {
                                FontWeight::MEDIUM
                            },
                        )
                        .child(name.clone()),
                );
            if entry.expanded != Some(true) && *sessions > 0 {
                let channel = match &entry.key {
                    Some(RowKey::Channel(id)) => Some(id),
                    _ => None,
                };
                let status = ui.read(|s| {
                    s.workspace
                        .sessions
                        .iter()
                        .filter(|x| Some(&x.channel) == channel)
                        .map(|x| x.status)
                        .min_by_key(|x| match x {
                            Status::NeedsYou => 0,
                            Status::Exited { .. } => 1,
                            Status::Working => 2,
                            _ => 3,
                        })
                        .unwrap_or(Status::Idle)
                });
                body = body.child(chrome::status(status)).child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(if current { 0xffffff } else { look.secondary }))
                        .child(sessions.to_string()),
                );
            }
        }
        RowKind::Session {
            title,
            status,
            unread,
            vip,
            ..
        } => {
            let fork = entry.depth > 1;
            indent = if entry.depth == 0 {
                8.
            } else {
                18. + f32::from(entry.depth - 1) * 16. - if fork { 19. } else { 0. }
            };
            if fork {
                body = body.child(chrome::icon(IconName::GitFork, faint, 12.));
            }
            let count = ui.read(|s| match &key {
                RowKey::Session(id) => inbox_count(s, id, *vip),
                _ => 0,
            });
            body = body.child(chrome::status(*status)).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    // Unread or selected rows read semibold (goldens 01, 16).
                    .when(*unread > 0 || current, |x| x.font_weight(FontWeight::SEMIBOLD))
                    .child(title.clone()),
            );
            if count > 0 {
                body = body.child(
                    row()
                        .gap(px(2.))
                        .child(chrome::icon(
                            IconName::Inbox,
                            if current { 0xffffff } else { 0xff453a },
                            12.,
                        ))
                        .child(div().text_size(px(11.)).child(count.to_string())),
                );
            }
        }
        RowKind::Section(_) => return div().into_any_element(),
    }
    let ui = ui.clone();
    let focus = focus.clone();
    let mut item = div()
        .id(row_id(entry, index))
        .role(Role::TreeItem)
        .aria_label(label(entry))
        .aria_selected(current)
        .h(px(theme::ROW))
        .w(px(width - 20.))
        .mx(px(10.))
        .pl(px(indent))
        .pr(px(8.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_color(rgb(selected))
        .when(current, |x| x.bg(rgb(look.selection)))
        .when(cursor && !current, |x| x.bg(rgb(look.fill)))
        .child(body)
        .on_click(move |_, window, cx| {
            window.focus(&focus, cx);
            ui.send(AppAction::Sidebar(SidebarAction::Activate(key.clone())), cx)
        });
    if let Some(open) = entry.expanded {
        item = item.aria_expanded(open);
    }
    item.test_support().into_any_element()
}

pub(crate) fn nav(
    ui: &Rc<Ui>,
    id: &'static str,
    title: &'static str,
    glyph: IconName,
    route: Route,
    on: bool,
    count: Option<usize>,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Link)
        .aria_label(title)
        .aria_selected(on)
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(25.))
        .mx(px(10.))
        .px(px(8.))
        .rounded(px(6.))
        .when(on, |x| x.bg(rgb(look.selection)).text_color(rgb(0xffffff)))
        .child(chrome::icon(
            glyph,
            if on { 0xffffff } else { look.accent },
            15.,
        ))
        .child(div().flex_1().child(title))
        .children(count.map(|n| {
            div()
                .text_size(px(11.))
                .when(!on, |x| x.text_color(rgb(look.secondary)))
                .child(n.to_string())
        }))
        .on_click(move |_, _, cx| ui.send(AppAction::Switch(route.clone()), cx))
        .test_support()
}

/// The line under an open channel that has no session yet (frame 35).
fn no_sessions(look: Look, width: f32) -> AnyElement {
    div()
        .h(px(theme::ROW))
        .w(px(width - 20.))
        .mx(px(10.))
        .pl(px(34.))
        .pr(px(8.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(rgb(look.tertiary))
        .child("No sessions yet")
        .into_any_element()
}

/// A faint line where a list has nothing yet (frame 34).
fn nothing_yet(look: Look, text: &'static str) -> Div {
    row()
        .h(px(26.))
        .mx(px(10.))
        .px(px(8.))
        .text_color(rgb(look.tertiary))
        .child(text)
}

/// The accent action that starts a list that has nothing yet (frames 34, 35).
fn start(
    ui: &Rc<Ui>,
    id: &'static str,
    text: &'static str,
    event: AppAction,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(text)
        .h(px(26.))
        .mx(px(10.))
        .px(px(8.))
        .gap(px(7.))
        .rounded(px(6.))
        .cursor_pointer()
        .text_color(rgb(look.accent))
        .child(chrome::icon(IconName::Plus, look.accent, 12.))
        .child(div().flex_1().child(text))
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
        .test_support()
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::of(cx);
        let (rows, folded, cursor, current, collapsed, width) = self.ui.read(|state| {
            let columns = state.layout.columns(
                f32::from(window.viewport_size().width),
                matches!(state.route, Route::Session(_)),
                state.panel().is_some(),
            );
            let width = if state.navigation_overlay {
                236.
            } else {
                columns.sidebar
            };
            (
                state.sidebar.rows.clone(),
                state.sidebar.folding.channels.clone(),
                state.sidebar.cursor.clone(),
                state.sidebar.current.clone(),
                columns.sidebar_hidden && !state.navigation_overlay,
                width,
            )
        });
        let channel_start = rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::Section("Channels")))
            .unwrap_or(0);
        let mut lines = Vec::new();
        for (index, row) in rows.iter().skip(channel_start + 1).enumerate() {
            let bare = matches!(row.kind, RowKind::Channel { sessions: 0, .. })
                && !matches!(&row.key, Some(RowKey::Channel(id)) if folded.contains(id));
            lines.push(Line::Row(index, row.clone()));
            if bare {
                lines.push(Line::NoSessions);
            }
        }
        if collapsed {
            return div().id("sidebar").into_any_element();
        }
        if cursor != self.shown_cursor {
            if let Some(index) = cursor.as_ref().and_then(|key| {
                lines.iter().position(
                    |line| matches!(line, Line::Row(_, row) if row.key.as_ref() == Some(key)),
                )
            }) {
                self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
            }
            self.shown_cursor.clone_from(&cursor);
        }
        let ui = self.ui.clone();
        let focus = self.focus.clone();
        let list = {
            let lines = lines.clone();
            let cursor = cursor.clone();
            let current = current.clone();
            uniform_list("sidebar-rows", lines.len(), move |range, _, _| {
                range
                    .filter_map(|at| {
                        Some(match lines.get(at)? {
                            Line::Row(index, row) => {
                                let at_cursor = row.key.is_some() && row.key == cursor;
                                let is_current = row.key.is_some() && row.key == current;
                                draw_row(
                                    &ui, &focus, row, *index, at_cursor, is_current, look, width,
                                )
                            }
                            Line::NoSessions => no_sessions(look, width),
                        })
                    })
                    .collect()
            })
            .track_scroll(&self.scroll)
            .flex_1()
        };
        let ui = &self.ui;
        let selected_host = ui.read(|s| s.hosts.selected.clone());
        // Once the workspace is here, a list with nothing in it says so and
        // offers the action that starts it (frames 34, 35).
        let (no_channels, only_this_mac) = ui.read(|s| {
            let ready = s.load == humata2_app::Load::Ready;
            (
                ready && s.workspace.channels.is_empty(),
                ready
                    && s.workspace
                        .machines
                        .iter()
                        .all(|m| matches!(m.kind, MachineKind::ThisMac)),
            )
        });
        let (route, waiting, machines) = ui.read(|s| {
            (
                s.route.clone(),
                humata2_inbox::waiting(&s.workspace),
                s.workspace
                    .machines
                    .iter()
                    .filter(|m| {
                        m.online
                            || !s
                                .workspace
                                .machine_health
                                .get(&m.id)
                                .is_some_and(|h| h.stopped)
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        });
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
                this.ui
                    .send(AppAction::Sidebar(SidebarAction::OpenCursor), cx);
            }))
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(look.sidebar))
            .border_r_1()
            .border_color(rgb(look.line))
            .text_color(rgb(look.text))
            .child(
                row()
                    .h(px(52.))
                    .pl(px(20.))
                    .pr(px(14.))
                    .gap(px(12.))
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window()
                        } else {
                            window.start_window_move()
                        }
                    })
                    .child(chrome::lights())
                    .child(div().flex_1())
                    .child(chrome::action(
                        ui,
                        "sidebar-hide",
                        "Hide sidebar",
                        IconName::PanelLeft,
                        if ui.read(|s| s.navigation_overlay) {
                            AppAction::CloseNavigationOverlay
                        } else {
                            AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar))
                        },
                        look.secondary,
                    ))
                    .child(chrome::action(
                        ui,
                        "sidebar-new-session",
                        "New session",
                        IconName::SquarePen,
                        AppAction::StartNewSession,
                        look.secondary,
                    )),
            )
            .child(
                div()
                    .id("search")
                    .role(Role::Button)
                    .aria_label("Search")
                    .mx(px(10.))
                    .mt(px(-4.))
                    .mb(px(6.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .bg(rgb(look.fill))
                    .text_color(rgb(look.tertiary))
                    .child(chrome::icon(IconName::Search, look.tertiary, 12.))
                    .child(div().flex_1().child("Search"))
                    .child(div().text_size(px(11.)).child("⌘K"))
                    .on_click({
                        let ui = ui.clone();
                        move |_, _, cx| ui.send(AppAction::OpenPalette, cx)
                    })
                    .test_support(),
            )
            // FIND (NV-20 to NV-25): the host's pins, in the person's order.
            .children({
                let send_pinned: crate::sidebar_pinned::SendPinned = Rc::new({
                    let ui = ui.clone();
                    move |action, cx: &mut App| ui.send(AppAction::Pinned(action), cx)
                });
                let shown = current.as_ref().map(|key| match key {
                    RowKey::Session(id) => humata2_clients::marks::Thing::Session(id.clone()),
                    RowKey::Channel(id) => humata2_clients::marks::Thing::Channel(id.clone()),
                });
                ui.read(|s| {
                    crate::sidebar_pinned::section(&s.pinned, &s.workspace, shown.as_ref(), look, &send_pinned)
                })
            })
            .child(div().h(px(4.)))
            .child(nav(
                ui,
                "rail-home",
                "Home",
                IconName::House,
                Route::Home,
                route == Route::Home,
                None,
                look,
            ))
            .child(nav(
                ui,
                "rail-inbox",
                "Inbox",
                IconName::Inbox,
                Route::Inbox,
                route == Route::Inbox,
                (waiting > 0).then_some(waiting),
                look,
            ))
            .child(nav(
                ui,
                "rail-board",
                "Board",
                IconName::SquareKanban,
                Route::Board(Scope::Global),
                matches!(route, Route::Board(_)),
                None,
                look,
            ))
            .child(ui.read(|s| crate::sidebar_desk::rows(ui, s, look)))
            .child(chrome::section(look, "Channels").child(chrome::action(
                ui,
                "sidebar-new-channel",
                "New channel",
                IconName::Plus,
                AppAction::StartNewChannel,
                look.secondary,
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .h(px(lines.len() as f32 * theme::ROW))
                    .flex_shrink(1.)
                    .child(list),
            )
            .when(no_channels, |bar| {
                bar.child(nothing_yet(look, "No channels yet")).child(start(
                    ui,
                    "sidebar-first-channel",
                    "New channel…",
                    AppAction::StartNewChannel,
                    look,
                ))
            })
            .child(chrome::section(look, "Hosts"))
            .children(machines.into_iter().enumerate().map(|(index, m)| {
                let ui = ui.clone();
                row()
                    .id(("sidebar-machine", index))
                    .role(Role::Link)
                    .aria_label(m.name.clone())
                    .h(px(26.))
                    .mx(px(10.))
                    .px(px(8.))
                    .gap(px(7.))
                    .child(chrome::status(if m.online {
                        Status::Working
                    } else {
                        Status::Offline
                    }))
                    .child(div().flex_1().child(m.name))
                    .when(!m.online, |x| {
                        x.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child("offline"),
                        )
                    })
                    .when(
                        (route == Route::Hosts && selected_host.as_ref() == Some(&m.id))
                            || route == Route::Machine(m.id.clone()),
                        |x| {
                            x.rounded(px(6.))
                                .bg(rgb(look.selection))
                                .text_color(rgb(0xffffff))
                        },
                    )
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Switch(Route::Machine(m.id.clone())), cx);
                    })
                    .test_support()
            }))
            .when(only_this_mac, |bar| {
                bar.child(start(
                    ui,
                    "sidebar-add-machine",
                    "Add a machine…",
                    AppAction::Hosts(humata2_hosts::HostsAction::StartAdd),
                    look,
                ))
            })
            .child(div().flex_1())
            .child(div().pb(px(10.)).child(nav(
                ui,
                "rail-settings",
                "Settings",
                IconName::Settings,
                Route::Settings,
                route == Route::Settings,
                None,
                look,
            )))
            .into_any_element()
    }
}

/// Only a VIP session shows its inbox count; it covers the pending requests
/// of its forks, however deep.
fn inbox_count(s: &AppState, id: &humata2_clients::model::SessionId, vip: bool) -> usize {
    if !vip {
        return 0;
    }
    let sessions = &s.workspace.sessions;
    let descends = |start: &humata2_clients::model::SessionId| {
        let mut at = Some(start);
        for _ in 0..=sessions.len() {
            let Some(cur) = at else { return false };
            if cur == id {
                return true;
            }
            at = sessions
                .iter()
                .find(|x| &x.id == cur)
                .and_then(|x| x.parent.as_ref());
        }
        false
    };
    s.workspace
        .requests
        .iter()
        .filter(|r| r.answer.is_none() && descends(&r.session))
        .count()
}
