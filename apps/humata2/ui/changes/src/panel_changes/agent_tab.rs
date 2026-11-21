//! Resizable right/bottom Agent dock and title-bar `Agent · N ⌘J` toggle
//! for the full-screen Review UI (`DECISIONS.md` §1).
use super::*;
use humata2_clients::model::Agent;
use humata2_session::SessionAction;
use std::cell::RefCell;

/// Maximum tabs allowed for any single agent kind (including plain Terminal / zsh).
pub(crate) const MAX_PER_KIND: usize = 2;

/// Clamped bounds for the resizable Agent dock.
pub(crate) const DEFAULT_DOCK_WIDTH: f32 = 340.;
pub(crate) const MIN_DOCK_WIDTH: f32 = 240.;
pub(crate) const MAX_DOCK_WIDTH: f32 = 640.;
pub(crate) const DEFAULT_DOCK_HEIGHT: f32 = 240.;
pub(crate) const MIN_DOCK_HEIGHT: f32 = 140.;
pub(crate) const MAX_DOCK_HEIGHT: f32 = 520.;

/// Where the Agent dock is anchored in full-screen review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockPosition {
    Right,
    Bottom,
}

/// One tab inside the Agent dock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DockTab {
    pub(crate) id: usize,
    pub(crate) agent: Agent,
    pub(crate) label: String,
}

/// UI state for the full-screen Agent dock, kept per UI thread across frames.
#[derive(Clone, Debug)]
struct AgentDockState {
    open: bool,
    position: DockPosition,
    width: f32,
    height: f32,
    drag: Option<(f32, f32)>,
    tabs: Vec<DockTab>,
    active: usize,
    next_id: usize,
    add_menu_open: bool,
}

impl Default for AgentDockState {
    fn default() -> Self {
        Self {
            open: false,
            position: DockPosition::Right,
            width: DEFAULT_DOCK_WIDTH,
            height: DEFAULT_DOCK_HEIGHT,
            drag: None,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            add_menu_open: false,
        }
    }
}

thread_local! {
    static AGENT_DOCK: RefCell<AgentDockState> = RefCell::new(AgentDockState::default());
}

/// Flat total cap for the Agent dock tab strip: `enabled_kinds + 2`.
pub(crate) fn flat_total_cap(enabled_kinds: usize) -> usize {
    enabled_kinds.saturating_add(2)
}

/// Checks whether a tab of `kind` can be added under the flat cap (`enabled_kinds + 2`,
/// max 2 per kind including plain `Terminal`/`zsh`).
pub(crate) fn can_add_agent_kind(
    existing: &[Agent],
    kind: Agent,
    enabled_kinds: usize,
) -> Result<(), &'static str> {
    let cap = flat_total_cap(enabled_kinds);
    if existing.len() >= cap {
        return Err("At tab limit");
    }
    let same_kind = existing.iter().filter(|a| **a == kind).count();
    if same_kind >= MAX_PER_KIND {
        return Err("Max 2 per kind");
    }
    Ok(())
}

/// Clamps the Right dock width to `[MIN_DOCK_WIDTH, MAX_DOCK_WIDTH]` while
/// leaving at least 260 px for the diff pane.
pub(crate) fn clamp_dock_width(width: f32, panel_width: f32) -> f32 {
    let max_for_panel = (panel_width - 260.).clamp(MIN_DOCK_WIDTH, MAX_DOCK_WIDTH);
    width.clamp(MIN_DOCK_WIDTH, max_for_panel)
}

/// Clamps the Bottom dock height to `[MIN_DOCK_HEIGHT, MAX_DOCK_HEIGHT]` while
/// leaving at least 180 px for the diff pane.
pub(crate) fn clamp_dock_height(height: f32, panel_height: f32) -> f32 {
    let max_for_panel = (panel_height - 180.).clamp(MIN_DOCK_HEIGHT, MAX_DOCK_HEIGHT);
    height.clamp(MIN_DOCK_HEIGHT, max_for_panel)
}

/// Ensures the dock has at least the open session's primary author tab.
fn ensure_seeded(dock: &mut AgentDockState, state: &AppState) {
    if !dock.tabs.is_empty() {
        return;
    }
    let primary = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
        .map(|s| s.agent)
        .unwrap_or(Agent::Claude);
    let label = if primary == Agent::Shell {
        "Terminal".to_owned()
    } else {
        format!("{} \u{b7} author", primary.command())
    };
    dock.tabs.push(DockTab {
        id: 0,
        agent: primary,
        label,
    });
    dock.active = 0;
}

/// Whether the Agent dock is currently open.
pub(super) fn is_open() -> bool {
    AGENT_DOCK.with(|cell| cell.borrow().open)
}

/// Current dock anchor position (Right or Bottom).
pub(super) fn position() -> DockPosition {
    AGENT_DOCK.with(|cell| cell.borrow().position)
}

/// Effective width when anchored on the Right.
pub(super) fn effective_width(panel_width: f32) -> f32 {
    AGENT_DOCK.with(|cell| clamp_dock_width(cell.borrow().width, panel_width))
}

/// Updates dock size during an active drag gesture; returns true if resized.
pub(super) fn drag_move(event: &MouseMoveEvent, panel_width: f32, panel_height: f32) -> bool {
    AGENT_DOCK.with(|cell| {
        let mut dock = cell.borrow_mut();
        let Some((start_coord, start_size)) = dock.drag else {
            return false;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            dock.drag = None;
            return false;
        }
        match dock.position {
            DockPosition::Right => {
                let delta = start_coord - f32::from(event.position.x);
                dock.width = clamp_dock_width(start_size + delta, panel_width);
            }
            DockPosition::Bottom => {
                let delta = start_coord - f32::from(event.position.y);
                dock.height = clamp_dock_height(start_size + delta, panel_height);
            }
        }
        true
    })
}

/// Ends any active dock drag gesture; returns true if a drag was active.
pub(super) fn drag_end() -> bool {
    AGENT_DOCK.with(|cell| cell.borrow_mut().drag.take().is_some())
}

/// Active drag orientation if the Agent dock divider is currently being dragged.
pub(super) fn dragging() -> Option<DockPosition> {
    AGENT_DOCK.with(|cell| {
        let dock = cell.borrow();
        dock.drag.map(|_| dock.position)
    })
}

/// Label for the consolidated Agent toggle in the full-screen review title bar.
pub(crate) fn agent_toggle_label(agent_count: usize) -> String {
    if agent_count > 0 {
        format!("Agent \u{b7} {agent_count} \u{2318}J")
    } else {
        "Agent \u{2318}J".to_owned()
    }
}

/// Single `Agent · N ⌘J` toggle button in the full-screen title bar.
pub(super) fn button(_ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let (open, count) = AGENT_DOCK.with(|cell| {
        let mut dock = cell.borrow_mut();
        ensure_seeded(&mut dock, state);
        (dock.open, dock.tabs.len())
    });
    let label = agent_toggle_label(count);
    div()
        .id("agent-dock-toggle")
        .role(Role::Button)
        .aria_label(label.clone())
        .aria_expanded(open)
        .test_support()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(24.))
        .px(px(9.))
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(if open { look.accent } else { look.line }))
        .bg(rgb(if open { look.fill } else { look.card }))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(if open { look.accent } else { look.text }))
        .whitespace_nowrap()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            AGENT_DOCK.with(|cell| {
                let mut dock = cell.borrow_mut();
                dock.open = !dock.open;
                if !dock.open {
                    dock.add_menu_open = false;
                }
            });
            cx.refresh_windows();
        })
        .child(label)
}

/// Drag-resize handle for the Agent dock (vertical for Right, horizontal for Bottom).
pub(super) fn resize_handle(look: Look, panel_width: f32, panel_height: f32) -> impl IntoElement {
    let pos = position();
    let is_right = pos == DockPosition::Right;
    let is_dragging = dragging().is_some();
    div()
        .id("agent-dock-divider")
        .role(Role::Splitter)
        .aria_label("Resize Agent dock")
        .test_support()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .group("agent-dock-divider")
        .when(is_right, |el| {
            el.w(px(crate::theme::DIVIDER))
                .h_full()
                .cursor_col_resize()
                .child(
                    div()
                        .w(px(4.))
                        .h(px(36.))
                        .rounded_full()
                        .bg(rgb(look.secondary))
                        .opacity(if is_dragging { 1. } else { 0. })
                        .group_hover("agent-dock-divider", |g| g.opacity(1.)),
                )
        })
        .when(!is_right, |el| {
            el.h(px(crate::theme::DIVIDER))
                .w_full()
                .cursor_row_resize()
                .child(
                    div()
                        .h(px(4.))
                        .w(px(36.))
                        .rounded_full()
                        .bg(rgb(look.secondary))
                        .opacity(if is_dragging { 1. } else { 0. })
                        .group_hover("agent-dock-divider", |g| g.opacity(1.)),
                )
        })
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            AGENT_DOCK.with(|cell| {
                let mut dock = cell.borrow_mut();
                if event.click_count == 2 {
                    dock.drag = None;
                    match dock.position {
                        DockPosition::Right => dock.width = DEFAULT_DOCK_WIDTH,
                        DockPosition::Bottom => dock.height = DEFAULT_DOCK_HEIGHT,
                    }
                } else {
                    match dock.position {
                        DockPosition::Right => {
                            let cur = clamp_dock_width(dock.width, panel_width);
                            dock.drag = Some((f32::from(event.position.x), cur));
                        }
                        DockPosition::Bottom => {
                            let cur = clamp_dock_height(dock.height, panel_height);
                            dock.drag = Some((f32::from(event.position.y), cur));
                        }
                    }
                }
            });
            cx.refresh_windows();
        })
}

/// The resizable Right/Bottom Agent dock with tab strip, `+` flat-cap menu,
/// position toggle (`bottom` / `right`), and close button.
pub(super) fn dock(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    panel_width: f32,
    panel_height: f32,
) -> impl IntoElement {
    let (pos, tabs, active, add_open, size_px) = AGENT_DOCK.with(|cell| {
        let mut dock = cell.borrow_mut();
        ensure_seeded(&mut dock, state);
        let size = match dock.position {
            DockPosition::Right => clamp_dock_width(dock.width, panel_width),
            DockPosition::Bottom => clamp_dock_height(dock.height, panel_height),
        };
        (
            dock.position,
            dock.tabs.clone(),
            dock.active.min(dock.tabs.len().saturating_sub(1)),
            dock.add_menu_open,
            size,
        )
    });
    let is_right = pos == DockPosition::Right;
    let active_tab = tabs.get(active).cloned();
    let tab_agents: Vec<Agent> = tabs.iter().map(|t| t.agent).collect();

    let header = row()
        .id("agent-dock-header")
        .test_support()
        .h(px(32.))
        .px(px(8.))
        .gap(px(4.))
        .flex_shrink_0()
        .items_center()
        .border_b_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.panel))
        .children(tabs.iter().enumerate().map(|(ix, tab)| {
            let selected = ix == active;
            let is_shell = tab.agent == Agent::Shell;
            row()
                .id(SharedString::from(format!("agent-dock-tab-{}", tab.id)))
                .role(Role::Tab)
                .aria_selected(selected)
                .test_support()
                .gap(px(5.))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(selected, |r| {
                    r.bg(rgb(look.card))
                        .text_color(rgb(look.text))
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!selected, |r| r.text_color(rgb(look.secondary)))
                .child(if is_shell {
                    icon(IconName::SquareTerminal, look.secondary, 11.).into_any_element()
                } else {
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .bg(rgb(if selected { GREEN } else { look.tertiary }))
                        .into_any_element()
                })
                .child(text(tab.label.clone()).whitespace_nowrap())
                .on_click(move |_, _, cx| {
                    AGENT_DOCK.with(|cell| {
                        cell.borrow_mut().active = ix;
                    });
                    cx.refresh_windows();
                })
        }))
        .child(
            div()
                .relative()
                .child(
                    div()
                        .id("agent-dock-add")
                        .role(Role::Button)
                        .aria_label("Add agent tab")
                        .test_support()
                        .w(px(22.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_size(px(14.))
                        .text_color(rgb(look.secondary))
                        .hover(move |s| s.bg(rgb(look.fill)))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            AGENT_DOCK.with(|cell| {
                                let mut dock = cell.borrow_mut();
                                dock.add_menu_open = !dock.add_menu_open;
                            });
                            cx.refresh_windows();
                        })
                        .child("+"),
                )
                .when(add_open, |wrap| {
                    wrap.child(deferred(add_menu_scrim())).child(deferred(add_menu(
                        ui,
                        state,
                        &tab_agents,
                        look,
                        is_right,
                        panel_width,
                        panel_height,
                    )))
                }),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("agent-dock-position")
                .role(Role::Button)
                .aria_label(if is_right { "Move dock to bottom" } else { "Move dock to right" })
                .test_support()
                .px(px(6.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(rgb(look.secondary))
                .hover(move |s| s.bg(rgb(look.fill)))
                .on_click(move |_, _, cx| {
                    AGENT_DOCK.with(|cell| {
                        let mut dock = cell.borrow_mut();
                        dock.position = match dock.position {
                            DockPosition::Right => DockPosition::Bottom,
                            DockPosition::Bottom => DockPosition::Right,
                        };
                    });
                    cx.refresh_windows();
                })
                .child(if is_right { "\u{21a6} bottom" } else { "\u{21a4} right" }),
        )
        .child(
            div()
                .id("agent-dock-close")
                .role(Role::Button)
                .aria_label("Close Agent dock")
                .test_support()
                .w(px(22.))
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .hover(move |s| s.bg(rgb(look.fill)))
                .on_click(move |_, _, cx| {
                    AGENT_DOCK.with(|cell| {
                        let mut dock = cell.borrow_mut();
                        dock.open = false;
                        dock.add_menu_open = false;
                    });
                    cx.refresh_windows();
                })
                .child("\u{00d7}"),
        );

    let session_summary = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
        .map(|s| format!("{} \u{b7} {}", s.title, s.branch))
        .unwrap_or_else(|| "Active review session".to_owned());

    let body = col()
        .id("agent-dock-body")
        .test_support()
        .flex_1()
        .min_h_0()
        .p(px(12.))
        .gap(px(8.))
        .bg(rgb(look.term))
        .child(
            secondary(
                look,
                format!(
                    "{} \u{b7} {}",
                    active_tab.as_ref().map_or("agent", |t| t.label.as_str()),
                    session_summary
                ),
            )
            .font_family("Menlo")
            .text_size(px(11.5)),
        )
        .child(
            secondary(
                look,
                "Synced with review notes. Ask about any file or line in this diff.",
            )
            .text_size(px(12.)),
        );

    col()
        .id("agent-dock")
        .test_support()
        .flex_shrink_0()
        .bg(rgb(look.term))
        .when(is_right, |d| {
            d.w(px(size_px))
                .h_full()
                .border_l_1()
                .border_color(rgb(look.line))
        })
        .when(!is_right, |d| {
            d.h(px(size_px))
                .w_full()
                .border_t_1()
                .border_color(rgb(look.line))
        })
        .child(header)
        .child(body)
}

fn add_menu_scrim() -> impl IntoElement {
    div()
        .id("agent-dock-add-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            AGENT_DOCK.with(|cell| {
                cell.borrow_mut().add_menu_open = false;
            });
            cx.refresh_windows();
        })
}

/// `+` popover inside the Agent dock tab strip enforcing the flat cap
/// (`enabled kinds + 2`, max 2 per kind including plain `Terminal` / `zsh`).
fn add_menu(
    ui: &Rc<Ui>,
    state: &AppState,
    existing: &[Agent],
    look: Look,
    is_right: bool,
    panel_width: f32,
    panel_height: f32,
) -> impl IntoElement {
    let missing = state.missing_agents();
    let mut offered = state.settings.offered_agents();
    offered.sort_by_key(|agent| missing.contains(agent));
    let enabled_kinds = offered.iter().filter(|a| !missing.contains(a)).count() + 1;
    let cap = flat_total_cap(enabled_kinds);
    let menu_w = (panel_width - 16.).clamp(180., 280.);
    let menu_max_h = (panel_height * 0.5).clamp(160., 360.);

    humata2_ui_core::action_menu::card("agent-dock-add-menu", menu_w, look)
        .when(is_right, |m| m.top(px(26.)).right(px(0.)))
        .when(!is_right, |m| m.bottom(px(26.)).left(px(0.)))
        .max_h(px(menu_max_h))
        .p(px(6.))
        .occlude()
        .child(
            div()
                .px(px(8.))
                .py(px(4.))
                .text_size(px(11.))
                .text_color(rgb(look.secondary))
                .child(format!(
                    "Open tab ({} / {cap} \u{b7} max {MAX_PER_KIND} per kind)",
                    existing.len()
                )),
        )
        .child(add_entry(
            ui,
            Agent::Shell,
            can_add_agent_kind(existing, Agent::Shell, enabled_kinds).err(),
            look,
        ))
        .child(div().mx(px(6.)).my(px(4.)).h(px(1.)).bg(rgb(look.line)))
        .children(offered.into_iter().map(|agent| {
            if missing.contains(&agent) {
                disabled_entry(agent, "not installed", look).into_any_element()
            } else if let Err(reason) = can_add_agent_kind(existing, agent, enabled_kinds) {
                disabled_entry(agent, reason, look).into_any_element()
            } else {
                add_entry(ui, agent, None, look).into_any_element()
            }
        }))
        .overflow_y_scroll()
        .test_support()
}

fn add_entry(
    ui: &Rc<Ui>,
    agent: Agent,
    disabled_reason: Option<&'static str>,
    look: Look,
) -> impl IntoElement {
    if let Some(reason) = disabled_reason {
        return disabled_entry(agent, reason, look).into_any_element();
    }
    let command = agent.command();
    let shell = agent == Agent::Shell;
    let name: SharedString = if shell { "Terminal (zsh)".into() } else { command.into() };
    let ui = ui.clone();
    row()
        .id(SharedString::from(format!("agent-dock-add-{command}")))
        .flex_1()
        .gap(px(8.))
        .h(px(26.))
        .px(px(8.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(rgb(look.text))
        .hover(move |r| r.bg(rgb(look.fill)))
        .child(if shell {
            icon(IconName::SquareTerminal, look.secondary, 12.).into_any_element()
        } else {
            div()
                .size(px(7.))
                .rounded_full()
                .bg(rgb(GREEN))
                .into_any_element()
        })
        .child(text(name))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            AGENT_DOCK.with(|cell| {
                let mut dock = cell.borrow_mut();
                let id = dock.next_id;
                dock.next_id += 1;
                let label = if shell {
                    "Terminal".to_owned()
                } else {
                    command.to_owned()
                };
                dock.tabs.push(DockTab { id, agent, label });
                dock.active = dock.tabs.len().saturating_sub(1);
                dock.add_menu_open = false;
            });
            ui.send(AppAction::Review(ReviewAction::AgentTabPicked(agent)), cx);
            if shell {
                ui.send(
                    AppAction::Session(SessionAction::OpenTerminal {
                        agent,
                        split: false,
                    }),
                    cx,
                );
            } else {
                let prompt = ui.read(|state| {
                    let id = state.session.open.as_ref()?;
                    let session = state
                        .workspace
                        .session(id)
                        .or_else(|| state.session.workspace.session(id))?;
                    Some(humata2_review::opening_prompt::opening_prompt(
                        &state.review,
                        session,
                    ))
                });
                if let Some(prompt) = prompt {
                    ui.send(
                        AppAction::Session(SessionAction::OpenAgentTab { agent, prompt }),
                        cx,
                    );
                }
            }
            cx.refresh_windows();
        })
        .test_support()
        .into_any_element()
}

fn disabled_entry(agent: Agent, reason: &str, look: Look) -> impl IntoElement {
    let command = agent.command();
    let label = if agent == Agent::Shell {
        format!("Terminal (zsh) \u{b7} {reason}")
    } else {
        format!("{command} \u{b7} {reason}")
    };
    row()
        .id(SharedString::from(format!("agent-dock-add-{command}-disabled")))
        .flex_1()
        .gap(px(8.))
        .h(px(26.))
        .px(px(8.))
        .text_size(px(12.))
        .text_color(rgb(look.secondary))
        .opacity(0.6)
        .child(div().size(px(7.)).rounded_full().bg(rgb(look.line)))
        .child(text(label))
        .test_support()
}

#[cfg(test)]
mod unified_review_unit_tests {
    use super::{
        MAX_DOCK_HEIGHT, MAX_DOCK_WIDTH, MIN_DOCK_HEIGHT, MIN_DOCK_WIDTH, agent_toggle_label,
        can_add_agent_kind, clamp_dock_height, clamp_dock_width, flat_total_cap,
    };
    use humata2_clients::model::{Agent, RepoBase, RepoDiffSpec, SessionId};
    use humata2_clients::views::{LayerRepoView, LayerView, PlanView, StackView};
    use humata2_review::ReviewState;
    use humata2_review::comments::post_replies_cta_label;

    #[test]
    fn agent_toggle_label_formats_count() {
        assert_eq!(agent_toggle_label(0), "Agent \u{2318}J");
        assert_eq!(agent_toggle_label(2), "Agent \u{b7} 2 \u{2318}J");
    }

    #[test]
    fn agent_dock_flat_cap_and_per_kind_limit_enforced() {
        assert_eq!(flat_total_cap(2), 4);
        let tabs = vec![Agent::Claude, Agent::Shell, Agent::Shell];
        assert_eq!(
            can_add_agent_kind(&tabs, Agent::Shell, 2),
            Err("Max 2 per kind")
        );
        assert_eq!(can_add_agent_kind(&tabs, Agent::Claude, 2), Ok(()));
        let full = vec![Agent::Claude, Agent::Claude, Agent::Shell, Agent::Shell];
        assert_eq!(
            can_add_agent_kind(&full, Agent::Codex, 2),
            Err("At tab limit")
        );
    }

    #[test]
    fn agent_dock_resize_clamps_within_bounds() {
        assert_eq!(clamp_dock_width(100., 1200.), MIN_DOCK_WIDTH);
        assert_eq!(clamp_dock_width(900., 1200.), MAX_DOCK_WIDTH);
        assert_eq!(clamp_dock_width(500., 600.), 340.);

        assert_eq!(clamp_dock_height(50., 900.), MIN_DOCK_HEIGHT);
        assert_eq!(clamp_dock_height(800., 900.), MAX_DOCK_HEIGHT);
        assert_eq!(clamp_dock_height(400., 400.), 220.);
    }

    #[test]
    fn post_replies_cta_label_formats_singular_and_plural() {
        assert_eq!(post_replies_cta_label(1), "Post 1 reply to GitHub");
        assert_eq!(post_replies_cta_label(3), "Post 3 replies to GitHub");
    }

    #[test]
    fn push_confirmation_summary_defaults_to_draft_and_reports_warnings() {
        let session = SessionId::new("se_1");
        let mut review = ReviewState {
            session: Some(session.clone()),
            ..ReviewState::default()
        };
        review.plans.insert(
            session.clone(),
            PlanView {
                stack: Some(StackView {
                    layers: vec![LayerView {
                        title: "schema".to_owned(),
                        repos: vec![LayerRepoView {
                            repo: "acme/api".to_owned(),
                            spec: RepoDiffSpec {
                                repo: "acme/api".to_owned(),
                                base: RepoBase::Range {
                                    base: "main".to_owned(),
                                    head: "HEAD".to_owned(),
                                },
                                options: Default::default(),
                            },
                            pull_request: String::new(),
                            files: 1,
                            drift: None,
                        }],
                        ..LayerView::default()
                    }],
                    whole: Vec::new(),
                }),
                ..PlanView::default()
            },
        );
        let summary = review.push_confirmation_summary(&session, 1);
        assert!(summary.draft);
        assert_eq!(summary.unreviewed_count, 1);
        assert_eq!(summary.open_notes, 1);
    }

    #[test]
    fn rebased_unchanged_layer_flag_tracks_session_and_layer() {
        let session = SessionId::new("se_1");
        let mut review = ReviewState::default();
        assert!(!review.is_layer_rebased_unchanged(&session, "schema"));
        review
            .rebased_unchanged_layers
            .insert((session.clone(), "schema".to_owned()));
        assert!(review.is_layer_rebased_unchanged(&session, "schema"));
    }
}
