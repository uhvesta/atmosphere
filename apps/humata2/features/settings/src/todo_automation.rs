//! Settings › General › To-do (TODO2WIRE, D902): what Humata does by itself
//! when pull requests and items change. A change shows at once, the whole
//! section is saved, and a refusal puts it back. It applies with no restart.

use humata2_clients::daemon::{TodoAutomation, TodoMode};
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// What the daemon last said about To-do automation, with changes shown at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodoAutomationState {
    /// `None` until the daemon has said.
    pub section: Option<TodoAutomation>,
    /// Whether the idle-nudge list is open.
    pub idle_menu: bool,
}

/// What happens on the To-do group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoAutomationAction {
    /// Read the section from the daemon.
    Load,
    /// The daemon gave the section, or couldn't.
    Loaded(Result<TodoAutomation, DaemonError>),
    /// Choose Ask or Auto for a merged pull request.
    SetOnPrMerged(TodoMode),
    /// Choose Ask or Auto for archiving the sessions of a closed item.
    SetOnClosedArchive(TodoMode),
    /// Wake the agent when checks fail, or not.
    ToggleWakeOnCiFail,
    /// Wake the agent on review comments, or not.
    ToggleWakeOnReviewComments,
    /// Open or close the idle-nudge list.
    IdleMenu,
    /// Pick the days of inactivity before an idle nudge, and close the list.
    SetIdleNudgeDays(u32),
    /// The daemon saved a change, or refused.
    Saved {
        /// What was shown before the change, to put back on a refusal.
        before: TodoAutomation,
        /// What the daemon holds now, or why it refused.
        result: Result<TodoAutomation, DaemonError>,
    },
}

/// Applies `action`, calling the daemon through `clients`. `error` is the
/// settings page's error line.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut TodoAutomationState,
    error: &mut Option<String>,
    action: TodoAutomationAction,
) -> Effect<TodoAutomationAction> {
    match action {
        TodoAutomationAction::Load => {
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                TodoAutomationAction::Loaded(daemon.todo_automation().await)
            })
        }
        TodoAutomationAction::Loaded(result) => {
            match result {
                Ok(section) => state.section = Some(section.normalized()),
                Err(failure) => *error = Some(failure.to_string()),
            }
            Effect::none()
        }
        TodoAutomationAction::SetOnPrMerged(mode) => {
            change(clients, state, |section| section.on_pr_merged = mode)
        }
        TodoAutomationAction::SetOnClosedArchive(mode) => {
            change(clients, state, |section| section.on_todo_closed_archive_sessions = mode)
        }
        TodoAutomationAction::ToggleWakeOnCiFail => change(clients, state, |section| {
            section.auto_wake_on_ci_fail = !section.auto_wake_on_ci_fail;
        }),
        TodoAutomationAction::ToggleWakeOnReviewComments => change(clients, state, |section| {
            section.auto_wake_on_review_comments = !section.auto_wake_on_review_comments;
        }),
        TodoAutomationAction::IdleMenu => {
            state.idle_menu = !state.idle_menu;
            Effect::none()
        }
        TodoAutomationAction::SetIdleNudgeDays(days) => {
            state.idle_menu = false;
            change(clients, state, |section| section.idle_nudge_days = days)
        }
        TodoAutomationAction::Saved { before, result } => {
            if let Err(failure) = result {
                state.section = Some(before);
                *error = Some(failure.to_string());
            }
            Effect::none()
        }
    }
}

/// Applies `edit` to the section at once and saves the whole section.
fn change(
    clients: &Clients,
    state: &mut TodoAutomationState,
    edit: impl FnOnce(&mut TodoAutomation),
) -> Effect<TodoAutomationAction> {
    let Some(section) = state.section.as_mut() else {
        return Effect::none();
    };
    let before = *section;
    edit(section);
    let saving = *section;
    let daemon = clients.daemon.clone();
    Effect::future(async move {
        TodoAutomationAction::Saved { before, result: daemon.set_todo_automation(saving).await }
    })
}
