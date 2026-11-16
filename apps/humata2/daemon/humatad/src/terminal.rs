//! TerminalService: tabs, each a shell kept by a holder of its own, typing
//! into a session, and pasted images. Every call is checked for its caller
//! first. Typing queued for a turn end isn't built yet.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, Capability, Target};
use humata2_chunk::Link;
use humata2_grpc::humata::v1 as pb;
use humata2_holder::{HolderError, HolderSpec, KeyState, KeyedInput};
use humata2_store::{SessionRecord, Store, StoreError, TabKind, TabRecord};
use pb::invalid_field::Problem;
use tokio::sync::watch;
use tonic::{Request, Response, Status, Streaming};

use crate::attach::{self, AttachStream};
use crate::channel_service::{Gated, answer_gated, gate};
use crate::compression::Compression;
use crate::guard::Guard;
use crate::holders::{Holders, TypeFailed};
use crate::identity::{self, Caller, Identities};
use crate::inbox::Inbox;
use crate::{paste, status};

/// How long a holder gets to answer a first connection before a tab is
/// listed without its offsets.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Results kept for retries with the same idempotency key.
const REMEMBERED_MAX: usize = 4096;

/// A new tab's size until a client attaches with its own.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// How long a tab's shell gets to take text typed into it.
const TYPE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct TerminalsConfig {
    pub(crate) store: Arc<Store>,
    pub(crate) machine: String,
    /// The `humata2_holder` binary.
    pub(crate) holder: PathBuf,
    pub(crate) shell: PathBuf,
    pub(crate) shell_args: Vec<OsString>,
    /// Where a machine shell starts.
    pub(crate) root: PathBuf,
    /// Where holders put their sockets.
    pub(crate) holders: PathBuf,
    /// Where holders write what goes wrong.
    pub(crate) logs: PathBuf,
    pub(crate) scrollback_bytes: usize,
    pub(crate) guard: Arc<Guard>,
    /// Gives each session tab's shell its session token.
    pub(crate) identities: Arc<Identities>,
    /// Picks the codec an attach's output goes with.
    pub(crate) compression: Arc<Compression>,
    /// Where a call a rule sends to Ask me waits for the user (D38.6).
    pub(crate) inbox: Arc<Inbox>,
}

/// The service. Its state sits behind an `Arc` so a call waiting for the
/// user's approval can own what it needs to run later.
pub(crate) struct Terminals {
    core: Arc<Core>,
}

pub(crate) struct Core {
    config: TerminalsConfig,
    remembered: Mutex<Remembered>,
    writers: Arc<Writers>,
    holders: Holders,
}

impl Terminals {
    /// The shared state, for the review's comment sink.
    pub(crate) fn core(&self) -> Arc<Core> {
        self.core.clone()
    }
}

impl std::ops::Deref for Terminals {
    type Target = Core;

    fn deref(&self) -> &Core {
        &self.core
    }
}

impl Terminals {
    pub(crate) fn new(config: TerminalsConfig) -> Self {
        Self {
            core: Arc::new(Core {
                remembered: Mutex::new(Remembered::default()),
                writers: Arc::new(Writers::default()),
                holders: Holders::new(config.scrollback_bytes),
                config,
            }),
        }
    }
}

/// What became of typing a batch of review comments (D138).
pub(crate) enum CommentsTyped {
    /// All of it reached the tab's shell.
    Yes,
    /// None of it was typed.
    No(Status),
    /// Some or all of it may have been typed.
    Unknown(Status),
}

/// Where a session's comment batches are typed: the tab, and the holder
/// process that held its terminal when the batch was begun.
pub(crate) struct CommentsTarget {
    pub(crate) tab: String,
    /// The holder's instance (`humata2_holder::KeyState`), or 0 when it
    /// can't confirm what it typed: an older holder, or one that didn't
    /// answer.
    pub(crate) holder: u64,
}

/// Whether a batch reached its tab, after a restart.
pub(crate) enum CommentsArrived {
    Yes,
    No,
    Unknown,
}

impl Core {
    /// The tab a session's typing goes to (its agent's, else its first
    /// shell) and its holder's instance, for the review's comment batches
    /// (SM-9). A holder that can't answer in time gives `holder: 0`.
    pub(crate) async fn comments_target(&self, session: &str) -> Result<CommentsTarget, Status> {
        let tab = self.typing_tab(session).await?;
        let asked = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::key_arrived(&tab.holder_socket, "instance"),
        )
        .await;
        let holder = match asked {
            Ok(Ok(KeyState::Arrived { holder } | KeyState::NotArrived { holder })) => holder,
            _ => 0,
        };
        Ok(CommentsTarget {
            tab: tab.id,
            holder,
        })
    }

    /// Types `text` into the target's tab, once for `key` when the holder
    /// can remember keys. Without that (`target.holder == 0`, or a holder
    /// that turns out not to know keyed input) it is plain typing, which
    /// cannot be told apart from a second try after a restart.
    pub(crate) async fn type_comments(
        &self,
        target: &CommentsTarget,
        key: &str,
        text: &[u8],
    ) -> CommentsTyped {
        let tab = match self.find_tab(&target.tab).await {
            Ok(Some(tab)) => tab,
            Ok(None) => return CommentsTyped::No(no_tab(&target.tab)),
            Err(status) => return CommentsTyped::No(status),
        };
        if target.holder == 0 {
            return self.type_plain(&tab, text).await;
        }
        let no_log = Path::new("");
        let typed = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::keyed_input(&tab.holder_socket, key, text),
        )
        .await;
        match typed {
            Ok(Ok(KeyedInput::Written | KeyedInput::Repeat)) => {
                CommentsTyped::Yes
            }
            Ok(Ok(KeyedInput::Unsupported)) => self.type_plain(&tab, text).await,
            // Failed before anything could be typed.
            Ok(Err(
                error @ (HolderError::Connect { .. }
                | HolderError::KeyTooLong { .. }
                | HolderError::KeyedInputTooLarge { .. }),
            )) => CommentsTyped::No(holder_failed("reach the tab's shell", &error, no_log)),
            // A refused keyed write may have been partly typed.
            Ok(Err(error)) => {
                CommentsTyped::Unknown(holder_failed("type the comments into the tab", &error, no_log))
            }
            Err(_) => CommentsTyped::Unknown(status::internal(
                format!(
                    "tab {}'s shell didn't take the comments within {} seconds; some of them may have been typed",
                    tab.id,
                    TYPE_TIMEOUT.as_secs()
                ),
                "",
            )),
        }
    }

    /// After a restart: whether the batch `key` reached the target's tab.
    /// Only the holder process that held the terminal when the batch was
    /// begun can say it never arrived: a holder started again for the same
    /// tab has an empty memory, so its "no" proves nothing and reads as
    /// `Unknown`.
    pub(crate) async fn comments_arrived(&self, target: &CommentsTarget, key: &str) -> CommentsArrived {
        if target.holder == 0 {
            return CommentsArrived::Unknown;
        }
        let tab = match self.find_tab(&target.tab).await {
            Ok(Some(tab)) => tab,
            _ => return CommentsArrived::Unknown,
        };
        let asked = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::key_arrived(&tab.holder_socket, key),
        )
        .await;
        match asked {
            Ok(Ok(KeyState::Arrived { holder })) if holder == target.holder => CommentsArrived::Yes,
            Ok(Ok(KeyState::NotArrived { holder })) if holder == target.holder => CommentsArrived::No,
            _ => CommentsArrived::Unknown,
        }
    }

    /// Plain typing through the tab's holder link.
    async fn type_plain(&self, tab: &TabRecord, text: &[u8]) -> CommentsTyped {
        let no_log = Path::new("");
        let link = match self.holders.link(&tab.id, &tab.holder_socket).await {
            Ok(link) => link,
            Err(error) => {
                return CommentsTyped::No(holder_failed("reach the tab's shell", &error, no_log));
            }
        };
        match tokio::time::timeout(TYPE_TIMEOUT, link.type_text(text)).await {
            Ok(Ok(())) => CommentsTyped::Yes,
            Ok(Err(TypeFailed::Exited)) => CommentsTyped::No(status::not_found(
                pb::EntityKind::Tab,
                &tab.id,
                format!("tab {}'s shell exited before the comments reached it", tab.id),
                "start the session's agent again, then send the comments",
            )),
            Ok(Err(failed @ TypeFailed::Refused { .. })) => {
                CommentsTyped::No(status::internal(status::chain(&failed), ""))
            }
            Ok(Err(TypeFailed::Lost(error))) => {
                CommentsTyped::Unknown(holder_failed("type the comments into the tab", &error, no_log))
            }
            Err(_) => {
                // A late answer must not reach the next call.
                self.holders.forget(&tab.id);
                CommentsTyped::Unknown(status::internal(
                    format!(
                        "tab {}'s shell didn't take the comments within {} seconds; some of them may have been typed",
                        tab.id,
                        TYPE_TIMEOUT.as_secs()
                    ),
                    "",
                ))
            }
        }
    }
}

impl Core {
    async fn find_session(&self, id: &str) -> Result<Option<SessionRecord>, Status> {
        let store = self.config.store.clone();
        let key = id.to_owned();
        match blocking(move || store.session(&key)).await? {
            Ok(found) => Ok(found),
            Err(StoreError::InvalidId { .. }) => Ok(None),
            Err(error) => Err(status::store(
                error,
                &self.config.machine,
                "look up the session",
            )),
        }
    }

    async fn session(&self, id: &str) -> Result<SessionRecord, Status> {
        self.find_session(id).await?.ok_or_else(|| no_session(id))
    }

    async fn find_tab(&self, id: &str) -> Result<Option<TabRecord>, Status> {
        let store = self.config.store.clone();
        let key = id.to_owned();
        match blocking(move || store.tab(&key)).await? {
            Ok(found) => Ok(found),
            Err(StoreError::InvalidId { .. }) => Ok(None),
            Err(error) => Err(status::store(
                error,
                &self.config.machine,
                "look up the tab",
            )),
        }
    }

    async fn tab(&self, id: &str, session: Option<&str>) -> Result<TabRecord, Status> {
        match self.find_tab(id).await? {
            Some(tab) if session.is_none_or(|session| tab.session.as_deref() == Some(session)) => {
                Ok(tab)
            }
            _ => Err(no_tab(id)),
        }
    }

    async fn tabs(&self) -> Result<Vec<TabRecord>, Status> {
        let store = self.config.store.clone();
        blocking(move || store.tabs())
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "list the tabs"))
    }

    /// What a call on `record` is checked against.
    fn session_target(&self, record: &SessionRecord) -> Target {
        Target::Session(identity::session_ref(record, &self.config.machine))
    }

    /// What typing into `tab` is checked against: its session, or for a
    /// machine shell (or a tab whose session is gone) the whole machine.
    async fn tab_target(&self, tab: &TabRecord) -> Result<Target, Status> {
        let Some(session) = &tab.session else {
            return Ok(Target::Global);
        };
        Ok(match self.find_session(session).await? {
            Some(record) => self.session_target(&record),
            None => Target::Global,
        })
    }

    /// Checks that `caller` may type into `target`: the user only.
    fn check_input(&self, caller: &Caller, target: Target) -> Result<(), Status> {
        self.config.guard.require(
            caller,
            &Call::Use {
                capability: Capability::TerminalInput,
                target,
            },
        )
    }

    /// `Some` when the rule for typing into `target` says Ask; an `approved`
    /// call goes on.
    fn gate_input(
        &self,
        caller: &Caller,
        target: Target,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        let call = Call::Use {
            capability: Capability::TerminalInput,
            target,
        };
        gate(&self.config.guard, caller, &call, approved)
    }

    /// The tab text typed into a session goes to: its agent's, else its
    /// first shell.
    async fn typing_tab(&self, session: &str) -> Result<TabRecord, Status> {
        let mut tabs: Vec<TabRecord> = self
            .tabs()
            .await?
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(session))
            .collect();
        tabs.sort_by_key(|tab| (tab.kind != TabKind::Agent, tab.created_unix_ms));
        tabs.into_iter().next().ok_or_else(|| {
            status::not_found(
                pb::EntityKind::Tab,
                "",
                format!("session {session} has no open tab to type into"),
                "open a shell tab in the session, or start its agent, then type again",
            )
        })
    }

    /// A tab as the API shows it, with the offsets its holder link last
    /// heard.
    async fn describe(&self, tab: &TabRecord) -> pb::Tab {
        let link = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.holders.link(&tab.id, &tab.holder_socket),
        )
        .await;
        let (earliest_offset, end_offset) = match link {
            Ok(Ok(link)) => {
                let offsets = link.offsets();
                (offsets.earliest, offsets.end)
            }
            _ => (0, 0),
        };
        pb::Tab {
            id: Some(pb::TabId {
                value: tab.id.clone(),
            }),
            session: tab.session.clone().map(|value| pb::SessionId { value }),
            kind: match tab.kind {
                TabKind::Agent => pb::tab::Kind::Agent,
                TabKind::Shell => pb::tab::Kind::Shell,
            } as i32,
            title: tab.title.clone(),
            end_offset,
            earliest_offset,
            machine: tab.session.is_none().then(|| pb::MachineId {
                value: self.config.machine.clone(),
            }),
        }
    }

    /// Starts a shell in `cwd` under a holder of its own and saves its tab.
    /// A session's shell gets a session token of its own, so what runs in
    /// it is that session's agent to the daemon.
    async fn open_shell(
        &self,
        session: Option<&SessionRecord>,
        cwd: PathBuf,
    ) -> Result<pb::Tab, Status> {
        let config = &self.config;
        let id = format!("tb_{}", uuid::Uuid::new_v4().simple());
        let socket = config.holders.join(format!("{id}.sock"));
        let log = config.logs.join(format!("{id}.log"));
        let mut env = Vec::new();
        if let Some(record) = session {
            let identities = config.identities.clone();
            let (tab, record) = (id.clone(), record.clone());
            let token = blocking(move || identities.mint(&tab, &record))
                .await?
                .map_err(|error| {
                    status::store(error, &config.machine, "save the tab's session token")
                })?;
            env.push((
                OsString::from(humata2_mcp::TOKEN_VAR),
                OsString::from(token),
            ));
        }
        let spec = HolderSpec {
            holder: config.holder.clone(),
            socket: socket.clone(),
            program: config.shell.clone(),
            args: config.shell_args.clone(),
            cwd,
            env,
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
            scrollback_bytes: config.scrollback_bytes,
            log: log.clone(),
        };
        let spawned = match blocking(move || humata2_holder::spawn(&spec)).await? {
            Ok(spawned) => spawned,
            Err(error) => {
                self.revoke(&id).await;
                return Err(holder_failed("start the tab's shell", &error, &log));
            }
        };
        let tab = TabRecord {
            id,
            session: session.map(|record| record.id.clone()),
            kind: TabKind::Shell,
            title: config
                .shell
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "shell".to_owned()),
            holder_socket: socket,
            holder_pid: spawned.pid,
            created_unix_ms: unix_ms(),
        };
        let store = config.store.clone();
        let saved = tab.clone();
        if let Err(error) = blocking(move || store.put_tab(&saved)).await? {
            // Nothing could find the shell again, so it doesn't stay.
            let _ = humata2_holder::shutdown(&tab.holder_socket).await;
            self.revoke(&tab.id).await;
            return Err(status::store(error, &config.machine, "save the new tab"));
        }
        Ok(self.describe(&tab).await)
    }

    /// Ends `tab`'s session token. A failure to save that is left for the
    /// next start, which drops tokens whose tab is gone.
    async fn revoke(&self, tab: &str) {
        let identities = self.config.identities.clone();
        let tab = tab.to_owned();
        let _ = blocking(move || identities.revoke(&tab)).await;
    }

    /// Types `text` into `tab`'s terminal through its holder link and
    /// waits until all of it is there.
    async fn type_into(&self, tab: &TabRecord, text: &[u8]) -> Result<(), Status> {
        let no_log = Path::new("");
        let link = self
            .holders
            .link(&tab.id, &tab.holder_socket)
            .await
            .map_err(|error| holder_failed("reach the tab's shell", &error, no_log))?;
        let typed = match tokio::time::timeout(TYPE_TIMEOUT, link.type_text(text)).await {
            Ok(typed) => typed,
            Err(_) => {
                // A late answer must not reach the next call: it gets a
                // connection of its own.
                self.holders.forget(&tab.id);
                return Err(status::internal(
                    format!(
                        "tab {}'s shell didn't take the text within {} seconds; some of it may have been typed",
                        tab.id,
                        TYPE_TIMEOUT.as_secs()
                    ),
                    "",
                ));
            }
        };
        typed.map_err(|failed| match failed {
            TypeFailed::Lost(error) => holder_failed("type into the tab", &error, no_log),
            TypeFailed::Refused { .. } => status::internal(status::chain(&failed), ""),
            TypeFailed::Exited => status::not_found(
                pb::EntityKind::Tab,
                &tab.id,
                format!("tab {}'s shell exited before the text reached it", tab.id),
                "open a new tab in the session and type again",
            ),
        })
    }

    fn recall(&self, key: Option<&str>) -> Option<Outcome> {
        let key = key?;
        lock(&self.remembered).results.get(key).cloned()
    }

    fn remember(&self, key: Option<String>, outcome: Outcome) {
        let Some(key) = key else { return };
        let mut remembered = lock(&self.remembered);
        if remembered.results.insert(key.clone(), outcome).is_none() {
            remembered.order.push_back(key);
        }
        while remembered.order.len() > REMEMBERED_MAX {
            if let Some(oldest) = remembered.order.pop_front() {
                remembered.results.remove(&oldest);
            }
        }
    }

    async fn tabs_listing(
        &self,
        caller: &Caller,
        request: pb::ListTabsRequest,
        approved: bool,
    ) -> Result<Gated<pb::ListTabsResponse>, Status> {
        let session = id_of(request.session.map(|session| session.value));
        match &session {
            Some(session) => {
                let record = self.session(session).await?;
                let read = Call::ReadSession(identity::session_ref(&record, &self.config.machine));
                if let Some(asked) = gate(&self.config.guard, caller, &read, approved)? {
                    return Ok(Gated::Ask(asked));
                }
            }
            None => {
                if let Some(asked) = self.gate_input(caller, Target::Global, approved)? {
                    return Ok(Gated::Ask(asked));
                }
            }
        }
        let mut tabs = Vec::new();
        for tab in self.tabs().await? {
            if tab.session == session {
                tabs.push(self.describe(&tab).await);
            }
        }
        Ok(Gated::Done(pb::ListTabsResponse { tabs }))
    }

    async fn shell_in_session(
        &self,
        caller: &Caller,
        request: pb::OpenShellTabRequest,
        approved: bool,
    ) -> Result<Gated<pb::Tab>, Status> {
        let key = idempotency("OpenShellTab", request.idempotency_key);
        let session = id_of(request.session.map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let session = self.session(&session).await?;
        if let Some(asked) = self.gate_input(caller, self.session_target(&session), approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(Outcome::Tab(tab)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(tab));
        }
        if request.repo.is_some() {
            return Err(status::invalid_field(
                "repo",
                Problem::NotAllowedHere,
                "starting a shell in one of the session's clones isn't built yet",
                "leave repo unset to start in the session folder, then cd into the clone",
            ));
        }
        let tab = self
            .open_shell(Some(&session), session.folder.clone())
            .await?;
        self.remember(key, Outcome::Tab(tab.clone()));
        Ok(Gated::Done(tab))
    }

    async fn shell_on_machine(
        &self,
        caller: &Caller,
        request: pb::OpenMachineShellRequest,
        approved: bool,
    ) -> Result<Gated<pb::Tab>, Status> {
        if let Some(asked) = self.gate_input(caller, Target::Global, approved)? {
            return Ok(Gated::Ask(asked));
        }
        let key = idempotency("OpenMachineShell", request.idempotency_key);
        if let Some(Outcome::Tab(tab)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(tab));
        }
        if let Some(machine) = id_of(request.machine.map(|machine| machine.value))
            && machine != self.config.machine
        {
            return Err(status::not_found(
                pb::EntityKind::Machine,
                &machine,
                format!(
                    "machine {machine} isn't this one, and this daemon can't open shells elsewhere yet"
                ),
                "open the shell through that machine's own daemon",
            ));
        }
        let tab = self.open_shell(None, self.config.root.clone()).await?;
        self.remember(key, Outcome::Tab(tab.clone()));
        Ok(Gated::Done(tab))
    }

    async fn close_one(
        &self,
        caller: &Caller,
        request: pb::CloseTabRequest,
        approved: bool,
    ) -> Result<Gated<pb::CloseTabResponse>, Status> {
        let key = idempotency("CloseTab", request.idempotency_key);
        let id = id_of(request.tab.map(|tab| tab.value)).ok_or_else(|| status::missing("tab"))?;
        let target = match self.find_tab(&id).await? {
            Some(tab) => self.tab_target(&tab).await?,
            None => Target::Global,
        };
        if let Some(asked) = self.gate_input(caller, target, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(Outcome::Closed) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(pb::CloseTabResponse {}));
        }
        let session = id_of(request.session.map(|session| session.value));
        let tab = self.tab(&id, session.as_deref()).await?;
        if tab.kind == TabKind::Agent {
            return Err(status::invalid_field(
                "tab",
                Problem::NotAllowedHere,
                format!("tab {id} is the session's agent, which can't be closed on its own"),
                "stop the session instead; that ends the agent",
            ));
        }
        match humata2_holder::shutdown(&tab.holder_socket).await {
            // A holder that's gone already has nothing left to stop.
            Ok(()) | Err(HolderError::Connect { .. }) => {}
            Err(error) => {
                return Err(holder_failed("stop the tab's shell", &error, Path::new("")));
            }
        }
        let store = self.config.store.clone();
        blocking(move || store.remove_tab(&id))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "forget the closed tab"))?;
        self.writers.forget(&tab.id);
        self.holders.forget(&tab.id);
        self.revoke(&tab.id).await;
        self.remember(key, Outcome::Closed);
        Ok(Gated::Done(pb::CloseTabResponse {}))
    }

    async fn type_in(
        &self,
        caller: &Caller,
        request: pb::TypeRequest,
        approved: bool,
    ) -> Result<Gated<pb::TypeResponse>, Status> {
        let key = idempotency("Type", request.idempotency_key);
        let session = id_of(request.session.map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let session = self.session(&session).await?;
        if let Some(asked) = gate(
            &self.config.guard,
            caller,
            &Call::Use {
                capability: Capability::SessionType,
                target: self.session_target(&session),
            },
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(Outcome::Typed(typed)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(typed));
        }
        if request.when == pb::type_request::When::TurnEnds as i32 {
            return Err(status::not_built(
                "humata.v1.TerminalService.Type with when TURN_ENDS",
            ));
        }
        let tab = match id_of(request.tab.map(|tab| tab.value)) {
            Some(tab) => self.tab(&tab, Some(&session.id)).await?,
            None => self.typing_tab(&session.id).await?,
        };
        self.type_into(&tab, request.text.as_bytes()).await?;
        let typed = pb::TypeResponse {
            queued: None,
            typed: true,
            turn_end_uncertain: false,
        };
        self.remember(key, Outcome::Typed(typed.clone()));
        Ok(Gated::Done(typed))
    }

    async fn paste_in(
        &self,
        caller: &Caller,
        request: pb::PasteImageRequest,
        approved: bool,
    ) -> Result<Gated<pb::PasteImageResponse>, Status> {
        let key = idempotency("PasteImage", request.idempotency_key);
        let session = id_of(request.session.map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let record = self.session(&session).await?;
        if let Some(asked) = self.gate_input(caller, self.session_target(&record), approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(Outcome::Paste(pasted)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(pasted));
        }
        let extension = paste::check(request.format, &request.data)?;
        let size_bytes = request.data.len() as u64;
        let store = self.config.store.clone();
        let folder = session.clone();
        let saved = blocking(move || store.save_paste(&folder, extension, &request.data)).await?;
        let path = match saved {
            Ok(path) => path,
            Err(StoreError::NoSession { .. } | StoreError::InvalidId { .. }) => {
                return Err(no_session(&session));
            }
            Err(error) => {
                return Err(status::store(
                    error,
                    &self.config.machine,
                    "save the pasted image",
                ));
            }
        };
        let Some(path) = path.to_str().map(str::to_owned) else {
            return Err(status::internal(
                format!(
                    "the pasted image was saved at {}, a path that isn't UTF-8",
                    path.display()
                ),
                "",
            ));
        };
        let pasted = pb::PasteImageResponse { path, size_bytes };
        self.remember(key, Outcome::Paste(pasted.clone()));
        Ok(Gated::Done(pasted))
    }
}

#[tonic::async_trait]
impl pb::terminal_service_server::TerminalService for Terminals {
    /// The tabs of `session`, or with no session, this machine's shells.
    async fn list_tabs(
        &self,
        request: Request<pb::ListTabsRequest>,
    ) -> Result<Response<pb::ListTabsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = None;
        let first = self.tabs_listing(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.ListTabsResponse",
            first,
            move || async move { core.tabs_listing(&who, request, true).await },
        )
        .await
    }

    async fn open_shell_tab(
        &self,
        request: Request<pb::OpenShellTabRequest>,
    ) -> Result<Response<pb::Tab>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.shell_in_session(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.Tab",
            first,
            move || async move { core.shell_in_session(&who, request, true).await },
        )
        .await
    }

    async fn open_machine_shell(
        &self,
        request: Request<pb::OpenMachineShellRequest>,
    ) -> Result<Response<pb::Tab>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.shell_on_machine(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.Tab",
            first,
            move || async move { core.shell_on_machine(&who, request, true).await },
        )
        .await
    }

    async fn close_tab(
        &self,
        request: Request<pb::CloseTabRequest>,
    ) -> Result<Response<pb::CloseTabResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.close_one(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.CloseTabResponse",
            first,
            move || async move { core.close_one(&who, request, true).await },
        )
        .await
    }

    type AttachStream = AttachStream;

    async fn attach(
        &self,
        request: Request<Streaming<pb::AttachClientMessage>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let caller = identity::caller(&request)?;
        let mut incoming = request.into_inner();
        let open = match incoming.message().await? {
            Some(pb::AttachClientMessage {
                message: Some(pb::attach_client_message::Message::Open(open)),
            }) => open,
            _ => {
                return Err(status::invalid_field(
                    "open",
                    Problem::Missing,
                    "an Attach must start with Open",
                    "send AttachOpen as the first message, naming the tab",
                ));
            }
        };
        let id = id_of(open.tab.clone().map(|tab| tab.value))
            .ok_or_else(|| status::missing("open.tab"))?;
        let session = id_of(open.session.clone().map(|session| session.value));
        let tab = self.tab(&id, session.as_deref()).await?;
        let target = self.tab_target(&tab).await?;
        match (&target, open.read_only) {
            (Target::Session(session), true) => {
                // A stream can't be held and replayed once approved, so an Ask stays refused here.
                self.config
                    .guard
                    .require(&caller, &Call::ReadSession(session.clone()))?;
            }
            // Same: a stream can't wait in the Inbox, so Ask stays refused.
            _ => self.check_input(&caller, target)?,
        }
        let codec = self
            .config
            .compression
            .pick(Link::Local, &open.accept_codecs);
        let stream = attach::start(&tab, &open, incoming, &self.writers, codec).await?;
        Ok(Response::new(stream))
    }

    /// Types text into a session's tab, as if typed at its keyboard: raw,
    /// with no bracketed paste around it.
    async fn r#type(
        &self,
        request: Request<pb::TypeRequest>,
    ) -> Result<Response<pb::TypeResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.type_in(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.TypeResponse",
            first,
            move || async move { core.type_in(&who, request, true).await },
        )
        .await
    }

    async fn cancel_queued_typing(
        &self,
        _request: Request<pb::CancelQueuedTypingRequest>,
    ) -> Result<Response<pb::CancelQueuedTypingResponse>, Status> {
        Err(status::not_built(
            "humata.v1.TerminalService.CancelQueuedTyping",
        ))
    }

    async fn list_queued(
        &self,
        _request: Request<pb::ListQueuedRequest>,
    ) -> Result<Response<pb::ListQueuedResponse>, Status> {
        Err(status::not_built("humata.v1.TerminalService.ListQueued"))
    }

    async fn send_queued_now(
        &self,
        request: Request<pb::SendQueuedNowRequest>,
    ) -> Result<Response<pb::SendQueuedNowResponse>, Status> {
        // Not built: nothing to run once approved, so Ask stays refused.
        self.check_input(&identity::caller(&request)?, Target::Global)?;
        Err(status::not_built("humata.v1.TerminalService.SendQueuedNow"))
    }

    async fn paste_image(
        &self,
        request: Request<pb::PasteImageRequest>,
    ) -> Result<Response<pb::PasteImageResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.paste_in(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.PasteImageResponse",
            first,
            move || async move { core.paste_in(&who, request, true).await },
        )
        .await
    }
}

/// What a mutation returned, for a retry with the same key.
#[derive(Clone)]
enum Outcome {
    Tab(pb::Tab),
    Closed,
    Paste(pb::PasteImageResponse),
    Typed(pb::TypeResponse),
}

#[derive(Default)]
struct Remembered {
    results: HashMap<String, Outcome>,
    order: VecDeque<String>,
}

/// How many attaches that can type each tab has (SE-24). Attaches watch
/// their tab's count to say how many others there are.
#[derive(Default)]
pub(crate) struct Writers {
    tabs: Mutex<HashMap<String, watch::Sender<u32>>>,
}

impl Writers {
    fn sender(&self, tab: &str) -> watch::Sender<u32> {
        lock(&self.tabs)
            .entry(tab.to_owned())
            .or_insert_with(|| watch::channel(0).0)
            .clone()
    }

    pub(crate) fn watch(&self, tab: &str) -> watch::Receiver<u32> {
        self.sender(tab).subscribe()
    }

    /// Counts an attach that can type until the guard is dropped.
    pub(crate) fn join(&self, tab: &str) -> WriterGuard {
        let sender = self.sender(tab);
        sender.send_modify(|count| *count += 1);
        WriterGuard { sender }
    }

    fn forget(&self, tab: &str) {
        lock(&self.tabs).remove(tab);
    }
}

pub(crate) struct WriterGuard {
    sender: watch::Sender<u32>,
}

impl Drop for WriterGuard {
    fn drop(&mut self) {
        self.sender
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs file work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))
}

/// An id field's value, or `None` when it's unset or empty.
fn id_of(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn idempotency(method: &str, key: Option<pb::IdempotencyKey>) -> Option<String> {
    id_of(key.map(|key| key.value)).map(|key| format!("{method}/{key}"))
}

fn no_session(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Session,
        id,
        format!("no session {id} is on this machine"),
        "check the session id; it may have been removed or be on another machine",
    )
}

pub(crate) fn no_tab(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Tab,
        id,
        format!("no tab {id} is open on this machine"),
        "list the session's tabs for the ones still open",
    )
}

fn holder_failed(doing: &str, error: &HolderError, log: &Path) -> Status {
    status::internal(
        format!("couldn't {doing}: {}", status::chain(error)),
        &log.display().to_string(),
    )
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
