//! The Journal panel: the session family's prepared views.
use super::*;

pub(super) fn journal(ui: &Rc<Ui>, state: &AppState, full: bool, look: Look) -> Div {
    // VW-10: every view prepared in the session and its forks, newest
    // first (the workspace lists them newest first).
    let views: Vec<_> = state
        .session
        .open
        .as_ref()
        .map(|open| {
            let family: Vec<&SessionId> = std::iter::once(open)
                .chain(state.workspace.forks(open).map(|fork| &fork.id))
                .collect();
            state
                .workspace
                .views
                .iter()
                .filter(|view| family.contains(&&view.session))
                .collect()
        })
        .unwrap_or_default();
    col()
        .size_full()
        .child(header(ui, Panel::Journal, full, None, look))
        .child(
            col()
                .id("session-journal")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .gap(px(8.))
                .p(px(10.))
                .text_size(px(12.5))
                .children(views.iter().map(|view| {
                    let note = if view.note.is_empty() {
                        view.prepared.clone()
                    } else if view.prepared.is_empty() {
                        view.note.clone()
                    } else {
                        format!("{} · {}", view.note, view.prepared)
                    };
                    card(look)
                        .p(px(10.))
                        .gap(px(2.))
                        .child(
                            row()
                                .gap(px(6.))
                                .child(icon(IconName::Sparkles, 0xbf5af2, 12.))
                                .child(text(view.title.clone()).font_weight(FontWeight::SEMIBOLD)),
                        )
                        .when(!note.is_empty(), |card| card.child(secondary(look, note)))
                }))
                .when(views.is_empty(), |list| {
                    list.child(secondary(
                        look,
                        "No views yet. Ask the agent to prepare one, such as \"prepare the PR review for me\".",
                    ))
                }),
        )
}
