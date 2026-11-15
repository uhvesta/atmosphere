//! Session panels drawn to the approved D76 frames. Data and operations
//! come from the existing feature states and clients.
use crate::bridge::Ui;
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _, TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
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

/// Tokens copied from the approved light/dark frames, rather than the
/// earlier prototype palette used by the rest of the shell.
#[derive(Clone, Copy)]
struct Look {
    panel: u32,
    line: u32,
    text: u32,
    secondary: u32,
    tertiary: u32,
    accent: u32,
    fill: u32,
    card: u32,
    term: u32,
    term_text: u32,
    term_dim: u32,
    add: u32,
    del: u32,
    content: u32,
}
impl Look {
    fn of(cx: &App) -> Self {
        if cx.theme().is_dark() {
            Self {
                panel: 0x252527,
                line: 0x3a3a3d,
                text: 0xe8e8ea,
                secondary: 0x98989e,
                tertiary: 0x6a6a70,
                accent: 0x3b8cff,
                fill: 0x323235,
                card: 0x2a2a2c,
                term: 0x1a1a1b,
                term_text: 0xdcdcde,
                term_dim: 0x7c7c82,
                add: 0x17301f,
                del: 0x3a1d1d,
                content: 0x1e1e1f,
            }
        } else {
            Self {
                panel: 0xf6f5f7,
                line: 0xdcdadd,
                text: 0x1d1d1f,
                secondary: 0x6e6e73,
                tertiary: 0xa1a1a6,
                accent: 0x0a6cff,
                fill: 0xeceaee,
                card: 0xffffff,
                term: 0xffffff,
                term_text: 0x1d1d1f,
                term_dim: 0x8a8a8f,
                add: 0xe6f6ea,
                del: 0xfdeaea,
                content: 0xffffff,
            }
        }
    }
}
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
                    .overflow_x_scroll()
                    .child(
                        col()
                            .py(px(6.))
                            .font_family("Menlo")
                            .text_size(px(11.5))
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
                                            .whitespace_nowrap()
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
fn request_card(
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
        format!("{} in {}", session.agent.name(), session.title)
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
                        .when(!pending && request.kind != RequestKind::Question, |row| {
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
                        })
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

pub(crate) fn panel(
    ui: &Rc<Ui>,
    state: &AppState,
    panel: Panel,
    width: f32,
    full: bool,
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
                    .child(
                        row().gap(px(8.)).children(
                            [0xff5f57, 0xfebc2e, 0x28c840]
                                .map(|color| div().size(px(12.)).rounded_full().bg(rgb(color))),
                        ),
                    )
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
                    .disabled(matches!(panel, Panel::Files | Panel::Info))
                    .when(on == Some(panel), |tab| tab.bg(rgb(look.fill)))
                    .child(
                        col()
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
                            .child(text(title(panel)).text_size(px(10.)).text_color(rgb(
                                if on == Some(panel) {
                                    look.text
                                } else {
                                    look.secondary
                                },
                            ))),
                    )
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Session(SessionAction::SetPanel(Some(panel))), cx)
                    })
            }),
        )
}
