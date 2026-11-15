//! The Fork session dialog.

use super::{Overlays, field, sheet, sync};
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::{ForkKind, Session};
use humata2_session::{ForkDraft, SessionAction};
use std::rc::Rc;

impl Overlays {
    /// The Fork session dialog.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fork(
        &self,
        fork: ForkDraft,
        forking: bool,
        fork_error: Option<String>,
        parent: Option<Session>,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.fork_title, &fork.title, window, cx);
        let mut body = sheet("fork-session", "Fork session", look)
            .w(px(560.))
            .max_w_full()
            .p(px(20.))
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Fork session"),
            )
            .child(div().text_color(rgb(look.secondary)).child(format!(
                "From {} · now",
                parent.as_ref().map_or("this session", |s| s.title.as_str())
            )))
            .child(field(
                "Title",
                Input::new(&self.fork_title)
                    .disabled(forking)
                    .aria_label("Fork title"),
            ));
        for (index, choice, description) in [
            (
                0usize,
                ForkKind::Worktree,
                "Own copies of the current files, a new branch, and the conversation.",
            ),
            (
                1,
                ForkKind::Conversation,
                "The conversation and read-only access to the parent's files.",
            ),
            (
                2,
                ForkKind::Shared,
                "Own copies of the current files and a fresh conversation.",
            ),
        ] {
            body = body.child(
                col()
                    .p(px(10.))
                    .gap(px(5.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(if fork.kind == choice {
                        look.accent
                    } else {
                        look.line
                    }))
                    .child(
                        widgets::send(
                            ui,
                            SharedString::from(format!("fork-kind-{index}")),
                            choice.name(),
                            AppAction::Session(SessionAction::ForkKind(choice)),
                        )
                        .disabled(forking),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(look.secondary))
                            .child(description),
                    ),
            );
        }
        if let Some(parent) = parent {
            let repos = ui.read(|state| {
                state
                    .workspace
                    .session_files
                    .get(&parent.id)
                    .into_iter()
                    .flatten()
                    .map(|f| f.repo.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            let repos = if repos.is_empty() {
                parent.repo.as_str()
            } else {
                repos.as_str()
            };
            let preview = if fork.kind == ForkKind::Conversation {
                format!(
                    "{} · {} · reads {} on the parent branch {}",
                    parent.agent.name(),
                    parent.machine,
                    repos,
                    parent.branch
                )
            } else {
                format!(
                    "{} · {} · own copy of {} on a new branch",
                    parent.agent.name(),
                    parent.machine,
                    repos
                )
            };
            body = body.child(
                div()
                    .p(px(10.))
                    .rounded(px(7.))
                    .bg(rgb(look.fill))
                    .text_size(px(12.))
                    .child(preview),
            );
        }
        body = body
            .children(fork_error.map(|error| {
                div()
                    .id("fork-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xff453a))
                    .child(error)
            }))
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        widgets::send(
                            ui,
                            "fork-cancel",
                            "Cancel",
                            AppAction::Session(SessionAction::CancelFork),
                        )
                        .disabled(forking),
                    )
                    .child(
                        widgets::primary(
                            ui,
                            "fork-create",
                            if forking { "Forking…" } else { "Create fork" },
                            AppAction::Session(SessionAction::ConfirmFork),
                        )
                        .disabled(forking || fork.title.trim().is_empty())
                        .bg(rgb(look.accent))
                        .text_color(rgb(0xffffff)),
                    ),
            );
        body.test_support()
            .focus_trap("fork-trap", &self.sheet)
            .into_any_element()
    }
}
