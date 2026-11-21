//! The grouped Allow / Ask / Never rows of Agent access (INBOXHOOKS I8).
//!
//! Each row stands for one or more access capabilities. Setting a row writes
//! every one of them; when they disagree the row reads Mixed. The table is
//! data so the view and the tests read the same rows.

use humata2_clients::model::{AccessRule, Capability, RuleSetting};

/// Where an Ask lands and what the agent does meanwhile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskLands {
    /// On the item in To-do; the agent carries on.
    Todo,
    /// In the Agent inbox; the agent waits.
    AgentInbox,
}

/// One group of rows under a heading.
#[derive(Clone, Copy, Debug)]
pub struct AccessGroup {
    /// The heading.
    pub title: &'static str,
    /// The small note after the heading.
    pub caption: &'static str,
    /// The rows.
    pub rows: &'static [AccessRow],
}

/// One row: a kind of action and the rules behind it.
#[derive(Clone, Copy, Debug)]
pub struct AccessRow {
    /// A stable key for the row.
    pub key: &'static str,
    /// What the row says.
    pub label: &'static str,
    /// Inline scope chip shown beside the label, if any.
    pub scope_chip: &'static str,
    /// The small line under the label, if any.
    pub detail: &'static str,
    /// The capability ids the row writes.
    pub capabilities: &'static [&'static str],
    /// The settings the row offers.
    pub options: &'static [RuleSetting],
    /// Where an Ask lands.
    pub ask_lands: AskLands,
}

/// What a row reads as, given a channel's rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowSetting {
    /// Every rule behind the row has this setting.
    Is(RuleSetting),
    /// The rules behind the row differ.
    Mixed,
}

use RuleSetting::{Allow, Ask, Never};

const ALL: &[RuleSetting] = &[Allow, Ask, Never];
const ASK_NEVER: &[RuleSetting] = &[Ask, Never];

const fn row(
    key: &'static str,
    label: &'static str,
    scope_chip: &'static str,
    detail: &'static str,
    capabilities: &'static [&'static str],
    options: &'static [RuleSetting],
) -> AccessRow {
    AccessRow {
        key,
        label,
        scope_chip,
        detail,
        capabilities,
        options,
        ask_lands: AskLands::AgentInbox,
    }
}

/// The rows as the signed-off frame shows them.
pub const GROUPS: &[AccessGroup] = &[
    AccessGroup {
        title: "LOCAL & SAME-CHANNEL",
        caption: "Runs automatically within scope \u{00b7} asks in Agent inbox outside scope",
        rows: &[
            row(
                "files-other-session",
                "Read or copy files from another session",
                "Same machine \u{00b7} this channel",
                "",
                &["files_read_other_session", "files_copy_in"],
                ALL,
            ),
            row(
                "session-start-local",
                "Start or fork a session",
                "Same machine \u{00b7} this channel \u{00b7} max 3",
                "",
                &["session_start_same_machine"],
                ALL,
            ),
            row(
                "session-note",
                "Send a note to another session",
                "This channel",
                "",
                &["session_note"],
                ALL,
            ),
            row(
                "session-own-end",
                "Stop or archive its own session",
                "Not delete",
                "",
                &["own_session_manage", "own_session_end"],
                ALL,
            ),
            row(
                "git-push-branch",
                "Push its own branch",
                "No open PR \u{00b7} non-force",
                "",
                &[],
                ALL,
            ),
            row(
                "pr-review-start",
                "Start a local PR review",
                "This channel\u{2019}s repos",
                "",
                &["pr_review_start"],
                ALL,
            ),
            row(
                "todo-agent-items",
                "Edit or close agent-created To-dos",
                "This channel \u{00b7} not yours",
                "",
                &["todo_close", "channel_board_edit_others"],
                ALL,
            ),
        ],
    },
    AccessGroup {
        title: "LEAVES THIS MACHINE OR AFFECTS OTHERS",
        caption: "Always stops and asks in Agent inbox unless you allow it",
        rows: &[
            row(
                "git-push-pr",
                "Open a PR or push to a branch with an open PR",
                "",
                "Silent when you approve in Review or explicitly tell the agent to push.",
                &["git_push"],
                ALL,
            ),
            row(
                "session-other-machine",
                "Start, fork, or move a session on another machine",
                "",
                "",
                &["session_start_other_machine", "session_move"],
                ALL,
            ),
            row(
                "network-reach",
                "Open a network connection through this Mac",
                "",
                "",
                &["network_reach"],
                ASK_NEVER,
            ),
            row(
                "session-other-end",
                "Stop or delete another session",
                "",
                "",
                &["session_stop_other", "session_delete_other"],
                ALL,
            ),
            row(
                "sessions-delete-many",
                "Delete multiple sessions or destroy a machine",
                "",
                "",
                &["sessions_delete_many", "machine_destroy"],
                ALL,
            ),
        ],
    },
];

/// The row with `key`.
#[must_use]
pub fn find(key: &str) -> Option<&'static AccessRow> {
    GROUPS
        .iter()
        .flat_map(|group| group.rows)
        .find(|row| row.key == key)
}

impl AccessRow {
    /// What the row reads as over `rules`: the one setting all its rules
    /// share, or Mixed. Capabilities the channel has no setting for are left
    /// out; a row with none of them (or none to write) reads as its first
    /// option.
    #[must_use]
    pub fn setting(&self, rules: &[AccessRule]) -> RowSetting {
        let mut found = self
            .capabilities
            .iter()
            .filter_map(|id| rules.iter().find(|rule| rule.capability.as_str() == *id))
            .filter_map(|rule| rule.setting);
        let fallback = self.options.first().copied().unwrap_or(Ask);
        let Some(first) = found.next() else {
            return RowSetting::Is(fallback);
        };
        if found.all(|setting| setting == first) {
            RowSetting::Is(first)
        } else {
            RowSetting::Mixed
        }
    }

    /// The capabilities to write when the row is set.
    #[must_use]
    pub fn capability_ids(&self) -> Vec<Capability> {
        self.capabilities
            .iter()
            .map(|id| Capability::new(*id))
            .collect()
    }

    /// The line to the right of the control for `setting`.
    #[must_use]
    pub fn note(&self, setting: RowSetting) -> &'static str {
        match setting {
            RowSetting::Is(Allow) => "",
            RowSetting::Is(Ask) => match self.ask_lands {
                AskLands::Todo => "Asks on the item in To-do; the agent carries on.",
                AskLands::AgentInbox => "Asks in the Agent inbox; the agent stops and waits.",
            },
            RowSetting::Is(Never) => "The agent is told no, with this rule named.",
            RowSetting::Mixed => "The rules under this row differ. Pick one to set them all.",
        }
    }
}
