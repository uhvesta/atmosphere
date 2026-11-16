//! The screens in the middle of the window, one per route.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable as _, input::InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_app::{
    PaneAction,
    layout::{FullScreen, PaneId},
};
use humata2_clients::model::{Agent, ForkKind, Session, Status};
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
        Route::Desk => "Desk".to_owned(),
        Route::Settings => "Settings".to_owned(),
        Route::Machine(id) => state
            .workspace
            .machine(id)
            .map_or_else(|| "Machine".to_owned(), |machine| machine.name.clone()),
    }
}

/// The screen for the current route.
pub fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    channel_filter: Entity<InputState>,
    board_inputs: &crate::board::Inputs,
    files_name: Entity<InputState>,
    terminals: &crate::terminals::Terminals,
    documents: &crate::document_tab::Documents,
    machine_inputs: &crate::machine_page::MachinePageInputs,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let look = Look::of(cx);
    match &state.route {
        Route::Home => home(ui, state, Look::of(cx)).into_any_element(),
        Route::Desk => crate::desk::page(ui, state, Look::of(cx)).into_any_element(),
        Route::Files => crate::file_manager::page(ui, &state.files, &files_name, Look::of(cx)),
        Route::Settings => crate::settings::page(ui, state, Look::of(cx)).into_any_element(),
        Route::Machine(id) => {
            crate::machine_page::route(ui, state, id, machine_inputs, Look::of(cx), window, cx)
        }
        Route::Hosts => crate::hosts::page(
            ui,
            state,
            f32::from(window.viewport_size().width) - state.layout.sidebar.width,
            Look::of(cx),
        )
        .into_any_element(),
        Route::Board(_) => {
            crate::board::page(ui, state, board_inputs, Look::of(cx)).into_any_element()
        }
        Route::Inbox => crate::lists::inbox(ui, state, Look::of(cx)).into_any_element(),
        Route::Channel(id) => state.workspace.channel(id).map_or_else(
            || {
                widgets::empty_in(look, "This channel is gone.", "Pick another in the sidebar.")
                    .into_any_element()
            },
            |channel| {
                crate::channel::page(
                    ui,
                    state,
                    channel,
                    channel_filter,
                    board_inputs,
                    Look::of(cx),
                )
            },
        ),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(
                || {
                    widgets::empty_in(look, "This session is gone.", "Pick another in the sidebar.")
                        .into_any_element()
                },
                |session| {
                    session_body(ui, state, session, Look::of(cx), terminals, documents, cx).into_any_element()
                },
            ),
        _ => widgets::empty_in(look, "Nothing here yet.", "This screen is being built.")
            .into_any_element(),
    }
}

pub(crate) fn home(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    crate::lists::home(ui, state, look)
}

pub(crate) fn session_header(
    ui: &Rc<Ui>,
    state: &AppState,
    collapsed: bool,
    width: f32,
    look: Look,
) -> Div {
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
    let terminal_full = state.full_screen() == Some(FullScreen::Terminal);
    let compact = width < 560.;
    let mut crumbs = row()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary));
    let desk = state.is_desk_session(&session.id);
    let event = AppAction::Switch(if desk {
        Route::Desk
    } else {
        Route::Channel(session.channel.clone())
    });
    let ui2 = ui.clone();
    crumbs = crumbs
        .child(
            div()
                .id("session-channel")
                .role(Role::Link)
                .aria_label(if desk { "Open Desk" } else { "Open channel" })
                .text_color(rgb(look.accent))
                .child(if desk {
                    "Desk".to_owned()
                } else {
                    format!("# {}", channel.map_or("", |x| x.name.as_str()))
                })
                .on_click(move |_, _, cx| ui2.send(event.clone(), cx))
                .test_support(),
        )
        .child("›");
    let mut heading = row()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .gap(px(7.))
        .child(chrome::status(session.status))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
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
    if let Some((words, color)) = note.filter(|_| !compact) {
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
        .gap(px(if compact { 8. } else { 12. }))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.content))
        .when(collapsed || terminal_full, |x| {
            x.child(chrome::lights()).child(div().w(px(4.)))
        })
        .child(
            col()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(crumbs)
                .child(heading),
        )
        .when(!compact, |bar| {
            bar.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!(
                        "{} · {}",
                        if desk {
                            crate::desk::runs(session.agent)
                        } else if session.agent == Agent::Claude {
                            "Claude Code"
                        } else {
                            session.agent.name()
                        },
                        machine.map_or("", |x| x.name.as_str())
                    )),
            )
        })
        .when(matches!(session.status, Status::Exited { .. }), |bar| {
            bar.child(
                widgets::primary_in(look,
                    ui,
                    "resume-agent",
                    "Resume agent",
                    AppAction::Session(SessionAction::Resume),
                )
                .small()
                .h(px(24.))
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff)),
            )
        })
        .when(!desk, |bar| {
            bar.child(chrome::action(
                ui,
                "session-fork",
                "Fork",
                IconName::GitFork,
                AppAction::Session(SessionAction::StartFork(ForkKind::Worktree)),
                look.secondary,
            ))
        })
        .child(chrome::action(
            ui,
            "terminal-full-screen",
            if terminal_full {
                "Return to columns"
            } else {
                "Expand terminal"
            },
            if terminal_full {
                IconName::Minimize2
            } else {
                IconName::Maximize2
            },
            AppAction::ToggleFullScreen(FullScreen::Terminal),
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
fn session_body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
    terminals: &crate::terminals::Terminals,
    documents: &crate::document_tab::Documents,
    cx: &mut App,
) -> Div {
    crate::session_tabs::body(ui, state, session, look, terminals, documents, cx)
}
