//! The command palette dialog.

use super::{Overlays, sync};
use crate::bridge::Ui;
use crate::widgets::{col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::base::{FocusTrapElement as _, TestTextExt as _};
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
        let mac = cx.platform_info().is_mac();
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
                    .flex_shrink_0()
                    .px(px(16.))
                    .pt(px(8.))
                    .pb(px(3.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.secondary))
                    .child(if palette.asking.is_some() {
                        "Move to which machine?"
                    } else {
                        name
                    }),
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
                    Command::Pin { .. } => IconName::Pin,
                    Command::Vip { .. } => IconName::Bell,
                    _ => IconName::ArrowRight,
                };
                let mut item = row()
                    .id(("palette-entry", at))
                    .role(Role::ListBoxOption)
                    .aria_label(entry.title.clone())
                    .aria_selected(selected)
                    .flex_shrink_0()
                    .h(px(32.))
                    .mx(px(6.))
                    .px(px(10.))
                    .gap(px(9.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |x| {
                        x.bg(rgb(look.selection)).text_color(rgb(0xffffff))
                    })
                    .child(chrome::icon(glyph, color, 15.))
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
                            .px(px(5.))
                            .rounded(px(4.))
                            .when(!selected, |x| x.bg(rgb(look.fill)))
                            .text_size(px(11.))
                            .text_color(rgb(color))
                            .child(crate::keys::label(&entry.hint, mac)),
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
                            .child(crate::keys::label(&entry.hint, mac)),
                    );
                }
                if let Command::Session(id) = &entry.command
                    && let Some(session) = palette.workspace.session(id)
                    && !session.last_used.is_empty()
                {
                    item = item.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(rgb(if selected { 0xffffff } else { look.tertiary }))
                            .child(format!("used {}", session.last_used)),
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
        // FIND (LD-23, LD-24): every session's matching turns, under the palette's own entries.
        let send_find: crate::palette_hits::SendFind = Rc::new({
            let ui = ui.clone();
            move |action, cx: &mut App| ui.send(AppAction::Find(action), cx)
        });
        // The cursor past the entries is on a hit, as a place in the hits.
        let selected = palette.cursor.checked_sub(palette.entries.len());
        let (conversations, no_hits, loading, failed) = ui.read(|s| {
            // Moving a session asks for a machine, not a conversation.
            let conversations = if s.palette.asking.is_some() {
                None
            } else {
                let offline = |machine: &humata2_clients::model::MachineId| {
                    s.workspace.machines.iter().any(|m| &m.id == machine && !m.online)
                };
                crate::palette_hits::conversations(&s.find, selected, &offline, look, &send_find)
            };
            (
                conversations,
                s.find.hits.is_empty(),
                s.find.loading,
                s.find.error.is_some(),
            )
        });
        list = list.children(conversations);
        // One empty-state line when nothing is listed at all. A failed search
        // says so in its own line, so this one stays away.
        if palette.entries.is_empty() && no_hits && !failed {
            let text = if palette.no_machine_to_move_to() {
                "No other machine this session can move to is online. Start one, or link \
                 another machine to its channel from Machines, then move again."
                    .to_owned()
            } else if loading {
                "Searching every machine's conversations…".to_owned()
            } else if crate::palette_hits::searchable(&palette.query) {
                format!(
                    "Nothing matches “{}”. Try fewer letters or fewer words.",
                    palette.query.trim()
                )
            } else {
                "Nothing matches. Try fewer letters.".to_owned()
            };
            list = list.child(
                div()
                    .flex_shrink_0()
                    .p(px(16.))
                    .text_color(rgb(look.secondary))
                    .child(text.clone())
                    .test_text(text),
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
            // Up and Down move the cursor before the query input can take them.
            .capture_key_down({
                let ui = ui.clone();
                move |event, _, cx| {
                    let modifiers = &event.keystroke.modifiers;
                    // macOS marks arrow keys with the function flag, so it
                    // doesn't count as a modifier here.
                    if modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift {
                        return;
                    }
                    let delta = match event.keystroke.key.as_str() {
                        "down" => 1,
                        "up" => -1,
                        _ => return,
                    };
                    cx.stop_propagation();
                    ui.send(AppAction::Palette(PaletteAction::Move(delta)), cx);
                }
            })
            .test_support()
            .child(
                row()
                    .h(px(48.))
                    .px(px(16.))
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .child(chrome::icon(IconName::Search, look.secondary, 15.))
                    .child(
                        // Frame 30: the text starts one gap after the icon. The icon is 15 pt
                        // and the input's own pad is 8 pt, so the margin is -9 pt.
                        div().flex_1().ml(px(-9.)).child(
                            Input::new(&self.query)
                                .aria_label("Search commands")
                                .appearance(false)
                                .text_size(px(16.)),
                        ),
                    ),
            )
            .child(list)
            .focus_trap("palette-trap", &self.query.focus_handle(cx))
            .into_any_element()
    }
}
