//! Comment cards and the comment box, as frames 42 and 43 draw them under a
//! source line: the same look for every file view (D166).

use std::rc::Rc;

use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::comments::{Anchor, Comment, CommentId, Status};
use humata2_clients::model::SessionId;
use humata2_review::comments::CommentsAction;

use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, col, row};

/// What starts re-attaching a detached comment: the next source line the
/// person clicks becomes its new place.
pub(crate) type StartReattach = Rc<dyn Fn(&CommentId, &mut App)>;

fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

/// "line 7" or "lines 13–14".
pub(crate) fn lines_label(anchor: &Anchor) -> String {
    if anchor.first == anchor.last {
        format!("line {}", anchor.first)
    } else {
        format!("lines {}–{}", anchor.first, anchor.last)
    }
}

fn shell(look: Look, open: bool) -> Div {
    col()
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .gap(px(4.))
        .max_w(px(640.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(if open { look.accent } else { look.line }))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .text_color(rgb(look.text))
}

fn chip(look: Look, label: &'static str) -> Div {
    text(label)
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(11.))
        .text_color(rgb(look.accent))
}

/// One comment under its line. `agent` is the command it was sent to, such
/// as `claude`. A detached comment offers Re-attach and Drop; while it is
/// being re-attached (`reattaching`) it says to click the line.
pub(crate) fn card(
    ui: &Rc<Ui>,
    look: Look,
    comment: &Comment,
    agent: &str,
    reattaching: bool,
    start_reattach: &StartReattach,
) -> impl IntoElement {
    let lines = lines_label(&comment.anchor);
    let id = comment.id.clone();
    let session = comment.session.clone();
    let label = if comment.anchor.detached {
        Some("Detached")
    } else {
        match comment.status {
            Status::Draft => Some("Draft"),
            Status::Waiting => Some("Waiting to send"),
            Status::Addressed => Some("Addressed"),
            Status::Resolved => Some("Resolved"),
            Status::Sent => None,
        }
    };
    let title = if comment.status == Status::Sent && !comment.anchor.detached {
        format!("You · {lines} · sent to {agent}")
    } else {
        format!("You · {lines}")
    };
    let open = matches!(comment.status, Status::Draft | Status::Waiting);
    shell(look, open)
        .id(SharedString::from(format!("doc-comment-{id}")))
        .test_support()
        .child(
            row()
                .gap(px(6.))
                .child(text(title).font_weight(FontWeight::SEMIBOLD))
                .children(label.map(|label| chip(look, label)))
                .child(div().flex_1())
                .children(comment.counts_as_draft().then(|| {
                    widgets::ghost_in(look,
                        ui,
                        SharedString::from(format!("doc-edit-comment-{id}")),
                        "Edit",
                        AppAction::Comments(CommentsAction::EditDraft(session.clone(), id.clone())),
                    )
                    .h(px(20.))
                    .px(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(look.accent))
                }))
                .children((comment.status == Status::Addressed && !comment.anchor.detached).then(
                    || {
                        widgets::ghost_in(look,
                            ui,
                            SharedString::from(format!("doc-resolve-comment-{id}")),
                            "Resolve",
                            AppAction::Comments(CommentsAction::Resolve(
                                session.clone(),
                                id.clone(),
                            )),
                        )
                        .h(px(20.))
                        .px(px(6.))
                        .text_size(px(11.))
                        .text_color(rgb(look.accent))
                    },
                )),
        )
        .child(text(comment.text.clone()))
        .children(comment.anchor.detached.then(|| {
            let start = start_reattach.clone();
            let again = id.clone();
            col()
                .gap(px(4.))
                .child(
                    text(if reattaching {
                        "Click the line this comment belongs on."
                    } else {
                        "Its lines are gone from the file."
                    })
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary)),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(div().flex_1())
                        .child(widgets::push_quiet(
                            ui,
                            SharedString::from(format!("doc-drop-comment-{id}")),
                            "Drop",
                            AppAction::Comments(CommentsAction::Drop(session.clone(), id.clone())),
                            look,
                        ))
                        .child(
                            widgets::push_quiet_shape(
                                SharedString::from(format!("doc-reattach-comment-{id}")),
                                "Re-attach",
                                look,
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                start(&again, cx);
                            }),
                        ),
                )
        }))
}

/// The box a comment is written in: who and where, the text, what Comment
/// does, then Cancel and Comment. `input` is the text field the holder keeps
/// in step with the box.
pub(crate) fn open_box(
    ui: &Rc<Ui>,
    look: Look,
    session: &SessionId,
    anchor: &Anchor,
    input: &Entity<TextareaState>,
    agent: &str,
) -> impl IntoElement {
    shell(look, true)
        .id("doc-comment-box")
        .test_support()
        .border_color(rgb(look.accent))
        .child(
            row()
                .gap(px(6.))
                .child(text(format!("You · {}", lines_label(anchor))).font_weight(FontWeight::SEMIBOLD))
                .child(chip(look, "Draft")),
        )
        .child(
            // The field reads as the card's text: the card's size and line
            // height, no border or fill, no inner padding, and as tall as its
            // lines. XSmall leaves the editor 4 pt on each side and the editor
            // keeps 10 more at the end of a line for its caret; the negative
            // margins take them back, so the text wraps where the card's
            // does. The card's accent border is the focus ring.
            Textarea::new(input)
                .xsmall()
                .appearance(false)
                .bordered(false)
                .text_size(px(12.))
                .line_height(phi())
                .ml(px(-4.))
                .mr(px(-14.))
                .aria_label("Comment"),
        )
        .child(
            text(format!(
                "Saved as a draft; Send types it into {agent} straight away"
            ))
            .text_size(px(11.5))
            .text_color(rgb(look.secondary)),
        )
        .child(
            row()
                .gap(px(8.))
                .pt(px(2.))
                .child(div().flex_1())
                .child(widgets::push_quiet(
                    ui,
                    "doc-comment-cancel",
                    "Cancel",
                    AppAction::Comments(CommentsAction::Cancel(session.clone())),
                    look,
                ))
                .child(widgets::push_primary(
                    ui,
                    "doc-comment-send",
                    "Comment",
                    AppAction::Comments(CommentsAction::Comment(session.clone())),
                    look,
                )),
        )
}
