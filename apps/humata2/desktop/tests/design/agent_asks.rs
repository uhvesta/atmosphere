//! D782 (2026-11-17): Settings > Agents with each agent's own "Ask in the
//! Inbox" switch. Fable accepted the native capture of this page on
//! 2026-11-17; this frame draws the same page.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::{page, toolbar};
use crate::sidebar::Side;

/// One built-in agent: its name, whether it asks in the Inbox, and the
/// command that installs it when missing.
struct Agent {
    name: &'static str,
    asks: bool,
    install: &'static str,
}

const AGENTS: &[Agent] = &[
    Agent { name: "Claude Code", asks: true, install: "curl -fsSL https://claude.ai/install.sh | bash" },
    Agent { name: "Codex", asks: true, install: "npm install -g @openai/codex" },
    Agent { name: "Copilot", asks: false, install: "npm install -g @github/copilot" },
    Agent { name: "OpenCode", asks: true, install: "curl -fsSL https://opencode.ai/install | bash" },
];

/// Settings > Agents: every agent on, Copilot asking in its own terminal.
pub fn ask_in_inbox(look: Look) -> Div {
    let nav = |glyph: IconName, text: &'static str, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(10.))
            .rounded(px(6.))
            .when(on, |row| row.bg(c(look.selection)).text_color(c(look.selection_text)))
            .child(icon(glyph, if on { look.selection_text } else { look.accent }))
            .child(text)
    };
    let row = |indent: bool, body: Div, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .pr(px(14.))
            .pl(px(if indent { 28. } else { 14. }))
            .py(px(10.))
            .border_b_1()
            .border_color(c(look.line))
            .child(body.flex_1())
            .child(switch(look, on))
    };
    let agent = |agent: &Agent| {
        let mut asks = div().flex().flex_col().child("Ask in the Inbox");
        if !agent.asks {
            asks = asks.child(secondary(look, "Off: it asks in its own terminal"));
        }
        vec![
            row(false, div().child(agent.name), true),
            row(true, asks, agent.asks),
            row(
                true,
                div()
                    .flex()
                    .flex_col()
                    .child("Install when missing")
                    .child(div().font_family(MONO).text_color(c(look.secondary)).child(agent.install))
                    .child(secondary(look, "Never replaces one that's there")),
                false,
            ),
        ]
    };
    page(
        look,
        Side::at("Settings"),
        toolbar(look, "Settings", None, vec![]),
        div()
            .flex()
            .text_size(px(12.5))
            .child(
                div()
                    .w(px(200.))
                    .h_full()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .border_r_1()
                    .border_color(c(look.line))
                    .child(nav(IconName::Settings, "General", false))
                    .child(nav(IconName::Bot, "Agents", true))
                    .child(nav(IconName::Server, "Machines", false))
                    .child(nav(IconName::Columns2, "Layout", false))
                    .child(nav(IconName::ShieldCheck, "Agent access", false))
                    .child(nav(IconName::HardDrive, "Storage", false))
                    .child(nav(IconName::Bell, "Notifications", false))
                    .child(nav(IconName::Keyboard, "Keyboard", false)),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label(look, "Agents"))
                    .child(secondary(look, "Agents shown in pickers and started on your machines"))
                    .child(card(look).mt(px(8.)).children(AGENTS.iter().flat_map(agent))),
            ),
    )
}
