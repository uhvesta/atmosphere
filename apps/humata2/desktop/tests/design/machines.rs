//! Adding a machine: the five steps with a failure (frame 56; SPEC HO-1,
//! HO-2), the interactive login (frame 57; MP-12 to MP-16) and the offer to
//! make it unattended (frame 58; MP-17), drawn over the Hosts page.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::overlays::scrim;
use crate::pages;

/// The dialog's card, centred under the top of the window.
fn dialog(look: Look, body: Div) -> Div {
    pages::hosts(look).relative().child(scrim(look)).child(
        div().absolute().top(px(60.)).left_0().w_full().flex().justify_center().child(
            body.w(px(620.))
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

fn title(text: &'static str) -> Div {
    div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text)
}

fn buttons(look: Look, left: Option<Div>, items: Vec<Div>) -> Div {
    div().flex().items_center().gap(px(8.)).pt(px(4.)).children(left).child(div().flex_1()).children(items).text_color(c(look.text))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Done,
    Failed,
    NotRun,
}

/// One step: its marker, its name and a line under it.
fn step(look: Look, state: Step, name: &'static str, detail: &'static str) -> Div {
    let marker = match state {
        Step::Done => div().size(px(14.)).flex().items_center().justify_center().child(small_icon(IconName::Check, GREEN)),
        Step::Failed => div().size(px(14.)).flex().items_center().justify_center().child(small_icon(IconName::CircleX, RED)),
        Step::NotRun => div().size(px(14.)).flex().items_center().justify_center().child(div().size(px(9.)).rounded_full().border_1().border_color(c(look.tertiary))),
    };
    div()
        .flex()
        .items_start()
        .gap(px(9.))
        .when(state == Step::NotRun, |row| row.opacity(0.6))
        .child(div().pt(px(1.)).flex_shrink_0().child(marker))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .child(div().font_weight(FontWeight::MEDIUM).text_color(c(if state == Step::Failed { RED } else { look.text })).child(name))
                .child(secondary(look, detail)),
        )
}

fn steps(look: Look, rows: Vec<Div>) -> Div {
    div().flex().flex_col().gap(px(9.)).p(px(12.)).rounded(px(8.)).bg(c(look.fill)).children(rows)
}

fn mono_line(hex: u32, text: &'static str) -> Div {
    div().whitespace_nowrap().font_family(MONO).text_size(px(11.5)).text_color(c(hex)).child(text)
}

/// Frame 56: step 3 failed, so Humata stopped there (HO-1, HO-2).
pub fn add_machine(look: Look) -> Div {
    let failure = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(c(look.del_bg))
        .border_1()
        .border_color(c(RED))
        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(c(RED)).child("Install humatad failed on devbox-2"))
        .child(div().child("Humata can't write to ~/.local/bin on devbox-2: permission denied, because the folder belongs to root."))
        .child(div().child("To fix it, run this on devbox-2, then Try again:"))
        .child(
            div()
                .px(px(8.))
                .py(px(5.))
                .rounded(px(6.))
                .bg(c(look.term))
                .border_1()
                .border_color(c(look.line))
                .child(mono_line(look.term_text, "sudo chown -R $USER ~/.local/bin"))
                .child(mono_line(look.term_dim, "# run this on devbox-2, then Try again")),
        );
    let body = div()
        .child(title("Add machine · devbox-2"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().w(px(50.)).flex_shrink_0().text_color(c(look.secondary)).child("Host"))
                .child(popup(look, "devbox-2").w(px(180.)))
                .child(secondary(look, "from ~/.ssh/config").flex_shrink_0())
                .child(field(look, "or type a host", true).flex_1().min_w_0()),
        )
        .child(steps(
            look,
            vec![
                step(look, Step::Done, "Connect over SSH", "devbox-2 answered as avesta@10.0.4.22, host key accepted"),
                step(look, Step::Done, "Check the system (arch, disk, git, gh)", "Linux x86_64 · git 2.47 · gh 2.83 · 38 GB free"),
                step(look, Step::Failed, "Install humatad", "Stopped here. Nothing was installed."),
                step(look, Step::NotRun, "Start it and open the link", "Not run"),
                step(look, Step::NotRun, "Find agents", "Not run"),
            ],
        ))
        .child(failure)
        .child(buttons(look, None, vec![button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Try again")]));
    dialog(look, body)
}

/// Frame 57: the connect command runs in a real terminal inside the dialog
/// (MP-13 to MP-16).
pub fn add_machine_login(look: Look) -> Div {
    let terminal = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(c(look.term))
        .border_1()
        .border_color(c(look.line))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(17.))
        .child(tl(look.term_dim, "$ corp-ssh corp-vm -- 'echo HUMATA_READY_7f3a'"))
        .child(tl(look.term_text, "Opening a browser… or enter the code at"))
        .child(tl(look.term_text, "https://corp.example/device to sign in."))
        .child(tl(look.term_text, ""))
        .child(
            div()
                .flex()
                .items_center()
                .whitespace_nowrap()
                .text_color(c(look.term_text))
                .child("Verification code: ")
                .child(div().w(px(8.)).h(px(15.)).bg(c(look.term_text))),
        );
    let body = div()
        .child(title("Add machine · corp-vm"))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(div().pt(px(1.)).flex_shrink_0().child(small_icon(IconName::SquareTerminal, look.accent)))
                .child(div().child("corp-ssh is asking for a verification code, so Humata opened it here. Finish logging in, then press Done.")),
        )
        .child(terminal)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(c(look.attention_bg))
                .child(div().pt(px(1.)).flex_shrink_0().child(small_icon(IconName::Info, ORANGE)))
                .child(div().child("Reconnecting will need you: sessions keep running, but corp-vm can't be reached until you log in again.")),
        )
        .child(buttons(look, None, vec![button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Done, I'm logged in")]));
    dialog(look, body)
}

/// Frame 58: logged in; offer to stop needing the user (MP-17).
pub fn add_machine_unattended(look: Look) -> Div {
    let body = div()
        .child(title("Add machine · corp-vm"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(small_icon(IconName::Check, GREEN))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Logged in to corp-vm · humatad 2.0.4 running")),
        )
        .child(steps(
            look,
            vec![
                step(look, Step::Done, "Log in", "Finished in the terminal; corp-ssh accepted the code"),
                step(look, Step::Done, "Check the system (arch, disk, git, gh)", "Linux x86_64 · git 2.47 · gh 2.83 · 38 GB free"),
                step(look, Step::Done, "Install humatad", "humatad 2.0.4, checksum ok"),
                step(look, Step::Done, "Start it and open the link", "Linked through the terminal you logged in with"),
                step(look, Step::Done, "Find agents", "claude, codex"),
            ],
        ))
        .child(hline(look))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Reconnect without you?"))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(9.))
                .child(radio(look, true, "Add a Humata key to authorized_keys", "Humata makes its own key and adds it on corp-vm. Reconnects won't ask you."))
                .child(radio(look, false, "Keep corp-ssh's cached token", "Reconnects work while the token is valid, then need you again."))
                .child(radio(look, false, "Keep logging in by hand", "Reconnects need you: corp-vm is unreachable until you log in.")),
        )
        .child(buttons(look, None, vec![button(look, Kind::Secondary, "Not now"), button(look, Kind::Primary, "Make it unattended")]));
    dialog(look, body)
}
