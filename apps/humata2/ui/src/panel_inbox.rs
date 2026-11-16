//! The session's Inbox panel and its request cards.
use super::*;

pub(super) fn inbox(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
    let family = family(state);
    let requests: Vec<_> = state
        .workspace
        .requests
        .iter()
        .filter(|request| family.contains(&request.session) && request.answer.is_none())
        .collect();
    let selected = state
        .inbox
        .selected
        .as_ref()
        .filter(|id| requests.iter().any(|request| request.id == **id))
        .or_else(|| requests.first().map(|request| &request.id));
    col()
        .size_full()
        .child(header(
            ui,
            Panel::Inbox,
            full,
            Some(secondary(look, "this session and its forks")),
            look,
        ))
        .child(
            col()
                .id("session-inbox")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .p(px(10.))
                .when(requests.is_empty(), |body| {
                    body.child(secondary(look, "Nothing waiting for you"))
                })
                .children(requests.iter().map(|request| {
                    request_card(ui, state, request, selected == Some(&request.id), look)
                }))
                .children(state.inbox.error.as_ref().map(|error| {
                    text(error.clone())
                        .id("panel-error")
                        .role(Role::Alert)
                        .aria_label("Answer failed")
                        .text_color(rgb(RED))
                        .test_support()
                })),
        )
}

pub(crate) fn request_card(
    ui: &Rc<Ui>,
    state: &AppState,
    request: &Request,
    open: bool,
    look: Look,
) -> impl IntoElement {
    let top = row()
        .gap(px(6.))
        .child(status(Status::NeedsYou))
        .child(
            text(request.title.clone())
                .font_weight(FontWeight::SEMIBOLD)
                .flex_1(),
        )
        .child(secondary(look, request.asked.clone()));
    let session = state.workspace.session(&request.session);
    let by = session.map_or_else(String::new, |session| {
        format!(
            "{} in {}{}",
            if session.agent == humata2_clients::model::Agent::Claude {
                "Claude Code"
            } else {
                session.agent.name()
            },
            session.title,
            if request.rule.is_empty() {
                String::new()
            } else {
                format!(" · {}", request.rule)
            }
        )
    });
    card(look)
        .id(SharedString::from(format!("request {}", request.id)))
        .role(if open { Role::Group } else { Role::Button })
        .aria_label(request.title.clone())
        .test_support()
        .when(!open, |card| {
            let ui = ui.clone();
            let id = request.id.clone();
            card.cursor_pointer().on_click(move |_, _, cx| {
                ui.send(AppAction::Inbox(InboxAction::Select(id.clone())), cx)
            })
        })
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .when(open, |card| card.border_color(rgb(ORANGE)))
        .child(top)
        .child(secondary(look, by))
        .when(open, |card| {
            let preview = match &request.preview {
                Preview::Text(value) | Preview::Command(value) => value.clone(),
                Preview::Diff(lines) => lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            let pending = state.inbox.pending.contains(&request.id);
            card.child(text(request.reason.clone()))
                .child(
                    text(preview)
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(rgb(look.fill))
                        .text_size(px(11.5)),
                )
                // The client has only a broader Always rule. This scoped
                // affordance stays disabled until a session grant exists.
                .child(
                    Checkbox::new("session-allow-rule")
                        .label("Allow this for the rest of the session")
                        .small()
                        .disabled(true),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .when(
                            !pending
                                && request.answer.is_none()
                                && request.kind != RequestKind::Question,
                            |row| {
                                row.child(action(
                                    ui,
                                    SharedString::from(format!("allow {}", request.id)),
                                    "Allow",
                                    AppAction::Inbox(InboxAction::Answer(
                                        request.id.clone(),
                                        Answer::Approved,
                                    )),
                                    look,
                                    true,
                                ))
                                .child(action(
                                    ui,
                                    SharedString::from(format!("deny {}", request.id)),
                                    "Deny",
                                    AppAction::Inbox(InboxAction::Answer(
                                        request.id.clone(),
                                        Answer::Denied { reply: None },
                                    )),
                                    look,
                                    false,
                                ))
                            },
                        )
                        .when(pending, |row| row.child(secondary(look, "Answering…")))
                        .child(div().flex_1())
                        .child(
                            action(
                                ui,
                                SharedString::from(format!("reply {}", request.id)),
                                "Reply…",
                                AppAction::Inbox(InboxAction::StartReply(request.id.clone())),
                                look,
                                false,
                            )
                            .disabled(true),
                        ),
                )
        })
}
