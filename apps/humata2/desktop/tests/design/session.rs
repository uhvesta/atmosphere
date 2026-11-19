//! The session screen (SE-6 to SE-20): a header, the terminal with its
//! tabs, and the panel on the right, in each state the session can be in.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::sidebar::{self, Side};

/// Which panel is open on the right.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Diff,
    Inbox,
    Files,
    Board,
    Journal,
    Info,
}

/// What the terminal shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Term {
    Working,
    NeedsYou,
    Exited,
    Offline,
    /// After a restart, the agent's resume failed (PER-15).
    CouldntResume,
}

/// What fills the window.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Columns,
    /// The sidebar and the panel collapsed to strips.
    Collapsed,
    TerminalFull,
    PanelFull,
}

#[derive(Clone, Copy)]
pub struct Screen {
    pub panel: Option<Panel>,
    pub term: Term,
    pub fill: Fill,
    pub side: Side,
}

impl Screen {
    pub const fn new(panel: Option<Panel>, term: Term) -> Self {
        Self {
            panel,
            term,
            fill: Fill::Columns,
            side: Side::at("Ledger schema migration"),
        }
    }
}

pub const PANEL_WIDTH: f32 = 340.;

fn header(look: Look, screen: Screen) -> Div {
    let full = screen.fill == Fill::TerminalFull;
    let (dot, words) = match screen.term {
        Term::Working => (St::Working, None),
        Term::NeedsYou => (St::NeedsYou, Some(("Needs you", ORANGE))),
        Term::Exited => (St::Exited, Some(("Agent exited", RED))),
        Term::Offline => (St::Offline, Some(("devbox-1 is offline", GREY))),
        Term::CouldntResume => (St::Exited, Some(("Couldn't resume", RED))),
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(52.))
        .px(px(16.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .bg(c(look.content))
        .when(full || screen.fill == Fill::Collapsed, |bar| bar.child(traffic_lights()).child(div().w(px(4.))))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_size(px(11.5))
                        .text_color(c(look.secondary))
                        .child(div().text_color(c(look.accent)).child("# payments-rewrite"))
                        .child("›"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(status(dot))
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child(screen.side.selected))
                        .children(words.map(|(words, hex)| div().text_size(px(11.5)).text_color(c(hex)).child(words))),
                ),
        )
        .child(div().flex_1())
        .child(secondary(look, "Claude Code · devbox-1"))
        .when(screen.term == Term::Exited, |bar| {
            bar.child(button(look, Kind::Primary, "Resume agent").child(div()))
        })
        .child(icon(IconName::GitFork, look.secondary))
        .child(icon(if full { IconName::Minimize2 } else { IconName::Maximize2 }, look.secondary))
        .child(icon(IconName::PanelRight, if screen.panel.is_some() { look.accent } else { look.secondary }))
        .child(icon(IconName::Ellipsis, look.secondary))
}

pub fn tab(look: Look, lead: Option<AnyElement>, text: &'static str, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h_full()
        .px(px(12.))
        .border_r_1()
        .border_color(c(look.line))
        .text_size(px(12.))
        .when(on, |tab| tab.bg(c(look.term)).text_color(c(look.text)))
        .when(!on, |tab| tab.text_color(c(look.secondary)))
        .children(lead)
        .child(text)
        .child(small_icon(IconName::X, look.tertiary))
}

fn tabs(look: Look, screen: Screen) -> Div {
    let dot = match screen.term {
        Term::Working => St::Working,
        Term::NeedsYou => St::NeedsYou,
        Term::Exited | Term::CouldntResume => St::Exited,
        Term::Offline => St::Offline,
    };
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(tab(look, Some(status(dot).into_any_element()), "claude", true))
        .child(tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false))
        .child(tab(look, Some(small_icon(IconName::FileCode, look.secondary).into_any_element()), "client.rs", false))
        .child(div().px(px(10.)).child(small_icon(IconName::Plus, look.secondary)))
        .child(div().flex_1())
        .when(screen.term == Term::Offline, |bar| {
            bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .mr(px(12.))
                    .text_size(px(11.5))
                    .text_color(c(look.secondary))
                    .child(small_icon(IconName::WifiOff, look.secondary))
                    .child("disconnected"),
            )
        })
        .when(screen.term == Term::Working, |bar| {
            bar.child(div().mr(px(12.)).child(secondary(look, "1 other can type here")))
        })
}

/// A line Humata writes into the terminal, set apart from the program's
/// own output.
pub fn notice(look: Look, glyph: IconName, hex: u32, text: &'static str, action: Option<&'static str>) -> Div {
    div()
        .mt(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(7.))
        .bg(c(look.fill))
        .font_family(SYSTEM_FONT)
        .text_size(px(12.))
        .text_color(c(look.text))
        .child(small_icon(glyph, hex))
        .child(div().flex_1().child(text))
        .children(action.map(|action| button(look, Kind::Secondary, action)))
}

pub fn transcript(look: Look) -> Vec<Div> {
    vec![
        tl(look.term_dim, "~/src/ledger (ledger-schema) $ claude"),
        tl(look.term_text, "> Design the ledger schema from PAY-3 and move the reads over."),
        tl(look.term_text, ""),
        tl(look.term_text, "● Read(ledger/docs/model.md)"),
        tl(look.term_dim, "  ⎿ Read 212 lines"),
        tl(look.term_text, "● Write(ledger/migrations/0042_accounts.sql)"),
        tl(look.term_dim, "  ⎿ Wrote 41 lines"),
        tl(look.term_text, "● Update(api/src/client.rs)"),
        tl(look.term_dim, "  ⎿ Updated with 62 additions and 21 removals"),
        tl(look.term_text, "● Bash(cargo test -p api ledger)"),
        tl(GREEN, "  ⎿ test result: ok. 14 passed; 0 failed"),
        tl(look.term_text, ""),
        tl(look.term_text, "● Updated plan"),
        tl(GREEN, "  ✓ Add accounts and entries tables"),
        tl(GREEN, "  ✓ Dual-write from client.rs"),
        tl(ORANGE, "  ◐ Backfill entries from ledger_entries"),
        tl(look.term_dim, "    Switch reads, drop the old index"),
        tl(look.term_text, ""),
    ]
}

pub fn input_box(look: Look, enabled: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .px(px(8.))
                .py(px(4.))
                .border_1()
                .border_color(c(look.term_dim))
                .rounded(px(4.))
                .flex()
                .gap(px(2.))
                .child(tl(if enabled { look.term_text } else { look.term_dim }, "> "))
                .when(enabled, |input| input.child(div().w(px(7.)).h(px(15.)).bg(c(look.term_text)))),
        )
        .child(tl(look.term_dim, "  ⏵⏵ accept edits on (shift+tab to cycle)"))
}

pub fn terminal(look: Look, term: Term) -> Div {
    let body = div()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .px(px(14.))
        .pt(px(10.))
        .pb(px(10.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .bg(c(look.term))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(c(look.term_text))
        .children(transcript(look));
    match term {
        Term::Working => body
            .child(tl(look.term_text, "● Bash(psql -f backfill.sql)"))
            .child(tl(look.term_dim, "  ⎿ INSERT 0 48210 … (running)"))
            .child(tl(look.term_text, ""))
            .child(tl(ORANGE, "✻ Backfilling entries… (2m 14s · ↓ 3.1k tokens · esc to interrupt)"))
            .child(div().flex_1())
            .child(input_box(look, true)),
        Term::NeedsYou => body
            .child(tl(look.term_text, "● humata board_item_move PAY-3 → Done"))
            .child(tl(ORANGE, "  ⎿ ⏳ waiting for your approval (Inbox)"))
            .child(tl(look.term_text, ""))
            .child(tl(ORANGE, "? Drop the old ledger_entries index before the switch, or keep"))
            .child(tl(ORANGE, "  both until the backfill ends?"))
            .child(tl(look.term_text, "  ❯ 1. Keep both"))
            .child(tl(look.term_dim, "    2. Drop it now"))
            .child(tl(look.term_dim, "    3. Type something else"))
            .child(div().flex_1())
            .child(
                div()
                    .font_family(SYSTEM_FONT)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(7.))
                    .bg(c(look.fill))
                    .text_size(px(12.))
                    .child(small_icon(IconName::MessageSquareText, look.secondary))
                    .child(div().flex_1().child("2 messages waiting to send"))
                    .child(div().text_color(c(look.accent)).child("Show")),
            ),
        Term::Exited => body
            .child(tl(look.term_text, "● All four steps are done. Reads go to entries now."))
            .child(tl(look.term_text, ""))
            .child(tl(look.term_dim, "~/src/ledger (ledger-schema) $ "))
            .child(notice(
                look,
                IconName::CircleX,
                RED,
                "claude exited with code 0. Resume starts it again with this conversation.",
                Some("Resume"),
            ))
            .child(div().flex_1()),
        Term::Offline => body
            .child(tl(look.term_text, "● Bash(psql -f backfill.sql)"))
            .child(tl(look.term_dim, "  ⎿ INSERT 0 48210 … (running)"))
            .child(notice(
                look,
                IconName::WifiOff,
                GREY,
                "Lost the connection to devbox-1 at 14:02. The agent keeps running there; this view catches up when it is back.",
                None,
            ))
            .child(div().flex_1())
            .child(input_box(look, false).opacity(0.5)),
        Term::CouldntResume => body.child(reboot_line(look)).child(couldnt_resume(look)).child(div().flex_1()),
    }
}

/// The line under the old scrollback that marks the reboot and how the
/// agent was started again (PER-17).
fn reboot_line(look: Look) -> Div {
    div()
        .mt(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .font_family(SYSTEM_FONT)
        .text_size(px(11.5))
        .text_color(c(look.secondary))
        .child(div().flex_1().h(px(1.)).bg(c(look.line)))
        .child("devbox-1 restarted at 09:12 · ran claude --resume 7f3a…")
        .child(div().flex_1().h(px(1.)).bg(c(look.line)))
}

/// A failed resume is never a silent fresh start (PER-15): the reason, the
/// two ways on, and the line to this restart's report (PER-29).
fn couldnt_resume(look: Look) -> Div {
    div()
        .mt(px(10.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(7.))
        .bg(c(look.fill))
        .font_family(SYSTEM_FONT)
        .text_size(px(12.))
        .text_color(c(look.text))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(small_icon(IconName::CircleX, RED))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Couldn't resume")),
        )
        .child(div().child(
            "claude started but opened a new conversation: 7f3a….jsonl is missing from ~/.claude/projects/ledger on devbox-1. Nothing was typed into it.",
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(2.))
                .child(button(look, Kind::Primary, "Try again"))
                .child(button(look, Kind::Secondary, "Start fresh with a summary"))
                .child(div().flex_1())
                .child(small_icon(IconName::FileText, look.secondary))
                .child(div().text_color(c(look.accent)).child("Restart report 09:12")),
        )
}

pub fn panel_header(look: Look, title: &'static str, full: bool, extra: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(36.))
        .px(px(14.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .children(extra)
        .child(div().flex_1())
        .child(small_icon(if full { IconName::Minimize2 } else { IconName::Maximize2 }, look.secondary))
        .child(small_icon(IconName::X, look.secondary))
}

/// The labelled strip down the right edge that picks the panel (SE-11).
pub fn strip(look: Look, on: Option<Panel>) -> Div {
    rail(look, on, Some("1"), Some("2"))
}

/// The rail beside a desk session: its Inbox counts what that session
/// asks, and a desk has no branch, so Diff counts nothing.
pub fn desk_strip(look: Look, on: Option<Panel>, inbox: Option<&'static str>) -> Div {
    rail(look, on, inbox, None)
}

/// The panel rail of the Ledger schema migration session: its Inbox count is
/// its own pending request plus its fork Try BRIN's (the Inbox panel lists both).
/// The rail beside Ledger schema migration: its Inbox counts its own
/// request only while it asks (`Side::asking`), plus its fork's.
/// Another selected session (frame 16's fork) has nothing waiting.
fn ledger_strip(look: Look, on: Option<Panel>, side: Side) -> Div {
    let inbox = (side.selected == "Ledger schema migration").then_some(if side.asking { "2" } else { "1" });
    rail(look, on, inbox, Some("2"))
}

/// `screen`'s sidebar: in a working terminal the session's own request
/// isn't waiting, so its row is working too (D113 item 9); once its agent
/// has exited, its row carries the exited mark its header does (CH-16).
fn side(screen: Screen) -> Side {
    Side { asking: screen.term != Term::Working, exited: screen.term == Term::Exited, ..screen.side }
}

/// The rail itself, with the counts its Inbox and Diff entries carry.
pub fn rail(look: Look, on: Option<Panel>, inbox: Option<&'static str>, diff: Option<&'static str>) -> Div {
    let item = |glyph: IconName, panel: Panel, text: &'static str, count: Option<&'static str>| {
        let lit = on == Some(panel);
        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .w(px(52.))
            .py(px(7.))
            .rounded(px(7.))
            .when(lit, |item| item.bg(c(look.fill)))
            .child(icon(glyph, if lit { look.accent } else { look.secondary }))
            .child(div().text_size(px(10.)).text_color(c(if lit { look.text } else { look.secondary })).child(text))
            .children(count.map(|count| div().absolute().top(px(2.)).right(px(6.)).child(badge(look, count, panel == Panel::Inbox))))
    };
    div()
        .w(px(64.))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .pt(px(60.))
        .flex_shrink_0()
        .bg(c(look.sidebar))
        .border_l_1()
        .border_color(c(look.line))
        .child(item(IconName::Inbox, Panel::Inbox, "Inbox", inbox))
        .child(item(IconName::FileDiff, Panel::Diff, "Diff", diff))
        .child(item(IconName::FolderTree, Panel::Files, "Files", None))
        .child(item(IconName::SquareKanban, Panel::Board, "Board", None))
        .child(item(IconName::BookOpenText, Panel::Journal, "Journal", None))
        .child(item(IconName::Info, Panel::Info, "Info", None))
}

/// One changed file as the Diff panel lists it: a 14 px status column (empty
/// for a modified file), the name, then its directory dimmed, the comment
/// count, and +a −r. `path` is the directory, then the name.
fn diff_file(
    look: Look,
    path: (&'static str, &'static str),
    mark: Option<&'static str>,
    add: &'static str,
    del: &'static str,
    on: bool,
    comments: Option<&'static str>,
) -> Div {
    let (dir, name) = path;
    let letter = mark.map(|mark| (mark, if mark == "A" { GREEN } else if mark == "D" { RED } else { look.secondary }));
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(25.))
        .pl(px(22.))
        .pr(px(8.))
        .rounded(px(5.))
        .when(on, |row| row.bg(c(look.fill)))
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .children(letter.map(|(letter, hex)| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(hex)).child(letter))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .child(div().flex_shrink_0().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(c(look.tertiary)).child(dir)),
        )
        .children(comments.map(|count| {
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(px(3.))
                .text_size(px(11.))
                .text_color(c(look.secondary))
                .child(small_icon(IconName::MessageSquare, look.secondary))
                .child(count)
        }))
        .child(div().flex_shrink_0().text_size(px(11.)).text_color(c(GREEN)).child(add))
        .child(div().flex_shrink_0().text_size(px(11.)).text_color(c(RED)).child(del))
}

/// A repo's one row: the fold chevron, its name, the base it compares
/// against in words (UXSIMPLE 1-2), a spacer and the repo's totals.
fn repo(look: Look, name: &'static str, add: &'static str, del: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .pr(px(8.))
        .whitespace_nowrap()
        .overflow_hidden()
        .child(div().w(px(20.)).h(px(20.)).flex_shrink_0().flex().items_center().justify_center().child(small_icon(IconName::ChevronDown, look.tertiary)))
        .child(div().font_weight(FontWeight::SEMIBOLD).min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
        .child(div().flex_shrink(4.).min_w_0().overflow_hidden().text_ellipsis().text_size(px(11.5)).text_color(c(look.secondary)).child("against main"))
        .child(div().flex_1())
        .child(div().flex_shrink_0().whitespace_nowrap().text_size(px(11.)).text_color(c(GREEN)).child(add))
        .child(div().flex_shrink_0().whitespace_nowrap().text_size(px(11.)).text_color(c(RED)).child(del))
}

/// The header's base chip: "against" and the picker, as the narrow header
/// carries it and the full-screen title bar carries it beside "Diff".
pub fn base_chip(look: Look) -> Div {
    div().flex().items_center().gap(px(6.)).child(secondary(look, "against")).child(popup(look, "main"))
}

fn diff_lines(look: Look) -> Div {
    let line = |n: &'static str, bg: Option<u32>, text: &'static str| {
        div()
            .flex()
            .when_some(bg, |row, bg| row.bg(c(bg)))
            .child(div().w(px(34.)).pr(px(8.)).text_color(c(look.term_dim)).flex().justify_end().child(n))
            .child(div().whitespace_nowrap().text_color(c(look.term_text)).child(text))
    };
    div()
        .flex()
        .flex_col()
        .py(px(6.))
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(px(17.))
        .bg(c(look.term))
        .child(div().px(px(10.)).text_color(c(look.term_dim)).child("impl Client {"))
        .child(line("40", None, "    pub async fn post(&self, entry: Entry)"))
        .child(line("41", Some(look.del_bg), "-       self.db.insert_legacy(&entry).await?;"))
        .child(line("41", Some(look.add_bg), "+       let tx = self.db.begin().await?;"))
        .child(line("42", Some(look.add_bg), "+       tx.insert_entry(&entry).await?;"))
        .child(line("43", Some(look.add_bg), "+       tx.insert_legacy(&entry).await?;"))
        .child(line("44", Some(look.add_bg), "+       tx.commit().await?;"))
}

pub fn comment(look: Look, who: &'static str, text: &'static str, draft: bool) -> Div {
    comment_card(look, who, text, draft, false, draft.then_some("Draft"))
}

/// A sent comment: the grey line, and the Draft card's header row with a
/// "Sent" chip (D851).
pub fn sent_comment(look: Look, who: &'static str, text: &'static str) -> Div {
    comment_card(look, who, text, false, false, Some("Sent"))
}

/// D126: a saved draft in the Diff panel can be sent on its own.
fn draft_with_send(look: Look, who: &'static str, text: &'static str) -> Div {
    comment_card(look, who, text, true, true, Some("Draft"))
}

fn comment_card(look: Look, who: &'static str, text: &'static str, draft: bool, send: bool, label: Option<&'static str>) -> Div {
    div()
        .max_w(px(720.))
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .rounded(px(8.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(if draft { look.accent } else { look.line }))
        .flex()
        .flex_col()
        .gap(px(4.))
        .font_family(SYSTEM_FONT)
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(who))
                .children(label.map(|label| chip(look, label, look.accent)))
                .when(send, |row| {
                    row.child(div().flex_1())
                        .child(div().px(px(6.)).text_size(px(11.)).text_color(c(look.accent)).child("Send"))
                }),
        )
        .child(text)
}

fn diff_panel(look: Look, full: bool) -> Div {
    let list = div()
        .flex()
        .flex_col()
        .p(px(8.))
        .gap(px(1.))
        .text_size(px(12.5))
        .when(full, |list| list.w(px(300.)).h_full().border_r_1().border_color(c(look.line)))
        .child(repo(look, "ledger", "+41", "−0"))
        .child(diff_file(look, ("migrations/", "0042_accounts.sql"), Some("A"), "+41", "−0", false, None))
        .child(repo(look, "api", "+143", "−24"))
        .child(diff_file(look, ("src/", "client.rs"), None, "+62", "−21", true, Some("2")))
        .child(diff_file(look, ("src/", "entry.rs"), None, "+81", "−3", false, None));
    let view = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .h(px(30.))
                .border_y_1()
                .border_color(c(look.line))
                .text_size(px(12.))
                .child(small_icon(IconName::FileCode, look.secondary))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .gap(px(4.))
                        .child(secondary(look, "api ·").flex_shrink_0())
                        .child(secondary(look, "src/").min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis())
                        .child(div().flex_shrink_0().font_weight(FontWeight::MEDIUM).child("client.rs")),
                )
                .child(div().w(px(20.)).h(px(20.)).flex_shrink_0().flex().items_center().justify_center().child(small_icon(IconName::ArrowUpRight, look.secondary))),
        )
        .child(diff_lines(look))
        .child(comment(look, "You", "Wrap the two writes in one transaction so a crash can't leave them out of step.", false))
        .child(draft_with_send(look, "You", "Keep insert_legacy until the backfill ends."));
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        // Full screen has one title bar (the shell's), carrying the base chip.
        .when(!full, |panel| panel.child(panel_header(look, "Diff", false, Some(base_chip(look)))))
        .child(if full { div().flex_1().flex().child(list).child(view) } else { div().flex_1().flex().flex_col().child(list).child(view) })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                .child(quiet(look, "Push & PR…"))
                .child(quiet(look, "Add repo…"))
                // Beside the terminal the button carries the count (DS-45).
                .when(full, |row| row.child(secondary(look, "1 draft comment")))
                .child(div().flex_1())
                .child(button(look, Kind::Primary, "Send 1 comment")),
        )
}

/// A quiet footer text button: secondary text, never the accent.
pub fn quiet(look: Look, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(18.))
        .px(px(6.))
        .rounded(px(5.))
        .whitespace_nowrap()
        .text_size(px(11.5))
        .text_color(c(look.secondary))
        .child(text)
}

fn inbox_panel(look: Look, full: bool) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .child(panel_header(look, "Inbox", full, Some(secondary(look, "this session and its forks"))))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(10.))
                .child(crate::pages::request_card(look, false))
                .child(
                    card(look)
                        .p(px(12.))
                        .gap(px(4.))
                        .text_size(px(12.5))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(small_icon(IconName::GitFork, look.secondary))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Push try-brin to origin"))
                                .child(div().flex_1())
                                .child(secondary(look, "9m")),
                        )
                        .child(secondary(look, "Claude Code in Try BRIN index")),
                )
                .child(div().pt(px(6.)).child(label(look, "Allowed without asking")))
                .child(secondary(look, "✓ Created PAY-9 “Backfill progress bar” · 12m"))
                .child(secondary(look, "✓ Commented on PAY-3 · 20m")),
        )
}

fn files_panel(look: Look, full: bool) -> Div {
    let entry = |indent: f32, glyph: IconName, name: &'static str, mark: Option<(&'static str, u32)>, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(23.))
            .pl(px(8. + indent))
            .pr(px(8.))
            .rounded(px(5.))
            .when(on, |row| row.bg(c(look.fill)))
            .child(small_icon(glyph, look.secondary))
            .child(div().flex_1().child(name))
            .children(mark.map(|(mark, hex)| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(hex)).child(mark)))
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .child(panel_header(look, "Files", full, None))
        .child(div().p(px(10.)).child(field(look, "Filter files", true)))
        .child(
            div()
                .flex()
                .flex_col()
                .px(px(6.))
                .text_size(px(12.5))
                .child(entry(0., IconName::FolderGit2, "ledger", None, false))
                .child(entry(14., IconName::FolderOpen, "migrations", None, false))
                .child(entry(28., IconName::FileCode, "0041_init.sql", None, false))
                .child(entry(28., IconName::FileCode, "0042_accounts.sql", Some(("A", GREEN)), false))
                .child(entry(14., IconName::Folder, "docs", None, false))
                .child(entry(14., IconName::FileText, "README.md", None, false))
                .child(entry(0., IconName::FolderGit2, "api", None, false))
                .child(entry(14., IconName::FolderOpen, "src", None, false))
                .child(entry(28., IconName::FileCode, "client.rs", Some(("M", ORANGE)), true))
                .child(entry(28., IconName::FileCode, "entry.rs", Some(("M", ORANGE)), false))
                .child(entry(28., IconName::FileCode, "lib.rs", None, false))
                .child(entry(14., IconName::FileText, "Cargo.toml", None, false)),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                .child(button(look, Kind::Secondary, "Open big"))
                .child(button(look, Kind::Secondary, "Open in editor"))
                .child(button(look, Kind::Secondary, "Upload…")),
        )
}

fn info_panel(look: Look, full: bool) -> Div {
    info_with(look, full, None)
}

/// The Info panel's body, with `extra` as a last section (frame 54 adds
/// Archived).
pub fn info_with(look: Look, full: bool, extra: Option<Div>) -> Div {
    let kv = |key: &'static str, value: Div| {
        div()
            .flex()
            .gap(px(10.))
            .py(px(3.))
            .child(div().w(px(84.)).flex_shrink_0().text_color(c(look.secondary)).child(key))
            .child(value.flex_1())
    };
    let link = |text: &'static str| div().text_color(c(look.accent)).child(text);
    div()
        .flex_1()
        .flex()
        .flex_col()
        .child(panel_header(look, "Info", full, None))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .text_size(px(12.5))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(kv("Agent", div().child("Claude Code")))
                        .child(kv("Machine", div().child("devbox-1 · Ubuntu 24.04")))
                        .child(kv("Started", div().child("Today 11:40")))
                        .child(kv("Forks", div().flex().flex_col().gap(px(3.)).child(link("Try BRIN index")).child(link("Review the schema"))))
                        .child(kv(
                            "Clones",
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(div().flex().gap(px(6.)).child("ledger").child(chip(look, "ledger-schema", look.secondary)).child(chip(look, "main clone", look.accent)))
                                .child(div().flex().gap(px(6.)).child("api").child(chip(look, "ledger-schema", look.secondary))),
                        )),
                )
                .child(
                    card(look)
                        .p(px(10.))
                        .gap(px(4.))
                        .child(div().flex().items_center().gap(px(6.)).child(small_icon(IconName::ShieldCheck, GREEN)).child(div().font_weight(FontWeight::SEMIBOLD).child("Survives restarts")))
                        .child(secondary(look, "humatad holds this terminal on devbox-1, not the app. After a reboot it runs claude --resume 7f3a… in the same folder. 10,000 lines of scrollback kept on disk there.")),
                )
                .child(
                    card(look)
                        .p(px(10.))
                        .gap(px(4.))
                        .child(div().flex().items_center().gap(px(6.)).child(small_icon(IconName::Bot, look.accent)).child(div().font_weight(FontWeight::SEMIBOLD).child("What this agent may do")))
                        .child(secondary(look, "Owns this session, its board (4 items), its clones and the 2 items it created. Everything else goes to the Inbox."))
                        .child(secondary(look, "Not contained: it can read and write anything you can on devbox-1."))
                        .child(link("Channel rules")),
                )
                .children(extra),
        )
}

fn board_panel(look: Look, full: bool) -> Div {
    let item = |id: &'static str, title: &'static str, state: Option<St>| {
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .px(px(10.))
            .py(px(7.))
            .rounded(px(7.))
            .bg(c(look.card))
            .border_1()
            .border_color(c(look.line))
            .child(div().text_size(px(11.)).text_color(c(look.secondary)).child(id))
            .child(div().flex_1().child(title))
            .children(state.map(status))
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        // Full screen has one title bar (the shell's), carrying the channel.
        .when(!full, |panel| panel.child(panel_header(look, "Board", false, Some(secondary(look, "payments-rewrite")))))
        // Expanded, the panel shows the channel's lanes side by side.
        .when(full, |panel| panel.child(crate::board::lanes(look, Some("payments-rewrite"), None)))
        .when(!full, |panel| panel.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(10.))
                .text_size(px(12.5))
                // The same items as the expanded board (frame 12), active
                // lanes first and Done last; empty lanes are left out (L14).
                .child(label(look, "In progress · 1"))
                .child(item("PAY-3", "Ledger schema v2", Some(St::Working)))
                .child(div().pt(px(6.)).child(label(look, "In review · 1")))
                .child(item("PAY-5", "API rate limiter", Some(St::Idle)))
                .child(div().pt(px(6.)).child(label(look, "To do · 2")))
                .child(item("PAY-9", "Backfill progress bar", Some(St::Working)))
                .child(item("PAY-7", "Refund retry with backoff", None))
                .child(div().pt(px(6.)).child(label(look, "Backlog · 1")))
                .child(item("PAY-12", "Multi-currency ledgers", None))
                .child(div().pt(px(6.)).child(label(look, "Done · 1")))
                .child(item("PAY-2", "Currency codes table", None).opacity(0.6)),
        ))
}

/// VW-10: every view prepared in the session and its forks, newest first:
/// its title and state, then its kind, the fork it came from (for a fork's
/// view) and when, its summary, and Open.
fn journal_panel(look: Look, full: bool) -> Div {
    let entry = |title: &'static str, state: &'static str, meta: &'static str, summary: &'static str| {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(10.))
            .rounded(px(7.))
            .bg(c(look.card))
            .border_1()
            .border_color(c(look.line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(small_icon(IconName::Sparkles, PURPLE))
                    .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(secondary(look, state)),
            )
            .child(secondary(look, meta))
            .child(secondary(look, summary))
            .child(div().flex().justify_end().pt(px(4.)).child(button(look, Kind::Secondary, "Open")))
    };
    div()
        .flex_1()
        .flex()
        .flex_col()
        .child(panel_header(look, "Journal", full, None))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(10.))
                .text_size(px(12.5))
                .child(entry("Review: ledger schema", "Opened", "Review · 14m", "A guided walk through the 3 files, risky parts first"))
                .child(entry("Backfill plan", "Opened and closed", "View · 1h", "Batches, timings and the rollback"))
                .child(entry("Index benchmark", "Later", "View · in Try BRIN index · yesterday", "BRIN vs B-tree on 48k rows")),
        )
}

fn title(which: Panel) -> &'static str {
    match which {
        Panel::Diff => "Diff",
        Panel::Inbox => "Inbox",
        Panel::Files => "Files",
        Panel::Info => "Info",
        Panel::Board => "Board",
        Panel::Journal => "Journal",
    }
}

pub fn panel(look: Look, screen: Screen, which: Panel) -> Div {
    let full = screen.fill == Fill::PanelFull;
    let body = match which {
        Panel::Diff => diff_panel(look, full),
        Panel::Inbox => inbox_panel(look, full),
        Panel::Files => files_panel(look, full),
        Panel::Info => info_panel(look, full),
        Panel::Board => board_panel(look, full),
        Panel::Journal => journal_panel(look, full),
    };
    // Full screen, the Diff's base chip sits beside its title in the one bar.
    // The Board's channel sits there the same way (DS-14, DV-18).
    let extra = match which {
        Panel::Diff if full => Some(base_chip(look)),
        Panel::Board if full => Some(secondary(look, "payments-rewrite")),
        _ => None,
    };
    panel_shell(look, full, title(which), extra, body)
}

/// The right panel's column around `body`; full screen, it carries the
/// window's own title bar, with `extra` beside the title.
fn panel_shell(look: Look, full: bool, title: &'static str, extra: Option<Div>, body: Div) -> Div {
    div()
        .when(full, |panel| panel.flex_1())
        .when(!full, |panel| panel.w(px(PANEL_WIDTH)).flex_shrink_0())
        .h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(c(look.panel))
        .text_size(px(12.5))
        .when(full, |panel| {
            panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .h(px(52.))
                    .px(px(16.))
                    .border_b_1()
                    .border_color(c(look.line))
                    .bg(c(look.content))
                    .child(traffic_lights())
                    .child(div().w(px(4.)))
                    .child(icon(IconName::PanelLeftOpen, look.secondary))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(secondary(look, "# payments-rewrite › Ledger schema migration"))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child(title))
                                    .children(extra),
                            ),
                    )
                    .child(div().flex_1())
                    // UXSIMPLE 3-3: full screen, the Diff opens an agent tab
                    // beside itself.
                    .when(title == "Diff", |bar| {
                        bar.child(
                            button(look, Kind::Secondary, "New agent tab")
                                .gap(px(6.))
                                .child(small_icon(IconName::ChevronDown, look.secondary)),
                        )
                    })
                    .child(button(look, Kind::Secondary, "Exit full screen"))
                    // The app's Close is a 16 pt button round its 12 pt glyph.
                    .child(div().size(px(16.)).flex().items_center().justify_center().flex_shrink_0().child(small_icon(IconName::X, look.secondary))),
            )
        })
        .child(body)
}

fn middle(look: Look, screen: Screen) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(header(look, screen))
        .when(screen.term == Term::NeedsYou, |column| {
            column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(7.))
                    .flex_shrink_0()
                    .bg(c(look.attention_bg))
                    .border_b_1()
                    .border_color(c(look.line))
                    .text_size(px(12.5))
                    .child(small_icon(IconName::Sparkles, PURPLE))
                    .child(div().flex_1().child("Claude Code prepared a view: Review: ledger schema"))
                    .child(button(look, Kind::Secondary, "Open it"))
                    .child(div().text_color(c(look.secondary)).child("Later"))
                    .child(div().text_color(c(look.secondary)).child("Not now")),
            )
        })
        .child(tabs(look, screen))
        .child(terminal(look, screen.term))
}

pub fn session(look: Look, screen: Screen) -> Div {
    let root = div().size_full().flex();
    match screen.fill {
        Fill::TerminalFull => return root.child(middle(look, screen)),
        Fill::PanelFull => return root.child(panel(look, screen, screen.panel.unwrap_or(Panel::Diff))),
        Fill::Collapsed => {
            return root
                .child(sidebar::strip(look))
                .child(middle(look, screen))
                .child(
                    div()
                        .w(px(14.))
                        .h_full()
                        .bg(c(look.panel))
                        .border_l_1()
                        .border_color(c(look.line))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(small_icon(IconName::ChevronLeft, look.secondary)),
                );
        }
        Fill::Columns => {}
    }
    root.child(sidebar::sidebar(look, side(screen)))
        .child(handle(look, true))
        .child(middle(look, screen))
        .when_some(screen.panel, |root, which| root.child(handle(look, true)).child(panel(look, screen, which)))
        .child(ledger_strip(look, screen.panel, side(screen)))
}

/// The session screen with `body` as its right panel, `on` lit in the
/// strip and `title` over it when it fills the window (`Fill::PanelFull`).
/// For the frames whose panel shows a state the stock panels don't.
pub fn session_with(look: Look, screen: Screen, on: Panel, title: &'static str, body: Div) -> Div {
    session_with_extra(look, screen, on, title, None, body)
}

/// `session_with`, with `extra` (the Diff's base chip) beside the title in
/// the one full-screen title bar. Beside the terminal the panel's own header
/// carries it, so `extra` is unused there.
pub fn session_with_extra(look: Look, screen: Screen, on: Panel, title: &'static str, extra: Option<Div>, body: Div) -> Div {
    let root = div().size_full().flex();
    if screen.fill == Fill::PanelFull {
        return root.child(panel_shell(look, true, title, extra, body));
    }
    root.child(sidebar::sidebar(look, side(screen)))
        .child(handle(look, true))
        .child(middle(look, screen))
        .child(handle(look, true))
        .child(panel_shell(look, false, title, None, body))
        .child(ledger_strip(look, Some(on), side(screen)))
}

/// A session-shaped screen from parts: the sidebar at `side`, `middle`
/// (a header, tabs and body the frame draws itself) and, when given, a
/// right panel (`on` lit in the strip, `title`, `body`). For the frames
/// whose header or terminal show a state the stock screen doesn't.
pub fn shell(look: Look, side: Side, middle: Div, panel: Option<(Panel, &'static str, Div)>) -> Div {
    let on = panel.as_ref().map(|(on, _, _)| *on);
    div()
        .size_full()
        .flex()
        .child(sidebar::sidebar(look, side))
        .child(handle(look, true))
        .child(middle.flex_1().min_w_0().h_full().flex().flex_col())
        .when_some(panel, |root, (_, title, body)| root.child(handle(look, true)).child(panel_shell(look, false, title, None, body)))
        .child(strip(look, on))
}

/// A session header like the stock one, with the words given: `crumb`
/// over the title, the status dot, `words` beside the title, then `right`
/// before the Fork, full screen, panel and menu icons.
pub fn header_with(look: Look, crumb: &'static str, dot: St, title: &'static str, words: Option<(&'static str, u32)>, right: Vec<Div>) -> Div {
    header_parts(look, false, crumb, dot, title, words, right)
}

/// [`header_with`], led by the Back ghost when `back` (D848).
pub fn header_parts(look: Look, back: bool, crumb: &'static str, dot: St, title: &'static str, words: Option<(&'static str, u32)>, right: Vec<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(52.))
        .px(px(16.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .bg(c(look.content))
        .when(back, |bar| bar.child(back_ghost(look)))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_size(px(11.5))
                        .text_color(c(look.secondary))
                        .child(div().text_color(c(look.accent)).child(crumb))
                        .child("›"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(status(dot))
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child(title))
                        .children(words.map(|(words, hex)| div().text_size(px(11.5)).text_color(c(hex)).child(words))),
                ),
        )
        .child(div().flex_1())
        .children(right)
        .child(icon(IconName::GitFork, look.secondary))
        .child(icon(IconName::Maximize2, look.secondary))
        .child(icon(IconName::PanelRight, look.accent))
        .child(icon(IconName::Ellipsis, look.secondary))
}

/// One tab group's row in a split: the same tabs as a lone terminal,
/// with the focused group's tab lit.
fn group_tabs(look: Look, state: St, agent: &'static str, extra: Option<&'static str>, focused: bool) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(tab(look, Some(status(state).into_any_element()), agent, true).when(!focused, |tab| tab.text_color(c(look.secondary))))
        .children(extra.map(|name| tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), name, false)))
        .child(div().px(px(10.)).child(small_icon(IconName::Plus, look.secondary)))
        .child(div().flex_1())
        .child(div().mr(px(10.)).child(small_icon(IconName::X, look.tertiary)))
}

fn small_terminal(look: Look, lines: Vec<Div>) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .bg(c(look.term))
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(px(16.))
        .children(lines)
}

/// One session with two agents' terminals side by side on the same code,
/// at most one terminal per agent; more copies are forks (SE-13).
pub fn split(look: Look) -> Div {
    let left = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(group_tabs(look, St::NeedsYou, "claude", Some("zsh"), true))
        .child(small_terminal(look, transcript(look)));
    let right = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(group_tabs(look, St::Working, "codex", None, false))
        .child(small_terminal(
            look,
            vec![
                tl(look.term_dim, "~/src/ledger (ledger-schema) $ codex"),
                tl(look.term_text, "> Review the migration Claude is writing; don't edit files."),
                tl(look.term_text, ""),
                tl(look.term_text, "• Read migrations/0042_accounts.sql"),
                tl(look.term_text, "• Read api/src/client.rs"),
                tl(look.term_text, ""),
                tl(look.term_text, "  The dual write isn't in a transaction: a crash between"),
                tl(look.term_text, "  insert_entry and insert_legacy leaves them out of step."),
                tl(ORANGE, "• Working (41s • esc to interrupt)"),
            ],
        ));
    div()
        .size_full()
        .flex()
        .child(sidebar::sidebar(look, Side::at("Ledger schema migration")))
        .child(handle(look, false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(header(look, Screen::new(None, Term::NeedsYou)))
                .child(div().flex_1().min_h_0().flex().child(left).child(handle(look, true)).child(right)),
        )
        .child(ledger_strip(look, None, Side::at("Ledger schema migration")))
}

/// Frame 124: a Working session's header with its run time after the
/// title, "Working · 2m 14s, as the terminal shows" (PROGRESS; `pages::WorkingNote`).
pub fn working_elapsed(look: Look) -> Div {
    let screen = Screen::new(None, Term::Working);
    let middle = div()
        .child(header_with(
            look,
            "# payments-rewrite",
            St::Working,
            "Ledger schema migration",
            Some(("Working · 2m 14s", look.secondary)),
            vec![secondary(look, "Claude Code · devbox-1")],
        ))
        .child(tabs(look, screen))
        .child(terminal(look, Term::Working));
    // Working, as in frame 01: green, and only the fork's request in the Inbox.
    shell(look, Side { asking: false, ..Side::at("Ledger schema migration") }, middle, None)
}

/// Frame 126: the Diff panel while a big diff is read. The sweep runs
/// along the top edge of the panel's body, full width and without a label,
/// until the rows draw (PROGRESS T7, D807 P1). A still frame draws the
/// sweep's segment, a quarter of the track, at 37%.
pub fn diff_loading(look: Look) -> Div {
    let sweep = div()
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .right(px(0.))
        .h(px(2.))
        .overflow_hidden()
        .bg(c(look.line))
        .child(
            div()
                .absolute()
                .top(px(0.))
                .h_full()
                .w(relative(0.25))
                .left(relative(0.37))
                .rounded(px(1.))
                .bg(c(look.accent)),
        );
    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(panel_header(look, "Diff", false, Some(base_chip(look))))
        .child(div().relative().flex_1().min_h_0().child(sweep));
    session_with(look, Screen::new(Some(Panel::Diff), Term::Working), Panel::Diff, "Diff", body)
}
