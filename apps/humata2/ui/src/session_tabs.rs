//! The real session's terminal/file row and its only split: two tab groups.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Agent, Session, Status};
use humata2_session::{SessionAction, Tab, TabGroup};
use humata2_views::ViewsAction;

use crate::{
    bridge::Ui,
    chrome, pages,
    theme::Look,
    widgets::{self, col, row},
};

fn send(ui: &Rc<Ui>, id: String, label: &str, action: SessionAction, look: Look) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id(SharedString::from(id))
        .role(Role::Button)
        .aria_label(label.to_owned())
        .cursor_pointer()
        .text_color(rgb(look.secondary))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(AppAction::Session(action.clone()), cx);
        })
}

fn tab_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    group: usize,
    tabs: &TabGroup,
    look: Look,
) -> Stateful<Div> {
    let split = state.session.view().is_some_and(|v| v.groups.len() == 2);
    row()
        .id(SharedString::from(format!("session-tabs-{group}")))
        .h(px(32.))
        .flex_shrink_0()
        .overflow_x_scroll()
        .bg(rgb(look.panel))
        .border_b_1()
        .border_color(rgb(look.line))
        .children(tabs.tabs.iter().enumerate().map(|(index, tab)| {
            let (name, glyph, id) = match tab {
                Tab::Terminal(tab) => (
                    if *tab == 0 {
                        session.agent.command().to_owned()
                    } else {
                        session
                            .shells
                            .get(tab - 1)
                            .cloned()
                            .unwrap_or_else(|| "terminal".into())
                    },
                    if *tab == 0 || session.terminal_agents.contains_key(tab) {
                        chrome::status(if *tab == 0 {
                            session.status
                        } else {
                            Status::Working
                        })
                        .into_any_element()
                    } else {
                        chrome::icon(IconName::SquareTerminal, look.secondary, 12.)
                            .into_any_element()
                    },
                    if group == 0 {
                        format!("terminal-tab {tab}")
                    } else {
                        format!("terminal-tab-{group}-{tab}")
                    },
                ),
                Tab::File { path, .. } => (
                    path.rsplit('/').next().unwrap_or(path).to_owned(),
                    chrome::icon(IconName::FileCode, look.secondary, 12.).into_any_element(),
                    format!("file-tab-{group}-{index}"),
                ),
                Tab::View(title) => (
                    title.clone(),
                    chrome::icon(IconName::Sparkles, 0xbf5af2, 12.).into_any_element(),
                    format!("view-tab-{group}-{index}"),
                ),
            };
            let ui2 = ui.clone();
            row()
                .id(SharedString::from(id))
                .role(Role::Tab)
                .aria_label(name.clone())
                .aria_selected(index == tabs.selected)
                .h_full()
                .flex_shrink_0()
                .px(px(12.))
                .gap(px(6.))
                .border_r_1()
                .border_color(rgb(look.line))
                .when(index == tabs.selected, |t| t.bg(rgb(look.term)))
                .text_size(px(12.))
                .text_color(rgb(if index == tabs.selected {
                    look.text
                } else {
                    look.secondary
                }))
                .child(glyph)
                .child(name)
                .when(!matches!(tab, Tab::Terminal(_)), |t| {
                    t.child(
                        send(
                            ui,
                            format!("close-tab-{group}-{index}"),
                            "Close tab",
                            SessionAction::CloseTab { group, index },
                            look,
                        )
                        .child(chrome::icon(IconName::X, look.tertiary, 12.))
                        .test_support(),
                    )
                })
                .on_click(move |_, _, cx| {
                    ui2.send(
                        AppAction::Session(SessionAction::SelectGroupTab { group, index }),
                        cx,
                    )
                })
                .test_support()
        }))
        .child(
            send(
                ui,
                format!("add-tab-{group}"),
                "New terminal",
                SessionAction::TerminalMenu(if state.session.terminal_menu == Some(group) {
                    None
                } else {
                    Some(group)
                }),
                look,
            )
            .px(px(10.))
            .h_full()
            .flex()
            .items_center()
            .child(chrome::icon(IconName::Plus, look.secondary, 12.))
            .test_support(),
        )
        .child(div().flex_1())
        .when(split, |bar| {
            bar.child(
                send(
                    ui,
                    format!("close-group-{group}"),
                    "Join tab groups",
                    SessionAction::CloseGroup(group),
                    look,
                )
                .mr(px(10.))
                .child(chrome::icon(IconName::X, look.tertiary, 12.))
                .test_support(),
            )
        })
        .when(!split && session.status == Status::Offline, |bar| {
            bar.child(
                row()
                    .mr(px(12.))
                    .gap(px(5.))
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(chrome::icon(IconName::WifiOff, look.secondary, 12.))
                    .child("disconnected"),
            )
        })
}

fn menu(ui: &Rc<Ui>, group: usize, look: Look) -> Stateful<Div> {
    col()
        .id("new-terminal-menu")
        .absolute()
        .top(px(32.))
        .left(px(8.))
        .w(px(280.))
        .p(px(5.))
        .gap(px(2.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .rounded(px(7.))
        .shadow_lg()
        .children(Agent::ALL.into_iter().map(|agent| {
            let name = if agent == Agent::Shell {
                "New terminal"
            } else {
                agent.name()
            };
            row()
                .gap(px(6.))
                .child(
                    send(
                        ui,
                        format!("new-terminal-{group}-{}", agent.command()),
                        name,
                        SessionAction::OpenTerminal {
                            agent,
                            split: false,
                        },
                        look,
                    )
                    .flex_1()
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .hover(move |x| x.bg(rgb(look.fill)))
                    .text_color(rgb(look.text))
                    .child(name)
                    .test_support(),
                )
                .child(
                    send(
                        ui,
                        format!("split-terminal-{group}-{}", agent.command()),
                        "Open in second group",
                        SessionAction::OpenTerminal { agent, split: true },
                        look,
                    )
                    .px(px(6.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .hover(move |x| x.bg(rgb(look.fill)))
                    .child(chrome::icon(IconName::Columns2, look.secondary, 14.))
                    .test_support(),
                )
        }))
        .child(
            send(
                ui,
                "close-terminal-menu".into(),
                "Cancel",
                SessionAction::TerminalMenu(None),
                look,
            )
            .px(px(8.))
            .py(px(5.))
            .child("Cancel")
            .test_support(),
        )
}

fn file_body(
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    look: Look,
) -> AnyElement {
    let content =
        state
            .session
            .file_contents
            .get(&(session.id.clone(), repo.to_owned(), path.to_owned()));
    col()
        .id("session-file")
        .role(Role::Document)
        .aria_label(format!("{repo}/{path}"))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .bg(rgb(look.term))
        .text_color(rgb(look.text))
        .child(
            div()
                .px(px(14.))
                .py(px(8.))
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(format!("{repo}/{path}")),
        )
        .child(match content {
            Some(Ok(text)) => col()
                .font_family("Menlo")
                .text_size(px(12.))
                .line_height(px(18.))
                .children(text.lines().enumerate().map(|(index, line)| {
                    row()
                        .gap(px(12.))
                        .px(px(12.))
                        .child(
                            div()
                                .w(px(30.))
                                .text_right()
                                .text_color(rgb(look.tertiary))
                                .child((index + 1).to_string()),
                        )
                        .child(
                            div()
                                .whitespace_nowrap()
                                .child(line.to_owned())
                                .test_text(line.to_owned()),
                        )
                }))
                .into_any_element(),
            Some(Err(error)) => div()
                .p(px(14.))
                .child(error.to_string())
                .test_text(error.to_string())
                .into_any_element(),
            None => div().p(px(14.)).child("Loading file…").into_any_element(),
        })
        .test_support()
        .into_any_element()
}

pub(crate) fn body(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> Div {
    let fallback = TabGroup {
        tabs: (0..=session.shells.len()).map(Tab::Terminal).collect(),
        selected: state.session.view().map_or(0, |v| v.tab),
    };
    let groups = state
        .session
        .view()
        .map(|v| v.groups.as_slice())
        .filter(|g| !g.is_empty())
        .unwrap_or(std::slice::from_ref(&fallback));
    col()
        .size_full()
        .children(
            state
                .views
                .offers()
                .into_iter()
                .filter(|v| v.session == session.id)
                .take(1)
                .map(|offer| {
                    row()
                        .px(px(14.))
                        .py(px(7.))
                        .gap(px(10.))
                        .flex_shrink_0()
                        .bg(rgb(if look.term == 0xffffff {
                            0xfff8e7
                        } else {
                            0x352d1d
                        }))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .text_size(px(12.5))
                        .child(chrome::icon(IconName::Sparkles, 0xbf5af2, 12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(rgb(look.text))
                                .child(format!(
                                    "{} prepared a view: {}",
                                    if session.agent == Agent::Claude {
                                        "Claude Code"
                                    } else {
                                        session.agent.name()
                                    },
                                    offer.title
                                )),
                        )
                        .child(widgets::send(
                            ui,
                            "offer-open",
                            "Open it",
                            AppAction::Views(ViewsAction::Accept(offer.title.clone())),
                        ))
                        .child(widgets::ghost(
                            ui,
                            "offer-later",
                            "Later",
                            AppAction::Views(ViewsAction::Later(offer.title.clone())),
                        ))
                        .child(widgets::ghost(
                            ui,
                            "offer-dismiss",
                            "Not now",
                            AppAction::Views(ViewsAction::Dismiss(offer.title.clone())),
                        ))
                }),
        )
        .children(state.session.error.clone().map(|error| {
            widgets::error_banner(
                ui,
                "session-error",
                error,
                AppAction::Session(SessionAction::DismissError),
            )
        }))
        .child(
            row()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .children(groups.iter().enumerate().map(|(group, tabs)| {
                    let selected = tabs.tabs.get(tabs.selected);
                    col()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .when(group > 0, |g| g.border_l_1().border_color(rgb(look.line)))
                        .child(tab_row(ui, state, session, group, tabs, look))
                        .child(match selected {
                            Some(Tab::File { repo, path }) => {
                                file_body(state, session, repo, path, look)
                            }
                            Some(Tab::View(title)) => {
                                let view = state
                                    .workspace
                                    .views
                                    .iter()
                                    .find(|v| v.session == session.id && v.title == *title);
                                col()
                                    .id("prepared-view-content")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p(px(18.))
                                    .gap(px(12.))
                                    .bg(rgb(look.content))
                                    .text_color(rgb(look.text))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title.clone()),
                                    )
                                    .children(view.map(|v| {
                                        div().child(v.body.clone()).test_text(v.body.clone())
                                    }))
                                    .test_support()
                                    .into_any_element()
                            }
                            Some(Tab::Terminal(tab)) => {
                                pages::terminal_body(ui, state, session, *tab, group, look)
                                    .into_any_element()
                            }
                            None => div().into_any_element(),
                        })
                        .when(state.session.terminal_menu == Some(group), |g| {
                            g.child(menu(ui, group, look).test_support())
                        })
                })),
        )
}
