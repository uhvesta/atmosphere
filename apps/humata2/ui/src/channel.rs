//! Channel pages fitted to frames 18–20, over client data and saved page state.
use crate::{
    bridge::Ui,
    chrome, lists,
    theme::Look,
    widgets::{self, col, row},
};
use gpui_kit::assets::IconName;
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
use humata2_clients::model::{
    ActivityKind, Agent, Channel, ChannelId, ForkKind, Scope, SessionId, Status,
};
use humata2_session::{Panel, SessionAction};
use std::rc::Rc;

fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .label(label)
        .small()
        .h(px(24.))
        .px(px(10.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(action.clone(), cx);
        })
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

pub(crate) fn header(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> Div {
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
    let machines = channel
        .machines
        .iter()
        .filter_map(|id| state.workspace.machine(id))
        .map(|m| m.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
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
                        .child(format!("{} · {machines}", channel.repos.join(", "))),
                )
                .child(
                    widgets::primary(
                        ui,
                        "channel-new-session",
                        "New session",
                        AppAction::StartNewSession,
                    )
                    .small()
                    .h(px(24.))
                    .bg(rgb(look.accent))
                    .text_color(rgb(0xffffff)),
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
                        (Tab::Sessions, "Sessions", Some(sessions)),
                        (Tab::Inbox, "Inbox", (waiting > 0).then_some(waiting)),
                        (Tab::Board, "Board", None),
                        (Tab::Repos, "Repos and machines", None),
                        (Tab::Access, "Agent access", None),
                        (Tab::Activity, "Activity", None),
                    ]
                    .into_iter()
                    .map(|(tab, name, count)| {
                        let ui = ui.clone();
                        row()
                            .id(SharedString::from(format!("channel-tab-{name}")))
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
                                div()
                                    .px(px(6.))
                                    .py(px(1.))
                                    .rounded_full()
                                    .bg(rgb(look.fill))
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
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
                        .child(Input::new(&filter).small().h(px(26.)))
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
                                .rounded(px(6.))
                                .text_size(px(12.))
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
                .child(
                    button(
                        ui,
                        "channel-expand-all",
                        "Expand all",
                        AppAction::Channel(Action::ExpandAll),
                        look,
                    )
                    .ghost()
                    .bg(rgba(0))
                    .px_0()
                    .text_color(rgb(look.accent)),
                )
                .child(
                    button(
                        ui,
                        "channel-collapse-all",
                        "Collapse all",
                        AppAction::Channel(Action::CollapseAll),
                        look,
                    )
                    .ghost()
                    .bg(rgba(0))
                    .px_0()
                    .text_color(rgb(look.accent)),
                ),
        )
        .children(view.clones.as_ref().map(|(host, repo)| {
            row()
                .gap(px(8.))
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(format!("Clones of {repo} on {host}"))
                .child(button(
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
                        .min_w(px(850.))
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
                        .child(column("Started", 90.)),
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
                        .min_w(px(850.))
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
                                        .small()
                                        .ghost()
                                        .h(px(20.))
                                        .px(px(2.))
                                        .text_color(rgb(0xff453a))
                                        .icon(IconName::Inbox)
                                        .label(r.pending.to_string())
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
                                    0xff9f0a
                                } else {
                                    look.secondary
                                }))
                                .child(
                                    if let Some(pull) = state
                                        .workspace
                                        .pulls
                                        .iter()
                                        .find(|p| p.session.as_ref() == Some(&s.id))
                                        .filter(|_| s.status == Status::Idle)
                                    {
                                        format!("{} · PR #{}", s.status.label(), pull.number)
                                    } else {
                                        s.status.label()
                                    },
                                ),
                        )
                        .child(
                            div()
                                .w(px(90.))
                                .flex_shrink_0()
                                .text_color(rgb(look.secondary))
                                .child(s.started.clone()),
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
                                ActivityKind::Inbox => (IconName::Inbox, 0xff9f0a),
                                ActivityKind::Fork => (IconName::GitFork, look.accent),
                                ActivityKind::Turn => (IconName::Check, 0x30d158),
                                ActivityKind::Item => (IconName::FileText, look.secondary),
                                ActivityKind::Offline => (IconName::WifiOff, 0x8e8e93),
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
                                            Button::new(SharedString::from(format!(
                                                "channel-event-{i}"
                                            )))
                                            .small()
                                            .h(px(24.))
                                            .label(match kind {
                                                ActivityKind::Inbox => "Review",
                                                ActivityKind::Turn => "Diff",
                                                _ => "Open PR",
                                            })
                                            .bg(rgb(look.fill))
                                            .text_color(rgb(look.text))
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
                                        .child(
                                            button(
                                                ui,
                                                SharedString::from(format!(
                                                    "channel-host-{}",
                                                    machine.id
                                                )),
                                                machine.name.clone(),
                                                AppAction::Navigate(Route::Hosts),
                                                look,
                                            )
                                            .ghost()
                                            .bg(rgba(0))
                                            .px_0()
                                            .justify_start()
                                            .text_color(rgb(look.accent)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(if machine.online {
                                                    look.secondary
                                                } else {
                                                    0x8e8e93
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
                                button(
                                    ui,
                                    SharedString::from(format!("channel-clones-{repo}-{machine}")),
                                    format!("{count} clone{}", if count == 1 { "" } else { "s" }),
                                    AppAction::Channel(Action::Clones {
                                        machine: machine.clone(),
                                        repo: repo.clone(),
                                    }),
                                    look,
                                )
                                .ghost()
                                .bg(rgba(0))
                                .px_0()
                                .justify_start()
                                .text_color(rgb(look.accent))
                                .into_any_element()
                            })
                        }))
                })),
        )
        .child(
            row()
                .gap(px(8.))
                .child(
                    Button::new("channel-add-repo")
                        .small()
                        .label("Add a GitHub repo…")
                        .disabled(true),
                )
                .child(
                    Button::new("channel-add-machine")
                        .small()
                        .label("Add a machine…")
                        .disabled(true),
                ),
        )
        .test_support()
        .into_any_element()
}

pub(crate) fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    channel: &Channel,
    filter: Entity<InputState>,
    look: Look,
) -> AnyElement {
    let view = page_view(state, &channel.id);
    match view.tab {
        Tab::Sessions => sessions(ui, state, channel, &view, filter, look),
        Tab::Activity => activity(ui, state, channel, look),
        Tab::Repos => repos(ui, state, channel, look),
        Tab::Inbox => lists::inbox(ui, state, look).into_any_element(),
        Tab::Board => col()
            .p(px(20.))
            .child(button(
                ui,
                "channel-open-board",
                "Open channel board",
                AppAction::Navigate(Route::Board(Scope::Channel(channel.id.clone()))),
                look,
            ))
            .into_any_element(),
        Tab::Access => col()
            .p(px(20.))
            .child(button(
                ui,
                "channel-open-access",
                "Agent access settings",
                AppAction::Navigate(Route::Settings),
                look,
            ))
            .into_any_element(),
    }
}
