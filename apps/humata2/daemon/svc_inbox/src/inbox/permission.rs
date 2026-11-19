//! Permission cards (HOOKS, D191): an agent's blocking permission hook asks
//! through the Inbox whether its TUI may run a tool. The card lives only
//! while a hook holds it: the last [`PermissionWait`] dropping withdraws it,
//! and an answer that comes after no hook is holding is refused as
//! withdrawn. On the host it isn't saved when raised, since a restart ends
//! every hook that held it; its answer or withdrawal is, for history. On a
//! machine it is saved and journaled when raised (D225, D744), so the host's
//! Inbox shows it and its answer comes back down the link; the machine's
//! next start withdraws it, and the journal tells the host.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use humata2_access::Verdict;
use humata2_daemon_ports::{AgentCaller, Caller};
use humata2_daemon_runtime::agent_switches::{self, Change, SwitchesError};
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::{blocking, helpers as status, log};
use humata2_sessions_core::Agents;
use humata2_store::{Store, StoreError};
use pb::invalid_field::Problem;
use thiserror::Error;
use tokio::sync::watch;
use tonic::Status;

use super::{Decided, Entry, Held, Inbox, inbox_ask, new_id, withdraw, withdrawn};
use crate::ports::{self, AgentGatePort};

pub mod codex_trust;

/// The ask key of a session's "waiting in its terminal" notice (D798).
const NOTICE_KEY: &str = "notice-in-terminal";

/// The store key D199's single switch was kept under: "true" when every
/// agent asked in its own prompt. D782 moved it to each kind's
/// `asks_in_inbox` in agents.toml; [`move_global_switch`] carries an old
/// "true" over at start and deletes the key.
const IN_AGENT_KEY: &str = "agents.permissions_in_agent";

/// Why D199's old switch couldn't be carried over to agents.toml.
#[derive(Debug, Error)]
pub enum MoveSwitchError {
    #[error("couldn't read D199's old \"ask in the agent\" setting from the store")]
    Read(#[source] StoreError),
    #[error(
        "couldn't turn off \"Ask in the Inbox\" for {kind} in agents.toml. The old setting is kept and moved again at the next start; until then {kind} asks in the Inbox"
    )]
    Write {
        kind: String,
        #[source]
        source: SwitchesError,
    },
    #[error(
        "turned \"Ask in the Inbox\" off for every agent, but couldn't delete the old setting from the store. Nothing is lost; the move runs again at the next start"
    )]
    Delete(#[source] StoreError),
    #[error(
        "couldn't set \"Ask in the Inbox\" for {kind} in agents.toml. The kinds before it were changed; set each agent's switch in Settings › Agents, or fix the file and try again"
    )]
    Set {
        kind: String,
        #[source]
        source: SwitchesError,
    },
}

/// D782's upgrade: if D199's old switch says every agent asks in its own
/// prompt, writes `asks_in_inbox = false` for every kind but the shell, then
/// deletes the old key. Gives whether there was anything to move. Without a
/// switches file (no home) it moves nothing and keeps the key.
pub fn move_global_switch(store: &Store, agents: &Agents) -> Result<bool, MoveSwitchError> {
    let Some(saved) = store.setting(IN_AGENT_KEY).map_err(MoveSwitchError::Read)? else {
        return Ok(false);
    };
    let Some(path) = agents.settings_file() else {
        return Ok(false);
    };
    if saved.value == "true" {
        let off = Change {
            asks_in_inbox: Some(false),
            ..Change::default()
        };
        for kind in agents.kinds().iter().filter(|kind| kind.name != "zsh") {
            agent_switches::set(path, &kind.name, off).map_err(|source| MoveSwitchError::Write {
                kind: agents.person_name(&kind.name),
                source,
            })?;
        }
    }
    store
        .delete_setting(IN_AGENT_KEY)
        .map_err(MoveSwitchError::Delete)?;
    Ok(saved.value == "true")
}

/// Whether every agent kind keeps its asks in its own terminal, so D199's
/// `permissions_in_agent` reads true (D782). False without agents to read.
pub fn every_kind_in_agent(agents: &Agents, gate: &dyn AgentGatePort) -> bool {
    let switches = gate.switches(agents).unwrap_or_default();
    let mut kinds = agents
        .kinds()
        .iter()
        .filter(|kind| kind.name != "zsh")
        .peekable();
    kinds.peek().is_some() && kinds.all(|kind| !switches.asks_in_inbox(&kind.name))
}

/// Sets D199's `permissions_in_agent` the D782 way: every kind's
/// `asks_in_inbox` to `!in_agent`.
pub fn set_every_kind(agents: &Agents, in_agent: bool) -> Result<(), MoveSwitchError> {
    let Some(path) = agents.settings_file() else {
        return Ok(());
    };
    let change = Change {
        asks_in_inbox: Some(!in_agent),
        ..Change::default()
    };
    for kind in agents.kinds().iter().filter(|kind| kind.name != "zsh") {
        agent_switches::set(path, &kind.name, change).map_err(|source| MoveSwitchError::Set {
            kind: agents.person_name(&kind.name),
            source,
        })?;
    }
    Ok(())
}

impl Inbox {
    /// A hook of a session of agent kind `kind` reached the daemon: if the
    /// kind carries Codex's hooks, they are trusted (D202). Failing to save
    /// that only leaves the trust step showing.
    pub async fn hook_heard(&self, kind: &str) {
        let (store, kind) = (self.store.clone(), kind.to_owned());
        if let Ok(Err(error)) = blocking(move || codex_trust::heard(&store, &kind)).await {
            log::line(&format!(
                "humatad2: couldn't note that Codex's hooks ran, so Settings may still ask to trust them: {}",
                status::chain(&error)
            ));
        }
    }

    /// Whether a session of agent kind `kind` asks in its own terminal
    /// (D782): its switch in agents.toml is off. Read at each ask, so
    /// running sessions follow a change; a missing or broken file, or no
    /// agents to read, means the Inbox.
    pub fn asks_in_agent(&self, kind: &str) -> bool {
        self.agents
            .get()
            .and_then(|agents| self.gate.get().and_then(|gate| gate.switches(agents)))
            .is_some_and(|switches| !switches.asks_in_inbox(kind))
    }
}

/// What an agent's hook asks to run, as its payload said.
pub struct Ask {
    /// The tool, as the agent names it.
    pub tool: String,
    /// All of what it runs: a command, a patch, or the tool's input.
    pub command: String,
    pub description: Option<String>,
    /// The tool changes files, so its card is an edit, not a command.
    pub edits_files: bool,
    /// The same ask gives the same key, so a rerun holds the same card.
    pub key: String,
}

/// What an agent's own ask-user form asks (D761), as its payload said.
pub struct Question {
    pub question: String,
    pub choices: Vec<String>,
    /// The same question gives the same key, so a rerun holds the same card.
    pub key: String,
    /// Only a reply naming one of `choices` (its words or its number) is
    /// taken, and it is kept as that choice's words (D767).
    pub only_choices: bool,
}

/// Whether the agent still asks a question on its screen (D767 amendment):
/// given a reply's choice, `Ok(false)` when the screen no longer asks it or
/// no longer lists that choice; given `None` (a Dismiss, D781), `Ok(false)`
/// only when the screen no longer asks it. An error when the screen can't
/// be read.
pub type StillAsked = Arc<
    dyn Fn(Option<String>) -> Pin<Box<dyn Future<Output = Result<bool, Status>> + Send>>
        + Send
        + Sync,
>;

/// How a card was settled, as the hook tells its agent.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny { message: String },
    /// A question card's answer (D761).
    Reply { text: String },
    /// Withdrawn, or no one answered: the agent's own prompt asks.
    Undecided,
}

/// One hook holding a permission card. Wait on it with [`Self::decided`];
/// dropping it lets go, and the last to let go withdraws the card.
pub struct PermissionWait {
    inbox: Arc<Inbox>,
    id: String,
    /// `None` only while dropping.
    answers: Option<watch::Receiver<Held>>,
}

impl PermissionWait {
    /// The card's request id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Waits until the card is answered or withdrawn, and says what the
    /// hook tells its agent: a withdrawn card is no decision.
    pub async fn decided(&mut self) -> Decision {
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
        Ok(pb::RequestState::Answered) => {
            let text = match request.answer.as_ref().and_then(|answer| answer.answer.as_ref()) {
                Some(pb::request_answer::Answer::Reply(reply)) => reply.text.clone(),
                _ => String::new(),
            };
            Some(Decision::Reply { text })
        }
        _ => Some(Decision::Undecided),
    }
}

impl Inbox {
    /// Raises a card for `ask` from `agent`'s hook, or, when the same ask is
    /// already pending in its session, holds that one too.
    pub async fn raise_permission(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        agent_name: &str,
        ask: Ask,
    ) -> Result<PermissionWait, Status> {
        let key = ask.key.clone();
        self.raise_held(caller, agent, &key, false, |id| {
            card(id, agent, agent_name, &self.machine, ask)
        })
        .await
    }

    /// D797: raises the card that says `agent_name` is waiting in its
    /// terminal, with its held first message `text` under it. Its one
    /// action is Approve ("Send it now"). Held as a permission card is:
    /// dropping the wait withdraws it.
    pub async fn raise_waiting(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        agent_name: &str,
        text: String,
    ) -> Result<PermissionWait, Status> {
        self.raise_held(caller, agent, "waiting-in-terminal", false, |id| {
            let mut card = card(
                id,
                agent,
                agent_name,
                &self.machine,
                Ask {
                    tool: String::new(),
                    command: text,
                    description: Some(
                        "Send it now types this message into the agent. Nothing else is sent."
                            .to_owned(),
                    ),
                    edits_files: false,
                    key: String::new(),
                },
            );
            card.action_title = format!("{agent_name} is waiting in its terminal");
            card
        })
        .await
    }

    /// D798: raises the card that says `agent_name` waits on a prompt in its
    /// terminal, with what the agent shows, `message`, under it. It has no
    /// action that types anything; Dismiss only closes it. Held as a
    /// permission card is: dropping the wait withdraws it.
    pub async fn raise_notice(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        agent_name: &str,
        message: String,
    ) -> Result<PermissionWait, Status> {
        self.raise_held(caller, agent, NOTICE_KEY, false, |id| {
            let mut card = card(
                id,
                agent,
                agent_name,
                &self.machine,
                Ask {
                    tool: String::new(),
                    command: message,
                    description: Some(
                        "Answer it in the session's terminal. Dismiss only closes this card; nothing is typed."
                            .to_owned(),
                    ),
                    edits_files: false,
                    key: String::new(),
                },
            );
            card.action_title = format!("{agent_name} is waiting in its terminal");
            card.dismiss_only = true;
            card
        })
        .await
    }

    /// Withdraws the "waiting in its terminal" card of `session`, if it has
    /// one: the agent went on, or asks another way.
    pub async fn withdraw_notice(&self, session: &str) {
        let key = format!("{session}/{NOTICE_KEY}");
        self.withdraw_held_where(session, |entry| entry.keys.contains(&key))
            .await;
    }

    /// Withdraws every ask a hook holds for `session` as resolved (D798):
    /// its turn ended, so each was answered in its terminal or is moot.
    /// Their hooks get no decision, and a later answer is refused.
    pub async fn withdraw_held(&self, session: &str) {
        self.withdraw_held_where(session, |_| true).await;
    }

    async fn withdraw_held_where(&self, session: &str, wanted: impl Fn(&Entry) -> bool) {
        let state = self.state.lock().await;
        for entry in &state.requests {
            if entry.permission
                && entry.session == session
                && entry.request.borrow().request.state == pb::RequestState::Pending as i32
                && wanted(entry)
            {
                self.withdraw_resolved(entry).await;
            }
        }
    }

    /// Raises a question card for the agent's own ask-user form (D761),
    /// held by its hook as a permission card is: Reply or Deny, left out of
    /// Answer all, withdrawn once no hook holds it.
    pub async fn raise_question(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        asked: Question,
    ) -> Result<PermissionWait, Status> {
        let Question {
            question,
            choices,
            key,
            only_choices,
        } = asked;
        self.raise_held(caller, agent, &key, only_choices, |id| {
            super::question(id, agent, question, choices)
        })
        .await
    }

    /// Raises a question the agent asks on its own screen (D767), as
    /// [`Self::raise_question`] does; a reply is taken only while
    /// `still_asked` says the screen still asks it with that choice, and
    /// withdraws the card otherwise. The card names `tab`, the agent tab's
    /// index among its session's tabs, so the app can show that screen
    /// under it (D781); a Deny of it first looks whether the screen still
    /// asks.
    pub async fn raise_screen_question(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        asked: Question,
        tab: u32,
        still_asked: StillAsked,
    ) -> Result<PermissionWait, Status> {
        let Question {
            question,
            choices,
            key,
            only_choices,
        } = asked;
        self.raise_held_asked(
            caller,
            agent,
            &key,
            only_choices,
            Some(still_asked),
            |id| {
                let mut card = super::question(id, agent, question, choices);
                card.on_screen = Some(pb::ScreenAsk { tab });
                card
            },
        )
        .await
    }

    /// Withdraws screen question `id` as resolved, if it is still pending:
    /// its screen no longer asks it, so it was answered in its terminal
    /// (D781). Nothing is typed.
    pub async fn withdraw_screen_resolved(&self, id: &str) {
        let state = self.state.lock().await;
        let Some(entry) = state.requests.iter().find(|entry| entry.id == id) else {
            return;
        };
        if entry.request.borrow().request.state == pb::RequestState::Pending as i32 {
            self.withdraw_resolved(entry).await;
        }
    }

    /// Raises the card `make` builds, or holds the pending one of the same
    /// `ask_key` in the agent's session.
    async fn raise_held(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        ask_key: &str,
        only_choices: bool,
        make: impl FnOnce(&str) -> pb::InboxRequest,
    ) -> Result<PermissionWait, Status> {
        self.raise_held_asked(caller, agent, ask_key, only_choices, None, make)
            .await
    }

    /// [`Self::raise_held`], for a card whose reply is checked against
    /// what the agent still asks.
    async fn raise_held_asked(
        self: &Arc<Self>,
        caller: &Caller,
        agent: &AgentCaller,
        ask_key: &str,
        only_choices: bool,
        still_asked: Option<StillAsked>,
        make: impl FnOnce(&str) -> pb::InboxRequest,
    ) -> Result<PermissionWait, Status> {
        // An Ask verdict is fine: the card is the ask.
        self.guard.check_inbox_ask(caller, &inbox_ask(agent))?;
        let session = agent.session.id.to_string();
        let key = format!("{session}/{ask_key}");
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
        let card = make(&id);
        let held = Held::new(card);
        let mut entry = Entry::new(id.clone(), session, None, None, Some(key), held.clone());
        entry.permission = true;
        entry.only_choices = only_choices;
        entry.still_asked = still_asked;
        if !self.guard.host_link().on_host() {
            // The host's Inbox is where the person answers (D744).
            self.commit(&entry, held, None, "save the permission ask")
                .await?;
        }
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

    /// Withdraws pending `entry`, whose question its agent no longer asks,
    /// and returns the refusal of the answer that found it so. Call it with
    /// the state locked.
    pub(super) async fn withdraw_unasked(&self, entry: &Entry) -> Status {
        withdrawn(&self.withdraw_resolved(entry).await)
    }

    /// Withdraws pending `entry` as resolved: the question it shows is no
    /// longer on its agent's screen, so it was answered there (D781).
    /// Returns the withdrawn request. Call it with the state locked.
    pub(super) async fn withdraw_resolved(&self, entry: &Entry) -> pb::InboxRequest {
        let mut next = entry.request.borrow().clone();
        withdraw(&mut next.request, pb::WithdrawReason::Resolved);
        let current = next.request.clone();
        let saved = self
            .commit(entry, next.clone(), None, "save the withdrawal")
            .await;
        if saved.is_err() {
            // Withdrawn even so: the question is gone from the screen.
            entry.request.send_replace(next);
        }
        self.tell(pb::Change::Updated, &entry.session, &current);
        current
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

/// What answering a card a hook holds does: a permission card takes
/// Approve or Deny, as the agent's own prompt would; a question card from
/// the agent's own form (D761) takes a reply or Deny.
pub(super) fn decide(
    current: &pb::InboxRequest,
    id: &str,
    mut answer: pb::request_answer::Answer,
    only_choices: bool,
) -> Result<Decided, Status> {
    use pb::request_answer::Answer as A;
    let question = current.kind == pb::RequestKind::Question as i32;
    let state = match &mut answer {
        A::Deny(_) => pb::RequestState::Denied,
        A::Reply(reply) if question => {
            if reply.text.trim().is_empty() {
                return Err(status::missing("answer.reply.text"));
            }
            if only_choices {
                let choices = choices_of(current);
                if choices.is_empty() {
                    // D785: the agent's screen showed only part of its
                    // choices, so the card offers none to pick.
                    return Err(status::invalid_field(
                        "answer.reply.text",
                        Problem::NotAllowedHere,
                        format!(
                            "request {id} offers no choice here: the agent's screen is too short to show them all, so none was taken and nothing was typed"
                        ),
                        "open the session's terminal, which shows every choice, and answer there; or dismiss the card",
                    ));
                }
                let Some(choice) = choice_named(&choices, &reply.text) else {
                    return Err(status::invalid_field(
                        "answer.reply.text",
                        Problem::NotAllowedHere,
                        format!(
                            "request {id} takes one of its choices, and \"{}\" is none of them",
                            reply.text.trim()
                        ),
                        format!(
                            "reply with one of: {} (its words or its number), or deny it to leave the question to the agent's screen",
                            choices.join("; ")
                        ),
                    ));
                };
                reply.text = choice;
            }
            pb::RequestState::Answered
        }
        A::Approve(_) | A::Always(_) if question => {
            return Err(status::invalid_field(
                "answer",
                Problem::NotAllowedHere,
                format!("request {id} is a question: it takes a reply, not an approval"),
                "reply with text (one of its choices, or your own), or deny it",
            ));
        }
        A::Approve(_) => pb::RequestState::Approved,
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

/// The choices a question card offers.
fn choices_of(request: &pb::InboxRequest) -> Vec<String> {
    match request
        .preview
        .as_ref()
        .and_then(|preview| preview.preview.as_ref())
    {
        Some(pb::request_preview::Preview::Question(question)) => question.choices.clone(),
        _ => Vec::new(),
    }
}

/// The choice `reply` names: its words, ignoring case and the space around
/// them, or its number counting from 1.
fn choice_named(choices: &[String], reply: &str) -> Option<String> {
    let reply = reply.trim();
    if let Ok(number) = reply.parse::<usize>() {
        return number
            .checked_sub(1)
            .and_then(|at| choices.get(at))
            .cloned();
    }
    choices
        .iter()
        .find(|choice| choice.trim().eq_ignore_ascii_case(reply))
        .cloned()
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
        // A file edit is a Files card; any other tool is a command.
        kind: (if ask.edits_files {
            pb::RequestKind::Files
        } else {
            pb::RequestKind::Session
        }) as i32,
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
        created_at: Some(ports::now_timestamp()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use humata2_daemon_runtime::agent_switches::AgentSwitches;

    use super::*;

    struct TestGate;

    impl AgentGatePort for TestGate {
        fn switches(&self, agents: &Agents) -> Option<AgentSwitches> {
            let path = agents.settings_file()?;
            AgentSwitches::read(path).ok()
        }
    }

    /// The built-in kinds, with their switches in a fresh agents.toml
    /// beside a fresh store.
    fn setup() -> (Store, Agents, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let agents = Agents::builtin().with_settings(dir.path().join("config/agents.toml"));
        (store, agents, dir)
    }

    fn asks(agents: &Agents, kind: &str) -> bool {
        TestGate
            .switches(agents)
            .unwrap_or_default()
            .asks_in_inbox(kind)
    }

    #[test]
    fn the_old_switch_on_turns_every_kinds_asks_off_and_is_forgotten() {
        let (store, agents, _dir) = setup();
        store.put_setting(IN_AGENT_KEY, "true", 3).unwrap();
        assert!(move_global_switch(&store, &agents).unwrap());
        assert!(!asks(&agents, "claude"));
        assert!(!asks(&agents, "copilot"));
        assert!(every_kind_in_agent(&agents, &TestGate));
        assert_eq!(store.setting(IN_AGENT_KEY).unwrap(), None);
        // A second start has nothing left to move.
        assert!(!move_global_switch(&store, &agents).unwrap());
    }

    #[test]
    fn the_old_switch_off_is_forgotten_and_changes_no_agent() {
        let (store, agents, _dir) = setup();
        store.put_setting(IN_AGENT_KEY, "false", 3).unwrap();
        assert!(!move_global_switch(&store, &agents).unwrap());
        assert!(asks(&agents, "claude"));
        assert!(!every_kind_in_agent(&agents, &TestGate));
        assert_eq!(store.setting(IN_AGENT_KEY).unwrap(), None);
    }

    #[test]
    fn the_old_switch_reads_true_only_while_every_kind_keeps_its_asks() {
        let (_store, agents, _dir) = setup();
        set_every_kind(&agents, true).unwrap();
        assert!(every_kind_in_agent(&agents, &TestGate));
        let path = agents.settings_file().unwrap();
        let on = Change {
            asks_in_inbox: Some(true),
            ..Change::default()
        };
        agent_switches::set(path, "copilot", on).unwrap();
        assert!(!every_kind_in_agent(&agents, &TestGate));
        set_every_kind(&agents, false).unwrap();
        assert!(asks(&agents, "claude") && asks(&agents, "copilot"));
    }

    #[test]
    fn a_broken_switches_file_keeps_every_kind_asking_in_the_inbox() {
        let (_store, agents, _dir) = setup();
        let path = agents.settings_file().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "[agents.copilot\nasks_in_inbox = false").unwrap();
        assert!(asks(&agents, "copilot"));
        assert!(!every_kind_in_agent(&agents, &TestGate));
    }
}
