//! The session's To-do panel: the To-do list narrowed to one session, at
//! panel width (frames b9-3 and b4-1). An item with a pending ask opens in
//! its drawer by itself, so the question is the first thing seen.

use crate::{
    chrome,
    theme::Look,
    todo::{self, Inputs, Send, Sessions},
    widgets::{col, push_primary_shape, push_quiet_shape, row},
};
use gpui_kit::{prelude::*, *};
use humata2_clients::todo::Entry;
use humata2_todo::{Row, State, Todo};
use humata2_todo_feature::{TodoAction as A, TodoState};

/// The items that belong to `session`, closed ones included.
pub(crate) fn session_entries<'a>(state: &'a TodoState, session: &str) -> Vec<&'a Entry> {
    state
        .items
        .iter()
        .filter(|e| e.todo.sessions.iter().any(|s| s == session))
        .collect()
}

/// The item the panel shows in its drawer: the one the person opened, else
/// the only item with a question waiting.
pub(crate) fn open_key(state: &TodoState, session: &str) -> Option<String> {
    let mine = session_entries(state, session);
    if let Some(key) = state.open.as_deref()
        && mine.iter().any(|e| e.todo.key == key)
    {
        return Some(key.to_owned());
    }
    let mut asking = mine.iter().filter(|e| e.ask_id.is_some() && !e.todo.suggested);
    match (asking.next(), asking.next()) {
        (Some(only), None) => Some(only.todo.key.clone()),
        _ => None,
    }
}

/// The session's open items in rank order, as the page ranks them.
pub(crate) fn ranked_keys(state: &TodoState, session: &str) -> Vec<Row> {
    let items: Vec<Todo> = session_entries(state, session)
        .into_iter()
        .map(|e| e.todo.clone())
        .filter(|t| t.state != State::Closed)
        .collect();
    let ranked = humata2_todo::rank(&items, state.today);
    [ranked.needs_attention, ranked.in_progress, ranked.next]
        .into_iter()
        .flatten()
        .collect()
}

/// The way back to the session's list while an item is open in the
/// drawer: "‹ To-do", then the item's key and channel (frame 191). Beside
/// the terminal the shell draws it as the panel header; `None` while the
/// list shows.
pub fn back(send: &Send, state: &TodoState, session: &str, look: Look) -> Option<Div> {
    let key = open_key(state, session)?;
    let e = state.items.iter().find(|e| e.todo.key == key)?;
    Some(
        row()
            .gap(px(8.))
            .min_w_0()
            .child(
                div()
                    .id("todo-panel-back")
                    .role(Role::Button)
                    .aria_label("Back to the To-do list")
                    .cursor_pointer()
                    .flex_shrink_0()
                    .text_color(rgb(look.accent))
                    .child("‹ To-do")
                    .on_click({
                        let send = send.clone();
                        move |_, _, cx| send(A::Close, cx)
                    })
                    .test_support(),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child(format!(
                        "{}{}",
                        e.todo.key,
                        e.todo.channel.as_ref().map_or(String::new(), |c| format!(" · {c}"))
                    )),
            ),
    )
}

/// The panel's body; the shell puts its header above it. With
/// `back_in_body`, an open item's [`back`] row leads the body (full screen,
/// where the header is the window's title bar).
pub fn panel(
    send: &Send,
    state: &TodoState,
    session: &str,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    back_in_body: bool,
    look: Look,
) -> Div {
    let open = open_key(state, session)
        .and_then(|k| state.items.iter().find(|e| e.todo.key == k));
    let scroll = |body: Div| {
        col().id("todo-panel").flex_1().min_h_0().overflow_y_scroll().p(px(12.)).child(body)
    };
    match open {
        // Frame 191: the drawer scrolls and its History · Pin · Hide until…
        // row stays at the panel's bottom, a divider above it.
        // It takes what the panel header leaves, so the footer stays in view.
        Some(e) => col()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(scroll(
                col()
                    .gap(px(8.))
                    .when(back_in_body, |body| body.children(back(send, state, session, look)))
                    .child(todo::drawer(send, state, e, inputs, sessions, look, false)),
            ))
            .child(todo::footer(send, state, &e.todo, look).flex_shrink_0().px(px(12.)).py(px(10.))),
        None => col().w_full().flex_1().min_h_0().child(scroll(list(send, state, session, look))),
    }
}

fn list(send: &Send, state: &TodoState, session: &str, look: Look) -> Div {
    let rows = ranked_keys(state, session);
    if rows.is_empty() {
        return chrome::secondary(look, "Nothing on the list for this session yet.");
    }
    col().gap(px(6.)).children(rows.into_iter().filter_map(|r| {
        let e = state.items.iter().find(|e| e.todo.key == r.key)?;
        let t = &e.todo;
        let open = A::Open(t.key.clone());
        let sender = send.clone();
        let chip = todo::due_chip(t, state.today);
        Some(
            col()
                .id(SharedString::from(format!("todo-panel-row {}", t.key)))
                .role(Role::ListItem)
                .aria_label(format!("{} {}", t.key, t.title))
                .cursor_pointer()
                .p(px(9.))
                .gap(px(6.))
                .rounded(px(9.))
                .border_1()
                .border_color(rgb(look.line))
                .bg(rgb(if t.ask.is_some() { look.attention_bg } else { look.card }))
                .on_click(move |_, _, cx| sender(open.clone(), cx))
                .test_support()
                .child(
                    row()
                        .gap(px(7.))
                        .child(div().text_size(px(12.)).text_color(rgb(look.secondary)).child(t.key.clone()))
                        .child(div().flex_1().min_w_0().font_weight(FontWeight::MEDIUM).child(t.title.clone()))
                        .children(chip.map(|c| div().text_size(px(12.)).child(c.text))),
                )
                .children(e.ask_id.as_ref().zip(t.ask.as_ref()).map(|(id, ask)| {
                    let (yes, no) = (send.clone(), send.clone());
                    let (yes_id, no_id) = (id.clone(), id.clone());
                    row()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .child(todo::ask_question(t, ask)),
                        )
                        .child(
                            push_primary_shape(
                                SharedString::from(format!("todo-panel-yes {}", t.key)),
                                "Yes",
                                look,
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                yes(A::Answer(yes_id.clone(), true), cx);
                            }),
                        )
                        .child(
                            push_quiet_shape(
                                SharedString::from(format!("todo-panel-no {}", t.key)),
                                "No",
                                look,
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                no(A::Answer(no_id.clone(), false), cx);
                            }),
                        )
                })),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    // gpui_kit's glob brings in its own `test` attribute; use std's.
    use core::prelude::v1::test;
    use humata2_clients_fakes::todo::FakeTodo;
    use humata2_todo_feature::View;

    fn state() -> TodoState {
        let fake = FakeTodo::frame_b1_1();
        let mut s = TodoState::new(View::default(), fake.today());
        s.items = fake.entries();
        s.loaded = true;
        s
    }

    fn session_with_ask(s: &TodoState) -> (String, String) {
        let e = s
            .items
            .iter()
            .find(|e| e.ask_id.is_some() && !e.todo.suggested && !e.todo.sessions.is_empty())
            .expect("an item with a question and a session");
        (e.todo.sessions[0].clone(), e.todo.key.clone())
    }

    #[test]
    fn a_session_with_one_question_opens_it() {
        let s = state();
        let (session, key) = session_with_ask(&s);
        let asking = session_entries(&s, &session)
            .iter()
            .filter(|e| e.ask_id.is_some() && !e.todo.suggested)
            .count();
        if asking == 1 {
            assert_eq!(open_key(&s, &session), Some(key));
        }
    }

    #[test]
    fn what_the_person_opened_wins_if_it_is_in_the_session() {
        let mut s = state();
        let (session, _) = session_with_ask(&s);
        let other = session_entries(&s, &session)[0].todo.key.clone();
        s.open = Some(other.clone());
        assert_eq!(open_key(&s, &session), Some(other));
        s.open = Some("NOT-THERE".to_owned());
        assert_ne!(open_key(&s, &session), Some("NOT-THERE".to_owned()));
    }

    #[test]
    fn the_list_holds_only_the_sessions_open_items() {
        let s = state();
        let (session, _) = session_with_ask(&s);
        let keys: Vec<_> = ranked_keys(&s, &session).into_iter().map(|r| r.key).collect();
        for key in keys {
            let e = s.items.iter().find(|e| e.todo.key == key).expect("listed item exists");
            assert!(e.todo.sessions.contains(&session));
            assert_ne!(e.todo.state, State::Closed);
        }
    }
}
