//! Small native window controls and shared panel/label primitives.
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Agent, Release, RepoDiff, SessionId, Status};
use humata2_session::{Panel, SessionAction};
use std::collections::HashSet;
use std::rc::Rc;

pub const GREEN: u32 = 0x30d158;
pub const ORANGE: u32 = 0xff9f0a;
pub const RED: u32 = 0xff453a;

pub fn icon(name: IconName, color: u32, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(rgb(color))
}
pub fn action(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: IconName,
    event: AppAction,
    color: u32,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(15.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(icon(glyph, color, 15.))
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
        .test_support()
}
pub fn status(value: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match value {
        Status::Working => dot.bg(rgb(crate::theme::GREEN)),
        Status::NeedsYou => dot.bg(rgb(crate::theme::ORANGE)),
        // Moving keeps the plain hollow dot (frame 64).
        Status::Idle | Status::ProbablyIdle | Status::Moving => {
            dot.border_1().border_color(rgb(crate::theme::GREY))
        }
        Status::Offline => dot.bg(rgb(crate::theme::GREY)),
        Status::Stopped | Status::AgentStopped => return stopped(),
        Status::Exited { .. } => {
            return div()
                .size(px(10.))
                .flex_shrink_0()
                .child(icon(IconName::X, crate::theme::RED_STATUS, 10.));
        }
    };
    div()
        .size(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}
/// The mark of a session or machine whose sessions were stopped: a small
/// grey square, unlike the round state dots (frames 73 to 75).
pub fn stopped() -> Div {
    div()
        .size(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .child(
            div()
                .size(px(7.))
                .rounded(px(1.5))
                .bg(rgb(crate::theme::GREY)),
        )
}
pub fn lights() -> Div {
    div().flex().gap(px(8.)).flex_shrink_0().children(
        [
            ("window-close", "Close window", 0xff5f57),
            ("window-minimize", "Minimize window", 0xfebc2e),
            ("window-zoom", "Zoom window", 0x28c840),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (id, label, color))| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .size(px(12.))
                .rounded_full()
                .bg(rgb(color))
                .on_click(move |_, window, _| match index {
                    0 => window.remove_window(),
                    1 => window.minimize_window(),
                    _ => window.zoom_window(),
                })
                .test_support()
        }),
    )
}
pub fn section(look: Look, title: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .px(px(18.))
        .pt(px(12.))
        .pb(px(3.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(div().flex_1().child(title))
}

/// What a Desk session runs, as the frames and dialogs name it.
pub fn runs(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude Code",
        Agent::Shell => "Terminal",
        other => other.name(),
    }
}

/// A release as people read it: "2.0.5 (a1b2c3d)", the bundle shortened.
pub fn release_label(release: &Release) -> String {
    let short: String = release.bundle.chars().take(7).collect();
    match (release.version.is_empty(), short.is_empty()) {
        (false, false) => format!("{} ({short})", release.version),
        (false, true) => release.version.clone(),
        (true, false) => short,
        (true, true) => "its release".to_owned(),
    }
}

/// The program as people know it.
pub const fn agent_label(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude Code",
        other => other.name(),
    }
}

pub fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

pub fn secondary(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}

pub fn panel_action(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    event: AppAction,
    look: Look,
    primary: bool,
) -> Button {
    let ui = ui.clone();
    let label = label.into();
    Button::new(id)
        .label(label.clone())
        .accessibility_label(label)
        .small()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .when(primary, |button| button.primary())
        .bg(rgb(if primary { look.accent } else { look.fill }))
        .text_color(rgb(if primary { 0xffffff } else { look.text }))
        .when(!primary, |button| {
            button.border_1().border_color(rgb(look.line))
        })
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
}

pub fn icon_action(
    ui: &Rc<Ui>,
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    glyph: IconName,
    event: AppAction,
    look: Look,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .child(icon(glyph, look.secondary, 12.))
        .accessibility_label(label)
        .tooltip(tooltip)
        .ghost()
        .small()
        .w(px(16.))
        .h(px(16.))
        .p_0()
        .flex_shrink_0()
        .text_color(rgb(look.secondary))
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
}

pub fn title(panel: Panel) -> &'static str {
    if panel == Panel::Changes {
        "Diff"
    } else {
        panel.name()
    }
}

/// A panel's header. Beside the terminal it is a 36 pt row: the title, what
/// the panel puts beside it, and the Full screen and Close buttons; a
/// double-click on it goes full screen. In full screen it is the one 52 pt
/// title bar: the window lights, "# place › session", the title with the
/// extra beside it, Exit full screen and Close. The Desk session's panels draw it
/// too.
pub fn panel_header(
    ui: &Rc<Ui>,
    state: &AppState,
    panel: Panel,
    full: bool,
    extra: Option<Div>,
    look: Look,
) -> Div {
    header_with(ui, panel, full, extra, None, look, session_line(state))
}

/// [`panel_header`] with `actions` drawn before Exit full screen in the
/// full-screen bar (the Diff's New agent tab, UXSIMPLE 3-3); beside the
/// terminal they don't show.
pub fn panel_header_actions(
    ui: &Rc<Ui>,
    state: &AppState,
    panel: Panel,
    full: bool,
    extra: Option<Div>,
    actions: Option<Div>,
    look: Look,
) -> Div {
    header_with(ui, panel, full, extra, actions, look, session_line(state))
}

/// "# place › session" of the open session, for the full-screen title bar.
pub fn session_line(state: &AppState) -> Option<SharedString> {
    let session = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))?;
    Some(SharedString::from(format!(
        "{} › {}",
        humata2_clients::desk::place(&state.workspace, &session.channel),
        session.title
    )))
}

/// The header's drawing; `line` is the "# place › session" of full screen.
pub fn header_with(
    ui: &Rc<Ui>,
    panel: Panel,
    full: bool,
    extra: Option<Div>,
    actions: Option<Div>,
    look: Look,
    line: Option<SharedString>,
) -> Div {
    if full {
        return row()
            .h(px(52.))
            .px(px(16.))
            .gap(px(12.))
            .flex_shrink_0()
            .bg(rgb(look.content))
            .border_b_1()
            .border_color(rgb(look.line))
            .child(lights())
            .child(div().w(px(4.)))
            .child(icon(IconName::PanelLeftOpen, look.secondary, 15.))
            // The names ellipsize; Exit full screen and Close always stay. The title
            // column is not clipped: the base menu is a deferred child that
            // must draw past it.
            .child(
                col()
                    .min_w_0()
                    .flex_shrink(1.)
                    .whitespace_nowrap()
                    .children(line.map(|line| {
                        crate::full_text::full_text(
                            secondary(look, line.clone())
                                .id("panel-session-line")
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis(),
                            line,
                        )
                        .test_support()
                    }))
                    .child(
                        row()
                            .gap(px(10.))
                            .min_w_0()
                            .child(
                                text(title(panel))
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .flex_shrink_0(),
                            )
                            .children(extra.map(|e| e.min_w_0().flex_shrink(1.))),
                    ),
            )
            .child(div().flex_1())
            .children(actions.map(|actions| actions.flex_shrink_0()))
            // UXSIMPLE 2-2: the way back says what it does, and the panel
            // can close from full screen too (owner's mark).
            .child(
                panel_action(
                    ui,
                    "show-terminal",
                    "Exit full screen",
                    AppAction::Session(SessionAction::ToggleWiden),
                    look,
                    false,
                )
                .tooltip("Exit full screen  ⌃⌘↩"),
            )
            .child(icon_action(
                ui,
                "close-panel",
                "Close panel",
                "Close panel",
                IconName::X,
                AppAction::Session(SessionAction::SetPanel(None)),
                look,
            ));
    }
    let toggle = ui.clone();
    // The row carries the id for the double-click; the wrapper keeps the
    // return type a plain `Div` for the callers that pick between headers.
    div().flex_shrink_0().child(
        row()
            .id("panel-header")
            .gap(px(10.))
            .h(px(36.))
            .px(px(14.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(look.line))
            // Double-click: the macOS zoom habit.
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                if event.click_count == 2 {
                    toggle.send(AppAction::Session(SessionAction::ToggleWiden), cx);
                }
            })
            .test_support()
            .child(
                text(title(panel))
                    .font_weight(FontWeight::SEMIBOLD)
                    .flex_shrink_0(),
            )
            // The extra shrinks before the buttons do; it is not clipped, because
            // the base menu is a deferred child that must draw past it.
            .children(extra.map(|e| e.min_w_0().flex_shrink(1.)))
            .child(div().flex_1())
            // The buttons are 16 pt for their 12 pt glyphs; the 6 pt gap and the
            // -2 pt margin put the glyphs 10 pt apart and 14 pt from the edge, as
            // the design draws them.
            .child(
                row()
                    .gap(px(6.))
                    .mr(px(-2.))
                    .flex_shrink_0()
                    .child(icon_action(
                        ui,
                        "expand-panel",
                        "Full screen",
                        "Full screen  ⌃⌘↩",
                        IconName::Maximize2,
                        AppAction::Session(SessionAction::ToggleWiden),
                        look,
                    ))
                    .child(icon_action(
                        ui,
                        "close-panel",
                        "Close panel",
                        "Close panel",
                        IconName::X,
                        AppAction::Session(SessionAction::SetPanel(None)),
                        look,
                    )),
            ),
    )
}

pub fn panel_status(value: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match value {
        Status::Working => dot.bg(rgb(GREEN)),
        Status::NeedsYou => dot.bg(rgb(ORANGE)),
        Status::Exited { .. } => return div().child(icon(IconName::X, RED, 10.)),
        Status::Offline | Status::Stopped | Status::AgentStopped => dot.bg(rgb(0x8e8e93)),
        _ => dot.border_1().border_color(rgb(0x8e8e93)),
    };
    row()
        .size(px(10.))
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}

pub fn card(look: Look) -> Div {
    col()
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
}

pub fn selected<'a>(
    state: &AppState,
    diffs: &[&'a RepoDiff],
) -> Option<(&'a RepoDiff, &'a humata2_clients::model::FileDiff)> {
    let selection = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.review.saved.selected.get(id));
    selection
        .and_then(|(repo, path)| {
            diffs.iter().find_map(|diff| {
                (diff.repo == *repo)
                    .then(|| {
                        diff.files
                            .iter()
                            .find(|file| file.path == *path)
                            .map(|file| (*diff, file))
                    })
                    .flatten()
            })
        })
        .or_else(|| {
            diffs
                .iter()
                .find_map(|diff| diff.files.first().map(|file| (*diff, file)))
        })
}

pub fn family(state: &AppState) -> HashSet<SessionId> {
    let mut family = HashSet::new();
    if let Some(id) = &state.session.open {
        family.insert(id.clone());
    }
    loop {
        let old = family.len();
        for session in &state.workspace.sessions {
            if session
                .parent
                .as_ref()
                .is_some_and(|parent| family.contains(parent))
            {
                family.insert(session.id.clone());
            }
        }
        if family.len() == old {
            return family;
        }
    }
}
