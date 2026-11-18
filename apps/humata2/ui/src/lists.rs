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
use humata2_inbox::{Bulk, Group, History, InboxAction, MigrateStatus};
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
                    .child(format!("{} waiting", state.workspace.waiting().count())),
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
            bar.child(
                row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).flex_shrink_0().children(
                    [
                        ("Waiting", History::Waiting, !state.inbox.show_answered),
                        ("Answered", History::Answered, state.inbox.answered_only),
                        (
                            "All",
                            History::All,
                            state.inbox.show_answered && !state.inbox.answered_only,
                        ),
                    ]
                    .into_iter()
                    .map(|(label, history, on)| {
                        let ui = ui.clone();
                        div()
                            .id(SharedString::from(format!("inbox-history-{label}")))
                            .role(Role::Tab)
                            .aria_label(label)
                            .aria_selected(on)
                            .px(px(12.))
                            .py(px(2.))
                            .text_size(px(11.5))
                            .rounded(px(5.))
                            .when(on, |x| x.bg(rgb(look.card)).shadow_sm())
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Inbox(InboxAction::SetHistory(history)), cx)
                            })
                            .child(label)
                            .test_support()
                    }),
                ),
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

/// Allow all n and Deny all over the list (IN-6), or the question Allow all
/// asks first when a waiting request can't be undone.
fn bulk_strip(ui: &Rc<Ui>, confirm: Option<Bulk>, waiting: usize, look: Look) -> Div {
    let bar = row()
        .flex_wrap()
        .gap(px(8.))
        .px(px(14.))
        .py(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line));
    let Some(bulk) = confirm else {
        return bar
            .child(widgets::push_quiet(
                ui,
                "inbox-allow-all",
                format!("Allow all {waiting}"),
                AppAction::Inbox(InboxAction::Bulk(Bulk::ApproveAll)),
                look,
            ))
            .child(widgets::push_quiet(
                ui,
                "inbox-deny-all",
                "Deny all",
                AppAction::Inbox(InboxAction::Bulk(Bulk::DenyAll)),
                look,
            ));
    };
    let (question, yes) = match bulk {
        Bulk::ApproveAll => (
            format!("Allow all {waiting}? Some of them can't be undone once allowed."),
            "Allow all",
        ),
        Bulk::DenyAll => (format!("Deny all {waiting}?"), "Deny all"),
    };
    bar.child(
        div()
            .w_full()
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(question),
    )
    .child(widgets::push_primary(
        ui,
        "inbox-bulk-confirm",
        yes,
        AppAction::Inbox(InboxAction::ConfirmBulk),
        look,
    ))
    .child(widgets::push_quiet(
        ui,
        "inbox-bulk-cancel",
        "Cancel",
        AppAction::Inbox(InboxAction::CancelBulk),
        look,
    ))
}

/// A list row's second line: a Migrate item is from Humata, and says where it
/// stands; any other request names who asks and how it was answered.
fn row_from(state: &AppState, request: &Request, session: Option<&Session>) -> String {
    let Some(card) = request.migrate() else {
        return asked_from(session, request.answer.as_ref(), request.on_screen.is_some());
    };
    let status = match state.inbox.migrate_status(&request.id) {
        Some(MigrateStatus::Migrating) => "migrating…".to_owned(),
        Some(MigrateStatus::Failed(_)) => "couldn't migrate".to_owned(),
        Some(MigrateStatus::Migrated { .. }) => "migrated".to_owned(),
        _ => state
            .workspace
            .machine(&card.machine)
            .map_or_else(|| card.machine.to_string(), |machine| machine.name.clone()),
    };
    format!("Humata · {status}")
}

/// A row's second line: who asks and from which session, then how it was
/// answered once it is (IN-7). `on_screen`: a question on its agent's
/// screen, whose Deny is Dismiss and leaves the question there (D781).
fn asked_from(session: Option<&Session>, answer: Option<&Answer>, on_screen: bool) -> String {
    let mut parts = Vec::new();
    if let Some(session) = session {
        parts.push(
            if session.agent == humata2_clients::model::Agent::Claude {
                "Claude Code"
            } else {
                session.agent.name()
            }
            .to_owned(),
        );
        parts.push(session.title.clone());
    }
    match answer {
        None => {}
        Some(Answer::Approved) => parts.push("approved".to_owned()),
        Some(Answer::Always { .. }) => parts.push("always allowed".to_owned()),
        Some(Answer::Denied { reply: None }) if on_screen => {
            parts.push("dismissed; the question is still on its screen".to_owned())
        }
        Some(Answer::Denied { reply: None }) => parts.push("denied".to_owned()),
        Some(Answer::Denied { reply: Some(note) }) => parts.push(format!("denied: {note}")),
        Some(Answer::Replied { text }) => parts.push(format!("replied: {text}")),
    }
    parts.join(" · ")
}

/// Under a question on its agent's screen: the live terminal at full width,
/// then the hint and Dismiss (D781). The terminal is the one the Inbox links
/// for the open item; until it is made, the hint shows alone. The hint sends
/// you to the terminal only while it shows the screen.
fn screen_question(ui: &Rc<Ui>, state: &AppState, request: &Request, look: Look) -> AnyElement {
    let pending = state.inbox.pending.contains(&request.id);
    let hint = if ui.inbox_room().screen() {
        "Answer in the terminal or pick a choice above. Dismiss closes this item and presses nothing."
    } else {
        "Pick a choice above. Dismiss closes this item and presses nothing."
    };
    col()
        .w_full()
        .gap(px(DETAIL_GAP))
        .children(ui.inbox_terminal())
        .child(
            row()
                .id("inbox-screen-hint")
                .role(Role::Group)
                .aria_label(hint)
                .test_support()
                .relative()
                .gap(px(12.))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(hint),
                )
                .child(
                    request_card::button(
                        ui,
                        SharedString::from(format!("dismiss {}", request.id)),
                        if pending { "Dismissing…" } else { "Dismiss" },
                        vec![AppAction::Inbox(InboxAction::Dismiss(request.id.clone()))],
                        look,
                        false,
                    )
                    .disabled(pending),
                )
                .child(InboxRoom::measure(ui, Measured::Hint)),
        )
        .into_any_element()
}

pub(crate) fn inbox(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let shown = state.inbox.shown();
    let selected = state
        .inbox
        .selection()
        .filter(|r| shown.iter().any(|v| v.id == r.id))
        .or_else(|| shown.first().copied());
    // Allow all and Deny all never answer a Migrate item.
    let waiting = state.inbox.bulk_count();
    let strip = (waiting > 1 || state.inbox.confirm.is_some())
        .then(|| bulk_strip(ui, state.inbox.confirm, waiting, look));
    let list = col()
        .id("global-inbox-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .children(state.inbox.groups().into_iter().map(|(group, requests)| {
            let heading = match &group {
                Group::Humata => "Humata".to_owned(),
                Group::Channel(id) if *id == humata2_clients::desk::channel() => "Desk".to_owned(),
                Group::Channel(id) => state
                    .workspace
                    .channel(id)
                    .map_or_else(|| format!("# {id}"), |channel| format!("# {}", channel.name)),
            };
            {
                col()
                    .child(
                        div()
                            .px(px(14.))
                            .pt(px(10.))
                            .pb(px(4.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(11.))
                            .text_color(rgb(look.secondary))
                            .child(heading),
                    )
                    .children(requests.into_iter().map(|request| {
                        let session = state.workspace.session(&request.session);
                        let on = selected.is_some_and(|r| r.id == request.id);
                        let ui = ui.clone();
                        let id = request.id.clone();
                        col()
                            .id(SharedString::from(format!("global-request {}", id)))
                            .role(Role::ListBoxOption)
                            .aria_label(request.title.clone())
                            .aria_selected(on)
                            .px(px(14.))
                            .py(px(9.))
                            .gap(px(2.))
                            .border_b_1()
                            .border_color(rgb(look.line))
                            .when(on, |x| x.bg(rgb(look.selection)))
                            .text_color(rgb(if on { 0xffffff } else { look.text }))
                            .opacity(if request.answer.is_some() { 0.55 } else { 1. })
                            .child(
                                row()
                                    .gap(px(6.))
                                    // The VIP mark asks for attention: answered rows don't carry it.
                                    .when(state.inbox.vip(request), |x| {
                                        x.child(request_card::vip_badge())
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(request.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(11.))
                                            .text_color(rgb(if on {
                                                0xffffff
                                            } else {
                                                look.secondary
                                            }))
                                            .child(request.asked.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(if on { 0xffffff } else { look.secondary }))
                                    .child(row_from(state, request, session)),
                            )
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Inbox(InboxAction::Select(id.clone())), cx)
                            })
                            .test_support()
                    }))
            }
        }));
    row()
        .size_full()
        .items_stretch()
        .text_size(px(12.5))
        .child(
            col()
                .w(px(330.))
                .max_w(relative(0.48))
                .flex_shrink_0()
                .h_full()
                .border_r_1()
                .border_color(rgb(look.line))
                .children(strip)
                .child(list),
        )
        .child(
            col()
                .id("global-inbox-detail")
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .p(px(DETAIL_PAD))
                .gap(px(DETAIL_GAP))
                .child(InboxRoom::measure(ui, Measured::Pane))
                .children(selected.map(|r| {
                    div()
                        .relative()
                        .w_full()
                        .max_w(px(560.))
                        .child(request_card::card(ui, state, r, true, look))
                        .child(InboxRoom::measure(ui, Measured::Card))
                }))
                .children(
                    selected
                        .filter(|r| r.on_screen.is_some() && r.answer.is_none())
                        .map(|r| screen_question(ui, state, r, look)),
                )
                .when(selected.is_none(), |x| {
                    x.child(widgets::empty_in(
                        look,
                        "Nothing waiting.",
                        "New requests appear here.",
                    ))
                }),
        )
}
