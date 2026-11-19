//! Closing a terminal tab asks what happens to what runs in it (D784): Keep
//! running, Stop (resumable) or End for good. A window-root overlay (D363),
//! 520 wide like frame 72's Stop confirm, whose red it shares.

use super::Overlays;
use super::fork::{card, heading};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::theme::{Look, RED_STATUS};
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_session::{CloseAction, CloseChoice, CloseConfirm, CloseKey, CloseKind, SessionAction};
use std::rc::Rc;

/// The question: "Closing this tab. What should happen to Ledger?"; an
/// untitled shell is "this shell".
pub(crate) fn question(confirm: &CloseConfirm) -> String {
    let what = confirm.title.as_deref().unwrap_or("this shell");
    format!("Closing this tab. What should happen to {what}?")
}

/// The muted line under it: "Copilot on devbox-1", "Shell on This Mac".
pub(crate) fn whose(confirm: &CloseConfirm) -> String {
    format!("{} on {}", confirm.agent, confirm.machine)
}

/// The words after a choice's bold label.
pub(crate) fn explains(confirm: &CloseConfirm, choice: CloseChoice) -> String {
    match choice {
        CloseChoice::KeepRunning => format!(
            "It stays on {m}. Find it on {m}'s page under Hosts, or with Search (\u{2318}K).",
            m = confirm.machine
        ),
        CloseChoice::Stop => "It ends now. Resume it later from the same place.".to_owned(),
        CloseChoice::EndForGood if confirm.kind == CloseKind::Shell => {
            "It ends. It can't be resumed.".to_owned()
        }
        CloseChoice::EndForGood => "It ends and is archived. It can't be resumed.".to_owned(),
    }
}

/// A choice's name, on its button and as the bold label of its line.
pub(crate) const fn name(choice: CloseChoice) -> &'static str {
    match choice {
        CloseChoice::KeepRunning => "Keep running",
        CloseChoice::Stop => "Stop",
        CloseChoice::EndForGood => "End for good",
    }
}

fn send(action: CloseAction) -> AppAction {
    AppAction::Session(SessionAction::Close(action))
}

impl Overlays {
    /// The close-tab dialog.
    pub(super) fn close_dialog(&self, confirm: &CloseConfirm, look: Look, ui: &Rc<Ui>) -> AnyElement {
        let busy = confirm.sending.is_some();
        let choices = confirm.choices();
        let mut body = card("close-dialog", "Close tab", 520., look)
            .track_focus(&self.sheet)
            .child(
                col()
                    .gap(px(3.))
                    .child(heading(question(confirm)))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(whose(confirm)),
                    ),
            )
            .child(col().gap(px(5.)).children(choices.iter().map(|choice| {
                row()
                    .id(("close-line", *choice as usize))
                    .items_start()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name(*choice)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(look.secondary))
                            .child(explains(confirm, *choice)),
                    )
            })));
        if let Some(error) = confirm.error.clone() {
            body = body.child(
                div()
                    .id("close-error")
                    .role(Role::Alert)
                    .text_color(rgb(RED_STATUS))
                    .child(error),
            );
        }
        let on = confirm.always_keep_running;
        let toggle = {
            let ui = ui.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                ui.send(send(CloseAction::AlwaysKeepRunning(!on)), cx);
            }
        };
        body = body.child(
            row()
                .id("close-always")
                .role(Role::CheckBox)
                .aria_label("Always keep running without asking")
                .aria_selected(on)
                .test_support()
                .gap(px(7.))
                .cursor_pointer()
                .on_click(toggle)
                .child(
                    row()
                        .size(px(14.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .justify_center()
                        .when(on, |box_| {
                            box_.bg(rgb(look.accent))
                                .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                        })
                        .when(!on, |box_| {
                            box_.border_1().border_color(rgb(look.tertiary)).bg(rgb(look.card))
                        }),
                )
                .child("Always keep running without asking"),
        );
        let pick = |choice: CloseChoice, id: &'static str| {
            widgets::push_quiet(ui, id, name(choice), send(CloseAction::Choose(choice)), look).disabled(busy)
        };
        let mut buttons = row().gap(px(8.)).pt(px(4.)).justify_end().child(
            widgets::push_quiet(ui, "close-cancel", "Cancel", send(CloseAction::Cancel), look)
                .disabled(busy),
        );
        if choices.contains(&CloseChoice::EndForGood) {
            buttons = buttons
                .child(pick(CloseChoice::EndForGood, "close-end").text_color(rgb(RED_STATUS)));
        }
        if choices.contains(&CloseChoice::Stop) {
            buttons = buttons.child(pick(CloseChoice::Stop, "close-stop"));
        }
        buttons = buttons.child(
            widgets::push_primary(
                ui,
                "close-keep",
                name(CloseChoice::KeepRunning),
                send(CloseAction::Choose(CloseChoice::KeepRunning)),
                look,
            )
            .disabled(busy),
        );
        // Return picks Keep running and Escape cancels, while the dialog
        // itself (not Stop or End for good) has focus.
        let (keys, dismiss, sheet) = (ui.clone(), ui.clone(), self.sheet.clone());
        let body = body
            .child(buttons)
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "enter" && sheet.is_focused(window) {
                    cx.stop_propagation();
                    keys.send(send(CloseAction::Key(CloseKey::Return)), cx);
                }
            })
            .on_action(move |_: &humata2_ui_core::keys::Dismiss, _, cx| {
                cx.stop_propagation();
                dismiss.send(send(CloseAction::Key(CloseKey::Escape)), cx);
            })
            .test_support()
            .focus_trap("close-trap", &self.sheet);
        div()
            .id("close-dialog-root")
            .role(Role::Group)
            .aria_label("close tab dialog")
            .test_support()
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The gpui glob above brings in its own `test` attribute, which expands
    // to `#[test]` again; the built-in one wins over a glob by name.
    use ::core::prelude::v1::test;

    fn confirm(kind: CloseKind, title: Option<&str>, agent: &str, machine: &str) -> CloseConfirm {
        CloseConfirm {
            session: humata2_clients::model::SessionId::new("s"),
            group: 0,
            index: 0,
            tab: 0,
            kind,
            title: title.map(str::to_owned),
            agent: agent.to_owned(),
            machine: machine.to_owned(),
            always_keep_running: false,
            sending: None,
            error: None,
        }
    }

    #[test]
    fn an_untitled_shell_is_this_shell() {
        let shell = confirm(CloseKind::Shell, None, "Shell", "This Mac");
        assert!(question(&shell).ends_with("happen to this shell?"));
        assert_eq!(whose(&shell), "Shell on This Mac");
    }

    #[test]
    fn a_titled_agent_is_named_with_its_machine() {
        let agent = confirm(CloseKind::Agent, Some("Fix retries"), "Copilot", "devbox-1");
        assert!(question(&agent).ends_with("happen to Fix retries?"));
        assert_eq!(whose(&agent), "Copilot on devbox-1");
    }

    #[test]
    fn only_an_agent_end_is_archived() {
        let agent = confirm(CloseKind::Agent, Some("A"), "Claude Code", "This Mac");
        let shell = confirm(CloseKind::Shell, None, "Shell", "This Mac");
        assert!(explains(&agent, CloseChoice::EndForGood).contains("archived"));
        assert!(!explains(&shell, CloseChoice::EndForGood).contains("archived"));
    }

    #[test]
    fn keep_running_names_the_machine_twice() {
        let agent = confirm(CloseKind::Agent, Some("A"), "Claude Code", "devbox-1");
        assert_eq!(explains(&agent, CloseChoice::KeepRunning).matches("devbox-1").count(), 2);
    }
}
