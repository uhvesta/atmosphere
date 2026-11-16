//! The Move session dialog (frame 63; MV-1 to MV-3, MV-6).

use super::Overlays;
use super::fork::{card, choice, heading, radio};
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::Session;
use humata2_session::{MoveDraft, SessionAction};
use std::rc::Rc;

impl Overlays {
    /// The Move session dialog.
    pub(super) fn move_dialog(
        &self,
        draft: MoveDraft,
        error: Option<String>,
        session: Option<Session>,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let (machines, health, files) = ui.read(|state| {
            let files = session
                .as_ref()
                .and_then(|s| state.workspace.session_files.get(&s.id))
                .map_or(0, Vec::len);
            (
                state.workspace.machines.clone(),
                state.workspace.machine_health.clone(),
                files,
            )
        });
        let title = session.as_ref().map_or("this session", |s| s.title.as_str());
        let here = session.as_ref().map(|s| s.machine.clone());
        let name_of = |id: &humata2_clients::model::MachineId| {
            machines
                .iter()
                .find(|m| &m.id == id)
                .map_or_else(|| id.to_string(), |m| m.name.clone())
        };
        let from = here.as_ref().map_or_else(String::new, name_of);
        let target = draft.to.as_ref().map(name_of);
        let rows = machines.iter().map(|m| {
            let is_here = here.as_ref() == Some(&m.id);
            let choosable = m.online && !is_here;
            let on = draft.to.as_ref() == Some(&m.id);
            let note = if is_here {
                "here now".to_owned()
            } else if !m.online {
                "offline".to_owned()
            } else {
                health.get(&m.id).map_or_else(
                    || "online".to_owned(),
                    |h| format!("online · {} GB free", h.disk_free_gb),
                )
            };
            choice(
                ui,
                SharedString::from(format!("move-machine-{}", m.id)),
                look,
                on,
                !choosable,
                AppAction::Session(SessionAction::MovePick(m.id.clone())),
                row()
                    .gap(px(8.))
                    .child(radio(look, on, m.name.clone(), "").flex_1())
                    .child(div().text_color(rgb(look.secondary)).child(note)),
            )
        });
        let branch = session.as_ref().map_or("", |s| s.branch.as_str());
        let shells = session.as_ref().map_or_else(Vec::new, |s| s.shells.clone());
        let shell_line = match shells.as_slice() {
            [] => None,
            [one] => Some(format!(
                "{one} moves as a fresh shell in the same folder; programs running in it don't come along."
            )),
            many => Some(format!(
                "{} move as fresh shells in the same folder; programs running in them don't come along.",
                many.join(", ")
            )),
        };
        let note = |glyph: IconName, text: String| {
            row()
                .items_start()
                .gap(px(8.))
                .child(div().mt(px(1.)).child(chrome::icon(glyph, look.secondary, 12.)))
                .child(div().flex_1().child(text))
        };
        let mut body = card("move-sheet", "Move session", 600., look)
            .child(heading(format!("Move “{title}” to another machine")))
            .child(col().gap(px(6.)).children(rows))
            .child(
                col()
                    .id("move-what")
                    .role(Role::Group)
                    .aria_label("What moves")
                    .test_support()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(look.secondary))
                            .child("What moves"),
                    )
                    .child(div().child(format!(
                        "The branch {branch}, {files} changed files and the conversation. The board, Inbox and comments stay."
                    ))),
            )
            .child(
                col()
                    .gap(px(7.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(look.fill))
                    .text_size(px(12.))
                    .children(shell_line.map(|line| note(IconName::SquareTerminal, line)))
                    .child(note(
                        IconName::ShieldCheck,
                        format!(
                            "{from}'s copy is deleted only after the session runs on {}.",
                            target.as_deref().unwrap_or("the new machine")
                        ),
                    )),
            );
        if let Some(error) = error {
            body = body.child(
                div()
                    .id("move-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xff453a))
                    .child(error),
            );
        }
        let body = body.child(
            row()
                .gap(px(8.))
                .pt(px(4.))
                .justify_end()
                .child(
                    widgets::push_quiet(
                        ui,
                        "move-cancel",
                        "Cancel",
                        AppAction::Session(SessionAction::CancelMove),
                        look,
                    )
                    .disabled(draft.sending),
                )
                .child(
                    widgets::push_primary(
                        ui,
                        "move-confirm",
                        match &target {
                            Some(name) => format!("Move to {name}"),
                            None => "Move".to_owned(),
                        },
                        AppAction::Session(SessionAction::ConfirmMove),
                        look,
                    )
                    .disabled(draft.to.is_none() || draft.sending),
                ),
        )
        .test_support()
        .focus_trap("move-trap", &self.sheet);
        div()
            .id("move-dialog")
            .role(Role::Group)
            .aria_label("move dialog")
            .test_support()
            .child(body)
            .into_any_element()
    }
}
