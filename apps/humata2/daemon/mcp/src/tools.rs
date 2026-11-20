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
    /// `TodoService.NextTodo`.
    TodoNext => "todo_next", "todo.next";
    /// `TodoService.CreateTodo`; an agent's item is created suggested.
    TodoAdd => "todo_add", "todo.add";
    /// `TodoService.UpdateTodo` with the state Closed.
    TodoClose => "todo_close", "todo.close";
    /// `InboxService.AskUser` anchored to a To-do item (free-text, single-choice, or multi-choice).
    TodoAskQuestion => "todo_ask_question", "todo.ask_question";
    /// `InboxService.AskUser` asking the user to perform an external action (`Done` / `Can't be done`).
    TodoRequestAction => "todo_request_action", "todo.request_action";
    /// `InboxService.AskUser` asking the user to review work on a To-do item (`Approved` / `Changes asked`).
    TodoRequestReview => "todo_request_review", "todo.request_review";
    /// `TodoService.UpdateTodo` updating checklist steps (markdown context, drop/restore, reorder).
    TodoStepUpdate => "todo_step_update", "todo.step_update";
    /// `TodoService.UpdateTodo` setting the stable-state checkpoint (`with_context`, `defined`, `done`).
    TodoSetCheckpoint => "todo_set_checkpoint", "todo.set_checkpoint";
    /// `TodoService.AddTodoComment` recording an agent run (tool, model, outcome, cost, links).
    TodoReportRun => "todo_report_run", "todo.report_run";
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
    /// `PullRequestReviewService.ListPullRequestCommits` of a pull request
    /// in the caller's review.
    ReviewCommits => "review_commits", "review.commits";
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
    /// `DeskService.ListDocVersions` of a Desk session's folder (DV-9).
    DocVersionsList => "doc_versions_list", "doc.versions_list";
    /// `DeskService.ReadDocVersion` (DV-9).
    DocVersionRead => "doc_version_read", "doc.version_read";
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
    /// `InboxService.ExplainAsk` of the caller's own request.
    InboxExplain => "inbox_explain", "inbox.explain";
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
    /// `SessionService.ContinueInChannel` of the caller (CV-8).
    SessionContinueInChannel => "session_continue_in_channel", "session.continue_in_channel";
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
    /// `SessionService.PlanDeleteSessions` and `DeleteSessions` for picked
    /// sessions, one channel or one machine; never all (MASSDELETE, D876).
    SessionsDeleteMany => "sessions_delete_many", "session.delete_many";
    /// `SessionService.MoveSession`.
    SessionMove => "session_move", "session.move";
    /// `DeskService.MoveDeskSessionToChannel` of the caller's own Desk session.
    DeskSessionMoveToChannel => "desk_session_move_to_channel", "desk.move_to_channel";
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
    /// `ProviderService.RequestProviderImport` (MP-23, D792): always asks.
    ProviderImport => "provider_import", "provider.import";
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
    // REPOS
    /// `RepoService.SearchRepos`: the library, then GitHub within the scope
    /// (RL-11).
    RepoSearch => "repo_search", "repo.search";
    /// `RepoService.ListRepoLibrary`.
    RepoLibrary => "repo_library", "repo.library";
}

impl Operation {
    /// True when the tool's schema requires `reason`. A call with it empty
    /// or blank must be refused: the schema only checks that it is there.
    pub fn requires_reason(self) -> bool {
        input_schema(self).requires("reason")
    }

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

/// The `tools/list` result: every tool the daemon runs (`serves`), never a
/// user-only one.
pub(crate) fn list(serves: impl Fn(Operation) -> bool) -> Value {
    let output = output_schema();
    let tools: Vec<Value> = Operation::ALL
        .iter()
        .filter(|operation| serves(**operation))
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
        Capability::TodoSuggest => "todo_suggest",
        Capability::TodoClose => "todo_close",
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
        Capability::ChannelRepoAdd => "channel_repo_add",
        Capability::NetworkReach => "network_reach",
        Capability::DeskMoveToChannel => "desk_move_to_channel",
        Capability::SessionStopOther => "session_stop_other",
        Capability::SessionDeleteOther => "session_delete_other",
        Capability::SessionsDeleteMany => "sessions_delete_many",
        Capability::MachineSessionsStop => "machine_sessions_stop",
        Capability::ChannelChange => "channel_change",
        Capability::MachineDestroy => "machine_destroy",
        Capability::MachineScriptRun => "machine_script_run",
        Capability::ProviderImport => "provider_import",
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
    string().describe("An item id such as PAY-7")
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

/// Whose folder a doc tool reads: the caller's own unless it names one.
fn doc_location() -> Schema {
    object(vec![opt("session", session().describe("Default: your session"))])
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
        object(vec![
            req("root", one_of_strings(["session_folder"])),
            opt(
                "session",
                session().describe("The session whose own folder holds the file (a Desk session's documents). Default: your session"),
            ),
            req("path", relative_path()),
            opt("machine", machine().describe("Default: your machine")),
        ]),
    ])
    .describe("A file or folder: in a clone, shared/, the channel folder, a main clone (read-only) or a session's own folder")
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

/// A stack layer's title (D800); compared ignoring case.
fn layer() -> Schema {
    string().max_length(60)
}

/// Files marked key or skim (D802); unlisted files are normal.
fn importance() -> Schema {
    array(object(vec![
        req("repo", repo()),
        req("path", relative_path()),
        req(
            "tier",
            one_of_strings(["key", "normal", "skim"])
                .describe("key: read first; skim: folded at the end; normal un-skims a lockfile or generated file"),
        ),
    ]))
    .describe("Each file once")
}

/// A stack of layers, bottom first (D800, `humata.v1.ReviewStack`).
fn stack() -> Schema {
    object(vec![req(
        "layers",
        array(object(vec![
            req("title", layer().describe("Short; unique in the stack")),
            opt("purpose", string().max_length(120).describe("One line, for navigating")),
            req(
                "repos",
                array(
                    object(vec![
                        req("repo", repo()),
                        opt(
                            "range",
                            object(vec![
                                req("base", string().describe("The layer below's head, or the trunk")),
                                req("head", string().describe("A commit, branch or tag")),
                            ]),
                        ),
                        opt(
                            "worktree",
                            object(vec![req(
                                "include_paths",
                                array(string().describe("A git pathspec; \".\" for everything")),
                            )])
                            .describe("Uncommitted work on top of HEAD"),
                        ),
                        opt("pull_request", string().describe("owner/repo#number")),
                    ])
                    .exactly_one_of(&["range", "worktree"]),
                ),
            ),
            opt("order", array(file_ref()).describe("This layer's reading order")),
            opt(
                "groups",
                array(object(vec![
                    req("title", title()),
                    opt("note", string()),
                    req("first", file_ref()),
                ])),
            ),
            opt("importance", importance()),
        ]))
        .max_items(50)
        .describe("Bottom of the stack first; after from_stack's layers when both are sent"),
    )])
}

/// A stack read from the repo's own stack metadata (D803).
fn from_stack() -> Schema {
    object(vec![
        req("repo", repo().describe("The daemon reads `stack status` in this session's checkout of it")),
        opt("tip", string().describe("The topic to end at; needed when the stack forks")),
        opt(
            "topics",
            array(object(vec![
                req("topic", string()),
                opt("title", layer()),
                opt("purpose", string().max_length(120)),
            ]))
            .describe("Your title or purpose for some topics"),
        ),
    ])
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
                        "stop_sessions",
                        "start_sessions",
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
                opt("board", board().describe("Default: your session board")),
                req("title", title()),
                opt("column", column().describe("Default: to_do")),
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
        Op::TodoNext => define(
            "The top-ranked open item of the user's To-do list, or null when nothing is open. Filter by channel or session.",
            Effect::Read,
            asks(vec![
                opt("channel", string().describe("A channel id; default: every channel")),
                opt("session", session().describe("Only items this session works on")),
            ]),
        ),
        Op::TodoAdd => define(
            "Suggest a To-do item. It is added as suggested and stays out of the ranked list until the user says Yes; you cannot accept it yourself.",
            Effect::Create,
            asks(vec![
                req("title", title()),
                opt("channel", string().describe("A channel id; default: no channel")),
                opt("description", string().max_length(20_000)),
                opt("due", string().describe("A day as YYYY-MM-DD")),
                opt("effort", one_of_strings(["s", "m", "l"])),
                opt("impact", one_of_strings(["low", "medium", "high"])),
                opt("tags", array(string().max_length(60))),
                opt("steps", array(string().max_length(500)).describe("Step texts, in order")),
                opt("evidence", array(string().max_length(2_000)).describe("What shows it is needed; shown when the user opens it")),
            ]),
        ),
        Op::TodoClose => define(
            "Close a To-do item because it is done. It is a request: the user's rules may send it to the Inbox, and the item stays open until they agree.",
            Effect::Change,
            asks(vec![
                req("key", string().describe("An item key such as PAY-3")),
                req("reason", reason()),
                opt("evidence", array(string().max_length(2_000)).describe("What shows it is done")),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::TodoAskQuestion => define(
            "Ask the user a structured question about a To-do item (free-text, single-choice, or multi-choice), optionally anchored to a checklist step with expandable Markdown details, and wait for the reply.",
            Effect::Create,
            object(vec![
                req("key", string().describe("An item key or id such as PAY-3")),
                req("question", string().max_length(2_000)),
                opt(
                    "choices",
                    array(string().max_length(200))
                        .max_items(4)
                        .describe("Up to 4 choices; omit for a free-text question"),
                ),
                opt(
                    "multi_select",
                    boolean().describe("Allow selecting multiple choices; default false"),
                ),
                opt(
                    "step_id",
                    string()
                        .max_length(120)
                        .describe("Optional checklist step id this question is anchored to"),
                ),
                opt(
                    "details",
                    string()
                        .max_length(10_000)
                        .describe("Optional expandable Markdown details"),
                ),
                opt("wait_seconds", wait_seconds()),
            ]),
        ),
        Op::TodoRequestAction => define(
            "Ask the user to perform an external action for a To-do item and wait for Done or Can't be done.",
            Effect::Create,
            object(vec![
                req("key", string().describe("An item key or id such as PAY-3")),
                req(
                    "action",
                    string()
                        .max_length(500)
                        .describe("The external action needed from the user"),
                ),
                opt(
                    "step_id",
                    string().max_length(120).describe("Optional checklist step id"),
                ),
                opt(
                    "details",
                    string()
                        .max_length(10_000)
                        .describe("Optional expandable Markdown details"),
                ),
                opt("wait_seconds", wait_seconds()),
            ]),
        ),
        Op::TodoRequestReview => define(
            "Ask the user to review work on a To-do item (distinct from closing it) and wait for Approved or Changes asked.",
            Effect::Create,
            object(vec![
                req("key", string().describe("An item key or id such as PAY-3")),
                req(
                    "summary",
                    string()
                        .max_length(500)
                        .describe("Short summary of what is ready for review"),
                ),
                opt(
                    "step_id",
                    string().max_length(120).describe("Optional checklist step id"),
                ),
                opt(
                    "details",
                    string()
                        .max_length(10_000)
                        .describe("Optional expandable Markdown details"),
                ),
                opt(
                    "round",
                    integer()
                        .range(1, 100)
                        .describe("Review round number, starting at 1"),
                ),
                opt(
                    "checkpoint",
                    one_of_strings(["with_context", "defined", "done"])
                        .describe("Optional checkpoint this review checks"),
                ),
                opt("wait_seconds", wait_seconds()),
            ]),
        ),
        Op::TodoStepUpdate => define(
            "Update a To-do item's checklist steps: set Markdown context, soft-drop or restore steps, or reorder them.",
            Effect::Change,
            asks(vec![
                req("key", string().describe("An item key or id such as PAY-3")),
                req(
                    "steps",
                    array(object(vec![
                        opt("id", string().max_length(120).describe("Stable step id")),
                        req("text", string().max_length(500).describe("Step text")),
                        opt("done", boolean().describe("Whether the step is completed")),
                        opt(
                            "markdown_context",
                            string()
                                .max_length(10_000)
                                .describe("Optional Markdown context for this step"),
                        ),
                        opt(
                            "dropped",
                            boolean().describe("Soft-drop (true) or restore (false) this step"),
                        ),
                    ]))
                    .describe("The full list of steps in their desired order"),
                ),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::TodoSetCheckpoint => define(
            "Set a To-do item's stable-state handoff checkpoint (with_context, defined, or done).",
            Effect::Change,
            asks(vec![
                req("key", string().describe("An item key or id such as PAY-3")),
                req("checkpoint", one_of_strings(["with_context", "defined", "done"])),
                opt("expected_revision", expected_revision()),
            ]),
        ),
        Op::TodoReportRun => define(
            "Record an agent run (tool, model, outcome, cost, and links) on a To-do item.",
            Effect::Create,
            asks(vec![
                req("key", string().describe("An item key or id such as PAY-3")),
                req(
                    "tool",
                    string()
                        .max_length(120)
                        .describe("Tool or agent name, e.g. claude"),
                ),
                opt(
                    "model",
                    string()
                        .max_length(120)
                        .describe("Model identifier used for the run"),
                ),
                req(
                    "outcome",
                    string().max_length(500).describe("Short outcome summary"),
                ),
                opt(
                    "cost_micros",
                    integer()
                        .at_least(0)
                        .describe("Run cost in microdollars (1000000 = $1.00)"),
                ),
                opt(
                    "links",
                    array(object(vec![
                        req("label", string().max_length(120)),
                        req("target", string().max_length(1_000)),
                    ]))
                    .max_items(20)
                    .describe("Optional links produced by the run"),
                ),
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
        Op::ReviewCommits => define(
            "The commits of one of this review's pull requests, oldest first, with which ones the user marked reviewed, which marks a force push carried to the same patch, and the history of marks. Only the user marks commits.",
            Effect::Read,
            object(vec![req("pr", string().describe("owner/repo#number"))]),
        ),
        Op::ReviewContext => define(
            "What the user is looking at in this review: the diff spec, the open file, the visible lines and selection, the order and groups, Viewed files and your notes.",
            Effect::Read,
            object(Vec::new()),
        ),
        Op::ReviewFocus => define(
            "Show the user a place in the review: open a file, scroll to and select lines, and highlight ranges with a short label, or move a stack review to one layer. It applies once the user stops scrolling or typing; they can Undo it or go Back.",
            Effect::Change,
            object(vec![
                opt("layer", layer().describe("Move to this stack layer, by title; \"\" for Whole stack. With no repo and path, only the layer changes")),
                opt("repo", repo()),
                opt("path", relative_path()),
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
                opt("layer", layer().describe("The stack layer the note is on, by title; leave out for Whole stack")),
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
            "Arrange a review for the user: what it compares (a diff spec, or a stack of layers the user steps through), the order to read the files in, groups with titles, which files are key or skim, and notes on files, hunks and lines. Send spec, or stack and/or from_stack, never both kinds. In a review chat it applies at once; elsewhere it is a draft view the user opens.",
            Effect::Create,
            asks(vec![
                req("title", title()),
                opt("spec", diff_spec()),
                opt("stack", stack()),
                opt("from_stack", from_stack()),
                opt("importance", importance()),
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
                        opt("layer", layer().describe("The stack layer the note is on; leave out for Whole stack")),
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
        Op::DocVersionsList => define(
            "List the versions of a file in a Desk session's folder, newest first, and whether the folder has changes no version holds. Your own session's folder needs no approval; another session's needs the user's rule.",
            Effect::Read,
            object(vec![req("path", relative_path()), opt("location", doc_location())]),
        ),
        Op::DocVersionRead => define(
            "Read one version of a file in a Desk session's folder: its text and, when they were recorded, the comments it had. Take the version id from doc_versions_list.",
            Effect::Read,
            object(vec![
                req("path", relative_path()),
                req("version", string().describe("A version id from doc_versions_list")),
                opt("location", doc_location()),
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
        // REPOS
        Op::RepoSearch => reaching_out(define(
            "Search the user's repo library, then GitHub: library repos first, most recently used first, then what gh finds within the user's saved scope. Name owners, or set all_of_github, to widen this one call; that is never saved and you cannot change the saved scope. A GitHub failure comes back as problem, with the library still answered.",
            Effect::Read,
            asks(vec![
                req("query", string().max_length(200)),
                opt(
                    "owners",
                    array(string().describe("A GitHub user or organization, such as acme"))
                        .max_items(20)
                        .describe("Search these owners for this call instead of the saved scope"),
                ),
                opt(
                    "all_of_github",
                    boolean().describe("Search all of GitHub for this call; not with owners"),
                ),
                opt("page_token", page_token()),
            ]),
        )),
        Op::RepoLibrary => define(
            "List the user's repo library, most recently used first, with where each repo was last used.",
            Effect::Read,
            asks(vec![]),
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
        Op::InboxExplain => define(
            "Answer the user's Ask why on one of your Inbox requests: say, briefly, why the task needs it. The request stays pending; call inbox_wait again afterwards.",
            Effect::Change,
            object(vec![
                req("request_id", string()),
                req("text", string().max_length(1000)),
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
            "Type text into another session's terminal and press Enter; submit: false leaves it in the input. Usually asks the user first.",
            Effect::Create,
            asks(vec![
                req("session", session()),
                req("text", string()),
                opt("submit", boolean()),
                opt("when", when()),
            ]),
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
                opt("agent", string().describe("The fork's agent; default: this session's agent")),
                opt(
                    "before_turn",
                    integer().range(2, 1_000_000).describe("Fork from just before this turn; default: from now"),
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
        Op::SessionContinueInChannel => define(
            "Start a new session in another channel that goes on from a copy of this session's conversation; this session stays as it is. Usually asks the user first; the card shows the channel and the turn.",
            Effect::Create,
            asks(vec![
                req("channel", string().describe("A channel id from channels_list; not this session's own channel, nor the Desk")),
                opt(
                    "before_turn",
                    integer().range(2, 1_000_000).describe("Go on from just before this turn; default: the whole conversation"),
                ),
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
            "Stop a session; your own without session. Usually asks the user first, for your own session and for another's.",
        ),
        Op::SessionArchive => session_end(
            "Archive a session; your own without session. Usually asks the user first, for your own session and for another's.",
        ),
        Op::SessionDelete => session_end(
            "Delete a session; your own without session. Your own usually asks the user first; another session's is refused unless the user changed the rule.",
        ),
        Op::SessionsDeleteMany => define(
            "Delete several sessions at once: the session_ids you name, or every session of one channel or one machine. Never your own session, never all. Asks the user first; a session with unpushed work blocks the delete unless delete_unpushed_work is true.",
            Effect::Remove,
            asks(vec![
                opt("session_ids", array(session()).describe("Sessions to delete, from topology_get")),
                opt("channel", string().describe("A channel id from channels_list")),
                opt("machine", machine()),
                opt(
                    "delete_unpushed_work",
                    boolean().describe("Also delete uncommitted or unpushed git work; default false"),
                ),
            ])
            .exactly_one_of(&["session_ids", "channel", "machine"]),
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
        Op::DeskSessionMoveToChannel => define(
            "Move your Desk session into a channel: its folder becomes a session folder there, each repository on a new branch with your uncommitted work, and repositories the channel lacks are added to it. Asks the user first, under that channel's rule.",
            Effect::Change,
            asks(vec![
                req("channel", string().describe("A channel id from channels_list")),
                opt(
                    "branch",
                    string()
                        .max_length(200)
                        .describe("The branch to put the work on; default: one from the session's title"),
                ),
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
        Op::ProviderImport => define(
            "Ask the user to import a machine provider file. Always asks: the Inbox shows the whole file and every command it runs, and only the user's approval adds it. A file with problems is refused at once.",
            Effect::Create,
            asks(vec![
                req(
                    "file_text",
                    string().describe("The provider TOML, at most 64 KiB, exactly as it should be imported"),
                ),
                opt("file_name", string().describe("A name for the Inbox card; no file is read")),
                req("reason", string().describe("Why the task needs this provider")),
            ]),
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
            "Propose adding a GitHub repository (owner and name; never a local path) to your channel, or to any channel from the Desk. The user is asked first, so give a reason.",
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
