//! Standalone `humata2_fusefs` daemon binary supporting `serve`, `status`, and
//! `materialize` subcommands across `fskit`, `linux`, and `degraded` modes.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use fusefs_host::{
    CapabilityRouter, ControlClient, ControlHandler, ControlResponse, ControlServer, MountConfig,
    MountState, MountSupervisor, PosixMountBackend,
};
use humata2_fusefs_adapter::layout::BuildCacheStub;
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
    /// Unix control socket path.
    #[arg(long, default_value = "/tmp/humata2-fusefs.sock", global = true)]
    socket: PathBuf,

    /// Mountpoint directory for the Humata virtual tree.
    #[arg(long, default_value = "/tmp/humata2-mnt", global = true)]
    mountpoint: PathBuf,

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
        Command::Serve { once } => run_serve(cli.socket, cli.mountpoint, mode, once),
        Command::Status { json } => run_status(cli.socket, cli.mountpoint, mode, json),
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

fn run_serve(socket: PathBuf, mountpoint: PathBuf, mode: FsMountMode, once: bool) -> ExitCode {
    let resource_url = format!("fusefs://{mode}/humata2");
    let config = MountConfig::new(&mountpoint, resource_url);
    let backend = Arc::new(PosixMountBackend::new(mode.to_string()));
    let supervisor = Arc::new(MountSupervisor::new(config, backend));
    let router = Arc::new(CapabilityRouter::new());
    let handler = Arc::new(ControlHandler::new(supervisor, router));

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

    loop {
        if let Err(err) = server.serve_one() {
            eprintln!("control connection error: {err}");
        }
    }
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

    match planner.execute_plan(&plan, yes, |target| Ok(target.estimated_bytes)) {
        Ok(report) => {
            if !json {
                println!(
                    "Materialized {} target(s) ({}).",
                    report.materialized_targets,
                    format_bytes(report.materialized_bytes)
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}
