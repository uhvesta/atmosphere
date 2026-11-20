//! Channel pages fitted to frames 18–20 and 35, over client data and saved page state.
use crate::{
    action_menu,
    bridge::Ui,
    chrome, inbox_page,
    theme::{self, Look},
    widgets::{col, push_primary_shape, push_quiet, push_quiet_shape, row},
};
use gpui_kit::assets::IconName;
use humata2_ui_core::widgets::push_primary;
use gpui_kit::base::{Disableable as _, TestTextExt as _};
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
};
use gpui_kit::{prelude::*, *};
use humata2_app::{
    AppAction, AppState, Route,
    channel::{self, Action, History, Tab, View},
};
use humata2_clients::marks::Thing;
use humata2_clients::model::{
    ActivityKind, Agent, Channel, ChannelId, ForkKind, SessionId, Status,
};
use humata2_find::PinnedAction;
use humata2_session::{Panel, SessionAction};
use std::rc::Rc;

/// The channel header's `…` menu button and its compact popover.
fn more_menu(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let open = state.channel_menu;
    let toggle_ui = ui.clone();
    let delete_ui = ui.clone();
    div()
        .relative()
        .child(
            Button::new("channel-more")
                .accessibility_label("More channel actions")
                .ghost()
                .small()
                .w(px(15.))
                .h(px(15.))
                .p_0()
                .child(chrome::icon(
                    IconName::Ellipsis,
                    if open { look.accent } else { look.tertiary },
                    15.,
                ))
                .on_click(move |_, _, cx| {
                    toggle_ui.send(AppAction::Channel(Action::ToggleMenu), cx);
                }),
        )
        .when(open, |menu| {
            menu.child(deferred(action_menu::scrim(
                ui,
                "channel-more-scrim",
                AppAction::Channel(Action::CloseMenu),
            )))
            .child(deferred(
                action_menu::card("channel-more-card", 180., look)
                    .test_support()
                    .top(px(22.))
                    .right(px(0.))
                    .child(
                        Button::new("channel-more-delete")
                            .accessibility_label("Delete channel…")
                            .ghost()
                            .small()
                            .w_full()
                            .h(px(26.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .child(
                                div()
                                    .w_full()
                                    .text_size(px(12.5))
                                    .text_color(rgb(theme::RED))
                                    .child("Delete channel…"),
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                delete_ui.send(AppAction::Channel(Action::StartDelete), cx);
                            }),
                    ),
            ))
        })
}

/// Confirmation modal overlay for deleting a channel (CH-24).
pub fn delete_modal(ui: &Rc<Ui>, state: &AppState, look: Look) -> Option<AnyElement> {
    let deleting = state.deleting_channel.as_ref()?;
    let scrim_ui = ui.clone();
    let confirm_ui = ui.clone();
    let can_confirm = !deleting.loading
        && !deleting.deleting
        && deleting.plan.is_some()
        && deleting
            .plan
            .as_ref()
            .is_none_or(|plan| !plan.has_unpushed_work() || deleting.delete_work);

    let mut body = col().gap(px(8.)).text_size(px(12.5)).text_color(rgb(look.text));
    if deleting.loading {
        body = body.child(
            div()
                .text_color(rgb(look.secondary))
                .child("Checking what deleting this channel will affect…"),
        );
    } else if let Some(plan) = &deleting.plan {
        let total = plan.total_sessions();
        let live = plan.live_sessions.len();
        let session_summary = match (total, live, plan.archived_sessions) {
            (0, _, _) => "No sessions in this channel.".to_owned(),
            (_, 0, archived) => format!(
                "{archived} archived {} will be moved to Trash.",
                if archived == 1 { "session" } else { "sessions" }
            ),
            (_, live, 0) => format!(
                "{live} {} will be moved to Trash.",
                if live == 1 { "session" } else { "sessions" }
            ),
            (_, live, archived) => format!(
                "{live} live and {archived} archived sessions will be moved to Trash."
            ),
        };
        body = body
            .child(session_summary)
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child("The channel folder and its records move to Trash and can be restored within 30 days."),
            );
        if live > 0 {
            let toggle_live_ui = ui.clone();
            body = body.child(
                col()
                    .gap(px(4.))
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(rgb(look.fill))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::ORANGE))
                            .child(if deleting.end_live {
                                format!(
                                    "{live} live {} will be ended when this channel is deleted.",
                                    if live == 1 { "session" } else { "sessions" }
                                )
                            } else {
                                format!(
                                    "{live} live {} will move to the Desk and keep running.",
                                    if live == 1 { "session" } else { "sessions" }
                                )
                            }),
                    )
                    .child(
                        row()
                            .id("channel-delete-end-live")
                            .role(Role::Button)
                            .aria_label("End live sessions")
                            .aria_selected(deleting.end_live)
                            .test_support()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(if deleting.end_live { "☑" } else { "☐" })
                            .child("End live sessions")
                            .on_click(move |_, _, cx| {
                                toggle_live_ui.send(AppAction::Channel(Action::ToggleEndLive), cx);
                            }),
                    ),
            );
        }
        if plan.board_items > 0 || plan.pending_requests > 0 {
            let mut extras = Vec::new();
            if plan.board_items > 0 {
                extras.push(format!(
                    "{} to-do {}",
                    plan.board_items,
                    if plan.board_items == 1 { "item" } else { "items" }
                ));
            }
            if plan.pending_requests > 0 {
                extras.push(format!(
                    "{} pending {}",
                    plan.pending_requests,
                    if plan.pending_requests == 1 {
                        "request"
                    } else {
                        "requests"
                    }
                ));
            }
            body = body.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!("Also removes {}.", extras.join(" and "))),
            );
        }
        if plan.has_unpushed_work() {
            let toggle_work_ui = ui.clone();
            let dirty_names: Vec<String> = plan
                .live_sessions
                .iter()
                .filter(|s| s.dirty_files > 0 || s.unpushed_commits > 0)
                .map(|s| s.title.clone())
                .collect();
            body = body.child(
                col()
                    .gap(px(4.))
                    .p(px(10.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(theme::RED))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::RED))
                            .child(format!(
                                "Uncommitted or unpushed work in: {}",
                                dirty_names.join(", ")
                            )),
                    )
                    .child(
                        row()
                            .id("channel-delete-work")
                            .role(Role::Button)
                            .aria_label("Delete uncommitted or unpushed work")
                            .aria_selected(deleting.delete_work)
                            .test_support()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .child(if deleting.delete_work { "☑" } else { "☐" })
                            .child("Delete uncommitted or unpushed work")
                            .on_click(move |_, _, cx| {
                                toggle_work_ui
                                    .send(AppAction::Channel(Action::ToggleDeleteWork), cx);
                            }),
                    ),
            );
        }
    }

    let modal = div()
        .id("channel-delete-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000055))
        .on_click(move |_, _, cx| {
            scrim_ui.send(AppAction::Channel(Action::CancelDelete), cx);
        })
        .child(
            col()
                .id("channel-delete-modal")
                .role(Role::Dialog)
                .aria_label(format!("Delete #{}", deleting.channel_name))
                .test_support()
                .w(px(440.))
                .max_w(px(440.))
                .p(px(18.))
                .gap(px(12.))
                .rounded(px(10.))
                .bg(rgb(look.card))
                .border_1()
                .border_color(rgb(look.line))
                .shadow_lg()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Delete # {}?", deleting.channel_name)),
                )
                .child(body)
                .children(deleting.error.clone().map(|error| {
                    div()
                        .id("channel-delete-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .test_support()
                        .text_size(px(11.5))
                        .text_color(rgb(theme::RED))
                        .child(error)
                }))
                .child(
                    row()
                        .justify_end()
                        .gap(px(8.))
                        .pt(px(4.))
                        .child(push_quiet(
                            ui,
                            "channel-delete-cancel",
                            "Cancel",
                            AppAction::Channel(Action::CancelDelete),
                            look,
                        ))
                        .child(
                            Button::new("channel-delete-confirm")
                                .accessibility_label("Delete channel")
                                .ghost()
                                .small()
                                .h(px(26.))
                                .px(px(12.))
                                .rounded(px(6.))
                                .bg(rgb(theme::RED))
                                .text_color(rgb(0xffffff))
                                .disabled(!can_confirm)
                                .child(if deleting.deleting {
                                    "Deleting…"
                                } else {
                                    "Delete channel"
                                })
                                .on_click(move |_, _, cx| {
                                    confirm_ui.send(AppAction::Channel(Action::ConfirmDelete), cx);
                                }),
                        ),
                ),
        )
        .into_any_element();
    Some(modal)
}
/// The channel's Pin, in the same 15 pt box as the inert icons: a click pins or
/// unpins the channel on the host, and the icon is accent-colored while pinned.
fn pin_icon(ui: &Rc<Ui>, channel: &ChannelId, pinned: bool, look: Look) -> Button {
    let ui = ui.clone();
    let thing = Thing::Channel(channel.clone());
    Button::new("channel-pin")
        .accessibility_label(if pinned {
            "Unpin this channel"
        } else {
            "Pin this channel"
        })
        .ghost()
        .small()
        .w(px(15.))
        .h(px(15.))
        .p_0()
        .child(chrome::icon(
            IconName::Pin,
            if pinned { look.accent } else { look.tertiary },
            15.,
        ))
        .on_click(move |_, _, cx| {
            ui.send(
                AppAction::Pinned(PinnedAction::Toggle {
                    thing: thing.clone(),
                    pin: !pinned,
                }),
                cx,
            )
        })
}
/// Accent text that acts as a button, as the frames draw table links.
fn link(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .cursor_pointer()
        .text_color(rgb(look.accent))
        .child(label)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(action.clone(), cx);
        })
        .test_support()
}
fn page_view(state: &AppState, id: &ChannelId) -> View {
    state.channel_pages.get(id).cloned().unwrap_or_default()
}
fn agent(agent: Agent) -> &'static str {
    if agent == Agent::Claude {
        "Claude Code"
    } else {
        agent.name()
    }
}
fn open(ui: &Rc<Ui>, id: &SessionId, panel: Option<Panel>, cx: &mut App) {
    ui.send(AppAction::Navigate(Route::Session(id.clone())), cx);
    if let Some(panel) = panel {
        ui.send(AppAction::Session(SessionAction::SetPanel(Some(panel))), cx);
    }
}

pub fn header(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> Div {
    let on = page_view(state, &channel.id).tab;
    let sessions = state
        .workspace
        .sessions
        .iter()
        .filter(|s| s.channel == channel.id && !s.archived)
        .count();
    let waiting = state
        .workspace
        .waiting()
        .filter(|r| r.channel == channel.id)
        .count();
    let desk = state.desk.channel_rows(&channel.id).len();
    let machines = channel
        .machines
        .iter()
        .filter_map(|id| state.workspace.machine(id))
        .map(|m| m.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let repos = if channel.repos.is_empty() {
        "no repos yet".to_owned()
    } else {
        channel.repos.join(", ")
    };
    let new_session = ui.clone();
    let pinned = state
        .pinned
        .list
        .items
        .iter()
        .any(|pin| pin.thing == Thing::Channel(channel.id.clone()));
    let has_sessions = state
        .workspace
        .sessions
        .iter()
        .any(|s| s.channel == channel.id);
    col()
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .h(px(52.))
                .px(px(20.))
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("# {}", channel.name)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(if machines.is_empty() {
                            repos
                        } else {
                            format!("{repos} · {machines}")
                        }),
                )
                // Frames 18 and 35 draw Pin and a "…" menu here.
                .when(has_sessions, |bar| bar.child(pin_icon(ui, &channel.id, pinned, look)))
                .child(more_menu(ui, state, look))
                .child(
                    push_primary_shape("channel-new-session", "New session", look).on_click(
                        move |_, _, cx| new_session.send(AppAction::StartNewSession, cx),
                    ),
                ),
        )
        .child(
            row()
                .id("channel-tabs")
                .h(px(32.))
                .px(px(20.))
                .gap(px(20.))
                .overflow_x_scroll()
                .items_end()
                .text_size(px(12.5))
                .children(
                    [
                        (Tab::Sessions, "Sessions", (sessions > 0).then_some(sessions)),
                        (Tab::Desk, "Desk", (desk > 0).then_some(desk)),
                        (Tab::Inbox, "Agent inbox", (waiting > 0).then_some(waiting)),
                        (Tab::Board, "To-do", None),
                        (Tab::Repos, "Repos and machines", None),
                        (Tab::Access, "Agent access", None),
                        (Tab::Activity, "Activity", None),
                    ]
                    .into_iter()
                    .map(|(tab, name, count)| {
                        let ui = ui.clone();
                        row()
                            .id(SharedString::from(format!(
                                "channel-tab-{}",
                                // Ids keep the tabs' first names, which tests click.
                                match tab {
                                    Tab::Inbox => "Inbox",
                                    Tab::Board => "Board",
                                    _ => name,
                                }
                            )))
                            .role(Role::Tab)
                            .aria_label(name)
                            .aria_selected(tab == on)
                            .h_full()
                            .px(px(2.))
                            .gap(px(5.))
                            .flex_shrink_0()
                            .border_b_2()
                            .border_color(rgb(if tab == on { look.accent } else { look.content }))
                            .text_color(rgb(if tab == on { look.text } else { look.secondary }))
                            .when(tab == on, |x| x.font_weight(FontWeight::MEDIUM))
                            .child(name)
                            .children(count.map(|n| {
                                row()
                                    .h(px(16.))
                                    .px(px(6.))
                                    .rounded_full()
                                    .bg(rgb(look.fill))
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(look.secondary))
                                    .child(n.to_string())
                            }))
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Channel(Action::Tab(tab)), cx)
                            })
                            .test_support()
                    }),
                ),
        )
}

fn sessions(
    ui: &Rc<Ui>,
    state: &AppState,
    channel: &Channel,
    view: &View,
    filter: Entity<InputState>,
    look: Look,
) -> AnyElement {
    let rows = channel::rows(&state.workspace, &channel.id, view);
    let column = |name: &'static str, width: f32| {
        div()
            .w(px(width))
            .flex_shrink_0()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(look.secondary))
            .child(name)
    };
    col()
        .id("channel-sessions")
        .size_full()
        .overflow_y_scroll()
        .p(px(20.))
        .gap(px(10.))
        .text_size(px(12.5))
        .child(
            row()
                .gap(px(10.))
                .child(
                    div()
                        .id("channel-session-filter")
                        .w(px(240.))
                        .child(
                            Input::new(&filter)
                                .small()
                                .min_h(px(26.))
                                .rounded(px(6.))
                                .bg(rgb(look.card))
                                .border_color(rgb(look.line))
                                .text_size(px(12.5)),
                        )
                        .test_support(),
                )
                .child(
                    row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(
                        [
                            (History::Open, "Open"),
                            (History::Archived, "Archived"),
                            (History::All, "All"),
                        ]
                        .into_iter()
                        .map(|(history, name)| {
                            let ui = ui.clone();
                            div()
                                .id(SharedString::from(format!("channel-history-{name}")))
                                .role(Role::Tab)
                                .aria_label(name)
                                .aria_selected(view.history == history)
                                .px(px(12.))
                                .py(px(2.))
                                .rounded(px(5.))
                                .text_size(px(11.5))
                                .when(view.history == history, |x| {
                                    x.bg(rgb(look.card)).shadow_sm()
                                })
                                .child(name)
                                .on_click(move |_, _, cx| {
                                    ui.send(AppAction::Channel(Action::History(history)), cx)
                                })
                                .test_support()
                        }),
                    ),
                )
                .child(div().flex_1())
                .child(link(
                    ui,
                    "channel-expand-all",
                    "Expand all",
                    AppAction::Channel(Action::ExpandAll),
                    look,
                ))
                .child(link(
                    ui,
                    "channel-collapse-all",
                    "Collapse all",
                    AppAction::Channel(Action::CollapseAll),
                    look,
                )),
        )
        .children(view.clones.as_ref().map(|(host, repo)| {
            row()
                .gap(px(8.))
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(format!("Clones of {repo} on {host}"))
                .child(push_quiet(
                    ui,
                    "channel-clear-clones",
                    "Show all sessions",
                    AppAction::Channel(Action::ClearClones),
                    look,
                ))
        }))
        .child(
            col()
                .id("channel-session-table")
                .w_full()
                .overflow_x_scroll()
                .rounded(px(9.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(look.card))
                .child(
                    row()
                        .min_w(px(948.))
                        .h(px(30.))
                        .px(px(12.))
                        .gap(px(8.))
                        .bg(rgb(look.panel))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(look.secondary))
                                .child("Session"),
                        )
                        .child(column("Agent", 100.))
                        .child(column("Machine", 80.))
                        .child(column("Status", 110.))
                        .child(column("Started", 90.))
                        .child(column("Last used ↓", 90.)),
                )
                .children(rows.iter().map(|r| {
                    let s = r.session;
                    let ui2 = ui.clone();
                    let id = s.id.clone();
                    let children = r.has_children;
                    let machine = state.workspace.machine(&s.machine);
                    let direct = state
                        .workspace
                        .waiting()
                        .filter(|r| r.session == s.id)
                        .count();
                    row()
                        .id(SharedString::from(format!("channel-session-{}", s.id)))
                        .role(Role::Link)
                        .aria_label(s.title.clone())
                        .min_w(px(948.))
                        .h(px(34.))
                        .px(px(12.))
                        .gap(px(8.))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .cursor_pointer()
                        .hover(move |x| x.bg(rgb(look.fill)))
                        .child(
                            row()
                                .flex_1()
                                .min_w_0()
                                .gap(px(6.))
                                .child(div().w(px(12.)).flex_shrink_0().when(children, |x| {
                                    let ui = ui.clone();
                                    let id = id.clone();
                                    x.child(
                                        div()
                                            .id(SharedString::from(format!("channel-fold-{id}")))
                                            .role(Role::Button)
                                            .aria_label("Fold session forks")
                                            .child(chrome::icon(
                                                if r.expanded {
                                                    IconName::ChevronDown
                                                } else {
                                                    IconName::ChevronRight
                                                },
                                                look.tertiary,
                                                12.,
                                            ))
                                            .on_click(move |_, _, cx| {
                                                cx.stop_propagation();
                                                ui.send(
                                                    AppAction::Channel(Action::Fold(id.clone())),
                                                    cx,
                                                );
                                            })
                                            .test_support(),
                                    )
                                }))
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .text_size(px(11.5))
                                        .text_color(rgb(look.tertiary))
                                        .whitespace_nowrap()
                                        .child(r.prefix.clone()),
                                )
                                .child(chrome::status(s.status))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .whitespace_nowrap()
                                        .child(s.title.clone()),
                                )
                                .children(s.fork.map(|kind| {
                                    div()
                                        .px(px(6.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(rgb(look.line))
                                        .text_size(px(11.))
                                        .text_color(rgb(look.secondary))
                                        .whitespace_nowrap()
                                        .child(match kind {
                                            ForkKind::Worktree => "⑂ code + context",
                                            ForkKind::Shared => "⑂ code only",
                                            ForkKind::Conversation => "⑂ context only",
                                        })
                                }))
                                .when(r.pending > 0, |x| {
                                    let ui = ui.clone();
                                    let id = id.clone();
                                    x.child(
                                        Button::new(SharedString::from(format!(
                                            "channel-inbox-{id}"
                                        )))
                                        .accessibility_label(r.pending.to_string())
                                        .small()
                                        .ghost()
                                        .h(px(20.))
                                        .px_0()
                                        .text_color(rgb(humata2_ui_core::theme::RED_STATUS))
                                        .child(
                                            row()
                                                .gap(px(2.))
                                                .text_size(px(11.))
                                                .child(chrome::icon(
                                                    IconName::Inbox,
                                                    humata2_ui_core::theme::RED_STATUS,
                                                    12.,
                                                ))
                                                .child(r.pending.to_string()),
                                        )
                                        .tooltip(format!(
                                            "{direct} from this session, {} from forks",
                                            r.pending - direct
                                        ))
                                        .on_click(
                                            move |_, _, cx| {
                                                cx.stop_propagation();
                                                open(&ui, &id, Some(Panel::Inbox), cx);
                                            },
                                        ),
                                    )
                                }),
                        )
                        .child(div().w(px(100.)).flex_shrink_0().child(agent(s.agent)))
                        .child(
                            div()
                                .w(px(80.))
                                .flex_shrink_0()
                                .text_color(rgb(if s.machine.as_str() == "this-mac" {
                                    look.tertiary
                                } else {
                                    look.text
                                }))
                                .child(machine.map_or("", |m| m.name.as_str()).to_owned()),
                        )
                        .child(
                            div()
                                .w(px(110.))
                                .flex_shrink_0()
                                .text_color(rgb(if s.status == Status::NeedsYou {
                                    humata2_ui_core::theme::ORANGE
                                } else {
                                    look.secondary
                                }))
                                .child({
                                    let state_text = if let Some(pull) = state
                                        .workspace
                                        .pulls
                                        .iter()
                                        .find(|p| p.session.as_ref() == Some(&s.id))
                                        .filter(|_| s.status == Status::Idle)
                                    {
                                        format!("{} · PR #{}", s.status.label(), pull.number)
                                    } else {
                                        s.status.label()
                                    };
                                    // The state reads in the view under the row's link.
                                    div().child(state_text.clone()).test_text(state_text)
                                }),
                        )
                        .child(
                            div()
                                .w(px(90.))
                                .flex_shrink_0()
                                .text_color(rgb(look.secondary))
                                .child(s.started.clone()),
                        )
                        .child(
                            div()
                                .w(px(90.))
                                .flex_shrink_0()
                                .text_color(rgb(look.secondary))
                                .child(s.last_used.clone()),
                        )
                        .on_click(move |_, _, cx| open(&ui2, &id, None, cx))
                        .test_support()
                })),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(
                    "Click a row to open the session. ✉ opens it with its Inbox, forks included.",
                ),
        )
        .test_support()
        .into_any_element()
}

fn activity(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> AnyElement {
    col()
        .id("channel-activity")
        .size_full()
        .overflow_y_scroll()
        .p(px(20.))
        .text_size(px(12.5))
        .child(
            col()
                .w_full()
                .rounded(px(9.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(look.card))
                .overflow_hidden()
                .children(
                    state
                        .workspace
                        .activity
                        .iter()
                        .filter(|e| e.channel == channel.id)
                        .enumerate()
                        .map(|(i, e)| {
                            let (icon, color) = match e.kind {
                                ActivityKind::Inbox => (IconName::Inbox, humata2_ui_core::theme::ORANGE),
                                ActivityKind::Fork => (IconName::GitFork, look.accent),
                                ActivityKind::Turn => (IconName::Check, humata2_ui_core::theme::GREEN),
                                ActivityKind::Item => (IconName::FileText, look.secondary),
                                ActivityKind::Offline => (IconName::WifiOff, humata2_ui_core::theme::GREY),
                                ActivityKind::PullRequest => (IconName::GitPullRequest, 0xbf5af2),
                            };
                            row()
                                .px(px(12.))
                                .py(px(8.))
                                .gap(px(10.))
                                .border_b_1()
                                .border_color(rgb(look.line))
                                .child(chrome::icon(icon, color, 12.))
                                .child(
                                    col()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div().child(e.text.clone()).test_text(e.text.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(rgb(look.secondary))
                                                .child(e.by.clone()),
                                        ),
                                )
                                .children(
                                    e.session
                                        .as_ref()
                                        .filter(|_| {
                                            matches!(
                                                e.kind,
                                                ActivityKind::Inbox
                                                    | ActivityKind::Turn
                                                    | ActivityKind::PullRequest
                                            )
                                        })
                                        .map(|id| {
                                            let ui = ui.clone();
                                            let id = id.clone();
                                            let kind = e.kind;
                                            let url = state
                                                .workspace
                                                .pulls
                                                .iter()
                                                .find(|p| p.session.as_ref() == Some(&id))
                                                .map(|p| {
                                                    format!(
                                                        "https://github.com/{}/pull/{}",
                                                        p.repo, p.number
                                                    )
                                                });
                                            push_quiet_shape(
                                                SharedString::from(format!("channel-event-{i}")),
                                                match kind {
                                                    ActivityKind::Inbox => "Review",
                                                    ActivityKind::Turn => "Diff",
                                                    _ => "Open PR",
                                                },
                                                look,
                                            )
                                            .on_click(move |_, _, cx| {
                                                if kind == ActivityKind::PullRequest {
                                                    if let Some(url) = &url {
                                                        cx.open_url(url);
                                                    }
                                                    return;
                                                }
                                                open(
                                                    &ui,
                                                    &id,
                                                    Some(if kind == ActivityKind::Inbox {
                                                        Panel::Inbox
                                                    } else {
                                                        Panel::Changes
                                                    }),
                                                    cx,
                                                )
                                            })
                                        }),
                                )
                                .child(
                                    div()
                                        .w(px(46.))
                                        .text_right()
                                        .text_size(px(11.5))
                                        .text_color(rgb(look.secondary))
                                        .child(e.at.clone()),
                                )
                        }),
                ),
        )
        .test_support()
        .into_any_element()
}

fn repos(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> AnyElement {
    let mut machines = channel.machines.clone();
    for s in state
        .workspace
        .sessions
        .iter()
        .filter(|s| s.channel == channel.id)
    {
        if !machines.contains(&s.machine) {
            machines.push(s.machine.clone());
        }
    }
    col()
        .id("channel-repos")
        .size_full()
        .overflow_y_scroll()
        .p(px(20.))
        .gap(px(12.))
        .text_size(px(12.5))
        .child(
            col()
                .id("channel-clone-table")
                .w_full()
                .rounded(px(9.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(look.card))
                .overflow_x_scroll()
                .child(
                    row()
                        .h(px(48.))
                        .px(px(14.))
                        .bg(rgb(look.panel))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .child(div().flex_1().min_w(px(180.)))
                        .children(
                            machines
                                .iter()
                                .filter_map(|id| state.workspace.machine(id))
                                .map(|machine| {
                                    let count = state
                                        .workspace
                                        .sessions
                                        .iter()
                                        .filter(|s| {
                                            s.channel == channel.id
                                                && s.machine == machine.id
                                                && !s.archived
                                        })
                                        .count();
                                    col()
                                        .w(px(150.))
                                        .flex_shrink_0()
                                        .child(row().font_weight(FontWeight::SEMIBOLD).child(
                                            link(
                                                ui,
                                                SharedString::from(format!(
                                                    "channel-host-{}",
                                                    machine.id
                                                )),
                                                machine.name.clone(),
                                                AppAction::Hosts(
                                                    humata2_hosts::HostsAction::Select(
                                                        machine.id.clone(),
                                                    ),
                                                ),
                                                look,
                                            ),
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(if machine.online {
                                                    look.secondary
                                                } else {
                                                    humata2_ui_core::theme::GREY
                                                }))
                                                .child(format!(
                                                    "{count} session{}{}",
                                                    if count == 1 { "" } else { "s" },
                                                    if machine.online { "" } else { " · offline" }
                                                )),
                                        )
                                }),
                        ),
                )
                .children(channel.repos.iter().map(|repo| {
                    row()
                        .h(px(38.))
                        .px(px(14.))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .child(
                            row()
                                .flex_1()
                                .min_w(px(180.))
                                .gap(px(7.))
                                .child(chrome::icon(IconName::FolderGit2, look.secondary, 12.))
                                .child(repo.clone()),
                        )
                        .children(machines.iter().map(|machine| {
                            let count = state
                                .workspace
                                .sessions
                                .iter()
                                .filter(|s| {
                                    s.channel == channel.id
                                        && s.machine == *machine
                                        && !s.archived
                                        && channel::has_repo(&state.workspace, s, repo)
                                })
                                .count();
                            div().w(px(150.)).flex_shrink_0().child(if count == 0 {
                                div()
                                    .text_color(rgb(look.tertiary))
                                    .child("—")
                                    .into_any_element()
                            } else {
                                link(
                                    ui,
                                    SharedString::from(format!("channel-clones-{repo}-{machine}")),
                                    format!("{count} clone{}", if count == 1 { "" } else { "s" }),
                                    AppAction::Channel(Action::Clones {
                                        machine: machine.clone(),
                                        repo: repo.clone(),
                                    }),
                                    look,
                                )
                                .into_any_element()
                            })
                        }))
                })),
        )
        .child(
            row()
                .gap(px(8.))
                .child(
                    push_quiet(
                        ui,
                        "channel-add-repo",
                        "Add a GitHub repo…",
                        AppAction::ChannelAddRepo(humata2_app::ChannelAddRepoAction::Open(
                            channel.id.clone(),
                        )),
                        look,
                    ),
                )
                .child(push_quiet(
                    ui,
                    "channel-add-machine",
                    "Add a machine…",
                    AppAction::Hosts(humata2_hosts::HostsAction::StartAdd),
                    look,
                )),
        )
        .test_support()
        .into_any_element()
}

/// The Sessions tab of a channel that has never had a session (frame 35).
fn empty(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> AnyElement {
    let machines = channel
        .machines
        .iter()
        .filter_map(|id| state.workspace.machine(id))
        .map(|m| m.name.as_str())
        .collect::<Vec<_>>();
    let picked = if machines.is_empty() {
        "Add a machine to run sessions there.".to_owned()
    } else {
        format!(
            "{} {} in. Add others to run sessions there.",
            machines.join(", "),
            if machines.len() == 1 { "is" } else { "are" }
        )
    };
    let todo_empty = !state
        .todo
        .items
        .iter()
        .any(|e| e.todo.channel.as_deref() == Some(channel.id.as_str()));
    let step = |glyph: IconName, title: &'static str, note: String, action: Option<Button>| {
        row()
            .gap(px(12.))
            .px(px(14.))
            .py(px(12.))
            .border_b_1()
            .border_color(rgb(look.line))
            .child(chrome::icon(glyph, look.accent, 15.))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::MEDIUM).child(title))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(note),
                    ),
            )
            .children(action)
    };
    div()
        .id("channel-empty")
        .size_full()
        .overflow_y_scroll()
        .flex()
        .items_start()
        .justify_center()
        .p(px(24.))
        .text_size(px(13.))
        .child(
            col()
                .w(px(640.))
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Nothing in # {} yet", channel.name))
                        .test_text(format!("Nothing in # {} yet", channel.name)),
                )
                .child(
                    col()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .bg(rgb(look.card))
                        .overflow_hidden()
                        .child(
                            row()
                                .gap(px(12.))
                                .px(px(14.))
                                .py(px(12.))
                                .border_b_1()
                                .border_color(rgb(look.line))
                                .child(chrome::icon(IconName::Check, humata2_ui_core::theme::GREEN, 15.))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_color(rgb(look.secondary))
                                        .child("Channel made"),
                                ),
                        )
                        .child(step(
                            IconName::FolderGit2,
                            "Add a repo",
                            "Sessions get their own clone of each repo you add.".to_owned(),
                            // CH-6: the frame's filled button opens the Add a GitHub repo sheet.
                            Some(push_primary(
                                ui,
                                "channel-empty-add-repo",
                                "Add a GitHub repo…",
                                AppAction::ChannelAddRepo(humata2_app::ChannelAddRepoAction::Open(
                                    channel.id.clone(),
                                )),
                                look,
                            )),
                        ))
                        .child(step(
                            IconName::Server,
                            "Pick machines",
                            picked,
                            Some(push_quiet(
                                ui,
                                "channel-empty-add-machine",
                                "Add a machine…",
                                AppAction::Hosts(humata2_hosts::HostsAction::StartAdd),
                                look,
                            )),
                        ))
                        .child(step(
                            IconName::SquareTerminal,
                            "Start a session",
                            "An agent's TUI or a plain shell. Works without a repo, in an empty folder."
                                .to_owned(),
                            Some(push_quiet(
                                ui,
                                "channel-empty-new-session",
                                "New session",
                                AppAction::StartNewSession,
                                look,
                            )),
                        )),
                )
                .children(todo_empty.then(|| {
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(
                            "The To-do list is empty too: add items from the To-do tab, or ask an agent to.",
                        )
                })),
        )
        .test_support()
        .into_any_element()
}

pub fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    channel: &Channel,
    filter: Entity<InputState>,
    todo: &dyn Fn() -> AnyElement,
    look: Look,
) -> AnyElement {
    let view = page_view(state, &channel.id);
    match view.tab {
        Tab::Sessions
            if !state
                .workspace
                .sessions
                .iter()
                .any(|s| s.channel == channel.id) =>
        {
            empty(ui, state, channel, look)
        }
        Tab::Sessions => sessions(ui, state, channel, &view, filter, look),
        Tab::Desk => humata2_ui_desk::channel_desk::tab(ui, state, channel, look),
        Tab::Activity => activity(ui, state, channel, look),
        Tab::Repos => repos(ui, state, channel, look),
        Tab::Inbox => inbox_page::inbox(ui, state, look).into_any_element(),
        Tab::Board => todo(),
        Tab::Access => humata2_ui_settings::access::tab(ui, state, channel, look).into_any_element(),
    }
}
