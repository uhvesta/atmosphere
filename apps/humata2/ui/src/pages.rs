//! The screens in the middle of the window, one per route.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_app::{
    PaneAction,
    layout::{FullScreen, PaneId},
};
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
                .id("terminal-tabs")
                .overflow_x_scroll()
                .h(px(32.))
                .flex_shrink_0()
                .bg(rgb(look.panel))
                .border_b_1()
                .border_color(rgb(look.line))
                .children(names.enumerate().map(|(index, name)| {
                    let ui = ui.clone();
                    row()
                        .id(SharedString::from(format!("terminal-tab {index}")))
                        .role(Role::Tab)
                        .aria_label(name.clone())
                        .aria_selected(active == index)
                        .h_full()
                        .flex_shrink_0()
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
                .child(div().flex_1())
                .when(session.status == Status::Offline, |bar| {
                    bar.child(
                        row()
                            .mr(px(12.))
                            .gap(px(5.))
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(chrome::icon(IconName::WifiOff, look.secondary, 12.))
                            .child("disconnected"),
                    )
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
        .child(terminal_body(ui, state, session, look))
}
fn terminal_body(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> impl IntoElement {
    let tab = state.session.view().map_or(0, |view| view.tab);
    let base = if tab == 0 {
        session.transcript.as_slice()
    } else {
        &[]
    };
    let live = state
        .session
        .live
        .get(&(session.id.clone(), tab))
        .map_or(&[][..], |lines| lines.as_slice());
    col()
        .id("terminal")
        .role(Role::Log)
        .aria_label(format!("{} terminal", session.title))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .overflow_x_hidden()
        .px(px(14.))
        .pt(px(10.))
        .pb(px(10.))
        .gap(px(1.))
        .bg(rgb(look.term))
        .font_family("Menlo")
        .text_size(px(12.))
        .line_height(px(17.))
        .children(base.iter().chain(live).map(|line| {
            div()
                .text_color(rgb(match line.from {
                    Speaker::System => look.term_dim,
                    Speaker::Tool => 0x30d158,
                    _ => look.term_text,
                }))
                .child(line.text.clone()).test_text(line.text.clone())
        }))
        .children(match (tab,session.status) {
            (0,Status::Exited{code})=>Some(row().mt(px(10.)).gap(px(8.)).p(px(8.)).rounded(px(7.)).bg(rgb(look.fill)).font_family(".SystemUIFont").text_size(px(12.))
                .child(chrome::icon(IconName::X,0xff453a,12.))
                .child(div().flex_1().min_w_0().child(format!("Agent exited with code {code}. Resume starts it again with this conversation.")))
                .child(widgets::send(ui,"resume-terminal","Resume",AppAction::Session(SessionAction::Resume)).small())),
            (_,Status::Offline)=>Some(row().mt(px(10.)).gap(px(8.)).p(px(8.)).rounded(px(7.)).bg(rgb(look.fill)).font_family(".SystemUIFont").text_size(px(12.))
                .child(chrome::icon(IconName::WifiOff,look.secondary,12.))
                .child(div().min_w_0().child(format!("Lost the connection to {}. The agent keeps running there; this view catches up when it is back.",session.machine)))),
            _=>None,
        })
        .child(div().flex_1())
        .when(tab>0 || !matches!(session.status,Status::Exited{..}|Status::NeedsYou),|body|body.child(
            col().gap(px(2.)).opacity(if session.status==Status::Offline{0.5}else{1.})
                .child(row().px(px(8.)).py(px(4.)).border_1().border_color(rgb(look.term_dim)).rounded(px(4.)).gap(px(2.))
                    .child(if tab==0&&session.agent!=Agent::Shell {"> "}else{"$ "}).when(session.status!=Status::Offline,|x|x.child(div().w(px(7.)).h(px(15.)).bg(rgb(look.term_text)))))
                .when(tab==0&&session.agent!=Agent::Shell,|body|body.child(div().text_color(rgb(look.term_dim)).child("  ⏵⏵ accept edits on (shift+tab to cycle)")))
        ))
        .test_support()
}
