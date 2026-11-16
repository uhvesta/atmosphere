use std::collections::BTreeMap;

use crate::model::{
    AccessRule, Capability, ChannelId, Grant, GrantId, RuleSetting, SessionId,
};

use RuleSetting::{Allow, Ask, Never};

/// The IN-17 rows in order: capability, label, default. `None` is a row
/// with no setting: the IN-16 row, and comment resolving (D128.2).
const ROWS: &[(&str, &str, Option<RuleSetting>)] = &[
    ("own_clones", "Read and write its own clones", None),
    ("own_board_write", "Its own session board: add, edit, move, delete", Some(Allow)),
    ("channel_read", "See its channel: machines, sessions, the channel board", Some(Allow)),
    ("channel_board_add", "Channel board: add items", Some(Allow)),
    ("channel_board_edit_own", "Channel board: edit, move or delete items it created", Some(Allow)),
    ("channel_board_edit_others", "Channel board: edit, move or delete anyone else's items", Some(Ask)),
    ("global_read", "Global board, or other channels and their boards: read", Some(Allow)),
    // D155: writes there ask by default.
    ("global_board_write", "Global board, or other channels' boards: change", Some(Ask)),
    ("shared_write", "Write into shared/ on its machine", Some(Allow)),
    (
        "files_fetch_local",
        "Fetch files from shared/, the channel folder or a main clone into its own clone",
        Some(Allow),
    ),
    ("views_prepare", "Prepare views for you", Some(Allow)),
    ("review_draft", "Draft PR review comments", Some(Allow)),
    // No setting either (D128.2): the daemon never checks this capability.
    (
        "own_comments_read",
        "Resolve comments sent to its own session, with a reply",
        None,
    ),
    ("inbox_ask", "Ask you a question, or ask ahead for access", Some(Allow)),
    ("files_read_other_session", "Read another session's files (same machine)", Some(Ask)),
    ("files_read_other_machine", "Read files on another machine", Some(Ask)),
    (
        "files_copy_in",
        "Copy files into its clone or shared/ from another session or machine",
        Some(Ask),
    ),
    ("session_note", "Send a note or question to another session", Some(Ask)),
    ("session_type", "Type into another session's terminal", Some(Ask)),
    ("session_start_same_machine", "Start or fork sessions (same machine)", Some(Ask)),
    ("session_start_other_machine", "Start or fork sessions on another machine", Some(Ask)),
    ("pr_review_start", "Start a PR review", Some(Ask)),
    ("own_session_manage", "Change its own session", Some(Ask)),
    ("own_session_end", "Stop, archive or delete its own session", Some(Ask)),
    ("session_move", "Move a session to another machine", Some(Ask)),
    ("git_push", "Push its branch or open a pull request", Some(Ask)),
    ("network_reach", "Open a network connection through the host", Some(Ask)),
    ("machine_provision", "Add, create, start or stop machines; sync skills", Some(Ask)),
    ("trash_restore", "Restore from trash", Some(Ask)),
    ("channel_create", "Create a channel", Some(Ask)),
    ("session_stop_other", "Stop or delete any session but itself", Some(Never)),
    ("channel_change", "Change channel settings or repos", Some(Never)),
    ("machine_destroy", "Remove, forget or delete machines", Some(Never)),
];

fn defaults() -> Vec<AccessRule> {
    ROWS.iter()
        .map(|(capability, label, setting)| AccessRule {
            capability: Capability::new(*capability),
            label: (*label).to_owned(),
            setting: *setting,
            grants: Vec::new(),
        })
        .collect()
}

/// Every channel's rules at their defaults, plus the example grant of
/// frame 49 on the payments channel when there is one.
pub(super) fn rules(channels: &[ChannelId]) -> BTreeMap<ChannelId, Vec<AccessRule>> {
    channels
        .iter()
        .map(|channel| {
            let mut rules = defaults();
            if channel.as_str() == "payments-rewrite"
                && let Some(row) = rules
                    .iter_mut()
                    .find(|rule| rule.capability.as_str() == "files_read_other_session")
            {
                row.grants.push(Grant {
                    id: GrantId::new("g-docs"),
                    session: SessionId::new("brin"),
                    scope: "only under ledger/docs/".to_owned(),
                    granted: "12m ago".to_owned(),
                });
            }
            (channel.clone(), rules)
        })
        .collect()
}
