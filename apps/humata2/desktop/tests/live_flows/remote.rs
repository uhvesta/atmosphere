//! A host daemon and fake boxes in one test, for a session that runs on a
//! machine other than the one the app talks to. It is the daemon tests'
//! multi-machine rig (`humata2_remote_rig`): the app is the host's user, a
//! box's agent calls its own box's daemon with its session token, and
//! everything the box tells the person arrives over the host.

use std::path::PathBuf;

use humata2_clients::Clients;
use humata2_clients::model::SessionId;
use humata2_remote_rig::common::Daemon;
use humata2_remote_rig::pr_github::{FakeGh, Pr, seed_pull_requests};
use humata2_remote_rig::remote_rig::{Rig, is_live};

use super::leftovers::CaseClock;
use super::world::{WorldError, live_clients};

pub(super) struct RemoteWorld {
    rig: Rig,
    repos: Vec<(String, String)>,
    /// Dropped last, so the time it tells includes the rig's end.
    _clock: CaseClock,
}

/// The clock of a case on `rig`, begun at `started`, watching the host's
/// and each box's folders.
fn clock_of(rig: &Rig, boxes: &[&'static str], started: std::time::Instant) -> CaseClock {
    let mut dirs = vec![rig.host.dir.path(), rig.world.root.as_path()];
    dirs.extend(boxes.iter().map(|name| rig.the(name).daemon().dir.path()));
    let mut clock = CaseClock::start(started, &dirs);
    clock.up();
    clock
}

/// The host's fake `gh` and the pull requests of flow 26.17, with the git
/// remotes the host and every box clone from.
pub(super) struct Github {
    pub(super) gh: FakeGh,
    pub(super) prs: Vec<Pr>,
}

impl RemoteWorld {
    /// A host and one fake box per name in `boxes`, each linked, with a git
    /// remote for each of `repos` (`owner/name`). Every box runs the admin
    /// daemon, so a test reads a box's own state and calls it as its agent.
    pub(super) async fn start(boxes: &[&'static str], repos: &[(&str, &str)]) -> Self {
        let started = std::time::Instant::now();
        let rig = Rig::start_seen(boxes, repos).await;
        Self {
            _clock: clock_of(&rig, boxes, started),
            rig,
            repos: repos
                .iter()
                .map(|(owner, name)| ((*owner).to_owned(), (*name).to_owned()))
                .collect(),
        }
    }

    /// A host and one fake box per name in `boxes`, as [`Self::start`], with
    /// acme/api#482, acme/api#500 and acme/ledger#91 on remotes every machine
    /// clones from, and a fake `gh` on the host alone (`gh` and the fetch of
    /// a pull request's head run on the host; a box never has a token).
    /// `env` is more environment for the host's daemon.
    pub(super) async fn start_with_github(boxes: &[&'static str], env: &[(&str, &str)]) -> (Self, Github) {
        let started = std::time::Instant::now();
        let mut host_env = vec![("HUMATA2_GH", "{dir}/gh/gh"), ("HUMATA2_PR_CHECK_REUSE_MS", "0")];
        host_env.extend_from_slice(env);
        let rig = Rig::start_seen_with_host_env(boxes, &[], &host_env).await;
        let gh = FakeGh::new(rig.world.root.join("gh"));
        let prs = seed_pull_requests(&rig.world, &gh);
        (
            Self {
                _clock: clock_of(&rig, boxes, started),
                rig,
                repos: Vec::new(),
            },
            Github { gh, prs },
        )
    }

    /// The host's folder, where every file of the host and the shared remotes
    /// lives.
    pub(super) fn host_root(&self) -> PathBuf {
        self.rig.world.root.clone()
    }

    /// Box `name`'s own daemon, as the person on that box would ask it.
    pub(super) fn box_daemon(&self, name: &str) -> &Daemon {
        self.rig.the(name).daemon()
    }

    /// The host's socket: what the app connects to, as the host's user.
    pub(super) fn host_socket(&self) -> PathBuf {
        self.rig.host.socket.clone()
    }

    /// The app's clients on the host, with the real machines client.
    pub(super) async fn clients(&self) -> Result<Clients, WorldError> {
        live_clients(self.host_socket(), true).await
    }

    /// Box `name`'s own daemon socket, which its sessions' agents call.
    pub(super) fn box_socket(&self, name: &str) -> PathBuf {
        self.rig.the(name).daemon().socket.clone()
    }

    /// Starts a session titled `title` on box `name`, in a channel of the
    /// world's repos shared by the host and the box, and returns once its
    /// agent is up.
    pub(super) async fn running_session_on(&self, name: &str, title: &str) -> SessionId {
        let (host, one) = (self.rig.host_id().await, Rig::box_id(name));
        let repos: Vec<(&str, &str)> = self
            .repos
            .iter()
            .map(|(owner, name)| (owner.as_str(), name.as_str()))
            .collect();
        let channel = self.rig.create_channel("Payments", &repos, &[host, one.clone()]).await;
        let session = self.rig.start_on(&channel, &one, title).await;
        self.rig.wait_session(&session, "live", is_live).await;
        SessionId::new(session.value)
    }

    /// The session token the daemon of box `name` gave `session`'s agent.
    pub(super) async fn agent_token(&self, name: &str, session: &SessionId) -> String {
        let id = humata2_grpc::humata::v1::SessionId {
            value: session.to_string(),
        };
        self.rig.agent_token(name, &id).await
    }
}
