//! The sidebar (NV-1–5, NV-16–23): the Pinned section, then channels with
//! their sessions and forks nested under them. Only the rows in view are
//! built, so five thousand sessions scroll as smoothly as five. With focus,
//! the arrow keys move a cursor through the rows, ← and → fold and unfold,
//! and Enter opens the row.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, AppState, PaneAction, Route};
use humata2_clients::model::{MachineKind, SessionId, Status};
use humata2_sidebar::{Move, Row, RowKey, RowKind, SidebarAction};
use humata2_ui_machines::machine_page::select_mode::{
    self, MassDeleteAction, Snapshot, Surface, Tick,
};

use humata2_ui_core::bridge::{Part, Ui};
use humata2_ui_core::keys;
use humata2_ui_core::theme;
use humata2_ui_core::widgets::{self, row};

/// Right-click context menu on a sidebar session row.
#[derive(Clone)]
struct RowMenu {
    session: SessionId,
    y: Pixels,
}

/// The sidebar's view.
pub struct Sidebar {
    ui: Rc<Ui>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    shown_cursor: Option<RowKey>,
    row_menu: Entity<Option<RowMenu>>,
    _row_menu_observer: Subscription,
}

impl Sidebar {
    /// The sidebar on `ui`, taking focus through `focus`.
    pub fn new(ui: Rc<Ui>, focus: FocusHandle, cx: &mut Context<Self>) -> Self {
        ui.attach(Part::Sidebar, cx.entity_id());
        let row_menu = cx.new(|_| None);
        let _row_menu_observer = cx.observe(&row_menu, |_, _, cx| cx.notify());
        Self {
            ui,
            focus,
            scroll: UniformListScrollHandle::new(),
            shown_cursor: None,
            row_menu,
            _row_menu_observer,
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
    /// The Desk row under an open channel (CD-2), with its sessions' count.
    ChannelDesk(humata2_clients::model::ChannelId, usize),
    /// One of that channel's open Desk sessions.
    ChannelDeskSession(Row),
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
        Some(RowKey::ChannelDesk(id)) => {
            ElementId::Name(format!("sidebar-channel-desk-{}", id.0).into())
        }
        Some(RowKey::Desk) => ElementId::Name("sidebar-desk".into()),
        Some(RowKey::DeskMore) => ElementId::Name("sidebar-desk-more".into()),
        None => ElementId::Name(format!("sidebar-section-{index}").into()),
    }
}

/// What a session row's tick box shows in sidebar select mode.
fn session_tick(ui: &Rc<Ui>, snap: &Snapshot, id: &SessionId) -> Tick {
    let offline = ui.read(|s| {
        s.workspace
            .session(id)
            .is_some_and(|sess| s.mass_delete.listing.offline.contains(&sess.machine))
    });
    if offline || (!snap.can_pick(id) && !snap.ticked(id)) {
        Tick::Barred
    } else {
        snap.tick(id)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    ui: &Rc<Ui>,
    focus: &FocusHandle,
    entry: &Row,
    index: usize,
    cursor: bool,
    current: bool,
    select: Option<&Snapshot>,
    visible: &Rc<Vec<SessionId>>,
    row_menu: &Entity<Option<RowMenu>>,
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
    let session_id = match &key {
        RowKey::Session(id) => Some(id.clone()),
        _ => None,
    };
    let ticked = select
        .zip(session_id.as_ref())
        .is_some_and(|(snap, id)| snap.ticked(id));
    let show_current = current && !ticked;
    let selected = if show_current { 0xffffff } else { look.text };
    let faint = if show_current { 0xffffff } else { look.tertiary };
    let mut body = row().gap(px(7.)).flex_1().min_w_0();
    let mut dimmed = false;
    let mut channel_select: Option<(bool, Vec<MassDeleteAction>)> = None;
    let mut barred_session = false;
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
            // An unread channel is bold, its folded count too, as in the design.
            let weight = if ui.read(|s| {
                s.workspace.sessions.iter().any(|x| {
                    Some(RowKey::Channel(x.channel.clone())) == entry.key && x.unread > 0
                })
            }) {
                FontWeight::BOLD
            } else {
                FontWeight::MEDIUM
            };
            let lead = if let (Some(snap), RowKey::Channel(id)) = (select, &key) {
                let (tick, can) = snap.channel(id);
                let actions = if can {
                    snap.channel_click(id)
                } else {
                    Vec::new()
                };
                channel_select = Some((can, actions));
                div()
                    .id(("sidebar-channel-tick", index))
                    .flex_shrink_0()
                    .when(can, |box_el| box_el.cursor_pointer())
                    .child(select_mode::tick_box(look, 13., tick))
                    .into_any_element()
            } else {
                let fold_ui = ui.clone();
                let key = key.clone();
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
                    .test_support()
                    .into_any_element()
            };
            body = body.child(lead).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .font_weight(weight)
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
                        // The design puts 5 pt, not the row's 7, between the dot and the count.
                        .ml(px(-2.))
                        .text_size(px(11.))
                        .font_weight(weight)
                        .text_color(rgb(if show_current { 0xffffff } else { look.secondary }))
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
            if let (Some(snap), Some(id)) = (select, session_id.as_ref()) {
                // MASSDELETE (D875): every tick box sits in the same left
                // gutter; forks indent after the box.
                indent = 4.;
                let tick = session_tick(ui, snap, id);
                barred_session = tick == Tick::Barred;
                body = body.child(select_mode::tick_box(look, 13., tick));
                if fork {
                    let extra = f32::from(entry.depth.saturating_sub(2)) * 14.;
                    body = body.child(
                        div()
                            .ml(px(extra))
                            .mr(px(-2.))
                            .flex_shrink_0()
                            .child(chrome::icon(IconName::GitFork, faint, 12.)),
                    );
                }
            } else {
                indent = if entry.depth == 0 {
                    8.
                } else {
                    18. + f32::from(entry.depth - 1) * 16. - if fork { 19. } else { 0. }
                };
                if fork {
                    // The design sits the fork glyph 2 pt nearer the status dot than the
                    // row's gap, with the dot and title where they are.
                    body = body.child(
                        div()
                            .ml(px(2.))
                            .mr(px(-2.))
                            .flex_shrink_0()
                            .child(chrome::icon(IconName::GitFork, faint, 12.)),
                    );
                }
            }
            // A session whose machine's sessions were stopped shows the square,
            // dimmed, with no inbox count: its requests ended (frame 73).
            // A session the person stopped (AgentStopped) draws its square
            // through `chrome::status` but keeps its row, selection and count.
            let halted = *status == Status::Stopped
                || ui.read(|s| match &key {
                    RowKey::Session(id) => stopped_on_machine(s, id),
                    _ => false,
                });
            let count = ui.read(|s| match &key {
                RowKey::Session(id) if !halted => inbox_count(s, id, *vip),
                _ => 0,
            });
            if halted {
                dimmed = true;
            }
            body = body
                .child(if halted {
                    chrome::stopped()
                } else {
                    chrome::status(*status)
                })
                .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    // Unread rows read semibold, and so does the selected row while
                    // it has requests waiting (golden 15 bolds it, golden 16 doesn't).
                    .when(*unread > 0 || (show_current && count > 0), |x| {
                        x.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(title.clone()),
            );
            if count > 0 {
                body = body.child(
                    row()
                        .gap(px(2.))
                        .child(chrome::icon(
                            IconName::Inbox,
                            if show_current { 0xffffff } else { 0xff453a },
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
    let selecting = select.is_some();
    let visible = visible.clone();
    let menu = row_menu.clone();
    let right_click_id = session_id.clone();
    let mut item = div()
        .id(row_id(entry, index))
        .role(Role::TreeItem)
        .aria_label(label(entry))
        .aria_selected(show_current || ticked)
        .h(px(theme::ROW))
        .w(px(width - 20.))
        .mx(px(10.))
        .pl(px(indent))
        .pr(px(8.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .text_color(rgb(selected))
        .when(ticked, |x| x.bg(select_mode::tint(look)))
        .when(show_current, |x| x.bg(rgb(look.selection)))
        .when(cursor && !show_current && !ticked, |x| x.bg(rgb(look.fill)))
        .when(dimmed, |x| x.opacity(0.5))
        .when(selecting && !barred_session && channel_select.as_ref().is_none_or(|(can, _)| *can), |x| {
            x.cursor_pointer()
        })
        .child(body)
        .on_click(move |event, window, cx| {
            if let Some((can, actions)) = &channel_select {
                cx.stop_propagation();
                if *can {
                    for action in actions {
                        ui.send(select_mode::act(action.clone()), cx);
                    }
                }
                return;
            }
            if let Some(id) = &session_id {
                let keys = event.modifiers();
                if selecting {
                    cx.stop_propagation();
                    if barred_session {
                        return;
                    }
                    let action = if keys.shift {
                        MassDeleteAction::RangeTo {
                            to: id.clone(),
                            visible: visible.to_vec(),
                        }
                    } else {
                        MassDeleteAction::Toggle(id.clone())
                    };
                    ui.send(select_mode::act(action), cx);
                    return;
                }
                if keys.platform {
                    cx.stop_propagation();
                    ui.send(
                        select_mode::act(MassDeleteAction::Enter {
                            surface: Surface::Sidebar,
                            first: id.clone(),
                        }),
                        cx,
                    );
                    return;
                }
            }
            window.focus(&focus, cx);
            ui.send(AppAction::Sidebar(SidebarAction::Activate(key.clone())), cx)
        });
    if !selecting
        && let Some(id) = right_click_id
    {
        item = item.on_mouse_down(MouseButton::Right, move |event, _, cx| {
            cx.stop_propagation();
            menu.update(cx, |m, cx| {
                *m = Some(RowMenu {
                    session: id.clone(),
                    y: event.position.y,
                });
                cx.notify();
            });
        });
    }
    if let Some(open) = entry.expanded {
        item = item.aria_expanded(open);
    }
    item.test_support().into_any_element()
}

fn nav(
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
        .h(px(26.))
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

/// The channel list's height, in points, above which it stops taking its
/// natural height and fills what the other rows leave instead.
const FILL_ABOVE: f32 = 1000.;

/// Whether the channel list is long enough to fill the column.
fn channels_fill(lines: usize) -> bool {
    lines as f32 * theme::ROW > FILL_ABOVE
}

/// The box around the channel list. A short list is as tall as its rows and
/// gives up height only after every other row has shrunk to its least, as the
/// design's channel block does (its weight is nil next to theirs). A long list
/// fills the rest of the column and scrolls, so Settings stays in view.
fn channels_box(lines: usize, list: impl IntoElement) -> Div {
    let frame = widgets::col().min_h_0();
    if channels_fill(lines) {
        frame.flex_1().child(list)
    } else {
        frame
            .h(px(lines as f32 * theme::ROW))
            .flex_shrink(SHRINK_LAST)
            .child(list)
    }
}

/// A shrink factor so small the other rows absorb the overflow first.
const SHRINK_LAST: f32 = 0.0001;

/// The Channels section's + (frame 16 draws it at 12 pt).
fn new_channel(ui: &Rc<Ui>, look: Look) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("sidebar-new-channel")
        .role(Role::Button)
        .aria_label("New channel")
        .size(px(12.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(chrome::icon(IconName::Plus, look.secondary, 12.))
        .on_click(move |_, _, cx| ui.send(AppAction::StartNewChannel, cx))
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
        let channel_desks = self.ui.read(|state| state.sidebar.channel_desks.clone());
        let mut lines = Vec::new();
        for (index, row) in rows.iter().skip(channel_start + 1).enumerate() {
            let bare = matches!(row.kind, RowKind::Channel { sessions: 0, .. })
                && !matches!(&row.key, Some(RowKey::Channel(id)) if folded.contains(id));
            lines.push(Line::Row(index, row.clone()));
            if bare {
                lines.push(Line::NoSessions);
            }
            // CD-2: a Desk row above the channel's sessions while it has open
            // channel Desk sessions, with them under it.
            if let Some(RowKey::Channel(id)) = &row.key
                && !folded.contains(id)
                && let Some(desk) = channel_desks.get(id)
                && !desk.is_empty()
            {
                lines.push(Line::ChannelDesk(id.clone(), desk.len()));
                lines.extend(desk.iter().cloned().map(Line::ChannelDeskSession));
            }
        }
        if collapsed {
            return div().id("sidebar").into_any_element();
        }
        if cursor != self.shown_cursor {
            if let Some(index) = cursor.as_ref().and_then(|key| {
                lines.iter().position(
                    |line| {
                        matches!(
                            line,
                            Line::Row(_, row) | Line::ChannelDeskSession(row)
                                if row.key.as_ref() == Some(key)
                        )
                    },
                )
            }) {
                self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
            }
            self.shown_cursor.clone_from(&cursor);
        }
        let select = Snapshot::on(&self.ui, &Surface::Sidebar);
        let selecting = select.is_some();
        let visible: Rc<Vec<SessionId>> = Rc::new(visible_from_lines(&lines));
        let ui = self.ui.clone();
        let focus = self.focus.clone();
        let row_menu = self.row_menu.clone();
        let list = {
            let lines = lines.clone();
            let cursor = cursor.clone();
            let current = current.clone();
            let select = select.clone();
            let visible = visible.clone();
            uniform_list("sidebar-rows", lines.len(), move |range, _, _| {
                range
                    .filter_map(|at| {
                        Some(match lines.get(at)? {
                            Line::Row(index, row) => {
                                let at_cursor = row.key.is_some() && row.key == cursor;
                                let is_current = row.key.is_some() && row.key == current;
                                draw_row(
                                    &ui,
                                    &focus,
                                    row,
                                    *index,
                                    at_cursor,
                                    is_current,
                                    select.as_ref(),
                                    &visible,
                                    &row_menu,
                                    look,
                                    width,
                                )
                            }
                            Line::NoSessions => no_sessions(look, width),
                            Line::ChannelDesk(id, count) => {
                                let at_cursor =
                                    cursor.as_ref() == Some(&RowKey::ChannelDesk(id.clone()));
                                humata2_ui_desk::sidebar_desk::channel_row(&ui, id, *count, at_cursor, look)
                            }
                            Line::ChannelDeskSession(entry) => ui
                                .read(|state| humata2_ui_desk::sidebar_desk::session(&ui, state, entry, look))
                                .unwrap_or_else(|| div().into_any_element()),
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
        let (route, waiting, reviews, machines) = ui.read(|s| {
            (
                s.route.clone(),
                humata2_inbox::waiting(&s.workspace) + s.pending_cli_logins.len(),
                reviews_count(s),
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
        // FIND (NV-20 to NV-25): the host's pins, in the person's order.
        let send_pinned: crate::sidebar_pinned::SendPinned = Rc::new({
            let ui = ui.clone();
            move |action, cx: &mut App| ui.send(AppAction::Pinned(action), cx)
        });
        let shown = current.as_ref().and_then(|key| match key {
            RowKey::Session(id) => Some(humata2_clients::marks::Thing::Session(id.clone())),
            RowKey::Channel(id) | RowKey::ChannelDesk(id) => {
                Some(humata2_clients::marks::Thing::Channel(id.clone()))
            }
            RowKey::Desk | RowKey::DeskMore => None,
        });
        let pinned = ui.read(|s| {
            crate::sidebar_pinned::section(&s.pinned, &s.workspace, shown.as_ref(), look, &send_pinned)
        });
        let pinned_gap = if pinned.is_some() { 5.5 } else { 4. };
        let open_menu = (!selecting)
            .then(|| self.row_menu.read(cx).clone())
            .flatten();
        let row_menu_handle = self.row_menu.clone();
        widgets::col()
            .id("sidebar")
            .relative()
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
            .text_color(rgb(look.text))
            // The divider is drawn over the column's last point instead of taking
            // it, so right-aligned items sit where the design puts them.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(1.))
                    .bg(rgb(look.line)),
            )
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
                    .child({
                        let mac = cx.platform_info().is_mac();
                        let tip: SharedString =
                            format!("Hide sidebar  {}", keys::label("⌘B", mac)).into();
                        let ui = ui.clone();
                        let action = if ui.read(|s| s.navigation_overlay) {
                            AppAction::CloseNavigationOverlay
                        } else {
                            AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar))
                        };
                        div()
                            .id("sidebar-hide")
                            .role(Role::Button)
                            .aria_label("Hide sidebar")
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                    .build(window, cx)
                            })
                            .size(px(15.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .child(chrome::icon(IconName::PanelLeft, look.secondary, 15.))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| ui.send(action.clone(), cx))
                            .test_support()
                    })
                    .child(if selecting {
                        div()
                            .id("sidebar-new-session")
                            .role(Role::Button)
                            .aria_label("New session")
                            .size(px(15.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.4)
                            .child(chrome::icon(IconName::SquarePen, look.secondary, 15.))
                            .test_support()
                            .into_any_element()
                    } else {
                        chrome::action(
                            ui,
                            "sidebar-new-session",
                            "New session",
                            IconName::SquarePen,
                            AppAction::StartNewSession,
                            look.secondary,
                        )
                        .into_any_element()
                    }),
            )
            .child(
                div()
                    .id("search")
                    .role(Role::Button)
                    .aria_label("Search")
                    .mx(px(10.))
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
            .children(pinned)
            // Frame 15 leaves 5.5 pt between the last pin and Home; 4 pt without
            // pins. It never shrinks, or the full column eats it.
            .child(div().flex_shrink_0().h(px(pinned_gap)))
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
                "Agent inbox",
                IconName::Inbox,
                Route::Inbox,
                route == Route::Inbox,
                (waiting > 0).then_some(waiting),
                look,
            ))
            .child(nav(
                ui,
                "rail-reviews",
                "Reviews",
                IconName::GitFork,
                Route::Reviews,
                route == Route::Reviews,
                (reviews > 0).then_some(reviews),
                look,
            ))
            .child(nav(
                ui,
                "rail-board",
                "To-do",
                IconName::SquareKanban,
                Route::Todo,
                route == Route::Todo,
                None,
                look,
            ))
            .child(ui.read(|s| humata2_ui_desk::sidebar_desk::rows(ui, s, look)))
            .child(chrome::section(look, "Channels").child(new_channel(ui, look)))
            .child(channels_box(lines.len(), list))
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
                // Its stop reached it: a stop still on its way keeps the offline dot.
                let halted = m.stop.as_ref().is_some_and(|stop| stop.delivered);
                let selected = !selecting
                    && ((route == Route::Hosts && selected_host.as_ref() == Some(&m.id))
                        || route == Route::Machine(m.id.clone()));
                let machine_select = select.as_ref().map(|snap| {
                    let (tick, can) = snap.machine(&m.id);
                    let actions = if can {
                        snap.machine_click(&m.id)
                    } else {
                        Vec::new()
                    };
                    (tick, can, actions)
                });
                let can_tick = machine_select.as_ref().is_some_and(|(_, can, _)| *can);
                row()
                    .id(("sidebar-machine", index))
                    .role(Role::Link)
                    .aria_label(m.name.clone())
                    .h(px(26.))
                    .mx(px(10.))
                    .pl(px(if selecting { 4. } else { 8. }))
                    .pr(px(8.))
                    .gap(px(7.))
                    .when(can_tick, |x| x.cursor_pointer())
                    .children(machine_select.as_ref().map(|(tick, _, _)| {
                        select_mode::tick_box(look, 13., *tick)
                    }))
                    .child(if halted {
                        chrome::stopped()
                    } else {
                        chrome::status(if m.online {
                            Status::Working
                        } else {
                            Status::Offline
                        })
                    })
                    .child(div().flex_1().child(m.name))
                    .when(!m.online && !halted, |x| {
                        x.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(if selected { 0xffffff } else { look.secondary }))
                                .child("offline"),
                        )
                    })
                    .when(selected, |x| {
                        x.rounded(px(6.))
                            .bg(rgb(look.selection))
                            .text_color(rgb(0xffffff))
                    })
                    .on_click(move |_, _, cx| {
                        if let Some((_, can, actions)) = &machine_select {
                            cx.stop_propagation();
                            if *can {
                                for action in actions {
                                    ui.send(select_mode::act(action.clone()), cx);
                                }
                            }
                        } else {
                            ui.send(AppAction::Switch(Route::Machine(m.id.clone())), cx);
                        }
                    })
                    .test_support()
            }))
            // The Hosts page's header offers the same action, so the
            // sidebar leaves it out there: one "Add a machine…" per window.
            .when(only_this_mac && route != Route::Hosts, |bar| {
                bar.child(start(
                    ui,
                    "sidebar-add-machine",
                    "Add a machine…",
                    AppAction::Hosts(humata2_hosts::HostsAction::StartAdd),
                    look,
                ))
            })
            .when(!channels_fill(lines.len()), |bar| bar.child(div().flex_1()))
            .children(select.as_ref().map(|snap| select_mode::card(ui, snap, look)))
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
            .when_some(open_menu, |bar, menu| {
                let close_left = row_menu_handle.clone();
                let close_right = row_menu_handle.clone();
                let close_pick = row_menu_handle.clone();
                let ui = ui.clone();
                let session = menu.session;
                let fill = Hsla::from(rgb(look.fill));
                let scrim = div()
                    .id("sidebar-row-menu-scrim")
                    .absolute()
                    .top(px(-10_000.))
                    .left(px(-10_000.))
                    .w(px(20_000.))
                    .h(px(20_000.))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        close_left.update(cx, |m, cx| {
                            *m = None;
                            cx.notify();
                        });
                    })
                    .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                        cx.stop_propagation();
                        close_right.update(cx, |m, cx| {
                            *m = None;
                            cx.notify();
                        });
                    });
                let card = crate::action_menu::card("sidebar-row-menu", 176., look)
                    .role(Role::Menu)
                    .aria_label("Session actions")
                    .left(px(10.))
                    .top(px(f32::from(menu.y).max(8.)))
                    .child(
                        Button::new("sidebar-row-select-sessions")
                            .custom(
                                ButtonCustomVariant::new(cx)
                                    .foreground(rgb(look.text).into())
                                    .hover(fill)
                                    .active(fill),
                            )
                            .accessibility_label("Select sessions\u{2026}")
                            .small()
                            .w_full()
                            .h(px(26.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .child(
                                div()
                                    .w_full()
                                    .text_size(px(12.5))
                                    .child("Select sessions\u{2026}"),
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                close_pick.update(cx, |m, cx| {
                                    *m = None;
                                    cx.notify();
                                });
                                ui.send(
                                    select_mode::act(MassDeleteAction::Enter {
                                        surface: Surface::Sidebar,
                                        first: session.clone(),
                                    }),
                                    cx,
                                );
                            }),
                    )
                    .test_support();
                bar.child(scrim).child(card)
            })
            .into_any_element()
    }
}

/// The visible session rows in the sidebar, top to bottom, for Shift-click
/// range selection.
fn visible_from_lines(lines: &[Line]) -> Vec<SessionId> {
    lines
        .iter()
        .filter_map(|line| match line {
            Line::Row(_, row) => match &row.key {
                Some(RowKey::Session(id)) => Some(id.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Whether the machine `id` runs on has its sessions stopped and the stop
/// delivered.
fn stopped_on_machine(s: &AppState, id: &humata2_clients::model::SessionId) -> bool {
    s.workspace
        .sessions
        .iter()
        .find(|session| &session.id == id)
        .and_then(|session| s.workspace.machine(&session.machine))
        .is_some_and(|machine| machine.stop.as_ref().is_some_and(|stop| stop.delivered))
}

/// Count of items waiting in the dedicated `Reviews` inbox (local review asks +
/// GitHub review requests + open watched PRs + open PR review sessions).
fn reviews_count(s: &AppState) -> usize {
    let mut seen_sessions = std::collections::HashSet::new();
    let mut seen_prs = std::collections::HashSet::new();
    let mut count = 0;

    for entry in &s.todo.items {
        let todo = &entry.todo;
        if todo.closed.is_some() || !todo.state.is_active() {
            continue;
        }
        if let Some(ask) = &todo.ask
            && matches!(ask.kind, humata2_todo::AskKind::ReviewRequest { .. })
        {
            if let Some(sid) = todo.sessions.first() {
                seen_sessions.insert(SessionId(sid.clone()));
            }
            count += 1;
        }
        if let Some(review) = &todo.review {
            for pr in &review.repo_prs {
                seen_prs.insert(pr.clone());
            }
            count += 1;
        }
        for pr in &todo.watched_prs {
            if !pr.merged && !pr.closed_without_merge && seen_prs.insert(pr.repo_pr.clone()) {
                count += 1;
            }
        }
    }
    for session in &s.workspace.sessions {
        if !session.archived
            && session.pull_request_review
            && seen_sessions.insert(session.id.clone())
        {
            count += 1;
        }
    }
    count
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

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_clients::model::{Agent, ChannelId};

    fn session_row(id: &str) -> Row {
        Row {
            key: Some(RowKey::Session(SessionId(id.to_owned()))),
            depth: 1,
            kind: RowKind::Session {
                title: id.to_owned(),
                status: Status::Idle,
                agent: Agent::Claude,
                unread: 0,
                vip: false,
                recovered: false,
            },
            expanded: None,
        }
    }

    #[::core::prelude::v1::test]
    fn visible_from_lines_keeps_only_channel_sessions_in_order() {
        let ch = ChannelId("c1".to_owned());
        let lines = vec![
            Line::Row(
                0,
                Row {
                    key: Some(RowKey::Channel(ch.clone())),
                    depth: 0,
                    kind: RowKind::Channel {
                        name: "payments".to_owned(),
                        sessions: 2,
                        needs_you: 0,
                    },
                    expanded: Some(true),
                },
            ),
            Line::ChannelDesk(ch, 1),
            Line::ChannelDeskSession(session_row("desk-1")),
            Line::Row(1, session_row("s1")),
            Line::Row(2, session_row("s2")),
            Line::NoSessions,
        ];
        assert_eq!(
            visible_from_lines(&lines),
            vec![SessionId("s1".to_owned()), SessionId("s2".to_owned())]
        );
    }
}
