//! The real session's terminal/file row and its only split: two tab groups.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Agent, Session, Status, Turn};
use humata2_session::{SessionAction, Tab, TabGroup, is_markdown};
use humata2_views::ViewsAction;

use crate::{
    bridge::Ui,
    chrome,
    code_editor,
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

/// A tab being dragged to the other group (SPEC 26.20): where it is now.
#[derive(Clone)]
struct TabDrag {
    group: usize,
    index: usize,
    label: String,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .py(px(5.))
            .rounded(px(6.))
            .bg(rgb(0x323235))
            .text_size(px(12.))
            .text_color(rgb(0xffffff))
            .child(self.label.clone())
    }
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
                    chrome::icon(
                        if is_markdown(path) {
                            IconName::FileText
                        } else {
                            IconName::FileCode
                        },
                        look.secondary,
                        12.,
                    )
                    .into_any_element(),
                    format!("file-tab-{group}-{index}"),
                ),
                Tab::View(title) => (
                    title.clone(),
                    chrome::icon(IconName::Sparkles, 0xbf5af2, 12.).into_any_element(),
                    format!("view-tab-{group}-{index}"),
                ),
                Tab::Conversation => (
                    "Conversation".to_owned(),
                    chrome::icon(IconName::FileText, look.secondary, 12.).into_any_element(),
                    format!("conversation-tab-{group}-{index}"),
                ),
            };
            let ui2 = ui.clone();
            let drag = TabDrag {
                group,
                index,
                label: name.clone(),
            };
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
                .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
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
        .children(crate::document_tab::mode_switch(ui, state, look, group, tabs))
        .when(session.status != Status::Offline, |bar| {
            let peers = tabs
                .tabs
                .get(tabs.selected)
                .and_then(|tab| match tab {
                    Tab::Terminal(tab) => state
                        .workspace
                        .terminal_peers
                        .get(&(session.id.clone(), *tab)),
                    _ => None,
                })
                .copied()
                .unwrap_or(0);
            bar.when(
                session.status == Status::Working && peers > 0 && !split,
                |bar| {
                    let label = format!(
                        "{peers} other{} can type here",
                        if peers == 1 { "" } else { "s" }
                    );
                    bar.child(
                        div()
                            .id(SharedString::from(format!("terminal-peers-{group}")))
                            .mr(px(12.))
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(label.clone())
                            .test_text(label)
                            .test_support(),
                    )
                },
            )
        })
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
        .drag_over::<TabDrag>(move |style, _, _, _| style.bg(rgba(look.accent << 8 | 0x26)))
        .on_drop({
            let ui = ui.clone();
            move |drag: &TabDrag, _, cx| {
                // Dropped on its own row, a tab stays where it is.
                if drag.group != group {
                    ui.send(
                        AppAction::Session(SessionAction::MoveTab {
                            group: drag.group,
                            index: drag.index,
                        }),
                        cx,
                    );
                }
            }
        })
}

fn notice(
    look: Look,
    glyph: IconName,
    color: u32,
    text: String,
    action: Option<AnyElement>,
) -> AnyElement {
    // Terminals::body places it under the terminal's content.
    row()
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .text_size(px(12.))
        .child(chrome::icon(glyph, color, 12.))
        .child(div().flex_1().min_w_0().child(text))
        .children(action)
        .into_any_element()
}

fn menu(ui: &Rc<Ui>, group: usize, look: Look) -> Stateful<Div> {
    crate::action_menu::card("new-terminal-menu", 280., look)
        .top(px(32.))
        .left(px(8.))
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
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    documents: &crate::document_tab::Documents,
    look: Look,
) -> AnyElement {
    let content =
        state
            .session
            .file_contents
            .get(&(session.id.clone(), repo.to_owned(), path.to_owned()));
    if let (Some(Ok(text)), Some(diff)) = (
        content,
        code_editor::diff(state, &session.id, repo, path),
    ) {
        return code_editor::body(
            ui,
            state,
            session,
            repo,
            path,
            text,
            diff,
            documents,
            look,
        );
    }
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

pub(crate) fn body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
    terminals: &crate::terminals::Terminals,
    documents: &crate::document_tab::Documents,
    cx: &mut App,
) -> Div {
    let fallback = TabGroup {
        tabs: (0..=session.shells.len()).map(Tab::Terminal).collect(),
        selected: state.session.view().map_or(0, |v| v.tab),
    };
    let dragging = cx.has_active_drag();
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
                        .h(px(40.))
                        .items_center()
                        .px(px(14.))
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
                        )
                        .outline()
                        .h(px(24.))
                        .px(px(11.))
                        .bg(rgb(look.fill))
                        .text_size(px(12.)))
                            .child(widgets::ghost_in(look,
                            ui,
                            "offer-later",
                            "Later",
                            AppAction::Views(ViewsAction::Later(offer.title.clone())),
                        ))
                        .child(widgets::ghost_in(look,
                            ui,
                            "offer-dismiss",
                            "Not now",
                            AppAction::Views(ViewsAction::Dismiss(offer.title.clone())),
                        ))
                }),
        )
        .children(state.session.error.clone().map(|error| {
            widgets::error_banner_in(look,
                ui,
                "session-error",
                error,
                AppAction::Session(SessionAction::DismissError),
            )
        }))
        .children(crate::desk_bar::bar(ui, state, session, look))
        .child(
            row().flex_1().min_h_0().items_stretch().children(
                groups
                    .iter()
                    .enumerate()
                    .map(|(group, tabs)| {
                        let selected = tabs.tabs.get(tabs.selected);
                        col()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .when(group > 0, |g| g.border_l_1().border_color(rgb(look.line)))
                            .child(tab_row(ui, state, session, group, tabs, look))
                            // One group: while a tab is dragged, dropping it on the
                            // right edge opens the second group beside it. The zone
                            // exists only during a drag, so it never takes the
                            // terminal's clicks.
                            .when(groups.len() == 1 && tabs.tabs.len() > 1 && dragging, |g| {
                                let ui = ui.clone();
                                g.child(
                                    div()
                                        .id("tab-drop-right")
                                        .absolute()
                                        .top(px(32.))
                                        .right(px(0.))
                                        .bottom(px(0.))
                                        .w(px(96.))
                                        .drag_over::<TabDrag>(move |style, _, _, _| {
                                            style.bg(rgba(look.accent << 8 | 0x26))
                                        })
                                        .on_drop(move |drag: &TabDrag, _, cx| {
                                            ui.send(
                                                AppAction::Session(SessionAction::MoveTab {
                                                    group: drag.group,
                                                    index: drag.index,
                                                }),
                                                cx,
                                            )
                                        })
                                        .test_support(),
                                )
                            })
                            .child(match selected {
                                Some(Tab::File { repo, path }) => documents
                                    .body(ui, state, session, repo, path, look)
                                    .filter(|_| is_markdown(path))
                                    .unwrap_or_else(|| {
                                        file_body(ui, state, session, repo, path, documents, look)
                                    }),
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
                                    let notice = match (*tab, session.status) {
                                        (0, Status::Exited { code }) => Some(notice(
                                            look,
                                            IconName::CircleX,
                                            0xff453a,
                                            format!(
                                                "{} exited with code {code}. Resume starts it again with this conversation.",
                                                session.agent.command()
                                            ),
                                            Some(
                                                widgets::send(
                                                    ui,
                                                    "resume-terminal",
                                                    "Resume",
                                                    AppAction::Session(SessionAction::Resume),
                                                )
                                                .outline()
                                                .h(px(24.))
                                                .px(px(11.))
                                                .into_any_element(),
                                            ),
                                        )),
                                        (_, Status::Offline) => {
                                            let found = state
                                                .workspace
                                                .machines
                                                .iter()
                                                .find(|x| x.id == session.machine);
                                            let machine = found.map_or("the machine", |x| x.name.as_str());
                                            let lost = found
                                                .and_then(|x| x.lost_at.as_deref())
                                                .map(|at| format!(" at {at}"))
                                                .unwrap_or_default();
                                            Some(notice(
                                                look,
                                                IconName::WifiOff,
                                                0x8e8e93,
                                                format!(
                                                    "Lost the connection to {machine}{lost}. The agent keeps running there; this view catches up when it is back."
                                                ),
                                                None,
                                            ))
                                        }
                                        _ => None,
                                    };
                                    // After a host restart (PER-15, PER-29, frame 46): the line that
                                    // marks it under the old output and, when the agent couldn't be
                                    // resumed, the card that says why, in place of the exit notice.
                                    let (reboot, failed) = if *tab == 0 {
                                        (
                                            crate::resume_failed::reboot_line(state, session, look),
                                            crate::resume_failed::note(ui, state, session, look),
                                        )
                                    } else {
                                        (None, None)
                                    };
                                    let notice = if reboot.is_some() || failed.is_some() {
                                        let usual = if failed.is_some() { None } else { notice };
                                        Some(
                                            col()
                                                .w_full()
                                                .gap(px(8.))
                                                .children(reboot)
                                                .children(failed)
                                                .children(usual)
                                                .into_any_element(),
                                        )
                                    } else {
                                        notice
                                    };
                                    terminals.set_offline_banner(
                                        &session.id,
                                        *tab,
                                        session.status != Status::Offline,
                                        cx,
                                    );
                                    let body = terminals.body(&session.id, *tab, group, notice, cx);
                                    col().flex_1().min_h_0().bg(rgb(look.term)).child(body).into_any_element()
                                }
                                Some(Tab::Conversation) => crate::conversation_tab::body(ui, state, session, look, &documents.conversation),
                                None => div().into_any_element(),
                            })
                            .when(state.session.terminal_menu == Some(group), |g| {
                                g.child(crate::action_menu::scrim(
                                    ui,
                                    "close-terminal-menu-scrim",
                                    AppAction::Session(SessionAction::TerminalMenu(None)),
                                ))
                                .child(menu(ui, group, look).test_support())
                            })
                    }),
            ),
        )
        .children(queue(ui, state, session, look))
}

fn queue(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> Option<AnyElement> {
    let view = state.session.view()?;
    if view.queue.is_empty() {
        return None;
    }
    let failed = state.session.failed_queues.contains(&session.id);
    let certain = !failed
        && matches!(session.agent, Agent::Claude | Agent::Copilot)
        && session.turn != Turn::Uncertain;
    let sending = state.session.sending;
    let waits = certain;
    let count = view.queue.len();
    let noun = "message";
    // SE-23 and D128.3: the bar says "waiting to send"; held text that goes
    // on its own at the turn's end needs no further note.
    let status = if failed {
        Some("send failed")
    } else if sending {
        Some("sending")
    } else if waits {
        None
    } else {
        Some("send manually")
    };
    let caption = format!(
        "{count} {noun}{} waiting to send{}",
        if count == 1 { "" } else { "s" },
        status
            .map(|status| format!(" · {status}"))
            .unwrap_or_default()
    );
    Some(
        col()
            .id("session-queue")
            .flex_shrink_0()
            .m(px(14.))
            .mt(px(0.))
            .mb(px(8.))
            .px(px(10.))
            .py(px(6.))
            .gap(px(8.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .text_color(rgb(look.text))
            .text_size(px(12.))
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        chrome::icon(IconName::MessageSquareText, look.secondary, 12.)
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(caption.clone())
                            .test_text(caption),
                    )
                    .when(!waits && !sending, |bar| {
                        bar.child(
                            widgets::send(
                                ui,
                                "send-next-queued",
                                "Send now",
                                AppAction::Session(SessionAction::SendNow),
                            )
                            .disabled(state.session.sending),
                        )
                    })
                    .child(
                        widgets::ghost_in(look,
                            ui,
                            "show-queued",
                            if state.session.queue_expanded {
                                "Hide"
                            } else {
                                "Show"
                            },
                            AppAction::Session(SessionAction::ShowQueue(
                                !state.session.queue_expanded,
                            )),
                        )
                        .h(px(18.))
                        .px(px(0.))
                        .text_color(rgb(look.accent)),
                    ),
            )
            .when(state.session.queue_expanded, |list| {
                list.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child("Humata holds these messages before sending them to the terminal."),
                )
                .child(
                    col()
                        .id("queued-list")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .children(view.queue.iter().enumerate().map(|(index, message)| {
                            let message = message.text();
                            row()
                                .id(SharedString::from(format!("queued-message-{index}")))
                                .items_start()
                                .gap(px(8.))
                                .py(px(5.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(message.clone())
                                        .test_text(message.clone()),
                                )
                                .child(
                                    widgets::send(
                                        ui,
                                        SharedString::from(format!("send-queued-{index}")),
                                        "Send now",
                                        AppAction::Session(SessionAction::SendQueued(index)),
                                    )
                                    .disabled(state.session.sending),
                                )
                                .child(
                                    widgets::ghost_in(look,
                                        ui,
                                        SharedString::from(format!("remove-queued-{index}")),
                                        "Remove",
                                        AppAction::Session(SessionAction::RemoveQueued(index)),
                                    )
                                    .disabled(state.session.sending),
                                )
                                .test_support()
                        })),
                )
            })
            .test_support()
            .into_any_element(),
    )
}
