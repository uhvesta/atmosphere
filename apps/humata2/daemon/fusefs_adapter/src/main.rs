//! Standalone `humata2_fusefs` daemon binary supporting `serve`, `status`, and
//! `materialize` subcommands across `fskit`, `linux`, and `degraded` modes.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use fusefs_host::{
    CapabilityRouter, ControlClient, ControlHandler, ControlResponse, ControlServer,
    ExtensionCallback, MountBackend, MountConfig, MountState, MountSupervisor, PosixMountBackend,
};
use humata2_fusefs_adapter::layout::{
    BuildCacheStub, DaemonLinkState, HostUpdate, HumataTopology, HumataVolumeHost,
};
use humata2_fusefs_adapter::materialize::{
    DEFAULT_CONFIRMATION_THRESHOLD_BYTES, FsMountMode, FsStatusSummary, MaterializePlanner,
    format_bytes,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliMode {
    Fskit,
    Linux,
    Degraded,
}

impl From<CliMode> for FsMountMode {
    fn from(value: CliMode) -> Self {
        match value {
            CliMode::Fskit => FsMountMode::FsKit,
            CliMode::Linux => FsMountMode::Linux,
            CliMode::Degraded => FsMountMode::Degraded,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "humata2_fusefs",
    about = "Humata 2 FUSEFS virtual filesystem daemon & CLI"
)]
struct Cli {
    /// Unix control socket path (required by `serve` and `status`). There is
    /// no default: a path in a shared folder like /tmp would let other users
    /// reach the mount. humatad2 passes a path in its private state folder.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    /// Mountpoint directory for the Humata virtual tree (required by `serve`).
    #[arg(long, global = true)]
    mountpoint: Option<PathBuf>,

    /// Optional Unix socket path for the macOS FSKit `.appex` bridge server.
    #[arg(long, global = true)]
    bridge_socket: Option<PathBuf>,

    /// Filesystem backend mode (`fskit`, `linux`, or `degraded`).
    #[arg(long, value_enum, default_value_t = default_cli_mode(), global = true)]
    mode: CliMode,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the FUSEFS control server and mount supervisor.
    Serve {
        /// Serve a single control connection and exit (useful for smoke tests).
        #[arg(long)]
        once: bool,
    },
    /// Show mount, cache, lease, and AWAY status.
    Status {
        /// Emit machine-readable JSON instead of human text.
        #[arg(long)]
        json: bool,
    },
    /// Estimate and explicitly materialize omitted BuildCache directories.
    Materialize {
        /// Target virtual path or path to a `.humata-omitted.json` stub.
        path: String,
        /// Estimated bytes when no stub file is present on disk.
        #[arg(long, default_value_t = 0)]
        bytes: u64,
        /// Estimated entry count when no stub file is present on disk.
        #[arg(long, default_value_t = 0)]
        entries: u64,
        /// Byte threshold requiring explicit `--yes` confirmation.
        #[arg(long, default_value_t = DEFAULT_CONFIRMATION_THRESHOLD_BYTES)]
        threshold_bytes: u64,
        /// Explicitly confirm materialization above the byte threshold.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Emit machine-readable JSON output.
        #[arg(long)]
        json: bool,
    },
}

fn default_cli_mode() -> CliMode {
    #[cfg(target_os = "macos")]
    {
        CliMode::Fskit
    }
    #[cfg(target_os = "linux")]
    {
        CliMode::Linux
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        CliMode::Degraded
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mode: FsMountMode = cli.mode.into();

    match cli.command {
        Command::Serve { once } => {
            let (Some(socket), Some(mountpoint)) = (cli.socket, cli.mountpoint) else {
                eprintln!(
                    "humata2_fusefs serve needs --socket and --mountpoint; pass a socket in a folder only you can open, and the folder to mount"
                );
                return ExitCode::from(2);
            };
            run_serve(socket, mountpoint, cli.bridge_socket, mode, once)
        }
        Command::Status { json } => {
            let Some(socket) = cli.socket else {
                eprintln!(
                    "humata2_fusefs status needs --socket; pass the socket humatad2 started the helper with (<state>/fusefs.sock)"
                );
                return ExitCode::from(2);
            };
            run_status(socket, cli.mountpoint.unwrap_or_default(), mode, json)
        }
        Command::Materialize {
            path,
            bytes,
            entries,
            threshold_bytes,
            yes,
            json,
        } => run_materialize(path, bytes, entries, threshold_bytes, yes, json),
    }
}

fn run_serve(
    socket: PathBuf,
    mountpoint: PathBuf,
    bridge_socket: Option<PathBuf>,
    mode: FsMountMode,
    once: bool,
) -> ExitCode {
    let host = match HumataVolumeHost::new(
        std::process::id(),
        HumataTopology::default(),
        None,
        None,
        None,
    ) {
        Ok(host) => Arc::new(host),
        Err(err) => {
            eprintln!("failed to build the Humata volume: {err}");
            return ExitCode::FAILURE;
        }
    };
    // No daemon has connected yet; humatad2 pushes its link state on connect.
    host.set_daemon_link(DaemonLinkState::Disconnected {
        last_daemon_pid: None,
        epoch: 0,
    });
    let router = Arc::new(CapabilityRouter::new());
    let (resource_url, backend) = mount_backend(mode, &host, &router, &socket, bridge_socket);
    let config = MountConfig::new(&mountpoint, resource_url);
    let supervisor = Arc::new(MountSupervisor::new(config, backend));
    let handler = Arc::new(
        ControlHandler::new(supervisor, router).with_extension(extension_callback(host)),
    );

    let server = match ControlServer::bind(&socket, handler) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("failed to bind control socket '{}': {err}", socket.display());
            return ExitCode::FAILURE;
        }
    };

    println!(
        "humata2_fusefs serving mode={} mountpoint={} socket={}",
        mode,
        mountpoint.display(),
        socket.display()
    );

    if once {
        if let Err(err) = server.serve_one() {
            eprintln!("serve_one error: {err}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // Connections are served on their own threads, so a pushed update never
    // waits behind another client. This thread has nothing left to do.
    let _handle = server.spawn();
    loop {
        std::thread::park();
    }
}

/// On Linux the helper mounts and serves the volume itself through
/// `fusermount3`; on macOS it starts the FSKit bridge server and mounts
/// `humatafs://humata2`; everywhere else the system mount tool does it.
#[cfg(target_os = "linux")]
fn mount_backend(
    mode: FsMountMode,
    host: &Arc<HumataVolumeHost>,
    _router: &Arc<CapabilityRouter>,
    _control_socket: &PathBuf,
    _bridge_socket: Option<PathBuf>,
) -> (String, Arc<dyn MountBackend>) {
    let url = format!("fusefs://{mode}/humata2");
    if mode == FsMountMode::Linux {
        return (
            url,
            Arc::new(humata2_fusefs_adapter::linux_mount::LinuxFuseBackend::new(
                host.clone(),
            )),
        );
    }
    (url, Arc::new(PosixMountBackend::new(mode.to_string())))
}

#[cfg(target_os = "macos")]
fn mount_backend(
    mode: FsMountMode,
    host: &Arc<HumataVolumeHost>,
    router: &Arc<CapabilityRouter>,
    control_socket: &PathBuf,
    bridge_socket: Option<PathBuf>,
) -> (String, Arc<dyn MountBackend>) {
    if mode == FsMountMode::FsKit {
        let sock = bridge_socket.unwrap_or_else(|| {
            humata2_fusefs_adapter::macos_mount::default_bridge_socket(control_socket)
        });
        let url = fusefs_fskit::MountSpec::for_humatafs_with_socket(
            humata2_fusefs_adapter::macos_mount::HUMATA_FSKIT_AUTHORITY,
            &sock,
            "/tmp",
        )
        .map(|s| s.resource_url)
        .unwrap_or_else(|_| {
            format!(
                "{}://{}",
                humata2_fusefs_adapter::macos_mount::HUMATA_FSKIT_FS_TYPE,
                humata2_fusefs_adapter::macos_mount::HUMATA_FSKIT_AUTHORITY
            )
        });
        return (
            url,
            Arc::new(humata2_fusefs_adapter::macos_mount::MacFsKitBackend::new(
                host.clone(),
                router.clone(),
                sock,
            )),
        );
    }
    (
        format!("fusefs://{mode}/humata2"),
        Arc::new(PosixMountBackend::new(mode.to_string())),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn mount_backend(
    mode: FsMountMode,
    _host: &Arc<HumataVolumeHost>,
    _router: &Arc<CapabilityRouter>,
    _control_socket: &PathBuf,
    _bridge_socket: Option<PathBuf>,
) -> (String, Arc<dyn MountBackend>) {
    (
        format!("fusefs://{mode}/humata2"),
        Arc::new(PosixMountBackend::new(mode.to_string())),
    )
}

/// Decodes the updates humatad2 pushes and applies them to the volume host.
fn extension_callback(host: Arc<HumataVolumeHost>) -> ExtensionCallback {
    Arc::new(move |body: serde_json::Value| {
        let update: HostUpdate = serde_json::from_value(body).map_err(|err| {
            format!("unrecognised update from humatad2 (is it the same release as this helper?): {err}")
        })?;
        update
            .apply(&host)
            .map_err(|err| format!("couldn't apply the update to the Humata volume: {err}"))?;
        Ok(serde_json::json!({ "applied": true }))
    })
}

fn run_status(socket: PathBuf, mountpoint: PathBuf, mode: FsMountMode, json: bool) -> ExitCode {
    let client = ControlClient::new(&socket);
    let (mount_state, daemon_connected, nobrowse) = match client.status() {
        Ok(ControlResponse::MountStatus { mount }) => (mount.state, true, mount.nobrowse),
        _ => (MountState::Unmounted, false, true),
    };

    let summary = FsStatusSummary {
        mode,
        mount_state,
        mountpoint,
        nobrowse,
        host_reachable: daemon_connected,
        daemon_connected,
        active_sessions: 0,
        active_leases: 0,
        pending_requests: 0,
        cached_chunks: 0,
        cached_bytes: 0,
        cache_budget_bytes: 2 * 1024 * 1024 * 1024,
        queued_invalidations: 0,
        away_journal_entries: 0,
    };

    if json {
        match serde_json::to_string_pretty(&summary) {
            Ok(s) => println!("{s}"),
            Err(err) => {
                eprintln!("failed to format status JSON: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("{}", summary.format_human());
    }
    ExitCode::SUCCESS
}

fn run_materialize(
    path: String,
    bytes: u64,
    entries: u64,
    threshold_bytes: u64,
    yes: bool,
    json: bool,
) -> ExitCode {
    let stub = match std::fs::read(&path) {
        Ok(raw) => match BuildCacheStub::from_json_bytes(&raw) {
            Ok(parsed) => parsed,
            Err(_) => BuildCacheStub::build_cache(&path, bytes, entries),
        },
        Err(_) => BuildCacheStub::build_cache(&path, bytes, entries),
    };

    let planner = MaterializePlanner::new(threshold_bytes);
    let plan = planner.plan_from_stubs(&[(path.clone(), stub)]);

    if json {
        if let Ok(s) = serde_json::to_string_pretty(&plan) {
            println!("{s}");
        }
    } else {
        println!(
            "Materialize plan for {}: {} ({} entries, confirmation_required={})",
            path,
            format_bytes(plan.total_bytes),
            plan.total_entries,
            plan.requires_confirmation
        );
    }

    // No volume is mounted yet, so nothing can be copied: report the plan,
    // then fail plainly instead of claiming the bytes were materialized.
    let _ = yes;
    eprintln!(
        "materialize is not implemented yet: nothing was copied. The plan above is what it would fetch once the FUSEFS volume serves files."
    );
    ExitCode::from(3)
}
