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
use crate::chrome::{
    GREEN, ORANGE, RED, card, family, header_with, icon, icon_action, panel_action as action,
    panel_status as status, secondary, selected, session_line, text, title,
};
pub(crate) use crate::chrome::panel_header;

use crate::panel_changes::*;
use crate::panel_files::*;
use crate::panel_inbox::*;
use crate::panel_info::*;
use crate::panel_journal::*;

pub(crate) fn panel(
    ui: &Rc<Ui>,
    state: &AppState,
    panel: Panel,
    width: f32,
    full: bool,
    file_filter: Entity<InputState>,
    window: &Window,
    documents: &crate::document_tab::Documents,
    todo_inputs: &crate::todo::Inputs,
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
        // Every session, a Desk session too, shows its slice of the To-do
        // list (frame b9-3).
        (Panel::Board, Some(session)) => todo_panel(ui, state, session, full, todo_inputs, look),
        (Panel::Journal, _) => journal(ui, state, full, look),
        (Panel::Files, Some(session)) => files(ui, state, session, full, file_filter, look),
        (Panel::Info, Some(session)) => info(ui, state, session, full, look),
        _ => col().child(panel_header(ui, state, panel, full, None, look)),
    };
    col()
        .id(if full {
            "panel-full-screen"
        } else {
            "side-panel"
        })
        .role(Role::Complementary)
        .aria_label(title(panel))
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
        .when(full, |frame| frame.flex_1())
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
                    .h(px(if panel == Panel::Inbox { 63. } else { 47. }))
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
                            .child(
                                // One centred line per word, so a two-word
                                // title wraps in the cell instead of clipping.
                                col().items_center().children(title(panel).split(' ').map(|word| {
                                    text(word.to_owned())
                                        .text_center()
                                        .whitespace_nowrap()
                                        .text_size(px(10.))
                                        .line_height(phi())
                                        .text_color(rgb(if on == Some(panel) {
                                            look.text
                                        } else {
                                            look.secondary
                                        }))
                                })),
                            ),
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

/// The session's To-do panel under the panel header.
fn todo_panel(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    full: bool,
    inputs: &crate::todo::Inputs,
    look: Look,
) -> Div {
    let send: crate::todo::Send = {
        let ui = ui.clone();
        Rc::new(move |action, cx| ui.send(AppAction::Todo(action), cx))
    };
    let chips = |id: &str| crate::pages::session_chip(state, id);
    let id = session.id.as_str();
    // Frame 191: beside the terminal an open item's way back is the header.
    let lead = (!full)
        .then(|| humata2_ui_workspace::todo_panel::back(&send, &state.todo, id, look))
        .flatten();
    let header = match lead {
        Some(lead) => crate::chrome::panel_header_lead(ui, Panel::Board, lead, None, look),
        None => panel_header(ui, state, Panel::Board, full, None, look),
    };
    col().size_full().child(header).child(humata2_ui_workspace::todo_panel::panel(
        &send,
        &state.todo,
        id,
        inputs,
        &chips,
        full,
        look,
    ))
}
