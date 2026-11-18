//! Native settings sections and the bounded session layout (frame 29).
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable as _, button::Button};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::Agent;
use humata2_settings::{AgentsAction, DeskRows, FONT_SIZES, Row, Section, SettingsAction as A, Theme};
use std::rc::Rc;
fn dim(look: Look, text: impl Into<SharedString>) -> Div {
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
    let mut rows = card(look);
    for switch in switches {
        let id = &switch.kind;
        // A custom kind (D782) shows the name its definition gives.
        let name = switch
            .agent
            .map_or_else(|| switch.name.clone(), |agent| crate::desk::runs(agent).to_owned());
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
                    .child("Ask in the Inbox")
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
    }
    block = block.child(rows);
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
fn card(look: Look) -> Div {
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
fn button(
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
pub(crate) fn page(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
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
                (Section::Machines, IconName::Server, "Machines"),
                (Section::Layout, IconName::Columns2, "Layout"),
                (Section::Access, IconName::ShieldCheck, "Agent access"),
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
    let mut body = col()
        .id("settings-content")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
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
                            .when(prefs.theme == theme, |x| x.bg(rgb(look.card)).shadow_sm())
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
                                .when(on, |x| x.bg(rgb(look.card)).shadow_sm())
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
        }
        Section::General => {
            body = body.child(
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
            )));
            // REPOS (ST-21): the repo library and the default scope.
            body = body.child(crate::settings_repositories::section(ui, state, look));
        }
        Section::Agents => {
            // AGENTSET (D725): the agents that are on, then the plain shell.
            let default_agents: Vec<Agent> = state
                .settings
                .offered_agents()
                .into_iter()
                .chain([Agent::Shell])
                .collect();
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
                        })),
                )
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
                // MACHINESCRIPTS (D680): rules and script names, with Edit rules and Reveal scripts.
                .when(state.settings.section == Section::Machines, |x| {
                    x.child(machine_scripts_block(ui, state, look))
                });
        }
        Section::Access => {
            body=body.child(div().font_weight(FontWeight::SEMIBOLD).child("Agent access")).child(dim(look,"Requests to change items you own wait in Inbox. Review each request before allowing it.")).child(button(ui,"settings-open-inbox","Review Inbox",AppAction::Switch(humata2_app::Route::Inbox),look));
        }
        Section::Notifications => {
            body = body
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Notifications"),
                )
                .child(dim(
                    look,
                    "Requests that need you appear in Inbox and beside the session in the sidebar.",
                ))
                .child(card(look).child(setting(
                    look,
                    "Permission request alerts",
                    "Show brief alerts for new permission requests. Choose where approvals appear in Agents.",
                    toggle(
                        ui,
                        "settings-permission-alerts",
                        prefs.permission_request_alerts,
                        A::TogglePermissionRequestAlerts,
                        look,
                    )
                    .into_any_element(),
                )));
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
    }
    row()
        .size_full()
        .min_h_0()
        .items_stretch()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .child(nav)
        .child(body.test_support())
}

pub(crate) fn toolbar(look: Look) -> Div {
    row()
        .h(px(52.))
        .flex_shrink_0()
        .px(px(20.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Settings"),
        )
}
