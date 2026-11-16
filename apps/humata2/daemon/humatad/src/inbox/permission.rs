//! Permission cards (HOOKS, D191): an agent's blocking permission hook asks
//! through the Inbox whether its TUI may run a tool. The card lives only
//! while a hook holds it: the last [`PermissionWait`] dropping withdraws it,
//! and an answer that comes after no hook is holding is refused as
//! withdrawn. It isn't saved when raised, since a restart ends every hook
//! that held it; its answer or withdrawal is, for history.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use tokio::sync::watch;
use tonic::Status;

use humata2_store::{Store, StoreError};

use super::{Decided, Entry, Held, Inbox, inbox_ask, new_id, withdraw};
use crate::attach;
use crate::channel_service::blocking;
use crate::identity::{AgentCaller, Caller};
use crate::status;

pub(crate) mod codex_trust;

/// The store key of Settings › Agents › Ask for permission in the Inbox
/// (D199): "true" when asks stay in the agent's own prompt. With no row,
/// asks wait in the Inbox.
const IN_AGENT_KEY: &str = "agents.permissions_in_agent";

/// Whether permission asks stay in the agent's own prompt (D199), and the
/// revision that was saved at (0 for the default, the Inbox).
pub(crate) fn in_agent(store: &Store) -> Result<(bool, u64), StoreError> {
    Ok(store
        .setting(IN_AGENT_KEY)?
        .map_or((false, 0), |saved| (saved.value == "true", saved.revision)))
}

/// Saves where permission asks go (D199) at `revision`.
pub(crate) fn put_in_agent(store: &Store, on: bool, revision: u64) -> Result<(), StoreError> {
    store.put_setting(IN_AGENT_KEY, if on { "true" } else { "false" }, revision)
}

impl Inbox {
    /// A hook of a session of agent kind `kind` reached the daemon: if the
    /// kind carries Codex's hooks, they are trusted (D202). Failing to save
    /// that only leaves the trust step showing.
    pub(crate) async fn hook_heard(&self, kind: &str) {
        let (store, kind) = (self.store.clone(), kind.to_owned());
        if let Ok(Err(error)) = blocking(move || codex_trust::heard(&store, &kind)).await {
            crate::log::say!(
                "humatad2: couldn't note that Codex's hooks ran, so Settings may still ask to trust them: {}",
                status::chain(&error)
            );
        }
    }

    /// Whether permission asks stay in the agent's own prompt (D199), read
    /// at each ask so running sessions follow a change.
    pub(crate) async fn asks_in_agent(&self) -> Result<bool, Status> {
        let store = self.store.clone();
        blocking(move || in_agent(&store))
            .await?
            .map(|(on, _)| on)
            .map_err(|error| {
                status::store(error, &self.machine, "read where agents ask for permission")
            })
    }
}

/// What an agent's hook asks to run, as its payload said.
pub(crate) struct Ask {
    /// The tool, as the agent names it.
    pub(crate) tool: String,
    /// All of what it runs: a command, a patch, or the tool's input.
    pub(crate) command: String,
    pub(crate) description: Option<String>,
    /// The same ask gives the same key, so a rerun holds the same card.
    pub(crate) key: String,
}

/// How a card was settled, as the hook tells its agent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Allow,
    Deny { message: String },
    /// Withdrawn, or no one answered: the agent's own prompt asks.
    Undecided,
}

/// One hook holding a permission card. Wait on it with [`Self::decided`];
/// dropping it lets go, and the last to let go withdraws the card.
pub(crate) struct PermissionWait {
    inbox: Arc<Inbox>,
    id: String,
    /// `None` only while dropping.
    answers: Option<watch::Receiver<Held>>,
}

impl PermissionWait {
    /// Waits until the card is answered or withdrawn, and says what the
    /// hook tells its agent: a withdrawn card is no decision.
    pub(crate) async fn decided(&mut self) -> Decision {
        let Some(answers) = self.answers.as_mut() else {
            return Decision::Undecided;
        };
        loop {
            let decision = decision_of(&answers.borrow_and_update().request);
            if let Some(decision) = decision {
                return decision;
            }
            if answers.changed().await.is_err() {
                return Decision::Undecided;
            }
        }
    }
}

impl Drop for PermissionWait {
    fn drop(&mut self) {
        // Let go first, so the count the release reads leaves this one out.
        self.answers.take();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let inbox = Arc::clone(&self.inbox);
        let id = std::mem::take(&mut self.id);
        runtime.spawn(async move {
            // The withdrawal couldn't be saved: the card is withdrawn in
            // memory all the same, and a restart never brings it back.
            let _ = inbox.release_permission(&id).await;
        });
    }
}

/// What a card's state tells the hook, once it's settled.
fn decision_of(request: &pb::InboxRequest) -> Option<Decision> {
    match pb::RequestState::try_from(request.state) {
        Ok(pb::RequestState::Pending) => None,
        Ok(pb::RequestState::Approved) => Some(Decision::Allow),
        Ok(pb::RequestState::Denied) => {
            let message = match request.answer.as_ref().and_then(|answer| answer.answer.as_ref()) {
                Some(pb::request_answer::Answer::Deny(deny)) => deny.note.clone(),
                _ => String::new(),
            };
            Some(Decision::Deny { message })
        }
        _ => Some(Decision::Undecided),
    }
}

impl Inbox {
    /// Raises a card for `ask` from `agent`'s hook, or, when the same ask is
    /// already pending in its session, holds that one too.
    pub(crate) async fn raise_permission(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        agent_name: &str,
        ask: Ask,
    ) -> Result<PermissionWait, Status> {
        // An Ask verdict is fine: the card is the ask.
        self.guard.check_inbox_ask(caller, &inbox_ask(agent))?;
        let session = agent.session.id.to_string();
        let key = format!("{session}/{}", ask.key);
        let mut state = self.state.lock().await;
        let pending = state.id_with(Some(key.as_str())).and_then(|id| {
            state.requests.iter().find(|entry| {
                entry.id == id
                    && entry.permission
                    && entry.request.borrow().request.state == pb::RequestState::Pending as i32
            })
        });
        if let Some(entry) = pending {
            return Ok(PermissionWait {
                inbox: Arc::clone(self),
                id: entry.id.clone(),
                answers: Some(entry.request.subscribe()),
            });
        }
        let id = new_id();
        let card = card(&id, agent, agent_name, &self.machine, ask);
        let mut entry = Entry::new(id.clone(), session, None, None, Some(key), Held::new(card));
        entry.permission = true;
        let answers = entry.request.subscribe();
        self.tell(
            pb::Change::Created,
            &entry.session,
            &entry.request.borrow().request,
        );
        state.add(entry);
        Ok(PermissionWait {
            inbox: Arc::clone(self),
            id,
            answers: Some(answers),
        })
    }

    /// Withdraws permission card `id` if it's pending and no hook holds it.
    async fn release_permission(&self, id: &str) -> Result<(), Status> {
        let state = self.state.lock().await;
        let Some(entry) = state.requests.iter().find(|entry| entry.id == id) else {
            return Ok(());
        };
        self.withdraw_unheld(entry).await
    }

    /// Withdraws `entry` when it's a pending permission card that no hook
    /// holds any more. Call it with the state locked.
    pub(super) async fn withdraw_unheld(&self, entry: &Entry) -> Result<(), Status> {
        if !entry.permission || entry.request.receiver_count() > 0 {
            return Ok(());
        }
        let mut next = entry.request.borrow().clone();
        if next.request.state != pb::RequestState::Pending as i32 {
            return Ok(());
        }
        withdraw(&mut next.request, pb::WithdrawReason::CallEnded);
        let current = next.request.clone();
        let saved = self
            .commit(entry, next.clone(), None, "save the withdrawal")
            .await;
        if saved.is_err() {
            // Withdrawn even so: no hook is left to hear an answer.
            entry.request.send_replace(next);
        }
        self.tell(pb::Change::Updated, &entry.session, &current);
        saved
    }
}

/// What answering a permission card does: it takes Approve or Deny, as the
/// agent's own prompt would.
pub(super) fn decide(id: &str, answer: pb::request_answer::Answer) -> Result<Decided, Status> {
    use pb::request_answer::Answer as A;
    let state = match &answer {
        A::Approve(_) => pb::RequestState::Approved,
        A::Deny(_) => pb::RequestState::Denied,
        A::Always(_) | A::Reply(_) => {
            return Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} asks to run a tool once: it takes allow or deny"),
                "allow it or deny it; to allow it every time, change the rule in the agent's own settings",
            ));
        }
    };
    Ok(Decided {
        state,
        answer,
        grant: None,
    })
}

/// The card: what the agent wants to run, all of it (D191).
fn card(
    id: &str,
    agent: &AgentCaller,
    agent_name: &str,
    machine: &str,
    ask: Ask,
) -> pb::InboxRequest {
    let who = if agent.title.is_empty() {
        agent_name
    } else {
        agent.title.as_str()
    };
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: id.to_owned(),
        }),
        channel: Some(pb::ChannelId {
            value: agent.session.channel.to_string(),
        }),
        kind: pb::RequestKind::Session as i32,
        state: pb::RequestState::Pending as i32,
        requester: Some(agent.actor()),
        operation: agent.operation(),
        action_title: format!("{who} wants to run {}", ask.tool),
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Command(pb::CommandPreview {
                machine: Some(pb::MachineId {
                    value: machine.to_owned(),
                }),
                command: ask.command,
                notes: ask.description.unwrap_or_default(),
            })),
        }),
        capability: pb::Capability::InboxAsk as i32,
        created_at: Some(attach::now()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}
