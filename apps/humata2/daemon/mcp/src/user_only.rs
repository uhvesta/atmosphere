//! Operations only the user may do (MCP.md §7, SEC-7, IN-23). They have no
//! tool and are absent from `tools/list`; a client that calls one by name
//! anyway gets `user_only`, and nothing reaches the daemon or the Inbox.

use humata2_access::Capability;
use serde_json::{Map, Value};

use crate::daemon::{ErrorKind, ToolError};
use crate::tools::capability_name;

/// MCP.md §7's operations as tool names (dots become underscores), each
/// with the capability that makes it the user's, when one does.
const NAMES: &[(&str, Option<Capability>)] = &[
    ("channel_delete", Some(Capability::ChannelDelete)),
    ("inbox_answer", Some(Capability::InboxAnswer)),
    ("inbox_answer_all", Some(Capability::InboxAnswer)),
    ("access_set_rule", Some(Capability::AccessChange)),
    ("access_set_away_rule", Some(Capability::AccessChange)),
    ("access_set_defaults", Some(Capability::AccessChange)),
    ("access_revoke_grant", Some(Capability::AccessChange)),
    ("access_set_passphrase", Some(Capability::AccessChange)),
    ("marks_set", Some(Capability::MarksChange)),
    ("pinned_reorder", Some(Capability::MarksChange)),
    ("review_submit", Some(Capability::PrSubmit)),
    ("review_set_push_allowed", Some(Capability::PrSubmit)),
    ("diff_mark_reviewed", Some(Capability::UserReviewState)),
    ("view_set_state", Some(Capability::UserReviewState)),
    ("comment_send", Some(Capability::UserReviewState)),
    ("trash_empty", Some(Capability::TrashEmpty)),
    ("host_set_variable", Some(Capability::HostSecrets)),
    ("host_remove_variable", Some(Capability::HostSecrets)),
    ("host_trust_new_key", Some(Capability::HostSecrets)),
    ("providers_set_config", Some(Capability::HostSecrets)),
    ("provider_login", Some(Capability::HostSecrets)),
    ("provider_make_unattended", Some(Capability::HostSecrets)),
    ("machine_change_root", Some(Capability::MachineSettings)),
    ("machine_set_vm_tier", Some(Capability::MachineSettings)),
    ("machine_shell", Some(Capability::TerminalInput)),
    ("terminal_attach", Some(Capability::TerminalInput)),
    ("terminal_open_shell_tab", Some(Capability::TerminalInput)),
    ("terminal_send_queued_now", Some(Capability::TerminalInput)),
    // An agent closes its own forwards with network_close.
    ("bridge_close", None),
    ("events_subscribe", None),
    ("audit_list", None),
    // REPOS: RL-4, RL-11, D479.
    ("repo_remove", None),
    ("repo_forget", None),
    ("repo_scope_update", None),
];

/// Whole families: the user's file manager and the app's own state.
const PREFIXES: &[&str] = &["transfer_", "view_state_"];

/// The refusal for `name` when it is a user-only operation.
pub(crate) fn refusal(name: &str) -> Option<ToolError> {
    let capability = match NAMES.iter().find(|(known, _)| *known == name) {
        Some((_, capability)) => *capability,
        None if PREFIXES.iter().any(|prefix| name.starts_with(prefix)) => None,
        None => return None,
    };
    let mut details = Map::new();
    if let Some(capability) = capability {
        details.insert(
            "capability".to_owned(),
            Value::from(capability_name(capability)),
        );
    }
    Some(ToolError {
        kind: ErrorKind::UserOnly,
        message: format!(
            "{name} is the user's to do: it has no tool, and no rule or grant lets an agent do it."
        ),
        fix: "Tell the user what you need and why with inbox_ask; they can do it themselves. \
              Nothing was added to the Inbox."
            .to_owned(),
        retry: None,
        details,
    })
}
