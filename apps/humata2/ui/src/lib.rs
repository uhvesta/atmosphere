//! Humata 2's desktop UI in GPUI, drawn from the app's tca-rust store.
//!
//! [`bridge`] is the one place the store meets GPUI. The views read the
//! state, send actions, and hold nothing the store doesn't except GPUI's
//! own handles: text fields' editing state, focus, scroll offsets. Every
//! control carries a role and a label, so flow tests, recordings and screen
//! readers find it by what it says.

pub use humata2_ui_changes::{
    changes_loading, code_editor, comment_cards, doc_history, document_tab, panel_changes,
    panel_stack,
};
pub use humata2_ui_core::{
    Assets, WINDOW, action_menu, bridge, chrome, code_wrap, full_text, init, keys,
    repo_search_field, theme, widgets,
};
pub use humata2_ui_desk::{
    channel_desk, desk, desk_bar, desk_files, desk_info, sidebar_desk,
};
pub use humata2_ui_machines::{
    away_left_out, hosts, machine_page, machine_running, transfers_tray,
};
pub use humata2_ui_overlays::{overlays, palette_hits};
pub use humata2_ui_report::{record, record_video, report_view};
pub use humata2_ui_settings::{access, settings, settings_providers, settings_repositories};
pub use humata2_ui_workspace::{
    channel, cli_login, conversation_tab, file_manager, inbox_page, inbox_terminal, panel_files, panel_inbox, panel_info, panel_journal, request_card, resume_failed, reviews_inbox,
    session_tabs, sidebar, sidebar_pinned, terminals, todo, view_tab,
};

mod lists;
mod pages;
mod panels;
mod restart_report;
mod shell;
mod starting;

use std::rc::Rc;

use gpui_kit::{prelude::*, *};

pub use shell::Shell;
pub use starting::{STARTING_WINDOW, Starting};

use crate::bridge::Ui;

/// Opens the app's window on `ui` with `options`, starts the store and
/// launches the app.
///
/// # Errors
///
/// What GPUI says when the window can't open.
pub fn open(
    cx: &mut App,
    ui: &Rc<Ui>,
    options: WindowOptions,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Shell>)> {
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let ui = ui.clone();
        cx.new(|cx| Shell::new(ui, window, cx))
    })?;
    ui.start(cx);
    Ok(opened)
}
