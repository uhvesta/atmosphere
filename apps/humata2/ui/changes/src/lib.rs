//! Diff, review, document history, and code editor GPUI views.

pub mod changes_loading;
pub mod code_editor;
pub mod comment_cards;
pub mod doc_history;
pub mod document_tab;
pub mod panel_changes;
pub mod panel_stack;

use humata2_ui_core::{
    action_menu, bridge, chrome, code_wrap, full_text, keys, theme, widgets,
};
