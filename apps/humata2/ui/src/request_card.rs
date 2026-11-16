//! The decision card (IN-9): what is asked, why, what it would do, what the
//! agent was doing, the rule behind it, and the answers.
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route, channel};
use humata2_clients::model::{
    Agent, Answer, LineKind, Preview, Request, RequestKind, RuleSetting, Status,
};
use humata2_inbox::InboxAction;
use std::rc::Rc;

const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
const GREY: u32 = 0x8e8e93;

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

fn setting_name(setting: RuleSetting) -> &'static str {
    match setting {
        RuleSetting::Allow => "Allow",
        RuleSetting::Ask => "Ask me",
        RuleSetting::Never => "Never",
    }
}

fn labelled(look: Look, label: &'static str, value: impl Into<SharedString>) -> Div {
    row()
        .gap(px(8.))
        .items_start()
        .child(secondary(look, label).w(px(64.)).flex_shrink_0())
        .child(
            text(value)
                .flex_1()
                .min_w_0()
                .text_size(px(11.5))
                .font_family("Menlo"),
        )
}

/// The IN-10 preview as labelled rows.
fn preview(request: &Request, look: Look) -> Div {
    let body = match &request.preview {
        Preview::Command(command) => col().child(labelled(look, "Command", command.clone())),
        Preview::Text(value) => {
            let label = match request.kind {
                RequestKind::Network => "Reach",
                RequestKind::Question => "Asks",
                RequestKind::Merge => "Merge",
                RequestKind::Destroy => "Affects",
                _ => "Detail",
            };
            col().child(labelled(look, label, value.clone()))
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
        .role(Role::Button)
        .aria_label(label)
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

/// One request as the decision card. `wide` is the Inbox's detail pane; the
/// session panel leaves out the machine, channel, Open session and the steps.
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
        .map(|c| c.name.clone());
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

    // 2. Who, where, and Open session.
    let by = session.map_or_else(String::new, |s| {
        let mut line = format!("{} in {}", agent_name(s.agent), s.title);
        if !request.operation.is_empty() {
            line.push_str(" · ");
            line.push_str(&request.operation);
        }
        line
    });
    let place = [machine_name, channel_name]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let who = row()
        .gap(px(8.))
        .child(secondary(look, by).flex_1().min_w_0())
        .when(wide && !place.is_empty(), |r| {
            r.child(secondary(look, place))
        })
        .when(wide, |r| {
            r.child(button(
                ui,
                SharedString::from(format!("open session {}", request.id)),
                "Open session ↗",
                vec![AppAction::Inbox(InboxAction::OpenRequest(
                    request.id.clone(),
                ))],
                look,
                false,
            ))
        });

    // 6. The rule line.
    let rule_line = request.asked_by.as_ref().map(|asked| {
        row()
            .gap(px(8.))
            .child(
                secondary(
                    look,
                    format!("{} · {}", setting_name(asked.setting), asked.label),
                )
                .flex_1(),
            )
            .child(button(
                ui,
                SharedString::from(format!("change rule {}", request.id)),
                "Change this rule",
                vec![
                    AppAction::Navigate(Route::Channel(request.channel.clone())),
                    AppAction::Channel(channel::Action::Tab(channel::Tab::Access)),
                ],
                look,
                false,
            ))
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
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(ORANGE))
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .child(top)
        .child(who)
        .child(text(request.reason.clone()))
        .when(!request.why_you.is_empty(), |c| {
            c.child(secondary(look, request.why_you.clone()))
        })
        .child(preview(request, look))
        .when(wide && !request.steps.is_empty(), |c| {
            c.child(
                col()
                    .id(SharedString::from(format!("steps {}", request.id)))
                    .test_support()
                    .gap(px(3.))
                    .child(secondary(look, "What it was doing").font_weight(FontWeight::SEMIBOLD))
                    .children(request.steps.iter().enumerate().map(|(index, step)| {
                        let last = index + 1 == request.steps.len();
                        text(format!("{}. {step}", index + 1))
                            .text_size(px(11.5))
                            .when(last, |t| t.font_weight(FontWeight::SEMIBOLD))
                    })),
            )
        })
        .children(rule_line)
        .when_some(request.undo_note.clone(), |c, note| {
            c.child(text(note).text_size(px(11.5)).text_color(rgb(RED)))
        })
        .when(waiting && !request.rule.is_empty() && !is_question, |c| {
            c.child(tick(ui, request, ticked, look))
        })
        .child(answers)
}
