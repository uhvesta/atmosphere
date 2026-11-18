//! The live Board, its table/dashboard and item drawer (frames 21–24).
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, push_primary, push_quiet, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    Sizable as _,
    button::Button,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_board::{Ask, BoardAction as A, Choice, ChannelView, Filter, Mode, PickerKind};
use humata2_clients::model::{Agent, Answer, Item, ItemId, ItemStatus as S, Scope};
use humata2_inbox::InboxAction;
use std::rc::Rc;

pub(crate) struct Inputs {
    filter: Entity<InputState>,
    title: Entity<InputState>,
    picker: Entity<InputState>,
    _subscriptions: [Subscription; 3],
}
impl Inputs {
    pub(crate) fn new(
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<crate::shell::Shell>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Item title"));
        let picker = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let one = ui.clone();
        let two = ui.clone();
        let three = ui.clone();
        let subs = [
            cx.subscribe(&filter, move |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    one.send(
                        AppAction::Board(A::FilterText(input.read(cx).value().to_string())),
                        cx,
                    );
                }
            }),
            cx.subscribe(&title, move |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    two.send(
                        AppAction::Board(A::NewTitle(input.read(cx).value().to_string())),
                        cx,
                    );
                }
            }),
            cx.subscribe(&picker, move |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    three.send(
                        AppAction::Board(A::PickerQuery(input.read(cx).value().to_string())),
                        cx,
                    );
                }
            }),
        ];
        Self {
            filter,
            title,
            picker,
            _subscriptions: subs,
        }
    }
    pub(crate) fn sync(&self, state: &AppState, window: &mut Window, cx: &mut App) {
        for (input, value) in [
            (&self.filter, state.board.view.query.as_str()),
            (&self.title, state.board.new_title.as_deref().unwrap_or("")),
            (
                &self.picker,
                state.board.picker.as_ref().map_or("", |p| p.query.as_str()),
            ),
        ] {
            if input.read(cx).value().as_ref() != value {
                input.update(cx, |i, cx| i.set_value(value.to_owned(), window, cx));
            }
        }
    }
}
fn secondary(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_color(rgb(look.secondary))
        .text_size(px(11.5))
        .child(text.into())
}
/// A small section heading.
fn heading(look: Look, title: &'static str) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(title)
}
fn card(look: Look) -> Div {
    col()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}
/// The frames' push button, filled when `primary`, sending `action`.
fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
    primary: bool,
) -> Button {
    if primary {
        push_primary(ui, id, label, action, look)
    } else {
        push_quiet(ui, id, label, action, look)
    }
}
/// A pop-up menu button: its value, then the up-down chevrons.
fn popup(
    ui: &Rc<Ui>,
    id: &'static str,
    label: impl Into<SharedString>,
    action: A,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    let label = label.into();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_shrink_0()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .cursor_pointer()
        .child(label)
        .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
        .on_click(move |_, _, cx| ui.send(AppAction::Board(action.clone()), cx))
        .test_support()
}
/// A bare icon that sends a board action when clicked.
fn glyph(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    name: IconName,
    action: A,
    color: u32,
    size: f32,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex_shrink_0()
        .cursor_pointer()
        .child(chrome::icon(name, color, size))
        .on_click(move |_, _, cx| ui.send(AppAction::Board(action.clone()), cx))
        .test_support()
}
fn act(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: A,
    look: Look,
) -> Button {
    button(ui, id, label, AppAction::Board(action), look, false)
}
fn view_label(state: &AppState, view: &ChannelView) -> String {
    let session_title = |id: &humata2_clients::model::SessionId| {
        state
            .workspace
            .session(id)
            .map_or_else(|| id.to_string(), |s| s.title.clone())
    };
    match view {
        ChannelView::Channel => "Channel board".into(),
        ChannelView::WithSessions => "Channel + every session board".into(),
        ChannelView::SessionAndForks(id) => format!("{} and its forks", session_title(id)),
        ChannelView::OneSession(id) => format!("{}'s board", session_title(id)),
    }
}
fn scope_name(state: &AppState) -> String {
    match &state.board.scope {
        Scope::Global => {
            if state.board.view.global_only {
                "Global items only".into()
            } else {
                "All channels".into()
            }
        }
        Scope::Channel(id) => {
            let name = state
                .workspace
                .channel(id)
                .map_or_else(|| id.to_string(), |c| format!("# {}", c.name));
            match state.board.channel_view() {
                ChannelView::Channel => name,
                view => format!("{name} · {}", view_label(state, &view)),
            }
        }
        Scope::Session { channel, .. } => state
            .workspace
            .channel(channel)
            .map_or_else(|| channel.to_string(), |c| format!("# {}", c.name)),
    }
}

pub(crate) fn toolbar(ui: &Rc<Ui>, state: &AppState, inputs: &Inputs, look: Look) -> Div {
    let scope = scope_name(state);
    row()
        .h(px(52.))
        .flex_shrink_0()
        .gap(px(10.))
        .px(px(20.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Board"),
        )
        .child(
            secondary(look, scope.clone())
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap(),
        )
        .child(popup(
            ui,
            "board-scope",
            format!("Scope: {scope}"),
            A::ScopeMenu,
            look,
        ))
        .child(
            row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(
                [
                    (Mode::Kanban, "Board"),
                    (Mode::Table, "Table"),
                    (Mode::Dashboard, "Dashboard"),
                ]
                .into_iter()
                .map(|(mode, name)| {
                    let ui = ui.clone();
                    div()
                        .id(SharedString::from(format!("board-mode-{name}")))
                        .role(Role::Tab)
                        .aria_label(name)
                        .aria_selected(mode == state.board.view.mode)
                        .px(px(12.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .text_size(px(11.5))
                        .when(mode == state.board.view.mode, |x| {
                            x.bg(rgb(look.card)).shadow_sm()
                        })
                        .child(name)
                        .on_click(move |_, _, cx| ui.send(AppAction::Board(A::Mode(mode)), cx))
                        .test_support()
                }),
            ),
        )
        .child(
            div()
                .id("board-filter-input")
                .w(px(150.))
                .flex_shrink_0()
                .child(
                    Input::new(&inputs.filter)
                        .small()
                        .min_h(px(26.))
                        .rounded(px(6.))
                        .bg(rgb(look.card))
                        .border_color(rgb(look.line))
                        .text_size(px(12.5)),
                )
                .test_support(),
        )
        .child(button(
            ui,
            "board-new-item",
            "New item",
            AppAction::Board(A::StartNew),
            look,
            true,
        ))
}
fn session_chip(state: &AppState, item: &Item, look: Look) -> Option<Div> {
    item.session
        .as_ref()
        .and_then(|id| state.workspace.session(id))
        .map(|s| {
            row()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .gap(px(5.))
                .px(px(6.))
                .py(px(1.))
                .rounded(px(4.))
                .bg(rgb(look.fill))
                .text_size(px(11.))
                .text_color(rgb(look.secondary))
                .when(item.status != S::Done, |x| {
                    x.child(chrome::status(s.status))
                })
                .child(s.title.clone())
        })
        .or_else(|| {
            state
                .workspace
                .item_details
                .get(&item.id)
                .and_then(|d| d.session_label.as_ref())
                .map(|label| {
                    row()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(rgb(look.fill))
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .text_color(rgb(look.secondary))
                        .child(label.clone())
                })
        })
}
#[derive(Clone)]
struct DragItem {
    id: ItemId,
    title: String,
}
impl Render for DragItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(px(8.))
            .rounded(px(6.))
            .bg(rgb(0x323235))
            .text_color(rgb(0xffffff))
            .child(self.title.clone())
    }
}
fn item_card(ui: &Rc<Ui>, state: &AppState, item: &Item, look: Look) -> impl IntoElement {
    let id = item.id.clone();
    let send = ui.clone();
    let asking = state.board.request(item).is_some();
    card(look)
        .id(SharedString::from(format!("board-card {}", item.id)))
        .role(Role::ListItem)
        .aria_label(format!("{} {}", item.id, item.title))
        .test_support()
        .min_w_0()
        .overflow_hidden()
        .p(px(9.))
        .gap(px(6.))
        .cursor_pointer()
        .when(state.board.drawer.as_ref() == Some(&item.id), |x| {
            x.border_2().border_color(rgb(look.accent))
        })
        .when(asking, |x| x.bg(rgb(look.attention_bg)))
        .child(
            row()
                .gap(px(6.))
                .child(secondary(look, item.id.to_string()))
                .child(div().flex_1())
                .child(
                    secondary(
                        look,
                        match &item.scope {
                            Scope::Global => "Global".into(),
                            Scope::Channel(id) | Scope::Session { channel: id, .. } => state
                                .workspace
                                .channel(id)
                                .map_or_else(|| id.to_string(), |c| c.name.clone()),
                        },
                    )
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap(),
                ),
        )
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(item.title.clone()),
        )
        .children(session_chip(state, item, look))
        .when(asking, |x| {
            x.child(
                row()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(crate::theme::ORANGE))
                    .child(chrome::icon(IconName::Inbox, crate::theme::ORANGE, 12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child("Asks to move to Done"),
                    )
                    .child(act(
                        ui,
                        SharedString::from(format!("board-review {}", item.id)),
                        "Review",
                        A::Open(item.id.clone()),
                        look,
                    )),
            )
        })
        .on_click(move |_, _, cx| send.send(AppAction::Board(A::Open(id.clone())), cx))
        .on_drag(
            DragItem {
                id: item.id.clone(),
                title: item.title.clone(),
            },
            |drag, _, _, cx| cx.new(|_| drag.clone()),
        )
}
fn statuses(state: &AppState) -> Vec<S> {
    let mut v = vec![S::Backlog, S::Todo, S::InProgress, S::InReview, S::Done];
    for status in [S::Blocked, S::WontDo] {
        if !state.board.column(status).is_empty() {
            v.push(status);
        }
    }
    v
}
fn lanes(ui: &Rc<Ui>, state: &AppState, look: Look) -> Stateful<Div> {
    row()
        .id("board-kanban")
        .items_stretch()
        .size_full()
        .min_h_0()
        .gap(px(10.))
        .p(px(14.))
        .text_size(px(12.5))
        .overflow_x_scroll()
        .children(statuses(state).into_iter().map(|status| {
            let items = state.board.column(status);
            let ui2 = ui.clone();
            col()
                .id(SharedString::from(format!("board-lane-{}", status.name())))
                .flex_1()
                .min_w(px(95.))
                .h_full()
                .gap(px(8.))
                .p(px(8.))
                .rounded(px(9.))
                .bg(rgb(look.panel))
                .overflow_y_scroll()
                .child(
                    row()
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(status.name()))
                        .child(secondary(look, items.len().to_string()))
                        .child(div().flex_1())
                        .child(glyph(
                            ui,
                            SharedString::from(format!("board-add-{}", status.name())),
                            format!("Add item to {}", status.name()),
                            IconName::Plus,
                            A::StartNewIn(status),
                            look.tertiary,
                            12.,
                        )),
                )
                .children(
                    items
                        .into_iter()
                        .map(|item| item_card(ui, state, item, look)),
                )
                .drag_over::<DragItem>(move |style, _, _, _| {
                    style.border_1().border_color(rgb(look.accent))
                })
                .on_drop(move |drag: &DragItem, _, cx| {
                    ui2.send(AppAction::Board(A::Move(drag.id.clone(), status)), cx)
                })
        }))
}
fn table(ui: &Rc<Ui>, state: &AppState, look: Look) -> Stateful<Div> {
    let cell = |label: &str, width: f32| {
        div()
            .w(px(width))
            .flex_shrink_0()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(look.secondary))
            .child(label.to_owned())
    };
    let mut body = card(look).min_w(px(810.)).overflow_hidden().child(
        row()
            .h(px(30.))
            .px(px(12.))
            .gap(px(8.))
            .bg(rgb(look.panel))
            .border_b_1()
            .border_color(rgb(look.line))
            .child(cell("Key", 60.))
            .child(cell("Title", 0.).flex_1())
            .child(cell("Worked on by", 220.))
            .child(cell("Channel", 130.))
            .child(cell("", 90.)),
    );
    for status in statuses(state) {
        let items = state.board.column(status);
        body = body
            .child(
                row()
                    .h(px(28.))
                    .px(px(12.))
                    .gap(px(6.))
                    .bg(rgb(look.fill))
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .child(chrome::icon(IconName::ChevronDown, look.secondary, 12.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(status.name()))
                    .child(secondary(look, items.len().to_string())),
            )
            .children(items.into_iter().map(|item| {
                let send = ui.clone();
                let id = item.id.clone();
                row()
                    .id(SharedString::from(format!("board-table {}", item.id)))
                    .role(Role::ListItem)
                    .aria_label(format!("{} {}", item.id, item.title))
                    .test_support()
                    .h(px(32.))
                    .px(px(12.))
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .cursor_pointer()
                    .when(state.board.request(item).is_some(), |x| {
                        x.bg(rgb(look.attention_bg))
                    })
                    .child(
                        secondary(look, item.id.to_string())
                            .text_size(px(12.5))
                            .w(px(60.))
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .font_weight(FontWeight::MEDIUM)
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .w(px(220.))
                            .flex_shrink_0()
                            .children(session_chip(state, item, look)),
                    )
                    .child(
                        secondary(
                            look,
                            match &item.scope {
                                Scope::Global => "Global".into(),
                                Scope::Channel(id) | Scope::Session { channel: id, .. } => state
                                    .workspace
                                    .channel(id)
                                    .map_or_else(|| id.to_string(), |c| c.name.clone()),
                            },
                        )
                        .text_size(px(12.5))
                        .w(px(130.))
                        .flex_shrink_0(),
                    )
                    .child(div().w(px(90.)).flex_shrink_0().when(
                        state.board.request(item).is_some(),
                        |x| {
                            // The golden's Review fills its column, label at the
                            // left, so it can't be the shared centred push button.
                            let review = ui.clone();
                            let open = item.id.clone();
                            x.child(
                                Button::new(SharedString::from(format!(
                                    "board-table-review {}",
                                    item.id
                                )))
                                .accessibility_label("Review")
                                .small()
                                .h(px(24.))
                                .px(px(11.))
                                .rounded(px(6.))
                                .font_weight(FontWeight::MEDIUM)
                                .bg(rgb(look.fill))
                                .text_color(rgb(look.text))
                                .border_1()
                                .border_color(rgb(look.line))
                                .child(
                                    div()
                                        .w_full()
                                        .text_left()
                                        .text_size(px(12.))
                                        .child("Review"),
                                )
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    review.send(AppAction::Board(A::Open(open.clone())), cx);
                                }),
                            )
                        },
                    ))
                    .on_click(move |_, _, cx| send.send(AppAction::Board(A::Open(id.clone())), cx))
            }));
    }
    col()
        .id("board-table")
        .size_full()
        .p(px(14.))
        .text_size(px(12.5))
        .overflow_scroll()
        .child(body)
}
fn dashboard(ui: &Rc<Ui>, state: &AppState, look: Look) -> Stateful<Div> {
    let items: Vec<_> = state
        .workspace
        .items
        .iter()
        .filter(|i| state.board.in_scope(i))
        .collect();
    let waiting = items
        .iter()
        .filter(|i| state.board.request(i).is_some())
        .count();
    // BD-13: the In progress line names what needs the user among them.
    let exited = state
        .workspace
        .sessions
        .iter()
        .filter(|s| {
            !s.archived
                && matches!(s.status, humata2_clients::model::Status::Exited { .. })
                && items
                    .iter()
                    .any(|i| i.status == S::InProgress && i.session.as_ref() == Some(&s.id))
        })
        .count();
    let progress_note = match exited {
        0 => "See active items →".to_owned(),
        1 => "1 session exited →".to_owned(),
        n => format!("{n} sessions exited →"),
    };
    // BD-13: Done this week carries the change on last week.
    let done_days: Vec<u8> = items
        .iter()
        .filter(|i| i.status == S::Done)
        .filter_map(|i| {
            state
                .workspace
                .item_details
                .get(&i.id)
                .and_then(|d| d.done_day)
        })
        .collect();
    let this_week = done_days.iter().filter(|d| **d >= 7).count();
    let last_week = done_days.iter().filter(|d| **d < 7).count();
    let done_note = match this_week.cmp(&last_week) {
        std::cmp::Ordering::Greater => format!("+{} on last week", this_week - last_week),
        std::cmp::Ordering::Less => format!("−{} on last week", last_week - this_week),
        std::cmp::Ordering::Equal => "Same as last week".to_owned(),
    };
    let tile = |id: &str, title: &str, value: usize, note: &str, filter: Filter, color: u32| {
        let ui = ui.clone();
        card(look)
            .id(SharedString::from(format!("board-tile-{id}")))
            .role(Role::Button)
            .aria_label(format!("{title}: {value}"))
            .test_support()
            .flex_1()
            .min_w_0()
            .p(px(12.))
            .gap(px(2.))
            .cursor_pointer()
            .child(secondary(look, title.to_owned()))
            .child(
                div()
                    .text_size(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(color))
                    .child(value.to_string()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(look.accent))
                    .child(note.to_owned()),
            )
            .on_click(move |_, _, cx| ui.send(AppAction::Board(A::Drill(filter)), cx))
    };
    let lane_colors = [look.tertiary, crate::theme::GREY, look.accent, 0xbf5af2, crate::theme::GREEN];
    let ss = [S::Backlog, S::Todo, S::InProgress, S::InReview, S::Done];
    let mut by_channel = card(look)
        .flex_1()
        .min_w_0()
        .p(px(12.))
        .gap(px(10.))
        .child(heading(look, "By channel"));
    for channel in &state.workspace.channels {
        let lane: Vec<_> = items
            .iter()
            .filter(|i| i.scope.channel() == Some(&channel.id))
            .collect();
        if lane.is_empty() {
            continue;
        }
        let bars = row()
            .flex_1()
            .h(px(16.))
            .rounded(px(4.))
            .overflow_hidden()
            .gap(px(2.))
            .children(ss.iter().zip(lane_colors).filter_map(|(status, color)| {
                let count = lane.iter().filter(|i| i.status == *status).count();
                if count == 0 {
                    return None;
                }
                let ui = ui.clone();
                let scope = Scope::Channel(channel.id.clone());
                let status = *status;
                Some(
                    div()
                        .id(SharedString::from(format!(
                            "board-bar-{}-{}",
                            channel.id,
                            status.name()
                        )))
                        .role(Role::Button)
                        .aria_label(format!("{} {}: {}", channel.name, status.name(), count))
                        .test_support()
                        .flex_grow(count as f32)
                        .flex_basis(px(0.))
                        .h_full()
                        .bg(rgb(color))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            ui.send(AppAction::Board(A::SetScope(scope.clone())), cx);
                            ui.send(AppAction::Board(A::Drill(Filter::Status(status))), cx);
                        }),
                )
            }));
        by_channel = by_channel.child(
            row()
                .gap(px(10.))
                .child(div().w(px(130.)).child(channel.name.clone()))
                .child(bars)
                .child(
                    row()
                        .w(px(30.))
                        .justify_end()
                        .child(secondary(look, lane.len().to_string())),
                ),
        );
    }
    by_channel = by_channel.child(row().flex_wrap().gap(px(12.)).children(
        ss.iter().zip(lane_colors).map(|(s, color)| {
            row()
                .gap(px(5.))
                .child(div().size(px(8.)).rounded(px(2.)).bg(rgb(color)))
                .child(secondary(look, s.name()))
        }),
    ));
    let mut counts = [0usize; 14];
    for item in &items {
        if item.status == S::Done {
            if let Some(day) = state
                .workspace
                .item_details
                .get(&item.id)
                .and_then(|d| d.done_day)
                .filter(|d| *d < 14)
            {
                counts[day as usize] += 1;
            }
        }
    }
    let chart =
        row()
            .items_end()
            .gap(px(6.))
            .h(px(90.))
            .children(counts.into_iter().enumerate().map(|(day, n)| {
                let ui = ui.clone();
                div()
                    .id(SharedString::from(format!("board-day-{day}")))
                    .role(Role::Button)
                    .aria_label(format!("Day {}: {n} done", day + 1))
                    .test_support()
                    .flex_1()
                    .h(px(4. + 16. * n as f32))
                    .rounded_t(px(3.))
                    .bg(rgb(if day == 13 { look.accent } else { look.fill }))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Board(A::Drill(Filter::DoneDay(day as u8))), cx)
                    })
            }));
    let mut sessions = card(look)
        .p(px(12.))
        .gap(px(2.))
        .child(heading(look, "By session"));
    // Frame 24: the sessions that need the person come first, then the exited ones.
    let mut by_need: Vec<_> = state.workspace.sessions.iter().collect();
    by_need.sort_by_key(|session| match session.status {
        humata2_clients::model::Status::NeedsYou => 0,
        humata2_clients::model::Status::Exited { .. } => 1,
        _ => 2,
    });
    for session in by_need {
        let count = items
            .iter()
            .filter(|i| i.session.as_ref() == Some(&session.id))
            .count();
        if count == 0 || session.archived {
            continue;
        }
        let ui = ui.clone();
        let id = session.id.clone();
        let exited = matches!(
            session.status,
            humata2_clients::model::Status::Exited { .. }
        );
        sessions = sessions.child(
            row()
                .id(SharedString::from(format!("board-session {}", session.id)))
                .role(Role::Button)
                .aria_label(session.title.clone())
                .test_support()
                .h(px(30.))
                .gap(px(8.))
                .border_b_1()
                .border_color(rgb(look.line))
                .cursor_pointer()
                .child(chrome::status(session.status))
                .child(div().flex_1().child(session.title.clone()))
                .child(
                    secondary(
                        look,
                        format!("{count} {}", if count == 1 { "item" } else { "items" }),
                    )
                    .text_size(px(12.5))
                    .w(px(80.)),
                )
                .child(
                    div()
                        .w(px(70.))
                        .text_color(rgb(look.accent))
                        .child(if exited { "Resume" } else { "Open" }),
                )
                .on_click(move |_, _, cx| {
                    ui.send(AppAction::Navigate(Route::Session(id.clone())), cx);
                    if exited {
                        ui.send(
                            AppAction::Session(humata2_session::SessionAction::Resume),
                            cx,
                        );
                    }
                }),
        );
    }
    col()
        .id("board-dashboard")
        .size_full()
        .overflow_y_scroll()
        .p(px(16.))
        .gap(px(12.))
        .text_size(px(12.5))
        .child(
            row()
                .gap(px(10.))
                .child(tile(
                    "waiting",
                    "Waiting on you",
                    waiting,
                    "Review the asks →",
                    Filter::Waiting,
                    crate::theme::ORANGE,
                ))
                .child(tile(
                    "progress",
                    "In progress",
                    items.iter().filter(|i| i.status == S::InProgress).count(),
                    progress_note.as_str(),
                    Filter::Status(S::InProgress),
                    look.text,
                ))
                .child(tile(
                    "done",
                    "Done this week",
                    this_week,
                    done_note.as_str(),
                    Filter::DoneWeek,
                    look.text,
                ))
                .child(tile(
                    "unassigned",
                    "Unassigned",
                    items
                        .iter()
                        .filter(|i| i.session.is_none() && !matches!(i.status, S::Done | S::WontDo))
                        .count(),
                    "Start a session →",
                    Filter::Unassigned,
                    look.text,
                )),
        )
        .child(
            row().items_stretch().gap(px(12.)).child(by_channel).child(
                card(look)
                    .w(px(330.))
                    .p(px(12.))
                    .gap(px(8.))
                    .child(
                        row()
                            .child(heading(look, "Done per day").flex_1())
                            .child(secondary(look, "14 days")),
                    )
                    .child(chart),
            ),
        )
        .child(sessions)
}
/// A control that is in the accessibility tree and tab order but is painted
/// only while its row is hovered or it has focus, and takes no layout space.
fn hover_control(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: &'static str,
    action: A,
    group: SharedString,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .absolute()
        .top(px(0.))
        .right(px(0.))
        .px(px(5.))
        .rounded(px(4.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.secondary))
        .cursor_pointer()
        .opacity(0.)
        .group_hover(group, |x| x.opacity(1.))
        .focus_visible(|x| x.opacity(1.))
        .child(glyph)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(AppAction::Board(action.clone()), cx)
        })
        .test_support()
}
/// What the agent is called in the drawer's ask.
fn agent_name(by: Option<Agent>) -> String {
    by.map_or_else(|| "An agent".to_owned(), |a| crate::desk::runs(a).to_owned())
}
/// The picker popover under a drawer row: a filter over the offered rows.
fn picker(ui: &Rc<Ui>, state: &AppState, inputs: &Inputs, kind: PickerKind, look: Look) -> Option<Stateful<Div>> {
    let open = state.board.picker.as_ref().filter(|p| p.kind == kind)?;
    let cursor = open.cursor;
    let rows = state.board.choices().into_iter().enumerate().map(|(at, choice)| {
        let (name, id) = match &choice {
            Choice::Session(id) => (
                state
                    .workspace
                    .session(id)
                    .map_or_else(|| id.to_string(), |s| s.title.clone()),
                format!("board-pick-session {id}"),
            ),
            Choice::Item(id) => (
                state.workspace.items.iter().find(|i| &i.id == id).map_or_else(
                    || id.to_string(),
                    |i| format!("{} {}", i.id, i.title),
                ),
                format!("board-pick-item {id}"),
            ),
        };
        let ui = ui.clone();
        div()
            .id(SharedString::from(id))
            .role(Role::Button)
            .aria_label(name.clone())
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(at == cursor, |x| x.bg(rgb(look.fill)))
            .child(name)
            .on_click(move |_, _, cx| {
                ui.send(AppAction::Board(A::Pick(choice.clone())), cx)
            })
            .test_support()
    });
    let keys = ui.clone();
    let dismiss = ui.clone();
    Some(
        col()
            .id(match kind {
                PickerKind::Sessions => "board-picker-sessions",
                PickerKind::Dependencies => "board-picker-dependencies",
            })
            // Escape is the app's Dismiss everywhere, a text box included,
            // so the key-down below never sees it: close the picker here, on
            // the way up, before the Shell's Dismiss.
            .on_action(move |_: &crate::keys::Dismiss, _, cx| {
                dismiss.send(AppAction::Board(A::ClosePicker), cx);
            })
            .gap(px(4.))
            .p(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.card))
            .on_key_down(move |event, _, cx| {
                let action = match event.keystroke.key.as_str() {
                    "up" => A::PickerUp,
                    "down" => A::PickerDown,
                    "enter" => A::PickerEnter,
                    "escape" => A::ClosePicker,
                    _ => return,
                };
                cx.stop_propagation();
                keys.send(AppAction::Board(action), cx);
            })
            .child(
                Input::new(&inputs.picker)
                    // The board's own filter is "Filter" too.
                    .aria_label(match kind {
                        PickerKind::Sessions => "Filter sessions",
                        PickerKind::Dependencies => "Filter dependencies",
                    })
                    .small()
                    .min_h(px(26.))
                    .rounded(px(6.))
                    .bg(rgb(look.content))
                    .border_color(rgb(look.line))
                    .text_size(px(12.)),
            )
            .child(col().id("board-picker-choices").gap(px(1.)).max_h(px(160.)).overflow_y_scroll().children(rows)),
    )
}
/// "Keep in <column>" for the ask's second button.
fn keep_label(status: S) -> String {
    let name = status.name();
    if status == S::InProgress {
        "Keep in progress".to_owned()
    } else {
        format!("Keep in {}", name.to_lowercase())
    }
}
fn drawer(ui: &Rc<Ui>, state: &AppState, inputs: &Inputs, item: &Item, look: Look) -> Stateful<Div> {
    let details = state.workspace.item_details.get(&item.id);
    let field = |name: &str, value: AnyElement| {
        row()
            .gap(px(10.))
            .child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .text_color(rgb(look.secondary))
                    .child(name.to_owned()),
            )
            .child(value)
    };
    let mut panel = col()
        .id("board-drawer")
        .h_full()
        .flex_shrink_0()
        .gap(px(12.))
        .p(px(16.))
        .bg(rgb(look.content))
        .border_l_1()
        .border_color(rgb(look.line))
        .text_size(px(12.5))
        .overflow_y_scroll()
        .when(state.board.expanded, |x| x.flex_1())
        .when(!state.board.expanded, |x| x.w(px(380.)))
        .child(
            row()
                .gap(px(8.))
                .child(secondary(look, item.id.to_string()))
                .child(div().flex_1())
                .child(glyph(
                    ui,
                    "board-expand",
                    "Expand item",
                    IconName::Maximize2,
                    A::Expand,
                    look.secondary,
                    15.,
                ))
                .child(glyph(
                    ui,
                    "board-close",
                    "Close item",
                    IconName::X,
                    A::Close,
                    look.secondary,
                    15.,
                )),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(item.title.clone()),
        )
        .child(field(
            "Status",
            popup(ui, "board-status", item.status.name(), A::StatusMenu, look).into_any_element(),
        ));
    if state.board.status_menu {
        // BD-1: a board has five columns; Blocked and Won't do have none on
        // the daemon, so the menu does not offer them.
        let offered = S::ALL
            .into_iter()
            .filter(|s| !matches!(s, S::Blocked | S::WontDo));
        panel = panel.child(col().gap(px(3.)).children(offered.map(|s| {
            act(
                ui,
                SharedString::from(format!("board-move-{}", s.name())),
                s.name(),
                A::Move(item.id.clone(), s),
                look,
            )
        })));
    }
    panel = panel.child(field(
        "Channel",
        div().child(scope_of(item)).into_any_element(),
    ));
    let ids = details
        .map(|d| d.sessions.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| item.session.iter().cloned().collect());
    let add = |ui: &Rc<Ui>,
               id: &'static str,
               label: &'static str,
               group: &'static str,
               kind: PickerKind| {
        let ui = ui.clone();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .absolute()
            .top(px(0.))
            .right(px(0.))
            .px(px(5.))
            .rounded(px(4.))
            .text_color(rgb(look.secondary))
            .cursor_pointer()
            .opacity(0.)
            .group_hover(group, |x| x.opacity(1.))
            .focus_visible(|x| x.opacity(1.))
            .child("+")
            .on_click(move |_, _, cx| ui.send(AppAction::Board(A::OpenPicker(kind)), cx))
            .test_support()
    };
    let mut sessions = col().relative().flex_1().gap(px(4.)).children(
        ids.into_iter()
            .filter_map(|id| state.workspace.session(&id))
            .map(|s| {
                let ui = ui.clone();
                let id = s.id.clone();
                let group = SharedString::from(format!("board-session-row {id}"));
                row()
                    .id(SharedString::from(format!("board-drawer-session {}", id)))
                    .role(Role::Button)
                    .aria_label(s.title.clone())
                    .test_support()
                    .relative()
                    .group(group.clone())
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(chrome::status(s.status))
                    .child(s.title.clone())
                    .child(hover_control(
                        &ui,
                        SharedString::from(format!("board-remove-session {id}")),
                        format!("Remove session {}", s.title),
                        "×",
                        A::RemoveSession(id.clone()),
                        group,
                        look,
                    ))
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Navigate(Route::Session(id.clone())), cx)
                    })
            }),
    );
    if let Some(open) = picker(ui, state, inputs, PickerKind::Sessions, look) {
        sessions = sessions.child(open);
    }
    // The + sits in the field's row and shows while the pointer is anywhere
    // on it: with nothing listed the value column has no height, so a + in
    // it would hang below the row, out of its own hover area.
    panel = panel.child(
        field("Sessions", sessions.into_any_element())
            .relative()
            .group("board-sessions-field")
            .child(add(ui, "board-add-session", "Add session", "board-sessions-field", PickerKind::Sessions)),
    );
    let mut depends = col()
        .relative()
        .flex_1()
        .items_start()
        .gap(px(4.))
        .children(
            item.depends_on
                .iter()
                .filter_map(|id| state.workspace.items.iter().find(|i| &i.id == id))
                .map(|i| {
                    let ui = ui.clone();
                    let id = i.id.clone();
                    let name = format!(
                        "{} {}{}",
                        i.id,
                        i.title,
                        if i.status == S::Done { " ✓" } else { "" }
                    );
                    let group = SharedString::from(format!("board-dependency-row {id}"));
                    div()
                        .id(SharedString::from(format!("board-dependency {}", i.id)))
                        .role(Role::Button)
                        .aria_label(name.clone())
                        .relative()
                        .group(group.clone())
                        .flex_shrink_0()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .text_size(px(11.))
                        .text_color(rgb(look.secondary))
                        .cursor_pointer()
                        .child(name)
                        .child(hover_control(
                            &ui,
                            SharedString::from(format!("board-remove-dependency {id}")),
                            format!("Remove dependency {id}"),
                            "×",
                            A::RemoveDependency(id.clone()),
                            group,
                            look,
                        ))
                        .on_click(move |_, _, cx| {
                            ui.send(AppAction::Board(A::Open(id.clone())), cx)
                        })
                        .test_support()
                }),
        );
    if let Some(open) = picker(ui, state, inputs, PickerKind::Dependencies, look) {
        depends = depends.child(open);
    }
    panel = panel.child(
        field("Depends on", depends.into_any_element())
            .relative()
            .group("board-dependencies-field")
            .child(add(ui, "board-add-dependency", "Add dependency", "board-dependencies-field", PickerKind::Dependencies)),
    );
    panel = panel.child(div().h(px(1.)).flex_shrink_0().bg(rgb(look.line)));
    if let (Some(request), Some(ask)) = (state.board.request(item), state.board.ask(item)) {
        let answer = |answer| AppAction::Inbox(InboxAction::Answer(request.id.clone(), answer));
        let (title, yes, no, changes, destructive) = match &ask {
            Ask::Move { by, to, keep } => (
                format!("{} asks to move this to {}", agent_name(*by), to.name()),
                format!("Move to {}", to.name()),
                keep_label(*keep),
                Vec::new(),
                false,
            ),
            Ask::Edit { by, changes } => (
                format!("{} asks to edit this", agent_name(*by)),
                "Apply edit".to_owned(),
                "Keep as is".to_owned(),
                changes.clone(),
                false,
            ),
            Ask::Delete { by } => (
                format!("{} asks to delete this", agent_name(*by)),
                "Delete item".to_owned(),
                "Keep item".to_owned(),
                Vec::new(),
                true,
            ),
        };
        let mut approve = button(ui, "board-approve", yes, answer(Answer::Approved), look, true);
        if destructive {
            approve = approve.bg(rgb(crate::theme::RED));
        }
        panel = panel.child(
            col()
                .gap(px(6.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(rgb(look.attention_bg))
                .child(
                    row()
                        .gap(px(6.))
                        .child(chrome::status(humata2_clients::model::Status::NeedsYou))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
                )
                .child(secondary(
                    look,
                    format!(
                        "“{}”",
                        details
                            .filter(|d| !d.request_summary.is_empty())
                            .map_or(request.reason.as_str(), |d| d.request_summary.as_str())
                    ),
                ))
                .children(changes.into_iter().map(|c| {
                    secondary(look, format!("{}: {} → {}", c.field, c.before, c.after))
                }))
                .child(
                    row()
                        .gap(px(6.))
                        .child(approve)
                        .child(button(
                            ui,
                            "board-deny",
                            no,
                            answer(Answer::Denied { reply: None }),
                            look,
                            false,
                        ))
                        .child(act(
                            ui,
                            "board-open-session",
                            "Open session",
                            A::OpenSession,
                            look,
                        )),
                ),
        );
    }
    panel
        .child(heading(look, "Description"))
        .child(
            secondary(
                look,
                details.map_or_else(|| "No description yet.".into(), |d| d.description.clone()),
            )
            .text_size(px(12.5)),
        )
        .child(heading(look, "History"))
        .children(
            state
                .board
                .history
                .iter()
                .map(|line| secondary(look, line.clone())),
        )
        .child(row().child(button(
            ui,
            "board-start-session",
            "Start a session for this item",
            AppAction::StartNewForItem(item.id.clone()),
            look,
            false,
        )))
}
fn scope_of(item: &Item) -> String {
    match &item.scope {
        Scope::Global => "Global board".into(),
        Scope::Channel(id) => format!("# {id}"),
        Scope::Session { channel, session } => format!("# {channel} · {session}'s board"),
    }
}

pub(crate) fn page(ui: &Rc<Ui>, state: &AppState, inputs: &Inputs, look: Look) -> Div {
    let mut page = col().size_full().min_h_0().text_color(rgb(look.text));
    // A board view being shown has its Showing bar here too (VW-8).
    let showing = state
        .views
        .showing
        .as_ref()
        .and_then(|showing| state.views.record(&showing.id))
        .filter(|record| {
            matches!(
                record.content,
                humata2_clients::views::ViewContent::Board(_)
                    | humata2_clients::views::ViewContent::SessionBoard { .. }
            )
        });
    if let Some(record) = showing {
        page = page.child(crate::view_tab::showing_bar(ui, state, record, look));
    }
    if state.board.scope_menu {
        page = page.child(
            row()
                .flex_wrap()
                .gap(px(6.))
                .p(px(10.))
                .child(act(
                    ui,
                    "board-scope-all",
                    "Everything",
                    A::GlobalOnly(false),
                    look,
                ))
                .child(act(
                    ui,
                    "board-scope-global",
                    "Global items only",
                    A::GlobalOnly(true),
                    look,
                ))
                .children(state.workspace.channels.iter().map(|c| {
                    act(
                        ui,
                        SharedString::from(format!("board-scope {}", c.id)),
                        format!("# {}", c.name),
                        A::SetScope(Scope::Channel(c.id.clone())),
                        look,
                    )
                }))
                .children(state.board.channel_views().into_iter().map(|view| {
                    act(
                        ui,
                        SharedString::from(format!(
                            "board-channel-view {}",
                            view_label(state, &view)
                        )),
                        view_label(state, &view),
                        A::SetChannelView(view),
                        look,
                    )
                })),
        );
    }
    if state.board.view.filter != Filter::All {
        page = page.child(
            row()
                .gap(px(8.))
                .px(px(14.))
                .py(px(6.))
                .child(secondary(
                    look,
                    match state.board.view.filter {
                        Filter::All => "All items".into(),
                        Filter::Status(s) => s.name().into(),
                        Filter::Waiting => "Waiting on you".into(),
                        Filter::Unassigned => "Unassigned".into(),
                        Filter::DoneWeek => "Done this week".into(),
                        Filter::DoneDay(day) => format!("Completed on day {}", day + 1),
                    },
                ))
                .child(act(
                    ui,
                    "board-clear-filter",
                    "Show all",
                    A::ClearFilter,
                    look,
                )),
        );
    }
    if let Some(error) = &state.board.error {
        page = page.child(
            div()
                .p(px(10.))
                .text_color(rgb(crate::theme::RED_STATUS))
                .child(error.clone()),
        );
    }
    if let Some(title) = &state.board.new_title {
        page = page.child(
            card(look)
                .m(px(14.))
                .p(px(12.))
                .gap(px(8.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("New item"))
                .child(Input::new(&inputs.title))
                .child(
                    row().flex_wrap().gap(px(4.)).children(
                        [S::Backlog, S::Todo, S::InProgress, S::InReview, S::Done]
                            .into_iter()
                            .map(|status| {
                                act(
                                    ui,
                                    SharedString::from(format!(
                                        "board-new-status-{}",
                                        status.name()
                                    )),
                                    status.name(),
                                    A::NewStatus(status),
                                    look,
                                )
                                .when(state.board.new_status == status, |b| {
                                    b.bg(rgb(look.accent)).text_color(rgb(0xffffff))
                                })
                            }),
                    ),
                )
                .child(
                    row()
                        .gap(px(6.))
                        .child(
                            button(
                                ui,
                                "board-create-item",
                                "Create item",
                                AppAction::Board(A::CreateNew),
                                look,
                                true,
                            )
                            .disabled(title.trim().is_empty()),
                        )
                        .child(act(ui, "board-cancel-item", "Cancel", A::CancelNew, look)),
                ),
        );
    }
    let body = match state.board.view.mode {
        Mode::Kanban => lanes(ui, state, look),
        Mode::Table => table(ui, state, look),
        Mode::Dashboard => dashboard(ui, state, look),
    };
    page.child(
        row()
            .flex_1()
            .min_h_0()
            .items_stretch()
            .when(
                !state.board.expanded || state.board.opened().is_none(),
                |x| {
                    x.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(body),
                    )
                },
            )
            .children(
                state
                    .board
                    .opened()
                    .map(|item| drawer(ui, state, inputs, item, look)),
            ),
    )
}
