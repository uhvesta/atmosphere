//! Frames 84 to 86 and 98 (VIEWS A1): the Diff panel with a base per repository.
//! The panel's picker reads "Comparing per repo", and each repo section
//! names the base it compares against. 84 is the panel beside the terminal,
//! 85 the panel full screen, and 86 the ledger repo's base menu open over
//! the panel. 98 is the refusal when a repo has no default branch.
//!
//! The repo, file and diff helpers below are copies of session.rs's private
//! ones, changed where these frames differ; session.rs stays as it is.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::session::{self, Fill, Panel, Screen, Term};

/// A repo's one row: the fold chevron, its name, the base it compares
/// against (every repo shares the session's branch, so none is shown), and
/// its totals at the right. Push & PR lives in the footer.
fn repo_head(look: Look, name: &'static str, base: &'static str, add: &'static str, del: &'static str, open: bool) -> Div {
    let against = div()
        .flex()
        .items_center()
        .min_w_0()
        .text_size(px(11.5))
        .whitespace_nowrap()
        .child(div().flex_shrink_0().mr(px(3.)).text_color(c(if open { look.accent } else { look.secondary })).child("against"))
        .child(div().min_w_0().overflow_hidden().text_ellipsis().text_color(c(if open { look.accent } else { look.text })).child(base))
        .child(div().flex_shrink_0().ml(px(3.)).child(small_icon(IconName::ChevronsUpDown, if open { look.accent } else { look.secondary })));
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
        .child(against)
        .child(div().flex_1())
        .child(div().flex_shrink_0().whitespace_nowrap().text_size(px(11.)).text_color(c(GREEN)).child(add))
        .child(div().flex_shrink_0().whitespace_nowrap().text_size(px(11.)).text_color(c(RED)).child(del))
}

/// A file under its repo: a 14 px status column (empty for a modified file),
/// the name, then its directory dimmed, the comment count and +a −r.
/// `path` is the directory, then the name.
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
        .child(div().px(px(10.)).text_color(c(look.term_dim)).child("@@ -40,7 +40,12 @@ impl Client {"))
        .child(line("40", None, "    pub async fn post(&self, entry: Entry)"))
        .child(line("41", Some(look.del_bg), "-       self.db.insert_legacy(&entry).await?;"))
        .child(line("41", Some(look.add_bg), "+       let tx = self.db.begin().await?;"))
        .child(line("42", Some(look.add_bg), "+       tx.insert_entry(&entry).await?;"))
        .child(line("43", Some(look.add_bg), "+       tx.insert_legacy(&entry).await?;"))
        .child(line("44", Some(look.add_bg), "+       tx.commit().await?;"))
}

/// A saved draft with its Send, as session.rs draws a draft in the panel.
fn draft_card(look: Look, who: &'static str, text: &'static str) -> Div {
    div()
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .rounded(px(8.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.accent))
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
                .child(chip(look, "Draft", look.accent))
                .child(div().flex_1())
                .child(div().px(px(6.)).text_size(px(11.)).text_color(c(look.accent)).child("Send")),
        )
        .child(text)
}

/// The selected file's diff and its comments, as the stock panel shows them.
fn file_view(look: Look) -> Div {
    div()
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
                .child(secondary(look, "Open in editor").flex_shrink_0().opacity(0.5)),
        )
        .child(diff_lines(look))
        .child(session::comment(look, "You", "Wrap the two writes in one transaction so a crash can't leave them out of step.", false))
        .child(draft_card(look, "You", "Keep insert_legacy until the backfill ends."))
}

fn footer(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(10.))
        .border_t_1()
        .border_color(c(look.line))
        .child(session::quiet(look, "Push & PR…"))
        .child(session::quiet(look, "Add repo…"))
        .child(secondary(look, "1 draft comment"))
        .child(div().flex_1())
        .child(button(look, Kind::Primary, "Send 1 comment"))
}

/// The repos and their files. `open` lights the ledger repo's base while its
/// menu is open.
fn repo_list(look: Look, full: bool, open: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .p(px(8.))
        .gap(px(1.))
        .text_size(px(12.5))
        .when(full, |list| list.w(px(300.)).h_full().border_r_1().border_color(c(look.line)))
        .child(repo_head(look, "ledger", "release/2.4", "+142", "−18", open))
        .child(file_row(look, ("migrations/", "0042_accounts.sql"), Some("A"), "+41", "−0", false, None))
        // Renamed from src/ledger.rs; not the open file, so no note line shows.
        .child(file_row(look, ("src/", "posting.rs"), Some("R"), "+101", "−18", false, None))
        .child(repo_head(look, "api", "main", "+143", "−24", false))
        .child(file_row(look, ("src/", "client.rs"), None, "+62", "−21", true, Some("2")))
        .child(file_row(look, ("src/", "entry.rs"), None, "+81", "−3", false, None))
}

/// The panel's column: its header with the picker, the repos and the
/// selected file, and the footer. `overlay` is laid over the column.
fn panel_body(look: Look, full: bool, overlay: Option<Div>) -> Div {
    let open = overlay.is_some();
    let content = div()
        .flex_1()
        .min_h_0()
        .flex()
        .when(!full, |row| row.flex_col())
        .child(repo_list(look, full, open))
        .child(file_view(look));
    div()
        .flex_1()
        .min_h_0()
        .relative()
        .flex()
        .flex_col()
        // Full screen has one title bar (the shell's), carrying the picker.
        .when(!full, |panel| panel.child(session::panel_header(look, "Diff", false, Some(popup(look, "Comparing per repo")))))
        .child(content)
        .child(footer(look))
        .children(overlay)
}

/// The base menu open under the ledger repo's base: what to compare with (the
/// header menu's words), a branch filter, the branches as of the last fetch
/// and a way to name a branch or commit.
fn base_menu(look: Look) -> Div {
    let item = |text: &'static str, checked: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .text_size(px(12.5))
            .whitespace_nowrap()
            .child(
                div()
                    .size(px(12.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(checked, |mark| mark.child(small_icon(IconName::Check, look.accent))),
            )
            .child(text)
    };
    div()
        .absolute()
        .top(px(72.))
        .left(px(84.))
        .w(px(240.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(px(6.))
        .rounded(px(9.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .shadow_lg()
        .child(div().px(px(8.)).pt(px(4.)).pb(px(2.)).child(label(look, "Compare with")))
        .child(item("Default branch (origin/HEAD)", false))
        .child(item("Uncommitted only", false))
        .child(item("Since I last reviewed", false))
        .child(hline(look).my(px(4.)))
        .child(div().px(px(8.)).pt(px(2.)).pb(px(2.)).child(label(look, "Branches · as of last fetch")))
        .child(field(look, "Filter branches", true).mx(px(2.)).mb(px(4.)))
        .child(item("main", false))
        .child(item("release/2.4", true))
        .child(item("release/2.3", false))
        .child(item("feat/ledger-v2", false))
        .child(hline(look).my(px(4.)))
        .child(item("Branch or commit…", false))
}

/// Frame 84 (light): the session beside its terminal, the Diff panel with
/// each repo against its own base.
pub fn beside_terminal(look: Look) -> Div {
    session::session_with(look, Screen::new(Some(Panel::Diff), Term::Working), Panel::Diff, "Diff", panel_body(look, false, None))
}

/// Frame 85 (dark): the Diff panel full screen, each repo's base inline on
/// its name's line.
pub fn full_screen(look: Look) -> Div {
    let screen = Screen { fill: Fill::PanelFull, ..Screen::new(Some(Panel::Diff), Term::Working) };
    session::session_with_extra(look, screen, Panel::Diff, "Diff", Some(popup(look, "Comparing per repo")), panel_body(look, true, None))
}

/// Frame 86 (light): frame 84 with the ledger repo's base menu open.
pub fn base_picker(look: Look) -> Div {
    session::session_with(look, Screen::new(Some(Panel::Diff), Term::Working), Panel::Diff, "Diff", panel_body(look, false, Some(base_menu(look))))
}

/// Frame 98 (light): the panel beside the terminal when the ledger repo has
/// no default branch. The body is the refusal block (D179, D293): what
/// failed, a button that has Humata find the default branch in that clone,
/// and the repo's "against…" chip to pick a branch instead.
pub fn no_default_branch(look: Look) -> Div {
    let block = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(px(8.))
        .p(px(12.))
        .child(secondary(look, "Couldn't read the changes."))
        .child(secondary(look, "ledger has no default branch to compare with."))
        .child(div().flex().child(button(look, Kind::Primary, "Find the default branch")))
        // The hint and the chip stack, so the whole chip fits the panel.
        .child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(6.))
                .child(secondary(look, "Or pick a branch to compare against."))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .text_size(px(11.5))
                        .whitespace_nowrap()
                        .child(div().mr(px(3.)).text_color(c(look.secondary)).child("against"))
                        .child(div().text_color(c(look.text)).child("pick a branch…"))
                        .child(div().ml(px(3.)).child(small_icon(IconName::ChevronsUpDown, look.secondary))),
                ),
        );
    let body = div()
        .flex_1()
        .min_h_0()
        .relative()
        .flex()
        .flex_col()
        .child(session::panel_header(look, "Diff", false, Some(popup(look, "Comparing per repo"))))
        .child(block)
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                // No diff, nothing to push: only Add repo… (as the app).
                .child(session::quiet(look, "Add repo…")),
        );
    session::session_with(look, Screen::new(Some(Panel::Diff), Term::Working), Panel::Diff, "Diff", body)
}
