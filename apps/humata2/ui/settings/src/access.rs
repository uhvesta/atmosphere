//! The channel's Agent access tab (INBOXHOOKS I8): grouped rows, each an
//! Allow / Ask / Never switch over the rules behind it, with where an Ask
//! lands. The rules no row covers sit behind "Other rules".

use crate::{
    bridge::Ui,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{AccessRule, Channel, ChannelId, Grant, RuleSetting};
use humata2_settings::{
    ACCESS_GROUPS, AccessAction, AccessRow, RowSetting, SettingsAction,
};
use std::rc::Rc;

const FOOTER: &str =
    "Shell commands and file edits are controlled inside each agent\u{2019}s own settings.";

const ADVANCED_CAPABILITIES: &[&str] = &[
    "files_read_other_machine",
    "trash_restore",
    "channel_create",
    "channel_change",
    "machine_provision",
    "machine_script_run",
];

/// The Agent access tab of `channel`.
pub fn tab(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> impl IntoElement {
    page(ui, state, channel.id.clone(), look)
}

/// Settings > Agent access: the same rows over the rules every channel starts with.
pub fn global(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    page(ui, state, ChannelId::global_rules(), look)
}

/// The grouped rows over the rules kept under `scope`: a channel's, or the
/// global ones.
fn page(ui: &Rc<Ui>, state: &AppState, scope: ChannelId, look: Look) -> impl IntoElement {
    let scope = &scope;
    let rules = state
        .workspace
        .rules
        .get(scope)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut body = Vec::new();
    let mut n = 0;
    for group in ACCESS_GROUPS {
        body.push(group_heading(group.title, group.caption, look).into_any_element());
        for row in group.rows {
            body.push(grouped_row(ui, state, scope, rules, row, n, look).into_any_element());
            for (g, grant) in rules
                .iter()
                .filter(|rule| row.capabilities.contains(&rule.capability.as_str()))
                .flat_map(|rule| rule.grants.iter())
                .enumerate()
            {
                body.push(grant_row(ui, state, n, g, grant, look).into_any_element());
            }
            n += 1;
        }
    }
    let covered = |rule: &AccessRule| {
        ACCESS_GROUPS
            .iter()
            .flat_map(|g| g.rows)
            .any(|row| row.capabilities.contains(&rule.capability.as_str()))
    };
    let show_all = state.settings.access.show_all;
    let others: Vec<_> = rules.iter().enumerate().filter(|(_, r)| !covered(r)).collect();
    let advanced_count = others
        .iter()
        .filter(|(_, r)| ADVANCED_CAPABILITIES.contains(&r.capability.as_str()))
        .count();
    let toggle_count = if advanced_count > 0 {
        advanced_count
    } else {
        others.len()
    };
    let toggle = {
        let ui = ui.clone();
        let action = AppAction::Settings(SettingsAction::Access(AccessAction::ToggleAll));
        div()
            .id("access-other-toggle")
            .role(Role::Button)
            .aria_label("Advanced rules")
            .flex_shrink_0()
            .cursor_pointer()
            .text_size(px(11.5))
            .text_color(rgb(look.accent))
            .on_click(move |_, _, cx| ui.send(action.clone(), cx))
            .child(if show_all {
                "Hide advanced rules \u{2303}".to_owned()
            } else {
                format!("Show {toggle_count} advanced rules \u{2304}")
            })
            .test_support()
    };
    body.push(
        row()
            .justify_between()
            .items_center()
            .gap(px(12.))
            .mt(px(14.))
            .pt(px(10.))
            .border_t_1()
            .border_color(rgb(look.line))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(FOOTER),
            )
            .child(toggle)
            .into_any_element(),
    );
    if show_all {
        let mut list = Vec::new();
        for (index, rule) in others {
            list.push(rule_row(ui, state, scope, index, rule, look).into_any_element());
            for (g, grant) in rule.grants.iter().enumerate() {
                list.push(grant_row(ui, state, 100 + index, g, grant, look).into_any_element());
            }
        }
        body.push(
            col()
                .id("access-rules")
                .role(Role::List)
                .aria_label("Other agent access rules")
                .mt(px(6.))
                .rounded(px(9.))
                .bg(rgb(look.card))
                .border_1()
                .border_color(rgb(look.line))
                .children(list)
                .test_support()
                .into_any_element(),
        );
    }
    col()
        .size_full()
        .p(px(20.))
        .text_size(px(12.5))
        .children(state.settings.access.error.as_ref().map(|error| {
            crate::widgets::error_banner_in(
                look,
                ui,
                "access-error",
                error.clone(),
                AppAction::Settings(SettingsAction::Access(AccessAction::DismissError)),
            )
        }))
        .child(
            col()
                .id("access-groups")
                .role(Role::List)
                .aria_label("Agent access")
                .flex_1()
                .min_h_0()
                .max_w(px(1120.))
                .overflow_y_scroll()
                .children(body),
        )
}

fn group_heading(title: &'static str, caption: &'static str, look: Look) -> impl IntoElement {
    row()
        .justify_between()
        .gap(px(10.))
        .mt(px(16.))
        .mb(px(6.))
        .flex_shrink_0()
        .items_baseline()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.tertiary))
                .child(caption),
        )
}

fn grouped_row(
    ui: &Rc<Ui>,
    state: &AppState,
    scope: &ChannelId,
    rules: &[AccessRule],
    def: &'static AccessRow,
    index: usize,
    look: Look,
) -> impl IntoElement {
    let current = def.setting(rules);
    let pending = def
        .capability_ids()
        .into_iter()
        .any(|c| state.settings.access.pending.contains(&(scope.clone(), c)));
    // A row with no rule to write (always Ask) is shown, not clickable.
    let editable = !def.capabilities.is_empty() && def.options.len() > 1;
    let control = row()
        .id(("access-setting", index))
        .role(Role::RadioGroup)
        .aria_label(def.label)
        .p(px(2.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .when(pending, |x| x.opacity(0.5))
        .children(def.options.iter().copied().enumerate().map(|(n, setting)| {
            let name = match setting {
                RuleSetting::Allow => "Allow",
                RuleSetting::Ask => "Ask",
                RuleSetting::Never => "Never",
            };
            let ui = ui.clone();
            let action = AppAction::Settings(SettingsAction::Access(AccessAction::SetRow {
                channel: scope.clone(),
                row: def.key.to_owned(),
                setting,
            }));
            let selected = current == RowSetting::Is(setting);
            div()
                .id(("access-option", index * 3 + n))
                .role(Role::RadioButton)
                .aria_label(format!("{}: {name}", def.label))
                .aria_selected(selected)
                .px(px(12.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(selected, |x| {
                    x.bg(rgb(look.card)).shadow_sm().font_weight(FontWeight::SEMIBOLD)
                })
                .when(!selected, |x| x.text_color(rgb(look.secondary)))
                .when(editable && !pending && !selected, |x| {
                    x.cursor_pointer().on_click(move |_, _, cx| {
                        ui.send(action.clone(), cx);
                    })
                })
                .child(name)
                .test_support()
        }));
    row()
        .id(("access-rule", index))
        .role(Role::ListItem)
        .aria_label(def.label)
        .gap(px(14.))
        .py(px(7.))
        .flex_shrink_0()
        .border_t_1()
        .border_color(rgb(look.line))
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    row()
                        .gap(px(8.))
                        .items_center()
                        .child(div().child(def.label))
                        .when(!def.scope_chip.is_empty(), |x| {
                            x.child(
                                div()
                                    .bg(rgb(look.fill))
                                    .rounded(px(5.))
                                    .px(px(6.))
                                    .py(px(1.))
                                    .text_size(px(11.))
                                    .text_color(rgb(look.secondary))
                                    .child(def.scope_chip),
                            )
                        }),
                )
                .when(!def.detail.is_empty(), |x| {
                    x.child(
                        div()
                            .mt(px(1.))
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(def.detail),
                    )
                }),
        )
        .child(
            row()
                .flex_shrink_0()
                .gap(px(8.))
                .items_center()
                .justify_end()
                .when(current == RowSetting::Mixed, |x| {
                    x.child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child("Mixed"),
                    )
                })
                .child(control),
        )
}

fn rule_row(
    ui: &Rc<Ui>,
    state: &AppState,
    scope: &ChannelId,
    index: usize,
    rule: &AccessRule,
    look: Look,
) -> impl IntoElement {
    let pending = state
        .settings
        .access
        .pending
        .contains(&(scope.clone(), rule.capability.clone()));
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
                        channel: scope.clone(),
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
        .child(humata2_ui_core::full_text::full_text(
            div()
                .id(("access-rule-label", index))
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(rule.label.clone()),
            rule.label.clone(),
        ))
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
        .child(humata2_ui_core::full_text::full_text(
            div()
                .id(("access-grant-text", rule * 1000 + n))
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(11.5))
                .text_color(rgb(look.tertiary))
                .child(text.clone()),
            text,
        ))
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
