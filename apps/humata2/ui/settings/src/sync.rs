//! Settings > Sync (FUSEFSWIRE, D904, frame 211): the master switch with what
//! is copied under it, each machine's departures from those defaults, and the
//! mount. Dim, never hide: a switched-off master or category dims what hangs
//! off it to 0.4, stops its input and keeps its values.
use crate::{
    bridge::Ui,
    chrome,
    settings::{card, dim},
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::daemon::{FuseMountStatus, MachineSyncSettings, SyncSkillsMode, SyncSnapshot};
use humata2_clients::model::{Machine, MachineKind};
use humata2_settings::{SettingsAction as A, SyncAction, SyncCategory, SyncColumn};
use std::rc::Rc;

/// The width of one column of the machines table.
const COLUMN: f32 = 104.;
/// What a dimmed control draws at.
const DIMMED: f32 = 0.4;

/// The columns of the machines table after the name, in order.
const COLUMNS: [&str; 5] = ["Sync", "Skills", "Instructions", "MCP servers", "Settings"];

/// Settings > Sync, as the body of the settings page.
pub(crate) fn body(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let Some(snapshot) = &state.settings.sync.snapshot else {
        return col().child(dim(look, "Loading Sync"));
    };
    let machines = ordered(&state.workspace.machines);
    col()
        .gap(px(14.))
        .child(master_card(ui, snapshot, look))
        .child(machines_block(ui, state, snapshot, &machines, look))
        .child(mount_card(ui, snapshot, look))
}

/// This Mac first, then the others in sidebar order.
fn ordered(machines: &[Machine]) -> Vec<&Machine> {
    let (this, others): (Vec<&Machine>, Vec<&Machine>) =
        machines.iter().partition(|machine| matches!(machine.kind, MachineKind::ThisMac));
    this.into_iter().chain(others).collect()
}

fn send(ui: &Rc<Ui>, action: SyncAction, cx: &mut App) {
    ui.send(AppAction::Settings(A::Sync(action)), cx);
}

/// One settings row: its title over its notes, then its controls.
fn line(look: Look, indent: bool, title: &'static str, notes: &[&'static str], controls: Vec<AnyElement>) -> Div {
    line_with(look, indent, title, notes, Vec::new(), controls)
}

/// [`line`] with more note elements after the static ones.
fn line_with(
    look: Look,
    indent: bool,
    title: &'static str,
    notes: &[&'static str],
    more: Vec<AnyElement>,
    controls: Vec<AnyElement>,
) -> Div {
    row()
        .gap(px(16.))
        .pr(px(14.))
        .pl(px(if indent { 28. } else { 14. }))
        .py(px(10.))
        .items_center()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(title)
                .children(notes.iter().map(|note| dim(look, *note)))
                .children(more),
        )
        .children(controls)
}

/// A switch that sends `action` when enabled; a disabled one only draws.
fn switch(ui: &Rc<Ui>, id: &'static str, on: bool, enabled: bool, action: SyncAction, look: Look) -> AnyElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(if on { "On" } else { "Off" })
        .aria_selected(on)
        .test_support()
        .w(px(28.))
        .h(px(16.))
        .rounded_full()
        .p(px(2.))
        .flex()
        .flex_shrink_0()
        .when(on, |x| x.bg(rgb(0x28cd41)).justify_end())
        .when(!on, |x| x.bg(rgb(look.fill)).border_1().border_color(rgb(look.line)))
        .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff)).shadow_sm())
        .when(enabled, |x| {
            x.cursor_pointer().on_click(move |_, _, cx| send(&ui, action.clone(), cx))
        })
        .into_any_element()
}

fn master_card(ui: &Rc<Ui>, snapshot: &SyncSnapshot, look: Look) -> Div {
    let sync = &snapshot.sync;
    let live = sync.sync_enabled;
    // A category row under the master: dims with it.
    let category = |title: &'static str, notes: &[&'static str], id: &'static str, which: SyncCategory, on: bool, extra: Vec<AnyElement>| {
        let mut controls = extra;
        controls.push(switch(ui, id, on, live, SyncAction::ToggleCategory(which), look));
        line(look, true, title, notes, controls).when(!live, |x| x.opacity(DIMMED))
    };
    let mode = mode_control(ui, sync.skills_mode, live && sync.skills, look);
    card(look)
        // `settings-sync-master`.
        .child(line(
            look,
            false,
            "Sync agent config from the host",
            &[
                "Every session, on any machine, gets a copy of the host's skills, instructions, MCP servers and settings. The host is this Mac.",
                "Takes effect at once, no restart. Never touches your real dotfiles: each session has its own config home.",
            ],
            vec![switch(ui, "settings-sync-master", live, true, SyncAction::ToggleEnabled, look)],
        ))
        .child(category(
            "Skills",
            &[
                "Host wins: a session keeps its own skills too, and the host's wins when two share a name.",
                "Host only: a session gets just the host's skills.",
            ],
            "settings-sync-skills",
            SyncCategory::Skills,
            sync.skills,
            vec![mode],
        ))
        .child(category(
            "Instructions",
            &["The host's instruction files for each agent."],
            "settings-sync-instructions",
            SyncCategory::Instructions,
            sync.instructions,
            vec![],
        ))
        .child(category(
            "MCP servers",
            &["The host's MCP servers. Off: a session uses only its own."],
            "settings-sync-mcp",
            SyncCategory::Mcp,
            sync.mcp,
            vec![],
        ))
        .child(category(
            "Settings",
            &["The host's agent settings, such as models and permissions. Off: a session uses only its own."],
            "settings-sync-settings",
            SyncCategory::Settings,
            sync.settings,
            vec![],
        ))
}

/// The Host wins / Host only control; `enabled` false dims it and stops input.
fn mode_control(ui: &Rc<Ui>, mode: SyncSkillsMode, enabled: bool, look: Look) -> AnyElement {
    row()
        .flex_shrink_0()
        .p(px(2.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .when(!enabled, |x| x.opacity(DIMMED))
        .children([
            ("settings-sync-mode-host-wins", "Host wins", SyncSkillsMode::HostWins),
            ("settings-sync-mode-host-only", "Host only", SyncSkillsMode::HostOnly),
        ].into_iter().map(|(id, name, value)| {
            let ui = ui.clone();
            let on = mode == value;
            div()
                .id(id)
                .role(Role::Tab)
                .aria_label(name)
                .aria_selected(on)
                .test_support()
                .px(px(12.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(on, |x| x.bg(rgb(look.card)).shadow_sm())
                .child(name)
                .when(enabled && !on, |x| {
                    x.cursor_pointer()
                        .on_click(move |_, _, cx| send(&ui, SyncAction::SetSkillsMode(value), cx))
                })
        }))
        .into_any_element()
}

fn machines_block(ui: &Rc<Ui>, state: &AppState, snapshot: &SyncSnapshot, machines: &[&Machine], look: Look) -> Div {
    let live = snapshot.sync.sync_enabled;
    let header = row()
        .px(px(14.))
        .py(px(6.))
        .items_center()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(div().flex_1().child(dim(look, "Machine")))
        .children(COLUMNS.iter().map(|name| div().w(px(COLUMN)).flex_shrink_0().child(dim(look, *name))));
    col()
        .gap(px(6.))
        .child(dim(look, "Machines").text_size(px(11.)).font_weight(FontWeight::SEMIBOLD))
        .child(dim(look, "Each machine follows the choices above unless you pick On or Off for it here."))
        .child(
            card(look)
                .mt(px(2.))
                .when(!live, |x| x.opacity(DIMMED))
                .child(header)
                .children(machines.iter().enumerate().map(|(idx, machine)| {
                    let flip_up = machines.len() >= 4 && idx + 1 == machines.len();
                    machine_row(ui, state, snapshot, machine, live, flip_up, look)
                })),
        )
}

/// One machine's departures, read from the snapshot.
fn departures(snapshot: &SyncSnapshot, machine: &Machine) -> MachineSyncSettings {
    snapshot.sync.machines.get(machine.id.as_str()).copied().unwrap_or_default()
}

fn machine_row(ui: &Rc<Ui>, state: &AppState, snapshot: &SyncSnapshot, machine: &Machine, live: bool, flip_up: bool, look: Look) -> Div {
    let current = departures(snapshot, machine);
    // This machine's Sync cell Off: its other cells keep their values, dimmed.
    let off = current.enabled == Some(false);
    let picks = [
        (SyncColumn::Sync, current.enabled),
        (SyncColumn::Skills, current.skills),
        (SyncColumn::Instructions, current.instructions),
        (SyncColumn::Mcp, current.mcp),
        (SyncColumn::Settings, current.settings),
    ];
    row()
        .px(px(14.))
        .py(px(8.))
        .items_center()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(div().flex_1().child(machine.name.clone()))
        .children(picks.into_iter().map(|(column, value)| {
            let cell = pick_cell(ui, state, machine, column, value, live, flip_up, look);
            cell.when(off && column != SyncColumn::Sync, |x| x.opacity(DIMMED))
        }))
}

/// The Default / On / Off menu's rows, as the frame draws them.
const CHOICES: [(&str, Option<bool>); 3] = [("Default", None), ("On", Some(true)), ("Off", Some(false))];

/// One Default / On / Off cell: a button, and under it the menu while open.
/// Default draws in the secondary colour and writes the field as unset.
#[allow(clippy::too_many_arguments)]
fn pick_cell(ui: &Rc<Ui>, state: &AppState, machine: &Machine, column: SyncColumn, value: Option<bool>, live: bool, flip_up: bool, look: Look) -> Div {
    let (text, color) = match value {
        None => ("Default", look.secondary),
        Some(true) => ("On", look.text),
        Some(false) => ("Off", look.text),
    };
    let slug = column_slug(column);
    let open = live && state.settings.sync.machine_menu.as_ref() == Some(&(machine.id.to_string(), column));
    let id = machine.id.to_string();
    let control = {
        let (ui, id) = (ui.clone(), id.clone());
        row()
            .id(SharedString::from(format!("settings-sync-machine-{}-{slug}", machine.id)))
            .role(Role::Button)
            .aria_label(format!("{} {}: {text}", machine.name, column_name(column)))
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
            .text_color(rgb(color))
            .flex_shrink_0()
            .child(text)
            .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .when(live, move |x| {
                x.cursor_pointer()
                    .on_click(move |_, _, cx| send(&ui, SyncAction::OpenMachineMenu(id.clone(), column), cx))
            })
    };
    let align_right = column == SyncColumn::Settings;
    div()
        .relative()
        .w(px(COLUMN))
        .flex()
        .flex_shrink_0()
        .child(control)
        .when(open, |cell| {
            let scrim_ui = ui.clone();
            cell.child(deferred(
                div()
                    .id(SharedString::from(format!(
                        "settings-sync-machine-{}-{}-scrim",
                        machine.id,
                        column_slug(column)
                    )))
                    .absolute()
                    .top(px(-10_000.))
                    .left(px(-10_000.))
                    .w(px(20_000.))
                    .h(px(20_000.))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        send(&scrim_ui, SyncAction::CloseMachineMenu, cx);
                    }),
            ))
            .child(deferred(pick_menu(
                ui,
                machine,
                column,
                value,
                look,
                flip_up,
                align_right,
            )))
        })
}

fn pick_menu(
    ui: &Rc<Ui>,
    machine: &Machine,
    column: SyncColumn,
    current: Option<bool>,
    look: Look,
    flip_up: bool,
    align_right: bool,
) -> impl IntoElement {
    col()
        .id(SharedString::from(format!("settings-sync-machine-{}-{}-menu", machine.id, column_slug(column))))
        .role(Role::ListBox)
        .aria_label(format!("{} {}", machine.name, column_name(column)))
        .test_support()
        .absolute()
        .when(flip_up, |m| m.bottom(px(28.)))
        .when(!flip_up, |m| m.top(px(28.)))
        .when(align_right, |m| m.right(px(0.)))
        .when(!align_right, |m| m.left(px(0.)))
        .w(px(96.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.))
        .children(CHOICES.map(|(name, value)| {
            let ui = ui.clone();
            let id = machine.id.to_string();
            let picked = value == current;
            row()
                .id(SharedString::from(format!("settings-sync-machine-{}-{}-{}", machine.id, column_slug(column), name.to_lowercase())))
                .role(Role::ListBoxOption)
                .aria_label(name)
                .aria_selected(picked)
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |x| x.bg(rgb(look.fill)))
                .child(div().w(px(12.)).flex_shrink_0().children(picked.then(|| chrome::icon(IconName::Check, look.accent, 12.))))
                .child(name)
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    send(&ui, SyncAction::PickMachine(id.clone(), column, value), cx);
                })
                .test_support()
        }))
}

fn column_slug(column: SyncColumn) -> &'static str {
    match column {
        SyncColumn::Sync => "sync",
        SyncColumn::Skills => "skills",
        SyncColumn::Instructions => "instructions",
        SyncColumn::Mcp => "mcp-servers",
        SyncColumn::Settings => "settings",
    }
}

fn column_name(column: SyncColumn) -> &'static str {
    match column {
        SyncColumn::Sync => COLUMNS[0],
        SyncColumn::Skills => COLUMNS[1],
        SyncColumn::Instructions => COLUMNS[2],
        SyncColumn::Mcp => COLUMNS[3],
        SyncColumn::Settings => COLUMNS[4],
    }
}

fn mount_card(ui: &Rc<Ui>, snapshot: &SyncSnapshot, look: Look) -> Div {
    // D908: `settings-fusefs-enabled` shows the saved switch and never dims;
    // the daemon's live state adds at most one note or one needs-you line.
    let status = match &snapshot.fusefs_status {
        FuseMountStatus::Starting => Some("Mounting…".to_owned()),
        FuseMountStatus::Mounted { path } => {
            let home = std::env::var("HOME").ok();
            Some(format!("Mounted at {}", with_home_as_tilde(path, home.as_deref())))
        }
        FuseMountStatus::Off | FuseMountStatus::CannotSetUp { .. } => None,
    };
    let more: Vec<AnyElement> = status
        .into_iter()
        .map(|text| dim(look, text).id("settings-fusefs-status").test_support().into_any_element())
        .collect();
    let card = card(look).child(line_with(
        look,
        false,
        "Browse other machines as files",
        &[
            "Mounts one folder that shows other machines' sessions, channels and requests.",
            "Session folders on this machine stay on its own disk either way.",
        ],
        more,
        vec![switch(ui, "settings-fusefs-enabled", snapshot.fusefs_enabled, true, SyncAction::ToggleFuseFs, look)],
    ));
    match &snapshot.fusefs_status {
        // With the switch off nothing is blocked, so nothing is said.
        FuseMountStatus::CannotSetUp { reason } if snapshot.fusefs_enabled => card.child(
            row()
                .id("settings-fusefs-problem")
                .test_support()
                .items_start()
                .gap(px(8.))
                .px(px(14.))
                .py(px(8.))
                .rounded_b(px(8.))
                .bg(rgb(look.attention_bg))
                .child(div().pt(px(1.)).flex_shrink_0().child(chrome::icon(IconName::Info, chrome::ORANGE, 12.)))
                .child(div().flex_1().child(format!("{reason} Sessions reach other machines through the files tools instead."))),
        ),
        _ => card,
    }
}

/// `path` with the home folder written as `~`, when it is inside it.
fn with_home_as_tilde(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.map(|home| home.trim_end_matches('/')).filter(|home| !home.is_empty()) else {
        return path.to_owned();
    };
    match path.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ordered, with_home_as_tilde};
    use humata2_clients::model::{Machine, MachineKind};

    fn machine(id: &str, kind: MachineKind) -> Machine {
        Machine {
            id: humata2_clients::model::MachineId::new(id),
            name: id.to_owned(),
            kind,
            online: true,
            detail: String::new(),
            last_seen: String::new(),
            lost_at: None,
            stop: None,
            root: String::new(),
        }
    }

    #[test]
    fn this_mac_comes_first_and_the_rest_keep_sidebar_order() {
        let machines = [
            machine("devbox", MachineKind::Ssh { address: "devbox".into() }),
            machine("mac", MachineKind::ThisMac),
            machine("gpu", MachineKind::Ssh { address: "gpu".into() }),
        ];
        let names: Vec<&str> = ordered(&machines).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(names, ["mac", "devbox", "gpu"]);
    }

    #[test]
    fn the_home_folder_is_written_as_a_tilde() {
        assert_eq!(with_home_as_tilde("/Users/me/Humata", Some("/Users/me")), "~/Humata");
        assert_eq!(with_home_as_tilde("/Users/me/Humata", Some("/Users/me/")), "~/Humata");
        assert_eq!(with_home_as_tilde("/Users/me", Some("/Users/me")), "~");
        assert_eq!(with_home_as_tilde("/Users/mel/Humata", Some("/Users/me")), "/Users/mel/Humata");
        assert_eq!(with_home_as_tilde("/mnt/Humata", Some("/Users/me")), "/mnt/Humata");
        assert_eq!(with_home_as_tilde("/mnt/Humata", None), "/mnt/Humata");
    }
}
