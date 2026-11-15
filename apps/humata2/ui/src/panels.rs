//! Session panels drawn to the approved D76 frames. Data and operations
//! come from the existing feature states and clients.
use crate::bridge::Ui;
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _, TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{
    Answer, Comment, Item, ItemStatus, LineKind, Preview, RepoDiff, Request, RequestKind, Scope,
    Session, SessionId, Status,
};
use humata2_inbox::InboxAction;
use humata2_review::ReviewAction;
use humata2_session::{Panel, SessionAction};
use std::collections::HashSet;
use std::rc::Rc;

use crate::theme::Look;
const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}
fn secondary(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}
fn icon(name: IconName, color: u32, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(rgb(color))
}
fn action(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    event: AppAction,
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
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
}
fn icon_action(
    ui: &Rc<Ui>,
    id: &'static str,
    label: &'static str,
    glyph: IconName,
    event: AppAction,
    look: Look,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .child(icon(glyph, look.secondary, 12.))
        .accessibility_label(label)
        .ghost()
        .small()
        .w(px(16.))
        .h(px(16.))
        .p_0()
        .flex_shrink_0()
        .text_color(rgb(look.secondary))
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
}
fn title(panel: Panel) -> &'static str {
    if panel == Panel::Changes {
        "Diff"
    } else {
        panel.name()
    }
}
fn header(ui: &Rc<Ui>, panel: Panel, full: bool, extra: Option<Div>, look: Look) -> Div {
    row()
        .gap(px(10.))
        .h(px(36.))
        .px(px(14.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(text(title(panel)).font_weight(FontWeight::SEMIBOLD))
        .children(extra)
        .child(div().flex_1())
        .child(icon_action(
            ui,
            "expand-panel",
            if full {
                "Show the terminal"
            } else {
                "Expand panel"
            },
            if full {
                IconName::Minimize2
            } else {
                IconName::Maximize2
            },
            AppAction::Session(SessionAction::ToggleWiden),
            look,
        ))
        .child(icon_action(
            ui,
            "close-panel",
            "Close panel",
            IconName::X,
            AppAction::Session(SessionAction::SetPanel(None)),
            look,
        ))
}
fn status(value: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match value {
        Status::Working => dot.bg(rgb(GREEN)),
        Status::NeedsYou => dot.bg(rgb(ORANGE)),
        Status::Exited { .. } => return div().child(icon(IconName::X, RED, 10.)),
        Status::Offline => dot.bg(rgb(0x8e8e93)),
        _ => dot.border_1().border_color(rgb(0x8e8e93)),
    };
    row()
        .size(px(10.))
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}
fn card(look: Look) -> Div {
    col()
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
}
fn selected<'a>(
    state: &AppState,
    diffs: &[&'a RepoDiff],
) -> Option<(&'a RepoDiff, &'a humata2_clients::model::FileDiff)> {
    let selection = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.review.saved.selected.get(id));
    selection
        .and_then(|(repo, path)| {
            diffs.iter().find_map(|diff| {
                (diff.repo == *repo)
                    .then(|| {
                        diff.files
                            .iter()
                            .find(|file| file.path == *path)
                            .map(|file| (*diff, file))
                    })
                    .flatten()
            })
        })
        .or_else(|| {
            diffs
                .iter()
                .find_map(|diff| diff.files.first().map(|file| (*diff, file)))
        })
}
fn changes(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
    let diffs: Vec<_> = state
        .workspace
        .diffs
        .iter()
        .filter(|diff| Some(&diff.session) == state.session.open.as_ref())
        .collect();
    let picked = selected(state, &diffs);
    let (files, added, removed) = diffs
        .iter()
        .flat_map(|diff| &diff.files)
        .fold((0_usize, 0_u64, 0_u64), |(n, a, r), f| {
            (n + 1, a + u64::from(f.added), r + u64::from(f.removed))
        });
    let branch = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
        .map_or("", |session| session.branch.as_str());
    let list = col()
        .p(px(8.))
        .gap(px(1.))
        .text_size(px(12.5))
        .when(full, |list| {
            list.w(px(300.))
                .h_full()
                .flex_shrink_0()
                .border_r_1()
                .border_color(rgb(look.line))
        })
        .child(
            row()
                .gap(px(6.))
                .px(px(4.))
                .pb(px(6.))
                .child(secondary(
                    look,
                    format!(
                        "{files} file{} · {} repo{}",
                        if files == 1 { "" } else { "s" },
                        diffs.len(),
                        if diffs.len() == 1 { "" } else { "s" }
                    ),
                ))
                .child(div().flex_1())
                .child(
                    text(format!("+{added}"))
                        .text_size(px(11.))
                        .text_color(rgb(GREEN)),
                )
                .child(
                    text(format!("−{removed}"))
                        .text_size(px(11.))
                        .text_color(rgb(RED)),
                ),
        )
        .children(diffs.iter().map(|diff| {
            col()
                .child(
                    row()
                        .gap(px(6.))
                        .h(px(26.))
                        .px(px(4.))
                        .child(icon(IconName::ChevronDown, look.tertiary, 12.))
                        .child(icon(IconName::FolderGit2, look.secondary, 12.))
                        .child(text(diff.repo.clone()).font_weight(FontWeight::SEMIBOLD))
                        .child(div().flex_1())
                        .child(secondary(look, branch.to_owned()).text_color(rgb(look.tertiary))),
                )
                .children(diff.files.iter().map(|file| {
                    let ui = ui.clone();
                    let repo = diff.repo.clone();
                    let path = file.path.clone();
                    let on = picked.is_some_and(|(d, f)| d.repo == repo && f.path == path);
                    row()
                        .id(SharedString::from(format!("change-file {repo}/{path}")))
                        .role(Role::ListBoxOption)
                        .aria_label(format!("{repo}/{path}"))
                        .aria_selected(on)
                        .test_support()
                        .gap(px(6.))
                        .h(px(24.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(on, |row| row.bg(rgb(look.fill)))
                        .child(icon(IconName::FileCode, look.secondary, 12.))
                        .child(
                            text(path.clone())
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap(),
                        )
                        .child(
                            text(format!("+{}", file.added))
                                .text_size(px(11.))
                                .text_color(rgb(GREEN)),
                        )
                        .child(
                            text(format!("−{}", file.removed))
                                .text_size(px(11.))
                                .text_color(rgb(RED)),
                        )
                        .on_click(move |_, _, cx| {
                            ui.send(
                                AppAction::Review(ReviewAction::SelectFile {
                                    repo: repo.clone(),
                                    path: path.clone(),
                                }),
                                cx,
                            )
                        })
                }))
        }));
    let queued = state.review.queued();
    let view = col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .when_some(picked, |view, (diff, file)| {
            let name = format!("{}/{}", diff.repo, file.path);
            view.child(
                row()
                    .h(px(30.))
                    .px(px(10.))
                    .gap(px(6.))
                    .flex_shrink_0()
                    .border_y_1()
                    .border_color(rgb(look.line))
                    .text_size(px(12.))
                    .child(icon(IconName::FileCode, look.secondary, 12.))
                    .child(text(name.clone()).font_weight(FontWeight::MEDIUM)),
            )
            .child(
                col()
                    .id(SharedString::from(format!("change-code {name}")))
                    .role(Role::Code)
                    .aria_label(name)
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(!state.review.wrap, |x| x.overflow_x_scroll())
                    .when(state.review.wrap, |x| x.overflow_x_hidden())
                    .child(
                        col()
                            .py(px(6.))
                            .font_family("Menlo")
                            .text_size(px(f32::from(state.settings.preferences.font_size) - 0.5))
                            .line_height(px(17.))
                            .bg(rgb(look.term))
                            .children(file.lines.iter().map(|line| {
                                let (sign, bg) = match line.kind {
                                    LineKind::Added => ("+", Some(look.add)),
                                    LineKind::Removed => ("−", Some(look.del)),
                                    _ => (" ", None),
                                };
                                if line.kind == LineKind::Hunk {
                                    return text(line.text.clone())
                                        .px(px(10.))
                                        .text_color(rgb(look.term_dim));
                                }
                                let number = if line.kind == LineKind::Removed {
                                    line.old
                                } else {
                                    line.new
                                };
                                row()
                                    .when_some(bg, |row, bg| row.bg(rgb(bg)))
                                    .child(
                                        text(number.map_or_else(String::new, |n| n.to_string()))
                                            .w(px(34.))
                                            .flex()
                                            .justify_end()
                                            .pr(px(8.))
                                            .text_color(rgb(look.term_dim))
                                            .flex_shrink_0(),
                                    )
                                    .child(
                                        text(format!("{sign} {}", line.text))
                                            .when(!state.review.wrap, |x| x.whitespace_nowrap())
                                            .when(state.review.wrap, |x| x.flex_1().min_w_0())
                                            .text_color(rgb(look.term_text)),
                                    )
                            })),
                    )
                    .children(
                        diff.comments
                            .iter()
                            .chain(queued)
                            .filter(|comment| comment.path == file.path)
                            .map(|comment| comment_card(look, comment)),
                    ),
            )
        })
        .when(picked.is_none(), |view| {
            view.child(secondary(look, "No changes yet").p(px(12.)))
        });
    let base = diffs.first().map_or("", |diff| diff.base.as_str());
    col()
        .size_full()
        .child(header(
            ui,
            Panel::Changes,
            full,
            Some(
                row().gap(px(6.)).child(secondary(look, "against")).child(
                    row()
                        .gap(px(6.))
                        .px(px(9.))
                        .h(px(24.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .bg(rgb(look.fill))
                        .child(text(base.to_owned()))
                        .child(icon(IconName::ChevronsUpDown, look.secondary, 12.)),
                ),
            ),
            look,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .when(!full, |body| body.flex_col())
                .child(list)
                .child(view),
        )
        .child(
            row()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .flex_shrink_0()
                .child(secondary(
                    look,
                    format!(
                        "{} draft comment{}",
                        queued.len(),
                        if queued.len() == 1 { "" } else { "s" }
                    ),
                ))
                .child(div().flex_1())
                .when(!queued.is_empty(), |footer| {
                    footer.child(action(
                        ui,
                        "send-comments",
                        format!("Send {} comments", queued.len()),
                        AppAction::Review(ReviewAction::Send),
                        look,
                        true,
                    ))
                }),
        )
}
fn comment_card(look: Look, comment: &Comment) -> Div {
    card(look)
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .gap(px(4.))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .when(!comment.sent, |card| card.border_color(rgb(look.accent)))
        .child(
            row()
                .gap(px(6.))
                .child(text("You").font_weight(FontWeight::SEMIBOLD))
                .when(!comment.sent, |row| {
                    row.child(
                        text("Draft")
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(rgb(look.line))
                            .text_size(px(11.))
                            .text_color(rgb(look.accent)),
                    )
                }),
        )
        .child(text(comment.body.clone()))
}
fn family(state: &AppState) -> HashSet<SessionId> {
    let mut family = HashSet::new();
    if let Some(id) = &state.session.open {
        family.insert(id.clone());
    }
    loop {
        let old = family.len();
        for session in &state.workspace.sessions {
            if session
                .parent
                .as_ref()
                .is_some_and(|parent| family.contains(parent))
            {
                family.insert(session.id.clone());
            }
        }
        if family.len() == old {
            return family;
        }
    }
}
fn inbox(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
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
fn board(ui: &Rc<Ui>, state: &AppState, session: &Session, full: bool, look: Look) -> Div {
    let channel = state.workspace.channel(&session.channel);
    let items: Vec<_> = state
        .workspace
        .items
        .iter()
        .filter(|item| item.scope == Scope::Channel(session.channel.clone()))
        .collect();
    let statuses = {
        let mut statuses = if full {
            vec![
                ItemStatus::Backlog,
                ItemStatus::Todo,
                ItemStatus::InProgress,
                ItemStatus::InReview,
                ItemStatus::Done,
            ]
        } else {
            vec![ItemStatus::InProgress, ItemStatus::Todo, ItemStatus::Done]
        };
        for status in ItemStatus::ALL {
            if statuses.contains(&status) {
                continue;
            }
            if items.iter().any(|item| item.status == status) {
                statuses.push(status);
            }
        }
        statuses
    };
    col()
        .size_full()
        .child(header(
            ui,
            Panel::Board,
            full,
            channel.map(|channel| secondary(look, channel.name.clone())),
            look,
        ))
        .child(
            div()
                .id("session-board")
                .flex_1()
                .min_h_0()
                .flex()
                .overflow_y_scroll()
                .when(full, |board| board.gap(px(10.)).p(px(14.)))
                .when(!full, |board| board.flex_col().gap(px(6.)).p(px(10.)))
                .children(statuses.into_iter().enumerate().map(|(index, status)| {
                    let lane: Vec<_> = items.iter().filter(|item| item.status == status).collect();
                    col()
                        .gap(px(if full { 8. } else { 6. }))
                        .text_size(px(12.5))
                        .when(full, |column| {
                            column
                                .flex_1()
                                .min_w_0()
                                .p(px(8.))
                                .rounded(px(9.))
                                .bg(rgb(look.panel))
                        })
                        .when(!full && index > 0, |column| column.pt(px(6.)))
                        .child(if full {
                            row()
                                .gap(px(6.))
                                .child(text(status.name()).font_weight(FontWeight::SEMIBOLD))
                                .child(secondary(look, lane.len().to_string()))
                                .child(div().flex_1())
                                .child(icon(IconName::Plus, look.tertiary, 12.))
                        } else {
                            row().child(
                                secondary(look, format!("{} · {}", status.name(), lane.len()))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                        })
                        .children(
                            lane.into_iter()
                                .map(|item| board_item(ui, state, item, full, look)),
                        )
                })),
        )
}
fn board_item(
    ui: &Rc<Ui>,
    state: &AppState,
    item: &Item,
    full: bool,
    look: Look,
) -> impl IntoElement {
    let session = item
        .session
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let mut card = card(look)
        .id(SharedString::from(format!("board-item {}", item.id)))
        .role(Role::ListItem)
        .aria_label(format!("{} {}", item.id, item.title))
        .test_support()
        .px(px(if full { 9. } else { 10. }))
        .py(px(7.))
        .gap(px(6.))
        .when(item.status == ItemStatus::Done, |card| card.opacity(0.6));
    if full {
        card = card
            .p(px(9.))
            .child(
                row()
                    .gap(px(6.))
                    .child(secondary(look, item.id.to_string()))
                    .child(div().flex_1())
                    .children(match &item.scope {
                        Scope::Channel(id) => state
                            .workspace
                            .channel(id)
                            .map(|channel| secondary(look, channel.name.clone())),
                        Scope::Global => None,
                    }),
            )
            .child(text(item.title.clone()).font_weight(FontWeight::MEDIUM))
            .children(session.map(|session| {
                row()
                    .gap(px(5.))
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(rgb(look.fill))
                    .child(status(session.status))
                    .child(secondary(look, session.title.clone()))
            }));
    } else {
        card = card.child(
            row()
                .gap(px(7.))
                .child(secondary(look, item.id.to_string()))
                .child(text(item.title.clone()).flex_1())
                .children(session.map(|session| status(session.status))),
        );
    }
    if let Some(session) = session {
        let ui = ui.clone();
        let id = session.id.clone();
        card = card
            .cursor_pointer()
            .on_click(move |_, _, cx| ui.send(AppAction::Navigate(Route::Session(id.clone())), cx));
    }
    card
}
fn journal(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
    col()
        .size_full()
        .child(header(ui, Panel::Journal, full, None, look))
        .child(
            col()
                .id("session-journal")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .p(px(10.))
                .text_size(px(12.5))
                .children(
                    state
                        .workspace
                        .journal
                        .iter()
                        .filter(|entry| entry.session.as_ref() == state.session.open.as_ref())
                        .map(|entry| {
                            card(look)
                                .p(px(10.))
                                .gap(px(2.))
                                .child(
                                    row()
                                        .gap(px(6.))
                                        .child(icon(IconName::Sparkles, 0xbf5af2, 12.))
                                        .child(
                                            text(entry.text.clone())
                                                .font_weight(FontWeight::SEMIBOLD),
                                        ),
                                )
                                .child(secondary(look, entry.at.clone()))
                        }),
                ),
        )
}

#[derive(Default)]
struct FileNode {
    folder: bool,
    children: std::collections::BTreeMap<String, FileNode>,
}
fn file_rows(
    ui: &Rc<Ui>,
    state: &AppState,
    repo: &str,
    node: &FileNode,
    path: &str,
    depth: usize,
    filter: &str,
    look: Look,
) -> Vec<AnyElement> {
    let mut children: Vec<_> = node.children.iter().collect();
    children.sort_by(|(an, a), (bn, b)| b.folder.cmp(&a.folder).then_with(|| an.cmp(bn)));
    let mut rows = Vec::new();
    for (name, child) in children {
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        if !filter.is_empty()
            && !path.to_lowercase().contains(filter)
            && !contains_file(child, &path, filter)
        {
            continue;
        }
        let folded = state
            .session
            .view()
            .is_some_and(|view| view.folded_files.contains(&(repo.to_owned(), path.clone())));
        let selected = state
            .session
            .view()
            .and_then(|view| view.file.as_ref())
            .is_some_and(|(r, p)| r == repo && p == &path);
        let changed = state
            .workspace
            .diffs
            .iter()
            .filter(|diff| Some(&diff.session) == state.session.open.as_ref() && diff.repo == repo)
            .any(|diff| diff.files.iter().any(|file| file.path == path));
        let event = if child.folder {
            SessionAction::ToggleFolder {
                repo: repo.to_owned(),
                path: path.clone(),
            }
        } else {
            SessionAction::SelectFile {
                repo: repo.to_owned(),
                path: path.clone(),
            }
        };
        let click_ui = ui.clone();
        rows.push(
            row()
                .id(SharedString::from(format!(
                    "session-{} {repo}/{path}",
                    if child.folder { "folder" } else { "file" }
                )))
                .role(if child.folder {
                    Role::Button
                } else {
                    Role::ListItem
                })
                .aria_label(format!("{repo}/{path}"))
                .test_support()
                .h(px(23.))
                .pl(px(8. + depth as f32 * 14.))
                .pr(px(8.))
                .gap(px(6.))
                .rounded(px(5.))
                .when(selected, |row| row.bg(rgb(look.fill)))
                .child(icon(
                    if child.folder {
                        if folded {
                            IconName::Folder
                        } else {
                            IconName::FolderOpen
                        }
                    } else {
                        IconName::FileCode
                    },
                    look.secondary,
                    12.,
                ))
                .child(text(name.clone()).flex_1())
                .when(changed && !child.folder, |row| {
                    row.child(
                        text("M")
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(ORANGE)),
                    )
                })
                .cursor_pointer()
                .on_click(move |_, _, cx| click_ui.send(AppAction::Session(event.clone()), cx))
                .into_any_element(),
        );
        if child.folder && (!folded || !filter.is_empty()) {
            rows.extend(file_rows(
                ui,
                state,
                repo,
                child,
                &path,
                depth + 1,
                filter,
                look,
            ));
        }
    }
    rows
}
fn contains_file(node: &FileNode, path: &str, filter: &str) -> bool {
    node.children.iter().any(|(name, child)| {
        let path = format!("{path}/{name}");
        path.to_lowercase().contains(filter) || contains_file(child, &path, filter)
    })
}
fn files(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    full: bool,
    input: Entity<InputState>,
    look: Look,
) -> Div {
    let mut repos = std::collections::BTreeMap::<String, FileNode>::new();
    for file in state
        .workspace
        .session_files
        .get(&session.id)
        .into_iter()
        .flatten()
    {
        let mut node = repos.entry(file.repo.clone()).or_default();
        for component in file.path.split('/').filter(|part| !part.is_empty()) {
            node.folder = true;
            node = node.children.entry(component.to_owned()).or_default();
        }
        node.folder |= file.folder;
    }
    let filter = state
        .session
        .view()
        .map_or("", |view| view.file_filter.as_str())
        .to_lowercase();
    let mut rows = Vec::new();
    for (repo, node) in &repos {
        let children = file_rows(ui, state, repo, node, "", 1, &filter, look);
        if !filter.is_empty() && children.is_empty() {
            continue;
        }
        let folded = state
            .session
            .view()
            .is_some_and(|view| view.folded_files.contains(&(repo.clone(), String::new())));
        let repo_name = repo.rsplit('/').next().unwrap_or(repo);
        let click_ui = ui.clone();
        let repo_id = repo.clone();
        rows.push(
            row()
                .id(SharedString::from(format!("session-folder {repo}/")))
                .role(Role::Button)
                .aria_label(repo.clone())
                .test_support()
                .h(px(23.))
                .px(px(8.))
                .gap(px(6.))
                .child(icon(IconName::FolderGit2, look.secondary, 12.))
                .child(text(repo_name.to_owned()))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    click_ui.send(
                        AppAction::Session(SessionAction::ToggleFolder {
                            repo: repo_id.clone(),
                            path: String::new(),
                        }),
                        cx,
                    )
                })
                .into_any_element(),
        );
        if !folded || !filter.is_empty() {
            rows.extend(children);
        }
    }
    col()
        .size_full()
        .child(header(ui, Panel::Files, full, None, look))
        .child(
            div()
                .id("session-file-filter")
                .test_support()
                .p(px(10.))
                .child(
                    Input::new(&input)
                        .aria_label("Filter files")
                        .small()
                        .h(px(26.))
                        .bg(rgb(look.card))
                        .border_color(rgb(look.line))
                        .text_color(rgb(look.text)),
                ),
        )
        .child(
            col()
                .id("session-files")
                .role(Role::List)
                .aria_label("Session files")
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(6.))
                .text_size(px(12.5))
                .children(rows),
        )
        .child(
            row()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(
                    action(
                        ui,
                        "files-open-big",
                        "Open big",
                        AppAction::Session(SessionAction::OpenFile),
                        look,
                        false,
                    )
                    .disabled(state.session.view().is_none_or(|v| v.file.is_none())),
                )
                .child(
                    Button::new("files-open-editor")
                        .label("Open in editor")
                        .small()
                        .disabled(true),
                )
                .child(
                    Button::new("files-upload")
                        .label("Upload…")
                        .small()
                        .disabled(true),
                ),
        )
}
fn info(ui: &Rc<Ui>, state: &AppState, session: &Session, full: bool, look: Look) -> Div {
    let kv = |key: &'static str, value: Div| {
        row()
            .items_start()
            .gap(px(10.))
            .py(px(3.))
            .child(
                text(key)
                    .w(px(84.))
                    .flex_shrink_0()
                    .text_color(rgb(look.secondary)),
            )
            .child(value.flex_1().min_w_0())
    };
    let machine = state.workspace.machine(&session.machine);
    let forks = state.workspace.forks(&session.id).map(|fork| {
        let click_ui = ui.clone();
        let id = fork.id.clone();
        Button::new(SharedString::from(format!("session-info-fork {}", fork.id)))
            .accessibility_label(fork.title.clone())
            .ghost()
            .small()
            .w_full()
            .h(px(23.))
            .p_0()
            .child(
                text(fork.title.clone())
                    .w_full()
                    .text_left()
                    .text_color(rgb(look.accent)),
            )
            .on_click(move |_, _, cx| {
                click_ui.send(AppAction::Navigate(Route::Session(id.clone())), cx)
            })
    });
    let recovery = session.recovery.as_ref().map_or_else(
        || "Recovery has not been reported for this session.".to_owned(),
        |recovery| recovery.label(),
    );
    let clones = state
        .workspace
        .session_files
        .get(&session.id)
        .into_iter()
        .flatten()
        .map(|file| file.repo.as_str())
        .chain((!session.repo.is_empty()).then_some(session.repo.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    let clone_rows = clones.into_iter().map(|repo| {
        row()
            .flex_wrap()
            .gap(px(6.))
            .child(text(repo.rsplit('/').next().unwrap_or(repo).to_owned()))
            .when(repo == session.repo, |row| {
                row.child(
                    text(session.branch.clone())
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .text_size(px(11.))
                        .text_color(rgb(look.secondary)),
                )
            })
    });
    let details = col()
        .child(kv("Agent", text(session.agent.name())))
        .child(kv(
            "Machine",
            text(machine.map_or_else(
                || session.machine.to_string(),
                |machine| format!("{} · {}", machine.name, machine.detail),
            )),
        ))
        .child(kv("Started", text(session.started.clone())))
        .child(kv("Forks", col().children(forks)))
        .child(kv("Clones", col().gap(px(3.)).children(clone_rows)));
    let restart = card(look)
        .p(px(10.))
        .gap(px(4.))
        .child(
            row()
                .gap(px(6.))
                .child(icon(IconName::ShieldCheck, GREEN, 12.))
                .child(text("Survives restarts").font_weight(FontWeight::SEMIBOLD)),
        )
        .child(secondary(look, recovery));
    let permissions = card(look)
        .p(px(10.))
        .gap(px(4.))
        .child(
            row()
                .gap(px(6.))
                .child(icon(IconName::Bot, look.accent, 12.))
                .child(text("What this agent may do").font_weight(FontWeight::SEMIBOLD)),
        )
        .child(secondary(
            look,
            "Permission rules have not been reported for this session.",
        ))
        .child(
            Button::new("session-channel-rules")
                .accessibility_label("Channel rules")
                .child(text("Channel rules").w_full().text_left())
                .ghost()
                .w_full()
                .small()
                .text_color(rgb(look.accent))
                .disabled(true),
        );
    col()
        .size_full()
        .child(header(ui, Panel::Info, full, None, look))
        .child(
            col()
                .id(SharedString::from(format!("session-info {}", session.id)))
                .test_support()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(10.))
                .p(px(12.))
                .text_size(px(12.5))
                .child(details)
                .child(restart)
                .child(permissions),
        )
}

pub(crate) fn panel(
    ui: &Rc<Ui>,
    state: &AppState,
    panel: Panel,
    width: f32,
    full: bool,
    file_filter: Entity<InputState>,
    cx: &App,
) -> impl IntoElement + use<> {
    let look = Look::of(cx);
    let session = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let body = match (panel, session) {
        (Panel::Changes, _) => changes(ui, state, full, look),
        (Panel::Inbox, _) => inbox(ui, state, full, look),
        (Panel::Board, Some(session)) => board(ui, state, session, full, look),
        (Panel::Journal, _) => journal(ui, state, full, look),
        (Panel::Files, Some(session)) => files(ui, state, session, full, file_filter, look),
        (Panel::Info, Some(session)) => info(ui, state, session, full, look),
        _ => col().child(header(ui, panel, full, None, look)),
    };
    col()
        .id(if full {
            "panel-full-screen"
        } else {
            "side-panel"
        })
        .role(Role::Complementary)
        .aria_label(panel.name())
        .test_support()
        .h_full()
        .overflow_hidden()
        .font_family(".SystemUIFont")
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .bg(rgb(look.panel))
        .when(!full, |panel| {
            panel
                .w(px(width))
                .flex_shrink_0()
                .border_l_1()
                .border_color(rgb(look.line))
        })
        .when(full, |frame| {
            frame.flex_1().child(
                row()
                    .h(px(52.))
                    .px(px(16.))
                    .gap(px(12.))
                    .flex_shrink_0()
                    .bg(rgb(look.content))
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .child(crate::chrome::lights())
                    .child(div().w(px(4.)))
                    .child(icon(IconName::PanelLeftOpen, look.secondary, 15.))
                    .child(
                        col()
                            .children(session.map(|session| {
                                secondary(
                                    look,
                                    format!(
                                        "# {} › {}",
                                        state
                                            .workspace
                                            .channel(&session.channel)
                                            .map_or("", |channel| channel.name.as_str()),
                                        session.title
                                    ),
                                )
                            }))
                            .child(
                                text(title(panel))
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD),
                            ),
                    )
                    .child(div().flex_1())
                    .child(action(
                        ui,
                        "show-terminal",
                        "Show the terminal",
                        AppAction::Session(SessionAction::ToggleWiden),
                        look,
                        false,
                    )),
            )
        })
        .child(body)
}

pub(crate) fn strip(ui: &Rc<Ui>, on: Option<Panel>, cx: &App) -> impl IntoElement + use<> {
    let look = Look::of(cx);
    let (pending, unsent) = ui.read(|state| {
        let sessions = family(state);
        (
            state
                .workspace
                .requests
                .iter()
                .filter(|request| request.answer.is_none() && sessions.contains(&request.session))
                .count(),
            state.review.queued().len(),
        )
    });
    col()
        .id("panel-strip")
        .role(Role::Toolbar)
        .aria_label("Session panels")
        .test_support()
        .w(px(64.))
        .h_full()
        .flex_shrink_0()
        .items_center()
        .gap(px(4.))
        .pt(px(60.))
        .bg(rgb(if cx.theme().is_dark() {
            0x2a2a2d
        } else {
            0xe9e7ea
        }))
        .border_l_1()
        .border_color(rgb(look.line))
        .children(
            [
                (Panel::Inbox, IconName::Inbox),
                (Panel::Changes, IconName::FileDiff),
                (Panel::Files, IconName::FolderTree),
                (Panel::Board, IconName::SquareKanban),
                (Panel::Journal, IconName::BookOpenText),
                (Panel::Info, IconName::Info),
            ]
            .map(|(panel, glyph)| {
                let ui = ui.clone();
                Button::new(SharedString::from(format!("panel-tab {}", panel.name())))
                    .accessibility_label(title(panel))
                    .ghost()
                    .selected(on == Some(panel))
                    .w(px(52.))
                    .h(px(48.))
                    .p_0()
                    .rounded(px(7.))
                    .when(on == Some(panel), |tab| tab.bg(rgb(look.fill)))
                    .child(
                        col()
                            .relative()
                            .items_center()
                            .gap(px(2.))
                            .py(px(7.))
                            .child(icon(
                                glyph,
                                if on == Some(panel) {
                                    look.accent
                                } else {
                                    look.secondary
                                },
                                15.,
                            ))
                            .children(
                                match panel {
                                    Panel::Inbox => Some(pending),
                                    Panel::Changes => Some(unsent),
                                    _ => None,
                                }
                                .filter(|count| *count > 0)
                                .map(|count| {
                                    div()
                                        .id(SharedString::from(format!(
                                            "panel-badge {}",
                                            panel.name()
                                        )))
                                        .role(Role::Status)
                                        .aria_label(format!("{}: {count}", title(panel)))
                                        .absolute()
                                        .top(px(-4.))
                                        .right(px(-8.))
                                        .min_w(px(16.))
                                        .h(px(16.))
                                        .px(px(3.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(10.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .bg(rgb(if panel == Panel::Inbox {
                                            look.selection
                                        } else {
                                            look.fill
                                        }))
                                        .text_color(rgb(if panel == Panel::Inbox {
                                            0xffffff
                                        } else {
                                            look.secondary
                                        }))
                                        .child(count.to_string())
                                        .test_support()
                                }),
                            )
                            .child(text(title(panel)).text_size(px(10.)).text_color(rgb(
                                if on == Some(panel) {
                                    look.text
                                } else {
                                    look.secondary
                                },
                            ))),
                    )
                    .on_click(move |_, _, cx| {
                        ui.send(
                            AppAction::Session(SessionAction::SetPanel(if on == Some(panel) {
                                None
                            } else {
                                Some(panel)
                            })),
                            cx,
                        )
                    })
            }),
        )
}
