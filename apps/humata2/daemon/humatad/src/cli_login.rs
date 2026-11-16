//! CliAuthService: `humata login` and `logout` (SEC-10, SEC-11, SEC-12,
//! BOUNDARIES.md §2.2). A login binds the terminal session that ran it and
//! is confirmed in the app; the CLI then sends its token on every call, and
//! a call carrying it is the present user only from a process still in
//! that terminal session. Logins live in memory: a daemon restart ends
//! them all, so nothing on disk lets another process of this user forge
//! one.
//!
//! The service also makes, lists and revokes script tokens (SEC-14), which
//! only the user may do; the tokens themselves live in `script_tokens`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{PresenceMethod, Refusal, Tier};
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::invalid_field::Problem;
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};

use crate::grant_target::capability_of;
use crate::identity::{self, Caller, UserVia};
use crate::script_tokens::ScriptTokens;
use crate::status;
use crate::terminal_session::{self, Processes, TerminalSession, TerminalSessionError};

/// The longest a login lasts (SEC-10).
const HOURS_MAX: u32 = 12;

/// How long a login waits for its answer in the app.
const CONFIRM_WAIT: Duration = Duration::from_secs(5 * 60);

/// Why a call's CLI login doesn't make it the user.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoginUnusable {
    #[error(
        "its CLI login isn't one this daemon gave out: it was logged out, or the daemon restarted, which ends every login"
    )]
    Unknown,
    #[error("its CLI login has expired")]
    Expired,
    #[error("its CLI login covers another terminal session, and only the one it was made in")]
    OtherTerminal {
        #[source]
        source: Option<TerminalSessionError>,
    },
    #[error(
        "the daemon couldn't tell which process made the call, so it can't check the CLI login"
    )]
    NoProcess {
        #[source]
        source: Option<TerminalSessionError>,
    },
}

/// The logins this daemon gave out and those waiting for the app's answer.
pub(crate) struct CliLogins {
    processes: Box<dyn Processes>,
    /// How long every login lasts instead of the hours it asked for: a
    /// test daemon's `--cli-login-seconds`. Release builds pass `None`.
    lifetime: Option<Duration>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, Pending>,
    /// By the hash of the login's token.
    logins: HashMap<String, Login>,
}

struct Pending {
    shown: pb::PendingCliLogin,
    answer: oneshot::Sender<bool>,
}

struct Login {
    id: String,
    /// How the user showed they were there.
    method: PresenceMethod,
    bound: TerminalSession,
    expires: Instant,
    /// As the API shows it, without the token.
    shown: pb::CliSession,
}

/// A login waiting for its answer. Dropping it takes the question back,
/// as when the CLI gives up waiting.
struct Asked<'a> {
    logins: &'a CliLogins,
    id: String,
}

impl Drop for Asked<'_> {
    fn drop(&mut self) {
        lock(&self.logins.state).pending.remove(&self.id);
    }
}

impl CliLogins {
    pub(crate) fn new(processes: Box<dyn Processes>, lifetime: Option<Duration>) -> Self {
        Self {
            processes,
            lifetime,
            state: Mutex::new(State::default()),
        }
    }

    /// The login `token` names, with how it was made, if `pid` is still in
    /// the terminal session it bound (SEC-11). An expired login is
    /// forgotten here.
    pub(crate) fn caller(
        &self,
        token: &str,
        pid: Option<i32>,
    ) -> Result<(String, PresenceMethod), LoginUnusable> {
        let hash = identity::digest(token);
        let (id, method, bound) = {
            let mut state = lock(&self.state);
            let Some(login) = state.logins.get(&hash) else {
                return Err(LoginUnusable::Unknown);
            };
            if login.expires <= Instant::now() {
                state.logins.remove(&hash);
                return Err(LoginUnusable::Expired);
            }
            (login.id.clone(), login.method, login.bound.clone())
        };
        let pid = pid.ok_or(LoginUnusable::NoProcess { source: None })?;
        match terminal_session::terminal_session(&*self.processes, pid) {
            Ok(found) if found == bound => Ok((id, method)),
            Ok(_) => Err(LoginUnusable::OtherTerminal { source: None }),
            Err(error @ TerminalSessionError::Read { .. }) => Err(LoginUnusable::NoProcess {
                source: Some(error),
            }),
            Err(error) => Err(LoginUnusable::OtherTerminal {
                source: Some(error),
            }),
        }
    }

    /// Puts a login up for the app's answer.
    fn ask(&self, terminal: String, hours: u32) -> (Asked<'_>, oneshot::Receiver<bool>) {
        let id = format!("cl_{}", uuid::Uuid::new_v4().simple());
        let (answer, answered) = oneshot::channel();
        let shown = pb::PendingCliLogin {
            id: id.clone(),
            terminal,
            hours,
            asked_at: Some(timestamp(SystemTime::now())),
        };
        lock(&self.state)
            .pending
            .insert(id.clone(), Pending { shown, answer });
        (Asked { logins: self, id }, answered)
    }

    fn pending(&self) -> Vec<pb::PendingCliLogin> {
        let mut pending: Vec<_> = lock(&self.state)
            .pending
            .values()
            .map(|pending| pending.shown.clone())
            .collect();
        pending.sort_by_key(|login| login.asked_at.as_ref().map(|at| (at.seconds, at.nanos)));
        pending
    }

    /// Answers the waiting login `id`; false when there's none.
    fn answer(&self, id: &str, allow: bool) -> bool {
        match lock(&self.state).pending.remove(id) {
            // The login stopped waiting just now if this fails: nothing to do.
            Some(pending) => {
                let _ = pending.answer.send(allow);
                true
            }
            None => false,
        }
    }

    /// Makes the login, and returns it with its token.
    fn grant(
        &self,
        bound: TerminalSession,
        terminal: String,
        hours: u32,
        machine: &str,
        method: PresenceMethod,
    ) -> pb::CliSession {
        let lasts = self
            .lifetime
            .unwrap_or(Duration::from_secs(u64::from(hours) * 60 * 60));
        let (token, hash) = identity::new_token();
        let id = format!("cl_{}", uuid::Uuid::new_v4().simple());
        let shown = pb::CliSession {
            id: id.clone(),
            machine: Some(pb::MachineId {
                value: machine.to_owned(),
            }),
            terminal,
            expires_at: Some(timestamp(SystemTime::now() + lasts)),
            token: String::new(),
        };
        let now = Instant::now();
        let mut state = lock(&self.state);
        state.logins.retain(|_, login| login.expires > now);
        state.logins.insert(
            hash,
            Login {
                id,
                method,
                bound,
                expires: now + lasts,
                shown: shown.clone(),
            },
        );
        pb::CliSession { token, ..shown }
    }

    fn list(&self) -> Vec<pb::CliSession> {
        let now = Instant::now();
        let mut state = lock(&self.state);
        state.logins.retain(|_, login| login.expires > now);
        let mut logins: Vec<_> = state
            .logins
            .values()
            .map(|login| login.shown.clone())
            .collect();
        logins.sort_by(|a, b| a.id.cmp(&b.id));
        logins
    }

    /// Ends login `id`; false when there's none.
    fn end(&self, id: &str) -> bool {
        let mut state = lock(&self.state);
        let before = state.logins.len();
        state.logins.retain(|_, login| login.id != id);
        state.logins.len() != before
    }
}

/// The service, over the logins and what it needs to place a caller.
pub(crate) struct CliAuth {
    pub(crate) logins: Arc<CliLogins>,
    pub(crate) scripts: Arc<ScriptTokens>,
    /// Where the session tabs are, to refuse a login from inside one.
    pub(crate) store: Arc<Store>,
    pub(crate) machine: String,
    pub(crate) role: pb::DaemonRole,
    /// Whether the host is reachable: the passphrase logs in only while it
    /// isn't.
    pub(crate) host: Arc<crate::away::HostLink>,
}

impl CliAuth {
    /// The terminal session `pid` is in, refusing one inside a session
    /// tab's terminal (SEC-12).
    async fn locate(&self, pid: i32) -> Result<(TerminalSession, String), Status> {
        let logins = self.logins.clone();
        let store = self.store.clone();
        let machine = self.machine.clone();
        tokio::task::spawn_blocking(move || {
            let processes = &*logins.processes;
            let session = match terminal_session::terminal_session(processes, pid) {
                Ok(session) => session,
                Err(error @ TerminalSessionError::Read { .. }) => {
                    return Err(status::internal(
                        format!(
                            "couldn't place the process that ran humata login: {}",
                            status::chain(&error)
                        ),
                        "",
                    ));
                }
                Err(error) => return Err(status::login_needs_terminal(&error)),
            };
            let tabs = store
                .tabs()
                .map_err(|error| status::store(error, &machine, "list the session tabs"))?;
            let holders: HashSet<i32> = tabs
                .iter()
                .filter(|tab| tab.session.is_some())
                .filter_map(|tab| i32::try_from(tab.holder_pid).ok())
                .collect();
            if terminal_session::descends_from(processes, pid, &session, &holders) {
                return Err(status::in_session_terminal());
            }
            let terminal = terminal_session::describe(processes, &session);
            Ok((session, terminal))
        })
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
    }
}

impl CliAuth {
    /// Checks the approval passphrase a machine's terminal typed while the
    /// host is away (SEC-13, AW-5): refuses unless it is right and the
    /// prompt isn't locked.
    async fn passphrase_login(&self, proof: Option<pb::presence_proof::Proof>) -> Result<(), Status> {
        use pb::presence_proof::Proof;
        if self.host.reachable() {
            return Err(status::login_host_reachable());
        }
        let store = self.store.clone();
        let held = tokio::task::spawn_blocking(move || crate::away::verifier(&store))
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| status::store(error, &self.machine, "read the approval passphrase"))?;
        if held.is_empty() {
            return Err(status::login_no_passphrase());
        }
        let accepted = vec![PresenceMethod::ApprovalPassphrase];
        let typed = match proof {
            None => return Err(status::refused(&Refusal::PresenceRequired { accepted })),
            Some(Proof::ConfirmInApp(_)) => {
                return Err(status::not_built(
                    "humata.v1.CliAuthService.BeginCliSession confirmed in the app from a machine",
                ));
            }
            Some(Proof::ApprovalPassphrase(typed)) => typed,
        };
        let store = self.store.clone();
        let checked = tokio::task::spawn_blocking(move || {
            crate::away::check_passphrase(&store, &typed)
        })
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
        .map_err(|error| match error {
            crate::away::CheckError::NoVerifier => status::login_no_passphrase(),
            error => status::internal(status::chain(&error), ""),
        })?;
        match checked {
            crate::away::Checked::Accepted => Ok(()),
            crate::away::Checked::Wrong { .. } => {
                Err(status::refused(&Refusal::PresenceRequired { accepted }))
            }
            crate::away::Checked::Locked { until } => {
                Err(status::passphrase_locked(&self.machine, until))
            }
        }
    }
}

#[tonic::async_trait]
impl pb::cli_auth_service_server::CliAuthService for CliAuth {
    async fn begin_cli_session(
        &self,
        request: Request<pb::BeginCliSessionRequest>,
    ) -> Result<Response<pb::CliSession>, Status> {
        match identity::caller_if_any(&request)? {
            Some(Caller::Agent(_)) => return Err(status::in_session_terminal()),
            Some(Caller::Script(_)) => {
                return Err(status::refused(&Refusal::UserOnly { capability: None }));
            }
            Some(Caller::Peer) => return Err(status::refused(&Refusal::PeerCannotCall)),
            Some(Caller::User(_)) | None => {}
        }
        let Some(pid) = identity::peer_pid(&request) else {
            return Err(status::login_unusable(&LoginUnusable::NoProcess {
                source: None,
            }));
        };
        let request = request.into_inner();
        let hours = match request.hours {
            0 => HOURS_MAX,
            hours if hours > HOURS_MAX => {
                return Err(status::invalid_field(
                    "hours",
                    Problem::OutOfRange,
                    format!("a login lasts at most {HOURS_MAX} hours, not {hours}"),
                    format!("ask for {HOURS_MAX} hours or fewer, then log in again when it ends"),
                ));
            }
            hours => hours,
        };
        let (bound, terminal) = self.locate(pid).await?;
        use pb::presence_proof::Proof;
        let proof = request.presence.and_then(|presence| presence.proof);
        if self.role == pb::DaemonRole::Machine {
            self.passphrase_login(proof).await?;
            return Ok(Response::new(self.logins.grant(
                bound,
                terminal,
                hours,
                &self.machine,
                PresenceMethod::ApprovalPassphrase,
            )));
        }
        match proof {
            Some(Proof::ConfirmInApp(_)) => {}
            // The passphrase is for machines while the host is away.
            None | Some(Proof::ApprovalPassphrase(_)) => {
                return Err(status::refused(&Refusal::PresenceRequired {
                    accepted: vec![PresenceMethod::ConfirmInApp],
                }));
            }
        }
        let (asked, answered) = self.logins.ask(terminal.clone(), hours);
        let answer = tokio::time::timeout(CONFIRM_WAIT, answered).await;
        drop(asked);
        match answer {
            Ok(Ok(true)) => Ok(Response::new(self.logins.grant(
                bound,
                terminal,
                hours,
                &self.machine,
                PresenceMethod::CliLogin,
            ))),
            // The question is gone only when the daemon is stopping.
            Ok(Ok(false) | Err(_)) => Err(status::login_denied()),
            Err(_) => Err(status::login_unanswered(CONFIRM_WAIT.as_secs() / 60)),
        }
    }

    /// An empty id ends the caller's own login.
    async fn end_cli_session(
        &self,
        request: Request<pb::EndCliSessionRequest>,
    ) -> Result<Response<pb::EndCliSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        let id = request.into_inner().id;
        let id = match (caller, id.is_empty()) {
            (Caller::User(UserVia::CliLogin { login, .. }), true) => login,
            (Caller::User(_), true) => return Err(status::missing("id")),
            (Caller::User(_), false) => id,
            (Caller::Agent(_) | Caller::Script(_), _) => {
                return Err(status::refused(&Refusal::UserOnly { capability: None }));
            }
            (Caller::Peer, _) => return Err(status::refused(&Refusal::PeerCannotCall)),
        };
        if !self.logins.end(&id) {
            return Err(no_login(&id));
        }
        Ok(Response::new(pb::EndCliSessionResponse {}))
    }

    async fn list_cli_sessions(
        &self,
        request: Request<pb::ListCliSessionsRequest>,
    ) -> Result<Response<pb::ListCliSessionsResponse>, Status> {
        match identity::caller(&request)? {
            Caller::User(_) => {}
            Caller::Agent(_) | Caller::Script(_) => {
                return Err(status::refused(&Refusal::UserOnly { capability: None }));
            }
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
        }
        Ok(Response::new(pb::ListCliSessionsResponse {
            sessions: self.logins.list(),
        }))
    }

    async fn list_pending_cli_logins(
        &self,
        request: Request<pb::ListPendingCliLoginsRequest>,
    ) -> Result<Response<pb::ListPendingCliLoginsResponse>, Status> {
        app_only(
            &identity::caller(&request)?,
            "see the logins waiting for an answer",
        )?;
        Ok(Response::new(pb::ListPendingCliLoginsResponse {
            logins: self.logins.pending(),
        }))
    }

    async fn answer_cli_login(
        &self,
        request: Request<pb::AnswerCliLoginRequest>,
    ) -> Result<Response<pb::AnswerCliLoginResponse>, Status> {
        app_only(&identity::caller(&request)?, "answer a login")?;
        let request = request.into_inner();
        if request.id.is_empty() {
            return Err(status::missing("id"));
        }
        if !self.logins.answer(&request.id, request.allow) {
            return Err(status::not_found(
                pb::EntityKind::Unspecified,
                &request.id,
                format!("no login {} is waiting for an answer", request.id),
                "it was answered already, or the terminal stopped waiting; list the waiting logins again",
            ));
        }
        Ok(Response::new(pb::AnswerCliLoginResponse {}))
    }

    async fn create_script_token(
        &self,
        request: Request<pb::CreateScriptTokenRequest>,
    ) -> Result<Response<pb::CreateScriptTokenResponse>, Status> {
        user_only(&identity::caller(&request)?)?;
        let request = request.into_inner();
        if request.name.trim().is_empty() {
            return Err(status::missing("name"));
        }
        if request.capabilities.is_empty() {
            return Err(status::invalid_field(
                "capabilities",
                Problem::Missing,
                "a script token needs at least one capability",
                "name the capabilities the script needs, such as channel.read, and send it again",
            ));
        }
        let mut capabilities = Vec::new();
        for value in &request.capabilities {
            let Some(capability) = capability_of(*value) else {
                return Err(status::invalid_field(
                    "capabilities",
                    Problem::UnknownValue,
                    format!("capability {value} isn't one this daemon knows"),
                    "list the capabilities with `humata access`, and name one of those",
                ));
            };
            if !matches!(
                capability.tier(),
                Tier::DefaultAllow | Tier::AskMe | Tier::NeverByDefault
            ) {
                return Err(status::invalid_field(
                    "capabilities",
                    Problem::NotAllowedHere,
                    format!(
                        "a script token can't hold {capability}: it can hold only what an agent could be allowed, never an action only the user may take"
                    ),
                    "remove it from the token; do that action yourself in the app or the CLI",
                ));
            }
            if !capabilities.contains(&capability) {
                capabilities.push(capability);
            }
        }
        let mut channels: Vec<String> = Vec::new();
        for channel in &request.channels {
            if channel.value.is_empty() {
                return Err(status::missing("channels"));
            }
            if channels.contains(&channel.value) {
                continue;
            }
            let store = self.store.clone();
            let id = channel.value.clone();
            let found = tokio::task::spawn_blocking(move || store.channel(&id))
                .await
                .map_err(|error| {
                    status::internal(format!("a background task failed: {error}"), "")
                })?
                .map_err(|error| {
                    status::store(error, &self.machine, "look up the token's channels")
                })?;
            if found.is_none() {
                return Err(status::not_found(
                    pb::EntityKind::Channel,
                    &channel.value,
                    format!("there is no channel {}", channel.value),
                    "list the channels, and name one of those; a token can cover only channels that exist",
                ));
            }
            channels.push(channel.value.clone());
        }
        let days = match request.days {
            0 => 30,
            days if days > 90 => {
                return Err(status::invalid_field(
                    "days",
                    Problem::OutOfRange,
                    format!("a script token lasts at most 90 days, not {days}"),
                    "ask for 90 days or fewer, then make a new token when it expires",
                ));
            }
            days => days,
        };
        let key = request
            .idempotency_key
            .map(|key| key.value)
            .filter(|value| !value.is_empty());
        let scripts = self.scripts.clone();
        let reply = tokio::task::spawn_blocking(move || {
            scripts.create(
                request.name.trim(),
                &capabilities,
                &channels,
                days,
                key.as_deref(),
            )
        })
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
        .map_err(|error| status::internal(status::chain(&error), ""))?;
        Ok(Response::new(reply))
    }

    async fn list_script_tokens(
        &self,
        request: Request<pb::ListScriptTokensRequest>,
    ) -> Result<Response<pb::ListScriptTokensResponse>, Status> {
        user_only(&identity::caller(&request)?)?;
        Ok(Response::new(pb::ListScriptTokensResponse {
            tokens: self.scripts.list(),
        }))
    }

    async fn revoke_script_token(
        &self,
        request: Request<pb::RevokeScriptTokenRequest>,
    ) -> Result<Response<pb::RevokeScriptTokenResponse>, Status> {
        user_only(&identity::caller(&request)?)?;
        let id = request.into_inner().token_id;
        if id.is_empty() {
            return Err(status::missing("token_id"));
        }
        let scripts = self.scripts.clone();
        let target = id.clone();
        let went = tokio::task::spawn_blocking(move || scripts.revoke(&target))
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| status::internal(status::chain(&error), ""))?;
        if !went {
            return Err(status::not_found(
                pb::EntityKind::Unspecified,
                &id,
                format!("no script token {id} exists"),
                "it was revoked already; list tokens with `humata token ls`",
            ));
        }
        Ok(Response::new(pb::RevokeScriptTokenResponse {}))
    }
}

/// Refuses everyone but the user, from the app or a CLI login.
fn user_only(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(_) => Ok(()),
        Caller::Agent(_) | Caller::Script(_) => {
            Err(status::refused(&Refusal::UserOnly { capability: None }))
        }
        Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
    }
}

/// Refuses everyone but the app.
fn app_only(caller: &Caller, what: &str) -> Result<(), Status> {
    match caller {
        Caller::User(UserVia::App) => Ok(()),
        Caller::User(UserVia::CliLogin { .. } | UserVia::Host) => Err(status::app_only(what)),
        Caller::Agent(_) | Caller::Script(_) => {
            Err(status::refused(&Refusal::UserOnly { capability: None }))
        }
        Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
    }
}

fn no_login(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Unspecified,
        id,
        format!("no CLI login {id} is live on this daemon"),
        "it has ended already: logged out, expired, or ended by a daemon restart. `humata logins` lists the live ones",
    )
}

fn timestamp(at: SystemTime) -> Timestamp {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_session::tests::{Fake, terminal};

    fn logins(fake: Fake, lifetime: Option<Duration>) -> CliLogins {
        CliLogins::new(Box::new(fake), lifetime)
    }

    /// A login for the session of `pid` in `logins`.
    fn log_in(logins: &CliLogins, pid: i32) -> pb::CliSession {
        let bound = terminal_session::terminal_session(&*logins.processes, pid).unwrap();
        logins.grant(bound, "tty7".to_owned(), 12, "ma_test", PresenceMethod::CliLogin)
    }

    #[test]
    fn a_login_covers_its_terminal_session() {
        let logins = logins(terminal(), None);
        let login = log_in(&logins, 21);

        let want = (login.id.clone(), PresenceMethod::CliLogin);
        assert_eq!(logins.caller(&login.token, Some(22)).unwrap(), want);
        assert_eq!(logins.caller(&login.token, Some(21)).unwrap(), want);
    }

    #[test]
    fn a_login_from_another_terminal_or_after_setsid_is_refused() {
        let mut fake = terminal();
        // Another terminal's shell.
        fake.add(50, 10, 50, Some(9));
        // A child of the command that called setsid.
        fake.add(30, 22, 30, None);
        let logins = logins(fake, None);
        let login = log_in(&logins, 21);

        for pid in [50, 30] {
            assert!(matches!(
                logins.caller(&login.token, Some(pid)),
                Err(LoginUnusable::OtherTerminal { .. })
            ));
        }
    }

    #[test]
    fn a_call_without_a_known_process_is_refused() {
        let logins = logins(terminal(), None);
        let login = log_in(&logins, 21);

        assert!(matches!(
            logins.caller(&login.token, None),
            Err(LoginUnusable::NoProcess { .. })
        ));
        assert!(matches!(
            logins.caller(&login.token, Some(999)),
            Err(LoginUnusable::NoProcess { .. })
        ));
    }

    #[test]
    fn an_expired_login_is_refused_and_forgotten() {
        let logins = logins(terminal(), Some(Duration::ZERO));
        let login = log_in(&logins, 21);

        assert!(matches!(
            logins.caller(&login.token, Some(22)),
            Err(LoginUnusable::Expired)
        ));
        assert!(matches!(
            logins.caller(&login.token, Some(22)),
            Err(LoginUnusable::Unknown)
        ));
        assert!(logins.list().is_empty());
    }

    #[test]
    fn an_ended_login_is_refused() {
        let logins = logins(terminal(), None);
        let login = log_in(&logins, 21);

        assert!(logins.end(&login.id));

        assert!(matches!(
            logins.caller(&login.token, Some(22)),
            Err(LoginUnusable::Unknown)
        ));
        assert!(!logins.end(&login.id));
    }

    #[test]
    fn listed_logins_never_carry_their_token() {
        let logins = logins(terminal(), None);
        let login = log_in(&logins, 21);

        let listed = logins.list();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, login.id);
        assert!(listed[0].token.is_empty());
        assert!(!login.token.is_empty());
    }

    #[test]
    fn a_made_up_token_is_unknown() {
        let logins = logins(terminal(), None);
        log_in(&logins, 21);

        assert!(matches!(
            logins.caller("0000", Some(22)),
            Err(LoginUnusable::Unknown)
        ));
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_login_and_takes_it_off_the_list() {
        let logins = logins(terminal(), None);
        let (asked, answered) = logins.ask("tty7".to_owned(), 12);
        let id = asked.id.clone();
        assert_eq!(logins.pending().len(), 1);

        assert!(logins.answer(&id, true));

        assert!(answered.await.unwrap());
        assert!(logins.pending().is_empty());
        assert!(!logins.answer(&id, true));
    }

    #[test]
    fn a_login_that_stops_waiting_leaves_the_list() {
        let logins = logins(terminal(), None);
        let (asked, _answered) = logins.ask("tty7".to_owned(), 12);
        let id = asked.id.clone();

        drop(asked);

        assert!(logins.pending().is_empty());
        assert!(!logins.answer(&id, true));
    }
}
