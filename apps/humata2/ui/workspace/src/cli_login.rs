//! Pending `humata login` approval banner and Inbox cards (SETTINGSFIX).

use std::rc::Rc;

use gpui_kit::base::{Disableable as _, TestSupportExt as _, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::PendingCliLogin;

use crate::request_card;
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{col, row};

/// Human-readable duration for a pending CLI login.
#[must_use]
pub fn duration_label(hours: u32) -> String {
    if hours == 1 {
        "1 hour".to_owned()
    } else {
        format!("{hours} hours")
    }
}

fn login_row(
    ui: &Rc<Ui>,
    login: &PendingCliLogin,
    busy: bool,
    compact: bool,
    look: Look,
) -> AnyElement {
    let duration = duration_label(login.hours);
    let terminal = if login.terminal.is_empty() {
        "External terminal".to_owned()
    } else {
        login.terminal.clone()
    };
    let summary = format!("{terminal} · {duration}");
    let id = login.id.clone();

    let approve = request_card::button(
        ui,
        SharedString::from(format!("cli-login-approve-{}", login.id)),
        if busy { "Approving…" } else { "Approve" },
        vec![AppAction::AnswerCliLogin {
            id: id.clone(),
            allow: true,
        }],
        look,
        true,
    )
    .disabled(busy);

    let deny = request_card::button(
        ui,
        SharedString::from(format!("cli-login-deny-{}", login.id)),
        "Deny",
        vec![AppAction::AnswerCliLogin { id, allow: false }],
        look,
        false,
    )
    .disabled(busy);

    if compact {
        row()
            .id(SharedString::from(format!("cli-login-banner-{}", login.id)))
            .role(Role::Alert)
            .aria_label(format!("CLI login request from {summary}"))
            .test_support()
            .w_full()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(8.))
            .bg(rgb(look.fill))
            .border_b_1()
            .border_color(rgb(look.accent))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        row()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(12.5))
                                    .text_color(rgb(look.text))
                                    .child("CLI login request")
                                    .test_text("CLI login request"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(look.secondary))
                                    .child(summary.clone())
                                    .test_text(summary),
                            ),
                    ),
            )
            .child(row().gap(px(8.)).flex_shrink_0().child(deny).child(approve))
            .into_any_element()
    } else {
        let description = format!(
            "An external terminal ({terminal}) ran `humata login` and is asking for CLI access as you for {duration}."
        );
        col()
            .id(SharedString::from(format!("cli-login-card-{}", login.id)))
            .role(Role::Group)
            .aria_label(format!("CLI login request from {summary}"))
            .test_support()
            .w_full()
            .p(px(14.))
            .gap(px(10.))
            .rounded(px(8.))
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(look.accent))
            .child(
                row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .text_color(rgb(look.text))
                            .child("CLI login request")
                            .test_text("CLI login request"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(look.secondary))
                            .child(summary.clone())
                            .test_text(summary),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child(description.clone())
                    .test_text(description),
            )
            .child(row().gap(px(8.)).justify_end().child(deny).child(approve))
            .into_any_element()
    }
}

/// Top-level banner shown across non-Inbox routes whenever a `humata login` is waiting.
pub fn banner(ui: &Rc<Ui>, state: &AppState, look: Look) -> Option<AnyElement> {
    if state.pending_cli_logins.is_empty() || state.route == Route::Inbox {
        return None;
    }
    Some(
        col()
            .id("cli-login-banner")
            .w_full()
            .flex_shrink_0()
            .children(state.pending_cli_logins.iter().map(|login| {
                let busy = state.answering_cli_logins.contains(&login.id);
                login_row(ui, login, busy, true, look)
            }))
            .into_any_element(),
    )
}

/// Full approval cards shown at the top of the Inbox page whenever a `humata login` is waiting.
pub fn inbox_cards(ui: &Rc<Ui>, state: &AppState, look: Look) -> Option<AnyElement> {
    if state.pending_cli_logins.is_empty() {
        return None;
    }
    Some(
        col()
            .id("inbox-cli-logins")
            .w_full()
            .gap(px(10.))
            .children(state.pending_cli_logins.iter().map(|login| {
                let busy = state.answering_cli_logins.contains(&login.id);
                login_row(ui, login, busy, false, look)
            }))
            .into_any_element(),
    )
}
