//! The Journal panel: the session family's prepared views.
use crate::panel_prelude::*;
use crate::view_tab::{kind_words, state_words};
use humata2_views::ViewsAction;

pub fn journal(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
    // VW-10: every view prepared in the session and its forks, newest
    // first (the daemon lists them newest first).
    let family: Vec<SessionId> = state
        .session
        .open
        .as_ref()
        .map(|open| {
            std::iter::once(open.clone())
                .chain(state.workspace.forks(open).map(|fork| fork.id.clone()))
                .collect()
        })
        .unwrap_or_default();
    let views = state.views.journal(&family);
    col()
        .size_full()
        .child(panel_header(ui, state, Panel::Journal, full, None, look))
        .child(
            col()
                .id("session-journal")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .p(px(10.))
                .text_size(px(12.5))
                .children(views.iter().enumerate().map(|(n, record)| {
                    let view = &record.view;
                    let mut meta = vec![kind_words(&record.content).to_owned()];
                    // Which session, for a fork.
                    if state.session.open.as_ref() != Some(&view.session) {
                        if let Some(owner) = state.workspace.session(&view.session) {
                            meta.push(format!("in {}", owner.title));
                        }
                    }
                    if !view.prepared.is_empty() {
                        meta.push(view.prepared.clone());
                    }
                    card(look)
                        .p(px(10.))
                        .gap(px(2.))
                        .child(
                            row()
                                .gap(px(6.))
                                .child(icon(IconName::Sparkles, 0xbf5af2, 12.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(text(view.title.clone()).font_weight(FontWeight::SEMIBOLD)),
                                )
                                .child(secondary(look, state_words(record.state))),
                        )
                        .child(secondary(look, meta.join(" · ")))
                        .when(!view.note.is_empty(), |card| {
                            card.child(secondary(look, view.note.clone()))
                        })
                        .child(
                            row().justify_end().pt(px(4.)).child(widgets::push_quiet(
                                ui,
                                SharedString::from(format!("journal-open-{n}")),
                                "Open",
                                AppAction::Views(ViewsAction::Open(record.id.clone())),
                                look,
                            )),
                        )
                }))
                .when(views.is_empty(), |list| {
                    list.child(secondary(
                        look,
                        "No views yet. Ask the agent to prepare one, such as \"prepare the PR review for me\".",
                    ))
                }),
        )
}
