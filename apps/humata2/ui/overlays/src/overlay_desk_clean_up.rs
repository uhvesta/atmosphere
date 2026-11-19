//! Clean up… (DK-11, MV-9, NV-27, frame 55): closed sessions, least recently
//! used first, with their folders' sizes.

use super::{ORANGE, Overlays, runs, button, check, header, primary, secondary};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::Workspace;
use humata2_clients::desk::{CleanUpRow, Folder};
use humata2_desk::{CleanUpDialog, DeskAction};
use std::rc::Rc;

/// A size as the frame prints it: "48 MB", "2.3 GB".
fn size(bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes >= GB {
        format!("{}.{} GB", bytes / GB, bytes % GB / (GB / 10))
    } else {
        format!("{} MB", bytes / MB)
    }
}

/// How many sessions the delete button names.
fn sessions(count: usize) -> String {
    match count {
        0 => "sessions".to_owned(),
        1 => "1 session".to_owned(),
        n => format!("{n} sessions"),
    }
}

impl Overlays {
    /// The Clean up… dialog.
    pub(super) fn desk_clean_up(
        &self,
        dialog: &CleanUpDialog,
        workspace: &Workspace,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let rows = dialog
            .rows
            .iter()
            .map(|entry| clean_up_row(ui, look, dialog, workspace, entry));
        let ticked = dialog.ticked.len();
        let freed = dialog.freed_bytes();
        let delete = format!(
            "Delete {}{}",
            sessions(ticked),
            if freed > 0 {
                format!(" · frees {}", size(freed))
            } else {
                String::new()
            }
        );
        let mut content = super::card("desk-clean-up-dialog", "Clean up closed sessions", 700., look)
            .track_focus(&self.sheet)
            .gap(px(6.))
            .child(header(look, "Clean up closed sessions", "Least recently used first"));
        if dialog.changed {
            content = content.child(
                div()
                    .id("desk-clean-up-changed")
                    .role(Role::Alert)
                    .aria_label("The list changed")
                    .test_support()
                    .text_color(rgb(ORANGE))
                    .child(
                        "The closed sessions changed while this was open. Nothing was deleted. \
                         Check the list again and delete.",
                    ),
            );
        }
        if dialog.loading {
            content = content.child(
                div()
                    .py(px(12.))
                    .child(secondary(look, "Listing closed sessions…")),
            );
        } else if dialog.rows.is_empty() && dialog.error.is_none() {
            content = content.child(
                div()
                    .py(px(12.))
                    .child(secondary(look, "No closed sessions to clean up")),
            );
        }
        content
            .children(rows)
            .children(dialog.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "desk-clean-up-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(10.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(look.tertiary))
                            .child("Deleted sessions go to the trash for 30 days"),
                    )
                    .child(div().flex_1())
                    .child(button(
                        ui,
                        look,
                        "desk-clean-up-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(primary(
                        ui,
                        look,
                        "desk-clean-up-delete",
                        delete,
                        AppAction::Desk(DeskAction::ConfirmCleanUp),
                        ticked == 0 || dialog.deleting || dialog.loading,
                    )),
            )
            .focus_trap("desk-clean-up-dialog", &self.sheet)
            .into_any_element()
    }
}

/// One closed session's row, with the notes under it.
fn clean_up_row(
    ui: &Rc<Ui>,
    look: Look,
    dialog: &CleanUpDialog,
    workspace: &Workspace,
    entry: &CleanUpRow,
) -> Div {
    let session = workspace.session(&entry.session);
    let record = workspace.desk.sessions.get(&entry.session);
    let title = session.map_or_else(|| entry.session.to_string(), |s| s.title.clone());
    let ran = session.map_or_else(String::new, |s| {
        let machine = workspace
            .machine(&s.machine)
            .map_or_else(|| s.machine.to_string(), |m| m.name.clone());
        format!("{} · {machine}", runs(s.agent))
    });
    let closed = record
        .and_then(|r| r.closed.as_ref())
        .map_or_else(String::new, |c| format!("closed {}", c.when));
    let picked = match record.map(|r| &r.folder) {
        Some(Folder::Picked { .. }) => true,
        Some(Folder::Scratch { .. }) => false,
        None => entry.size_bytes.is_none(),
    };
    let folder = if picked {
        "your folder".to_owned()
    } else {
        match (entry.size_bytes, &entry.unpushed) {
            (Some(bytes), None) => format!("scratch folder {}", size(bytes)),
            (Some(bytes), Some(_)) => size(bytes),
            (None, _) => "scratch folder".to_owned(),
        }
    };
    let on = dialog.ticked.contains(&entry.session);
    let note = if let Some(unpushed) = &entry.unpushed {
        Some((format!("{unpushed}. Deleting deletes that work."), ORANGE))
    } else if picked {
        Some(("Your folder: left in place".to_owned(), look.secondary))
    } else {
        None
    };
    col()
        .gap(px(3.))
        .py(px(8.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .gap(px(10.))
                .child(
                    check(
                        ui,
                        look,
                        SharedString::from(format!("desk-clean-up-tick-{}", entry.session.as_str()))
                            .into(),
                        title.into(),
                        on,
                        !dialog.deleting,
                        AppAction::Desk(DeskAction::ToggleCleanUp(entry.session.clone())),
                    )
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap(),
                )
                .child(
                    secondary(look, ran)
                        .w(px(160.))
                        .flex_shrink_0()
                        .whitespace_nowrap(),
                )
                .child(
                    secondary(look, closed)
                        .w(px(96.))
                        .flex_shrink_0()
                        .whitespace_nowrap(),
                )
                .child(
                    div()
                        .w(px(150.))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .whitespace_nowrap()
                        .text_color(rgb(look.text))
                        .child(folder),
                ),
        )
        .children(note.map(|(text, color)| {
            div()
                .pl(px(21.))
                .text_size(px(11.5))
                .text_color(rgb(color))
                .child(text)
        }))
}
