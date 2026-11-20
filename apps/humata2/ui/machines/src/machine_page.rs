//! A machine's page (HO-3 to HO-5, HO-13, MP-12 to MP-17, SK-2, SK-5, SK-8;
//! frame 59): its system, sessions, Humata folders, health and what its
//! agents can reach, with Sync skills and host variables.
//!
//! The page draws a [`MachinePageState`] and sends [`MachinePageAction`]s;
//! each click is one action. An offline machine keeps showing what was last
//! loaded, with one line saying only the link is down (HO-5).

use crate::bridge::Ui;
use crate::chrome;
use humata2_ui_core::chrome::release_label;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::machines::{ClashChoice, GitHubState, MachineDetail, Reach, ScriptOutcome, SkillsPreview, tool_label};
use humata2_clients::model::{Channel, MachineId, MachineStop, RuleSetting, Session, Status};
use humata2_hosts::page::{
    MachinePageAction, MachinePageState, MenuPick, Refused, SessionsChange, SkillsDialog,
    Tab, Upgrade,
};
use std::cell::RefCell;
use std::rc::Rc;

pub mod select_mode;
use select_mode::{Snapshot, Surface, Tick};

const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
const MONO: &str = "Menlo";
/// A disk is low above this share used (HO-4).
const LOW_DISK_PERCENT: u64 = 85;
/// The Disk bar's fill scale, as the design draws it: the right column (400)
/// less the card's padding, the key and the gap after it.
const DISK_BAR: f32 = 400. - 24. - 84. - 8.;

/// How the page's actions become the app's.
pub type Wrap = fn(MachinePageAction) -> AppAction;

/// What the page needs from the workspace to name things.
pub struct PageContext {
    /// The machine's name.
    pub name: String,
    /// Whether its daemon answers now.
    pub online: bool,
    /// Sessions that may be listed; the page picks those the detail names.
    pub sessions: Vec<Session>,
    /// Channels, to name folders and sessions.
    pub channels: Vec<Channel>,
    /// Who stopped its sessions, as the banner says it after "by": "you",
    /// "a terminal on devbox-1". `None` while they run.
    pub stopped_by: Option<String>,
    /// The clock time the app's link to it was lost.
    pub lost_at: Option<String>,
}

/// The page's two inputs: the variable's name and its masked value.
pub struct MachinePageInputs {
    name: Entity<InputState>,
    name_echo: Rc<RefCell<Option<String>>>,
    value: Entity<InputState>,
    value_echo: Rc<RefCell<Option<String>>>,
    _subscriptions: Vec<Subscription>,
}

impl MachinePageInputs {
    /// The inputs; `wrap` carries a page action as the app's, such as
    /// `AppAction::MachinePage`.
    pub fn new(ui: &Rc<Ui>, wrap: Wrap, window: &mut Window, cx: &mut App) -> Self {
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

/// Whether the page shows the loading sweep in place of its detail: a load is
/// running and there is no detail to show yet.
fn sweep_without_detail(loading: bool, has_detail: bool) -> bool {
    loading && !has_detail
}

/// The loading sweep of the machine page: a full-width line, no words.
fn loading_sweep(look: Look) -> AnyElement {
    div()
        .id("machine-loading")
        .role(Role::ProgressIndicator)
        .aria_label("Loading")
        .test_support()
        .child(widgets::progress_bar(
            look,
            None,
            widgets::BarMetrics::line(look),
            "machine-sweep",
        ))
        .into_any_element()
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
    link_when(true, ui, id, text, action, look)
}

/// [`link`], or when `enabled` is false the same words dimmed in the
/// tertiary color that do nothing on a click (a read-only machine).
fn link_when(
    enabled: bool,
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    text: &str,
    action: AppAction,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    let link = div()
        .id(id)
        .role(Role::Button)
        .aria_label(text.to_owned())
        .flex_shrink_0()
        .whitespace_nowrap()
        .child(text.to_owned());
    if enabled {
        link.text_color(rgb(look.accent))
            .cursor_pointer()
            .on_click(move |_, _, cx| ui.send(action.clone(), cx))
            .test_support()
    } else {
        link.text_color(rgb(look.tertiary)).test_support()
    }
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn whole_gigabytes(bytes: u64) -> u64 {
    bytes / 1_000_000_000
}

/// The machine's stop that reached it: its sessions are stopped, not only
/// waiting to be.
fn delivered(state: &MachinePageState) -> Option<&MachineStop> {
    state.stop().filter(|stop| stop.delivered)
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
pub fn route(
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
        stopped_by: state
            .machine_page
            .stop()
            .map(|stop| stop.by.phrase(&state.workspace)),
        lost_at: found.and_then(|found| found.lost_at.clone()),
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
pub fn machine_page(
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
    // MASSDELETE (D875): the bottom bar while select mode is on this page.
    if let Some(machine) = state.machine.clone().map(Surface::Machine)
        && let Some(snap) = Snapshot::on(ui, &machine)
    {
        let visible = state
            .detail
            .iter()
            .flat_map(|detail| detail.sessions.iter().cloned())
            .collect();
        page = page.child(select_mode::bar(ui, snap, visible, context.name.clone(), look));
    }
    if let Some(dialog) = &state.skills {
        page = page.child(skills_sheet(ui, wrap, dialog, context, look));
    }
    // The Stop and Start confirms are drawn over the whole window by the overlays.
    page.test_support().into_any_element()
}

/// The machine's system line: its OS, architecture, address and how it is
/// reached, and its daemon's version. A part its machine didn't report is
/// left out, never drawn as an empty "· ·".
fn system_line(os: &str, arch: &str, address: &str, reach: &str, version: &str) -> String {
    let address = match (address.trim(), reach.trim()) {
        ("", "") => String::new(),
        ("", reach) => format!("({reach})"),
        (address, "") => address.to_owned(),
        (address, reach) => format!("{address} ({reach})"),
    };
    let version = match version.trim() {
        "" => String::new(),
        version => format!("humatad {version}"),
    };
    [os.trim(), arch.trim(), address.as_str(), version.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn header(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    context: &PageContext,
    look: Look,
) -> Div {
    // Read-only until migrated: both stay clickable so the page can say why
    // they don't work, but read as off (frames 79 and 80).
    let read_only = state.upgrade().read_only();
    let system = state.detail.as_ref().map(|detail| {
        system_line(
            &detail.os,
            &detail.arch,
            &detail.address,
            &reach_name(&detail.reach),
            &detail.daemon_version,
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
                .child(if delivered(state).is_some() {
                    chrome::stopped()
                } else {
                    chrome::status(if context.online {
                        Status::Working
                    } else {
                        Status::Offline
                    })
                })
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
                // While stopped, both stay clickable so the page can say why
                // they don't work, but read as off (frames 73 and 76).
                .children((delivered(state).is_some() || state.refused == Some(Refused::SessionsStopped)).then(|| {
                    div()
                        .id("machine-start-first")
                        .role(Role::Status)
                        .aria_label("Start sessions first")
                        .text_size(px(11.5))
                        .text_color(rgb(look.tertiary))
                        .child("Start sessions first")
                        .test_support()
                }))
                .child(
                    widgets::push_quiet(
                        ui,
                        "machine-open-shell",
                        "Open a shell",
                        wrap(MachinePageAction::OpenShell),
                        look,
                    )
                    .disabled(!context.online)
                    .when(state.stop().is_some() || read_only, |button| button.opacity(0.5)),
                )
                .child(
                    widgets::push_primary(
                        ui,
                        "machine-new-session",
                        format!("New session on {}", context.name),
                        wrap(MachinePageAction::NewSession),
                        look,
                    )
                    .disabled(!context.online)
                    .when(state.stop().is_some() || read_only, |button| button.opacity(0.5)),
                )
                // The frame's ⋯ menu sits last and holds Sync skills and Reconnect now.
                .child(
                    div()
                        .relative()
                        // A bare 15 pt glyph after the actions, as in frame 59.
                        .child(chrome::action(
                            ui,
                            "machine-menu",
                            "More",
                            IconName::Ellipsis,
                            wrap(MachinePageAction::OpenMenu),
                            look.secondary,
                        ))
                        .when(state.menu_open, |menu| {
                            menu.child(crate::action_menu::scrim(
                                ui,
                                "machine-menu-scrim",
                                wrap(MachinePageAction::CloseMenu),
                            ))
                            .child(
                                crate::action_menu::card("machine-menu-card", 180., look)
                                    .top(px(26.))
                                    .right(px(0.))
                                    .child(
                                        widgets::ghost_in(
                                            look,
                                            ui,
                                            "machine-sync-skills",
                                            "Sync skills",
                                            wrap(MachinePageAction::PickMenu(
                                                humata2_hosts::page::MenuPick::SyncSkills,
                                            )),
                                        )
                                        .w_full(),
                                    )
                                    // While the link-down row shows, Reconnect now lives here (HO-5).
                                    // Offered while the sessions run (D260).
                                    .children(
                                        (state.detail.is_some() && state.stop().is_none()).then(|| {
                                            widgets::ghost_in(
                                                look,
                                                ui,
                                                "machine-stop-sessions",
                                                "Stop all sessions\u{2026}",
                                                wrap(MachinePageAction::PickMenu(
                                                    MenuPick::StopSessions,
                                                )),
                                            )
                                            .w_full()
                                            .disabled(state.changing.is_some())
                                        }),
                                    )
                                    .children((context.online || state.detail.is_none()).then(
                                        || {
                                            widgets::ghost_in(
                                                look,
                                                ui,
                                                "machine-reconnect-menu",
                                                "Reconnect now",
                                                wrap(MachinePageAction::PickMenu(
                                                    humata2_hosts::page::MenuPick::ReconnectNow,
                                                )),
                                            )
                                            .w_full()
                                        },
                                    )),
                            )
                        }),
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
    let upgrade = state.upgrade();
    page = page
        .children(upgrade_banner(ui, wrap, &upgrade, look))
        .children(holders_moving_row(ui, wrap, state, look))
        .children(migration_refusal(ui, wrap, state, context, look));
    if let Some(stop) = state.stop() {
        page = page.child(stop_banner(ui, wrap, state, stop, context, look));
    } else if let Some(change) = state.changing {
        page = page.child(
            div()
                .id("machine-changing")
                .role(Role::Status)
                .aria_label(match change {
                    SessionsChange::Stop => "Stopping sessions",
                    SessionsChange::Start => "Starting sessions",
                    SessionsChange::CancelStop => "Cancelling the stop",
                })
                .text_color(rgb(look.secondary))
                .child(match change {
                    SessionsChange::Stop => format!("Stopping the sessions on {}\u{2026}", context.name),
                    SessionsChange::Start => format!("Starting the sessions on {}\u{2026}", context.name),
                    SessionsChange::CancelStop => "Cancelling the stop\u{2026}".to_owned(),
                })
                .test_support(),
        );
    }
    // A stop still on its way says the machine can't be reached, with its own
    // Reconnect now, so the link-down row would say it twice (frame 76).
    if !context.online
        && state.detail.is_some()
        && !state.stop().is_some_and(|stop| !stop.delivered)
    {
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
    if let Some(error) = state.shown_load_error() {
        page = page.child(
            row()
                .id("machine-load-error")
                .role(Role::Alert)
                .aria_label(error.to_owned())
                .gap(px(10.))
                .text_color(rgb(RED))
                .child(div().child(error.to_owned()))
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
    if sweep_without_detail(state.loading, state.detail.is_some()) {
        page = page.child(loading_sweep(look));
    }
    let Some(detail) = &state.detail else {
        return page;
    };
    page.child(
        row()
            .items_start()
            .gap(px(16.))
            .child(left(ui, wrap, state, detail, context, look))
            .child(right(ui, wrap, state, detail, &upgrade, inputs, look)),
    )
}

fn left(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
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
    let stopped = delivered(state).is_some();
    let mut sessions = card(look).overflow_hidden();
    let listed: Vec<&Session> = detail
        .sessions
        .iter()
        .filter_map(|id| context.sessions.iter().find(|session| &session.id == id))
        .collect();
    // MASSDELETE (D875): select mode on this machine's page.
    let surface = state.machine.clone().map(Surface::Machine);
    let snap = surface.as_ref().and_then(|surface| Snapshot::on(ui, surface));
    let visible = Rc::new(listed.iter().map(|session| session.id.clone()).collect::<Vec<_>>());
    for (index, session) in listed.iter().copied().enumerate() {
        let click = surface.clone().map(|surface| {
            select_mode::row_click(
                ui,
                surface,
                visible.clone(),
                session.id.clone(),
                snap.is_some(),
                wrap(MachinePageAction::OpenSession(session.id.clone())),
            )
        });
        let ui = ui.clone();
        let action = wrap(MachinePageAction::OpenSession(session.id.clone()));
        let halted = stopped || session.status.is_stopped();
        let ticked = snap.as_ref().is_some_and(|snap| snap.ticked(&session.id));
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
                .when(ticked, |line| line.bg(select_mode::tint(look)))
                .children(snap.as_ref().map(|snap| {
                    select_mode::tick_box(look, 14., snap.tick(&session.id))
                }))
                .child(if halted {
                    chrome::stopped()
                } else {
                    chrome::status(session.status)
                })
                .child(
                    div()
                        .w(px(190.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .font_weight(FontWeight::MEDIUM)
                        .when(halted, |name| name.text_color(rgb(look.secondary)))
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
                .children(halted.then(|| {
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_color(rgb(look.tertiary))
                        .child(Status::Stopped.label())
                }))
                .on_click(move |event, window, cx| match &click {
                    Some(click) => click(event, window, cx),
                    None => ui.send(action.clone(), cx),
                })
                .test_support(),
        );
    }
    // D784: once the machine's running holders are listed they are the
    // "Sessions on it" list, and the ones no channel has follow them.
    let (running_count, running_card, running_orphans) = if state.running.holders.is_empty() {
        (None, None, None)
    } else {
        let (count, listed, orphans) = crate::machine_running::listing(
            ui,
            wrap,
            state,
            &context.sessions,
            &channel_name,
            look,
        );
        (Some(count), Some(listed), orphans)
    };
    let mut folders = card(look).overflow_hidden();
    for (index, folder) in detail.folders.iter().enumerate() {
        let sessions_text = match folder.sessions {
            0 => "no sessions".to_owned(),
            1 => "1 session".to_owned(),
            n => format!("{n} sessions"),
        };
        let channel = channel_name(&folder.channel);
        folders = folders.child(
            row()
                .id(("machine-folder", index))
                .role(Role::ListItem)
                .aria_label(channel.clone())
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
                        .child(channel),
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
    let heading = label(
        look,
        {
            let count = running_count.unwrap_or(detail.sessions.len());
            match (&context.lost_at, state.stop().is_some_and(|stop| !stop.delivered)) {
                (Some(at), true) => format!("Sessions on it · {count} · as last seen at {at}"),
                _ => format!("Sessions on it · {count}"),
            }
        },
    );
    // The heading offers Select; in select mode it is the select-all box.
    let heading = match (&snap, running_card.is_none() && !listed.is_empty()) {
        (Some(snap), _) => select_all_heading(ui, snap, &visible, &context.name, look)
            .into_any_element(),
        (None, true) => {
            let enter = link(
                ui,
                "machine-select",
                "Select",
                select_mode::act(select_mode::MassDeleteAction::EnterEmpty(
                    surface.clone().unwrap_or(Surface::Home),
                )),
                look,
            )
            .text_size(px(11.));
            row()
                .items_center()
                .child(div().flex_1().child(heading))
                .when(surface.is_some(), |line| line.child(enter))
                .into_any_element()
        }
        (None, false) => heading.into_any_element(),
    };
    col()
        .flex_1()
        .min_w_0()
        .gap(px(16.))
        .child(
            col()
                .gap(px(6.))
                .child(heading)
                .child(match running_card {
                    Some(listed) => listed.id("machine-running").test_support().into_any_element(),
                    None => sessions.into_any_element(),
                }),
        )
        .children(running_orphans)
        .child(
            col()
                .gap(px(6.))
                .child(label(look, "Humata folders by channel"))
                .child(folders),
        )
        // MACHINESCRIPTS (D686): drawn only when the machine has rules or a problem to say.
        .children(scripts_section(ui, wrap, state, context, look))
}

/// The sessions list's heading in select mode: the box that ticks every
/// session on the machine, "Select all on {machine}", "{k} of {n}" and Done.
fn select_all_heading(
    ui: &Rc<Ui>,
    snap: &Snapshot,
    visible: &Rc<Vec<humata2_clients::model::SessionId>>,
    machine: &str,
    look: Look,
) -> impl IntoElement {
    let ticked = visible.iter().filter(|id| snap.ticked(id)).count();
    let all = !visible.is_empty() && ticked == visible.len();
    let toggle = if all {
        select_mode::MassDeleteAction::SelectNothing
    } else {
        select_mode::MassDeleteAction::SelectAllVisible(visible.to_vec())
    };
    let (ui_box, ui_done) = (ui.clone(), ui.clone());
    let tick = if all {
        Tick::On
    } else if ticked > 0 {
        Tick::Dash
    } else {
        Tick::Off
    };
    row()
        .id("machine-select-all")
        .role(Role::CheckBox)
        .aria_label(format!("Select all on {machine}"))
        .aria_selected(all)
        .gap(px(8.))
        .child(
            row()
                .id("machine-select-all-box")
                .gap(px(8.))
                .cursor_pointer()
                .child(select_mode::tick_box(look, 14., tick))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Select all on {machine}")),
                )
                .child(dim(look, format!("{ticked} of {}", visible.len())))
                .on_click(move |_, _, cx| {
                    ui_box.send(select_mode::act(toggle.clone()), cx);
                }),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("machine-select-done")
                .role(Role::Button)
                .aria_label("Done")
                .text_size(px(12.))
                .text_color(rgb(look.accent))
                .cursor_pointer()
                .child("Done")
                .on_click(move |_, _, cx| {
                    ui_done.send(select_mode::act(select_mode::MassDeleteAction::Exit), cx);
                })
                .test_support(),
        )
        .test_support()
}

/// A run's colour: green passed, red failed, orange going.
fn script_outcome_color(look: Look, outcome: ScriptOutcome) -> u32 {
    match outcome {
        ScriptOutcome::Passed => GREEN,
        ScriptOutcome::Failed | ScriptOutcome::TimedOut => RED,
        ScriptOutcome::Running => ORANGE,
        ScriptOutcome::Skipped => look.secondary,
    }
}

/// Only a running machine script step shows the sweep (PROGRESS T9); a
/// finished or failed step never carries progress.
fn script_step_sweeps(outcome: ScriptOutcome) -> bool {
    outcome == ScriptOutcome::Running
}

/// The last lines of a step's output, in the page's mono face.
fn script_tail(look: Look, tail: &[String], color: u32) -> Div {
    col()
        .gap(px(1.))
        .px(px(12.))
        .pb(px(8.))
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(rgb(color))
        .children(tail.iter().map(|line| div().whitespace_nowrap().overflow_hidden().child(line.clone())))
        .when(tail.is_empty(), |x| x.child(dim(look, "No output.")))
}

/// Scripts (MACHINESCRIPTS, D686): the owner's rules that apply here, each
/// with its last run and Run now. A failed run shows the end of the failed
/// step's output; a click on a rule opens every step. Nothing is drawn for a
/// machine with no rules, so a quiet page stays quiet.
fn scripts_section(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    context: &PageContext,
    look: Look,
) -> Option<Div> {
    let scripts = &state.scripts;
    if !scripts.visible() {
        return None;
    }
    let mut list = card(look).overflow_hidden();
    for (index, line) in scripts.rows().into_iter().enumerate() {
        let name = line.rule.name.clone();
        let outcome_text = if line.running {
            "Running\u{2026}".to_owned()
        } else {
            line.last.map_or_else(
                || "Never run".to_owned(),
                |run| format!("{} \u{b7} {}", run.outcome.label(), run.at),
            )
        };
        let color = if line.running {
            ORANGE
        } else {
            line.last.map_or(look.secondary, |run| script_outcome_color(look, run.outcome))
        };
        let ui_toggle = ui.clone();
        let toggle = wrap(MachinePageAction::ToggleScript(name.clone()));
        let header = row()
            .id(("machine-script", index))
            .role(Role::Button)
            .aria_label(name.clone())
            .gap(px(10.))
            .px(px(12.))
            .h(px(36.))
            .cursor_pointer()
            .child(
                div()
                    .w(px(190.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .font_weight(FontWeight::MEDIUM)
                    .child(name.clone()),
            )
            .child(div().flex_1().whitespace_nowrap().overflow_hidden().child(dim(
                look,
                line.rule.triggers.join(" \u{b7} "),
            )))
            .child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_color(rgb(color))
                    .child(outcome_text),
            )
            .child(
                widgets::push_quiet(
                    ui,
                    ("machine-script-run", index),
                    if line.running { "Running\u{2026}" } else { "Run now" },
                    wrap(MachinePageAction::RunScript(name.clone())),
                    look,
                )
                .disabled(line.running || !context.online),
            )
            .on_click(move |_, _, cx| ui_toggle.send(toggle.clone(), cx))
            .test_support();
        let mut block = col().border_b_1().border_color(rgb(look.line)).child(header);
        // PROGRESS T9: a running rule has no honest total, so it sweeps.
        if line.running {
            block = block.child(
                div()
                    .id(("machine-script-progress", index))
                    .role(Role::ProgressIndicator)
                    .aria_label("Running")
                    .test_support()
                    .px(px(12.))
                    .pb(px(8.))
                    .child(widgets::progress_bar(
                        look,
                        None,
                        widgets::BarMetrics::line(look),
                        SharedString::from(format!("machine-script-sweep-{name}")),
                    )),
            );
        }
        if let Some(run) = line.last.filter(|run| run.outcome.is_problem())
            && let Some((step, tail)) = run.failure_tail()
        {
            let exit = step.exit.map_or_else(String::new, |exit| format!(" (exit {exit})"));
            block = block
                .child(
                    div()
                        .px(px(12.))
                        .pb(px(4.))
                        .text_color(rgb(RED))
                        .child(format!("Step {}: {}{exit}", step.index + 1, step.label)),
                )
                .child(script_tail(look, tail, RED));
        }
        if let Some(reason) = line.refused {
            block = block.child(
                div()
                    .id(("machine-script-refused", index))
                    .role(Role::Alert)
                    .aria_label(reason.to_owned())
                    .px(px(12.))
                    .pb(px(8.))
                    .text_color(rgb(RED))
                    .child(reason.to_owned())
                    .test_support(),
            );
        }
        if line.open {
            for step in &line.rule.steps {
                block = block.child(
                    row()
                        .gap(px(8.))
                        .px(px(12.))
                        .pb(px(4.))
                        .child(chip(look, step.kind.clone()))
                        .child(dim(look, step.label.clone())),
                );
            }
            if let Some(run) = line.last {
                for step in &run.steps {
                    block = block
                        .child(
                            div()
                                .px(px(12.))
                                .pb(px(2.))
                                .text_color(rgb(script_outcome_color(look, step.outcome)))
                                .child(format!("Step {}: {} \u{b7} {}", step.index + 1, step.label, step.outcome.label())),
                        )
                        .when(script_step_sweeps(step.outcome), |x| {
                            x.child(
                                div()
                                    .id(SharedString::from(format!(
                                        "machine-script-step-progress-{index}-{}",
                                        step.index
                                    )))
                                    .role(Role::ProgressIndicator)
                                    .aria_label("Running")
                                    .test_support()
                                    .px(px(12.))
                                    .pb(px(4.))
                                    .child(widgets::progress_bar(
                                        look,
                                        None,
                                        widgets::BarMetrics::line(look),
                                        SharedString::from(format!(
                                            "machine-script-step-sweep-{name}-{}",
                                            step.index
                                        )),
                                    )),
                            )
                        })
                        .when(!step.tail.is_empty(), |x| x.child(script_tail(look, &step.tail, look.secondary)));
                }
            }
        }
        list = list.child(block);
    }
    let mut section = col().gap(px(6.)).child(label(look, "Scripts"));
    if let Some(problem) = scripts.problem() {
        section = section.child(
            div()
                .id("machine-scripts-problem")
                .role(Role::Alert)
                .aria_label(problem.to_owned())
                .text_color(rgb(RED))
                .child(problem.to_owned())
                .test_support(),
        );
    }
    if let Some(error) = state.shown_scripts_error() {
        section = section.child(
            div()
                .id("machine-scripts-error")
                .role(Role::Alert)
                .aria_label(error.to_owned())
                .text_color(rgb(RED))
                .child(error.to_owned())
                .test_support(),
        );
    }
    Some(section.child(list))
}

fn right(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    detail: &MachineDetail,
    upgrade: &Upgrade,
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
    // Not found and signed out each say their own fix (owner, 0.2.1).
    let github_problem = |what: &'static str, fix: &'static str| {
        col()
            .gap(px(2.))
            .child(div().text_color(rgb(ORANGE)).child(what))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(fix),
            )
    };
    let github = match health.github() {
        GitHubState::SignedIn(login) => row()
            .flex_wrap()
            .gap(px(6.))
            .child(chrome::icon(IconName::Check, GREEN, 12.))
            .child(format!("signed in as {}", login.user))
            .children(login.scopes.iter().map(|scope| chip(look, scope.clone()))),
        GitHubState::SignedOut => github_problem(
            "not signed in",
            "Run `gh auth login` in a terminal on this machine, then reopen this page.",
        ),
        GitHubState::NotFound => github_problem(
            "GitHub CLI not found",
            "Install gh on this machine (on a Mac: brew install gh), then reopen this page. If gh already runs in your terminal, put its folder on PATH in your login profile (~/.zprofile): Humata's daemon reads your login shell's PATH when it starts.",
        ),
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
                        .w(px(DISK_BAR * used_percent.min(100) as f32 / 100.))
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
    // A read-only machine takes no variable changes: its links are dimmed.
    let editable = !upgrade.read_only();
    let mut variables = row().flex_wrap().gap(px(6.));
    for (index, name) in health.variables.iter().enumerate() {
        variables = variables.child(
            col()
                .gap(px(2.))
                .px(px(4.))
                .py(px(3.))
                .rounded(px(5.))
                .border_1()
                .border_color(rgb(look.line))
                .child(
                    div()
                        .whitespace_nowrap()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .child(name.clone()),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(link_when(
                            editable,
                            ui,
                            ("machine-variable-set", index),
                            "Set value…",
                            wrap(MachinePageAction::BeginVariable(Some(name.clone()))),
                            look,
                        ))
                        .child(link_when(
                            editable,
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
        .gap(px(8.))
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
                            widgets::primary_in(look,
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
        variable_cell = variable_cell.child(link_when(
            editable,
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
        .child(kv("Variables", variable_cell))
        .child(kv("Humata", humata_cell(detail, upgrade, look)));
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
                widgets::primary_in(look,
                    ui,
                    "machine-skills-sync",
                    if dialog.syncing { "Syncing…" } else { "Sync" },
                    wrap(MachinePageAction::ConfirmSync),
                )
                .disabled(blocked || dialog.syncing),
            ),
    );
    // The sheet sits over a scrim that covers the page.
    layer("machine-skills", 96., look, sheet)
}

/// `sheet` over a scrim that covers the page, `top` from the page's top.
fn layer(
    id: &'static str,
    top: f32,
    look: Look,
    sheet: Stateful<Div>,
) -> Stateful<Div> {
    let scrim = div()
        .id(SharedString::from(format!("{id}-scrim")))
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
    div()
        .id(SharedString::from(format!("{id}-layer")))
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
                .pt(px(top))
                .child(div().occlude().child(sheet.test_support())),
        )
}

/// A dialog's card, `width` wide.
fn dialog_card(id: &'static str, label: &'static str, width: f32, look: Look) -> Stateful<Div> {
    col()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .w(px(width))
        .max_w_full()
        .gap(px(8.))
        .p(px(20.))
        .rounded(px(12.))
        .bg(rgb(look.card))
        .text_color(rgb(look.text))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .text_size(px(12.5))
}

fn dialog_title(text: String) -> Div {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

/// The Stop confirm's card (frame 72): what ends, what is kept; `cancel`
/// and `go` are what its two buttons send.
pub fn stop_confirm(
    ui: &Rc<Ui>,
    name: &str,
    sessions: usize,
    cancel: AppAction,
    go: AppAction,
    look: Look,
) -> Stateful<Div> {
    let part = |head: &'static str, text: &'static str| {
        col()
            .gap(px(2.))
            .pt(px(4.))
            .child(label(look, head))
            .child(div().child(text))
    };
    dialog_card("machine-stop-dialog", "Stop all sessions", 520., look)
        .child(dialog_title(format!("Stop all sessions on {name}?")))
        .child(
            div()
                .text_color(rgb(look.secondary))
                .child(format!(
                    "Every agent and shell in {name}'s {sessions} sessions gets 2 seconds to quit, then is killed."
                )),
        )
        .child(part(
            "Kept",
            "Scrollback, conversations, todos, rewind points and open tabs.",
        ))
        .child(part(
            "While stopped",
            "Agents can't type, call Humata or be answered. Their open requests end.",
        ))
        .child(
            div()
                .mt(px(4.))
                .p(px(10.))
                .rounded(px(7.))
                .bg(rgb(look.fill))
                .text_color(rgb(look.secondary))
                .child(format!(
                    "{name}'s sessions stay stopped, even after it restarts, until you choose Start sessions."
                )),
        )
        .child(
            row()
                .justify_end()
                .gap(px(8.))
                .pt(px(8.))
                .child(widgets::push_quiet(
                    ui,
                    "machine-stop-cancel",
                    "Cancel",
                    cancel,
                    look,
                ))
                .child(
                    widgets::push_quiet(
                        ui,
                        "machine-stop-confirm",
                        "Stop all sessions",
                        go,
                        look,
                    )
                    .border_color(rgb(RED))
                    .text_color(rgb(RED)),
                ),
        )
}

/// The Start confirm (frame 75), for the machine's page and the Hosts page:
/// `count` sessions, stopped at `at`; `cancel` and `go` are what its two
/// buttons send.
pub fn start_confirm(
    ui: &Rc<Ui>,
    name: &str,
    count: usize,
    at: Option<&str>,
    cancel: AppAction,
    go: AppAction,
    look: Look,
) -> Stateful<Div> {
    let when = at.map_or_else(String::new, |at| format!(" stopped at {at}"));
    dialog_card("machine-start-dialog", "Start sessions", 440., look)
        .child(dialog_title(format!("Start sessions on {name}?")))
        .child(div().text_color(rgb(look.secondary)).child(format!(
            "Humata brings back the {count} sessions{when} as after a restart: agents resume their conversations, idle, and shells open fresh in the same folders. A report shows what came back."
        )))
        .child(
            row()
                .justify_end()
                .gap(px(8.))
                .pt(px(8.))
                .child(widgets::push_quiet(
                    ui,
                    "machine-start-cancel",
                    "Cancel",
                    cancel,
                    look,
                ))
                .child(
                    widgets::push_primary(ui, "machine-start-confirm", "Start sessions", go, look)
                        // The banner's button reads "Start sessions" too; the
                        // confirm's name says how many it brings back.
                        .accessibility_label(start_name(count)),
                ),
        )
}

/// The start confirm's accessible name: "Start 3 sessions", "Start 1 session".
fn start_name(count: usize) -> String {
    if count == 1 {
        "Start 1 session".to_owned()
    } else {
        format!("Start {count} sessions")
    }
}

/// An "Open in Inbox" text link to the machine's Migrate item.
fn open_in_inbox(ui: &Rc<Ui>, wrap: Wrap, look: Look) -> ObservedElement<Stateful<Div>> {
    link(
        ui,
        "machine-open-in-inbox",
        "Open in Inbox",
        wrap(MachinePageAction::OpenInInbox),
        look,
    )
}

/// The banner while a Migrate item holds the page read-only, is migrating or
/// failed (frames 79 and 80). Migrated and current pages have none.
fn upgrade_banner(
    ui: &Rc<Ui>,
    wrap: Wrap,
    upgrade: &Upgrade,
    look: Look,
) -> Option<ObservedElement<Stateful<Div>>> {
    let (glyph, hex, bg, label, text) = match upgrade {
        Upgrade::Current | Upgrade::Migrated { .. } => return None,
        Upgrade::ReadOnly { to, .. } => (
            IconName::Info,
            look.secondary,
            look.fill,
            "Read-only until migrated",
            format!(
                "Read-only until migrated to {}: new sessions, typing and moves wait. Sessions keep running.",
                release_label(to)
            ),
        ),
        Upgrade::Migrating { to, .. } => (
            IconName::Info,
            ORANGE,
            look.attention_bg,
            "Migrating",
            format!(
                "Migrating to {}… Sessions keep running; the link drops for a few seconds.",
                release_label(to)
            ),
        ),
        Upgrade::Failed { to, failure, .. } => (
            IconName::CircleX,
            RED,
            look.attention_bg,
            "Couldn't migrate",
            format!(
                "Read-only until migrated to {}: new sessions, typing and moves wait. The last try failed: {} {}",
                release_label(to),
                failure.message,
                failure.fix
            ),
        ),
    };
    Some(
        row()
            .id("machine-upgrade-banner")
            .role(Role::Status)
            .aria_label(label)
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .bg(rgb(bg))
            .child(div().flex_shrink_0().child(chrome::icon(glyph, hex, 12.)))
            .child(div().flex_1().min_w_0().child(text))
            .child(open_in_inbox(ui, wrap, look))
            .test_support(),
    )
}

/// While a Holders Migrate moves this machine's terminals: how many moved so
/// far (frame 125), under the banner, with the way to the item.
fn holders_moving_row(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    look: Look,
) -> Option<ObservedElement<Stateful<Div>>> {
    let moved = state.holders_moving()?;
    Some(
        row()
            .id("machine-holders-moving")
            .role(Role::Status)
            .aria_label("Moving terminals")
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .bg(rgb(look.fill))
            .child(div().flex_1().min_w_0().child(widgets::progress_line(
                look,
                &moved.progress(),
                Some(ui.now()),
            )))
            .child(open_in_inbox(ui, wrap, look))
            .test_support(),
    )
}

/// What a click refused for a migration says, with the way to the item when
/// one is known (NV-10): inline, never a toast.
fn migration_refusal(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    context: &PageContext,
    look: Look,
) -> Option<ObservedElement<Stateful<Div>>> {
    let Some(Refused::MigrationNeeded(request)) = &state.refused else {
        return None;
    };
    let text = format!(
        "Migrate the daemon on {} first. Nothing was started or sent; its sessions keep running.",
        context.name
    );
    Some(
        row()
            .id("machine-migration-refusal")
            .role(Role::Alert)
            .aria_label(text.clone())
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .bg(rgb(look.attention_bg))
            .child(div().flex_shrink_0().child(chrome::icon(IconName::Inbox, ORANGE, 12.)))
            .child(div().flex_1().min_w_0().child(text))
            .children(request.is_some().then(|| open_in_inbox(ui, wrap, look)))
            .test_support(),
    )
}

/// The Health "Humata" row: the daemon's release, and where its migration is.
fn humata_cell(detail: &MachineDetail, upgrade: &Upgrade, look: Look) -> Div {
    let version = detail.daemon_version.clone();
    match upgrade {
        Upgrade::Current => div().child(version),
        Upgrade::ReadOnly { to, .. } => div()
            .text_color(rgb(ORANGE))
            .child(format!("{version} · migrate to {} waits in the Inbox", release_label(to))),
        Upgrade::Migrating { to, .. } => div()
            .text_color(rgb(ORANGE))
            .child(format!("{version} · migrating to {}…", release_label(to))),
        Upgrade::Failed { to, .. } => div()
            .text_color(rgb(RED))
            .child(format!("{version} · couldn't migrate to {}", release_label(to))),
        Upgrade::Migrated { to, at } => col()
            .child(
                row()
                    .gap(px(6.))
                    .child(chrome::icon(IconName::Check, GREEN, 12.))
                    .child(format!("Migrated to {} · {at}", release_label(to))),
            )
            .child(dim(look, version)),
    }
}

/// The banner at the top of the page while the sessions are stopped
/// (frame 73), or the stop is still on its way (frame 76).
fn stop_banner(
    ui: &Rc<Ui>,
    wrap: Wrap,
    state: &MachinePageState,
    stop: &MachineStop,
    context: &PageContext,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let who = context.stopped_by.clone().unwrap_or_default();
    let (glyph, hex, bg, text) = if stop.delivered {
        (
            IconName::Info,
            look.secondary,
            look.fill,
            format!(
                "Sessions stopped by {who} at {}: agents and shells ended, everything else is kept, and they stay stopped after restarts.",
                stop.at
            ),
        )
    } else {
        (
            IconName::WifiOff,
            ORANGE,
            look.attention_bg,
            format!(
                "Stop not delivered yet: {} can't be reached, so its sessions may still be running. Humata stops them as soon as it connects again.",
                context.name
            ),
        )
    };
    let busy = state.changing.is_some();
    let banner = row()
        .id("machine-stop-banner")
        .role(Role::Status)
        .aria_label(if stop.delivered {
            "Sessions stopped"
        } else {
            "Stop not delivered"
        })
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(rgb(bg))
        .child(div().flex_shrink_0().child(chrome::icon(glyph, hex, 12.)))
        .child(div().flex_1().min_w_0().child(text));
    let banner = if stop.delivered {
        banner.child(
            widgets::push_primary(
                ui,
                "machine-start-sessions",
                if state.changing == Some(SessionsChange::Start) {
                    "Starting\u{2026}"
                } else {
                    "Start sessions"
                },
                wrap(MachinePageAction::StartSessions),
                look,
            )
            .disabled(busy),
        )
    } else {
        banner
            .child(widgets::push_quiet(
                ui,
                "machine-reconnect",
                "Reconnect now",
                wrap(MachinePageAction::ReconnectNow),
                look,
            ))
            .child(
                widgets::push_quiet(
                    ui,
                    "machine-cancel-stop",
                    if state.changing == Some(SessionsChange::CancelStop) {
                        "Cancelling\u{2026}"
                    } else {
                        "Cancel stop"
                    },
                    wrap(MachinePageAction::CancelStop),
                    look,
                )
                .disabled(busy),
            )
    };
    banner.test_support()
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

#[cfg(test)]
mod system_line_tests {
    use ::core::prelude::v1::test;

    use super::system_line;

    #[test]
    fn a_part_the_machine_did_not_report_is_left_out() {
        assert_eq!(
            system_line("macOS 26.1", "arm64", "", "", "0.2.5"),
            "macOS 26.1 · arm64 · humatad 0.2.5"
        );
        assert_eq!(
            system_line("macOS 26.1", "arm64", "10.0.0.2", "ssh", "0.2.5"),
            "macOS 26.1 · arm64 · 10.0.0.2 (ssh) · humatad 0.2.5"
        );
        assert!(!system_line("", "", "", "", "").contains('·'));
    }
}

#[cfg(test)]
mod loading_sweep_tests {
    use ::core::prelude::v1::test;

    use super::sweep_without_detail;

    #[test]
    fn a_first_load_shows_the_sweep() {
        assert!(sweep_without_detail(true, false));
    }

    #[test]
    fn a_page_that_is_not_loading_shows_no_sweep() {
        assert!(!sweep_without_detail(false, false));
    }

    #[test]
    fn a_page_with_detail_does_not_show_the_sweep_in_place_of_it() {
        assert!(!sweep_without_detail(true, true));
    }
}

#[cfg(test)]
mod script_sweep_tests {
    use ::core::prelude::v1::test;

    use super::script_step_sweeps;
    use humata2_clients::machines::ScriptOutcome;

    #[test]
    fn only_a_running_script_step_sweeps() {
        assert!(script_step_sweeps(ScriptOutcome::Running));
        assert!(!script_step_sweeps(ScriptOutcome::Failed));
        assert!(!script_step_sweeps(ScriptOutcome::TimedOut));
        assert!(!script_step_sweeps(ScriptOutcome::Passed));
        assert!(!script_step_sweeps(ScriptOutcome::Skipped));
    }
}
