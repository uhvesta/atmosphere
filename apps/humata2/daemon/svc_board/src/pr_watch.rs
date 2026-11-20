//! PR watcher & session lifecycle prompt helpers (TODO2).

use humata2_todo::State;

use super::todo_fields::{
    CloseReadinessRow, LifecyclePromptRow, SessionSignalRow, StepRow, WatchedPrRow,
};

/// Evaluates a watched PR's state and emits quiet 1-click lifecycle prompts.
pub(super) fn evaluate_watched_pr(pr: &WatchedPrRow) -> Vec<LifecyclePromptRow> {
    let pr_ref = pr.repo_pr();
    let mut prompts = Vec::new();
    if pr.merged {
        prompts.push(LifecyclePromptRow {
            id: format!("pr-merged:{pr_ref}"),
            kind: "pr_merged".to_owned(),
            title: format!("PR {pr_ref} merged — close to-do?"),
            detail: pr.title.clone(),
            choices: vec!["Close".to_owned(), "Keep open".to_owned()],
            pr_ref: pr_ref.clone(),
            session: None,
        });
        return prompts;
    }
    if pr.closed_without_merge {
        prompts.push(LifecyclePromptRow {
            id: format!("pr-closed:{pr_ref}"),
            kind: "pr_closed".to_owned(),
            title: format!("PR {pr_ref} closed without merge"),
            detail: pr.title.clone(),
            choices: vec!["Won't do".to_owned(), "Keep open".to_owned()],
            pr_ref: pr_ref.clone(),
            session: None,
        });
        return prompts;
    }
    let ci_lower = pr.ci_state.to_lowercase();
    if matches!(ci_lower.as_str(), "failed" | "failing" | "failure" | "error") {
        prompts.push(LifecyclePromptRow {
            id: format!("pr-ci:{pr_ref}"),
            kind: "pr_ci_failed".to_owned(),
            title: format!("CI failed on {pr_ref}"),
            detail: format!("CI status: {}", pr.ci_state),
            choices: vec!["Send to session".to_owned(), "Dismiss".to_owned()],
            pr_ref: pr_ref.clone(),
            session: None,
        });
    }
    if pr.unresolved_comments > 0 {
        let word = if pr.unresolved_comments == 1 {
            "comment"
        } else {
            "comments"
        };
        prompts.push(LifecyclePromptRow {
            id: format!("pr-comments:{pr_ref}"),
            kind: "pr_comments".to_owned(),
            title: format!("{} unresolved review {word} on {pr_ref}", pr.unresolved_comments),
            detail: pr.title.clone(),
            choices: vec![
                "Send comments to session".to_owned(),
                "Dismiss".to_owned(),
            ],
            pr_ref: pr_ref.clone(),
            session: None,
        });
    }
    if pr.has_conflicts {
        prompts.push(LifecyclePromptRow {
            id: format!("pr-conflicts:{pr_ref}"),
            kind: "pr_conflicts".to_owned(),
            title: format!("Merge conflicts on {pr_ref}"),
            detail: pr.title.clone(),
            choices: vec!["Send to session".to_owned(), "Dismiss".to_owned()],
            pr_ref: pr_ref.clone(),
            session: None,
        });
    }
    if pr.upstream_merged {
        prompts.push(LifecyclePromptRow {
            id: format!("pr-stack-rebase:{pr_ref}"),
            kind: "pr_stack_rebase".to_owned(),
            title: format!("Upstream stack PR merged — rebase {pr_ref}?"),
            detail: pr.stack_key.clone(),
            choices: vec!["Send to session".to_owned(), "Dismiss".to_owned()],
            pr_ref,
            session: None,
        });
    }
    prompts
}

/// Emits a soft idle nudge prompt when a session has been idle past `threshold_days`.
pub(super) fn evaluate_session_idle(
    signal: &SessionSignalRow,
    threshold_days: u32,
) -> Option<LifecyclePromptRow> {
    if signal.active_turn || signal.idle_days < threshold_days {
        return None;
    }
    Some(LifecyclePromptRow {
        id: format!("session-idle:{}", signal.session),
        kind: "session_idle".to_owned(),
        title: format!("Idle {}d — resume or pause?", signal.idle_days),
        detail: signal.label.clone(),
        choices: vec!["Resume".to_owned(), "Pause".to_owned(), "Dismiss".to_owned()],
        pr_ref: String::new(),
        session: Some(signal.session.clone()),
    })
}

/// Emits a prompt when the last active session on an `InProgress` item is archived.
pub(super) fn evaluate_last_session_archived(
    item_key: &str,
    state: State,
    remaining_active_sessions: usize,
) -> Option<LifecyclePromptRow> {
    if state != State::InProgress || remaining_active_sessions > 0 {
        return None;
    }
    Some(LifecyclePromptRow {
        id: format!("last-session-archived:{item_key}"),
        kind: "last_session_archived".to_owned(),
        title: format!("Last session archived on {item_key}"),
        detail: "Close this item or return it to Open?".to_owned(),
        choices: vec![
            "Close".to_owned(),
            "Return to Open".to_owned(),
            "Dismiss".to_owned(),
        ],
        pr_ref: String::new(),
        session: None,
    })
}

/// Computes a compact `CloseReadinessRow` from active steps, watched PRs, and commit count.
pub(super) fn compute_close_readiness(
    steps: &[StepRow],
    watched_prs: &[WatchedPrRow],
    commits: u32,
) -> CloseReadinessRow {
    let active_steps: Vec<&StepRow> = steps.iter().filter(|s| !s.dropped).collect();
    let steps_total = u32::try_from(active_steps.len()).unwrap_or(u32::MAX);
    let steps_done = u32::try_from(active_steps.iter().filter(|s| s.done).count()).unwrap_or(u32::MAX);
    let unresolved_comments = watched_prs
        .iter()
        .fold(0u32, |acc, pr| acc.saturating_add(pr.unresolved_comments));
    let ci_status = watched_prs
        .iter()
        .map(|pr| pr.ci_state.as_str())
        .find(|s| matches!(s.to_lowercase().as_str(), "failed" | "failing" | "failure"))
        .or_else(|| {
            watched_prs
                .iter()
                .map(|pr| pr.ci_state.as_str())
                .find(|s| !s.is_empty())
        })
        .unwrap_or("")
        .to_owned();
    CloseReadinessRow {
        steps_done,
        steps_total,
        unresolved_comments,
        ci_status,
        commits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watched_pr_prompts_cover_merge_ci_comments_conflicts_and_stack() {
        let merged = WatchedPrRow {
            owner: "acme".into(),
            name: "api".into(),
            number: 42,
            merged: true,
            ..WatchedPrRow::default()
        };
        let p = evaluate_watched_pr(&merged);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].kind, "pr_merged");

        let noisy = WatchedPrRow {
            owner: "acme".into(),
            name: "api".into(),
            number: 43,
            ci_state: "failed".into(),
            unresolved_comments: 2,
            has_conflicts: true,
            upstream_merged: true,
            ..WatchedPrRow::default()
        };
        let noisy_prompts = evaluate_watched_pr(&noisy);
        let kinds: Vec<&str> = noisy_prompts.iter().map(|x| x.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["pr_ci_failed", "pr_comments", "pr_conflicts", "pr_stack_rebase"]
        );
    }

    #[test]
    fn idle_session_and_last_archived_emit_expected_prompts() {
        let idle = SessionSignalRow {
            session: "se_1".into(),
            label: "claude".into(),
            status: "idle".into(),
            idle_days: 4,
            active_turn: false,
        };
        assert!(evaluate_session_idle(&idle, 3).is_some());
        assert!(evaluate_session_idle(&idle, 5).is_none());
        assert!(evaluate_last_session_archived("PAY-1", State::InProgress, 0).is_some());
        assert!(evaluate_last_session_archived("PAY-1", State::InProgress, 1).is_none());
        assert!(evaluate_last_session_archived("PAY-1", State::Open, 0).is_none());
    }

    #[test]
    fn close_readiness_ignores_dropped_steps_and_aggregates_prs() {
        let steps = vec![
            StepRow {
                text: "a".into(),
                done: true,
                ..StepRow::default()
            },
            StepRow {
                text: "b".into(),
                done: false,
                dropped: true,
                ..StepRow::default()
            },
            StepRow {
                text: "c".into(),
                done: false,
                ..StepRow::default()
            },
        ];
        let prs = vec![WatchedPrRow {
            ci_state: "passing".into(),
            unresolved_comments: 3,
            ..WatchedPrRow::default()
        }];
        let cr = compute_close_readiness(&steps, &prs, 2);
        assert_eq!(cr.steps_done, 1);
        assert_eq!(cr.steps_total, 2);
        assert_eq!(cr.unresolved_comments, 3);
        assert_eq!(cr.ci_status, "passing");
        assert_eq!(cr.commits, 2);
    }
}
