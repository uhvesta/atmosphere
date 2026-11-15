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
        Route::Settings => "Settings".to_owned(),
    }
}

/// The screen for the current route.
pub fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    channel_filter: Entity<InputState>,
    board_inputs: &crate::board::Inputs,
    host_address: Entity<InputState>,
    terminals: &crate::terminals::Terminals,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match &state.route {
        Route::Home if state.workspace.channels.is_empty() => {
            first_run(ui, Look::of(cx)).into_any_element()
        }
        Route::Home => home(ui, state, Look::of(cx)).into_any_element(),
        Route::Settings => crate::settings::page(ui, state, Look::of(cx)).into_any_element(),
        Route::Hosts => crate::hosts::page(
            ui,
            state,
            f32::from(window.viewport_size().width) - state.layout.sidebar.width,
            host_address,
            Look::of(cx),
        )
        .into_any_element(),
        Route::Board(_) => {
            crate::board::page(ui, state, board_inputs, Look::of(cx)).into_any_element()
        }
        Route::Inbox => crate::lists::inbox(ui, state, Look::of(cx)).into_any_element(),
        Route::Channel(id) => state.workspace.channel(id).map_or_else(
            || {
                widgets::empty("This channel is gone.", "Pick another in the sidebar.")
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
                    widgets::empty("This session is gone.", "Pick another in the sidebar.")
                        .into_any_element()
                },
                |session| {
                    session_body(ui, state, session, Look::of(cx), terminals, cx).into_any_element()
                },
            ),
        _ => widgets::empty("Nothing here yet.", "This screen is being built.").into_any_element(),
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
                        if session.agent == Agent::Claude {
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
                widgets::primary(
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
    cx: &mut App,
) -> Div {
    crate::session_tabs::body(ui, state, session, look, terminals, cx)
}

fn first_run(ui: &Rc<Ui>, look: Look) -> Div {
    col().size_full().items_center().justify_center().p(px(20.)).text_size(px(13.)).text_color(rgb(look.text)).child(
        col().w(px(460.)).max_w_full().items_center().gap(px(12.))
            .child(chrome::icon(IconName::SquareTerminal,look.accent,40.))
            .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child("Start with a channel"))
            .child(div().text_color(rgb(look.secondary)).text_center().child("A channel is a project: its repos, the machines that work on it, its board and its sessions."))
            .child(row().flex_wrap().justify_center().gap(px(10.)).pt(px(6.)).child(widgets::primary(ui,"first-channel-create","Create your first channel",AppAction::StartNewChannel).small().h(px(24.)).bg(rgb(look.accent)).text_color(rgb(0xffffff))).child(widgets::send(ui,"first-machine-add","Add a machine over SSH",AppAction::Hosts(humata2_hosts::HostsAction::StartAdd)).small().h(px(24.)).bg(rgb(look.fill)).text_color(rgb(look.text))))
            .child(div().pt(px(8.)).text_size(px(11.5)).text_color(rgb(look.secondary)).text_center().child("GitHub access comes from the gh CLI. There is no sign-in in Humata."))
    )
}
