//! The VIP "Migrate <thing> on <machine>" Inbox items (UP-1, D281 b, D290):
//! one pending item per thing and machine, kept up to date while the
//! machine stays at another release and withdrawn as resolved once it is
//! at the host's again. Only the user answers one, and only by migrating:
//! that marks it Migrating, hands it to what carries the migration out
//! (`Carry`), and it ends as Approved ("Migrated to ...") when the machine
//! is seen at the new release, or stays pending as Failed with the error.
//! A CLI or host tools item has no restart to wait for: once `Carry` put the
//! file in place it is Approved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use tonic::Status;

use super::build::{Release, short};
use super::holders::shell_line;
use super::remote;
use super::run::RunError;
use super::tools;
use crate::attach;
use crate::inbox::Inbox;
use crate::status;

/// Why a Migrate item couldn't be posted, changed or withdrawn.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MigrateError {
    #[error(
        "couldn't show the Migrate item for {machine} in the Inbox; the machine stays read-only until it is migrated. Restart humatad2 on this Mac, or reconnect {machine}, to post it again"
    )]
    Post {
        machine: String,
        #[source]
        source: Status,
    },

    #[error(
        "couldn't update the Migrate item {request} for {machine}; it keeps showing what the machine was. Reconnect {machine} to update it"
    )]
    Update {
        machine: String,
        request: String,
        #[source]
        source: Status,
    },

    #[error(
        "couldn't withdraw the Migrate item {request} for {machine}, though it is at this Mac's release; it stays in the Inbox until this daemon restarts. Restart humatad2 on this Mac to clear it"
    )]
    Withdraw {
        machine: String,
        request: String,
        #[source]
        source: Status,
    },

    #[error(
        "couldn't mark the Migrate item {request} for {machine} as done, though it is at this Mac's release; it keeps showing Migrating until this daemon restarts. Restart humatad2 on this Mac to clear it"
    )]
    Settle {
        machine: String,
        request: String,
        #[source]
        source: Status,
    },
}

/// What carries a Migrate out. `migrate` returns once the machine is on its
/// way to `to`: the Migrate item is then finished when the machine is seen
/// at that release (a link-up at it, or this daemon's own start at it). An
/// error is why it can't get there; the item shows it and Migrate retries.
/// For a CLI or host tools item (`thing`) there is no way to wait for: Ok
/// means the file is in place.
#[tonic::async_trait]
pub(crate) trait Carry: Send + Sync {
    /// `progress` moves the card's steps along as the work goes (D379).
    async fn migrate(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        to: &pb::BuildInfo,
        progress: &Progress,
    ) -> Result<(), RunError>;

    /// The labels of the steps `migrate` goes through for `thing` on
    /// `machine`, in order (D379). None for a Holders item, which shows rows.
    fn steps(&self, thing: pb::MigrateThing, _machine: &pb::MachineId) -> &'static [&'static str] {
        match thing {
            pb::MigrateThing::Daemon => &DAEMON_HERE,
            pb::MigrateThing::Cli | pb::MigrateThing::HostTools => &TOOL,
            _ => &[],
        }
    }

    /// How long `machine` has, once `migrate` returned, to be seen at the new
    /// release before the item goes back to pending as Failed. None: no
    /// limit (this daemon's own restart ends its item by itself).
    fn deadline(&self, _machine: &pb::MachineId) -> Option<Duration> {
        None
    }
}

/// What carries the Desk folders Migrate out (D458): the Desk service, which
/// sets it once it is made. An error is why a folder couldn't be migrated;
/// the item shows it and Migrate retries.
#[tonic::async_trait]
pub(crate) trait FolderMigrator: Send + Sync {
    async fn migrate_folders(&self, machine: &pb::MachineId) -> Result<(), pb::Error>;
}

/// The steps of migrating the daemon of the machine this one runs on. The
/// store is copied by the new daemon as it starts, before it opens the
/// store, so "Copy the store" is under way from when the upgrade is recorded
/// until the new daemon is up.
pub(crate) const DAEMON_HERE: [&str; 5] = [
    "Check files",
    "Copy the store",
    "Swap files",
    "Restart",
    "Back at the new release",
];

/// The steps of migrating a machine's daemon from the host.
pub(crate) const DAEMON_THERE: [&str; 4] = ["Copy files over SSH", "Swap files", "Restart", "Link back"];

/// The steps of putting a CLI or host tools back.
const TOOL: [&str; 1] = ["Swap files"];

/// Moves the steps of one Migrate item along: each call is one pushed
/// update of the item.
pub(crate) struct Progress {
    /// The Migrations and id of the item to move; none moves nothing.
    item: Option<(Migrations, String)>,
}

impl Progress {
    /// A progress that moves no item.
    #[cfg(test)]
    pub(crate) fn silent() -> Self {
        Self { item: None }
    }

    /// Step `doing` is under way: the ones before it are done, the ones
    /// after it not begun.
    pub(crate) async fn at(&self, doing: usize) {
        let Some((migrations, id)) = &self.item else {
            return;
        };
        let updated = migrations
            .inner
            .inbox
            .update_migrate(id, |request| {
                if let Some(card) = card_mut(request) {
                    advance(card, doing);
                }
            })
            .await;
        if let Err(error) = updated {
            crate::log::say!(
                "humatad2: warning: couldn't show the Migrate item {} moving on, so its steps are behind until it ends: {}",
                id,
                status::chain(&error)
            );
        }
    }
}

/// Steps in a card: `doing` under way, the ones before it done, the ones
/// after it not begun.
fn advance(card: &mut pb::MigratePreview, doing: usize) {
    for (at, step) in card.steps.iter_mut().enumerate() {
        step.state = match at.cmp(&doing) {
            std::cmp::Ordering::Less => pb::MigrateStepState::Done,
            std::cmp::Ordering::Equal => pb::MigrateStepState::Doing,
            std::cmp::Ordering::Greater => pb::MigrateStepState::Pending,
        } as i32;
    }
}

/// The step under way (the first one not done when none is) failed.
fn fail_step(card: &mut pb::MigratePreview) {
    let at = card
        .steps
        .iter()
        .position(|step| step.state() == pb::MigrateStepState::Doing)
        .or_else(|| card.steps.iter().position(|step| step.state() != pb::MigrateStepState::Done));
    if let Some(step) = at.and_then(|at| card.steps.get_mut(at)) {
        step.state = pb::MigrateStepState::Failed as i32;
    }
}

/// Every step of `request`'s card is done: its Migrate ended.
pub(crate) fn finish_steps(request: &mut pb::InboxRequest) {
    if let Some(card) = card_mut(request) {
        for step in &mut card.steps {
            step.state = pb::MigrateStepState::Done as i32;
        }
    }
}

/// The steps `labels` name, the first under way.
fn fresh_steps(labels: &[&str]) -> Vec<pb::MigrateStep> {
    let mut card = pb::MigratePreview {
        steps: labels
            .iter()
            .map(|label| pb::MigrateStep {
                label: (*label).to_owned(),
                state: pb::MigrateStepState::Pending as i32,
            })
            .collect(),
        ..Default::default()
    };
    advance(&mut card, 0);
    card.steps
}

/// What the Inbox shows of one pending item, as the mismatch last looked.
#[derive(Clone)]
struct Pending {
    id: pb::RequestId,
    from: pb::BuildInfo,
    to: pb::BuildInfo,
    /// Migrate was clicked and the machine isn't at `to` yet.
    migrating: bool,
    /// What a Holders item lists (D373); none for any other.
    detail: Option<Detail>,
}

/// What a Holders item shows besides the release: the agent sessions that
/// wait for a new holder and what stays as it is (D373).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Detail {
    /// The agent sessions still on an old holder, sorted.
    pub(crate) waiting: Vec<String>,
    /// The lines of what keeps running as it is.
    pub(crate) keeps_running: Vec<String>,
    /// A row for each agent session and each shell on an old holder, as the
    /// card shows them (D379).
    pub(crate) rows: Vec<pb::HolderSession>,
    /// Why the machine's last restart of an agent failed, as it reported it
    /// (a machine's daemon only); the item shows Failed with it.
    pub(crate) failed: Option<pb::Error>,
}

struct Inner {
    inbox: Arc<Inbox>,
    carry: Arc<dyn Carry>,
    /// What migrates the Desk folders; set once by the Desk service (D458).
    folders: std::sync::OnceLock<Arc<dyn FolderMigrator>>,
    /// The pending item of each (thing, machine). In memory; the Inbox
    /// holds the items themselves.
    pending: Mutex<HashMap<(pb::MigrateThing, String), Pending>>,
    /// Held across a post, an update or a withdrawal, so two detections of
    /// one machine never post twice.
    serial: tokio::sync::Mutex<()>,
    /// What the items call a machine; set once at boot.
    names: std::sync::OnceLock<Names>,
    // SEAM (MACHINESCRIPTS D689): told the machine a Migrate finished on.
    migrated: std::sync::OnceLock<Arc<dyn Fn(&str) + Send + Sync>>,
}

/// What a machine is called in text a person reads (D500). Ids stay in the
/// structured fields. The host's own machine is "This Mac", as the app
/// labels it. Another machine's id is its name until machines get ids of
/// their own (D61), so it reads as its name; an id with no name known reads
/// as the id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Names {
    this_mac: Option<String>,
    named: HashMap<String, String>,
}

impl Names {
    /// The host's own machine, `machine`, is "This Mac".
    pub(crate) fn of_host(machine: &str) -> Self {
        Self {
            this_mac: Some(machine.to_owned()),
            named: HashMap::new(),
        }
    }

    /// `machine` is called `name`.
    #[cfg(test)]
    fn naming(mut self, machine: &str, name: &str) -> Self {
        self.named.insert(machine.to_owned(), name.to_owned());
        self
    }

    /// What `machine` is called: its name, "This Mac" for the host's own
    /// machine, and the id when neither is known.
    pub(crate) fn of(&self, machine: &str) -> String {
        if self.this_mac.as_deref() == Some(machine) {
            return "This Mac".to_owned();
        }
        match self.named.get(machine).filter(|name| !name.is_empty()) {
            Some(name) => name.clone(),
            None => machine.to_owned(),
        }
    }
}

/// The Migrate items of this host's Inbox.
#[derive(Clone)]
pub(crate) struct Migrations {
    inner: Arc<Inner>,
}

/// What the Inbox calls when Migrate is clicked on an item. It holds the
/// Migrations weakly, because they hold the Inbox.
pub(crate) struct Approver {
    inner: Weak<Inner>,
}

impl Approver {
    /// See [`Migrations::approve`].
    pub(crate) async fn approve(&self, id: &str) -> Result<pb::InboxRequest, Status> {
        match self.inner.upgrade() {
            Some(inner) => Migrations { inner }.approve(id).await,
            None => Err(gone(id)),
        }
    }
}

impl Migrations {
    /// The Migrate items of `inbox`, which `carry` carries out. The pending
    /// ones the Inbox kept across a restart are the ones a second detection
    /// updates.
    pub(crate) fn new(inbox: Arc<Inbox>, carry: Arc<dyn Carry>) -> Self {
        let pending = inbox.kept_migrates().iter().filter_map(restored).collect();
        let migrations = Self {
            inner: Arc::new(Inner {
                inbox: inbox.clone(),
                carry,
                folders: std::sync::OnceLock::new(),
                pending: Mutex::new(pending),
                serial: tokio::sync::Mutex::new(()),
                names: std::sync::OnceLock::new(),
                migrated: std::sync::OnceLock::new(),
            }),
        };
        inbox.migrating(Approver {
            inner: Arc::downgrade(&migrations.inner),
        });
        migrations
    }

    /// Says what the items call a machine. Set once at boot; until then an
    /// id is shown as it is.
    pub(crate) fn set_names(&self, names: Names) {
        let _ = self.inner.names.set(names);
    }

    /// What `machine` is called in an item's text or a refusal (D500).
    pub(crate) fn name_of(&self, machine: &pb::MachineId) -> String {
        match self.inner.names.get() {
            Some(names) => names.of(&machine.value),
            None => machine.value.clone(),
        }
    }

    /// SEAM (MACHINESCRIPTS D689): `hear` is told the machine each time one
    /// of its Migrate items ends migrated (not withdrawn), so its `upgraded`
    /// rules run. Set once at boot.
    pub(crate) fn on_migrated(&self, hear: Arc<dyn Fn(&str) + Send + Sync>) {
        let _ = self.inner.migrated.set(hear);
    }

    /// The Desk service says how the Desk folders item is carried out.
    pub(crate) fn set_desk_folders(&self, folders: Arc<dyn FolderMigrator>) {
        let _ = self.inner.folders.set(folders);
    }

    /// `machine`'s `thing` is `from` while the host's is `to`: one pending
    /// item says so. A second detection updates the item (same id, a new
    /// revision) when what it shows changed, rather than posting again.
    ///
    /// A daemon item takes in the machine's CLI and host tools items: they
    /// are withdrawn as resolved, and the differing tools are lines of it
    /// (D281 j).
    pub(crate) async fn observe(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        from: pb::BuildInfo,
        to: pb::BuildInfo,
    ) -> Result<pb::RequestId, MigrateError> {
        let _serial = self.inner.serial.lock().await;
        let id = self.observe_locked(thing, machine, from, to).await?;
        if thing == pb::MigrateThing::Daemon {
            for (_, tool) in tools::TOOLS {
                self.resolved_if_waiting_locked(tool, machine).await?;
            }
        }
        Ok(id)
    }

    /// As [`Self::observe`] for a CLI or host tools item, which waits while
    /// the machine's daemon item does: the tool is then a line of that item.
    pub(crate) async fn observe_tool(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        from: pb::BuildInfo,
        to: pb::BuildInfo,
    ) -> Result<(), MigrateError> {
        let _serial = self.inner.serial.lock().await;
        if self.known(&(pb::MigrateThing::Daemon, machine.value.clone())).is_some() {
            return self.resolved_if_waiting_locked(thing, machine).await;
        }
        self.observe_locked(thing, machine, from, to).await.map(|_| ())
    }

    async fn observe_locked(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        from: pb::BuildInfo,
        to: pb::BuildInfo,
    ) -> Result<pb::RequestId, MigrateError> {
        let key = (thing, machine.value.clone());
        if let Some(known) = self.known(&key) {
            if known.from == from && known.to == to {
                return Ok(known.id);
            }
            let (id, shown) = (known.id.value.clone(), (from.clone(), to.clone()));
            let name = self.name_of(machine);
            let updated = self
                .inner
                .inbox
                .update_migrate(&id, |request| show(request, thing, machine, &name, &shown.0, &shown.1))
                .await
                .map_err(|source| MigrateError::Update {
                    machine: machine.value.clone(),
                    request: id.clone(),
                    source,
                })?;
            if updated {
                let migrating = known.migrating;
                self.remember(key, Pending { id: known.id.clone(), from, to, migrating, detail: None });
                return Ok(known.id);
            }
            // The Inbox no longer holds it pending: it is posted again.
            self.forget(&key);
        }
        let request = item(thing, machine, &self.name_of(machine), &from, &to);
        let id = request.id.clone().unwrap_or_default();
        self.inner
            .inbox
            .post_migrate(request)
            .await
            .map_err(|source| MigrateError::Post {
                machine: machine.value.clone(),
                source,
            })?;
        self.remember(key, Pending { id: id.clone(), from, to, migrating: false, detail: None });
        Ok(id)
    }

    /// `machine`'s `thing` is at the host's release: its item, if any, is
    /// finished: Approved when it was being migrated, otherwise withdrawn
    /// as resolved.
    pub(crate) async fn resolved(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
    ) -> Result<(), MigrateError> {
        let _serial = self.inner.serial.lock().await;
        self.resolved_locked(thing, machine).await
    }

    /// As [`Self::resolved`], except an item being migrated is left alone:
    /// its own end (the daemon's arrival, a failure) settles it.
    pub(crate) async fn resolved_if_waiting(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
    ) -> Result<(), MigrateError> {
        let _serial = self.inner.serial.lock().await;
        self.resolved_if_waiting_locked(thing, machine).await
    }

    async fn resolved_if_waiting_locked(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
    ) -> Result<(), MigrateError> {
        let migrating = self
            .known(&(thing, machine.value.clone()))
            .is_some_and(|known| known.migrating);
        if migrating {
            return Ok(());
        }
        self.resolved_locked(thing, machine).await
    }

    async fn resolved_locked(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
    ) -> Result<(), MigrateError> {
        let key = (thing, machine.value.clone());
        let Some(known) = self.known(&key) else {
            return Ok(());
        };
        if known.migrating {
            let done = if thing == pb::MigrateThing::DeskFolders {
                "Desk folders migrated".to_owned()
            } else {
                format!("Migrated to {}", shown(&known.to))
            };
            self.inner
                .inbox
                .settle_migrate(&known.id.value, done)
                .await
                .map_err(|source| MigrateError::Settle {
                    machine: machine.value.clone(),
                    request: known.id.value.clone(),
                    source,
                })?;
        } else {
            self.inner
                .inbox
                .withdraw_migrate(&known.id.value, pb::WithdrawReason::Resolved)
                .await
                .map_err(|source| MigrateError::Withdraw {
                    machine: machine.value.clone(),
                    request: known.id.value.clone(),
                    source,
                })?;
        }
        self.forget(&key);
        // SEAM (MACHINESCRIPTS D689): a finished Migrate upgraded the machine;
        // the Desk folders are not the machine.
        if known.migrating
            && thing != pb::MigrateThing::DeskFolders
            && let Some(hear) = self.inner.migrated.get()
        {
            hear(&machine.value);
        }
        Ok(())
    }

    /// The user clicked Migrate on item `id`: it becomes Migrating, `Carry`
    /// takes it from there, and the item as it stands is returned. A
    /// failure is shown on the item (Failed, still pending), not returned;
    /// Migrate again retries. A second click while it migrates changes
    /// nothing.
    async fn approve(&self, id: &str) -> Result<pb::InboxRequest, Status> {
        let mut begun = None;
        let mut item = None;
        let carry = &self.inner.carry;
        let updated = self
            .inner
            .inbox
            .update_migrate(id, |request| {
                if let Some(card) = card_mut(request)
                    && card.phase() != pb::MigratePhase::Migrating
                {
                    card.phase = pb::MigratePhase::Migrating as i32;
                    card.last_error = None;
                    // D379: the steps are set now and move as the work does.
                    let machine = card.machine.clone().unwrap_or_default();
                    card.steps = fresh_steps(carry.steps(card.thing(), &machine));
                    begun = Some((
                        card.thing(),
                        card.machine.clone().unwrap_or_default(),
                        card.to.clone().unwrap_or_default(),
                    ));
                }
                item = Some(request.clone());
            })
            .await?;
        let (true, Some(item)) = (updated, item) else {
            return Err(gone(id));
        };
        let Some((thing, machine, to)) = begun else {
            return Ok(item);
        };
        let key = (thing, machine.value.clone());
        self.set_migrating(&key, true);
        if thing == pb::MigrateThing::DeskFolders {
            return self.approve_desk_folders(id, &key, &machine, item).await;
        }
        let progress = Progress {
            item: Some((self.clone(), id.to_owned())),
        };
        match self.inner.carry.migrate(thing, &machine, &to, &progress).await {
            Ok(()) => {
                if thing == pb::MigrateThing::Cli || thing == pb::MigrateThing::HostTools {
                    // The file is in place: there is no restart or link-up to wait for.
                    if let Err(error) = self.resolved(thing, &machine).await {
                        crate::log::say!("humatad2: warning: {}", status::chain(&error));
                    }
                } else if thing != pb::MigrateThing::Holders
                    // A holders Migrate waits for each agent's next idle
                    // moment, which can be as long as a turn: no deadline.
                    && let Some(within) = self.inner.carry.deadline(&machine)
                {
                    self.expect_by(key, item.id.clone().unwrap_or_default().value, to.bundle, within);
                }
                Ok(item)
            }
            Err(error) => {
                self.set_migrating(&key, false);
                let problem = status::error_of(&error.status()).unwrap_or_else(|| pb::Error {
                    message: status::chain(&error),
                    ..Default::default()
                });
                let mut failed = None;
                self.inner
                    .inbox
                    .update_migrate(id, |request| {
                        if let Some(card) = card_mut(request) {
                            card.phase = pb::MigratePhase::Failed as i32;
                            card.last_error = Some(problem);
                            fail_step(card);
                        }
                        failed = Some(request.clone());
                    })
                    .await?;
                failed.ok_or_else(|| gone(id))
            }
        }
    }

    /// The Desk folders item was clicked (D458): the Desk service makes each
    /// folder a repository, and the item ends Approved; a folder that fails
    /// leaves it pending as Failed with what to fix.
    async fn approve_desk_folders(
        &self,
        id: &str,
        key: &(pb::MigrateThing, String),
        machine: &pb::MachineId,
        item: pb::InboxRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let outcome = match self.inner.folders.get() {
            Some(folders) => folders.migrate_folders(machine).await,
            None => Err(pb::Error {
                message: "the Desk isn't running yet, so its folders couldn't be migrated".to_owned(),
                fix: "wait a moment and click Migrate again; if it stays, restart humatad2 on this Mac".to_owned(),
                retry: None,
                kind: None,
            }),
        };
        let Err(problem) = outcome else {
            if let Err(error) = self.resolved(key.0, machine).await {
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
            }
            return Ok(item);
        };
        self.set_migrating(key, false);
        let mut failed = None;
        self.inner
            .inbox
            .update_migrate(id, |request| {
                if let Some(card) = card_mut(request) {
                    card.phase = pb::MigratePhase::Failed as i32;
                    card.last_error = Some(problem);
                    fail_step(card);
                }
                failed = Some(request.clone());
            })
            .await?;
        failed.ok_or_else(|| gone(id))
    }

    /// Starts the clock on a migration that was carried out: if `within`
    /// passes and item `id` still waits for release `bundle`, it fails.
    fn expect_by(&self, key: (pb::MigrateThing, String), id: String, bundle: String, within: Duration) {
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(within).await;
            if let Err(error) = me.stalled(&key, &id, &bundle, within).await {
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
            }
        });
    }

    /// The machine didn't come back at `bundle` in time: item `id`, if it is
    /// still migrating to it, goes back to pending as Failed, so Migrate
    /// can be clicked again. An item that arrived, was withdrawn or is
    /// waiting for another release is left alone.
    async fn stalled(
        &self,
        key: &(pb::MigrateThing, String),
        id: &str,
        bundle: &str,
        within: Duration,
    ) -> Result<(), Status> {
        let waiting = self
            .known(key)
            .is_some_and(|known| known.migrating && known.id.value == id && known.to.bundle == bundle);
        if !waiting {
            return Ok(());
        }
        self.set_migrating(key, false);
        let machine = &key.1;
        let problem = pb::Error {
            message: format!(
                "{machine} wasn't back at release {} within {} seconds of being told to migrate",
                short(bundle),
                within.as_secs()
            ),
            fix: format!(
                "look at ~/.humata2/logs on {machine} to see why its daemon didn't start, then click Migrate again; the new files stay staged there, and the old daemon is kept as ~/.humata2/bin/humatad2.previous"
            ),
            retry: None,
            kind: None,
        };
        self.inner
            .inbox
            .update_migrate(id, |request| {
                if let Some(card) = card_mut(request)
                    && card.phase() == pb::MigratePhase::Migrating
                {
                    card.phase = pb::MigratePhase::Failed as i32;
                    card.last_error = Some(problem);
                    fail_step(card);
                }
            })
            .await?;
        Ok(())
    }

    /// This machine's holders: the agent sessions in `detail.waiting` are on
    /// a holder other than the release `build` names (D373). One pending item
    /// lists them; a second detection updates it (same id, a new revision)
    /// when the list or the lines changed, and an item being migrated stays
    /// so. `detail.waiting` must not be empty: with none waiting the item is
    /// finished by [`Self::resolved`].
    pub(crate) async fn observe_holders(
        &self,
        machine: &pb::MachineId,
        build: pb::BuildInfo,
        detail: Detail,
    ) -> Result<(), MigrateError> {
        let _serial = self.inner.serial.lock().await;
        let key = (pb::MigrateThing::Holders, machine.value.clone());
        if let Some(known) = self.known(&key) {
            if known.from == build && known.detail.as_ref() == Some(&detail) {
                return Ok(());
            }
            let id = known.id.value.clone();
            let shown_now = (build.clone(), detail.clone());
            let cleared = known.detail.as_ref().is_some_and(|before| before.failed.is_some());
            let name = self.name_of(machine);
            let updated = self
                .inner
                .inbox
                .update_migrate(&id, |request| show_holders(request, machine, &name, &shown_now.0, &shown_now.1, cleared))
                .await
                .map_err(|source| MigrateError::Update {
                    machine: machine.value.clone(),
                    request: id.clone(),
                    source,
                })?;
            if updated {
                // A failed restart ends the round: Migrate is on offer again.
                let migrating = known.migrating && detail.failed.is_none();
                self.remember(
                    key,
                    Pending {
                        id: known.id,
                        from: build.clone(),
                        to: build,
                        migrating,
                        detail: Some(detail),
                    },
                );
                return Ok(());
            }
            // The Inbox no longer holds it pending: it is posted again.
            self.forget(&key);
        }
        let mut request = blank();
        show_holders(&mut request, machine, &self.name_of(machine), &build, &detail, false);
        let id = request.id.clone().unwrap_or_default();
        self.inner
            .inbox
            .post_migrate(request)
            .await
            .map_err(|source| MigrateError::Post {
                machine: machine.value.clone(),
                source,
            })?;
        self.remember(
            key,
            Pending {
                id,
                from: build.clone(),
                to: build,
                migrating: false,
                detail: Some(detail),
            },
        );
        Ok(())
    }

    /// Whether `machine`'s `thing` item was told to Migrate and hasn't
    /// finished.
    pub(crate) fn is_migrating(&self, thing: pb::MigrateThing, machine: &pb::MachineId) -> bool {
        self.known(&(thing, machine.value.clone()))
            .is_some_and(|known| known.migrating)
    }

    /// Migrating `machine`'s holders stopped on `problem`: the item shows it
    /// as Failed and is no longer migrating, so Migrate retries.
    pub(crate) async fn fail_holders(&self, machine: &pb::MachineId, problem: pb::Error) {
        let _serial = self.inner.serial.lock().await;
        let key = (pb::MigrateThing::Holders, machine.value.clone());
        let Some(known) = self.known(&key) else {
            return;
        };
        self.set_migrating(&key, false);
        let shown = self
            .inner
            .inbox
            .update_migrate(&known.id.value, |request| {
                if let Some(card) = card_mut(request) {
                    card.phase = pb::MigratePhase::Failed as i32;
                    card.last_error = Some(problem);
                    fail_step(card);
                }
            })
            .await;
        if let Err(error) = shown {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    /// This host's daemon is at release `bundle` on `machine`, its own
    /// machine: a Migrate item for it that was waiting for that release is
    /// finished.
    pub(crate) async fn arrived(&self, machine: &str, bundle: &str) {
        let key = (pb::MigrateThing::Daemon, machine.to_owned());
        if bundle.is_empty() || !self.known(&key).is_some_and(|known| known.to.bundle == bundle) {
            return;
        }
        let machine = pb::MachineId {
            value: machine.to_owned(),
        };
        if let Err(error) = self.resolved(pb::MigrateThing::Daemon, &machine).await {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    /// The upgrade of this host's own daemon to release `to_bundle` was
    /// undone: its Migrate item, if it was waiting for that release, shows
    /// `problem` as Failed and is no longer migrating, so Migrate retries.
    pub(crate) async fn failed(&self, machine: &str, to_bundle: &str, problem: pb::Error) {
        let _serial = self.inner.serial.lock().await;
        let key = (pb::MigrateThing::Daemon, machine.to_owned());
        let Some(known) = self.known(&key).filter(|known| known.to.bundle == to_bundle) else {
            return;
        };
        self.set_migrating(&key, false);
        let shown = self
            .inner
            .inbox
            .update_migrate(&known.id.value, |request| {
                if let Some(card) = card_mut(request) {
                    card.phase = pb::MigratePhase::Failed as i32;
                    card.last_error = Some(problem);
                    fail_step(card);
                }
            })
            .await;
        if let Err(error) = shown {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    fn set_migrating(&self, key: &(pb::MigrateThing, String), migrating: bool) {
        if let Some(known) = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(key)
        {
            known.migrating = migrating;
        }
    }

    /// The pending item for `thing` on `machine`, if there is one.
    #[cfg(test)]
    pub(crate) fn pending(&self, thing: pb::MigrateThing, machine: &pb::MachineId) -> Option<pb::RequestId> {
        self.known(&(thing, machine.value.clone())).map(|known| known.id)
    }

    /// A pending item of `machine`, the daemon's first: what a refusal
    /// points the user at.
    pub(crate) fn mismatched(
        &self,
        machine: &pb::MachineId,
    ) -> Option<(pb::MigrateThing, pb::RequestId, pb::BuildInfo, pb::BuildInfo)> {
        let pending = self.inner.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending
            .iter()
            .filter(|((_, name), _)| *name == machine.value)
            .min_by_key(|((thing, _), _)| *thing as i32)
            .map(|((thing, _), known)| (*thing, known.id.clone(), known.from.clone(), known.to.clone()))
    }

    fn known(&self, key: &(pb::MigrateThing, String)) -> Option<Pending> {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn remember(&self, key: (pb::MigrateThing, String), known: Pending) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, known);
    }

    fn forget(&self, key: &(pb::MigrateThing, String)) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}

/// What each link-up does about releases: compares the machine's build
/// with this host's and posts, updates or withdraws its Migrate item.
#[derive(Clone)]
pub(crate) struct Watch {
    migrations: Migrations,
    release: Release,
}

impl Watch {
    pub(crate) fn new(migrations: Migrations, release: Release) -> Self {
        Self { migrations, release }
    }

    /// The link to `machine` is up and its Hello said `build` (`None` for a
    /// daemon too old to say). A host with no release of its own compares
    /// nothing (D290 l).
    pub(crate) async fn linked(&self, machine: &str, build: Option<&pb::BuildInfo>) {
        let bundle = self.release.bundle();
        if bundle.is_empty() {
            return;
        }
        let machine = pb::MachineId {
            value: machine.to_owned(),
        };
        let theirs = build.cloned().unwrap_or_default();
        if let Err(error) = self.compare(&machine, theirs).await {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    /// `machine` reported the holders on it that are on an old holder file
    /// (D378): `report` is the whole list. One Holders item for the machine
    /// lists its agent sessions and shells; with no agent waiting the item
    /// ends (Approved when it was being migrated). `after_link` is whether
    /// this is the first report of a link: an item that is being migrated and
    /// still has agents waiting then asks the machine again over `channel`,
    /// because a link that dropped may have taken the first ask with it.
    pub(crate) async fn holders(
        &self,
        machine: &str,
        report: pb::HoldersReport,
        after_link: bool,
        channel: &tonic::transport::Channel,
    ) {
        if self.release.bundle().is_empty() {
            return;
        }
        let id = pb::MachineId {
            value: machine.to_owned(),
        };
        let waiting: Vec<String> = report
            .holders
            .iter()
            .filter_map(|row| row.session.as_ref())
            .filter(|session| !session.value.is_empty())
            .map(|session| session.value.clone())
            .collect();
        let shells = report.holders.len() - waiting.len();
        let migrations = &self.migrations;
        if waiting.is_empty() {
            if let Err(error) = migrations.resolved(pb::MigrateThing::Holders, &id).await {
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
            }
            return;
        }
        let build = tools::release_only(&self.release.info());
        let detail = Detail {
            waiting,
            keeps_running: shell_line(shells).into_iter().collect(),
            rows: report.holders,
            failed: report.failed,
        };
        if let Err(error) = migrations.observe_holders(&id, build.clone(), detail).await {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
            return;
        }
        if after_link && migrations.is_migrating(pb::MigrateThing::Holders, &id) {
            remote::ask_holders(channel, machine, &build.bundle).await;
        }
    }

    /// Posts, updates or withdraws what `theirs` needs: the daemon when it
    /// is another release; otherwise its CLI and host tools that aren't the
    /// ones the release names, by what it says it has installed (D371).
    async fn compare(&self, machine: &pb::MachineId, theirs: pb::BuildInfo) -> Result<(), MigrateError> {
        let ours = tools::release_only(&self.release.info());
        if theirs.bundle != ours.bundle {
            self.migrations
                .observe(pb::MigrateThing::Daemon, machine, theirs, ours)
                .await?;
            return Ok(());
        }
        self.migrations.resolved(pb::MigrateThing::Daemon, machine).await?;
        let differing = tools::differing(&theirs.files, &theirs.installed);
        for thing in tools::THINGS {
            if tools::wants(&differing, thing) {
                self.migrations
                    .observe_tool(thing, machine, tools::from_installed(&theirs), ours.clone())
                    .await?;
            } else {
                self.migrations.resolved(thing, machine).await?;
            }
        }
        Ok(())
    }
}

/// The card of a Migrate item, to change.
fn card_mut(request: &mut pb::InboxRequest) -> Option<&mut pb::MigratePreview> {
    match request.preview.as_mut()?.preview.as_mut()? {
        pb::request_preview::Preview::Migrate(card) => Some(card),
        _ => None,
    }
}

/// What the pending map holds for a Migrate item the Inbox kept: nothing
/// when its card doesn't say which machine it is for.
fn restored(request: &pb::InboxRequest) -> Option<((pb::MigrateThing, String), Pending)> {
    let Some(pb::request_preview::Preview::Migrate(card)) =
        request.preview.as_ref().and_then(|preview| preview.preview.as_ref())
    else {
        return None;
    };
    let machine = card.machine.as_ref()?;
    Some((
        (card.thing(), machine.value.clone()),
        Pending {
            id: request.id.clone()?,
            from: card.from.clone().unwrap_or_default(),
            to: card.to.clone().unwrap_or_default(),
            migrating: card.phase() == pb::MigratePhase::Migrating,
            detail: None,
        },
    ))
}

/// The refusal for a click on an item that isn't waiting to be migrated.
fn gone(id: &str) -> Status {
    status::failure(
        tonic::Code::FailedPrecondition,
        pb::Error {
            message: format!("Migrate item {id} is no longer waiting: it was finished or withdrawn"),
            fix: "open the Inbox again; if the machine still needs migrating, a new item is posted when it links".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

fn thing_name(thing: pb::MigrateThing) -> &'static str {
    match thing {
        pb::MigrateThing::Daemon | pb::MigrateThing::Unspecified => "daemon",
        pb::MigrateThing::Cli => "CLI",
        pb::MigrateThing::Store => "store",
        pb::MigrateThing::Holders => "holders",
        pb::MigrateThing::HostTools => "host tools",
        pb::MigrateThing::DeskFolders => "Desk folders",
    }
}

/// "2.0.5 (a1b2c3d)", or just the version when the build names no release.
fn shown(build: &pb::BuildInfo) -> String {
    match (short(&build.bundle), build.version.as_str()) {
        ("", version) => version.to_owned(),
        (id, "") => id.to_owned(),
        (id, version) => format!("{version} ({id})"),
    }
}

/// What the item shows of the mismatch: its card, and why.
fn show(
    request: &mut pb::InboxRequest,
    thing: pb::MigrateThing,
    machine: &pb::MachineId,
    name: &str,
    from: &pb::BuildInfo,
    to: &pb::BuildInfo,
) {
    request.action_title = format!("Migrate the {} on {name}", thing_name(thing));
    let is_tool = matches!(thing, pb::MigrateThing::Cli | pb::MigrateThing::HostTools);
    request.reason = if thing == pb::MigrateThing::DeskFolders {
        "Desk folders made before documents had versions have no version history yet.".to_owned()
    } else if is_tool {
        format!(
            "The {} on {name} isn't the one release {} names.",
            thing_name(thing),
            shown(to)
        )
    } else {
        format!("{name} runs {}; this app is {}.", shown(from), shown(to))
    };
    let (phase, steps) = match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Migrate(card)) => (card.phase, card.steps.clone()),
        _ => (pb::MigratePhase::Waiting as i32, Vec::new()),
    };
    request.preview = Some(pb::RequestPreview {
        preview: Some(pb::request_preview::Preview::Migrate(pb::MigratePreview {
            thing: thing as i32,
            machine: Some(machine.clone()),
            from: Some(from.clone()),
            to: Some(to.clone()),
            changes: changes(thing, name, from, to),
            keeps_running: vec!["Agent sessions and shells on it keep running".to_owned()],
            phase,
            last_error: None,
            waiting_sessions: Vec::new(),
            steps,
            holders: Vec::new(),
        })),
    });
}

/// The lines of a card: what Migrate would replace. A daemon's say too what
/// it brings along: a store schema it moves (D372) and a CLI or host tools
/// that aren't the ones the machine's release names (D281 j).
fn changes(thing: pb::MigrateThing, name: &str, from: &pb::BuildInfo, to: &pb::BuildInfo) -> Vec<String> {
    if matches!(thing, pb::MigrateThing::Cli | pb::MigrateThing::HostTools) {
        return vec![format!(
            "Put release {}'s {} back on {name}",
            shown(to),
            thing_name(thing)
        )];
    }
    if thing == pb::MigrateThing::DeskFolders {
        return vec![format!(
            "Set up version history in each Desk folder Humata made on {name}; no files change and no version is saved"
        )];
    }
    let mut lines = vec![format!(
        "Replace the {} on {name}: {} to {}",
        thing_name(thing),
        shown(from),
        shown(to)
    )];
    if thing == pb::MigrateThing::Daemon {
        if from.store_schema != 0 && to.store_schema != 0 && from.store_schema != to.store_schema {
            lines.push(format!(
                "Store · schema {} → {} (a copy is kept first)",
                from.store_schema, to.store_schema
            ));
        }
        for file in tools::differing(&from.files, &from.installed) {
            let thing = tools::TOOLS
                .iter()
                .find(|(tool, _)| *tool == file)
                .map_or(pb::MigrateThing::Unspecified, |(_, thing)| *thing);
            lines.push(format!(
                "Replace the {} on {name}: it isn't the one release {} names",
                thing_name(thing),
                shown(from)
            ));
        }
    }
    lines
}

fn item(
    thing: pb::MigrateThing,
    machine: &pb::MachineId,
    name: &str,
    from: &pb::BuildInfo,
    to: &pb::BuildInfo,
) -> pb::InboxRequest {
    let mut request = blank();
    show(&mut request, thing, machine, name, from, to);
    request
}

/// A new pending VIP Migrate item that shows nothing yet.
fn blank() -> pb::InboxRequest {
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: format!("rq_{}", uuid::Uuid::new_v4().simple()),
        }),
        kind: pb::RequestKind::Migrate as i32,
        state: pb::RequestState::Pending as i32,
        vip: true,
        created_at: Some(attach::now()),
        revision: Some(pb::Revision { value: 1 }),
        ..Default::default()
    }
}

/// What a Holders item shows: its card lists the sessions that wait and
/// what keeps running (D373). A change keeps the phase the item is in.
fn show_holders(
    request: &mut pb::InboxRequest,
    machine: &pb::MachineId,
    name: &str,
    build: &pb::BuildInfo,
    detail: &Detail,
    cleared: bool,
) {
    let count = detail.waiting.len();
    request.action_title = format!("Migrate the holders on {name}");
    request.reason = format!(
        "{count} agent session{} on {name} still run{} on a holder from an earlier release than {}.",
        if count == 1 { "" } else { "s" },
        if count == 1 { "s" } else { "" },
        shown(build)
    );
    // A machine's reported failure shows the item Failed; the report after it
    // that carries none (`cleared`) puts a Failed item back to waiting.
    let phase = match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        _ if detail.failed.is_some() => pb::MigratePhase::Failed as i32,
        Some(pb::request_preview::Preview::Migrate(card)) => {
            if cleared && card.phase() == pb::MigratePhase::Failed {
                pb::MigratePhase::Waiting as i32
            } else {
                card.phase
            }
        }
        _ => pb::MigratePhase::Waiting as i32,
    };
    request.preview = Some(pb::RequestPreview {
        preview: Some(pb::request_preview::Preview::Migrate(pb::MigratePreview {
            thing: pb::MigrateThing::Holders as i32,
            machine: Some(machine.clone()),
            from: Some(build.clone()),
            to: Some(build.clone()),
            changes: vec![format!(
                "Restart {count} agent session{} on release {}'s holder, each when it is next idle; the same conversation continues",
                if count == 1 { "" } else { "s" },
                shown(build)
            )],
            keeps_running: detail.keeps_running.clone(),
            phase,
            last_error: detail.failed.clone(),
            waiting_sessions: detail
                .waiting
                .iter()
                .map(|session| pb::SessionId { value: session.clone() })
                .collect(),
            steps: Vec::new(),
            holders: detail.rows.clone(),
        })),
    });
}

/// Whether `request` is a Migrate item.
pub(crate) fn is_migrate(request: &pb::InboxRequest) -> bool {
    request.kind() == pb::RequestKind::Migrate
}

/// Why `answer` can't be given to the Migrate item `request`, if it can't.
/// `None` for any other request, and for Migrate (Approve), the one answer
/// it takes: the Inbox hands that to `Migrations::approve`.
pub(crate) fn refuse_answer(
    request: &pb::InboxRequest,
    answer: &pb::request_answer::Answer,
) -> Option<Status> {
    use pb::request_answer::Answer;
    if !is_migrate(request) {
        return None;
    }
    let id = request.id.as_ref().map_or("", |id| id.value.as_str());
    match answer {
        Answer::Approve(_) => None,
        Answer::Deny(_) | Answer::Always(_) | Answer::Reply(_) => Some(status::invalid_field(
            "answer",
            Problem::NotAllowedHere,
            format!("request {id} is a Migrate item: it takes Migrate and nothing else"),
            "click Migrate on the item; it can't be denied, always allowed or replied to, and it goes away once the machine is at this Mac's release",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::events::Feed;
    use crate::guard::Guard;
    use humata2_store::Store;

    const HOST: &str = "ma_host";

    /// What carries out a Migrate in these tests: counts the calls and
    /// fails them while `fails` is set.
    #[derive(Default)]
    struct Fake {
        calls: std::sync::atomic::AtomicUsize,
        fails: std::sync::atomic::AtomicBool,
    }

    #[tonic::async_trait]
    impl Carry for Fake {
        async fn migrate(
            &self,
            _thing: pb::MigrateThing,
            machine: &pb::MachineId,
            _to: &pb::BuildInfo,
            _progress: &Progress,
        ) -> Result<(), RunError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fails.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(RunError::NotHere {
                    machine: machine.value.clone(),
                });
            }
            Ok(())
        }
    }

    fn idle() -> Arc<dyn Carry> {
        Arc::new(Fake::default())
    }

    fn calls(fake: &Fake) -> usize {
        fake.calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn inbox(directory: &Path) -> Arc<Inbox> {
        let store = Arc::new(Store::open(directory).unwrap());
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        Arc::new(Inbox::new(guard, store, HOST.to_owned(), Arc::new(Feed::default())))
    }

    fn build(version: &str, bundle: &str) -> pb::BuildInfo {
        pb::BuildInfo {
            version: version.to_owned(),
            bundle: bundle.to_owned(),
            ..Default::default()
        }
    }

    fn gpu() -> pb::MachineId {
        pb::MachineId {
            value: "gpu".to_owned(),
        }
    }

    fn card(request: &pb::InboxRequest) -> &pb::MigratePreview {
        match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
            Some(pb::request_preview::Preview::Migrate(card)) => card,
            other => panic!("not a Migrate card: {other:?}"),
        }
    }

    async fn held(inbox: &Inbox, id: &pb::RequestId) -> pb::InboxRequest {
        inbox.held_request(&id.value).await.unwrap()
    }

    async fn posted(migrations: &Migrations, inbox: &Inbox, machine: &pb::MachineId) -> pb::InboxRequest {
        let id = migrations
            .observe(pb::MigrateThing::Daemon, machine, build("2.0.4", "aaaaaaaa"), build("2.0.5", "bbbbbbbb"))
            .await
            .unwrap();
        held(inbox, &id).await
    }

    #[tokio::test]
    async fn a_named_machines_item_is_titled_by_its_name_and_keeps_its_id() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        migrations.set_names(Names::of_host(HOST).naming("ma_bcc83", "gpu-box"));
        let machine = pb::MachineId { value: "ma_bcc83".to_owned() };

        let item = posted(&migrations, &inbox, &machine).await;

        assert!(item.action_title.contains("gpu-box"), "{}", item.action_title);
        assert!(!item.action_title.contains("ma_bcc83"), "{}", item.action_title);
        assert!(!item.reason.contains("ma_bcc83"), "{}", item.reason);
        assert_eq!(card(&item).machine, Some(machine));
    }

    #[tokio::test]
    async fn the_hosts_own_machine_is_titled_this_mac() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        migrations.set_names(Names::of_host(HOST));
        let machine = pb::MachineId { value: HOST.to_owned() };

        let item = posted(&migrations, &inbox, &machine).await;

        assert!(item.action_title.contains("This Mac"), "{}", item.action_title);
        assert!(!item.action_title.contains(HOST), "{}", item.action_title);
        assert_eq!(card(&item).machine, Some(machine));
    }

    #[tokio::test]
    async fn a_machine_with_no_known_name_is_titled_by_its_id() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        migrations.set_names(Names::of_host(HOST));

        let item = posted(&migrations, &inbox, &gpu()).await;

        assert!(item.action_title.contains("gpu"), "{}", item.action_title);
    }

    #[tokio::test]
    async fn a_holders_item_is_titled_by_the_machines_name() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        migrations.set_names(Names::of_host(HOST));
        let machine = pb::MachineId { value: HOST.to_owned() };
        let detail = Detail { waiting: vec!["se_1".to_owned()], ..Default::default() };

        migrations.observe_holders(&machine, build("2.0.5", "bbbbbbbb"), detail).await.unwrap();

        let id = migrations.pending(pb::MigrateThing::Holders, &machine).unwrap();
        let item = held(&inbox, &id).await;
        assert!(item.action_title.contains("This Mac"), "{}", item.action_title);
        assert!(item.reason.contains("This Mac"), "{}", item.reason);
    }

    #[tokio::test]
    async fn a_first_detection_posts_one_vip_item_with_no_requester_waiting() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());

        let id = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.4", "aaaaaaaa"), build("2.0.5", "bbbbbbbb"))
            .await
            .unwrap();

        let item = held(&inbox, &id).await;
        assert_eq!(item.kind(), pb::RequestKind::Migrate);
        assert_eq!(item.state(), pb::RequestState::Pending);
        assert!(item.vip);
        assert_eq!(item.requester, None);
        assert_eq!(item.always, None);
        assert_eq!(card(&item).phase(), pb::MigratePhase::Waiting);
        assert_eq!(card(&item).machine, Some(gpu()));
        assert_eq!(migrations.pending(pb::MigrateThing::Daemon, &gpu()), Some(id));
    }

    #[tokio::test]
    async fn a_second_detection_updates_the_same_item_and_bumps_its_revision() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let to = build("2.0.5", "bbbbbbbb");
        let first = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.4", "aaaaaaaa"), to.clone())
            .await
            .unwrap();
        let before = held(&inbox, &first).await;

        let second = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.3", "cccccccc"), to)
            .await
            .unwrap();

        assert_eq!(second, first);
        let after = held(&inbox, &first).await;
        assert_eq!(card(&after).from.as_ref().map(|from| from.bundle.as_str()), Some("cccccccc"));
        assert!(after.revision.as_ref().unwrap().value > before.revision.as_ref().unwrap().value);
        assert_eq!(after.state(), pb::RequestState::Pending);
    }

    #[tokio::test]
    async fn a_detection_that_shows_nothing_new_leaves_the_item_as_it_was() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let (from, to) = (build("2.0.4", "aaaaaaaa"), build("2.0.5", "bbbbbbbb"));
        let id = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), from.clone(), to.clone())
            .await
            .unwrap();
        let before = held(&inbox, &id).await;

        migrations.observe(pb::MigrateThing::Daemon, &gpu(), from, to).await.unwrap();

        assert_eq!(held(&inbox, &id).await, before);
    }

    #[tokio::test]
    async fn each_thing_and_machine_has_its_own_item() {
        let directory = tempfile::tempdir().unwrap();
        let migrations = Migrations::new(inbox(directory.path()), idle());
        let (from, to) = (build("2.0.4", "aaaaaaaa"), build("2.0.5", "bbbbbbbb"));
        let other = pb::MachineId {
            value: "tpu".to_owned(),
        };

        let daemon = migrations.observe(pb::MigrateThing::Daemon, &gpu(), from.clone(), to.clone()).await.unwrap();
        let cli = migrations.observe(pb::MigrateThing::Cli, &gpu(), from.clone(), to.clone()).await.unwrap();
        let elsewhere = migrations.observe(pb::MigrateThing::Daemon, &other, from, to).await.unwrap();

        assert_ne!(daemon, cli);
        assert_ne!(daemon, elsewhere);
        let (thing, id, _, _) = migrations.mismatched(&gpu()).unwrap();
        assert_eq!((thing, id), (pb::MigrateThing::Daemon, daemon));
        assert_eq!(migrations.mismatched(&pb::MachineId { value: "none".to_owned() }), None);
    }

    #[tokio::test]
    async fn a_finished_migrate_tells_the_hook_once_and_a_withdrawn_item_does_not() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        let ears = heard.clone();
        migrations.on_migrated(Arc::new(move |machine: &str| ears.lock().unwrap().push(machine.to_owned())));
        let (from, to) = (build("2.0.4", "aaaaaaaa"), build("2.0.5", "bbbbbbbb"));

        // A CLI Migrate is done when its file is in place: it settles as migrated.
        let cli = migrations.observe(pb::MigrateThing::Cli, &gpu(), from.clone(), to.clone()).await.unwrap();
        migrations.approve(&cli.value).await.unwrap();
        assert_eq!(*heard.lock().unwrap(), vec!["gpu".to_owned()]);

        // An item resolved without a Migrate (withdrawn) is no upgrade.
        migrations.observe(pb::MigrateThing::Daemon, &gpu(), from, to).await.unwrap();
        migrations.resolved(pb::MigrateThing::Daemon, &gpu()).await.unwrap();
        assert_eq!(heard.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn resolving_withdraws_the_item_as_resolved_and_a_later_mismatch_posts_a_new_one() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let (from, to) = (build("2.0.4", "aaaaaaaa"), build("2.0.5", "bbbbbbbb"));
        let id = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), from.clone(), to.clone())
            .await
            .unwrap();

        migrations.resolved(pb::MigrateThing::Daemon, &gpu()).await.unwrap();
        migrations.resolved(pb::MigrateThing::Daemon, &gpu()).await.unwrap();

        let done = held(&inbox, &id).await;
        assert_eq!(done.state(), pb::RequestState::Withdrawn);
        assert_eq!(done.withdraw_reason(), pb::WithdrawReason::Resolved);
        assert_eq!(migrations.pending(pb::MigrateThing::Daemon, &gpu()), None);
        let again = migrations.observe(pb::MigrateThing::Daemon, &gpu(), from, to).await.unwrap();
        assert_ne!(again, id);
    }

    #[test]
    fn only_migrate_is_an_answer_to_a_migrate_item() {
        use pb::request_answer::Answer;
        let migrate = item(pb::MigrateThing::Daemon, &gpu(), "gpu", &build("1", "a"), &build("2", "b"));
        let question = pb::InboxRequest {
            kind: pb::RequestKind::Question as i32,
            ..Default::default()
        };
        for answer in [
            Answer::Deny(pb::Deny::default()),
            Answer::Always(pb::AlwaysLet::default()),
            Answer::Reply(pb::Reply { text: "later".to_owned() }),
        ] {
            let refused = refuse_answer(&migrate, &answer).expect("refused");
            assert_eq!(refused.code(), tonic::Code::InvalidArgument);
            assert!(refuse_answer(&question, &answer).is_none());
        }
        let approve = Answer::Approve(pb::Approve {});
        assert!(refuse_answer(&migrate, &approve).is_none());
        assert!(refuse_answer(&question, &approve).is_none());
    }

    /// A pending Daemon item for `gpu` and the machinery around it.
    async fn waiting(
        directory: &Path,
    ) -> (Arc<Inbox>, Arc<Fake>, Migrations, pb::RequestId, pb::BuildInfo) {
        let inbox = inbox(directory);
        let fake = Arc::new(Fake::default());
        let migrations = Migrations::new(inbox.clone(), fake.clone());
        let to = build("2.0.5", "bbbbbbbb");
        let id = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.4", "aaaaaaaa"), to.clone())
            .await
            .unwrap();
        (inbox, fake, migrations, id, to)
    }

    #[tokio::test]
    async fn migrate_marks_the_item_migrating_and_hands_it_to_what_carries_it_out() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, fake, migrations, id, _) = waiting(directory.path()).await;

        let shown = migrations.approve(&id.value).await.unwrap();

        assert_eq!(card(&shown).phase(), pb::MigratePhase::Migrating);
        assert_eq!(shown.state(), pb::RequestState::Pending);
        assert_eq!(calls(&fake), 1);
        assert_eq!(card(&held(&inbox, &id).await).phase(), pb::MigratePhase::Migrating);
    }

    #[tokio::test]
    async fn a_second_click_while_it_migrates_carries_nothing_out_again() {
        let directory = tempfile::tempdir().unwrap();
        let (_, fake, migrations, id, _) = waiting(directory.path()).await;
        migrations.approve(&id.value).await.unwrap();

        let again = migrations.approve(&id.value).await.unwrap();

        assert_eq!(card(&again).phase(), pb::MigratePhase::Migrating);
        assert_eq!(calls(&fake), 1);
    }

    #[tokio::test]
    async fn a_migration_that_fails_stays_pending_as_failed_with_its_error_and_retries() {
        let directory = tempfile::tempdir().unwrap();
        let (_, fake, migrations, id, _) = waiting(directory.path()).await;
        fake.fails.store(true, std::sync::atomic::Ordering::SeqCst);

        let failed = migrations.approve(&id.value).await.unwrap();

        assert_eq!(failed.state(), pb::RequestState::Pending);
        assert_eq!(card(&failed).phase(), pb::MigratePhase::Failed);
        assert!(card(&failed).last_error.as_ref().is_some_and(|error| !error.message.is_empty()));

        fake.fails.store(false, std::sync::atomic::Ordering::SeqCst);
        let retried = migrations.approve(&id.value).await.unwrap();

        assert_eq!(card(&retried).phase(), pb::MigratePhase::Migrating);
        assert_eq!(card(&retried).last_error, None);
        assert_eq!(calls(&fake), 2);
    }

    #[tokio::test]
    async fn a_machine_not_back_by_its_deadline_returns_to_pending_as_failed_and_can_migrate_again() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, fake, migrations, id, to) = waiting(directory.path()).await;
        migrations.approve(&id.value).await.unwrap();
        let key = (pb::MigrateThing::Daemon, "gpu".to_owned());

        migrations.stalled(&key, &id.value, &to.bundle, Duration::from_secs(60)).await.unwrap();

        let item = held(&inbox, &id).await;
        assert_eq!(item.state(), pb::RequestState::Pending);
        assert_eq!(card(&item).phase(), pb::MigratePhase::Failed);
        assert!(card(&item).last_error.as_ref().is_some_and(|error| !error.fix.is_empty()));
        let retried = migrations.approve(&id.value).await.unwrap();
        assert_eq!(card(&retried).phase(), pb::MigratePhase::Migrating);
        assert_eq!(calls(&fake), 2);
    }

    #[tokio::test]
    async fn a_machine_that_arrived_before_its_deadline_is_not_failed() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, _, migrations, id, to) = waiting(directory.path()).await;
        migrations.approve(&id.value).await.unwrap();
        migrations.resolved(pb::MigrateThing::Daemon, &gpu()).await.unwrap();
        let key = (pb::MigrateThing::Daemon, "gpu".to_owned());

        migrations.stalled(&key, &id.value, &to.bundle, Duration::from_secs(60)).await.unwrap();

        assert_eq!(held(&inbox, &id).await.state(), pb::RequestState::Approved);
    }

    #[tokio::test]
    async fn an_undone_upgrade_leaves_the_item_pending_as_failed_so_a_click_retries() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, fake, migrations, id, to) = waiting(directory.path()).await;
        migrations.approve(&id.value).await.unwrap();
        let problem = pb::Error {
            message: "went back".to_owned(),
            ..Default::default()
        };

        migrations.failed("gpu", "another release", problem.clone()).await;
        assert_eq!(card(&held(&inbox, &id).await).phase(), pb::MigratePhase::Migrating, "another release's failure is not this item's");

        migrations.failed("gpu", &to.bundle, problem).await;

        let shown = held(&inbox, &id).await;
        assert_eq!(shown.state(), pb::RequestState::Pending);
        assert_eq!(card(&shown).phase(), pb::MigratePhase::Failed);
        assert!(card(&shown).last_error.is_some());
        let retried = migrations.approve(&id.value).await.unwrap();
        assert_eq!(card(&retried).phase(), pb::MigratePhase::Migrating);
        assert_eq!(calls(&fake), 2);
    }

    #[tokio::test]
    async fn the_machine_arriving_at_the_release_finishes_a_migrating_item_as_approved() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, _, migrations, id, to) = waiting(directory.path()).await;
        migrations.approve(&id.value).await.unwrap();

        migrations.resolved(pb::MigrateThing::Daemon, &gpu()).await.unwrap();

        let done = held(&inbox, &id).await;
        assert_eq!(done.state(), pb::RequestState::Approved);
        assert!(done.reason.contains(short(&to.bundle)), "{}", done.reason);
        assert_eq!(migrations.pending(pb::MigrateThing::Daemon, &gpu()), None);
        let again = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.4", "aaaaaaaa"), to)
            .await
            .unwrap();
        assert_ne!(again, id, "a later mismatch is a new item");
    }

    #[tokio::test]
    async fn arriving_at_the_release_an_item_waits_for_finishes_it_and_another_release_does_not() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, _, migrations, id, to) = waiting(directory.path()).await;
        migrations.approve(&id.value).await.unwrap();

        migrations.arrived("gpu", "cccccccc").await;
        assert_eq!(held(&inbox, &id).await.state(), pb::RequestState::Pending);
        migrations.arrived("tpu", &to.bundle).await;
        assert_eq!(held(&inbox, &id).await.state(), pb::RequestState::Pending);

        migrations.arrived("gpu", &to.bundle).await;
        assert_eq!(held(&inbox, &id).await.state(), pb::RequestState::Approved);
    }

    #[tokio::test]
    async fn an_item_nobody_clicked_is_withdrawn_when_its_machine_arrives() {
        let directory = tempfile::tempdir().unwrap();
        let (inbox, _, migrations, id, to) = waiting(directory.path()).await;

        migrations.arrived("gpu", &to.bundle).await;

        let done = held(&inbox, &id).await;
        assert_eq!(done.state(), pb::RequestState::Withdrawn);
        assert_eq!(done.withdraw_reason(), pb::WithdrawReason::Resolved);
    }

    #[tokio::test]
    async fn clicking_an_item_that_is_finished_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let (_, fake, migrations, id, _) = waiting(directory.path()).await;
        migrations.resolved(pb::MigrateThing::Daemon, &gpu()).await.unwrap();

        let refused = migrations.approve(&id.value).await.expect_err("nothing to migrate");

        assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
        assert_eq!(calls(&fake), 0);
    }

    #[tokio::test]
    async fn a_restart_keeps_the_pending_item_so_a_detection_after_it_updates_that_item() {
        let directory = tempfile::tempdir().unwrap();
        let (first, _, _, id, to) = waiting(directory.path()).await;
        drop(first);

        let migrations = Migrations::new(inbox(directory.path()), idle());

        assert_eq!(migrations.pending(pb::MigrateThing::Daemon, &gpu()), Some(id.clone()));
        let again = migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.4", "aaaaaaaa"), to)
            .await
            .unwrap();
        assert_eq!(again, id);
    }

    #[tokio::test]
    async fn a_host_with_no_release_posts_nothing_on_link_up() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox, idle());
        let bin = directory.path().join("bin");
        let release = Release::read(&bin, &directory.path().join("humatad2"), "2.0.5", 1);
        assert_eq!(release.bundle(), "");

        Watch::new(migrations.clone(), release)
            .linked("gpu", Some(&build("2.0.4", "aaaaaaaa")))
            .await;

        assert_eq!(migrations.pending(pb::MigrateThing::Daemon, &gpu()), None);
    }

    /// The host's release: a daemon binary in `directory`, and a
    /// `bin/bundle.json` naming it as this platform's `humatad2`, read with
    /// store schema `store_schema`.
    fn host_release(directory: &Path, store_schema: u32) -> Release {
        use sha2::{Digest, Sha256};
        let sha = |bytes: &[u8]| -> String {
            Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
        };
        let daemon: &[u8] = b"the host's daemon";
        let own_exe = directory.join("humatad2");
        std::fs::write(&own_exe, daemon).expect("the daemon binary is written");
        let platform = humata2_machines::Platform::host()
            .expect("the tests run on a platform Humata ships")
            .bundle_name();
        let mut platforms = serde_json::Map::new();
        platforms.insert(
            platform.to_owned(),
            serde_json::json!({
                "humatad2": {"path": "humatad2", "sha256": sha(daemon)},
                "humata2_holder": {"path": "humata2_holder", "sha256": sha(b"holder")},
                "humata": {"path": "humata", "sha256": sha(b"cli")},
            }),
        );
        let bin = directory.join("bin");
        std::fs::create_dir_all(&bin).expect("the bin folder is made");
        std::fs::write(
            bin.join("bundle.json"),
            serde_json::json!({ "platforms": platforms }).to_string(),
        )
        .expect("the bundle manifest is written");
        let release = Release::read(&bin, &own_exe, "2.0.5", store_schema);
        assert!(!release.bundle().is_empty(), "the host names its release");
        release
    }

    /// A machine at another release than the host's, with store schema
    /// `store_schema`; everything else the same for every machine.
    fn elsewhere(store_schema: u32) -> pb::BuildInfo {
        pb::BuildInfo {
            store_schema,
            ..build("2.0.4", "aaaaaaaa")
        }
    }

    fn tpu() -> pb::MachineId {
        pb::MachineId {
            value: "tpu".to_owned(),
        }
    }

    /// How many lines of changes the daemon item for `machine` shows.
    async fn change_lines(migrations: &Migrations, inbox: &Inbox, machine: &pb::MachineId) -> usize {
        let id = migrations
            .pending(pb::MigrateThing::Daemon, machine)
            .unwrap_or_else(|| panic!("{} has a daemon item", machine.value));
        card(&held(inbox, &id).await).changes.len()
    }

    #[tokio::test]
    async fn a_tool_item_waits_for_no_daemon_item_and_is_folded_into_one_that_appears() {
        let directory = tempfile::tempdir().unwrap();
        let migrations = Migrations::new(inbox(directory.path()), idle());
        let release = build("2.0.5", "bbbbbbbb");

        migrations
            .observe_tool(pb::MigrateThing::Cli, &gpu(), release.clone(), release.clone())
            .await
            .unwrap();
        assert!(migrations.pending(pb::MigrateThing::Cli, &gpu()).is_some());

        migrations
            .observe(pb::MigrateThing::Daemon, &gpu(), build("2.0.4", "aaaaaaaa"), release.clone())
            .await
            .unwrap();
        assert!(migrations.pending(pb::MigrateThing::Cli, &gpu()).is_none(), "the CLI item was folded in");

        migrations
            .observe_tool(pb::MigrateThing::Cli, &gpu(), release.clone(), release)
            .await
            .unwrap();
        assert!(migrations.pending(pb::MigrateThing::Cli, &gpu()).is_none(), "no CLI item beside a daemon item");
        assert!(migrations.pending(pb::MigrateThing::Daemon, &gpu()).is_some());
    }

    #[tokio::test]
    async fn a_machine_whose_store_schema_differs_shows_one_more_change_on_its_daemon_item() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let watch = Watch::new(migrations.clone(), host_release(directory.path(), 3));

        watch.linked("gpu", Some(&elsewhere(3))).await;
        watch.linked("tpu", Some(&elsewhere(2))).await;

        let same = change_lines(&migrations, &inbox, &gpu()).await;
        let older = change_lines(&migrations, &inbox, &tpu()).await;
        assert_eq!(older, same + 1);
    }

    #[tokio::test]
    async fn a_machine_whose_store_schema_is_unknown_shows_no_store_change() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let watch = Watch::new(migrations.clone(), host_release(directory.path(), 3));

        watch.linked("gpu", Some(&elsewhere(3))).await;
        watch.linked("tpu", Some(&elsewhere(0))).await;

        let same = change_lines(&migrations, &inbox, &gpu()).await;
        let unknown = change_lines(&migrations, &inbox, &tpu()).await;
        assert_eq!(unknown, same);
    }

    fn waiting_on(sessions: &[&str]) -> Detail {
        Detail {
            waiting: sessions.iter().map(|session| (*session).to_owned()).collect(),
            keeps_running: vec!["1 shell stays on its holder until it is closed".to_owned()],
            rows: sessions
                .iter()
                .map(|session| pb::HolderSession {
                    session: Some(pb::SessionId {
                        value: (*session).to_owned(),
                    }),
                    title: String::new(),
                    note: pb::HolderNote::RestartsWhenIdle as i32,
                })
                .collect(),
            failed: None,
        }
    }

    #[tokio::test]
    async fn a_holders_item_lists_the_waiting_sessions_and_a_change_updates_the_same_item() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let release = build("2.0.5", "bbbbbbbb");

        migrations.observe_holders(&gpu(), release.clone(), waiting_on(&["ss_1", "ss_2"])).await.unwrap();
        let id = migrations.pending(pb::MigrateThing::Holders, &gpu()).unwrap();
        migrations.observe_holders(&gpu(), release.clone(), waiting_on(&["ss_2"])).await.unwrap();

        assert_eq!(migrations.pending(pb::MigrateThing::Holders, &gpu()), Some(id.clone()));
        let shown = held(&inbox, &id).await;
        let sessions: Vec<_> = card(&shown).waiting_sessions.iter().map(|session| session.value.as_str()).collect();
        assert_eq!(sessions, ["ss_2"]);
        assert!(shown.vip);
        assert_eq!(card(&shown).keeps_running.len(), 1);
        assert_eq!(migrations.mismatched(&gpu()).map(|(thing, ..)| thing), Some(pb::MigrateThing::Holders));
    }

    #[tokio::test]
    async fn a_holders_item_being_migrated_keeps_its_phase_as_its_list_shrinks_and_ends_approved() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let fake = Arc::new(Fake::default());
        let migrations = Migrations::new(inbox.clone(), fake.clone());
        let release = build("2.0.5", "bbbbbbbb");
        migrations.observe_holders(&gpu(), release.clone(), waiting_on(&["ss_1", "ss_2"])).await.unwrap();
        let id = migrations.pending(pb::MigrateThing::Holders, &gpu()).unwrap();

        migrations.approve(&id.value).await.unwrap();
        migrations.observe_holders(&gpu(), release, waiting_on(&["ss_2"])).await.unwrap();

        assert_eq!(calls(&fake), 1);
        assert!(migrations.is_migrating(pb::MigrateThing::Holders, &gpu()));
        assert_eq!(card(&held(&inbox, &id).await).phase(), pb::MigratePhase::Migrating);
        migrations.resolved(pb::MigrateThing::Holders, &gpu()).await.unwrap();
        assert_eq!(held(&inbox, &id).await.state(), pb::RequestState::Approved);
        assert_eq!(migrations.pending(pb::MigrateThing::Holders, &gpu()), None);
    }

    fn failed_restart() -> pb::Error {
        pb::Error {
            message: "couldn't start session ss_1's agent again".to_owned(),
            fix: "click Migrate again, or close session ss_1".to_owned(),
            retry: None,
            kind: None,
        }
    }

    #[tokio::test]
    async fn a_report_with_a_failed_restart_fails_the_item_with_its_error_and_a_clean_one_clears_it() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let fake = Arc::new(Fake::default());
        let migrations = Migrations::new(inbox.clone(), fake.clone());
        let release = build("2.0.5", "bbbbbbbb");
        migrations.observe_holders(&gpu(), release.clone(), waiting_on(&["ss_1"])).await.unwrap();
        let id = migrations.pending(pb::MigrateThing::Holders, &gpu()).unwrap();
        migrations.approve(&id.value).await.unwrap();

        let failed = Detail { failed: Some(failed_restart()), ..waiting_on(&["ss_1"]) };
        migrations.observe_holders(&gpu(), release.clone(), failed).await.unwrap();

        let shown = held(&inbox, &id).await;
        assert_eq!(card(&shown).phase(), pb::MigratePhase::Failed);
        assert_eq!(card(&shown).last_error, Some(failed_restart()));
        assert!(!migrations.is_migrating(pb::MigrateThing::Holders, &gpu()));
        assert_eq!(shown.state(), pb::RequestState::Pending);

        migrations.observe_holders(&gpu(), release, waiting_on(&["ss_1"])).await.unwrap();

        let shown = held(&inbox, &id).await;
        assert_eq!(card(&shown).phase(), pb::MigratePhase::Waiting);
        assert_eq!(card(&shown).last_error, None);
        migrations.approve(&id.value).await.unwrap();
        assert_eq!(calls(&fake), 2);
    }

    #[tokio::test]
    async fn a_report_listing_no_old_holder_after_a_failure_ends_the_item() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let failed = Detail { failed: Some(failed_restart()), ..waiting_on(&["ss_1"]) };
        migrations.observe_holders(&gpu(), build("2.0.5", "bbbbbbbb"), failed).await.unwrap();
        let id = migrations.pending(pb::MigrateThing::Holders, &gpu()).unwrap();
        assert_eq!(card(&held(&inbox, &id).await).phase(), pb::MigratePhase::Failed);

        migrations.resolved(pb::MigrateThing::Holders, &gpu()).await.unwrap();

        assert_eq!(held(&inbox, &id).await.state(), pb::RequestState::Withdrawn);
    }

    #[tokio::test]
    async fn a_holders_item_that_nothing_waits_for_any_more_is_withdrawn_as_resolved() {
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        migrations
            .observe_holders(&gpu(), build("2.0.5", "bbbbbbbb"), waiting_on(&["ss_1"]))
            .await
            .unwrap();
        let id = migrations.pending(pb::MigrateThing::Holders, &gpu()).unwrap();

        migrations.resolved(pb::MigrateThing::Holders, &gpu()).await.unwrap();

        let done = held(&inbox, &id).await;
        assert_eq!(done.state(), pb::RequestState::Withdrawn);
        assert_eq!(done.withdraw_reason(), pb::WithdrawReason::Resolved);
    }

    // A store schema bump (a new migration step) ships in a new daemon, so a
    // machine still on the schema before it is at another release: it gets
    // the one VIP daemon item, whose card says which schema it moves from
    // and to, and the gate holds its sessions until it is migrated (UP-1).
    #[tokio::test]
    async fn a_machine_on_the_store_schema_before_this_one_gets_a_vip_daemon_item_that_moves_it() {
        let current = u32::try_from(humata2_store::SCHEMA_VERSION).expect("the schema version fits in u32");
        let directory = tempfile::tempdir().unwrap();
        let inbox = inbox(directory.path());
        let migrations = Migrations::new(inbox.clone(), idle());
        let watch = Watch::new(migrations.clone(), host_release(directory.path(), current));

        watch.linked("gpu", Some(&elsewhere(current - 1))).await;

        let id = migrations
            .pending(pb::MigrateThing::Daemon, &gpu())
            .expect("an older store schema is a daemon item");
        let item = held(&inbox, &id).await;
        assert_eq!(item.kind(), pb::RequestKind::Migrate);
        assert!(item.vip);
        assert_eq!(card(&item).from.as_ref().map(|from| from.store_schema), Some(current - 1));
        assert_eq!(card(&item).to.as_ref().map(|to| to.store_schema), Some(current));
        assert_eq!(migrations.mismatched(&gpu()).map(|(thing, ..)| thing), Some(pb::MigrateThing::Daemon));
    }
}
