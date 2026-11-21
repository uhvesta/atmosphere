//! Sidebar, channel, session tabs, terminals, To-do, file manager, inbox, and
//! session panels for Humata 2's GPUI desktop UI.

pub(crate) use humata2_ui_core::{
    action_menu, bridge, chrome, code_wrap, full_text, keys, repo_search_field, theme, widgets,
};

pub mod channel;
pub mod cli_login;
pub mod conversation_tab;
pub mod file_manager;
pub mod inbox_page;
pub mod inbox_terminal;
pub mod panel_files;
pub mod panel_inbox;
pub mod panel_info;
pub mod panel_journal;
pub mod request_card;
pub mod resume_failed;
pub mod session_tabs;
pub mod sidebar;
pub mod sidebar_pinned;
pub mod terminals;
pub mod todo;
pub mod todo_panel;
pub mod view_tab;

/// Shared imports for `panel_{files,inbox,info,journal}` extracted from `panels.rs`.
#[allow(unused_imports)]
mod panel_prelude {
    pub(crate) use std::collections::HashSet;
    pub(crate) use std::rc::Rc;

    pub(crate) use gpui_kit::assets::IconName;
    pub(crate) use gpui_kit::base::{
        Disableable as _, Selectable as _, TestSupportExt as _, TestTextExt as _,
    };
    pub(crate) use gpui_kit::component::button::{Button, ButtonVariants as _};
    pub(crate) use gpui_kit::component::input::{Input, InputState};
    pub(crate) use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
    pub(crate) use gpui_kit::{prelude::*, *};
    pub(crate) use humata2_app::{AppAction, AppState, Route};
    pub(crate) use humata2_clients::Workspace;
    pub(crate) use humata2_clients::model::{
        Item, ItemStatus, Request, Scope, Session, SessionId, Status,
    };
    pub(crate) use humata2_inbox::InboxAction;
    pub(crate) use humata2_session::{Panel, SessionAction};
    pub(crate) use humata2_ui_core::bridge::Ui;
    pub(crate) use humata2_ui_core::chrome::{
        GREEN, ORANGE, RED, card, family, header_with, icon, icon_action, panel_action as action,
        panel_header, panel_status as status, secondary, selected, session_line, text, title,
    };
    pub(crate) use humata2_ui_core::theme::Look;
    pub(crate) use humata2_ui_core::widgets::{self, col, row};
}
