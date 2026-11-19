//! New agent tab ⌄ in the full-screen Diff header (UXSIMPLE 3-3, 3-4): a
//! second agent opens as a tab in the session's own tab strip, the same
//! thing the strip's `+` does, and full screen gives way so the Diff sits
//! beside it.
use super::*;
use humata2_clients::model::Agent;
use humata2_session::SessionAction;

/// The button and, while it is open, its menu.
pub(super) fn button(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let open = state.review.agent_tab_menu;
    div()
        .relative()
        .child(
            action(
                ui,
                "new-agent-tab",
                "New agent tab",
                AppAction::Review(ReviewAction::ToggleAgentTabMenu),
                look,
                false,
            )
            .gap(px(4.))
            .child(icon(IconName::ChevronDown, look.secondary, 11.)),
        )
        .when(open, |wrap| {
            wrap.child(deferred(humata2_ui_core::action_menu::scrim(
                ui,
                "new-agent-tab-scrim",
                AppAction::Review(ReviewAction::ToggleAgentTabMenu),
            )))
            .child(deferred(menu(ui, state, look)))
        })
}

/// Terminal, a rule, then the agents Settings offers, the last pick marked.
fn menu(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let last = state.review.last_agent_tab;
    humata2_ui_core::action_menu::card("new-agent-tab-menu", 330., look)
        .top(px(30.))
        .right(px(0.))
        .p(px(6.))
        .occlude()
        .child(
            div()
                .px(px(10.))
                .pt(px(6.))
                .pb(px(8.))
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(
                    "Opens a new tab in this session, beside the diff. It starts with no \
                     history. Pick the model inside the agent.",
                ),
        )
        .child(entry(ui, Agent::Shell, last, look))
        .child(div().mx(px(6.)).my(px(4.)).h(px(1.)).bg(rgb(look.line)))
        .children(
            state
                .settings
                .offered_agents()
                .into_iter()
                .map(|agent| entry(ui, agent, last, look)),
        )
        .test_support()
}

/// One row: the pick closes the menu, leaves full screen and opens the tab.
fn entry(ui: &Rc<Ui>, agent: Agent, last: Option<Agent>, look: Look) -> impl IntoElement {
    let command = agent.command();
    let shell = agent == Agent::Shell;
    let name: SharedString = if shell { "Terminal".into() } else { command.into() };
    let ui = ui.clone();
    row()
        .id(SharedString::from(format!("new-agent-tab-{command}")))
        .flex_1()
        .gap(px(8.))
        .h(px(28.))
        .px(px(10.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .hover(move |r| r.bg(rgb(look.fill)))
        .child(if shell {
            icon(IconName::SquareTerminal, look.secondary, 13.).into_any_element()
        } else {
            div()
                .size(px(7.))
                .rounded_full()
                .bg(rgb(GREEN))
                .into_any_element()
        })
        .child(text(name.clone()))
        .child(div().flex_1())
        .when(shell, |r| r.child(text("⌘T").text_color(rgb(look.secondary))))
        .when(!shell && last == Some(agent), |r| {
            r.child(
                text("last used")
                    .text_color(rgb(look.secondary))
                    .id(SharedString::from(format!("new-agent-tab-last-used-{command}")))
                    .test_support(),
            )
        })
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(AppAction::Review(ReviewAction::AgentTabPicked(agent)), cx);
            ui.send(AppAction::Session(SessionAction::ToggleWiden), cx);
            if shell {
                ui.send(
                    AppAction::Session(SessionAction::OpenTerminal {
                        agent,
                        split: false,
                    }),
                    cx,
                );
                return;
            }
            // The wording is fixed now, from what the Diff shows.
            let prompt = ui.read(|state| {
                let id = state.session.open.as_ref()?;
                let session = state.session.workspace.session(id)?;
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
        })
        .test_support()
}
