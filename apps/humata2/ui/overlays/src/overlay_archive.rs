//! The Archive dialog for a session that holds unpushed work (frame 65;
//! MV-7 to MV-9).

use super::Overlays;
use super::fork::{card, heading};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_session::{ArchiveConfirm, SessionAction};
use std::rc::Rc;

const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;

impl Overlays {
    /// The Archive dialog.
    pub(super) fn archive_dialog(
        &self,
        confirm: &ArchiveConfirm,
        error: Option<String>,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let frees = confirm.plan.frees.as_ref().map_or_else(String::new, |size| {
            format!(" It frees about {size} once the Undo window passes.")
        });
        let forks = confirm.plan.forks_keep_running;
        let send = |action: SessionAction| AppAction::Session(action);
        // D784: name the agent it ends and where, as the close dialog does.
        let ends = if confirm.agent.is_empty() {
            "the agent".to_owned()
        } else {
            format!("{} on {}", confirm.agent, confirm.machine)
        };
        let mut body = card("archive-sheet", "Archive session", 660., look)
            .child(heading(format!("Archive “{}”?", confirm.title)))
            .child(div().text_color(rgb(look.secondary)).child(format!(
                "Archiving stops {ends} and keeps the conversation, the record, the scrollback and a summary of the diff.{frees}"
            )));
        if forks > 0 {
            body = body.child(div().child(format!(
                "Its {forks} fork{} keep{} running; {} move{} up in the tree.",
                if forks == 1 { "" } else { "s" },
                if forks == 1 { "s" } else { "" },
                if forks == 1 { "it" } else { "they" },
                if forks == 1 { "s" } else { "" },
            )));
        }
        let drafts = confirm.plan.unsubmitted_drafts;
        if drafts > 0 {
            body = body.child(div().id("archive-drafts").test_support().child(format!(
                "{drafts} draft{} on the pull request {} never submitted; {} kept with the archived review.",
                if drafts == 1 { "" } else { "s" },
                if drafts == 1 { "was" } else { "were" },
                if drafts == 1 { "it's" } else { "they're" },
            )));
        }
        let unpushed = !confirm.plan.unpushed.is_empty();
        if unpushed {
            body = body.child(
            col()
                .gap(px(5.))
                .mt(px(4.))
                .p(px(10.))
                .rounded(px(7.))
                .bg(rgb(look.attention_bg))
                .text_color(rgb(ORANGE))
                .children(confirm.plan.unpushed.iter().map(|work| {
                    row()
                        .items_start()
                        .gap(px(7.))
                        .child(chrome::icon(IconName::GitBranch, ORANGE, 12.))
                        .child(div().flex_1().child(format!("{}: {}", work.repo, work.what)))
                }))
                .child(
                    div()
                        .pl(px(19.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Archiving deletes that work."),
                ),
            );
        }
        if let Some(error) = error {
            body = body.child(
                div()
                    .id("archive-error")
                    .role(Role::Alert)
                    .text_color(rgb(RED))
                    .child(error),
            );
        }
        let busy = confirm.sending;
        let mut buttons = row().gap(px(8.)).pt(px(10.)).justify_end().child(
            widgets::push_quiet(ui, "archive-cancel", "Cancel", send(SessionAction::CancelArchive), look)
                .disabled(busy),
        );
        buttons = if unpushed {
            buttons
                .child(
                    widgets::push_quiet(
                        ui,
                        "archive-delete",
                        "Archive and delete the work",
                        send(SessionAction::ConfirmArchive(false)),
                        look,
                    )
                    .border_color(rgb(RED))
                    .text_color(rgb(RED))
                    .disabled(busy),
                )
                .child(
                    widgets::push_primary(
                        ui,
                        "archive-push",
                        "Commit, push and open a PR, then archive",
                        send(SessionAction::ConfirmArchive(true)),
                        look,
                    )
                    .disabled(busy),
                )
        } else {
            buttons.child(
                widgets::push_primary(
                    ui,
                    "archive-confirm",
                    "Archive",
                    send(SessionAction::ConfirmArchive(false)),
                    look,
                )
                .disabled(busy),
            )
        };
        let body = body.child(buttons)
        .test_support()
        .focus_trap("archive-trap", &self.sheet);
        div()
            .id("archive-dialog")
            .role(Role::Group)
            .aria_label("archive dialog")
            .test_support()
            .child(body)
            .into_any_element()
    }
}
