//! The Fork dialog (frame 62; SPEC FK-1 to FK-5, FC-15, D102), the Move
//! dialog (frame 63; MV-1 to MV-3, MV-6) and a session mid-move (frame 64;
//! MV-4, flow 26.6).

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::overlays::scrim;
use crate::session::{self, Panel, Screen, Term};
use crate::sidebar::Side;

/// `under` with `body` as a dialog card, framed like the New session
/// dialog (frame 31).
fn dialog(look: Look, under: Div, top: f32, width: f32, body: Div) -> Div {
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(top)).left_0().w_full().flex().justify_center().child(
            body.w(px(width))
                .flex()
                .flex_col()
                .gap(px(12.))
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

fn heading(text: &'static str) -> Div {
    div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text)
}

/// One kind of fork as a radio card; the selected one has an accent border.
fn kind_card(look: Look, on: bool, name: &'static str, note: &'static str) -> Div {
    div()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(c(if on { look.accent } else { look.line }))
        .bg(c(if on { look.fill } else { look.card }))
        .child(pick(look, on, name, note))
}

/// A radio whose words wrap inside its card: the circle keeps its size and
/// the note line shows only when there is one.
fn pick(look: Look, on: bool, name: &'static str, note: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .mt(px(2.))
                .size(px(14.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(on, |b| b.bg(c(look.accent)).child(div().size(px(5.)).rounded_full().bg(c(0xffffff))))
                .when(!on, |b| b.border_1().border_color(c(look.tertiary)).bg(c(look.card))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.5)).child(name))
                .when(!note.is_empty(), |words| words.child(secondary(look, note))),
        )
}

fn row(look: Look, key: &'static str, value: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().w(px(100.)).flex().justify_end().flex_shrink_0().text_color(c(look.secondary)).child(key))
        .child(value.flex_1())
}

/// Frame 62: the Fork dialog over the stock session screen (FK-2 to FK-5,
/// FC-15; D102).
pub fn fork(look: Look) -> Div {
    let under = session::session(look, Screen::new(Some(Panel::Diff), Term::Working));
    let preview = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(7.))
        .bg(c(look.fill))
        .font_family(MONO)
        .text_size(px(11.))
        .line_height(px(16.))
        .child(tl(look.text, "Ledger schema migration   [api, ledger]  Claude Code · devbox-1 · 14 turns"))
        .child(tl(look.text, "└─⑂ Ledger schema migration 2   code + context"))
        .child(tl(look.secondary, "     new clones of api, ledger on uhvesta/ledger-schema-migration-2"))
        .child(tl(look.secondary, "     conversation copied exactly (same agent)"));
    let body = div()
        .child(heading("Fork “Ledger schema migration”"))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(kind_card(look, true, "Code + context", "New copies of the files as they are now, uncommitted changes included, and an agent that knows the conversation."))
                .child(kind_card(look, false, "Context only", "An agent that knows the conversation and works in this session's files, with no copies, so its edits land here."))
                .child(kind_card(look, false, "Code only", "New copies of the files and a fresh agent with no conversation.")),
        )
        .child(row(
            look,
            "Fork from",
            div().flex().items_center().gap(px(8.)).child(popup(look, "Now")).child(secondary(look, "or before an earlier turn")).child(div().flex_1()),
        ))
        .child(row(look, "Agent", div().flex().child(popup(look, "Claude Code")).child(div().flex_1())))
        .child(row(look, "Machine", div().flex().child(popup(look, "devbox-1")).child(div().flex_1())))
        .child(row(
            look,
            "Branch",
            div().flex().flex_col().gap(px(3.)).child(field(look, "uhvesta/ledger-schema-migration-2", false)).child(secondary(look, "Humata creates this branch in each new clone.")),
        ))
        .child(div().flex().flex_col().gap(px(5.)).child(label(look, "Preview")).child(preview))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(small_icon(IconName::Check, GREEN))
                .child(secondary(look, "Same agent: the conversation is copied exactly.")),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Fork")),
        );
    dialog(look, under, 24., 640., body)
}

/// A machine as a radio row: its name, a state note, dimmed when it can't
/// be picked.
fn machine_row(look: Look, on: bool, name: &'static str, note: &'static str, choosable: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(c(if on { look.accent } else { look.line }))
        .bg(c(if on { look.fill } else { look.card }))
        .when(!choosable, |row| row.opacity(0.5))
        .child(pick(look, on, name, "").flex_1())
        .child(secondary(look, note))
}

/// The "API rate limiter" session on devbox-1 as it runs, under the Move
/// dialog.
fn rate_limiter(look: Look) -> Div {
    let tabs = div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(session::tab(look, Some(status(St::Working).into_any_element()), "claude", true, false))
        .child(session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false, false));
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
        .child(tl(look.term_dim, "~/src/api (api-rate-limiter) $ claude"))
        .child(tl(look.term_text, "● Update(api/src/limiter.rs)"))
        .child(tl(look.term_dim, "  ⎿ Updated with 38 additions and 4 removals"))
        .child(tl(look.term_text, "● Bash(cargo test -p api limiter)"))
        .child(tl(look.term_dim, "  ⎿ test result: ok. 9 passed; 0 failed"))
        .child(div().flex_1())
        .child(session::input_box(look, true));
    let middle = div()
        .child(session::header_with(look, "# payments-rewrite", St::Working, "API rate limiter", None, vec![secondary(look, "Claude Code · devbox-1")]))
        .child(tabs)
        .child(body);
    session::shell(look, Side::at("API rate limiter"), middle, None)
}

/// Frame 63: the Move dialog (MV-1 to MV-3, MV-6).
pub fn move_dialog(look: Look) -> Div {
    let under = rate_limiter(look);
    let line = |glyph: IconName, text: &'static str| {
        div().flex().items_start().gap(px(8.)).child(div().mt(px(1.)).child(small_icon(glyph, look.secondary))).child(div().flex_1().child(text))
    };
    let body = div()
        .child(heading("Move “API rate limiter” to another machine"))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(machine_row(look, true, "This Mac", "online · 210 GB free", true))
                .child(machine_row(look, false, "devbox-1", "here now", false))
                .child(machine_row(look, false, "gpu-box", "offline", false)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label(look, "What moves"))
                .child(div().child("The branch uhvesta/api-rate-limiter, 3 uncommitted files and the conversation. The board, Inbox and comments stay.")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(c(look.fill))
                .text_size(px(12.))
                .child(line(IconName::SquareTerminal, "zsh moves as a fresh shell in the same folder; programs running in it don't come along."))
                .child(line(IconName::ShieldCheck, "devbox-1's copy is deleted only after the session runs on This Mac.")),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Move to This Mac")),
        );
    dialog(look, under, 110., 600., body)
}

/// Draft frame 113 (ROAM R2): Continue in another channel… (CV-8), the
/// Move dialog's choices as channels and the Fork dialog's "Fork from"
/// row as "Continue from", over the session it continues.
pub fn continue_in_channel(look: Look) -> Div {
    let under = rate_limiter(look);
    let line = |glyph: IconName, text: &'static str| {
        div().flex().items_start().gap(px(8.)).child(div().mt(px(1.)).child(small_icon(glyph, look.secondary))).child(div().flex_1().min_w_0().child(text))
    };
    let body = div()
        .child(heading("Continue “API rate limiter” in another channel"))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(machine_row(look, false, "# payments-rewrite", "here now", false))
                .child(machine_row(look, false, "# infra", "acme/infra", true))
                .child(machine_row(look, false, "# pr-reviews", "acme/api", true))
                .child(machine_row(look, true, "# web", "acme/web", true)),
        )
        .child(row(
            look,
            "Continue from",
            div().flex().items_center().gap(px(8.)).child(popup(look, "Before turn 3")).child(secondary(look, "or before an earlier turn")).child(div().flex_1()),
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(c(look.fill))
                .text_size(px(12.))
                .child(line(IconName::FolderGit2, "A new session in # web, on its repos and a branch of its own; its agent picks up a copy of the conversation."))
                .child(line(IconName::ShieldCheck, "“API rate limiter” stays as it is.")),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Continue in # web")),
        );
    dialog(look, under, 110., 600., body)
}

/// One step of the move: a mark, its words, and how it stands.
pub(crate) fn step(look: Look, mark: &'static str, hex: u32, text: &'static str, dim: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .whitespace_nowrap()
        .child(div().w(px(14.)).flex().justify_center().text_color(c(hex)).child(mark))
        .child(div().text_color(c(if dim { look.secondary } else { look.text })).child(text))
}

/// Frame 64: the session while it moves (MV-4, flow 26.6): "Moving to
/// This Mac…", the steps, and a read-only terminal.
pub fn moving(look: Look) -> Div {
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
        .child(div().flex_1())
        .child(div().mr(px(12.)).child(secondary(look, "read-only until the agent resumes")));
    let steps = card(look)
        .gap(px(8.))
        .p(px(12.))
        .mt(px(10.))
        .font_family(SYSTEM_FONT)
        .text_size(px(12.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Moving to This Mac"))
        .child(step(look, "✓", GREEN, "Paused the agent", false))
        .child(step(look, "✓", GREEN, "Copied the branch and 3 uncommitted files", false))
        .child(step(look, "●", ORANGE, "Copying the conversation (fixing paths)…", false))
        .child(step(look, "○", look.tertiary, "Resume claude on This Mac", true))
        .child(step(look, "○", look.tertiary, "Delete the copy on devbox-1", true))
        .child(secondary(look, "If a step fails, the session keeps running on devbox-1 and nothing is deleted."));
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
        .child(tl(look.term_dim, "~/src/api (api-rate-limiter) $ claude"))
        .child(tl(look.term_dim, "● Update(api/src/limiter.rs)"))
        .child(tl(look.term_dim, "  ⎿ Updated with 38 additions and 4 removals"))
        .child(tl(look.term_dim, "● Bash(cargo test -p api limiter)"))
        .child(tl(look.term_dim, "  ⎿ test result: ok. 9 passed; 0 failed"))
        .child(tl(look.term_dim, ""))
        .child(steps)
        .child(div().flex_1())
        .child(div().opacity(0.5).child(session::input_box(look, false)))
        .child(div().pt(px(4.)).font_family(SYSTEM_FONT).text_size(px(11.5)).text_color(c(look.secondary)).child("Read-only until the agent resumes on This Mac."));
    let middle = div()
        .child(session::header_with(
            look,
            "# payments-rewrite",
            St::Working,
            "API rate limiter",
            Some(("Moving to This Mac…", ORANGE)),
            vec![secondary(look, "Claude Code · devbox-1 → This Mac")],
        ))
        .child(tabs)
        .child(body);
    session::shell(look, Side::at("API rate limiter"), middle, None)
}
