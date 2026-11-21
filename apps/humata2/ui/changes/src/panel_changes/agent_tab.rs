//! New agent tab ⌄ in the full-screen Diff header (UXSIMPLE 3-3, 3-4; D858):
//! a second agent opens directly as a fresh tab in the session's own tab strip
//! via `TerminalService.OpenAgentTab` (keeping its label and agent kind on the
//! daemon), and full screen gives way so the Diff sits beside it.
use super::*;
use humata2_clients::model::Agent;
use humata2_session::SessionAction;

/// Label for the consolidated Agent toggle in the full-screen review title bar.
pub(crate) fn agent_toggle_label(agent_count: usize) -> String {
    if agent_count > 0 {
        format!("Agent \u{b7} {agent_count} \u{2318}J")
    } else {
        "Agent \u{2318}J".to_owned()
    }
}

/// The button and, while it is open, its menu.
pub(super) fn button(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let open = state.review.agent_tab_menu;
    let count = usize::from(state.review.last_agent_tab.is_some());
    div()
        .relative()
        .child(
            action(
                ui,
                "new-agent-tab",
                agent_toggle_label(count),
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
    // Installed agents first, the ones the machine lacks after them.
    let missing = state.missing_agents();
    let mut offered = state.settings.offered_agents();
    offered.sort_by_key(|agent| missing.contains(agent));
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
        .children(offered.into_iter().map(|agent| {
            if missing.contains(&agent) {
                missing_entry(agent, look).into_any_element()
            } else {
                entry(ui, agent, last, look).into_any_element()
            }
        }))
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
            if !shell && ui.read(|state| state.missing_agents().contains(&agent)) {
                return;
            }
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
            // The wording is fixed now, from what the Diff shows; the daemon's
            // OpenAgentTab launches the agent directly and delivers it (D858).
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
        })
        .test_support()
}

/// An agent the session's machine lacks: dimmed, with no hover and no click.
fn missing_entry(agent: Agent, look: Look) -> impl IntoElement {
    let command = agent.command();
    row()
        .id(SharedString::from(format!("new-agent-tab-{command}")))
        .flex_1()
        .gap(px(8.))
        .h(px(28.))
        .px(px(10.))
        .text_size(px(12.5))
        .text_color(rgb(look.secondary))
        .opacity(0.6)
        .child(div().size(px(7.)).rounded_full().bg(rgb(look.line)))
        .child(text(SharedString::from(format!("{command} · not installed"))))
        .test_support()
}

#[cfg(test)]
mod unified_review_unit_tests {
    use super::agent_toggle_label;
    use humata2_clients::model::{RepoBase, RepoDiffSpec, SessionId};
    use humata2_clients::views::{LayerRepoView, LayerView, PlanView, StackView};
    use humata2_review::ReviewState;
    use humata2_review::comments::post_replies_cta_label;

    #[test]
    fn agent_toggle_label_formats_count() {
        assert_eq!(agent_toggle_label(0), "Agent \u{2318}J");
        assert_eq!(agent_toggle_label(2), "Agent \u{b7} 2 \u{2318}J");
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
