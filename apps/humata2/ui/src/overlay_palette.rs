//! The command palette dialog.

use super::{Overlays, sync};
use crate::bridge::Ui;
use crate::widgets::{col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_palette::{Command, PaletteAction, PaletteState};
use std::rc::Rc;

impl Overlays {
    /// The command palette: grouped, searchable entries.
    pub(super) fn palette(
        &self,
        palette: &PaletteState,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.query, &palette.query, window, cx);
        let group = |command: &Command| match command {
            Command::Session(_) => 0,
            Command::File { .. } => 1,
            Command::Channel(_) => 2,
            _ => 3,
        };
        let mut list = col()
            .id("palette-entries")
            .role(Role::ListBox)
            .aria_label("Search results")
            .overflow_y_scroll()
            .max_h(px(420.));
        for (index, name) in ["Sessions", "Files", "Channels", "Commands"]
            .into_iter()
            .enumerate()
        {
            if !palette
                .entries
                .iter()
                .any(|entry| group(&entry.command) == index)
            {
                continue;
            }
            list = list.child(
                div()
                    .px(px(16.))
                    .pt(px(8.))
                    .pb(px(3.))
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.secondary))
                    .child(name),
            );
            for (at, entry) in palette
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| group(&entry.command) == index)
            {
                let selected = at == palette.cursor;
                let ui = ui.clone();
                let color = if selected { 0xffffff } else { look.secondary };
                let glyph = match &entry.command {
                    Command::Session(id) => {
                        if palette
                            .workspace
                            .session(id)
                            .is_some_and(|s| s.parent.is_some())
                        {
                            IconName::GitFork
                        } else {
                            IconName::SquareTerminal
                        }
                    }
                    Command::File { .. } => IconName::FileCode,
                    Command::Channel(_) => IconName::FolderGit2,
                    Command::Fork => IconName::GitFork,
                    Command::TerminalFullScreen => IconName::Maximize2,
                    Command::TogglePanel => IconName::Columns2,
                    _ => IconName::ArrowRight,
                };
                let mut item = row()
                    .id(("palette-entry", at))
                    .role(Role::ListBoxOption)
                    .aria_label(entry.title.clone())
                    .aria_selected(selected)
                    .h(px(32.))
                    .mx(px(6.))
                    .px(px(10.))
                    .gap(px(9.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |x| {
                        x.bg(rgb(look.selection)).text_color(rgb(0xffffff))
                    })
                    .child(chrome::icon(glyph, color, 14.))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(entry.title.clone()),
                    );
                if index == 3 {
                    item = item.child(div().flex_1()).child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(color))
                            .child(entry.hint.clone()),
                    );
                } else {
                    item = item.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(11.5))
                            .text_color(rgb(color))
                            .child(entry.hint.clone()),
                    );
                }
                list = list.child(
                    item.on_click(move |_, _, cx| {
                        ui.send(AppAction::Palette(PaletteAction::Pick(at)), cx)
                    })
                    .test_support(),
                );
            }
        }
        if palette.entries.is_empty() {
            list = list.child(
                div()
                    .p(px(16.))
                    .text_color(rgb(look.secondary))
                    .child("Nothing matches. Try fewer letters."),
            );
        }
        col()
            .id("palette")
            .role(Role::Dialog)
            .aria_label("Command palette")
            .w(px(600.))
            .max_w_full()
            .pb(px(6.))
            .rounded(px(12.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .shadow_lg()
            .text_size(px(13.))
            .text_color(rgb(look.text))
            .test_support()
            .child(
                row()
                    .h(px(48.))
                    .px(px(16.))
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .child(chrome::icon(IconName::Search, look.secondary, 14.))
                    .child(
                        Input::new(&self.query)
                            .aria_label("Search commands")
                            .border_0()
                            .bg(rgb(look.card))
                            .text_size(px(16.)),
                    ),
            )
            .child(list)
            .focus_trap("palette-trap", &self.query.focus_handle(cx))
            .into_any_element()
    }
}
