//! The channel's Agent access tab (frame 49): one row per rule with an
//! Allow / Ask me / Never switch, and the grants kept under a rule.

use crate::{
    bridge::Ui,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{AccessRule, Channel, Grant, RuleSetting};
use humata2_settings::{AccessAction, SettingsAction};
use std::rc::Rc;

const INTRO: &str = "What agents in this channel may do through Humata. Anything set to Ask me lands in your Inbox, batched with every other agent's requests. Never tells the agent no at once, naming the rule, and adds nothing to the Inbox.";

/// The Agent access tab of `channel`.
pub(crate) fn tab(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> impl IntoElement {
    let rules = state
        .workspace
        .rules
        .get(&channel.id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut rows = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        rows.push(rule_row(ui, state, channel, index, rule, look).into_any_element());
        for (n, grant) in rule.grants.iter().enumerate() {
            rows.push(grant_row(ui, state, index, n, grant, look).into_any_element());
        }
    }
    col()
        .size_full()
        .p(px(20.))
        .text_size(px(12.5))
        .gap(px(12.))
        .child(
            div()
                .max_w(px(820.))
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(INTRO),
        )
        .children(state.settings.access.error.as_ref().map(|error| {
            crate::widgets::error_banner(
                ui,
                "access-error",
                error.clone(),
                AppAction::Settings(SettingsAction::Access(AccessAction::DismissError)),
            )
        }))
        .child(
            col()
                .id("access-rules")
                .role(Role::List)
                .aria_label("Agent access rules")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .rounded(px(9.))
                .bg(rgb(look.card))
                .border_1()
                .border_color(rgb(look.line))
                .children(rows)
                .test_support(),
        )
}

fn rule_row(
    ui: &Rc<Ui>,
    state: &AppState,
    channel: &Channel,
    index: usize,
    rule: &AccessRule,
    look: Look,
) -> impl IntoElement {
    let pending = state
        .settings
        .access
        .pending
        .contains(&(channel.id.clone(), rule.capability.clone()));
    let control = match rule.setting {
        None => div()
            .flex_shrink_0()
            .text_color(rgb(look.tertiary))
            .child("Always · no setting")
            .into_any_element(),
        Some(current) => row()
            .id(("access-setting", index))
            .role(Role::RadioGroup)
            .aria_label(rule.label.clone())
            .p(px(2.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .flex_shrink_0()
            .when(pending, |x| x.opacity(0.5))
            .children(
                [
                    (RuleSetting::Allow, "Allow"),
                    (RuleSetting::Ask, "Ask me"),
                    (RuleSetting::Never, "Never"),
                ]
                .into_iter()
                .enumerate()
                .map(|(n, (setting, name))| {
                    let ui = ui.clone();
                    let action = AppAction::Settings(SettingsAction::Access(AccessAction::Set {
                        channel: channel.id.clone(),
                        capability: rule.capability.clone(),
                        setting,
                    }));
                    let selected = setting == current;
                    div()
                        .id(("access-option", index * 3 + n))
                        .role(Role::RadioButton)
                        .aria_label(format!("{}: {name}", rule.label))
                        .aria_selected(selected)
                        .px(px(12.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .text_size(px(11.5))
                        .when(selected, |x| x.bg(rgb(look.card)).shadow_sm())
                        .when(!pending && !selected, |x| {
                            x.cursor_pointer().on_click(move |_, _, cx| {
                                ui.send(action.clone(), cx);
                            })
                        })
                        .child(name)
                        .test_support()
                }),
            )
            .into_any_element(),
    };
    row()
        .id(("access-rule", index))
        .role(Role::ListItem)
        .aria_label(rule.label.clone())
        .gap(px(12.))
        .h(px(27.))
        .px(px(12.))
        .flex_shrink_0()
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
        .child(control)
}

fn grant_row(
    ui: &Rc<Ui>,
    state: &AppState,
    rule: usize,
    n: usize,
    grant: &Grant,
    look: Look,
) -> impl IntoElement {
    let title = state
        .workspace
        .session(&grant.session)
        .map_or("a deleted session", |session| session.title.as_str());
    let text = format!(
        "Always allow · {title}, {} · granted {}",
        grant.scope, grant.granted
    );
    let remove = AppAction::Settings(SettingsAction::Access(AccessAction::RemoveGrant(
        grant.id.clone(),
    )));
    let ui = ui.clone();
    row()
        .id(("access-grant", rule * 1000 + n))
        .role(Role::ListItem)
        .aria_label(text.clone())
        .gap(px(10.))
        .h(px(24.))
        .pl(px(28.))
        .pr(px(12.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(px(11.5))
                .text_color(rgb(look.tertiary))
                .child(text),
        )
        .child(
            div()
                .id(("access-remove", rule * 1000 + n))
                .role(Role::Link)
                .aria_label("Remove")
                .flex_shrink_0()
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(rgb(look.accent))
                .child("Remove")
                .on_click(move |_, _, cx| ui.send(remove.clone(), cx))
                .test_support(),
        )
}
