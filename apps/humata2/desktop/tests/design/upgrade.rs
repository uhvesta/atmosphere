//! Migrate, the daemon upgrade as an Inbox item (frames 77 to 83; UPGRADE,
//! D281k; drafts): the card, its details, migrating, failed and migrated, the
//! machine page's banner and Health row in each state, the refusal inside a
//! session, and the holders item. "Migrate" is the one verb everywhere.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::fork_move::step;
use crate::kit::*;
use crate::machine_page::{self, Health, Machine};
use crate::pages::{inbox_row, page, toolbar};
use crate::session::{self, notice};
use crate::sidebar::{Link, Side};

/// Where the Migrate item stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Migrating,
    Failed,
    Migrated,
}

/// The purple VIP badge, as the Inbox list draws it.
fn vip() -> Div {
    div().px(px(4.)).rounded(px(3.)).bg(c(PURPLE)).text_size(px(9.5)).font_weight(FontWeight::BOLD).text_color(c(0xffffff)).child("VIP")
}

/// A card's first row: VIP, the Server icon where the status dot goes, the
/// title and its age.
fn title_row(look: Look, title: &'static str, when: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(vip())
        .child(small_icon(IconName::Server, look.accent))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(div().flex_1())
        .child(secondary(look, when))
}

/// "Asks you because:" and its reason (no rule link on a Migrate card).
fn asks(look: Look, why: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(5.))
        .text_size(px(11.5))
        .child(div().flex_none().text_color(c(look.tertiary)).child("Asks you because:"))
        .child(div().flex_1().min_w_0().child(why))
}

/// What the migration installs and restarts, with the folded list of what
/// changes; `open` unfolds it.
fn plan(look: Look, open: bool) -> Div {
    let line = |key: &'static str, text: &'static str| {
        div()
            .flex()
            .gap(px(8.))
            .child(div().w(px(70.)).flex_shrink_0().text_color(c(look.secondary)).child(key))
            .child(div().flex_1().min_w_0().child(text))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(8.))
        .rounded(px(6.))
        .bg(c(look.fill))
        .text_size(px(11.5))
        .child(line("Installs", "2.0.5 (a1b2c3d) over the SSH path that set gpu-box up"))
        .child(line("Restarts", "humatad on gpu-box, in place"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, look.secondary))
                .child("What changes · 4"),
        )
        .when(open, |plan| {
            plan.child(line("Daemon", "humatad 2.0.4 (9f3e2d1) → 2.0.5 (a1b2c3d)"))
                .child(line("CLI", "humata, same release"))
                .child(line("Store", "schema 41 → 43 (a copy is kept first)"))
                .child(line("Host tools", "systemd unit, ~/.local/bin/humata"))
        })
}

/// "What keeps running": the sessions stay, the link drops briefly.
fn keeps_running(look: Look) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(label(look, "What keeps running"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(small_icon(IconName::Check, GREEN))
                .child("3 sessions stay running (same holders)"),
        )
        .child(secondary(look, "The link to gpu-box drops for a few seconds"))
}

/// The steps while migrating, in frame 64's style.
fn steps(look: Look) -> Div {
    card(look)
        .gap(px(8.))
        .p(px(12.))
        .text_size(px(12.))
        .child(step(look, "✓", GREEN, "Copied 2.0.5 to gpu-box and checked it", false))
        .child(step(look, "✓", GREEN, "Kept a copy of the store", false))
        .child(step(look, "●", ORANGE, "Restarting humatad on gpu-box…", false))
        .child(step(look, "○", look.tertiary, "Migrate the store to schema 43", true))
        .child(step(look, "○", look.tertiary, "Wait for gpu-box to link at 2.0.5", true))
        .child(secondary(look, "If a step fails, 2.0.4 keeps running and nothing is lost."))
}

/// The failure block, in the style of the session's "Couldn't resume".
fn couldnt_migrate(look: Look) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(7.))
        .bg(c(look.fill))
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(small_icon(IconName::CircleX, RED))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Couldn't migrate the daemon on gpu-box")),
        )
        .child(div().child("2.0.5 failed to start twice, so 2.0.4 (9f3e2d1) is running again. Sessions kept running."))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().child("Why:"))
                .child(div().font_family(MONO).text_size(px(11.5)).child("~/.humata2/logs/humatad.log"))
                .child(div().child("on gpu-box"))
                .child(div().flex_1())
                .child(div().text_color(c(look.accent)).child("Open in Files")),
        )
        .child(div().child("Fix what the log names, then Migrate again. Migrating again without a fix fails the same way."))
}

/// The open Migrate request in `phase`; `open` unfolds "What changes".
fn migrate_card(look: Look, phase: Phase, open: bool) -> Div {
    let base = card(look).p(px(12.)).gap(px(8.)).text_size(px(12.5)).max_w(px(560.)).child(title_row(
        look,
        "Migrate the daemon on gpu-box",
        if phase == Phase::Migrated { "2m" } else { "1m" },
    ));
    let from = |base: Div| match phase {
        Phase::Failed => base.child(div().text_size(px(11.5)).text_color(c(RED)).child("From Humata · couldn't migrate")),
        _ => base.child(secondary(look, "From Humata · found when gpu-box linked")),
    };
    let base = from(base);
    if phase == Phase::Migrated {
        return base
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(small_icon(IconName::Check, GREEN))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Migrated to 2.0.5 (a1b2c3d)")),
            )
            .child(secondary(look, "2 min ago · 3 sessions kept running"));
    }
    base.border_color(c(if phase == Phase::Failed { RED } else { ORANGE }))
        .child(
            div()
                .flex()
                .items_center()
                .child(secondary(look, "gpu-box · Debian 13"))
                .child(div().flex_1())
                .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Open gpu-box ↗")),
        )
        .child(div().child(
            "gpu-box runs 2.0.4 (9f3e2d1); this app is 2.0.5 (a1b2c3d). Until it's migrated, gpu-box is read-only: its sessions keep running, but new sessions, typing and moves wait.",
        ))
        .child(match phase {
            Phase::Migrating => steps(look),
            Phase::Failed => couldnt_migrate(look),
            _ => plan(look, open),
        })
        .when(phase != Phase::Failed, |card| card.child(keeps_running(look)))
        .child(asks(look, "gpu-box runs another version, and Humata doesn't mix them."))
        .child(div().flex().gap(px(8.)).child(match phase {
            Phase::Migrating => button(look, Kind::Primary, "Migrating…").opacity(0.4),
            Phase::Failed => button(look, Kind::Primary, "Migrate again"),
            _ => button(look, Kind::Primary, "Migrate"),
        }))
}

/// The holders item: sessions still in 2.0.4's holder.
fn holders_card(look: Look) -> Div {
    let session = |state: St, name: &'static str, note: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(status(state))
            .child(div().font_weight(FontWeight::MEDIUM).child(name))
            .child(secondary(look, note))
    };
    card(look)
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .max_w(px(560.))
        .border_color(c(ORANGE))
        .child(title_row(look, "Migrate holders on gpu-box", "1m"))
        .child(secondary(look, "From Humata · found after gpu-box migrated to 2.0.5"))
        .child(div().child("3 sessions on gpu-box still run in 2.0.4's holder. They work as they are; new sessions start on 2.0.5 (a1b2c3d)."))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(label(look, "Sessions waiting · 2"))
                .child(session(St::Working, "Benchmark BRIN · Codex", "restarts when it's idle"))
                .child(session(St::Idle, "Embeddings sweep · Claude Code", "restarts now and resumes its conversation")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(label(look, "Stays until closed · 1"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(small_icon(IconName::SquareTerminal, look.secondary))
                        .child(div().font_weight(FontWeight::MEDIUM).child("zsh in CUDA 13 smoke test"))
                        .child(secondary(look, "stays on 2.0.4 until you close it"))
                        .child(div().flex_1())
                        .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Open session ↗")),
                ),
        )
        .child(asks(look, "each agent restarts once; Humata waits until it's idle."))
        .child(div().flex().gap(px(8.)).child(button(look, Kind::Primary, "Migrate")))
}

/// A list's group heading.
fn heading(look: Look, name: &'static str) -> Div {
    div().px(px(14.)).pt(px(10.)).pb(px(4.)).child(label(look, name))
}

/// The Inbox page: the list column (`rows`, after the Allow all strip) beside
/// the open `request`.
fn inbox(look: Look, side: Side, note: &'static str, tab: usize, rows: Vec<Div>, request: Div) -> Div {
    page(
        look,
        side,
        toolbar(look, "Agent inbox", Some(note), vec![segmented(look, &["Waiting", "Answered", "All"], tab)]),
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
                    .children(rows),
            )
            .child(div().flex_1().p(px(20.)).flex().flex_col().gap(px(12.)).child(request)),
    )
}

/// The waiting groups from frame 26, PAY-3 still VIP but not selected.
fn old_groups(look: Look) -> Vec<Div> {
    vec![
        heading(look, "# payments-rewrite"),
        inbox_row(look, "Move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", false, true),
        inbox_row(look, "Push try-brin to origin", "Claude Code · Try BRIN index", "9m", false, false),
        inbox_row(look, "Which currency for refunds?", "Copilot · Refund retries", "15m", false, false),
        heading(look, "# infra"),
        inbox_row(look, "Run a command on gpu-box", "Codex · Benchmark BRIN", "22m", false, false),
        inbox_row(look, "Delete the old snapshots", "Claude Code · Upgrade Postgres 17", "1h", false, false),
    ]
}

/// The Waiting list with the Humata group first and `row` selected in it.
fn waiting_list(look: Look, row: Div) -> Vec<Div> {
    let mut rows = vec![heading(look, "Humata"), row];
    rows.extend(old_groups(look));
    rows
}

fn waiting_side() -> Side {
    Side { gpu_box: Link::LinkedWaiting, ..Side::at("Agent inbox") }
}

fn migrate_row(look: Look, from: &'static str) -> Div {
    inbox_row(look, "Migrate the daemon on gpu-box", from, "1m", true, true)
}

fn waiting_page(look: Look, phase: Phase, open: bool, from: &'static str) -> Div {
    inbox(look, waiting_side(), "8 waiting", 0, waiting_list(look, migrate_row(look, from)), migrate_card(look, phase, open))
}

/// Frame 77: the Migrate request, selected first in the new Humata group.
pub fn inbox_migrate(look: Look) -> Div {
    waiting_page(look, Phase::Waiting, false, "Humata · gpu-box")
}

/// Frame 78: the same page with "What changes" unfolded.
pub fn details(look: Look) -> Div {
    waiting_page(look, Phase::Waiting, true, "Humata · gpu-box")
}

/// Frame 79, Inbox: migrating, with its steps.
pub fn migrating(look: Look) -> Div {
    waiting_page(look, Phase::Migrating, false, "Humata · migrating…")
}

/// Frame 80, Inbox: 2.0.4 is running again; fix, then Migrate again.
pub fn failed(look: Look) -> Div {
    waiting_page(look, Phase::Failed, false, "Humata · couldn't migrate")
}

/// Frame 81, Inbox: answered, with what it did.
pub fn migrated(look: Look) -> Div {
    let side = Side { gpu_box: Link::Linked, ..Side::at("Agent inbox") };
    let answered = |title: &'static str, from: &'static str, when: &'static str| inbox_row(look, title, from, when, false, false).opacity(0.55);
    let rows = vec![
        heading(look, "Humata"),
        inbox_row(look, "Migrate the daemon on gpu-box", "Humata · migrated", "2m", true, true).opacity(0.55),
        heading(look, "# payments-rewrite"),
        answered("Create PAY-9 Backfill progress bar", "Claude Code · Ledger schema migration · approved", "40m"),
        heading(look, "# infra"),
        answered("Open port 5432 on gpu-box", "Codex · Benchmark BRIN · denied: use the read replica", "2h"),
    ];
    inbox(look, side, "7 waiting", 1, rows, migrate_card(look, Phase::Migrated, false))
}

/// Frame 83: the holders item after gpu-box runs 2.0.5.
pub fn holders(look: Look) -> Div {
    let row = inbox_row(look, "Migrate holders on gpu-box", "Humata · gpu-box", "1m", true, true);
    inbox(look, waiting_side(), "8 waiting", 0, waiting_list(look, row), holders_card(look))
}

/// The shared progress line, drawn minimally: title and label over a 2 px
/// track filled to `fill` (`widgets::progress_line`).
fn count_line(look: Look, title: &'static str, label: &'static str, fill: f32) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_center()
                .w_full()
                .gap(px(6.))
                .text_size(px(11.))
                .whitespace_nowrap()
                .child(div().flex_1().min_w_0().truncate().text_color(c(look.secondary)).child(title))
                .child(div().flex_shrink_0().text_color(c(look.secondary)).child(label)),
        )
        .child(
            div()
                .w_full()
                .h(px(2.))
                .rounded(px(1.))
                .bg(c(look.line))
                .relative()
                .overflow_hidden()
                .child(div().h_full().rounded(px(1.)).bg(c(look.accent)).w(relative(fill))),
        )
}

/// Frame 125: the holders item (frame 83) while Migrate runs, with the count
/// of terminals moved so far.
pub fn holders_moving(look: Look) -> Div {
    let session = |state: St, name: &'static str, note: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(status(state))
            .child(div().font_weight(FontWeight::MEDIUM).child(name))
            .child(secondary(look, note))
    };
    let card = card(look)
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .max_w(px(560.))
        .border_color(c(ORANGE))
        .child(title_row(look, "Migrate holders on gpu-box", "1m"))
        .child(secondary(look, "From Humata · migrating…"))
        .child(div().child("7 terminals on gpu-box move onto 2.0.5's holder in place. Nothing restarts; each keeps its screen."))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(label(look, "Still to move · 3 sessions"))
                .child(session(St::Working, "Benchmark BRIN · Codex", "moving now"))
                .child(session(St::Idle, "Embeddings sweep · Claude Code", "next"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(small_icon(IconName::SquareTerminal, look.secondary))
                        .child(div().font_weight(FontWeight::MEDIUM).child("zsh in CUDA 13 smoke test"))
                        .child(secondary(look, "next")),
                ),
        )
        .child(count_line(look, "Moving Benchmark BRIN · Codex", "3 of 7 terminals moved", 3.0 / 7.0));
    let row = inbox_row(look, "Migrate holders on gpu-box", "Humata · gpu-box", "1m", true, true);
    inbox(look, waiting_side(), "8 waiting", 0, waiting_list(look, row), card)
}

const SUB_OLD: &str = "Debian 13 · x86_64 · gpu-box.lan (ssh) · humatad 2.0.4 (9f3e2d1)";
const SUB_NEW: &str = "Debian 13 · x86_64 · gpu-box.lan (ssh) · humatad 2.0.5 (a1b2c3d)";

/// gpu-box's page with the given words: the fixture everything shares.
fn gpu_box(subtitle: &'static str, dim: bool, banner: Option<&'static str>, health: &'static [Health]) -> Machine {
    Machine {
        name: "gpu-box",
        new_label: "New session on gpu-box",
        subtitle,
        dim_shell: dim,
        dim_new: dim,
        read_only: dim,
        sessions_heading: "Sessions on it · 3",
        sessions: &[
            (St::Working, "Benchmark BRIN", "# infra", "Codex"),
            (St::Working, "Embeddings sweep", "# infra", "Claude Code"),
            (St::Idle, "CUDA 13 smoke test", "# infra", "Claude Code"),
        ],
        folders: &[("# infra", "3 sessions · 4.2 GB")],
        health,
        banner,
    }
}

fn overview(look: Look, side: Side, machine: &Machine) -> Div {
    machine_page::overview(look, side, machine)
}

/// Frame 79, machine page: migrating, the session buttons off.
pub fn machine_migrating(look: Look) -> Div {
    let machine = gpu_box(
        SUB_OLD,
        true,
        Some("Migrating to 2.0.5 (a1b2c3d)… Sessions keep running; the link drops for a few seconds."),
        &[
            Health::Agents(&["claude 2.4", "codex 0.98"]),
            Health::Disk { used: 0.69, hex: GREEN, text: "62 GB free of 200 GB", note: None },
            Health::Variables(&["HF_TOKEN"]),
            Health::Note { key: "Humata", text: "2.0.4 (9f3e2d1) · migrating to 2.0.5 (a1b2c3d)…", hex: None, check: false, detail: None },
        ],
    );
    overview(look, waiting_side_machine(), &machine)
}

/// Frame 80, machine page: read-only after the failed try.
pub fn machine_read_only(look: Look) -> Div {
    let machine = gpu_box(
        SUB_OLD,
        true,
        Some("Read-only until migrated: new sessions, typing and moves wait. The last try failed."),
        &[
            Health::Agents(&["claude 2.4", "codex 0.98"]),
            Health::Disk { used: 0.69, hex: GREEN, text: "62 GB free of 200 GB", note: None },
            Health::Variables(&["HF_TOKEN"]),
            Health::Note { key: "Humata", text: "2.0.4 (9f3e2d1) · couldn't migrate to 2.0.5", hex: Some(RED), check: false, detail: None },
        ],
    );
    overview(look, waiting_side_machine(), &machine)
}

/// Frame 81, machine page: migrated, nothing dimmed.
pub fn machine_migrated(look: Look) -> Div {
    let machine = gpu_box(
        SUB_NEW,
        false,
        None,
        &[
            Health::Agents(&["claude 2.4", "codex 0.98"]),
            Health::Disk { used: 0.69, hex: GREEN, text: "62 GB free of 200 GB", note: None },
            Health::Variables(&["HF_TOKEN"]),
            Health::Note {
                key: "Humata",
                text: "Migrated to 2.0.5 (a1b2c3d) · 2 min ago",
                hex: None,
                check: true,
                detail: Some("humatad, CLI and host tools; store schema 41 → 43; 3 sessions kept running"),
            },
        ],
    );
    overview(look, Side { gpu_box: Link::Linked, ..Side::at("gpu-box") }, &machine)
}

fn waiting_side_machine() -> Side {
    Side { gpu_box: Link::LinkedWaiting, ..Side::at("gpu-box") }
}

/// Frame 82: typing into a session on an old machine is refused inline, not
/// as a toast (NV-10), with the way to the Migrate item.
pub fn migrate_first(look: Look) -> Div {
    let tabs = div()
        .flex()
        .items_center()
        .h(px(32.))
        .flex_shrink_0()
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(session::tab(look, Some(status(St::Working).into_any_element()), "codex", true))
        .child(session::tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false))
        .child(div().flex_1())
        .child(div().mr(px(12.)).child(secondary(look, "read-only until gpu-box is migrated")));
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
        .child(tl(look.term_text, "● Bash(pgbench -c 8 -T 600)"))
        .child(tl(look.term_dim, "  ⎿ progress: 240 s, 1830 tps (running)"))
        .child(notice(
            look,
            IconName::Inbox,
            ORANGE,
            "Migrate the daemon on gpu-box first. What you type isn't sent until then; the agent keeps running.",
            Some("Open in Inbox"),
        ))
        .child(div().flex_1())
        .child(div().opacity(0.5).child(session::input_box(look, false)));
    let middle = div()
        .child(session::header_with(look, "# infra", St::Working, "Benchmark BRIN", None, vec![secondary(look, "Codex · gpu-box")]))
        .child(tabs)
        .child(body);
    let side = Side { gpu_box: Link::LinkedWaiting, ..Side::at("Benchmark BRIN") };
    session::shell(look, side, middle, None)
}
