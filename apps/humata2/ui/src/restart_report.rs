//! Home after a restart (frame 47): the sessions that couldn't be resumed and
//! the ones that came back idle, above the usual groups.
use std::rc::Rc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route, restart_groups};
use humata2_clients::model::Session;

use crate::{
    bridge::Ui,
    chrome,
    resume_failed::clock,
    theme::Look,
    widgets::{col, row},
};

fn restart_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    group: &'static str,
    index: usize,
    note: String,
    look: Look,
) -> impl IntoElement {
    let id = session.id.clone();
    let open = ui.clone();
    let button = ui.clone();
    let target = session.id.clone();
    let place = humata2_clients::desk::place(&state.workspace, &session.channel);
    let host = state
        .workspace
        .machine(&session.machine)
        .map_or("", |m| m.name.as_str());
    let at = state
        .workspace
        .restarts
        .iter()
        .find(|restart| restart.machine == session.machine)
        .map(|restart| clock(restart.at_unix_ms))
        .unwrap_or_default();
    row()
        .id((group, index))
        .role(Role::Link)
        .aria_label(session.title.clone())
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(rgb(look.line))
        .cursor_pointer()
        .child(chrome::status(session.status))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(session.title.clone()))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(format!("{place} · {host}")),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(look.secondary))
                        .child(note),
                ),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(at),
        )
        .child(
            Button::new(SharedString::from(format!("{group}-open-{index}")))
                .label("Open")
                .small()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    button.send(AppAction::Navigate(Route::Session(target.clone())), cx);
                }),
        )
        .on_click(move |_, _, cx| open.send(AppAction::Navigate(Route::Session(id.clone())), cx))
}

fn group(
    ui: &Rc<Ui>,
    state: &AppState,
    title: String,
    id: &'static str,
    sessions: &[&Session],
    note: impl Fn(&Session) -> String,
    look: Look,
) -> Div {
    col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(title),
        )
        .child(
            col()
                .w_full()
                .rounded(px(9.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(look.card))
                .overflow_hidden()
                .children(sessions.iter().enumerate().map(|(index, session)| {
                    restart_row(ui, state, session, id, index, note(session), look)
                })),
        )
}

/// The restart groups for the top of Home; empty when nothing needs saying.
pub(crate) fn home_groups(ui: &Rc<Ui>, state: &AppState, look: Look) -> Vec<Div> {
    let groups = restart_groups(&state.workspace);
    let mut out = Vec::new();
    if !groups.couldnt_resume.is_empty() {
        out.push(group(
            ui,
            state,
            format!(
                "Couldn't resume after the restart · {}",
                groups.couldnt_resume.len()
            ),
            "home-couldnt-resume",
            &groups.couldnt_resume,
            |_| "Couldn't resume. Open it to try again or start fresh with a summary.".to_owned(),
            look,
        ));
    }
    if !groups.idle.is_empty() {
        out.push(group(
            ui,
            state,
            format!("Idle after the restart · {}", groups.idle.len()),
            "home-idle-after-restart",
            &groups.idle,
            |_| "Came back idle after its host restarted. If it was working, tell it to continue.".to_owned(),
            look,
        ));
    }
    out
}
