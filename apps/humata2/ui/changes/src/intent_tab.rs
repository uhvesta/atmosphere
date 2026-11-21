//! Intent Spec (`humata.intent/v1`) viewer for `intent.yaml` and `*.intent.yaml`
//! files opened as a session tab.
//!
//! Renders the approved Structured | YAML | Diff layout with schema validity
//! pill, version stepper, inline comments, and the `Approve & Start Milestone`
//! action bar.

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, TestSupportExt as _, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, NewSessionAction};
use humata2_clients::comments::{Anchor, Comment, Side};
use humata2_clients::docs::VersionText;
use humata2_clients::model::{DiffLine, ItemId, LineKind, Session};
use humata2_intent_spec::{IntentDiff, IntentSpec, ValidationReport};
use humata2_review::comments::CommentsAction;
use humata2_session::{CompareDiff, DocHistory, DocumentMode, HistoryAction, SessionAction};

use super::{Documents, Target, file_name};
use crate::bridge::Ui;
use crate::chrome;
use crate::comment_cards::{self, StartReattach};
use crate::theme::Look;
use crate::widgets::{self, col, row};

/// Active view mode for an Intent Spec tab, mapped onto [`DocumentMode`] so
/// existing session persistence works without new state or proto fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentMode {
    Structured,
    Yaml,
    Diff,
}

impl From<DocumentMode> for IntentMode {
    fn from(mode: DocumentMode) -> Self {
        match mode {
            DocumentMode::Preview => Self::Structured,
            DocumentMode::Source => Self::Yaml,
            DocumentMode::SideBySide => Self::Diff,
        }
    }
}

/// Pure, testable view model derived from the raw YAML text of an Intent Spec.
#[derive(Debug)]
pub struct IntentViewModel {
    pub parsed: Result<(IntentSpec, ValidationReport), String>,
    pub diff: Option<IntentDiff>,
    pub read_only: bool,
    pub can_approve: bool,
}

impl IntentViewModel {
    /// Parses and validates `text`, computes a semantic diff against `prev_yaml`
    /// when both parse, and evaluates the `Approve & Start Milestone` gate.
    pub fn from_yaml(text: &str, prev_yaml: Option<&str>, read_only: bool) -> Self {
        let parsed = humata2_intent_spec::parse_and_validate(text).map_err(|err| err.to_string());
        let diff = match (&parsed, prev_yaml) {
            (Ok((after, _)), Some(prev)) => humata2_intent_spec::parse(prev)
                .ok()
                .map(|before| IntentDiff::compute(&before, after)),
            _ => None,
        };
        let can_approve = !read_only
            && parsed
                .as_ref()
                .is_ok_and(|(_, report)| report.can_approve());
        Self {
            parsed,
            diff,
            read_only,
            can_approve,
        }
    }

    /// Returns `(is_valid, label)` for the header schema validity pill.
    pub fn schema_badge(&self) -> (bool, String) {
        match &self.parsed {
            Ok((_, report)) if report.is_schema_valid() => (true, "✓ Schema valid".to_owned()),
            Ok((_, report)) => {
                let n = report.errors.len();
                let noun = if n == 1 { "error" } else { "errors" };
                (false, format!("✕ {n} schema {noun}"))
            }
            Err(_) => (false, "✕ Parse error".to_owned()),
        }
    }

    /// Explains why `Approve & Start Milestone` is disabled, if it is.
    pub fn approval_disabled_reason(&self) -> Option<String> {
        if self.can_approve {
            return None;
        }
        if self.read_only {
            return Some("Viewing an older version (read-only)".to_owned());
        }
        match &self.parsed {
            Err(_) => Some("Fix YAML parse error before approving".to_owned()),
            Ok((_, report)) if !report.is_schema_valid() => {
                Some("Fix schema validation errors before approving".to_owned())
            }
            Ok((_, report)) if !report.blocking_open_questions.is_empty() => {
                let n = report.blocking_open_questions.len();
                let noun = if n == 1 { "question" } else { "questions" };
                Some(format!("Resolve {n} blocking open {noun} before approving"))
            }
            Ok(_) => Some("Cannot approve".to_owned()),
        }
    }
}

/// Renders the Intent Spec tab body for `intent.yaml` / `*.intent.yaml`.
#[allow(clippy::too_many_arguments)]
pub(super) fn body(
    docs: &Documents,
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    look: Look,
) -> Option<AnyElement> {
    let key = (session.id.clone(), repo.to_owned(), path.to_owned());
    let Some(Ok(current_text)) = state.session.file_contents.get(&key) else {
        return None;
    };
    let shown = state
        .session
        .view()
        .map(|view| view.document(repo, path))
        .unwrap_or_default();
    let mode = IntentMode::from(shown.mode);

    let history = if repo.is_empty() {
        state.session.history(&session.id, path)
    } else {
        None
    };
    let old = history.filter(|h| h.read_only());
    let read = old.and_then(|h| h.viewed.as_ref());
    let reading = old.is_some() && read.is_none();

    if reading {
        return Some(crate::doc_history::reading(look));
    }

    let text = read.map_or(current_text.as_str(), |r| r.text.as_str());
    let prev_yaml = if read.is_some() {
        Some(current_text.as_str())
    } else {
        history
            .and_then(|h| h.viewed.as_ref())
            .map(|v| v.text.as_str())
    };
    let vm = IntentViewModel::from_yaml(text, prev_yaml, old.is_some());

    let header = header_bar(ui, path, mode, &vm, history, repo, look);
    let meta = meta_strip(&vm, look);
    let content = match mode {
        IntentMode::Structured => {
            structured_view(docs, ui, state, session, repo, path, &vm, read, look)
        }
        IntentMode::Yaml => yaml_view(docs, ui, state, session, repo, path, text, read, look),
        IntentMode::Diff => diff_view(state, repo, path, &vm, look),
    };
    let footer = action_bar(ui, state, session, repo, path, text, &vm, history, look);

    Some(
        col()
            .id("intent-spec-tab")
            .role(Role::Region)
            .aria_label(format!("{} intent spec", file_name(path)))
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(rgb(look.content))
            .child(header)
            .children(meta)
            .child(content)
            .child(footer)
            .into_any_element(),
    )
}

#[allow(clippy::too_many_arguments)]
fn header_bar(
    ui: &Rc<Ui>,
    path: &str,
    mode: IntentMode,
    vm: &IntentViewModel,
    history: Option<&DocHistory>,
    repo: &str,
    look: Look,
) -> Stateful<Div> {
    let dark = ((look.content >> 8) & 0xff) < 0x80;
    let (valid, badge_label) = vm.schema_badge();
    let (pill_bg, pill_fg) = if valid {
        if dark {
            (0x17301f, 0x30d158)
        } else {
            (0xe6f6ea, 0x1b7f37)
        }
    } else if dark {
        (0x3a1d1d, 0xff453a)
    } else {
        (0xfdeaea, 0xc92a2a)
    };

    let segments = [
        (DocumentMode::Preview, IntentMode::Structured, "Structured", "structured"),
        (DocumentMode::Source, IntentMode::Yaml, "YAML", "yaml"),
        (DocumentMode::SideBySide, IntentMode::Diff, "Diff", "diff"),
    ];
    let switcher = row()
        .id("intent-mode-switch")
        .role(Role::RadioGroup)
        .aria_label("Intent view mode")
        .test_support()
        .p(px(2.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .flex_shrink_0()
        .children(segments.into_iter().map(|(doc_mode, seg_mode, label, slug)| {
            let on = mode == seg_mode;
            let ui = ui.clone();
            let (repo, path) = (repo.to_owned(), path.to_owned());
            div()
                .id(SharedString::from(format!("intent-mode-{slug}")))
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(on)
                .px(px(10.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(rgb(look.text))
                .cursor_pointer()
                .when(on, |s| s.bg(rgb(look.card)).shadow_sm())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    ui.send(
                        AppAction::Session(SessionAction::DocumentMode {
                            repo: repo.clone(),
                            path: path.clone(),
                            mode: doc_mode,
                        }),
                        cx,
                    );
                })
                .child(label)
                .test_support()
        }));

    let stepper = history.map(|h| {
        let act = |action: HistoryAction| AppAction::Session(SessionAction::History(action));
        let label = match (h.viewing(), h.newest()) {
            (Some(v), newest) => format!("v{} of {}", v.number, newest.unwrap_or(v.number)),
            (None, Some(newest)) => format!("v{newest} of {newest}"),
            (None, None) => "Current".to_owned(),
        };
        row()
            .id("intent-version-stepper")
            .gap(px(2.))
            .px(px(4.))
            .py(px(2.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.card))
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(
                widgets::ghost_in(
                    look,
                    ui,
                    "intent-history-back",
                    "",
                    act(HistoryAction::Back {
                        path: path.to_owned(),
                    }),
                )
                .icon(IconName::ChevronLeft)
                .w(px(20.))
                .h(px(20.))
                .px_0()
                .accessibility_label("Older version")
                .disabled(!h.can_back()),
            )
            .child(div().px(px(4.)).child(label))
            .child(
                widgets::ghost_in(
                    look,
                    ui,
                    "intent-history-forward",
                    "",
                    act(HistoryAction::Forward {
                        path: path.to_owned(),
                    }),
                )
                .icon(IconName::ChevronRight)
                .w(px(20.))
                .h(px(20.))
                .px_0()
                .accessibility_label("Newer version")
                .disabled(!h.can_forward()),
            )
            .test_support()
    });

    row()
        .id("intent-header")
        .h(px(40.))
        .px(px(14.))
        .gap(px(10.))
        .flex_shrink_0()
        .bg(rgb(look.panel))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.))
                .text_color(rgb(look.text))
                .child(file_name(path).to_owned()),
        )
        .child(
            div()
                .id("intent-schema-badge")
                .px(px(8.))
                .py(px(2.))
                .rounded(px(999.))
                .bg(rgb(pill_bg))
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(pill_fg))
                .child(badge_label)
                .test_support(),
        )
        .child(div().flex_1())
        .child(switcher)
        .children(stepper)
}

fn meta_strip(vm: &IntentViewModel, look: Look) -> Option<Stateful<Div>> {
    let Ok((spec, report)) = &vm.parsed else {
        return None;
    };
    let dark = ((look.content >> 8) & 0xff) < 0x80;
    let (schema_bg, schema_fg) = if dark {
        (0x2b2242, 0xc4b5fd)
    } else {
        (0xf1ebff, 0x6b21a8)
    };
    let unresolved = spec.unresolved_open_questions();
    let blocking = report.blocking_open_questions.len();
    let q_label = if unresolved == 1 {
        "1 open question".to_owned()
    } else {
        format!("{unresolved} open questions")
    };
    let q_color = if blocking > 0 {
        crate::theme::ORANGE
    } else {
        look.secondary
    };

    Some(
        row()
            .id("intent-meta-strip")
            .px(px(14.))
            .py(px(8.))
            .gap(px(6.))
            .flex_shrink_0()
            .bg(rgb(look.content))
            .border_b_1()
            .border_color(rgb(look.line))
            .text_size(px(11.5))
            .child(
                div()
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .bg(rgb(schema_bg))
                    .text_color(rgb(schema_fg))
                    .font_weight(FontWeight::MEDIUM)
                    .child(spec.schema.clone()),
            )
            .child(
                div()
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .bg(rgb(look.fill))
                    .text_color(rgb(look.text))
                    .child(format!(
                        "{} · {}",
                        spec.metadata.id,
                        spec.metadata.status.label()
                    )),
            )
            .children(spec.metadata.repos.iter().map(|r| {
                div()
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .bg(rgb(look.fill))
                    .text_color(rgb(look.secondary))
                    .child(r.clone())
            }))
            .child(div().flex_1())
            .child(
                div()
                    .text_color(rgb(q_color))
                    .when(blocking > 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(q_label),
            ),
    )
}

#[allow(clippy::too_many_arguments)]
fn structured_view(
    docs: &Documents,
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    vm: &IntentViewModel,
    read: Option<&VersionText>,
    look: Look,
) -> AnyElement {
    let dark = ((look.content >> 8) & 0xff) < 0x80;
    let mut sections: Vec<AnyElement> = Vec::new();

    let (spec, report) = match &vm.parsed {
        Err(err) => {
            let (bg, fg) = if dark {
                (0x3a1d1d, 0xff8a80)
            } else {
                (0xfdeaea, 0xc92a2a)
            };
            return div()
                .id("intent-parse-error")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .p(px(16.))
                .overflow_y_scroll()
                .child(
                    col()
                        .p(px(14.))
                        .gap(px(6.))
                        .rounded(px(10.))
                        .bg(rgb(bg))
                        .border_1()
                        .border_color(rgb(fg))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(13.))
                                .text_color(rgb(fg))
                                .child("Could not parse intent.yaml"),
                        )
                        .child(
                            div()
                                .font_family("Menlo")
                                .text_size(px(11.5))
                                .text_color(rgb(look.text))
                                .child(err.clone()),
                        ),
                )
                .test_support()
                .into_any_element();
        }
        Ok(pair) => pair,
    };

    if !report.errors.is_empty() {
        let (bg, fg) = if dark {
            (0x3a1d1d, 0xff8a80)
        } else {
            (0xfdeaea, 0xc92a2a)
        };
        sections.push(
            col()
                .id("intent-validation-errors")
                .p(px(12.))
                .gap(px(4.))
                .rounded(px(10.))
                .bg(rgb(bg))
                .border_1()
                .border_color(rgb(fg))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(12.))
                        .text_color(rgb(fg))
                        .child(format!("{} schema validation issue(s)", report.errors.len())),
                )
                .children(report.errors.iter().map(|issue| {
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(look.text))
                        .child(format!("{}: {}", issue.path, issue.message))
                }))
                .test_support()
                .into_any_element(),
        );
    }

    // 2-column grid row 1: OUTCOME | NON-GOALS
    let outcome_card = section_card("OUTCOME", None, look).child(
        div()
            .text_size(px(12.5))
            .line_height(px(18.))
            .text_color(rgb(look.text))
            .child(spec.outcome.clone()),
    );
    let non_goals_card = section_card("NON-GOALS", None, look).child(bullet_list(
        &spec.non_goals,
        "None specified",
        look,
    ));
    sections.push(
        row()
            .gap(px(12.))
            .items_stretch()
            .child(outcome_card.flex_1().min_w_0())
            .child(non_goals_card.flex_1().min_w_0())
            .into_any_element(),
    );

    // 2-column grid row 2: INVARIANTS | CONSTRAINTS & GUARDRAILS
    let invariants_card = section_card("INVARIANTS", None, look).child(bullet_list(
        &spec.invariants,
        "None specified",
        look,
    ));
    let (fence_bg, fence_fg) = if dark {
        (0x3a1d1d, 0xff6b6b)
    } else {
        (0xfdeaea, 0xc92a2a)
    };
    let hard_fence = div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgb(fence_bg))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(fence_fg))
        .child("HARD FENCE")
        .into_any_element();
    let mut constraint_items: Vec<String> = spec.constraints.forbidden_actions.clone();
    constraint_items.extend(spec.constraints.approval_boundaries.iter().cloned());
    let constraints_card = section_card("CONSTRAINTS & GUARDRAILS", Some(hard_fence), look).child(
        bullet_list(&constraint_items, "No constraints specified", look),
    );
    sections.push(
        row()
            .gap(px(12.))
            .items_stretch()
            .child(invariants_card.flex_1().min_w_0())
            .child(constraints_card.flex_1().min_w_0())
            .into_any_element(),
    );

    // STEPS card + inline comments
    let steps_count = div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(format!(
            "{} {}",
            spec.steps.len(),
            if spec.steps.len() == 1 { "step" } else { "steps" }
        ))
        .into_any_element();
    let (badge_bg, badge_fg) = if dark {
        (0x1c2a3f, 0x60a5fa)
    } else {
        (0xeaf2ff, 0x0a6cff)
    };
    let mut steps_card = section_card(
        "STEPS · MAPS 1:1 TO TO-DO MILESTONE",
        Some(steps_count),
        look,
    );
    for (idx, step) in spec.steps.iter().enumerate() {
        steps_card = steps_card.child(
            row()
                .gap(px(10.))
                .items_start()
                .child(
                    div()
                        .w(px(20.))
                        .h(px(20.))
                        .flex_shrink_0()
                        .rounded(px(6.))
                        .bg(rgb(badge_bg))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(badge_fg))
                        .child((idx + 1).to_string()),
                )
                .child(
                    col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(look.text))
                                .child(step.title.clone()),
                        )
                        .when(!step.outcome.is_empty(), |c| {
                            c.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(look.secondary))
                                    .child(step.outcome.clone()),
                            )
                        })
                        .when(!step.verify.is_empty(), |c| {
                            c.child(
                                div()
                                    .font_family("Menlo")
                                    .text_size(px(11.))
                                    .text_color(rgb(look.secondary))
                                    .child(format!("verify: {}", step.verify)),
                            )
                        }),
                ),
        );
    }

    // Inline comments on the file (recorded if old version, live otherwise).
    let by_line = match read {
        Some(r) => crate::doc_history::recorded_by_line(r),
        None => state.comments.on_file(&session.id, repo, path),
    };
    let editing = state
        .comments
        .sessions
        .get(&session.id)
        .and_then(|held| held.editing.as_ref())
        .filter(|b| b.anchor.repo == repo && b.anchor.path == path)
        .filter(|_| read.is_none());
    let agent = session.agent.command();
    let reattaching = docs.reattach.borrow().clone();
    let cell = docs.reattach.clone();
    let start: StartReattach = Rc::new(move |id, cx| {
        *cell.borrow_mut() = Some(id.clone());
        cx.refresh_windows();
    });
    let todo_ctx = comment_cards::todo_context(state, &session.id);

    for comment in by_line.values().flatten().copied() {
        if read.is_some() {
            steps_card = steps_card.child(crate::doc_history::recorded_card(look, comment));
        } else {
            steps_card = steps_card.child(comment_cards::card_with_todo(
                ui,
                look,
                comment,
                agent,
                session.pull_request_review,
                reattaching.as_ref() == Some(&comment.id),
                &start,
                &todo_ctx,
            ));
        }
    }
    if let Some(b) = editing {
        steps_card = steps_card.child(comment_cards::open_box(
            ui,
            look,
            &session.id,
            &b.anchor,
            &docs.comment_input,
            agent,
            &b.text,
            &comment_cards::offers(state, &session.id, &b.anchor.repo),
        ));
    }
    sections.push(steps_card.into_any_element());

    // ACCEPTANCE (GIVEN / WHEN / THEN)
    if !spec.acceptance.is_empty() {
        let ac_color = if dark { 0xc4b5fd } else { 0x6b21a8 };
        let mut ac_card = section_card("ACCEPTANCE (GIVEN / WHEN / THEN)", None, look);
        for ac in &spec.acceptance {
            ac_card = ac_card.child(
                row()
                    .gap(px(8.))
                    .items_start()
                    .text_size(px(12.))
                    .child(
                        div()
                            .font_family("Menlo")
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(ac_color))
                            .flex_shrink_0()
                            .child(ac.id.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(look.text))
                            .child(format!(
                                "Given {}, when {}, then {}.",
                                ac.given, ac.when, ac.then
                            )),
                    ),
            );
        }
        sections.push(ac_card.into_any_element());
    }

    // OPEN QUESTIONS (when non-empty)
    if !spec.open_questions.is_empty() {
        let mut q_card = section_card("OPEN QUESTIONS", None, look);
        for q in &spec.open_questions {
            let (tag_label, tag_color) = if q.resolved {
                ("Resolved", crate::theme::GREEN)
            } else if q.blocking {
                ("Blocking", crate::theme::RED_STATUS)
            } else {
                ("Open", crate::theme::ORANGE)
            };
            q_card = q_card.child(
                col()
                    .gap(px(2.))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family("Menlo")
                                    .text_size(px(11.))
                                    .text_color(rgb(look.secondary))
                                    .child(q.id.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .text_color(rgb(look.text))
                                    .child(q.question.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(tag_color))
                                    .child(tag_label),
                            ),
                    )
                    .when_some(q.default_choice.as_ref(), |c, def| {
                        c.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(format!("Default: {def}")),
                        )
                    }),
            );
        }
        sections.push(q_card.into_any_element());
    }

    div()
        .id("intent-structured")
        .role(Role::Document)
        .aria_label("Structured intent spec")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .p(px(14.))
        .child(col().gap(px(12.)).children(sections))
        .into_any_element()
}

fn section_card(title: &str, right: Option<AnyElement>, look: Look) -> Div {
    col()
        .p(px(12.))
        .gap(px(8.))
        .rounded(px(10.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(look.secondary))
                        .child(title.to_owned()),
                )
                .child(div().flex_1())
                .children(right),
        )
}

fn bullet_list(items: &[String], empty: &str, look: Look) -> Div {
    if items.is_empty() {
        return div()
            .text_size(px(12.))
            .text_color(rgb(look.tertiary))
            .child(empty.to_owned());
    }
    col().gap(px(4.)).children(items.iter().map(|item| {
        div()
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(rgb(look.text))
            .child(format!("• {item}"))
    }))
}

#[allow(clippy::too_many_arguments)]
fn yaml_view(
    docs: &Documents,
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    text: &str,
    read: Option<&VersionText>,
    look: Look,
) -> AnyElement {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let total = lines.len() as u32;
    let by_line = match read {
        Some(r) => crate::doc_history::recorded_by_line(r),
        None => state.comments.on_file(&session.id, repo, path),
    };
    let editing = state
        .comments
        .sessions
        .get(&session.id)
        .and_then(|held| held.editing.as_ref())
        .filter(|b| b.anchor.repo == repo && b.anchor.path == path)
        .filter(|_| read.is_none());
    let box_range = editing.map(|b| (b.anchor.first, b.anchor.last));
    let box_comment = editing.and_then(|b| b.comment.clone());
    let reattaching = docs.reattach.borrow().clone();
    let lit_color = if look.term == 0xffffff {
        0xeaf2ff
    } else {
        0x1c2a3f
    };
    let markers: BTreeSet<u32> = by_line
        .values()
        .flatten()
        .filter(|c| !c.anchor.detached)
        .map(|c| c.anchor.first)
        .chain(box_range.map(|(first, _)| first))
        .collect();
    let agent = session.agent.command();
    let cell = docs.reattach.clone();
    let start: StartReattach = Rc::new(move |id, cx| {
        *cell.borrow_mut() = Some(id.clone());
        cx.refresh_windows();
    });
    let target = Rc::new(Target {
        ui: ui.clone(),
        session: session.id.clone(),
        repo: repo.to_owned(),
        path: path.to_owned(),
        lines: lines.clone(),
        reattach: docs.reattach.clone(),
    });
    let todo_ctx = comment_cards::todo_context(state, &session.id);
    let dark = ((look.term >> 8) & 0xff) < 0x80;
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let syntax = humata2_highlight::highlight_lines_cached(
        humata2_highlight::Language::Yaml,
        &line_refs,
        dark,
    );

    let mut children: Vec<AnyElement> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let n = index as u32 + 1;
        let lit = box_range.is_some_and(|(first, last)| first <= n && n <= last);
        let line_spans = syntax.line(index);
        let click = target.clone();
        let gutter = div()
            .id(SharedString::from(format!("intent-yaml-line-{n}")))
            .flex()
            .flex_shrink_0()
            .when(read.is_none(), move |g| {
                g.role(Role::Button)
                    .aria_label(format!("Comment on line {n}"))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        click.click(n, event.modifiers.shift, cx);
                    })
            })
            .child(
                div()
                    .w(px(34.))
                    .flex_shrink_0()
                    .pr(px(6.))
                    .flex()
                    .justify_end()
                    .text_color(rgb(look.term_dim))
                    .child(n.to_string()),
            )
            .child(
                div()
                    .w(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(lit, |g| g.border_l_2().border_color(rgb(look.accent)))
                    .when(markers.contains(&n), |g| {
                        g.child(chrome::icon(IconName::MessageSquare, look.accent, 12.))
                    }),
            )
            .test_support();

        let text_cell = div()
            .flex_1()
            .min_w_0()
            .pr(px(8.))
            .text_color(rgb(look.term_text));
        let text_cell = if line_spans.is_empty() {
            text_cell.child(line.clone())
        } else {
            text_cell.child(StyledText::new(line.clone()).with_highlights(line_spans.to_vec()))
        };
        children.push(
            div()
                .flex()
                .flex_shrink_0()
                .min_h(px(18.))
                .line_height(px(18.))
                .when(lit, |r| r.bg(rgb(lit_color)))
                .child(gutter)
                .child(text_cell.test_text(line.clone()))
                .into_any_element(),
        );

        let last_line = n == total;
        let mut under: Vec<&Comment> = by_line.get(&n).cloned().unwrap_or_default();
        if last_line {
            for later in by_line.range(n + 1..).map(|(_, cs)| cs) {
                under.extend(later.iter().copied());
            }
        }
        for comment in under {
            if Some(&comment.id) == box_comment.as_ref() {
                continue;
            }
            children.push(if read.is_some() {
                crate::doc_history::recorded_card(look, comment).into_any_element()
            } else {
                comment_cards::card_with_todo(
                    ui,
                    look,
                    comment,
                    agent,
                    session.pull_request_review,
                    reattaching.as_ref() == Some(&comment.id),
                    &start,
                    &todo_ctx,
                )
                .into_any_element()
            });
        }
        if let (Some(b), Some((_, last))) = (editing, box_range)
            && (last == n || (last_line && last > total))
        {
            children.push(
                comment_cards::open_box(
                    ui,
                    look,
                    &session.id,
                    &b.anchor,
                    &docs.comment_input,
                    agent,
                    &b.text,
                    &comment_cards::offers(state, &session.id, &b.anchor.repo),
                )
                .into_any_element(),
            );
        }
    }

    div()
        .id("intent-yaml")
        .role(Role::Code)
        .aria_label("Intent spec YAML")
        .test_support()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .py(px(10.))
        .overflow_y_scroll()
        .bg(rgb(look.term))
        .font_family("Menlo")
        .text_size(px(11.5))
        .text_color(rgb(look.term_text))
        .children(children)
        .into_any_element()
}

fn diff_view(
    state: &AppState,
    _repo: &str,
    path: &str,
    vm: &IntentViewModel,
    look: Look,
) -> AnyElement {
    let raw_lines: Option<&[DiffLine]> = state
        .session
        .compare
        .as_ref()
        .filter(|c| c.path == path)
        .and_then(|c| match &c.diff {
            CompareDiff::Loaded(lines) if !lines.is_empty() => Some(lines.as_slice()),
            _ => None,
        });

    let summary = vm.diff.as_ref().map(IntentDiff::summary_lines);
    let has_semantic = summary.as_ref().is_some_and(|s| !s.is_empty());

    if !has_semantic && raw_lines.is_none() {
        return div()
            .id("intent-diff-empty")
            .role(Role::Region)
            .aria_label("Intent diff")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.5))
            .text_color(rgb(look.secondary))
            .child("No previous version to compare yet")
            .into_any_element();
    }

    let mut sections: Vec<AnyElement> = Vec::new();
    if let Some(lines) = summary.filter(|s| !s.is_empty()) {
        sections.push(
            section_card("SEMANTIC CHANGES", None, look)
                .child(bullet_list(&lines, "No semantic changes", look))
                .into_any_element(),
        );
    }
    if let Some(lines) = raw_lines {
        let rows: Vec<AnyElement> = lines
            .iter()
            .map(|line| {
                let (sign, tint) = match line.kind {
                    LineKind::Added => ("+", Some(look.add)),
                    LineKind::Removed => ("−", Some(look.del)),
                    LineKind::Hunk => ("@", None),
                    _ => (" ", None),
                };
                row()
                    .flex_shrink_0()
                    .min_h(px(18.))
                    .px(px(8.))
                    .when_some(tint, |r, t| r.bg(rgb(t)))
                    .child(div().w(px(14.)).child(sign))
                    .child(div().whitespace_nowrap().child(line.text.clone()))
                    .into_any_element()
            })
            .collect();
        sections.push(
            col()
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(look.term))
                .py(px(6.))
                .font_family("Menlo")
                .text_size(px(11.5))
                .text_color(rgb(look.term_text))
                .children(rows)
                .into_any_element(),
        );
    }

    div()
        .id("intent-diff")
        .role(Role::Region)
        .aria_label("Intent diff")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .p(px(14.))
        .child(col().gap(px(12.)).children(sections))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn action_bar(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    text: &str,
    vm: &IntentViewModel,
    history: Option<&DocHistory>,
    look: Look,
) -> Stateful<Div> {
    let add_note = {
        let ui = ui.clone();
        let session_id = session.id.clone();
        let (repo, path) = (repo.to_owned(), path.to_owned());
        let first_line = text.lines().next().unwrap_or_default().to_owned();
        widgets::push_quiet_shape("intent-add-note", "Add note", look)
            .disabled(vm.read_only)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let anchor = Anchor {
                    repo: repo.clone(),
                    path: path.clone(),
                    side: Side::New,
                    first: 1,
                    last: 1,
                    quoted: if first_line.is_empty() {
                        Vec::new()
                    } else {
                        vec![first_line.clone()]
                    },
                    base: None,
                    detached: false,
                    outdated: false,
                };
                ui.send(
                    AppAction::Comments(CommentsAction::OpenBox(session_id.clone(), anchor)),
                    cx,
                );
            })
    };

    let version_id = history
        .and_then(|h| h.versions.first())
        .map(|v| v.id.clone());

    let approve_payload = vm.parsed.as_ref().ok().map(|(spec, _)| {
        let brief = spec.to_milestone_brief(version_id.as_deref());
        let title = spec.metadata.title.clone();
        let channel_id = state
            .workspace
            .channels
            .iter()
            .find(|c| c.name == spec.metadata.channel || c.id.as_str() == spec.metadata.channel)
            .map(|c| c.id.clone());
        let todo_item = spec.metadata.todo_key.as_deref().and_then(|key| {
            let exists = state.workspace.items.iter().any(|i| i.id.as_str() == key)
                || state.todo.items.iter().any(|e| e.todo.key == key);
            exists.then(|| ItemId::new(key))
        });
        (title, brief, channel_id, todo_item)
    });

    let approve_btn = {
        let ui = ui.clone();
        widgets::push_primary_shape("intent-approve-start", "Approve & Start Milestone", look)
            .disabled(!vm.can_approve)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let Some((title, brief, channel_id, todo_item)) = approve_payload.clone() else {
                    return;
                };
                if let Some(item_id) = todo_item {
                    ui.send(AppAction::StartNewForItem(item_id), cx);
                } else {
                    ui.send(AppAction::StartNewSession, cx);
                }
                ui.send(AppAction::NewSession(NewSessionAction::Title(title)), cx);
                if let Some(ch_id) = channel_id {
                    ui.send(AppAction::NewSession(NewSessionAction::Channel(ch_id)), cx);
                }
                ui.send(
                    AppAction::NewSession(NewSessionAction::FirstMessage(brief)),
                    cx,
                );
            })
    };

    let hint = vm
        .approval_disabled_reason()
        .unwrap_or_else(|| "Creates milestone + pre-fills session".to_owned());

    row()
        .id("intent-action-bar")
        .h(px(46.))
        .px(px(14.))
        .gap(px(10.))
        .flex_shrink_0()
        .bg(rgb(look.panel))
        .border_t_1()
        .border_color(rgb(look.line))
        .child(add_note)
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(hint),
        )
        .child(approve_btn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    const VALID_YAML: &str = r#"schema: humata.intent/v1
metadata:
  id: PAY-14
  title: Cursor pagination on GET /users
  status: draft
  channel: payments
  repos:
    - acme/api
outcome: Replace offset pagination on GET /users with opaque keyset cursors.
non_goals:
  - No offset-based fallback
invariants:
  - Stable ordering; zero duplicate rows across pages
constraints:
  forbidden_actions:
    - No new crates in Cargo.toml
  approval_boundaries:
    - Push to open PR still asks in Inbox
touch_points: []
steps:
  - id: S1
    title: Opaque cursor codec
    outcome: Encode and decode opaque base64url cursors
    depends_on: []
    crates:
      - api
    verify: bq //api/users:cursor_codec_test
acceptance:
  - id: AC-1
    given: 10 users
    when: limit=2
    then: returns 2 rows and non-null next_cursor
open_questions: []
"#;

    #[test]
    fn valid_spec_enables_approve_and_shows_valid_badge() {
        let vm = IntentViewModel::from_yaml(VALID_YAML, None, false);
        assert!(vm.parsed.is_ok());
        assert!(vm.can_approve);
        assert_eq!(vm.schema_badge(), (true, "✓ Schema valid".to_owned()));
        assert_eq!(vm.approval_disabled_reason(), None);
    }

    #[test]
    fn blocking_open_question_disables_approve() {
        let yaml = VALID_YAML.replace(
            "open_questions: []",
            "open_questions:\n  - id: Q1\n    question: Drop offset?\n    blocking: true\n    resolved: false",
        );
        let vm = IntentViewModel::from_yaml(&yaml, None, false);
        assert!(vm.parsed.is_ok());
        assert!(!vm.can_approve);
        assert!(vm
            .approval_disabled_reason()
            .is_some_and(|r| r.contains("blocking")));
    }

    #[test]
    fn schema_error_disables_approve_and_sets_error_badge() {
        let yaml = VALID_YAML.replace("outcome: Replace offset pagination on GET /users with opaque keyset cursors.", "outcome: \"\"");
        let vm = IntentViewModel::from_yaml(&yaml, None, false);
        assert!(vm.parsed.is_ok());
        assert!(!vm.can_approve);
        let (ok, label) = vm.schema_badge();
        assert!(!ok);
        assert!(label.contains("schema error"));
    }

    #[test]
    fn read_only_old_version_disables_approve() {
        let vm = IntentViewModel::from_yaml(VALID_YAML, None, true);
        assert!(vm.parsed.is_ok());
        assert!(!vm.can_approve);
        assert!(vm
            .approval_disabled_reason()
            .is_some_and(|r| r.contains("read-only")));
    }

    #[test]
    fn diff_mode_computes_semantic_diff_against_previous_version() {
        let updated = VALID_YAML.replace("status: draft", "status: approved");
        let vm = IntentViewModel::from_yaml(&updated, Some(VALID_YAML), false);
        let diff = vm.diff.expect("semantic diff computed");
        assert!(!diff.is_empty());
        assert!(!diff.summary_lines().is_empty());
    }

    #[test]
    fn document_mode_maps_to_intent_modes() {
        assert_eq!(IntentMode::from(DocumentMode::Preview), IntentMode::Structured);
        assert_eq!(IntentMode::from(DocumentMode::Source), IntentMode::Yaml);
        assert_eq!(IntentMode::from(DocumentMode::SideBySide), IntentMode::Diff);
    }
}
