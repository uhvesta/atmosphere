//! Prepared views on the session page: the offer bar (VW-2), the Showing
//! bar (VW-8) and the view tab with its steps (VW-6).
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, TestSupportExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Agent, Session, ViewId, ViewState};
use humata2_clients::views::{ViewContent, ViewRecord};
use humata2_views::ViewsAction;

use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{self, col, row},
};

/// The state in the Journal's words (VW-10).
pub(crate) fn state_words(state: ViewState) -> &'static str {
    match state {
        ViewState::Draft => "Draft",
        ViewState::Waiting => "Waiting for you",
        ViewState::Later => "Later",
        ViewState::NotNow => "Not now",
        ViewState::Opened => "Opened",
        ViewState::Reverted => "Reverted",
        ViewState::OpenedAndClosed => "Opened and closed",
        ViewState::Kept => "Kept",
    }
}

/// What kind of view it is, in a word.
pub(crate) fn kind_words(content: &ViewContent) -> &'static str {
    match content {
        ViewContent::Walkthrough(_) => "Review",
        ViewContent::Board(_) | ViewContent::SessionBoard { .. } => "Board",
        ViewContent::PullRequest => "Pull request",
        ViewContent::ReviewPlan(_) => "Review plan",
        ViewContent::Summary => "View",
    }
}

/// Who prepared it: the name the daemon gave, else the session's agent.
fn prepared_by(record: &ViewRecord, session: &Session) -> String {
    if !record.prepared_by.is_empty() {
        record.prepared_by.clone()
    } else if session.agent == Agent::Claude {
        "Claude Code".to_owned()
    } else {
        session.agent.name().to_owned()
    }
}

fn bar(look: Look) -> Div {
    row()
        .h(px(40.))
        .items_center()
        .px(px(14.))
        .gap(px(10.))
        .flex_shrink_0()
        .bg(rgb(if look.term == 0xffffff {
            0xfff8e7
        } else {
            0x352d1d
        }))
        .border_b_1()
        .border_color(rgb(look.line))
        .text_size(px(12.5))
        .child(chrome::icon(IconName::Sparkles, 0xbf5af2, 12.))
}

fn quiet_link(
    ui: &Rc<Ui>,
    id: &'static str,
    label: &'static str,
    action: ViewsAction,
    look: Look,
) -> impl IntoElement {
    widgets::ghost_in(look, ui, id, label, AppAction::Views(action))
        .px(px(0.))
        .bg(transparent_black())
        .border_color(transparent_black())
        .text_color(rgb(look.secondary))
}

/// The Showing bar (VW-8): the view on screen, with Revert and Keep. The
/// session page and the board page both draw it.
pub(crate) fn showing_bar(
    ui: &Rc<Ui>,
    state: &AppState,
    record: &ViewRecord,
    look: Look,
) -> AnyElement {
    let by = match state.workspace.session(&record.view.session) {
        Some(session) => prepared_by(record, session),
        None => record.prepared_by.clone(),
    };
    bar(look)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(rgb(look.text))
                .child(format!(
                    "Showing “{}”, prepared by {}",
                    record.view.title,
                    by
                )),
        )
        .child(widgets::push_quiet(
            ui,
            "view-revert",
            "Revert",
            AppAction::Views(ViewsAction::Revert),
            look,
        ))
        .child(widgets::push_primary(
            ui,
            "view-keep",
            "Keep",
            AppAction::Views(ViewsAction::Keep),
            look,
        ))
        .into_any_element()
}

/// The bars above a session's tabs: the offer waiting for the person, the
/// view showing, and the last answer that failed.
pub(crate) fn bars(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Vec<AnyElement> {
    let mut bars = Vec::new();
    if let Some(offer) = state.views.offers(&session.id).into_iter().next() {
        let id = offer.id.clone();
        let summary = offer.view.note.lines().next().unwrap_or_default().to_owned();
        bars.push(
            bar(look)
                .child(
                    col()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .whitespace_nowrap()
                                .text_color(rgb(look.text))
                                .child(format!(
                                    "{} in “{}” prepared a view for you: {}",
                                    prepared_by(offer, session),
                                    session.title,
                                    offer.view.title
                                )),
                        )
                        .when(!summary.is_empty(), |c| {
                            c.child(
                                div()
                                    .whitespace_nowrap()
                                    .text_size(px(11.))
                                    .text_color(rgb(look.secondary))
                                    .child(summary),
                            )
                        }),
                )
                .child(widgets::push_quiet(
                    ui,
                    "offer-open",
                    "Open it",
                    AppAction::Views(ViewsAction::Open(id.clone())),
                    look,
                ))
                .child(quiet_link(ui, "offer-later", "Later", ViewsAction::Later(id.clone()), look))
                .child(quiet_link(ui, "offer-dismiss", "Not now", ViewsAction::NotNow(id), look))
                .into_any_element(),
        );
    }
    let showing = state
        .views
        .showing
        .as_ref()
        .and_then(|showing| state.views.record(&showing.id))
        .filter(|record| record.view.session == session.id);
    if let Some(record) = showing {
        bars.push(showing_bar(ui, state, record, look));
    }
    if let Some(error) = state.views.error.clone() {
        bars.push(
            widgets::error_banner_in(
                look,
                ui,
                "views-error",
                error,
                AppAction::Views(ViewsAction::DismissError),
            )
            .into_any_element(),
        );
    }
    bars
}

/// The tab's label: the view's title, or its id while the record is not
/// known yet.
pub(crate) fn tab_label(state: &AppState, id: &str) -> String {
    state
        .views
        .record(&ViewId::new(id))
        .map_or_else(|| id.to_owned(), |record| record.view.title.clone())
}

/// One stop of the view: where it points and what the agent says.
struct Step {
    place: String,
    note: String,
}

fn steps(record: &ViewRecord) -> Vec<Step> {
    match &record.content {
        ViewContent::Walkthrough(steps) => steps
            .iter()
            .map(|step| Step {
                place: format!("{} {}", step.repo, step.path),
                note: step.note.clone(),
            })
            .collect(),
        ViewContent::ReviewPlan(plan) => plan
            .groups
            .iter()
            .map(|group| Step {
                place: group.title.clone(),
                note: group.note.clone(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn pick(ui: &Rc<Ui>, id: String, action: ViewsAction) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id(SharedString::from(id))
        .role(Role::Button)
        .cursor_pointer()
        .on_click(move |_, _, cx| ui.send(AppAction::Views(action.clone()), cx))
}

/// The content of `Tab::View(id)`: the title, who prepared it and when, the
/// summary and numbered steps on the left; the current step's file and the
/// agent's note on the right, with Prev and Next.
pub(crate) fn body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    id: &str,
    look: Look,
) -> AnyElement {
    let view_id = ViewId::new(id);
    let Some(record) = state.views.record(&view_id) else {
        return col()
            .id("prepared-view-content")
            .flex_1()
            .p(px(18.))
            .bg(rgb(look.content))
            .text_color(rgb(look.secondary))
            .child("This view is not here any more.")
            .test_support()
            .into_any_element();
    };
    let steps = steps(record);
    let last = steps.len().saturating_sub(1);
    let current = state.view_step.get(&view_id).copied().unwrap_or(0).min(last);
    let when = if record.view.prepared.is_empty() {
        String::new()
    } else {
        format!(" · {}", record.view.prepared)
    };
    let left = col()
        .id("prepared-view-steps")
        .w(px(300.))
        .flex_shrink_0()
        .h_full()
        .overflow_y_scroll()
        .p(px(18.))
        .gap(px(10.))
        .border_r_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(record.view.title.clone()),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child(format!("prepared by {}{when}", prepared_by(record, session))),
        )
        .child(div().child(record.view.body.clone()))
        .when(!steps.is_empty(), |c| {
            c.child(
                div()
                    .pt(px(8.))
                    .border_t_1()
                    .border_color(rgb(look.line))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.secondary))
                    .child(format!("Steps · {}", steps.len())),
            )
        })
        .children(steps.iter().enumerate().map(|(n, step)| {
            pick(
                ui,
                format!("view-step-{n}"),
                ViewsAction::Step {
                    id: view_id.clone(),
                    to: n,
                },
            )
            .p(px(8.))
            .rounded(px(6.))
            .when(n == current, |s| s.bg(rgb(look.fill)))
            .child(
                div()
                    .text_size(px(12.5))
                    .child(format!("{}. {}", n + 1, step.place)),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child(step.note.clone()),
            )
        }));
    let right = steps.get(current).map(|step| {
        col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                row()
                    .h(px(34.))
                    .px(px(14.))
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .text_size(px(12.5))
                    .child(div().child(step.place.clone()))
                    .child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child(format!("Step {} of {}", current + 1, steps.len())),
                    ),
            )
            .child(
                col()
                    .flex_1()
                    .min_h_0()
                    .p(px(14.))
                    .child(
                        col()
                            .p(px(12.))
                            .gap(px(4.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(look.line))
                            .child(
                                row()
                                    .gap(px(6.))
                                    .items_center()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(chrome::icon(IconName::Sparkles, 0xbf5af2, 12.))
                                    .child(prepared_by(record, session)),
                            )
                            .child(div().child(step.note.clone())),
                    ),
            )
            .child(
                row()
                    .h(px(46.))
                    .px(px(14.))
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(look.line))
                    .child(
                        widgets::push_quiet(
                            ui,
                            "view-prev",
                            "‹ Prev",
                            AppAction::Views(ViewsAction::Step {
                                id: view_id.clone(),
                                to: current.saturating_sub(1),
                            }),
                            look,
                        )
                        .disabled(current == 0),
                    )
                    .child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child(format!("{} / {}", current + 1, steps.len())),
                    )
                    .child(
                        widgets::push_primary(
                            ui,
                            "view-next",
                            "Next ›",
                            AppAction::Views(ViewsAction::Step {
                                id: view_id.clone(),
                                to: current + 1,
                            }),
                            look,
                        )
                        .disabled(current >= last),
                    ),
            )
    });
    row()
        .id("prepared-view-content")
        .flex_1()
        .min_h_0()
        .bg(rgb(look.content))
        .text_color(rgb(look.text))
        .child(left)
        .children(right)
        .test_support()
        .into_any_element()
}
