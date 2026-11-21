//! Channel › Settings tab (SETTINGSFIX): per-channel overrides for branch prefix
//! (Inherit vs custom), default agent & model, contain agents, and retention.

use std::rc::Rc;

use gpui_kit::component::{Sizable as _, input::Input};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::daemon::sample_branch;
use humata2_clients::model::{Agent, Channel};
use humata2_settings::{ChannelSettingsAction as C, SettingsAction as A};

use crate::bridge::Ui;
use crate::settings::{ProvidersInputs, button, card, dim};
use crate::theme::Look;
use crate::widgets::{col, row};

fn send(action: C) -> AppAction {
    AppAction::Settings(A::ChannelSettings(action))
}

/// Renders the per-channel Settings tab.
pub fn tab(
    ui: &Rc<Ui>,
    state: &AppState,
    channel: &Channel,
    inputs: &ProvidersInputs,
    look: Look,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let cs = &state.settings.channel_settings;
    let loaded = cs.for_channel(&channel.id);
    let global_prefix = state
        .settings
        .general
        .settings
        .as_ref()
        .map(|s| s.branch_prefix.as_str())
        .unwrap_or("");
    let is_custom_prefix = if cs.active_channel.as_ref() == Some(&channel.id) {
        cs.branch_prefix_custom
    } else {
        loaded.is_some_and(|s| s.branch_prefix.is_some())
    };
    let prefix_draft = if cs.active_channel.as_ref() == Some(&channel.id) {
        cs.branch_prefix_draft.as_str()
    } else {
        loaded.and_then(|s| s.branch_prefix.as_deref()).unwrap_or("")
    };
    let effective_prefix = if is_custom_prefix {
        prefix_draft
    } else {
        global_prefix
    };
    let preview = sample_branch(effective_prefix);

    let prefix_input = inputs.text_input(
        &format!("ch-branch-prefix-{}", channel.id),
        "e.g. team/",
        prefix_draft,
        |text| send(C::BranchPrefixChanged(text)),
        Some(send(C::SaveBranchPrefix(channel.id.clone()))),
        window,
        cx,
    );

    let model_draft = if cs.active_channel.as_ref() == Some(&channel.id) {
        cs.default_model_draft.as_str()
    } else {
        loaded
            .and_then(|s| s.default_agent.as_ref())
            .map(|a| a.model.as_str())
            .unwrap_or("")
    };
    let model_input = inputs.text_input(
        &format!("ch-default-model-{}", channel.id),
        "Leave empty for agent default",
        model_draft,
        |text| send(C::DefaultModelChanged(text)),
        Some(send(C::SaveDefaultModel(channel.id.clone()))),
        window,
        cx,
    );

    let pr_days_draft = if cs.active_channel.as_ref() == Some(&channel.id) {
        cs.archive_pr_days_draft.as_str()
    } else {
        "0"
    };
    let idle_days_draft = if cs.active_channel.as_ref() == Some(&channel.id) {
        cs.archive_idle_days_draft.as_str()
    } else {
        "0"
    };
    let pr_days_input = inputs.text_input(
        &format!("ch-retention-pr-{}", channel.id),
        "0",
        pr_days_draft,
        |text| send(C::ArchivePrDaysChanged(text)),
        Some(send(C::SaveRetention(channel.id.clone()))),
        window,
        cx,
    );
    let idle_days_input = inputs.text_input(
        &format!("ch-retention-idle-{}", channel.id),
        "0",
        idle_days_draft,
        |text| send(C::ArchiveIdleDaysChanged(text)),
        Some(send(C::SaveRetention(channel.id.clone()))),
        window,
        cx,
    );

    let current_agent = loaded.and_then(|s| s.default_agent.as_ref()).map(|a| a.agent.as_str());
    let current_contain = loaded.and_then(|s| s.contain_agents);
    let offered: Vec<Agent> = state
        .settings
        .offered_agents()
        .into_iter()
        .chain([Agent::Shell])
        .collect();

    let ch_id = channel.id.clone();
    let mode_btn = |id: &'static str, label: &'static str, selected: bool, custom: bool| {
        let ui = ui.clone();
        let ch_id = ch_id.clone();
        div()
            .id(id)
            .role(Role::Tab)
            .aria_label(label)
            .aria_selected(selected)
            .test_support()
            .px(px(12.))
            .py(px(2.))
            .rounded(px(5.))
            .text_size(px(11.5))
            .when(selected, |x| x.bg(rgb(look.card)).shadow_sm())
            .child(label)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                if !selected {
                    ui.send(send(C::SetBranchPrefixMode { channel: ch_id.clone(), custom }), cx);
                }
            })
    };

    let contain_btn = |id: &'static str, label: &'static str, value: Option<bool>| {
        let ui = ui.clone();
        let ch_id = channel.id.clone();
        let selected = current_contain == value;
        div()
            .id(id)
            .role(Role::Tab)
            .aria_label(label)
            .aria_selected(selected)
            .test_support()
            .px(px(12.))
            .py(px(2.))
            .rounded(px(5.))
            .text_size(px(11.5))
            .when(selected, |x| x.bg(rgb(look.card)).shadow_sm())
            .child(label)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                if !selected {
                    ui.send(send(C::SetContainAgents { channel: ch_id.clone(), value }), cx);
                }
            })
    };

    let mut body = col()
        .id("channel-settings")
        .size_full()
        .overflow_y_scroll()
        .p(px(20.))
        .gap(px(14.))
        .text_size(px(12.5))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("Settings for # {}", channel.name)),
        )
        .child(dim(
            look,
            "Overrides for this channel only. Anything set to Inherit follows your global Settings.",
        ));

    if let Some(error) = &cs.error {
        body = body.child(
            div()
                .id("channel-settings-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .text_size(px(11.5))
                .text_color(rgb(0xff453a))
                .child(error.clone())
                .test_support(),
        );
    }

    // 1. Branch prefix card
    body = body.child(
        card(look)
            .p(px(14.))
            .gap(px(10.))
            .child(
                row()
                    .justify_between()
                    .items_center()
                    .child(
                        col()
                            .child("Branch prefix")
                            .child(dim(look, "Prefix for new session branches in this channel.")),
                    )
                    .child(
                        row()
                            .p(px(2.))
                            .rounded(px(7.))
                            .bg(rgb(look.fill))
                            .child(mode_btn("ch-prefix-inherit", "Inherit", !is_custom_prefix, false))
                            .child(mode_btn("ch-prefix-custom", "Custom", is_custom_prefix, true)),
                    ),
            )
            .when(is_custom_prefix, |x| {
                x.child(
                    row()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            div().w(px(220.)).child(
                                Input::new(&prefix_input)
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
                            "ch-prefix-save",
                            "Save",
                            send(C::SaveBranchPrefix(channel.id.clone())),
                            look,
                        )),
                )
            })
            .child(
                row()
                    .gap(px(6.))
                    .child(dim(look, "Sample branch:"))
                    .child(
                        div()
                            .id("ch-branch-prefix-preview")
                            .font_family("Menlo")
                            .text_size(px(11.5))
                            .text_color(rgb(look.text))
                            .child(preview),
                    ),
            ),
    );

    // 2. Default agent & model card
    body = body.child(
        card(look)
            .p(px(14.))
            .gap(px(10.))
            .child(
                col()
                    .child("Default agent")
                    .child(dim(look, "Agent and optional model for new sessions in this channel.")),
            )
            .child(
                row()
                    .gap(px(8.))
                    .flex_wrap()
                    .child(
                        button(
                            ui,
                            "ch-agent-inherit",
                            "Inherit global",
                            send(C::SetDefaultAgent {
                                channel: channel.id.clone(),
                                agent: None,
                            }),
                            look,
                        )
                        .when(current_agent.is_none(), |x| {
                            x.bg(rgb(look.accent)).text_color(rgb(0xffffff))
                        }),
                    )
                    .children(offered.into_iter().map(|agent| {
                        let cmd = agent.command().to_owned();
                        let picked = current_agent == Some(agent.command());
                        button(
                            ui,
                            SharedString::from(format!("ch-agent-{}", agent.command())),
                            agent.name(),
                            send(C::SetDefaultAgent {
                                channel: channel.id.clone(),
                                agent: Some(cmd),
                            }),
                            look,
                        )
                        .when(picked, |x| x.bg(rgb(look.accent)).text_color(rgb(0xffffff)))
                    })),
            )
            .when(current_agent.is_some(), |x| {
                x.child(
                    row()
                        .gap(px(8.))
                        .items_center()
                        .child(dim(look, "Model:"))
                        .child(
                            div().w(px(220.)).child(
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
                            "ch-model-save",
                            "Save model",
                            send(C::SaveDefaultModel(channel.id.clone())),
                            look,
                        )),
                )
            }),
    );

    // 3. Contain agents card
    body = body.child(
        card(look).p(px(14.)).child(
            row()
                .justify_between()
                .items_center()
                .child(
                    col()
                        .child("Contain agents")
                        .child(dim(look, "Restrict agents in this channel to their session clones.")),
                )
                .child(
                    row()
                        .p(px(2.))
                        .rounded(px(7.))
                        .bg(rgb(look.fill))
                        .child(contain_btn("ch-contain-inherit", "Inherit", None))
                        .child(contain_btn("ch-contain-on", "On", Some(true)))
                        .child(contain_btn("ch-contain-off", "Off", Some(false))),
                ),
        ),
    );

    // 4. Retention card
    body = body.child(
        card(look)
            .p(px(14.))
            .gap(px(10.))
            .child(
                col()
                    .child("Retention")
                    .child(dim(look, "Days before auto-archiving PR reviews and idle sessions (0 = off).")),
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
                        "ch-retention-save",
                        "Save",
                        send(C::SaveRetention(channel.id.clone())),
                        look,
                    )),
            ),
    );

    div().size_full().child(body.test_support())
}
