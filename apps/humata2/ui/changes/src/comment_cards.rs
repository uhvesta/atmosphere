//! Comment cards and the comment box, as frames 42 and 43 draw them under a
//! source line: the same look for every file view (D166).

use std::rc::Rc;

use gpui_kit::base::{ObservedElement, TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, AskAction};
use humata2_clients::comments::{Anchor, Author, Comment, CommentId, Destination, Status};
use humata2_clients::model::SessionId;
use humata2_review::comments::{CommentsAction, github_target};

use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, col, row};

/// What starts re-attaching a detached comment: the next source line the
/// person clicks becomes its new place.
pub type StartReattach = Rc<dyn Fn(&CommentId, &mut App)>;

impl BoxOffers {
    /// The box of a comment that already exists for a pull request: it
    /// can't be turned into another draft.
    pub fn without_github(mut self) -> Self {
        self.github = None;
        self
    }
}

/// What a comment box offers besides Comment in a pull request review
/// (PR-11, PR-19).
#[derive(Clone, Debug, Default)]
pub struct BoxOffers {
    /// The session reviews a pull request: the box offers Ask.
    review: bool,
    /// Draft for GitHub, with the pull request it is for.
    github: Option<(String, u32)>,
}

/// What the box for a comment on `repo` of `session` offers.
pub fn offers(state: &AppState, session: &SessionId, repo: &str) -> BoxOffers {
    BoxOffers {
        review: state
            .workspace
            .session(session)
            .is_some_and(|found| found.pull_request_review),
        github: github_target(&state.workspace, session, repo),
    }
}

fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

/// Who wrote a draft, as people read it: the agent by its product name.
fn agent_label(author: &Author) -> String {
    match author {
        Author::User => "you".to_owned(),
        Author::Agent(name) if name == "claude" => "Claude Code".to_owned(),
        Author::Agent(name) => name.clone(),
    }
}

/// "line 7" or "lines 13–14".
pub fn lines_label(anchor: &Anchor) -> String {
    if anchor.first == anchor.last {
        format!("line {}", anchor.first)
    } else {
        format!("lines {}–{}", anchor.first, anchor.last)
    }
}

/// What a comment's card says and offers, from its status. Draft and Waiting
/// are one "Draft" state; Addressed and Resolved are both "Done", by whoever
/// did it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardState {
    /// The chip next to the title.
    pub chip: Option<String>,
    /// Reopen is offered.
    pub reopen: bool,
    /// The comment is done: addressed or resolved.
    pub done: bool,
}

/// Whether a comment counts as done (addressed by the agent or resolved).
#[must_use]
pub fn is_done(comment: &Comment) -> bool {
    matches!(comment.status, Status::Addressed | Status::Resolved) && !comment.anchor.detached
}

/// The state a comment's card shows. `agent` is the command it was sent to.
#[must_use]
pub fn card_state(comment: &Comment, agent: &str) -> CardState {
    let done = is_done(comment);
    let chip = if comment.anchor.detached {
        Some("Detached".to_owned())
    } else if comment.destination == Destination::Note {
        Some("Note".to_owned())
    } else {
        match comment.status {
            Status::Draft | Status::Waiting => Some("Draft".to_owned()),
            Status::Sent => Some("Sent".to_owned()),
            Status::Addressed => Some(format!("Done by {agent}")),
            Status::Resolved => Some(match &comment.resolved_by {
                Some(Author::Agent(name)) => format!("Done by {name}"),
                _ => "Done by you".to_owned(),
            }),
            Status::Posted => Some("Posted".to_owned()),
        }
    };
    CardState {
        chip,
        reopen: done,
        done,
    }
}

/// The last thing the agent said about a comment, if it replied.
#[must_use]
pub fn agent_reply(comment: &Comment) -> Option<(&str, &str)> {
    comment.replies.iter().rev().find_map(|reply| match &reply.author {
        Author::Agent(name) => Some((name.as_str(), reply.text.as_str())),
        Author::User => None,
    })
}

/// How many comments are done when every one of `comments` is, else `None`.
#[must_use]
pub fn all_done<'a>(comments: impl IntoIterator<Item = &'a Comment>) -> Option<usize> {
    let mut count = 0;
    for comment in comments {
        if !is_done(comment) {
            return None;
        }
        count += 1;
    }
    (count > 0).then_some(count)
}

/// Files whose done comments are shown instead of folded, as `repo/path`.
pub type Unfolded = Rc<std::cell::RefCell<std::collections::BTreeSet<String>>>;

/// The one line a file's done comments fold into, with Show (or Hide) to
/// unfold them.
pub fn fold_line(look: Look, key: &str, count: usize, open: bool, unfolded: &Unfolded) -> impl IntoElement {
    let key = key.to_owned();
    let cell = unfolded.clone();
    let noun = if count == 1 { "comment" } else { "comments" };
    let tones = done_tones(look);
    shell(look, false)
        .bg(rgb(tones.card))
        .py(px(7.))
        .px(px(12.))
        .id(SharedString::from(format!("doc-done-fold {key}")))
        .test_support()
        .child(
            row()
                .gap(px(8.))
                .child(text("✓").text_color(rgb(tones.badge_text)))
                .child(text(format!("{count} {noun} done on this file")).text_color(rgb(tones.body)))
                .child(
                    link(
                        look,
                        SharedString::from(format!("doc-done-toggle {key}")),
                        if open { "Hide" } else { "Show" },
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        {
                            let mut set = cell.borrow_mut();
                            if !set.remove(&key) {
                                set.insert(key.clone());
                            }
                        }
                        cx.refresh_windows();
                    }),
                ),
        )
}

/// The done states' colours (UXSIMPLE 4-2, 4-3): a quiet card, a green badge
/// and a blue link.
struct DoneTones {
    card: u32,
    body: u32,
    badge_text: u32,
    badge_fill: u32,
    badge_line: u32,
}

fn done_tones(look: Look) -> DoneTones {
    if look.panel < 0x80_80_80 {
        DoneTones {
            card: 0x242426,
            body: look.secondary,
            badge_text: 0x6f_d0_8c,
            badge_fill: 0x17_30_1f,
            badge_line: 0x2c_5a_3a,
        }
    } else {
        DoneTones {
            card: 0xf7_f7_f9,
            body: 0x6e_6e_73,
            badge_text: 0x1c_7c_36,
            badge_fill: 0xe9_f7_ed,
            badge_line: 0xb7_e0_c2,
        }
    }
}

fn done_chip(look: Look, label: impl Into<SharedString>) -> Div {
    let tones = done_tones(look);
    text(label)
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .border_1()
        .border_color(rgb(tones.badge_line))
        .bg(rgb(tones.badge_fill))
        .text_size(px(12.))
        .text_color(rgb(tones.badge_text))
}

/// A blue text link.
fn link(look: Look, id: SharedString, label: &'static str) -> ObservedElement<Stateful<Div>> {
    text(label)
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .test_support()
        .text_color(rgb(look.accent))
        .cursor_pointer()
}

fn shell(look: Look, open: bool) -> Div {
    col()
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .gap(px(4.))
        // DS-15: cards are at most 720 pt wide.
        .max_w(px(720.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(if open { look.accent } else { look.line }))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .text_color(rgb(look.text))
}

fn chip(look: Look, label: impl Into<SharedString>) -> Div {
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
///
/// An agent's note is the one agent card (UXSIMPLE 3-4): Send to the
/// session's agent, Edit and Drop, and in a pull request `review` also Draft
/// for GitHub.
pub fn card(
    ui: &Rc<Ui>,
    look: Look,
    comment: &Comment,
    agent: &str,
    review: bool,
    reattaching: bool,
    start_reattach: &StartReattach,
) -> AnyElement {
    if is_agent_note(comment) && !comment.anchor.detached {
        return agent_note(ui, look, comment, agent, review).into_any_element();
    }
    plain_card(ui, look, comment, agent, reattaching, start_reattach).into_any_element()
}

/// Whether the card is an agent's note, which has its own card.
fn is_agent_note(comment: &Comment) -> bool {
    humata2_review::comments::is_agent_note(comment)
}

/// The orange of a note (UXSIMPLE 3-4).
const NOTE_ORANGE: u32 = 0xd98200;

/// A small blue-text action of the note card.
fn note_action(
    ui: &Rc<Ui>,
    id: String,
    label: String,
    action: AppAction,
    color: u32,
    strong: bool,
    faded: bool,
) -> impl IntoElement {
    let ui = ui.clone();
    text(label.clone())
        .when(strong, |t| t.font_weight(FontWeight::SEMIBOLD))
        .when(faded, |t| t.opacity(0.7))
        .id(SharedString::from(id))
        .role(Role::Button)
        .aria_label(label)
        .test_support()
        .text_color(rgb(color))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(action.clone(), cx);
        })
}

/// What an agent wrote on a line, as one card (UXSIMPLE 3-4): "copilot
/// [Note] line 88", the text, then Send to claude, Edit and Drop.
fn agent_note(ui: &Rc<Ui>, look: Look, comment: &Comment, agent: &str, review: bool) -> impl IntoElement {
    let id = comment.id.clone();
    let session = comment.session.clone();
    let dark = look.panel < 0x80_80_80;
    let (fill, edge) = if dark { (look.fill, look.line) } else { (0xff_f6_e3, 0xf0_d4_9a) };
    let who = match &comment.author {
        Author::Agent(name) => name.clone(),
        Author::User => "You".to_owned(),
    };
    let github = review.then(|| {
        note_action(
            ui,
            format!("doc-make-draft-{id}"),
            "Draft for GitHub".to_owned(),
            AppAction::Comments(CommentsAction::MakeDraft(session.clone(), id.clone(), None)),
            look.accent,
            false,
            false,
        )
    });
    shell(look, false)
        .id(SharedString::from(format!("doc-comment-{id}")))
        .test_support()
        .gap(px(0.))
        .line_height(relative(1.4))
        .child(
            row()
                .gap(px(8.))
                .mb(px(5.))
                .whitespace_nowrap()
                .child(text(who).font_weight(FontWeight::SEMIBOLD).flex_shrink_0())
                .child(
                    text("Note")
                        .flex_shrink_0()
                        .px(px(5.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(edge))
                        .bg(rgb(fill))
                        .text_size(px(10.5))
                        .line_height(px(12.))
                        .text_color(rgb(if dark { 0xff_9f_0a } else { NOTE_ORANGE })),
                )
                .child(text(lines_label(&comment.anchor)).text_color(rgb(look.term_dim)).flex_shrink_0()),
        )
        .child(text(comment.text.clone()))
        .child(
            row()
                .gap(px(16.))
                .mt(px(8.))
                .child(note_action(
                    ui,
                    format!("doc-send-note-{id}"),
                    format!("Send to {agent}"),
                    AppAction::Comments(CommentsAction::SendNote(session.clone(), id.clone())),
                    look.accent,
                    true,
                    false,
                ))
                .child(note_action(
                    ui,
                    format!("doc-edit-comment-{id}"),
                    "Edit".to_owned(),
                    AppAction::Comments(CommentsAction::EditDraft(session.clone(), id.clone())),
                    look.accent,
                    false,
                    false,
                ))
                .children(github)
                .child(note_action(
                    ui,
                    format!("doc-drop-comment-{id}"),
                    "Drop".to_owned(),
                    AppAction::Comments(CommentsAction::Drop(session.clone(), id.clone())),
                    look.accent,
                    false,
                    true,
                )),
        )
}

fn plain_card(
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
    let is_note = comment.destination == Destination::Note;
    let for_github = matches!(comment.destination, Destination::PullRequest { .. });
    // A draft for GitHub is titled by what it is and who wrote it, with
    // Edit and Drop under its text (frame 68).
    let github_draft = for_github && comment.status == Status::Draft && !comment.archived;
    let state = card_state(comment, agent);
    let label = if github_draft { Some("Draft".to_owned()) } else { state.chip.clone() };
    let who = match &comment.author {
        Author::User => "You",
        Author::Agent(name) => name.as_str(),
    };
    let title = if github_draft {
        format!("Draft for GitHub · by {}", agent_label(&comment.author))
    } else {
        format!("{who} · {lines}")
    };
    let open = matches!(comment.status, Status::Draft | Status::Waiting);
    // The agent's draft for GitHub takes the accent outline; the person's
    // keeps the grey line.
    let outlined = if github_draft {
        matches!(comment.author, Author::Agent(_))
    } else {
        open
    };
    // A done card is quiet: "You [Done by claude] line 43 … Reopen".
    let quiet = state.done && !github_draft;
    let tones = done_tones(look);
    shell(look, outlined)
        .when(quiet, |card| card.bg(rgb(tones.card)).text_color(rgb(tones.body)))
        .id(SharedString::from(format!("doc-comment-{id}")))
        .test_support()
        .child(
            row()
                .gap(px(if quiet { 8. } else { 6. }))
                .child(text(if quiet { who.to_owned() } else { title }).font_weight(FontWeight::SEMIBOLD))
                .children(label.map(|label| if quiet { done_chip(look, label) } else { chip(look, label) }))
                .children(quiet.then(|| text(lines.clone()).text_color(rgb(look.secondary))))
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
                .children(is_note.then(|| {
                    widgets::ghost_in(look,
                        ui,
                        SharedString::from(format!("doc-make-draft-{id}")),
                        "Make it a draft",
                        AppAction::Comments(CommentsAction::MakeDraft(session.clone(), id.clone(), None)),
                    )
                    .h(px(20.))
                    .px(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(look.accent))
                }))
                .children((state.reopen && !github_draft).then(|| {
                    let ui = ui.clone();
                    let action = AppAction::Comments(CommentsAction::Reopen(session.clone(), id.clone()));
                    link(look, SharedString::from(format!("doc-reopen-comment-{id}")), "Reopen")
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            ui.send(action.clone(), cx);
                        })
                })),
        )
        .child(text(comment.text.clone()))
        .children(state.done.then(|| agent_reply(comment)).flatten().map(|(who, said)| {
            row()
                .gap(px(8.))
                .mt(px(4.))
                .pt(px(8.))
                .border_t_1()
                .border_color(rgb(look.line))
                .items_start()
                .child(text(who.to_owned()).font_weight(FontWeight::SEMIBOLD).flex_shrink_0())
                .child(text(said.to_owned()).flex_1().min_w_0())
        }))
        .children(github_draft.then(|| {
            row()
                .gap(px(6.))
                .child(widgets::push_quiet(
                    ui,
                    SharedString::from(format!("doc-edit-comment-{id}")),
                    "Edit",
                    AppAction::Comments(CommentsAction::EditDraft(session.clone(), id.clone())),
                    look,
                ))
                // A detached card has its own Drop below.
                .children((!comment.anchor.detached).then(|| {
                    widgets::push_quiet(
                        ui,
                        SharedString::from(format!("doc-drop-comment-{id}")),
                        "Drop",
                        AppAction::Comments(CommentsAction::Drop(session.clone(), id.clone())),
                        look,
                    )
                }))
        }))
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
/// does, then Cancel, Ask and Draft for GitHub in a pull request review, and
/// Comment. `input` is the text field the holder keeps
/// in step with the box.
#[allow(clippy::too_many_arguments)]
pub fn open_box(
    ui: &Rc<Ui>,
    look: Look,
    session: &SessionId,
    anchor: &Anchor,
    input: &Entity<TextareaState>,
    agent: &str,
    question: &str,
    offers: &BoxOffers,
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
                .children(offers.review.then(|| {
                    widgets::push_quiet(
                        ui,
                        "doc-comment-ask",
                        "Ask",
                        AppAction::PrAsk(AskAction::Ask {
                            session: session.clone(),
                            anchor: anchor.clone(),
                            question: question.to_owned(),
                        }),
                        look,
                    )
                }))
                .children(offers.github.clone().map(|(repo, number)| {
                    widgets::push_quiet(
                        ui,
                        "doc-comment-github",
                        "Draft for GitHub",
                        AppAction::Comments(CommentsAction::DraftForGitHub(session.clone(), repo, number)),
                        look,
                    )
                }))
                .child(widgets::push_primary(
                    ui,
                    "doc-comment-send",
                    "Comment",
                    AppAction::Comments(CommentsAction::Comment(session.clone())),
                    look,
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;
    use humata2_clients::comments::ThreadReply;

    fn comment(status: Status) -> Comment {
        Comment {
            id: CommentId::new("s/c-1"),
            session: SessionId::new("s"),
            anchor: Anchor {
                repo: "api".into(),
                path: "src/a.rs".into(),
                side: humata2_clients::comments::Side::New,
                first: 3,
                last: 3,
                quoted: Vec::new(),
                base: None,
                detached: false,
                outdated: false,
            },
            text: "Fix".into(),
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

    #[test]
    fn draft_and_waiting_are_one_draft_state() {
        for status in [Status::Draft, Status::Waiting] {
            let state = card_state(&comment(status), "claude");
            assert_eq!(state.chip.as_deref(), Some("Draft"));
            assert!(!state.done && !state.reopen);
        }
    }

    #[test]
    fn a_sent_comment_says_sent_and_offers_nothing() {
        let state = card_state(&comment(Status::Sent), "claude");
        assert_eq!(state.chip.as_deref(), Some("Sent"));
        assert!(!state.done && !state.reopen);
    }

    #[test]
    fn an_addressed_comment_is_done_by_the_agent_and_can_reopen() {
        let state = card_state(&comment(Status::Addressed), "claude");
        assert_eq!(state.chip.as_deref(), Some("Done by claude"));
        assert!(state.done && state.reopen);
    }

    #[test]
    fn a_resolved_comment_is_done_by_you_and_can_reopen() {
        let state = card_state(&comment(Status::Resolved), "claude");
        assert_eq!(state.chip.as_deref(), Some("Done by you"));
        assert!(state.done && state.reopen);
    }

    #[test]
    fn the_agents_last_reply_is_found_and_the_users_is_not() {
        let mut c = comment(Status::Addressed);
        c.replies = vec![
            ThreadReply { id: "1".into(), author: Author::Agent("claude".into()), text: "done".into() },
            ThreadReply { id: "2".into(), author: Author::User, text: "thanks".into() },
        ];
        assert_eq!(agent_reply(&c), Some(("claude", "done")));
        assert_eq!(agent_reply(&comment(Status::Sent)), None);
    }

    #[test]
    fn a_file_folds_only_when_every_comment_is_done() {
        let done = [comment(Status::Addressed), comment(Status::Resolved)];
        assert_eq!(all_done(&done), Some(2));
        let mixed = [comment(Status::Addressed), comment(Status::Draft)];
        assert_eq!(all_done(&mixed), None);
        assert_eq!(all_done(&[]), None);
    }
}
