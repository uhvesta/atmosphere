//! How the machine's socket daemon recognises the peer link's connection
//! (D139, GRPC.md §3). The link process connects to the socket and proves
//! once, bound to the host's own ProveHost: it calls BeginRelay and gets a
//! fresh nonce, which it offers the host as the machine's Hello nonce;
//! when the host proves it holds the enrollment key over that nonce, the
//! link calls CompleteRelay with the host's nonce and proof exactly as
//! received. The daemon verifies the host's proof itself, over the nonce
//! it issued on that same connection, and marks the connection proven.
//! Calls carry no credential; there is nothing to replay on another
//! connection or after a pid is reused.
//!
//! CompleteRelay is honoured only when all of these hold, checked in this
//! order: the daemon's role is machine; the kernel named the peer's pid at
//! accept and its uid is the daemon's; the pid is outside every session's
//! process tree; a nonce was issued on this connection, unused and under
//! 30 seconds old (any attempt uses it up); and the enrollment key passes
//! its file checks and the host's proof verifies. On a proven connection
//! every call is the user's, via the host, if its method is on the relay's
//! allowlist and it carries no credential; anything else is refused. A
//! connection that never completes is an ordinary one.
//!
//! State is kept per connection, by the id the socket's accept gives it
//! (`ConnId`), and forgotten when the daemon restarts: the link's
//! connection drops with it, the link exits and the host redials.
//!
//! The risk is D139's, stated there: any uncontained process of the same
//! user can read the key, play the host to this daemon and make
//! allowlisted calls as the user without `humata login`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use tonic::body::Body;
use tonic::codegen::http;
use tonic::{Code, Status};

use crate::link_proof::{
    EnrollmentKey, HOST_NONCE_MAX, HOST_NONCE_MIN, KeyError, MACHINE_NONCE_BYTES, Side,
};
use crate::relay;
use crate::serve::Role;
use crate::status;

/// How long an issued nonce can be completed.
const NONCE_LIFE: Duration = Duration::from_secs(30);

/// The most connections kept in the table: an unproven entry older than
/// `NONCE_LIFE` is dropped first, then the oldest.
const TABLE_MAX: usize = 256;

/// One accepted connection of the socket. Minted by the accept loop
/// (`PeerInfo::conn`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ConnId(pub(crate) u64);

/// The method a request called, kept as the request's extension (the
/// authentication interceptor can't see the path). Only the server sets
/// it, so a client can't.
#[derive(Clone, Debug)]
pub(crate) struct MethodPath(pub(crate) String);

/// Records the method of a request for `RelayAuth::authorize`. Runs
/// before the authentication interceptor.
pub(crate) fn mark_method(mut request: http::Request<Body>) -> http::Request<Body> {
    let path = request.uri().path().to_owned();
    request.extensions_mut().insert(MethodPath(path));
    request
}

/// What is known about the process at the other end of the socket. The one
/// place to add checks of the link process (its executable, its ancestry):
/// add a field here and a step to `check_peer`, and no caller changes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PeerFacts {
    /// The pid the kernel reported for the socket's peer, if it did.
    pub(crate) pid: Option<i32>,
    /// The user the kernel reported for the socket's peer, if it did.
    pub(crate) uid: Option<u32>,
}

/// A call that came from the host through the link process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FromHost;

/// Why a relay call was refused. Each says what failed; `status` says what
/// to do.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Refusal {
    #[error("this daemon's role is {role:?}: only a machine's daemon takes the peer link's connection")]
    WrongRole { role: Role },

    #[error("the kernel didn't say which process or user is at the other end of this connection")]
    UnknownPeer,

    #[error("the process at the other end runs as user {uid}, not as the user running humatad2")]
    OtherUser { uid: u32 },

    #[error("process {pid} is inside a session's terminal, so it isn't the link process")]
    InsideASession { pid: i32 },

    #[error("no BeginRelay nonce is waiting on this connection: none was issued here, or it was already used")]
    NoNonce,

    #[error("the BeginRelay nonce on this connection is {age_secs}s old; it is good for {life_secs}s")]
    NonceExpired { age_secs: u64, life_secs: u64 },

    #[error("the host's nonce is {len} bytes; the machine takes {HOST_NONCE_MIN} to {HOST_NONCE_MAX}")]
    HostNonceLength { len: usize },

    #[error("this machine has no usable enrollment key, so the host's proof can't be checked")]
    NoKey {
        #[source]
        source: KeyError,
    },

    #[error("the host's proof doesn't match this connection's nonce and this machine's enrollment key")]
    BadProof,

    #[error("a call on the peer link's connection carried a credential, which the link never sends")]
    CarriedCredential,

    #[error("{method} isn't a call the peer link relays from the host")]
    OffList { method: String },
}

impl Refusal {
    /// The refusal as the status the caller gets.
    pub(crate) fn status(&self) -> Status {
        let (code, fix) = match self {
            Refusal::WrongRole { .. } => (
                Code::PermissionDenied,
                "calls from the host go to the machine's own daemon through the peer link; connect from the Mac to this machine instead".to_owned(),
            ),
            Refusal::OffList { .. } => (
                Code::PermissionDenied,
                "do this from the Mac's own daemon, or on this machine with `humata`".to_owned(),
            ),
            Refusal::InsideASession { .. } | Refusal::OtherUser { .. } | Refusal::UnknownPeer => (
                Code::PermissionDenied,
                "only the peer link process the Mac started, outside every session's terminal, can do this; in a session's terminal use `humata login`".to_owned(),
            ),
            Refusal::CarriedCredential => (
                Code::PermissionDenied,
                "send these calls without a session token, login or script token; the link adds none".to_owned(),
            ),
            Refusal::NoNonce | Refusal::NonceExpired { .. } | Refusal::HostNonceLength { .. } => (
                Code::FailedPrecondition,
                "drop the peer link and connect to the machine again, which starts a new link and a new proof".to_owned(),
            ),
            Refusal::BadProof => (
                Code::Unauthenticated,
                "drop the link and connect to the machine again; if this repeats, the Mac and the machine hold different enrollment keys, so forget the machine in Humata on the Mac and add it again. Don't just re-pin or copy a key over".to_owned(),
            ),
            Refusal::NoKey { source } => (Code::Unauthenticated, source.fix().to_owned()),
        };
        status::failure(
            code,
            pb::Error {
                message: format!("humatad2 refused the call: {}", status::chain(self)),
                fix,
                retry: None,
                kind: None,
            },
        )
    }
}

/// One connection's state.
struct Entry {
    /// The nonce BeginRelay issued, and when.
    nonce: Option<([u8; MACHINE_NONCE_BYTES], Instant)>,
    /// CompleteRelay succeeded here.
    proven: bool,
    since: Instant,
}

/// The socket daemon's relay state and checks.
pub(crate) struct RelayAuth {
    role: Role,
    key_file: PathBuf,
    /// The daemon's own user.
    uid: u32,
    /// Whether a pid is inside any session's process tree. Fails closed.
    in_a_session: Arc<dyn Fn(i32) -> bool + Send + Sync>,
    connections: Mutex<HashMap<ConnId, Entry>>,
}

impl RelayAuth {
    /// `key_file` is the enrollment key in the state folder; `uid` the
    /// user running this daemon.
    pub(crate) fn new(
        role: Role,
        key_file: PathBuf,
        uid: u32,
        in_a_session: Arc<dyn Fn(i32) -> bool + Send + Sync>,
    ) -> Self {
        Self {
            role,
            key_file,
            uid,
            in_a_session,
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// BeginRelay: a fresh random nonce for `conn`, replacing any earlier
    /// one. Grants nothing by itself.
    pub(crate) fn begin(&self, conn: ConnId, now: Instant) -> [u8; MACHINE_NONCE_BYTES] {
        let mut nonce = [0u8; MACHINE_NONCE_BYTES];
        rand::fill(&mut nonce[..]);
        self.begin_with(conn, nonce, now);
        nonce
    }

    fn begin_with(&self, conn: ConnId, nonce: [u8; MACHINE_NONCE_BYTES], now: Instant) {
        let mut table = lock(&self.connections);
        if table.len() >= TABLE_MAX && !table.contains_key(&conn) {
            table.retain(|_, entry| entry.proven || now.duration_since(entry.since) < NONCE_LIFE);
            if table.len() >= TABLE_MAX
                && let Some(oldest) = table.iter().min_by_key(|(_, entry)| entry.since).map(|(id, _)| *id)
            {
                table.remove(&oldest);
            }
        }
        let entry = table.entry(conn).or_insert(Entry {
            nonce: None,
            proven: false,
            since: now,
        });
        entry.nonce = Some((nonce, now));
    }

    /// CompleteRelay: whether `host_nonce` and `proof` are the host's
    /// proof over `conn`'s nonce. On success the connection is the
    /// host's. The first failing check refuses.
    pub(crate) fn complete(
        &self,
        conn: ConnId,
        peer: &PeerFacts,
        host_nonce: &[u8],
        proof: &[u8],
        now: Instant,
    ) -> Result<(), Refusal> {
        if self.role != Role::Machine {
            return Err(Refusal::WrongRole { role: self.role });
        }
        let pid = self.check_peer(peer)?;
        if (self.in_a_session)(pid) {
            return Err(Refusal::InsideASession { pid });
        }
        // Any attempt past here uses the nonce up.
        let issued = lock(&self.connections)
            .get_mut(&conn)
            .and_then(|entry| entry.nonce.take());
        let (nonce, at) = issued.ok_or(Refusal::NoNonce)?;
        let age = now.saturating_duration_since(at);
        if age > NONCE_LIFE {
            return Err(Refusal::NonceExpired {
                age_secs: age.as_secs(),
                life_secs: NONCE_LIFE.as_secs(),
            });
        }
        if !(HOST_NONCE_MIN..=HOST_NONCE_MAX).contains(&host_nonce.len()) {
            return Err(Refusal::HostNonceLength {
                len: host_nonce.len(),
            });
        }
        let key = EnrollmentKey::read(&self.key_file).map_err(|source| Refusal::NoKey { source })?;
        if !key.verify(Side::Host, host_nonce, &nonce, proof) {
            return Err(Refusal::BadProof);
        }
        if let Some(entry) = lock(&self.connections).get_mut(&conn) {
            entry.proven = true;
        }
        Ok(())
    }

    /// The checks of the peer process that need no key: the kernel named
    /// its pid and its user is ours. Later checks of the link process go
    /// here.
    fn check_peer(&self, peer: &PeerFacts) -> Result<i32, Refusal> {
        let (Some(pid), Some(uid)) = (peer.pid, peer.uid) else {
            return Err(Refusal::UnknownPeer);
        };
        if uid != self.uid {
            return Err(Refusal::OtherUser { uid });
        }
        Ok(pid)
    }

    /// Whether the call to `method` on `conn` is from the host: `Ok(None)`
    /// for a connection that never completed (an ordinary one),
    /// `Ok(Some)` for an allowed call on a proven one, a refusal for
    /// anything else on a proven one. `carries_credential` says the
    /// request has a session token, login or script token.
    pub(crate) fn authorize(
        &self,
        conn: ConnId,
        method: &str,
        carries_credential: bool,
    ) -> Result<Option<FromHost>, Refusal> {
        let proven = lock(&self.connections)
            .get(&conn)
            .is_some_and(|entry| entry.proven);
        if !proven {
            return Ok(None);
        }
        if carries_credential {
            return Err(Refusal::CarriedCredential);
        }
        if !relay::allows(method) {
            return Err(Refusal::OffList {
                method: method.to_owned(),
            });
        }
        Ok(Some(FromHost))
    }

    /// Forgets `conn`, when its connection ends.
    #[allow(dead_code)]
    pub(crate) fn forget(&self, conn: ConnId) {
        lock(&self.connections).remove(&conn);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;
    use crate::link_proof::KEY_FILE;

    const LIST_SESSIONS: &str = "/humata.v1.SessionService/ListSessions";
    const UID: u32 = 501;
    const PID: i32 = 4242;
    const HOST_NONCE: [u8; 32] = [9; 32];

    fn write_key(dir: &Path, byte: u8) -> EnrollmentKey {
        let path = dir.join(KEY_FILE);
        std::fs::write(&path, format!("{}\n", format!("{byte:02x}").repeat(32))).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        EnrollmentKey::read(&path).unwrap()
    }

    fn auth_with(dir: &Path, role: Role, inside: impl Fn(i32) -> bool + Send + Sync + 'static) -> RelayAuth {
        RelayAuth::new(role, dir.join(KEY_FILE), UID, Arc::new(inside))
    }

    fn auth(dir: &Path) -> RelayAuth {
        auth_with(dir, Role::Machine, |_| false)
    }

    fn peer() -> PeerFacts {
        PeerFacts {
            pid: Some(PID),
            uid: Some(UID),
        }
    }

    /// The host's proof for `nonce`, as the host computes it.
    fn host_proof(key: &EnrollmentKey, nonce: &[u8]) -> Vec<u8> {
        key.proof(Side::Host, &HOST_NONCE, nonce)
    }

    const A: ConnId = ConnId(1);
    const B: ConnId = ConnId(2);

    #[test]
    fn a_valid_proof_makes_the_connection_the_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &nonce), now).unwrap();
        assert_eq!(auth.authorize(A, LIST_SESSIONS, false).unwrap(), Some(FromHost));
    }

    #[test]
    fn a_proof_made_for_one_connections_nonce_is_refused_on_another() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce_a = auth.begin(A, now);
        let nonce_b = auth.begin(B, now);
        assert_ne!(nonce_a, nonce_b);
        let refused = auth.complete(B, &peer(), &HOST_NONCE, &host_proof(&key, &nonce_a), now);
        assert!(matches!(refused, Err(Refusal::BadProof)));
        assert_eq!(auth.authorize(B, LIST_SESSIONS, false).unwrap(), None);
    }

    #[test]
    fn a_nonce_is_used_up_by_any_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let good = host_proof(&key, &nonce);
        assert!(matches!(
            auth.complete(A, &peer(), &HOST_NONCE, &[0; 32], now),
            Err(Refusal::BadProof)
        ));
        let again = auth.complete(A, &peer(), &HOST_NONCE, &good, now);
        assert!(matches!(again, Err(Refusal::NoNonce)));
    }

    #[test]
    fn a_proof_is_refused_once_it_has_been_used() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let proof = host_proof(&key, &nonce);
        auth.complete(A, &peer(), &HOST_NONCE, &proof, now).unwrap();
        let replay = auth.complete(A, &peer(), &HOST_NONCE, &proof, now);
        assert!(matches!(replay, Err(Refusal::NoNonce)));
    }

    #[test]
    fn a_new_begin_replaces_the_earlier_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let first = auth.begin(A, now);
        let second = auth.begin(A, now);
        let refused = auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &first), now);
        assert!(matches!(refused, Err(Refusal::BadProof)));
        auth.begin_with(A, second, now);
        assert!(auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &second), now).is_ok());
    }

    #[test]
    fn an_expired_nonce_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let late = now + NONCE_LIFE + Duration::from_secs(1);
        let refused = auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &nonce), late);
        assert!(matches!(refused, Err(Refusal::NonceExpired { .. })));
    }

    #[test]
    fn a_wrong_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_key(dir.path(), 0x5a);
        let elsewhere = tempfile::tempdir().unwrap();
        let other = write_key(elsewhere.path(), 0x6b);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let refused = auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&other, &nonce), now);
        assert!(matches!(refused, Err(Refusal::BadProof)));
    }

    #[test]
    fn complete_without_begin_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let refused = auth(dir.path()).complete(
            A,
            &peer(),
            &HOST_NONCE,
            &host_proof(&key, &[0; 32]),
            Instant::now(),
        );
        assert!(matches!(refused, Err(Refusal::NoNonce)));
    }

    #[test]
    fn a_machine_with_no_key_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let auth = auth(dir.path());
        let now = Instant::now();
        auth.begin(A, now);
        let refused = auth.complete(A, &peer(), &HOST_NONCE, &[0; 32], now);
        assert!(matches!(refused, Err(Refusal::NoKey { .. })));
    }

    #[test]
    fn a_pid_inside_a_session_is_refused_despite_a_valid_proof() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth_with(dir.path(), Role::Machine, |pid| pid == PID);
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let refused = auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &nonce), now);
        assert!(matches!(refused, Err(Refusal::InsideASession { pid: PID })));
        assert_eq!(auth.authorize(A, LIST_SESSIONS, false).unwrap(), None);
    }

    #[test]
    fn a_host_role_daemon_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth_with(dir.path(), Role::Host, |_| false);
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let refused = auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &nonce), now);
        assert!(matches!(refused, Err(Refusal::WrongRole { .. })));
    }

    #[test]
    fn an_unknown_or_foreign_peer_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let key = write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        let proof = host_proof(&key, &nonce);
        for facts in [
            PeerFacts { pid: None, uid: Some(UID) },
            PeerFacts { pid: Some(PID), uid: None },
        ] {
            assert!(matches!(
                auth.complete(A, &facts, &HOST_NONCE, &proof, now),
                Err(Refusal::UnknownPeer)
            ));
        }
        let foreign = PeerFacts { pid: Some(PID), uid: Some(UID + 1) };
        assert!(matches!(
            auth.complete(A, &foreign, &HOST_NONCE, &proof, now),
            Err(Refusal::OtherUser { .. })
        ));
    }

    #[test]
    fn a_host_nonce_of_the_wrong_length_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_key(dir.path(), 0x5a);
        let auth = auth(dir.path());
        let now = Instant::now();
        auth.begin(A, now);
        let refused = auth.complete(A, &peer(), &[1; 8], &[0; 32], now);
        assert!(matches!(refused, Err(Refusal::HostNonceLength { len: 8 })));
    }

    fn proven(dir: &Path) -> RelayAuth {
        let key = write_key(dir, 0x5a);
        let auth = auth(dir);
        let now = Instant::now();
        let nonce = auth.begin(A, now);
        auth.complete(A, &peer(), &HOST_NONCE, &host_proof(&key, &nonce), now).unwrap();
        auth
    }

    #[test]
    fn a_proven_connection_is_refused_everything_off_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let auth = proven(dir.path());
        for method in [
            "/humata.v1.AccessService/ListRules",
            "/humata.v1.InboxService/Answer",
            "/humata.v1.CliAuthService/BeginCliSession",
            "/humata.v1.DaemonService/WhoAmI",
            "/humata.v1.SessionService/DeleteSession",
            "",
        ] {
            assert!(matches!(auth.authorize(A, method, false), Err(Refusal::OffList { .. })), "{method}");
        }
    }

    #[test]
    fn a_proven_connection_is_refused_a_call_carrying_a_credential() {
        let dir = tempfile::tempdir().unwrap();
        let auth = proven(dir.path());
        assert!(matches!(
            auth.authorize(A, LIST_SESSIONS, true),
            Err(Refusal::CarriedCredential)
        ));
    }

    #[test]
    fn a_connection_that_never_completed_is_ordinary() {
        let dir = tempfile::tempdir().unwrap();
        let auth = proven(dir.path());
        for method in [LIST_SESSIONS, "/humata.v1.AccessService/ListRules"] {
            assert_eq!(auth.authorize(B, method, false).unwrap(), None);
            assert_eq!(auth.authorize(B, method, true).unwrap(), None);
        }
    }

    #[test]
    fn every_refusal_is_a_failed_status() {
        let refusals = [
            Refusal::WrongRole { role: Role::Host },
            Refusal::UnknownPeer,
            Refusal::OtherUser { uid: 1 },
            Refusal::InsideASession { pid: 1 },
            Refusal::NoNonce,
            Refusal::NonceExpired { age_secs: 31, life_secs: 30 },
            Refusal::HostNonceLength { len: 1 },
            Refusal::BadProof,
            Refusal::CarriedCredential,
            Refusal::OffList { method: "/x/y".to_owned() },
        ];
        for refusal in refusals {
            assert_ne!(refusal.status().code(), Code::Ok, "{refusal:?}");
        }
    }

    #[test]
    fn every_request_gets_its_method_marked() {
        let request = http::Request::builder()
            .uri(LIST_SESSIONS)
            .body(Body::empty())
            .unwrap();
        let marked = mark_method(request);
        assert_eq!(marked.extensions().get::<MethodPath>().unwrap().0, LIST_SESSIONS);
    }
}
