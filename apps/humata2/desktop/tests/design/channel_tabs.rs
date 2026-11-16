//! The channel's Inbox tab (frame 48) and Agent access tab (frame 49):
//! IN-1, IN-3, IN-5 to IN-7 and IN-16 to IN-19, SM-19.

use gpui_kit::{prelude::*, *};

use crate::channel::{Tab, frame};
use crate::kit::*;
use crate::pages::{inbox_row, request_card};

/// Frame 48: the channel's own requests, the first one open.
pub fn inbox(look: Look) -> Div {
    frame(
        look,
        Tab::Inbox,
        div()
            .size_full()
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
                            .flex_col()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(10.))
                            .border_b_1()
                            .border_color(c(look.line))
                            .child(segmented(look, &["Waiting", "Answered", "All"], 0))
                            .child(div().flex().gap(px(8.)).child(button(look, Kind::Secondary, "Allow all 3")).child(button(look, Kind::Secondary, "Deny all"))),
                    )
                    .child(inbox_row(look, "Move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", true, true))
                    .child(inbox_row(look, "Push try-brin to origin", "Claude Code · Try BRIN index", "9m", false, false))
                    .child(inbox_row(look, "Which currency for refunds?", "Copilot · Refund retries", "15m", false, false)),
            )
            .child(div().flex_1().p(px(20.)).flex().flex_col().gap(px(12.)).child(request_card(look, true).max_w(px(560.)))),
    )
}

/// Frame 49: what agents in the channel may do, one switch per rule.
pub fn access(look: Look) -> Div {
    // Default of each rule: 0 Allow, 1 Ask me, 2 Never.
    let rules: &[(&'static str, usize)] = &[
        ("Its own session board: add, edit, move, delete", 0),
        ("See its channel: machines, sessions, the channel board", 0),
        ("Channel board: add items", 0),
        ("Channel board: edit, move or delete items it created", 0),
        ("Channel board: edit, move or delete anyone else's items", 1),
        ("Global board, or other channels and their boards: read", 0),
        ("Global board, or other channels' boards: change", 1),
        ("Write into shared/ on its machine", 0),
        ("Fetch files from shared/, the channel folder or a main clone into its own clone", 0),
        ("Prepare views for you", 0),
        ("Draft PR review comments", 0),
        ("Ask you a question, or ask ahead for access", 0),
        ("Read another session's files (same machine)", 1),
        ("Read files on another machine", 1),
        ("Copy files into its clone or shared/ from another session or machine", 1),
        ("Send a note or question to another session", 1),
        ("Type into another session's terminal", 1),
        ("Start or fork sessions (same machine)", 1),
        ("Start or fork sessions on another machine", 1),
        ("Start a PR review", 1),
        ("Change its own session", 1),
        ("Stop, archive or delete its own session", 1),
        ("Move a session to another machine", 1),
        ("Push its branch or open a pull request", 1),
        ("Open a network connection through the host", 1),
        ("Add, create, start or stop machines; sync skills", 1),
        ("Restore from trash", 1),
        ("Create a channel", 1),
        ("Stop or delete any session but itself", 2),
        ("Change channel settings or repos", 2),
        ("Remove, forget or delete machines", 2),
    ];
    let row = |text: &'static str, control: Div| {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(27.))
            .px(px(12.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(c(look.line))
            .child(div().flex_1().min_w_0().whitespace_nowrap().child(text))
            .child(control)
    };
    let always = || div().text_color(c(look.tertiary)).child("Always · no setting");
    let mut rows = vec![row("Read and write its own clones", always())];
    for (text, default) in rules {
        rows.push(row(text, segmented(look, &["Allow", "Ask me", "Never"], *default)));
        if *text == "Draft PR review comments" {
            // D128.2: the daemon never checks this one, so it has no setting.
            rows.push(row("Resolve comments sent to its own session, with a reply", always()));
        }
        if *text == "Read another session's files (same machine)" {
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(24.))
                    .pl(px(28.))
                    .pr(px(12.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(c(look.line))
                    .child(div().flex_1().text_size(px(11.5)).text_color(c(look.tertiary)).child("Always allow · Try BRIN index, only under ledger/docs/ · granted 12m ago"))
                    .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Remove")),
            );
        }
    }
    frame(
        look,
        Tab::Access,
        div().p(px(20.)).text_size(px(12.5)).flex().flex_col().gap(px(12.)).child(
            div()
                .max_w(px(820.))
                .text_size(px(11.5))
                .text_color(c(look.secondary))
                .child("What agents in this channel may do through Humata. Anything set to Ask me lands in your Inbox, batched with every other agent's requests. Never tells the agent no at once, naming the rule, and adds nothing to the Inbox."),
        )
        .child(card(look).flex_shrink_0().overflow_hidden().children(rows)),
    )
}
