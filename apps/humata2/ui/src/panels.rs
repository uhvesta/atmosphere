//! Session panels drawn to the approved D76 frames. Data and operations
//! come from the existing feature states and clients.
use crate::bridge::Ui;
use crate::widgets::{self, col, row};
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
use humata2_clients::comments::{Comment as DaemonComment, Status as CommentStatus};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{Panel, SessionAction};
use std::collections::HashSet;
use std::rc::Rc;

use crate::code_wrap;
use crate::theme::Look;
use unicode_width::UnicodeWidthStr;
const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;

#[path = "panel_changes.rs"]
mod changes_panel;
#[path = "panel_inbox.rs"]
mod inbox_panel;
#[path = "panel_board.rs"]
mod board_panel;
#[path = "panel_journal.rs"]
mod journal_panel;
#[path = "panel_files.rs"]
mod files_panel;
#[path = "panel_info.rs"]
mod info_panel;

use changes_panel::*;
use inbox_panel::*;
use board_panel::*;
use journal_panel::*;
use files_panel::*;
use info_panel::*;

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
/// A panel's header: its title, what the panel puts beside it, and the
/// expand and close buttons. The Desk session's panels draw it too.
pub(crate) fn header(
    ui: &Rc<Ui>,
    panel: Panel,
    full: bool,
    extra: Option<Div>,
    look: Look,
) -> Div {
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
pub(crate) fn panel(
    ui: &Rc<Ui>,
    state: &AppState,
    panel: Panel,
    width: f32,
    full: bool,
    file_filter: Entity<InputState>,
    window: &Window,
    documents: &crate::document_tab::Documents,
    cx: &App,
) -> impl IntoElement + use<> {
    let look = Look::of(cx);
    let session = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let body = match (panel, session) {
        (Panel::Changes, _) => changes(ui, state, full, look, width, window, documents),
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
            // The border takes the divider's 1 pt, so the panel keeps its
            // full width inside it, as the frames draw it.
            panel
                .w(px(width + 1.))
                .ml(px(-1.))
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
                                        "{} › {}",
                                        humata2_clients::desk::place(&state.workspace, &session.channel),
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

/// The comments the Diff panel counts for the session: the diff's sent
/// ones and the daemon's drafts.
fn changes_badge(state: &AppState) -> usize {
    let session = state.session.open.as_ref();
    let drafts = session
        .and_then(|session| state.comments.sessions.get(session))
        .map_or(0, |held| held.draft_count as usize);
    let sent: usize = state
        .workspace
        .diffs
        .iter()
        .filter(|diff| Some(&diff.session) == session)
        .map(|diff| diff.comments.len())
        .sum();
    sent + drafts
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
            changes_badge(state),
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
        .gap(px(5.))
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
                    .h(px(46.))
                    .p_0()
                    .rounded(px(7.))
                    .when(on == Some(panel), |tab| tab.bg(rgb(look.fill)))
                    .child(
                        col()
                            .relative()
                            .w_full()
                            .h_full()
                            .items_center()
                            .justify_center()
                            .gap(px(2.))
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
                                        .top(px(2.))
                                        .right(px(6.))
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
