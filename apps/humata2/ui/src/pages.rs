//! The screens in the middle of the window, one per route.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_app::{PaneAction, layout::PaneId};
use humata2_clients::model::{Agent, ForkKind, Session, Speaker, Status};
use humata2_session::SessionAction;

use crate::bridge::Ui;
use crate::widgets::{self, col, row};

/// The screen's title, for the top bar.
#[must_use]
pub fn title(state: &AppState) -> String {
    match &state.route {
        Route::Home => "Home".to_owned(),
        Route::Inbox => "Inbox".to_owned(),
        Route::Board(_) => "Board".to_owned(),
        Route::Channel(id) => state
            .workspace
            .channels
            .iter()
            .find(|channel| &channel.id == id)
            .map_or_else(
                || "Channel".to_owned(),
                |channel| format!("#{}", channel.name),
            ),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(|| "Session".to_owned(), |session| session.title.clone()),
        Route::Hosts => "Hosts".to_owned(),
        Route::Files => "Files".to_owned(),
        Route::Views => "Views".to_owned(),
        Route::Settings => "Settings".to_owned(),
    }
}

/// The screen for the current route.
pub fn page(ui: &Rc<Ui>, state: &AppState, _window: &mut Window, cx: &mut App) -> AnyElement {
    match &state.route {
        Route::Home => home(ui, state, Look::of(cx)).into_any_element(),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(
                || {
                    widgets::empty("This session is gone.", "Pick another in the sidebar.")
                        .into_any_element()
                },
                |session| session_body(ui, state, session, Look::of(cx)).into_any_element(),
            ),
        _ => widgets::empty("Nothing here yet.", "This screen is being built.").into_any_element(),
    }
}

fn session_row(
    ui: &Rc<Ui>,
    session: &Session,
    index: usize,
    group: &'static str,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    let id = session.id.clone();
    widgets::list_item((group, index), Role::Link, session.title.clone(), false)
        .h(px(34.))
        .child(
            widgets::dot(session.status)
                .id((group, index + 10_000))
                .tooltip({
                    let words = session.status.label();
                    move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(words.clone()).build(window, cx)
                    }
                }),
        )
        .child(div().flex_1().child(session.title.clone()))
        .text_color(rgb(look.text))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(session.active.clone()),
        )
        .on_click(move |_, _, cx| ui.send(AppAction::Navigate(Route::Session(id.clone())), cx))
        .test_support()
}

pub(crate) fn home(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let needs: Vec<&Session> = state
        .workspace
        .sessions
        .iter()
        .filter(|session| matches!(session.status, Status::NeedsYou) && !session.archived)
        .collect();
    let working: Vec<&Session> = state
        .workspace
        .sessions
        .iter()
        .filter(|session| matches!(session.status, Status::Working) && !session.archived)
        .collect();
    let section = |heading: &'static str,
                   group: &'static str,
                   sessions: Vec<&Session>,
                   none: &'static str| {
        col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.secondary))
                    .child(heading),
            )
            .children(
                sessions
                    .iter()
                    .enumerate()
                    .map(|(index, session)| session_row(ui, session, index, group, look)),
            )
            .when(sessions.is_empty(), |list| list.child(widgets::dim(none)))
    };
    col()
        .id("home")
        .size_full()
        .overflow_y_scroll()
        .p_6()
        .child(
            col()
                .w_full()
                .max_w(px(880.))
                .mx_auto()
                .gap_6()
                .child(section(
                    "Needs you",
                    "home-needs",
                    needs,
                    "Nothing is waiting on you.",
                ))
                .child(section(
                    "Working",
                    "home-working",
                    working,
                    "No agent is working right now.",
                )),
        )
}

pub(crate) fn session_header(ui: &Rc<Ui>, state: &AppState, collapsed: bool, look: Look) -> Div {
    let Some(session) = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
    else {
        return div();
    };
    let channel = state
        .workspace
        .channels
        .iter()
        .find(|x| x.id == session.channel);
    let machine = state
        .workspace
        .machines
        .iter()
        .find(|x| x.id == session.machine);
    let mut crumbs = row()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary));
    let event = AppAction::Switch(Route::Channel(session.channel.clone()));
    let ui2 = ui.clone();
    crumbs = crumbs
        .child(
            div()
                .id("session-channel")
                .role(Role::Link)
                .aria_label("Open channel")
                .text_color(rgb(look.accent))
                .child(format!("# {}", channel.map_or("", |x| x.name.as_str())))
                .on_click(move |_, _, cx| ui2.send(event.clone(), cx))
                .test_support(),
        )
        .child("›");
    let mut heading = row()
        .gap(px(7.))
        .child(chrome::status(session.status))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(14.))
                .child(session.title.clone()),
        );
    let note = match session.status {
        Status::NeedsYou => Some(("Needs you".into(), 0xff9f0a)),
        Status::Exited { .. } => Some(("Agent exited".into(), 0xff453a)),
        Status::Offline => Some((
            format!(
                "{} is offline",
                machine.map_or("Machine", |x| x.name.as_str())
            ),
            0x8e8e93,
        )),
        _ => None,
    };
    if let Some((words, color)) = note {
        heading = heading.child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(color))
                .child(words),
        );
    }
    row()
        .h(px(52.))
        .px(px(16.))
        .gap(px(12.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.content))
        .when(collapsed, |x| {
            x.child(chrome::lights()).child(chrome::action(
                ui,
                "show-sidebar",
                "Show sidebar",
                IconName::PanelLeft,
                AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)),
                look.secondary,
            ))
        })
        .child(col().min_w_0().child(crumbs).child(heading))
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(format!(
                    "{} · {}",
                    if session.agent == Agent::Claude {
                        "Claude Code"
                    } else {
                        session.agent.name()
                    },
                    machine.map_or("", |x| x.name.as_str())
                )),
        )
        .child(chrome::action(
            ui,
            "session-fork",
            "Fork",
            IconName::GitFork,
            AppAction::Session(SessionAction::StartFork(ForkKind::Worktree)),
            look.secondary,
        ))
        .child(chrome::action(
            ui,
            "toggle-side-panel",
            "Toggle side panel",
            IconName::PanelRight,
            AppAction::Pane(PaneAction::Toggle(PaneId::Panel)),
            if state.panel().is_some() {
                look.accent
            } else {
                look.secondary
            },
        ))
        .child(chrome::action(
            ui,
            "session-menu",
            "Session actions",
            IconName::Ellipsis,
            AppAction::OpenPalette,
            look.secondary,
        ))
}
fn session_body(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> Div {
    let active = state.session.view().map_or(0, |x| x.tab);
    let names = std::iter::once(
        match session.agent {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Copilot => "copilot",
            Agent::OpenCode => "opencode",
            Agent::Shell => "zsh",
        }
        .to_owned(),
    )
    .chain(session.shells.clone());
    col()
        .size_full()
        .child(
            row()
                .h(px(32.))
                .flex_shrink_0()
                .bg(rgb(look.panel))
                .border_b_1()
                .border_color(rgb(look.line))
                .children(names.enumerate().map(|(index, name)| {
                    let ui = ui.clone();
                    row()
                        .id(("terminal-tab", index))
                        .role(Role::Tab)
                        .aria_label(name.clone())
                        .aria_selected(active == index)
                        .h_full()
                        .px(px(12.))
                        .gap(px(6.))
                        .border_r_1()
                        .border_color(rgb(look.line))
                        .when(active == index, |x| x.bg(rgb(look.term)))
                        .text_size(px(12.))
                        .text_color(rgb(if active == index {
                            look.text
                        } else {
                            look.secondary
                        }))
                        .child(if index == 0 {
                            chrome::status(session.status).into_any_element()
                        } else {
                            chrome::icon(IconName::SquareTerminal, look.secondary, 12.)
                                .into_any_element()
                        })
                        .child(name)
                        .on_click(move |_, _, cx| {
                            ui.send(AppAction::Session(SessionAction::SelectTab(index)), cx)
                        })
                        .test_support()
                }))
                .child(div().flex_1()),
        )
        .child(terminal_body(session, look))
}
fn terminal_body(session: &Session, look: Look) -> impl IntoElement {
    col()
        .id("terminal")
        .role(Role::Log)
        .aria_label(format!("{} terminal", session.title))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(14.))
        .pt(px(10.))
        .pb(px(10.))
        .gap(px(1.))
        .bg(rgb(look.term))
        .font_family("Menlo")
        .text_size(px(12.))
        .line_height(px(17.))
        .children(session.transcript.iter().map(|line| {
            div()
                .text_color(rgb(match line.from {
                    Speaker::System => look.term_dim,
                    Speaker::Tool => 0x30d158,
                    _ => look.term_text,
                }))
                .child(line.text.clone())
        }))
        .test_support()
}
