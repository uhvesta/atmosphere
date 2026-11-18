//! Archiving and deleting (frames 65, 66 and 71; SPEC MV-7 to MV-9, LD-1 to
//! LD-4, CH-24, CH-25): the Archive confirmation with unpushed work, an
//! archived session opened read-only, and the Delete channel… dialog.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::channel;
use crate::kit::*;
use crate::overlays::scrim;
use crate::session::{self, Panel, Screen, Term};
use crate::sidebar::Side;

/// A dialog's card, `width` wide.
fn dialog(look: Look, width: f32) -> Div {
    div()
        .w(px(width))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(20.))
        .rounded(px(12.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .shadow_lg()
        .text_size(px(12.5))
}

/// A dialog's title.
fn title(text: &'static str) -> Div {
    div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text)
}

/// A button that destroys: red text and border. `on` is false when it is
/// disabled.
fn danger(look: Look, on: bool, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .whitespace_nowrap()
        .border_1()
        .when(on, |b| b.border_color(c(RED)).text_color(c(RED)).bg(c(look.fill)))
        .when(!on, |b| b.border_color(c(look.line)).text_color(c(look.tertiary)).bg(c(look.fill)))
        .child(text)
}

/// A line of work that would be lost, in orange.
fn work(text: &'static str) -> Div {
    div().flex().items_start().gap(px(7.)).text_color(c(ORANGE)).child(small_icon(IconName::GitBranch, ORANGE)).child(div().flex_1().child(text))
}

/// Frame 65: archive a session that holds unpushed work (MV-7 to MV-9).
pub fn archive_unpushed(look: Look) -> Div {
    session::session(look, Screen::new(Some(Panel::Diff), Term::Working))
        .relative()
        .child(scrim(look))
        .child(
            div().absolute().top(px(120.)).left_0().w_full().flex().justify_center().child(
                dialog(look, 660.)
                    .child(title("Archive “Ledger schema migration”?"))
                    .child(div().text_color(c(look.secondary)).child("Archiving stops the agent and keeps the conversation, the record, the scrollback and a summary of the diff. It frees about 2.4 GB once the Undo window passes."))
                    .child(div().text_color(c(look.text)).child("Its 2 forks keep running; they move up in the tree."))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .mt(px(4.))
                            .p(px(10.))
                            .rounded(px(7.))
                            .bg(c(look.attention_bg))
                            .child(work("ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed."))
                            .child(work("api: 1 commit on uhvesta/ledger-reads not pushed."))
                            .child(div().pl(px(19.)).font_weight(FontWeight::SEMIBOLD).text_color(c(ORANGE)).child("Archiving deletes that work.")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .pt(px(10.))
                            .child(div().flex_1())
                            .child(button(look, Kind::Secondary, "Cancel"))
                            .child(danger(look, true, "Archive and delete the work"))
                            .child(button(look, Kind::Primary, "Commit, push and open a PR, then archive")),
                    ),
            ),
        )
}

/// Frame 66: an archived session opened, read-only, with Restore… and
/// Delete (LD-2, LD-3).
pub fn archived(look: Look) -> Div {
    let tabs = div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(session::tab(look, Some(status(St::Idle).into_any_element()), "claude", true, false))
        .child(div().flex_1());
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
        .text_color(c(look.term_dim))
        .child(div().opacity(0.7).flex().flex_col().gap(px(1.)).children(session::transcript(look)))
        .child(div().flex_1())
        .child(session::notice(look, IconName::Archive, GREY, "Archived Nov 12. Read-only: Restore asks for a machine and clones each branch again.", None));
    let middle = div()
        .child(session::header_with(
            look,
            "# payments-rewrite",
            St::Idle,
            "Currency codes",
            Some(("Archived Nov 12", GREY)),
            vec![button(look, Kind::Secondary, "Delete"), button(look, Kind::Primary, "Restore…")],
        ))
        .child(tabs)
        .child(body);

    let repo = |name: &'static str, branch: &'static str, pushed: &'static str, files: Vec<(&'static str, &'static str, &'static str)>| {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().flex().items_center().gap(px(6.)).child(small_icon(IconName::FolderGit2, look.secondary)).child(div().font_weight(FontWeight::SEMIBOLD).child(name)))
            .child(div().pl(px(18.)).flex().items_center().gap(px(6.)).child(small_icon(IconName::GitBranch, look.secondary)).child(div().whitespace_nowrap().child(branch)))
            .child(div().pl(px(18.)).whitespace_nowrap().text_size(px(11.5)).text_color(c(look.secondary)).child(pushed))
            .children(files.into_iter().map(|(path, add, del)| {
                div()
                    .pl(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(path))
                    .child(div().flex_shrink_0().text_color(c(GREEN)).child(add))
                    .child(div().flex_shrink_0().text_color(c(RED)).child(del))
            }))
    };
    let kv = |key: &'static str, value: &'static str| {
        div()
            .flex()
            .gap(px(10.))
            .py(px(2.))
            .child(div().w(px(84.)).flex_shrink_0().text_color(c(look.secondary)).child(key))
            .child(div().flex_1().child(value))
    };
    let diff = div()
        .flex_1()
        .flex()
        .flex_col()
        .child(session::panel_header(look, "Diff summary", false, None))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(12.))
                .text_size(px(12.5))
                .child(secondary(look, "As it was when archived. The clones are gone."))
                .child(repo(
                    "ledger",
                    "uhvesta/currency-codes",
                    "last pushed: 4be91c2 Add ISO 4217 table",
                    vec![("src/currency.rs", "+84", "−12"), ("src/money.rs", "+19", "−7"), ("tests/codes.rs", "+61", "−0")],
                ))
                .child(repo("api", "uhvesta/currency-api", "last pushed: 9d03a7e Accept currency on /charge", vec![("src/charge.rs", "+23", "−9")]))
                .child(hline(look))
                .child(div().flex().flex_col().child(label(look, "Session record")).child(div().h(px(4.))).child(kv("Agent", "Claude Code")).child(kv("Machine", "devbox-1")).child(kv("Fork of", "none")).child(kv("Board item", "PAY-7"))),
        );
    session::shell(look, Side::at("payments-rewrite"), middle, Some((Panel::Diff, "Diff summary", diff)))
}

/// One live session in the Delete channel… dialog, with the work it holds.
fn doomed(look: Look, name: &'static str, note: Option<&'static str>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().flex().items_center().gap(px(7.)).child(small_icon(IconName::SquareTerminal, look.secondary)).child(div().whitespace_nowrap().child(name)))
        .children(note.map(|note| div().pl(px(19.)).text_size(px(11.5)).text_color(c(ORANGE)).child(note)))
}

/// Frame 71: Delete channel… (CH-24; draft, D11 is open).
pub fn delete_channel(look: Look) -> Div {
    channel::sessions(look).relative().child(scrim(look)).child(
        div().absolute().top(px(40.)).left_0().w_full().flex().justify_center().child(
            dialog(look, 700.)
                .gap(px(6.))
                .child(title("Delete #payments-rewrite?"))
                .child(div().text_color(c(look.secondary)).child("Everything in the channel goes with it."))
                .child(div().pt(px(4.)).child(label(look, "Live sessions (6)")))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(doomed(look, "Ledger schema migration and its 3 forks", Some("ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed.")))
                        .child(div().pl(px(19.)).child(checkbox(look, true, "Delete that work"))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(doomed(look, "Refund retries", Some("2 commits on uhvesta/refund-retries never pushed. This blocks the delete until it is pushed or you tick the box.")))
                        .child(div().pl(px(19.)).child(checkbox(look, false, "Delete that work"))),
                )
                .child(doomed(look, "API rate limiter", None))
                .child(div().pt(px(4.)).child(label(look, "Also in the channel")))
                .child(secondary(look, "Archived sessions: 2 (Currency codes, Webhook retries)"))
                .child(secondary(look, "Board items: 14"))
                .child(secondary(look, "Desk sessions: 2 (1 on devbox-1). They close on their own machines; a machine that is offline closes them when it links."))
                .child(secondary(look, "Pending Inbox requests: 2, withdrawn"))
                .child(
                    div()
                        .mt(px(4.))
                        .p(px(10.))
                        .rounded(px(7.))
                        .bg(c(look.fill))
                        .text_color(c(look.secondary))
                        .child("The agents stop. Everything goes to the trash on This Mac, devbox-1 and gpu-box. Restore it from the host's trash within 30 days and the channel comes back with its sessions archived."),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pt(px(8.))
                        .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("Tick “Delete that work” for Refund retries to continue"))
                        .child(div().flex_1())
                        .child(button(look, Kind::Secondary, "Cancel"))
                        .child(danger(look, false, "Delete channel")),
                ),
        ),
    )
}
