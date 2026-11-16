//! The daemon as the commands see it: gRPC calls named by their path, with
//! encoded messages going in and out, so a test can stand in for it.

use std::future::Future;
use std::path::PathBuf;

use humata2_proto::humata::v1 as pb;
use tokio::sync::mpsc;

/// One gRPC method: its path, and the message it answers with, which is
/// what `--json` walks to print the reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rpc {
    pub path: &'static str,
    pub response: &'static str,
}

/// Replies on a stream, in order. The sender closes it after the last one.
pub type Replies = mpsc::Receiver<Result<Vec<u8>, DaemonError>>;

/// What `humata` asks of the daemon. Every command is calls on this; the
/// live one is [`crate::live::LiveDaemon`].
pub trait Daemon {
    fn unary(&self, rpc: Rpc, request: Vec<u8>) -> impl Future<Output = Result<Vec<u8>, DaemonError>>;

    fn server_stream(&self, rpc: Rpc, request: Vec<u8>) -> impl Future<Output = Result<Replies, DaemonError>>;

    fn client_stream(
        &self,
        rpc: Rpc,
        requests: mpsc::Receiver<Vec<u8>>,
    ) -> impl Future<Output = Result<Vec<u8>, DaemonError>>;

    fn bidi_stream(
        &self,
        rpc: Rpc,
        requests: mpsc::Receiver<Vec<u8>>,
    ) -> impl Future<Output = Result<Replies, DaemonError>>;

    /// I9 bug reports (D31, D133): asked of the running app over its own
    /// socket, not of humatad.
    fn report(&self, request: ReportRequest) -> impl Future<Output = Result<ReportReply, DaemonError>>;
}

/// `humata report record|stop|snapshot`, until the proto has a message
/// for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportRequest {
    /// Start recording interactions, actions, trees and RPCs.
    Record { title: Option<String> },
    /// Stop the recording and save it.
    Stop,
    /// Save the state, view and layout trees now.
    Snapshot { title: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportReply {
    /// The report's folder, once saved; empty while recording.
    pub folder: String,
    pub recording: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// The daemon answered with a Humata error (GRPC.md §10).
    #[error("{}", .error.message)]
    Refused {
        rpc: &'static str,
        code: i32,
        error: Box<pb::Error>,
    },
    /// A gRPC status without a Humata error in it.
    #[error("{rpc} failed with gRPC status {code}: {message}")]
    Status {
        rpc: &'static str,
        code: i32,
        message: String,
    },
    #[error("couldn't connect to humatad at {}", .socket.display())]
    Connect {
        socket: PathBuf,
        #[source]
        source: tonic::transport::Error,
    },
    #[error("humatad isn't running: no socket at {}", list(.tried))]
    NotRunning { tried: Vec<PathBuf> },
    #[error("can't find humatad's socket: HOME isn't set")]
    NoHome,
    /// A token gRPC metadata can't carry (not visible ASCII).
    #[error("{variable} isn't a token humata can send")]
    BadToken { variable: &'static str },
    #[error("the reply to {rpc} isn't a {expected}")]
    Decode {
        rpc: &'static str,
        expected: &'static str,
        #[source]
        source: prost::DecodeError,
    },
    #[error("{rpc} ended before it answered")]
    Ended { rpc: &'static str },
    #[error("this humatad can't make bug reports yet: the API has no ReportService (I9, D31)")]
    NoReportService,
    /// The app's report socket couldn't be reached (D133).
    #[error("couldn't reach the Humata 2 app for the report: {source}")]
    App {
        #[source]
        source: humata2_report::remote::RemoteError,
    },
    /// The app was reached and said no.
    #[error("the Humata 2 app didn't do it: {}", .refusal.message)]
    AppRefused {
        refusal: humata2_report::remote::Refusal,
    },
}

fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

macro_rules! service {
    ($module:ident, $service:literal { $($name:ident = $method:literal -> $response:literal),* $(,)? }) => {
        pub mod $module {
            use super::Rpc;
            $(
                pub const $name: Rpc = Rpc {
                    path: concat!("/humata.v1.", $service, "/", $method),
                    response: concat!("humata.v1.", $response),
                };
            )*
        }
    };
}

/// The calls `humata` makes, by service.
pub mod rpc {
    use super::Rpc;

    service!(daemon, "DaemonService" {
        GET_SERVER_INFO = "GetServerInfo" -> "ServerInfo",
        WHO_AM_I = "WhoAmI" -> "WhoAmIResponse",
    });

    service!(access, "AccessService" {
        GET_CHANNEL_RULES = "GetChannelRules" -> "ChannelRules",
        SET_CHANNEL_RULE = "SetChannelRule" -> "ChannelRules",
        SET_AWAY_RULE = "SetAwayRule" -> "ChannelRules",
        LIST_GRANTS = "ListGrants" -> "ListGrantsResponse",
        REVOKE_GRANT = "RevokeGrant" -> "RevokeGrantResponse",
        EXPLAIN = "Explain" -> "ExplainResponse",
        SET_APPROVAL_PASSPHRASE = "SetApprovalPassphrase" -> "SetApprovalPassphraseResponse",
    });

    service!(cli_auth, "CliAuthService" {
        BEGIN_CLI_SESSION = "BeginCliSession" -> "CliSession",
        END_CLI_SESSION = "EndCliSession" -> "EndCliSessionResponse",
        LIST_CLI_SESSIONS = "ListCliSessions" -> "ListCliSessionsResponse",
        CREATE_SCRIPT_TOKEN = "CreateScriptToken" -> "CreateScriptTokenResponse",
        LIST_SCRIPT_TOKENS = "ListScriptTokens" -> "ListScriptTokensResponse",
        REVOKE_SCRIPT_TOKEN = "RevokeScriptToken" -> "RevokeScriptTokenResponse",
    });

    service!(audit, "AuditService" {
        LIST_AUDIT = "ListAudit" -> "ListAuditResponse",
    });

    service!(board, "BoardService" {
        LIST_ITEMS = "ListItems" -> "ListItemsResponse",
        CREATE_ITEM = "CreateItem" -> "WriteItemResponse",
        UPDATE_ITEM = "UpdateItem" -> "WriteItemResponse",
        MOVE_ITEM = "MoveItem" -> "WriteItemResponse",
        DELETE_ITEM = "DeleteItem" -> "WriteItemResponse",
        GET_ITEM_HISTORY = "GetItemHistory" -> "GetItemHistoryResponse",
    });

    service!(channel, "ChannelService" {
        CREATE_CHANNEL = "CreateChannel" -> "CreateChannelResponse",
        GET_CHANNEL = "GetChannel" -> "Channel",
        LIST_CHANNELS = "ListChannels" -> "ListChannelsResponse",
        RENAME_CHANNEL = "RenameChannel" -> "Channel",
        ADD_REPO = "AddRepo" -> "Operation",
        PLAN_REMOVE_REPO = "PlanRemoveRepo" -> "RemoveRepoPlan",
        REMOVE_REPO = "RemoveRepo" -> "Channel",
        ADD_MACHINE = "AddMachine" -> "Operation",
        PLAN_REMOVE_MACHINE = "PlanRemoveMachine" -> "RemoveChannelMachinePlan",
        REMOVE_MACHINE = "RemoveMachine" -> "Operation",
        GET_GRID = "GetGrid" -> "ChannelGrid",
        PLAN_DELETE_CHANNEL = "PlanDeleteChannel" -> "DeleteChannelPlan",
        DELETE_CHANNEL = "DeleteChannel" -> "DeleteChannelResponse",
    });

    service!(activity, "ActivityService" {
        LIST_ACTIVITY = "ListActivity" -> "ListActivityResponse",
    });

    service!(events, "EventService" {
        SUBSCRIBE = "Subscribe" -> "Event",
    });

    service!(files, "FileService" {
        LIST_DIRECTORY = "ListDirectory" -> "ListDirectoryResponse",
        READ_FILE = "ReadFile" -> "FileChunk",
        WRITE_FILE = "WriteFile" -> "FileEntry",
        CREATE_FOLDER = "CreateFolder" -> "FileEntry",
        RENAME = "Rename" -> "FileEntry",
        MOVE_TO_TRASH = "MoveToTrash" -> "MoveToTrashResponse",
        FIND_FILES = "FindFiles" -> "FindFilesResponse",
    });

    service!(transfer, "TransferService" {
        START_TRANSFER = "StartTransfer" -> "Transfer",
        LIST_TRANSFERS = "ListTransfers" -> "ListTransfersResponse",
        PAUSE_TRANSFER = "PauseTransfer" -> "Transfer",
        RESUME_TRANSFER = "ResumeTransfer" -> "Transfer",
        CANCEL_TRANSFER = "CancelTransfer" -> "Transfer",
    });

    service!(trash, "TrashService" {
        LIST_TRASH = "ListTrash" -> "ListTrashResponse",
        RESTORE_FROM_TRASH = "RestoreFromTrash" -> "RestoreFromTrashResponse",
        PLAN_EMPTY_TRASH = "PlanEmptyTrash" -> "EmptyTrashPlan",
        EMPTY_TRASH = "EmptyTrash" -> "EmptyTrashResponse",
    });

    service!(machine, "MachineService" {
        LIST_MACHINES = "ListMachines" -> "ListMachinesResponse",
        GET_MACHINE = "GetMachine" -> "MachineDetail",
        ADD_SSH_MACHINE = "AddSshMachine" -> "Operation",
        RECONNECT_MACHINE = "ReconnectMachine" -> "Machine",
        PLAN_REMOVE_MACHINE = "PlanRemoveMachine" -> "RemoveMachinePlan",
        REMOVE_MACHINE = "RemoveMachine" -> "Operation",
        PLAN_FORGET_MACHINE = "PlanForgetMachine" -> "ForgetMachinePlan",
        FORGET_MACHINE = "ForgetMachine" -> "ForgetMachineResponse",
        PLAN_CHANGE_ROOT = "PlanChangeRoot" -> "ChangeRootPlan",
        CHANGE_ROOT = "ChangeRoot" -> "Operation",
        SET_VM_TIER = "SetVmTier" -> "Machine",
        GET_STORAGE = "GetStorage" -> "StorageReport",
        CLEAR_BUILD_CACHE = "ClearBuildCache" -> "ClearBuildCacheResponse",
        TRIM_LOGS = "TrimLogs" -> "TrimLogsResponse",
        LIST_HOST_VARIABLES = "ListHostVariables" -> "ListHostVariablesResponse",
        SET_HOST_VARIABLE = "SetHostVariable" -> "SetHostVariableResponse",
        REMOVE_HOST_VARIABLE = "RemoveHostVariable" -> "SetHostVariableResponse",
        TRUST_NEW_HOST_KEY = "TrustNewHostKey" -> "Machine",
    });

    service!(provider, "ProviderService" {
        LIST_PROVIDERS = "ListProviders" -> "ListProvidersResponse",
        GET_PROVIDERS_CONFIG = "GetProvidersConfig" -> "ProvidersConfig",
        SET_PROVIDERS_CONFIG = "SetProvidersConfig" -> "SetProvidersConfigResponse",
        PREVIEW_PROVIDER_IMPORT = "PreviewProviderImport" -> "ProviderImportPlan",
        IMPORT_PROVIDER = "ImportProvider" -> "SetProvidersConfigResponse",
        CREATE_MACHINE = "CreateMachine" -> "Operation",
        RUN_PROVIDER_ACTION = "RunProviderAction" -> "Operation",
        INTERACTIVE_LOGIN = "InteractiveLogin" -> "LoginServerMessage",
    });

    service!(skills, "SkillsService" {
        PREVIEW_SKILLS_SYNC = "PreviewSkillsSync" -> "SkillsSyncPlan",
        SYNC_SKILLS = "SyncSkills" -> "Operation",
    });

    service!(bridge, "BridgeService" {
        CLOSE_PORT_FORWARD = "ClosePortForward" -> "ClosePortForwardResponse",
        LIST_PORT_FORWARDS = "ListPortForwards" -> "ListPortForwardsResponse",
    });

    service!(inbox, "InboxService" {
        LIST_REQUESTS = "ListRequests" -> "ListRequestsResponse",
        GET_REQUEST = "GetRequest" -> "InboxRequest",
        ANSWER_REQUEST = "AnswerRequest" -> "InboxRequest",
        ANSWER_ALL = "AnswerAll" -> "AnswerAllResponse",
        REQUEST_ACCESS = "RequestAccess" -> "WaitResponse",
        WAIT_FOR_ANSWER = "WaitForAnswer" -> "WaitResponse",
    });

    service!(operation, "OperationService" {
        GET_OPERATION = "GetOperation" -> "Operation",
        WATCH_OPERATION = "WatchOperation" -> "Operation",
        LIST_OPERATIONS = "ListOperations" -> "ListOperationsResponse",
        CANCEL_OPERATION = "CancelOperation" -> "Operation",
        RETRY_OPERATION = "RetryOperation" -> "Operation",
    });

    service!(diff, "DiffService" {
        GET_DIFF = "GetDiff" -> "GetDiffResponse",
        MARK_REVIEWED = "MarkReviewed" -> "MarkReviewedResponse",
    });

    service!(comment, "CommentService" {
        LIST_COMMENTS = "ListComments" -> "ListCommentsResponse",
        CREATE_COMMENT = "CreateComment" -> "Comment",
        DROP_COMMENT = "DropComment" -> "DropCommentResponse",
        SEND_COMMENTS = "SendComments" -> "SendCommentsResponse",
    });

    service!(review, "PullRequestReviewService" {
        RESOLVE_PULL_REQUEST = "ResolvePullRequest" -> "PullRequestInfo",
        START_REVIEW = "StartReview" -> "Operation",
        SET_PUSH_ALLOWED = "SetPushAllowed" -> "SetPushAllowedResponse",
        PLAN_SUBMIT = "PlanSubmit" -> "SubmitPlan",
        SUBMIT_REVIEW = "SubmitReview" -> "SubmitReviewResponse",
    });

    service!(view, "ViewService" {
        LIST_VIEWS = "ListViews" -> "ListViewsResponse",
        SET_VIEW_STATE = "SetViewState" -> "PreparedView",
    });

    service!(session, "SessionService" {
        START_SESSION = "StartSession" -> "StartSessionResponse",
        LIST_SESSIONS = "ListSessions" -> "ListSessionsResponse",
        RENAME_SESSION = "RenameSession" -> "Session",
        STOP_SESSION = "StopSession" -> "Session",
        RESUME_SESSION = "ResumeSession" -> "Operation",
        PREVIEW_FORK = "PreviewFork" -> "ForkPreview",
        FORK_SESSION = "ForkSession" -> "Operation",
        GIVE_OWN_COPY = "GiveOwnCopy" -> "Operation",
        MOVE_SESSION = "MoveSession" -> "Operation",
        PLAN_ARCHIVE = "PlanArchive" -> "ArchivePlan",
        ARCHIVE_SESSION = "ArchiveSession" -> "Operation",
        RESTORE_SESSION = "RestoreSession" -> "Operation",
        PLAN_DELETE = "PlanDelete" -> "DeletePlan",
        DELETE_SESSION = "DeleteSession" -> "DeleteSessionResponse",
        REBUILD_SESSION = "RebuildSession" -> "Operation",
        ADD_REPO_TO_SESSION = "AddRepoToSession" -> "Operation",
        PLAN_DROP_REPO = "PlanDropRepo" -> "DropRepoPlan",
        DROP_REPO = "DropRepo" -> "Session",
        PLAN_PUSH = "PlanPush" -> "PushPlan",
        PUSH_AND_OPEN_PULL_REQUESTS = "PushAndOpenPullRequests" -> "Operation",
        READ_CONVERSATION = "ReadConversation" -> "ReadConversationResponse",
        SEND_NOTE = "SendNote" -> "SendNoteResponse",
        ASK_SESSION = "AskSession" -> "AskSessionResponse",
        REPORT_TURN = "ReportTurn" -> "ReportTurnResponse",
        HOOK = "Hook" -> "HookReply",
    });

    service!(settings, "SettingsService" {
        GET_SETTINGS = "GetSettings" -> "Settings",
        UPDATE_SETTINGS = "UpdateSettings" -> "Settings",
        GET_CHANNEL_SETTINGS = "GetChannelSettings" -> "ChannelSettings",
        UPDATE_CHANNEL_SETTINGS = "UpdateChannelSettings" -> "ChannelSettings",
    });

    service!(agents, "AgentService" {
        LIST_AGENTS = "ListAgents" -> "ListAgentsResponse",
        ADD_AGENT = "AddAgent" -> "AgentDefinition",
        UPDATE_AGENT = "UpdateAgent" -> "AgentDefinition",
    });

    service!(marks, "MarkService" {
        SET_MARKS = "SetMarks" -> "Marks",
        LIST_PINNED = "ListPinned" -> "ListPinnedResponse",
        REORDER_PINNED = "ReorderPinned" -> "ListPinnedResponse",
    });

    service!(history, "HistoryService" {
        SEARCH_SESSIONS = "SearchSessions" -> "SearchSessionsResponse",
        READ_TURNS = "ReadTurns" -> "ReadTurnsResponse",
    });

    service!(desk, "DeskService" {
        KEEP_DESK_FOLDER = "KeepDeskFolder" -> "DeskSession",
    });

    service!(terminal, "TerminalService" {
        OPEN_MACHINE_SHELL = "OpenMachineShell" -> "Tab",
        ATTACH = "Attach" -> "AttachServerMessage",
        TYPE = "Type" -> "TypeResponse",
        LIST_QUEUED = "ListQueued" -> "ListQueuedResponse",
        SEND_QUEUED_NOW = "SendQueuedNow" -> "SendQueuedNowResponse",
    });

    /// Every call above, so a test can check each has a descriptor.
    pub const ALL: &[Rpc] = &[
        daemon::GET_SERVER_INFO,
        daemon::WHO_AM_I,
        access::GET_CHANNEL_RULES,
        access::SET_CHANNEL_RULE,
        access::SET_AWAY_RULE,
        access::LIST_GRANTS,
        access::REVOKE_GRANT,
        access::EXPLAIN,
        access::SET_APPROVAL_PASSPHRASE,
        cli_auth::BEGIN_CLI_SESSION,
        cli_auth::END_CLI_SESSION,
        cli_auth::LIST_CLI_SESSIONS,
        cli_auth::CREATE_SCRIPT_TOKEN,
        cli_auth::LIST_SCRIPT_TOKENS,
        cli_auth::REVOKE_SCRIPT_TOKEN,
        audit::LIST_AUDIT,
        board::LIST_ITEMS,
        board::CREATE_ITEM,
        board::UPDATE_ITEM,
        board::MOVE_ITEM,
        board::DELETE_ITEM,
        board::GET_ITEM_HISTORY,
        channel::CREATE_CHANNEL,
        channel::GET_CHANNEL,
        channel::LIST_CHANNELS,
        channel::RENAME_CHANNEL,
        channel::ADD_REPO,
        channel::PLAN_REMOVE_REPO,
        channel::REMOVE_REPO,
        channel::ADD_MACHINE,
        channel::PLAN_REMOVE_MACHINE,
        channel::REMOVE_MACHINE,
        channel::GET_GRID,
        channel::PLAN_DELETE_CHANNEL,
        channel::DELETE_CHANNEL,
        activity::LIST_ACTIVITY,
        events::SUBSCRIBE,
        files::LIST_DIRECTORY,
        files::READ_FILE,
        files::WRITE_FILE,
        files::CREATE_FOLDER,
        files::RENAME,
        files::MOVE_TO_TRASH,
        files::FIND_FILES,
        transfer::START_TRANSFER,
        transfer::LIST_TRANSFERS,
        transfer::PAUSE_TRANSFER,
        transfer::RESUME_TRANSFER,
        transfer::CANCEL_TRANSFER,
        trash::LIST_TRASH,
        trash::RESTORE_FROM_TRASH,
        trash::PLAN_EMPTY_TRASH,
        trash::EMPTY_TRASH,
        machine::LIST_MACHINES,
        machine::GET_MACHINE,
        machine::ADD_SSH_MACHINE,
        machine::RECONNECT_MACHINE,
        machine::PLAN_REMOVE_MACHINE,
        machine::REMOVE_MACHINE,
        machine::PLAN_FORGET_MACHINE,
        machine::FORGET_MACHINE,
        machine::PLAN_CHANGE_ROOT,
        machine::CHANGE_ROOT,
        machine::SET_VM_TIER,
        machine::GET_STORAGE,
        machine::CLEAR_BUILD_CACHE,
        machine::TRIM_LOGS,
        machine::LIST_HOST_VARIABLES,
        machine::SET_HOST_VARIABLE,
        machine::REMOVE_HOST_VARIABLE,
        machine::TRUST_NEW_HOST_KEY,
        provider::LIST_PROVIDERS,
        provider::GET_PROVIDERS_CONFIG,
        provider::SET_PROVIDERS_CONFIG,
        provider::PREVIEW_PROVIDER_IMPORT,
        provider::IMPORT_PROVIDER,
        provider::CREATE_MACHINE,
        provider::RUN_PROVIDER_ACTION,
        provider::INTERACTIVE_LOGIN,
        skills::PREVIEW_SKILLS_SYNC,
        skills::SYNC_SKILLS,
        bridge::CLOSE_PORT_FORWARD,
        bridge::LIST_PORT_FORWARDS,
        inbox::LIST_REQUESTS,
        inbox::GET_REQUEST,
        inbox::ANSWER_REQUEST,
        inbox::ANSWER_ALL,
        inbox::REQUEST_ACCESS,
        inbox::WAIT_FOR_ANSWER,
        operation::GET_OPERATION,
        operation::WATCH_OPERATION,
        operation::LIST_OPERATIONS,
        operation::CANCEL_OPERATION,
        operation::RETRY_OPERATION,
        diff::GET_DIFF,
        diff::MARK_REVIEWED,
        comment::LIST_COMMENTS,
        comment::CREATE_COMMENT,
        comment::DROP_COMMENT,
        comment::SEND_COMMENTS,
        review::RESOLVE_PULL_REQUEST,
        review::START_REVIEW,
        review::SET_PUSH_ALLOWED,
        review::PLAN_SUBMIT,
        review::SUBMIT_REVIEW,
        view::LIST_VIEWS,
        view::SET_VIEW_STATE,
        session::START_SESSION,
        session::LIST_SESSIONS,
        session::RENAME_SESSION,
        session::STOP_SESSION,
        session::RESUME_SESSION,
        session::PREVIEW_FORK,
        session::FORK_SESSION,
        session::GIVE_OWN_COPY,
        session::MOVE_SESSION,
        session::PLAN_ARCHIVE,
        session::ARCHIVE_SESSION,
        session::RESTORE_SESSION,
        session::PLAN_DELETE,
        session::DELETE_SESSION,
        session::REBUILD_SESSION,
        session::ADD_REPO_TO_SESSION,
        session::PLAN_DROP_REPO,
        session::DROP_REPO,
        session::PLAN_PUSH,
        session::PUSH_AND_OPEN_PULL_REQUESTS,
        session::READ_CONVERSATION,
        session::SEND_NOTE,
        session::ASK_SESSION,
        session::REPORT_TURN,
        session::HOOK,
        settings::GET_SETTINGS,
        settings::UPDATE_SETTINGS,
        settings::GET_CHANNEL_SETTINGS,
        settings::UPDATE_CHANNEL_SETTINGS,
        agents::LIST_AGENTS,
        agents::ADD_AGENT,
        agents::UPDATE_AGENT,
        marks::SET_MARKS,
        marks::LIST_PINNED,
        marks::REORDER_PINNED,
        history::SEARCH_SESSIONS,
        history::READ_TURNS,
        desk::KEEP_DESK_FOLDER,
        terminal::OPEN_MACHINE_SHELL,
        terminal::ATTACH,
        terminal::TYPE,
        terminal::LIST_QUEUED,
        terminal::SEND_QUEUED_NOW,
    ];
}
