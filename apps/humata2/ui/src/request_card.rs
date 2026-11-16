//! The decision card (IN-9): what is asked, why, what it would do, the rule
//! behind it, and the answers.
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route, channel};
use humata2_clients::model::{
    Agent, Answer, LineKind, Preview, Request, RequestKind, Status,
};
use humata2_inbox::InboxAction;
use std::rc::Rc;

use crate::theme::{GREEN, GREY, ORANGE, RED_STATUS as RED};

fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

fn secondary(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}

fn dot(status: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match status {
        Status::Working => dot.bg(rgb(GREEN)),
        Status::NeedsYou => dot.bg(rgb(ORANGE)),
        Status::Exited { .. } => dot.bg(rgb(RED)),
        Status::Offline => dot.bg(rgb(GREY)),
        _ => dot.border_1().border_color(rgb(GREY)),
    };
    row()
        .size(px(10.))
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}

/// A button that sends `events` in order when clicked.
fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    events: Vec<AppAction>,
    look: Look,
    primary: bool,
) -> Button {
    let ui = ui.clone();
    let label = label.into();
    Button::new(id)
        .label(label.clone())
        .accessibility_label(label)
        .small()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .when(primary, |button| button.primary())
        .bg(rgb(if primary { look.accent } else { look.fill }))
        .text_color(rgb(if primary { 0xffffff } else { look.text }))
        .when(!primary, |button| {
            button.border_1().border_color(rgb(look.line))
        })
        .on_click(move |_, _, cx| {
            for event in &events {
                ui.send(event.clone(), cx);
            }
        })
}

fn agent_name(agent: Agent) -> &'static str {
    if agent == Agent::Claude {
        "Claude Code"
    } else {
        agent.name()
    }
}

fn labelled(look: Look, label: &'static str, value: impl Into<SharedString>, mono: bool) -> Div {
    row()
        .gap(px(8.))
        .items_start()
        .child(secondary(look, label).w(px(64.)).flex_shrink_0())
        .child(
            text(value)
                .flex_1()
                .min_w_0()
                .text_size(px(11.5))
                .when(mono, |value| value.font_family("Menlo")),
        )
}

/// The IN-10 preview as labelled rows.
fn preview(request: &Request, look: Look) -> Div {
    let body = match &request.preview {
        Preview::Command(command) => col().child(labelled(look, "Command", command.clone(), true)),
        Preview::Text(value) => {
            let label = match request.kind {
                RequestKind::Network => "Reach",
                RequestKind::Question => "Asks",
                RequestKind::Merge => "Merge",
                RequestKind::Destroy => "Affects",
                _ => "Detail",
            };
            col().child(labelled(look, label, value.clone(), false))
        }
        Preview::Diff(lines) => col().gap(px(1.)).children(lines.iter().map(|line| {
            let color = match line.kind {
                LineKind::Added => GREEN,
                LineKind::Removed => RED,
                LineKind::Hunk => look.secondary,
                LineKind::Context => look.text,
            };
            text(line.text.clone())
                .text_size(px(11.5))
                .font_family("Menlo")
                .text_color(rgb(color))
        })),
    };
    body.p(px(8.)).rounded(px(6.)).bg(rgb(look.fill))
}

fn tick(ui: &Rc<Ui>, request: &Request, on: bool, look: Look) -> impl IntoElement {
    let ui = ui.clone();
    let id = request.id.clone();
    let label = "Allow this for the rest of the session";
    row()
        .id(SharedString::from(format!("always {}", request.id)))
        .role(Role::CheckBox)
        .aria_label(label)
        .aria_selected(on)
        .test_support()
        .gap(px(8.))
        .cursor_pointer()
        .child(
            row()
                .size(px(14.))
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(rgb(look.line))
                .when(on, |b| b.bg(rgb(look.accent)).text_color(rgb(0xffffff)))
                .text_size(px(10.))
                .child(if on { "✓" } else { "" }),
        )
        .child(text(label))
        .on_click(move |_, _, cx| {
            ui.send(AppAction::Inbox(InboxAction::ToggleAlways(id.clone())), cx);
        })
}

/// Who asks: the agent and its session, then with `tool` what it called (the
/// daemon's operation id, or else the rule's tool name, as the design draws).
pub(crate) fn requester(state: &AppState, request: &Request, tool: bool) -> String {
    let Some(s) = state.workspace.session(&request.session) else {
        return String::new();
    };
    let runs = if humata2_clients::desk::is_desk(s) {
        crate::desk::runs(s.agent)
    } else {
        agent_name(s.agent)
    };
    let mut line = format!("{runs} in {}", s.title);
    let called = if request.operation.is_empty() { &request.rule } else { &request.operation };
    if tool && !called.is_empty() {
        line.push_str(" · ");
        line.push_str(called);
    }
    line
}

/// One request as the decision card. `wide` is the Inbox's detail pane; the
/// session panel leaves out the machine, channel and Open session.
pub(crate) fn card(
    ui: &Rc<Ui>,
    state: &AppState,
    request: &Request,
    wide: bool,
    look: Look,
) -> impl IntoElement {
    let session = state.workspace.session(&request.session);
    let pending = state.inbox.pending.contains(&request.id);
    let waiting = request.answer.is_none();
    let ticked = state.inbox.always.contains(&request.id);
    let channel_name = state
        .workspace
        .channel(&request.channel)
        .map(|c| c.name.clone())
        .or_else(|| (request.channel == humata2_clients::desk::channel()).then(|| "Desk".to_owned()));
    let machine_name = session
        .and_then(|s| state.workspace.machine(&s.machine))
        .map(|m| m.name.clone());

    // 1. Title, the requester's status, the age.
    let top = row()
        .gap(px(6.))
        .child(dot(session.map_or(Status::NeedsYou, |s| s.status)))
        .child(
            text(request.title.clone())
                .font_weight(FontWeight::SEMIBOLD)
                .flex_1(),
        )
        .child(secondary(look, request.asked.clone()));

    // 2. Who asks; in the detail pane, where it runs and a link to its session.
    let by = requester(state, request, true);
    let place = [machine_name, channel_name.map(|name| format!("# {name}"))]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let who = row()
        .gap(px(8.))
        .child(secondary(look, by).flex_1().min_w_0());
    let open_ui = ui.clone();
    let open_id = request.id.clone();
    let meta = wide.then(|| {
        row()
            .gap(px(8.))
            .child(secondary(look, place).flex_1().min_w_0())
            .child(
                row()
                    .id(SharedString::from(format!("open session {}", request.id)))
                    .role(Role::Button)
                    .aria_label("Open session ↗")
                    .test_support()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(look.accent))
                    .child(text("Open session ↗"))
                    .on_click(move |_, _, cx| {
                        open_ui.send(
                            AppAction::Inbox(InboxAction::OpenRequest(open_id.clone())),
                            cx,
                        );
                    }),
            )
    });

    // 5. What the agent was doing: its last steps, the pending call last.
    let steps = (wide && !request.steps.is_empty()).then(|| {
        let count = request.steps.len();
        col()
            .id(SharedString::from(format!("steps {}", request.id)))
            .test_support()
            .gap(px(4.))
            .child(secondary(look, "What it was doing").font_weight(FontWeight::SEMIBOLD))
            .children(request.steps.iter().enumerate().map(|(index, step)| {
                let color = if waiting && index + 1 == count {
                    ORANGE
                } else {
                    GREEN
                };
                row()
                    .gap(px(6.))
                    .child(div().size(px(6.)).rounded_full().flex_shrink_0().bg(rgb(color)))
                    .child(
                        text(step.clone())
                            .text_size(px(11.5))
                            .font_family("Menlo"),
                    )
            }))
    });

    // 6. Why it asks you, with the rule's change link when a rule sent it.
    let why = if request.why_you.is_empty() {
        request
            .asked_by
            .as_ref()
            .map_or_else(String::new, |asked| asked.label.clone())
    } else {
        request.why_you.clone()
    };
    let change_rule = request.asked_by.as_ref().map(|_| {
        button(
            ui,
            SharedString::from(format!("change rule {}", request.id)),
            "Change this rule",
            vec![
                AppAction::Navigate(Route::Channel(request.channel.clone())),
                AppAction::Channel(channel::Action::Tab(channel::Tab::Access)),
            ],
            look,
            false,
        )
    });
    let why_line = (!why.is_empty()).then(|| {
        row()
            .gap(px(8.))
            .items_start()
            .child(secondary(look, "Asks you because:").flex_shrink_0())
            .child(text(why).text_size(px(11.5)).flex_1().min_w_0())
            .children(change_rule)
    });

    // 8. The answers.
    let is_question = request.kind == RequestKind::Question;
    let id = request.id.clone();
    let reply = state
        .inbox
        .reply
        .as_ref()
        .filter(|(reply_id, _)| *reply_id == request.id)
        .map(|(_, draft)| draft.clone());
    let answers = if !waiting {
        col().child(secondary(look, "Answered"))
    } else if pending {
        col().child(secondary(look, "Answering…"))
    } else if let Some(draft) = reply {
        col()
            .gap(px(6.))
            .child(
                text(if draft.is_empty() {
                    "Write a reply…".to_owned()
                } else {
                    draft
                })
                .id(SharedString::from(format!("reply text {}", request.id)))
                .test_support()
                .p(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(look.card)),
            )
            .child(
                row()
                    .gap(px(8.))
                    .child(button(
                        ui,
                        SharedString::from(format!("send reply {}", request.id)),
                        "Send reply",
                        vec![AppAction::Inbox(InboxAction::SendReply)],
                        look,
                        true,
                    ))
                    .child(button(
                        ui,
                        SharedString::from(format!("cancel reply {}", request.id)),
                        "Cancel",
                        vec![AppAction::Inbox(InboxAction::CancelReply)],
                        look,
                        false,
                    )),
            )
    } else {
        col().child(
            row()
                .gap(px(8.))
                .flex_wrap()
                .when(!is_question, |r| {
                    r.child(button(
                        ui,
                        SharedString::from(format!("allow {}", request.id)),
                        "Allow",
                        vec![AppAction::Inbox(InboxAction::Answer(
                            id.clone(),
                            Answer::Approved,
                        ))],
                        look,
                        true,
                    ))
                    .child(button(
                        ui,
                        SharedString::from(format!("deny {}", request.id)),
                        "Deny",
                        vec![AppAction::Inbox(InboxAction::Answer(
                            id.clone(),
                            Answer::Denied { reply: None },
                        ))],
                        look,
                        false,
                    ))
                })
                .when(is_question, |r| {
                    r.children(request.choices.iter().enumerate().map(|(index, choice)| {
                        button(
                            ui,
                            SharedString::from(format!("choose {} {index}", request.id)),
                            choice.clone(),
                            vec![AppAction::Inbox(InboxAction::Choose(
                                id.clone(),
                                choice.clone(),
                            ))],
                            look,
                            index == 0,
                        )
                    }))
                })
                .child(div().flex_1())
                .child(button(
                    ui,
                    SharedString::from(format!("reply {}", request.id)),
                    "Reply…",
                    vec![AppAction::Inbox(InboxAction::StartReply(id.clone()))],
                    look,
                    false,
                )),
        )
    };

    col()
        .id(SharedString::from(format!("request {}", request.id)))
        .role(Role::Group)
        .aria_label(request.title.clone())
        .test_support()
        .rounded(px(9.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(ORANGE))
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .child(top)
        .child(who)
        .children(meta)
        .child(text(request.reason.clone()))
        .child(preview(request, look))
        .children(steps)
        .children(why_line)
        .when_some(request.undo_note.clone(), |c, note| {
            c.child(text(note).text_size(px(11.5)).text_color(rgb(RED)))
        })
        .when(waiting && !request.rule.is_empty() && !is_question, |c| {
            c.child(tick(ui, request, ticked, look))
        })
        .child(answers)
}
