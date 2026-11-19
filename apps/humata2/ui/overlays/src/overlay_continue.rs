//! The Continue in another channel dialog (CV-8; draft frame 113): the Move
//! dialog's choices, here the channels, and the Fork dialog's "Fork from"
//! row as "Continue from".

use super::Overlays;
use super::fork::{card, choice, form_row, heading, popup, radio};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::Session;
use humata2_session::{ContinueDraft, SessionAction};
use std::rc::Rc;

impl Overlays {
    /// The Continue in another channel dialog.
    pub(super) fn continue_dialog(
        &self,
        draft: ContinueDraft,
        error: Option<String>,
        session: Option<Session>,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let send = AppAction::Session;
        let channels = ui.read(|state| state.workspace.channels.clone());
        let title = session.as_ref().map_or("this session", |s| s.title.as_str());
        let here = session.as_ref().map(|s| s.channel.clone());
        let target = draft
            .channel
            .as_ref()
            .and_then(|id| channels.iter().find(|c| &c.id == id))
            .map(|c| c.name.clone());
        let rows = channels.iter().map(|channel| {
            let is_here = here.as_ref() == Some(&channel.id);
            let on = draft.channel.as_ref() == Some(&channel.id);
            let note = if is_here {
                "here now".to_owned()
            } else if channel.repos.is_empty() {
                "no repos".to_owned()
            } else {
                channel.repos.join(", ")
            };
            choice(
                ui,
                SharedString::from(format!("continue-channel-{}", channel.id)),
                look,
                on,
                is_here,
                send(SessionAction::ContinuePick(channel.id.clone())),
                row()
                    .gap(px(8.))
                    .child(radio(look, on, format!("# {}", channel.name), "").flex_1())
                    .child(div().text_color(rgb(look.secondary)).child(note)),
            )
        });
        let from = match draft.before_turn {
            None => "Now".to_owned(),
            Some(turn) => format!("Before turn {turn}"),
        };
        let mut body = card("continue-sheet", "Continue in another channel", 600., look)
            .child(heading(format!("Continue “{title}” in another channel")))
            .child(col().gap(px(6.)).children(rows))
            .child(form_row(
                look,
                "Continue from",
                row()
                    .gap(px(8.))
                    .child(popup(
                        ui,
                        look,
                        "continue-from",
                        from,
                        send(SessionAction::ContinueTurnPicker(!draft.picking_turn)),
                    ))
                    .child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child("or before an earlier turn"),
                    ),
            ));
        if draft.picking_turn {
            let mut options = row().flex_wrap().gap(px(6.)).child(widgets::push_quiet(
                ui,
                "continue-from-now",
                "Now",
                send(SessionAction::ContinueBeforeTurn(None)),
                look,
            ));
            // Nothing comes before the first turn.
            for turn in draft.turns.iter().flatten().filter(|turn| turn.index > 1) {
                let text: String = turn.text.chars().take(40).collect();
                options = options.child(widgets::push_quiet(
                    ui,
                    SharedString::from(format!("continue-from-{}", turn.index)),
                    format!("Before {}: {text}", turn.index),
                    send(SessionAction::ContinueBeforeTurn(Some(turn.index))),
                    look,
                ));
            }
            body = body.child(options);
        }
        let note = |glyph: IconName, text: String| {
            row()
                .items_start()
                .gap(px(8.))
                .child(div().mt(px(1.)).child(chrome::icon(glyph, look.secondary, 12.)))
                .child(div().flex_1().min_w_0().child(text))
        };
        let place = target
            .as_ref()
            .map_or_else(|| "the channel you pick".to_owned(), |name| format!("# {name}"));
        body = body.child(
            col()
                .gap(px(7.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(rgb(look.fill))
                .text_size(px(12.))
                .child(note(
                    IconName::FolderGit2,
                    format!(
                        "A new session in {place}, on its repos and a branch of its own; its agent picks up a copy of the conversation."
                    ),
                ))
                .child(note(IconName::ShieldCheck, format!("“{title}” stays as it is."))),
        );
        if let Some(error) = error {
            body = body.child(
                div()
                    .id("continue-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xff453a))
                    .child(error),
            );
        }
        let body = body
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .justify_end()
                    .child(
                        widgets::push_quiet(
                            ui,
                            "continue-cancel",
                            "Cancel",
                            send(SessionAction::CancelContinue),
                            look,
                        )
                        .disabled(draft.sending),
                    )
                    .child(
                        widgets::push_primary(
                            ui,
                            "continue-confirm",
                            match &target {
                                Some(name) => format!("Continue in # {name}"),
                                None => "Continue".to_owned(),
                            },
                            send(SessionAction::ConfirmContinue),
                            look,
                        )
                        .disabled(draft.channel.is_none() || draft.sending),
                    ),
            )
            .children(
                draft
                    .progress
                    .as_ref()
                    .filter(|_| draft.sending)
                    .map(|p| widgets::progress_line(look, p, Some(ui.now()))),
            )
            .test_support()
            .focus_trap("continue-trap", &self.sheet);
        div()
            .id("continue-dialog")
            .role(Role::Group)
            .aria_label("continue dialog")
            .test_support()
            .child(body)
            .into_any_element()
    }
}
