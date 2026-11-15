//! The live Board, its table/dashboard and item drawer (frames 21–24).
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_board::{BoardAction as A, Filter, Mode};
use humata2_clients::model::{Answer, Item, ItemId, ItemStatus as S, Scope};
use humata2_inbox::InboxAction;
use std::rc::Rc;

pub(crate) struct Inputs {
    filter: Entity<InputState>,
    title: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}
impl Inputs {
    pub(crate) fn new(
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<crate::shell::Shell>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Item title"));
        let one = ui.clone();
        let two = ui.clone();
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
        ];
        Self {
            filter,
            title,
            _subscriptions: subs,
        }
    }
    pub(crate) fn sync(&self, state: &AppState, window: &mut Window, cx: &mut App) {
        for (input, value) in [
            (&self.filter, state.board.view.query.as_str()),
            (&self.title, state.board.new_title.as_deref().unwrap_or("")),
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
fn card(look: Look) -> Div {
    col()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}
fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
    primary: bool,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .label(label)
        .small()
        .h(px(24.))
        .px(px(10.))
        .bg(rgb(if primary { look.accent } else { look.fill }))
        .text_color(rgb(if primary { 0xffffff } else { look.text }))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(action.clone(), cx);
        })
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
fn scope_name(state: &AppState) -> String {
    match &state.board.scope {
        Scope::Global => {
            if state.board.view.global_only {
                "Global items only".into()
            } else {
                "All channels".into()
            }
        }
        Scope::Channel(id) => state
            .workspace
            .channel(id)
            .map_or_else(|| id.to_string(), |c| format!("# {}", c.name)),
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
        .child(
            act(
                ui,
                "board-scope",
                format!("Scope: {scope}"),
                A::ScopeMenu,
                look,
            )
            .icon(IconName::ChevronDown),
        )
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
                        .rounded(px(6.))
                        .text_size(px(12.))
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
                .child(Input::new(&inputs.filter).small())
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
                .when(item.status != S::Done, |x| {
                    x.child(chrome::status(s.status))
                })
                .child(secondary(look, s.title.clone()))
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
                        .child(secondary(look, label.clone()))
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
                            Scope::Channel(id) => state
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
                    .text_color(rgb(0xff9500))
                    .child(chrome::icon(IconName::Inbox, 0xff9500, 12.))
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
                        .child(
                            act(
                                ui,
                                SharedString::from(format!("board-add-{}", status.name())),
                                "",
                                A::StartNewIn(status),
                                look,
                            )
                            .ghost()
                            .bg(gpui_kit::transparent_black())
                            .w(px(16.))
                            .h(px(16.))
                            .p(px(0.))
                            .icon(IconName::Plus)
                            .accessibility_label(format!("Add item to {}", status.name())),
                        ),
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
                                Scope::Channel(id) => id.to_string(),
                            },
                        )
                        .w(px(130.))
                        .flex_shrink_0(),
                    )
                    .child(div().w(px(90.)).flex_shrink_0().when(
                        state.board.request(item).is_some(),
                        |x| {
                            x.child(act(
                                ui,
                                SharedString::from(format!("board-table-review {}", item.id)),
                                "Review",
                                A::Open(item.id.clone()),
                                look,
                            ))
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
        .filter(|i| match &state.board.scope {
            Scope::Global => !state.board.view.global_only || i.scope == Scope::Global,
            scope => &i.scope == scope,
        })
        .collect();
    let waiting = items
        .iter()
        .filter(|i| state.board.request(i).is_some())
        .count();
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
    let lane_colors = [look.tertiary, 0x8e8e93, look.accent, 0xaf52de, 0x28cd41];
    let ss = [S::Backlog, S::Todo, S::InProgress, S::InReview, S::Done];
    let mut by_channel = card(look)
        .flex_1()
        .min_w_0()
        .p(px(12.))
        .gap(px(10.))
        .child(secondary(look, "By channel").font_weight(FontWeight::SEMIBOLD));
    for channel in &state.workspace.channels {
        let lane: Vec<_> = items
            .iter()
            .filter(|i| i.scope == Scope::Channel(channel.id.clone()))
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
                .child(secondary(look, lane.len().to_string()).w(px(30.))),
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
        .child(secondary(look, "By session").font_weight(FontWeight::SEMIBOLD));
    for session in &state.workspace.sessions {
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
                    0xff9500,
                ))
                .child(tile(
                    "progress",
                    "In progress",
                    items.iter().filter(|i| i.status == S::InProgress).count(),
                    "See active items →",
                    Filter::Status(S::InProgress),
                    look.text,
                ))
                .child(tile(
                    "done",
                    "Done this week",
                    items
                        .iter()
                        .filter(|i| {
                            i.status == S::Done
                                && state
                                    .workspace
                                    .item_details
                                    .get(&i.id)
                                    .and_then(|d| d.done_day)
                                    .is_some_and(|d| d >= 7)
                        })
                        .count(),
                    "See completed items →",
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
                            .child(
                                secondary(look, "Done per day")
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex_1(),
                            )
                            .child(secondary(look, "14 days")),
                    )
                    .child(chart),
            ),
        )
        .child(sessions)
}
fn drawer(ui: &Rc<Ui>, state: &AppState, item: &Item, look: Look) -> Stateful<Div> {
    let details = state.workspace.item_details.get(&item.id);
    let field = |name: &str, value: AnyElement| {
        row()
            .gap(px(10.))
            .child(secondary(look, name.to_owned()).w(px(80.)).flex_shrink_0())
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
                .child(
                    act(ui, "board-expand", "", A::Expand, look)
                        .ghost()
                        .bg(gpui_kit::transparent_black())
                        .w(px(16.))
                        .h(px(16.))
                        .p(px(0.))
                        .icon(IconName::Maximize2)
                        .accessibility_label("Expand item"),
                )
                .child(
                    act(ui, "board-close", "", A::Close, look)
                        .ghost()
                        .bg(gpui_kit::transparent_black())
                        .w(px(16.))
                        .h(px(16.))
                        .p(px(0.))
                        .icon(IconName::X)
                        .accessibility_label("Close item"),
                ),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(item.title.clone()),
        )
        .child(field(
            "Status",
            act(ui, "board-status", item.status.name(), A::StatusMenu, look)
                .icon(IconName::ChevronDown)
                .into_any_element(),
        ));
    if state.board.status_menu {
        panel = panel.child(col().gap(px(3.)).children(S::ALL.into_iter().map(|s| {
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
    panel = panel.child(field(
        "Sessions",
        col()
            .gap(px(4.))
            .children(
                ids.into_iter()
                    .filter_map(|id| state.workspace.session(&id))
                    .map(|s| {
                        let ui = ui.clone();
                        let id = s.id.clone();
                        row()
                            .id(SharedString::from(format!("board-drawer-session {}", id)))
                            .role(Role::Button)
                            .aria_label(s.title.clone())
                            .test_support()
                            .gap(px(6.))
                            .cursor_pointer()
                            .child(chrome::status(s.status))
                            .child(s.title.clone())
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Navigate(Route::Session(id.clone())), cx)
                            })
                    }),
            )
            .into_any_element(),
    ));
    panel = panel.child(field(
        "Depends on",
        col()
            .gap(px(4.))
            .children(
                item.depends_on
                    .iter()
                    .filter_map(|id| state.workspace.items.iter().find(|i| &i.id == id))
                    .map(|i| {
                        act(
                            ui,
                            SharedString::from(format!("board-dependency {}", i.id)),
                            format!(
                                "{} {}{}",
                                i.id,
                                i.title,
                                if i.status == S::Done { " ✓" } else { "" }
                            ),
                            A::Open(i.id.clone()),
                            look,
                        )
                    }),
            )
            .into_any_element(),
    ));
    panel = panel.child(div().h(px(1.)).flex_shrink_0().bg(rgb(look.line)));
    if let Some(request) = state.board.request(item) {
        let answer = |answer| AppAction::Inbox(InboxAction::Answer(request.id.clone(), answer));
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
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Claude Code asks to move this to Done"),
                        ),
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
                .child(
                    row()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(button(
                            ui,
                            "board-approve",
                            "Move to Done",
                            answer(Answer::Approved),
                            look,
                            true,
                        ))
                        .child(button(
                            ui,
                            "board-deny",
                            "Keep in progress",
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
        .child(button(
            ui,
            "board-start-session",
            "Start a session for this item",
            AppAction::StartNewForItem(item.id.clone()),
            look,
            false,
        ))
        .child(secondary(look, "Description").font_weight(FontWeight::SEMIBOLD))
        .child(
            secondary(
                look,
                details.map_or_else(|| "No description yet.".into(), |d| d.description.clone()),
            )
            .text_size(px(12.5)),
        )
        .child(secondary(look, "History").font_weight(FontWeight::SEMIBOLD))
        .children(
            details
                .into_iter()
                .flat_map(|d| &d.history)
                .map(|line| secondary(look, line.clone())),
        )
}
fn scope_of(item: &Item) -> String {
    match &item.scope {
        Scope::Global => "Global board".into(),
        Scope::Channel(id) => format!("# {id}"),
    }
}

pub(crate) fn page(ui: &Rc<Ui>, state: &AppState, inputs: &Inputs, look: Look) -> Div {
    let mut page = col().size_full().min_h_0().text_color(rgb(look.text));
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
                .text_color(rgb(0xff453a))
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
                    .map(|item| drawer(ui, state, item, look)),
            ),
    )
}
