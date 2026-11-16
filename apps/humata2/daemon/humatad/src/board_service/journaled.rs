//! A machine's agent calls its board while the host is away (AWAY T10c and
//! T10d, D227, AW-7 rows 2 and 3).
//!
//! A write the rules allow and the policy copy proves the agent may make is
//! kept in the machine's journal, whole, for the host to replay; the reply
//! says it was journaled and where, and carries no item, since a new item has
//! no id until the host replays it. One the rules ask about, under the While
//! I'm away rule Wait, waits as a request in the machine's Inbox; approving it
//! there journals the write with the request's id as its approval (D268).
//! Any other write, and every comment or view draft, is host_unreachable with
//! nothing kept.
//!
//! A read is answered from the host's last answer to the same call, which the
//! machine keeps while the host is linked, marked with the time the host
//! gave it. The rules judge first, so a read that is now refused isn't served;
//! a read never made while the Mac was reachable is host_unreachable.

use std::collections::BTreeSet;

use humata2_access::{AnswerAt, BoardAct, BoardRef, Call, ChannelId, MachineId, SessionId, SessionRef};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{BoardItemRecord, BoardKey};
use prost::Message;
use tonic::metadata::MetadataValue;
use tonic::{Code, Response, Status};

use super::asks::away_ask;
use super::{BoardSvc, Op, item_id};
use crate::away::{self, Method, Row, stale};
use crate::guard::AwayCall;
use crate::identity::{AgentCaller, Caller};
use crate::status;
use crate::inbox::{Deferred, Run};
use crate::trash_service::{answer_key, blocking, now_ms};

/// A call that can't go on while the Mac is away, as host_unreachable: what
/// stopped it, and what the agent can do.
fn unreachable(message: String, fix: &str) -> Status {
    status::failure(
        Code::Unavailable,
        pb::Error {
            message,
            fix: fix.to_owned(),
            retry: Some(pb::RetryAdvice {
                after: None,
                same_key: true,
            }),
            kind: Some(pb::error::Kind::HostUnreachable(pb::HostUnreachable {
                since: None,
            })),
        },
    )
}

/// The refusal for a read the machine holds no answer to.
fn never_read(row: &Row) -> Status {
    unreachable(
        format!(
            "{} was never read while the Mac was reachable, so this machine has no copy of its answer, and the Mac is away",
            row.name
        ),
        "repeat the call once the Mac is reachable; a read made then is kept for the next time the Mac is away",
    )
}

/// Whether a change to `item` changes who the copy says owns what: a new or
/// deleted item that a session made, or that sits on a session's board.
pub(super) fn changes_owned_items(change: pb::Change, item: &BoardItemRecord) -> bool {
    matches!(change, pb::Change::Created | pb::Change::Deleted)
        && (!item.created_by.is_empty() || matches!(item.board, BoardKey::Session(_)))
}

/// What an away write names.
enum Target<'a> {
    /// A new item on this board.
    New(Option<&'a pb::BoardRef>),
    /// An item that exists already.
    Item(Option<&'a pb::BoardItemId>),
}

/// What an away write is judged as, and whether the copy proves it.
struct Judged {
    board: BoardRef,
    act: BoardAct,
    proved: bool,
}

impl BoardSvc {
    /// The caller when it is an agent on a machine whose host is away: its
    /// board calls are the machine's to journal or answer from the copy.
    pub(super) fn away_agent<'a>(&self, caller: &'a Caller) -> Option<&'a AgentCaller> {
        let Caller::Agent(agent) = caller else {
            return None;
        };
        let forwarder = self.forwarder.as_ref()?;
        (forwarder.on_machine() && !forwarder.forwards()).then_some(agent)
    }

    fn policy_board(
        &self,
        agent: &AgentCaller,
        board: Option<&pb::BoardRef>,
        channel: Option<&pb::ChannelId>,
    ) -> BoardRef {
        use pb::board_ref::Board;
        // What can't be read is judged as the Global board, which isn't served.
        match board.and_then(|board| board.board.as_ref()) {
            Some(Board::Channel(id)) => BoardRef::Channel(ChannelId::new(id.value.as_str())),
            Some(Board::Session(id)) => BoardRef::Session(SessionRef {
                id: SessionId::new(id.value.as_str()),
                channel: channel.map_or_else(
                    || agent.session.channel.clone(),
                    |channel| ChannelId::new(channel.value.as_str()),
                ),
                machine: MachineId::new(self.machine.as_str()),
            }),
            Some(Board::Global(_)) | None => BoardRef::Global,
        }
    }

    // ------------------------------------------------------------- writes

    /// A write made while the host is away: `Some` is its reply, journaled,
    /// and an error is host_unreachable with nothing kept. `None` leaves the
    /// call to the usual path, for one that isn't an away agent's.
    pub(super) async fn journaled(
        &self,
        caller: &Caller,
        op: &Op,
    ) -> Result<Option<Response<pb::WriteItemResponse>>, Status> {
        let Some(agent) = self.away_agent(caller) else {
            return Ok(None);
        };
        let (name, what, idempotency) = op.identity();
        let key = answer_key(
            name,
            caller,
            idempotency
                .clone()
                .map(|value| pb::IdempotencyKey { value })
                .as_ref(),
            &what,
        );
        if let Some(done) = key.as_deref().and_then(|key| self.finished.get(key)) {
            return Ok(Some(Response::new(done)));
        }
        let revision =
            |revision: &Option<pb::Revision>| revision.as_ref().map(|revision| revision.value);
        let (method, bytes, base, target) = match op {
            Op::Create(request) => (
                Method::CreateItem,
                request.encode_to_vec(),
                None,
                Target::New(request.board.as_ref()),
            ),
            Op::Update(request) => (
                Method::UpdateItem,
                request.encode_to_vec(),
                revision(&request.expected_revision),
                Target::Item(request.id.as_ref()),
            ),
            Op::Move(request) => (
                Method::MoveItem,
                request.encode_to_vec(),
                revision(&request.expected_revision),
                Target::Item(request.id.as_ref()),
            ),
            Op::Delete(request) => (
                Method::DeleteItem,
                request.encode_to_vec(),
                revision(&request.expected_revision),
                Target::Item(request.id.as_ref()),
            ),
        };
        let row = method.row();
        let Some(operation) = row.journal else {
            return Err(status::internal(
                format!("{} has no name in the journal", row.name),
                "",
            ));
        };
        let session = agent.session.id.to_string();
        let judged = self.judged_away(agent, &session, row, target).await?;
        let verdict = self.guard.check_away(
            caller,
            &Call::Board {
                board: judged.board,
                act: judged.act,
            },
        )?;
        let asked = match verdict {
            AwayCall::Journal => None,
            // Only a request the machine answers is kept here: one the Mac
            // answers means the link came back since the rules were read.
            AwayCall::Asks(asked) if matches!(asked.answer_at, AnswerAt::Machine(_)) => Some(asked),
            AwayCall::Asks(_) | AwayCall::LastCopy | AwayCall::Reachable => {
                return Err(unreachable(
                    format!(
                        "the link to the Mac changed while {} was being judged, so it wasn't kept",
                        row.name
                    ),
                    "repeat the call with the same idempotency key",
                ));
            }
        };
        if !judged.proved {
            return Err(unreachable(
                format!(
                    "{} can't be kept while the Mac is away: it isn't a new item on your own board or your channel's board, or a change to an item the Mac's copy lists as yours",
                    row.name
                ),
                "repeat the call once the Mac is reachable; nothing was kept",
            ));
        }
        // The rules ask: the write waits in this machine's Inbox, and is
        // journaled, with the request's id, once the user approves it there.
        if let Some(asked) = asked {
            let (me, who) = (self.clone(), session);
            let run: Run = Box::new(move |request| {
                Box::pin(async move {
                    let response = me
                        .journal_write(&who, operation, bytes, base, Some(request))
                        .await?;
                    Ok(Deferred {
                        result: response.encode_to_vec(),
                        result_type: "humata.v1.WriteItemResponse".to_owned(),
                    })
                })
            });
            return Err(self
                .inbox
                .defer_board(caller, *asked, idempotency, away_ask(op), run)
                .await);
        }
        let response = self.journal_write(&session, operation, bytes, base, None).await?;
        if let Some(key) = key {
            self.finished.put(key, response.clone());
        }
        Ok(Some(Response::new(response)))
    }

    /// Keeps a write in the machine's journal for the host, naming the
    /// request the user approved for it when its rule asked, and says where it
    /// sits. A failure keeps nothing, and says so (D227).
    async fn journal_write(
        &self,
        session: &str,
        operation: &'static str,
        bytes: Vec<u8>,
        base: Option<u64>,
        approved: Option<String>,
    ) -> Result<pb::WriteItemResponse, Status> {
        let host_link = self.guard.host_link();
        let seq = away::record_board_journal(
            &self.store,
            &host_link,
            session,
            operation,
            bytes,
            base,
            approved,
        )
        .await
        .map_err(|error| {
            status::internal(
                format!(
                    "this change wasn't kept and will not reach the Mac: {}",
                    status::chain(&error)
                ),
                "",
            )
        })?;
        Ok(pb::WriteItemResponse {
            delivery: pb::Delivery::Journaled as i32,
            item: None,
            journal: Some(pb::JournalPosition {
                machine: Some(pb::MachineId {
                    value: self.machine.clone(),
                }),
                seq,
            }),
            provisional_id: None,
        })
    }

    /// The board and act an away write is judged by, and whether the policy
    /// copy proves the agent may journal it (D227): a new item on its own
    /// session board or its channel's board, or a change to an item the copy
    /// lists with the agent as owner.
    async fn judged_away(
        &self,
        agent: &AgentCaller,
        session: &str,
        row: &Row,
        target: Target<'_>,
    ) -> Result<Judged, Status> {
        match target {
            Target::New(board) => {
                let board = board.ok_or_else(|| status::missing("board"))?;
                let proved = match board.board.as_ref() {
                    Some(pb::board_ref::Board::Session(id)) => id.value == session,
                    Some(pb::board_ref::Board::Channel(id)) => {
                        id.value.as_str() == agent.session.channel.as_str()
                    }
                    Some(pb::board_ref::Board::Global(_)) | None => false,
                };
                Ok(Judged {
                    board: self.policy_board(agent, Some(board), None),
                    act: BoardAct::Add,
                    proved,
                })
            }
            Target::Item(id) => {
                let id = item_id(id)?;
                let (store, owner, item) = (self.store.clone(), session.to_owned(), id.clone());
                let owned = blocking(move || away::owned_item(&store, &owner, &item))
                    .await?
                    .map_err(|error| {
                        status::internal(
                            format!(
                                "couldn't read this machine's policy copy: {}",
                                status::chain(&error)
                            ),
                            "",
                        )
                    })?;
                let Some(owned) = owned else {
                    return Err(unreachable(
                        format!(
                            "{} can't be kept while the Mac is away: the Mac's copy doesn't list {id} as yours",
                            row.name
                        ),
                        "repeat the call once the Mac is reachable; nothing was kept",
                    ));
                };
                if owned.board.is_none() {
                    return Err(unreachable(
                        format!(
                            "{} can't be kept while the Mac is away: the Mac's copy doesn't say which board {id} is on",
                            row.name
                        ),
                        "repeat the call once the Mac is reachable; nothing was kept",
                    ));
                }
                Ok(Judged {
                    board: self.policy_board(agent, owned.board.as_ref(), None),
                    act: BoardAct::Edit {
                        created_by: Some(agent.session.id.clone()),
                    },
                    proved: true,
                })
            }
        }
    }

    // -------------------------------------------------------------- reads

    /// Keeps the host's answer to a read it was just sent (D227), for the
    /// next time the host is away. Best effort: a failure is logged.
    pub(super) async fn keep_last_copy(
        &self,
        session: &str,
        row: &'static Row,
        request: Vec<u8>,
        response: &[u8],
    ) {
        let (me, session, name, response, at) = (
            self.clone(),
            session.to_owned(),
            row.name,
            response.to_vec(),
            now_ms(),
        );
        let kept = blocking(move || {
            me.store
                .put_last_copy(&session, name, &request, &response, at)
        })
        .await;
        match kept {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: warning: the Mac's answer to {name} wasn't kept for when it is away: {}",
                status::chain(&error)
            ),
            Err(failed) => crate::log::say!(
                "humatad2: warning: keeping the Mac's answer to {name} stopped: {}",
                failed.message()
            ),
        }
    }

    /// Answers `request` for the away agent from the host's last answer to
    /// it, marked with when the host gave it. `boards` names the boards the
    /// answer shows, which the rules are asked about first.
    pub(super) async fn last_copy<Req: Message, Res: Message + Default>(
        &self,
        caller: &Caller,
        agent: &AgentCaller,
        row: &'static Row,
        request: &Req,
        boards: fn(&Self, &AgentCaller, &Req, &Res) -> Vec<BoardRef>,
    ) -> Result<Response<Res>, Status> {
        let (me, session, name, bytes) = (
            self.clone(),
            agent.session.id.to_string(),
            row.name,
            request.encode_to_vec(),
        );
        let kept = blocking(move || me.store.last_copy(&session, name, &bytes))
            .await?
            .map_err(|error| self.failed(error, "read the Mac's last answer"))?;
        let Some(kept) = kept else {
            return Err(never_read(row));
        };
        let Ok(response) = Res::decode(kept.response.as_slice()) else {
            // Kept by another version: as good as never read.
            return Err(never_read(row));
        };
        let boards: BTreeSet<BoardRef> = boards(self, agent, request, &response)
            .into_iter()
            .collect();
        for board in boards {
            match self.guard.check_away(
                caller,
                &Call::Board {
                    board,
                    act: BoardAct::Read,
                },
            )? {
                AwayCall::LastCopy => {}
                AwayCall::Journal | AwayCall::Asks(_) | AwayCall::Reachable => {
                    return Err(unreachable(
                        format!(
                            "{} can't be answered from the last copy: reading what it shows needs the Mac while it is away",
                            row.name
                        ),
                        "repeat the call once the Mac is reachable",
                    ));
                }
            }
        }
        let mark = MetadataValue::try_from(stale::format(kept.answered_unix_ms).as_str()).map_err(
            |error| {
                status::internal(
                    format!("couldn't mark the answer with how old it is: {error}"),
                    "",
                )
            },
        )?;
        let mut reply = Response::new(response);
        reply.metadata_mut().insert(stale::HEADER, mark);
        Ok(reply)
    }

    /// The boards a list shows: the one it asked for, and each item's.
    pub(super) fn list_boards(
        &self,
        agent: &AgentCaller,
        request: &pb::ListItemsRequest,
        response: &pb::ListItemsResponse,
    ) -> Vec<BoardRef> {
        use pb::board_view::View;
        let mut boards = Vec::new();
        match request.view.as_ref().and_then(|view| view.view.as_ref()) {
            Some(View::Board(board)) => boards.push(self.policy_board(agent, Some(board), None)),
            Some(View::ChannelWithSessions(channel)) => {
                boards.push(BoardRef::Channel(ChannelId::new(channel.value.as_str())))
            }
            Some(View::SessionWithForks(session)) => boards.push(BoardRef::Session(SessionRef {
                id: SessionId::new(session.value.as_str()),
                channel: agent.session.channel.clone(),
                machine: MachineId::new(self.machine.as_str()),
            })),
            Some(View::GlobalWithChannels(_)) => boards.push(BoardRef::Global),
            None => {}
        }
        for item in &response.items {
            boards.push(self.policy_board(agent, item.board.as_ref(), item.channel.as_ref()));
        }
        boards
    }

    /// The board an item is on.
    pub(super) fn item_boards(
        &self,
        agent: &AgentCaller,
        _request: &pb::GetItemRequest,
        response: &pb::BoardItem,
    ) -> Vec<BoardRef> {
        vec![self.policy_board(agent, response.board.as_ref(), response.channel.as_ref())]
    }

    /// A history doesn't say which board its item is on, so it is judged as
    /// a read of the agent's own channel.
    pub(super) fn history_boards(
        &self,
        agent: &AgentCaller,
        _request: &pb::GetItemHistoryRequest,
        _response: &pb::GetItemHistoryResponse,
    ) -> Vec<BoardRef> {
        vec![BoardRef::Channel(agent.session.channel.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(created_by: &str, board: BoardKey) -> BoardItemRecord {
        BoardItemRecord {
            key: "PAY-1".to_owned(),
            board,
            channel: "ch_1".to_owned(),
            title: "t".to_owned(),
            description: String::new(),
            column: 1,
            rank: "m".to_owned(),
            created_by: created_by.to_owned(),
            created_unix_ms: 1,
            updated_unix_ms: 1,
            revision: 1,
            done_unix_ms: None,
            deleted: false,
            sessions: Vec::new(),
            depends_on: Vec::new(),
        }
    }

    #[test]
    fn a_new_or_deleted_item_of_a_session_changes_what_the_copy_lists() {
        let channel = || BoardKey::Channel("ch_1".to_owned());
        assert!(changes_owned_items(
            pb::Change::Created,
            &item("se_a", channel())
        ));
        assert!(changes_owned_items(
            pb::Change::Deleted,
            &item("se_a", channel())
        ));
        assert!(changes_owned_items(
            pb::Change::Created,
            &item("", BoardKey::Session("se_a".to_owned()))
        ));
    }

    #[test]
    fn what_the_user_made_on_a_shared_board_or_an_edit_changes_nothing_the_copy_lists() {
        let channel = || BoardKey::Channel("ch_1".to_owned());
        assert!(!changes_owned_items(
            pb::Change::Created,
            &item("", channel())
        ));
        assert!(!changes_owned_items(
            pb::Change::Created,
            &item("", BoardKey::Global)
        ));
        assert!(!changes_owned_items(
            pb::Change::Updated,
            &item("se_a", channel())
        ));
    }
}
