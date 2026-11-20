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
const ASK_ONLY: &[RuleSetting] = &[Ask];

const fn row(
    key: &'static str,
    label: &'static str,
    detail: &'static str,
    capabilities: &'static [&'static str],
    ask_lands: AskLands,
) -> AccessRow {
    AccessRow {
        key,
        label,
        detail,
        capabilities,
        options: ALL,
        ask_lands,
    }
}

/// The rows as the signed-off frame shows them.
pub const GROUPS: &[AccessGroup] = &[
    AccessGroup {
        title: "TO-DO",
        caption: "never stops an agent and never notifies you",
        rows: &[
            row(
                "todo-own",
                "Tick steps and edit items it made",
                "",
                &["own_board_write", "channel_board_edit_own"],
                AskLands::Todo,
            ),
            row(
                "todo-add",
                "Add an item",
                "It appears under Needs attention, marked Suggested",
                &["todo_suggest", "channel_board_add"],
                AskLands::Todo,
            ),
            row(
                "todo-edit-others",
                "Close or edit an item it didn\u{2019}t make",
                "State, due date, effort, impact, tags, what it waits on",
                &["todo_close", "channel_board_edit_others"],
                AskLands::Todo,
            ),
            row(
                "todo-delete",
                "Delete an item",
                "",
                &["global_board_write"],
                AskLands::Todo,
            ),
        ],
    },
    AccessGroup {
        title: "FILES AND SESSIONS",
        caption: "risky: the agent stops until you answer, and you get a notification",
        rows: &[
            row(
                "files-own",
                "Move or copy files inside its own session",
                "",
                &["shared_write", "files_fetch_local"],
                AskLands::AgentInbox,
            ),
            row(
                "files-other-session",
                "Copy files from another session",
                "",
                &["files_read_other_session", "files_copy_in"],
                AskLands::AgentInbox,
            ),
            row(
                "files-other-machine",
                "Move files or its session to another machine",
                "Running terminals move along and keep running",
                &["files_read_other_machine", "session_move"],
                AskLands::AgentInbox,
            ),
            row(
                "session-start",
                "Start or fork a session",
                "",
                &["session_start_same_machine", "session_start_other_machine"],
                AskLands::AgentInbox,
            ),
            row(
                "session-end",
                "End a session or a terminal",
                "Only you end them",
                &["own_session_end", "session_stop_other", "session_delete_other"],
                AskLands::AgentInbox,
            ),
            row(
                "sessions-delete-many",
                "Delete many sessions at once",
                "Never ends a running terminal without asking you",
                &["sessions_delete_many"],
                AskLands::AgentInbox,
            ),
        ],
    },
    AccessGroup {
        title: "ACCESS AND QUESTIONS",
        caption: "",
        rows: &[
            row(
                "read-other",
                "Read other channels\u{2019} to-dos and files",
                "",
                &["global_read"],
                AskLands::AgentInbox,
            ),
            AccessRow {
                key: "ask-access",
                label: "Ask for access it doesn\u{2019}t have",
                detail: "",
                capabilities: &["inbox_ask"],
                options: ASK_NEVER,
                ask_lands: AskLands::AgentInbox,
            },
            AccessRow {
                key: "ask-question",
                label: "Ask you a question",
                detail: "",
                // Always Ask: no rule to write.
                capabilities: &[],
                options: ASK_ONLY,
                ask_lands: AskLands::AgentInbox,
            },
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
