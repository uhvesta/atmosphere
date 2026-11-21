//! Supervision of the `humata2_fusefs` helper (FUSEFSWIRE, D895).
//!
//! The mount is off unless Settings turns it on: while it is off, humatad2
//! spawns no helper and mounts nothing. Turning it on adopts a helper that
//! already answers on `<state>/fusefs.sock` (so a humatad2 restart never
//! unmounts), or starts one in its own session, then asks it to mount.
//! Turning it off asks the helper to unmount and leaves it running idle: the
//! helper has no shutdown request, and an idle helper holds nothing.
//!
//! Nothing here blocks the caller or the daemon's start: every step runs on
//! its own thread and a failure is logged, never raised.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use humata2_fusefs_control::{
    ControlClient, ControlResponse, DaemonLinkState, FsMountMode, HostUpdate, MachineProjection,
    MountState, SessionWire, TopologyWire, probe_mount_mode,
};
use humata2_grpc::humata::v1 as pb;

use crate::events::Feed;
use crate::log::say;

/// How long a freshly started helper has to answer on its socket.
const HELPER_START_WAIT: Duration = Duration::from_secs(5);

/// A helper that stayed up this long before dying earns a fresh automatic
/// respawn; one that dies sooner is not started again by itself.
const RESPAWN_RESET: Duration = Duration::from_secs(30);

/// What Settings should say about the mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FuseMountStatus {
    /// The mount isn't wanted.
    Off,
    /// The mount is wanted and being set up.
    Starting,
    /// The tree is mounted; agents get `HUMATA_FUSE_ROOT`.
    Mounted,
    /// The mount is wanted but can't run. Agents use the files tools. The
    /// reason says what failed and, where possible, how to fix it.
    CannotSetUp { reason: String },
}

/// Called after the mount status changes.
type StatusNotice = Arc<dyn Fn() + Send + Sync>;

/// Called by a launcher when the helper it started has exited.
type ExitNotice = Box<dyn FnOnce() + Send>;

/// The latest request from Settings. The generation grows with every request.
struct Desired {
    on: bool,
    generation: u64,
}

/// What went wrong talking to or starting the helper.
#[derive(Debug, thiserror::Error)]
enum FuseFsError {
    #[error(
        "couldn't make the FUSEFS mount folder {}; check that its parent exists and belongs to you, or pass --fusefs-mount",
        path.display()
    )]
    MountFolder {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "couldn't start the FUSEFS helper {}; reinstall Humata on this machine, or pass --fusefs with a working humata2_fusefs",
        path.display()
    )]
    Spawn {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the FUSEFS helper didn't answer on {} within {} seconds; the mount stays off. Turn it off and on in Settings to try again",
        socket.display(),
        HELPER_START_WAIT.as_secs()
    )]
    NoAnswer { socket: PathBuf },

    #[error(
        "couldn't talk to the FUSEFS helper on {}; turn the mount off and on in Settings to restart the conversation",
        socket.display()
    )]
    Call {
        socket: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("the FUSEFS helper refused (code {errno}): {message}")]
    Refused { errno: i32, message: String },

    #[error(
        "the FUSEFS helper answered something this humatad2 doesn't know; the helper and humatad2 may be from different releases"
    )]
    Unexpected,

    #[error("couldn't encode an update for the FUSEFS helper")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

/// Starts the helper process and says which mode this machine can run.
trait HelperLauncher: Send + Sync {
    fn mode(&self) -> FsMountMode;
    /// Starts the helper. `on_exit` must be called once when it exits.
    fn spawn(&self, mode: FsMountMode, on_exit: ExitNotice) -> Result<(), FuseFsError>;
}

/// Starts the real `humata2_fusefs serve` in a session of its own, stdio
/// detached, so it outlives a humatad2 exec swap.
struct ProcessLauncher {
    helper: PathBuf,
    socket: PathBuf,
    mount_root: PathBuf,
}

impl HelperLauncher for ProcessLauncher {
    fn mode(&self) -> FsMountMode {
        probe_mount_mode()
    }

    fn spawn(&self, mode: FsMountMode, on_exit: ExitNotice) -> Result<(), FuseFsError> {
        let mut command = Command::new(&self.helper);
        command
            .arg("serve")
            .arg("--socket")
            .arg(&self.socket)
            .arg("--mountpoint")
            .arg(&self.mount_root)
            .arg("--mode")
            .arg(mode.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: between fork and exec this makes one system call that is
        // safe there: it starts a session, leaving humatad2's, so a signal to
        // humatad2's process group never reaches the helper (and its mount).
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|source| FuseFsError::Spawn {
            path: self.helper.clone(),
            source,
        })?;
        // Collects the exit status, so a helper that dies isn't left a zombie.
        let reaper = std::thread::Builder::new()
            .name("fusefs-reaper".to_owned())
            .spawn(move || {
                if let Ok(status) = child.wait() {
                    say!("humatad2: the FUSEFS helper exited ({status})");
                }
                on_exit();
            });
        if let Err(error) = reaper {
            say!("humatad2: couldn't watch the FUSEFS helper's exit: {error}");
        }
        Ok(())
    }
}

/// Reads the machines and sessions the helper projects.
type TopologySource = Arc<dyn Fn() -> TopologyWire + Send + Sync>;

struct Inner {
    socket: PathBuf,
    mount_root: PathBuf,
    launcher: Arc<dyn HelperLauncher>,
    topology: Mutex<Option<TopologySource>>,
    /// One bring-up, take-down or push at a time. Holds the newest request
    /// generation a switch has applied.
    work: Mutex<u64>,
    /// What Settings last asked for.
    desired: Mutex<Desired>,
    status: Mutex<FuseMountStatus>,
    /// Told after the status changes, so Settings can say so.
    on_status_change: Mutex<Option<StatusNotice>>,
    /// Ids of helpers this daemon started; `current_spawn` is the live one,
    /// or 0 for an adopted helper (which has no child to wait on).
    spawn_counter: AtomicU64,
    current_spawn: AtomicU64,
    /// Automatic respawns left for this bring-up.
    respawns_left: AtomicU32,
    /// When the mount last came up.
    up_since: Mutex<Option<Instant>>,
    /// A helper is up and told about this daemon, so pushes mean something.
    live: AtomicBool,
    /// The tree is mounted: set only after a mount succeeds.
    mounted: Arc<AtomicBool>,
    link_epoch: AtomicU64,
    push_wanted: AtomicBool,
    pusher_running: AtomicBool,
    /// Threads still working, and a bell for whoever waits for none.
    pending: Mutex<usize>,
    idle: Condvar,
}

/// Runs the FUSEFS helper while Settings has the mount on.
#[derive(Clone)]
pub(crate) struct FuseFsSupervisor {
    inner: Arc<Inner>,
}

impl FuseFsSupervisor {
    /// A supervisor of `helper`, serving on `<state>/fusefs.sock` and
    /// mounting at `mount_root`. Starts nothing. `mounted` is the flag this
    /// supervisor keeps: true only while the tree is mounted.
    pub(crate) fn new(
        helper: PathBuf,
        state: &Path,
        mount_root: PathBuf,
        mounted: Arc<AtomicBool>,
    ) -> Self {
        let socket = state.join("fusefs.sock");
        let launcher = Arc::new(ProcessLauncher {
            helper,
            socket: socket.clone(),
            mount_root: mount_root.clone(),
        });
        Self::with_launcher(socket, mount_root, launcher, mounted)
    }

    fn with_launcher(
        socket: PathBuf,
        mount_root: PathBuf,
        launcher: Arc<dyn HelperLauncher>,
        mounted: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                socket,
                mount_root,
                launcher,
                topology: Mutex::new(None),
                work: Mutex::new(0),
                desired: Mutex::new(Desired {
                    on: false,
                    generation: 0,
                }),
                status: Mutex::new(FuseMountStatus::Off),
                on_status_change: Mutex::new(None),
                spawn_counter: AtomicU64::new(0),
                current_spawn: AtomicU64::new(0),
                respawns_left: AtomicU32::new(1),
                up_since: Mutex::new(None),
                live: AtomicBool::new(false),
                mounted,
                link_epoch: AtomicU64::new(0),
                push_wanted: AtomicBool::new(false),
                pusher_running: AtomicBool::new(false),
                pending: Mutex::new(0),
                idle: Condvar::new(),
            }),
        }
    }

    /// Where the Humata tree is mounted: what agents get as `HUMATA_FUSE_ROOT`.
    pub(crate) fn mount_root(&self) -> &Path {
        &self.inner.mount_root
    }

    /// Whether the tree is mounted right now: false while the mount is off,
    /// the machine can't mount, the helper won't start or the mount was refused.
    #[allow(dead_code, reason = "the daemon reads the shared flag it passed to `new`; tests read it here")]
    pub(crate) fn is_mounted(&self) -> bool {
        self.inner.mounted.load(Ordering::SeqCst)
    }

    /// What Settings should show: off, starting, mounted, or why it can't run.
    pub(crate) fn mount_status(&self) -> FuseMountStatus {
        lock(&self.inner.status).clone()
    }

    /// [`Self::mount_status`] as Settings sends it (D908).
    pub(crate) fn mount_status_wire(&self) -> pb::FuseMountStatus {
        use pb::fuse_mount_status::State;
        let (state, mount_root, reason) = match self.mount_status() {
            FuseMountStatus::Off => (State::Off, String::new(), String::new()),
            FuseMountStatus::Starting => (State::Starting, String::new(), String::new()),
            FuseMountStatus::Mounted => {
                (State::Mounted, self.mount_root().to_string_lossy().into_owned(), String::new())
            }
            FuseMountStatus::CannotSetUp { reason } => (State::CannotSetUp, String::new(), reason),
        };
        pb::FuseMountStatus { state: state as i32, mount_root, reason }
    }

    /// Calls `notice` after each change of `mount_status`, from whichever
    /// thread made it. It must not block.
    pub(crate) fn on_status_change(&self, notice: StatusNotice) {
        *lock(&self.inner.on_status_change) = Some(notice);
    }

    /// Where the snapshot of machines and sessions comes from.
    pub(crate) fn use_topology(&self, source: TopologySource) {
        *lock(&self.inner.topology) = Some(source);
    }

    /// Turns the mount on or off. Returns at once; the work runs on its own
    /// thread and a failure is logged and shown in `mount_status`. The last
    /// request wins: a worker applies the newest request it finds, and one
    /// that finds nothing newer than what was applied does nothing.
    pub(crate) fn set_enabled(&self, on: bool) {
        {
            let mut desired = lock(&self.inner.desired);
            desired.on = on;
            desired.generation += 1;
        }
        if on {
            self.inner.set_status(FuseMountStatus::Starting);
        }
        let inner = Arc::clone(&self.inner);
        self.inner.spawn_work("fusefs-switch", move || {
            let mut applied = lock(&inner.work);
            let (on, generation) = {
                let desired = lock(&inner.desired);
                (desired.on, desired.generation)
            };
            if generation <= *applied {
                return;
            }
            *applied = generation;
            if on {
                inner.respawns_left.store(1, Ordering::SeqCst);
            }
            let result = if on { inner.bring_up() } else { inner.take_down() };
            if let Err(error) = result {
                say!(
                    "humatad2: turning the FUSEFS mount {} failed: {}",
                    if on { "on" } else { "off" },
                    crate::status::chain(&error)
                );
            }
        });
    }

    /// Sends the helper a fresh snapshot of machines and sessions. Returns at
    /// once; several calls close together share one push. Does nothing while
    /// the mount is off.
    pub(crate) fn push_topology(&self) {
        self.inner.push_wanted.store(true, Ordering::SeqCst);
        if self.inner.pusher_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let inner = Arc::clone(&self.inner);
        self.inner.spawn_work("fusefs-push", move || {
            loop {
                while inner.push_wanted.swap(false, Ordering::SeqCst) {
                    let _turn = lock(&inner.work);
                    if !inner.live.load(Ordering::SeqCst) {
                        continue;
                    }
                    if let Err(error) = inner.push_topology_now() {
                        // One failed push doesn't turn pushing off: the next
                        // event tries again. A helper that is gone is replaced.
                        say!(
                            "humatad2: couldn't update the FUSEFS helper's view: {}",
                            crate::status::chain(&error)
                        );
                        inner.recover_if_gone();
                    }
                }
                inner.pusher_running.store(false, Ordering::SeqCst);
                // A push asked for after the last check but before the flag
                // dropped: take the turn back, or leave it to its asker.
                if !inner.push_wanted.load(Ordering::SeqCst)
                    || inner.pusher_running.swap(true, Ordering::SeqCst)
                {
                    break;
                }
            }
        });
    }

    /// Pushes a fresh snapshot whenever a session, channel, machine or desk
    /// changes. Needs a Tokio runtime.
    pub(crate) fn follow(&self, feed: &Feed) {
        let mut events = feed.subscribe_events();
        let me = self.clone();
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if changes_topology(&event) {
                            me.push_topology();
                        }
                    }
                    Err(RecvError::Lagged(_)) => me.push_topology(),
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    /// Waits until no switch or push is running.
    pub(crate) fn wait_idle(&self) {
        let mut pending = lock(&self.inner.pending);
        while *pending > 0 {
            pending = self
                .inner
                .idle
                .wait(pending)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn changes_topology(event: &pb::Event) -> bool {
    use pb::event::Payload;
    matches!(
        event.payload,
        Some(
            Payload::Session(_)
                | Payload::Channel(_)
                | Payload::Machine(_)
                | Payload::Desk(_)
                | Payload::Resync(_)
        )
    )
}

impl Inner {
    /// Runs `work` on a thread of its own, counted until it ends.
    fn spawn_work(self: &Arc<Self>, name: &str, work: impl FnOnce() + Send + 'static) {
        *lock(&self.pending) += 1;
        let me = Arc::clone(self);
        let started = std::thread::Builder::new().name(name.to_owned()).spawn(move || {
            work();
            me.finished();
        });
        if let Err(error) = started {
            say!("humatad2: couldn't start a thread for the FUSEFS helper: {error}");
            self.finished();
        }
    }

    /// Records the mount status and, when it changed, tells the notice.
    fn set_status(&self, status: FuseMountStatus) {
        let changed = {
            let mut held = lock(&self.status);
            let changed = *held != status;
            *held = status;
            changed
        };
        if changed {
            let notice = lock(&self.on_status_change).clone();
            if let Some(notice) = notice {
                notice();
            }
        }
    }

    fn finished(&self) {
        let mut pending = lock(&self.pending);
        *pending = pending.saturating_sub(1);
        if *pending == 0 {
            self.idle.notify_all();
        }
    }

    fn client(&self) -> ControlClient {
        ControlClient::new(&self.socket)
    }

    fn call_error(&self, source: io::Error) -> FuseFsError {
        FuseFsError::Call {
            socket: self.socket.clone(),
            source,
        }
    }

    /// Adopts a live helper or starts one, tells it about this daemon and
    /// asks it to mount. Records the outcome in the mount status. Needs the
    /// `work` turn.
    fn bring_up(self: &Arc<Self>) -> Result<(), FuseFsError> {
        self.set_status(FuseMountStatus::Starting);
        let result = self.bring_up_steps();
        if let Err(error) = &result {
            self.mounted.store(false, Ordering::SeqCst);
            self.set_status(FuseMountStatus::CannotSetUp { reason: reason_of(error) });
        }
        result
    }

    fn mark_mounted(&self) {
        self.mounted.store(true, Ordering::SeqCst);
        *lock(&self.up_since) = Some(Instant::now());
        self.set_status(FuseMountStatus::Mounted);
    }

    fn bring_up_steps(self: &Arc<Self>) -> Result<(), FuseFsError> {
        self.mounted.store(false, Ordering::SeqCst);
        let client = self.client();
        let mode = self.launcher.mode();
        let adopted = matches!(client.health_ping(), Ok(ControlResponse::Pong { .. }));
        if adopted {
            say!("humatad2: the FUSEFS helper on {} is running; using it", self.socket.display());
            self.current_spawn.store(0, Ordering::SeqCst);
        } else {
            make_private_folder(&self.mount_root)?;
            say!("humatad2: starting the FUSEFS helper in {mode} mode");
            let id = self.spawn_counter.fetch_add(1, Ordering::SeqCst) + 1;
            self.current_spawn.store(id, Ordering::SeqCst);
            let weak = Arc::downgrade(self);
            self.launcher.spawn(
                mode,
                Box::new(move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.helper_exited(id);
                    }
                }),
            )?;
            self.wait_for_helper(&client)?;
        }
        self.live.store(true, Ordering::SeqCst);

        let epoch = self.link_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.send(
            &client,
            &HostUpdate::LinkState {
                state: DaemonLinkState::Connected {
                    daemon_pid: std::process::id(),
                    epoch,
                },
            },
        )?;
        self.push_topology_now()?;

        if mode == FsMountMode::Degraded {
            say!("humatad2: FSKit and /dev/fuse are unavailable here, so the FUSEFS tree isn't mounted");
            self.set_status(FuseMountStatus::CannotSetUp {
                reason: "This Mac has neither FSKit nor /dev/fuse.".to_owned(),
            });
            return Ok(());
        }
        let status = client.status().map_err(|source| self.call_error(source))?;
        if matches!(&status, ControlResponse::MountStatus { mount } if mount.state == MountState::Healthy)
        {
            self.mark_mounted();
            return Ok(());
        }
        expect_mount(client.mount().map_err(|source| self.call_error(source))?)?;
        self.mark_mounted();
        Ok(())
    }

    /// The helper this daemon started (id `id`) exited. Clears the flags at
    /// once so new agents fall back, then brings the mount back if it's wanted.
    fn helper_exited(self: &Arc<Self>, id: u64) {
        if self.current_spawn.load(Ordering::SeqCst) != id {
            return;
        }
        self.mounted.store(false, Ordering::SeqCst);
        self.live.store(false, Ordering::SeqCst);
        let me = Arc::clone(self);
        self.spawn_work("fusefs-exit", move || {
            let _turn = lock(&me.work);
            // A newer helper (a switch ran meanwhile) makes this exit stale.
            if me.current_spawn.load(Ordering::SeqCst) != id {
                return;
            }
            me.recover();
        });
    }

    /// After a failed call: if the helper no longer answers, treat it as
    /// exited. This is how an adopted helper's death is noticed. Needs the
    /// `work` turn.
    fn recover_if_gone(self: &Arc<Self>) {
        if matches!(self.client().health_ping(), Ok(ControlResponse::Pong { .. })) {
            return;
        }
        self.recover();
    }

    /// The helper is gone. Clears the flags and, when the mount is still
    /// wanted and this bring-up has a respawn left, starts it again. Needs the
    /// `work` turn.
    fn recover(self: &Arc<Self>) {
        self.mounted.store(false, Ordering::SeqCst);
        self.live.store(false, Ordering::SeqCst);
        if !lock(&self.desired).on {
            self.set_status(FuseMountStatus::Off);
            return;
        }
        if lock(&self.up_since).is_some_and(|since| since.elapsed() >= RESPAWN_RESET) {
            self.respawns_left.store(1, Ordering::SeqCst);
        }
        let may_respawn = self
            .respawns_left
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok();
        if !may_respawn {
            self.set_status(FuseMountStatus::CannotSetUp {
                reason: "The mount helper keeps stopping. Check that it is installed, then turn this off and on to try again.".to_owned(),
            });
            return;
        }
        say!("humatad2: the FUSEFS helper stopped; starting it again");
        if let Err(error) = self.bring_up() {
            say!(
                "humatad2: couldn't bring the FUSEFS mount back: {}",
                crate::status::chain(&error)
            );
        }
    }

    /// Asks a running helper to unmount; with none running there is nothing to do.
    fn take_down(&self) -> Result<(), FuseFsError> {
        self.live.store(false, Ordering::SeqCst);
        self.mounted.store(false, Ordering::SeqCst);
        self.set_status(FuseMountStatus::Off);
        match self.client().unmount(false) {
            Ok(answer) => expect_mount(answer),
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(())
            }
            Err(source) => Err(self.call_error(source)),
        }
    }

    fn wait_for_helper(&self, client: &ControlClient) -> Result<(), FuseFsError> {
        let deadline = Instant::now() + HELPER_START_WAIT;
        loop {
            if matches!(client.health_ping(), Ok(ControlResponse::Pong { .. })) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(FuseFsError::NoAnswer {
                    socket: self.socket.clone(),
                });
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn push_topology_now(&self) -> Result<(), FuseFsError> {
        let source = lock(&self.topology).clone();
        let Some(source) = source else {
            return Ok(());
        };
        let topology = source();
        self.send(&self.client(), &HostUpdate::Topology { topology })
    }

    fn send(&self, client: &ControlClient, update: &HostUpdate) -> Result<(), FuseFsError> {
        let body = serde_json::to_value(update).map_err(|source| FuseFsError::Encode { source })?;
        match client.extension(body).map_err(|source| self.call_error(source))? {
            ControlResponse::ExtensionReply { .. } => Ok(()),
            ControlResponse::Error { errno, message } => Err(FuseFsError::Refused { errno, message }),
            _ => Err(FuseFsError::Unexpected),
        }
    }
}

/// What Settings says when the mount can't be set up: what failed and, when
/// something can be done here, how to fix it.
fn reason_of(error: &FuseFsError) -> String {
    match error {
        FuseFsError::MountFolder { path, .. } => format!(
            "Humata couldn't create the mount folder {}. Check that its parent folder exists and belongs to you, then turn this off and on to try again.",
            path.display()
        ),
        FuseFsError::Spawn { .. } => {
            "The mount helper wouldn't start. Reinstall Humata on this machine, then turn this off and on to try again.".to_owned()
        }
        FuseFsError::NoAnswer { .. } => {
            "The mount helper wouldn't start. Turn this off and on to try again.".to_owned()
        }
        FuseFsError::Call { .. } | FuseFsError::Encode { .. } => {
            "Humata couldn't talk to the mount helper. Turn this off and on to try again.".to_owned()
        }
        FuseFsError::Refused { message, .. } => format!(
            "The mount helper refused to mount: {}. Turn this off and on to try again.",
            message.trim().trim_end_matches('.')
        ),
        FuseFsError::Unexpected => {
            "The mount helper and Humata are from different releases. Update Humata, then turn this off and on.".to_owned()
        }
    }
}

/// A mount or unmount answer that isn't a status is a refusal.
fn expect_mount(answer: ControlResponse) -> Result<(), FuseFsError> {
    match answer {
        ControlResponse::MountStatus { .. } => Ok(()),
        ControlResponse::Error { errno, message } => Err(FuseFsError::Refused { errno, message }),
        _ => Err(FuseFsError::Unexpected),
    }
}

fn make_private_folder(path: &Path) -> Result<(), FuseFsError> {
    use std::os::unix::fs::DirBuilderExt;
    // A killed FUSE helper leaves the mountpoint returning ENOTCONN/ESTALE
    // until the replacement helper runs `fusermount3 -uz`; the underlying
    // directory already exists, so don't let `DirBuilder::create` fail on it.
    if let Err(err) = std::fs::metadata(path)
        && matches!(
            rustix::io::Errno::from_io_error(&err),
            Some(rustix::io::Errno::NOTCONN | rustix::io::Errno::STALE)
        )
    {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|source| FuseFsError::MountFolder {
            path: path.to_path_buf(),
            source,
        })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The machines and sessions in `store`, for the helper's tree. `root` is
/// this machine's root folder. A session of another machine carries no root
/// here, and every other machine is listed offline: the store doesn't know
/// whether it is reachable.
pub(crate) fn topology_from_store(
    store: &humata2_store::Store,
    machine: &str,
    root: &Path,
) -> TopologyWire {
    let sessions = store.sessions().unwrap_or_else(|error| {
        say!("humatad2: couldn't list sessions for the FUSEFS helper: {error}");
        Vec::new()
    });
    let tabs = store.tabs().unwrap_or_else(|error| {
        say!("humatad2: couldn't list tabs for the FUSEFS helper: {error}");
        Vec::new()
    });
    let mut machines = vec![MachineProjection {
        machine_id: machine.to_owned(),
        display_name: machine.to_owned(),
        online: true,
        machine_root: root.to_path_buf(),
    }];
    let mut wires = Vec::with_capacity(sessions.len());
    for session in sessions {
        let session_machine = if session.machine.is_empty() {
            machine.to_owned()
        } else {
            session.machine
        };
        if !machines.iter().any(|known| known.machine_id == session_machine) {
            machines.push(MachineProjection {
                machine_id: session_machine.clone(),
                display_name: session_machine.clone(),
                online: false,
                machine_root: PathBuf::new(),
            });
        }
        let local = session_machine == machine;
        let holder_pid = tabs
            .iter()
            .find(|tab| tab.session.as_deref() == Some(session.id.as_str()))
            .map_or(0, |tab| tab.holder_pid);
        wires.push(SessionWire {
            session_id: session.id,
            machine_id: session_machine,
            channel: Some(session.channel).filter(|channel| !channel.is_empty()),
            desk: None,
            holder_pid,
            machine_root: if local { root.to_path_buf() } else { PathBuf::new() },
        });
    }
    TopologyWire {
        local_machine_id: machine.to_owned(),
        local_machine_root: root.to_path_buf(),
        machines,
        sessions: wires,
        desks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::mpsc;

    use fusefs_host::{
        CapabilityRouter, ControlHandler, ControlServer, ControlServerHandle, ExtensionCallback,
        MountBackend, MountConfig, MountSupervisor,
    };

    use super::*;

    /// A mount backend that records calls and mounts nothing.
    #[derive(Default)]
    struct RecordingBackend {
        mounts: AtomicU32,
        unmounts: AtomicU32,
        refuse_mount: AtomicBool,
    }

    impl MountBackend for RecordingBackend {
        fn mount(&self, _config: &MountConfig) -> io::Result<Option<u32>> {
            self.mounts.fetch_add(1, Ordering::SeqCst);
            if self.refuse_mount.load(Ordering::SeqCst) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "refused"));
            }
            Ok(None)
        }

        fn unmount(&self, _mountpoint: &Path, _force: bool) -> io::Result<()> {
            self.unmounts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn is_stale_mount(&self, _mountpoint: &Path) -> io::Result<bool> {
            Ok(false)
        }

        fn cleanup_stale_mount(&self, _mountpoint: &Path) -> io::Result<()> {
            Ok(())
        }

        fn is_server_alive(&self, _pid: u32) -> bool {
            true
        }
    }

    /// An in-process helper on a temp socket.
    struct FakeHelper {
        backend: Arc<RecordingBackend>,
        updates: Arc<Mutex<Vec<HostUpdate>>>,
        /// How many of the next updates the helper answers with an error.
        fail_updates: Arc<AtomicU32>,
        server: ControlServerHandle,
    }

    fn start_helper(socket: &Path, mount_root: &Path) -> FakeHelper {
        let backend = Arc::new(RecordingBackend::default());
        let supervisor = Arc::new(MountSupervisor::new(
            MountConfig::new(mount_root, "fusefs://test"),
            backend.clone(),
        ));
        let updates = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&updates);
        let fail_updates = Arc::new(AtomicU32::new(0));
        let failing = Arc::clone(&fail_updates);
        let callback: ExtensionCallback = Arc::new(move |body: serde_json::Value| {
            if failing
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
                .is_ok()
            {
                return Err("busy".to_owned());
            }
            let update: HostUpdate = serde_json::from_value(body).map_err(|e| e.to_string())?;
            seen.lock().unwrap().push(update);
            Ok(serde_json::json!({ "applied": true }))
        });
        let handler = ControlHandler::new(supervisor, Arc::new(CapabilityRouter::new()))
            .with_extension(callback);
        let server = ControlServer::bind(socket, Arc::new(handler))
            .unwrap()
            .spawn();
        FakeHelper {
            backend,
            updates,
            fail_updates,
            server,
        }
    }

    /// Starts an in-process helper instead of a process, and counts starts.
    struct FakeLauncher {
        mode: FsMountMode,
        socket: PathBuf,
        mount_root: PathBuf,
        spawns: AtomicU32,
        started: Mutex<Option<FakeHelper>>,
        fail_spawn: AtomicBool,
        /// The exit notice of the newest helper.
        exit_notice: Mutex<Option<ExitNotice>>,
        /// When armed, the next spawn says so on `entered` and then waits for
        /// `release`.
        entered: Mutex<Option<mpsc::Sender<()>>>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
    }

    impl FakeLauncher {
        /// Makes the next spawn wait; returns the bell that says it began and
        /// the key that lets it go on.
        fn block_next_spawn(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            *self.entered.lock().unwrap() = Some(entered_tx);
            *self.release.lock().unwrap() = Some(release_rx);
            (entered_rx, release_tx)
        }

        /// The helper dies: its socket goes away and its exit is reported.
        fn helper_dies(&self) {
            if let Some(helper) = self.started.lock().unwrap().take() {
                helper.server.shutdown();
            }
            let notice = self.exit_notice.lock().unwrap().take();
            if let Some(notice) = notice {
                notice();
            }
        }
    }

    impl HelperLauncher for FakeLauncher {
        fn mode(&self) -> FsMountMode {
            self.mode
        }

        fn spawn(&self, _mode: FsMountMode, on_exit: ExitNotice) -> Result<(), FuseFsError> {
            let entered = self.entered.lock().unwrap().take();
            if let Some(entered) = entered {
                let _ = entered.send(());
            }
            let release = self.release.lock().unwrap().take();
            if let Some(release) = release {
                let _ = release.recv();
            }
            self.spawns.fetch_add(1, Ordering::SeqCst);
            if self.fail_spawn.load(Ordering::SeqCst) {
                return Err(FuseFsError::Spawn {
                    path: PathBuf::from("missing-helper"),
                    source: io::Error::from(io::ErrorKind::NotFound),
                });
            }
            *self.started.lock().unwrap() = Some(start_helper(&self.socket, &self.mount_root));
            *self.exit_notice.lock().unwrap() = Some(on_exit);
            Ok(())
        }
    }

    /// Fields drop in order: the started helper (in `launcher`) stops while
    /// its socket still exists, then the folder goes. A helper whose socket
    /// is already gone can't be woken to stop, and its drop waits forever.
    struct Rig {
        supervisor: FuseFsSupervisor,
        launcher: Arc<FakeLauncher>,
        socket: PathBuf,
        mount_root: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn rig(mode: FsMountMode) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("fusefs.sock");
        let mount_root = dir.path().join("mnt");
        let launcher = Arc::new(FakeLauncher {
            mode,
            socket: socket.clone(),
            mount_root: mount_root.clone(),
            spawns: AtomicU32::new(0),
            started: Mutex::new(None),
            fail_spawn: AtomicBool::new(false),
            exit_notice: Mutex::new(None),
            entered: Mutex::new(None),
            release: Mutex::new(None),
        });
        let supervisor =
            FuseFsSupervisor::with_launcher(
                socket.clone(),
                mount_root.clone(),
                launcher.clone(),
                Arc::new(AtomicBool::new(false)),
            );
        supervisor.use_topology(Arc::new(|| TopologyWire {
            local_machine_id: "m1".into(),
            ..TopologyWire::default()
        }));
        Rig {
            _dir: dir,
            supervisor,
            launcher,
            socket,
            mount_root,
        }
    }

    #[test]
    fn the_mount_root_is_the_one_given() {
        let rig = rig(FsMountMode::FsKit);
        assert_eq!(rig.supervisor.mount_root(), rig.mount_root);
    }

    #[test]
    fn a_live_helper_is_adopted_and_nothing_is_started() {
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);

        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 0);
        assert_eq!(helper.backend.mounts.load(Ordering::SeqCst), 1);
        let updates = helper.updates.lock().unwrap();
        assert!(matches!(
            updates.first(),
            Some(HostUpdate::LinkState {
                state: DaemonLinkState::Connected { .. }
            })
        ));
        assert!(
            updates
                .iter()
                .any(|update| matches!(update, HostUpdate::Topology { .. }))
        );
        drop(updates);
        helper.server.shutdown();
    }

    #[test]
    fn turning_it_on_with_no_helper_starts_one_and_mounts() {
        let rig = rig(FsMountMode::FsKit);

        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 1);
        let started = rig.launcher.started.lock().unwrap();
        let helper = started.as_ref().expect("the helper was started");
        assert_eq!(helper.backend.mounts.load(Ordering::SeqCst), 1);
        assert!(rig.mount_root.is_dir());
    }

    #[test]
    fn a_degraded_machine_gets_a_helper_but_no_mount() {
        let rig = rig(FsMountMode::Degraded);

        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 1);
        let started = rig.launcher.started.lock().unwrap();
        let helper = started.as_ref().expect("the helper was started");
        assert_eq!(helper.backend.mounts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_good_mount_is_mounted_and_turning_it_off_is_not() {
        let rig = rig(FsMountMode::FsKit);
        assert!(!rig.supervisor.is_mounted());

        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(rig.supervisor.is_mounted());

        rig.supervisor.set_enabled(false);
        rig.supervisor.wait_idle();
        assert!(!rig.supervisor.is_mounted());
    }

    #[test]
    fn a_degraded_machine_is_not_mounted() {
        let rig = rig(FsMountMode::Degraded);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(!rig.supervisor.is_mounted());
    }

    #[test]
    fn a_helper_that_fails_to_start_is_not_mounted() {
        let rig = rig(FsMountMode::FsKit);
        rig.launcher.fail_spawn.store(true, Ordering::SeqCst);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(!rig.supervisor.is_mounted());
    }

    #[test]
    fn a_refused_mount_is_not_mounted() {
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);
        helper.backend.refuse_mount.store(true, Ordering::SeqCst);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(!rig.supervisor.is_mounted());
        helper.server.shutdown();
    }

    #[test]
    fn turning_it_off_unmounts_the_helper() {
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();

        rig.supervisor.set_enabled(false);
        rig.supervisor.wait_idle();

        assert_eq!(helper.backend.unmounts.load(Ordering::SeqCst), 1);
        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 0);
        helper.server.shutdown();
    }

    #[test]
    fn off_with_no_helper_starts_nothing() {
        let rig = rig(FsMountMode::FsKit);

        rig.supervisor.set_enabled(false);
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 0);
        assert!(!rig.socket.exists());
    }

    #[test]
    fn a_topology_push_reaches_a_live_helper_and_is_dropped_when_off() {
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);
        rig.supervisor.push_topology();
        rig.supervisor.wait_idle();
        assert!(helper.updates.lock().unwrap().is_empty());

        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        let before = helper.updates.lock().unwrap().len();
        rig.supervisor.push_topology();
        rig.supervisor.wait_idle();
        assert_eq!(helper.updates.lock().unwrap().len(), before + 1);
        helper.server.shutdown();
    }

    #[test]
    fn a_helper_that_exits_is_started_again_and_mounted_again() {
        let rig = rig(FsMountMode::FsKit);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(rig.supervisor.is_mounted());

        let (entered, release) = rig.launcher.block_next_spawn();
        rig.launcher.helper_dies();
        assert!(!rig.supervisor.is_mounted());
        entered.recv().unwrap();
        assert!(!rig.supervisor.is_mounted());
        release.send(()).unwrap();
        rig.supervisor.wait_idle();

        assert!(rig.supervisor.is_mounted());
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Mounted);
        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_helper_that_exits_and_cannot_come_back_says_why() {
        let rig = rig(FsMountMode::FsKit);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();

        rig.launcher.fail_spawn.store(true, Ordering::SeqCst);
        rig.launcher.helper_dies();
        rig.supervisor.wait_idle();

        assert!(!rig.supervisor.is_mounted());
        assert!(matches!(
            rig.supervisor.mount_status(),
            FuseMountStatus::CannotSetUp { reason } if !reason.is_empty()
        ));
    }

    #[test]
    fn a_helper_that_keeps_dying_is_started_only_once_more() {
        let rig = rig(FsMountMode::FsKit);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();

        rig.launcher.helper_dies();
        rig.supervisor.wait_idle();
        assert!(rig.supervisor.is_mounted());
        rig.launcher.helper_dies();
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 2);
        assert!(!rig.supervisor.is_mounted());
        assert!(matches!(
            rig.supervisor.mount_status(),
            FuseMountStatus::CannotSetUp { .. }
        ));
    }

    #[test]
    fn the_exit_of_a_helper_that_is_switched_off_leaves_it_off() {
        let rig = rig(FsMountMode::FsKit);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        rig.supervisor.set_enabled(false);
        rig.supervisor.wait_idle();

        rig.launcher.helper_dies();
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 1);
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Off);
    }

    #[test]
    fn an_adopted_helper_that_dies_is_noticed_by_a_failed_push() {
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(rig.supervisor.is_mounted());
        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 0);

        helper.server.shutdown();
        rig.supervisor.push_topology();
        rig.supervisor.wait_idle();

        assert_eq!(rig.launcher.spawns.load(Ordering::SeqCst), 1);
        assert!(rig.supervisor.is_mounted());
    }

    #[test]
    fn one_failed_push_does_not_stop_the_next_one() {
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        let before = helper.updates.lock().unwrap().len();

        helper.fail_updates.store(1, Ordering::SeqCst);
        rig.supervisor.push_topology();
        rig.supervisor.wait_idle();
        assert_eq!(helper.updates.lock().unwrap().len(), before);
        assert!(rig.supervisor.is_mounted());

        rig.supervisor.push_topology();
        rig.supervisor.wait_idle();
        assert_eq!(helper.updates.lock().unwrap().len(), before + 1);
        helper.server.shutdown();
    }

    #[test]
    fn on_then_off_ends_off() {
        let rig = rig(FsMountMode::FsKit);
        let (entered, release) = rig.launcher.block_next_spawn();

        rig.supervisor.set_enabled(true);
        entered.recv().unwrap();
        rig.supervisor.set_enabled(false);
        release.send(()).unwrap();
        rig.supervisor.wait_idle();

        assert!(!rig.supervisor.is_mounted());
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Off);
        let started = rig.launcher.started.lock().unwrap();
        let helper = started.as_ref().expect("the helper was started");
        assert_eq!(
            helper.backend.mounts.load(Ordering::SeqCst),
            helper.backend.unmounts.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn off_then_on_ends_on() {
        let rig = rig(FsMountMode::FsKit);
        let (entered, release) = rig.launcher.block_next_spawn();

        rig.supervisor.set_enabled(true);
        entered.recv().unwrap();
        rig.supervisor.set_enabled(false);
        rig.supervisor.set_enabled(true);
        release.send(()).unwrap();
        rig.supervisor.wait_idle();

        assert!(rig.supervisor.is_mounted());
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Mounted);
    }

    #[test]
    fn the_status_follows_the_mount() {
        let rig = rig(FsMountMode::FsKit);
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Off);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Mounted);
        rig.supervisor.set_enabled(false);
        rig.supervisor.wait_idle();
        assert_eq!(rig.supervisor.mount_status(), FuseMountStatus::Off);
    }

    #[test]
    fn each_change_of_the_status_is_announced_once() {
        let rig = rig(FsMountMode::FsKit);
        let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        rig.supervisor.on_status_change({
            let told = told.clone();
            Arc::new(move || {
                told.fetch_add(1, Ordering::SeqCst);
            })
        });
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        // Off to Starting, then Starting to Mounted.
        assert_eq!(told.load(Ordering::SeqCst), 2);
        rig.supervisor.set_enabled(false);
        rig.supervisor.wait_idle();
        assert_eq!(told.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_wire_status_carries_the_folder_when_mounted_and_the_reason_when_not() {
        use pb::fuse_mount_status::State;
        let rig = rig(FsMountMode::FsKit);
        assert_eq!(rig.supervisor.mount_status_wire().state, State::Off as i32);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        let mounted = rig.supervisor.mount_status_wire();
        assert_eq!(mounted.state, State::Mounted as i32);
        assert_eq!(mounted.mount_root, rig.mount_root.to_string_lossy());

        let broken = self::rig(FsMountMode::Degraded);
        broken.supervisor.set_enabled(true);
        broken.supervisor.wait_idle();
        let wire = broken.supervisor.mount_status_wire();
        assert_eq!(wire.state, State::CannotSetUp as i32);
        assert!(!wire.reason.is_empty());
    }

    #[test]
    fn a_machine_that_cannot_mount_says_so() {
        let rig = rig(FsMountMode::Degraded);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(matches!(
            rig.supervisor.mount_status(),
            FuseMountStatus::CannotSetUp { .. }
        ));
    }

    #[test]
    fn a_failed_start_says_why() {
        let rig = rig(FsMountMode::FsKit);
        rig.launcher.fail_spawn.store(true, Ordering::SeqCst);
        rig.supervisor.set_enabled(true);
        rig.supervisor.wait_idle();
        assert!(matches!(
            rig.supervisor.mount_status(),
            FuseMountStatus::CannotSetUp { .. }
        ));
    }

    #[test]
    fn the_helper_socket_a_real_helper_binds_is_private() {
        // The socket the supervisor talks to is the control server's own.
        let rig = rig(FsMountMode::FsKit);
        let helper = start_helper(&rig.socket, &rig.mount_root);
        let mode = std::fs::metadata(&rig.socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        helper.server.shutdown();
    }

    #[test]
    fn topology_from_store_projects_empty_machine_session_under_local_machine() {
        let dir = tempfile::tempdir().unwrap();
        let store = humata2_store::Store::open(dir.path().join("state")).unwrap();
        store
            .put_session(&humata2_store::SessionRecord {
                id: "se_moved".into(),
                channel: "ch_1".into(),
                machine: String::new(),
                title: "moved".into(),
                agent: "shell".into(),
                folder: dir.path().join("se_moved"),
                created_unix_ms: 1,
            })
            .unwrap();
        let root = dir.path().join("root");
        let wire = topology_from_store(&store, "ma_local", &root);

        assert!(
            wire.machines.iter().all(|m| !m.machine_id.is_empty()),
            "no empty machine_id entry in machines: {:?}",
            wire.machines
        );
        assert_eq!(wire.machines.len(), 1);
        assert_eq!(wire.machines[0].machine_id, "ma_local");
        assert!(wire.machines[0].online);
        assert_eq!(wire.machines[0].machine_root, root);

        assert_eq!(wire.sessions.len(), 1);
        assert_eq!(wire.sessions[0].session_id, "se_moved");
        assert_eq!(wire.sessions[0].machine_id, "ma_local");
        assert_eq!(wire.sessions[0].machine_root, root);
    }
}
