//! Home's grouped lists and the wide Inbox, over the existing feature state.
use crate::{
    bridge::Ui,
    chrome,
    inbox_terminal::{DETAIL_GAP, DETAIL_PAD, InboxRoom, Measured},
    request_card,
    theme::Look,
    widgets::{self, col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, NeedsRow, Route};
use humata2_clients::model::{
    Answer, MachineKind, Request, RequestKind, Session, SessionId, Status,
};
use humata2_inbox::{Bulk, Group, InboxAction, MigrateStatus};
use humata2_session::{Panel, SessionAction};
use std::rc::Rc;

pub(crate) fn toolbar(
    ui: &Rc<Ui>,
    state: &AppState,
    title: String,
    hidden: bool,
    look: Look,
) -> Div {
    row()
        .h(px(52.))
        .px(px(20.))
        .gap(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .when(hidden, |bar| bar.child(chrome::lights()))
        .when(!state.trail.is_empty(), |bar| {
            bar.child(widgets::ghost_in(look, ui, "back", "‹ Back", AppAction::Back))
        })
        .child(
            div()
                .id("screen-title")
                .role(Role::Heading)
                .aria_label(title.clone())
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title)
                .test_support(),
        )
        .when(state.route == Route::Inbox, |bar| {
            bar.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!("{} waiting on you", state.workspace.waiting().count())),
            )
        })
        .child(div().flex_1())
        .when(
            state.route == Route::Home && !state.workspace.channels.is_empty(),
            |bar| {
                bar.child(widgets::push_primary(
                    ui,
                    "home-new-session",
                    "New session",
                    AppAction::StartNewSession,
                    look,
                ))
            },
        )
        // With no channel yet a session can only start on the Desk (frame 34).
        .when(
            state.route == Route::Home && state.workspace.channels.is_empty(),
            |bar| {
                bar.child(widgets::push_primary(
                    ui,
                    "home-new-desk-session",
                    "New desk session",
                    AppAction::StartNewDeskSession,
                    look,
                ))
            },
        )
        .when(state.route == Route::Inbox, |bar| {
            let history = state.inbox.show_answered;
            let ui = ui.clone();
            bar.child(
                div()
                    .id("inbox-history")
                    .role(Role::Button)
                    .aria_label(if history { "Back to waiting" } else { "History" })
                    .text_size(px(12.5))
                    .text_color(rgb(look.accent))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| ui.send(AppAction::Inbox(InboxAction::ToggleHistory), cx))
                    .child(if history { "Back to waiting" } else { "History" })
                    .test_support(),
            )
        })
}

/// What a Home row says its agent waits on (frame 25): "Asks to move PAY-3
/// to Done" for an action, the question itself for a question.
fn asks(request: &Request) -> String {
    if request.kind == RequestKind::Question {
        return request.title.clone();
    }
    let mut title = request.title.chars();
    match title.next() {
        Some(first) => format!("Asks to {}{}", first.to_lowercase(), title.as_str()),
        None => String::new(),
    }
}

/// One session on Home. With the `request` its agent waits on, the row says
/// what it asks and its primary button reviews the request.
fn home_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    request: Option<&Request>,
    group: &'static str,
    index: usize,
    look: Look,
) -> impl IntoElement {
    let ui2 = ui.clone();
    let id = session.id.clone();
    let asked = request.map(|request| request.id.clone());
    let detail = request.map_or_else(|| session.activity.clone(), asks);
    let place = humata2_clients::desk::place(&state.workspace, &session.channel);
    let description = if let Some(parent) = session
        .parent
        .as_ref()
        .and_then(|id| state.workspace.session(id))
    {
        format!("fork of {}", parent.title)
    } else {
        // HM-1: a row names its machine when that isn't This Mac.
        match state.workspace.machine(&session.machine) {
            Some(machine) if !matches!(machine.kind, MachineKind::ThisMac) => {
                format!("{place} · {}", machine.name)
            }
            _ => place,
        }
    };
    let pull = state
        .workspace
        .pulls
        .iter()
        .find(|p| p.session.as_ref() == Some(&session.id));
    let open_url = if group == "home-recent" && !session.title.contains('#') {
        pull.map(|p| format!("https://github.com/{}/pull/{}", p.repo, p.number))
    } else {
        None
    };
    let resumable = crate::session_tabs::offers_resume(session);
    let need = asked.is_some() || session.status == Status::NeedsYou || resumable;
    let label = if asked.is_some() {
        Some("Review")
    } else if resumable {
        Some("Resume")
    } else if session.status == Status::NeedsYou {
        Some("Review")
    } else if open_url.is_some() {
        Some("Open PR")
    } else if group == "home-recent" {
        Some("Review")
    } else {
        None
    };
    row()
        .id((group, index))
        .role(Role::Link)
        .aria_label(session.title.clone())
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(rgb(look.line))
        .cursor_pointer()
        .child(chrome::status(session.status))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(session.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(description),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(
                            if session.status == Status::NeedsYou || asked.is_some() {
                                0xff9f0a
                            } else {
                                look.secondary
                            },
                        ))
                        .child(detail.clone())
                        .test_text(detail),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(session.active.clone()),
        )
        .children(label.map(|label| {
            let ui = ui.clone();
            let url = open_url.clone();
            let session = session.id.clone();
            let exited = asked.is_none() && resumable;
            let button_id = SharedString::from(format!("{group}-action-{index}"));
            let button = if need && !exited {
                widgets::push_primary_shape(button_id, label, look)
            } else {
                widgets::push_quiet_shape(button_id, label, look)
            };
            let asked = asked.clone();
            button.on_click(move |_, _, cx| {
                cx.stop_propagation();
                if let Some(url) = &url {
                    cx.open_url(url);
                    return;
                }
                if let Some(request) = &asked {
                    ui.send(AppAction::Inbox(InboxAction::OpenRequest(request.clone())), cx);
                    return;
                }
                ui.send(AppAction::Navigate(Route::Session(session.clone())), cx);
                if exited {
                    ui.send(AppAction::Session(SessionAction::Resume), cx);
                } else if need {
                    ui.send(
                        AppAction::Session(SessionAction::SetPanel(Some(Panel::Inbox))),
                        cx,
                    );
                }
            })
        }))
        .on_click(move |_, _, cx| ui2.send(AppAction::Navigate(Route::Session(id.clone())), cx))
        .test_support()
}
fn card(look: Look) -> Div {
    col()
        .w_full()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}

/// A titled group on Home: its small heading over `body`.
fn group(look: Look, title: String, body: Div) -> Div {
    col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(title),
        )
        .child(body)
}

/// A group of session rows, or `none` in the card when there is no session.
fn session_group(
    ui: &Rc<Ui>,
    state: &AppState,
    title: String,
    id: &'static str,
    sessions: &[&Session],
    none: &'static str,
    look: Look,
) -> Div {
    let body = if sessions.is_empty() {
        card(look)
            .px(px(14.))
            .py(px(12.))
            .text_color(rgb(look.tertiary))
            .child(none)
    } else {
        card(look).overflow_hidden().children(
            sessions
                .iter()
                .enumerate()
                .map(|(index, session)| home_row(ui, state, session, None, id, index, look)),
        )
    };
    group(look, title, body)
}

/// Home's "Needs you" group: a session row for each session that needs the
/// person (HM-1), and a row for each pending request with no session on Home.
fn needs_group(
    ui: &Rc<Ui>,
    state: &AppState,
    title: String,
    rows: &[NeedsRow<'_>],
    look: Look,
) -> Div {
    let body = if rows.is_empty() {
        card(look)
            .px(px(14.))
            .py(px(12.))
            .text_color(rgb(look.tertiary))
            .child("Nothing needs you.")
    } else {
        card(look).overflow_hidden().children(rows.iter().enumerate().map(|(index, row)| {
            match row {
                NeedsRow::Session { session, request } => {
                    home_row(ui, state, session, *request, "home-needs", index, look)
                        .into_any_element()
                }
                NeedsRow::Request(request) => {
                    request_row(ui, request, index, look).into_any_element()
                }
            }
        }))
    };
    group(look, title, body)
}

/// A pending request with no session listed: its title, why it is asked, and
/// when. The row opens the request.
fn request_row(ui: &Rc<Ui>, request: &Request, index: usize, look: Look) -> impl IntoElement {
    let ui2 = ui.clone();
    let id = request.id.clone();
    row()
        .id(("home-needs-request", index))
        .role(Role::Link)
        .aria_label(request.title.clone())
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(rgb(look.line))
        .cursor_pointer()
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(request.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(0xff9f0a))
                        .child(request.reason.clone()),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(request.asked.clone()),
        )
        .on_click(move |_, _, cx| {
            ui2.send(AppAction::Inbox(InboxAction::OpenRequest(id.clone())), cx)
        })
        .test_support()
}

/// One row of Get set up: what to do, why, and the button that starts it.
fn step(
    look: Look,
    glyph: IconName,
    title: &'static str,
    note: &'static str,
    action: Button,
) -> Div {
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
        .child(action)
}

/// What Home offers when there is no channel yet (DK-6, frame 34).
fn get_set_up(ui: &Rc<Ui>, look: Look) -> Div {
    group(
        look,
        "Get set up".to_owned(),
        card(look)
            .overflow_hidden()
            .child(step(
                look,
                IconName::Plus,
                "Make a channel",
                "A project: its repos, the machines that work on it, its board and its sessions.",
                widgets::push_primary(
                    ui,
                    "first-channel-create",
                    "New channel…",
                    AppAction::StartNewChannel,
                    look,
                ),
            ))
            .child(step(
                look,
                IconName::Server,
                "Add a machine",
                "Run sessions on another computer over SSH. This Mac works already.",
                widgets::push_quiet(
                    ui,
                    "first-machine-add",
                    "Add a machine…",
                    AppAction::Hosts(humata2_hosts::HostsAction::StartAdd),
                    look,
                ),
            ))
            .child(step(
                look,
                IconName::SquareTerminal,
                "Use the Desk",
                "An agent or a shell in a scratch folder, no channel needed: plan, tidy the board, write.",
                widgets::push_quiet(
                    ui,
                    "first-desk-open",
                    "Open Desk",
                    AppAction::Switch(Route::Desk),
                    look,
                ),
            )),
    )
}

fn counted(name: &str, count: usize) -> String {
    if count == 0 {
        name.to_owned()
    } else {
        format!("{name} · {count}")
    }
}

pub(crate) fn home(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let needs = humata2_app::needs_you(&state.workspace);
    // A session in "Needs you" is not repeated under Working or Recent.
    let listed: Vec<&SessionId> = needs
        .rows
        .iter()
        .filter_map(|row| match row {
            NeedsRow::Session { session, .. } => Some(&session.id),
            NeedsRow::Request(_) => None,
        })
        .collect();
    let all = state
        .workspace
        .sessions
        .iter()
        .filter(|s| !s.archived && !listed.contains(&&s.id));
    let working = all
        .clone()
        .filter(|s| s.status == Status::Working)
        .collect::<Vec<_>>();
    let recent = all
        .filter(|s| {
            matches!(s.status, Status::Idle | Status::ProbablyIdle) && !s.activity.is_empty()
        })
        .collect::<Vec<_>>();
    let groups = col()
        .w_full()
        .max_w(px(720.))
        .gap(px(20.))
        // After a restart (frame 47): what couldn't be resumed and what came back idle.
        .children(crate::restart_report::home_groups(ui, state, look))
        .child(needs_group(ui, state, counted("Needs you", needs.len()), &needs.rows, look))
        .child(session_group(
            ui,
            state,
            counted("Working", working.len()),
            "home-working",
            &working,
            "No sessions are running.",
            look,
        ));
    let groups = if state.workspace.channels.is_empty() {
        groups.child(get_set_up(ui, look)).child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child("GitHub access comes from the gh CLI. There is no sign-in in Humata."),
        )
    } else {
        groups.child(session_group(
            ui,
            state,
            "Recent".to_owned(),
            "home-recent",
            &recent,
            "Nothing recent yet.",
            look,
        ))
    };
    col()
        .id("home")
        .size_full()
        .overflow_y_scroll()
        .p(px(24.))
        .items_center()
        .text_size(px(13.))
        .child(groups)
}

pub(crate) use crate::inbox_page::inbox;
