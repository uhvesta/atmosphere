//! Frames 67-69: reviewing a pull request (PR-1 to PR-17): the Review a pull
//! request dialog opened from ⌘K, the review session with its Diff panel and
//! the agent's drafts, and the Submit review dialog.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::overlays::scrim;
use crate::session::{self, Panel};
use crate::sidebar::Side;

/// A dialog row: the key on the left, the value filling the rest.
fn dialog_row(look: Look, key: &'static str, value: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().w(px(80.)).flex().justify_end().flex_shrink_0().text_color(c(look.secondary)).child(key))
        .child(value.flex_1().min_w_0())
}

/// The dialog's shell: centred under the top edge over the scrim.
fn dialog(look: Look, under: Div, top: f32, width: f32, body: Div) -> Div {
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(top)).left_0().w_full().flex().justify_center().child(
            body.w(px(width))
                .flex()
                .flex_col()
                .gap(px(11.))
                .p(px(20.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(12.5)),
        ),
    )
}

fn dialog_title(look: Look, title: &'static str, note: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .child(secondary(look, note))
}

/// Frame 67: ⌘K with a pasted PR link, then the Review a pull request
/// dialog with the facts Humata fetched (PR-1 to PR-5).
pub fn review_dialog(look: Look) -> Div {
    let strip = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(34.))
        .px(px(12.))
        .rounded(px(9.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .shadow_lg()
        .text_size(px(12.5))
        .child(icon(IconName::Search, look.secondary))
        .child(div().whitespace_nowrap().child("https://github.com/acme/api/pull/482"))
        .child(div().flex_1())
        .child(kbd(look, "↩"));
    let facts = card(look)
        .p(px(10.))
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(small_icon(IconName::GitPullRequest, look.accent))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("acme/api #482"))
                .child(div().flex_1().whitespace_nowrap().overflow_hidden().child("Fix refund rounding for partial captures")),
        )
        .child(secondary(look, "by rkhan · uhvesta/fix-refund-rounding → main"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.5))
                .child(div().text_color(c(look.secondary)).child("7 files"))
                .child(div().text_color(c(GREEN)).child("+214"))
                .child(div().text_color(c(RED)).child("−38")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.5))
                .child(small_icon(IconName::Check, GREEN))
                .child(div().text_color(c(look.secondary)).child("checks: 11 passed,"))
                .child(small_icon(IconName::CircleX, RED))
                .child(div().text_color(c(RED)).child("1 failed (lint)")),
        );
    let also = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(button(look, Kind::Secondary, "+ add a PR in another repo"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .h(px(24.))
                .rounded(px(6.))
                .bg(c(look.fill))
                .child(small_icon(IconName::GitPullRequest, look.secondary))
                .child(div().whitespace_nowrap().child("acme/ledger #91"))
                .child(secondary(look, "linked"))
                .child(small_icon(IconName::X, look.tertiary)),
        );
    let machine = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(popup(look, "This Mac"))
        .child(div().flex_1().whitespace_nowrap().text_size(px(11.)).text_color(c(look.tertiary)).child("This Mac · devbox-1 · gpu-box (offline)"));
    let context = div()
        .min_h(px(46.))
        .p(px(8.))
        .rounded(px(6.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .child("rounding on partial captures; anything that can double-refund");
    let body = div()
        .child(dialog_title(look, "Review a pull request", "in # pr-reviews"))
        .child(facts)
        .child(dialog_row(look, "Also review", also))
        .child(dialog_row(look, "Machine", machine))
        .child(dialog_row(look, "Agent", div().flex().child(popup(look, "Claude Code")).child(div().flex_1())))
        .child(dialog_row(look, "Context", context))
        .child(dialog_row(look, "", checkbox(look, false, "Let the agent push to the PR branch")))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Start the review")),
        );
    let screen = crate::pages::home_list(look);
    let screen = dialog(look, screen, 96., 640., body);
    screen.child(
        div()
            .absolute()
            .top(px(40.))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(div().w(px(640.)).child(strip)),
    )
}

/// One diff line: number, tint, text.
fn diff_line(look: Look, n: &'static str, bg: Option<u32>, text: &'static str) -> Div {
    div()
        .flex()
        .when_some(bg, |line, bg| line.bg(c(bg)))
        .child(div().w(px(36.)).pr(px(8.)).flex().justify_end().flex_shrink_0().text_color(c(look.term_dim)).child(n))
        .child(div().whitespace_nowrap().text_color(c(look.term_text)).child(text))
}

fn repo_group(look: Look, open: bool, repo: &'static str, number: &'static str, branch: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .text_size(px(12.5))
        .child(small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, look.tertiary))
        .child(small_icon(IconName::FolderGit2, look.secondary))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(repo))
        .child(div().text_color(c(look.accent)).child(number))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(11.)).text_color(c(look.tertiary)).child(branch))
}

/// The Diff panel for the review: the base, each repo's PR, the agent's
/// draft under its line, and the draft count.
fn review_diff(look: Look) -> Div {
    let file = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(8.))
        .rounded(px(5.))
        .bg(c(look.fill))
        .text_size(px(12.5))
        .child(small_icon(IconName::FileCode, look.secondary))
        .child(div().flex_1().whitespace_nowrap().child("refund.rs"))
        .child(div().text_size(px(11.)).text_color(c(GREEN)).child("+62"))
        .child(div().text_size(px(11.)).text_color(c(RED)).child("−21"));
    let lines = div()
        .flex()
        .flex_col()
        .py(px(6.))
        .overflow_hidden()
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(px(17.))
        .bg(c(look.term))
        .child(diff_line(look, "138", None, "    let amount = capture.remaining();"))
        .child(diff_line(look, "139", Some(look.del_bg), "-   let cents = amount.floor();"))
        .child(
            div()
                .flex()
                .bg(c(look.add_bg))
                .child(
                    div()
                        .w(px(36.))
                        .pr(px(8.))
                        .flex()
                        .justify_end()
                        .flex_shrink_0()
                        .border_l_2()
                        .border_color(c(look.accent))
                        .text_color(c(look.accent))
                        .font_weight(FontWeight::BOLD)
                        .child("140"),
                )
                .child(div().whitespace_nowrap().text_color(c(look.term_text)).child("+   let cents = amount.round();")),
        );
    let draft = session::comment(
        look,
        "Draft for GitHub · by Claude Code",
        "round() is half-away-from-zero, so a split capture can refund a cent more than was taken. Use floor() on each part.",
        true,
    )
    .child(div().flex().gap(px(6.)).child(button(look, Kind::Secondary, "Edit")).child(button(look, Kind::Secondary, "Drop")));
    let after = div()
        .flex()
        .flex_col()
        .py(px(6.))
        .overflow_hidden()
        .font_family(MONO)
        .text_size(px(11.5))
        .line_height(px(17.))
        .bg(c(look.term))
        .child(diff_line(look, "141", None, "    refund.issue(cents)?;"));
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .p(px(8.))
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(4.))
                        .child(small_icon(IconName::GitPullRequest, look.secondary))
                        .child(secondary(look, "PR #482 against main · 2 repos")),
                )
                .child(repo_group(look, true, "api", "#482", "uhvesta/fix-refund-rounding"))
                .child(file)
                .child(lines)
                .child(draft)
                .child(after)
                .child(repo_group(look, false, "ledger", "#91", "uhvesta/refund-ledger")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .h(px(38.))
                .px(px(10.))
                .flex_shrink_0()
                .border_t_1()
                .border_color(c(look.line))
                .child(secondary(look, "Drafts: 4 (3 by agent, 1 by you)")),
        )
}

/// Frame 68: the review session, its chat on the left and the PR's diff
/// with the agent's drafts on the right (PR-6 to PR-11, PR-15).
pub fn review_session(look: Look) -> Div {
    let header = session::header_with(
        look,
        "# pr-reviews",
        St::Working,
        "acme/api #482",
        None,
        vec![secondary(look, "Claude Code · This Mac"), button(look, Kind::Primary, "Submit review")],
    );
    let tabs = div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(session::tab(look, Some(status(St::Working).into_any_element()), "claude", true, false))
        .child(session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false, false))
        .child(div().px(px(10.)).child(small_icon(IconName::Plus, look.secondary)));
    let note_line = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(tl(look.term_text, "● review_note refund.rs:140"))
        .child(button(look, Kind::Secondary, "Undo"));
    let terminal = div()
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
        .child(tl(look.term_dim, "~/src/api (fix-refund-rounding) $ claude"))
        .child(tl(look.term_text, ""))
        .child(tl(look.term_text, "> why round() and not floor()?"))
        .child(tl(look.term_text, "● review_focus refund.rs:140"))
        .child(tl(look.term_dim, "  It rounds half away from zero …"))
        .child(tl(look.term_text, ""))
        .child(tl(look.term_text, "> note that on the line for me"))
        .child(note_line)
        .child(tl(look.term_text, ""))
        .child(session::input_box(look, true));
    let bar = div()
        .px(px(10.))
        .pb(px(10.))
        .flex_shrink_0()
        .bg(c(look.term))
        .child(session::notice(look, IconName::Info, ORANGE, "2 new commits on the PR branch", Some("Update the clone")));
    let middle = div().child(header).child(tabs).child(terminal).child(bar);
    session::shell(look, Side::at(""), middle, Some((Panel::Diff, "Diff", review_diff(look))))
}

/// One draft in the Submit review dialog.
fn draft_row(look: Look, place: &'static str, who: &'static str, text: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .py(px(6.))
        .child(div().w(px(150.)).flex_shrink_0().whitespace_nowrap().font_family(MONO).text_size(px(11.)).text_color(c(look.secondary)).child(place))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().text_size(px(12.)).child(text))
                .child(div().text_size(px(10.5)).text_color(c(look.tertiary)).child(who)),
        )
        .child(button(look, Kind::Secondary, "Edit"))
        .child(button(look, Kind::Secondary, "Drop"))
}

/// One PR's section: its drafts and its verdict, and Add a summary for a PR
/// whose review has no summary yet.
fn pr_section(look: Look, title: &'static str, drafts: Vec<Div>, verdict: usize, add_summary: bool) -> Div {
    let last = drafts.len();
    card(look)
        .p(px(10.))
        .gap(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .pb(px(4.))
                .child(small_icon(IconName::GitPullRequest, look.accent))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(secondary(look, if last == 1 { "1 draft" } else { "3 drafts" })),
        )
        .children(drafts)
        .child(hline(look))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .pt(px(6.))
                .child(div().text_color(c(look.secondary)).child("Verdict"))
                .child(segmented(look, &["Comment", "Approve", "Request changes"], verdict))
                .when(add_summary, |row| row.child(div().flex_1()).child(button(look, Kind::Secondary, "Add a summary"))),
        )
}

/// Frame 69: the Submit review dialog over the review session: each PR's
/// drafts and verdict, the agent's summary, one review per PR (PR-12, PR-13).
pub fn submit_review(look: Look) -> Div {
    let api = pr_section(
        look,
        "acme/api #482",
        vec![
            draft_row(look, "refund.rs:140", "Claude Code", "round() can refund a cent more than was captured."),
            draft_row(look, "refund.rs:212", "Claude Code", "This retry can double-refund; key it on the capture id."),
            draft_row(look, "refund_test.rs:58", "You", "Add a case for three-way splits."),
        ],
        2,
        false,
    );
    let ledger = pr_section(look, "acme/ledger #91", vec![draft_row(look, "ledger.rs:33", "Claude Code", "Record the rounded amount, not the requested one.")], 0, true);
    let summary = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(label(look, "Summary for acme/api #482 · drafted by Claude Code"))
        .child(
            div()
                .p(px(8.))
                .rounded(px(6.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.accent))
                .text_size(px(12.))
                .child("Rounding on partial captures can over-refund by a cent, and the retry path can refund twice. Both need fixing before this merges."),
        );
    let body = div()
        .child(dialog_title(look, "Submit review", "2 pull requests"))
        .child(api)
        .child(ledger)
        .child(summary)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(2.))
                .child(div().flex_1().whitespace_nowrap().text_size(px(11.)).text_color(c(look.tertiary)).child("Posts one review per PR with This Mac's gh"))
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Submit 2 reviews")),
        );
    dialog(look, review_session(look), 24., 700., body)
}
