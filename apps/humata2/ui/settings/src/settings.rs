//! Native settings sections and the bounded session layout (frame 29).
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable as _, button::Button, input::Input, scroll::ScrollableElement as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::daemon::{
    IDLE_NUDGE_CHOICES, TodoAutomation, TodoMode, WebTodoStatus, sample_branch,
};
use humata2_clients::model::Agent;
use humata2_settings::TodoAutomationAction as TodoAction;
use humata2_settings::{
    AgentsAction, DeskRows, FONT_SIZES, GeneralAction as G, Row, Section, SettingsAction as A,
    Theme, WebTodoAction,
};
use std::rc::Rc;

use crate::settings_providers as providers;
pub use providers::ProvidersInputs;

pub fn dim(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}
/// Settings › Agents › Agents (AGENTSET, D725): one switch per agent kind,
/// custom kinds included, and under an enabled one "Ask in the Inbox" (D782)
/// and, when it has an install command, "Install when missing".
/// Nothing but a loading line until the daemon has said which are on.
fn agent_switches_block(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let mut block = col()
        .gap(px(8.))
        .child(dim(look, "Agents").font_weight(FontWeight::SEMIBOLD))
        .child(dim(look, "Agents shown in pickers and started on your machines"));
    let Some(switches) = &state.settings.agents.switches else {
        return block.child(dim(look, "Loading agents"));
    };
    let line = |content: AnyElement, pad: f32| {
        row()
            .gap(px(16.))
            .pl(px(14. + pad))
            .pr(px(14.))
            .py(px(10.))
            .border_b_1()
            .border_color(rgb(look.line))
            .items_center()
            .child(content)
    };
    let mut list = col().gap(px(12.));
    for switch in switches {
        let mut rows = card(look);
        let id = &switch.kind;
        // A custom kind (D782) shows the name its definition gives.
        let name = switch
            .agent
            .map_or_else(|| switch.name.clone(), |agent| humata2_ui_core::chrome::runs(agent).to_owned());
        rows = rows.child(line(
            div()
                .flex_1()
                .min_w_0()
                .child(name)
                .into_any_element(),
            0.,
        ).child(toggle(
            ui,
            &format!("settings-agent-enabled-{id}"),
            switch.enabled,
            A::Agents(AgentsAction::Toggle(switch.kind.clone())),
            look,
        )));
        if switch.enabled {
            // D782: where this agent's own approval dialogs go; the
            // subtitle shows only while they stay in its terminal.
            rows = rows.child(line(
                col()
                    .flex_1()
                    .min_w_0()
                    .child("Ask in the Agent inbox")
                    .when(!switch.asks_in_inbox, |x| {
                        x.child(dim(look, "Off: it asks in its own terminal"))
                    })
                    .into_any_element(),
                18.,
            ).child(toggle(
                ui,
                &format!("settings-agent-asks-{id}"),
                switch.asks_in_inbox,
                A::Agents(AgentsAction::ToggleAsks(switch.kind.clone())),
                look,
            )));
        }
        if let (true, Some(command)) = (switch.enabled, &switch.install_command) {
            rows = rows.child(line(
                col()
                    .flex_1()
                    .min_w_0()
                    .child("Install when missing")
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(command.clone()),
                    )
                    .child(dim(look, "Never replaces one that's there"))
                    .into_any_element(),
                18.,
            ).child(toggle(
                ui,
                &format!("settings-agent-install-{id}"),
                switch.install_when_missing,
                A::Agents(AgentsAction::ToggleInstall(switch.kind.clone())),
                look,
            )));
        }
        list = list.child(rows);
    }
    block = block.child(list);
    if let Some(error) = &state.settings.error {
        block = block.child(
            div()
                .id("settings-agents-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_size(px(11.5))
                .text_color(rgb(0xff453a))
                .child(error.clone())
                .test_support(),
        );
    }
    block
}
fn open_card(look: Look) -> Div {
    col()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}
pub fn card(look: Look) -> Div {
    // A clipped card in the scrolling page would otherwise shrink to fit
    // the window and hide its rows instead of letting the page scroll.
    open_card(look).overflow_hidden().flex_shrink_0()
}
/// Scripts (MACHINESCRIPTS, D680): the owner's rules and script names, read
/// from the host, with the two ways to change them: the owner's editor and
/// the scripts folder in Finder. Nothing here writes a rule or a script.
fn machine_scripts_block(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let settings = &state.settings;
    let mut block = col()
        .gap(px(8.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Scripts"))
        .child(dim(
            look,
            "What Humata runs on a machine when it is added, reconnects, or a session arrives or leaves. The rules and scripts are your own files; Humata never changes them.",
        ));
    if let Some(error) = &settings.scripts_error {
        block = block.child(
            div()
                .id("settings-scripts-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_size(px(11.5))
                .text_color(rgb(0xff453a))
                .child(error.clone())
                .test_support(),
        );
    }
    if let Some(scripts) = &settings.scripts {
        if let Some(problem) = &scripts.problem {
            block = block.child(
                div()
                    .id("settings-scripts-problem")
                    .role(Role::Alert)
                    .aria_label(problem.clone())
                    .text_size(px(11.5))
                    .text_color(rgb(0xff453a))
                    .child(problem.clone())
                    .test_support(),
            );
        }
        if !scripts.rules.is_empty() {
            let mut rules = card(look);
            for rule in &scripts.rules {
                let steps = match rule.steps.len() {
                    1 => "1 step".to_owned(),
                    n => format!("{n} steps"),
                };
                rules = rules.child(setting(
                    look,
                    &rule.name,
                    &format!("{} \u{b7} {steps}", rule.triggers.join(" \u{b7} ")),
                    div().into_any_element(),
                ));
            }
            block = block.child(rules);
        }
        block = block.child(dim(
            look,
            if scripts.script_names.is_empty() {
                "No scripts in the scripts folder yet.".to_owned()
            } else {
                format!("Scripts: {}", scripts.script_names.join(", "))
            },
        ));
        for warning in &scripts.warnings {
            block = block.child(dim(look, warning.clone()));
        }
    }
    block.child(
        row()
            .gap(px(8.))
            .child(button(ui, "settings-scripts-edit", "Edit rules", AppAction::Settings(A::EditScriptRules), look))
            .child(button(
                ui,
                "settings-scripts-reveal",
                "Reveal scripts",
                AppAction::Settings(A::RevealScripts),
                look,
            )),
    )
}
/// The terminal font pop-up: the closed control, and the size list under it while open.
fn font_popup(ui: &Rc<Ui>, size: u8, open: bool, look: Look) -> Div {
    let label = SharedString::from(format!("Menlo {size}"));
    let toggle = {
        let ui = ui.clone();
        row()
            .id("settings-font")
            .role(Role::Button)
            .aria_label(format!("Terminal font: {label}"))
            .flex_shrink_0()
            .gap(px(6.))
            .h(px(24.))
            .pl(px(9.))
            .pr(px(5.))
            .rounded(px(6.))
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .cursor_pointer()
            .child(label)
            .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .on_click(move |_, _, cx| ui.send(AppAction::Settings(A::FontMenu), cx))
            .test_support()
    };
    div()
        .relative()
        .flex_shrink_0()
        .child(toggle)
        .when(open, |wrap| wrap.child(deferred(font_menu(ui, size, look))))
}
fn font_menu(ui: &Rc<Ui>, current: u8, look: Look) -> impl IntoElement {
    col()
        .id("settings-font-menu")
        .role(Role::ListBox)
        .aria_label("Terminal font size")
        .absolute()
        .top(px(28.))
        .right(px(0.))
        .w(px(120.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.))
        .children(FONT_SIZES.map(|size| {
            let ui = ui.clone();
            row()
                .id(SharedString::from(format!("settings-font-{size}")))
                .role(Role::ListBoxOption)
                .aria_label(format!("Menlo {size}"))
                .aria_selected(size == current)
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |x| x.bg(rgb(look.fill)))
                .child(div().w(px(12.)).flex_shrink_0().children(
                    (size == current).then(|| chrome::icon(IconName::Check, look.accent, 12.)),
                ))
                .child(format!("Menlo {size}"))
                .on_click(move |_, _, cx| ui.send(AppAction::Settings(A::SetFontSize(size)), cx))
                .test_support()
        }))
}
/// The Desk sessions in sidebar pop-up (D793): the closed control, and the
/// choices under it while open.
fn desk_rows_popup(ui: &Rc<Ui>, rows: DeskRows, open: bool, look: Look) -> Div {
    let label = SharedString::from(rows.label());
    let toggle = {
        let ui = ui.clone();
        row()
            .id("settings-desk-rows")
            .role(Role::Button)
            .aria_label(format!("Desk sessions in sidebar: {label}"))
            .flex_shrink_0()
            .gap(px(6.))
            .h(px(24.))
            .pl(px(9.))
            .pr(px(5.))
            .rounded(px(6.))
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .cursor_pointer()
            .child(label)
            .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .on_click(move |_, _, cx| ui.send(AppAction::Settings(A::DeskRowsMenu), cx))
            .test_support()
    };
    div()
        .relative()
        .flex_shrink_0()
        .child(toggle)
        .when(open, |wrap| wrap.child(deferred(desk_rows_menu(ui, rows, look))))
}
fn desk_rows_menu(ui: &Rc<Ui>, current: DeskRows, look: Look) -> impl IntoElement {
    col()
        .id("settings-desk-rows-menu")
        .role(Role::ListBox)
        .aria_label("Desk sessions in sidebar")
        .absolute()
        .top(px(28.))
        .right(px(0.))
        .w(px(120.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.))
        .children(DeskRows::OPTIONS.map(|rows| {
            let ui = ui.clone();
            let label = rows.label();
            let picked = rows == current;
            row()
                .id(SharedString::from(format!(
                    "settings-desk-rows-{}",
                    label.to_lowercase()
                )))
                .role(Role::ListBoxOption)
                .aria_label(label.clone())
                .aria_selected(picked)
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |x| x.bg(rgb(look.fill)))
                .child(div().w(px(12.)).flex_shrink_0().children(
                    picked.then(|| chrome::icon(IconName::Check, look.accent, 12.)),
                ))
                .child(label)
                .on_click(move |_, _, cx| ui.send(AppAction::Settings(A::SetDeskRows(rows)), cx))
                .test_support()
        }))
}
/// Settings › General › To-do (TODO2WIRE, D902): what Humata does by itself
/// when pull requests and items change. Shows the defaults until the daemon
/// has said.
fn todo_automation_block(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let section: TodoAutomation = state.settings.todo.section.unwrap_or_default();
    let send = |action: TodoAction| AppAction::Settings(A::Todo(action));
    let ask_auto = |id: &'static str, current: TodoMode, set: fn(TodoMode) -> TodoAction| {
        segmented(
            ui,
            look,
            [(TodoMode::Ask, "Ask"), (TodoMode::Auto, "Auto")].into_iter().map(|(mode, name)| {
                (format!("{id}-{name}"), name, current == mode, send(set(mode)))
            }),
        )
    };
    let wake = |id: &str, on: bool, action: TodoAction| {
        toggle(ui, id, on, A::Todo(action), look).into_any_element()
    };
    let web_todo_enabled = state.settings.web_todo.enabled();
    let web_todo_desc = match state
        .settings
        .web_todo
        .snapshot
        .as_ref()
        .map(|s| &s.status)
    {
        Some(WebTodoStatus::Listening { url }) => format!("Serve the To-do board in a local browser at {url}"),
        Some(WebTodoStatus::Starting) => "Starting the local loopback To-do web server…".to_owned(),
        Some(WebTodoStatus::Failed { reason }) => format!("Could not start the local To-do web server: {reason}"),
        _ => "Serve the To-do board at a local loopback URL so you can view it in a browser. Off by default.".to_owned(),
    };
    col()
        .gap(px(8.))
        .child(dim(look, "To-do").font_weight(FontWeight::SEMIBOLD))
        .child(dim(
            look,
            "What Humata does by itself when pull requests and items change. Ask puts a one-click question on the item in To-do and never stops an agent.",
        ))
        .child(
            open_card(look)
                .child(setting(
                    look,
                    "Web To-do",
                    &web_todo_desc,
                    toggle(
                        ui,
                        "settings-web-todo-enabled",
                        web_todo_enabled,
                        A::WebTodo(WebTodoAction::ToggleEnabled),
                        look,
                    )
                    .into_any_element(),
                ))
                .child(setting(
                    look,
                    "When a pull request merges",
                    "Ask: its item asks whether to close. Auto: Humata closes the item and shows Undo.",
                    ask_auto("settings-todo-pr-merged", section.on_pr_merged, TodoAction::SetOnPrMerged),
                ))
                .child(setting(
                    look,
                    "When an item closes",
                    "Ask: the item offers to archive its sessions. Auto: Humata archives them and shows Undo. A session with unsaved work or a running terminal is kept either way.",
                    ask_auto(
                        "settings-todo-close-archive",
                        section.on_todo_closed_archive_sessions,
                        TodoAction::SetOnClosedArchive,
                    ),
                ))
                .child(setting(
                    look,
                    "Wake the agent when checks fail",
                    "Sends the failing checks to the item’s session as a note when its turn ends. Off: the item shows Needs attention and you send it.",
                    wake("settings-todo-wake-ci", section.auto_wake_on_ci_fail, TodoAction::ToggleWakeOnCiFail),
                ))
                .child(setting(
                    look,
                    "Wake the agent on review comments",
                    "Sends new review comments to the item’s session the same way. Off: the item offers Send comments to session.",
                    wake(
                        "settings-todo-wake-comments",
                        section.auto_wake_on_review_comments,
                        TodoAction::ToggleWakeOnReviewComments,
                    ),
                ))
                .child(setting(
                    look,
                    "Nudge about idle items",
                    "An In progress item with no activity for this long asks whether to carry on, take it back or close it.",
                    idle_nudge_popup(ui, section.idle_nudge_days, state.settings.todo.idle_menu, look)
                        .into_any_element(),
                )),
        )
}
/// The segmented control the Long lines row uses, for any labelled choices.
fn segmented(
    ui: &Rc<Ui>,
    look: Look,
    options: impl Iterator<Item = (String, &'static str, bool, AppAction)>,
) -> AnyElement {
    row()
        .flex_shrink_0()
        .p(px(2.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .children(options.map(|(id, name, on, action)| {
            let ui = ui.clone();
            div()
                .id(SharedString::from(id))
                .role(Role::Tab)
                .aria_label(name)
                .aria_selected(on)
                .test_support()
                .px(px(12.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(on, |x| {
                    x.bg(rgb(look.card))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border(px(0.5))
                        .border_color(look.hairline())
                        .shadow_xs()
                })
                .child(name)
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    if !on {
                        ui.send(action.clone(), cx)
                    }
                })
        }))
        .into_any_element()
}
fn idle_days_label(days: u32) -> String {
    if days == 1 { "1 day".to_owned() } else { format!("{days} days") }
}
/// The Nudge about idle items pop-up: the closed control, and the choices
/// under it while open.
fn idle_nudge_popup(ui: &Rc<Ui>, days: u32, open: bool, look: Look) -> Div {
    let label = SharedString::from(idle_days_label(days));
    let toggle = {
        let ui = ui.clone();
        row()
            .id("settings-todo-idle-nudge")
            .role(Role::Button)
            .aria_label(format!("Nudge about idle items: {label}"))
            .flex_shrink_0()
            .gap(px(6.))
            .h(px(24.))
            .pl(px(9.))
            .pr(px(5.))
            .rounded(px(6.))
            .bg(rgb(look.fill))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .cursor_pointer()
            .child(label)
            .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
            .on_click(move |_, _, cx| {
                ui.send(AppAction::Settings(A::Todo(TodoAction::IdleMenu)), cx)
            })
            .test_support()
    };
    div()
        .relative()
        .flex_shrink_0()
        .child(toggle)
        .when(open, |wrap| wrap.child(deferred(idle_nudge_menu(ui, days, look))))
}
fn idle_nudge_menu(ui: &Rc<Ui>, current: u32, look: Look) -> impl IntoElement {
    col()
        .id("settings-todo-idle-nudge-menu")
        .role(Role::ListBox)
        .aria_label("Nudge about idle items")
        .absolute()
        .top(px(28.))
        .right(px(0.))
        .w(px(120.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.))
        .children(IDLE_NUDGE_CHOICES.map(|days| {
            let ui = ui.clone();
            let label = idle_days_label(days);
            let picked = days == current;
            row()
                .id(SharedString::from(format!("settings-todo-idle-nudge-{days}")))
                .role(Role::ListBoxOption)
                .aria_label(label.clone())
                .aria_selected(picked)
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |x| x.bg(rgb(look.fill)))
                .child(div().w(px(12.)).flex_shrink_0().children(
                    picked.then(|| chrome::icon(IconName::Check, look.accent, 12.)),
                ))
                .child(label)
                .on_click(move |_, _, cx| {
                    ui.send(AppAction::Settings(A::Todo(TodoAction::SetIdleNudgeDays(days))), cx)
                })
                .test_support()
        }))
}
pub fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .label(label)
        .small()
        .h(px(24.))
        .px(px(10.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}
fn toggle(ui: &Rc<Ui>, id: &str, on: bool, action: A, look: Look) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(SharedString::from(id.to_owned()))
        .role(Role::Button)
        .aria_label(if on { "On" } else { "Off" })
        .aria_selected(on)
        .test_support()
        .w(px(28.))
        .h(px(16.))
        .rounded_full()
        .p(px(2.))
        .bg(rgb(if on { 0x28cd41 } else { look.fill }))
        .flex()
        .when(on, |x| x.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff)))
        .cursor_pointer()
        .on_click(move |_, _, cx| ui.send(AppAction::Settings(action.clone()), cx))
}
/// Settings > Storage > Sessions (MASSDELETE, D875): Delete all sessions…
/// opens the plan; deleting only some is done by ticking them.
fn sessions_block(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let count = state
        .workspace
        .sessions
        .iter()
        .filter(|session| !state.workspace.desk.sessions.contains_key(&session.id))
        .count();
    let note = format!(
        "Every live and archived session on every machine that can be reached: {count} now. \
         You see the full plan first and type the count to confirm. Desk sessions are not part of this."
    );
    col()
        .gap(px(8.))
        .child(dim(look, "Sessions").font_weight(FontWeight::SEMIBOLD))
        .child(card(look).child(setting(
            look,
            "Delete all sessions",
            &note,
            button(
                ui,
                "settings-delete-all-sessions",
                "Delete all sessions\u{2026}",
                AppAction::MassDelete(humata2_mass_delete::MassDeleteAction::OpenAll),
                look,
            )
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(0xff453a))
            .text_color(rgb(0xff453a))
            .into_any_element(),
        )))
        .child(dim(
            look,
            "To delete only some, tick them in the sidebar, on Home or on a machine\u{2019}s page: \u{2318}-click a session, or use Select.",
        ))
}

fn setting(look: Look, title: &str, note: &str, control: AnyElement) -> Div {
    row()
        .gap(px(16.))
        .px(px(14.))
        .py(px(10.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(title.to_owned())
                .child(dim(look, note.to_owned())),
        )
        .child(control)
}
pub fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    provider_inputs: &ProvidersInputs,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let prefs = &state.settings.preferences;
    let nav = col()
        .w(px(200.))
        .h_full()
        .flex_shrink_0()
        .p(px(10.))
        .gap(px(1.))
        .border_r_1()
        .border_color(rgb(look.line))
        .children(
            [
                (Section::General, IconName::Settings, "General"),
                (Section::Agents, IconName::Bot, "Agents"),
                (Section::Sync, IconName::FolderTree, "Sync"),
                (Section::Machines, IconName::Server, "Machines"),
                (Section::Layout, IconName::Columns2, "Layout"),
                (Section::Access, IconName::ShieldCheck, "Agent access"),
                (Section::Browser, IconName::Globe, "Browser"),
                (Section::Storage, IconName::HardDrive, "Storage"),
                (Section::Notifications, IconName::Bell, "Notifications"),
                (Section::Keyboard, IconName::Keyboard, "Keyboard"),
            ]
            .into_iter()
            .map(|(section, glyph, name)| {
                let ui = ui.clone();
                let selected = state.settings.section == section;
                row()
                    .id(SharedString::from(format!("settings-section-{name}")))
                    .role(Role::Tab)
                    .aria_label(name)
                    .aria_selected(selected)
                    .test_support()
                    .h(px(28.))
                    .px(px(10.))
                    .gap(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |x| {
                        x.bg(rgb(look.selection)).text_color(rgb(0xffffff))
                    })
                    .child(chrome::icon(
                        glyph,
                        if selected { 0xffffff } else { look.accent },
                        15.,
                    ))
                    .child(name)
                    .on_click(move |_, _, cx| ui.send(AppAction::Settings(A::Section(section)), cx))
            }),
        );
    let scroll = window
        .use_keyed_state("settings-content-scroll", cx, |_, _| ScrollHandle::new())
        .read(cx)
        .clone();
    let mut body = col()
        .id("settings-content")
        .size_full()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .p(px(20.))
        .gap(px(14.));
    match state.settings.section {
        Section::Layout => {
            let themes = row()
                .flex_shrink_0()
                .p(px(2.))
                .rounded(px(7.))
                .bg(rgb(look.fill))
                .children(
                    [
                        (Theme::System, "System"),
                        (Theme::Light, "Light"),
                        (Theme::Dark, "Dark"),
                    ]
                    .into_iter()
                    .map(|(theme, name)| {
                        let ui = ui.clone();
                        div()
                            .id(SharedString::from(format!("settings-theme-{name}")))
                            .role(Role::Tab)
                            .aria_label(name)
                            .aria_selected(prefs.theme == theme)
                            .test_support()
                            .px(px(12.))
                            .py(px(2.))
                            .rounded(px(5.))
                            .text_size(px(11.5))
                            .when(prefs.theme == theme, |x| {
                                x.bg(rgb(look.card))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .border(px(0.5))
                                    .border_color(look.hairline())
                                    .shadow_xs()
                            })
                            .child(name)
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::Settings(A::SetTheme(theme)), cx)
                            })
                    }),
                );
            let long_lines =
                row()
                    .flex_shrink_0()
                    .p(px(2.))
                    .rounded(px(7.))
                    .bg(rgb(look.fill))
                    .children([(true, "Wrap"), (false, "Scroll")].into_iter().map(
                        |(wrap, name)| {
                            let ui = ui.clone();
                            let on = prefs.wrap_diffs == wrap;
                            div()
                                .id(SharedString::from(format!("settings-long-lines-{name}")))
                                .role(Role::Tab)
                                .aria_label(name)
                                .aria_selected(on)
                                .test_support()
                                .px(px(12.))
                                .py(px(2.))
                                .rounded(px(5.))
                                .text_size(px(11.5))
                                .when(on, |x| {
                    x.bg(rgb(look.card))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border(px(0.5))
                        .border_color(look.hairline())
                        .shadow_xs()
                })
                                .child(name)
                                .cursor_pointer()
                                .on_click(move |_, _, cx| {
                                    if !on {
                                        ui.send(AppAction::Settings(A::ToggleWrap), cx)
                                    }
                                })
                        },
                    ));
            body = body.child(
                open_card(look)
                    .child(setting(
                        look,
                        "Appearance",
                        "Follows the system unless you pick one.",
                        themes.into_any_element(),
                    ))
                    .child(setting(
                        look,
                        "Terminal font",
                        "Used by every terminal and the diff.",
                        font_popup(ui, prefs.font_size, state.settings.font_menu, look)
                            .into_any_element(),
                    ))
                    .child(setting(
                        look,
                        "Sidebar shows Pinned",
                        "Pinned sessions, reviews and machines at the top.",
                        toggle(
                            ui,
                            "settings-pinned",
                            prefs.show_pinned,
                            A::TogglePinned,
                            look,
                        )
                        .into_any_element(),
                    ))
                    .child(setting(
                        look,
                        "Desk sessions in sidebar",
                        "The ones you used last, under Desk on every page.",
                        desk_rows_popup(ui, prefs.desk_rows, state.settings.desk_rows_menu, look)
                            .into_any_element(),
                    ))
                    .child(setting(
                        look,
                        "Long lines",
                        "Code in the diff, the editor, Files and reviews.",
                        long_lines.into_any_element(),
                    )),
            );
            let font_input = provider_inputs.text_input(
                "settings-terminal-font",
                "Menlo",
                &state.settings.general.terminal_font_draft,
                |text| AppAction::Settings(A::General(G::TerminalFontChanged(text))),
                Some(AppAction::Settings(A::General(G::SaveTerminalFont))),
                window,
                cx,
            );
            let scrollback_input = provider_inputs.text_input(
                "settings-terminal-scrollback",
                "10000",
                &state.settings.general.scrollback_draft,
                |text| AppAction::Settings(A::General(G::ScrollbackChanged(text))),
                Some(AppAction::Settings(A::General(G::SaveScrollback))),
                window,
                cx,
            );
            body = body.child(
                card(look)
                    .child(setting(
                        look,
                        "Terminal font family",
                        "Synced with the daemon across clients.",
                        row()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div().w(px(160.)).child(
                                    Input::new(&font_input)
                                        .small()
                                        .min_h(px(26.))
                                        .rounded(px(6.))
                                        .bg(rgb(look.content))
                                        .border_color(rgb(look.line))
                                        .text_size(px(12.)),
                                ),
                            )
                            .child(button(
                                ui,
                                "settings-terminal-font-save",
                                "Save",
                                AppAction::Settings(A::General(G::SaveTerminalFont)),
                                look,
                            ))
                            .into_any_element(),
                    ))
                    .child(setting(
                        look,
                        "Scrollback lines",
                        "Lines kept per terminal tab on each machine.",
                        row()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div().w(px(100.)).child(
                                    Input::new(&scrollback_input)
                                        .small()
                                        .min_h(px(26.))
                                        .rounded(px(6.))
                                        .bg(rgb(look.content))
                                        .border_color(rgb(look.line))
                                        .text_size(px(12.)),
                                ),
                            )
                            .child(button(
                                ui,
                                "settings-scrollback-save",
                                "Save",
                                AppAction::Settings(A::General(G::SaveScrollback)),
                                look,
                            ))
                            .into_any_element(),
                    )),
            );
        }
        Section::General => {
            let general = &state.settings.general;
            let current = general.current();
            let branch_prefix_input = provider_inputs.text_input(
                "settings-branch-prefix",
                "e.g. alice/",
                &general.branch_prefix_draft,
                |text| AppAction::Settings(A::General(G::BranchPrefixChanged(text))),
                Some(AppAction::Settings(A::General(G::SaveBranchPrefix))),
                window,
                cx,
            );
            let pr_days_input = provider_inputs.text_input(
                "settings-retention-pr-days",
                "0",
                &general.archive_pr_days_draft,
                |text| AppAction::Settings(A::General(G::ArchivePrDaysChanged(text))),
                Some(AppAction::Settings(A::General(G::SaveRetention))),
                window,
                cx,
            );
            let idle_days_input = provider_inputs.text_input(
                "settings-retention-idle-days",
                "0",
                &general.archive_idle_days_draft,
                |text| AppAction::Settings(A::General(G::ArchiveIdleDaysChanged(text))),
                Some(AppAction::Settings(A::General(G::SaveRetention))),
                window,
                cx,
            );
            let preview = sample_branch(&general.branch_prefix_draft);
            let reset_label = reset_branch_prefix_label(general.default_branch_prefix.as_deref());

            body = body
                .child(
                    card(look)
                        .p(px(14.))
                        .gap(px(10.))
                        .child(
                            col()
                                .child("Default branch prefix")
                                .child(dim(
                                    look,
                                    if current.branch_prefix_is_default {
                                        "Using your GitHub login as the default prefix for new session branches."
                                    } else {
                                        "Custom prefix used for new session branches unless a channel overrides it."
                                    },
                                )),
                        )
                        .child(
                            row()
                                .gap(px(8.))
                                .items_center()
                                .child(
                                    div().w(px(220.)).child(
                                        Input::new(&branch_prefix_input)
                                            .small()
                                            .min_h(px(26.))
                                            .rounded(px(6.))
                                            .bg(rgb(look.content))
                                            .border_color(rgb(look.line))
                                            .text_size(px(12.)),
                                    ),
                                )
                                .child(button(
                                    ui,
                                    "settings-branch-prefix-save",
                                    "Save",
                                    AppAction::Settings(A::General(G::SaveBranchPrefix)),
                                    look,
                                ))
                                .when(!current.branch_prefix_is_default, |x| {
                                    x.child(button(
                                        ui,
                                        "settings-branch-prefix-reset",
                                        reset_label.clone(),
                                        AppAction::Settings(A::General(G::ResetBranchPrefix)),
                                        look,
                                    ))
                                })
                                .child(button(
                                    ui,
                                    "settings-refresh-gh-login",
                                    "Refresh GitHub login",
                                    AppAction::Settings(A::General(G::RefreshGhLogin)),
                                    look,
                                )),
                        )
                        .child(
                            row()
                                .gap(px(6.))
                                .child(dim(look, "Sample branch:"))
                                .child(
                                    div()
                                        .id("settings-branch-prefix-preview")
                                        .font_family("Menlo")
                                        .text_size(px(11.5))
                                        .text_color(rgb(look.text))
                                        .child(preview),
                                ),
                        )
                        .children(general.error.clone().map(|error| {
                            div()
                                .id("settings-general-error")
                                .role(Role::Alert)
                                .aria_label(error.clone())
                                .text_size(px(11.5))
                                .text_color(rgb(0xff453a))
                                .child(error)
                                .test_support()
                        })),
                )
                .child(card(look).child(setting(
                    look,
                    "Contain agents",
                    "Security: restrict agents to their session clones. Applies when a session next launches or resumes.",
                    toggle(
                        ui,
                        "settings-contain-agents",
                        current.security.contain_agents,
                        A::General(G::ToggleContainAgents),
                        look,
                    )
                    .into_any_element(),
                )))
                .child(
                    card(look)
                        .p(px(14.))
                        .gap(px(10.))
                        .child(
                            col()
                                .child("Retention defaults")
                                .child(dim(
                                    look,
                                    "Days before auto-archiving PR reviews and idle sessions (0 = off).",
                                )),
                        )
                        .child(
                            row()
                                .gap(px(10.))
                                .items_center()
                                .child(dim(look, "PR reviews (days):"))
                                .child(
                                    div().w(px(70.)).child(
                                        Input::new(&pr_days_input)
                                            .small()
                                            .min_h(px(26.))
                                            .rounded(px(6.))
                                            .bg(rgb(look.content))
                                            .border_color(rgb(look.line))
                                            .text_size(px(12.)),
                                    ),
                                )
                                .child(dim(look, "Idle sessions (days):"))
                                .child(
                                    div().w(px(70.)).child(
                                        Input::new(&idle_days_input)
                                            .small()
                                            .min_h(px(26.))
                                            .rounded(px(6.))
                                            .bg(rgb(look.content))
                                            .border_color(rgb(look.line))
                                            .text_size(px(12.)),
                                    ),
                                )
                                .child(button(
                                    ui,
                                    "settings-retention-save",
                                    "Save",
                                    AppAction::Settings(A::General(G::SaveRetention)),
                                    look,
                                )),
                        ),
                )
                .child(
                    card(look).child(setting(
                        look,
                        "Confirm bulk approvals",
                        "Ask before approving requests that cannot be undone.",
                        toggle(
                            ui,
                            "settings-confirm",
                            prefs.confirm_destructive,
                            A::ToggleConfirm,
                            look,
                        )
                        .into_any_element(),
                    )),
                )
            .child(card(look).child(setting(
                look,
                Row::FreeHiddenTerminals.title(),
                "A terminal you have not looked at for 30 seconds gives its screen memory back and shows again when you open it. Turn off to keep every screen in memory. Terminals with unsent or unconfirmed typing are never freed.",
                toggle(
                    ui,
                    "settings-free-hidden-terminals",
                    prefs.free_hidden_terminals,
                    A::ToggleFreeHiddenTerminals,
                    look,
                )
                .into_any_element(),
            )))
            // FAST (D787): less CPU and fewer redraws.
            .child(card(look).child(setting(
                look,
                Row::FastMode.title(),
                "Use less CPU and draw less often. Terminal cursors stop blinking, so a terminal you are not typing in draws nothing.",
                toggle(
                    ui,
                    "settings-fast-mode",
                    prefs.fast_mode,
                    A::ToggleFastMode,
                    look,
                )
                .into_any_element(),
            )))
            .child(card(look).child(setting(
                look,
                Row::UseWorktrees.title(),
                "Cache one bare git checkout per repository and create new sessions, forks, and PR reviews as git worktrees instead of full clones. Existing sessions keep their current checkouts.",
                toggle(
                    ui,
                    "settings-use-worktrees",
                    current.use_worktrees,
                    A::General(G::ToggleUseWorktrees),
                    look,
                )
                .into_any_element(),
            )));
            body = body.child(todo_automation_block(ui, state, look));
            // REPOS (ST-21): the repo library and the default scope.
            body = body.child(crate::settings_repositories::section(
                ui,
                state,
                provider_inputs,
                look,
                window,
                cx,
            ));
        }
        Section::Agents => {
            // AGENTSET (D725): the agents that are on, then the plain shell.
            let offered = state.settings.offered_agents();
            let default_agents: Vec<Agent> = offered
                .iter()
                .copied()
                .chain([Agent::Shell])
                .collect();
            // AGENTOFF (D915 6): when the saved default is turned off, explain
            // which agent new sessions fall back to (or Terminal when none is on).
            let off_default_note = if !default_agents.contains(&prefs.default_agent) {
                let saved = prefs.default_agent.name();
                Some(match offered.first() {
                    Some(fallback) => format!(
                        "{saved}, your default, is off. New sessions start with {} until you turn {saved} back on or pick another.",
                        fallback.name()
                    ),
                    None => format!(
                        "{saved}, your default, is off, and so is every other agent. New sessions can only start Terminal until you turn an agent on."
                    ),
                })
            } else {
                None
            };
            body = body
                .child(agent_switches_block(ui, state, look))
                .child(dim(look, "Default agent").font_weight(FontWeight::SEMIBOLD))
                .child(
                    card(look)
                        .p(px(14.))
                        .gap(px(8.))
                        .children(default_agents.into_iter().map(|agent| {
                            button(
                                ui,
                                SharedString::from(format!("settings-agent-{}", agent.command())),
                                agent.name(),
                                AppAction::Settings(A::SetAgent(agent)),
                                look,
                            )
                            .when(agent == prefs.default_agent, |x| {
                                x.bg(rgb(look.accent)).text_color(rgb(0xffffff))
                            })
                        }))
                        .when_some(off_default_note, |x, note| x.child(dim(look, note))),
                )
                .child({
                    let model_input = provider_inputs.text_input(
                        "settings-default-model",
                        "Leave empty for agent default",
                        &state.settings.general.default_model_draft,
                        |text| AppAction::Settings(A::General(G::DefaultModelChanged(text))),
                        Some(AppAction::Settings(A::General(G::SaveDefaultModel))),
                        window,
                        cx,
                    );
                    card(look).child(setting(
                        look,
                        "Default model",
                        "Optional model name passed to the default agent (preserved when switching agents).",
                        row()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div().w(px(200.)).child(
                                    Input::new(&model_input)
                                        .small()
                                        .min_h(px(26.))
                                        .rounded(px(6.))
                                        .bg(rgb(look.content))
                                        .border_color(rgb(look.line))
                                        .text_size(px(12.)),
                                ),
                            )
                            .child(button(
                                ui,
                                "settings-default-model-save",
                                "Save",
                                AppAction::Settings(A::General(G::SaveDefaultModel)),
                                look,
                            ))
                            .into_any_element(),
                    ))
                })
                .when(state.settings.codex_needs_trust, |x| {
                    x.child(card(look).child(setting(
                        look,
                        "Trust Humata's hooks in Codex",
                        "Start a Codex session and run /hooks once, then trust Humata's",
                        div().into_any_element(),
                    )))
                });
        }
        Section::Machines | Section::Storage => {
            body = body
                .child(
                    dim(
                        look,
                        if state.settings.section == Section::Storage {
                            "Storage by machine"
                        } else {
                            "Machines"
                        },
                    )
                    .font_weight(FontWeight::SEMIBOLD),
                )
                .children(state.workspace.machines.iter().map(|m| {
                    col()
                        .gap(px(8.))
                        .child(button(
                            ui,
                            SharedString::from(format!("settings-machine {}", m.id)),
                            m.name.clone(),
                            AppAction::Hosts(humata2_hosts::HostsAction::Select(m.id.clone())),
                            look,
                        ))
                        .when(state.settings.section == Section::Machines, |x| {
                            // Change… opens the Humata folder sheet: type
                            // the folder, Plan, then choose (FC-10).
                            x.child(card(look).child(setting(
                                look,
                                "Humata folder",
                                &m.root,
                                button(
                                    ui,
                                    SharedString::from(format!("settings-machine-root {}", m.id)),
                                    "Change…",
                                    AppAction::Hosts(humata2_hosts::HostsAction::OpenRoot(m.id.clone())),
                                    look,
                                )
                                .into_any_element(),
                            )))
                        })
                }))
                // PROVIDERS: what makes machines, above Scripts.
                .when(state.settings.section == Section::Machines, |x| {
                    x.child(providers::providers_block(ui, state, provider_inputs, look, window, cx))
                })
                // MACHINESCRIPTS (D680): rules and script names, with Edit rules and Reveal scripts.
                .when(state.settings.section == Section::Machines, |x| {
                    x.child(machine_scripts_block(ui, state, look))
                })
                // MASSDELETE (D875): the way to delete every session.
                .when(state.settings.section == Section::Storage, |x| {
                    x.child(sessions_block(ui, state, look))
                });
        }
        Section::Access => {
            body = body
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Agent access"))
                .child(dim(
                    look,
                    "What agents may do in every channel that doesn\u{2019}t say otherwise. A channel\u{2019}s own Agent access tab overrides these.",
                ))
                .child(crate::access::global(ui, state, look))
                .child(button(
                    ui,
                    "settings-open-inbox",
                    "Review Agent inbox",
                    AppAction::Switch(humata2_app::Route::Inbox),
                    look,
                ));
        }
        Section::Notifications => {
            body = body
                .child(crate::notifications::section(
                    ui,
                    state,
                    state.settings.notification_permission,
                    look,
                    AppAction::Settings(A::SendTestNotification),
                    AppAction::Settings(A::ToggleInAppToasts),
                    AppAction::Settings(A::RequestNotificationPermission),
                ))
                .child({
                    let general = &state.settings.general;
                    let current = general.current();
                    let quiet_from_input = provider_inputs.text_input(
                        "settings-quiet-from",
                        "22:00",
                        &general.quiet_from_draft,
                        |text| AppAction::Settings(A::General(G::QuietFromChanged(text))),
                        Some(AppAction::Settings(A::General(G::SaveQuietHours))),
                        window,
                        cx,
                    );
                    let quiet_until_input = provider_inputs.text_input(
                        "settings-quiet-until",
                        "07:30",
                        &general.quiet_until_draft,
                        |text| AppAction::Settings(A::General(G::QuietUntilChanged(text))),
                        Some(AppAction::Settings(A::General(G::SaveQuietHours))),
                        window,
                        cx,
                    );
                    card(look)
                        .child(setting(
                            look,
                            "Quiet hours",
                            "Local 24-hour times (HH:MM). Leave both empty to disable quiet hours.",
                            row()
                                .gap(px(8.))
                                .items_center()
                                .child(dim(look, "From"))
                                .child(
                                    div().w(px(76.)).child(
                                        Input::new(&quiet_from_input)
                                            .small()
                                            .min_h(px(26.))
                                            .rounded(px(6.))
                                            .bg(rgb(look.content))
                                            .border_color(rgb(look.line))
                                            .text_size(px(12.)),
                                    ),
                                )
                                .child(dim(look, "Until"))
                                .child(
                                    div().w(px(76.)).child(
                                        Input::new(&quiet_until_input)
                                            .small()
                                            .min_h(px(26.))
                                            .rounded(px(6.))
                                            .bg(rgb(look.content))
                                            .border_color(rgb(look.line))
                                            .text_size(px(12.)),
                                    ),
                                )
                                .child(button(
                                    ui,
                                    "settings-quiet-hours-save",
                                    "Save",
                                    AppAction::Settings(A::General(G::SaveQuietHours)),
                                    look,
                                ))
                                .into_any_element(),
                        ))
                        .child(setting(
                            look,
                            "VIP breaks quiet hours",
                            "Allow notifications from VIP sessions even during quiet hours.",
                            toggle(
                                ui,
                                "settings-vip-breaks-quiet-hours",
                                current.notifications.vip_breaks_quiet_hours,
                                A::General(G::ToggleVipBreaksQuietHours),
                                look,
                            )
                            .into_any_element(),
                        ))
                });
        }
        Section::Keyboard => {
            body = body
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Keyboard"))
                .child(button(
                    ui,
                    "settings-shortcuts",
                    "Keyboard shortcuts",
                    AppAction::ShowShortcuts,
                    look,
                ))
                .child(card(look).child(setting(
                    look,
                    Row::OptionAsAlt.title(),
                    "For Mac keyboards. ⌥ keys send Esc and the key, for shells and editors that bind ⌥ letters, instead of the character the layout gives. Some layouts then can't type characters that need ⌥. ⌥←, ⌥→ and ⌥⌫ move and delete by word either way.",
                    toggle(
                        ui,
                        "settings-option-as-alt",
                        prefs.option_as_alt,
                        A::ToggleOptionAsAlt,
                        look,
                    )
                    .into_any_element(),
                )));
        }
        // FUSEFSWIRE (D904): frame 211.
        Section::Sync => {
            body = body.child(crate::sync::body(ui, state, look));
        }
        // BROWSER: frame b1-settings-browser.
        Section::Browser => {
            body = body.child(crate::settings_browser::body(ui, state, look));
        }
    }
    row()
        .size_full()
        .min_h_0()
        .items_stretch()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .child(nav)
        .child({
            let pane = div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(body.test_support());
            if chrome::native_traffic_lights() {
                pane.vertical_scrollbar(&scroll).into_any_element()
            } else {
                pane.into_any_element()
            }
        })
}

pub fn toolbar(collapsed: bool, look: Look) -> Div {
    chrome::titlebar_drag(
        row()
            .h(px(52.))
            .flex_shrink_0()
            .px(px(20.))
            .gap(px(10.))
            .border_b_1()
            .border_color(rgb(look.line)),
    )
        .when(collapsed, |bar| bar.child(chrome::lights()))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Settings"),
        )
}

fn reset_branch_prefix_label(default_prefix: Option<&str>) -> String {
    match default_prefix.map(|p| p.trim_end_matches('/')).filter(|p| !p.is_empty()) {
        Some(user) => format!("Reset to @{user} default"),
        None => "Reset to default".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::reset_branch_prefix_label;
    use humata2_clients::daemon::sample_branch;
    use humata2_settings::{Row, Section};

    #[core::prelude::v1::test]
    fn sample_branch_preview_joins_prefix_with_fix_login() {
        assert_eq!(sample_branch("alice/"), "alice/fix-login");
        assert_eq!(sample_branch(""), "fix-login");
    }

    #[core::prelude::v1::test]
    fn reset_button_names_the_github_user_when_known() {
        assert_eq!(
            reset_branch_prefix_label(Some("alice/")),
            "Reset to @alice default"
        );
        assert_eq!(reset_branch_prefix_label(None), "Reset to default");
    }

    #[core::prelude::v1::test]
    fn play_sound_row_sits_between_desktop_notifications_and_permission_alerts() {
        let rows: Vec<Row> = Row::ALL
            .into_iter()
            .filter(|r| r.section() == Section::Notifications)
            .collect();
        assert_eq!(
            rows,
            vec![
                Row::DesktopNotifications,
                Row::PlaySound,
                Row::PermissionRequestAlerts,
            ]
        );
        assert_eq!(Row::PlaySound.title(), "Play a sound");
        assert_eq!(
            crate::notifications::play_sound_description(),
            "Play a short chime when a notice or in-app toast fires. Silent when the exact request is already open in front of you."
        );
    }
}
