//! Colours and small parts shared by every mock.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};

/// One appearance's colours. The app follows the system's light or dark.
#[derive(Clone, Copy)]
pub struct Look {
    pub dark: bool,
    /// The sidebar, which runs under the title bar.
    pub sidebar: u32,
    /// Pages and toolbars.
    pub content: u32,
    /// The right-hand panel.
    pub panel: u32,
    /// Hairlines.
    pub line: u32,
    pub text: u32,
    /// Secondary text and section headers.
    pub secondary: u32,
    /// Placeholders and the faintest text.
    pub tertiary: u32,
    pub selection: u32,
    pub selection_text: u32,
    pub accent: u32,
    /// Grouped controls, inactive tabs, hovered rows.
    pub fill: u32,
    /// Cards that sit on the page.
    pub card: u32,
    pub term: u32,
    pub term_text: u32,
    pub term_dim: u32,
    pub add_bg: u32,
    pub del_bg: u32,
    /// The tinted background behind a needs-you card or line.
    pub attention_bg: u32,
}

pub const LIGHT: Look = Look {
    dark: false,
    sidebar: 0xe9e7ea,
    content: 0xffffff,
    panel: 0xf6f5f7,
    line: 0xdcdadd,
    text: 0x1d1d1f,
    secondary: 0x6e6e73,
    tertiary: 0xa1a1a6,
    selection: 0x0a6cff,
    selection_text: 0xffffff,
    accent: 0x0a6cff,
    fill: 0xeceaee,
    card: 0xffffff,
    term: 0xffffff,
    term_text: 0x1d1d1f,
    term_dim: 0x8a8a8f,
    add_bg: 0xe6f6ea,
    del_bg: 0xfdeaea,
    attention_bg: 0xfff4e0,
};

pub const DARK: Look = Look {
    dark: true,
    sidebar: 0x2a2a2d,
    content: 0x1e1e1f,
    panel: 0x252527,
    line: 0x3a3a3d,
    text: 0xe8e8ea,
    secondary: 0x98989e,
    tertiary: 0x6a6a70,
    selection: 0x0a6cff,
    selection_text: 0xffffff,
    accent: 0x3b8cff,
    fill: 0x323235,
    card: 0x2a2a2c,
    term: 0x1a1a1b,
    term_text: 0xdcdcde,
    term_dim: 0x7c7c82,
    add_bg: 0x17301f,
    del_bg: 0x3a1d1d,
    attention_bg: 0x3a2c12,
};

pub const SYSTEM_FONT: &str = ".SystemUIFont";
pub const MONO: &str = "Menlo";
pub const GREEN: u32 = 0x30d158;
pub const ORANGE: u32 = 0xff9f0a;
pub const RED: u32 = 0xff453a;
pub const GREY: u32 = 0x8e8e93;
pub const PURPLE: u32 = 0xbf5af2;

pub fn c(hex: u32) -> Rgba {
    rgb(hex)
}

/// A session's or machine's state, drawn as its dot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum St {
    Working,
    NeedsYou,
    Idle,
    Exited,
    Offline,
}

pub fn status(state: St) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match state {
        St::Working => dot.bg(c(GREEN)),
        St::NeedsYou => dot.bg(c(ORANGE)),
        St::Idle => dot.border_1().border_color(c(GREY)),
        St::Offline => dot.bg(c(GREY)),
        St::Exited => return div().flex_shrink_0().child(Icon::new(IconName::X).size(px(10.)).text_color(c(RED))),
    };
    div().size(px(10.)).flex().items_center().justify_center().flex_shrink_0().child(dot)
}

pub fn icon(name: IconName, hex: u32) -> Icon {
    Icon::new(name).size(px(15.)).text_color(c(hex))
}

pub fn small_icon(name: IconName, hex: u32) -> Icon {
    Icon::new(name).size(px(12.)).text_color(c(hex))
}

/// The session header's Back: a 24 pt ghost with a 14 pt chevron (DS-58,
/// D848).
pub fn back_ghost(look: Look) -> Div {
    div()
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .child(Icon::new(IconName::ChevronLeft).size(px(14.)).text_color(c(look.secondary)))
}

pub fn traffic_lights() -> Div {
    div()
        .flex()
        .gap(px(8.))
        .flex_shrink_0()
        .child(div().size(px(12.)).rounded_full().bg(c(0xff5f57)))
        .child(div().size(px(12.)).rounded_full().bg(c(0xfebc2e)))
        .child(div().size(px(12.)).rounded_full().bg(c(0x28c840)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
}

pub fn button(look: Look, kind: Kind, label: &'static str) -> Div {
    let base = div()
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0();
    let base = match kind {
        Kind::Primary => base.bg(c(look.accent)).text_color(c(0xffffff)),
        Kind::Secondary => base.bg(c(look.fill)).text_color(c(look.text)).border_1().border_color(c(look.line)),
    };
    base.child(label)
}

/// A round count beside a row or tab.
pub fn badge(look: Look, text: &'static str, strong: bool) -> Div {
    div()
        .px(px(6.))
        .h(px(16.))
        .flex()
        .items_center()
        .rounded_full()
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .when(strong, |badge| badge.bg(c(look.accent)).text_color(c(0xffffff)))
        .when(!strong, |badge| badge.bg(c(look.fill)).text_color(c(look.secondary)))
        .child(text)
}

pub fn chip(look: Look, text: &'static str, hex: u32) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(look.line))
        .text_size(px(11.))
        .text_color(c(hex))
        .flex_shrink_0()
        .child(text)
}

pub fn kbd(look: Look, key: &'static str) -> Div {
    div()
        .px(px(5.))
        .rounded(px(4.))
        .bg(c(look.fill))
        .text_size(px(11.))
        .text_color(c(look.secondary))
        .flex_shrink_0()
        .child(key)
}

pub fn label(look: Look, title: &'static str) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(look.secondary))
        .child(title)
}

pub fn secondary(look: Look, text: impl Into<SharedString>) -> Div {
    div().text_size(px(11.5)).text_color(c(look.secondary)).child(text.into())
}

/// The drag handle between two columns; `grip` draws it as it looks under
/// the pointer.
pub fn handle(look: Look, grip: bool) -> Div {
    div()
        .w(px(1.))
        .h_full()
        .flex_shrink_0()
        .bg(c(look.line))
        .flex()
        .items_center()
        .justify_center()
        .when(grip, |handle| {
            handle.child(div().w(px(5.)).h(px(36.)).rounded_full().bg(c(look.secondary)).flex_shrink_0())
        })
}

pub fn hline(look: Look) -> Div {
    div().h(px(1.)).w_full().flex_shrink_0().bg(c(look.line))
}

/// A text field drawn with its placeholder or value.
pub fn field(look: Look, text: &'static str, placeholder: bool) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(26.))
        .px(px(8.))
        .rounded(px(6.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .text_size(px(12.5))
        .text_color(c(if placeholder { look.tertiary } else { look.text }))
        .child(text)
}

/// A pop-up menu button: its value and a chevron.
pub fn popup(look: Look, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(c(look.fill))
        .border_1()
        .border_color(c(look.line))
        .text_size(px(12.))
        .flex_shrink_0()
        .child(text)
        .child(small_icon(IconName::ChevronsUpDown, look.secondary))
}

/// A segmented control; `on` is the selected segment.
pub fn segmented(look: Look, items: &[&'static str], on: usize) -> Div {
    div()
        .flex()
        .p(px(2.))
        .rounded(px(7.))
        .bg(c(look.fill))
        .flex_shrink_0()
        .children(items.iter().enumerate().map(|(index, item)| {
            div()
                .px(px(12.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(index == on, |segment| segment.bg(c(look.card)).shadow_sm())
                .child(*item)
        }))
}

pub fn checkbox(look: Look, on: bool, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .text_size(px(12.5))
        .child(
            div()
                .size(px(14.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .when(on, |b| b.bg(c(look.accent)).child(Icon::new(IconName::Check).size(px(10.)).text_color(c(0xffffff))))
                .when(!on, |b| b.border_1().border_color(c(look.tertiary)).bg(c(look.card))),
        )
        .child(text)
}

pub fn radio(look: Look, on: bool, text: &'static str, note: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .mt(px(2.))
                .size(px(14.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(on, |b| b.bg(c(look.accent)).child(div().size(px(5.)).rounded_full().bg(c(0xffffff))))
                .when(!on, |b| b.border_1().border_color(c(look.tertiary)).bg(c(look.card))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.5)).child(text))
                .child(secondary(look, note)),
        )
}

pub fn switch(look: Look, on: bool) -> Div {
    div()
        .w(px(28.))
        .h(px(16.))
        .rounded_full()
        .p(px(2.))
        .flex()
        .flex_shrink_0()
        .when(on, |s| s.bg(c(GREEN)).justify_end())
        .when(!on, |s| s.bg(c(look.fill)).border_1().border_color(c(look.line)))
        .child(div().size(px(12.)).rounded_full().bg(c(0xffffff)).shadow_sm())
}

/// A card: rounded, hairline border, on the card colour.
pub fn card(look: Look) -> Div {
    div().flex().flex_col().rounded(px(9.)).bg(c(look.card)).border_1().border_color(c(look.line))
}

/// A monospace line of terminal output.
pub fn tl(hex: u32, text: &'static str) -> Div {
    div().whitespace_nowrap().text_color(c(hex)).child(if text.is_empty() { " " } else { text })
}
