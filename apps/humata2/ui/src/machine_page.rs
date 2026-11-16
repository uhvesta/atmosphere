//! A machine's page (HO-3 to HO-5, HO-13, MP-12 to MP-17, SK-2, SK-5, SK-8;
//! frame 59): its system, sessions, Humata folders, health and what its
//! agents can reach, with Sync skills and host variables.
//!
//! The page draws a [`MachinePageState`] and sends [`MachinePageAction`]s;
//! each click is one action. An offline machine keeps showing what was last
//! loaded, with one line saying only the link is down (HO-5).

use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::machines::{ClashChoice, MachineDetail, Reach, SkillsPreview, tool_label};
use humata2_clients::model::{Channel, MachineId, RuleSetting, Session, Status};
use humata2_hosts::page::{MachinePageAction, MachinePageState, SkillsDialog, Tab};
use std::cell::RefCell;
use std::rc::Rc;

const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
const MONO: &str = "Menlo";
/// A disk is low above this share used (HO-4).
const LOW_DISK_PERCENT: u64 = 85;

/// How the page's actions become the app's.
pub(crate) type Wrap = fn(MachinePageAction) -> AppAction;

/// What the page needs from the workspace to name things.
pub(crate) struct PageContext {
    /// The machine's name.
    pub(crate) name: String,
    /// Whether its daemon answers now.
    pub(crate) online: bool,
    /// Sessions that may be listed; the page picks those the detail names.
    pub(crate) sessions: Vec<Session>,
    /// Channels, to name folders and sessions.
    pub(crate) channels: Vec<Channel>,
}

/// The page's two inputs: the variable's name and its masked value.
pub(crate) struct MachinePageInputs {
    name: Entity<InputState>,
    name_echo: Rc<RefCell<Option<String>>>,
    value: Entity<InputState>,
    value_echo: Rc<RefCell<Option<String>>>,
    _subscriptions: Vec<Subscription>,
}

impl MachinePageInputs {
    /// The inputs; `wrap` carries a page action as the app's, such as
    /// `AppAction::MachinePage`.
    pub(crate) fn new(ui: &Rc<Ui>, wrap: Wrap, window: &mut Window, cx: &mut App) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("DATABASE_URL"));
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value")
                .masked(true)
        });
        let name_echo = Rc::new(RefCell::new(None::<String>));
        let value_echo = Rc::new(RefCell::new(None::<String>));
        let mut subscriptions = Vec::new();
        for (input, echo, change) in [
            (
                &name,
                &name_echo,
                MachinePageAction::VariableName as fn(String) -> MachinePageAction,
            ),
            (&value, &value_echo, MachinePageAction::VariableValue),
        ] {
            let (ui, echo) = (ui.clone(), echo.clone());
            subscriptions.push(cx.subscribe(
                input,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        if echo.borrow().as_deref() == Some(text.as_str()) {
                            *echo.borrow_mut() = None;
                            return;
                        }
                        ui.send(wrap(change(text)), cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        ui.send(wrap(MachinePageAction::SendVariable), cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ));
        }
        Self {
            name,
            name_echo,
            value,
            value_echo,
            _subscriptions: subscriptions,
        }
    }

    fn set(
        input: &Entity<InputState>,
        echo: &Rc<RefCell<Option<String>>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        if input.read(cx).value().as_ref() != text {
            *echo.borrow_mut() = Some(text.to_owned());
            let text = text.to_owned();
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }
}

fn dim(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

fn card(look: Look) -> Div {
    col()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}

fn label(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(text.into())
}

fn chip(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(11.))
        .text_color(rgb(look.secondary))
        .flex_shrink_0()
        .child(text.into())
}

/// A text link that sends `action`.
fn link(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    text: &str,
    action: AppAction,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(text.to_owned())
        .flex_shrink_0()
        .whitespace_nowrap()
        .text_color(rgb(look.accent))
        .cursor_pointer()
        .child(text.to_owned())
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        .test_support()
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn whole_gigabytes(bytes: u64) -> u64 {
    bytes / 1_000_000_000
}

fn reach_name(reach: &Reach) -> String {
    match reach {
        Reach::Ssh => "ssh".to_owned(),
        Reach::Provider(provider) => provider.clone(),
        Reach::Local => "local".to_owned(),
    }
}

/// The page for the machine route: names things from the workspace and
/// draws the page's own state.
pub(crate) fn route(
    ui: &Rc<Ui>,
    state: &AppState,
    machine: &MachineId,
    inputs: &MachinePageInputs,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let found = state.workspace.machine(machine);
    let context = PageContext {
        name: found.map_or_else(|| machine.to_string(), |found| found.name.clone()),
        online: found.is_some_and(|found| found.online),
        sessions: state
            .workspace
            .sessions
            .iter()
            .filter(|session| &session.machine == machine)
            .cloned()
            .collect(),
        channels: state.workspace.channels.clone(),
    };
    machine_page(
        ui,
        AppAction::MachinePage,
        &state.machine_page,
        &context,
        inputs,
        look,
        window,
        cx,
    )
}

/// The machine's page, for `state`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn machine_page(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    context: &PageContext,
    inputs: &MachinePageInputs,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (variable_name, variable_value) = state
        .variable
        .as_ref()
        .map_or(("", ""), |edit| (edit.name.as_str(), edit.value.as_str()));
    MachinePageInputs::set(&inputs.name, &inputs.name_echo, variable_name, window, cx);
    MachinePageInputs::set(&inputs.value, &inputs.value_echo, variable_value, window, cx);

    let mut page = col()
        .id("machine-page")
        .role(Role::Document)
        .aria_label(format!("Machine {}", context.name))
        .relative()
        .size_full()
        .min_h_0()
        .text_color(rgb(look.text))
        .child(header(ui, wrap, state, context, look))
        .child(tabs(ui, wrap, state, look));
    let body = match state.tab {
        Tab::Overview => overview(ui, wrap, state, context, inputs, look)
            .test_support()
            .into_any_element(),
        // The Storage tab's body is drawn by another milestone (frame 60).
        Tab::Storage => div().id("machine-storage").flex_1().into_any_element(),
    };
    page = page.child(body);
    if let Some(dialog) = &state.skills {
        page = page.child(skills_sheet(ui, wrap, dialog, context, look));
    }
    page.test_support().into_any_element()
}

fn header(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    context: &PageContext,
    look: Look,
) -> Div {
    let system = state.detail.as_ref().map(|detail| {
        format!(
            "{} · {} · {} ({}) · humatad {}",
            detail.os,
            detail.arch,
            detail.address,
            reach_name(&detail.reach),
            detail.daemon_version
        )
    });
    col()
        .flex_shrink_0()
        .child(
            row()
                .gap(px(8.))
                .h(px(48.))
                .px(px(20.))
                .child(
                    div()
                        .text_size(px(15.))
                        .text_color(rgb(look.secondary))
                        .child("Hosts"),
                )
                .child(chrome::icon(IconName::ChevronRight, look.tertiary, 12.))
                .child(chrome::status(if context.online {
                    Status::Working
                } else {
                    Status::Offline
                }))
                .child(
                    div()
                        .id("machine-title")
                        .role(Role::Heading)
                        .aria_label(context.name.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child(context.name.clone())
                        .test_support(),
                )
                .child(div().flex_1())
                // The actions the frame's ⋯ stands for, until the app has a menu to hold them.
                .child(
                    row()
                        .id("machine-menu")
                        .role(Role::Toolbar)
                        .aria_label("More")
                        .gap(px(4.))
                        .child(widgets::ghost(
                            ui,
                            "machine-sync-skills",
                            "Sync skills",
                            wrap(MachinePageAction::SyncSkills),
                        ))
                        // While the link-down row shows, Reconnect now lives there (HO-5).
                        .children((context.online || state.detail.is_none()).then(|| {
                            widgets::ghost(
                                ui,
                                "machine-reconnect-menu",
                                "Reconnect now",
                                wrap(MachinePageAction::ReconnectNow),
                            )
                        }))
                        .test_support(),
                )
                .child(
                    widgets::push_quiet(
                        ui,
                        "machine-open-shell",
                        "Open a shell",
                        wrap(MachinePageAction::OpenShell),
                        look,
                    )
                    .disabled(!context.online),
                )
                .child(
                    widgets::push_primary(
                        ui,
                        "machine-new-session",
                        format!("New session on {}", context.name),
                        wrap(MachinePageAction::NewSession),
                        look,
                    )
                    .disabled(!context.online),
                ),
        )
        .child(
            div()
                .px(px(20.))
                .pb(px(8.))
                .child(dim(look, system.unwrap_or_default())),
        )
}

fn tabs(ui: &Rc<Ui>, wrap: Wrap, state: &MachinePageState, look: Look) -> Stateful<Div> {
    row()
        .id("machine-tabs")
        .role(Role::Toolbar)
        .aria_label("Machine tabs")
        .items_end()
        .gap(px(20.))
        .h(px(32.))
        .px(px(20.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .text_size(px(12.5))
        .children([("Overview", Tab::Overview), ("Storage", Tab::Storage)].map(
            |(name, tab)| {
                let on = state.tab == tab;
                let ui = ui.clone();
                row()
                    .id(SharedString::from(format!("machine-tab-{name}")))
                    .role(Role::Tab)
                    .aria_label(name)
                    .aria_selected(on)
                    .h_full()
                    .px(px(2.))
                    .border_b_2()
                    .border_color(rgb(if on { look.accent } else { look.content }))
                    .text_color(rgb(if on { look.text } else { look.secondary }))
                    .when(on, |x| x.font_weight(FontWeight::MEDIUM))
                    .cursor_pointer()
                    .child(name)
                    .on_click(move |_, _, cx| {
                        ui.send(wrap(MachinePageAction::SelectTab(tab)), cx);
                    })
                    .test_support()
            },
        ))
}

fn overview(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    context: &PageContext,
    inputs: &MachinePageInputs,
    look: Look,
) -> Stateful<Div> {
    let mut page = col()
        .id("machine-overview")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .gap(px(12.))
        .p(px(20.))
        .text_size(px(12.5));
    if !context.online && state.detail.is_some() {
        page = page.child(
            row()
                .id("machine-link-down")
                .role(Role::Status)
                .aria_label("The link is down")
                .gap(px(10.))
                .child(div().child(format!(
                    "Only the link to {} is down. What you see is from the last time it answered.",
                    context.name
                )))
                .child(widgets::push_quiet(
                    ui,
                    "machine-reconnect",
                    "Reconnect now",
                    wrap(MachinePageAction::ReconnectNow),
                    look,
                ))
                .test_support(),
        );
    }
    if let Some(error) = &state.load_error {
        page = page.child(
            row()
                .id("machine-load-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .gap(px(10.))
                .text_color(rgb(RED))
                .child(div().child(error.clone()))
                .child(widgets::push_quiet(
                    ui,
                    "machine-reload",
                    "Try again",
                    wrap(MachinePageAction::Reload),
                    look,
                ))
                .test_support(),
        );
    }
    if let Some(error) = &state.error {
        page = page.child(
            div()
                .id("machine-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_color(rgb(RED))
                .child(error.clone())
                .test_support(),
        );
    }
    let Some(detail) = &state.detail else {
        if state.loading {
            page = page.child(dim(look, "Loading…"));
        }
        return page;
    };
    page.child(
        row()
            .items_start()
            .gap(px(16.))
            .child(left(ui, wrap, detail, context, look))
            .child(right(ui, wrap, state, detail, inputs, look)),
    )
}

fn left(
    ui: &Rc<Ui>,
    wrap: Wrap,
    detail: &MachineDetail,
    context: &PageContext,
    look: Look,
) -> Div {
    let channel_name = |id: &humata2_clients::model::ChannelId| {
        context
            .channels
            .iter()
            .find(|channel| &channel.id == id)
            .map_or_else(
                || if id.as_str() == humata2_clients::desk::FOLDER { "Desk".to_owned() } else { id.to_string() },
                |channel| format!("# {}", channel.name),
            )
    };
    let mut sessions = card(look).overflow_hidden();
    let listed = detail
        .sessions
        .iter()
        .filter_map(|id| context.sessions.iter().find(|session| &session.id == id));
    for (index, session) in listed.enumerate() {
        let ui = ui.clone();
        let action = wrap(MachinePageAction::OpenSession(session.id.clone()));
        sessions = sessions.child(
            row()
                .id(("machine-session", index))
                .role(Role::Button)
                .aria_label(session.title.clone())
                .gap(px(10.))
                .px(px(12.))
                .h(px(36.))
                .border_b_1()
                .border_color(rgb(look.line))
                .cursor_pointer()
                .child(chrome::status(session.status))
                .child(
                    div()
                        .w(px(190.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .font_weight(FontWeight::MEDIUM)
                        .child(session.title.clone()),
                )
                .child(
                    div()
                        .w(px(130.))
                        .whitespace_nowrap()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(channel_name(&session.channel)),
                )
                .child(
                    div()
                        .flex_1()
                        .whitespace_nowrap()
                        .text_color(rgb(look.secondary))
                        .child(if session.agent == humata2_clients::model::Agent::Claude {
                            "Claude Code"
                        } else {
                            session.agent.name()
                        }),
                )
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
                .test_support(),
        );
    }
    let mut folders = card(look).overflow_hidden();
    for (index, folder) in detail.folders.iter().enumerate() {
        let sessions_text = match folder.sessions {
            0 => "no sessions".to_owned(),
            1 => "1 session".to_owned(),
            n => format!("{n} sessions"),
        };
        folders = folders.child(
            row()
                .id(("machine-folder", index))
                .role(Role::ListItem)
                .aria_label(folder.path.clone())
                .gap(px(10.))
                .px(px(12.))
                .h(px(36.))
                .border_b_1()
                .border_color(rgb(look.line))
                .child(chrome::icon(IconName::FolderOpen, look.secondary, 12.))
                .child(
                    div()
                        .w(px(220.))
                        .whitespace_nowrap()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .child(folder.path.clone()),
                )
                .child(div().flex_1().whitespace_nowrap().child(dim(
                    look,
                    format!("{sessions_text} · {}", gigabytes(folder.bytes)),
                )))
                .child(link(
                    ui,
                    ("machine-folder-open", index),
                    "Open in Files",
                    wrap(MachinePageAction::OpenFolder(folder.channel.clone())),
                    look,
                ))
                .test_support(),
        );
    }
    col()
        .flex_1()
        .min_w_0()
        .gap(px(16.))
        .child(
            col()
                .gap(px(6.))
                .child(label(
                    look,
                    format!("Sessions on it · {}", detail.sessions.len()),
                ))
                .child(sessions),
        )
        .child(
            col()
                .gap(px(6.))
                .child(label(look, "Humata folders by channel"))
                .child(folders),
        )
}

fn right(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    detail: &MachineDetail,
    inputs: &MachinePageInputs,
    look: Look,
) -> Div {
    let health = &detail.health;
    let kv = |key: &'static str, value: Div| {
        row()
            .items_center()
            .gap(px(8.))
            .min_h(px(22.))
            .child(
                div()
                    .w(px(84.))
                    .flex_shrink_0()
                    .text_color(rgb(look.secondary))
                    .child(key),
            )
            .child(value.flex_1().min_w_0())
    };
    let github = match &health.gh {
        Some(login) => row()
            .flex_wrap()
            .gap(px(6.))
            .child(chrome::icon(IconName::Check, GREEN, 12.))
            .child(format!("signed in as {}", login.user))
            .children(login.scopes.iter().map(|scope| chip(look, scope.clone()))),
        None => row()
            .gap(px(6.))
            .text_color(rgb(ORANGE))
            .child("not signed in"),
    };
    let used = health.disk_total.saturating_sub(health.disk_free);
    let used_percent = if health.disk_total == 0 {
        0
    } else {
        used * 100 / health.disk_total
    };
    let low = used_percent > LOW_DISK_PERCENT;
    let disk = col()
        .gap(px(4.))
        .child(
            div()
                .id("machine-disk")
                .role(Role::Status)
                .aria_label(format!("Disk {used_percent}% used"))
                .h(px(5.))
                .w_full()
                .rounded_full()
                .bg(rgb(look.fill))
                .child(
                    div()
                        .w(relative(used_percent.min(100) as f32 / 100.))
                        .h_full()
                        .rounded_full()
                        .bg(rgb(if low { RED } else { look.accent })),
                )
                .test_support(),
        )
        .child(div().whitespace_nowrap().child(format!(
            "{} GB free of {} GB",
            whole_gigabytes(health.disk_free),
            whole_gigabytes(health.disk_total)
        )))
        .children(low.then(|| {
            div()
                .whitespace_nowrap()
                .text_size(px(11.5))
                .text_color(rgb(ORANGE))
                .child("New clones stop at 5 GB.")
        }));
    let mut variables = row().flex_wrap().gap(px(6.));
    for (index, name) in health.variables.iter().enumerate() {
        variables = variables.child(
            col()
                .gap(px(2.))
                .p(px(4.))
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(look.line))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.))
                        .child(name.clone()),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(link(
                            ui,
                            ("machine-variable-set", index),
                            "Set value…",
                            wrap(MachinePageAction::BeginVariable(Some(name.clone()))),
                            look,
                        ))
                        .child(link(
                            ui,
                            ("machine-variable-remove", index),
                            "Remove",
                            wrap(MachinePageAction::RemoveVariable(name.clone())),
                            look,
                        )),
                ),
        );
    }
    let mut variable_cell = col()
        .gap(px(6.))
        .child(variables)
        .child(dim(look, "Names only; values stay on the host."));
    if let Some(edit) = &state.variable {
        variable_cell = variable_cell.child(
            col()
                .id("machine-variable-form")
                .role(Role::Group)
                .aria_label("Set a variable")
                .gap(px(6.))
                .child(
                    Input::new(&inputs.name)
                        .disabled(edit.sending)
                        .aria_label("Variable name"),
                )
                .child(
                    Input::new(&inputs.value)
                        .disabled(edit.sending)
                        .aria_label("Variable value"),
                )
                .children(edit.error.clone().map(|error| {
                    div()
                        .id("machine-variable-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .text_color(rgb(RED))
                        .child(error)
                        .test_support()
                }))
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            widgets::primary(
                                ui,
                                "machine-variable-send",
                                if edit.sending { "Storing…" } else { "Set value" },
                                wrap(MachinePageAction::SendVariable),
                            )
                            .disabled(
                                edit.sending
                                    || edit.name.trim().is_empty()
                                    || edit.value.is_empty(),
                            ),
                        )
                        .child(
                            widgets::send(
                                ui,
                                "machine-variable-cancel",
                                "Cancel",
                                wrap(MachinePageAction::CancelVariable),
                            )
                            .disabled(edit.sending),
                        ),
                )
                .test_support(),
        );
    } else {
        variable_cell = variable_cell.child(link(
            ui,
            "machine-variable-new",
            "New variable…",
            wrap(MachinePageAction::BeginVariable(None)),
            look,
        ));
    }
    let mut health_card = card(look)
        .id("machine-health")
        .role(Role::Group)
        .aria_label("Health")
        .p(px(12.))
        .gap(px(8.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Health"))
        .child(kv("GitHub", github))
        .child(kv(
            "Agents",
            row().flex_wrap().gap(px(6.)).children(
                health
                    .agents
                    .iter()
                    .map(|(agent, version)| chip(look, tool_label(agent.command(), version))),
            ),
        ))
        .child(kv("Disk", disk))
        .child(kv("Variables", variable_cell));
    if let Some(problem) = &health.skills_problem {
        health_card = health_card.child(kv(
            "Skills",
            div().text_color(rgb(ORANGE)).child(problem.clone()),
        ));
    }

    let mut rules = card(look).overflow_hidden();
    for (index, rule) in detail.rules.iter().enumerate() {
        let (text, hex) = match rule.setting {
            RuleSetting::Allow => ("Allow", GREEN),
            RuleSetting::Ask => ("Ask me", ORANGE),
            RuleSetting::Never => ("Never", RED),
        };
        rules = rules.child(
            row()
                .id(("machine-rule", index))
                .role(Role::ListItem)
                .aria_label(format!("{}: {text}", rule.label))
                .gap(px(8.))
                .px(px(12.))
                .h(px(32.))
                .border_b_1()
                .border_color(rgb(look.line))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(rule.label.clone()),
                )
                .child(
                    div()
                        .w(px(46.))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(hex))
                        .child(text),
                )
                .test_support(),
        );
    }
    rules = rules.child(
        row().px(px(12.)).h(px(30.)).child(link(
            ui,
            "machine-change-access",
            "Change in Agent access",
            wrap(MachinePageAction::ChangeAgentAccess),
            look,
        )),
    );
    col()
        .w(px(400.))
        .flex_shrink_0()
        .gap(px(12.))
        .child(health_card.test_support())
        .child(
            col()
                .gap(px(6.))
                .child(label(look, "What its agents can reach through the host"))
                .child(rules),
        )
}

/// The skills preview as a small sheet over the page (SK-5, SK-11).
fn skills_sheet(
    ui: &Rc<Ui>,
    wrap: Wrap,
    dialog: &SkillsDialog,
    context: &PageContext,
    look: Look,
) -> Stateful<Div> {
    let group = |title: &'static str, names: &[String], hex: u32| {
        (!names.is_empty()).then(|| {
            col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(hex))
                        .child(format!("{title} · {}", names.len())),
                )
                .child(dim(look, names.join(", ")))
        })
    };
    let mut sheet = col()
        .id("machine-skills")
        .role(Role::Dialog)
        .aria_label("Sync skills")
        .w(px(520.))
        .max_w(relative(0.94))
        .max_h(relative(0.8))
        .gap(px(12.))
        .p(px(16.))
        .rounded(px(10.))
        .bg(rgb(look.card))
        .text_color(rgb(look.text))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .text_size(px(12.5))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Sync skills to {}", context.name)),
        );
    match &dialog.preview {
        None => {
            if dialog.error.is_none() {
                sheet = sheet.child(dim(look, "Checking what would change…"));
            }
        }
        Some(preview) => {
            sheet = sheet.child(preview_body(ui, wrap, preview, context, look, &group));
        }
    }
    if let Some(error) = &dialog.error {
        sheet = sheet.child(
            div()
                .id("machine-skills-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_color(rgb(RED))
                .child(error.clone())
                .test_support(),
        );
    }
    let blocked = dialog
        .preview
        .as_ref()
        .is_none_or(|preview| !preview.clashes.is_empty());
    sheet = sheet.child(
        row()
            .justify_end()
            .gap(px(8.))
            .child(
                widgets::send(
                    ui,
                    "machine-skills-cancel",
                    "Cancel",
                    wrap(MachinePageAction::CancelSync),
                )
                .disabled(dialog.syncing),
            )
            .child(
                widgets::primary(
                    ui,
                    "machine-skills-sync",
                    if dialog.syncing { "Syncing…" } else { "Sync" },
                    wrap(MachinePageAction::ConfirmSync),
                )
                .disabled(blocked || dialog.syncing),
            ),
    );
    let scrim = div()
        .id("machine-skills-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(if look.content == 0xffffff {
            rgba(0x0000_0030)
        } else {
            rgba(0x0000_0080)
        })
        .occlude();
    // The sheet sits over a scrim that covers the page.
    div()
        .id("machine-skills-layer")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(scrim)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .justify_center()
                .pt(px(96.))
                .child(div().occlude().child(sheet.test_support())),
        )
}

fn preview_body(
    ui: &Rc<Ui>,
    wrap: Wrap,
    preview: &SkillsPreview,
    context: &PageContext,
    look: Look,
    group: &dyn Fn(&'static str, &[String], u32) -> Option<Div>,
) -> Div {
    let mut body = col().gap(px(10.));
    if preview.first_sync {
        body = body.child(dim(
            look,
            format!("This is the first sync to {}.", context.name),
        ));
    }
    body = body
        .children(group("Added", &preview.added, GREEN))
        .children(group("Updated", &preview.updated, look.accent))
        .children(group("Removed", &preview.removed, RED));
    if preview.added.is_empty()
        && preview.updated.is_empty()
        && preview.removed.is_empty()
        && preview.clashes.is_empty()
    {
        body = body.child(dim(look, "Nothing would change."));
    }
    if !preview.clashes.is_empty() {
        body = body.child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(ORANGE))
                .child(format!(
                    "Clashes · {}: {} already has a skill of the same name. Pick which one its agents use.",
                    preview.clashes.len(),
                    context.name
                )),
        );
        for (index, clash) in preview.clashes.iter().enumerate() {
            body = body.child(
                row()
                    .id(("machine-clash", index))
                    .role(Role::Group)
                    .aria_label(clash.name.clone())
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(MONO)
                            .child(clash.name.clone()),
                    )
                    .child(widgets::send(
                        ui,
                        ("machine-clash-synced", index),
                        "Use the synced one",
                        wrap(MachinePageAction::ResolveClash(
                            clash.name.clone(),
                            ClashChoice::Synced,
                        )),
                    ))
                    .child(widgets::send(
                        ui,
                        ("machine-clash-local", index),
                        "Keep the one on the machine",
                        wrap(MachinePageAction::ResolveClash(
                            clash.name.clone(),
                            ClashChoice::MachineLocal,
                        )),
                    ))
                    .test_support(),
            );
        }
    }
    body
}
