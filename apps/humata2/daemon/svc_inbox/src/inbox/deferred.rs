//! Ordinary calls a channel rule sent to Ask me (D38.6). The call isn't
//! refused: it becomes an Inbox request, the agent waits on it (TP-11,
//! TP-12), and approving it runs the call (TP-13) whether or not anyone
//! still waits. The result is kept in memory for 24 hours; a restart loses
//! the closure that would run the call and any result, so an approved call
//! the restart cut off asks its agent to repeat the call with the same
//! idempotency key, which then runs it without asking again.

use std::sync::PoisonError;
use std::time::{Duration, SystemTime};

use humata2_access::{AccessRequest, AnswerError, Call, Capability, Decision, Place, Root};
use humata2_daemon_ports::{AgentCaller, Caller, Names};
use crate::pb;
use humata2_rpc_kit::helpers as status;
use humata2_store::Store;
use tonic::{Code, Status};

use super::{
    Decided, Entry, Held, Inbox, State, answered_by, id_of, kind_of, new_id, settle, settled,
};
use crate::ports::{self, InboxGuardPort};

/// How long an approved call's result stays available to a wait (TP-12).
const RESULT_KEPT: Duration = Duration::from_secs(24 * 60 * 60);

pub use humata2_daemon_ports::{BoardAsk, Deferred, Run};

/// How an approved call ended.
#[derive(Clone)]
pub(super) struct Outcome {
    pub(super) result: Vec<u8>,
    pub(super) result_type: String,
    /// What it failed with, or why it was denied after all.
    pub(super) error: Option<pb::Error>,
    pub(super) kept_until: SystemTime,
}

impl Outcome {
    fn of(done: Result<Deferred, Status>) -> Self {
        let kept_until = SystemTime::now() + RESULT_KEPT;
        match done {
            Ok(done) => Self {
                result: done.result,
                result_type: done.result_type,
                error: None,
                kept_until,
            },
            Err(failed) => Self {
                result: Vec::new(),
                result_type: String::new(),
                error: Some(status::status_error_of(&failed).unwrap_or_else(|| pb::Error {
                    message: failed.message().to_owned(),
                    fix: "check the message, then repeat the call".to_owned(),
                    retry: None,
                    kind: None,
                })),
                kept_until,
            },
        }
    }
}

/// Approved, but the call neither runs nor has a result: the process that
/// held it restarted.
pub(super) fn lost(held: &Held) -> bool {
    held.deferred
        && held.request.state == pb::RequestState::Approved as i32
        && !held.running
        && held.outcome.is_none()
}

pub(super) fn lost_error(request: &str) -> pb::Error {
    status::error(
        format!(
            "request {request} was approved, but humatad2 restarted before the call finished, so its result is gone"
        ),
        "repeat the original call with the same idempotency key: it runs again without asking the user a second time, unless the rule has changed since",
        pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn {
            request: Some(pb::RequestId {
                value: request.to_owned(),
            }),
        }),
    )
}

/// Names an asked read for its card: one card per session (added by the
/// caller), capability, path and operation (D768). The operation is in it
/// because an approval runs that read and the wait returns its result, which
/// must be the result of the call that waits; the entry kind isn't.
fn read_key(asked: &AccessRequest, operation: &str) -> String {
    match &asked.call {
        Call::ReadFiles(place) => format!(
            "read/{operation}/{:?}/{:?}/{:?}/{:?}",
            asked.capability, place.machine, place.root, place.path
        ),
        call => format!("read/{operation}/{:?}/{call:?}", asked.capability),
    }
}

/// An asked read's result went to a waiter, or the read was repeated: its
/// bytes are dropped, and the card now tells a later waiter to repeat the
/// read, which goes through for the rest of the keep time (D768).
pub(super) fn delivered(held: &tokio::sync::watch::Sender<Held>) {
    held.send_if_modified(|held| {
        if !held.once {
            return false;
        }
        held.taken = true;
        if let Some(outcome) = &mut held.outcome {
            if outcome.error.is_none() && outcome.result_type != crate::ports::READ_APPROVED {
                outcome.result = Vec::new();
                crate::ports::READ_APPROVED.clone_into(&mut outcome.result_type);
            }
        }
        true
    });
}

/// How long an approved read's bytes wait for the agent before they're
/// dropped and it is told to repeat the read (D768).
const READ_HELD: Duration = Duration::from_secs(10 * 60);

/// What a repeat of an asked read meets in its card (D768).
enum Standing {
    /// The user hasn't answered: the read waits on this request.
    Waiting(pb::RequestId),
    /// Approved: the read goes through.
    Through,
    /// The user denied it: the read is told so.
    Denied(pb::Error),
    /// Withdrawn, failed, lost to a restart or past its keep time: the read
    /// asks again.
    Gone,
}

fn standing(held: &Held) -> Standing {
    match pb::RequestState::try_from(held.request.state) {
        Ok(pb::RequestState::Pending) => {
            Standing::Waiting(held.request.id.clone().unwrap_or_default())
        }
        Ok(pb::RequestState::Approved) if held.running => Standing::Through,
        Ok(pb::RequestState::Approved) => match &held.outcome {
            Some(outcome) if outcome.error.is_none() && SystemTime::now() <= outcome.kept_until => {
                Standing::Through
            }
            _ => Standing::Gone,
        },
        Ok(pb::RequestState::Denied) => {
            settled(held).error.map_or(Standing::Gone, Standing::Denied)
        }
        _ => Standing::Gone,
    }
}

/// Turns `place` (a file or folder read) into a `pb::RepoPath`, splitting out
/// the `owner/name` repository prefix when `place` targets a repository clone.
fn repo_path_of(place: &Place, store: &Store) -> pb::RepoPath {
    let raw = place.path.as_str();
    match &place.root {
        Root::MainClone { repo, .. } => {
            let repo = repo
                .as_str()
                .split_once('/')
                .map(|(owner, name)| pb::RepoRef {
                    owner: owner.to_owned(),
                    name: name.to_owned(),
                });
            pb::RepoPath {
                repo,
                path: raw.to_owned(),
            }
        }
        Root::Clone(session) => {
            let repos = store
                .session_detail(session.id.as_str())
                .ok()
                .flatten()
                .map(|detail| detail.repos)
                .unwrap_or_default();
            for repo in repos {
                let prefix = format!("{}/{}", repo.owner, repo.name);
                if raw == prefix {
                    return pb::RepoPath {
                        repo: Some(pb::RepoRef {
                            owner: repo.owner,
                            name: repo.name,
                        }),
                        path: String::new(),
                    };
                }
                if let Some(rest) = raw.strip_prefix(&format!("{prefix}/")) {
                    return pb::RepoPath {
                        repo: Some(pb::RepoRef {
                            owner: repo.owner,
                            name: repo.name,
                        }),
                        path: rest.to_owned(),
                    };
                }
            }
            pb::RepoPath {
                repo: None,
                path: raw.to_owned(),
            }
        }
        _ => pb::RepoPath {
            repo: None,
            path: raw.to_owned(),
        },
    }
}

/// Builds a `FileAccessPreview` for an asked `Call::ReadFiles(place)`.
fn file_read_preview(place: &Place, store: &Store) -> pb::FileAccessPreview {
    let source_session = match &place.root {
        Root::Clone(session) | Root::Pastes(session) | Root::AgentState(session) => Some(pb::SessionId {
            value: session.id.to_string(),
        }),
        _ => None,
    };
    pb::FileAccessPreview {
        source_session,
        source_machine: Some(pb::MachineId {
            value: place.machine.to_string(),
        }),
        mode: pb::file_access_preview::Mode::ReadThisCall as i32,
        paths: vec![repo_path_of(place, store)],
    }
}

/// The card for a deferred call (IN-9, IN-10). It offers "Always allow" of
/// the exact target IN-22 names for the call, when the call has one (IN-14).
#[allow(clippy::too_many_arguments)]
fn card(
    guard: &dyn InboxGuardPort,
    store: &Store,
    id: &str,
    caller: &Caller,
    asked: &AccessRequest,
    decision: Option<Decision>,
    board: Option<BoardAsk>,
    names: &Names,
    working_on: String,
) -> pb::InboxRequest {
    let (requester, operation, title) = match caller {
        Caller::Agent(agent) => (Some(agent.actor()), agent.operation(), agent.title.clone()),
        Caller::User(_) => (None, String::new(), "CLI user".to_owned()),
        Caller::Script(script) => (None, String::new(), format!("Script: {}", script.name)),
        Caller::Peer => (None, String::new(), "Peer".to_owned()),
    };
    let capability = guard.capability_pb(asked.capability) as i32;
    let offered = asked.always_let.as_ref();
    let target = offered.map(|target| guard.target_pb(target));
    let target_label =
        offered.map_or_else(|| asked.capability.label().to_owned(), super::describe);
    let (reason, preview, mut details) = match board {
        Some(board) => {
            let details = match (&board.preview, &asked.call) {
                (pb::request_preview::Preview::FileAccess(access), Call::ReadFiles(place)) => {
                    super::facts::file_read_details(access, place, store, names)
                }
                (preview, _) => super::facts::details(preview, names),
            };
            (board.reason, board.preview, details)
        }
        None => match &asked.call {
            Call::ReadFiles(place) => {
                let file_access = file_read_preview(place, store);
                let details = super::facts::file_read_details(&file_access, place, store, names);
                (
                    String::new(),
                    pb::request_preview::Preview::FileAccess(file_access),
                    details,
                )
            }
            _ => (
                String::new(),
                pb::request_preview::Preview::AccessAhead(pb::AccessAheadPreview {
                    capability,
                    target: target.clone(),
                    target_label: target_label.clone(),
                }),
                super::facts::Details::default(),
            ),
        },
    };
    // A start or fork names where the new session goes (D910).
    let start = super::facts::start_facts(&asked.call, store, names);
    details.facts.splice(0..0, start);
    // A Desk agent's start grant names no channel, so the option says so.
    let always_label = if super::facts::always_covers_any_channel(asked) {
        format!("{target_label}, in any channel")
    } else {
        target_label
    };
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: id.to_owned(),
        }),
        channel: Some(pb::ChannelId {
            value: asked.channel.to_string(),
        }),
        kind: kind_of(asked.capability) as i32,
        state: pb::RequestState::Pending as i32,
        requester,
        operation,
        action_title: super::asks_title(&title, asked.capability, false),
        reason,
        preview: Some(pb::RequestPreview {
            preview: Some(preview),
        }),
        capability,
        rule_decision: decision.map_or(pb::Decision::Unspecified, super::decision_of) as i32,
        always: target.map(|target| pb::AlwaysOption {
            label: format!("Always allow “{title}” {always_label}"),
            target: Some(target),
        }),
        created_at: Some(ports::now_timestamp()),
        revision: Some(pb::Revision { value: 1 }),
        facts: details.facts,
        affects: details.affects,
        undo_text: details.undo_text,
        working_on,
        rule_label: super::facts::rule_label(asked.capability),
        ..Default::default()
    }
}

impl Inbox {
    /// "Working on <item key> <title>" when the asking session is on a board
    /// item of its channel; empty when it is on none or the store can't say.
    pub(super) fn working_on(&self, agent: &AgentCaller) -> String {
        self.store
            .board_items_in_channel(&agent.session.channel.to_string())
            .map(|items| super::facts::working_on(&items, &agent.session.id.to_string()))
            .unwrap_or_default()
    }

    /// Puts the agent's call in the Inbox, to run once the user approves it
    /// (D38.6). Returns the status the handler returns: the call is waiting
    /// on that request (`approval_pending`), or why it can't be (a store
    /// failure, a rule that is Never by now). The same caller and key
    /// return the request already made (TP-14).
    pub async fn defer(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        key: Option<String>,
        run: Run,
    ) -> Status {
        match self.deferred(caller, asked, key, None, run).await {
            Ok(id) => crate::ports::approval_pending(&id),
            Err(refused) => refused,
        }
    }

    /// [`Inbox::defer`] for a board call or a fork: its card shows `board`'s
    /// change or the fork's new session rather than the capability (BD-15,
    /// IN-10).
    pub async fn defer_board(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        key: Option<String>,
        board: BoardAsk,
        run: Run,
    ) -> Status {
        match self.deferred(caller, asked, key, Some(board), run).await {
            Ok(id) => crate::ports::approval_pending(&id),
            Err(refused) => refused,
        }
    }

    /// An agent's file read the channel's rule asks about (D768): the read
    /// isn't held open. The first one raises an Inbox card and answers
    /// `approval_pending` at once; a repeat of it (same session, capability,
    /// path and `operation`) answers with that card while it waits, goes
    /// through once it is approved (for as long as an approved call's result
    /// is kept), and is told the user's deny. Past that, the next read asks
    /// again. Approving runs `run`, whose result the wait returns; with no
    /// `run` (a read that can't be replayed) the agent repeats the read.
    pub async fn ask_read(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status> {
        self.ask_read_reason(caller, asked, operation, String::new(), run)
            .await
    }

    /// [`Inbox::ask_read`] with the agent's `reason` for reading, shown on the
    /// card's `FileAccessPreview` when `asked.call` is a file read.
    pub async fn ask_read_reason(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        reason: String,
        run: Option<Run>,
    ) -> Result<(), Status> {
        let board = match &asked.call {
            Call::ReadFiles(place) if !reason.trim().is_empty() => Some(BoardAsk {
                preview: pb::request_preview::Preview::FileAccess(file_read_preview(
                    place,
                    &self.store,
                )),
                reason,
            }),
            _ => None,
        };
        self.ask_read_with(caller, asked, operation, run, board).await
    }

    /// [`Inbox::ask_read`] whose card shows `board`'s preview and reason
    /// in place of the capability, as a copy's card shows what it copies.
    pub async fn ask_read_with(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
        board: Option<BoardAsk>,
    ) -> Result<(), Status> {
        let Caller::Agent(agent) = caller else {
            return Err(crate::ports::needs_approval(&*self.guard, asked.capability));
        };
        let runs = run.is_some();
        let key = read_key(&asked, operation);
        let full = format!("{}/{key}", agent.session.id);
        let mut state = self.state.lock().await;
        let held = state
            .id_with(Some(&full))
            .and_then(|id| state.requests.iter().find(|entry| entry.id == id))
            .map(|entry| entry.request.borrow().clone());
        if let Some(held) = held {
            match standing(&held) {
                Standing::Waiting(id) => return Err(crate::ports::read_approval_pending(&id, runs)),
                Standing::Through => {
                    if let Some(entry) = state
                        .id_with(Some(&full))
                        .and_then(|id| state.requests.iter().find(|entry| entry.id == id))
                    {
                        delivered(&entry.request);
                    }
                    return Ok(());
                }
                Standing::Denied(error) => {
                    state.keys.remove(&full);
                    return Err(status::failure(Code::PermissionDenied, error));
                }
                Standing::Gone => {
                    state.keys.remove(&full);
                }
            }
        }
        let run: Run = run.unwrap_or_else(|| {
            Box::new(|_| {
                Box::pin(async {
                    Ok(Deferred {
                        result: Vec::new(),
                        result_type: crate::ports::READ_APPROVED.to_owned(),
                    })
                })
            })
        });
        let id = self
            .deferred_in(&mut state, caller, asked, Some(key), board, run)
            .await?;
        if runs {
            if let Some(entry) = state.requests.iter().find(|entry| entry.id == id.value) {
                entry.request.send_modify(|held| held.once = true);
            }
        }
        Err(crate::ports::read_approval_pending(&id, runs))
    }

    /// The pending requests whose card shows a board change or a new item,
    /// as they stand, for the board to answer in place (BD-15).
    pub async fn pending_board_asks(&self) -> Vec<pb::InboxRequest> {
        let state = self.state.lock().await;
        state
            .requests
            .iter()
            .map(|entry| entry.request.borrow().request.clone())
            .filter(|request| {
                request.state == pb::RequestState::Pending as i32
                    && matches!(
                        request
                            .preview
                            .as_ref()
                            .and_then(|preview| preview.preview.as_ref()),
                        Some(pb::request_preview::Preview::BoardChange(_))
                    )
            })
            .collect()
    }

    /// Withdraws every pending request about board item `item`, as when it
    /// is deleted or purged. Their calls never run.
    pub async fn withdraw_about_item(&self, item: &str) -> Result<(), Status> {
        let state = self.state.lock().await;
        for entry in &state.requests {
            let mut next = entry.request.borrow().clone();
            let about = match next
                .request
                .preview
                .as_ref()
                .and_then(|p| p.preview.as_ref())
            {
                Some(pb::request_preview::Preview::BoardChange(change)) => {
                    change.item.as_ref().is_some_and(|id| id.value == item)
                }
                _ => false,
            };
            if !about || next.request.state != pb::RequestState::Pending as i32 {
                continue;
            }
            super::withdraw(&mut next.request, pb::WithdrawReason::ItemDeleted);
            let current = next.request.clone();
            self.commit(entry, next, None, "save the withdrawal")
                .await?;
            entry
                .run
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.tell(pb::Change::Updated, &entry.session, &current);
        }
        Ok(())
    }

    async fn deferred(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        key: Option<String>,
        board: Option<BoardAsk>,
        run: Run,
    ) -> Result<pb::RequestId, Status> {
        let mut state = self.state.lock().await;
        self.deferred_in(&mut state, caller, asked, key, board, run)
            .await
    }

    /// [`Inbox::deferred`] with the Inbox's state already held.
    async fn deferred_in(
        &self,
        state: &mut State,
        caller: &Caller,
        asked: AccessRequest,
        key: Option<String>,
        board: Option<BoardAsk>,
        run: Run,
    ) -> Result<pb::RequestId, Status> {
        let (session, key_prefix, working_on) = match caller {
            Caller::Agent(agent) => {
                let session = agent.session.id.to_string();
                let working_on = self.working_on(agent);
                (session.clone(), session, working_on)
            }
            Caller::User(_) => (String::new(), "user".to_owned(), String::new()),
            Caller::Script(script) => (
                String::new(),
                format!("script:{}", script.id),
                String::new(),
            ),
            Caller::Peer => {
                return Err(status::internal(
                    "a call was deferred for a peer daemon",
                    "",
                ));
            }
        };
        let key = id_of(key).map(|key| format!("{key_prefix}/{key}"));
        if let Some(id) = state.id_with(key.as_deref()) {
            if let Some(entry) = state.requests.iter().find(|entry| entry.id == id) {
                self.rerun(entry, &asked, run);
            }
            return Ok(pb::RequestId { value: id });
        }
        if asked.capability == Capability::DeskMoveToChannel && !session.is_empty() {
            // DK-19: a session has one pending move request; a new one
            // withdraws the old.
            self.withdraw_moves_of(&state.requests, &session).await?;
        }
        let id = new_id();
        let decision = self.rule_now(&asked);
        let names = Names::of_host(&self.machine);
        let mut held = Held::new(card(
            self.guard.as_ref(),
            &self.store,
            &id,
            caller,
            &asked,
            decision,
            board,
            &names,
            working_on,
        ));
        held.deferred = true;
        let entry = Entry::new(id.clone(), session, Some(asked), None, key, held.clone());
        *entry.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(run);
        self.commit(&entry, held, None, "save the request").await?;
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        state.add(entry);
        Ok(pb::RequestId { value: id })
    }

    /// Withdraws `session`'s pending requests to move it into a channel, as
    /// superseded by the one it is asking now. Their calls never run.
    async fn withdraw_moves_of(&self, requests: &[Entry], session: &str) -> Result<(), Status> {
        let moves = requests.iter().filter(|entry| {
            entry.session == session
                && entry
                    .access
                    .as_ref()
                    .is_some_and(|asked| asked.capability == Capability::DeskMoveToChannel)
        });
        for entry in moves {
            let mut next = entry.request.borrow().clone();
            if next.request.state != pb::RequestState::Pending as i32 {
                continue;
            }
            super::withdraw(&mut next.request, pb::WithdrawReason::Superseded);
            let current = next.request.clone();
            self.commit(entry, next, None, "save the withdrawal")
                .await?;
            entry
                .run
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.tell(pb::Change::Updated, &entry.session, &current);
        }
        Ok(())
    }

    /// A repeat of a call a restart cut off after it was approved runs it
    /// again, unless its rule is Never by now.
    fn rerun(&self, entry: &Entry, asked: &AccessRequest, run: Run) {
        if !lost(&entry.request.borrow()) || self.rule_now(asked) == Some(Decision::Never) {
            return;
        }
        *entry.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(run);
        entry.request.send_modify(|held| held.running = true);
        self.start(entry);
    }

    /// The channel's rule for `asked`'s capability as it is now.
    fn rule_now(&self, asked: &AccessRequest) -> Option<Decision> {
        ports::with_policy(self.guard.as_ref(), |policy, _| {
            policy
                .channel_rules(&asked.channel)
                .and_then(|rules| rules.decision(asked.capability))
        })
    }

    /// Settles `entry` as denied when the user approves a deferred call
    /// whose rule has become Never since it was asked (IN-21). Returns the
    /// status the answer fails with, or `None` when there is nothing to
    /// deny. The caller tells the feed.
    pub(super) async fn deny_if_never(
        &self,
        entry: &Entry,
        caller: &Caller,
        user: &humata2_access::PresentUser,
        approving: bool,
        at: SystemTime,
    ) -> Result<Option<Status>, Status> {
        let Some(asked) = entry.access.as_ref().filter(|_| approving) else {
            return Ok(None);
        };
        if !entry.request.borrow().deferred || self.rule_now(asked) != Some(Decision::Never) {
            return Ok(None);
        }
        let denied = pb::Denied {
            source: pb::denied::Source::NeverRule as i32,
            capability: self.guard.capability_pb(asked.capability) as i32,
            request: Some(pb::RequestId {
                value: entry.id.clone(),
            }),
            user_note: String::new(),
        };
        let error = status::error(
            format!(
                "the rule for \"{}\" became Never after this was asked, so the user couldn't approve it",
                asked.capability
            ),
            "don't repeat the call; ask the user to change the rule in Agent access if it should run",
            pb::error::Kind::Denied(denied),
        );
        let decided = Decided {
            state: pb::RequestState::Denied,
            answer: pb::request_answer::Answer::Deny(pb::Deny::default()),
            grant: None,
        };
        let mut next = entry.request.borrow().clone();
        settle(&mut next, decided, answered_by(caller, user), at);
        next.outcome = Some(Outcome {
            result: Vec::new(),
            result_type: String::new(),
            error: Some(error),
            kept_until: at + RESULT_KEPT,
        });
        self.commit(entry, next, Some(user), "save the answer")
            .await?;
        let error = AnswerError::RuleIsNever {
            channel: asked.channel.clone(),
            capability: asked.capability,
        };
        Ok(Some(self.guard.answer_failed(&error)))
    }

    /// Runs `entry`'s call if it was just approved, in a task of its own
    /// that finishes whether or not anyone waits (TP-13).
    pub(super) fn start(&self, entry: &Entry) {
        {
            let held = entry.request.borrow();
            if !held.deferred || held.request.state != pb::RequestState::Approved as i32 {
                return;
            }
        }
        let run = entry
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let sender = entry.request.clone();
        let id = entry.id.clone();
        let Some(run) = run else {
            sender.send_modify(|held| held.running = false);
            return;
        };
        tokio::spawn(async move {
            // A task of its own, so a panic in the call ends it as a failure.
            let done = match tokio::spawn(run(id)).await {
                Ok(done) => done,
                Err(error) => Err(status::internal(format!("the call failed: {error}"), "")),
            };
            let outcome = Outcome::of(done);
            let (mut once, mut taken) = (false, false);
            sender.send_modify(|held| {
                held.running = false;
                held.outcome = Some(outcome);
                (once, taken) = (held.once, held.taken);
            });
            // D768: an asked read's bytes go to its first taker, and are
            // never held past READ_HELD.
            if once {
                if !taken {
                    tokio::time::sleep(READ_HELD).await;
                }
                delivered(&sender);
            }
        });
    }
}
