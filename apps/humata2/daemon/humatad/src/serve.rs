//! `humatad2 serve`: opens the state, binds the socket and serves until a
//! stop signal. Terminals live in their holders, so they outlive this.
//! With `--stdio` it serves the peer link instead (`peer_link`), and no
//! socket.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_chunk::Link;
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Builds, Bundle, MANIFEST, Machines, MachinesConfig, Platform, RetryPolicy};
use humata2_store::Store;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tonic::service::InterceptorLayer;
use tonic::transport::Server;
use tower::util::MapRequestLayer;

use crate::access_service::Access;
use crate::board_service::BoardSvc;
use crate::channel_service::ChannelSvc;
use crate::cli_login::{CliAuth, CliLogins};
use crate::carry::Carry;
use crate::carry_service::SessionCarrySvc;
use crate::compression::Compression;
use crate::daemon::Daemon;
use crate::error::DaemonError;
use crate::events::{EventSvc, Feed};
use crate::files::{Files, Roots};
use crate::guard::Guard;
use crate::identity::{self, Authenticate, Identities};
use crate::inbox::Inbox;
use crate::last_good;
use crate::machine_service::{self, MachineSvc, Work};
use crate::provider_service::ProviderSvc;
use crate::not_built;
use crate::operations::{OperationSvc, Operations};
use crate::paste::PASTE_MAX_BYTES;
use crate::peer_link;
use crate::relay_auth::RelayAuth;
use crate::restart;
use crate::remote_terminal::RemoteTerminal;
use crate::remote_work::{RemoteWork, StoreMemory};
use crate::session_service::{SessionSvc, Tell, TellSink};
use crate::sync;
use crate::terminal::{Terminals, TerminalsConfig};
use crate::terminal_session::Kernel;
use crate::transfer_end_service::TransferEndSvc;
use crate::transfer_service::TransferSvc;
use crate::trash_service::{Bin, TrashSvc};

/// What Unix sockets allow in a path, without its terminating NUL.
#[cfg(target_os = "linux")]
pub(crate) const SOCKET_PATH_MAX: usize = 107;
#[cfg(not(target_os = "linux"))]
pub(crate) const SOCKET_PATH_MAX: usize = 103;

/// How long a stopping daemon gives its connections to finish once its
/// event subscriptions have ended, before it cuts them and exits.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// The file a daemon locks in its state folder. Its pid is written in it.
const LOCK_FILE: &str = "daemon.lock";

/// Room for a pasted image and the message around it.
const DECODE_MAX_BYTES: usize = PASTE_MAX_BYTES + (1 << 20);

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Role {
    /// The Mac: the source of truth.
    Host,
    /// Any other machine.
    Machine,
}

#[derive(clap::Args)]
pub(crate) struct ServeArgs {
    /// The socket to serve on, made 0600 in a folder only you can open.
    /// Default: ~/.humata2/humatad.sock. With --stdio, the machine's daemon
    /// socket the link relays the host's calls to (D139, D146).
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Serve the peer link on stdin and stdout instead of a socket: what
    /// the Mac runs over SSH or a provider's connect command (MP-7). It
    /// serves PeerService and what the Mac relays, takes every call for
    /// the Mac's, and ends when stdin closes. A peer link is always a
    /// machine's, whatever --role says. It relays what the host may call to
    /// the machine's daemon on --socket, or on the default socket when
    /// neither --socket nor --state is given; with --state and no --socket
    /// it relays nothing (D146).
    #[arg(long)]
    stdio: bool,
    /// With --stdio, print `HUMATA_SYNC_<nonce>` on a line of its own
    /// before the stream, so a terminal Humata took over can tell where
    /// the stream starts (MP-15). Letters and digits only.
    #[arg(long, requires = "stdio", value_parser = sync::parse_nonce)]
    sync: Option<String>,
    /// Where the daemon keeps its state. Default: ~/.humata2/state.
    #[arg(long)]
    state: Option<PathBuf>,
    /// The folder a machine shell starts in. Default: your home folder.
    #[arg(long)]
    root: Option<PathBuf>,
    /// The humata2_holder binary. Default: the one beside humatad2.
    #[arg(long)]
    holder: Option<PathBuf>,
    /// The humata binary the agents' turn hooks run (D169). Default: the
    /// one beside humatad2.
    #[arg(long)]
    humata: Option<PathBuf>,
    /// The tools bundle machines get their humatad2, humata2_holder and
    /// humata from (TS-2): a folder with a manifest.json. Default: the
    /// `tools` folder beside humatad2 when it has one, else this host's
    /// own three files beside humatad2 (machines of this platform only).
    #[arg(long)]
    tools: Option<PathBuf>,
    /// The home folder the host's agent skill folders (.claude/skills,
    /// .codex/skills, .copilot/skills) are read from. Default: $HOME.
    #[arg(long)]
    skills_home: Option<PathBuf>,
    /// The ssh config SSH machines' aliases are read from. Default:
    /// ~/.ssh/config.
    #[arg(long)]
    ssh_config: Option<PathBuf>,
    /// The shell new shell tabs run. Default: $SHELL, else /bin/sh.
    #[arg(long)]
    shell: Option<PathBuf>,
    /// An argument for the shell; repeat for more. Default: -l.
    #[arg(long = "shell-arg", allow_hyphen_values = true)]
    shell_args: Vec<OsString>,
    #[arg(long, value_enum, default_value_t = Role::Host)]
    role: Role,
    /// How much of each tab's output its holder keeps.
    #[arg(long, default_value_t = 8 << 20)]
    scrollback_bytes: usize,
    /// An open pipe to write a byte to, then close, once the socket
    /// listens: how `humatad2 bridge` hears that a daemon it started is
    /// ready. Not for people.
    #[arg(long, hide = true, conflicts_with = "stdio", value_parser = clap::value_parser!(i32).range(3..))]
    ready_fd: Option<i32>,
    /// Test daemons only: a call with no credential acts as the user.
    /// Builds without the `admin` feature, as releases are, lack this flag.
    /// Never with --stdio: the peer link is the Mac's, not the user's.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with = "stdio")]
    admin: bool,
    /// Test daemons only: as --admin, but only for calls from this
    /// process, so a test can play the app while every other process is
    /// judged as a release daemon judges it.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with_all = ["stdio", "admin"])]
    admin_pid: Option<i32>,
    /// Test daemons only: every CLI login lasts this many seconds, not the
    /// hours it asked for, so tests can see one expire.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with = "stdio")]
    cli_login_seconds: Option<u64>,
    /// How long every script token lasts instead of the days it asked for, so tests can see one expire.
    #[cfg(feature = "admin")]
    #[arg(long, conflicts_with = "stdio")]
    script_token_seconds: Option<u64>,
    /// The folder every channel's folder is made in (FC-1). Default:
    /// Humata inside --root.
    #[arg(long)]
    channels: Option<PathBuf>,
    /// A JSON list of agents (name, program, start_args, resume_args,
    /// conversation_file) that replaces the built-in claude. For tests.
    #[arg(long, hide = true)]
    agents: Option<PathBuf>,
    /// The agent a new channel starts with.
    #[arg(long, default_value = "claude")]
    default_agent: String,
    /// Where repositories are cloned from, with {owner} and {name}. For tests.
    #[arg(long, hide = true, default_value = humata2_sessions::DEFAULT_REMOTE_TEMPLATE)]
    remote_template: String,
}

/// `~/.humata2`, where the daemon lives unless told otherwise.
fn home_folder() -> Result<PathBuf, DaemonError> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".humata2"))
        .ok_or(DaemonError::NoHome)
}

pub(crate) fn default_socket() -> Result<PathBuf, DaemonError> {
    Ok(home_folder()?.join("humatad.sock"))
}

pub(crate) async fn run(args: ServeArgs) -> Result<(), DaemonError> {
    // SAFETY: the process that started this one opened the descriptor for
    // it alone, and nothing else here takes it. Owning it from the start
    // closes it on any way out, which tells the bridge this one won't
    // listen.
    let ready = args
        .ready_fd
        .map(|fd| File::from(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }));
    // D146: a link given its own state folder reaches no socket unless told.
    let state_given = args.state.is_some();
    let state = match args.state {
        Some(state) => state,
        None => home_folder()?.join("state"),
    };
    if args.stdio {
        private_folder(&state)?;
        private_folder(&state.join("logs"))?;
        let machine = machine_id(&state.join("machine-id"))?;
        let socket = match (args.socket, state_given) {
            (Some(socket), _) => Some(socket),
            (None, false) => Some(default_socket()?),
            // A test's state folder: never reach the real socket.
            (None, true) => None,
        };
        return peer_link::run(&state, machine, args.sync.as_deref(), socket).await;
    }
    let socket = match args.socket {
        Some(socket) => socket,
        None => default_socket()?,
    };
    let root = match args.root {
        Some(root) => root,
        None => std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(DaemonError::NoHome)?,
    };
    let holder = match args.holder {
        Some(holder) => holder,
        None => beside_this_binary("humata2_holder"),
    };
    if !holder.is_file() {
        return Err(DaemonError::NoHolder { path: holder });
    }
    let shell = args
        .shell
        .or_else(|| {
            std::env::var_os("SHELL")
                .filter(|shell| !shell.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let shell_args = if args.shell_args.is_empty() {
        vec![OsString::from("-l")]
    } else {
        args.shell_args
    };

    private_folder(&state)?;
    // Paths handed to clients and holders are absolute and resolved.
    let state = std::fs::canonicalize(&state).map_err(|source| DaemonError::CreateDir {
        path: state.clone(),
        source,
    })?;
    let holders = state.join("holders");
    let logs = state.join("logs");
    crate::terminal::check_holder_sockets(&holders)?;
    private_folder(&holders)?;
    private_folder(&logs)?;
    // Locked before the socket is bound or the state touched, so a second
    // daemon on this state stops here, and it can't take the first one's
    // socket. The lock lasts as long as this function does.
    let _state_lock = lock_state(&state)?;
    // Bound before the store is opened, so a second daemon on the same
    // socket stops here; and only the daemon that has the socket keeps
    // the log.
    let (listener, socket_id) = bind(&socket)?;
    let daemon_log = crate::log::start(&logs);
    let (store, restored) = Store::open_or_restore(state.join("store"))
        .map_err(|source| DaemonError::Store { source })?;
    let store = Arc::new(store);
    let machine = machine_id(&state.join("machine-id"))?;
    let role = match args.role {
        Role::Host => pb::DaemonRole::Host,
        Role::Machine => pb::DaemonRole::Machine,
    };

    let identities = Arc::new(Identities::load(
        state.join("session-tokens"),
        &store,
        &machine,
    )?);
    #[cfg(feature = "admin")]
    let login_lifetime = args.cli_login_seconds.map(std::time::Duration::from_secs);
    #[cfg(not(feature = "admin"))]
    let login_lifetime = None;
    let logins = Arc::new(CliLogins::new(Box::new(Kernel), login_lifetime));
    #[cfg(feature = "admin")]
    let script_lifetime = args.script_token_seconds.map(std::time::Duration::from_secs);
    #[cfg(not(feature = "admin"))]
    let script_lifetime = None;
    let scripts = Arc::new(
        crate::script_tokens::ScriptTokens::load(store.clone(), script_lifetime)
            .map_err(|source| DaemonError::ScriptTokens { source })?,
    );
    #[cfg(feature = "admin")]
    let authenticate = match (args.admin, args.admin_pid) {
        (true, _) => {
            crate::log::say!(
                "humatad2: warning: admin mode: a call with no credential acts as the user. This is a test daemon; don't use it as your real one"
            );
            Authenticate::admin(
                identities.clone(),
                logins.clone(),
                scripts.clone(),
                store.clone(),
                Arc::new(Kernel),
                role,
                identity::Admin::Anyone,
            )
        }
        (false, Some(pid)) => {
            crate::log::say!(
                "humatad2: warning: admin mode: a call with no credential from process {pid} acts as the user. This is a test daemon; don't use it as your real one"
            );
            Authenticate::admin(
                identities.clone(),
                logins.clone(),
                scripts.clone(),
                store.clone(),
                Arc::new(Kernel),
                role,
                identity::Admin::Process(pid),
            )
        }
        (false, None) => Authenticate::new(
            identities.clone(),
            logins.clone(),
            scripts.clone(),
            store.clone(),
            Arc::new(Kernel),
            role,
        ),
    };
    #[cfg(not(feature = "admin"))]
    let authenticate = Authenticate::new(
        identities.clone(),
        logins.clone(),
        scripts.clone(),
        store.clone(),
        Arc::new(Kernel),
        role,
    );
    // D144: a connection that completes the relay's proof is the host's.
    // The session check inside it uses the interceptor as it is now, with
    // no relay, so there is no cycle.
    let relay = Arc::new(RelayAuth::new(
        args.role,
        state.join(crate::link_proof::KEY_FILE),
        rustix::process::geteuid().as_raw(),
        {
            let plain = authenticate.clone();
            Arc::new(move |pid| plain.in_a_session(Some(pid)))
        },
    ));
    let authenticate = authenticate.with_relay(relay.clone());
    let guard = Arc::new(Guard::new(&machine, role));
    crate::rules_kept::restore(&store, &guard).map_err(DaemonError::RulesKept)?;

    let channels_folder = args.channels.clone().unwrap_or_else(|| root.join("Humata"));
    let agents = match &args.agents {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|source| DaemonError::AgentsRead {
                path: path.clone(),
                source,
            })?;
            humata2_sessions::Agents::from_json(&bytes).map_err(|source| DaemonError::Agents {
                path: path.clone(),
                source,
            })?
        }
        None => humata2_sessions::Agents::builtin(),
    };
    // D169: Claude Code starts with hooks that run `humata turn`; the file is
    // written now and again before each launch.
    let humata = args
        .humata
        .clone()
        .unwrap_or_else(|| beside_this_binary("humata"));
    let humata = std::fs::canonicalize(&humata).unwrap_or(humata);
    let hooks = crate::session_service::install_hooks(
        state.join("claude-hooks.json"),
        humata.clone(),
        socket.clone(),
    )
    .map_err(|source| DaemonError::Hooks { source })?;
    // HOOKS (D193): Codex's hooks go in as `-c hooks=…`, Copilot's as a
    // plugin in the state folder; the agents' own settings stay as they are.
    let agent_hooks = crate::session_service::install_agent_hooks(&state, &humata, &socket)
        .map_err(|source| DaemonError::Hooks { source })?;
    let agents = agents
        .with_hooks(&hooks)
        .with_agent_hooks(&agent_hooks.codex, &agent_hooks.copilot_plugin)
        // D201a: OpenCode runs through `humata opencode` on this daemon.
        .with_wrapper(&humata, &socket, &state);
    crate::session_service::set_sources(&agents);
    let reports = state.join("restarts");
    private_folder(&reports)?;
    let revive = restart::Revive {
        holder: holder.clone(),
        logs: logs.clone(),
        scrollback_bytes: args.scrollback_bytes,
        shell: shell.clone(),
        shell_args: shell_args.clone(),
        home: root.clone(),
        reports,
        machine: machine.clone(),
        daemon_log,
    };
    let starter = restart::starter(
        &store,
        revive,
        restart::Sessions {
            channels: channels_folder.clone(),
            remote_template: args.remote_template.clone(),
            agents,
            holders: holders.clone(),
        },
    );
    let feed = Arc::new(Feed::default());
    let tell = crate::session_service::Tell::new(feed.clone(), store.clone(), machine.clone());
    let last_start = restart::in_background(
        store.clone(),
        identities.clone(),
        starter.clone(),
        restored,
        tell,
        socket.clone(),
    );
    last_good::keep(store.clone(), last_good::Quiet::default());
    let compression = Arc::new(Compression::new(&state));
    let inbox = Arc::new(Inbox::new(
        guard.clone(),
        store.clone(),
        machine.clone(),
        feed.clone(),
    ));
    // The host's machine list and its links come first: the terminal, channel and
    // session services each hold a part of them (D139). A machine's own daemon has
    // none, so every session it knows is here and its local path is untouched.
    let host_work = match role {
        pb::DaemonRole::Host => Some(Arc::new(machine_work(
            &state,
            args.tools,
            args.ssh_config,
            feed.clone(),
        )?)),
        _ => None,
    };
    let remote_slot = Arc::new(std::sync::OnceLock::new());
    let session_tell =
        Tell::new(feed.clone(), store.clone(), machine.clone()).with_remote(remote_slot.clone());
    let remote_work = host_work.as_ref().map(|work| {
        let remote = RemoteWork::new(
            work.links(),
            work.reports(),
            Arc::new(StoreMemory(store.clone())),
            Arc::new(TellSink(session_tell.clone())),
        );
        let _ = remote_slot.set(remote.clone());
        // What the machines report, for the daemon's life (SL-3, TR-7).
        tokio::spawn({
            let remote = remote.clone();
            async move { remote.follow_reports().await }
        });
        remote
    });
    let remote_terminal = host_work
        .as_ref()
        .map(|work| RemoteTerminal::new(work.links()));
    // FM-10: the home folder a file manager pane may show, for the user only.
    let home = root.clone();
    let terminals = Terminals::new(TerminalsConfig {
        store: store.clone(),
        machine: machine.clone(),
        holder,
        shell,
        shell_args,
        root,
        holders,
        logs,
        scrollback_bytes: args.scrollback_bytes,
        guard: guard.clone(),
        identities: identities.clone(),
        compression: compression.clone(),
        inbox: inbox.clone(),
        remote: remote_terminal,
        socket: socket.clone(),
    });
    let roots = Arc::new(Roots::new(
        store.clone(),
        machine.clone(),
        channels_folder.clone(),
        home,
    ));
    // LD-6: one trash per machine, in the folder the channels live in.
    let bin = Arc::new(Bin::new(channels_folder.join(".trash")));
    let files = Files::new(
        roots.clone(),
        guard.clone(),
        bin.clone(),
        compression.clone(),
        Link::Local,
    );
    let trash = TrashSvc::new(bin.clone(), guard.clone(), machine.clone());
    // D194: a restored channel's records come back into the state.
    trash.use_store(store.clone());
    let channel_bin = bin.clone();
    // FM-9: the far end of a transfer the host drives on this machine.
    let transfer_end = TransferEndSvc::new(
        roots.clone(),
        guard.clone(),
        bin.clone(),
        machine.clone(),
    );
    let transfers = TransferSvc::new(
        roots,
        guard.clone(),
        bin.clone(),
        state.clone(),
        machine.clone(),
        compression.clone(),
    );
    // LD-7, PER-27: trash past its 30 days goes, and transfers a restart
    // cut short continue or finish undoing, off the runtime.
    tokio::task::spawn_blocking({
        let transfers = transfers.clone();
        let sweep_bin = bin.clone();
        move || {
            sweep_bin.sweep();
            transfers.recover();
        }
    });
    let review = crate::review::Review::new(
        store.clone(),
        guard.clone(),
        machine.clone(),
        feed.clone(),
        Arc::new(crate::review::TerminalSink::new(terminals.core())),
    );
    // D138: a batch of comments the last run was still typing is settled
    // before anyone asks.
    {
        let review = review.clone();
        tokio::spawn(async move { review.recover_comments().await });
    }
    let access = Access::new(guard.clone(), feed.clone(), inbox.clone(), store.clone());
    let events = EventSvc::new(feed.clone(), guard.clone(), machine.clone());
    // NV-24, D170: pins and VIP marks, in a file of their own in the state folder.
    let marks = crate::mark_service::MarkSvc::new(
        state.join("marks.db"),
        guard.clone(),
        feed.clone(),
        store.clone(),
    );
    // LD-8, LD-23 to LD-25: search and read the mirrored conversations, kept under the state folder.
    let history = Arc::new(crate::history_service::HistorySvc::new(
        state.clone(),
        guard.clone(),
        feed.clone(),
        store.clone(),
    ));
    let operations = Arc::new(Operations::new(feed.clone()));
    let channel_svc = ChannelSvc::new(
        guard.clone(),
        operations.clone(),
        starter.clone(),
        machine.clone(),
        args.default_agent.clone(),
        inbox.clone(),
        remote_work.clone(),
        host_work.clone(),
    );
    // D194: a delete a restart cut short is finished once the start is done.
    channel_svc.recover_deletes_after(channel_bin, last_start.clone());
    let board_svc = BoardSvc::new(
        guard.clone(),
        store.clone(),
        feed.clone(),
        inbox.clone(),
        machine.clone(),
    );
    // DK-14, D182: the Desk's sessions, beside the session service.
    let desk = crate::desk_service::server(crate::desk_service::DeskDeps {
        guard: guard.clone(),
        operations: operations.clone(),
        starter: starter.clone(),
        identities: identities.clone(),
        inbox: inbox.clone(),
        tell: session_tell.clone(),
        machine: machine.clone(),
        socket: socket.clone(),
        default_agent: args.default_agent.clone(),
        bin: bin.clone(),
        terminals: terminals.core(),
        state: state.clone(),
    });
    // D195: how a session leaves this machine or arrives on it.
    let carry_svc = SessionCarrySvc::new(starter.clone(), bin.clone(), host_work.is_some());
    let session_svc = SessionSvc::new(
        guard.clone(),
        operations.clone(),
        starter,
        identities.clone(),
        machine.clone(),
        inbox.clone(),
        remote_work,
        session_tell,
        socket.clone(),
    );
    // Archives a cut run left are finished once the start has brought the
    // sessions back (MV-15, PER-27).
    session_svc.finish_archives_after(last_start.clone());
    // CH-8: a machine's removal moves and archives its sessions through the session service.
    channel_svc.use_sessions(session_svc.clone());
    // SL-4: a rebuilt session's conversation comes from the host's mirror.
    {
        let history = history.clone();
        session_svc.use_raw_lines(Arc::new(move |session| history.raw_lines(session)));
    }
    let settings = crate::settings_service::SettingsSvc::new(guard.clone(), store.clone(), machine.clone(), feed.clone());
    // The default branch prefix is the host's gh login; a start never waits for it (D196).
    settings.follow_gh_login();
    let machines = match host_work {
        Some(work) => {
            let skills_home = match args.skills_home {
                Some(home) => home,
                None => std::env::var_os("HOME")
                    .filter(|home| !home.is_empty())
                    .map(PathBuf::from)
                    .ok_or(DaemonError::NoHome)?,
            };
            let skills = Arc::new(crate::skills_service::SkillsHost::new(&state, skills_home));
            // Set before any link starts, so the first link-up reaches it.
            let _ = work.skills.set(skills.clone());
            // LD-11: forgetting a machine archives its sessions through the SessionService.
            work.forgetting.set(Arc::new(session_svc.clone()));
            // FM-8, FM-9: transfers reach other machines over their links.
            transfers.use_links(work.links());
            // D195: the host moves sessions between machines.
            session_svc.use_carry(Carry::new(
                carry_svc.clone(),
                work.links(),
                bin.clone(),
                state.clone(),
                machine.clone(),
                channels_folder.clone(),
                compression.clone(),
            ));
            // MV-14: a move the last run cut short is undone or finished once its machines are linked.
            session_svc.recover_moves_after(last_start.clone());
            // A pane on another machine: its file and trash calls are forwarded.
            files.use_links(work.links());
            // D189: other machines' conversations reach the host's mirror.
            history.use_links(work.links(), work.reports());
            trash.use_links(work.links());
            machine_service::recover_in_background(work.clone());
            let providers = ProviderSvc::new(guard.clone(), work.clone(), operations.clone(), inbox.clone());
            let skills = crate::skills_service::SkillsSvc::new(
                guard.clone(),
                work.clone(),
                operations.clone(),
                skills,
                inbox.clone(),
            );
            Some((
                MachineSvc::new(guard, work, operations.clone(), inbox.clone()),
                providers,
                skills,
            ))
        }
        _ => None,
    };

    // Every call is matched to its caller before a service sees it. The
    // first layer is the outer one: it marks the methods open to anyone,
    // which the interceptor can't see the path of.
    let router = Server::builder()
        .layer(MapRequestLayer::new(identity::mark_open))
        .layer(InterceptorLayer::new(authenticate))
        .add_service(pb::daemon_service_server::DaemonServiceServer::new(
            Daemon::new(machine.clone(), role)
                .offering(compression)
                .started(last_start)
                .with_relay(relay),
        ))
        .add_service(pb::file_service_server::FileServiceServer::new(files))
        .add_service(pb::trash_service_server::TrashServiceServer::new(trash))
        .add_service(pb::transfer_service_server::TransferServiceServer::from_arc(transfers))
        .add_service(
            pb::transfer_end_service_server::TransferEndServiceServer::new(transfer_end)
                .max_decoding_message_size(DECODE_MAX_BYTES),
        )
        .add_service(
            pb::terminal_service_server::TerminalServiceServer::new(terminals)
                .max_decoding_message_size(DECODE_MAX_BYTES),
        )
        .add_service(pb::inbox_service_server::InboxServiceServer::from_arc(
            inbox,
        ))
        .add_service(pb::diff_service_server::DiffServiceServer::new(
            review.clone(),
        ))
        .add_service(pb::review_chat_service_server::ReviewChatServiceServer::new(review.clone()))
        .add_service(pb::comment_service_server::CommentServiceServer::new(
            review.clone(),
        ))
        .add_service(pb::view_service_server::ViewServiceServer::new(review))
        .add_service(pb::access_service_server::AccessServiceServer::new(access))
        .add_service(pb::channel_service_server::ChannelServiceServer::new(
            channel_svc,
        ))
        .add_service(pb::session_service_server::SessionServiceServer::new(
            session_svc,
        ))
        .add_service(pb::session_carry_service_server::SessionCarryServiceServer::from_arc(carry_svc))
        .add_service(desk)
        .add_service(pb::board_service_server::BoardServiceServer::new(board_svc))
        .add_service(pb::cli_auth_service_server::CliAuthServiceServer::new(
            CliAuth {
                logins,
                scripts: scripts.clone(),
                store,
                machine,
                role,
            },
        ));
    let router = router
        .add_service(pb::operation_service_server::OperationServiceServer::new(
            OperationSvc::new(operations),
        ))
        .add_service(pb::event_service_server::EventServiceServer::new(events));
    // Only the host keeps machines; a machine's daemon forwards these to
    // it once the link does (GRPC.md §4).
    let router = match machines {
        Some((machines, providers, skills)) => router
            .add_service(pb::machine_service_server::MachineServiceServer::new(
                machines,
            ))
            .add_service(pb::provider_service_server::ProviderServiceServer::new(
                providers,
            ))
            .add_service(pb::skills_service_server::SkillsServiceServer::new(
                skills,
            )),
        None => not_built::register_host_only(router),
    };
    let router = router.add_service(pb::settings_service_server::SettingsServiceServer::new(settings));
    let router = router.add_service(pb::mark_service_server::MarkServiceServer::new(marks));
    let router = router.add_service(pb::history_service_server::HistoryServiceServer::from_arc(history));
    let router = not_built::register(router);

    let stop = stop_signal()?;
    if let Some(mut ready) = ready {
        // A bridge that went meanwhile has nothing to hear.
        let _ = ready.write_all(b"\n");
    }
    let incoming = humata2_app_peer::incoming(listener, app_requirement());
    // A stop ends every event subscription: graceful shutdown would wait
    // on them for as long as the app keeps them open. The socket goes at
    // once too, so nothing new reaches a daemon that is leaving.
    let (stopped, stopping) = tokio::sync::oneshot::channel::<()>();
    let shutdown = {
        let feed = feed.clone();
        let socket = socket.clone();
        async move {
            stop.await;
            feed.end_subscriptions();
            unlink_socket(&socket, socket_id);
            let _ = stopped.send(());
        }
    };
    let served = router.serve_with_incoming_shutdown(incoming, shutdown);
    tokio::pin!(served);
    let outcome = tokio::select! {
        outcome = &mut served => outcome,
        () = cut_after_grace(stopping) => {
            crate::log::say!(
                "humatad2: connections were still open {}s after the stop, so they were cut",
                SHUTDOWN_GRACE.as_secs()
            );
            Ok(())
        }
    };
    unlink_socket(&socket, socket_id);
    outcome.map_err(|source| DaemonError::Serve { source })
}

/// Resolves once a stop has waited out [`SHUTDOWN_GRACE`]. Never resolves
/// when the server ends without a stop, which its own result reports.
async fn cut_after_grace(stopping: tokio::sync::oneshot::Receiver<()>) {
    if stopping.await.is_ok() {
        tokio::time::sleep(SHUTDOWN_GRACE).await;
    } else {
        std::future::pending::<()>().await;
    }
}

/// The signed app's identity (D135), from the build: none in an unsigned
/// build, or one made outside Bazel, so nothing is taken for the app.
fn app_requirement() -> Option<humata2_app_peer::Requirement> {
    humata2_app_peer::Requirement::app(
        option_env!("HUMATA2_TEAM_ID").unwrap_or_default(),
        option_env!("HUMATA2_BUNDLE_ID").unwrap_or_default(),
    )
}

/// The host's machine list, its own known_hosts and the tools bundle.
fn machine_work(
    state: &Path,
    tools: Option<PathBuf>,
    ssh_config: Option<PathBuf>,
    feed: Arc<Feed>,
) -> Result<Work, DaemonError> {
    let builds: Box<dyn Builds + Send + Sync> = match tools {
        Some(tools) => {
            Box::new(Bundle::open(&tools).map_err(|source| DaemonError::Tools { source })?)
        }
        None => {
            let beside = beside_this_binary("tools");
            if beside.join(MANIFEST).is_file() {
                Box::new(Bundle::open(&beside).map_err(|source| DaemonError::Tools { source })?)
            } else {
                let here = beside_this_binary("humatad2");
                let folder = here.parent().map(Path::to_owned).unwrap_or_default();
                let platform = Platform::host().ok_or(DaemonError::UnknownHostPlatform)?;
                Box::new(Bundle::beside(&folder, platform))
            }
        }
    };
    let ssh_config = match ssh_config {
        Some(path) => path,
        None => std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(".ssh/config"))
            .ok_or(DaemonError::NoHome)?,
    };
    let machines = Machines::open(MachinesConfig {
        hosts_file: state.join("hosts.json"),
        known_hosts: state.join("known_hosts"),
        ssh_config,
        peer_keys: state.join(crate::peer_client::KEYS_DIR),
        retry: RetryPolicy::default(),
    });
    Ok(Work::new(
        machines,
        builds,
        feed,
        Arc::new(std::time::SystemTime::now),
    ))
}

fn beside_this_binary(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Makes `path` if it's missing, readable only by this user, and refuses
/// one that others can open.
fn private_folder(path: &Path) -> Result<(), DaemonError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|source| DaemonError::CreateDir {
            path: path.to_owned(),
            source,
        })?;
    let mode = std::fs::metadata(path)
        .map_err(|source| DaemonError::CreateDir {
            path: path.to_owned(),
            source,
        })?
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(DaemonError::FolderOpenToOthers {
            path: path.to_owned(),
            mode,
        });
    }
    Ok(())
}

/// This machine's id, made once and kept. Daemons starting for the first
/// time at once agree on one: each writes an id of its own aside, synced,
/// and links it into place, which only the first can do; the others read
/// the first one's.
fn machine_id(path: &Path) -> Result<String, DaemonError> {
    let failed = |source| DaemonError::MachineId {
        path: path.to_owned(),
        source,
    };
    if let Some(id) = read_machine_id(path).map_err(failed)? {
        return Ok(id);
    }
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(failed(std::io::Error::new(
            ErrorKind::InvalidInput,
            "the path names no file in a folder",
        )));
    };
    let id = format!("ma_{}", uuid::Uuid::new_v4().simple());
    let aside = folder.join(format!(
        ".{}.{}.partial",
        name.to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    let linked = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&aside)?;
        file.write_all(id.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::hard_link(&aside, path)
    })();
    // Linked or not, the id is in place or never will be from this copy.
    let _ = std::fs::remove_file(&aside);
    match linked {
        Ok(()) => {
            File::open(folder)
                .and_then(|folder| folder.sync_all())
                .map_err(failed)?;
            Ok(id)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => read_machine_id(path)
            .map_err(failed)?
            .ok_or_else(|| failed(ErrorKind::NotFound.into())),
        Err(error) => Err(failed(error)),
    }
}

/// The id kept at `path`, if there is one. An empty file is damage, not a
/// missing id: making a new one would make the host see a new machine.
fn read_machine_id(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(id) if !id.trim().is_empty() => Ok(Some(id.trim().to_owned())),
        Ok(_) => Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "the file is empty",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// A socket file's device and inode: what tells the socket a daemon bound
/// from one a later daemon binds at the same path.
type SocketId = (u64, u64);

fn socket_id(socket: &Path) -> std::io::Result<SocketId> {
    std::fs::symlink_metadata(socket).map(|metadata| (metadata.dev(), metadata.ino()))
}

/// Removes `socket` if it is still the one bound with `id`. A daemon that
/// started since has bound its own there, and that one stays.
fn unlink_socket(socket: &Path, id: SocketId) {
    if socket_id(socket).is_ok_and(|now| now == id) {
        let _ = std::fs::remove_file(socket);
    }
}

/// Takes the state folder's lock, which lasts as long as the daemon does.
/// A second daemon on the same state is refused here, and the pid the lock
/// holder wrote is what the refusal names.
fn lock_state(state: &Path) -> Result<File, DaemonError> {
    let path = state.join(LOCK_FILE);
    let lock_failed = |source| DaemonError::StateLock {
        path: path.clone(),
        source,
    };
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(lock_failed)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let pid = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            return Err(match pid {
                Some(pid) => DaemonError::StateInUse {
                    state: state.to_owned(),
                    pid,
                },
                None => DaemonError::StateInUseUnknownPid {
                    state: state.to_owned(),
                },
            });
        }
        Err(TryLockError::Error(source)) => return Err(lock_failed(source)),
    }
    // Only the lock's holder writes the file, so the pid written is this one's.
    file.set_len(0)
        .and_then(|()| file.write_all(format!("{}\n", std::process::id()).as_bytes()))
        .and_then(|()| file.sync_data())
        .map_err(lock_failed)?;
    Ok(file)
}

/// Listens on `socket`, 0600 in a folder only this user can open. A socket
/// left by a daemon that died is taken over; a live one is refused. Returns
/// the listener and the socket's id, for [`unlink_socket`].
fn bind(socket: &Path) -> Result<(UnixListener, SocketId), DaemonError> {
    if socket.as_os_str().len() > SOCKET_PATH_MAX {
        return Err(DaemonError::SocketPathTooLong {
            socket: socket.to_owned(),
            len: socket.as_os_str().len(),
            max: SOCKET_PATH_MAX,
        });
    }
    if let Some(folder) = socket
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        private_folder(folder)?;
    }
    if socket.exists() {
        if std::os::unix::net::UnixStream::connect(socket).is_ok() {
            return Err(DaemonError::AlreadyRunning {
                socket: socket.to_owned(),
            });
        }
        std::fs::remove_file(socket).map_err(|source| DaemonError::Bind {
            socket: socket.to_owned(),
            source,
        })?;
    }
    let bind_failed = |source| DaemonError::Bind {
        socket: socket.to_owned(),
        source,
    };
    let listener = UnixListener::bind(socket).map_err(bind_failed)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .map_err(bind_failed)?;
    let id = socket_id(socket).map_err(bind_failed)?;
    Ok((listener, id))
}

/// Resolves on SIGTERM or SIGINT.
pub(crate) fn stop_signal() -> Result<impl Future<Output = ()>, DaemonError> {
    let mut terminate =
        signal(SignalKind::terminate()).map_err(|source| DaemonError::Signals { source })?;
    let mut interrupt =
        signal(SignalKind::interrupt()).map_err(|source| DaemonError::Signals { source })?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[test]
    fn first_starts_at_once_agree_on_one_machine_id() {
        const STARTS: usize = 16;
        for _ in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("machine-id");
            let barrier = Arc::new(Barrier::new(STARTS));
            let starts: Vec<_> = (0..STARTS)
                .map(|_| {
                    let (path, barrier) = (path.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        machine_id(&path).unwrap()
                    })
                })
                .collect();
            let ids: Vec<String> = starts
                .into_iter()
                .map(|start| start.join().unwrap())
                .collect();
            assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
            assert_eq!(
                read_machine_id(&path).unwrap().as_deref(),
                Some(ids[0].as_str())
            );
            // Nothing is left aside.
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn a_kept_machine_id_is_used_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine-id");
        let first = machine_id(&path).unwrap();
        assert_eq!(machine_id(&path).unwrap(), first);
    }

    mod idle {
        //! A daemon serving its socket on tokio's paused clock: time moves
        //! only when every task waits, so an hour of idling takes no time,
        //! and a timer anywhere in the daemon would wake the runtime for
        //! each time it fires (TR-7).

        use std::os::fd::{IntoRawFd, OwnedFd};
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        use clap::Parser;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;
        use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf, pipe};

        use super::super::{ServeArgs, run};

        const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
        const DATA: u8 = 0;
        const HEADERS: u8 = 1;
        const SETTINGS: u8 = 4;
        const PING: u8 = 6;
        const WINDOW_UPDATE: u8 = 8;
        const ACK: u8 = 1;
        const END_STREAM: u8 = 1;
        const END_HEADERS: u8 = 4;
        const HOUR: Duration = Duration::from_secs(3600);

        #[derive(Parser)]
        struct Command {
            #[command(flatten)]
            serve: ServeArgs,
        }

        struct Frame {
            kind: u8,
            flags: u8,
            payload: Vec<u8>,
        }

        fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
            let length = u32::try_from(payload.len()).unwrap().to_be_bytes();
            let mut bytes = vec![length[1], length[2], length[3], kind, flags];
            bytes.extend_from_slice(&stream.to_be_bytes());
            bytes.extend_from_slice(payload);
            bytes
        }

        async fn read_frame(from: &mut OwnedReadHalf) -> Option<Frame> {
            let mut head = [0u8; 9];
            from.read_exact(&mut head).await.ok()?;
            let length = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let mut payload = vec![0; length];
            from.read_exact(&mut payload).await.ok()?;
            Some(Frame {
                kind: head[3],
                flags: head[4],
                payload,
            })
        }

        /// A header as HPACK spells it with no index and no Huffman code.
        fn header(block: &mut Vec<u8>, name: &str, value: &str) {
            block.push(0);
            block.push(u8::try_from(name.len()).unwrap());
            block.extend_from_slice(name.as_bytes());
            block.push(u8::try_from(value.len()).unwrap());
            block.extend_from_slice(value.as_bytes());
        }

        /// Calls GetServerInfo on stream 1 and reads up to its trailers.
        async fn get_server_info(from: &mut OwnedReadHalf, to: &mut OwnedWriteHalf) {
            let mut block = Vec::new();
            header(&mut block, ":method", "POST");
            header(&mut block, ":scheme", "http");
            header(
                &mut block,
                ":path",
                "/humata.v1.DaemonService/GetServerInfo",
            );
            header(&mut block, ":authority", "localhost");
            header(&mut block, "content-type", "application/grpc");
            header(&mut block, "te", "trailers");
            to.write_all(&frame(HEADERS, END_HEADERS, 1, &block))
                .await
                .unwrap();
            to.write_all(&frame(DATA, END_STREAM, 1, &[0, 0, 0, 0, 0]))
                .await
                .unwrap();
            loop {
                let next = read_frame(from).await.expect("the daemon hung up mid-call");
                match (next.kind, next.flags) {
                    (SETTINGS, flags) if flags & ACK == 0 => {
                        to.write_all(&frame(SETTINGS, ACK, 0, &[])).await.unwrap();
                    }
                    (HEADERS, flags) if flags & END_STREAM != 0 => return,
                    _ => {}
                }
            }
        }

        /// Waits `span` for frames, and returns how many times the runtime
        /// woke meanwhile and the frames heard besides SETTINGS and
        /// WINDOW_UPDATE. Hanging up counts as a frame of type 255.
        async fn idle(from: &mut OwnedReadHalf, span: Duration) -> (u64, Vec<u8>) {
            let before = parks();
            let mut heard = Vec::new();
            let _ = tokio::time::timeout(span, async {
                loop {
                    match read_frame(from).await {
                        Some(next) if matches!(next.kind, SETTINGS | WINDOW_UPDATE) => {}
                        Some(next) => heard.push(next.kind),
                        None => {
                            heard.push(u8::MAX);
                            std::future::pending::<()>().await;
                        }
                    }
                }
            })
            .await;
            (parks() - before, heard)
        }

        /// Whether a restart report with its `restart.json` is in `reports`.
        fn reconciled(reports: &std::path::Path) -> bool {
            std::fs::read_dir(reports).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.path().join("restart.json").is_file())
            })
        }

        fn parks() -> u64 {
            tokio::runtime::Handle::current()
                .metrics()
                .worker_park_count(0)
        }

        #[tokio::test(start_paused = true)]
        async fn an_idle_daemon_with_a_connected_client_fires_no_timers() {
            let dir = tempfile::Builder::new()
                .prefix("h2u")
                .tempdir_in("/tmp")
                .unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let socket = dir.path().join("d.sock");
            let (ready, ready_writer) = std::io::pipe().unwrap();
            let holder = std::env::current_exe().unwrap();
            let ready_fd = ready_writer.into_raw_fd().to_string();
            let args = Command::try_parse_from([
                "serve".into(),
                "--socket".into(),
                socket.clone().into_os_string(),
                "--state".into(),
                dir.path().join("state").into_os_string(),
                "--root".into(),
                dir.path().as_os_str().to_owned(),
                "--holder".into(),
                holder.into_os_string(),
                "--ready-fd".into(),
                ready_fd.into(),
            ] as [std::ffi::OsString; 11])
            .unwrap()
            .serve;
            let served = tokio::spawn(run(args));
            let mut ready = pipe::Receiver::from_owned_fd(OwnedFd::from(ready)).unwrap();
            assert_eq!(
                ready.read(&mut [0; 1]).await.unwrap(),
                1,
                "the daemon listens"
            );
            let (mut from, mut to) = UnixStream::connect(&socket).await.unwrap().into_split();
            to.write_all(PREFACE).await.unwrap();
            to.write_all(&frame(SETTINGS, 0, 0, &[])).await.unwrap();
            get_server_info(&mut from, &mut to).await;
            // The start brings sessions back beside serving, and its last
            // step writes the restart report. Wakes from that work landing
            // in one window or the other made this test flaky: wait for it.
            let reports = dir.path().join("state").join("restarts");
            while !reconciled(&reports) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            // Waking a few times to wait at all is much the same for a
            // minute as for ten hours: a long wait is refiled through the
            // levels of tokio's timer wheel a few times. A timer that fires
            // on its own every 30 s wakes it 1200 times; hourly, 10.
            let (minute, heard_in_a_minute) = idle(&mut from, Duration::from_secs(60)).await;
            let (hours, heard_in_hours) = idle(&mut from, 10 * HOUR).await;

            assert!(
                heard_in_a_minute.is_empty() && heard_in_hours.is_empty(),
                "the daemon sent {heard_in_a_minute:?} {heard_in_hours:?} while idle"
            );
            assert!(
                hours <= minute + 4,
                "the runtime woke {hours} times in ten idle hours, {minute} in a minute"
            );
            to.write_all(&frame(PING, 0, 0, &[7; 8])).await.unwrap();
            let pong = loop {
                let next = read_frame(&mut from).await.expect("the daemon hung up");
                if next.kind == PING {
                    break next;
                }
            };
            assert_eq!((pong.flags, pong.payload), (ACK, vec![7; 8]));
            served.abort();
        }
    }

    #[test]
    fn an_empty_machine_id_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine-id");
        std::fs::write(&path, "").unwrap();
        assert!(matches!(
            machine_id(&path),
            Err(DaemonError::MachineId { .. })
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }
}
