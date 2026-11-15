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
use humata2_settings::{Section, SettingsAction as A, Theme};
use std::rc::Rc;
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
        .overflow_hidden()
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
                        14.,
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
            let themes = row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(
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
                        .rounded(px(6.))
                        .when(prefs.theme == theme, |x| x.bg(rgb(look.card)).shadow_sm())
                        .child(name)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            ui.send(AppAction::Settings(A::SetTheme(theme)), cx)
                        })
                }),
            );
            body = body
                .child(
                    card(look)
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
                            row()
                                .gap(px(6.))
                                .child(button(
                                    ui,
                                    "settings-font-minus",
                                    "−",
                                    AppAction::Settings(A::FontSize(-1)),
                                    look,
                                ))
                                .child(dim(look, format!("Menlo {}", prefs.font_size)))
                                .child(button(
                                    ui,
                                    "settings-font-plus",
                                    "+",
                                    AppAction::Settings(A::FontSize(1)),
                                    look,
                                ))
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
                        )),
                )
                .child(dim(look, "Panes").font_weight(FontWeight::SEMIBOLD));
            body=body.child(card(look).p(px(14.)).gap(px(10.)).children([(false,"Fixed","One tab group beside the session panel."),(true,"Simple","Two tab groups side by side in one session.")].into_iter().map(|(split,name,note)|{
                let ui=ui.clone();row().id(SharedString::from(format!("settings-panes-{name}"))).role(Role::Button).aria_label(name).aria_selected(prefs.split_groups==split).test_support().items_start().gap(px(8.)).cursor_pointer().child(div().size(px(14.)).rounded_full().border_1().border_color(rgb(if prefs.split_groups==split{look.accent}else{look.secondary})).when(prefs.split_groups==split,|x|x.bg(rgb(look.accent)).child(div().m(px(4.)).size(px(4.)).rounded_full().bg(rgb(0xffffff))))).child(col().child(name).child(dim(look,note))).on_click(move|_,_,cx|ui.send(AppAction::Settings(A::SplitGroups(split)),cx))
            }))).child(dim(look,"Switching to Fixed keeps both groups. Simple restores them when you widen the layout."));
        }
        Section::General => {
            body = body.child(
                card(look)
                    .child(setting(
                        look,
                        "Long lines",
                        "Wrap code and diffs at the pane width.",
                        toggle(ui, "settings-wrap", prefs.wrap_diffs, A::ToggleWrap, look)
                            .into_any_element(),
                    ))
                    .child(setting(
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
            );
        }
        Section::Agents => {
            body = body
                .child(dim(look, "Default agent").font_weight(FontWeight::SEMIBOLD))
                .child(
                    card(look)
                        .p(px(14.))
                        .gap(px(8.))
                        .children(Agent::ALL.into_iter().map(|agent| {
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
                );
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
                    button(
                        ui,
                        SharedString::from(format!("settings-machine {}", m.id)),
                        m.name.clone(),
                        AppAction::Hosts(humata2_hosts::HostsAction::Select(m.id.clone())),
                        look,
                    )
                }));
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
                ));
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
                ));
        }
    }
    row()
        .size_full()
        .min_h_0()
        .items_stretch()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .child(nav)
        .child(body)
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
