//! Drafts for the owner (2026-11-15): the window with no channels and no
//! machines but This Mac, a channel with nothing in it yet, and the
//! Desk: sessions outside any channel, in a scratch folder, for
//! running the board, adding projects and writing things down.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::{page, toolbar};
use crate::session::{self, Panel};
use crate::sidebar::Side;

const fn empty(selected: &'static str) -> Side {
    Side { selected, pinned: false, folded: false, desk: true, empty: true, fresh: None }
}

const fn desk_side(selected: &'static str) -> Side {
    Side { selected, pinned: false, folded: true, desk: true, empty: false, fresh: None }
}

fn step(look: Look, glyph: IconName, title: &'static str, note: &'static str, action: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(14.))
        .py(px(12.))
        .border_b_1()
        .border_color(c(look.line))
        .child(icon(glyph, look.accent))
        .child(div().flex_1().flex().flex_col().child(div().font_weight(FontWeight::MEDIUM).child(title)).child(secondary(look, note)))
        .child(action)
}

fn nothing(look: Look, text: &'static str) -> Div {
    card(look).px(px(14.)).py(px(12.)).text_color(c(look.tertiary)).child(text)
}

/// Home with no channels and no machines but This Mac: the sidebar keeps
/// its rows, and Home says what to set up.
pub fn empty_home(look: Look) -> Div {
    page(
        look,
        empty("Home"),
        toolbar(look, "Home", None, vec![button(look, Kind::Primary, "New desk session")]),
        div().flex().justify_center().p(px(24.)).text_size(px(13.)).child(
            div()
                .w(px(720.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(div().flex().flex_col().gap(px(6.)).child(label(look, "Needs you")).child(nothing(look, "Nothing needs you.")))
                .child(div().flex().flex_col().gap(px(6.)).child(label(look, "Working")).child(nothing(look, "No sessions are running.")))
                .child(
                    div().flex().flex_col().gap(px(6.)).child(label(look, "Get set up")).child(
                        card(look)
                            .overflow_hidden()
                            .child(step(look, IconName::Plus, "Make a channel", "A project: its repos, the machines that work on it, its board and its sessions.", button(look, Kind::Primary, "New channel…")))
                            .child(step(look, IconName::Server, "Add a machine", "Run sessions on another computer over SSH. This Mac works already.", button(look, Kind::Secondary, "Add a machine…")))
                            .child(step(look, IconName::SquareTerminal, "Use the Desk", "An agent or a shell in a scratch folder, no channel needed: plan, tidy the board, write.", button(look, Kind::Secondary, "Open Desk"))),
                    ),
                )
                .child(secondary(look, "GitHub access comes from the gh CLI. There is no sign-in in Humata.")),
        ),
    )
}

/// A channel just made: no repos, no sessions, only This Mac.
pub fn empty_channel(look: Look) -> Div {
    let tab = |text: &'static str, lit: bool| {
        div()
            .flex()
            .items_center()
            .h_full()
            .px(px(2.))
            .border_b_2()
            .border_color(c(if lit { look.accent } else { look.content }))
            .text_color(c(if lit { look.text } else { look.secondary }))
            .when(lit, |tab| tab.font_weight(FontWeight::MEDIUM))
            .child(text)
    };
    let header = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(52.))
                .px(px(20.))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("# web-revamp"))
                .child(secondary(look, "no repos yet · This Mac"))
                .child(div().flex_1())
                .child(icon(IconName::Ellipsis, look.secondary))
                .child(button(look, Kind::Primary, "New session")),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(20.))
                .h(px(32.))
                .px(px(20.))
                .text_size(px(12.5))
                .child(tab("Sessions", true))
                .child(tab("Inbox", false))
                .child(tab("Board", false))
                .child(tab("Repos and machines", false))
                .child(tab("Agent access", false))
                .child(tab("Activity", false)),
        );
    let done = |text: &'static str| {
        div().flex().items_center().gap(px(12.)).px(px(14.)).py(px(12.)).border_b_1().border_color(c(look.line)).child(icon(IconName::Check, GREEN)).child(div().flex_1().text_color(c(look.secondary)).child(text))
    };
    page(
        look,
        Side { fresh: Some("web-revamp"), ..empty("web-revamp") },
        header,
        div().flex().justify_center().p(px(24.)).text_size(px(13.)).child(
            div()
                .w(px(640.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Nothing in # web-revamp yet"))
                .child(
                    card(look)
                        .overflow_hidden()
                        .child(done("Channel made"))
                        .child(step(look, IconName::FolderGit2, "Add a repo", "Sessions get their own clone of each repo you add.", button(look, Kind::Primary, "Add a GitHub repo…")))
                        .child(step(look, IconName::Server, "Pick machines", "This Mac is in. Add others to run sessions there.", button(look, Kind::Secondary, "Add a machine…")))
                        .child(step(look, IconName::SquareTerminal, "Start a session", "An agent's TUI or a plain shell. Works without a repo, in an empty folder.", button(look, Kind::Secondary, "New session"))),
                )
                .child(secondary(look, "The board is empty too: add items from the Board tab, or ask an agent to.")),
        ),
    )
}

struct DeskSession {
    state: St,
    title: &'static str,
    runs: &'static str,
    folder: &'static str,
    repos: &'static [&'static str],
    when: &'static str,
}

const OPEN: &[DeskSession] = &[
    DeskSession { state: St::Working, title: "Plan Q4 board", runs: "Claude Code · This Mac", folder: "scratch folder", repos: &[], when: "now" },
    DeskSession { state: St::Idle, title: "BRIN rollout RFC", runs: "Claude Code · devbox-1", folder: "scratch folder · kept", repos: &["acme/ledger", "acme/api"], when: "Today 12:05" },
    DeskSession { state: St::Idle, title: "Tidy infra items", runs: "Terminal · This Mac", folder: "~/notes", repos: &[], when: "Nov 2" },
];

fn desk_row(look: Look, session: &DeskSession, action: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(c(look.line))
        .child(status(session.state))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().font_weight(FontWeight::MEDIUM).child(session.title))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(secondary(look, session.runs))
                        .child(secondary(look, "·"))
                        .child(small_icon(IconName::Folder, look.secondary))
                        .child(secondary(look, session.folder))
                        .children(session.repos.iter().map(|repo| chip(look, *repo, look.secondary))),
                ),
        )
        .child(div().w(px(110.)).flex().flex_col().items_end().child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("last used")).child(secondary(look, session.when)))
        .children(action)
}

/// The Desk: sessions outside any channel.
pub fn desk(look: Look) -> Div {
    let doc = |name: &'static str, from: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(9.))
            .border_b_1()
            .border_color(c(look.line))
            .child(small_icon(IconName::FileText, look.secondary))
            .child(div().flex_1().flex().gap(px(8.)).child(div().font_family(MONO).text_size(px(12.)).child(name)).child(secondary(look, from)))
            .child(button(look, Kind::Secondary, "Copy to a channel…"))
    };
    page(
        look,
        desk_side("Desk"),
        toolbar(look, "Desk", Some("Sessions outside any channel"), vec![popup(look, "Sort: last used"), button(look, Kind::Secondary, "Clean up…"), button(look, Kind::Primary, "New desk session")]),
        div().flex().justify_center().p(px(24.)).text_size(px(13.)).child(
            div()
                .w(px(760.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(
                    div().flex().flex_col().gap(px(6.)).child(label(look, "Open · 3")).child(
                        card(look).overflow_hidden().children(OPEN.iter().map(|session| desk_row(look, session, None))),
                    ),
                )
                .child(
                    div().flex().flex_col().gap(px(6.)).child(label(look, "Documents")).child(
                        card(look)
                            .overflow_hidden()
                            .child(doc("rfc-brin.md", "BRIN rollout RFC"))
                            .child(doc("q4-plan.md", "Plan Q4 board")),
                    ),
                )
                .child(
                    div().flex().flex_col().gap(px(6.)).child(label(look, "Closed · scratch folders go after 7 days")).child(
                        card(look).overflow_hidden().child(desk_row(
                            look,
                            &DeskSession { state: St::Exited, title: "Sketch the release checklist", runs: "Codex · This Mac", folder: "deleted in 6 days", repos: &[], when: "Nov 14" },
                            Some(div().flex().gap(px(6.)).child(button(look, Kind::Secondary, "Keep")).child(button(look, Kind::Secondary, "Reopen"))),
                        )),
                    ),
                ),
        ),
    )
}

/// Where a desk session works: its folder, its machine and its
/// repos, each changeable from here or with `humata desk`.
fn place(look: Look, folder: &'static str, scratch: bool, machine: &'static str, repos: &[&'static str]) -> Div {
    let repo = |name: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(7.))
            .h(px(20.))
            .rounded(px(5.))
            .bg(c(look.fill))
            .child(small_icon(IconName::FolderGit2, look.secondary))
            .child(name)
            .child(small_icon(IconName::X, look.tertiary))
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(34.))
        .px(px(14.))
        .flex_shrink_0()
        .bg(c(look.content))
        .border_b_1()
        .border_color(c(look.line))
        .text_size(px(12.))
        .child(small_icon(IconName::Folder, look.secondary))
        .child(div().font_family(MONO).text_size(px(11.5)).child(folder))
        .when(scratch, |bar| bar.child(chip(look, "scratch", look.secondary)))
        .child(div().w(px(1.)).h(px(16.)).bg(c(look.line)))
        .child(small_icon(IconName::Server, look.secondary))
        .child(div().flex().items_center().gap(px(3.)).child(machine).child(small_icon(IconName::ChevronDown, look.secondary)))
        .child(div().w(px(1.)).h(px(16.)).bg(c(look.line)))
        .children(repos.iter().map(|name| repo(*name)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(c(look.accent))
                .child(small_icon(IconName::Plus, look.accent))
                .when(repos.is_empty(), |add| add.child("Add repo")),
        )
        .child(div().flex_1())
        .when(scratch, |bar| bar.child(div().text_color(c(look.accent)).child("Keep folder")))
}

fn desk_header(look: Look, state: St, title: &'static str, runs: &'static str, panel: bool) -> Div {
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
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().flex().items_center().gap(px(5.)).text_size(px(11.5)).text_color(c(look.secondary)).child(div().text_color(c(look.accent)).child("Desk")).child("›"))
                .child(div().flex().items_center().gap(px(7.)).child(status(state)).child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child(title))),
        )
        .child(div().flex_1())
        .child(secondary(look, runs))
        .child(icon(IconName::Maximize2, look.secondary))
        .child(icon(IconName::PanelRight, if panel { look.accent } else { look.secondary }))
        .child(icon(IconName::Ellipsis, look.secondary))
}

fn desk_tabs(look: Look, state: St, on: usize, file: Option<(&'static str, IconName)>) -> Div {
    let mut bar = div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(session::tab(look, Some(status(state).into_any_element()), "claude", on == 0, false))
        .child(session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", on == 1, false));
    if let Some((name, glyph)) = file {
        bar = bar.child(session::tab(look, Some(small_icon(glyph, look.secondary).into_any_element()), name, on == 2, true));
    }
    bar.child(div().px(px(10.)).child(small_icon(IconName::Plus, look.secondary)))
}

fn term(look: Look, lines: Vec<Div>) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .px(px(14.))
        .py(px(10.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .bg(c(look.term))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(17.))
        .text_color(c(look.term_text))
        .children(lines)
}

fn desk_screen(look: Look, side: Side, middle: Div, right: Div, panel: Panel) -> Div {
    div()
        .size_full()
        .flex()
        .child(crate::sidebar::sidebar(look, side))
        .child(handle(look, true))
        .child(middle)
        .child(handle(look, true))
        .child(right)
        .child(session::strip(look, Some(panel)))
}

/// A desk session running the board: the agent adds items across
/// channels and asks before it makes a channel.
pub fn board_session(look: Look) -> Div {
    let lines = vec![
        tl(look.term_dim, "~/humata/desk/plan-q4 $ claude"),
        tl(look.term_text, "> Plan Q4 for payments: add the backfill follow-ups, and make a"),
        tl(look.term_text, "  channel for the web revamp."),
        tl(look.term_text, ""),
        tl(look.term_text, "● humata board_items channel=payments-rewrite"),
        tl(look.term_dim, "  ⎿ 12 items · 3 in progress · 2 in review"),
        tl(look.term_text, "● humata board_item_create PAY-13 “Drop the ledger_entries index”"),
        tl(look.term_dim, "  ⎿ Added to # payments-rewrite · To do"),
        tl(look.term_text, "● humata board_item_create PAY-14 “Backfill metrics dashboard”"),
        tl(look.term_dim, "  ⎿ Added to # payments-rewrite · To do · depends on PAY-13"),
        tl(look.term_text, "● humata channel_create web-revamp repos=acme/web"),
        tl(ORANGE, "  ⎿ ⏳ waiting for your approval (Inbox)"),
        tl(look.term_text, ""),
        tl(look.term_text, "● Write(q4-plan.md)"),
        tl(look.term_dim, "  ⎿ Wrote 38 lines"),
        tl(look.term_text, ""),
        tl(ORANGE, "✻ Ordering the payments lanes… (48s · esc to interrupt)"),
    ];
    let middle = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(desk_header(look, St::Working, "Plan Q4 board", "Claude Code · This Mac", true))
        .child(place(look, "~/humata/desk/plan-q4", true, "This Mac", &[]))
        .child(desk_tabs(look, St::Working, 0, Some(("q4-plan.md", IconName::FileText))))
        .child(term(look, lines).child(div().flex_1()).child(session::input_box(look, true)));
    desk_screen(
        look,
        desk_side("Plan Q4 board"),
        middle,
        board_panel(look),
        Panel::Board,
    )
}

/// The Board panel in a desk: every channel's items, newest first.
fn board_panel(look: Look) -> Div {
    let item = |key: &'static str, title: &'static str, channel: &'static str, new: bool| {
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .p(px(9.))
            .rounded(px(7.))
            .bg(c(look.card))
            .border_1()
            .border_color(c(if new { look.accent } else { look.line }))
            .child(div().flex().gap(px(6.)).child(div().text_color(c(look.secondary)).child(key)).child(div().flex_1().font_weight(FontWeight::MEDIUM).child(title)))
            .child(div().flex().gap(px(6.)).child(secondary(look, channel)).when(new, |row| row.child(div().text_size(px(11.)).text_color(c(look.accent)).child("added by this session"))))
    };
    let lane = |title: &'static str| div().pt(px(8.)).child(label(look, title));
    div()
        .w(px(session::PANEL_WIDTH))
        .flex_shrink_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(c(look.panel))
        .text_size(px(12.5))
        .child(session::panel_header(look, "Board", false, Some(popup(look, "All channels"))))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(12.))
                .child(lane("To do · 4"))
                .child(item("PAY-13", "Drop the ledger_entries index", "# payments-rewrite", true))
                .child(item("PAY-14", "Backfill metrics dashboard", "# payments-rewrite", true))
                .child(item("PAY-9", "Backfill progress bar", "# payments-rewrite", false))
                .child(item("INF-4", "Rotate the backup keys", "# infra", false))
                .child(lane("In progress · 2"))
                .child(item("PAY-3", "Ledger schema", "# payments-rewrite", false))
                .child(item("INF-2", "Upgrade Postgres 17", "# infra", false)),
        )
}

fn desk_files(look: Look, machine: &'static str) -> Div {
    let entry = |indent: f32, glyph: IconName, name: &'static str, note: Option<&'static str>, on: bool| {
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
            .children(note.map(|note| secondary(look, note)))
    };
    div()
        .w(px(session::PANEL_WIDTH))
        .flex_shrink_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(c(look.panel))
        .text_size(px(12.5))
        .child(session::panel_header(look, "Files", false, Some(secondary(look, machine))))
        .child(
            div()
                .flex()
                .flex_col()
                .p(px(6.))
                .child(entry(0., IconName::FileText, "rfc-brin.md", None, true))
                .child(entry(0., IconName::FolderOpen, "notes", None, false))
                .child(entry(14., IconName::FileText, "bench-results.md", None, false))
                .child(entry(0., IconName::FolderGit2, "ledger", Some("acme/ledger · main"), false))
                .child(entry(0., IconName::FolderGit2, "api", Some("acme/api · main"), false)),
        )
        .child(div().flex_1())
        .child(div().p(px(12.)).border_t_1().border_color(c(look.line)).child(secondary(look, "Repos you add are cloned into this folder. Removing one deletes its clone, not your branch.")))
}

/// The same desk, moved to another machine with repos added, from
/// its shell.
pub fn move_session(look: Look) -> Div {
    let lines = vec![
        tl(look.term_dim, "~/humata/desk/brin-rfc $ humata desk move devbox-1"),
        tl(look.term_text, "Copying 3 files to devbox-1… done"),
        tl(look.term_text, "claude restarts on devbox-1 with its conversation."),
        tl(look.term_dim, "~/humata/desk/brin-rfc $ humata desk add acme/ledger acme/api"),
        tl(look.term_text, "Cloned acme/ledger (main) into ./ledger"),
        tl(look.term_text, "Cloned acme/api (main) into ./api"),
        tl(look.term_dim, "~/humata/desk/brin-rfc $ humata desk keep"),
        tl(look.term_text, "This folder stays after the session closes."),
        tl(look.term_dim, "~/humata/desk/brin-rfc $ "),
    ];
    let middle = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(desk_header(look, St::Idle, "BRIN rollout RFC", "Claude Code · devbox-1", true))
        .child(place(look, "~/humata/desk/brin-rfc", false, "devbox-1", &["acme/ledger", "acme/api"]))
        .child(desk_tabs(look, St::Idle, 1, Some(("rfc-brin.md", IconName::FileText))))
        .child(term(look, lines));
    desk_screen(look, desk_side("BRIN rollout RFC"), middle, desk_files(look, "devbox-1"), Panel::Files)
}

/// rfc-brin.md rendered. With `mark`, the block a comment is on is lit
/// and carries the comment count.
fn brin_doc(look: Look, width: Option<f32>, mark: bool) -> Div {
    let h = |text: &'static str| div().pt(px(10.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text);
    let p = |text: &'static str| div().text_color(c(look.text)).child(text);
    let li = |text: &'static str| div().flex().gap(px(8.)).child("•").child(div().flex_1().min_w_0().child(text));
    let lit = div()
        .flex()
        .gap(px(8.))
        .ml(px(-10.))
        .pl(px(8.))
        .pr(px(4.))
        .border_l_2()
        .border_color(c(look.accent))
        .bg(c(if look.dark { 0x1c2a3f } else { 0xeaf2ff }))
        .child("•")
        .child(div().flex_1().min_w_0().child("Keep the btree until the backfill ends, then drop it in one migration."))
        .child(div().flex().items_center().gap(px(3.)).text_size(px(11.)).text_color(c(look.accent)).child(small_icon(IconName::MessageSquare, look.accent)).child("1"));
    let body = div()
        .pt(px(22.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child("BRIN index rollout"))
        .child(secondary(look, "Draft · written with Claude Code · rfc-brin.md"))
        .child(h("Problem"))
        .child(p("ledger_entries is 410 GB and its btree on posted_at is 38 GB. Range reads by day are 92% of the load."))
        .child(h("Proposal"))
        .child(li("Add a BRIN index on posted_at with 64 pages per range (PAY-13)."))
        .child(if mark { lit } else { li("Keep the btree until the backfill ends, then drop it in one migration.") })
        .child(li("Watch p95 for range reads on the metrics board (PAY-14)."))
        .child(h("Bench"))
                .child(
                    div()
                        .p(px(10.))
                        .rounded(px(6.))
                        .bg(c(look.fill))
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(17.))
                .child("btree  38 GB   range p95 41 ms")
                .child("brin   2.1 MB  range p95 57 ms"),
        );
    div()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .bg(c(look.content))
        .flex()
        .justify_center()
        .text_size(px(13.))
        .line_height(px(20.))
        .child(match width {
            Some(width) => body.w(px(width)),
            None => body.flex_1().min_w_0().px(px(24.)),
        })
}

/// A document open in a desk tab, beside its folder.
pub fn doc_session(look: Look) -> Div {
    let doc = brin_doc(look, Some(560.), false);
    let middle = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(desk_header(look, St::Idle, "BRIN rollout RFC", "Claude Code · devbox-1", true))
        .child(place(look, "~/humata/desk/brin-rfc", false, "devbox-1", &["acme/ledger", "acme/api"]))
        .child(desk_tabs(look, St::Idle, 2, Some(("rfc-brin.md", IconName::FileText))))
        .child(doc);
    desk_screen(look, desk_side("BRIN rollout RFC"), middle, desk_files(look, "devbox-1"), Panel::Files)
}

/// New desk session, over the Desk.
pub fn new_desk(look: Look) -> Div {
    let row = |key: &'static str, value: Div| {
        div()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(div().w(px(110.)).pt(px(3.)).flex().justify_end().text_color(c(look.secondary)).child(key))
            .child(value.flex_1())
    };
    let scrim = div().absolute().top_0().left_0().size_full().bg(if look.dark { rgba(0x0000_0080) } else { rgba(0x0000_0030) });
    desk(look).relative().child(scrim).child(
        div().absolute().top(px(70.)).left_0().w_full().flex().justify_center().child(
            div()
                .w(px(560.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(20.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(12.5))
                .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("New desk session")).child(secondary(look, "outside any channel")))
                .child(row("Title", field(look, "Plan Q4 board", false)))
                .child(row("Runs", div().flex().child(segmented(look, &["Claude Code", "Codex", "Copilot", "OpenCode", "Terminal"], 0)).child(div().flex_1())))
                .child(row("Machine", div().flex().gap(px(8.)).child(popup(look, "This Mac")).child(secondary(look, "Move it later with humata desk move")).child(div().flex_1())))
                .child(row(
                    "Folder",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(radio(look, true, "A new scratch folder", "~/humata/desk/plan-q4 · deleted 7 days after you close it, unless you keep it"))
                        .child(radio(look, false, "A folder on the machine", "Pick one. Humata never deletes it.")),
                ))
                .child(row(
                    "Repos",
                    div().flex().items_center().gap(px(8.)).child(button(look, Kind::Secondary, "Add a repo…")).child(secondary(look, "Optional. Cloned into the folder.")).child(div().flex_1()),
                ))
                .child(row(
                    "Agent access",
                    div().flex().flex_col().gap(px(4.)).child(checkbox(look, true, "Read and change every channel's board")).child(checkbox(look, false, "Make channels and add machines (asks you each time when off)")),
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .pt(px(4.))
                        .child(div().flex_1())
                        .child(button(look, Kind::Secondary, "Cancel"))
                        .child(button(look, Kind::Primary, "Start")),
                ),
        ),
    )
}

/// One tab group's row in a two-group split; the focused group's tab is lit.
fn group_row(look: Look, tabs: Vec<Div>, extra: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .children(tabs)
        .child(div().px(px(10.)).child(small_icon(IconName::Plus, look.secondary)))
        .child(div().flex_1())
        .children(extra)
}

/// How a markdown tab shows its file: the rendered page, its source, or
/// both side by side with linked scrolling.
fn md_modes(look: Look, on: usize) -> Div {
    div().mr(px(8.)).child(segmented(look, &["Preview", "Source", "Side by side"], on))
}

fn md_tab(look: Look, on: bool) -> Div {
    session::tab(look, Some(small_icon(IconName::FileText, look.secondary).into_any_element()), "rfc-brin.md", on, true)
}

/// Reviewing a document while the agent that wrote it stays in view:
/// the terminal in one tab group, the rendered file in the other.
pub fn split_review(look: Look) -> Div {
    let lines = vec![
        tl(look.term_dim, "~/humata/desk/brin-rfc $ claude"),
        tl(look.term_text, "> Write up the BRIN rollout as an RFC from bench-results.md."),
        tl(look.term_text, ""),
        tl(look.term_text, "● Read(notes/bench-results.md)"),
        tl(look.term_text, "● Write(rfc-brin.md)"),
        tl(look.term_dim, "  ⎿ Wrote 22 lines"),
        tl(look.term_text, ""),
        tl(look.term_text, "  The RFC is in rfc-brin.md: problem, proposal and the bench"),
        tl(look.term_text, "  numbers. Want me to file PAY-13 and PAY-14 on the board?"),
        tl(look.term_text, ""),
        tl(look.term_text, "> Not yet, I'm reading it."),
    ];
    let left = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(group_row(
            look,
            vec![
                session::tab(look, Some(status(St::Idle).into_any_element()), "claude", true, false),
                session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false, false),
            ],
            None,
        ))
        .child(term(look, lines).child(div().flex_1()).child(session::input_box(look, true)));
    let right = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(group_row(look, vec![md_tab(look, true).text_color(c(look.secondary))], Some(md_modes(look, 0))))
        .child(brin_doc(look, None, false));
    let middle = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(desk_header(look, St::Idle, "BRIN rollout RFC", "Claude Code · devbox-1", false))
        .child(place(look, "~/humata/desk/brin-rfc", false, "devbox-1", &["acme/ledger", "acme/api"]))
        .child(div().flex_1().min_h_0().flex().child(left).child(handle(look, true)).child(right));
    div()
        .size_full()
        .flex()
        .child(crate::sidebar::sidebar(look, desk_side("BRIN rollout RFC")))
        .child(handle(look, true))
        .child(middle)
        .child(session::strip(look, None))
}

/// One source line: number, comment marker in the gutter, then the text,
/// tinted like markdown.
fn src(look: Look, n: &'static str, text: &'static str, lit: bool, marker: bool) -> Div {
    let tint = if text.starts_with('#') {
        look.accent
    } else if text.starts_with("```") || text.starts_with('_') {
        look.term_dim
    } else {
        look.term_text
    };
    div()
        .flex()
        .flex_shrink_0()
        .min_h(px(18.))
        .line_height(px(18.))
        .when(lit, |row| row.bg(c(if look.dark { 0x1c2a3f } else { 0xeaf2ff })))
        .child(div().w(px(30.)).flex_shrink_0().pr(px(6.)).flex().justify_end().text_color(c(look.term_dim)).child(n))
        .child(
            div()
                .w(px(18.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(lit, |gutter| gutter.border_l_2().border_color(c(look.accent)))
                .when(marker, |gutter| gutter.child(small_icon(IconName::MessageSquare, look.accent))),
        )
        .child(div().flex_1().min_w_0().pr(px(8.)).text_color(c(tint)).child(text))
}

/// rfc-brin.md's source with a draft comment on lines 13–14 and one the
/// user sent on line 7. The agent answers in its terminal, not here.
fn source_pane(look: Look) -> Div {
    let rows: [(&str, &str); 22] = [
        ("1", "# BRIN index rollout"),
        ("2", ""),
        ("3", "_Draft · written with Claude Code_"),
        ("4", ""),
        ("5", "## Problem"),
        ("6", ""),
        ("7", "ledger_entries is 410 GB and its btree on posted_at is 38 GB."),
        ("8", "Range reads by day are 92% of the load."),
        ("9", ""),
        ("10", "## Proposal"),
        ("11", ""),
        ("12", "- Add a BRIN index on posted_at with 64 pages per range (PAY-13)."),
        ("13", "- Keep the btree until the backfill ends, then drop it in one"),
        ("14", "  migration."),
        ("15", "- Watch p95 for range reads on the metrics board (PAY-14)."),
        ("16", ""),
        ("17", "## Bench"),
        ("18", ""),
        ("19", "```"),
        ("20", "btree  38 GB   range p95 41 ms"),
        ("21", "brin   2.1 MB  range p95 57 ms"),
        ("22", "```"),
    ];
    let mut source = div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .py(px(10.))
        .flex()
        .flex_col()
        .bg(c(look.term))
        .font_family(MONO)
        .text_size(px(11.5));
    for (n, text) in rows {
        let lit = n == "13" || n == "14";
        source = source.child(src(look, n, text, lit, n == "13" || n == "7"));
        if n == "7" {
            source = source.child(session::comment(look, "You · line 7 · sent to claude", "Where are the sizes from?", false));
        }
        if n == "14" {
            source = source
                .child(
                    session::comment(look, "You · lines 13–14", "Keep both until range p95 holds for a week on devbox-1, not just until the backfill ends.", true)
                        .child(secondary(look, "Typed into claude when its turn ends"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .pt(px(2.))
                                .child(div().flex_1())
                                .child(button(look, Kind::Secondary, "Cancel"))
                                .child(button(look, Kind::Primary, "Comment")),
                        ),
                );
        }
    }
    source
}

fn linked_bar(look: Look, long: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(12.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .text_size(px(11.5))
        .text_color(c(look.secondary))
        .child(small_icon(IconName::Columns2, look.secondary))
        .child(if long { "Scrolling is linked: the page follows the source. Comment on source lines; the page shows where they are." } else { "Scrolling is linked" })
        .child(div().flex_1())
        .child(checkbox(look, true, "Link scrolling"))
}

/// The source and the rendered page side by side, under the link bar.
fn side_by_side(look: Look, long: bool) -> Div {
    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .child(source_pane(look))
        .child(div().w(px(1.)).h_full().bg(c(look.line)))
        .child(div().flex_1().min_w_0().flex().flex_col().child(brin_doc(look, None, true)));
    div().flex_1().min_h_0().min_w_0().flex().flex_col().child(linked_bar(look, long)).child(body)
}

/// Side by side: comments go on the markdown source, by line, and the
/// rendered page beside it scrolls with the source.
pub fn source_review(look: Look) -> Div {
    let middle = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(desk_header(look, St::Idle, "BRIN rollout RFC", "Claude Code · devbox-1", false))
        .child(place(look, "~/humata/desk/brin-rfc", false, "devbox-1", &["acme/ledger", "acme/api"]))
        .child(group_row(
            look,
            vec![
                session::tab(look, Some(status(St::Idle).into_any_element()), "claude", false, false),
                session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false, false),
                md_tab(look, true),
            ],
            Some(md_modes(look, 2)),
        ))
        .child(side_by_side(look, true));
    div()
        .size_full()
        .flex()
        .child(crate::sidebar::sidebar(look, desk_side("BRIN rollout RFC")))
        .child(handle(look, true))
        .child(middle)
        .child(session::strip(look, None))
}

/// The agent's terminal, the markdown source and its rendered page, all
/// at once: the terminal is one tab group, the file side by side the other.
pub fn terminal_source_page(look: Look) -> Div {
    let lines = vec![
        tl(look.term_dim, "~/humata/desk/brin-rfc $ claude"),
        tl(look.term_text, "> Write up the BRIN rollout as an"),
        tl(look.term_text, "  RFC from bench-results.md."),
        tl(look.term_text, ""),
        tl(look.term_text, "● Write(rfc-brin.md)"),
        tl(look.term_dim, "  ⎿ Wrote 22 lines"),
        tl(look.term_text, ""),
        tl(look.term_text, "> [comment on rfc-brin.md:7]"),
        tl(look.term_text, "  Where are the sizes from?"),
        tl(look.term_text, ""),
        tl(look.term_text, "● Bash(psql -c \"select"),
        tl(look.term_text, "  pg_relation_size(...)\")"),
        tl(look.term_dim, "  ⎿ 410 GB · 38 GB"),
        tl(look.term_text, ""),
        tl(look.term_text, "  They're pg_relation_size on"),
        tl(look.term_text, "  devbox-1 today. Want a note on"),
        tl(look.term_text, "  line 7 saying so?"),
    ];
    let left = div()
        .w(px(330.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .child(group_row(
            look,
            vec![
                session::tab(look, Some(status(St::Idle).into_any_element()), "claude", true, false).text_color(c(look.secondary)),
                session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false, false),
            ],
            None,
        ))
        .child(term(look, lines).child(div().flex_1()).child(session::input_box(look, true)));
    let right = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(group_row(look, vec![md_tab(look, true)], Some(md_modes(look, 2))))
        .child(side_by_side(look, false));
    let middle = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(desk_header(look, St::Idle, "BRIN rollout RFC", "Claude Code · devbox-1", false))
        .child(place(look, "~/humata/desk/brin-rfc", false, "devbox-1", &["acme/ledger", "acme/api"]))
        .child(div().flex_1().min_h_0().flex().child(left).child(handle(look, true)).child(right));
    div()
        .size_full()
        .flex()
        .child(crate::sidebar::sidebar(look, desk_side("BRIN rollout RFC")))
        .child(handle(look, true))
        .child(middle)
        .child(session::strip(look, None))
}
