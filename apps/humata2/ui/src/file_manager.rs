//! The Files page (frame 61; FM-1 to FM-3, FM-7, FM-8, FM-12, FM-13, FM-15,
//! LD-7, LD-13 to LD-15, LD-17, LD-19): two panes, the action bar, the
//! trash sheet and the low-disk dialog. Views only; the reducer is
//! `humata2_files`.

use std::rc::Rc;

use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::{EntryKind, StorageAction, StorageKind, StorageRow};
use humata2_files::{FilesAction as A, FilesState, NamingKind, Side, human_bytes};

const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
const SIZE_W: f32 = 80.;
const WHEN_W: f32 = 112.;

fn dim(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .small()
        .h(px(24.))
        .px(px(11.))
        .label(label)
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(action.clone(), cx);
        })
}

fn primary(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Button {
    button(ui, id, label, action, look)
        .bg(rgb(look.accent))
        .text_color(rgb(0xffffff))
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// The action's button label on a Storage row.
pub(crate) fn action_label(action: &StorageAction) -> Option<&'static str> {
    match action {
        StorageAction::None => None,
        StorageAction::Archive(_) => Some("Archive…"),
        StorageAction::ClearBuildCache(_) => Some("Clear"),
        StorageAction::EmptyTrash => Some("Empty…"),
        StorageAction::TrimLogs => Some("Trim…"),
    }
}

/// The kind column's text.
pub(crate) const fn kind_label(kind: StorageKind) -> &'static str {
    match kind {
        StorageKind::SessionClones => "Session clones",
        StorageKind::MainClones => "Main clones",
        StorageKind::BuildCache => "Build caches",
        StorageKind::Trash => "Trash",
        StorageKind::Logs => "Logs",
    }
}

fn columns(look: Look) -> Div {
    let head = |text: &'static str| {
        div()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(look.secondary))
            .child(text)
    };
    row()
        .gap(px(6.))
        .h(px(24.))
        .px(px(12.))
        .bg(rgb(look.panel))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(div().w(px(15.)).flex_shrink_0())
        .child(div().flex_1().child(head("Name")))
        .child(
            div()
                .w(px(SIZE_W))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(head("Size")),
        )
        .child(
            div()
                .w(px(WHEN_W))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(head("Modified")),
        )
}

/// One listing row; `act` is None where the pane is stale.
#[allow(clippy::too_many_arguments)]
fn line(
    id: String,
    label: String,
    glyph: IconName,
    name: String,
    size: String,
    when: String,
    on: bool,
    active: bool,
    stale: bool,
    look: Look,
    on_down: Option<Box<dyn Fn(&MouseDownEvent, &mut App)>>,
) -> ObservedElement<Stateful<Div>> {
    let (text, secondary) = if stale {
        (look.secondary, look.tertiary)
    } else if on && active {
        (0xffffff, 0xffffff)
    } else {
        (look.text, look.secondary)
    };
    let mut line = row()
        .id(SharedString::from(id))
        .role(Role::ListItem)
        .aria_label(label)
        .aria_selected(on)
        .test_support()
        .gap(px(6.))
        .h(px(26.))
        .px(px(12.))
        .flex_shrink_0()
        .text_size(px(12.5))
        .when(on && !stale, |x| x.bg(rgb(if active { look.selection } else { look.fill })))
        .child(crate::chrome::icon(glyph, secondary, 12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(rgb(text))
                .child(name),
        )
        .child(
            div()
                .w(px(SIZE_W))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .whitespace_nowrap()
                .text_color(rgb(secondary))
                .child(size),
        )
        .child(
            div()
                .w(px(WHEN_W))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .whitespace_nowrap()
                .text_color(rgb(secondary))
                .child(when),
        );
    if let Some(on_down) = on_down {
        line = line
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |event, _, cx| on_down(event, cx));
    }
    line
}

fn pane(ui: &Rc<Ui>, state: &FilesState, side: Side, look: Look) -> Stateful<Div> {
    let pane = state.pane(side);
    let tag = side_name(side);
    let active = state.active == side;
    let stale = pane.stale;

    let current = pane.machine.as_ref().map_or_else(
        || "Choose a machine".to_owned(),
        |machine| state.machine_name(machine),
    );
    let popup = {
        let ui = ui.clone();
        row()
            .id(SharedString::from(format!("files-picker {tag}")))
            .role(Role::Button)
            .aria_label(format!("Machine of the {tag} pane: {current}"))
            .test_support()
            .gap(px(6.))
            .h(px(24.))
            .pl(px(9.))
            .pr(px(5.))
            .rounded(px(6.))
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .flex_shrink_0()
            .cursor_pointer()
            .child(current)
            .child(crate::chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                ui.send(AppAction::Files(A::TogglePicker(side)), cx);
            })
    };
    let path = pane
        .folder
        .as_ref()
        .map_or_else(String::new, |folder| folder.display_path());
    let head = row()
        .gap(px(8.))
        .h(px(40.))
        .px(px(12.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(popup)
        .child(
            div()
                .id(SharedString::from(format!("files-path {tag}")))
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(px(12.5))
                .text_color(rgb(look.secondary))
                .child(path),
        );

    let mut rows = col().flex_1().min_h_0();
    if stale {
        let name = pane
            .machine
            .as_ref()
            .map_or_else(|| "This machine".to_owned(), |m| state.machine_name(m));
        rows = rows.child(
            div()
                .id(SharedString::from(format!("files-stale {tag}")))
                .role(Role::Alert)
                .aria_label("Machine offline")
                .test_support()
                .px(px(12.))
                .py(px(6.))
                .flex_shrink_0()
                .bg(rgb(look.attention_bg))
                .text_size(px(11.5))
                .text_color(rgb(ORANGE))
                .child(format!(
                    "{name} is offline. This is the last listing read; nothing here can be changed until it is back."
                )),
        );
    }
    if let Some(error) = &pane.error {
        rows = rows.child(
            div()
                .id(SharedString::from(format!("files-error {tag}")))
                .role(Role::Alert)
                .aria_label(error.clone())
                .test_support()
                .px(px(12.))
                .py(px(6.))
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(RED))
                .child(error.clone()),
        );
    }
    if pane.folder.as_ref().is_some_and(|f| f.parent().is_some()) {
        let ui2 = ui.clone();
        rows = rows.child(line(
            format!("files-row {tag} .."),
            "Up one folder".to_owned(),
            IconName::ChevronRight,
            "..".to_owned(),
            String::new(),
            String::new(),
            false,
            active,
            stale,
            look,
            (!stale).then(|| {
                Box::new(move |_: &MouseDownEvent, cx: &mut App| {
                    ui2.send(AppAction::Files(A::Activate(side)), cx);
                    ui2.send(AppAction::Files(A::Up(side)), cx);
                }) as Box<dyn Fn(&MouseDownEvent, &mut App)>
            }),
        ));
    }
    for entry in &pane.entries {
        let is_folder = entry.kind == EntryKind::Folder;
        let on = pane.selected.contains(&entry.name);
        let ui2 = ui.clone();
        let name = entry.name.clone();
        rows = rows.child(line(
            format!("files-row {tag} {}", entry.name),
            entry.name.clone(),
            if is_folder && on {
                IconName::FolderOpen
            } else if is_folder {
                IconName::Folder
            } else {
                IconName::FileText
            },
            if is_folder {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            },
            entry.size.map_or_else(|| "—".to_owned(), human_bytes),
            state.modified(side, entry),
            on,
            active,
            stale,
            look,
            (!stale).then(|| {
                Box::new(move |event: &MouseDownEvent, cx: &mut App| {
                    ui2.send(AppAction::Files(A::Activate(side)), cx);
                    ui2.send(
                        AppAction::Files(A::Select(
                            side,
                            name.clone(),
                            event.modifiers.secondary(cx) || event.modifiers.shift,
                        )),
                        cx,
                    );
                    if is_folder && event.click_count == 2 {
                        ui2.send(AppAction::Files(A::OpenFolder(side, name.clone())), cx);
                    }
                }) as Box<dyn Fn(&MouseDownEvent, &mut App)>
            }),
        ));
    }
    if pane.loading && !pane.loaded {
        rows = rows.child(div().px(px(12.)).py(px(8.)).child(dim(look, "Reading…")));
    }
    if pane.loaded && !stale {
        rows = rows.child(
            div()
                .id(SharedString::from(format!("files-drop {tag}")))
                .mx(px(12.))
                .mt(px(10.))
                .h(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(rgb(look.panel))
                .border_1()
                .border_color(rgb(look.line))
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child("Drop here, or drag from Finder"),
        );
    }

    let ui2 = ui.clone();
    col()
        .id(SharedString::from(format!("files-pane {tag}")))
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .when(stale, |x| x.opacity(0.7))
        .child(head)
        .child(columns(look))
        .child(
            rows.id(SharedString::from(format!("files-list {tag}")))
                .overflow_y_scroll(),
        )
        .children(picker_list(ui, state, side, look))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            ui2.send(AppAction::Files(A::Activate(side)), cx);
        })
}

/// The open machine list under a pane's popup button: every machine, the
/// current one ticked, offline ones dimmed and not clickable (FM-12). It is
/// the pane's last child, so it paints over the rows.
fn picker_list(
    ui: &Rc<Ui>,
    state: &FilesState,
    side: Side,
    look: Look,
) -> Option<ObservedElement<Stateful<Div>>> {
    if state.picker != Some(side) {
        return None;
    }
    let tag = side_name(side);
    let current = state.pane(side).machine.clone();
    let mut list = col()
        .id(SharedString::from(format!("files-picker-list {tag}")))
        .role(Role::ListBox)
        .aria_label("Machines")
        .test_support()
        .absolute()
        .top(px(36.))
        .left(px(12.))
        .w(px(220.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5));
    for machine in &state.workspace.machines {
        let here = current.as_ref() == Some(&machine.id);
        let online = machine.online;
        let ui = ui.clone();
        let id = machine.id.clone();
        let mut item = row()
            .id(SharedString::from(format!("files-picker-item {tag} {}", machine.id)))
            .role(Role::ListBoxOption)
            .aria_label(machine.name.clone())
            .aria_selected(here)
            .test_support()
            .gap(px(8.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .text_color(rgb(if online { look.text } else { look.tertiary }))
            .child(div().w(px(12.)).flex_shrink_0().children(
                here.then(|| crate::chrome::icon(IconName::Check, look.accent, 12.)),
            ))
            .child(div().flex_1().min_w_0().whitespace_nowrap().child(machine.name.clone()))
            .children((!online).then(|| dim(look, "offline")));
        if online {
            item = item.cursor_pointer().on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                ui.send(AppAction::Files(A::PickMachine(side, id.clone())), cx);
            });
        } else {
            // An offline machine can be listed but not picked: swallow the click.
            item = item.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        }
        list = list.child(item);
    }
    Some(list)
}

fn action_bar(ui: &Rc<Ui>, state: &FilesState, name: &Entity<InputState>, look: Look) -> Div {
    let from = state.active_pane();
    let to = state.pane(state.active.other());
    let usable = !from.stale && from.folder.is_some();
    let picked = usable && !from.selected.is_empty();
    let can_transfer = picked && !to.stale && to.folder.is_some();
    let arrow_right = state.active == Side::Left;
    let (copy, mv) = if arrow_right {
        ("Copy →", "Move →")
    } else {
        ("← Copy", "← Move")
    };

    // New folder and Rename open the name prompt; the shell-owned input is
    // given the prompt's starting text first.
    let opener = |id: &'static str, label: &'static str, kind: NamingKind, enabled: bool| {
        let ui = ui.clone();
        let input = name.clone();
        let initial = match kind {
            NamingKind::NewFolder => String::new(),
            NamingKind::Rename => from.selected.first().cloned().unwrap_or_default(),
        };
        let action = match kind {
            NamingKind::NewFolder => A::BeginNewFolder,
            NamingKind::Rename => A::BeginRename,
        };
        Button::new(id)
            .small()
            .h(px(24.))
            .px(px(11.))
            .label(label)
            .bg(rgb(look.fill))
            .text_color(rgb(look.text))
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                let initial = initial.clone();
                input.update(cx, |input, cx| input.set_value(initial, window, cx));
                ui.send(AppAction::Files(action.clone()), cx);
            })
    };
    let new_folder = opener("files-new-folder", "New folder", NamingKind::NewFolder, usable);
    let rename = opener(
        "files-rename",
        "Rename",
        NamingKind::Rename,
        usable && from.selected.len() == 1,
    );

    let trash = from.machine.clone().map(|machine| {
        let ui = ui.clone();
        let label = format!("Trash on {}", state.machine_name(&machine));
        row()
            .gap(px(6.))
            .child(
                Button::new("files-open-trash")
                    .small()
                    .ghost()
                    .h(px(24.))
                    .label(label)
                    .text_color(rgb(look.secondary))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        ui.send(AppAction::Files(A::OpenTrash(machine.clone())), cx);
                    }),
            )
            .child(
                div()
                    .id("files-trash-count")
                    .px(px(6.))
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(rgb(look.fill))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.secondary))
                    .child(state.trash_count.to_string()),
            )
    });

    row()
        .gap(px(8.))
        .h(px(42.))
        .px(px(16.))
        .flex_shrink_0()
        .border_t_1()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(new_folder)
        .child(rename)
        .child(
            primary(ui, "files-copy", copy, AppAction::Files(A::Copy), look)
                .disabled(!can_transfer),
        )
        .child(button(ui, "files-move", mv, AppAction::Files(A::Move), look).disabled(!can_transfer))
        .child(button(ui, "files-delete", "Delete", AppAction::Files(A::Delete), look).disabled(!picked))
        .child(div().flex_1())
        .children(trash)
}

/// The name prompt: a small centred sheet over the page, shown while
/// `state.naming` is open. `name` is the shell-owned input.
fn name_prompt(ui: &Rc<Ui>, state: &FilesState, name: &Entity<InputState>, look: Look) -> Option<Div> {
    let naming = state.naming.as_ref()?;
    let (title, confirm) = match naming.kind {
        NamingKind::NewFolder => ("New folder".to_owned(), "Create"),
        NamingKind::Rename => (format!("Rename \u{201c}{}\u{201d}", naming.initial), "Rename"),
    };
    let commit = {
        let ui = ui.clone();
        let input = name.clone();
        move |cx: &mut App| {
            let text = input.read(cx).value().to_string();
            ui.send(AppAction::Files(A::CommitName(text)), cx);
        }
    };
    let on_enter = commit.clone();
    let on_click = commit;
    let mut prompt = sheet("files-name-prompt", "Name", look)
        .w(px(360.))
        .on_key_down(move |event, _, cx| {
            if event.keystroke.key == "enter" {
                on_enter(cx);
            }
        })
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(Input::new(name).aria_label("Name"));
    if let Some(error) = &naming.error {
        prompt = prompt.child(
            div()
                .id("files-name-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .test_support()
                .text_size(px(12.))
                .text_color(rgb(RED))
                .child(error.clone()),
        );
    }
    Some(scrim(
        prompt.child(
            row()
                .justify_end()
                .gap(px(8.))
                .child(button(
                    ui,
                    "files-name-cancel",
                    "Cancel",
                    AppAction::Files(A::CancelName),
                    look,
                ))
                .child(
                    Button::new("files-name-commit")
                        .small()
                        .h(px(24.))
                        .px(px(11.))
                        .label(confirm)
                        .bg(rgb(look.accent))
                        .text_color(rgb(0xffffff))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            on_click(cx);
                        }),
                ),
        ),
    ))
}

fn sheet(id: &'static str, label: &'static str, look: Look) -> ObservedElement<Stateful<Div>> {
    col()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .test_support()
        .w(px(crate::theme::SHEET))
        .max_h(relative(0.8))
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .bg(rgb(look.card))
        .text_color(rgb(look.text))
        .text_size(px(12.5))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
}

fn scrim(sheet: impl IntoElement) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x0000_0066))
        .child(sheet)
}

fn trash_sheet(ui: &Rc<Ui>, state: &FilesState, look: Look) -> Option<Div> {
    let trash = state.trash.as_ref()?;
    let machine = state.machine_name(&trash.machine);
    let mut list = col().gap(px(2.));
    match &trash.listing {
        None => list = list.child(dim(look, "Reading the trash…")),
        Some(listing) if listing.items.is_empty() => {
            list = list.child(dim(look, "The trash is empty."));
        }
        Some(listing) => {
            for (index, item) in listing.items.iter().enumerate() {
                list = list.child(
                    row()
                        .id(SharedString::from(format!("files-trash-item {}", item.id)))
                        .role(Role::ListItem)
                        .aria_label(item.description.clone())
                        .test_support()
                        .gap(px(10.))
                        .min_h(px(32.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(item.description.clone()),
                        )
                        .child(
                            div()
                                .w(px(70.))
                                .flex_shrink_0()
                                .text_color(rgb(look.secondary))
                                .child(human_bytes(item.bytes)),
                        )
                        .child(
                            div()
                                .w(px(110.))
                                .flex_shrink_0()
                                .text_color(rgb(look.secondary))
                                .child(format!("empties {}", state.empties_in(item))),
                        )
                        .child(button(
                            ui,
                            ("files-trash-restore", index),
                            "Restore",
                            AppAction::Files(A::Restore(item.id.clone())),
                            look,
                        )),
                );
            }
        }
    }
    let has_items = trash.listing.as_ref().is_some_and(|l| !l.items.is_empty());
    let footer = if let Some(plan) = state.empty_plan.as_ref().filter(|p| p.machine == trash.machine)
    {
        row()
            .gap(px(8.))
            .child(
                div()
                    .id("files-empty-plan")
                    .flex_1()
                    .child(format!(
                        "Emptying frees {} by deleting {} {} for good.",
                        human_bytes(plan.frees_bytes),
                        plan.items,
                        if plan.items == 1 { "item" } else { "items" }
                    )),
            )
            .child(primary(
                ui,
                "files-empty-confirm",
                "Empty",
                AppAction::Files(A::ConfirmEmptyTrash),
                look,
            ))
            .child(button(
                ui,
                "files-empty-cancel",
                "Cancel",
                AppAction::Files(A::CancelEmptyTrash),
                look,
            ))
    } else {
        row()
            .gap(px(8.))
            .children(state.last_freed.map(|freed| {
                dim(look, format!("Last emptying freed {}.", human_bytes(freed)))
            }))
            .child(div().flex_1())
            .child(
                button(
                    ui,
                    "files-empty-trash",
                    "Empty trash…",
                    AppAction::Files(A::PlanEmptyTrash),
                    look,
                )
                .disabled(!has_items),
            )
            .child(button(
                ui,
                "files-trash-close",
                "Close",
                AppAction::Files(A::CloseTrash),
                look,
            ))
    };
    Some(scrim(
        sheet("files-trash", "Trash", look)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Trash on {machine}")),
            )
            .child(
                div()
                    .id("files-trash-list")
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .child(footer),
    ))
}

fn low_disk_sheet(ui: &Rc<Ui>, state: &FilesState, look: Look) -> Option<Div> {
    let low = state.low_disk.as_ref()?;
    let machine = state.machine_name(&low.machine);
    let mut rows = col().gap(px(2.));
    if low.largest.is_empty() {
        rows = rows.child(dim(
            look,
            format!("Humata couldn't measure what it keeps on {machine}. Free some space there by hand."),
        ));
    }
    for (index, row_) in low.largest.iter().enumerate() {
        rows = rows.child(low_row(ui, index, row_, look));
    }
    Some(scrim(
        sheet("files-low-disk", "Not enough room", look)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{machine} is short on room")),
            )
            .child(div().child(format!(
                "This needs {} and only {} are free on {machine}. Free some of what Humata keeps there, then try again.",
                human_bytes(low.needs_bytes),
                human_bytes(low.free_bytes)
            )))
            .child(
                div()
                    .id("files-low-disk-rows")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .child(primary(
                        ui,
                        "files-low-retry",
                        "Try again",
                        AppAction::Files(A::RetryAfterLowDisk),
                        look,
                    ))
                    .child(button(
                        ui,
                        "files-low-cancel",
                        "Cancel",
                        AppAction::Files(A::DismissLowDisk),
                        look,
                    )),
            ),
    ))
}

fn low_row(ui: &Rc<Ui>, index: usize, item: &StorageRow, look: Look) -> Div {
    row()
        .gap(px(10.))
        .min_h(px(32.))
        .child(
            div()
                .w(px(110.))
                .flex_shrink_0()
                .text_color(rgb(look.secondary))
                .child(kind_label(item.kind)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .child(item.what.clone()),
        )
        .child(
            div()
                .w(px(70.))
                .flex_shrink_0()
                .child(human_bytes(item.bytes)),
        )
        .child(match action_label(&item.action) {
            Some(label) => button(
                ui,
                ("files-low-act", index),
                label,
                AppAction::Files(A::LowDiskAct(index)),
                look,
            )
            .into_any_element(),
            None => div()
                .text_color(rgb(look.tertiary))
                .child("—")
                .into_any_element(),
        })
}

/// The Files page. `name` is the one text field the name prompt (New
/// folder, Rename) shows; the shell owns the entity, and the buttons that
/// open the prompt set its text.
pub(crate) fn page(
    ui: &Rc<Ui>,
    state: &FilesState,
    name: &Entity<InputState>,
    look: Look,
) -> AnyElement {
    let mut body = col().size_full().min_h_0();
    body = body.child(
        row()
            .h(px(52.))
            .flex_shrink_0()
            .px(px(20.))
            .border_b_1()
            .border_color(rgb(look.line))
            .child(
                div()
                    .flex_1()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Files"),
            ),
    );
    if let Some(error) = &state.error {
        body = body.child(
            row()
                .id("files-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .test_support()
                .flex_shrink_0()
                .gap(px(10.))
                .px(px(16.))
                .py(px(6.))
                .bg(rgb(look.del))
                .text_size(px(12.))
                .text_color(rgb(look.text))
                .child(div().flex_1().min_w_0().child(error.clone()))
                .child(button(
                    ui,
                    "files-error-dismiss",
                    "Dismiss",
                    AppAction::Files(A::DismissError),
                    look,
                )),
        );
    }
    body = body
        .child(
            row()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .child(pane(ui, state, Side::Left, look))
                .child(div().w(px(1.)).h_full().flex_shrink_0().bg(rgb(look.line)))
                .child(pane(ui, state, Side::Right, look)),
        )
        .child(action_bar(ui, state, name, look))
        .child(crate::transfers_tray::tray(ui, state, look));
    let picker_open = state.picker.is_some();
    let ui2 = ui.clone();
    col()
        .id("files-page")
        .relative()
        .size_full()
        .min_h_0()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        // A click anywhere that doesn't stop it closes an open machine list.
        .when(picker_open, |page| {
            page.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                ui2.send(AppAction::Files(A::ClosePicker), cx);
            })
        })
        .child(body)
        .children(name_prompt(ui, state, name, look))
        .children(trash_sheet(ui, state, look))
        .children(low_disk_sheet(ui, state, look))
        .into_any_element()
}
