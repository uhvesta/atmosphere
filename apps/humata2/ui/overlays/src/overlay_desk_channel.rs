//! A channel's Desk dialogs (CD-4, frame 94): the confirmation before Close
//! session. The rejected "New desk session" modal (ATM-39-01) is retired.

use super::{Overlays, button, card, primary, secondary};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_desk::{ChannelCloseSheet, DeskAction, DeskState};
use std::rc::Rc;

/// Retired placeholder for the rejected "New desk session" modal (ATM-39-01).
pub(super) struct NewView;

/// Always returns `None` because the "New desk session" modal was rejected (ATM-39-01).
pub(super) fn new_view(_desk: &DeskState) -> Option<NewView> {
    None
}

/// What the close confirmation draws.
pub(super) struct CloseView {
    sheet: ChannelCloseSheet,
    title: String,
    files: Vec<String>,
    lines: std::collections::BTreeMap<String, u32>,
    /// The folder holds a version, so Close keeps it (DV-13).
    kept: bool,
}

pub(super) fn close_view(desk: &DeskState) -> Option<CloseView> {
    let sheet = desk.channel_close.clone()?;
    let title = desk
        .workspace
        .session(&sheet.session)
        .map_or_else(|| sheet.session.to_string(), |found| found.title.clone());
    let record = desk.channel_record(&sheet.session);
    let files = record
        .map(|found| found.documents.clone())
        .unwrap_or_default();
    let lines = record
        .map(|found| found.document_lines.clone())
        .unwrap_or_default();
    let kept = record.is_some_and(|found| found.desk.version_count > 0);
    Some(CloseView {
        sheet,
        title,
        files,
        lines,
        kept,
    })
}

/// "54 lines", as a Markdown file's size reads in the confirmation.
fn lines_label(lines: u32) -> String {
    if lines == 1 {
        "1 line".to_owned()
    } else {
        format!("{lines} lines")
    }
}

impl Overlays {
    /// Retired stub for the rejected "New desk session" modal (ATM-39-01).
    pub(super) fn desk_channel_new(
        &self,
        _view: &NewView,
        _look: Look,
        _ui: &Rc<Ui>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        div().into_any_element()
    }

    /// Close session on a channel Desk session asks first (CD-4): a folder
    /// with no version goes to the trash now, and its Markdown files can be
    /// copied out; one that holds a version stays under Closed (DV-13).
    pub(super) fn desk_channel_close(
        &self,
        view: &CloseView,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let busy = view.sheet.closing;
        let files = (!view.files.is_empty()).then(|| {
            col()
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(look.secondary))
                        .child(format!("Markdown files in it · {}", view.files.len())),
                )
                .children(view.files.iter().map(|name| {
                    let copied = view.sheet.copied.contains(name);
                    row()
                        .gap(px(10.))
                        .py(px(7.))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .child(chrome::icon(IconName::FileText, look.secondary, 14.))
                        .child(
                            row()
                                .flex_1()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .text_size(px(12.))
                                        .child(name.clone()),
                                )
                                .children(
                                    view.lines
                                        .get(name)
                                        .map(|lines| secondary(look, lines_label(*lines))),
                                ),
                        )
                        .child(button(
                            ui,
                            look,
                            SharedString::from(format!("channel-desk-copy-{name}")),
                            if copied { "Copied" } else { "Copy to shared/" },
                            AppAction::Desk(DeskAction::CopyToShared(name.clone())),
                        ))
                }))
        });
        card("channel-desk-close", "Close session", 560., look)
            .gap(px(12.))
            .child(
                col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Close \u{201c}{}\u{201d}?", view.title)),
                    )
                    // DV-13: a folder that holds a version is kept.
                    .child(if view.kept {
                        div()
                            .id("channel-desk-close-kept")
                            .role(Role::Status)
                            .aria_label("Folder kept")
                            .test_support()
                            .child("Its folder holds a version, so it stays on the channel's Desk under Closed, where you can reopen it.")
                    } else {
                        div()
                            .id("channel-desk-close-trash")
                            .role(Role::Status)
                            .aria_label("Folder goes to the trash")
                            .test_support()
                            .child("Its scratch folder goes to the trash now.")
                    }),
            )
            .children(files)
            .children((!view.kept).then(|| {
                secondary(
                    look,
                    "Its record and conversation stay in the trash for 30 days. You can restore them from there.",
                )
            }))
            .children(view.sheet.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "channel-desk-close-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(
                        ui,
                        look,
                        "channel-desk-close-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(primary(
                        ui,
                        look,
                        "channel-desk-close-confirm",
                        if busy { "Closing…" } else { "Close session" },
                        AppAction::Desk(DeskAction::ConfirmCloseChannel),
                        busy,
                    )),
            )
            .focus_trap("channel-desk-close", &self.sheet)
            .into_any_element()
    }
}
