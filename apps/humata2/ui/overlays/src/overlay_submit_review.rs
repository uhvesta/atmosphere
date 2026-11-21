//! The Submit review dialog (PR-9, PR-12 to PR-14, PR-16; frame 69).
//!
//! It draws [`SubmitState`] and sends [`SubmitAction`]s: the drafts of each
//! pull request with Edit and Drop, a verdict for each, the agent's summary,
//! and one Submit that posts one review per pull request. A pull request
//! whose post failed shows its error on its own card (PR-13) and keeps its
//! drafts; the ones that posted show their link.
//!
//! This file is a child module of `overlay_pr_review.rs`.

use super::super::{Overlays, sheet};
use super::{alert, card, secondary};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::chrome::runs;
use humata2_ui_core::theme::{self, Look};
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::comments::{Anchor, Author, CommentId};
use humata2_clients::model::Agent;
use humata2_clients::pull_requests::{Draft, LeftOut, LeftOutReason, PullRequestRef, Verdict};
use humata2_pr_review::submit::{Outcome, PullRequestReview, Step, SubmitAction, SubmitState};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What the dialog keeps that the state doesn't: the draft being edited and
/// its field, and a summary field for each pull request that has one.
#[derive(Default)]
pub(super) struct Fields {
    /// The draft whose text is being edited.
    editing: RefCell<Option<CommentId>>,
    /// Whether the edit field already holds that draft's text.
    edit_filled: Cell<bool>,
    edit: RefCell<Option<Entity<InputState>>>,
    summaries: RefCell<HashMap<PullRequestRef, Entity<TextareaState>>>,
    /// Pull requests whose summary box shows although the summary is empty.
    summary_open: RefCell<HashSet<PullRequestRef>>,
    subscriptions: RefCell<Vec<Subscription>>,
}

impl Fields {
    /// Clears any in-progress draft edit when the dialog opens.
    pub(super) fn reset(&self) {
        *self.editing.borrow_mut() = None;
        self.edit_filled.set(false);
    }
}

fn submit(action: SubmitAction) -> AppAction {
    AppAction::PrSubmit(action)
}

/// The words for an agent author, such as "Claude Code".
fn agent_name(name: &str) -> String {
    Agent::ALL
        .into_iter()
        .find(|agent| agent.command() == name)
        .map_or_else(|| name.to_owned(), |agent| runs(agent).to_owned())
}

fn who(author: &Author) -> String {
    match author {
        Author::User => "You".to_owned(),
        Author::Agent(name) => agent_name(name),
    }
}

/// `refund.rs:140`, or `refund.rs:140-146` for a range.
fn place(draft: &Draft) -> String {
    let file = draft
        .anchor
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&draft.anchor.path);
    if draft.anchor.last > draft.anchor.first {
        format!("{file}:{}-{}", draft.anchor.first, draft.anchor.last)
    } else {
        format!("{file}:{}", draft.anchor.first)
    }
}

fn count(n: usize, one: &'static str, many: &'static str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn verdict_id(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Comment => "comment",
        Verdict::Approve => "approve",
        Verdict::RequestChanges => "request-changes",
    }
}

impl Overlays {
    /// The summary field of `pr`, made the first time it is asked for.
    fn summary_field(
        &self,
        pr: &PullRequestRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        let fields = &self.pr.submit;
        if let Some(field) = fields.summaries.borrow().get(pr) {
            return field.clone();
        }
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What should the author know before reading the comments?")
                .auto_grow(2, 8)
        });
        let for_pr = pr.clone();
        fields.subscriptions.borrow_mut().push(cx.subscribe(
            &field,
            move |this: &mut Overlays, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let summary = input.read(cx).value().to_string();
                    this.ui.send(
                        submit(SubmitAction::SetSummary {
                            pr: for_pr.clone(),
                            summary,
                        }),
                        cx,
                    );
                }
                InputEvent::PressEnter { secondary: true, .. } => {
                    this.ui.send(submit(SubmitAction::Submit), cx);
                }
                _ => {}
            },
        ));
        fields.summaries.borrow_mut().insert(pr.clone(), field.clone());
        field
    }

    /// The draft-edit field, made the first time a draft is edited.
    fn edit_field(&self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let fields = &self.pr.submit;
        if let Some(field) = fields.edit.borrow().as_ref() {
            return field.clone();
        }
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("What the comment says"));
        fields.subscriptions.borrow_mut().push(cx.subscribe(
            &field,
            |this: &mut Overlays, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_edit(cx);
                }
            },
        ));
        *fields.edit.borrow_mut() = Some(field.clone());
        field
    }

    /// Sends the edited text of the draft being edited and stops editing.
    fn save_edit(&self, cx: &mut Context<Self>) {
        let fields = &self.pr.submit;
        let id = fields.editing.borrow_mut().take();
        let field = fields.edit.borrow().clone();
        if let (Some(id), Some(field)) = (id, field) {
            let text = field.read(cx).value().to_string();
            self.ui.send(submit(SubmitAction::EditDraft { id, text }), cx);
        }
        cx.notify();
    }

    /// The Submit review dialog.
    pub(super) fn submit_review(
        &self,
        state: &SubmitState,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The machine whose gh posts: the review session's machine, else the
        // one open.
        let gh = ui.read(|app| {
            let session = state
                .session
                .as_ref()
                .and_then(|id| app.workspace.session(id))
                .or_else(|| app.session.open.as_ref().and_then(|id| app.workspace.session(id)));
            session
                .and_then(|session| app.workspace.machine(&session.machine))
                .map(|machine| format!("{}'s gh", machine.name))
        });
        let gh = gh.unwrap_or_else(|| "the machine's gh".to_owned());

        let mut body = sheet("submit-review-dialog", "Submit review", look)
            .w(px(700.))
            .max_w_full()
            .max_h(relative(0.94))
            .gap(px(11.))
            .p(px(20.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .test_support()
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Submit review"),
                    )
                    .child(secondary(look, count(state.reviews.len(), "pull request", "pull requests"))),
            );
        if state.step == Step::Planning {
            body = body.child(
                div()
                    .id("submit-review-planning")
                    .role(Role::Status)
                    .aria_label("Working out what would post")
                    .test_support()
                    .text_color(rgb(look.secondary))
                    .child("Working out what would post\u{2026}"),
            );
        }
        // The edit field takes its text the moment editing starts.
        if self.pr.submit.editing.borrow().is_some() {
            let field = self.edit_field(window, cx);
            if !self.pr.submit.edit_filled.get() {
                let text = self
                    .pr
                    .submit
                    .editing
                    .borrow()
                    .as_ref()
                    .and_then(|id| {
                        state.reviews.iter().find_map(|review| {
                            review
                                .drafts
                                .iter()
                                .chain(review.left_out.iter().map(|left| &left.draft))
                                .find(|draft| &draft.id == id)
                                .map(|draft| draft.text.clone())
                        })
                    })
                    .unwrap_or_default();
                field.update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                    input.focus(window, cx);
                });
                self.pr.submit.edit_filled.set(true);
            }
        }
        for review in &state.reviews {
            body = body.child(self.pr_section(review, look, ui, cx));
        }
        // The summaries: one box for each pull request that has one.
        let mut summaries = col().gap(px(8.));
        let shown: Vec<&PullRequestReview> = state
            .reviews
            .iter()
            .filter(|review| self.shows_summary(review))
            .collect();
        for review in &shown {
            let field = self.summary_field(&review.pr, window, cx);
            super::sync_area(&field, &review.summary, window, cx);
            let by = review
                .drafts
                .iter()
                .find_map(|draft| match &draft.author {
                    Author::Agent(name) => Some(agent_name(name)),
                    Author::User => None,
                });
            let title = match (&by, state.reviews.len()) {
                (Some(by), 1) => format!("Summary \u{b7} drafted by {by}"),
                (Some(by), _) => format!("Summary for {} #{} \u{b7} drafted by {by}", review.pr.repo, review.pr.number),
                (None, 1) => "Summary".to_owned(),
                (None, _) => format!("Summary for {} #{}", review.pr.repo, review.pr.number),
            };
            summaries = summaries.child(
                col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(look.secondary))
                            .child(title),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("submit-summary-{}", review.pr)))
                            .role(Role::Group)
                            .aria_label(format!("Summary for {} #{}", review.pr.repo, review.pr.number))
                            .test_support()
                            // Frame 69: 8 around the text (XSmall adds the
                            // other 4 on each side) and the panel's line
                            // height, as the design's summary box.
                            .px(px(4.))
                            .py(px(8.))
                            .rounded(px(6.))
                            .bg(rgb(look.card))
                            .border_1()
                            .border_color(rgb(look.accent))
                            .child(
                                Textarea::new(&field)
                                    .xsmall()
                                    .appearance(false)
                                    .bordered(false)
                                    .text_size(px(12.))
                                    .line_height(phi())
                                    .aria_label(format!("Summary for {} #{}", review.pr.repo, review.pr.number)),
                            ),
                    ),
            );
        }
        if !shown.is_empty() {
            body = body.child(summaries);
        }
        if let Some(error) = &state.error {
            body = body.child(alert("submit-review-error", error.to_string()));
        }
        let posting = state.reviews.iter().filter(|review| review.has_something_to_post()).count();
        let label = match state.step {
            Step::Posting => "Posting\u{2026}".to_owned(),
            _ => match posting {
                0 | 1 => "Submit review".to_owned(),
                n => format!("Submit {n} reviews"),
            },
        };
        body.child(
            row()
                .gap(px(8.))
                .pt(px(2.))
                .child(
                    div()
                        .flex_1()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_size(px(11.))
                        .text_color(rgb(look.tertiary))
                        .child(format!("Posts one review per PR with {gh}")),
                )
                .child(widgets::push_quiet(
                    ui,
                    "submit-review-cancel",
                    "Cancel",
                    submit(SubmitAction::Close),
                    look,
                ))
                .child(
                    widgets::push_primary(ui, "submit-review-submit", label, submit(SubmitAction::Submit), look)
                        .disabled(!state.can_submit()),
                ),
        )
        .focus_trap("submit-review-dialog", &self.sheet)
        .into_any_element()
    }

    /// Whether `review` shows its summary box: it has a summary, or the
    /// person asked to write one.
    fn shows_summary(&self, review: &PullRequestReview) -> bool {
        !review.summary.trim().is_empty() || self.pr.submit.summary_open.borrow().contains(&review.pr)
    }

    /// One pull request's card: its drafts, what couldn't be posted, its
    /// verdict, and how its last post went.
    fn pr_section(
        &self,
        review: &PullRequestReview,
        look: Look,
        ui: &Rc<Ui>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pr = &review.pr;
        let mut section = card(look)
            .id(SharedString::from(format!("submit-pr-{pr}")))
            .role(Role::Group)
            .aria_label(format!("{} #{}", pr.repo, pr.number))
            .test_support()
            .p(px(10.))
            .gap(px(2.))
            .child(
                row()
                    .gap(px(7.))
                    .pb(px(4.))
                    .child(chrome::icon(IconName::GitPullRequest, look.accent, 12.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} #{}", pr.repo, pr.number)),
                    )
                    .child(secondary(look, count(review.drafts.len(), "draft", "drafts"))),
            );
        if review.drafts.is_empty() {
            section = section.child(secondary(look, "No drafts on this pull request."));
        }
        for draft in &review.drafts {
            section = section.child(self.draft_row(draft, None, look, ui, cx));
        }
        for left in &review.left_out {
            section = section.child(self.draft_row(&left.draft, Some(left), look, ui, cx));
        }
        match &review.outcome {
            Some(Outcome::Failed(error)) => {
                section = section.child(alert(
                    SharedString::from(format!("submit-error-{pr}")),
                    error.to_string(),
                ));
            }
            Some(Outcome::Posted(url)) => {
                section = section.child(
                    row()
                        .id(SharedString::from(format!("submit-posted-{pr}")))
                        .role(Role::Status)
                        .aria_label(format!("Posted to {} #{}", pr.repo, pr.number))
                        .test_support()
                        .gap(px(5.))
                        .child(chrome::icon(IconName::Check, theme::GREEN, 12.))
                        .child(secondary(look, format!("Posted: {url}"))),
                );
            }
            None => {}
        }
        let summary_box_shown = self.shows_summary(review);
        let for_pr = pr.clone();
        section
            .child(div().h(px(1.)).w_full().flex_shrink_0().bg(rgb(look.line)))
            .child(
                row()
                    .gap(px(10.))
                    .pt(px(6.))
                    .child(div().text_color(rgb(look.secondary)).child("Verdict"))
                    .child(self.verdicts(review, look, ui))
                    .child(div().flex_1())
                    .when(!summary_box_shown, |line| {
                        line.child(
                            widgets::push_quiet_shape(
                                SharedString::from(format!("submit-add-summary-{pr}")),
                                "Add a summary",
                                look,
                            )
                            .on_click(cx.listener(move |this: &mut Overlays, _: &ClickEvent, _, cx| {
                                this.pr.submit.summary_open.borrow_mut().insert(for_pr.clone());
                                cx.notify();
                            })),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The verdict choice of one pull request.
    fn verdicts(&self, review: &PullRequestReview, look: Look, ui: &Rc<Ui>) -> impl IntoElement + use<> {
        let pr = review.pr.clone();
        let chosen = review.verdict;
        row()
            .flex_shrink_0()
            .p(px(2.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .children(
                [
                    (Verdict::Comment, "Comment"),
                    (Verdict::Approve, "Approve"),
                    (Verdict::RequestChanges, "Request changes"),
                ]
                .into_iter()
                .map(|(verdict, text)| {
                    let ui = ui.clone();
                    let action = submit(SubmitAction::SetVerdict {
                        pr: pr.clone(),
                        verdict,
                    });
                    div()
                        .id(SharedString::from(format!("submit-verdict-{}-{}", pr, verdict_id(verdict))))
                        .role(Role::Tab)
                        .aria_label(text)
                        .aria_selected(chosen == verdict)
                        .test_support()
                        .px(px(12.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .text_size(px(11.5))
                        .when(chosen == verdict, |segment| segment.bg(rgb(look.card)).shadow_sm())
                        .cursor_pointer()
                        .child(text)
                        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
                }),
            )
    }

    /// One draft: where, what, who, and Edit and Drop. A draft the plan left
    /// out says why and offers only Drop.
    fn draft_row(
        &self,
        draft: &Draft,
        left_out: Option<&LeftOut>,
        look: Look,
        ui: &Rc<Ui>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editing = self.pr.submit.editing.borrow().as_ref() == Some(&draft.id);
        let id = draft.id.clone();
        let text = if editing {
            let field = self.pr.submit.edit.borrow().clone();
            match field {
                Some(field) => div()
                    .id(SharedString::from(format!("submit-draft-field-{}", draft.id)))
                    .role(Role::Group)
                    .aria_label("Comment text")
                    .test_support()
                    .h(px(26.))
                    .child(
                        Input::new(&field)
                            .small()
                            .min_h(px(26.))
                            .text_size(px(12.))
                            .focus_bordered(false)
                            .aria_label("Comment text"),
                    )
                    .into_any_element(),
                None => div().into_any_element(),
            }
        } else {
            div().text_size(px(12.)).child(draft.text.clone()).into_any_element()
        };
        let reason = left_out.map(|left| match left.reason {
            LeftOutReason::Outdated => {
                "Not posted: its lines changed since it was written. Drop it, or point it at the new lines in the diff."
            }
            LeftOutReason::OutOfDiff => {
                "Not posted: its line isn't in this pull request's diff, so GitHub would refuse it. Drop it, or point it at a line in the diff."
            }
        });
        let edit_button = if editing {
            widgets::push_primary_shape(
                SharedString::from(format!("submit-draft-save-{}", draft.id)),
                "Save",
                look,
            )
            .on_click(cx.listener(|this: &mut Overlays, _: &ClickEvent, _, cx| this.save_edit(cx)))
        } else {
            let draft_id = id.clone();
            widgets::push_quiet_shape(SharedString::from(format!("submit-draft-edit-{}", draft.id)), "Edit", look)
                .on_click(cx.listener(move |this: &mut Overlays, _: &ClickEvent, _, cx| {
                    *this.pr.submit.editing.borrow_mut() = Some(draft_id.clone());
                    this.pr.submit.edit_filled.set(false);
                    cx.notify();
                }))
        };
        row()
            .id(SharedString::from(format!("submit-draft-{}", draft.id)))
            .role(Role::Group)
            .aria_label(format!("Draft at {}", place(draft)))
            .test_support()
            .items_start()
            .gap(px(8.))
            .py(px(6.))
            .child(
                div()
                    .w(px(150.))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .text_color(rgb(look.secondary))
                    .child(place(draft)),
            )
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(text)
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(look.tertiary))
                            .child(who(&draft.author)),
                    )
                    .children(reason.map(|reason| {
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(theme::ORANGE))
                            .child(reason)
                    })),
            )
            .when(left_out.is_none(), |line| line.child(edit_button))
            .children(left_out.filter(|left| left.reason == LeftOutReason::Outdated).map(|_| {
                // Re-attach keeps an outdated draft's own lines and clears the
                // outdated mark: the person confirms them by pressing it (D630).
                // A draft outside the diff offers only Drop here: GitHub would
                // refuse its line again, and Submit review has no line picker.
                let anchor = Anchor {
                    detached: false,
                    outdated: false,
                    ..draft.anchor.clone()
                };
                widgets::push_quiet(
                    ui,
                    SharedString::from(format!("submit-draft-reattach-{}", draft.id)),
                    "Re-attach",
                    submit(SubmitAction::Reattach {
                        id: draft.id.clone(),
                        anchor,
                    }),
                    look,
                )
            }))
            .child(widgets::push_quiet(
                ui,
                SharedString::from(format!("submit-draft-drop-{}", draft.id)),
                "Drop",
                submit(SubmitAction::DropDraft(id)),
                look,
            ))
            .into_any_element()
    }
}
