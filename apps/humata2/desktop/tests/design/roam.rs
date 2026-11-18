//! Frames 87 to 95 (ROAM F1), drafts: an agent proposing a repo (CH-26),
//! a Desk session moving into a channel (DK-15 to DK-19), the refusal for a
//! conversation format Humata hasn't checked (CV-3), and a channel's own
//! Desk (CD-1 to CD-6, CH-11).
//!
//! The Inbox page, the dialog frame and the Info rows below are small
//! copies of the private ones in pages.rs, fork_move.rs and session.rs.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::desk;
use crate::kit::*;
use crate::overlays::scrim;
use crate::pages::{inbox_row, page, toolbar};
use crate::session::{self, Panel, Screen, Term};
use crate::sidebar::Side;

// ---------------------------------------------------------------- shared

/// A dialog over `under`: the scrim, then a card `width` wide `top` px down.
fn modal(look: Look, under: Div, top: f32, width: f32, parts: Vec<Div>) -> Div {
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(top)).left_0().w_full().flex().justify_center().child(
            div()
                .w(px(width))
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
                .children(parts),
        ),
    )
}

fn heading(look: Look, text: &'static str, note: Option<&'static str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text))
        .children(note.map(|note| secondary(look, note)))
}

/// A form row: the key right-aligned in a 110 px column, then the value.
fn form_row(look: Look, key: &'static str, value: Div) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(div().w(px(110.)).flex_shrink_0().pt(px(3.)).flex().justify_end().text_color(c(look.secondary)).child(key))
        .child(value.flex_1().min_w_0())
}

fn actions(left: Option<Div>, buttons: Vec<Div>) -> Div {
    div().flex().items_center().gap(px(8.)).pt(px(4.)).children(left).child(div().flex_1()).children(buttons)
}

/// A key and its value inside a request's preview box.
fn kv(look: Look, key: &'static str, value: Div) -> Div {
    div()
        .flex()
        .gap(px(8.))
        .child(div().w(px(84.)).flex_shrink_0().text_color(c(look.secondary)).child(key))
        .child(value.flex_1().min_w_0())
}

/// The open request, as pages.rs draws it: who asks and where, the reason,
/// what will happen, the rule behind it, and the answers. None of these
/// offers an "Always allow" (D304, D305).
fn ask_card(
    look: Look,
    title: &'static str,
    from: &'static str,
    place: &'static str,
    reason: &'static str,
    preview: Vec<Div>,
    tail: Vec<Div>,
    because: &'static str,
    buttons: Vec<Div>,
) -> Div {
    card(look)
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .border_color(c(ORANGE))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(status(St::NeedsYou))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().flex_1())
                .child(secondary(look, "1m")),
        )
        .child(secondary(look, from))
        .child(
            div()
                .flex()
                .items_center()
                .child(secondary(look, place))
                .child(div().flex_1())
                .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Open session ↗")),
        )
        .child(div().child(reason))
        .child(div().flex().flex_col().gap(px(4.)).p(px(8.)).rounded(px(6.)).bg(c(look.fill)).text_size(px(11.5)).children(preview))
        .children(tail)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(5.))
                .text_size(px(11.5))
                .child(div().flex_none().text_color(c(look.tertiary)).child("Asks you because:"))
                .child(div().flex_1().min_w_0().child(because))
                .child(div().flex_none().text_color(c(look.accent)).child("Change this rule")),
        )
        .child(div().flex().gap(px(8.)).children(buttons))
}

/// The Inbox page: the list of `groups` beside the open request `open`.
fn inbox_page(look: Look, groups: Vec<Div>, open: Div) -> Div {
    page(
        look,
        Side::at("Inbox"),
        toolbar(look, "Inbox", Some("7 waiting"), vec![segmented(look, &["Waiting", "Answered", "All"], 0)]),
        div()
            .flex()
            .text_size(px(12.5))
            .child(
                div()
                    .w(px(330.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(c(look.line))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(10.))
                            .border_b_1()
                            .border_color(c(look.line))
                            .child(button(look, Kind::Secondary, "Allow all 7"))
                            .child(button(look, Kind::Secondary, "Deny all")),
                    )
                    .children(groups),
            )
            .child(div().flex_1().p(px(20.)).flex().flex_col().gap(px(12.)).child(open.max_w(px(560.)))),
    )
}

fn group_heading(look: Look, name: &'static str) -> Div {
    div().px(px(14.)).pt(px(10.)).pb(px(4.)).child(label(look, name))
}

// ------------------------------------------------------------- frames 87-90

/// Frame 87 (light): an agent proposes adding acme/web to the channel
/// (CH-26). The card has Allow and Deny, and the tick that adds the repo to
/// the proposing session too.
pub fn inbox_repo_proposal(look: Look) -> Div {
    let machines = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().child("This Mac"))
        .child(div().child("devbox-1"))
        .child(div().flex().gap(px(6.)).child("gpu-box").child(div().text_color(c(ORANGE)).child("offline: waits for gpu-box")));
    let open = ask_card(
        look,
        "Add acme/web to payments-rewrite",
        "Claude Code in Ledger schema migration · channel_add_repo",
        "devbox-1 · # payments-rewrite",
        "The checkout page in acme/web calls the old ledger endpoint. To move it to the new schema I need the repo here, and I can't read it from this session.",
        vec![
            kv(look, "Repo", div().font_weight(FontWeight::MEDIUM).child("acme/web")),
            kv(look, "Source", div().child("Saved repository")),
            kv(look, "Cloned on", machines),
        ],
        vec![checkbox(look, true, "Also add it to Ledger schema migration")],
        "Add a repo to a channel",
        vec![button(look, Kind::Primary, "Allow"), button(look, Kind::Secondary, "Deny"), div().flex_1(), button(look, Kind::Secondary, "Reply…")],
    );
    inbox_page(
        look,
        vec![
            group_heading(look, "# payments-rewrite"),
            inbox_row(look, "Add acme/web to payments-rewrite", "Claude Code · Ledger schema migration", "1m", true, true),
            inbox_row(look, "Move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", false, true),
            inbox_row(look, "Push try-brin to origin", "Claude Code · Try BRIN index", "9m", false, false),
            inbox_row(look, "Which currency for refunds?", "Copilot · Refund retries", "15m", false, false),
            group_heading(look, "# infra"),
            inbox_row(look, "Run a command on gpu-box", "Codex · Benchmark BRIN", "22m", false, false),
            inbox_row(look, "Delete the old snapshots", "Claude Code · Upgrade Postgres 17", "1h", false, false),
        ],
        open,
    )
}

/// Frame 88 (dark): a Desk session proposes moving into the channel
/// (DK-16). Its machine isn't in the channel, so the card has a picker. The
/// card offers Accept and Decline, never Always allow.
pub fn inbox_desk_move(look: Look) -> Div {
    let open = ask_card(
        look,
        "Move into payments-rewrite",
        "Claude Code in Plan Q4 · desk_session_move_to_channel",
        "laptop-2 · Desk",
        "The Q4 plan is settled and every item in it is for payments-rewrite. I'd like to keep working on the api repo from inside the channel.",
        vec![
            kv(look, "Channel", div().child("# payments-rewrite")),
            kv(
                look,
                "Repos",
                div()
                    .flex()
                    .flex_col()
                    .child(div().child("acme/api → uhvesta/plan-q4"))
                    .child(secondary(look, "starts from main · 2 uncommitted files kept")),
            ),
            kv(look, "Also adds", div().child("acme/web to the channel")),
            kv(
                look,
                "Runs on",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().flex().child(popup(look, "devbox-1")))
                    .child(secondary(look, "laptop-2 isn't in this channel. Pick one of its online machines.")),
            ),
        ],
        vec![
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(small_icon(IconName::ChevronRight, look.secondary))
                .child(secondary(look, "Details")),
            secondary(look, "Unfinished input and background jobs don't come along"),
        ],
        "Move a Desk session into a channel",
        vec![button(look, Kind::Primary, "Accept"), button(look, Kind::Secondary, "Decline"), div().flex_1(), button(look, Kind::Secondary, "Reply…")],
    );
    inbox_page(
        look,
        vec![
            group_heading(look, "Desk"),
            inbox_row(look, "Move into payments-rewrite", "Claude Code · Plan Q4", "3m", true, false),
            group_heading(look, "# payments-rewrite"),
            inbox_row(look, "Move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", false, true),
            inbox_row(look, "Push try-brin to origin", "Claude Code · Try BRIN index", "9m", false, false),
            inbox_row(look, "Which currency for refunds?", "Copilot · Refund retries", "15m", false, false),
            group_heading(look, "# infra"),
            inbox_row(look, "Run a command on gpu-box", "Codex · Benchmark BRIN", "22m", false, false),
        ],
        open,
    )
}

/// Frame 89 (light): the user's Move into a channel… dialog over a Desk
/// session (DK-15, DK-16): the channel, the folder, an editable branch per
/// repo, what it adds, where it runs.
pub fn move_into_channel(look: Look) -> Div {
    modal(
        look,
        desk::board_session(look),
        60.,
        600.,
        vec![
            heading(look, "Move “Plan Q4 board” into a channel", None),
            form_row(look, "Channel", div().flex().child(popup(look, "payments-rewrite"))),
            form_row(
                look,
                "Repos",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(small_icon(IconName::FolderGit2, look.secondary))
                            .child(div().font_weight(FontWeight::MEDIUM).child("acme/api"))
                            .child(secondary(look, "starts from main · 2 uncommitted files kept")),
                    )
                    .child(div().flex().items_center().gap(px(8.)).child(secondary(look, "Branch")).child(field(look, "uhvesta/plan-q4-board", false).flex_1())),
            ),
            form_row(look, "Also adds", div().pt(px(2.)).child("acme/web to the channel")),
            form_row(
                look,
                "Machine",
                div().flex().flex_col().gap(px(2.)).pt(px(2.)).child("This Mac").child(secondary(look, "It stays where it runs. The old folder goes to the trash after the move is checked.")),
            ),
            secondary(look, "Unfinished input and background jobs don't come along"),
            actions(None, vec![button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Move into payments-rewrite")]),
        ],
    )
}

/// Frame 90 (dark): the move refused (CV-3). Nothing was copied.
pub fn move_refused(look: Look) -> Div {
    modal(
        look,
        desk::board_session(look),
        110.,
        560.,
        vec![
            heading(look, "Move “Plan Q4 board” into payments-rewrite", None),
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(8.))
                .bg(c(look.attention_bg))
                .border_1()
                .border_color(c(ORANGE))
                .child(div().mt(px(1.)).child(icon(IconName::X, RED)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Can't carry this conversation"))
                        .child(div().child("Codex 0.171.0 stores it in a format Humata hasn't been checked against. Nothing was copied.")),
                ),
            secondary(look, "Start fresh opens a new session in payments-rewrite with a summary of this conversation. The session you have stays as it is."),
            actions(None, vec![button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Start fresh with a summary")]),
        ],
    )
}

// ------------------------------------------------------------- frames 91-95

/// The channel page header with its new tab row (CH-11): Sessions, Desk,
/// Inbox, Board, Repos and machines, Agent access, Activity. `lit` is the
/// open tab.
fn channel_header(look: Look, lit: &'static str) -> Div {
    let tab = |text: &'static str, count: Option<&'static str>| {
        let on = text == lit;
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .h_full()
            .px(px(2.))
            .border_b_2()
            .border_color(c(if on { look.accent } else { look.content }))
            .text_color(c(if on { look.text } else { look.secondary }))
            .when(on, |tab| tab.font_weight(FontWeight::MEDIUM))
            .child(text)
            .children(count.map(|count| badge(look, count, false)))
    };
    div()
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
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("# payments-rewrite"))
                .child(secondary(look, "acme/ledger, acme/api · This Mac, devbox-1"))
                .child(div().flex_1())
                .child(icon(IconName::Pin, look.secondary))
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
                .child(tab("Sessions", Some("11")))
                .child(tab("Desk", Some("2")))
                .child(tab("Inbox", Some("7")))
                .child(tab("Board", None))
                .child(tab("Repos and machines", None))
                .child(tab("Agent access", None))
                .child(tab("Activity", None)),
        )
}

/// The sidebar with the channel's Desk row and its two sessions (CD-2).
const fn desk_side(selected: &'static str) -> Side {
    Side { channel_desk: true, ..Side::at(selected) }
}

/// The channel's Desk tab, as frame 91 shows it.
fn desk_tab(look: Look) -> Div {
    let head = |text: &'static str| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.secondary)).child(text);
    let row = |state: St, title: &'static str, target: &'static str, agent: &'static str, machine: &'static str, words: &'static str, used: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(34.))
            .border_b_1()
            .border_color(c(look.line))
            .child(div().flex_1().min_w_0().flex().items_center().gap(px(7.)).child(status(state)).child(div().font_weight(FontWeight::MEDIUM).whitespace_nowrap().child(title)))
            .child(div().w(px(190.)).pr(px(12.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(target))
            .child(div().w(px(100.)).child(agent))
            .child(div().w(px(80.)).text_color(c(look.text)).child(machine))
            .child(div().w(px(80.)).text_color(c(look.secondary)).child(words))
            .child(div().w(px(90.)).text_color(c(look.secondary)).child(used))
    };
    page(
        look,
        desk_side("payments-rewrite"),
        channel_header(look, "Desk"),
        div()
            .p(px(20.))
            .text_size(px(12.5))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(secondary(look, "Short-lived sessions that look after this channel or one of its sessions."))
                    .child(div().flex_1())
                    .child(button(look, Kind::Primary, "New desk session")),
            )
            .child(
                card(look)
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(12.))
                            .h(px(30.))
                            .bg(c(look.panel))
                            .border_b_1()
                            .border_color(c(look.line))
                            .child(head("Session").flex_1())
                            .child(head("Target").w(px(190.)))
                            .child(head("Agent").w(px(100.)))
                            .child(head("Machine").w(px(80.)))
                            .child(head("Status").w(px(80.)))
                            .child(head("Last used ↓").w(px(90.))),
                    )
                    .child(row(St::Working, "Triage flaky tests", "This channel", "Claude Code", "This Mac", "Working", "Today 13:10"))
                    .child(row(St::Idle, "Watch the backfill", "Session: Ledger schema migration", "Codex", "devbox-1", "Idle", "Today 11:52")),
            )
            .child(secondary(look, "A desk session works in a scratch folder and has no clones of its own. Closing it sends the folder to the trash, unless it holds a saved version.")),
    )
}

/// Frame 91 (light): the channel's Desk tab, with the sidebar's Desk row.
pub fn channel_desk_tab(look: Look) -> Div {
    desk_tab(look)
}

/// One management switch (CD-3): its words, an optional detail, and the
/// switch. A blocked one says so and links to the rule.
fn switch_row(look: Look, on: bool, text: &'static str, detail: Option<&'static str>, blocked: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().flex().items_center().gap(px(10.)).child(div().flex_1().min_w_0().child(text)).child(div().when(blocked, |pill| pill.opacity(0.5)).child(switch(look, on))))
        .children(detail.map(|detail| secondary(look, detail)))
        .when(blocked, |col| {
            col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.5))
                    .child(div().text_color(c(ORANGE)).child("Blocked by the channel rule: Never"))
                    .child(div().text_color(c(look.accent)).child("Agent access")),
            )
        })
}

/// Frame 92 (light): New desk session over the Desk tab (CD-6), with the
/// five switches.
pub fn new_channel_desk(look: Look) -> Div {
    modal(
        look,
        desk_tab(look),
        50.,
        600.,
        vec![
            heading(look, "New desk session", Some("in # payments-rewrite")),
            form_row(look, "Title", field(look, "Watch the backfill", false)),
            form_row(look, "Runs", div().flex().child(segmented(look, &["Claude Code", "Codex", "Copilot", "OpenCode", "Terminal"], 1)).child(div().flex_1())),
            form_row(
                look,
                "Target",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().flex().child(segmented(look, &["This channel", "A session"], 1)).child(div().flex_1()))
                    .child(div().flex().child(popup(look, "Ledger schema migration")))
                    .child(secondary(look, "Covers that session and its forks, including forks made later.")),
            ),
            form_row(look, "Machine", div().flex().items_center().gap(px(8.)).child(popup(look, "devbox-1")).child(secondary(look, "Where the target session runs")).child(div().flex_1())),
            form_row(
                look,
                "May do",
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(switch_row(look, true, "Read its files", None, false))
                    .child(switch_row(look, true, "Send notes and questions", None, false))
                    .child(switch_row(look, false, "Type into its terminal", None, false))
                    .child(switch_row(look, false, "Start and fork sessions", None, false))
                    .child(switch_row(look, false, "Stop and archive sessions", None, false))
                    .child(switch_row(look, false, "Change anyone's items on the channel board", None, false)),
            ),
            secondary(look, "Anything left off follows the channel's rules. You can change these in its Info panel."),
            actions(None, vec![button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Start")]),
        ],
    )
}

fn info_kv(look: Look, key: &'static str, value: Div) -> Div {
    div().flex().gap(px(10.)).py(px(3.)).child(div().w(px(84.)).flex_shrink_0().text_color(c(look.secondary)).child(key)).child(value.flex_1().min_w_0())
}

/// A channel Desk session's Info panel (frame 93).
fn desk_info(look: Look) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .child(session::panel_header(look, "Info", false, None))
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
                        .child(info_kv(look, "Agent", div().child("Claude Code")))
                        .child(info_kv(look, "Machine", div().child("This Mac")))
                        .child(info_kv(look, "Target", div().child("This channel")))
                        .child(info_kv(look, "Folder", div().font_family(MONO).text_size(px(11.)).child("~/Humata/payments-rewrite/desk/triage-flaky-tests")))
                        .child(info_kv(look, "Reads", div().child("The main clones and shared/"))),
                )
                .child(
                    card(look)
                        .p(px(10.))
                        .gap(px(9.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("What this session may do"))
                        .child(switch_row(look, true, "Read its files", None, false))
                        .child(switch_row(look, true, "Send notes and questions", None, false))
                        .child(switch_row(look, false, "Type into its terminal", None, false))
                        .child(switch_row(look, false, "Start and fork sessions", None, false))
                        .child(switch_row(look, true, "Stop and archive sessions", None, true))
                        .child(switch_row(look, false, "Change anyone's items on the channel board", None, false))
                        .child(secondary(look, "Only you change these. Everything else follows the channel's rules.")),
                ),
        )
}

/// A channel Desk session's screen: header, scratch folder line, tabs and
/// terminal, with `info` as the right panel when given.
fn desk_session(look: Look, info: Option<Div>) -> Div {
    let place = div()
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
        .child(div().font_family(MONO).text_size(px(11.5)).child("~/Humata/payments-rewrite/desk/triage-flaky-tests"))
        .child(chip(look, "scratch", look.secondary))
        .child(div().flex_1())
        .child(div().text_color(c(look.accent)).child("Close session"));
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
    let lines = vec![
        tl(look.term_dim, "~/Humata/payments-rewrite/desk/triage-flaky-tests $ claude"),
        tl(look.term_text, "> Find the flaky tests in acme/api and write up what's failing."),
        tl(look.term_text, ""),
        tl(look.term_text, "● Read(../api/tests/ledger.rs)"),
        tl(look.term_dim, "  ⎿ Read 188 lines"),
        tl(look.term_text, "● Bash(gh run list --workflow ci --limit 20)"),
        tl(look.term_dim, "  ⎿ 5 of the last 20 runs failed on ledger::dual_write"),
        tl(look.term_text, "● Write(flaky-report.md)"),
        tl(look.term_dim, "  ⎿ Wrote 54 lines"),
        tl(look.term_text, "● Write(retry-plan.md)"),
        tl(look.term_dim, "  ⎿ Wrote 23 lines"),
        tl(look.term_text, ""),
        tl(ORANGE, "✻ Checking the timings… (31s · esc to interrupt)"),
    ];
    let term = div()
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
        .child(div().flex_1())
        .child(session::input_box(look, true));
    let middle = div()
        .child(session::header_with(look, "# payments-rewrite · Desk", St::Working, "Triage flaky tests", None, vec![secondary(look, "Claude Code · This Mac")]))
        .child(place)
        .child(tabs)
        .child(term);
    session::shell(look, desk_side("Triage flaky tests"), middle, info.map(|body| (Panel::Info, "Info", body)))
}

/// Frame 93 (dark): a channel Desk session's Info panel with the switches.
/// "Stop and archive sessions" is on, but the channel rule is Never.
pub fn desk_info_switches(look: Look) -> Div {
    desk_session(look, Some(desk_info(look)))
}

/// Frame 94 (light): closing a channel Desk session (CD-4). The scratch
/// folder goes to the trash now; its Markdown files can be copied out first.
pub fn desk_close(look: Look) -> Div {
    let file = |name: &'static str, size: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(7.))
            .border_b_1()
            .border_color(c(look.line))
            .child(small_icon(IconName::FileText, look.secondary))
            .child(div().flex_1().flex().gap(px(8.)).child(div().font_family(MONO).text_size(px(12.)).child(name)).child(secondary(look, size)))
            .child(button(look, Kind::Secondary, "Copy to shared/"))
    };
    modal(
        look,
        desk_session(look, None),
        110.,
        560.,
        vec![
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Close “Triage flaky tests”?"))
                .child(div().child("Its scratch folder goes to the trash now.")),
            div()
                .flex()
                .flex_col()
                .child(label(look, "Markdown files in it · 2"))
                .child(file("flaky-report.md", "54 lines"))
                .child(file("retry-plan.md", "23 lines")),
            secondary(look, "Its record and conversation stay in the trash for 30 days. You can restore them from there."),
            actions(None, vec![button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Close session")]),
        ],
    )
}

/// Frame 95 (dark): a session screen whose sidebar shows the channel's
/// Desk row, with its two sessions, above the session tree (CD-2).
pub fn sidebar_channel_desk(look: Look) -> Div {
    let screen = Screen { side: desk_side("Ledger schema migration"), ..Screen::new(None, Term::Working) };
    session::session(look, screen)
}

/// Frame 97 (light): a session mid-move to another machine (MV-4, MV-17).
/// Every step has its state, and the running copy step names its progress.
pub fn moving_copy_progress(look: Look) -> Div {
    use crate::fork_move::step;
    let steps = card(look)
        .gap(px(8.))
        .p(px(12.))
        .mt(px(10.))
        .font_family(SYSTEM_FONT)
        .text_size(px(12.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Moving to devbox-1"))
        .child(step(look, "\u{2713}", GREEN, "Paused the agent", false))
        .child(step(look, "\u{2713}", GREEN, "Put the clones on devbox-1 and carried 3 uncommitted files", false))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .whitespace_nowrap()
                .child(div().w(px(14.)).flex().justify_center().text_color(c(ORANGE)).child("\u{25CF}"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().text_color(c(look.text)).child("Copying the session (the rest is already there)"))
                        // Same bar as ui's steps_card: 62 of 118 MB.
                        .child(
                            div().w(px(180.)).h(px(3.)).rounded(px(2.)).bg(c(look.line)).child(
                                div().h_full().rounded(px(2.)).bg(c(look.accent)).w(relative(62. / 118.)),
                            ),
                        )
                        .child(div().text_color(c(look.secondary)).child("62 of 118 MB")),
                ),
        )
        .child(step(look, "\u{25CB}", look.tertiary, "Resume claude on devbox-1", true))
        .child(step(look, "\u{25CB}", look.tertiary, "Delete the copy on This Mac", true))
        .child(secondary(look, "If a step fails, the session keeps running on This Mac and nothing is deleted."));
    let tabs = div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(session::tab(look, Some(status(St::Working).into_any_element()), "claude", true, false))
        .child(div().flex_1())
        .child(div().mr(px(12.)).child(secondary(look, "read-only until the agent resumes")));
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
        .child(tl(look.term_dim, "plan-q4 $ claude"))
        .child(tl(look.term_dim, "\u{25CF} humata board_items channel=payments-rewrite"))
        .child(tl(look.term_dim, "  \u{23BF} 6 items \u{00B7} 1 in progress \u{00B7} 1 in review"))
        .child(tl(look.term_dim, ""))
        .child(steps)
        .child(div().flex_1())
        .child(div().opacity(0.5).child(session::input_box(look, false)))
        .child(div().pt(px(4.)).font_family(SYSTEM_FONT).text_size(px(11.5)).text_color(c(look.secondary)).child("Read-only until the agent resumes on devbox-1."));
    let middle = div()
        .child(session::header_with(
            look,
            "Desk",
            St::Working,
            "Plan Q4 board",
            Some(("Moving to devbox-1\u{2026}", ORANGE)),
            vec![secondary(look, "Claude Code \u{00B7} This Mac \u{2192} devbox-1")],
        ))
        .child(tabs)
        .child(body);
    session::shell(look, Side::at("Plan Q4 board"), middle, None)
}
