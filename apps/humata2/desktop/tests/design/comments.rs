//! Frames 50-54: review comments on a diff (RV-4, RV-25, RV-26, SM-9, SM-11,
//! SM-12, SM-15 to SM-21, NV-10, DK-9): the comment box, an addressed
//! comment, a resolved one with the agent's reply, archiving with its Undo
//! toast, and the Archived section in Info.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::session::{self, Fill, Panel, Screen, Term};

const LINE_41: &str = "Wrap the two writes in one transaction so a crash can't leave them out of step.";

/// What sits under each hunk line, and what is lit.
#[derive(Default)]
struct Threads {
    l41: Option<Div>,
    l43: Option<Div>,
    l44: Option<Div>,
    lit43: bool,
}

/// The frame's footer: what it says and whether Send is live.
struct Footer {
    drafts: &'static str,
    send: &'static str,
    live: bool,
}

fn row(look: Look, n: &'static str, bg: Option<u32>, text: &'static str, lit: bool) -> Div {
    div()
        .flex()
        .when_some(bg, |row, bg| row.bg(c(bg)))
        .child(
            div()
                .w(px(40.))
                .pr(px(8.))
                .flex()
                .justify_end()
                .when(lit, |cell| cell.border_l_2().border_color(c(look.accent)).text_color(c(look.accent)).font_weight(FontWeight::BOLD))
                .when(!lit, |cell| cell.text_color(c(look.term_dim)))
                .child(n),
        )
        .child(div().whitespace_nowrap().text_color(c(look.term_text)).child(text))
}

fn hunk(look: Look, threads: Threads) -> Div {
    let Threads { l41, l43, l44, lit43 } = threads;
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_col()
                .py(px(6.))
                .font_family(MONO)
                .text_size(px(11.5))
                .line_height(px(17.))
                .bg(c(look.term))
                .child(div().px(px(10.)).text_color(c(look.term_dim)).child("impl Client {"))
                .child(row(look, "40", None, "    pub async fn post(&self, entry: Entry)", false))
                .child(row(look, "41", Some(look.del_bg), "-       self.db.insert_legacy(&entry).await?;", false))
                .child(row(look, "41", Some(look.add_bg), "+       let tx = self.db.begin().await?;", false)),
        )
        .children(l41)
        .child(
            div()
                .flex()
                .flex_col()
                .font_family(MONO)
                .text_size(px(11.5))
                .line_height(px(17.))
                .bg(c(look.term))
                .child(row(look, "42", Some(look.add_bg), "+       tx.insert_entry(&entry).await?;", false))
                .child(row(look, "43", Some(look.add_bg), "+       tx.insert_legacy(&entry).await?;", lit43)),
        )
        .children(l43)
        .child(
            div()
                .flex()
                .flex_col()
                .py(px(0.))
                .font_family(MONO)
                .text_size(px(11.5))
                .line_height(px(17.))
                .bg(c(look.term))
                .child(row(look, "44", Some(look.add_bg), "+       tx.commit().await?;", false)),
        )
        .children(l44)
}

/// One changed file as the Diff panel lists it: a 14 px status column (empty
/// for a modified file), the name, then its directory dimmed, the comment
/// count, and +a −r. `path` is the directory, then the name.
fn file_row(
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
fn repo_row(look: Look, name: &'static str, add: &'static str, del: &'static str) -> Div {
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

/// The Diff panel full screen: files left, the hunk with its threads right.
/// `above` sits over the file view, `file_extra` in its header row.
fn diff_screen(look: Look, count: &'static str, above: Option<Div>, file_extra: Option<Div>, threads: Threads, footer: Footer) -> Div {
    let list = div()
        .flex()
        .flex_col()
        .p(px(8.))
        .gap(px(1.))
        .w(px(300.))
        .h_full()
        .flex_shrink_0()
        .border_r_1()
        .border_color(c(look.line))
        .text_size(px(12.5))
        .child(repo_row(look, "ledger", "+41", "−0"))
        .child(file_row(look, ("migrations/", "0042_accounts.sql"), Some("A"), "+41", "−0", false, None))
        .child(repo_row(look, "api", "+143", "−24"))
        .child(file_row(look, ("src/", "client.rs"), None, "+62", "−21", true, Some(count)))
        .child(file_row(look, ("src/", "entry.rs"), None, "+81", "−3", false, None));
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
                .h(px(34.))
                .flex_shrink_0()
                .border_b_1()
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
                .children(file_extra)
                .child(div().w(px(20.)).h(px(20.)).flex_shrink_0().flex().items_center().justify_center().child(small_icon(IconName::ArrowUpRight, look.secondary))),
        )
        .children(above)
        .child(div().flex_1().min_h_0().overflow_hidden().child(hunk(look, threads)));
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        // One title bar (the shell's) carries "Diff" and the base chip.
        .child(div().flex_1().min_h_0().flex().child(list).child(view))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                .child(session::quiet(look, "Push & PR…"))
                .child(session::quiet(look, "Add repo…"))
                .child(secondary(look, footer.drafts))
                .child(div().flex_1())
                .child(button(look, Kind::Primary, footer.send).when(!footer.live, |send| send.opacity(0.4))),
        )
}

fn full_diff(look: Look, body: Div) -> Div {
    let mut screen = Screen::new(Some(Panel::Diff), Term::Working);
    screen.fill = Fill::PanelFull;
    session::session_with_extra(look, screen, Panel::Diff, "Diff", Some(session::base_chip(look)), body)
}

/// A card under a hunk line, with room beside the line numbers.
fn thread(look: Look, border: u32) -> Div {
    div()
        .max_w(px(720.))
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .rounded(px(8.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(border))
        .flex()
        .flex_col()
        .gap(px(5.))
        .font_family(SYSTEM_FONT)
        .text_size(px(12.))
        .line_height(px(17.))
}

fn mini(look: Look, heading: &'static str, lines: Vec<(u32, &'static str)>) -> Div {
    div()
        .flex()
        .flex_col()
        .child(div().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.secondary)).child(heading))
        .child(
            div().flex().flex_col().rounded(px(5.)).overflow_hidden().font_family(MONO).text_size(px(11.)).line_height(px(16.)).children(
                lines.into_iter().map(|(bg, text)| div().px(px(6.)).whitespace_nowrap().bg(c(bg)).text_color(c(look.term_text)).child(text)),
            ),
        )
}

fn addressed_chip(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(3.))
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(GREEN))
        .text_size(px(11.))
        .text_color(c(GREEN))
        .child(small_icon(IconName::Check, GREEN))
        .child("addressed")
        .when(look.dark, |chip| chip.bg(c(look.add_bg)))
}

/// Frame 50: line 43 lit, its comment box open under it.
pub fn comment_box(look: Look) -> Div {
    let text_area = div()
        .min_h(px(60.))
        .p(px(8.))
        .rounded(px(6.))
        .bg(c(look.content))
        .border_1()
        .border_color(c(look.accent))
        .text_size(px(12.5))
        .flex()
        .items_start()
        .child(div().max_w(px(560.)).child("Keep insert_legacy until the backfill ends, then drop it in one migration."))
        .child(div().ml(px(1.)).mt(px(2.)).w(px(1.5)).h(px(15.)).bg(c(look.accent)));
    let open = thread(look, look.accent)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(small_icon(IconName::MessageSquare, look.accent))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("You · line 43"))
                .child(chip(look, "Draft", look.accent)),
        )
        .child(text_area)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("Saved as a draft; Send types it into claude straight away"))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Comment")),
        );
    let threads = Threads {
        l41: Some(session::comment(look, "You · line 41 · sent to claude", LINE_41, false)),
        l43: Some(open),
        l44: None,
        lit43: true,
    };
    full_diff(look, diff_screen(look, "2", None, None, threads, Footer { drafts: "1 draft comment", send: "Send 1 comment", live: true }))
}

/// Frame 51: two comments the agent addressed, each with its Resolve.
pub fn addressed(look: Look) -> Div {
    let resolve = |look: Look| button(look, Kind::Secondary, "Resolve");
    let before = mini(look, "Before", vec![(look.del_bg, "-       self.db.insert_legacy(&entry).await?;")]);
    let after = mini(
        look,
        "After",
        vec![(look.add_bg, "+       let tx = self.db.begin().await?;"), (look.add_bg, "+       tx.commit().await?;")],
    );
    let first = thread(look, GREEN)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("You · line 41"))
                .child(addressed_chip(look)),
        )
        .child(LINE_41)
        .child(div().flex().gap(px(10.)).child(div().flex_1().min_w_0().overflow_hidden().child(before)).child(div().flex_1().min_w_0().overflow_hidden().child(after)))
        .child(div().flex().child(div().flex_1()).child(resolve(look)));
    let second = thread(look, GREEN).child(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("You · line 44"))
            .child(addressed_chip(look))
            .child(div().flex_1().whitespace_nowrap().child("Commit before the metrics call."))
            .child(resolve(look)),
    );
    let threads = Threads { l41: Some(first), l43: None, l44: Some(second), lit43: false };
    full_diff(
        look,
        diff_screen(
            look,
            "2",
            None,
            Some(button(look, Kind::Secondary, "Resolve 2 addressed")),
            threads,
            Footer { drafts: "No drafts", send: "Send comments", live: false },
        ),
    )
}

/// Frame 52: Show resolved on; the resolved comment dimmed with the agent's
/// reply under it and a Reopen.
pub fn resolved(look: Look) -> Div {
    let header = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(small_icon(IconName::Check, GREEN))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("You · line 41"))
        .child(div().text_color(c(look.secondary)).child("Resolved by claude"))
        .child(div().flex_1())
        .child(button(look, Kind::Secondary, "Reopen"));
    let reply = div()
        .flex()
        .gap(px(6.))
        .pt(px(6.))
        .border_t_1()
        .border_color(c(look.line))
        .child(div().pt(px(1.)).child(small_icon(IconName::Bot, look.accent)))
        .child(
            div()
                .flex_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("claude"))
                .child("Wrapped both writes in one transaction in client.rs:41–44; the old insert stays until the backfill ends."),
        );
    let done = thread(look, look.line).child(header).child(div().text_color(c(look.secondary)).child(LINE_41)).child(reply).opacity(0.65);
    let open = session::comment(look, "You · line 43 · sent to claude", "Keep insert_legacy until the backfill ends.", false);
    let toggle = div()
        .flex()
        .items_center()
        .h(px(30.))
        .px(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .child(checkbox(look, true, "Show resolved"));
    let threads = Threads { l41: Some(done), l43: Some(open), l44: None, lit43: false };
    full_diff(look, diff_screen(look, "2", Some(toggle), None, threads, Footer { drafts: "No drafts", send: "Send comments", live: false }))
}

/// Frame 53: the line-43 draft archived; the Undo toast over the screen.
pub fn archived(look: Look) -> Div {
    let threads = Threads {
        l41: Some(session::comment(look, "You · line 41 · sent to claude", LINE_41, false)),
        l43: None,
        l44: None,
        lit43: false,
    };
    let screen = full_diff(look, diff_screen(look, "1", None, None, threads, Footer { drafts: "No drafts", send: "Send comments", live: false }));
    crate::overlays::toast(look, screen, "Archived your comment on api/src/client.rs:43", "Undo")
}

/// Frame 54: Info with its Archived section.
pub fn info_archived(look: Look) -> Div {
    let entry = |place: &'static str, text: &'static str, was: &'static str| {
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(div().font_family(MONO).text_size(px(11.)).text_color(c(look.tertiary)).child(place))
            .child(div().child(text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(secondary(look, was))
                    .child(div().flex_1())
                    .child(button(look, Kind::Secondary, "Unarchive")),
            )
    };
    let section = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(label(look, "Archived · 2"))
        .child(
            card(look)
                .p(px(10.))
                .gap(px(8.))
                .child(entry("api/src/client.rs:43", "Keep insert_legacy until the backfill ends.", "draft"))
                .child(hline(look))
                .child(entry("rfc-brin.md:13–14", "Keep both until range p95 holds for a week.", "sent to claude")),
        )
        .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("Archived comments are never sent and stay in the Journal."));
    session::session_with(look, Screen::new(Some(Panel::Info), Term::Working), Panel::Info, "Info", session::info_with(look, false, Some(section)))
}
