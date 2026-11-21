//! Drawing the pane. The file list and the body are virtualised: only the
//! rows on screen are built each frame. Every control carries a role and a
//! label, so tests and assistive technology find it by what it says.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::base::TestTextExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, Div, ElementId, FontWeight, ImageSource, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ScrollEvent, Pixels, Role, Rgba, SharedString,
    Stateful, Toggled, Window, canvas, div, img, list, px, uniform_list,
};
use gpui_kit::TestSupportExt as _;

use super::{ReviewPane, Theme};
use crate::images::{DiffSlot, ImageSide};
use crate::markdown::{BlockKind, Change, RenderedBlock};
use crate::model::{
    AgentNote, AgentState, BodyRow, ChangeKind, Comment, CommentId, Content, Delivery,
    DeliveryState, Failure, FileChange, Hold, ImageMode, Kind, LineKind, ListRow, NoteId, Side,
    Status, Switch, Viewed,
};
use super::NoteChange;

/// The largest an image is drawn, in pixels.
const IMAGE_BOX: (f32, f32) = (420.0, 280.0);

impl Render for ReviewPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let root = div()
            .id("review")
            .test_support()
            .role(Role::Group)
            .aria_label("Review")
            .track_focus(&self.focus)
            .key_context("Review")
            .on_key_down(cx.listener(|this, event, window, cx| this.key_down(event, window, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.dragging = false),
            )
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(theme.background)
            .text_color(theme.text)
            .text_size(px(theme.font_size));
        let Some(session) = self.current.clone() else {
            return root.child(message(&theme, "No session is open."));
        };
        if self.loading_current() {
            return root.child(message(&theme, "Loading the review…"));
        }
        if let Some(problem) = self.broken_current() {
            let text = format!(
                "This session's review can't be shown: {problem} Nothing is saved for it until it loads."
            );
            return root.child(message(&theme, text));
        }
        if self.session(&session).is_none() {
            return root.child(message(&theme, "Loading the review…"));
        }
        let preview = self.preview.clone().map(|preview| self.render_preview(preview, cx));
        let prepared = self.render_prepared(cx);
        let agent = self.render_agent(cx);
        root.child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .min_w(px(0.))
                    .children(prepared)
                    .child(self.render_status(cx))
                    .children(self.render_compare(cx))
                    .children(agent)
                    .children(self.render_queue(cx))
                    .children(self.notice.clone().map(|notice| {
                        div()
                            .id("notice")
                            .test_support()
                            .role(Role::Status)
                            .aria_label("Notice")
                            .px_3()
                            .py_1()
                            .bg(theme.panel_raised)
                            .text_color(theme.warning)
                            .child(notice.clone())
                            .test_text(notice)
                    }))
                    .child(
                        div()
                            .id("body")
                            .test_support()
                            .flex_1()
                            .min_h(px(0.))
                            // Scrolling holds the agent's view changes until
                            // the user stops (PR-24).
                            .on_scroll(cx.listener(|this, _: &ScrollEvent, _, cx| this.user_active(cx)))
                            .child(
                                list(
                                    self.list.clone(),
                                    cx.processor(|this, index, window, cx| this.render_row(index, window, cx)),
                                )
                                .size_full(),
                            ),
                    ),
            )
            .children(preview)
    }
}

impl ReviewPane {
    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let Some(review) = self.current_review() else {
            return div();
        };
        let filters = review.memory().filters;
        let files = review.review().files();
        let (added, removed) = files.iter().fold((0, 0), |(a, r), file| {
            let (fa, fr) = file.counts();
            (a + fa, r + fr)
        });
        let viewed = files
            .iter()
            .filter(|file| review.memory().viewed(&file.key, &file.blob) == Viewed::Yes)
            .count();
        let summary: SharedString =
            format!("{} files · +{added} −{removed} · {viewed} viewed", files.len()).into();
        let count = self.list_rows.len();
        div()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.line)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_1()
                    .p_2()
                    .child(chip(
                        "filter-changed",
                        "Changed",
                        "Changed since viewed",
                        Some(filters.changed),
                        theme,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.on_memory(|memory| memory.filters.changed ^= true, cx);
                        }),
                    ))
                    .child(chip(
                        "filter-commented",
                        "Commented",
                        "Commented",
                        Some(filters.commented),
                        theme,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.on_memory(|memory| memory.filters.commented ^= true, cx);
                        }),
                    ))
                    .child(chip(
                        "filter-unviewed",
                        "Not viewed",
                        "Not viewed",
                        Some(filters.unviewed),
                        theme,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.on_memory(|memory| memory.filters.unviewed ^= true, cx);
                        }),
                    )),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_color(theme.dim)
                    .child(summary.clone())
                    .test_text(summary),
            )
            .child(
                div()
                    .id("files-list")
                    .test_support()
                    .role(Role::List)
                    .aria_label("Changed files")
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        uniform_list(
                            "files",
                            count,
                            cx.processor(|this, range: Range<usize>, _window, cx| {
                                this.render_list_rows(range, cx)
                            }),
                        )
                        .track_scroll(&self.files_scroll)
                        .size_full(),
                    ),
            )
    }

    fn render_list_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.note_list_mounted(range.len());
        let theme = self.theme.clone();
        let current = self.cursor.as_ref().map(BodyRow::file);
        let Some(review) = self.current_review() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(range.len());
        for index in range {
            let Some(row) = self.list_rows.get(index) else {
                continue;
            };
            match row {
                ListRow::Repo {
                    repo,
                    files,
                    collapsed,
                } => {
                    let name = repo.clone();
                    let text: SharedString = format!(
                        "{} {repo} ({files})",
                        if *collapsed { "▸" } else { "▾" }
                    )
                    .into();
                    out.push(
                        div()
                            .id(("repo", index))
                            .test_support()
                            .role(Role::Button)
                            .aria_label(format!("Repository {repo}"))
                            .aria_expanded(!collapsed)
                            .h(px(24.))
                            .px_2()
                            .flex()
                            .items_center()
                            .text_color(theme.dim)
                            .font_weight(FontWeight::SEMIBOLD)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let repo = name.clone();
                                this.on_memory(|memory| memory.toggle_repo(&repo), cx);
                            }))
                            .child(div().child(text.clone()).test_text(text))
                            .into_any_element(),
                    );
                }
                ListRow::Group { group, files } => {
                    let title: SharedString = match (group, review.memory().prepared.as_ref()) {
                        (Some(g), Some(showing)) => showing
                            .view
                            .groups
                            .get(*g)
                            .map_or_else(|| "Other changes".to_owned(), |group| group.title.clone())
                            .into(),
                        _ => "Other changes".into(),
                    };
                    let text: SharedString = format!("{title} ({files})").into();
                    out.push(
                        div()
                            .id(("list-group", index))
                            .test_support()
                            .role(Role::Heading)
                            .aria_label(title)
                            .aria_level(2)
                            .h(px(24.))
                            .px_2()
                            .flex()
                            .items_center()
                            .text_color(theme.accent)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().child(text.clone()).test_text(text))
                            .into_any_element(),
                    );
                }
                &ListRow::File(file) => {
                    let Some(change) = review.review().files().get(file) else {
                        continue;
                    };
                    let viewed = review.memory().viewed(&change.key, &change.blob);
                    let comments = review
                        .book()
                        .on_file(&change.key)
                        .filter(|c| c.status != Status::Resolved)
                        .count();
                    let (added, removed) = change.counts();
                    let mark = match viewed {
                        Viewed::Yes => "✓",
                        Viewed::ChangedSince => "●",
                        Viewed::No => " ",
                    };
                    let name: SharedString = change.key.path.clone().into();
                    let counts: SharedString = counts_text(change, added, removed).into();
                    let description = match viewed {
                        Viewed::Yes => "viewed",
                        Viewed::ChangedSince => "changed since viewed",
                        Viewed::No => "not viewed",
                    };
                    out.push(
                        div()
                            .id(("file", file))
                            .test_support()
                            .role(Role::ListItem)
                            .aria_label(change.key.to_string())
                            .aria_description(description)
                            .aria_selected(current == Some(file))
                            .h(px(24.))
                            .pl_4()
                            .pr_2()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .when(current == Some(file), |row| row.bg(theme.selected_background))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.anchor = None;
                                this.go_to(BodyRow::File(file), cx);
                            }))
                            .child(
                                div()
                                    .w(px(12.))
                                    .text_color(if viewed == Viewed::ChangedSince {
                                        theme.warning
                                    } else {
                                        theme.added
                                    })
                                    .child(mark),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .text_color(if viewed == Viewed::Yes { theme.faint } else { theme.text })
                                    .child(name.clone())
                                    .test_text(name),
                            )
                            .when(comments > 0, |row| {
                                let text: SharedString = format!("💬{comments}").into();
                                row.child(div().text_color(theme.accent).child(text.clone()).test_text(text))
                            })
                            .child(div().text_color(theme.dim).child(counts.clone()).test_text(counts))
                            .into_any_element(),
                    );
                }
            }
        }
        out
    }

    fn render_status(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let Some(review) = self.current_review() else {
            return div();
        };
        let pr = self.pr_review();
        let agent = agent_text(review.book().agent(), pr);
        let drafts = review.book().drafts().len();
        let unsent = review.book().unsent();
        let saving: SharedString = match (self.save_failure_current(), self.unsaved_current()) {
            (Some(error), _) => format!("Not saved: {error}").into(),
            (None, Some(true)) => "Saving…".into(),
            _ => "Saved".into(),
        };
        let failed = self.save_failure_current().is_some();
        let counts: SharedString = if pr {
            format!("{unsent} not with the review chat yet").into()
        } else {
            format!("{unsent} not with the agent yet").into()
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(32.))
            .border_b_1()
            .border_color(theme.line)
            .bg(theme.panel)
            .child(
                div()
                    .id("agent")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(if pr { "Review chat" } else { "Agent" })
                    .aria_value(agent)
                    .flex_1()
                    .text_color(theme.dim)
                    .child(agent)
                    .test_text(agent),
            )
            .child(div().text_color(theme.dim).child(counts.clone()).test_text(counts))
            .child(
                div()
                    .id("saving")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Saved")
                    .aria_value(saving.clone())
                    .text_color(if failed { theme.removed } else { theme.faint })
                    .child(saving.clone())
                    .test_text(saving),
            )
            .when(failed, |row| {
                row.child(chip(
                    "save-again",
                    "Save again",
                    "Save again",
                    None,
                    theme,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.save_again(cx)),
                ))
            })
            .when(drafts > 0, |row| {
                row.child(chip(
                    "send-all",
                    format!("Send all drafts ({drafts})"),
                    "Send all drafts",
                    None,
                    theme,
                    cx.listener(|this, _: &ClickEvent, _, cx| this.open_preview(None, cx)),
                ))
            })
    }

    /// The review chat's bar (PR-24, PR-25): the "Agent may" switches,
    /// Back, and the agent's changes, newest first, each with Undo. Shown in
    /// a PR review, and wherever the agent changed the view.
    fn render_agent(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let pr = self.pr_review();
        let review = self.current_review()?;
        let agent = &review.memory().agent;
        let changes: Vec<(String, String)> = agent
            .changes
            .iter()
            .rev()
            .filter(|change| !change.undone)
            .take(3)
            .map(|change| (change.id.clone(), change.summary.clone()))
            .collect();
        let back = agent.back.is_some();
        if !pr && changes.is_empty() && !back {
            return None;
        }
        let may = agent.may;
        let mut row = div()
            .id("agent-bar")
            .test_support()
            .role(Role::Group)
            .aria_label("Agent may")
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(theme.line)
            .bg(theme.panel)
            .child(div().text_color(theme.dim).child("Agent may:").test_text("Agent may:"));
        for switch in [Switch::Focus, Switch::Notes, Switch::Order] {
            let label: SharedString = format!("Agent may {switch}").into();
            row = row.child(chip(
                SharedString::from(format!("agent-may-{switch}")),
                switch.to_string(),
                label,
                Some(may.allows(switch)),
                &theme,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.flip_agent_may(switch, cx)),
            ));
        }
        if back {
            row = row.child(chip(
                "agent-back",
                "Back",
                "Back",
                None,
                &theme,
                cx.listener(|this, _: &ClickEvent, _, cx| this.back(cx)),
            ));
        }
        for (id, summary) in changes {
            let text: SharedString = format!("● {summary}").into();
            row = row.child(div().text_color(theme.faint).child(text.clone()).test_text(text));
            let change = id.clone();
            row = row.child(chip(
                SharedString::from(format!("agent-undo-{id}")),
                "Undo",
                format!("Undo {summary}"),
                None,
                &theme,
                cx.listener(move |this, _: &ClickEvent, _, cx| this.undo_current(&change, cx)),
            ));
        }
        Some(row.into_any_element())
    }

    /// The compare bar (RV-28–RV-30): what the review is against, the
    /// picker of reviews to compare with, and Mark reviewed.
    fn render_compare(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let session = self.current.clone()?;
        self.history.as_ref()?;
        let review = self.current_review()?;
        let reviewed = self.reviewed.get(&session).cloned().unwrap_or_default();
        let base = review.memory().base;
        let since = |number: u32| {
            reviewed
                .iter()
                .find(|r| r.number == number)
                .map_or_else(|| format!("Since review {number}"), |r| format!("Since review {number}, {}", r.when))
        };
        let comparing: SharedString = match base {
            crate::model::Base::Session => "All changes since the session's base".into(),
            crate::model::Base::Since(number) => since(number).into(),
        };
        let menu = self.base_menu.then(|| {
            let mut items = vec![chip(
                "base-session",
                "All changes since the session's base",
                "Compare with the session's base",
                Some(base == crate::model::Base::Session),
                &theme,
                cx.listener(|this, _: &ClickEvent, _, cx| this.choose_base(crate::model::Base::Session, cx)),
            )];
            for r in &reviewed {
                let number = r.number;
                let text = format!("Since review {number}, {}", r.when);
                items.push(chip(
                    ("base-since", number as usize),
                    text.clone(),
                    text,
                    Some(base == crate::model::Base::Since(number)),
                    &theme,
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.choose_base(crate::model::Base::Since(number), cx);
                    }),
                ));
            }
            div()
                .id("bases")
                .test_support()
                .role(Role::Menu)
                .aria_label("Bases")
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .children(items)
        });
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(theme.line)
                .bg(theme.panel)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("comparing")
                                .test_support()
                                .role(Role::Status)
                                .aria_label("Comparing")
                                .aria_value(comparing.clone())
                                .flex_1()
                                .text_color(theme.dim)
                                .child(comparing.clone())
                                .test_text(comparing),
                        )
                        .child(chip(
                            "choose-base",
                            "Compare with…",
                            "Choose base",
                            Some(self.base_menu),
                            &theme,
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.base_menu ^= true;
                                cx.notify();
                            }),
                        ))
                        .child(chip(
                            "mark-reviewed",
                            "Mark reviewed",
                            "Mark reviewed",
                            None,
                            &theme,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.mark_reviewed(cx)),
                        )),
                )
                .children(menu)
                .into_any_element(),
        )
    }

    /// The prepared view's bar (VW-8): whose view this is, with Revert and
    /// Keep, until one is picked.
    fn render_prepared(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let showing = self.current_review()?.memory().prepared.as_ref()?;
        showing.before.as_ref()?;
        let text: SharedString =
            format!("Showing “{}”, prepared by {}", showing.view.title, showing.view.agent).into();
        let summary: SharedString = showing.view.summary.clone().into();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(theme.accent)
                .bg(theme.selected_background)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("prepared")
                                .test_support()
                                .role(Role::Status)
                                .aria_label("Prepared view")
                                .aria_value(text.clone())
                                .flex_1()
                                .child(text.clone())
                                .test_text(text),
                        )
                        .child(chip(
                            "prepared-revert",
                            "Revert",
                            "Revert",
                            None,
                            &theme,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.end_prepared(true, cx)),
                        ))
                        .child(chip(
                            "prepared-keep",
                            "Keep",
                            "Keep",
                            None,
                            &theme,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.end_prepared(false, cx)),
                        )),
                )
                .when(!summary.is_empty(), |bar| {
                    bar.child(div().text_color(theme.dim).child(summary.clone()).test_text(summary))
                })
                .into_any_element(),
        )
    }

    /// The send queue: each delivery not yet with the agent, why it waits
    /// and what can be done about it.
    fn render_queue(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let pr = self.pr_review();
        let review = self.current_review()?;
        let book = review.book();
        let agent = book.agent();
        let list = "Send queue";
        let waiting: Vec<Delivery> = book
            .deliveries()
            .iter()
            .filter(|delivery| delivery.state != DeliveryState::Sent)
            .cloned()
            .collect();
        if waiting.is_empty() {
            return None;
        }
        let rows = waiting.into_iter().map(|delivery| {
            let id = delivery.id;
            let state: SharedString = delivery_text(&delivery, agent, pr).into();
            let mut row = div()
                .id((SharedString::from(format!("delivery-{list}")), usize::try_from(id.0).unwrap_or(usize::MAX)))
                .test_support()
                .role(Role::ListItem)
                .aria_label(format!("Message {id}"))
                .aria_description(state.clone())
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .child(state.clone())
                        .test_text(state),
                );
            match &delivery.state {
                DeliveryState::Queued { hold, send_now } => {
                    if !send_now && matches!(hold, Hold::Busy | Hold::TurnEndUncertain) {
                        row = row.child(chip(
                            (SharedString::from(format!("send-now-{list}")), id.0 as usize),
                            "Send now",
                            format!("Send now {id}"),
                            None,
                            &theme,
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.on_book(|book| book.send_now(id), cx);
                            }),
                        ));
                    }
                    if !delivery.in_doubt {
                        row = row.child(chip(
                            (SharedString::from(format!("cancel-{list}")), id.0 as usize),
                            "Don't send",
                            format!("Cancel send {id}"),
                            None,
                            &theme,
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.on_book(|book| book.cancel(id), cx);
                            }),
                        ));
                    }
                }
                DeliveryState::Failed { .. } => {
                    row = row.child(chip(
                        (SharedString::from(format!("retry-{list}")), id.0 as usize),
                        "Retry",
                        format!("Retry send {id}"),
                        None,
                        &theme,
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.on_book(|book| book.retry(id), cx);
                        }),
                    ));
                }
                DeliveryState::Sending { .. } | DeliveryState::Sent => {}
            }
            row.into_any_element()
        });
        Some(
            div()
                .id(SharedString::from(list))
                .test_support()
                .role(Role::List)
                .aria_label(list)
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(theme.line)
                .bg(theme.panel_raised)
                .children(rows)
                .into_any_element(),
        )
    }

    fn render_preview(&mut self, preview: super::Preview, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme.clone();
        let agent = self
            .current_review()
            .map_or(AgentState::Unknown, |review| review.book().agent());
        let pr = self.pr_review();
        let count = preview.ids.len();
        let who = if pr { "the review chat" } else { "the agent" };
        let title: SharedString = match count {
            1 => format!("Send this to {who}").into(),
            _ => format!("Send these {count} to {who}, as one message").into(),
        };
        let (action, label, note): (Option<&str>, &str, SharedString) = match agent {
            AgentState::Idle if pr => (Some("Send"), "Send", "It's typed into the review chat now.".into()),
            AgentState::Idle => (Some("Send"), "Send", "It's typed into the agent's terminal now.".into()),
            AgentState::Exited if pr => (
                None,
                "",
                "The review chat exited, so nothing can be typed into it. Start it again from the review's terminal, then send again.".into(),
            ),
            AgentState::Working | AgentState::ProbablyIdle if pr => (
                Some("Queue"),
                "Queue",
                "The review chat is answering. It's queued and typed when it's done.".into(),
            ),
            AgentState::Exited => (
                None,
                "",
                "The agent exited, so nothing can be typed into it. Resume the agent in its terminal, then send again.".into(),
            ),
            AgentState::Disconnected => (
                Some("Queue"),
                "Queue",
                "The machine is offline. It's queued and typed when the machine reconnects.".into(),
            ),
            AgentState::Working | AgentState::ProbablyIdle => (
                Some("Queue"),
                "Queue",
                "The agent is busy. It's queued and typed when its turn ends.".into(),
            ),
            AgentState::Unknown => (
                Some("Queue"),
                "Queue",
                "The terminal hasn't said whether the agent is free. It's queued and typed when it is.".into(),
            ),
        };
        let lines = preview.text.lines().map(|line| {
            let line: SharedString = SharedString::from(line.to_owned());
            div().min_h(px(theme.line_height)).child(line.clone()).test_text(line)
        });
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui_kit::rgba(0x00000080))
            .child(
                div()
                    .id("preview")
                    .test_support()
                    .role(Role::Dialog)
                    .aria_label(if pr { "Send to the review chat" } else { "Send to the agent" })
                    .w(px(560.))
                    .max_h(px(520.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.line)
                    .bg(theme.panel)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title.clone())
                            .test_text(title),
                    )
                    .child(
                        div()
                            .id("preview-text")
                            .test_support()
                            .role(Role::Document)
                            .aria_label("Message")
                            .aria_value(preview.text.clone())
                            .overflow_y_scroll()
                            .flex_1()
                            .p_2()
                            .bg(theme.background)
                            .font_family(theme.mono.clone())
                            .children(lines),
                    )
                    .child(
                        div()
                            .id("preview-note")
                            .test_support()
                            .role(Role::Status)
                            .aria_label("Delivery")
                            .text_color(if action.is_none() { theme.warning } else { theme.dim })
                            .child(note.clone())
                            .test_text(note),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(chip(
                                "preview-close",
                                "Close",
                                "Close",
                                None,
                                &theme,
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.preview = None;
                                    cx.notify();
                                }),
                            ))
                            .children(action.map(|text| {
                                chip(
                                    "preview-send",
                                    text,
                                    label,
                                    None,
                                    &theme,
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.confirm_preview(cx)),
                                )
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        self.note_mounted(&row);
        let element = match &row {
            BodyRow::File(file) => self.render_file_header(*file, cx),
            BodyRow::Hunk { file, hunk } => self.render_hunk(*file, *hunk, cx),
            BodyRow::Line { file, hunk, line } => self.render_line(index, *file, *hunk, *line, cx),
            BodyRow::Comment { id, .. } => self.render_comment(*id, cx),
            BodyRow::Group(group) => self.render_group(*group),
            BodyRow::AgentNote { note, .. } => self.render_agent_note(*note, cx),
            BodyRow::Composer { .. } => self.render_composer(window, cx),
            BodyRow::Image(file) => self.render_image(*file, cx),
            BodyRow::Block { file, block } => self.render_block(index, *file, *block, cx),
            BodyRow::Note(file) => self.render_note(*file),
        };
        // A column stretches its child, so every row spans the body's width.
        div()
            .w_full()
            .flex()
            .flex_col()
            .children(element)
            .into_any_element()
    }

    fn render_file_header(&mut self, file: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let review = self.current_review()?;
        let change = review.review().files().get(file)?;
        let key = change.key.to_string();
        let collapsed = review.memory().collapsed.contains(&change.key);
        let viewed = review.memory().viewed(&change.key, &change.blob);
        let rendered = review.memory().rendered.contains(&change.key);
        let markdown = matches!(change.content, Content::Markdown(_));
        let (added, removed) = change.counts();
        let counts: SharedString = counts_text(change, added, removed).into();
        let kind = match change.change {
            ChangeKind::Added => "added",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Renamed => "renamed",
            ChangeKind::Modified => "",
        };
        let at_cursor = self.cursor == Some(BodyRow::File(file));
        let name: SharedString = key.clone().into();
        Some(
            div()
                .id(("header", file))
                .test_support()
                .role(Role::Heading)
                .aria_label(key.clone())
                .aria_level(2)
                .h(px(32.))
                .px_3()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .bg(theme.panel)
                .border_t_1()
                .border_b_1()
                .border_color(if at_cursor { theme.accent } else { theme.line })
                .child(
                    div()
                        .id(("toggle", file))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(format!("Collapse {key}"))
                        .aria_expanded(!collapsed)
                        .w(px(14.))
                        .text_color(theme.dim)
                        .child(if collapsed { "▸" } else { "▾" })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(key) = this.file_key(file) {
                                this.on_memory(|memory| memory.toggle_collapsed(&key), cx);
                            }
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .font_family(theme.mono.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.clone())
                        .test_text(name),
                )
                .when(!kind.is_empty(), |row| row.child(div().text_color(theme.dim).child(kind)))
                .when(viewed == Viewed::ChangedSince, |row| {
                    row.child(
                        div()
                            .text_color(theme.warning)
                            .child("changed since viewed")
                            .test_text("changed since viewed"),
                    )
                })
                .child(div().text_color(theme.dim).child(counts.clone()).test_text(counts))
                .when(self.pr_review(), |row| {
                    row.child(chip(
                        ("ask-file", file),
                        "Ask",
                        format!("Ask about {key}"),
                        None,
                        &theme,
                        cx.listener(move |this, _: &ClickEvent, window, cx| this.ask_about(file, None, window, cx)),
                    ))
                })
                .when(markdown, |row| {
                    row.child(chip(
                        ("rendered", file),
                        "Rendered",
                        format!("Rendered {key}"),
                        Some(rendered),
                        &theme,
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_rendered_file(file, cx)),
                    ))
                })
                .child(
                    div()
                        .id(("viewed", file))
                        .test_support()
                        .role(Role::CheckBox)
                        .aria_label(format!("Viewed {key}"))
                        .aria_toggled(if viewed == Viewed::Yes { Toggled::True } else { Toggled::False })
                        .flex()
                        .flex_row()
                        .gap_1()
                        .items_center()
                        .text_color(if viewed == Viewed::Yes { theme.added } else { theme.dim })
                        .child(if viewed == Viewed::Yes { "☑" } else { "☐" })
                        .child("Viewed")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.toggle_viewed(file, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.anchor = None;
                    this.go_to(BodyRow::File(file), cx);
                }))
                .into_any_element(),
        )
    }

    fn render_hunk(&mut self, file: usize, hunk: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let asks = self.pr_review();
        let review = self.current_review()?;
        let change = review.review().files().get(file)?;
        let key = change.key.to_string();
        let header: SharedString = change.text()?.hunks.get(hunk)?.header.clone().into();
        Some(
            div()
                .h(px(theme.line_height + 4.))
                .px_3()
                .flex()
                .flex_row()
                .gap_2()
                .items_center()
                .bg(theme.panel_raised)
                .text_color(theme.faint)
                .font_family(theme.mono.clone())
                .child(div().flex_1().child(header.clone()).test_text(header))
                .when(asks, |row| {
                    row.child(chip(
                        (SharedString::from(format!("ask-hunk-{file}")), hunk),
                        "Ask",
                        format!("Ask about hunk {} of {key}", hunk + 1),
                        None,
                        &theme,
                        cx.listener(move |this, _: &ClickEvent, window, cx| this.ask_about(file, Some(hunk), window, cx)),
                    ))
                })
                .into_any_element(),
        )
    }

    fn render_group(&mut self, group: Option<usize>) -> Option<AnyElement> {
        let theme = &self.theme;
        let review = self.current_review()?;
        let showing = review.memory().prepared.as_ref()?;
        let title: SharedString = match group {
            Some(g) => showing.view.groups.get(g)?.title.clone().into(),
            None => "Other changes".into(),
        };
        Some(
            div()
                .id(("group", group.map_or(usize::MAX, |g| g)))
                .test_support()
                .role(Role::Heading)
                .aria_label(title.clone())
                .aria_level(1)
                .px_3()
                .pt_3()
                .pb_1()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.accent)
                .child(title.clone())
                .test_text(title)
                .into_any_element(),
        )
    }

    /// An agent's note for the user (PR-23), marked with its name, with
    /// Make it a draft, Edit and Drop.
    fn render_agent_note(&mut self, note: NoteId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let review = self.current_review()?;
        let item: AgentNote = review.memory().agent.note(note)?.clone();
        let pr = self.pr_review();
        let place: SharedString = match &item.at {
            Some(at) if !item.outdated && at.lines.len() == 1 => format!("Note from {} on line {}", item.agent, at.lines.first),
            Some(at) if !item.outdated => format!("Note from {} on lines {}–{}", item.agent, at.lines.first, at.lines.last),
            Some(_) => format!("Note from {}, on lines that changed since", item.agent),
            None => format!("Note from {} on this file", item.agent),
        }
        .into();
        let text: SharedString = item.text.clone().into();
        let n = usize::try_from(note.0).unwrap_or(usize::MAX);
        let mut actions: Vec<AnyElement> = Vec::new();
        let mut action = |name: &str, text: &str, label: String, change: NoteChange| {
            actions.push(chip(
                (SharedString::from(format!("note-{name}")), n),
                text.to_owned(),
                label,
                None,
                &theme,
                cx.listener(move |this, _: &ClickEvent, window, cx| this.on_note(note, change, window, cx)),
            ));
        };
        if pr {
            action("draft", "Make it a draft", format!("Make note {note} a draft"), NoteChange::Draft);
            action("edit", "Edit", format!("Edit note {note}"), NoteChange::Edit);
        }
        action("drop", "Drop", format!("Drop note {note}"), NoteChange::Drop);
        Some(
            div()
                .id(("agent-note", n))
                .test_support()
                .role(Role::Note)
                .aria_label(format!("Note {note}"))
                .aria_value(text.clone())
                .mx_3()
                .my_1()
                .px_2()
                .py_1()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_l_2()
                .border_color(theme.accent)
                .bg(theme.panel_raised)
                .child(div().text_color(if item.outdated { theme.warning } else { theme.dim }).child(place.clone()).test_text(place))
                .child(div().child(text.clone()).test_text(text))
                .child(div().flex().flex_row().gap_1().children(actions))
                .into_any_element(),
        )
    }

    /// The list rows the selection covers.
    fn selection(&self) -> Option<(usize, usize)> {
        let head = self.cursor.as_ref()?;
        if !matches!(head, BodyRow::Line { .. } | BodyRow::Block { .. }) {
            return None;
        }
        let anchor = self.anchor.as_ref().unwrap_or(head);
        if anchor.file() != head.file() {
            return Some((self.row_index[head], self.row_index[head]));
        }
        let (a, b) = (*self.row_index.get(anchor)?, *self.row_index.get(head)?);
        Some((a.min(b), a.max(b)))
    }

    fn render_line(&mut self, index: usize, file: usize, hunk: usize, line: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let model = match self.lines.get(&(file, hunk, line)) {
            Some(model) => {
                self.stats.lines_reused += 1;
                model.clone()
            }
            None => {
                let diff_line = self
                    .current_review()?
                    .review()
                    .files()
                    .get(file)?
                    .text()?
                    .hunks
                    .get(hunk)?
                    .lines
                    .get(line)?
                    .clone();
                self.line_model(file, hunk, &diff_line, line)
            }
        };
        let key = self.file_key(file)?;
        let selected = self.selection().is_some_and(|(a, b)| (a..=b).contains(&index));
        let highlight: Option<(bool, SharedString)> = model.number.and_then(|(side, number)| {
            let review = self.current_review()?;
            let h = review.memory().agent.highlights_at(&key, side, number).next()?;
            Some((h.lines.first == number, h.label.clone().into()))
        });
        let theme = &self.theme;
        let (sign, color, background) = match model.kind {
            LineKind::Added => ("+", theme.added, Some(theme.added_background)),
            LineKind::Removed => ("−", theme.removed, Some(theme.removed_background)),
            LineKind::Context => (" ", theme.text, None),
        };
        let label = match model.kind {
            LineKind::Removed => format!("{key} old line {}", model.old),
            _ => format!("{key} line {}", model.new),
        };
        let row = BodyRow::Line { file, hunk, line };
        let pressed = row.clone();
        let moved = row.clone();
        Some(
            div()
                .id(("line", index))
                .test_support()
                .role(Role::ListItem)
                .aria_label(label)
                .aria_selected(selected)
                .h(px(theme.line_height))
                .flex()
                .flex_row()
                .font_family(theme.mono.clone())
                .when_some(background, |row, bg| row.bg(bg))
                .when(highlight.is_some(), |row| row.bg(theme.highlight_background))
                .when(selected, |row| row.bg(theme.selected_background))
                .when_some(highlight.as_ref().map(|(_, label)| label.clone()), |row, label| {
                    row.aria_description(format!("Highlighted: {label}"))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.dragging = !event.modifiers.shift;
                        this.select(pressed.clone(), event.modifiers.shift, cx);
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.dragging && event.pressed_button == Some(MouseButton::Left) && this.cursor.as_ref() != Some(&moved) {
                        this.select(moved.clone(), true, cx);
                    }
                }))
                .child(gutter(theme, model.old.clone()))
                .child(gutter(theme, model.new.clone()))
                .child(div().w(px(14.)).text_color(color).child(sign))
                .child({
                    let dark = (theme.background.r + theme.background.g + theme.background.b) < 1.5;
                    let language = humata2_highlight::Language::from_path(&key.path);
                    let spans = humata2_highlight::highlight_line(language, &model.text, dark);
                    let cell = div()
                        .flex_1()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(color);
                    let cell = if spans.is_empty() {
                        cell.child(model.text.clone())
                    } else {
                        cell.child(
                            gpui_kit::StyledText::new(model.text.clone()).with_highlights(spans),
                        )
                    };
                    cell.test_text(model.text.clone())
                })
                .when_some(highlight.filter(|(first, _)| *first).map(|(_, label)| label), |row, label| {
                    row.child(
                        div()
                            .px_2()
                            .text_color(theme.accent)
                            .child(label.clone())
                            .test_text(label),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_comment(&mut self, id: CommentId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let review = self.current_review()?;
        let comment: Comment = review.book().comment(id)?.clone();
        let at_cursor = matches!(&self.cursor, Some(BodyRow::Comment { id: at, .. }) if *at == id);
        let editing = review
            .composer()
            .is_some_and(|draft| draft.editing == Some(id));
        let (noun, title) = match comment.kind {
            Kind::Comment => ("comment", "Comment"),
            Kind::Question => ("question", "Question"),
            Kind::GitHub => ("draft", "Draft for GitHub"),
        };
        let github = comment.kind == Kind::GitHub;
        let status: &str = if github {
            "not posted: posting is the PR's own step"
        } else {
            status_text(&comment)
        };
        let by: Option<SharedString> = comment.by.clone().map(|by| format!("by {by}").into());
        let anchor = &comment.anchor;
        let side = if anchor.side == Side::Old { " (old side)" } else { "" };
        let place: SharedString = match &anchor.rendered {
            Some(span) => format!("{id} on “{}”, lines {}–{}{side}", short(&span.text, 48), anchor.lines.first, anchor.lines.last),
            None if anchor.lines.is_file() => format!("{id} on the whole file"),
            None if anchor.lines.len() == 1 => format!("{id} on line {}{side}", anchor.lines.first),
            None => format!("{id} on lines {}–{}{side}", anchor.lines.first, anchor.lines.last),
        }
        .into();
        let body: SharedString = comment.body.clone().into();
        let mut actions: Vec<AnyElement> = Vec::new();
        let mut action = |name: &str, label: String, handler: Box<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>| {
            let handler = Rc::new(handler);
            actions.push(chip(
                (SharedString::from(format!("{noun}-{name}-{}", id.0)), 0usize),
                name.to_owned(),
                label,
                None,
                &theme,
                cx.listener(move |this, _: &ClickEvent, window, cx| handler(this, window, cx)),
            ));
        };
        match comment.status {
            Status::Draft => {
                if !github {
                    action("Send", format!("Send {noun} {id}"), Box::new(move |this, _, cx| this.open_preview(Some(vec![id]), cx)));
                }
                action("Edit", format!("Edit {noun} {id}"), Box::new(move |this, window, cx| this.edit_comment(id, window, cx)));
                action("Drop", format!("Drop {noun} {id}"), Box::new(move |this, _, cx| this.on_book(|book| book.delete(id), cx)));
            }
            Status::Queued | Status::Failed => {
                if comment.status == Status::Failed
                    && let Some(delivery) = comment.delivery
                {
                    action("Retry", format!("Retry {noun} {id}"), Box::new(move |this, _, cx| this.on_book(|book| book.retry(delivery), cx)));
                }
                action("Edit", format!("Edit {noun} {id}"), Box::new(move |this, window, cx| this.edit_comment(id, window, cx)));
                action("Drop", format!("Drop {noun} {id}"), Box::new(move |this, _, cx| this.on_book(|book| book.delete(id), cx)));
            }
            Status::Sending => {}
            Status::Sent | Status::Addressed => {
                action("Resolve", format!("Resolve {noun} {id}"), Box::new(move |this, _, cx| this.on_book(|book| book.resolve(id), cx)));
                // A draft again, to change and send once more.
                action("Draft again", format!("Draft {noun} {id} again"), Box::new(move |this, _, cx| this.on_book(|book| book.reopen(id), cx)));
            }
            Status::Resolved => {
                action("Reopen", format!("Reopen {noun} {id}"), Box::new(move |this, _, cx| this.on_book(|book| book.reopen(id), cx)));
            }
        }
        if comment.outdated && comment.status.unsent() && comment.status != Status::Sending {
            action("Re-attach", format!("Re-attach {noun} {id}"), Box::new(move |this, _, cx| this.reattach_comment(id, cx)));
        }
        let outdated = comment.outdated.then(|| {
            let text = "The code here changed since this was written. Select the lines it's about now and re-attach it, or drop it.";
            div().text_color(theme.warning).child(text).test_text(text)
        });
        Some(
            div()
                .id((noun, usize::try_from(id.0).unwrap_or(usize::MAX)))
                .test_support()
                .role(Role::Group)
                .aria_label(format!("{title} {id}"))
                .aria_description(status)
                .mx_3()
                .my_1()
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(if at_cursor { theme.accent } else { theme.line })
                .bg(theme.panel)
                .when(editing, |card| card.opacity(0.5))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(row) = this.rows.iter().find(|row| matches!(row, BodyRow::Comment { id: at, .. } if *at == id)).cloned() {
                        this.anchor = None;
                        this.go_to(row, cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(div().flex_1().text_color(theme.dim).child(place.clone()).test_text(place))
                        .children(by.map(|by| div().text_color(theme.dim).child(by.clone()).test_text(by)))
                        .child(
                            div()
                                .text_color(if github { theme.dim } else { status_color(&theme, comment.status) })
                                .child(status)
                                .test_text(status),
                        ),
                )
                .child(div().child(body.clone()).test_text(body))
                .children(outdated)
                .child(div().flex().flex_row().gap_1().children(actions))
                .into_any_element(),
        )
    }

    fn edit_comment(&mut self, id: CommentId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.current_review_mut() else {
            return;
        };
        match review.edit_composer(id) {
            Ok(()) => {
                self.notice = None;
                window.focus(&self.composer_focus, cx);
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        if let Some(session) = self.current.clone() {
            self.changed(&session, cx);
        }
    }

    /// Points an outdated comment at the selected lines.
    fn reattach_comment(&mut self, id: CommentId, cx: &mut Context<Self>) {
        let Some((file, side, lines, rendered)) = self.selected_lines() else {
            self.notice = Some("Select the lines the comment is about now, then re-attach it.".into());
            cx.notify();
            return;
        };
        let Some(key) = self.file_key(file) else {
            return;
        };
        let Some(review) = self.current_review_mut() else {
            return;
        };
        let at = crate::model::ComposerAt { file: key, side, lines };
        let Some(anchor) = review.anchor(&at, rendered) else {
            return;
        };
        self.on_book(|book| book.reattach(id, anchor), cx);
    }

    fn render_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let review = self.current_review()?;
        let draft = review.composer()?.clone();
        let focused = self.composer_focus.is_focused(window);
        let pr = self.pr_review();
        let (noun, what) = match draft.kind {
            Kind::Comment => ("Comment", "comment"),
            Kind::Question => ("Question", "question"),
            Kind::GitHub => ("Draft for GitHub", "draft"),
        };
        let who = if pr { "the review chat" } else { "the agent" };
        let title: SharedString = match (draft.editing, draft.at.lines) {
            (Some(id), _) => format!("Editing {what} {id}").into(),
            (None, lines) if lines.is_file() => format!("{noun} on the whole file").into(),
            (None, lines) if lines.len() == 1 => format!("{noun} on line {}", lines.first).into(),
            (None, lines) => format!("{noun} on lines {}–{}", lines.first, lines.last).into(),
        };
        let shown: SharedString = match (draft.text.is_empty(), draft.kind) {
            (true, Kind::Comment) => format!("Write what {who} should do here. cmd-enter adds it.").into(),
            (true, Kind::Question) => format!("Ask {who} about this code. It answers and changes nothing. cmd-enter adds it.").into(),
            (true, Kind::GitHub) => "Write the review comment for GitHub. It's posted only when you post the review.".into(),
            (false, _) => draft.text.clone().into(),
        };
        let lines = shown.lines().map(|line| {
            let line = SharedString::from(line.to_owned());
            div().min_h(px(theme.line_height)).child(line.clone()).test_text(line)
        }).collect::<Vec<_>>();
        let submit = match (draft.editing.is_some(), draft.kind) {
            (true, _) => "Save",
            (false, Kind::Comment) => "Add comment",
            (false, Kind::Question) => "Add question",
            (false, Kind::GitHub) => "Add draft",
        };
        let kinds: Vec<AnyElement> = if pr {
            [(Kind::Comment, "Send to agent"), (Kind::Question, "Ask"), (Kind::GitHub, "Draft for GitHub")]
                .into_iter()
                .map(|(kind, text)| {
                    chip(
                        SharedString::from(format!("composer-kind-{text}")),
                        text,
                        text,
                        Some(draft.kind == kind),
                        &theme,
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.set_composer_kind(kind, cx)),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(
            div()
                .mx_3()
                .my_1()
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(theme.accent)
                .bg(theme.panel)
                .child(div().text_color(theme.dim).child(title.clone()).test_text(title))
                .child(
                    div()
                        .id("composer")
                        .test_support()
                        .role(Role::TextInput)
                        .aria_label(match draft.kind {
                            Kind::Comment => "Comment for the agent",
                            Kind::Question => "Question for the agent",
                            Kind::GitHub => "Draft for GitHub",
                        })
                        .aria_value(draft.text.clone())
                        .aria_placeholder(match draft.kind {
                            Kind::Comment => "Write what the agent should do here",
                            Kind::Question => "Ask about this code",
                            Kind::GitHub => "Write the comment for GitHub",
                        })
                        .track_focus(&self.composer_focus)
                        .min_h(px(54.))
                        .p_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(if focused { theme.accent } else { theme.line })
                        .bg(theme.background)
                        .text_color(if draft.text.is_empty() { theme.faint } else { theme.text })
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            window.focus(&this.composer_focus, cx);
                        }))
                        .on_key_down(cx.listener(|this, event, window, cx| this.composer_key_down(event, window, cx)))
                        .children(lines),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap_1()
                        .children(kinds)
                        .child(div().flex_1())
                        .child(chip(
                            "composer-cancel",
                            "Cancel",
                            format!("Cancel {what}"),
                            None,
                            &theme,
                            cx.listener(|this, _: &ClickEvent, window, cx| this.cancel_composer(window, cx)),
                        ))
                        .child(chip(
                            "composer-submit",
                            submit,
                            submit,
                            None,
                            &theme,
                            cx.listener(|this, _: &ClickEvent, window, cx| this.submit_composer(window, cx)),
                        )),
                )
                .into_any_element(),
        )
    }

    fn render_note(&mut self, file: usize) -> Option<AnyElement> {
        let theme = &self.theme;
        let change = self.current_review()?.review().files().get(file)?;
        let size = |bytes: Option<u64>| bytes.map_or("nothing".to_owned(), human_bytes);
        let text: SharedString = match &change.content {
            Content::Binary { old_size, new_size } => format!(
                "Binary file, not shown: {} before, {} after.",
                size(*old_size),
                size(*new_size)
            )
            .into(),
            _ => "Not shown.".into(),
        };
        Some(
            div()
                .px_3()
                .py_2()
                .text_color(theme.dim)
                .child(text.clone())
                .test_text(text)
                .into_any_element(),
        )
    }

    fn render_block(&mut self, index: usize, file: usize, block: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let change = self.current_review()?.review().files().get(file)?;
        let key = change.key.clone();
        let blocks = self.rendered.get(&(change.key.clone(), change.blob.clone()))?.clone();
        let rendered: &RenderedBlock = blocks.get(block)?;
        let selected = self.selection().is_some_and(|(a, b)| (a..=b).contains(&index));
        let (mark, label) = match rendered.change {
            Change::Same => (None, "unchanged"),
            Change::Added => (Some(theme.added), "added"),
            Change::Changed => (Some(theme.changed_mark), "changed"),
            Change::Removed => (Some(theme.removed), "removed"),
        };
        let text: SharedString = match rendered.block.kind {
            BlockKind::ListItem => format!("• {}", rendered.block.text).into(),
            BlockKind::Rule => "―――".into(),
            _ => rendered.block.text.clone().into(),
        };
        let row = BodyRow::Block { file, block };
        let pressed = row.clone();
        let moved = row.clone();
        let removed = rendered.change == Change::Removed;
        let mut body = div()
            .flex_1()
            .py_1()
            .child(text.clone())
            .when(removed, |body| body.line_through().text_color(theme.faint));
        body = match rendered.block.kind {
            BlockKind::Heading(level) => body
                .font_weight(FontWeight::BOLD)
                .text_size(px(theme.font_size + f32::from(6u8.saturating_sub(level)) * 2.)),
            BlockKind::Code => body.font_family(theme.mono.clone()).bg(theme.panel).p_1(),
            BlockKind::Quote => body.text_color(theme.dim).border_l_2().border_color(theme.line).pl_2(),
            _ => body,
        };
        Some(
            div()
                .id(("block", index))
                .test_support()
                .role(Role::ListItem)
                .aria_label(format!("{key} block {}", block + 1))
                .aria_description(label)
                .aria_selected(selected)
                .px_3()
                .flex()
                .flex_row()
                .gap_2()
                .when(selected, |row| row.bg(theme.selected_background))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.dragging = !event.modifiers.shift;
                        this.select(pressed.clone(), event.modifiers.shift, cx);
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.dragging && event.pressed_button == Some(MouseButton::Left) && this.cursor.as_ref() != Some(&moved) {
                        this.select(moved.clone(), true, cx);
                    }
                }))
                .child(div().w(px(3.)).when_some(mark, |bar, color| bar.bg(color)))
                .child(body.test_text(text))
                .into_any_element(),
        )
    }

    fn render_image(&mut self, file: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme.clone();
        let review = self.current_review()?;
        let change = review.review().files().get(file)?;
        let Content::Image(image) = &change.content else {
            return None;
        };
        let key = change.key.to_string();
        let view = review.memory().image(&change.key);
        let (old_blob, new_blob) = (image.old.clone(), image.new.clone());
        let mut jobs = Vec::new();
        let old = self.images.side(old_blob.as_ref(), &mut jobs);
        let new = self.images.side(new_blob.as_ref(), &mut jobs);
        let both = matches!((&old, &new), (ImageSide::Ready { .. }, ImageSide::Ready { .. }));
        let diff = match (&old_blob, &new_blob) {
            (Some(a), Some(b)) if both => self.images.diff(a, b, &mut jobs),
            _ => None,
        };
        self.run_jobs(jobs, cx);
        let size = display_size(&old, &new);
        let bounds = self
            .image_bounds
            .entry(file)
            .or_insert_with(|| Rc::new(Cell::new(Bounds::default())))
            .clone();
        let mode = if both { view.mode } else { ImageMode::SideBySide };
        let picture = match (&diff, view.highlight) {
            (Some(DiffSlot::Ready { picture, .. }), true) => Some(picture.clone()),
            _ => None,
        };
        let summary: Option<SharedString> = match &diff {
            Some(DiffSlot::Ready { changed, total, resized, .. }) => Some(
                format!(
                    "{changed} of {total} pixels changed ({:.1}%){}",
                    percent(*changed, *total),
                    if *resized { "; the size changed too" } else { "" }
                )
                .into(),
            ),
            Some(DiffSlot::Loading) => Some("Comparing pixels…".into()),
            None => None,
        };
        let content: AnyElement = match mode {
            ImageMode::SideBySide => div()
                .flex()
                .flex_row()
                .gap_3()
                .child(side_panel(&theme, "Before", &old, size, None, &key))
                .child(side_panel(&theme, "After", &new, size, picture.clone(), &key))
                .into_any_element(),
            ImageMode::Swipe { position } => {
                let (ImageSide::Ready { image: before, .. }, ImageSide::Ready { image: after, .. }) = (&old, &new) else {
                    return None;
                };
                let cut = size.0 * f32::from(position) / 1000.;
                stage(&theme, size, &bounds, file, &key, view.mode, cx)
                    .child(picture_at(before.clone().into(), size))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .w(px(cut))
                            .h(px(size.1))
                            .overflow_hidden()
                            .child(picture_at(after.clone().into(), size)),
                    )
                    .children(picture.clone().map(|p| picture_at(p.into(), size).absolute().top_0().left_0().opacity(0.7)))
                    .child(div().absolute().top_0().left(px(cut)).w(px(2.)).h(px(size.1)).bg(theme.accent))
                    .into_any_element()
            }
            ImageMode::Onion { opacity } => {
                let (ImageSide::Ready { image: before, .. }, ImageSide::Ready { image: after, .. }) = (&old, &new) else {
                    return None;
                };
                stage(&theme, size, &bounds, file, &key, view.mode, cx)
                    .child(picture_at(before.clone().into(), size))
                    .child(
                        picture_at(after.clone().into(), size)
                            .absolute()
                            .top_0()
                            .left_0()
                            .opacity(f32::from(opacity) / 1000.),
                    )
                    .children(picture.clone().map(|p| picture_at(p.into(), size).absolute().top_0().left_0().opacity(0.7)))
                    .into_any_element()
            }
        };
        let modes: Vec<AnyElement> = if both {
            let mode_chip = |id: &'static str, text: &'static str, chosen: ImageMode, on: bool, cx: &mut Context<Self>| {
                chip(
                    (id, file),
                    text,
                    format!("{text} {key}"),
                    Some(on),
                    &theme,
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.set_image_mode(file, Some(chosen), None, cx)),
                )
            };
            let position = match view.mode {
                ImageMode::Swipe { position } => position,
                _ => 500,
            };
            let opacity = match view.mode {
                ImageMode::Onion { opacity } => opacity,
                _ => 500,
            };
            vec![
                mode_chip("side", "Side by side", ImageMode::SideBySide, matches!(view.mode, ImageMode::SideBySide), cx),
                mode_chip("swipe", "Swipe", ImageMode::Swipe { position }, matches!(view.mode, ImageMode::Swipe { .. }), cx),
                mode_chip("onion", "Onion skin", ImageMode::Onion { opacity }, matches!(view.mode, ImageMode::Onion { .. }), cx),
                chip(
                    ("highlight", file),
                    "Highlight changes",
                    format!("Highlight changes {key}"),
                    Some(view.highlight),
                    &theme,
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.set_image_mode(file, None, Some(!view.highlight), cx)),
                ),
            ]
        } else {
            Vec::new()
        };
        let value: Option<SharedString> = match view.mode {
            ImageMode::Swipe { position } if both => Some(format!("swipe at {}%", position / 10).into()),
            ImageMode::Onion { opacity } if both => Some(format!("after at {}% opacity", opacity / 10).into()),
            _ => None,
        };
        Some(
            div()
                .id(("image", file))
                .test_support()
                .role(Role::Figure)
                .aria_label(format!("Image {key}"))
                .aria_description(match mode {
                    ImageMode::SideBySide => "side by side",
                    ImageMode::Swipe { .. } => "swipe",
                    ImageMode::Onion { .. } => "onion skin",
                })
                .px_3()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.anchor = None;
                    this.go_to(BodyRow::Image(file), cx);
                }))
                .child(div().flex().flex_row().gap_1().children(modes))
                .child(content)
                .children(value.map(|value| div().text_color(theme.dim).child(value.clone()).test_text(value)))
                .children(summary.filter(|_| both).map(|summary| div().text_color(theme.dim).child(summary.clone()).test_text(summary)))
                .into_any_element(),
        )
    }
}

/// The box a swipe or onion skin is drawn in; dragging in it moves the cut
/// or the opacity.
fn stage(
    theme: &Theme,
    size: (f32, f32),
    bounds: &Rc<Cell<Bounds<Pixels>>>,
    file: usize,
    key: &str,
    mode: ImageMode,
    cx: &mut Context<ReviewPane>,
) -> Stateful<Div> {
    let store = bounds.clone();
    let down = bounds.clone();
    let moved = bounds.clone();
    let value = match mode {
        ImageMode::Swipe { position } => position,
        ImageMode::Onion { opacity } => opacity,
        ImageMode::SideBySide => 500,
    };
    let set = move |this: &mut ReviewPane, x: Pixels, bounds: Bounds<Pixels>, cx: &mut Context<ReviewPane>| {
        let width = f32::from(bounds.size.width);
        if width <= 0. {
            return;
        }
        let at = ((f32::from(x - bounds.origin.x) / width) * 1000.).clamp(0., 1000.);
        let at = at.round() as u16;
        let mode = match mode {
            ImageMode::Swipe { .. } => ImageMode::Swipe { position: at },
            ImageMode::Onion { .. } => ImageMode::Onion { opacity: at },
            ImageMode::SideBySide => return,
        };
        this.set_image_mode(file, Some(mode), None, cx);
    };
    let set = Rc::new(set);
    let set_down = set.clone();
    div()
        .id(("stage", file))
        .relative()
        .w(px(size.0))
        .h(px(size.1))
        .bg(theme.panel)
        .overflow_hidden()
        .child(
            canvas(move |b, _, _| store.set(b), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .child(
            div()
                .id(("stage-handle", file))
                .test_support()
                .role(Role::Slider)
                .aria_label(match mode {
                    ImageMode::Onion { .. } => format!("Onion skin opacity {key}"),
                    _ => format!("Swipe position {key}"),
                })
                .aria_value(format!("{}", value / 10))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        set_down(this, event.position.x, down.get(), cx);
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        set(this, event.position.x, moved.get(), cx);
                    }
                })),
        )
}

fn side_panel(
    theme: &Theme,
    title: &str,
    side: &ImageSide,
    size: (f32, f32),
    highlight: Option<std::sync::Arc<gpui_kit::RenderImage>>,
    key: &str,
) -> AnyElement {
    let (caption, body): (String, AnyElement) = match side {
        ImageSide::Absent => (
            format!("{title}: none"),
            placeholder(theme, size, if title == "Before" { "Added: there was no image before." } else { "Deleted: there's no image now." }),
        ),
        ImageSide::Loading => (format!("{title}: decoding…"), placeholder(theme, size, "Decoding…")),
        ImageSide::TooLarge { bytes, pixels } => {
            let what = match pixels {
                Some((w, h)) => format!("{w}×{h}, {}", human_bytes(*bytes as u64)),
                None => human_bytes(*bytes as u64),
            };
            (
                format!("{title}: too large"),
                placeholder(theme, size, &format!("Too large to show here ({what}). Open the file in an image viewer to see it.")),
            )
        }
        ImageSide::Broken { reason } => (
            format!("{title}: can't be shown"),
            placeholder(
                theme,
                size,
                &format!("This image doesn't decode, so the file may be damaged or in a format Humata can't read. Open it in an image viewer to check. ({reason})"),
            ),
        ),
        ImageSide::Ready { image, width, height } => {
            let shown = highlight.unwrap_or_else(|| image.clone());
            (format!("{title}: {width}×{height}"), picture_at(shown.into(), size).into_any_element())
        }
    };
    let caption: SharedString = caption.into();
    div()
        .id(SharedString::from(format!("{title} {key}")))
        .test_support()
        .role(Role::Figure)
        .aria_label(format!("{title} {key}"))
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_color(theme.dim).child(caption.clone()).test_text(caption))
        .child(body)
        .into_any_element()
}

/// Narrower than this, a placeholder's sentence wraps a word to a line.
const PLACEHOLDER_WIDTH: f32 = 240.;

/// Stands in for an image that can't be drawn: as wide as the image, or wide
/// enough to read, and as tall as its text needs. The text wraps inside it.
fn placeholder(theme: &Theme, size: (f32, f32), text: &str) -> AnyElement {
    let text: SharedString = text.to_owned().into();
    div()
        .w(px(size.0.max(PLACEHOLDER_WIDTH)))
        .min_h(px(size.1))
        .p_3()
        .bg(theme.panel)
        .text_color(theme.dim)
        .child(div().w_full().child(text.clone()))
        .test_text(text)
        .into_any_element()
}

fn picture_at(source: ImageSource, size: (f32, f32)) -> gpui_kit::Img {
    img(source).w(px(size.0)).h(px(size.1)).object_fit(ObjectFit::Contain)
}

/// The size both sides are drawn at: the larger image's, fitted in the box.
fn display_size(old: &ImageSide, new: &ImageSide) -> (f32, f32) {
    let dims = |side: &ImageSide| match side {
        ImageSide::Ready { width, height, .. } => Some((*width as f32, *height as f32)),
        _ => None,
    };
    let (w, h) = match (dims(old), dims(new)) {
        (Some(a), Some(b)) => (a.0.max(b.0), a.1.max(b.1)),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return (IMAGE_BOX.0 / 2., IMAGE_BOX.1 / 2.),
    };
    let scale = (IMAGE_BOX.0 / w).min(IMAGE_BOX.1 / h).min(1.);
    ((w * scale).max(1.), (h * scale).max(1.))
}

fn gutter(theme: &Theme, number: SharedString) -> impl IntoElement {
    div()
        .w(px(44.))
        .pr_2()
        .flex()
        .justify_end()
        .text_color(theme.faint)
        .child(number)
}

/// A small button. `toggled` makes it a toggle that says whether it's on.
fn chip(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    label: impl Into<SharedString>,
    toggled: Option<bool>,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let text = text.into();
    let on = toggled == Some(true);
    div()
        .id(id)
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .when_some(toggled, |chip, on| {
            chip.aria_toggled(if on { Toggled::True } else { Toggled::False })
        })
        .px_2()
        .h(px(22.))
        .flex()
        .items_center()
        .rounded_sm()
        .border_1()
        .border_color(if on { theme.accent } else { theme.line })
        .bg(if on { theme.selected_background } else { theme.panel_raised })
        .text_color(if on { theme.text } else { theme.dim })
        .cursor_pointer()
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx);
        })
        .child(div().child(text.clone()).test_text(text))
        .into_any_element()
}

fn message(theme: &Theme, text: impl Into<SharedString>) -> AnyElement {
    let text = text.into();
    div()
        .id("pane-message")
        .test_support()
        .role(Role::Status)
        .aria_label("Review")
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.dim)
        .child(text.clone())
        .test_text(text)
        .into_any_element()
}

fn counts_text(change: &FileChange, added: usize, removed: usize) -> String {
    match &change.content {
        Content::Image(_) => "image".to_owned(),
        Content::Binary { .. } => "binary".to_owned(),
        _ => format!("+{added} −{removed}"),
    }
}

fn agent_text(agent: AgentState, pr: bool) -> &'static str {
    if pr {
        return match agent {
            AgentState::Unknown => "Waiting to hear from the review chat; questions and comments wait until it says.",
            AgentState::Idle => "Review chat idle: questions and comments go now.",
            AgentState::Working => "Review chat answering: questions and comments wait for it.",
            AgentState::ProbablyIdle => "Review chat probably idle: they wait; Send now types them.",
            AgentState::Disconnected => "Machine offline: questions and comments wait until it reconnects.",
            AgentState::Exited => "Review chat exited: start it again in its terminal, then retry.",
        };
    }
    match agent {
        AgentState::Unknown => "Waiting to hear from the agent's terminal; comments wait until it says.",
        AgentState::Idle => "Agent idle: comments go now.",
        AgentState::Working => "Agent working: comments wait for its turn to end.",
        AgentState::ProbablyIdle => "Agent probably idle: comments wait; Send now types them.",
        AgentState::Disconnected => "Machine offline: comments wait until it reconnects.",
        AgentState::Exited => "Agent exited: resume it in its terminal, then retry.",
    }
}

fn delivery_text(delivery: &Delivery, agent: AgentState, pr: bool) -> String {
    let what = match delivery.comments.len() {
        1 => "1 comment".to_owned(),
        n => format!("{n} comments"),
    };
    let (who, resume) = if pr {
        ("the review chat", "Start it again in its terminal, then retry")
    } else {
        ("the agent", "Resume it in its terminal, then retry")
    };
    let state = match &delivery.state {
        DeliveryState::Queued { send_now: true, .. } => "goes as soon as the terminal takes it".to_owned(),
        DeliveryState::Queued { hold, .. } => match (hold, agent) {
            (_, AgentState::Exited) => format!("can't go: {who} exited. {resume}"),
            (Hold::Ready, _) => "next to go".to_owned(),
            (Hold::Busy, _) => format!("waits for {who}'s turn to end"),
            (Hold::TurnEndUncertain, _) => format!("waits: {who}'s turn may not have ended. Send now types it anyway"),
            (Hold::Disconnected, _) => "waits for the machine to reconnect".to_owned(),
            (Hold::Unknown, _) => format!("waits to hear from {who}'s terminal"),
        },
        DeliveryState::Sending { .. } => "being typed into the terminal".to_owned(),
        DeliveryState::Sent => format!("with {who}"),
        DeliveryState::Failed { failure: Failure::Exited } => format!("didn't go: {who} exited. {resume}"),
        DeliveryState::Failed { failure: Failure::Refused { reason } } => {
            format!("didn't go: the terminal refused it ({reason}). Check the terminal, then retry")
        }
    };
    let doubt = if delivery.in_doubt {
        ". It may already have arrived, so it's sent again under the same key, which the terminal types at most once"
    } else {
        ""
    };
    format!("Message {} ({what}) {state}{doubt}.", delivery.id)
}

fn status_text(comment: &Comment) -> &'static str {
    match comment.status {
        Status::Draft if comment.outdated => "draft, outdated",
        Status::Draft => "draft",
        Status::Queued => "queued",
        Status::Sending => "sending",
        Status::Sent => "sent",
        Status::Failed => "failed",
        Status::Addressed => "addressed",
        Status::Resolved => "resolved",
    }
}

fn status_color(theme: &Theme, status: Status) -> Rgba {
    match status {
        Status::Draft => theme.dim,
        Status::Queued | Status::Sending => theme.warning,
        Status::Sent | Status::Addressed => theme.added,
        Status::Failed => theme.removed,
        Status::Resolved => theme.faint,
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < UNITS.len() {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.
    } else {
        part as f64 * 100. / whole as f64
    }
}

fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(limit).collect();
        format!("{cut}…")
    }
}
