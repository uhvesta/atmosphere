//! FUSEFSWIRE: Settings > Sync. One switch stops every copy of the host's
//! agent config at once; under it sit what is copied, each machine's
//! departures from those defaults, and the folder that shows other machines
//! as files.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::toolbar;
use crate::sidebar::Side;

/// What one machine does about one default: follow it, or depart from it.
#[derive(Clone, Copy)]
enum Pick {
    Default,
    On,
    Off,
}

/// One machine's picks, in column order: sync, skills, instructions, MCP
/// servers, settings.
const MACHINES: &[(&str, [Pick; 5])] = &[
    ("This Mac", [Pick::Default; 5]),
    ("devbox-1", [Pick::Default, Pick::Default, Pick::Default, Pick::On, Pick::Default]),
    ("gpu-box", [Pick::Off, Pick::Default, Pick::Default, Pick::Default, Pick::Default]),
];

const COLUMNS: [&str; 5] = ["Sync", "Skills", "Instructions", "MCP servers", "Settings"];

/// The width of one column of the machines table.
const COLUMN: f32 = 104.;

/// What the daemon reports about the mount, as the mount card shows it.
/// Starting draws as [`Mount::On`] with the line "Mounting…".
#[derive(Clone, Copy)]
enum Mount {
    /// The switch is off: no status line.
    Off,
    /// The switch is on, with one quiet status line under the notes.
    On(&'static str),
    /// The switch is on and the mount can't be set up: the daemon's reason.
    CannotSetUp(&'static str),
}

/// Settings > Sync at its defaults: sync on, skills and instructions copied,
/// MCP servers and settings not, the mount off, and two machines departing.
pub fn page(look: Look) -> Div {
    draw(look, Mount::Off)
}

/// Candidate 211b: the mount is switched on and can't be set up. The card
/// says why and what sessions use instead (D908).
pub fn mount_cannot(look: Look) -> Div {
    draw(look, Mount::CannotSetUp("This Mac has neither FSKit nor /dev/fuse."))
}

/// Candidate 211c: the mount is up, and the card says where it is (D908).
pub fn mounted(look: Look) -> Div {
    draw(look, Mount::On("Mounted at ~/Humata"))
}

fn draw(look: Look, mount: Mount) -> Div {
    let nav = |glyph: IconName, text: &'static str, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(10.))
            .rounded(px(6.))
            .when(on, |row| row.bg(c(look.selection)).text_color(c(look.selection_text)))
            .child(icon(glyph, if on { look.selection_text } else { look.accent }))
            .child(text)
    };
    // A settings row: its title over its notes, then its controls.
    let row = |indent: bool, title: &'static str, notes: &[&'static str], controls: Vec<Div>| {
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .pr(px(14.))
            .pl(px(if indent { 28. } else { 14. }))
            .py(px(10.))
            .border_b_1()
            .border_color(c(look.line))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(title)
                    .children(notes.iter().map(|note| secondary(look, *note))),
            )
            .children(controls)
    };
    let pick = |pick: Pick| {
        let cell = div().w(px(COLUMN)).flex().flex_shrink_0();
        match pick {
            Pick::Default => cell.child(popup(look, "Default").text_color(c(look.secondary))),
            Pick::On => cell.child(popup(look, "On")),
            Pick::Off => cell.child(popup(look, "Off")),
        }
    };
    let machine = |(name, picks): &(&'static str, [Pick; 5])| {
        // Sync off for this machine: its other picks keep their values, dimmed.
        let off = matches!(picks[0], Pick::Off);
        div()
            .flex()
            .items_center()
            .px(px(14.))
            .py(px(8.))
            .border_b_1()
            .border_color(c(look.line))
            .child(div().flex_1().child(*name))
            .children(picks.iter().copied().enumerate().map(|(column, value)| pick(value).when(off && column > 0, |cell| cell.opacity(0.4))))
    };
    let columns = div()
        .flex()
        .items_center()
        .px(px(14.))
        .py(px(6.))
        .border_b_1()
        .border_color(c(look.line))
        .child(div().flex_1().child(secondary(look, "Machine")))
        .children(COLUMNS.iter().map(|column| div().w(px(COLUMN)).flex_shrink_0().child(secondary(look, *column))));
    crate::pages::page(
        look,
        Side::at("Settings"),
        toolbar(look, "Settings", None, vec![]),
        div()
            .flex()
            .text_size(px(12.5))
            .child(
                div()
                    .w(px(200.))
                    .h_full()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .border_r_1()
                    .border_color(c(look.line))
                    .child(nav(IconName::Settings, "General", false))
                    .child(nav(IconName::Bot, "Agents", false))
                    .child(nav(IconName::FolderTree, "Sync", true))
                    .child(nav(IconName::Server, "Machines", false))
                    .child(nav(IconName::Columns2, "Layout", false))
                    .child(nav(IconName::ShieldCheck, "Agent access", false))
                    .child(nav(IconName::HardDrive, "Storage", false))
                    .child(nav(IconName::Bell, "Notifications", false))
                    .child(nav(IconName::Keyboard, "Keyboard", false)),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        card(look)
                            // The app's `settings-sync-master`.
                            .child(row(
                                false,
                                "Sync agent config from the host",
                                &[
                                    "Every session, on any machine, gets a copy of the host's skills, instructions, MCP servers and settings. The host is this Mac.",
                                    "Takes effect at once, no restart. Never touches your real dotfiles: each session has its own config home.",
                                ],
                                vec![switch(look, true)],
                            ))
                            .child(row(
                                true,
                                "Skills",
                                &[
                                    "Host wins: a session keeps its own skills too, and the host's wins when two share a name.",
                                    "Host only: a session gets just the host's skills.",
                                ],
                                vec![segmented(look, &["Host wins", "Host only"], 0), switch(look, true)],
                            ))
                            .child(row(true, "Instructions", &["The host's instruction files for each agent."], vec![switch(look, true)]))
                            .child(row(
                                true,
                                "MCP servers",
                                &["The host's MCP servers. Off: a session uses only its own."],
                                vec![switch(look, false)],
                            ))
                            .child(row(
                                true,
                                "Settings",
                                &["The host's agent settings, such as models and permissions. Off: a session uses only its own."],
                                vec![switch(look, false)],
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(label(look, "Machines"))
                            .child(secondary(look, "Each machine follows the choices above unless you pick On or Off for it here."))
                            .child(card(look).mt(px(2.)).child(columns).children(MACHINES.iter().map(machine))),
                    )
                    .child({
                        const TITLE: &str = "Browse other machines as files";
                        const WHAT: &str = "Mounts one folder that shows other machines' sessions, channels and requests.";
                        const LOCAL: &str = "Session folders on this machine stay on its own disk either way.";
                        // The app's `settings-fusefs-enabled`.
                        match mount {
                            Mount::Off => card(look).child(row(false, TITLE, &[WHAT, LOCAL], vec![switch(look, false)])),
                            // The app's `settings-fusefs-status`.
                            Mount::On(status) => card(look).child(row(false, TITLE, &[WHAT, LOCAL, status], vec![switch(look, true)])),
                            // The app's `settings-fusefs-problem`: the
                            // needs-you line, under the row and inside the card.
                            Mount::CannotSetUp(reason) => card(look).child(row(false, TITLE, &[WHAT, LOCAL], vec![switch(look, true)])).child(
                                div()
                                    .flex()
                                    .items_start()
                                    .gap(px(8.))
                                    .px(px(14.))
                                    .py(px(8.))
                                    .rounded_b(px(8.))
                                    .bg(c(look.attention_bg))
                                    .child(div().pt(px(1.)).flex_shrink_0().child(small_icon(IconName::Info, ORANGE)))
                                    .child(div().flex_1().child(format!("{reason} Sessions reach other machines through the files tools instead."))),
                            ),
                        }
                    }),
            ),
    )
}
