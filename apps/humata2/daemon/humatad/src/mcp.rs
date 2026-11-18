//! `humatad2 mcp`: the MCP server an agent starts (MCP.md), as a client of
//! this machine's daemon. Every call carries the session token from
//! `HUMATA_SESSION_TOKEN`, and the daemon decides what it may do; this
//! process only turns tools into gRPC calls and answers back.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{
    Call, Daemon, ErrorKind, Operation, Reply, RequestId, Retry, Server, SessionToken, ToolError,
    WaitEvent,
};
use hyper_util::rt::TokioIo;
use serde_json::{Map, Value, json};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

use crate::error::DaemonError;
use crate::identity::TOKEN_HEADER;
use crate::{serve, status};

mod boards;
// REPOS (CH-21)
mod session_repo;
mod doc_versions;
mod history;
mod pull_requests;
// REPOS
mod repos;
// TP-1 (D772).
mod topology;

#[derive(clap::Args)]
pub(crate) struct McpArgs {
    /// The daemon's socket. Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
}

pub(crate) async fn run(args: McpArgs) -> Result<(), DaemonError> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => serve::default_socket()?,
    };
    Server::new(GrpcDaemon::new(socket), SessionToken::from_env())
        .serve_stdio()
        .await
        .map_err(|source| DaemonError::Mcp { source })
}

/// The daemon on its socket. Connects on the first call, and again after
/// the daemon restarts.
struct GrpcDaemon {
    socket: PathBuf,
    channel: Channel,
    token: Mutex<Option<SessionToken>>,
    /// The release this binary is, named on every call (UP-1).
    bundle: Option<tonic::metadata::AsciiMetadataValue>,
}

/// The bundle id of the release this binary was installed with: the sha256
/// of the `bundle.json` beside it, as the daemon names its own (D281 a).
/// None when there is none, as in a build run from the tree.
fn bundle_id() -> Option<tonic::metadata::AsciiMetadataValue> {
    use sha2::{Digest, Sha256};
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let bytes = std::fs::read(folder.join("bundle.json")).ok()?;
    let hex: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    MetadataValue::try_from(hex).ok()
}

impl GrpcDaemon {
    fn new(socket: PathBuf) -> Self {
        let path = socket.clone();
        // The address is a placeholder: the connector dials the socket.
        let channel =
            Endpoint::from_static("http://[::]:50051").connect_with_connector_lazy(
                tower::service_fn(move |_: Uri| {
                    let path = path.clone();
                    async move {
                        Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?))
                    }
                }),
            );
        Self {
            socket,
            channel,
            token: Mutex::new(None),
            bundle: bundle_id(),
        }
    }

    /// `message` with the session token in its metadata, marked sensitive
    /// so nothing logs it.
    fn request<T>(&self, message: T) -> Result<Request<T>, ToolError> {
        let mut request = Request::new(message);
        if let Some(token) = lock(&self.token).as_ref() {
            let mut value = MetadataValue::try_from(token.expose()).map_err(|_| not_a_token())?;
            value.set_sensitive(true);
            request.metadata_mut().insert(TOKEN_HEADER, value);
        }
        // UP-1, D290 m: a call with a session token is a CLI's, so it names
        // its release as the humata CLI does.
        if let Some(bundle) = &self.bundle {
            request.metadata_mut().insert(crate::upgrade::build::BUNDLE_HEADER, bundle.clone());
        }
        Ok(request)
    }

    fn failed(&self, status: &Status) -> ToolError {
        from_status(status, &self.socket)
    }

    fn daemon(&self) -> pb::daemon_service_client::DaemonServiceClient<Channel> {
        pb::daemon_service_client::DaemonServiceClient::new(self.channel.clone())
    }

    fn inbox(&self) -> pb::inbox_service_client::InboxServiceClient<Channel> {
        pb::inbox_service_client::InboxServiceClient::new(self.channel.clone())
    }

    fn channels(&self) -> pb::channel_service_client::ChannelServiceClient<Channel> {
        pb::channel_service_client::ChannelServiceClient::new(self.channel.clone())
    }

    fn sessions(&self) -> pb::session_service_client::SessionServiceClient<Channel> {
        pb::session_service_client::SessionServiceClient::new(self.channel.clone())
    }

    fn files(&self) -> pb::file_service_client::FileServiceClient<Channel> {
        pb::file_service_client::FileServiceClient::new(self.channel.clone())
    }

    fn access(&self) -> pb::access_service_client::AccessServiceClient<Channel> {
        pb::access_service_client::AccessServiceClient::new(self.channel.clone())
    }

    fn terminals(&self) -> pb::terminal_service_client::TerminalServiceClient<Channel> {
        pb::terminal_service_client::TerminalServiceClient::new(self.channel.clone())
    }

    fn review_chat(&self) -> pb::review_chat_service_client::ReviewChatServiceClient<Channel> {
        pb::review_chat_service_client::ReviewChatServiceClient::new(self.channel.clone())
    }

    fn comments(&self) -> pb::comment_service_client::CommentServiceClient<Channel> {
        pb::comment_service_client::CommentServiceClient::new(self.channel.clone())
    }

    fn views(&self) -> pb::view_service_client::ViewServiceClient<Channel> {
        pb::view_service_client::ViewServiceClient::new(self.channel.clone())
    }

    /// The caller's own session: the review chat acts on its own review.
    async fn own_session(&self) -> Result<pb::SessionId, ToolError> {
        let (agent, _) = self.agent().await?;
        agent.session.ok_or_else(not_a_token)
    }

    /// `review_context`: what the user is looking at in this review.
    async fn review_context(&self) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let request = self.request(pb::GetReviewContextRequest {
            session: Some(session),
        })?;
        let context = self
            .review_chat()
            .get_review_context(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(review::context_json(&context), None))
    }

    /// `review_focus`: show the user a place in the review.
    async fn review_focus(&self, call: Call) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let arguments = &call.arguments;
        let highlights = arguments
            .get("highlights")
            .and_then(Value::as_array)
            .map(|highlights| {
                highlights
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|highlight| pb::Highlight {
                        file: Some(review::repo_path(highlight)),
                        lines: review::lines(highlight.get("lines")),
                        side: review::side(highlight),
                        label: text(highlight, "label"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let request = self.request(pb::FocusReviewRequest {
            idempotency_key: Some(key(&call)),
            session: Some(session),
            file: Some(review::repo_path(arguments)),
            lines: review::lines(arguments.get("lines")),
            side: review::side(arguments),
            highlights,
        })?;
        let change = self
            .review_chat()
            .focus_review(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({ "undo_id": change.undo_id, "summary": change.summary });
        Ok(applied(result, None))
    }

    /// `review_note`: a note for the user on a file, a hunk or lines.
    async fn review_note(&self, call: Call) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let arguments = &call.arguments;
        // A comment's anchor has no hunk: a note on a hunk is on the line
        // inside it.
        let lines = review::lines(arguments.get("lines")).or_else(|| {
            review::number(arguments.get("hunk_at")).map(|line| pb::LineRange {
                first: line,
                last: line,
            })
        });
        let request = self.request(pb::CreateCommentRequest {
            idempotency_key: Some(key(&call)),
            session: Some(session),
            anchor: Some(pb::CommentAnchor {
                file: Some(review::repo_path(arguments)),
                lines,
                side: review::side(arguments),
                quoted: text(arguments, "quoted"),
                ..pb::CommentAnchor::default()
            }),
            text: text(arguments, "text"),
            destination: Some(pb::CommentDestination {
                destination: Some(pb::comment_destination::Destination::Note(pb::UserNote {})),
            }),
        })?;
        let note = self
            .comments()
            .create_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({ "note_id": note.id.map(|id| id.value), "undo_id": note.undo_id });
        Ok(applied(result, None))
    }

    /// `review_note_edit`: new text for a note.
    async fn review_note_edit(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::EditCommentRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::CommentId {
                value: text(&call.arguments, "note_id"),
            }),
            text: text(&call.arguments, "text"),
            expected_revision: None,
        })?;
        let note = self
            .comments()
            .edit_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({ "note_id": note.id.map(|id| id.value), "undo_id": note.undo_id });
        Ok(applied(result, None))
    }

    /// `review_note_drop`.
    async fn review_note_drop(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::DropCommentRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::CommentId {
                value: text(&call.arguments, "note_id"),
            }),
            expected_revision: None,
        })?;
        let dropped = self
            .comments()
            .drop_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "dropped": true, "undo_id": dropped.undo_id }),
            None,
        ))
    }

    /// `review_prepare`: in the review chat, the plan applies at once.
    async fn review_prepare(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::PrepareViewRequest {
            idempotency_key: Some(key(&call)),
            title: text(arguments, "title"),
            summary: String::new(),
            content: Some(pb::prepare_view_request::Content::ReviewPlan(review::plan(
                arguments,
            ))),
        })?;
        let prepared = self
            .views()
            .prepare_view(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = if prepared.undo_id.is_empty() {
            json!({ "view_id": prepared.id.map(|id| id.value), "state": "draft" })
        } else {
            json!({ "applied": true, "undo_id": prepared.undo_id })
        };
        Ok(applied(result, None))
    }

    /// `view_prepare`: a draft view, kept until `view_mark_ready`.
    async fn view_prepare(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let content = self.view_content(arguments.get("content")).await?;
        let request = self.request(pb::PrepareViewRequest {
            idempotency_key: Some(key(&call)),
            title: text(arguments, "title"),
            summary: text(arguments, "summary"),
            content: content.map(views::prepare_content),
        })?;
        let prepared = self
            .views()
            .prepare_view(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(views::view_json(&prepared), None))
    }

    /// `view_revise`: new parts for a draft view.
    async fn view_revise(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let content = self.view_content(arguments.get("content")).await?;
        let request = self.request(pb::ReviseViewRequest {
            idempotency_key: Some(key(&call)),
            id: Some(views::id(arguments)),
            title: text(arguments, "title"),
            summary: text(arguments, "summary"),
            content: content.map(views::revise_content),
            expected_revision: arguments
                .get("expected_revision")
                .and_then(Value::as_u64)
                .map(|value| pb::Revision { value }),
        })?;
        let revised = self
            .views()
            .revise_view(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(views::view_json(&revised), None))
    }

    /// `view_mark_ready`: offer the draft to the user.
    async fn view_mark_ready(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::MarkReadyRequest {
            idempotency_key: Some(key(&call)),
            id: Some(views::id(&call.arguments)),
        })?;
        let ready = self
            .views()
            .mark_ready(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(views::view_json(&ready), None))
    }

    /// `views_list`: the caller's views, drafts included.
    async fn views_list(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::ListViewsRequest {
            session: None,
            include_forks: call
                .arguments
                .get("include_forks")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            page: None,
        })?;
        let listed = self
            .views()
            .list_views(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let views: Vec<Value> = listed.views.iter().map(views::view_json).collect();
        Ok(applied(json!({ "views": views }), None))
    }

    /// `comments_list`: the comments the user sent to the caller's session.
    async fn comments_list(&self, call: Call) -> Result<Reply, ToolError> {
        let asked: Vec<&str> = call
            .arguments
            .get("status")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let statuses: Vec<i32> = if asked.is_empty() {
            vec![
                pb::CommentStatus::Sent as i32,
                pb::CommentStatus::Queued as i32,
                pb::CommentStatus::Addressed as i32,
            ]
        } else {
            asked
                .iter()
                .filter_map(|name| match *name {
                    "sent" => Some(pb::CommentStatus::Sent as i32),
                    "queued" => Some(pb::CommentStatus::Queued as i32),
                    "addressed" => Some(pb::CommentStatus::Addressed as i32),
                    _ => None,
                })
                .collect()
        };
        let session = self.own_session().await?;
        let mut listed: Vec<pb::Comment> = Vec::new();
        let mut page_token = String::new();
        for _ in 0..COMMENT_PAGES_MAX {
            let request = self.request(pb::ListCommentsRequest {
                session: Some(session.clone()),
                statuses: statuses.clone(),
                file: None,
                page: Some(pb::PageRequest { page_size: 0, page_token }),
                archived: false,
            })?;
            let answer = self
                .comments()
                .list_comments(request)
                .await
                .map_err(|status| self.failed(&status))?
                .into_inner();
            listed.extend(answer.comments);
            page_token = answer.page.map(|page| page.next_page_token).unwrap_or_default();
            if page_token.is_empty() {
                break;
            }
        }
        let only = call.arguments.get("path").and_then(Value::as_str);
        let comments: Vec<Value> = listed
            .iter()
            .filter(|comment| {
                only.is_none_or(|path| {
                    comment
                        .anchor
                        .as_ref()
                        .and_then(|anchor| anchor.file.as_ref())
                        .is_some_and(|file| file.path == path)
                })
            })
            .map(review::comment_json)
            .collect();
        Ok(applied(json!({ "comments": comments }), None))
    }

    /// `comment_reply`: a reply in the thread of a comment sent to the
    /// caller's session.
    async fn comment_reply(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::ReplyToCommentRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::CommentId {
                value: text(&call.arguments, "comment_id"),
            }),
            text: text(&call.arguments, "text"),
        })?;
        let comment = self
            .comments()
            .reply_to_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(review::comment_answer(&comment), None))
    }

    /// `comment_resolve`: resolve a comment sent to the caller's session,
    /// with a note on what was done.
    async fn comment_resolve(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::ResolveCommentRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::CommentId {
                value: text(&call.arguments, "comment_id"),
            }),
            expected_revision: None,
            reply: text(&call.arguments, "reply"),
        })?;
        let comment = self
            .comments()
            .resolve_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(review::comment_answer(&comment), None))
    }

    /// A view's `content`: a board names the caller's own session or
    /// channel unless it names another session.
    async fn view_content(
        &self,
        content: Option<&Value>,
    ) -> Result<Option<views::Content>, ToolError> {
        let Some(content) = content.and_then(Value::as_object) else {
            return Ok(None);
        };
        if let Some(board) = content.get("board") {
            let scope = match board {
                Value::String(scope) if scope == "session" => {
                    pb::board_view_offer::Scope::OneSession(self.own_session().await?)
                }
                Value::String(scope) if scope == "channel" => {
                    let (_, channel) = self.agent().await?;
                    pb::board_view_offer::Scope::Channel(channel.unwrap_or_default())
                }
                Value::Object(other) => pb::board_view_offer::Scope::OneSession(pb::SessionId {
                    value: text(other, "session"),
                }),
                _ => return Err(views::global_board()),
            };
            return Ok(Some(views::Content::Board(pb::BoardViewOffer {
                scope: Some(scope),
            })));
        }
        Ok(views::content(content))
    }

    /// The agent this connection is, per the daemon.
    async fn agent(&self) -> Result<(pb::AgentActor, Option<pb::ChannelId>), ToolError> {
        let request = self.request(pb::WhoAmIRequest {})?;
        let reply = self
            .daemon()
            .who_am_i(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        match reply.actor.and_then(|actor| actor.actor) {
            Some(pb::actor::Actor::Agent(agent)) => Ok((agent, reply.channel)),
            // The token went missing on the way, and the daemon took this
            // for the user: never act as them.
            _ => Err(not_a_token()),
        }
    }

    async fn whoami(&self) -> Result<Reply, ToolError> {
        let (agent, channel) = self.agent().await?;
        let result = json!({
            "session": agent.session.map(|id| id.value),
            "channel": channel.map(|id| id.value),
            "machine": agent.machine.map(|id| id.value),
            "agent": agent.agent.map(|id| id.value),
            "host_reachable": true,
            "policy_as_of": Value::Null,
        });
        Ok(applied(result, None))
    }

    async fn ask(&self, call: Call) -> Result<Reply, ToolError> {
        let choices = call
            .arguments
            .get("choices")
            .and_then(Value::as_array)
            .map(|choices| {
                choices
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let request = self.request(pb::AskUserRequest {
            idempotency_key: Some(key(&call)),
            question: text(&call.arguments, "question"),
            choices,
            wait_seconds: 0,
        })?;
        let response = self
            .inbox()
            .ask_user(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        outcome(response)
    }

    /// `inbox_request` (IN-26): asks ahead for `capability` on `target`.
    async fn request_access(&self, call: Call) -> Result<Reply, ToolError> {
        let name = text(&call.arguments, "capability");
        let capability =
            pb::Capability::from_str_name(&format!("CAPABILITY_{}", name.to_ascii_uppercase()))
                .ok_or_else(|| access::unknown_capability(&name))?;
        let target = match call.arguments.get("target").and_then(Value::as_object) {
            Some(target) => self.grant_target(target).await?,
            None => pb::GrantTarget::default(),
        };
        let request = self.request(pb::RequestAccessRequest {
            idempotency_key: Some(key(&call)),
            capability: capability as i32,
            target: Some(target),
            reason: text(&call.arguments, "reason"),
            wait_seconds: 0,
        })?;
        let response = self
            .inbox()
            .request_access(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        outcome(response)
    }

    /// A tool's `target` as a GrantTarget; a board named `session` or
    /// `channel` is the caller's own.
    async fn grant_target(
        &self,
        target: &Map<String, Value>,
    ) -> Result<pb::GrantTarget, ToolError> {
        use pb::grant_target::Target;
        let board = match target.get("board") {
            Some(Value::String(scope)) if scope == "session" => {
                Some(pb::board_ref::Board::Session(self.own_session().await?))
            }
            Some(Value::String(scope)) if scope == "channel" => {
                let (_, channel) = self.agent().await?;
                Some(pb::board_ref::Board::Channel(channel.unwrap_or_default()))
            }
            Some(Value::String(_)) => Some(pb::board_ref::Board::Global(pb::GlobalBoard {})),
            Some(Value::Object(other)) => Some(pb::board_ref::Board::Session(pb::SessionId {
                value: text(other, "session"),
            })),
            _ => None,
        };
        let target = if let Some(board) = board {
            Target::Board(pb::BoardRef { board: Some(board) })
        } else {
            access::target(target)
        };
        Ok(pb::GrantTarget {
            target: Some(target),
        })
    }

    /// `channels_list`.
    async fn channels_list(&self) -> Result<Reply, ToolError> {
        let request = self.request(pb::ListChannelsRequest { page: None })?;
        let listed = self
            .channels()
            .list_channels(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(channels_json(&listed), None))
    }

    /// `session_info`: the caller's own session.
    async fn session_info(&self) -> Result<Reply, ToolError> {
        let id = self.own_session().await?;
        let request = self.request(pb::GetSessionRequest { id: Some(id) })?;
        let session = self
            .sessions()
            .get_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(session_json(&session), None))
    }

    /// The session a tool names, or the caller's own.
    async fn named_session(
        &self,
        arguments: &Map<String, Value>,
    ) -> Result<pb::SessionId, ToolError> {
        let named = text(arguments, "session");
        if named.is_empty() {
            self.own_session().await
        } else {
            Ok(pb::SessionId { value: named })
        }
    }

    /// `session_stop`.
    async fn session_stop(&self, call: Call) -> Result<Reply, ToolError> {
        let id = self.named_session(&call.arguments).await?;
        let request = self.request(pb::StopSessionRequest {
            idempotency_key: Some(key(&call)),
            id: Some(id),
            for_move: false,
        })?;
        let session = self
            .sessions()
            .stop_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(session_json(&session), None))
    }

    /// `session_fork` (flow 26.5): forks the caller's own session; the
    /// daemon asks the user first when the rule says so.
    async fn session_fork(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let ForkChoices { kind, agent, before_turn } = fork_choices(arguments)?;
        let some_repos = arguments
            .get("repos")
            .and_then(Value::as_array)
            .is_some_and(|repos| !repos.is_empty());
        if some_repos || !text(arguments, "first_message").is_empty() {
            return Err(fork_unsupported(
                "forking only some repos or with a first message isn't built yet".to_owned(),
                "leave out repos and first_message to fork every repo, then type the message with session_type once the fork runs",
            ));
        }
        let machine = text(arguments, "machine");
        let request = self.request(pb::ForkSessionRequest {
            idempotency_key: Some(key(&call)),
            parent: Some(self.own_session().await?),
            kind: kind as i32,
            before_turn,
            agent: agent.map(|value| pb::AgentId { value }),
            title: text(arguments, "title"),
            machine: (!machine.is_empty()).then_some(pb::MachineId { value: machine }),
            reason: text(arguments, "reason"),
            ..Default::default()
        })?;
        let operation = self
            .sessions()
            .fork_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "operation": operation.id.map(|id| id.value) }),
            None,
        ))
    }

    /// `session_continue_in_channel` (CV-8): a new session in another channel
    /// from a copy of the caller's conversation; the daemon asks the user
    /// first when the rule says so.
    async fn session_continue_in_channel(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::ContinueInChannelRequest {
            idempotency_key: Some(key(&call)),
            session: Some(self.own_session().await?),
            channel: Some(pb::ChannelId {
                value: text(arguments, "channel"),
            }),
            before_turn: arguments.get("before_turn").and_then(Value::as_u64).and_then(|turn| u32::try_from(turn).ok()).unwrap_or(0),
        })?;
        let started = self
            .sessions()
            .continue_in_channel(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({
                "operation": started.operation.and_then(|operation| operation.id).map(|id| id.value),
                "session": started.session.map(|id| id.value),
            }),
            None,
        ))
    }

    /// `session_archive`: plans, then archives with nothing unpushed lost.
    async fn session_archive(&self, call: Call) -> Result<Reply, ToolError> {
        let id = self.named_session(&call.arguments).await?;
        let plan = self
            .sessions()
            .plan_archive(self.request(pb::PlanArchiveRequest {
                id: Some(id.clone()),
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let request = self.request(pb::ArchiveSessionRequest {
            idempotency_key: Some(key(&call)),
            id: Some(id),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
        })?;
        let operation = self
            .sessions()
            .archive_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "operation": operation.id.map(|id| id.value) }),
            None,
        ))
    }

    /// Deletes a session through the same plan and delete calls as the
    /// CLI, so the daemon's rules (another session's: Never, D306) apply.
    async fn session_delete(&self, call: Call) -> Result<Reply, ToolError> {
        let id = self.named_session(&call.arguments).await?;
        let plan = self
            .sessions()
            .plan_delete(self.request(pb::PlanDeleteRequest {
                id: Some(id.clone()),
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let request = self.request(pb::DeleteSessionRequest {
            idempotency_key: Some(key(&call)),
            id: Some(id),
            plan: plan.token,
        })?;
        let deleted = self
            .sessions()
            .delete_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "trash_item": deleted.trash_item.map(|item| item.value) }),
            None,
        ))
    }

    /// A tool's `location` as a FileLocation; the caller's own session,
    /// channel and machine are the defaults.
    async fn file_location(
        &self,
        arguments: &Map<String, Value>,
    ) -> Result<pb::FileLocation, ToolError> {
        use pb::file_location::Root;
        let location = arguments
            .get("location")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let machine = text(&location, "machine");
        let repo = review::repo_path(&location).repo.unwrap_or_default();
        let root = match text(&location, "root").as_str() {
            "clone" => Root::SessionClone(pb::SessionClone {
                session: Some(self.named_session(&location).await?),
                repo: Some(repo),
            }),
            "session_folder" => Root::SessionFolder(pb::SessionFolder {
                session: Some(self.named_session(&location).await?),
            }),
            "shared" => Root::ChannelShared(pb::ChannelShared {
                channel: Some(self.own_channel().await?),
            }),
            "main_clone" => Root::MainClone(pb::MainClone {
                channel: Some(self.own_channel().await?),
                repo: Some(repo),
            }),
            _ => Root::ChannelFolder(pb::ChannelFolder {
                channel: Some(self.own_channel().await?),
            }),
        };
        Ok(pb::FileLocation {
            machine: (!machine.is_empty()).then_some(pb::MachineId { value: machine }),
            root: Some(root),
            path: text(&location, "path"),
        })
    }

    async fn own_channel(&self) -> Result<pb::ChannelId, ToolError> {
        let (_, channel) = self.agent().await?;
        channel.ok_or_else(not_a_token)
    }

    /// `files_list`.
    async fn files_list(&self, call: Call) -> Result<Reply, ToolError> {
        let location = self.file_location(&call.arguments).await?;
        let request = self.request(pb::ListDirectoryRequest {
            location: Some(location),
            page: Some(pb::PageRequest {
                page_token: text(&call.arguments, "page_token"),
                ..pb::PageRequest::default()
            }),
        })?;
        let listed = self
            .files()
            .list_directory(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(directory_json(&listed), None))
    }

    /// `file_read`: the file as text, or base64 when it isn't UTF-8.
    async fn file_read(&self, call: Call) -> Result<Reply, ToolError> {
        let location = self.file_location(&call.arguments).await?;
        let number = |field: &str| {
            call.arguments
                .get(field)
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let request = self.request(pb::ReadFileRequest {
            location: Some(location),
            offset: number("offset"),
            max_bytes: number("max_bytes"),
            reason: text(&call.arguments, "reason"),
            accept_codecs: Vec::new(),
        })?;
        let mut chunks = self
            .files()
            .read_file(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks
            .message()
            .await
            .map_err(|status| self.failed(&status))?
        {
            bytes.extend_from_slice(&chunk.data);
        }
        let result = match String::from_utf8(bytes) {
            Ok(text) => json!({ "text": text }),
            Err(error) => json!({ "base64": files::base64_encode(error.as_bytes()) }),
        };
        Ok(applied(result, None))
    }

    /// `file_write`: one header, then the bytes in chunks.
    async fn file_write(&self, call: Call) -> Result<Reply, ToolError> {
        use sha2::{Digest, Sha256};
        let location = self.file_location(&call.arguments).await?;
        let bytes = match call.arguments.get("text").and_then(Value::as_str) {
            Some(text) => text.as_bytes().to_vec(),
            None => files::base64_decode(&text(&call.arguments, "base64"))
                .ok_or_else(files::bad_base64)?,
        };
        let header = pb::WriteFileMessage {
            message: Some(pb::write_file_message::Message::Header(
                pb::WriteFileHeader {
                    idempotency_key: Some(key(&call)),
                    location: Some(location),
                    size: bytes.len() as u64,
                    sha256: files::hex(&Sha256::digest(&bytes)),
                    expected_sha256: None,
                    replace: call
                        .arguments
                        .get("replace")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
            )),
        };
        let mut messages = vec![header];
        let mut offset = 0usize;
        loop {
            let end = (offset + files::CHUNK).min(bytes.len());
            messages.push(pb::WriteFileMessage {
                message: Some(pb::write_file_message::Message::Chunk(pb::FileChunk {
                    offset: offset as u64,
                    data: bytes[offset..end].to_vec(),
                    last: end == bytes.len(),
                    ..pb::FileChunk::default()
                })),
            });
            if end == bytes.len() {
                break;
            }
            offset = end;
        }
        let request = self.request(tokio_stream::iter(messages))?;
        let entry = self
            .files()
            .write_file(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(file_json(&entry), None))
    }

    /// `inbox_withdraw`.
    async fn inbox_withdraw(&self, call: Call) -> Result<Reply, ToolError> {
        let id = RequestId::new(text(&call.arguments, "request_id"));
        self.withdraw(&id).await?;
        Ok(applied(json!({ "withdrawn": true }), None))
    }

    /// `inbox_mine`: the caller's own requests.
    async fn inbox_mine(&self, call: Call) -> Result<Reply, ToolError> {
        let session = self.own_session().await?;
        let pending_only = call
            .arguments
            .get("pending_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let request = self.request(pb::ListRequestsRequest {
            scope: Some(pb::InboxScope {
                scope: Some(pb::inbox_scope::Scope::Session(pb::SessionInbox {
                    session: Some(session),
                    with_forks: false,
                })),
            }),
            include_answered: !pending_only,
            page: None,
        })?;
        let listed = self
            .inbox()
            .list_requests(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let requests: Vec<Value> = listed
            .requests
            .iter()
            .map(|request| {
                json!({
                    "request_id": request.id.as_ref().map(|id| id.value.clone()),
                    "state": lower(request.state().as_str_name(), "REQUEST_STATE_"),
                    "action": request.action_title,
                    "capability": capability_name(request.capability),
                })
            })
            .collect();
        Ok(applied(json!({ "requests": requests }), None))
    }

    /// `access_explain`: what the caller's rules decide.
    async fn access_explain(&self, call: Call) -> Result<Reply, ToolError> {
        let name = text(&call.arguments, "capability");
        let capability =
            pb::Capability::from_str_name(&format!("CAPABILITY_{}", name.to_ascii_uppercase()))
                .ok_or_else(|| access::unknown_capability(&name))?;
        let target = match call.arguments.get("target").and_then(Value::as_object) {
            Some(target) => Some(self.grant_target(target).await?),
            None => None,
        };
        let request = self.request(pb::ExplainRequest {
            session: None,
            capability: capability as i32,
            target,
        })?;
        let explained = self
            .access()
            .explain(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(explain_json(&explained), None))
    }

    async fn session_type(&self, call: Call) -> Result<Reply, ToolError> {
        let when = match call.arguments.get("when").and_then(Value::as_str) {
            Some("turn_end") => pb::type_request::When::TurnEnds,
            _ => pb::type_request::When::Now,
        };
        let request = self.request(pb::TypeRequest {
            idempotency_key: Some(key(&call)),
            session: Some(pb::SessionId {
                value: text(&call.arguments, "session"),
            }),
            tab: None,
            text: typed_text(&call.arguments),
            when: when as i32,
        })?;
        let typed = self
            .terminals()
            .r#type(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({
            "typed": typed.typed,
            "submitted": typed.submitted,
            "agent_busy": typed.agent_busy,
            "queued_id": typed.queued.map(|id| id.value),
            "turn_end_uncertain": typed.turn_end_uncertain,
        });
        Ok(applied(result, None))
    }
}

/// How long `session_ask` waits for the asked session's answer: TP-11's
/// longest wait.
const ASK_WAIT_SECONDS: u32 = 600;

impl GrpcDaemon {
    /// `session_note` (TP-5): typed once the other session's turn is over.
    async fn session_note(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::SendNoteRequest {
            idempotency_key: Some(key(&call)),
            to: Some(pb::SessionId {
                value: text(&call.arguments, "session"),
            }),
            text: text(&call.arguments, "text"),
            reason: String::new(),
        })?;
        let sent = self
            .sessions()
            .send_note(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(json!({ "typed": sent.typed }), None))
    }

    /// `session_ask` (TP-5): waits for the other session's `session_answer`.
    async fn session_ask(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::AskSessionRequest {
            idempotency_key: Some(key(&call)),
            to: Some(pb::SessionId {
                value: text(&call.arguments, "session"),
            }),
            question: text(&call.arguments, "question"),
            reason: String::new(),
            timeout_seconds: ASK_WAIT_SECONDS,
        })?;
        let asked = self
            .sessions()
            .ask_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let outcome = match asked.outcome() {
            pb::ask_session_response::Outcome::Answered => "answered",
            pb::ask_session_response::Outcome::Declined => "declined",
            _ => "expired",
        };
        Ok(applied(json!({ "outcome": outcome, "answer": asked.answer }), None))
    }

    /// `session_answer` (TP-5): answers a question typed into the caller's
    /// own terminal.
    async fn session_answer(&self, call: Call) -> Result<Reply, ToolError> {
        let request = self.request(pb::AnswerQuestionRequest {
            idempotency_key: Some(key(&call)),
            question: Some(pb::QuestionId {
                value: text(&call.arguments, "question_id"),
            }),
            answer: text(&call.arguments, "answer"),
            decline: false,
        })?;
        self.sessions()
            .answer_question(request)
            .await
            .map_err(|status| self.failed(&status))?;
        Ok(applied(json!({ "answered": true }), None))
    }
}

/// `session_type`'s text, with Enter pressed after it (a trailing newline
/// is the submit, D671) unless `submit` is false or it already ends in one.
fn typed_text(arguments: &Map<String, Value>) -> String {
    let mut typed = text(arguments, "text");
    let submit = arguments.get("submit").and_then(Value::as_bool).unwrap_or(true);
    if submit && !typed.ends_with(['\n', '\r']) {
        typed.push('\n');
    }
    typed
}

impl GrpcDaemon {
    /// `desk_session_move_to_channel`: asks to move the caller's own Desk
    /// session into a channel (DK-15, D305). The daemon plans it when it asks.
    async fn desk_move_to_channel(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::MoveDeskSessionToChannelRequest {
            idempotency_key: Some(key(&call)),
            session: None,
            channel: Some(pb::ChannelId {
                value: text(arguments, "channel"),
            }),
            plan: None,
            branch: text(arguments, "branch"),
            reason: text(arguments, "reason"),
            ..Default::default()
        })?;
        let operation = pb::desk_service_client::DeskServiceClient::new(self.channel.clone())
            .move_desk_session_to_channel(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({ "operation": operation.id.map(|id| id.value) }),
            None,
        ))
    }
}

impl Daemon for GrpcDaemon {
    /// The tools `call` dispatches below; every other one answers
    /// `not_built`, so `tools/list` leaves it out. Keep the two in step.
    fn serves(&self, operation: Operation) -> bool {
        matches!(
            operation,
            Operation::Whoami |
            Operation::InboxAsk |
            Operation::InboxRequest |
            Operation::SessionType |
            Operation::ReviewContext |
            Operation::ReviewFocus |
            Operation::ReviewNote |
            Operation::ReviewNoteEdit |
            Operation::ReviewNoteDrop |
            Operation::ReviewPrepare |
            Operation::ViewPrepare |
            Operation::ViewRevise |
            Operation::ViewMarkReady |
            Operation::ViewsList |
            Operation::CommentsList |
            Operation::CommentReply |
            Operation::CommentResolve |
            Operation::DocVersionsList |
            Operation::DocVersionRead |
            Operation::ChannelsList |
            Operation::SessionInfo |
            Operation::TopologyGet |
            Operation::SessionStop |
            Operation::SessionArchive |
            Operation::SessionDelete |
            Operation::SessionFork |
            Operation::SessionContinueInChannel |
            Operation::FileWrite |
            Operation::FilesList |
            Operation::FileRead |
            Operation::InboxWithdraw |
            Operation::InboxMine |
            Operation::AccessExplain |
            Operation::BoardItemsList |
            Operation::BoardItemGet |
            Operation::BoardItemHistory |
            Operation::BoardItemCreate |
            Operation::BoardItemUpdate |
            Operation::BoardItemMove |
            Operation::BoardItemDelete |
            Operation::SessionsSearch |
            Operation::RepoSearch | // REPOS
            Operation::RepoLibrary |
            Operation::SessionRead |
            Operation::ReviewDraft |
            Operation::ReviewDraftEdit |
            Operation::ReviewDraftDrop |
            Operation::ReviewSummaryDraft |
            Operation::PrReviewStart
        )
    }

    async fn bind(&self, token: &SessionToken) -> Result<(), ToolError> {
        *lock(&self.token) = Some(token.clone());
        self.agent().await.map(|_| ())
    }

    async fn call(&self, call: Call) -> Result<Reply, ToolError> {
        let reply = match call.operation {
            Operation::Whoami => self.whoami().await,
            Operation::InboxAsk => self.ask(call).await,
            Operation::InboxRequest => self.request_access(call).await,
            Operation::SessionType => self.session_type(call).await,
            Operation::SessionNote => self.session_note(call).await,
            Operation::SessionAsk => self.session_ask(call).await,
            Operation::SessionAnswer => self.session_answer(call).await,
            Operation::ReviewCommits => self.review_commits(call).await,
            Operation::ReviewContext => self.review_context().await,
            Operation::ReviewFocus => self.review_focus(call).await,
            Operation::ReviewNote => self.review_note(call).await,
            Operation::ReviewNoteEdit => self.review_note_edit(call).await,
            Operation::ReviewNoteDrop => self.review_note_drop(call).await,
            Operation::ReviewPrepare => self.review_prepare(call).await,
            Operation::ViewPrepare => self.view_prepare(call).await,
            Operation::ViewRevise => self.view_revise(call).await,
            Operation::ViewMarkReady => self.view_mark_ready(call).await,
            Operation::ViewsList => self.views_list(call).await,
            Operation::CommentsList => self.comments_list(call).await,
            Operation::CommentReply => self.comment_reply(call).await,
            Operation::CommentResolve => self.comment_resolve(call).await,
            Operation::DocVersionsList => self.doc_versions_list(call).await,
            Operation::DocVersionRead => self.doc_version_read(call).await,
            Operation::ChannelsList => self.channels_list().await,
            Operation::SessionInfo => self.session_info().await,
            Operation::TopologyGet => self.topology_get().await,
            Operation::SessionStop => self.session_stop(call).await,
            Operation::SessionArchive => self.session_archive(call).await,
            Operation::SessionDelete => self.session_delete(call).await,
            Operation::SessionFork => self.session_fork(call).await,
            Operation::SessionContinueInChannel => self.session_continue_in_channel(call).await,
            Operation::SessionAddRepo => self.session_add_repo(call).await, // REPOS (CH-21)
            Operation::FileWrite => self.file_write(call).await,
            Operation::FilesList => self.files_list(call).await,
            Operation::FileRead => self.file_read(call).await,
            Operation::InboxWithdraw => self.inbox_withdraw(call).await,
            Operation::InboxMine => self.inbox_mine(call).await,
            Operation::AccessExplain => self.access_explain(call).await,
            Operation::BoardItemsList => self.board_items_list(call).await,
            Operation::BoardItemGet => self.board_item_get(call).await,
            Operation::BoardItemHistory => self.board_item_history(call).await,
            Operation::BoardItemCreate => self.board_item_create(call).await,
            Operation::BoardItemUpdate => self.board_item_update(call).await,
            Operation::BoardItemMove => self.board_item_move(call).await,
            Operation::BoardItemDelete => self.board_item_delete(call).await,
            Operation::SessionsSearch => self.sessions_search(call).await,
            Operation::RepoSearch => self.repo_search(call).await, // REPOS
            Operation::RepoLibrary => self.repo_library().await,
            Operation::SessionRead => self.session_read(call).await,
            Operation::ReviewDraft => self.review_draft(call).await,
            Operation::ReviewDraftEdit => self.review_draft_edit(call).await,
            Operation::ReviewDraftDrop => self.review_draft_drop(call).await,
            Operation::ReviewSummaryDraft => self.review_summary_draft(call).await,
            Operation::PrReviewStart => self.pr_review_start(call).await,
            Operation::DeskSessionMoveToChannel => self.desk_move_to_channel(call).await,
            other => Err(not_built(other)),
        };
        pending_or(reply)
    }

    async fn wait(
        &self,
        request: &RequestId,
        wait: Duration,
    ) -> Result<mpsc::Receiver<WaitEvent>, ToolError> {
        let message = self.request(pb::WaitForAnswerRequest {
            id: Some(pb::RequestId {
                value: request.as_str().to_owned(),
            }),
            wait_seconds: u32::try_from(wait.as_secs()).unwrap_or(u32::MAX),
        })?;
        let mut answers = self
            .inbox()
            .wait_for_answer(message)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let socket = self.socket.clone();
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            loop {
                // Dropping the stream when the server stops listening ends
                // the daemon's side of the wait too.
                let next = tokio::select! {
                    () = sender.closed() => return,
                    next = answers.message() => next,
                };
                let event = match next {
                    Ok(Some(response)) if response.keepalive => WaitEvent::Keepalive,
                    Ok(Some(response)) => WaitEvent::Done(pending_or(outcome(response))),
                    // Ended without an answer: the server reports pending.
                    Ok(None) => return,
                    Err(status) => WaitEvent::Done(pending_or(Err(from_status(&status, &socket)))),
                };
                let done = matches!(event, WaitEvent::Done(_));
                if sender.send(event).await.is_err() || done {
                    return;
                }
            }
        });
        Ok(receiver)
    }

    async fn withdraw(&self, request: &RequestId) -> Result<(), ToolError> {
        let message = self.request(pb::WithdrawRequestRequest {
            idempotency_key: None,
            id: Some(pb::RequestId {
                value: request.as_str().to_owned(),
            }),
        })?;
        self.inbox()
            .withdraw_request(message)
            .await
            .map(|_| ())
            .map_err(|status| self.failed(&status))
    }
}

/// What the daemon's answer to an Inbox request means for the call.
fn outcome(response: pb::WaitResponse) -> Result<Reply, ToolError> {
    let id = RequestId::new(response.id.map(|id| id.value).unwrap_or_default());
    if let Some(error) = response.error {
        return Err(tool_error(Code::FailedPrecondition, error));
    }
    match pb::RequestState::try_from(response.state) {
        Ok(pb::RequestState::Pending) => Ok(Reply::Pending { request: id }),
        Ok(pb::RequestState::Answered | pb::RequestState::Approved) => {
            let result = match response.answer.and_then(|answer| answer.answer) {
                Some(pb::request_answer::Answer::Reply(reply)) => json!({ "reply": reply.text }),
                Some(_) => approved_result(&response.result_type, &response.result),
                None => json!({}),
            };
            Ok(applied(result, Some(id)))
        }
        _ => Err(ToolError {
            kind: ErrorKind::Internal,
            message: format!(
                "the daemon answered request {} with a state this MCP server doesn't know",
                id.as_str()
            ),
            fix: "call inbox_wait with the request id to see where it stands".to_owned(),
            retry: None,
            details: Map::new(),
        }),
    }
}

fn applied(result: Value, request: Option<RequestId>) -> Reply {
    Reply::Applied {
        result,
        request,
        stale_as_of: None,
    }
}

fn key(call: &Call) -> pb::IdempotencyKey {
    pb::IdempotencyKey {
        value: call.idempotency_key.as_str().to_owned(),
    }
}

/// A string argument; the server checked the schema already.
fn text(arguments: &Map<String, Value>, field: &str) -> String {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// What a `session_fork` call chooses beyond the machine: the copy mode, the
/// fork's agent (none: the parent's) and the turn to fork before (0: now).
#[derive(Debug)]
struct ForkChoices {
    kind: pb::ForkKind,
    agent: Option<String>,
    before_turn: u32,
}

fn fork_choices(arguments: &Map<String, Value>) -> Result<ForkChoices, ToolError> {
    let kind = match text(arguments, "kind").as_str() {
        "code_and_context" => pb::ForkKind::CodeAndContext,
        "code_only" => pb::ForkKind::CodeOnly,
        "context_only" => pb::ForkKind::ContextOnly,
        other => return Err(fork_unsupported(format!("kind {other:?} isn't a fork kind"), "send kind code_and_context, code_only or context_only")),
    };
    let agent = Some(text(arguments, "agent")).filter(|agent| !agent.is_empty());
    let before_turn = match arguments.get("before_turn") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .and_then(|turn| u32::try_from(turn).ok())
            .filter(|turn| *turn >= 2)
            .ok_or_else(|| {
                fork_unsupported(
                    format!("before_turn {value} isn't a turn a fork can start before"),
                    "send a whole number from 2 (1 is the first prompt, so nothing comes before it), or leave before_turn out to fork from now",
                )
            })?,
    };
    Ok(ForkChoices { kind, agent, before_turn })
}

/// A `session_fork` argument this daemon can't act on.
fn fork_unsupported(message: String, fix: &str) -> ToolError {
    ToolError {
        kind: ErrorKind::InvalidField,
        message,
        fix: fix.to_owned(),
        retry: None,
        details: Map::new(),
    }
}

fn not_built(operation: Operation) -> ToolError {
    ToolError {
        kind: ErrorKind::PeerVersionUnsupported,
        message: format!(
            "{} isn't built into this humatad2 yet",
            operation.tool_name()
        ),
        fix: "ask the user with inbox_ask to do it in the app; updating Humata on this machine brings the tool".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

/// The token isn't one the daemon takes as an agent's.
fn not_a_token() -> ToolError {
    ToolError {
        kind: ErrorKind::IdentityMismatch,
        message: "HUMATA_SESSION_TOKEN isn't a session token humatad2 gave out".to_owned(),
        fix: "start the agent from a Humata session tab, which sets it; don't copy a token from another tab, since the daemon refuses it".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

fn from_status(status: &Status, socket: &Path) -> ToolError {
    if let Some(error) = status::error_of(status) {
        if let Some(pb::error::Kind::ApprovalPending(pending)) = &error.kind {
            let mut marked = tool_error(status.code(), error.clone());
            let request = pending
                .request
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default();
            marked
                .details
                .insert(PENDING_MARK.to_owned(), json!(request));
            return marked;
        }
        return tool_error(status.code(), error);
    }
    if status.code() == Code::Unavailable {
        let mut details = Map::new();
        details.insert("socket".to_owned(), json!(socket.display().to_string()));
        return ToolError {
            kind: ErrorKind::DaemonNotRunning,
            message: format!("humatad2 isn't answering on {}", socket.display()),
            fix: "start it with `humatad2 serve` on this machine, or open the Humata app, which starts it; the call can then be made again".to_owned(),
            retry: Some(Retry {
                retryable: true,
                after: None,
                same_key: true,
            }),
            details,
        };
    }
    ToolError {
        kind: ErrorKind::Internal,
        message: format!("humatad2 failed the call: {}", status.message()),
        fix: "try again; if it keeps failing, the daemon's log on this machine says why".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

fn tool_error(code: Code, error: pb::Error) -> ToolError {
    let kind = match &error.kind {
        Some(kind) => kind_of(kind),
        // GRPC.md §10: a method this daemon doesn't have.
        None if code == Code::Unimplemented => ErrorKind::PeerVersionUnsupported,
        None => ErrorKind::Internal,
    };
    let retry = error.retry.map(|advice| Retry {
        retryable: true,
        after: advice.after.map(|after| {
            SystemTime::now()
                + Duration::new(
                    u64::try_from(after.seconds).unwrap_or(0),
                    u32::try_from(after.nanos).unwrap_or(0),
                )
        }),
        same_key: advice.same_key,
    });
    let details = error.kind.as_ref().map(details_of).unwrap_or_default();
    ToolError {
        kind,
        message: error.message,
        fix: error.fix,
        retry,
        details,
    }
}

macro_rules! kinds {
    ($($kind:ident),* $(,)?) => {
        /// `humata.v1.Error`'s kind as agents see it.
        fn kind_of(kind: &pb::error::Kind) -> ErrorKind {
            match kind {
                $(pb::error::Kind::$kind(_) => ErrorKind::$kind,)*
                // Turned into a pending reply before an agent sees it.
                pb::error::Kind::ApprovalPending(_) => ErrorKind::Internal,
            }
        }
    };
}

kinds! {
    NotFound, MachineOffline, HostUnreachable, Denied, UserOnly, PresenceRequired,
    IdentityMismatch, PeerUnproven, RevisionConflict, PlanStale, NameTaken, InvalidField, UnpushedWork,
    SessionBusy, ParentReadOnly, MainCloneReadOnly, OutsideHumataFolders, DiskFull, RateLimited,
    GithubCli, GitFailed, PushRefused, SshFailed, HostKeyChanged, HostKeyUnknown, ChecksumMismatch, ResumeFailed,
    AgentLacksFeature, ProviderStepFailed, ProviderIdentityInvalid, LoginNeedsUser,
    PeerVersionUnsupported, OffsetTrimmed, NotCancellable, AlreadyAnswered, RequestWithdrawn,
    UserAway, NotInChannel, PrClosed, ReviewPartlyPosted, SkillNameClash, LingerUnavailable,
    DaemonAlreadyRunning, DaemonNotRunning, JournalEntryRejected, RepoNotVisible, StepFailed,
    PassphraseLocked, DependencyLoop, Internal,
    MigrationNeeded,
    HostNeedsMigrate,
    NoDefaultBranch,
}

/// The fields of the kinds agents most often branch on, with enum values
/// lowercase and without their prefix (D25). Other kinds carry none yet.
fn details_of(kind: &pb::error::Kind) -> Map<String, Value> {
    let fields = match kind {
        pb::error::Kind::Denied(denied) => json!({
            "source": denied.source().as_str_name().strip_prefix("SOURCE_").map(str::to_ascii_lowercase),
            "capability": capability_name(denied.capability),
            "request": denied.request.as_ref().map(|id| id.value.clone()),
            "user_note": denied.user_note,
        }),
        pb::error::Kind::UserOnly(user_only) => json!({
            "capability": capability_name(user_only.capability),
        }),
        pb::error::Kind::NotFound(missing) => json!({
            "kind": missing.kind().as_str_name().strip_prefix("ENTITY_KIND_").map(str::to_ascii_lowercase),
            "id": missing.id,
            "in_trash": missing.in_trash,
        }),
        pb::error::Kind::InvalidField(field) => json!({
            "field": field.field,
            "problem": field.problem().as_str_name().strip_prefix("PROBLEM_").map(str::to_ascii_lowercase),
        }),
        pb::error::Kind::DependencyLoop(looped) => json!({
            "cycle": looped.cycle.iter().map(|id| id.value.clone()).collect::<Vec<_>>(),
        }),
        pb::error::Kind::RequestWithdrawn(withdrawn) => json!({
            "request": withdrawn.request.as_ref().map(|id| id.value.clone()),
        }),
        pb::error::Kind::UserAway(away) => json!({
            "channel": away.channel.as_ref().map(|id| id.value.clone()),
        }),
        pb::error::Kind::Internal(internal) => json!({
            "incident_id": internal.incident_id,
            "log_path": internal.log_path,
        }),
        _ => return Map::new(),
    };
    match fields {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

fn capability_name(value: i32) -> Option<String> {
    pb::Capability::try_from(value)
        .ok()
        .and_then(|capability| {
            capability
                .as_str_name()
                .strip_prefix("CAPABILITY_")
                .map(str::to_ascii_lowercase)
        })
        .filter(|name| name != "unspecified")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn channels_json(listed: &pb::ListChannelsResponse) -> Value {
    let channels: Vec<Value> = listed
        .channels
        .iter()
        .map(|channel| {
            json!({
                "channel_id": channel.id.as_ref().map(|id| id.value.clone()),
                "name": channel.name,
                "folder": channel.folder,
                "repos": channel.repos.iter().map(|repo| format!("{}/{}", repo.owner, repo.name)).collect::<Vec<_>>(),
                "machines": channel.machines.iter().map(|machine| machine.value.clone()).collect::<Vec<_>>(),
                "live_sessions": channel.live_sessions,
                "pending_requests": channel.pending_requests,
            })
        })
        .collect();
    json!({ "channels": channels })
}

fn directory_json(listed: &pb::ListDirectoryResponse) -> Value {
    let entries: Vec<Value> = listed.entries.iter().map(file_json).collect();
    let next = listed
        .page
        .as_ref()
        .map(|page| page.next_page_token.clone())
        .filter(|token| !token.is_empty());
    json!({ "entries": entries, "next_page_token": next })
}

fn explain_json(explained: &pb::ExplainResponse) -> Value {
    json!({
        "decision": lower(explained.decision().as_str_name(), "DECISION_"),
        "source": lower(explained.source().as_str_name(), "DECISION_SOURCE_"),
        "grant_id": explained.grant.as_ref().map(|id| id.value.clone()),
        "host_away": explained.host_away,
        "away_mode": lower(explained.away_mode().as_str_name(), "AWAY_MODE_"),
    })
}

/// What an approved request's kept result shows the agent, by the
/// response type the daemon encoded it as. A type this bridge doesn't
/// know still reports the approval, with a note.
fn approved_result(result_type: &str, bytes: &[u8]) -> Value {
    use prost::Message;
    let shown = match result_type {
        crate::inbox::GRANT_TYPE => pb::Grant::decode(bytes).ok().map(
            |grant| json!({ "decision": "approved", "grant_id": grant.id.map(|id| id.value) }),
        ),
        "humata.v1.ListChannelsResponse" => pb::ListChannelsResponse::decode(bytes)
            .ok()
            .map(|r| channels_json(&r)),
        "humata.v1.ListDirectoryResponse" => pb::ListDirectoryResponse::decode(bytes)
            .ok()
            .map(|r| directory_json(&r)),
        "humata.v1.FileEntry" => pb::FileEntry::decode(bytes)
            .ok()
            .map(|entry| file_json(&entry)),
        // D768: an approved file_read, held whole (uncompressed) up to a cap.
        "humata.v1.FileChunk" => pb::FileChunk::decode(bytes).ok().map(|chunk| {
            let next = chunk.offset + chunk.data.len() as u64;
            let mut shown = match String::from_utf8(chunk.data) {
                Ok(text) => json!({ "text": text }),
                Err(error) => json!({ "base64": files::base64_encode(error.as_bytes()) }),
            };
            if !chunk.last {
                shown["more_from_offset"] = json!(next);
            }
            shown
        }),
        "humata.v1.Session" => pb::Session::decode(bytes)
            .ok()
            .map(|session| session_json(&session)),
        "humata.v1.Operation" => pb::Operation::decode(bytes)
            .ok()
            .map(|operation| json!({ "operation": operation.id.map(|id| id.value) })),
        "humata.v1.ExplainResponse" => pb::ExplainResponse::decode(bytes)
            .ok()
            .map(|r| explain_json(&r)),
        // D768: approving an asked read runs nothing; the read itself must
        // be repeated, so the agent isn't left with a bare approval.
        status::READ_APPROVED => Some(json!({
            "decision": "approved",
            "repeat_call": true,
            "note": "the user approved this read; make the same read again to get its result",
        })),
        _ => None,
    };
    shown.unwrap_or_else(|| {
        let mut result = json!({ "decision": "approved", "grant_id": Value::Null });
        if !result_type.is_empty() {
            result["note"] = json!(format!(
                "the user approved it and it ran, but this MCP server can't show a result of type {}; check the outcome where it applies",
                result_type
            ));
        }
        result
    })
}

fn lower(name: &str, prefix: &str) -> String {
    name.strip_prefix(prefix)
        .unwrap_or(name)
        .to_ascii_lowercase()
}

fn session_json(session: &pb::Session) -> Value {
    json!({
        "session": session.id.as_ref().map(|id| id.value.clone()),
        "channel": session.channel.as_ref().map(|id| id.value.clone()),
        "title": session.title,
        "status": lower(session.status().as_str_name(), "SESSION_STATUS_"),
        "machine": session.machine.as_ref().map(|id| id.value.clone()),
        "repos": session.repos.iter().map(|repo| json!({
            "repo": repo.repo.as_ref().map(|repo| format!("{}/{}", repo.owner, repo.name)),
            "branch": repo.branch,
        })).collect::<Vec<_>>(),
    })
}

fn file_json(entry: &pb::FileEntry) -> Value {
    json!({
        "path": entry.location.as_ref().map(|location| location.path.clone()),
        "kind": lower(entry.kind().as_str_name(), "KIND_"),
        "size": entry.size,
        "writable": entry.writable,
    })
}

/// A call the Inbox holds for the user comes back as a pending reply
/// (TP-11): `from_status` marks it, and here the mark becomes `Pending`,
/// so the MCP server waits on the request.
fn pending_or(reply: Result<Reply, ToolError>) -> Result<Reply, ToolError> {
    match reply {
        Err(error) => match error.details.get(PENDING_MARK).and_then(Value::as_str) {
            Some(request) => Ok(Reply::Pending {
                request: RequestId::new(request.to_owned()),
            }),
            None => Err(error),
        },
        reply => reply,
    }
}

/// `comments_list` stops after this many pages, so a token that never
/// ends can't keep it asking.
const COMMENT_PAGES_MAX: usize = 100;

/// The detail `from_status` sets on an approval-pending error. Never
/// reaches an agent: `pending_or` turns it into a reply.
const PENDING_MARK: &str = "approval_pending_request";

/// File bytes in and out of JSON.
mod files {
    use humata2_mcp::{ErrorKind, ToolError};
    use serde_json::Map;

    /// Bytes per chunk of a write, under the 1 MiB a chunk may carry.
    pub(super) const CHUNK: usize = 512 * 1024;

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub(super) fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub(super) fn base64_encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for group in bytes.chunks(3) {
            let value = group.iter().enumerate().fold(0u32, |acc, (i, byte)| {
                acc | (u32::from(*byte) << (16 - 8 * i))
            });
            for i in 0..4 {
                if i <= group.len() {
                    out.push(char::from(
                        ALPHABET[((value >> (18 - 6 * i)) & 63) as usize],
                    ));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub(super) fn base64_decode(text: &str) -> Option<Vec<u8>> {
        let digits = text.trim_end_matches('=').as_bytes();
        if digits.len() % 4 == 1 {
            return None;
        }
        let mut out = Vec::with_capacity(digits.len() * 3 / 4);
        for group in digits.chunks(4) {
            let mut value = 0u32;
            for (i, digit) in group.iter().enumerate() {
                let sextet = ALPHABET.iter().position(|letter| letter == digit)? as u32;
                value |= sextet << (18 - 6 * i);
            }
            for i in 0..group.len() - 1 {
                out.push((value >> (16 - 8 * i)) as u8);
            }
        }
        Some(out)
    }

    pub(super) fn bad_base64() -> ToolError {
        ToolError {
            kind: ErrorKind::InvalidField,
            message: "base64 isn't valid base64".to_owned(),
            fix: "send standard base64 with + and /, or send the file as text".to_owned(),
            retry: None,
            details: Map::new(),
        }
    }
}

/// The view tools' arguments and answers, as JSON.
mod views {
    use humata2_grpc::humata::v1 as pb;
    use humata2_mcp::{ErrorKind, ToolError};
    use serde_json::{Map, Value, json};

    /// What a view shows; the board's scope is resolved by the caller.
    pub(super) enum Content {
        Review(pb::ReviewWalkthrough),
        Board(pb::BoardViewOffer),
        PullRequest(pb::PullRequestDraft),
    }

    pub(super) fn prepare_content(content: Content) -> pb::prepare_view_request::Content {
        use pb::prepare_view_request::Content as Wire;
        match content {
            Content::Review(review) => Wire::Review(review),
            Content::Board(board) => Wire::Board(board),
            Content::PullRequest(draft) => Wire::PullRequest(draft),
        }
    }

    pub(super) fn revise_content(content: Content) -> pb::revise_view_request::Content {
        use pb::revise_view_request::Content as Wire;
        match content {
            Content::Review(review) => Wire::Review(review),
            Content::Board(board) => Wire::Board(board),
            Content::PullRequest(draft) => Wire::PullRequest(draft),
        }
    }

    pub(super) fn id(arguments: &Map<String, Value>) -> pb::ViewId {
        pb::ViewId {
            value: super::text(arguments, "view_id"),
        }
    }

    /// The objects in `value`'s list `field`.
    fn objects<'a>(value: &'a Value, field: &str) -> impl Iterator<Item = &'a Map<String, Value>> {
        value
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
    }

    fn repo(arguments: &Map<String, Value>) -> pb::RepoRef {
        super::review::repo_path(arguments).repo.unwrap_or_default()
    }

    /// A walkthrough or pull request `content`; the server checked the
    /// schema, so one of them is there.
    pub(super) fn content(content: &Map<String, Value>) -> Option<Content> {
        if let Some(review) = content.get("review") {
            let steps = objects(review, "steps")
                .map(|step| pb::review_walkthrough::Step {
                    file: Some(super::review::repo_path(step)),
                    lines: super::review::lines(step.get("lines")),
                    note: super::text(step, "note"),
                })
                .collect();
            return Some(Content::Review(pb::ReviewWalkthrough { steps }));
        }
        let draft = content.get("pull_request")?;
        let repos = objects(draft, "repos")
            .map(|one| pb::pull_request_draft::PerRepo {
                repo: Some(repo(one)),
                title: super::text(one, "title"),
                body: super::text(one, "body"),
                base_branch: super::text(one, "base_branch"),
                draft: one.get("draft").and_then(Value::as_bool).unwrap_or(false),
            })
            .collect();
        Some(Content::PullRequest(pb::PullRequestDraft { repos }))
    }

    /// The global board isn't a channel's (VW-1).
    pub(super) fn global_board() -> ToolError {
        let mut details = Map::new();
        details.insert("field".to_owned(), json!("content.board"));
        details.insert("problem".to_owned(), json!("not_allowed_here"));
        ToolError {
            kind: ErrorKind::InvalidField,
            message: "a view can't offer the global board; a view stays in your session's channel"
                .to_owned(),
            fix:
                "offer \"session\", \"channel\" or {\"session\": ...} for a session of your channel"
                    .to_owned(),
            retry: None,
            details,
        }
    }

    fn state(state: i32) -> &'static str {
        match pb::ViewState::try_from(state).unwrap_or_default() {
            pb::ViewState::Unspecified => "unspecified",
            pb::ViewState::Draft => "draft",
            pb::ViewState::Waiting => "waiting",
            pb::ViewState::Later => "later",
            pb::ViewState::NotNow => "not_now",
            pb::ViewState::Opened => "opened",
            pb::ViewState::Reverted => "reverted",
            pb::ViewState::OpenedAndClosed => "opened_and_closed",
            pb::ViewState::Kept => "kept",
        }
    }

    pub(super) fn view_json(view: &pb::PreparedView) -> Value {
        json!({
            "view_id": view.id.as_ref().map(|id| id.value.clone()),
            "title": view.title,
            "state": state(view.state),
            "revision": view.revision.as_ref().map(|revision| revision.value),
        })
    }
}

/// `inbox_request`'s arguments, as the access protos.
mod access {
    use humata2_grpc::humata::v1 as pb;
    use humata2_mcp::{ErrorKind, ToolError};
    use serde_json::{Map, Value, json};

    use super::text;

    /// A capability name this humatad2 doesn't know; the server checked
    /// the schema, so the MCP server and this daemon differ.
    pub(super) fn unknown_capability(name: &str) -> ToolError {
        let mut details = Map::new();
        details.insert("field".to_owned(), json!("capability"));
        details.insert("problem".to_owned(), json!("unknown_value"));
        ToolError {
            kind: ErrorKind::InvalidField,
            message: format!("capability {name:?} isn't one this humatad2 knows"),
            fix: "use a capability from the tool's list, or update Humata on this machine"
                .to_owned(),
            retry: None,
            details,
        }
    }

    fn machine(arguments: &Map<String, Value>) -> Option<pb::MachineId> {
        let value = text(arguments, "machine");
        (!value.is_empty()).then_some(pb::MachineId { value })
    }

    fn session(arguments: &Map<String, Value>) -> Option<pb::SessionId> {
        let value = text(arguments, "session");
        (!value.is_empty()).then_some(pb::SessionId { value })
    }

    fn repo(arguments: &Map<String, Value>) -> pb::RepoRef {
        super::review::repo_path(arguments).repo.unwrap_or_default()
    }

    /// Every target but a board; the server checked that exactly one is
    /// there.
    pub(super) fn target(target: &Map<String, Value>) -> pb::grant_target::Target {
        use pb::grant_target::Target;
        let inner = |field: &str| target.get(field).and_then(Value::as_object);
        if let Some(path) = inner("path") {
            return Target::Path(pb::PathScope {
                machine: machine(path),
                session: session(path),
                prefix: Some(super::review::repo_path(path)),
            });
        }
        if let Some(network) = inner("network") {
            return Target::Network(pb::NetworkTarget {
                host: text(network, "host"),
                port: network
                    .get("port")
                    .and_then(Value::as_u64)
                    .and_then(|port| u32::try_from(port).ok())
                    .unwrap_or_default(),
                machine: machine(network),
            });
        }
        if let Some(agent) = inner("agent_on_machine") {
            return Target::AgentOnMachine(pb::AgentOnMachine {
                machine: machine(agent),
                agent: Some(pb::AgentId {
                    value: text(agent, "agent"),
                }),
            });
        }
        if let Some(branch) = inner("repo_branch") {
            return Target::RepoBranch(pb::RepoBranch {
                repo: Some(repo(branch)),
                branch: text(branch, "branch"),
            });
        }
        if let Some(action) = inner("machine_action") {
            let kind = format!(
                "MACHINE_ACTION_KIND_{}",
                text(action, "action").to_ascii_uppercase()
            );
            return Target::MachineAction(pb::MachineAction {
                machine: machine(action),
                new_machine: text(action, "new_machine"),
                action: pb::MachineActionKind::from_str_name(&kind).unwrap_or_default() as i32,
            });
        }
        Target::Session(pb::SessionId {
            value: text(target, "session"),
        })
    }
}

/// The review tools' arguments and answers, as JSON.
mod review {
    use humata2_grpc::humata::v1 as pb;
    use serde_json::{Map, Value, json};

    /// `repo` (`owner/name`) and `path` of a tool's arguments.
    pub(super) fn repo_path(arguments: &Map<String, Value>) -> pb::RepoPath {
        let repo = super::text(arguments, "repo");
        let (owner, name) = repo.split_once('/').unwrap_or(("", repo.as_str()));
        pb::RepoPath {
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            path: super::text(arguments, "path"),
        }
    }

    pub(super) fn lines(lines: Option<&Value>) -> Option<pb::LineRange> {
        let lines = lines?;
        Some(pb::LineRange {
            first: number(lines.get("first"))?,
            last: number(lines.get("last"))?,
        })
    }

    pub(super) fn number(value: Option<&Value>) -> Option<u32> {
        value
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    }

    fn strings(value: Option<&Value>) -> Vec<String> {
        value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn objects(value: Option<&Value>) -> impl Iterator<Item = &Map<String, Value>> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
    }

    /// `review_prepare`'s arguments as a plan. The server checked them
    /// against the schema; the daemon checks them against the diff.
    pub(super) fn plan(arguments: &Map<String, Value>) -> pb::ReviewPlan {
        let spec = arguments.get("spec");
        pb::ReviewPlan {
            spec: Some(pb::DiffSpec {
                repos: objects(spec.and_then(|spec| spec.get("repos")))
                    .map(repo_spec)
                    .collect(),
            }),
            order: objects(arguments.get("order")).map(repo_path).collect(),
            groups: objects(arguments.get("groups"))
                .map(|group| pb::ReviewGroup {
                    title: super::text(group, "title"),
                    note: super::text(group, "note"),
                    first: group.get("first").and_then(Value::as_object).map(repo_path),
                })
                .collect(),
            notes: objects(arguments.get("notes"))
                .map(|note| {
                    let (target, lines) =
                        match (lines(note.get("lines")), number(note.get("hunk_at"))) {
                            (Some(lines), _) => (pb::review_note::Target::Lines, Some(lines)),
                            (None, Some(line)) => (
                                pb::review_note::Target::Hunk,
                                Some(pb::LineRange {
                                    first: line,
                                    last: line,
                                }),
                            ),
                            (None, None) => (pb::review_note::Target::File, None),
                        };
                    pb::ReviewNote {
                        file: Some(repo_path(note)),
                        target: target as i32,
                        lines,
                        side: side(note),
                        text: super::text(note, "text"),
                        ..pb::ReviewNote::default()
                    }
                })
                .collect(),
        }
    }

    fn repo_spec(repo: &Map<String, Value>) -> pb::RepoDiffSpec {
        use pb::diff_spec_base::Base;
        use pb::diff_spec_compare::{Untracked, Upto};
        let base = repo.get("base").and_then(Value::as_object);
        let given = |field: &str| base.and_then(|base| base.get(field));
        let base = if let Some(branch) = given("merge_base_with").and_then(Value::as_str) {
            Some(Base::MergeBaseWith(branch.to_owned()))
        } else if let Some(rev) = given("rev").and_then(Value::as_str) {
            Some(Base::Rev(rev.to_owned()))
        } else if let Some(n) = number(given("reviewed_snapshot")) {
            Some(Base::ReviewedSnapshot(n))
        } else if given("fork_point").is_some() {
            Some(Base::ForkPoint(pb::ForkPointBase {}))
        } else if given("index").is_some() {
            Some(Base::Index(pb::IndexBase {}))
        } else {
            None
        };
        let upto = match repo.get("upto").and_then(Value::as_str) {
            Some("commits") => Upto::Commits,
            Some("staged") => Upto::Staged,
            _ => Upto::Unstaged,
        };
        let untracked = match repo.get("untracked").and_then(Value::as_str) {
            Some("none") => Untracked::None,
            Some("paths") => Untracked::Paths,
            _ => Untracked::All,
        };
        let whitespace = match repo.get("whitespace").and_then(Value::as_str) {
            Some("ignore_changes") => pb::DiffWhitespace::IgnoreChanges,
            Some("ignore_all") => pb::DiffWhitespace::IgnoreAll,
            _ => pb::DiffWhitespace::Unspecified,
        };
        pb::RepoDiffSpec {
            repo: repo_path(repo).repo,
            base: Some(pb::DiffSpecBase { base }),
            compare: Some(pb::DiffSpecCompare {
                upto: upto as i32,
                untracked: untracked as i32,
                untracked_paths: strings(repo.get("untracked_paths")),
            }),
            include_paths: strings(repo.get("include_paths")),
            exclude_paths: strings(repo.get("exclude_paths")),
            whitespace: whitespace as i32,
            no_renames: repo.get("renames").and_then(Value::as_bool) == Some(false),
        }
    }

    pub(super) fn side(arguments: &Map<String, Value>) -> i32 {
        match arguments.get("side").and_then(Value::as_str) {
            Some("old") => pb::DiffSide::Old as i32,
            _ => pb::DiffSide::New as i32,
        }
    }

    fn path_json(path: Option<&pb::RepoPath>) -> Value {
        path.map_or(Value::Null, |path| {
            let repo = path.repo.as_ref().map_or_else(String::new, |repo| {
                if repo.owner.is_empty() {
                    repo.name.clone()
                } else {
                    format!("{}/{}", repo.owner, repo.name)
                }
            });
            json!({ "repo": repo, "path": path.path })
        })
    }

    fn lines_json(lines: Option<&pb::LineRange>) -> Value {
        lines.map_or(
            Value::Null,
            |lines| json!({ "first": lines.first, "last": lines.last }),
        )
    }

    fn comment_status_name(status: i32) -> &'static str {
        match pb::CommentStatus::try_from(status) {
            Ok(pb::CommentStatus::Queued) => "queued",
            Ok(pb::CommentStatus::Addressed) => "addressed",
            Ok(pb::CommentStatus::Resolved) => "resolved",
            _ => "sent",
        }
    }

    /// What a reply or resolve tool answers with.
    pub(super) fn comment_answer(comment: &pb::Comment) -> Value {
        json!({
            "comment_id": comment.id.as_ref().map(|id| id.value.clone()),
            "status": comment_status_name(comment.status),
            "replies": comment.replies.len(),
        })
    }

    /// One comment as `comments_list` shows it.
    pub(super) fn comment_json(comment: &pb::Comment) -> Value {
        let mut out = Map::new();
        out.insert(
            "id".to_owned(),
            json!(comment.id.as_ref().map(|id| id.value.clone())),
        );
        if let Some(anchor) = &comment.anchor {
            if let Some(file) = &anchor.file {
                let path = path_json(Some(file));
                out.insert("repo".to_owned(), path["repo"].clone());
                out.insert("path".to_owned(), path["path"].clone());
            }
            if let Some(lines) = &anchor.lines {
                out.insert("lines".to_owned(), lines_json(Some(lines)));
            }
            if anchor.side != pb::DiffSide::Unspecified as i32 {
                out.insert("side".to_owned(), json!(side_json(anchor.side)));
            }
            out.insert("quoted".to_owned(), json!(anchor.quoted));
        }
        out.insert("text".to_owned(), json!(comment.text));
        out.insert(
            "status".to_owned(),
            json!(comment_status_name(comment.status)),
        );
        if comment.status == pb::CommentStatus::Addressed as i32
            && !comment.addressed_text.is_empty()
        {
            out.insert("addressed_text".to_owned(), json!(comment.addressed_text));
        }
        let replies: Vec<Value> = comment
            .replies
            .iter()
            .map(|reply| {
                let by = match reply.author.as_ref().and_then(|actor| actor.actor.as_ref()) {
                    Some(pb::actor::Actor::Agent(agent)) => agent
                        .agent
                        .as_ref()
                        .map_or_else(|| "agent".to_owned(), |id| id.value.clone()),
                    _ => "user".to_owned(),
                };
                json!({ "by": by, "text": reply.text })
            })
            .collect();
        out.insert("replies".to_owned(), Value::Array(replies));
        Value::Object(out)
    }

    fn side_json(side: i32) -> &'static str {
        if side == pb::DiffSide::Old as i32 {
            "old"
        } else {
            "new"
        }
    }

    pub(super) fn context_json(context: &pb::ReviewContext) -> Value {
        let view = context.view.clone().unwrap_or_default();
        let open = view.open.as_ref().map_or(Value::Null, |place| {
            json!({
                "file": path_json(place.file.as_ref()),
                "visible": lines_json(place.visible.as_ref()),
                "selection": lines_json(place.selection.as_ref()),
                "side": side_json(place.side),
            })
        });
        let switches = context.switches.unwrap_or_default();
        json!({
            "open": open,
            "selected_text": context.selected_text,
            "order": view.order.iter().map(|path| path_json(Some(path))).collect::<Vec<_>>(),
            "viewed": view.viewed.iter().map(|viewed| path_json(viewed.file.as_ref())).collect::<Vec<_>>(),
            "highlights": view.highlights.iter().map(|highlight| json!({
                "file": path_json(highlight.file.as_ref()),
                "lines": lines_json(highlight.lines.as_ref()),
                "side": side_json(highlight.side),
                "label": highlight.label,
            })).collect::<Vec<_>>(),
            "notes": context.notes.iter().map(|note| {
                let anchor = note.anchor.clone().unwrap_or_default();
                json!({
                    "note_id": note.id.as_ref().map(|id| id.value.clone()),
                    "file": path_json(anchor.file.as_ref()),
                    "lines": lines_json(anchor.lines.as_ref()),
                    "text": note.text,
                    "outdated": anchor.outdated,
                })
            }).collect::<Vec<_>>(),
            "agent_may": { "focus": switches.focus, "notes": switches.notes, "order": switches.order },
        })
    }
}

#[cfg(test)]
mod approved_read_tests {
    use super::*;

    #[test]
    fn an_approved_read_tells_the_agent_to_repeat_it() {
        let shown = approved_result(status::READ_APPROVED, &[]);
        assert_eq!(shown["decision"], "approved");
        assert_eq!(shown["repeat_call"], true);
    }

    #[test]
    fn an_approved_file_read_shows_the_files_text() {
        use prost::Message;
        let chunk = pb::FileChunk {
            data: b"hello".to_vec(),
            last: true,
            ..Default::default()
        };
        let shown = approved_result("humata.v1.FileChunk", &chunk.encode_to_vec());
        assert_eq!(shown["text"], "hello");
        assert!(shown.get("more_from_offset").is_none());
    }

    #[test]
    fn an_approved_read_cut_short_says_where_the_rest_starts() {
        use prost::Message;
        let chunk = pb::FileChunk {
            offset: 10,
            data: b"hello".to_vec(),
            last: false,
            ..Default::default()
        };
        let shown = approved_result("humata.v1.FileChunk", &chunk.encode_to_vec());
        assert_eq!(shown["more_from_offset"], 15);
    }

    #[test]
    fn another_bare_approval_is_not_told_to_repeat() {
        let shown = approved_result("", &[]);
        assert_ne!(shown["repeat_call"], true);
    }
}

#[cfg(test)]
mod fork_choice_tests {
    use super::*;

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn a_fork_with_only_a_kind_forks_from_now_with_the_parents_agent() {
        let choices = fork_choices(&arguments(json!({ "kind": "code_only" }))).unwrap();
        assert_eq!(choices.kind, pb::ForkKind::CodeOnly);
        assert_eq!(choices.agent, None);
        assert_eq!(choices.before_turn, 0);
    }

    #[test]
    fn the_agent_and_turn_are_taken_as_given() {
        let choices = fork_choices(&arguments(json!({ "kind": "context_only", "agent": "codex", "before_turn": 5 }))).unwrap();
        assert_eq!(choices.kind, pb::ForkKind::ContextOnly);
        assert_eq!(choices.agent.as_deref(), Some("codex"));
        assert_eq!(choices.before_turn, 5);
    }

    #[test]
    fn every_copy_mode_is_taken() {
        for (name, expected) in [
            ("code_and_context", pb::ForkKind::CodeAndContext),
            ("code_only", pb::ForkKind::CodeOnly),
            ("context_only", pb::ForkKind::ContextOnly),
        ] {
            assert_eq!(fork_choices(&arguments(json!({ "kind": name }))).unwrap().kind, expected);
        }
    }

    #[test]
    fn a_turn_before_the_second_or_not_a_number_is_refused() {
        for turn in [json!(0), json!(1), json!(-3), json!("4"), json!(1.5), json!(10_000_000_000u64)] {
            let result = fork_choices(&arguments(json!({ "kind": "code_only", "before_turn": turn })));
            assert!(matches!(result, Err(ToolError { kind: ErrorKind::InvalidField, .. })), "{turn}");
        }
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        assert!(fork_choices(&arguments(json!({ "kind": "everything" }))).is_err());
    }
}
