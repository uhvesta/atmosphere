//! The tools of MCP.md §6: names, operation ids, descriptions, input
//! schemas and annotations.
//!
//! A tool takes no requester, caller-session or caller-channel argument
//! (MCP.md §1): the caller is the bound session. Tools whose call a channel
//! rule governs also take `reason` (shown on the Inbox card) and
//! `wait_seconds`, since a user can set any rule to Ask me.

use humata2_access::{Capability, Tier};
use serde_json::{Value, json};

use crate::outcome::output_schema;
use crate::schema::{
    Property, Schema, any_object, array, boolean, integer, object, one_of, one_of_strings, opt,
    req, string,
};

macro_rules! operations {
    ($($(#[$meta:meta])* $variant:ident => $tool:literal, $id:literal;)*) => {
        /// One tool, and the daemon operation it is (MCP.md §2: the tool is
        /// `<area>_<verb>`, the operation id `<area>.<verb>`). Each variant
        /// names the gRPC calls humatad makes for it.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Operation {
            $($(#[$meta])* $variant,)*
        }

        impl Operation {
            /// Every tool, in `tools/list` order.
            pub const ALL: &'static [Operation] = &[$(Operation::$variant,)*];

            /// The tool's name in `tools/list`.
            pub fn tool_name(self) -> &'static str {
                match self {
                    $(Operation::$variant => $tool,)*
                }
            }

            /// The operation id used in audit, history, the Inbox card and
            /// the journal.
            pub fn id(self) -> &'static str {
                match self {
                    $(Operation::$variant => $id,)*
                }
            }
        }
    };
}

operations! {
    /// `DaemonService.WhoAmI`.
    Whoami => "whoami", "daemon.whoami";
    /// `SessionService.GetSession` of the caller.
    SessionInfo => "session_info", "session.get";
    /// `DaemonService.WhoAmI`, `ChannelService.GetChannel` and
    /// `SessionService.ListSessions` (TP-1).
    TopologyGet => "topology_get", "channel.topology";
    /// `AccessService.Explain` for the caller.
    AccessExplain => "access_explain", "access.explain";
    /// `BoardService.ListItems`.
    BoardItemsList => "board_items_list", "board.items_list";
    /// `BoardService.GetItem`.
    BoardItemGet => "board_item_get", "board.item_get";
    /// `BoardService.GetItemHistory`.
    BoardItemHistory => "board_item_history", "board.item_history";
    /// `BoardService.CreateItem`.
    BoardItemCreate => "board_item_create", "board.item_create";
    /// `BoardService.UpdateItem`.
    BoardItemUpdate => "board_item_update", "board.item_update";
    /// `BoardService.MoveItem`.
    BoardItemMove => "board_item_move", "board.item_move";
    /// `BoardService.DeleteItem`.
    BoardItemDelete => "board_item_delete", "board.item_delete";
    /// `ViewService.PrepareView`.
    ViewPrepare => "view_prepare", "view.prepare";
    /// `ViewService.ReviseView`.
    ViewRevise => "view_revise", "view.revise";
    /// `ViewService.MarkReady`.
    ViewMarkReady => "view_mark_ready", "view.mark_ready";
    /// `ViewService.ListViews` of the caller.
    ViewsList => "views_list", "view.list";
    /// `CommentService.CreateComment` with the pull request as destination.
    ReviewDraft => "review_draft", "review.draft";
    /// `CommentService.EditComment`.
    ReviewDraftEdit => "review_draft_edit", "review.draft_edit";
    /// `CommentService.DropComment`.
    ReviewDraftDrop => "review_draft_drop", "review.draft_drop";
    /// `PullRequestReviewService.DraftSummary`.
    ReviewSummaryDraft => "review_summary_draft", "review.summary_draft";
    /// `ReviewChatService.GetReviewContext` of the caller's review.
    ReviewContext => "review_context", "review.context";
    /// `ReviewChatService.FocusReview`.
    ReviewFocus => "review_focus", "review.focus";
    /// `CommentService.CreateComment` with a note for the user as
    /// destination.
    ReviewNote => "review_note", "review.note";
    /// `CommentService.EditComment` of a note the caller wrote.
    ReviewNoteEdit => "review_note_edit", "review.note_edit";
    /// `CommentService.DropComment` of a note the caller wrote.
    ReviewNoteDrop => "review_note_drop", "review.note_drop";
    /// `ViewService.PrepareView` with a review plan; in the review chat it
    /// applies at once (PR-24).
    ReviewPrepare => "review_prepare", "review.prepare";
    /// `CommentService.ListComments` of the caller's session.
    CommentsList => "comments_list", "comment.list";
    /// `CommentService.ReplyToComment` on a comment typed into the caller's
    /// session.
    CommentReply => "comment_reply", "comment.reply";
    /// `CommentService.ResolveComment` with a reply.
    CommentResolve => "comment_resolve", "comment.resolve";
    /// `FileService.ListDirectory`.
    FilesList => "files_list", "file.list";
    /// `FileService.ReadFile`.
    FileRead => "file_read", "file.read";
    /// `FileService.FindFiles` in the caller's session.
    FilesFind => "files_find", "file.find";
    /// `HistoryService.SearchSessions`.
    SessionsSearch => "sessions_search", "session.search";
    /// `HistoryService.ReadTurns`.
    SessionRead => "session_read", "session.read";
    /// `FileService.WriteFile`.
    FileWrite => "file_write", "file.write";
    /// `FileService.CreateFolder`.
    FolderCreate => "folder_create", "file.folder_create";
    /// `TransferService.StartTransfer`, a copy on the caller's machine.
    FileFetch => "file_fetch", "file.fetch";
    /// `InboxService.AskUser`.
    InboxAsk => "inbox_ask", "inbox.ask";
    /// `InboxService.RequestAccess`.
    InboxRequest => "inbox_request", "inbox.request";
    /// `InboxService.WaitForAnswer`. The server runs it with
    /// [`crate::Daemon::wait`], never [`crate::Daemon::call`].
    InboxWait => "inbox_wait", "inbox.wait";
    /// `InboxService.WithdrawRequest`.
    InboxWithdraw => "inbox_withdraw", "inbox.withdraw";
    /// `InboxService.ListRequests` of the caller.
    InboxMine => "inbox_mine", "inbox.mine";
    /// `SessionService.AnswerQuestion`.
    SessionAnswer => "session_answer", "session.answer";
    /// The journal of the caller's machine (GRPC.md §9). The protos have no
    /// RPC for it yet.
    JournalStatus => "journal_status", "journal.status";
    /// `OperationService.GetOperation`.
    OperationGet => "operation_get", "operation.get";
    /// `BridgeService.ClosePortForward` of the caller's own forward.
    NetworkClose => "network_close", "bridge.close";
    /// `ChannelService.ListChannels`.
    ChannelsList => "channels_list", "channel.list";
    /// `TransferService.StartTransfer` into the caller's clone or `shared/`.
    FilesCopyIn => "files_copy_in", "file.copy_in";
    /// `SessionService.SendNote`.
    SessionNote => "session_note", "session.note";
    /// `SessionService.AskSession`.
    SessionAsk => "session_ask", "session.ask";
    /// `TerminalService.Type`.
    SessionType => "session_type", "session.type";
    /// `SessionService.StartSession`.
    SessionStart => "session_start", "session.start";
    /// `SessionService.ForkSession` of the caller.
    SessionFork => "session_fork", "session.fork";
    /// `BridgeService.OpenPortForward`.
    NetworkConnect => "network_connect", "bridge.connect";
    /// `SessionService.RenameSession` of the caller.
    SessionRename => "session_rename", "session.rename";
    /// `SessionService.AddRepoToSession` of the caller.
    SessionAddRepo => "session_add_repo", "session.add_repo";
    /// `SessionService.PlanDropRepo` and `DropRepo` of the caller.
    SessionDropRepo => "session_drop_repo", "session.drop_repo";
    /// `SessionService.GiveOwnCopy` of the caller.
    SessionOwnCopy => "session_own_copy", "session.own_copy";
    /// `SessionService.StopSession`; the caller's own session without
    /// `session`.
    SessionStop => "session_stop", "session.stop";
    /// `SessionService.PlanArchive` and `ArchiveSession`.
    SessionArchive => "session_archive", "session.archive";
    /// `SessionService.PlanDelete` and `DeleteSession`.
    SessionDelete => "session_delete", "session.delete";
    /// `SessionService.MoveSession`.
    SessionMove => "session_move", "session.move";
    /// `SessionService.PlanPush` and `PushAndOpenPullRequests`.
    SessionPush => "session_push", "session.push";
    /// `PullRequestReviewService.StartReview`.
    PrReviewStart => "pr_review_start", "review.start";
    /// `MachineService.AddSshMachine`.
    MachineAddSsh => "machine_add_ssh", "machine.add_ssh";
    /// `ProviderService.CreateMachine`.
    MachineCreate => "machine_create", "machine.create";
    /// `ProviderService.RunProviderAction` (start).
    MachineStart => "machine_start", "machine.start";
    /// `ProviderService.RunProviderAction` (stop).
    MachineStop => "machine_stop", "machine.stop";
    /// `MachineService.ReconnectMachine`.
    MachineReconnect => "machine_reconnect", "machine.reconnect";
    /// `SkillsService.PreviewSkillsSync` and `SyncSkills`.
    SkillsSync => "skills_sync", "skills.sync";
    /// `TrashService.RestoreFromTrash`.
    TrashRestore => "trash_restore", "trash.restore";
    /// `ChannelService.CreateChannel`.
    ChannelCreate => "channel_create", "channel.create";
    /// `SessionService.RestoreSession`.
    SessionRestore => "session_restore", "session.restore";
    /// `SessionService.RebuildSession`.
    SessionRebuild => "session_rebuild", "session.rebuild";
    /// `ChannelService.RenameChannel` of the caller's channel.
    ChannelRename => "channel_rename", "channel.rename";
    /// `ChannelService.AddRepo` to the caller's channel.
    ChannelAddRepo => "channel_add_repo", "channel.add_repo";
    /// `ChannelService.PlanRemoveRepo` and `RemoveRepo`.
    ChannelRemoveRepo => "channel_remove_repo", "channel.remove_repo";
    /// `ChannelService.AddMachine` to the caller's channel.
    ChannelAddMachine => "channel_add_machine", "channel.add_machine";
    /// `ChannelService.PlanRemoveMachine` and `RemoveMachine`.
    ChannelRemoveMachine => "channel_remove_machine", "channel.remove_machine";
    /// `MachineService.PlanRemoveMachine` and `RemoveMachine`.
    MachineRemove => "machine_remove", "machine.remove";
    /// `MachineService.PlanForgetMachine` and `ForgetMachine`.
    MachineForget => "machine_forget", "machine.forget";
    /// `ProviderService.DeleteWhatWasMade`.
    MachineDelete => "machine_delete", "machine.delete";
}

impl Operation {
    pub(crate) fn from_tool_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|operation| operation.tool_name() == name)
    }
}

/// What a call does, for the tool's annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// Changes nothing.
    Read,
    /// Sets something; the same call again changes nothing more.
    Change,
    /// Makes something new each time it is called with a new request id.
    Create,
    /// Removes or stops something.
    Remove,
}

struct Definition {
    description: &'static str,
    effect: Effect,
    /// Reaches past Humata's machines: the network, GitHub or a provider.
    open_world: bool,
    input: Schema,
}

fn define(description: &'static str, effect: Effect, input: Schema) -> Definition {
    Definition {
        description,
        effect,
        open_world: false,
        input,
    }
}

fn reaching_out(mut definition: Definition) -> Definition {
    definition.open_world = true;
    definition
}

/// The tool's input schema, which every call is checked against.
pub(crate) fn input_schema(operation: Operation) -> Schema {
    definition(operation).input
}

/// The `tools/list` result: every tool, never a user-only one.
pub(crate) fn list() -> Value {
    let output = output_schema();
    let tools: Vec<Value> = Operation::ALL
        .iter()
        .map(|operation| {
            let definition = definition(*operation);
            json!({
                "name": operation.tool_name(),
                "description": definition.description,
                "inputSchema": definition.input.to_json(),
                "outputSchema": output.clone(),
                "annotations": {
                    "readOnlyHint": definition.effect == Effect::Read,
                    "destructiveHint": definition.effect == Effect::Remove,
                    "idempotentHint": definition.effect != Effect::Create,
                    "openWorldHint": definition.open_world,
                },
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// The capability's name in `capabilities.proto`, lowercase without the
/// prefix (D25).
pub(crate) fn capability_name(capability: Capability) -> &'static str {
    match capability {
        Capability::OwnBoardWrite => "own_board_write",
        Capability::ChannelRead => "channel_read",
        Capability::ChannelBoardAdd => "channel_board_add",
        Capability::ChannelBoardEditOwn => "channel_board_edit_own",
        Capability::SharedWrite => "shared_write",
        Capability::FilesFetchLocal => "files_fetch_local",
        Capability::ViewsPrepare => "views_prepare",
        Capability::ReviewDraft => "review_draft",
        Capability::InboxAsk => "inbox_ask",
        Capability::ReviewFocus => "review_focus",
        Capability::ReviewNote => "review_note",
        Capability::ReviewOrder => "review_order",
        Capability::CommentReply => "comment_reply",
        Capability::CommentResolve => "comment_resolve",
        Capability::ChannelBoardEditOthers => "channel_board_edit_others",
        Capability::GlobalRead => "global_read",
        Capability::GlobalBoardWrite => "global_board_write",
        Capability::FilesReadOtherSession => "files_read_other_session",
        Capability::FilesReadOtherMachine => "files_read_other_machine",
        Capability::FilesCopyIn => "files_copy_in",
        Capability::SessionNote => "session_note",
        Capability::SessionType => "session_type",
        Capability::SessionStartSameMachine => "session_start_same_machine",
        Capability::SessionStartOtherMachine => "session_start_other_machine",
        Capability::PrReviewStart => "pr_review_start",
        Capability::OwnSessionManage => "own_session_manage",
        Capability::OwnSessionEnd => "own_session_end",
        Capability::SessionMove => "session_move",
        Capability::GitPush => "git_push",
        Capability::MachineProvision => "machine_provision",
        Capability::TrashRestore => "trash_restore",
        Capability::ChannelCreate => "channel_create",
        Capability::NetworkReach => "network_reach",
        Capability::SessionStopOther => "session_stop_other",
        Capability::ChannelChange => "channel_change",
        Capability::MachineDestroy => "machine_destroy",
        Capability::InboxAnswer => "inbox_answer",
        Capability::AccessChange => "access_change",
        Capability::SettingsChange => "settings_change",
        Capability::MarksChange => "marks_change",
        Capability::PrSubmit => "pr_submit",
        Capability::UserReviewState => "user_review_state",
        Capability::TrashEmpty => "trash_empty",
        Capability::HostSecrets => "host_secrets",
        Capability::ChannelDelete => "channel_delete",
        Capability::MachineSettings => "machine_settings",
        Capability::TerminalInput => "terminal_input",
        Capability::PeerReport => "peer_report",
        Capability::HostControl => "host_control",
    }
}

// Common fields (MCP.md §6 notation).

const COLUMNS: [&str; 5] = ["backlog", "to_do", "in_progress", "in_review", "done"];

fn reason() -> Schema {
    string()
        .max_length(500)
        .describe("Why, in your words; shown on the user's Inbox card if the call asks.")
}

fn wait_seconds() -> Schema {
    integer().range(0, 600).default_value(45).describe(
        "How long to wait for the user before returning pending (default 45, at most 600). \
         After pending, call inbox_wait with the request_id; don't repeat the call.",
    )
}

/// An object for a call a channel rule governs: it also takes `reason` and
/// `wait_seconds` unless it already has them.
fn asks(properties: Vec<Property>) -> Schema {
    let mut schema = object(properties);
    if !schema.has_property("reason") {
        schema.push_property(opt("reason", reason()));
    }
    if !schema.has_property("wait_seconds") {
        schema.push_property(opt("wait_seconds", wait_seconds()));
    }
    schema
}

fn session() -> Schema {
    string().describe("A session id from topology_get")
}

fn machine() -> Schema {
    string().describe("A machine id or name from topology_get")
}

fn repo() -> Schema {
    string().describe("A repository as owner/name")
}

fn page_token() -> Schema {
    string().describe("From the last page's next_page_token")
}

fn expected_revision() -> Schema {
    integer()
        .at_least(0)
        .describe("From your last read; a stale one fails with revision_conflict")
}

fn item_id() -> Schema {
    string().describe("An item id such as PAY-7, or a provisional ~machine.seq id")
}

fn title() -> Schema {
    string().max_length(200)
}

fn column() -> Schema {
    one_of_strings(COLUMNS)
}

fn board() -> Schema {
    one_of(vec![
        one_of_strings(["session", "channel", "global"]),
        object(vec![req("session", session())]),
    ])
    .describe("session (your session board), channel, global, or {session} for another session's")
}

fn relative_path() -> Schema {
    string().describe("A path relative to the root")
}

fn location() -> Schema {
    one_of(vec![
        object(vec![
            req("root", one_of_strings(["clone"])),
            req("repo", repo()),
            opt("session", session().describe("Default: your session")),
            opt("machine", machine().describe("Default: your machine")),
            req("path", relative_path()),
        ]),
        object(vec![
            req("root", one_of_strings(["shared"])),
            req("path", relative_path()),
            opt("machine", machine().describe("Default: your machine")),
        ]),
        object(vec![
            req("root", one_of_strings(["channel_folder"])),
            req("path", relative_path()),
            opt("machine", machine().describe("Default: your machine")),
        ]),
        object(vec![
            req("root", one_of_strings(["main_clone"])),
            req("repo", repo()),
            req("path", relative_path()),
            opt("machine", machine().describe("Default: your machine")),
        ]),
    ])
    .describe("A file or folder: in a clone, shared/, the channel folder or a main clone (read-only)")
}

fn on_existing() -> Schema {
    one_of_strings(["fail", "replace", "keep_both"]).describe("Default: fail")
}

fn lines() -> Schema {
    object(vec![
        req("first", integer().at_least(1)),
        req("last", integer().at_least(1)),
    ])
}

fn side() -> Schema {
    one_of_strings(["old", "new"]).describe("Default: new")
}

fn file_ref() -> Schema {
    object(vec![req("repo", repo()), req("path", relative_path())])
}

/// A diff spec (RV-31, `humata.v1.DiffSpec`).
fn diff_spec() -> Schema {
    object(vec![req(
        "repos",
        array(object(vec![
            req("repo", repo()),
            req(
                "base",
                object(vec![
                    opt(
                        "merge_base_with",
                        string().describe("A branch, as GitHub shows a pull request: origin/main"),
                    ),
                    opt("rev", string().describe("A branch, tag or commit, as it is")),
                    opt(
                        "reviewed_snapshot",
                        integer()
                            .at_least(0)
                            .describe("A review the user marked reviewed; 0 for the latest"),
                    ),
                    opt("fork_point", boolean().describe("Where this session was forked")),
                    opt("index", boolean().describe("What is staged")),
                ])
                .exactly_one_of(&[
                    "merge_base_with",
                    "rev",
                    "reviewed_snapshot",
                    "fork_point",
                    "index",
                ]),
            ),
            opt(
                "upto",
                one_of_strings(["commits", "staged", "unstaged"]).describe("Default: unstaged"),
            ),
            opt(
                "untracked",
                one_of_strings(["none", "all", "paths"]).describe("Default: all"),
            ),
            opt(
                "untracked_paths",
                array(relative_path()).describe("With untracked: paths, the folders or files"),
            ),
            opt("include_paths", array(string().describe("A git pathspec"))),
            opt("exclude_paths", array(string().describe("A git pathspec"))),
            opt(
                "whitespace",
                one_of_strings(["counts", "ignore_changes", "ignore_all"])
                    .describe("Default: counts"),
            ),
            opt("renames", boolean().describe("Default: true")),
        ])),
    )])
}

fn when() -> Schema {
    one_of_strings(["now", "turn_end"]).describe("Default: now")
}

fn capability() -> Schema {
    one_of_strings(
        Capability::ALL
            .iter()
            .copied()
            .filter(|capability| capability.tier() != Tier::Peer)
            .map(capability_name),
    )
    .describe("A capability, as in access_explain")
}

/// What a grant or an explanation is about (`humata.v1.GrantTarget`).
fn grant_target() -> Schema {
    object(vec![
        opt("session", session()),
        opt("board", board()),
        opt(
            "path",
            object(vec![
                opt("machine", machine()),
                opt("session", session()),
                req("repo", repo()),
                req("path", relative_path()),
            ]),
        ),
        opt(
            "network",
            object(vec![
                req("host", string()),
                req("port", integer().range(1, 65_535)),
                opt("machine", machine()),
            ]),
        ),
        opt(
            "agent_on_machine",
            object(vec![req("machine", machine()), req("agent", string())]),
        ),
        opt(
            "repo_branch",
            object(vec![req("repo", repo()), req("branch", string())]),
        ),
        opt(
            "machine_action",
            object(vec![
                opt("machine", machine()),
                opt("new_machine", string().describe("The name of a machine to add or create")),
                req(
                    "action",
                    one_of_strings([
                        "add",
                        "create",
                        "start",
                        "stop",
                        "reconnect",
                        "sync_skills",
                        "remove",
                        "forget",
                        "delete",
                    ]),
                ),
            ]),
        ),
    ])
    .exactly_one_of(&[
        "session",
        "board",
        "path",
        "network",
        "agent_on_machine",
        "repo_branch",
        "machine_action",
    ])
}

fn view_content() -> Schema {
    object(vec![
        opt(
            "review",
            object(vec![req(
                "steps",
                array(object(vec![
                    req("repo", repo()),
                    req("path", relative_path()),
                    req("lines", lines()),
                    req("note", string()),
                ])),
            )]),
        ),
        opt("board", board()),
        opt(
            "pull_request",
            object(vec![req(
                "repos",
                array(object(vec![
                    req("repo", repo()),
                    req("title", string()),
                    req("body", string()),
                    req("base_branch", string()),
                    req("draft", boolean()),
                ])),
            )]),
        ),
    ])
    .exactly_one_of(&["review", "board", "pull_request"])
}

fn session_end(description: &'static str) -> Definition {
    define(
        description,
        Effect::Remove,
        asks(vec![opt(
            "session",
            session().describe("Default: your own session"),
        )]),
    )
}

fn machine_only(description: &'static str, effect: Effect) -> Definition {
    define(description, effect, asks(vec![req("machine", machine())]))
}

fn definition(operation: Operation) -> Definition {
    use Operation as Op;
    match operation {
        Op::Whoami => define(
            "Who you are: your session, channel, machine and agent, whether the Mac is reachable, and how old your machine's copy of the rules is.",
            Effect::Read,
            object(vec![]),
        ),
        Op::SessionInfo => define(
            "Your session: title, status, repositories with branch, head and whether they're pushed or dirty, what it was forked from and spec mode.",
            Effect::Read,
            object(vec![]),
        ),
        Op::TopologyGet => define(
            "Your channel's machines, whether each is reachable, and the sessions on each.",
            Effect::Read,
            asks(vec![]),
        ),
        Op::AccessExplain => define(
            "What your rules decide for a capability and target, and why: allowed, ask, or never, and whether the Mac is away.",
            Effect::Read,
            object(vec![req("capability", capability()), opt("target", grant_target())]),
        ),
        Op::BoardItemsList => define(
            "The items on a board, with each item's sessions and what it depends on.",
            Effect::Read,
            asks(vec![
                req("board", board()),
                opt("columns", array(column())),
                opt("page_token", page_token()),
            ]),
        ),
        Op::BoardItemGet => define(
            "One board item.",
            Effect::Read,
            asks(vec![req("id", item_id())]),
        ),
        Op::BoardItemHistory => define(
            "A board item's history: who changed what, and changes that lost a conflict.",
            Effect::Read,
            asks(vec![req("id", item_id()), opt("page_token", page_token())]),
        ),
        Op::BoardItemCreate => define(
            "Add an item to a board. Your session board and your channel's board need no approval; the Global board and other channels' boards go to the user's Inbox.",
            Effect::Create,
            asks(vec![
                req("board", board()),
                req("title", title()),
                opt("column", column().describe("Default: backlog")),
                opt("description", string().max_length(20_000)),
                opt("sessions", array(session()).describe("The sessions working on it")),
                opt(
                    "depends_on",
                    array(item_id()).describe("Items it depends on, on any board you can read"),
                ),
            ]),
        ),
        Op::BoardItemUpdate => define(
            "Change a board item's title or description, add or remove its sessions, or add or remove what it depends on. Your own items and your session board need no approval; other items go to the user's Inbox.",
            Effect::Change,
            asks(vec![
                req("id", item_id()),
                opt("title", title()),
                opt("description", string().max_length(20_000)),
                opt("sessions_add", array(session())),
                opt("sessions_remove", array(session())),
                opt(
                    "depends_on_add",
                    array(item_id()).describe("A loop fails with dependency_loop naming it"),
                ),
                opt("depends_on_remove", array(item_id())),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::BoardItemMove => define(
            "Move a board item to a column, after another item or to the top.",
            Effect::Change,
            asks(vec![
                req("id", item_id()),
                req("column", column()),
                opt("after", item_id().describe("The item to put it after; default the top")),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::BoardItemDelete => define(
            "Delete a board item. It stays in the board's history.",
            Effect::Remove,
            asks(vec![
                req("id", item_id()),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::ViewPrepare => define(
            "Prepare a view for the user: a guided review, a board or pull requests. It stays a draft until view_mark_ready; nothing opens and nothing is pushed.",
            Effect::Create,
            asks(vec![
                req("title", title()),
                req("summary", string()),
                req("content", view_content()),
            ]),
        ),
        Op::ViewRevise => define(
            "Replace parts of a view you prepared.",
            Effect::Change,
            asks(vec![
                req("view_id", string()),
                opt("title", title()),
                opt("summary", string()),
                opt("content", view_content()),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::ViewMarkReady => define(
            "Offer a view you prepared to the user. Preparing a pull request view pushes nothing.",
            Effect::Change,
            asks(vec![req("view_id", string())]),
        ),
        Op::ViewsList => define(
            "The views you prepared and their state.",
            Effect::Read,
            object(vec![opt(
                "include_forks",
                boolean().describe("Also your forks' views"),
            )]),
        ),
        Op::ReviewDraft => define(
            "Draft a review comment on a pull request line range. Only the user posts it.",
            Effect::Create,
            asks(vec![
                req("pr", string().describe("owner/repo#number")),
                req("path", relative_path()),
                req("lines", lines()),
                opt("side", one_of_strings(["old", "new"]).describe("Default: new")),
                req("quoted", string().describe("The lines you're commenting on, as you read them")),
                req("text", string()),
            ]),
        ),
        Op::ReviewDraftEdit => define(
            "Change a draft review comment you wrote.",
            Effect::Change,
            asks(vec![
                req("comment_id", string()),
                req("text", string()),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::ReviewDraftDrop => define(
            "Drop a draft review comment you wrote.",
            Effect::Remove,
            asks(vec![req("comment_id", string())]),
        ),
        Op::ReviewSummaryDraft => define(
            "Draft the review's summary for the user to edit and post.",
            Effect::Change,
            asks(vec![
                req("pr", string().describe("owner/repo#number")),
                req("summary", string()),
            ]),
        ),
        Op::ReviewContext => define(
            "What the user is looking at in this review: the diff spec, the open file, the visible lines and selection, the order and groups, Viewed files and your notes.",
            Effect::Read,
            object(Vec::new()),
        ),
        Op::ReviewFocus => define(
            "Show the user a place in the review: open a file, scroll to and select lines, and highlight ranges with a short label. It applies once the user stops scrolling or typing; they can Undo it or go Back.",
            Effect::Change,
            object(vec![
                req("repo", repo()),
                req("path", relative_path()),
                opt("lines", lines()),
                opt("side", side()),
                opt(
                    "highlights",
                    array(object(vec![
                        req("repo", repo()),
                        req("path", relative_path()),
                        req("lines", lines()),
                        opt("side", side()),
                        opt("label", string().max_length(80)),
                    ]))
                    .max_items(20)
                    .describe("Replaces your earlier highlights"),
                ),
            ]),
        ),
        Op::ReviewNote => define(
            "Leave the user a note on a file, a hunk or lines, to help them understand the change. Notes are never posted; the user can make one a draft.",
            Effect::Create,
            object(vec![
                req("repo", repo()),
                req("path", relative_path()),
                opt("lines", lines().describe("The lines; leave out for a note on the file")),
                opt(
                    "hunk_at",
                    integer()
                        .at_least(1)
                        .describe("Instead of lines: a line inside the hunk the note is on"),
                ),
                opt("side", side()),
                opt("quoted", string().describe("The lines, as you read them")),
                req("text", string().max_length(4000)),
            ]),
        ),
        Op::ReviewNoteEdit => define(
            "Change the text of a note you left the user.",
            Effect::Change,
            object(vec![
                req("note_id", string()),
                req("text", string().max_length(4000)),
            ]),
        ),
        Op::ReviewNoteDrop => define(
            "Remove a note you left the user.",
            Effect::Remove,
            object(vec![req("note_id", string())]),
        ),
        Op::ReviewPrepare => define(
            "Arrange a review for the user: what it compares (a diff spec), the order to read the files in, groups with titles, and notes on files, hunks and lines. In a review chat it applies at once; elsewhere it is a draft view the user opens.",
            Effect::Create,
            asks(vec![
                req("title", title()),
                req("spec", diff_spec()),
                opt("order", array(file_ref()).describe("Files in reading order; the rest follow")),
                opt(
                    "groups",
                    array(object(vec![
                        req("title", title()),
                        opt("note", string()),
                        req("first", file_ref().describe("The ordered file the group starts at")),
                    ])),
                ),
                opt(
                    "notes",
                    array(object(vec![
                        req("repo", repo()),
                        req("path", relative_path()),
                        opt("lines", lines()),
                        opt("hunk_at", integer().at_least(1)),
                        opt("side", side()),
                        req("text", string().max_length(4000)),
                    ])),
                ),
            ]),
        ),
        Op::CommentsList => define(
            "List the comments the user sent to your session, with their ids, for comment_reply and comment_resolve.",
            Effect::Read,
            object(vec![
                opt(
                    "status",
                    array(one_of_strings(["sent", "queued", "addressed"])),
                ),
                opt("path", relative_path()),
            ]),
        ),
        Op::CommentReply => define(
            "Answer the user in the thread of a comment they sent to your session; find ids with comments_list. You still answer in your terminal as usual. This is a mechanical call: hand it to a cheap subagent if your CLI has subagents.",
            Effect::Create,
            object(vec![
                req("comment_id", string()),
                req("text", string().max_length(4000)),
            ]),
        ),
        Op::CommentResolve => define(
            "Mark a comment the user sent to your session as resolved, with a short note on what you did (required; it shows in the thread under your name). The user can reopen it. Hand this to a cheap subagent if your CLI has subagents.",
            Effect::Change,
            object(vec![
                req("comment_id", string()),
                req("reply", string().max_length(4000)),
            ]),
        ),
        Op::FilesList => define(
            "List a folder. Your own clones, shared/ and the channel folder need no approval; another session's or another machine's go to the user's Inbox.",
            Effect::Read,
            asks(vec![req("location", location()), opt("page_token", page_token())]),
        ),
        Op::FileRead => define(
            "Read a file, as text or base64. Your own clones, shared/ and the channel folder need no approval; another session's or another machine's go to the user's Inbox.",
            Effect::Read,
            asks(vec![
                req("location", location()),
                opt("offset", integer().at_least(0)),
                opt("max_bytes", integer().at_least(1)),
            ]),
        ),
        Op::FilesFind => define(
            "Find files in your session by name.",
            Effect::Read,
            object(vec![req("query", string())]),
        ),
        Op::SessionsSearch => define(
            "Search the conversations of your channel, or of another channel with the user's approval: every word must match, the last also as a prefix. A hit names the session, machine, agent and turn.",
            Effect::Read,
            asks(vec![
                req("query", string().max_length(1_000)),
                opt("channel", string().describe("A channel id; default: your own")),
                opt("machine", machine()),
                opt("agent", string().describe("An agent kind such as claude")),
                opt("since", string().describe("RFC 3339 time, such as 2026-11-15T09:30:00Z")),
                opt("until", string().describe("RFC 3339 time, such as 2026-11-15T09:30:00Z")),
                opt("page_token", page_token()),
            ]),
        ),
        Op::SessionRead => define(
            "Read a session's conversation: its prompts, answers and tool calls, in order, with secrets scrubbed.",
            Effect::Read,
            asks(vec![
                req("session", session()),
                opt("first_turn", integer().at_least(0)),
                opt("last_turn", integer().at_least(0)),
                opt("page_token", page_token()),
            ]),
        ),
        Op::FileWrite => define(
            "Create or replace one file in your own clone or shared/. A replaced file goes to the trash.",
            Effect::Change,
            asks(vec![
                req("location", location()),
                opt("text", string()),
                opt("base64", string()),
                opt("replace", boolean().describe("Replace a file that's there; default false")),
            ])
            .exactly_one_of(&["text", "base64"]),
        ),
        Op::FolderCreate => define(
            "Create a folder in your own clone or shared/.",
            Effect::Change,
            asks(vec![req("location", location())]),
        ),
        Op::FileFetch => define(
            "Copy from shared/, the channel folder or a main clone on your machine into your own clone.",
            Effect::Create,
            asks(vec![
                req("source", location()),
                req("destination", location()),
                opt("on_existing", on_existing()),
            ]),
        ),
        Op::InboxAsk => define(
            "Ask the user a question in their Inbox, with up to 4 choices, and wait for the reply.",
            Effect::Create,
            object(vec![
                req("question", string()),
                opt("choices", array(string()).max_items(4)),
                opt("wait_seconds", wait_seconds()),
            ]),
        ),
        Op::InboxRequest => define(
            "Ask the user for access to one thing; approving grants it for that target. A Never rule is refused at once.",
            Effect::Create,
            object(vec![
                req("capability", capability()),
                req("target", grant_target()),
                req("reason", reason()),
                opt("wait_seconds", wait_seconds()),
            ]),
        ),
        Op::InboxWait => define(
            "Wait again for an Inbox request that returned pending. Returns what the original call would have: its result once approved, denied with the user's note, or pending again.",
            Effect::Read,
            object(vec![
                req("request_id", string()),
                opt("wait_seconds", wait_seconds()),
            ]),
        ),
        Op::InboxWithdraw => define(
            "Withdraw one of your Inbox requests; the user can no longer answer it.",
            Effect::Remove,
            object(vec![req("request_id", string())]),
        ),
        Op::InboxMine => define(
            "Your Inbox requests and their state.",
            Effect::Read,
            object(vec![opt("pending_only", boolean())]),
        ),
        Op::SessionAnswer => define(
            "Answer a question another session asked you; the answer returns to its call.",
            Effect::Change,
            object(vec![req("question_id", string()), req("answer", string())]),
        ),
        Op::JournalStatus => define(
            "Whether the Mac is reachable, and the writes this machine journaled while it was away.",
            Effect::Read,
            object(vec![]),
        ),
        Op::OperationGet => define(
            "A long operation you started, with its steps.",
            Effect::Read,
            object(vec![req("operation_id", string())]),
        ),
        Op::NetworkClose => define(
            "Close a network forward you opened.",
            Effect::Remove,
            object(vec![req("forward_id", string())]),
        ),
        Op::ChannelsList => define(
            "Every channel, with its machines and sessions. Usually asks the user first.",
            Effect::Read,
            asks(vec![]),
        ),
        Op::FilesCopyIn => define(
            "Copy files from another session or machine into your own clone or shared/. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("sources", array(location())),
                req("destination", location()),
                opt("on_existing", on_existing()),
            ]),
        ),
        Op::SessionNote => define(
            "Type a note into another session's terminal as one message naming you. Usually asks the user first.",
            Effect::Create,
            asks(vec![req("session", session()), req("text", string()), opt("when", when())]),
        ),
        Op::SessionAsk => define(
            "Ask another session a question and wait for its session_answer. Usually asks the user first.",
            Effect::Create,
            asks(vec![req("session", session()), req("question", string())]),
        ),
        Op::SessionType => define(
            "Type text into another session's terminal as one bracketed paste. Usually asks the user first.",
            Effect::Create,
            asks(vec![req("session", session()), req("text", string()), opt("when", when())]),
        ),
        Op::SessionStart => define(
            "Start a new session in your channel. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("machine", machine()),
                opt("agent", string()),
                opt("model", string()),
                opt("repos", array(repo())),
                opt("title", title()),
                opt("first_message", string()),
                req("reason", reason()),
            ]),
        ),
        Op::SessionFork => define(
            "Start a fork of this session. Usually asks the user first; the card shows agent, machine, kind, repos and cost.",
            Effect::Create,
            asks(vec![
                req("machine", machine()),
                req(
                    "kind",
                    one_of_strings(["code_and_context", "code_only", "context_only"]),
                ),
                opt(
                    "repos",
                    array(repo()).describe("Default: all of this session's repos"),
                ),
                opt("title", title()),
                opt("first_message", string()),
                req("reason", reason()),
            ]),
        ),
        Op::NetworkConnect => reaching_out(define(
            "Reach a host and port through the Mac, as a unix socket in your session folder that stays open until your session stops. Always asks the user, and can't be approved while the Mac is away.",
            Effect::Create,
            asks(vec![
                req("host", string()),
                req("port", integer().range(1, 65_535)),
                req("reason", reason()),
            ]),
        )),
        Op::SessionRename => define(
            "Rename your session. Usually asks the user first.",
            Effect::Change,
            asks(vec![req("title", title())]),
        ),
        Op::SessionAddRepo => define(
            "Add a repository of your channel to your session. Usually asks the user first.",
            Effect::Change,
            asks(vec![req("repo", repo())]),
        ),
        Op::SessionDropRepo => define(
            "Drop a repository from your session. Usually asks the user first; the card shows what would be lost.",
            Effect::Remove,
            asks(vec![req("repo", repo())]),
        ),
        Op::SessionOwnCopy => define(
            "Give your session its own copy of the code it shares with its parent, on a new branch; your conversation goes on. Usually asks the user first.",
            Effect::Change,
            asks(vec![]),
        ),
        Op::SessionStop => session_end(
            "Stop a session; your own without session. Your own usually asks the user first; another session's is refused unless the user changed the rule.",
        ),
        Op::SessionArchive => session_end(
            "Archive a session; your own without session. Your own usually asks the user first; another session's is refused unless the user changed the rule.",
        ),
        Op::SessionDelete => session_end(
            "Delete a session; your own without session. Your own usually asks the user first; another session's is refused unless the user changed the rule.",
        ),
        Op::SessionMove => define(
            "Move a session, your own without session, to another machine. Usually asks the user first.",
            Effect::Change,
            asks(vec![
                opt("session", session().describe("Default: your own session")),
                req("machine", machine()),
                req("reason", reason()),
            ]),
        ),
        Op::SessionPush => reaching_out(define(
            "Push your branches and open pull requests. Usually asks the user first; the card shows repos, branch, commits and PR title.",
            Effect::Create,
            asks(vec![
                opt("repos", array(repo()).describe("Default: all of this session's repos")),
                opt("open_pr", boolean()),
                opt("title", string()),
                opt("body", string()),
            ]),
        )),
        Op::PrReviewStart => reaching_out(define(
            "Start a read-only session reviewing pull requests. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("prs", array(string().describe("owner/repo#number"))),
                req("machine", machine()),
                opt("agent", string()),
            ]),
        )),
        Op::MachineAddSsh => reaching_out(define(
            "Add a machine reached over ssh. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("ssh_host", string().describe("A host from your ssh config, or user@host")),
                req("name", string()),
            ]),
        )),
        Op::MachineCreate => reaching_out(define(
            "Create a machine with a provider. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("provider", string()),
                req("name", string()),
                req("params", any_object().describe("The provider's parameters")),
            ]),
        )),
        Op::MachineStart => machine_only(
            "Start a provider's machine. Usually asks the user first.",
            Effect::Change,
        ),
        Op::MachineStop => machine_only(
            "Stop a provider's machine. Usually asks the user first.",
            Effect::Remove,
        ),
        Op::MachineReconnect => machine_only(
            "Reconnect to a machine. Usually asks the user first.",
            Effect::Change,
        ),
        Op::SkillsSync => machine_only(
            "Copy your skills to a machine. Usually asks the user first.",
            Effect::Change,
        ),
        Op::TrashRestore => define(
            "Put something back from the trash. Usually asks the user first.",
            Effect::Change,
            asks(vec![req("trash_item_id", string())]),
        ),
        Op::ChannelCreate => define(
            "Create a channel. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("name", string()),
                req("repos", array(repo())),
                req("machines", array(machine())),
            ]),
        ),
        Op::SessionRestore => define(
            "Bring back an archived session. Usually asks the user first.",
            Effect::Change,
            asks(vec![
                req("session", session()),
                opt("machine", machine()),
                req("reason", reason()),
            ]),
        ),
        Op::SessionRebuild => define(
            "Rebuild a session from its record. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("session", session()),
                opt("machine", machine()),
                req("reason", reason()),
            ]),
        ),
        Op::ChannelRename => define(
            "Rename your channel. Refused unless the user changed the rule.",
            Effect::Change,
            asks(vec![req("name", string())]),
        ),
        Op::ChannelAddRepo => define(
            "Add a repository to your channel. Refused unless the user changed the rule.",
            Effect::Change,
            asks(vec![req("repo", repo())]),
        ),
        Op::ChannelRemoveRepo => define(
            "Remove a repository from your channel. Refused unless the user changed the rule.",
            Effect::Remove,
            asks(vec![req("repo", repo())]),
        ),
        Op::ChannelAddMachine => define(
            "Add a machine to your channel. Refused unless the user changed the rule.",
            Effect::Change,
            asks(vec![req("machine", machine())]),
        ),
        Op::ChannelRemoveMachine => define(
            "Remove a machine from your channel. Refused unless the user changed the rule.",
            Effect::Remove,
            asks(vec![req("machine", machine())]),
        ),
        Op::MachineRemove => machine_only(
            "Remove a machine from Humata. Refused unless the user changed the rule.",
            Effect::Remove,
        ),
        Op::MachineForget => machine_only(
            "Forget a machine Humata can't reach. Refused unless the user changed the rule.",
            Effect::Remove,
        ),
        Op::MachineDelete => machine_only(
            "Delete what a provider made for a machine. Refused unless the user changed the rule.",
            Effect::Remove,
        ),
    }
}
