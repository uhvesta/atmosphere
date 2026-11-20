//! Settings and channel access views for Humata 2.

// The Settings pages' nested element types pass the default limit of 128.
#![recursion_limit = "256"]

pub(crate) use humata2_ui_core::{bridge, chrome, theme, widgets};

pub mod access;
pub mod settings;
pub mod settings_providers;
pub mod settings_repositories;
mod sync;
