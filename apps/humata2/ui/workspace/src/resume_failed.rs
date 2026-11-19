//! A session whose agent a host restart interrupted: the reboot line under
//! its old scrollback and, when the resume failed, the card that names the
//! reason and the two ways on (PER-15, PER-17, PER-29, frame 46).
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, RestartAction};
use humata2_clients::model::{ResumeFailure, Session, SummarySource};

use crate::{
    bridge::Ui,
    chrome,
    theme::{Look, RED},
    widgets::{self, col, row},
};

/// "09:12" for a Unix time in ms (UTC; the frames show no zone).
pub fn clock(unix_ms: i64) -> String {
    let minutes = unix_ms.div_euclid(60_000);
    format!(
        "{:02}:{:02}",
        minutes.div_euclid(60).rem_euclid(24),
        minutes.rem_euclid(60)
    )
}

/// Why the resume failed and what that means, in words for the person.
fn reason(session: &Session, failure: ResumeFailure, host: &str) -> String {
    let agent = session.agent.command();
    match failure {
        ResumeFailure::ConversationMissing => format!(
            "{agent} can't find this conversation's file on {host}. Nothing was typed into it."
        ),
        ResumeFailure::OtherConversation => format!(
            "{agent} started but opened a new conversation instead of this one on {host}. Nothing was typed into it."
        ),
        ResumeFailure::AgentExited(code) => format!(
            "{agent} exited with code {code} while resuming on {host}. Nothing was typed into it."
        ),
        ResumeFailure::AgentNotInstalled => format!(
            "{agent} isn't installed on {host} any more. Install it there, then try again."
        ),
        ResumeFailure::ConversationUnknown => format!(
            "{agent} on {host} was adopted, and its conversation isn't known, so it can't be resumed. End it for good, or keep it running."
        ),
        ResumeFailure::FolderMissing => format!(
            "This session's folder is gone from {host}. Restore it there, then try again, or start fresh."
        ),
    }
}

/// The Start fresh button's words: it says when there is nothing left of the
/// old conversation to summarize, so the person knows what the new one starts
/// from.
pub fn start_fresh_label(source: Option<SummarySource>) -> &'static str {
    match source {
        Some(SummarySource::Nothing) => "Start fresh (nothing left to summarize)",
        _ => "Start fresh with a summary",
    }
}

/// The line under the old scrollback that marks the restart.
pub fn reboot_line(state: &AppState, session: &Session, look: Look) -> Option<AnyElement> {
    let at = session.interrupted_unix_ms?;
    let host = state
        .workspace
        .machine(&session.machine)
        .map_or("The host", |m| m.name.as_str());
    let rule = || div().flex_1().h(px(1.)).bg(rgb(look.line));
    Some(
        row()
            .id("reboot-line")
            .role(Role::Status)
            .aria_label(format!("{host} restarted at {}", clock(at)))
            .mt(px(10.))
            .gap(px(8.))
            .text_size(px(11.5))
            .text_color(rgb(look.secondary))
            .child(rule())
            .child(format!("{host} restarted at {}", clock(at)))
            .child(rule())
            .test_support()
            .into_any_element(),
    )
}

/// The card for a failed resume: the reason, Try again, Start fresh with a
/// summary, and the link to the restart report. `None` when it didn't fail.
pub fn note(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    let failure = session.resume_failed?;
    let host = state.workspace.machine(&session.machine);
    let host_name = host.map_or("the host", |m| m.name.as_str());
    let report_time = state
        .workspace
        .restarts
        .iter()
        .find(|restart| restart.machine == session.machine)
        .map(|restart| clock(restart.at_unix_ms));
    let id = session.id.clone();
    let machine = session.machine.clone();
    let open = ui.clone();
    let words = reason(session, failure, host_name);
    Some(
        col()
            .id("couldnt-resume")
            .role(Role::Group)
            .aria_label("Couldn't resume")
            .mt(px(10.))
            .gap(px(6.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .text_size(px(12.))
            .text_color(rgb(look.text))
            .child(
                row()
                    .gap(px(8.))
                    .child(chrome::icon(IconName::CircleX, RED, 12.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Couldn't resume")),
            )
            .child(div().child(words.clone()).test_text(words))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(2.))
                    .child(
                        widgets::primary_in(look,
                            ui,
                            "resume-try-again",
                            "Try again",
                            AppAction::Restart(RestartAction::TryAgain(id.clone())),
                        )
                        .h(px(24.))
                        .px(px(11.)),
                    )
                    .child(
                        widgets::send(
                            ui,
                            "resume-start-fresh",
                            start_fresh_label(session.summary_source),
                            AppAction::Restart(RestartAction::StartFresh(id)),
                        )
                        .outline()
                        .h(px(24.))
                        .px(px(11.)),
                    )
                    .child(div().flex_1())
                    .children(report_time.map(|time| {
                        let label = format!("Restart report {time}");
                        row()
                            .id("restart-report-link")
                            .role(Role::Link)
                            .aria_label(label.clone())
                            .gap(px(4.))
                            .cursor_pointer()
                            .text_color(rgb(look.accent))
                            .on_click(move |_, _, cx| {
                                open.send(
                                    AppAction::Restart(RestartAction::OpenReport(machine.clone())),
                                    cx,
                                )
                            })
                            .child(chrome::icon(IconName::FileText, look.secondary, 12.))
                            .child(label)
                            .test_support()
                    })),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Named, not `super::*`: the module's gpui glob brings in gpui's own
    // `test` attribute, which would shadow the built-in one.
    use super::{SummarySource, start_fresh_label};

    #[test]
    fn only_a_conversation_with_nothing_left_changes_the_button() {
        let plain = start_fresh_label(None);
        assert_eq!(start_fresh_label(Some(SummarySource::Transcript)), plain);
        assert_eq!(start_fresh_label(Some(SummarySource::Scrollback)), plain);
        assert_ne!(start_fresh_label(Some(SummarySource::Nothing)), plain);
    }
}
