//! The command palette dialog.

use super::{Overlays, sync};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::widgets::{col, row};
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
        &mut self,
        palette: &PaletteState,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.query, &palette.query, window, cx);
        let mac = cx.platform_info().is_mac();
        let viewport_h = f32::from(window.viewport_size().height);
        let max_list_h = if viewport_h > 0. {
            (viewport_h - 220.).clamp(160., 420.)
        } else {
            420.
        };
        let group = |command: &Command| match command {
            // D784: a session's terminals, hidden ones too, sit with its sessions.
            Command::Session(_) | Command::Terminal { .. } => 0,
            Command::File { .. } => 1,
            Command::Channel(_) => 2,
            _ => 3,
        };
        let mut child_count = 0usize;
        let mut selected_child = None;
        let mut list = col()
            .id("palette-entries")
            .role(Role::ListBox)
            .aria_label("Search results")
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.palette_scroll)
            .max_h(px(max_list_h))
            .test_support();
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
            child_count += 1;
            for (at, entry) in palette
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| group(&entry.command) == index)
            {
                let selected = at == palette.cursor;
                if selected {
                    selected_child = Some(child_count);
                }
                child_count += 1;
                let ui = ui.clone();
                let danger = entry.command == Command::DeleteAllSessions;
                let color = if selected { 0xffffff } else { look.secondary };
                let name_color = if danger && !selected { Some(0xff453a) } else { None };
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
                    Command::Terminal { .. } => IconName::SquareTerminal,
                    Command::File { .. } => IconName::FileCode,
                    Command::Channel(_) => IconName::FolderGit2,
                    Command::Fork => IconName::GitFork,
                    Command::SelectSessions => IconName::ListChecks,
                    Command::DeleteAllSessions => IconName::Trash,
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
                    .child(chrome::icon(glyph, name_color.unwrap_or(color), 15.))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .when_some(name_color, |x, red| x.text_color(rgb(red)))
                            .child(entry.title.clone()),
                    );
                // The two session-deleting entries carry a note, not a shortcut.
                let noted = matches!(
                    entry.command,
                    Command::SelectSessions | Command::DeleteAllSessions
                );
                if index == 3 && !noted {
                    item = item.child(div().flex_1()).child(
                        div()
                            .px(px(5.))
                            .rounded(px(4.))
                            .when(!selected, |x| x.bg(rgb(look.fill)))
                            .text_size(px(11.))
                            .text_color(rgb(color))
                            .child(humata2_ui_core::keys::label(&entry.hint, mac)),
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
                            .child(humata2_ui_core::keys::label(&entry.hint, mac)),
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
        // The cursor past the entries is on a hit, as a place in the visible hits.
        let selected = palette.cursor.checked_sub(palette.entries.len());
        let (conversations, selected_hit_offset, no_hits, loading, failed) = ui.read(|s| {
            let visible = s.visible_find_hits();
            let filtered_out =
                s.palette.asking.is_some() || humata2_palette::parse_query(&s.palette.query).filter.is_some();
            let (rows, hit_offset) = if filtered_out {
                (Vec::new(), None)
            } else {
                let offline = |machine: &humata2_clients::model::MachineId| {
                    s.workspace.machines.iter().any(|m| &m.id == machine && !m.online)
                };
                crate::palette_hits::conversation_rows(
                    &visible,
                    &s.find.query,
                    &s.find.next_page,
                    s.find.loading,
                    s.find.error.as_deref(),
                    selected,
                    &offline,
                    look,
                    &send_find,
                )
            };
            (
                rows,
                hit_offset,
                visible.is_empty(),
                !filtered_out && s.find.loading,
                !filtered_out && s.find.error.is_some(),
            )
        });
        if let Some(offset) = selected_hit_offset {
            selected_child = Some(child_count + offset);
        }
        list = list.children(conversations);
        let query_changed = self.palette_query != palette.query;
        let cursor_changed = self.palette_cursor != Some(palette.cursor);
        if query_changed || self.palette_cursor.is_none() {
            self.palette_scroll.set_offset(point(px(0.), px(0.)));
            self.palette_query.clone_from(&palette.query);
        } else if cursor_changed
            && let Some(item_index) = selected_child
        {
            self.palette_scroll.scroll_to_item(item_index);
        }
        self.palette_cursor = Some(palette.cursor);
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
            .when(palette.asking.is_none(), |dialog| {
                let archived_on = palette.shows_archived();
                let toggle_ui = ui.clone();
                dialog.child(
                    row()
                        .id("palette-footer")
                        .flex_shrink_0()
                        .px(px(14.))
                        .pt(px(6.))
                        .pb(px(2.))
                        .mt(px(2.))
                        .border_t_1()
                        .border_color(rgb(look.line))
                        .gap(px(10.))
                        .text_size(px(11.))
                        .text_color(rgb(look.tertiary))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child("> commands   # channels   @ sessions   / files"),
                        )
                        .child(
                            row()
                                .id("palette-toggle-archived")
                                .role(Role::CheckBox)
                                .aria_label("Include archived")
                                .aria_selected(archived_on)
                                .flex_shrink_0()
                                .gap(px(5.))
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .when(archived_on, |x| {
                                    x.text_color(rgb(look.accent))
                                })
                                .hover(move |x| x.bg(rgb(look.fill)))
                                .child(
                                    div()
                                        .size(px(11.))
                                        .rounded(px(3.))
                                        .border_1()
                                        .border_color(rgb(if archived_on {
                                            look.accent
                                        } else {
                                            look.tertiary
                                        }))
                                        .when(archived_on, |x| x.bg(rgb(look.accent))),
                                )
                                .child("Include archived")
                                .on_click(move |_, _, cx| {
                                    toggle_ui.send(
                                        AppAction::Palette(PaletteAction::ToggleArchived),
                                        cx,
                                    );
                                })
                                .test_support(),
                        ),
                )
            })
            .focus_trap("palette-trap", &self.query.focus_handle(cx))
            .into_any_element()
    }
}
