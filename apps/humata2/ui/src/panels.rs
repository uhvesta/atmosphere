//! Session panels drawn to the approved D76 frames. Data and operations
//! come from the existing feature states and clients.
use crate::bridge::Ui;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _, TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{
    Comment, Item, ItemStatus, LineKind, RepoDiff, Request, Scope, Session, SessionId, Status,
};
use humata2_inbox::InboxAction;
use humata2_clients::comments::Comment as DaemonComment;
use humata2_clients::comments::Status as CommentStatus;
use humata2_clients::Workspace;
use humata2_review::ReviewAction;
use humata2_review::comments::{CommentsAction, CommentsState};
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
        .child(
            text(title(panel))
                .font_weight(FontWeight::SEMIBOLD)
                .flex_shrink_0(),
        )
        // The extra shrinks before the buttons do; it is not clipped, because
        // the base menu is a deferred child that must draw past it.
        .children(extra.map(|e| e.min_w_0().flex_shrink(1.)))
        .child(div().flex_1())
        // The buttons are 16 pt for their 12 pt glyphs; the 6 pt gap and the
        // -2 pt margin put the glyphs 10 pt apart and 14 pt from the edge, as
        // the design draws them.
        .child(
            row()
                .gap(px(6.))
                .mr(px(-2.))
                .flex_shrink_0()
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
                )),
        )
}
fn status(value: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match value {
        Status::Working => dot.bg(rgb(GREEN)),
        Status::NeedsYou => dot.bg(rgb(ORANGE)),
        Status::Exited { .. } => return div().child(icon(IconName::X, RED, 10.)),
        Status::Offline | Status::Stopped | Status::AgentStopped => dot.bg(rgb(0x8e8e93)),
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
                    // The names ellipsize; Show the terminal always stays.
                    .child(
                        col()
                            .min_w_0()
                            .flex_shrink(1.)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .children(session.map(|session| {
                                secondary(
                                    look,
                                    format!(
                                        "{} › {}",
                                        humata2_clients::desk::place(&state.workspace, &session.channel),
                                        session.title
                                    ),
                                )
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
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
        // Under the 52 pt title bar a full-height body would run that far
        // past the window, so it takes what is left.
        .child(body.flex_1().min_h_0())
}

/// The comments the Diff panel counts for the session (D173.2).
fn changes_badge(state: &AppState) -> usize {
    changes_count(state.session.open.as_ref(), &state.workspace, &state.comments)
}

/// The count behind [`changes_badge`]: the diff's sent comments, the sent
/// comments on the session's documents, and the daemon's drafts. A comment
/// made in a diff sits in the diff's list and not again on its document,
/// so it counts once; a draft counts once, in the daemon's draft count.
fn changes_count(
    session: Option<&SessionId>,
    workspace: &Workspace,
    comments: &CommentsState,
) -> usize {
    let held = session.and_then(|session| comments.sessions.get(session));
    let drafts = held.map_or(0, |held| held.draft_count as usize);
    let sent_on_diffs: usize = workspace
        .diffs
        .iter()
        .filter(|diff| Some(&diff.session) == session)
        .map(|diff| diff.comments.len())
        .sum();
    let sent_on_documents = held.map_or(0, |held| {
        held.comments
            .iter()
            .filter(|comment| {
                comment.anchor.base.is_none()
                    && comment.destination == humata2_clients::comments::Destination::Terminal
                    && !comment.archived
                    && !matches!(
                        comment.status,
                        CommentStatus::Draft | CommentStatus::Resolved
                    )
            })
            .count()
    });
    sent_on_diffs + sent_on_documents + drafts
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
                    .h(px(47.))
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
                                        .h(px(16.))
                                        .px(px(6.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .bg(rgb(if panel == Panel::Inbox {
                                            look.accent
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
                            .child(text(title(panel)).text_size(px(10.)).line_height(phi()).text_color(rgb(
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    // The glob above brings GPUI's `test` macro; `#[test]` is the standard one.
    use ::core::prelude::v1::test;
    use humata2_clients::comments::{Anchor, Author, CommentId, Destination, Side};
    use humata2_review::comments::SessionComments;

    const SESSION: &str = "s-rfc";

    fn session() -> SessionId {
        SessionId::new(SESSION)
    }

    /// An anchor on a document's new side; `base` is set only for a comment
    /// made in a diff.
    fn anchor(base: Option<&str>) -> Anchor {
        Anchor {
            repo: String::new(),
            path: "rfc-brin.md".into(),
            side: Side::New,
            first: 7,
            last: 7,
            quoted: Vec::new(),
            base: base.map(str::to_owned),
            detached: false,
            outdated: false,
        }
    }

    fn comment(id: &str, session: &str, anchor: Anchor, status: CommentStatus) -> DaemonComment {
        DaemonComment {
            id: CommentId::new(id),
            session: SessionId::new(session),
            anchor,
            text: "Where are the sizes from?".into(),
            status,
            revision: 1,
            unconfirmed: false,
            archived: false,
            replies: Vec::new(),
            resolved_by: None,
            addressed: None,
            destination: Destination::Terminal,
            author: Author::User,
            posted_url: None,
            post_unconfirmed: false,
        }
    }

    /// A session's held comments with the daemon's draft count.
    fn held(comments: Vec<DaemonComment>, draft_count: u32) -> SessionComments {
        let mut held = SessionComments::default();
        held.comments = comments;
        held.draft_count = draft_count;
        held
    }

    fn comments(sessions: Vec<(&str, SessionComments)>) -> CommentsState {
        CommentsState {
            sessions: sessions
                .into_iter()
                .map(|(id, held)| (SessionId::new(id), held))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    /// A diff of the open session with one sent comment on it.
    fn sent_diff_comment(session: &str) -> RepoDiff {
        RepoDiff {
            session: SessionId::new(session),
            repo: "acme/api".into(),
            base: "main".into(),
            files: Vec::new(),
            comments: vec![Comment {
                path: "src/client.rs".into(),
                line: 42,
                body: "Refresh once only.".into(),
                sent: true,
                context: None,
            }],
        }
    }

    fn workspace(diffs: Vec<RepoDiff>) -> Workspace {
        Workspace {
            diffs,
            ..Workspace::default()
        }
    }

    #[test]
    fn a_sent_document_comment_counts_beside_a_sent_diff_comment() {
        let sent_on_document = comment("s-rfc/c-1", SESSION, anchor(None), CommentStatus::Sent);
        let badge = changes_count(
            Some(&session()),
            &workspace(vec![sent_diff_comment(SESSION)]),
            &comments(vec![(SESSION, held(vec![sent_on_document], 0))]),
        );
        assert_eq!(badge, 2);
    }

    #[test]
    fn a_sent_document_comment_counts_beside_a_draft() {
        let sent_on_document = comment("s-rfc/c-1", SESSION, anchor(None), CommentStatus::Sent);
        let draft_on_document = comment("s-rfc/c-2", SESSION, anchor(None), CommentStatus::Draft);
        let badge = changes_count(
            Some(&session()),
            &workspace(Vec::new()),
            &comments(vec![(
                SESSION,
                held(vec![sent_on_document, draft_on_document], 1),
            )]),
        );
        assert_eq!(badge, 2);
    }

    #[test]
    fn a_comment_made_in_a_diff_counts_once_when_it_also_shows_on_the_document() {
        let made_in_diff = comment(
            "s-rfc/c-1",
            SESSION,
            anchor(Some("origin/main")),
            CommentStatus::Sent,
        );
        let badge = changes_count(
            Some(&session()),
            &workspace(vec![sent_diff_comment(SESSION)]),
            &comments(vec![(SESSION, held(vec![made_in_diff], 0))]),
        );
        assert_eq!(badge, 1);
    }

    #[test]
    fn resolved_and_archived_document_comments_do_not_count() {
        let resolved = comment("s-rfc/c-1", SESSION, anchor(None), CommentStatus::Resolved);
        let mut archived = comment("s-rfc/c-2", SESSION, anchor(None), CommentStatus::Sent);
        archived.archived = true;
        let badge = changes_count(
            Some(&session()),
            &workspace(Vec::new()),
            &comments(vec![(SESSION, held(vec![resolved, archived], 0))]),
        );
        assert_eq!(badge, 0);
    }

    #[test]
    fn another_sessions_document_comments_do_not_count() {
        let elsewhere = comment("s-other/c-1", "s-other", anchor(None), CommentStatus::Sent);
        let badge = changes_count(
            Some(&session()),
            &workspace(Vec::new()),
            &comments(vec![("s-other", held(vec![elsewhere], 0))]),
        );
        assert_eq!(badge, 0);
    }
}
