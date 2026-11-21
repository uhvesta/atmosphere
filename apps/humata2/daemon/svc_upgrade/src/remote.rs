//! Carrying a Migrate out on another machine (UP-1, D281 f, D302): the host
//! stages the release for that machine's platform over the SSH path that
//! installed it, then calls the Upgrade of its daemon over the peer link.
//! The daemon swaps the staged files in and re-executes; the item ends when
//! the link comes back at the new release (`items::Watch`), or goes back to
//! pending as Failed if it hasn't within [`DEADLINE`].
//!
//! `Routed` picks who carries a Migrate: this daemon's own `Runner` for this
//! machine, `Remote` for any other.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use humata2_daemon_ports::PortError;
use crate::pb;
use humata2_machines::{
    BuildsError, LoginError, ManageError, Platform, RemoteStageError, StageError, StagedRelease,
    SystemRunner, existing_link, stage_next_remote,
};
use tonic::{Request, Status};

use super::items::{Carry, DAEMON_THERE, Progress};
use super::run::{RunError, new_id};
use crate::ports::RemoteHostPort;

/// How long a machine has to be seen at the new release after its daemon was
/// told to upgrade.
const DEADLINE: Duration = Duration::from_secs(60);

/// How long the machine's daemon has to answer the Upgrade call.
const CALL_DEADLINE: Duration = Duration::from_secs(30);

/// Why a machine other than this one couldn't be sent its release.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(
        "this daemon is not the host that installed the other machines, so it can't migrate {machine}. Click Migrate in the Humata app on the Mac that runs the host"
    )]
    NotHost { machine: String },

    #[error("{machine} isn't in this Mac's machine list, so it can't be migrated. Add it again, then click Migrate")]
    Unknown {
        machine: String,
        #[source]
        source: ManageError,
    },

    #[error(
        "this Mac hasn't read {machine}'s platform (it has {os:?} {arch:?}), so it can't tell which release to send. Use Reconnect now on the machine's page, then click Migrate"
    )]
    Platform { machine: String, os: String, arch: String },

    #[error(
        "{machine} has no SSH path to stage the release over, so it can't be migrated from here. Reconnect it from its page so Humata can reach it over SSH, then click Migrate"
    )]
    NoSshPath {
        machine: String,
        #[source]
        source: LoginError,
    },

    #[error("this Humata has no release for {machine}'s platform to send. Reinstall Humata, then click Migrate")]
    NoRelease {
        machine: String,
        #[source]
        source: Option<BuildsError>,
    },

    #[error("the release for {machine} is damaged. Reinstall Humata, then click Migrate")]
    Release {
        machine: String,
        #[source]
        source: StageError,
    },

    #[error("couldn't stage the release on {machine}; nothing running there was changed")]
    Stage {
        machine: String,
        #[source]
        source: RemoteStageError,
    },

    #[error(
        "this Mac's tools are release {staged}, not the {wanted} the Migrate item names, so nothing was swapped on {machine}. Wait for the machine to show up in the Inbox again, then click Migrate"
    )]
    OtherRelease {
        machine: String,
        staged: String,
        wanted: String,
    },

    #[error("{machine} has no live link to call its daemon over. Wait until it shows online or use Reconnect now, then click Migrate; the release stays staged")]
    Offline {
        machine: String,
        #[source]
        source: PortError,
    },

    #[error(
        "{machine}'s daemon didn't answer the upgrade within {} seconds. Look at ~/.humata2/logs on {machine}, then click Migrate again",
        CALL_DEADLINE.as_secs()
    )]
    Slow { machine: String },

    #[error("{machine}'s daemon refused the upgrade. Fix what it says, then click Migrate again")]
    Refused {
        machine: String,
        #[source]
        source: Status,
    },

    #[error("the work that stages a release on {machine} didn't finish. Click Migrate again")]
    Task {
        machine: String,
        #[source]
        source: tokio::task::JoinError,
    },
}

/// What `Remote` needs of the host: staging a machine's release and calling
/// its daemon's Upgrade.
#[tonic::async_trait]
pub trait Reach: Send + Sync {
    /// Stages the release for `machine`'s platform on it; the bundle id.
    async fn stage(&self, machine: &str) -> Result<String, RemoteError>;

    /// Calls `machine`'s daemon's Upgrade of `thing` to `to_bundle`.
    async fn upgrade(
        &self,
        machine: &str,
        to_bundle: &str,
        upgrade_id: &str,
        thing: pb::MigrateThing,
    ) -> Result<(), RemoteError>;
}

/// Carries a Migrate out on a machine other than this one.
pub struct Remote {
    reach: Arc<dyn Reach>,
}

impl Remote {
    pub fn new(reach: Arc<dyn Reach>) -> Self {
        Self { reach }
    }

    /// Over the host's SSH links and peer links; `host` is set once the
    /// host's machine work exists (never, on a machine's own daemon).
    pub fn of_host(host: Arc<OnceLock<Arc<dyn RemoteHostPort>>>) -> Self {
        Self::new(Arc::new(HostReach { host }))
    }
}

#[tonic::async_trait]
impl Carry for Remote {
    async fn migrate(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        to: &pb::BuildInfo,
        progress: &Progress,
    ) -> Result<(), RunError> {
        if thing == pb::MigrateThing::Holders {
            // Nothing is staged or swapped: the machine restarts its own agents.
            // A link that drops during the call is no failure: the item stays
            // Migrating and the machine's report on its next link settles it (D378).
            return match self
                .reach
                .upgrade(&machine.value, &to.bundle, &new_id(), thing)
                .await
            {
                Err(RemoteError::Refused { source, .. }) if link_dropped(&source) => Ok(()),
                Err(RemoteError::Slow { .. }) => Ok(()),
                other => other.map_err(RunError::from),
            };
        }
        let staged = self.reach.stage(&machine.value).await?;
        if staged != to.bundle {
            return Err(RemoteError::OtherRelease {
                machine: machine.value.clone(),
                staged,
                wanted: to.bundle.clone(),
            }
            .into());
        }
        // The release is on the machine; its daemon swaps it in, then restarts.
        let daemon = thing == pb::MigrateThing::Daemon;
        if daemon {
            progress.at(1).await;
        }
        self.reach.upgrade(&machine.value, &staged, &new_id(), thing).await?;
        if daemon {
            progress.at(2).await;
        }
        Ok(())
    }

    fn steps(&self, thing: pb::MigrateThing, _machine: &pb::MachineId) -> &'static [&'static str] {
        match thing {
            pb::MigrateThing::Daemon => &DAEMON_THERE,
            pb::MigrateThing::Cli | pb::MigrateThing::HostTools => &["Swap files"],
            _ => &[],
        }
    }

    fn deadline(&self, _machine: &pb::MachineId) -> Option<Duration> {
        Some(DEADLINE)
    }
}

/// Sends a Migrate of this machine to `here` and any other to `there`.
pub struct Routed {
    machine: String,
    here: Arc<dyn Carry>,
    there: Arc<dyn Carry>,
}

impl Routed {
    /// `here` carries migrations of the machine `machine`, `there` the rest.
    pub fn new(machine: &str, here: Arc<dyn Carry>, there: Arc<dyn Carry>) -> Self {
        Self {
            machine: machine.to_owned(),
            here,
            there,
        }
    }

    fn pick(&self, machine: &pb::MachineId) -> &Arc<dyn Carry> {
        if machine.value == self.machine { &self.here } else { &self.there }
    }
}

#[tonic::async_trait]
impl Carry for Routed {
    async fn migrate(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        to: &pb::BuildInfo,
        progress: &Progress,
    ) -> Result<(), RunError> {
        self.pick(machine).migrate(thing, machine, to, progress).await
    }

    fn steps(&self, thing: pb::MigrateThing, machine: &pb::MachineId) -> &'static [&'static str] {
        self.pick(machine).steps(thing, machine)
    }

    fn deadline(&self, machine: &pb::MachineId) -> Option<Duration> {
        self.pick(machine).deadline(machine)
    }
}

struct HostReach {
    host: Arc<OnceLock<Arc<dyn RemoteHostPort>>>,
}

impl HostReach {
    fn work(&self, machine: &str) -> Result<Arc<dyn RemoteHostPort>, RemoteError> {
        self.host.get().cloned().ok_or_else(|| RemoteError::NotHost {
            machine: machine.to_owned(),
        })
    }
}

#[tonic::async_trait]
impl Reach for HostReach {
    async fn stage(&self, machine: &str) -> Result<String, RemoteError> {
        let work = self.work(machine)?;
        let name = machine.to_owned();
        tokio::task::spawn_blocking(move || stage_blocking(work.as_ref(), &name))
            .await
            .map_err(|source| RemoteError::Task {
                machine: machine.to_owned(),
                source,
            })?
    }

    async fn upgrade(
        &self,
        machine: &str,
        to_bundle: &str,
        upgrade_id: &str,
        thing: pb::MigrateThing,
    ) -> Result<(), RemoteError> {
        let work = self.work(machine)?;
        let channel = work.channel(machine).map_err(|source| RemoteError::Offline {
            machine: machine.to_owned(),
            source,
        })?;
        call_upgrade(channel, machine, to_bundle, upgrade_id, thing).await
    }
}

/// Calls the Upgrade of `machine`'s daemon over `channel`.
async fn call_upgrade(
    channel: tonic::transport::Channel,
    machine: &str,
    to_bundle: &str,
    upgrade_id: &str,
    thing: pb::MigrateThing,
) -> Result<(), RemoteError> {
    let mut client = pb::daemon_service_client::DaemonServiceClient::new(channel);
    let call = client.upgrade(Request::new(pb::UpgradeRequest {
        to_bundle: to_bundle.to_owned(),
        upgrade_id: upgrade_id.to_owned(),
        thing: thing as i32,
    }));
    match tokio::time::timeout(CALL_DEADLINE, call).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(source)) => Err(RemoteError::Refused {
            machine: machine.to_owned(),
            source,
        }),
        Err(_) => Err(RemoteError::Slow {
            machine: machine.to_owned(),
        }),
    }
}

/// Asks `machine`'s daemon again, over a link that just came up, to restart
/// its agents onto the current holder (D378). The ask is idempotent. A
/// refusal is logged: the item stays Migrating, and the next link asks again.
pub(crate) async fn ask_holders(channel: &tonic::transport::Channel, machine: &str, to_bundle: &str) {
    if let Err(error) = call_upgrade(
        channel.clone(),
        machine,
        to_bundle,
        &new_id(),
        pb::MigrateThing::Holders,
    )
    .await
    {
        humata2_rpc_kit::log::say!("humatad2: warning: {}", humata2_rpc_kit::helpers::chain(&error));
    }
}

/// Whether `status` is what a call gets when the link under it ends, rather
/// than an answer from the machine's daemon.
fn link_dropped(status: &Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::Unavailable | tonic::Code::Cancelled | tonic::Code::Unknown | tonic::Code::Aborted
    )
}

/// Stages `machine`'s release over the SSH link that installed it. Blocks.
fn stage_blocking(work: &dyn RemoteHostPort, machine: &str) -> Result<String, RemoteError> {
    let entry = work.machines().get(machine).map_err(|source| RemoteError::Unknown {
        machine: machine.to_owned(),
        source,
    })?;
    let platform = platform_of(&entry.os, &entry.arch).ok_or_else(|| RemoteError::Platform {
        machine: machine.to_owned(),
        os: entry.os.clone(),
        arch: entry.arch.clone(),
    })?;
    let link = existing_link(work.machines(), machine).map_err(|source| RemoteError::NoSshPath {
        machine: machine.to_owned(),
        source,
    })?;
    let no_release = |source| RemoteError::NoRelease {
        machine: machine.to_owned(),
        source,
    };
    let files = work.builds().payloads(platform).map_err(|source| no_release(Some(source)))?;
    let bundle_json = work
        .builds()
        .bundle_json(platform)
        .map_err(|source| no_release(Some(source)))?
        .ok_or_else(|| no_release(None))?;
    let release = StagedRelease::new(platform, bundle_json.to_vec(), files).map_err(|source| RemoteError::Release {
        machine: machine.to_owned(),
        source,
    })?;
    stage_next_remote(&link, &SystemRunner::new(&()), &release).map_err(|source| RemoteError::Stage {
        machine: machine.to_owned(),
        source,
    })
}

/// The shipped platform a machine's recorded os and arch name.
fn platform_of(os: &str, arch: &str) -> Option<Platform> {
    let os = match os.to_ascii_lowercase().as_str() {
        "macos" => "darwin".to_owned(),
        other => other.to_owned(),
    };
    let arch = match arch.to_ascii_lowercase().as_str() {
        "amd64" => "x86_64".to_owned(),
        "arm64" => "aarch64".to_owned(),
        other => other.to_owned(),
    };
    Platform::SHIPPED
        .into_iter()
        .find(|platform| platform.os.to_string() == os && platform.arch.to_string() == arch)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Says who was asked to migrate what.
    #[derive(Default)]
    struct Marked {
        asked: Mutex<Vec<String>>,
    }

    #[tonic::async_trait]
    impl Carry for Marked {
        async fn migrate(
            &self,
            _thing: pb::MigrateThing,
            machine: &pb::MachineId,
            _to: &pb::BuildInfo,
            _progress: &Progress,
        ) -> Result<(), RunError> {
            self.asked.lock().unwrap().push(machine.value.clone());
            Ok(())
        }

        fn deadline(&self, _machine: &pb::MachineId) -> Option<Duration> {
            Some(Duration::from_secs(7))
        }
    }

    fn id(name: &str) -> pb::MachineId {
        pb::MachineId { value: name.to_owned() }
    }

    fn release(bundle: &str) -> pb::BuildInfo {
        pb::BuildInfo {
            bundle: bundle.to_owned(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn this_machine_goes_to_the_local_carry_and_any_other_to_the_remote_one() {
        let (here, there) = (Arc::new(Marked::default()), Arc::new(Marked::default()));
        let routed = Routed::new("ma_host", here.clone(), there.clone());

        routed.migrate(pb::MigrateThing::Daemon, &id("ma_host"), &release("b"), &Progress::silent()).await.unwrap();
        routed.migrate(pb::MigrateThing::Daemon, &id("gpu"), &release("b"), &Progress::silent()).await.unwrap();

        assert_eq!(*here.asked.lock().unwrap(), ["ma_host"]);
        assert_eq!(*there.asked.lock().unwrap(), ["gpu"]);
    }

    #[test]
    fn only_a_remote_migration_has_a_deadline() {
        let routed = Routed::new("ma_host", Arc::new(Marked::default()), Arc::new(Remote::new(Arc::new(Fake::default()))));

        assert_eq!(routed.deadline(&id("gpu")), Some(DEADLINE));
        assert_eq!(routed.deadline(&id("ma_host")), Some(Duration::from_secs(7)));
    }

    /// Stages `staged` and records the Upgrade calls.
    #[derive(Default)]
    struct Fake {
        staged: String,
        upgrades: Mutex<Vec<(String, String, String)>>,
    }

    #[tonic::async_trait]
    impl Reach for Fake {
        async fn stage(&self, _machine: &str) -> Result<String, RemoteError> {
            Ok(self.staged.clone())
        }

        async fn upgrade(
            &self,
            machine: &str,
            to_bundle: &str,
            upgrade_id: &str,
            _thing: pb::MigrateThing,
        ) -> Result<(), RemoteError> {
            self.upgrades
                .lock()
                .unwrap()
                .push((machine.to_owned(), to_bundle.to_owned(), upgrade_id.to_owned()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_staged_release_is_what_the_machines_daemon_is_told_to_upgrade_to() {
        let fake = Arc::new(Fake {
            staged: "bbbb".to_owned(),
            ..Default::default()
        });

        Remote::new(fake.clone()).migrate(pb::MigrateThing::Daemon, &id("gpu"), &release("bbbb"), &Progress::silent()).await.unwrap();

        let upgrades = fake.upgrades.lock().unwrap();
        assert_eq!(upgrades.len(), 1);
        assert_eq!((upgrades[0].0.as_str(), upgrades[0].1.as_str()), ("gpu", "bbbb"));
        assert!(!upgrades[0].2.is_empty(), "the upgrade has an id");
    }

    #[tokio::test]
    async fn tools_of_another_release_than_the_item_names_are_not_swapped_in() {
        let fake = Arc::new(Fake {
            staged: "cccc".to_owned(),
            ..Default::default()
        });

        let refused = Remote::new(fake.clone()).migrate(pb::MigrateThing::Daemon, &id("gpu"), &release("bbbb"), &Progress::silent()).await;

        assert!(matches!(refused, Err(RunError::Remote(RemoteError::OtherRelease { .. }))));
        assert!(fake.upgrades.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_daemon_that_is_not_the_host_cannot_migrate_another_machine() {
        let remote = Remote::of_host(Arc::new(OnceLock::new()));

        let refused = remote.migrate(pb::MigrateThing::Daemon, &id("gpu"), &release("bbbb"), &Progress::silent()).await;

        assert!(matches!(refused, Err(RunError::Remote(RemoteError::NotHost { .. }))));
    }

    #[test]
    fn a_platform_is_read_from_the_recorded_os_and_arch_in_either_spelling() {
        let name = |os, arch| platform_of(os, arch).map(Platform::bundle_name);

        assert_eq!(name("linux", "x86_64"), Some("linux-x86_64"));
        assert_eq!(name("macos", "arm64"), Some("darwin-arm64"));
        assert_eq!(name("", ""), None);
    }
}
