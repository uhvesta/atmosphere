//! The To-do milestone's later frames (BOARDSIMPLE; drafts): Do next's New
//! session sheet, the migration message, a pull request review session and a
//! session's To-do panel holding an agent's ask. Drawn from the signed-off
//! mockups in `workflow/milestones/mockups/boardsimple/`.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::session::Panel;

const AMBER: u32 = 0xf0b35a;
const AMBER_TEXT: u32 = 0xb25000;
const TEAL: u32 = 0x0f766e;
const ADD_GREEN: u32 = 0x1a8a3c;
const DEL_RED: u32 = 0xc0392b;

/// Text with the given phrases in bold; it wraps like any text. A phrase
/// starting with `last:` bolds its last occurrence, not its first.
fn rich(text: &'static str, bold: &[&str]) -> StyledText {
    let mut spans: Vec<_> = bold
        .iter()
        .filter_map(|phrase| {
            let (phrase, found) = match phrase.strip_prefix("last:") {
                Some(rest) => (rest, text.rfind(rest)),
                None => (*phrase, text.find(phrase)),
            };
            found.map(|start| (start..start + phrase.len(), HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() }))
        })
        .collect();
    spans.sort_by_key(|(range, _)| range.start);
    StyledText::new(text).with_highlights(spans)
}

/// A small filled tag: a key, a tag name.
fn tag(look: Look, text: &'static str) -> Div {
    crate::todo::tag(look, text)
}

fn link(look: Look, text: &'static str) -> Div {
    div().text_color(c(look.accent)).child(text)
}

/// The shared mockup sidebar: the lit row's label, the Agent inbox badge and
/// whether Ledger entries' dot is orange.
#[derive(Clone, Copy)]
struct Side(&'static str, Option<u32>, bool);

fn side_col(look: Look, side: Side) -> Div {
    if side.0 == "Ledger reads behind a flag" {
        crate::mock_sidebar::sidebar_pr_reviews_open(look, side.0, side.1, side.2)
    } else {
        crate::mock_sidebar::sidebar(look, side.0, side.1, side.2)
    }
}

/// A 1600 px page: the 280 px sidebar, then a toolbar over the body.
fn wide_page(look: Look, side: Side, bar: Div, body: Div) -> Div {
    div()
        .size_full()
        .flex()
        .line_height(relative(1.35))
        .child(side_col(look, side))
        .child(handle(look, false))
        .child(div().flex_1().min_w_0().flex().flex_col().bg(c(look.content)).child(bar).child(body.flex_1().min_h_0().overflow_hidden()))
}

// ---------------------------------------------------------------- Do next

/// Frame 188: Do next opens the New session sheet filled from the top item,
/// over `background`.
pub fn do_next(look: Look, background: Div) -> Div {
    let row = |key: &'static str, value: Div| {
        div().flex().items_center().min_h(px(31.)).child(div().w(px(96.)).flex_shrink_0().text_color(c(look.secondary)).child(key)).child(value.flex_1().min_w_0())
    };
    let message = div()
        .px(px(11.))
        .py(px(9.))
        .rounded(px(8.))
        .bg(c(look.card))
        .border_1()
        .border_color(crate::todo::rule(look))
        .flex()
        .flex_col()
        .gap(px(19.5))
        .line_height(px(19.5))
        .text_size(px(13.))
        .child("Rotate the staging database credentials (INF-6).")
        .child("The staging password is 90 days old today. Issue new credentials in Vault, roll the three services that read them, and check each one reconnects.")
        .child("Tick the to-do’s steps as you finish them, and ask me before closing it.");
    let sheet = div()
        .w(px(640.))
        .flex()
        .flex_col()
        .px(px(26.))
        .pt(px(22.))
        .pb(px(20.))
        .rounded(px(14.))
        .bg(c(look.card))
        .border_1()
        .border_color(crate::todo::rule(look))
        .shadow_lg()
        .text_size(px(13.))
        .child(div().mb(px(14.)).text_size(px(18.)).font_weight(FontWeight::BOLD).child("New session for “Rotate the staging database credentials”"))
        .child(row("To-do", div().flex().items_center().gap(px(8.)).child(tag(look, "INF-6")).child(secondary(look, "top of your list · moves to In progress when the session starts"))))
        .child(row("Channel", div().child("# infra")))
        .child(row("Agent", div().flex().child(crate::todo::segs(look, &["claude", "copilot", "codex", "Terminal only"], "claude"))))
        .child(row("Machine", div().flex().child(crate::todo::segs(look, &["This Mac", "devbox-1", "gpu-box"], "devbox-1"))))
        .child(row("Repositories", div().flex().items_center().gap(px(8.)).child(tag(look, "acme/infra")).child(link(look, "+ Add"))))
        .child(div().mt(px(13.)).mb(px(5.)).child(label(look, "FIRST MESSAGE")))
        .child(message)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(16.))
                .child(div().flex().gap(px(4.)).text_size(px(12.)).text_color(c(look.secondary)).child("Not this one?").child(link(look, "Skip to PAY-7 Refund retry with backoff")))
                .child(div().flex_1())
                .child(crate::todo::btn(look, "Cancel", false))
                .child(crate::todo::btn(look, "Start session", true)),
        );
    background.relative().child(div().absolute().top_0().bottom_0().left(px(280.)).right_0().bg(rgba(0x1414_1947))).child(
        div().absolute().top(px(110.)).left(px(280.)).right_0().flex().justify_center().child(sheet),
    )
}

/// Frame 188 over its placeholder background.
pub fn do_next_frame(look: Look) -> Div {
    do_next(look, crate::todo::list(look))
}

// ------------------------------------------------------- Migration message

/// Frame 189: the one-time message that moves the boards into To-do.
pub fn migrate_message(look: Look) -> Div {
    let ft_row = |key: &'static str, text: &'static str, bold: &[&str]| {
        div().flex().child(div().w(px(66.)).flex_shrink_0().text_color(c(look.secondary)).child(key)).child(div().flex_1().min_w_0().child(rich(text, bold)))
    };
    let item = |mark: &'static str, hex: u32, text: &'static str, bold: &[&str]| {
        div()
            .flex()
            .gap(px(8.))
            .py(px(1.5))
            .line_height(relative(1.45))
            .child(div().w(px(14.)).flex_shrink_0().flex().justify_center().font_weight(FontWeight::SEMIBOLD).text_color(c(hex)).child(mark))
            .child(div().flex_1().min_w_0().child(rich(text, bold)))
    };
    let left = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(div().mb(px(4.)).child(label(look, "WHAT CHANGES")))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .px(px(12.))
                .py(px(9.))
                .rounded(px(8.))
                .bg(tone_c(look, 0xf5f5f8, 0x2d2d30))
                .text_size(px(13.))
                .child(ft_row("Boards", "1 global, 4 channel and 6 session boards → one list, filtered by channel and session", &["one list"]))
                .child(ft_row("Items", "37 items keep their keys, titles, descriptions, sessions and history", &[]))
                .child(ft_row("Columns", "Backlog 9 and To do 11 → Open · In progress 6 and In review 2 → In progress · Done 9 → Closed", &["Open", "last:In progress", "Closed"]))
                .child(ft_row("Order", "Your order in each column is kept as the starting rank", &[]))
                .child(ft_row("Marks", "3 VIP items become pinned", &["pinned"])),
        )
        .child(div().mt(px(9.)).mb(px(4.)).child(label(look, "ALSO MOVES")))
        .child(div().child("The Inbox becomes the Agent inbox. 2 waiting board asks become Needs attention on their items; the 1 waiting file ask stays."));
    let right = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .text_size(px(13.))
        .child(div().flex().items_baseline().gap(px(5.)).mb(px(4.)).child(label(look, "WHAT ELSE IT AFFECTS")).child(div().text_size(px(11.)).text_color(c(look.secondary)).child("worked out by Humata")))
        .child(item("✓", TEAL, "5 running sessions and their terminals keep running", &["5 running sessions and their terminals keep running"]))
        .child(item("✓", TEAL, "Both machines migrate together; neither is left on the old boards", &[]))
        .child(item("!", AMBER_TEXT, "8 items on session boards move to their channel, with that session attached", &[]))
        .child(item("!", AMBER_TEXT, "The Board, Table and Dashboard views and the Scope picker go away", &[]))
        .child(item("!", AMBER_TEXT, "Agents’ board tools are replaced; running agents get the new ones on their next call", &[]))
        .child(div().mt(px(10.)).mb(px(4.)).child(label(look, "CAN IT BE UNDONE?")))
        .child(div().child("Yes, for 30 days: Settings › General › Restore boards brings back yesterday’s boards. Changes made after migrating are not carried back."));
    let request = div()
        .flex()
        .flex_col()
        .px(px(18.))
        .pt(px(16.))
        .pb(px(14.))
        .rounded(px(12.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(AMBER))
        .shadow(vec![BoxShadow { color: c(AMBER).into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false }])
        .text_size(px(13.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(status(St::NeedsYou))
                .child(div().text_size(px(15.5)).font_weight(FontWeight::BOLD).child("Move your boards into the one To-do list?"))
                .child(div().flex_1())
                .child(secondary(look, "Humata 0.4 · This Mac and devbox-1").text_size(px(12.5))),
        )
        .child(div().mt(px(10.)).flex().gap(px(28.)).child(left).child(right))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(14.))
                .pt(px(12.))
                .border_t_1()
                .border_color(crate::todo::rule(look))
                .child(crate::todo::btn(look, "Migrate", true))
                .child(crate::todo::btn(look, "What’s new in To-do", false))
                .child(div().flex_1())
                .child(secondary(look, "Humata can’t open To-do or the old boards until this has run").text_size(px(12.5))),
        );
    let top = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(60.))
        .px(px(28.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(crate::todo::rule(look))
        .child(div().text_size(px(17.)).font_weight(FontWeight::BOLD).child("Agent inbox"))
        .child(div().text_color(c(look.secondary)).child("1 waiting on you"))
        .child(div().flex_1())
        .child(link(look, "History"));
    let chips = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .pl(px(27.))
        .pr(px(28.))
        .pt(px(12.))
        .pb(px(4.))
        .flex_shrink_0()
        .child(crate::todo::filter_chip(look, "All", "1", true))
        .child(div().flex_1())
        .child(div().text_size(px(12.)).text_color(c(look.secondary)).child("An update needs you once. No agent is stopped."));
    wide_page(
        look,
        Side("Agent inbox", Some(1), false),
        top,
        div().flex().flex_col().child(chips).child(div().px(px(28.)).py(px(16.)).child(request)),
    )
}

// -------------------------------------------------------- Session screens

const WIDE_PANEL: f32 = 520.;

/// The rail beside a session in the To-do milestone: the Inbox is the Agent
/// inbox and the Board is To-do. `lit` is the open panel.
fn rail(look: Look, lit: Panel) -> Div {
    let item = |glyph: IconName, panel: Panel, text: &'static str| {
        let on = lit == panel;
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(5.))
            .child(icon(glyph, if on { look.accent } else { look.secondary }))
            .child(div().flex().flex_col().items_center().text_size(px(11.)).text_color(c(if on { look.accent } else { soft_hex(look) })).children(text.split(' ').map(|word| div().child(word.to_string()))))
    };
    div()
        .w(px(76.))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(19.))
        .pt(px(62.))
        .flex_shrink_0()
        .bg(c(look.content))
        .border_l_1()
        .border_color(crate::todo::rule(look))
        .child(item(IconName::Inbox, Panel::Inbox, "Agent inbox"))
        .child(item(IconName::FileDiff, Panel::Diff, "Diff"))
        .child(item(IconName::FolderTree, Panel::Files, "Files"))
        .child(item(IconName::SquareKanban, Panel::Board, "To-do"))
        .child(item(IconName::BookOpenText, Panel::Journal, "Journal"))
        .child(item(IconName::Info, Panel::Info, "Info"))
}

fn soft_hex(look: Look) -> u32 {
    if look.dark { 0xc4c4ca } else { 0x55555b }
}

/// A session screen: the sidebar, the middle, and a wide panel with `header`
/// over `body`, `lit` in the rail.
fn screen(look: Look, side: Side, middle: Div, lit: Panel, header: Div, body: Div) -> Div {
    div()
        .size_full()
        .flex()
        .line_height(relative(1.35))
        .child(side_col(look, side))
        .child(middle.flex_1().min_w_0().h_full().flex().flex_col().bg(c(look.content)))
        .child(
            div()
                .w(px(WIDE_PANEL))
                .flex_shrink_0()
                .h_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .bg(c(look.content))
                .border_l_1()
                .border_color(crate::todo::rule(look))
                .text_size(px(13.))
                .child(header)
                .child(body),
        )
        .child(rail(look, lit))
}

fn session_tab(look: Look, on: bool, glyph: AnyElement, text: &'static str) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(14.))
        .py(px(9.))
        .border_r_1()
        .border_color(crate::todo::rule(look))
        .when(on, |tab| tab.bg(c(look.content)).child(div().absolute().left_0().right_0().bottom_0().h(px(2.)).bg(c(look.accent))))
        .child(glyph)
        .child(text)
        .child(small_icon(IconName::X, look.secondary))
}

fn tab_bar(look: Look, extra_zsh: bool) -> Div {
    div()
        .flex()
        .h(px(36.))
        .flex_shrink_0()
        .bg(if look.dark { c(look.panel) } else { c(0xfafafb) })
        .border_b_1()
        .border_color(crate::todo::rule(look))
        .child(session_tab(look, true, status(St::Working).into_any_element(), "claude"))
        .when(extra_zsh, |bar| bar.child(session_tab(look, false, small_icon(IconName::SquareTerminal, look.secondary).into_any_element(), "zsh")))
        .child(div().flex().items_center().px(px(14.)).child(small_icon(IconName::Plus, look.secondary)))
}

/// The session header: channel crumb, name, agent and three icons.
fn session_header(look: Look, channel: &'static str, name: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(60.))
        .px(px(18.))
        .gap(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(crate::todo::rule(look))
        // Back leads the header, as in 68 (D848).
        .child(crate::kit::back_ghost(look))
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.)).text_color(c(look.accent)).child(format!("# {channel} ›")))
                .child(div().flex().items_center().gap(px(8.)).text_size(px(15.)).font_weight(FontWeight::BOLD).child(status(St::Working)).child(name)),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .text_color(c(look.secondary))
                .child("Claude Code · devbox-1")
                .child(small_icon(IconName::GitFork, look.secondary))
                .child(small_icon(IconName::Maximize2, look.secondary))
                .child(small_icon(IconName::Ellipsis, look.secondary)),
        )
}

fn term(look: Look, lines: Vec<Div>) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .px(px(18.))
                .pt(px(16.))
                .flex()
                .flex_col()
                .font_family(MONO)
                .text_size(px(12.5))
                .line_height(px(20.625))
                .text_color(c(look.term_text))
                .children(lines),
        )
        .child(
            div()
                .mx(px(18.))
                .mb(px(8.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .border_1()
                .border_color(tone_c(look, 0xcfcfd5, 0x48484c))
                .font_family(MONO)
                .text_size(px(12.5))
                .child("> ▌"),
        )
        .child(div().mx(px(18.)).mb(px(12.)).font_family(MONO).text_size(px(12.)).text_color(c(0x8a8a90)).child("▸▸ accept edits on (shift+tab to cycle)"))
}

fn tone_c(look: Look, light: u32, dark: u32) -> Rgba {
    c(if look.dark { dark } else { light })
}

/// A panel's header bar: `lead` and `note`, then the expand and close icons.
fn panel_bar(look: Look, lead: Div, note: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(44.))
        .px(px(14.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(crate::todo::rule(look))
        .child(lead)
        .child(secondary(look, note).text_size(px(13.)))
        .child(div().flex_1())
        .child(small_icon(IconName::Maximize2, look.secondary))
        .child(small_icon(IconName::X, look.secondary))
}

/// Frame 190: the review session a pull request review item starts.
pub fn pr_review_session(look: Look) -> Div {
    let header = session_header(look, "pr-reviews", "Ledger reads behind a flag");
    let dim = 0x8a8a90;
    let t = look.term_text;
    let lines = vec![
        tl(dim, "~/reviews/ledger-reads $ claude"),
        tl(dim, "Started from your to-do “Review: Ledger reads"),
        tl(dim, "behind a flag”. dana-k asked 2 days ago."),
        tl(t, ""),
        tl(t, "● This is a stack of 2 pull requests, 8 files:"),
        tl(t, "    1 api   acme/api#483  reads behind a flag"),
        tl(t, "    2 web   acme/web#210  totals from the ledger"),
        tl(t, ""),
        tl(t, "● I've laid out layer 1 in the Diff panel, the"),
        tl(t, "  flag and its fallback first, since that's what"),
        tl(t, "  dana-k asked about."),
        tl(t, ""),
        tl(t, "  I don't review. Ask me what anything does, or"),
        tl(t, "  to show something another way."),
        tl(t, ""),
        tl(dim, "The to-do closes itself when you submit."),
    ];
    let middle = div().child(header).child(tab_bar(look, false)).child(term(look, lines));
    let layer = |n: &'static str, name: &'static str, sub: &'static str, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(9.))
            .mx(px(6.))
            .px(px(14.))
            .py(px(7.))
            .rounded(px(8.))
            .when(on, |row| row.bg(c(look.fill)))
            .child(
                div()
                    .size(px(18.))
                    .flex_shrink_0()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .when(on, |dot| dot.bg(c(look.accent)).text_color(c(0xffffff)))
                    .when(!on, |dot| dot.border_1().border_color(c(look.secondary)))
                    .child(n),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(secondary(look, sub).text_size(px(13.)))
    };
    let file = |name: &'static str, dir: &'static str, add: &'static str, del: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(38.))
            .pr(px(14.))
            .py(px(5.))
            .child(div().size(px(13.)).flex_shrink_0().rounded_full().border_1().border_color(c(look.tertiary)))
            .child(div().child(name))
            .child(secondary(look, dir).text_size(px(13.)))
            .child(div().flex_1())
            .child(div().text_color(c(ADD_GREEN)).child(add))
            .child(div().text_color(c(DEL_RED)).child(del))
    };
    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(div().px(px(14.)).pt(px(12.)).pb(px(6.)).child(label(look, "STACK · 2 LAYERS")))
        .child(layer("1", "api", "acme/api#483 · 6 files", true))
        .child(layer("2", "web", "acme/web#210 · 2 files", false))
        .child(
            div()
                .mt(px(10.))
                .pt(px(8.))
                .border_t_1()
                .border_color(crate::todo::rule(look))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .px(px(14.))
                        .py(px(5.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("api"))
                        .child(secondary(look, "against main").text_size(px(13.)))
                        .child(div().flex_1())
                        .child(div().text_color(c(ADD_GREEN)).child("+205"))
                        .child(div().text_color(c(DEL_RED)).child("−45")),
                )
                .child(file("flags.rs", "src/", "+18", "−0"))
                .child(file("reads.rs", "src/ledger/", "+81", "−21"))
                .child(file("client.rs", "src/", "+62", "−21"))
                .child(file("entry.rs", "src/", "+30", "−3"))
                .child(file("reads_test.rs", "tests/", "+14", "−0"))
                .child(file("CHANGELOG.md", "", "+2", "−0")),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .py(px(12.))
                .border_t_1()
                .border_color(crate::todo::rule(look))
                .child(secondary(look, "No comments yet").text_size(px(13.)))
                .child(div().flex_1())
                .child(crate::todo::btn(look, "Submit review…", true)),
        );
    let bar = panel_bar(look, div().text_size(px(15.)).font_weight(FontWeight::BOLD).child("Diff"), "2 pull requests · 8 files");
    screen(look, Side("Ledger reads behind a flag", None, false), middle, Panel::Diff, bar, body)
}

/// Frame 191: a Needs attention to-do opened in the session's To-do panel,
/// with the agent's ask beside the terminal that asked it.
pub fn session_todo_ask(look: Look) -> Div {
    let header = session_header(look, "payments-rewrite", "Ledger entries");
    let dim = 0x8a8a90;
    let t = look.term_text;
    let lines = vec![
        tl(dim, "~/src/ledger (ledger-schema) $ claude"),
        tl(t, "> Design the ledger schema from PAY-3 and move the reads over."),
        tl(t, ""),
        tl(t, "● Bash(cargo test -p api ledger)"),
        tl(0x0f8a3c, "  ⎿ test result: ok. 212 passed; 0 failed"),
        tl(t, "● Bash(psql -f backfill.sql)"),
        tl(dim, "  ⎿ INSERT 0 3104882 · 4m12s"),
        tl(t, "● humata todo close PAY-3"),
        tl(t, "    --reason \"Migration and backfill pass on staging;"),
        tl(t, "              steps 1 to 4 are done.\""),
        tl(t, "    --evidence 3 commits, acme/ledger#92, 212 tests"),
        tl(dim, "  ⎿ Asked the owner. Not waiting; carrying on."),
        tl(t, ""),
        tl(t, "● Writing the cut-over runbook while that waits."),
        tl(0xb86a00, "✱ Writing docs/cutover.md… (40s · esc to interrupt)"),
    ];
    let middle = div().child(header).child(tab_bar(look, true)).child(term(look, lines));
    let body = drawer_body(look, false);
    let bar = panel_bar(look, div().flex().gap(px(6.)).text_size(px(13.)).text_color(c(look.accent)).child("‹ To-do"), "PAY-3 · payments-rewrite");
    screen(look, Side("Ledger entries", None, false), middle, Panel::Board, bar, body)
}

/// The open drawer of PAY-3 in the session's To-do panel (frame 191): the
/// agent's ask, the fields, the steps and the foot.
/// `narrow`: at side-panel width Effort and Impact take a row each, as
/// the pair does not fit on one.
pub(crate) fn drawer_body(look: Look, narrow: bool) -> Div {
    let fact = |key: &'static str, value: Div| {
        div().flex().child(div().w(px(74.)).flex_shrink_0().text_color(c(look.secondary)).child(key)).child(value.flex_1().min_w_0())
    };
    let ask = div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .px(px(14.))
        .pt(px(10.5))
        .pb(px(12.))
        .rounded(px(10.))
        .bg(c(look.attention_bg))
        .child(div().mb(px(-2.)).text_size(px(14.5)).font_weight(FontWeight::SEMIBOLD).child("Close Ledger schema v2?"))
        .child(div().text_color(c(if look.dark { 0xe8d6b0 } else { 0x3a2c12 })).text_size(px(13.)).child("Migration and backfill pass on staging: 3.1M rows in 4m12s. Steps 1 to 4 are done; step 5, the production cut-over, is yours."))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .px(px(11.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(c(look.card))
                .text_size(px(13.))
                .child(fact("Evidence", div().flex().flex_wrap().gap(px(4.)).child(link(look, "3 commits ·")).child(link(look, "acme/ledger#92 ·")).child(link(look, "212 tests passed"))))
                .child(fact("Change", div().child(rich("State: In progress → Closed", &["Closed"]))))
                .child(fact("Also", div().child("WEB-13 stops waiting · this session keeps running")))
                .child(fact("Asked by", div().child("claude in this session · 1m ago"))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .mt(px(1.))
                .gap(px(8.))
                .child(crate::todo::btn(look, "Yes", true).min_w(px(64.)).flex().justify_center())
                .child(crate::todo::btn(look, "No", false).min_w(px(64.)).flex().justify_center())
                .child(div().flex_1())
                .child(crate::todo::btn(look, "Ask why", false)),
        );
    let prop = |key: &'static str, value: Div| div().flex().items_center().child(div().w(px(96.)).flex_shrink_0().text_color(c(look.secondary)).child(key)).child(value);
    let step = |done: bool, text: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .size(px(14.))
                    .flex_shrink_0()
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(done, |b| b.bg(c(TEAL)).child(small_icon(IconName::Check, 0xffffff).size(px(10.))))
                    .when(!done, |b| b.border_1().border_color(c(look.tertiary)).bg(c(look.card))),
            )
            .child(div().text_color(c(if done { look.secondary } else { look.text })).child(text))
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .px(px(16.))
        .pt(px(13.))
        .pb(px(18.))
        .child(div().mb(px(18.)).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Ledger schema v2"))
        .child(ask.mb(px(3.)))
        .child(
            div()
                .flex()
                .flex_col()
                .child(prop("State", div().flex().child(crate::todo::segs(look, &["Open", "In progress", "Closed"], "In progress"))).mb(px(3.)))
                .child(prop("Due", div().flex().items_center().gap(px(10.)).child(field(look, "Fri 20 Nov", false).w(px(150.)).h(px(28.))).child(secondary(look, "tomorrow"))).mb(px(4.)))
                .when(!narrow, |props| {
                    props.child(prop(
                        "Effort",
                        div()
                            .flex()
                            .items_center()
                            .child(crate::todo::segs(look, &["S", "M", "L"], "M"))
                            .child(secondary(look, "Impact").text_size(px(13.)).ml(px(24.)).mr(px(14.)))
                            .child(crate::todo::segs(look, &["Low", "Medium", "High"], "High")),
                    ).mb(px(8.)))
                })
                .when(narrow, |props| {
                    props
                        .child(prop("Effort", div().flex().child(crate::todo::segs(look, &["S", "M", "L"], "M"))).mb(px(4.)))
                        .child(prop("Impact", div().flex().child(crate::todo::segs(look, &["Low", "Medium", "High"], "High"))).mb(px(8.)))
                })
                .child(prop("Tags", div().flex().items_center().gap(px(4.)).child(tag(look, "migration")).child(tag(look, "db")).child(link(look, "+ Add").ml(px(5.))))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .pt(px(20.))
                .child(div().flex().items_center().mb(px(1.)).child(label(look, "STEPS · 4 OF 5")).child(div().flex_1()).child(secondary(look, "ticked by claude")))
                .child(step(true, "Add ledger_entries and ledger_balances"))
                .child(step(true, "Dual-write from the posting path"))
                .child(step(true, "Backfill the old table in batches"))
                .child(step(true, "Verify the balances match on staging"))
                .child(step(false, "Cut production reads over"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_color(c(look.secondary))
                        .child(div().size(px(14.)).flex_shrink_0().rounded(px(4.)).border_1().border_dashed().border_color(c(0xb9b9c0)))
                        .child("Add a step…"),
                ),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(21.5))
                .pt(px(12.))
                .border_t_1()
                .border_color(crate::todo::rule(look))
                .text_color(c(look.secondary))
                .child("History")
                .child(div().flex().items_center().gap(px(4.)).child(small_icon(IconName::Pin, look.secondary)).child("Pin"))
                .child("Hide until…"),
        )
}

/// Frame 12: PAY-3 open in the session's To-do panel at full screen, drawn
/// as the app's panel draws it (`ui/workspace/src/todo_panel.rs` and the
/// narrow drawer in `todo.rs`, approved by Fable in round 4, D872): the way
/// back leads the scrolling body, the fields sit a row each, and the
/// History · Pin · Hide until… row is pinned under a divider.
pub(crate) fn panel_drawer_full(look: Look) -> Div {
    let row = || div().flex().flex_row().items_center();
    let col = || div().flex().flex_col();
    let small = |text: &'static str| div().text_color(c(look.secondary)).text_size(px(12.)).child(text);
    let link = |text: &'static str| div().flex_shrink_0().whitespace_nowrap().text_color(c(look.accent)).child(text);
    let field = |label: &'static str, body: Div| row().gap(px(12.)).py(px(4.)).child(small(label).w(px(96.)).flex_shrink_0()).child(body.min_w_0());
    // TODO2: the panel's segments wrap and take the narrow 7 pt padding.
    let seg = |options: &[&'static str], on: &str| {
        row().flex_wrap().min_w_0().p(px(2.)).rounded(px(7.)).bg(c(look.fill)).children(options.iter().map(|name| {
            div()
                .px(px(7.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(12.))
                .when(*name == on, |d| d.bg(c(look.card)).font_weight(FontWeight::SEMIBOLD))
                .child(*name)
        }))
    };
    let pill = |text: &'static str| {
        div()
            .flex_shrink_0()
            .whitespace_nowrap()
            .px(px(7.))
            .py(px(1.))
            .rounded(px(5.))
            .text_size(px(12.))
            .bg(c(look.fill))
            .text_color(tone_c(look, 0x55555b, 0xc4c4ca))
            .child(text)
    };
    let teal = tone_c(look, 0x0f766e, 0x35c4b5);
    let step = |done: bool, text: &'static str| {
        let boxed = div()
            .size(px(15.))
            .rounded(px(4.))
            .border_1()
            .border_color(if done { teal } else { tone_c(look, 0xb9b9c0, 0x55555a) })
            .when(done, |b| b.bg(teal).text_color(c(0xffffff)).text_size(px(10.)).child("✓"));
        row().gap(px(9.)).py(px(3.)).child(boxed).child(div().when(done, |d| d.text_color(c(look.secondary))).child(text))
    };
    let fact = |label: &'static str, body: Div| row().items_start().child(small(label).w(px(74.)).flex_shrink_0()).child(div().flex_1().min_w_0().child(body));
    let dot = || div().text_color(c(look.secondary)).child("·");
    let ask = col()
        .mt(px(6.))
        .p(px(12.))
        .gap(px(8.))
        .rounded(px(10.))
        .bg(tone_c(look, 0xfff4e0, 0x3b2b10))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Close Ledger schema v2?"))
        .child(
            div()
                .text_color(tone_c(look, 0x5c4a2a, 0xe0cfa8))
                .child("Migration and backfill pass on staging: 3.1M rows in 4m12s. Steps 1 to 4 are done; step 5, the production cut-over, is yours."),
        )
        .child(
            col()
                .p(px(8.))
                .rounded(px(8.))
                .bg(c(look.card))
                .child(fact(
                    "Evidence",
                    row()
                        .flex_wrap()
                        .gap(px(4.))
                        .child(link("3 commits"))
                        .child(dot())
                        .child(link("acme/ledger#92"))
                        .child(dot())
                        .child(link("212 tests passed")),
                ))
                .child(fact("Change", div().child("State: In progress → Closed")))
                .child(fact("Also", div().child("WEB-13 stops waiting · this session keeps running")))
                .child(fact("Asked by", div().child("claude in this session · 1m ago"))),
        )
        .child(
            row()
                .gap(px(8.))
                .child(button(look, Kind::Primary, "Yes"))
                .child(button(look, Kind::Secondary, "No"))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Ask why"))
                .child(button(look, Kind::Secondary, "Open session")),
        );
    // TODO2 (T1): one wrapping row for the exact date, then the loose dates,
    // which are never red. "Next week" is the loose chip, not a link.
    let loose = |text: &'static str| {
        div()
            .px(px(7.))
            .py(px(1.))
            .rounded(px(5.))
            .text_size(px(11.5))
            .bg(c(look.fill))
            .text_color(tone_c(look, 0x55555b, 0xc4c4ca))
            .child(text)
    };
    let due = col()
        .gap(px(4.))
        .child(
            row()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    row()
                        .gap(px(10.))
                        .child(
                            row()
                                .w(px(120.))
                                .flex_shrink(1.)
                                .min_w_0()
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(c(look.line))
                                .bg(c(look.card))
                                .child(div().flex_1().min_w_0().child("Fri 20 Nov"))
                                .child(div().text_color(c(look.secondary)).child("×")),
                        )
                        .child(small("tomorrow")),
                )
                .child(row().gap(px(8.)).flex_none().child(link("Today")).child(link("Tomorrow"))),
        )
        .child(
            row()
                .gap(px(6.))
                .flex_wrap()
                .child(small("Loose:"))
                .child(loose("In a few days"))
                .child(loose("Next week"))
                .child(loose("Next month")),
        );
    // TODO2 (T4): an unset field sits in a faint dashed box.
    let unset = |label: &'static str, body: Div| {
        row()
            .gap(px(12.))
            .py(px(4.))
            .px(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_dashed()
            .border_color(tone_c(look, 0xe2e2e8, 0x35353a))
            .child(small(label).w(px(96.)).flex_shrink_0())
            .child(body.min_w_0())
    };
    let drawer = col()
        .gap(px(6.))
        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child("Ledger schema v2"))
        .child(ask)
        .child(field("State", row().gap(px(6.)).flex_wrap().child(seg(&["Open", "In progress", "Closed", "Won't do"], "In progress"))))
        .child(unset("Need", seg(&["Idea", "Nice to have", "Expected", "Critical"], "")))
        .child(field("Due", due))
        .child(field("Effort", row().gap(px(14.)).flex_wrap().child(seg(&["S", "M", "L"], "M"))))
        .child(field("Impact", seg(&["Low", "Medium", "High"], "High")))
        .child(field("Tags", row().flex_wrap().gap(px(6.)).child(pill("migration")).child(pill("db")).child(link("+ Add"))))
        .child(
            row()
                .mt(px(10.))
                .child(small("STEPS · 4 OF 5").font_weight(FontWeight::SEMIBOLD))
                .child(div().flex_1())
                .child(small("ticked by claude")),
        )
        .child(step(true, "Add ledger_entries and ledger_balances"))
        .child(step(true, "Dual-write from the posting path"))
        .child(step(true, "Backfill the old table in batches"))
        .child(step(true, "Verify the balances match on staging"))
        .child(step(false, "Cut production reads over"))
        .child(
            row()
                .gap(px(9.))
                .py(px(3.))
                .text_color(c(look.secondary))
                .child(div().size(px(15.)).flex_shrink_0().rounded(px(4.)).border_1().border_dashed().border_color(tone_c(look, 0xb9b9c0, 0x55555a)))
                .child("Add a step…"),
        );
    let back = row()
        .gap(px(8.))
        .min_w_0()
        .child(div().flex_shrink_0().text_color(c(look.accent)).child("‹ To-do"))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(12.))
                .text_color(c(look.secondary))
                .child("PAY-3 · payments-rewrite"),
        );
    col()
        .size_full()
        .child(col().flex_1().min_h_0().overflow_hidden().p(px(12.)).child(col().gap(px(8.)).child(back).child(drawer)))
        .child(
            row()
                .gap(px(8.))
                .border_t_1()
                .border_color(c(look.line))
                .flex_shrink_0()
                .px(px(12.))
                .py(px(10.))
                .child(link("History"))
                .child(div().flex_1())
                .child(link("Pin"))
                .child(link("Hide until…")),
        )
}
