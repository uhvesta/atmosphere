//! N1: candidate macOS-native looks for the window, drawn headless on the
//! same made-up content so the owner can pick one before the real screens
//! are restyled. Nothing here is the app; each direction is a still mock.


use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};
use humata2_engine::Resources;
use humata2_snapshot::{Tolerance, snapshots};

/// One direction's colours and choices.
#[derive(Clone, Copy)]
struct Look {
    /// The sidebar, which runs under the title bar.
    sidebar: u32,
    /// The content area.
    content: u32,
    /// The inspector on the right.
    inspector: u32,
    /// Hairlines.
    line: u32,
    /// Body text.
    text: u32,
    /// Secondary text and section headers.
    secondary: u32,
    /// The selected sidebar row.
    selection: u32,
    /// Text on the selected row.
    selection_text: u32,
    /// Links, focus, the primary button.
    accent: u32,
    /// Hovered or grouped rows in the content.
    fill: u32,
    /// Xcode-style icon tabs above the sidebar list instead of screen rows.
    navigator_tabs: bool,
    /// The toolbar title sits centred with a subtitle, not on the left.
    centred_title: bool,
}

const LIGHT: Look = Look {
    sidebar: 0xe9e7ea,
    content: 0xffffff,
    inspector: 0xf6f5f7,
    line: 0xdcdadd,
    text: 0x1d1d1f,
    secondary: 0x86868b,
    selection: 0x0a6cff,
    selection_text: 0xffffff,
    accent: 0x0a6cff,
    fill: 0xf2f1f4,
    navigator_tabs: false,
    centred_title: false,
};

const DARK: Look = Look {
    sidebar: 0x2a2a2d,
    content: 0x1e1e1f,
    inspector: 0x252527,
    line: 0x3a3a3d,
    text: 0xe8e8ea,
    secondary: 0x98989e,
    selection: 0x0a6cff,
    selection_text: 0xffffff,
    accent: 0x3b8cff,
    fill: 0x2a2a2c,
    navigator_tabs: false,
    centred_title: false,
};

const XCODE: Look = Look {
    sidebar: 0x262628,
    content: 0x1c1c1e,
    inspector: 0x262628,
    line: 0x38383b,
    text: 0xe6e6e8,
    secondary: 0x8e8e93,
    selection: 0x3a3a3e,
    selection_text: 0xffffff,
    accent: 0x3b8cff,
    fill: 0x29292b,
    navigator_tabs: true,
    centred_title: true,
};

const SYSTEM_FONT: &str = ".SystemUIFont";
const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const GREY: u32 = 0x8e8e93;

struct Mock(Look);

fn c(hex: u32) -> Rgba {
    rgb(hex)
}

fn dot(hex: u32) -> Div {
    div().size(px(8.)).rounded_full().bg(c(hex)).flex_shrink_0()
}

fn traffic_lights() -> Div {
    div()
        .flex()
        .gap(px(8.))
        .child(div().size(px(12.)).rounded_full().bg(c(0xff5f57)))
        .child(div().size(px(12.)).rounded_full().bg(c(0xfebc2e)))
        .child(div().size(px(12.)).rounded_full().bg(c(0x28c840)))
}

fn icon(name: IconName, hex: u32) -> Icon {
    Icon::new(name).size(px(15.)).text_color(c(hex))
}

impl Mock {
    fn sidebar_row(&self, glyph: Option<IconName>, status: Option<u32>, label: &str, indent: f32, on: bool) -> Div {
        let look = self.0;
        let text = if on { look.selection_text } else { look.text };
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .h(px(28.))
            .mx(px(10.))
            .pl(px(8. + indent))
            .pr(px(8.))
            .rounded(px(6.))
            .when(on, |row| row.bg(c(look.selection)))
            .text_color(c(text))
            .children(glyph.map(|glyph| icon(glyph, if on { look.selection_text } else { look.accent })))
            .children(status.map(dot))
            .child(div().flex_1().child(label.to_owned()))
    }

    fn section(&self, title: &str) -> Div {
        div()
            .px(px(18.))
            .pt(px(14.))
            .pb(px(4.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(self.0.secondary))
            .child(title.to_owned())
    }

    fn sidebar(&self) -> Div {
        let look = self.0;
        let top = div().flex().items_center().h(px(52.)).px(px(20.)).child(traffic_lights());
        let screens = if look.navigator_tabs {
            div()
                .flex()
                .justify_around()
                .h(px(30.))
                .mx(px(10.))
                .mb(px(4.))
                .border_b_1()
                .border_color(c(look.line))
                .child(icon(IconName::Inbox, look.accent))
                .child(icon(IconName::House, look.secondary))
                .child(icon(IconName::LayoutDashboard, look.secondary))
                .child(icon(IconName::Server, look.secondary))
                .child(icon(IconName::Files, look.secondary))
        } else {
            div()
                .flex()
                .flex_col()
                .child(self.sidebar_row(Some(IconName::House), None, "Home", 0., false))
                .child(
                    self.sidebar_row(Some(IconName::Inbox), None, "Inbox", 0., false).child(
                        div().text_size(px(11.)).text_color(c(look.secondary)).child("3"),
                    ),
                )
                .child(self.sidebar_row(Some(IconName::LayoutDashboard), None, "Board", 0., false))
        };
        div()
            .w(px(250.))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(c(look.sidebar))
            .border_r_1()
            .border_color(c(look.line))
            .child(top)
            .child(screens)
            .child(self.section("Channels"))
            .child(self.sidebar_row(None, None, "# payments", 0., false))
            .child(self.sidebar_row(None, Some(ORANGE), "Ledger schema", 14., true))
            .child(self.sidebar_row(None, Some(GREEN), "Refund retries", 14., false))
            .child(self.sidebar_row(None, None, "# infra", 0., false))
            .child(self.sidebar_row(None, Some(GREEN), "Benchmark BRIN", 14., false))
            .child(self.sidebar_row(None, Some(GREY), "Upgrade Postgres", 14., false))
            .child(self.sidebar_row(None, None, "# web", 0., false))
            .child(self.section("Hosts"))
            .child(self.sidebar_row(None, Some(GREEN), "This Mac", 0., false))
            .child(self.sidebar_row(None, Some(GREEN), "build-box", 0., false))
    }

    fn toolbar(&self) -> Div {
        let look = self.0;
        let title = div()
            .flex()
            .flex_col()
            .when(look.centred_title, |title| title.items_center())
            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(13.)).child("Ledger schema"))
            .child(div().text_size(px(11.)).text_color(c(look.secondary)).child("#payments · claude · This Mac"));
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .h(px(52.))
            .px(px(16.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(c(look.line))
            .child(icon(IconName::ChevronLeft, look.secondary))
            .when(look.centred_title, |bar| bar.child(div().flex_1()))
            .child(title)
            .child(div().flex_1())
            .child(icon(IconName::Search, look.secondary))
            .child(icon(IconName::PanelRight, look.accent))
    }

    fn message(&self, who: &str, tint: u32, text: &str) -> Div {
        let look = self.0;
        div()
            .flex()
            .gap(px(10.))
            .py(px(8.))
            .child(div().size(px(26.)).rounded(px(6.)).bg(c(tint)).flex_shrink_0())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(who.to_owned()))
                    .child(div().text_color(c(look.text)).child(text.to_owned())),
            )
    }

    fn content(&self) -> Div {
        let look = self.0;
        let question = div()
            .mt(px(10.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(c(look.fill))
            .border_1()
            .border_color(c(look.line))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("claude is asking"))
            .child("Drop the old ledger_entries index before the migration, or keep both until the backfill ends?")
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(c(look.accent))
                            .text_color(c(0xffffff))
                            .child("Keep both"),
                    )
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(4.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(c(look.line))
                            .child("Drop it"),
                    ),
            );
        let composer = div()
            .m(px(16.))
            .h(px(38.))
            .px(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(c(look.line))
            .bg(c(look.fill))
            .flex()
            .items_center()
            .text_color(c(look.secondary))
            .child("Message claude…");
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(c(look.content))
            .child(self.toolbar())
            .child(
                div()
                    .flex_1()
                    .px(px(20.))
                    .py(px(10.))
                    .flex()
                    .flex_col()
                    .child(self.message("You", 0x5e5ce6, "Move the ledger to the new schema and keep reads working."))
                    .child(self.message(
                        "claude",
                        0xff9f0a,
                        "Planned 4 steps. The migration and the dual-write are done; the backfill is next.",
                    ))
                    .child(question),
            )
            .child(composer)
    }

    fn inspector(&self) -> Div {
        let look = self.0;
        let tab = |label: &str, on: bool| {
            div()
                .flex_1()
                .flex()
                .justify_center()
                .py(px(3.))
                .rounded(px(5.))
                .when(on, |tab| tab.bg(c(look.content)))
                .text_size(px(11.))
                .child(label.to_owned())
        };
        let file = |name: &str, add: &str, del: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(26.))
                .px(px(14.))
                .child(icon(IconName::File, look.secondary))
                .child(div().flex_1().child(name.to_owned()))
                .child(div().text_size(px(11.)).text_color(c(GREEN)).child(add.to_owned()))
                .child(div().text_size(px(11.)).text_color(c(0xff453a)).child(del.to_owned()))
        };
        div()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .bg(c(look.inspector))
            .border_l_1()
            .border_color(c(look.line))
            .child(
                div().h(px(52.)).flex().items_center().px(px(12.)).border_b_1().border_color(c(look.line)).child(
                    div()
                        .flex_1()
                        .flex()
                        .p(px(2.))
                        .rounded(px(7.))
                        .bg(c(look.fill))
                        .child(tab("Changes", true))
                        .child(tab("Inbox", false))
                        .child(tab("Files", false))
                        .child(tab("Info", false)),
                ),
            )
            .child(self.section("3 files changed"))
            .child(file("schema.sql", "+48", "−3"))
            .child(file("ledger.rs", "+112", "−20"))
            .child(file("backfill.rs", "+64", "−0"))
    }
}

impl Render for Mock {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .font_family(SYSTEM_FONT)
            .text_size(px(13.))
            .text_color(c(self.0.text))
            .bg(c(self.0.content))
            .child(self.sidebar())
            .child(self.content())
            .child(self.inspector())
    }
}

// The glob import brings in gpui's own `test` attribute, so name std's.
#[::core::prelude::v1::test]
fn native_directions() {
    let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets)).expect("the headless context starts");
    cx.update(humata2_ui::init);
    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    for (name, look) in [("a-light", LIGHT), ("b-dark", DARK), ("c-xcode", XCODE)] {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(760.))))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let window: AnyWindowHandle = cx
            .update(|cx| cx.open_window(options, |_, cx| cx.new(|_| Mock(look))))
            .expect("the window opens")
            .into();
        cx.run_until_parked();
        let image = cx.capture_screenshot(window).expect("the window draws");
        snapshots.assert_image(name, &image, Tolerance::default());
    }
}
