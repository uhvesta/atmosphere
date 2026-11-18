//! `humatad2 serve`: opens the state, binds the socket and serves until a
//! stop signal. Terminals live in their holders, so they outlive this.
//! With `--stdio` it serves the peer link instead (`peer_link`), and no
//! socket.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_chunk::Link;
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Builds, Bundle, MANIFEST, Machines, MachinesConfig, Platform, RetryPolicy, StackConfig};
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
use crate::upgrade::build::Release;

/// What Unix sockets allow in a path, without its terminating NUL.
#[cfg(target_os = "linux")]
pub(crate) const SOCKET_PATH_MAX: usize = 107;
#[cfg(not(target_os = "linux"))]
pub(crate) const SOCKET_PATH_MAX: usize = 103;

/// How long a stopping daemon gives its connections to finish once its
/// streams have ended (D270), before it cuts them and exits: only a call
/// still being answered can take this long.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// The file a daemon locks in its state folder. Its pid is written in it.
const LOCK_FILE: &str = "daemon.lock";
/// How long a start waits for the state lock to come free before it says
/// another daemon has it. On a loaded machine the lock of a daemon that
/// just ended, or just started itself again, can stay taken for a moment.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

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
    /// The tools bundle machines get their humatad2, humata2_holder, humata
    /// and stack from (TS-2): a folder with a manifest.json. Default: the
    /// `tools` folder beside humatad2 when it has one, else this host's
    /// own files beside humatad2 (machines of this platform only).
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
    /// An open pipe to write READY=1 to, then close, once the socket
    /// listens (D398): how `humatad2 bridge` and tests hear that a daemon
    /// they started is ready. Not for people.
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
    // Owning the pipe from the start closes it on any way out, which tells
    // the starter this one won't listen (D398).
    let ready = args
        .ready_fd
        .map(crate::ready::take_pipe)
        .transpose()
        .map_err(|source| DaemonError::Ready { source })?;
    // D146: a link given its own state folder reaches no socket unless told.
    let state_given = args.state.is_some();
    let state = match args.state {
        Some(state) => state,
        None => home_folder()?.join("state"),
    };
    // UP-1, D281: which release this daemon is, read at boot before any
    // swap of the binary: this binary's hash, and the bundle in `bin`
    // beside the state folder.
    let own_exe = std::env::current_exe().unwrap_or_default();
    let bin = state
        .parent()
        .map(|dir| dir.join("bin"))
        .unwrap_or_else(|| PathBuf::from("bin"));
    let store_schema = u32::try_from(humata2_store::SCHEMA_VERSION).unwrap_or(u32::MAX);
    let release = Release::read(&bin, &own_exe, env!("CARGO_PKG_VERSION"), store_schema);
    // Where an upgrade of this daemon swaps the release in (UP-1, D281 f).
    let upgrade_bin = bin;
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
        return peer_link::run(&state, machine, args.sync.as_deref(), socket, release).await;
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
    // HO-13: where this machine keeps its host variable values; the one
    // place a store is chosen (a keychain or another store would be set here).
    crate::host_vars::install(Arc::new(crate::host_vars::FileStore::in_folder(&state)));
    let holders = state.join("holders");
    let logs = state.join("logs");
    crate::terminal::check_holder_sockets(&holders)?;
    private_folder(&holders)?;
    private_folder(&logs)?;
    // Locked before the socket is bound or the state touched, so a second
    // daemon on this state stops here, and it can't take the first one's
    // socket. The lock lasts as long as this function does.
    let state_lock = lock_state(&state, LOCK_WAIT).await?;
    // UP-1, D281 g: this start is counted against an upgrade under way,
    // before anything else runs.
    let upgrades = crate::upgrade::record::UpgradeFolder::new(&state);
    if let Err(error) = upgrades.count_boot() {
        crate::log::say!("humatad2: warning: {}", crate::status::chain(&error));
    }
    // UP-1, D281 g: a first start of the new build keeps a copy of the
    // store; a second without ready, or a swap that was cut, puts the old
    // release and store back and starts the old daemon. Before the store
    // is opened.
    match crate::upgrade::rollback::at_start(&state, &upgrade_bin, &release) {
        Ok(crate::upgrade::rollback::Start::Proceed) => {}
        Ok(crate::upgrade::rollback::Start::Restart(path)) => {
            let _ = state_lock.unlock();
            // SEAM (D439): the old release says it listens in this one's
            // place, on the pipe handed on to it.
            let owed = ready.map_or(crate::stop::exec::Ready::Told, crate::stop::exec::Ready::Owed);
            return match crate::stop::exec::exec_at(path, "upgrade.rollback.exec", owed) {
                Ok(never) => match never {},
                Err(source) => Err(DaemonError::Relaunch { source }),
            };
        }
        Err(source) => return Err(DaemonError::Upgrade { source }),
    }
    // A rolled back or abandoned upgrade put the previous bundle.json back:
    // this daemon is the release that one names.
    let release = release.reread(&upgrade_bin);
    // D261, D265: whether this machine's sessions are stopped, read before
    // anything is started.
    let stop_folder = crate::stop::StopFolder::new(&state);
    let stop_mode = crate::stop::boot::read_mode(&stop_folder);
    let stopped_now = matches!(stop_mode, crate::stop::boot::Mode::Stopped(_));
    // Bound before the store is opened, so a second daemon on the same
    // socket stops here; and only the daemon that has the socket keeps
    // the log.
    let (listener, socket_id) = bind(&socket)?;
    let daemon_log = crate::log::start(&logs);
    let (store, restored) = Store::open_or_restore(state.join("store"))
        .map_err(|source| DaemonError::Store { source })?;
    crate::fault::point("upgrade.schema.migrated");
    let store = Arc::new(store);
    let machine = machine_id(&state.join("machine-id"))?;
    let role = match args.role {
        Role::Host => pb::DaemonRole::Host,
        Role::Machine => pb::DaemonRole::Machine,
    };
    // D500: text a person reads calls the host's own machine "This Mac"; a
    // machine's own daemon has no name of its own, so it uses its id.
    let names = match role {
        pb::DaemonRole::Host => crate::upgrade::items::Names::of_host(&machine),
        _ => crate::upgrade::items::Names::default(),
    };
    if stopped_now {
        crate::stop::gate::enter(&machine, &names.of(&machine));
    }

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
    let stop_guard = guard.clone();
    let login_host_link = guard.host_link();
    crate::rules_kept::restore(&store, &guard).map_err(DaemonError::RulesKept)?;
    // AW-8: a machine judges by the host's last copy, kept across restarts.
    if role == pb::DaemonRole::Machine
        && let Err(error) = crate::away::restore_policy_copy(&store, &guard)
    {
        crate::log::say!("humatad2: warning: {}", crate::status::chain(&error));
    }

    // FC-10: the root the user saved for this Mac wins over the folder the
    // daemon was started with; every service below shares this one layout, so
    // a root change reaches them while the daemon runs.
    let layout = crate::machine_service::start_layout(
        &store,
        args.channels.clone().unwrap_or_else(|| root.join("Humata")),
        &root,
    );
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
    // MV-16: when the daemon runs with Claude's, Codex's or Copilot's own
    // dir set (CLAUDE_CONFIG_DIR, CODEX_HOME, COPILOT_HOME), the agents it
    // starts inherit it, so their conversations live there and a carry
    // must look and place there.
    let agents = [
        ("CLAUDE_CONFIG_DIR", humata2_sessions::ConversationFormat::ClaudeJsonl),
        ("CODEX_HOME", humata2_sessions::ConversationFormat::CodexRollout),
        ("COPILOT_HOME", humata2_sessions::ConversationFormat::CopilotSessionState),
    ]
    .into_iter()
    .fold(agents, |agents, (var, format)| match std::env::var_os(var).filter(|dir| !dir.is_empty()) {
        Some(dir) => agents.with_cli_home(format, Path::new(&dir)),
        None => agents,
    });
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
    // D748: each agent's CLI gets this daemon's MCP server (`humatad2 mcp`)
    // for its session, from files in the state folder that hold no token.
    let humatad = std::env::current_exe()
        .map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe))
        .unwrap_or_else(|_| beside_this_binary("humatad2"));
    let humata_tools = crate::session_service::install_mcp_configs(&state, &humatad, &socket)
        .map_err(|source| DaemonError::McpConfigs { source })?;
    let agents = agents
        .with_hooks(&hooks)
        .with_agent_hooks(&agent_hooks.codex, &agent_hooks.copilot_plugin)
        // D201a: OpenCode runs through `humata opencode` on this daemon.
        .with_wrapper(&humata, &socket, &state)
        .with_humata_tools(&humata_tools)
        // D312: a Terminal session runs the shell new shell tabs run.
        .with_shell(&shell, &shell_args);
    // D748: every tab's PATH starts with the folder `humata` is in.
    if let Some(tools) = humata.parent() {
        crate::identity::set_tools_folder(tools);
    }
    // D748: an agent saved by an earlier daemon resumes with this one's tools.
    tools_in_saved_launches(&store, &agents);
    crate::session_service::set_sources(&agents);
    // AGENTSET (D720): which agents are on, read afresh from the humata home's
    // `config/agents.toml` for every start; a machine reads its shipped copy.
    let agents = match state.parent() {
        Some(home) => agents.with_settings(crate::agent_switches::host_path(home)),
        None => agents,
    };
    // ST-1: AgentService lists these kinds; the built-in list when no agents file was given.
    let agent_service = crate::agent_service::AgentSvc::new(agents.clone(), args.agents.is_none());
    // D782: a write to agents.toml reaches every linked machine, once the
    // machines are served (below).
    let agents_changed = agent_service.on_change();
    // D782's upgrade: D199's one "ask in the agent" switch becomes every
    // kind's own `asks_in_inbox`, once.
    if let Err(error) = crate::inbox::permission::move_global_switch(&store, &agents) {
        crate::log::say!("humatad2: {}", crate::status::chain(&error));
    }
    // ST-15: Settings checks a default agent it saves against these.
    let settings_agents = agents.clone();
    // B9: the history learns a conversation its agent wrote before the
    // session named it, as a resume does.
    let history_agents = agents.clone();
    // D782: the Inbox reads each agent's own "Ask in the Inbox".
    let inbox_agents = agents.clone();
    let reports = state.join("restarts");
    private_folder(&reports)?;
    // D373: which holder file each holder is spawned from.
    let builds = Arc::new(crate::upgrade::holders::Builds::new(&state, &holder));
    let revive = restart::Revive {
        builds: Some(builds.clone()),
        contain: Some(Arc::new(crate::contained::Contained::new(
            store.clone(),
            layout.clone(),
            state.clone(),
            socket.clone(),
            root.clone(),
        ).needing_host_rules(guard.clone()))),
        holder: holder.clone(),
        logs: logs.clone(),
        scrollback_bytes: args.scrollback_bytes,
        shell: shell.clone(),
        shell_args: shell_args.clone(),
        home: root.clone(),
        // B5: the agents' CLIs keep their conversations under the home they
        // run in, which `--root` doesn't move.
        agent_home: agent_home(std::env::var_os("HOME"), &root),
        reports,
        machine: machine.clone(),
        daemon_log,
    };
    // The names a live holder's process may have, for the stop's sweep.
    let holder_names = crate::stop::sweep::holder_names(&holder);
    let mut starter = restart::starter(
        &store,
        revive,
        restart::Sessions {
            channels: layout.clone(),
            remote_template: args.remote_template.clone(),
            agents,
            holders: holders.clone(),
        },
    );
    // D763: a start's first message waits for the agent's input to clear
    // (D762's gate), and one left unsent is asked about in the Inbox.
    let start_messages = Arc::new(crate::start_message::StartMessages::default());
    starter.first_message = humata2_sessions::FirstMessage::default()
        .with_gate(Arc::new(crate::input_gate::StartGate))
        .with_held(start_messages.clone());
    let feed = Arc::new(Feed::default());
    // AWAY: the peer link's PeerUplink, and whether the host is reachable.
    // AW-8: the policy copy, made here on the host, put in force on a machine.
    let policy_copy = crate::away::PolicySource::new(guard.clone(), store.clone());
    let tell = crate::session_service::Tell::new(feed.clone(), store.clone(), machine.clone());
    // D158.7: the app hears when an agent starts or stops waiting on its own request.
    crate::session_service::tell_asks(tell.clone());
    // Bug 23: the pointer loop of this tell and the file service tell file changes through one.
    let file_changes = tell.file_changes();
    // D265: a start after a stop names the stop once it has brought back.
    let after_stop = match &stop_mode {
        crate::stop::boot::Mode::Starting(record) => Some(crate::stop::boot::AfterStop {
            record: record.clone(),
            folder: stop_folder.clone(),
            machine: machine.clone(),
        }),
        _ => None,
    };
    // D261: a stopped boot brings nothing back; its start is set once the
    // sweep has run.
    let last_start = if stopped_now {
        Arc::new(restart::Started::default())
    } else {
        restart::in_background(
            store.clone(),
            identities.clone(),
            starter.clone(),
            restored,
            tell,
            socket.clone(),
            after_stop,
        )
    };
    last_good::keep(store.clone(), last_good::Quiet::default());
    let compression = Arc::new(Compression::new(&state));
    let inbox = Arc::new(Inbox::new(
        guard.clone(),
        store.clone(),
        machine.clone(),
        feed.clone(),
    ));
    // D782: each ask reads its agent's own switch.
    inbox.use_agents(inbox_agents);
    start_messages.tell(inbox.clone(), store.clone(), machine.clone());
    // D261: a stopped boot ends every holder, withdraws the Inbox's pending
    // requests and settles the record; that is its start.
    if let crate::stop::boot::Mode::Stopped(record) = stop_mode {
        let settle = crate::stop::boot::Settle {
            folder: stop_folder.clone(),
            record,
            sweep: crate::stop::sweep::Sweep {
                holders: holders.clone(),
                grace: crate::stop::sweep::GRACE,
                programs: holder_names,
            },
            store: store.clone(),
            inbox: inbox.clone(),
        };
        let started = last_start.clone();
        tokio::spawn(async move { started.set(settle.run().await) });
    }
    // AWAY: the peer link's PeerUplink, which applies the answers the host relays to this Inbox.
    let uplinks = Arc::new(crate::away::Uplinks::new(
        guard.host_link(),
        feed.clone(),
        policy_copy.clone(),
        inbox.clone(),
    ));
    // D268: a wait on a request the host holds goes up the same link.
    inbox.forwarding(uplinks.forwarder());
    // PER-30, D242 and UP-1, D281 f: the restart doctor's store repair and an
    // upgrade of this daemon make it leave once it has answered; `leave` is
    // what they wake.
    let leave = Arc::new(tokio::sync::Notify::new());
    let restarting = crate::upgrade::run::Restarting::new(leave.clone());
    // UP-1, D281: running an upgrade of this daemon, and the Migrate items of
    // this Inbox, which carry theirs out the same way (D290 n).
    // Another machine's is staged and called by the host, once its machine work exists.
    let host_slot = Arc::new(std::sync::OnceLock::new());
    let holders_slot = Arc::new(std::sync::OnceLock::new());
    let upgrade = crate::upgrade::run::Runner::new(&machine, upgrade_bin.clone(), upgrades, release.clone(), restarting.clone())
        .with_holders(holders_slot.clone());
    let carry = crate::upgrade::remote::Routed::new(
        &machine,
        Arc::new(crate::upgrade::holders::Split::new(&machine, Arc::new(upgrade.clone()), holders_slot.clone())),
        Arc::new(crate::upgrade::remote::Remote::of_host(host_slot.clone())),
    );
    let migrations = crate::upgrade::items::Migrations::new(inbox.clone(), Arc::new(carry));
    migrations.set_names(names.clone());
    // UP-1, D370, D371: a release the app staged beside this host's daemon,
    // and a CLI or holder that isn't the release's, are Migrate items. A
    // machine's own tools are checked by the host from what the machine
    // reports, so a machine's daemon never posts for itself.
    let local = (role == pb::DaemonRole::Host).then(|| {
        crate::upgrade::local::Local::new(migrations.clone(), release.clone(), &machine, upgrade_bin)
    });
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
    // D373: the agents still on an old holder after an upgrade are one Holders
    // item, and Migrate restarts them. D378: a machine's daemon looks at its own
    // the same way and reports the list to the host, which posts the item.
    {
        let report = (role == pb::DaemonRole::Machine).then(|| {
            let (report, held) = tokio::sync::watch::channel(None);
            uplinks.report_holders(held);
            Arc::new(report)
        });
        let item = crate::upgrade::holders::Holders::new(crate::upgrade::holders::Deps {
            machine: machine.clone(),
            release: release.clone(),
            builds: builds.clone(),
            migrations: migrations.clone(),
            starter: starter.clone(),
            identities: identities.clone(),
            socket: socket.clone(),
            tell: session_tell.clone(),
            feed: feed.clone(),
            report,
        });
        let _ = holders_slot.set(item.clone());
        if let Some(local) = &local {
            local.wake_after_looks(item.wake());
        }
        item.start(last_start.clone());
    }
    let remote_work = host_work.as_ref().map(|work| {
        let remote = RemoteWork::new(
            work.links(),
            work.reports(),
            Arc::new(StoreMemory(store.clone())),
            Arc::new(TellSink(session_tell.clone())),
        );
        // D262: the stops kept for other machines, on this host's disk, set
        // before any link starts.
        let stops = crate::stop::remote::HostStops::open(&state);
        let _ = work.stops.set(stops.clone());
        remote.use_stops(stops);
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
        logs: logs.clone(),
        scrollback_bytes: args.scrollback_bytes,
        guard: guard.clone(),
        identities: identities.clone(),
        compression: compression.clone(),
        inbox: inbox.clone(),
        remote: remote_terminal,
        socket: socket.clone(),
        migrations: migrations.clone(),
        builds: builds.clone(),
        tell: session_tell.clone(),
        started: last_start.clone(),
    });
    // D767: a question an agent asks on its screen becomes an Inbox question.
    crate::screen_ask::start(terminals.holder_links(), inbox.clone(), store.clone(), machine.clone());
    let roots = Arc::new(Roots::new(
        store.clone(),
        machine.clone(),
        layout.clone(),
        home.clone(),
    ));
    // LD-6: one trash per machine, in the folder the channels live in.
    let bin = Arc::new(Bin::new(layout.channel(".trash")));
    let files = Arc::new(Files::new(
        roots.clone(),
        guard.clone(),
        bin.clone(),
        compression.clone(),
        Link::Local,
        inbox.clone(),
    ));
    let trash = TrashSvc::new(bin.clone(), guard.clone(), machine.clone());
    // D194: a restored channel's records come back into the state.
    trash.use_store(store.clone());
    // CD-4, CD-5, CH-24: the trash and the channel delete reach the Desk
    // service through this handle, set when the Desk service is made below.
    let desk_handle = crate::desk_service::channel_desk_close::DeskHandle::new(feed.clone());
    trash.use_desk(desk_handle.clone());
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
        inbox.clone(),
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
    // BUG 42: the user's own write of a file is not the agent's change.
    files.use_user_writes(Arc::new(review.clone()));
    files.use_changes(file_changes);
    // D138: a batch of comments the last run was still typing is settled
    // before anyone asks.
    // A stopped machine types nothing, so it settles them at its Start.
    if !stopped_now {
        let review = review.clone();
        let operations = last_start.operations().clone();
        // D241: each batch in doubt is named pending before the report is written.
        let doubtful = review.note_comments(&operations);
        tokio::spawn(async move { review.recover_comments(&operations, doubtful).await });
    }
    let access = Access::new(
        guard.clone(),
        feed.clone(),
        inbox.clone(),
        store.clone(),
        policy_copy.clone(),
    );
    let events = EventSvc::new(feed.clone(), guard.clone(), machine.clone());
    // NV-24, D170: pins and VIP marks, in a file of their own in the state folder.
    let marks = crate::mark_service::MarkSvc::new(
        state.join("marks.db"),
        guard.clone(),
        feed.clone(),
        store.clone(),
    );
    // CH-16, D394: a view of a remote session is held open on its machine.
    marks.use_remote(remote_slot.clone(), machine.clone());
    // REPOS (RL-5): the repo library (RL-1) in `<state>/repos.db` and the default
    // search scope in `<home>/config/repos.toml` (RL-7). Opened when first used.
    let repos = crate::repo_service::RepoSvc::new(
        state.join("repos.db"),
        state.parent().unwrap_or(&state),
        guard.clone(),
    );
    // LD-8, LD-23 to LD-25: search and read the mirrored conversations, kept under the state folder.
    let history = Arc::new(crate::history_service::HistorySvc::new(
        state.clone(),
        guard.clone(),
        feed.clone(),
        store.clone(),
        history_agents,
        agent_home(std::env::var_os("HOME"), &home),
    ));
    let operations = Arc::new(Operations::new(feed.clone()));
    let pull_requests = crate::review::PullRequests::new(
        store.clone(),
        guard.clone(),
        machine.clone(),
        starter.clone(),
        session_tell.clone(),
        feed.clone(),
        args.default_agent.clone(),
        review.clone(),
        operations.clone(),
    );
    // D286: a stopped machine asks GitHub nothing on its own; its Start
    // (the daemon starting itself again, running) follows the reviews again.
    if !stopped_now {
        pull_requests.spawn_poll();
    }
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
    // CH-24: a delete closes the channel's Desk sessions first.
    channel_svc.use_desk(desk_handle.clone());
    // REPOS (RL-1): every successful repo add is recorded in the repo library.
    match repos.library() {
        Ok(library) => channel_svc.use_repo_library(library),
        Err(error) => crate::log::say!(
            "humatad2: warning: repos added now won't be listed in the repo library: {}",
            crate::status::chain(&error)
        ),
    }
    // D194: a delete a restart cut short is finished once the start is done.
    channel_svc.recover_deletes_after(channel_bin, last_start.clone());
    // D227: on a machine, the agents' board calls go to the host while its Uplink is open.
    let board_svc = BoardSvc::new(
        guard.clone(),
        store.clone(),
        feed.clone(),
        inbox.clone(),
        machine.clone(),
    )
    .forwarding(uplinks.forwarder())
    .telling(session_tell.clone())
    // REPOS (RL-12, D568): a machine's agents search the host's library.
    .with_repos(repos.clone());
    // REPOS (RL-12, D568): on a machine, its RepoService sends up the Uplink.
    let repo_forwarder = uplinks.forwarder();
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
        handle: desk_handle,
        remote: remote_work.clone(),
        migrations: migrations.clone(),
        channels: channel_svc.clone(),
        last_start: last_start.clone(),
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
        migrations.clone(),
    );
    // Archives a cut run left are finished once the start has brought the
    // sessions back (MV-15, PER-27).
    session_svc.finish_archives_after(last_start.clone());
    // CH-8: a machine's removal moves and archives its sessions through the session service.
    channel_svc.use_sessions(session_svc.clone());
    // A removal of a machine the last run cut short is finished once the
    // start is done; a stopped machine moves nothing until its Start.
    if !stopped_now {
        channel_svc.recover_removals_after(last_start.clone());
    }
    // REPOS (CH-21): AddRepoToSession adds a repo the channel lacks.
    session_svc.use_channels(channel_svc.clone());
    // PR-17: Archive names a review's unsubmitted drafts.
    session_svc.use_review(review.clone());
    pull_requests.use_sessions(session_svc.clone());
    // D287: a submit cut short is settled against GitHub before anyone
    // posts again. A stopped machine asks GitHub nothing; its records stay
    // until the next start.
    if !stopped_now {
        let pull_requests = pull_requests.clone();
        tokio::spawn(async move { pull_requests.reconcile_submits().await });
    }
    // SL-4: a rebuilt session's conversation comes from the host's mirror.
    {
        let history = history.clone();
        let rebase = history.clone();
        session_svc.use_raw_lines(Arc::new(move |session| history.raw_lines(session)));
        // MV-13, D209: a moved or rebuilt conversation is mirrored once.
        session_svc.use_rebase(Arc::new(move |session, machine, skip| rebase.rebase_source(session, machine, skip)));
    }
    let settings = crate::settings_service::SettingsSvc::new(
        guard.clone(),
        store.clone(),
        machine.clone(),
        feed.clone(),
        &state,
        settings_agents,
        args.default_agent.clone(),
    )
    .with_agents_changed(agents_changed.clone());
    // The default branch prefix is the host's gh login; a start never waits for it (D196).
    settings.follow_gh_login();
    // A machine's own storage and Humata folder, for the host's relayed
    // Storage and root-change calls.
    let box_machines = machine_service::BoxMachineSvc::new(
        Arc::new(
            crate::storage::Storage::new(store.clone(), machine.clone(), layout.clone(), bin.clone(), logs.clone())
                .with_home(home.clone()),
        ),
        machine.clone(),
        operations.clone(),
        // D768: its agents' machine lookups go to the host.
        uplinks.forwarder(),
    );
    // D784: a stopped shell is started again as the restart doctor does.
    box_machines.use_sessions(session_svc.clone());
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
            // AW-8: each link sends its machine the policy copy.
            let _ = work.policy.set(policy_copy.clone());
            // D226: each link replays its machine's Inbox journal into the host's Inbox.
            let _ = work.inbox.set(inbox.clone());
            // UP-1: a Migrate of another machine stages its release and calls its daemon.
            let _ = host_slot.set(work.clone());
            // UP-1: each link-up posts or withdraws its machine's Migrate item.
            let _ = work
                .upgrade
                .set(crate::upgrade::items::Watch::new(migrations.clone(), release.clone()));
            if let Some(remote) = remote_slot.get() {
                remote.use_upgrade(crate::upgrade::items::Watch::new(migrations.clone(), release.clone()));
            }
            // D227: each link runs its machine's forwarded board calls and replays its board journal.
            let _ = work.boards.set(board_svc.clone());
            // VW-12: each link keeps the views its machine's agents offered.
            let _ = work.views.set(review.clone());
            // LD-11: forgetting a machine archives its sessions through the SessionService.
            work.forgetting.set(Arc::new(session_svc.clone()));
            // MACHINESCRIPTS (D681): the owner's rules run as machines and sessions change. Set before
            // any link starts, so the first link-up reaches it.
            if let Some(humata_home) = state.parent() {
                let scripts = crate::machine_scripts::build(crate::machine_scripts::Parts {
                    work: work.clone(),
                    sessions: session_svc.clone(),
                    terminal: terminals.core(),
                    feed: feed.clone(),
                    inbox: inbox.clone(),
                    humata_home: humata_home.to_owned(),
                    state: state.clone(),
                });
                session_svc.use_machine_scripts(scripts.clone());
                work.install_scripts(scripts, humata_home);
            }
            // MACHINESCRIPTS (D689): a finished Migrate runs that machine's
            // `upgraded` rules; This Mac has no triggers (D681).
            let upgraded = work.clone();
            let this_mac = machine.clone();
            migrations.on_migrated(Arc::new(move |migrated: &str| {
                if migrated != this_mac {
                    upgraded.fire_upgraded(migrated);
                }
            }));
            // A forget the last run began and didn't answer is finished now.
            tokio::spawn(machine_service::finish_forgets(work.clone(), last_start.clone()));
            // FM-8, FM-9: transfers reach other machines over their links.
            transfers.use_links(work.links());
            // D195: the host moves sessions between machines.
            session_svc.use_carry(Carry::new(
                carry_svc.clone(),
                work.links(),
                bin.clone(),
                state.clone(),
                machine.clone(),
                layout.clone(),
                compression.clone(),
            ));
            // MV-14: a move the last run cut short is undone or finished once its machines are linked.
            // A stopped machine moves nothing until its Start.
            if !stopped_now {
                session_svc.recover_moves_after(last_start.clone());
            }
            // A pane on another machine: its file and trash calls are forwarded.
            files.use_links(work.links());
            files.use_migrations(migrations.clone());
            // D189: other machines' conversations reach the host's mirror.
            history.use_links(work.links(), work.reports());
            trash.use_links(work.links());
            machine_service::recover_in_background(work.clone());
            // D782: a changed agents.toml goes to every linked machine now,
            // not only at its next link-up.
            let pushes = work.clone();
            agents_changed.set(move || machine_service::agents_changed(&pushes));
            let providers = ProviderSvc::new(guard.clone(), work.clone(), operations.clone(), inbox.clone());
            let skills = crate::skills_service::SkillsSvc::new(
                guard.clone(),
                work.clone(),
                operations.clone(),
                skills,
                inbox.clone(),
            );
            Some((
                MachineSvc::new(
                    guard,
                    work,
                    operations.clone(),
                    inbox.clone(),
                    Arc::new(
                        crate::storage::Storage::new(
                            store.clone(),
                            machine.clone(),
                            layout.clone(),
                            bin.clone(),
                            logs.clone(),
                        )
                        .with_home(home.clone()),
                    ),
                ),
                providers,
                skills,
            ))
        }
        _ => None,
    };
    // D768: only the host keeps machines and links them, so a machine's
    // agents look machines up, read other machines' files and copy between
    // machines through the host, which answers as that agent.
    if let Some((machine_svc, _, _)) = &machines {
        // D784: a stopped shell is started again as the restart doctor does.
        machine_svc.use_sessions(session_svc.clone());
    }
    match &machines {
        Some((machine_svc, _, _)) => board_svc.answer_host_calls(crate::board_service::HostCalls {
            machines: machine_svc.clone(),
            files: files.clone(),
            transfers: transfers.clone(),
        }),
        None if role == pb::DaemonRole::Machine => {
            files.ask_host_for_agents(uplinks.forwarder());
            transfers.ask_host_for_agents(uplinks.forwarder());
        }
        None => {}
    }

    // The restart doctor's store repair makes this daemon leave once it has
    // answered, so its next start restores the last good copy.
    let relaunch = crate::stop::service::Relaunch::new(leave.clone());
    let doctor = crate::doctor::Doctor::new(
        session_svc.clone(),
        inbox.clone(),
        store.clone(),
        machine.clone(),
        leave.clone(),
    );
    // Every call is matched to its caller before a service sees it. The
    // first layer is the outer one: it marks the methods open to anyone,
    // which the interceptor can't see the path of. The last, next to the
    // services, ends their streams when the daemon leaves (D270).
    // D241: every recovery of a cut operation has noted itself pending by
    // now, so the restart report that follows names each one.
    last_start.recoveries_noted();
    let (leaving, cut_streams) = crate::stop::streams::Leaving::signal();
    let router = Server::builder()
        .layer(MapRequestLayer::new(identity::mark_open))
        .layer(InterceptorLayer::new(authenticate))
        // UP-1, D281, D290 m: a CLI at another release than this daemon's is refused.
        .layer(crate::upgrade::gate::CliRelease::new(release.clone(), &machine).named(&names.of(&machine)))
        .layer(cut_streams)
        .add_service(pb::daemon_service_server::DaemonServiceServer::new(
            Daemon::new(machine.clone(), role)
                .offering(compression)
                .with_build(release.clone())
                .with_upgrade(upgrade.clone())
                .started(last_start)
                .with_relay(relay)
                .with_uplinks(uplinks)
                .with_doctor(doctor)
                .with_stop(crate::stop::service::StopSvc::new(
                    stop_folder,
                    machine.clone(),
                    relaunch.clone(),
                    stop_guard,
                    inbox.clone(),
                )),
        ))
        .add_service(pb::file_service_server::FileServiceServer::from_arc(files.clone()))
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
        .add_service(pb::pull_request_review_service_server::PullRequestReviewServiceServer::new(pull_requests))
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
                host: login_host_link,
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
        None => not_built::register_host_only(router.add_service(
            pb::machine_service_server::MachineServiceServer::new(box_machines),
        )),
    };
    let router = router.add_service(pb::settings_service_server::SettingsServiceServer::new(settings));
    let router = router.add_service(pb::mark_service_server::MarkServiceServer::new(marks));
    // REPOS (RL-5): the library is the host's. A machine's agents search it
    // there through the Uplink, and the rest is the host's alone (RL-12, D568).
    let router = if role == pb::DaemonRole::Host {
        router.add_service(pb::repo_service_server::RepoServiceServer::new(repos))
    } else {
        router.add_service(pb::repo_service_server::RepoServiceServer::new(
            crate::repo_service::OnMachine::new(repo_forwarder),
        ))
    };
    let router = router.add_service(pb::history_service_server::HistoryServiceServer::from_arc(history));
    let router = router.add_service(pb::agent_service_server::AgentServiceServer::new(agent_service));
    let router = not_built::register(router);

    let stop = {
        let signal = stop_signal()?;
        async move {
            tokio::select! {
                () = signal => {}
                () = leave.notified() => {}
            }
        }
    };
    if let Err(error) = crate::ready::tell(ready, crate::ready::notify_socket().as_deref()) {
        crate::log::say!("humatad2: warning: {}", crate::status::chain(&error));
    }
    // UP-1, D281 g: ready. The upgrade this start came from is settled, and
    // the Migrate item waiting for it is finished.
    upgrade.ready(&migrations).await;
    // What is in the bin folder is looked at before the first call is answered.
    if let Some(local) = &local {
        local.start().await;
    }
    let incoming = humata2_app_peer::incoming(listener, app_requirement());
    // A stop ends every event subscription and every other stream:
    // graceful shutdown would wait on them for as long as the app keeps
    // them open. The socket goes at once too, so nothing new reaches a
    // daemon that is leaving.
    let (stopped, stopping) = tokio::sync::oneshot::channel::<()>();
    let shutdown = {
        let feed = feed.clone();
        let socket = socket.clone();
        async move {
            stop.await;
            feed.end_subscriptions();
            leaving.now();
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
    // UP-1, D281 f: an upgrade has the daemon start the file it swapped in,
    // in this process, once the Upgrade call's answer has gone out. Holders
    // aren't touched: the new run reattaches them.
    if outcome.is_ok()
        && let Some(path) = restarting.take()
    {
        let _ = state_lock.unlock();
        return match crate::stop::exec::exec_at(path, "upgrade.exec", crate::stop::exec::Ready::Told) {
            Ok(never) => match never {},
            Err(source) => Err(DaemonError::Relaunch { source }),
        };
    }
    // D261, D265: a stop or a start has the daemon start itself again, in
    // this process. The state lock and the socket are close-on-exec, so the
    // new run takes both.
    if outcome.is_ok()
        && let Some(point) = relaunch.take()
    {
        // Let go of the lock now rather than at the exec, which on a
        // loaded machine can leave it taken past the new run's start.
        let _ = state_lock.unlock();
        return match crate::stop::exec::installed_path()
            .and_then(|path| crate::stop::exec::exec_at(path, point, crate::stop::exec::Ready::Told))
        {
            Ok(never) => match never {},
            Err(source) => Err(DaemonError::Relaunch { source }),
        };
    }
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
    let bundle = match tools {
        Some(tools) => Bundle::open(&tools).map_err(|source| DaemonError::Tools { source })?,
        None => {
            let beside = beside_this_binary("tools");
            if beside.join(MANIFEST).is_file() {
                Bundle::open(&beside).map_err(|source| DaemonError::Tools { source })?
            } else {
                let here = beside_this_binary("humatad2");
                let folder = here.parent().map(Path::to_owned).unwrap_or_default();
                let platform = Platform::host().ok_or(DaemonError::UnknownHostPlatform)?;
                let host = Bundle::beside(&folder, platform);
                // TS-2: the app installed the Linux tools it carries in
                // `~/.humata2/machine-tools`; machines of those platforms are set up from them.
                match state.parent() {
                    Some(home) => with_machine_tools(host, home, &folder),
                    None => host,
                }
            }
        }
    };
    // REPOS (RL-12): the host's `config/repos.toml` goes to machines beside `stack.toml`.
    let bundle = match state.parent() {
        Some(home) => bundle.with_repos_config(StackConfig::repos_host_path(home)),
        None => bundle,
    };
    // AGENTSET (D724): the host's `config/agents.toml` goes to machines the same way.
    let bundle = match state.parent() {
        Some(home) => bundle.with_agents_config(StackConfig::agents_host_path(home)),
        None => bundle,
    };
    // MACHINESCRIPTS (D680): the host's `config/machine-scripts.toml` goes to
    // machines the same way.
    let bundle = match state.parent() {
        Some(home) => bundle.with_machine_scripts_config(StackConfig::machine_scripts_host_path(home)),
        None => bundle,
    };
    // The host's `stack.toml` machines get beside the tools (D411) lives in
    // the humata home, the folder the state folder is in (`~/.humata2`).
    let builds: Box<dyn Builds + Send + Sync> = Box::new(match state.parent() {
        Some(home) => bundle.with_stack_config(StackConfig::host_path(home)),
        None => bundle,
    });
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

/// Where the app installs the Linux tools it carries, in the humata home.
const MACHINE_TOOLS: &str = "machine-tools";
/// A release's manifest, installed beside its tools.
const RELEASE_JSON: &str = "bundle.json";

/// `host` (this machine's own tools in `bin`) plus the platforms the app put
/// in `home`'s `machine-tools` (TS-2). The folder is used only when its
/// `bundle.json` is the one in `bin`: the bundle id covers every platform,
/// so tools of another release would give machines an id the host's own
/// release (UP-1) doesn't have. After an app update that is the window until
/// Migrate on This Mac; the daemon restarts at the new release and then
/// agrees.
fn with_machine_tools(host: Bundle, home: &Path, bin: &Path) -> Bundle {
    let folder = home.join(MACHINE_TOOLS);
    if !folder.join(MANIFEST).is_file() {
        return host;
    }
    match (std::fs::read(folder.join(RELEASE_JSON)), std::fs::read(bin.join(RELEASE_JSON))) {
        (Ok(theirs), Ok(ours)) if theirs == ours => {}
        _ => {
            crate::log::say!(
                "humatad2: warning: the machine tools in {} are from another release than this daemon, so Linux machines can't be set up until it is migrated; use Migrate on This Mac, or open Humata again",
                folder.display()
            );
            // Adding a machine of those platforms says to Migrate This Mac first.
            return match Bundle::open(&folder) {
                Ok(tools) => host.with_stale_platforms(tools.platforms()),
                Err(_) => host,
            };
        }
    }
    match Bundle::open(&folder) {
        Ok(tools) => host.with_platforms_from(tools),
        Err(error) => {
            crate::log::say!(
                "humatad2: warning: couldn't read the machine tools in {}: {error}; open Humata again so it installs them",
                folder.display()
            );
            host
        }
    }
}

/// Gives every saved agent launch this daemon's Humata tools arguments
/// for its kind (D748), so an agent saved before them resumes with them.
/// One that can't be read or saved resumes as it was, and the log says so.
fn tools_in_saved_launches(store: &humata2_store::Store, agents: &humata2_sessions::Agents) {
    let saved = match store.agents() {
        Ok(saved) => saved,
        Err(error) => {
            crate::log::say!(
                "humatad2: couldn't read the saved agents to give them Humata's tools, so agents started before this version resume without them until restarted: {}",
                crate::status::chain(&error)
            );
            return;
        }
    };
    for mut agent in saved {
        if !agents.with_tools_in(&mut agent.launch) {
            continue;
        }
        if let Err(error) = store.put_agent(&agent) {
            crate::log::say!(
                "humatad2: couldn't save session {}'s agent with Humata's tools, so it resumes without them: {}",
                agent.session,
                crate::status::chain(&error)
            );
        }
    }
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
/// A lock still taken after `wait` is a second daemon on the same state,
/// refused here; the pid the lock holder wrote is what the refusal names.
async fn lock_state(state: &Path, wait: std::time::Duration) -> Result<File, DaemonError> {
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
        Err(TryLockError::Error(source)) => return Err(lock_failed(source)),
        Err(TryLockError::WouldBlock) => {
            // Wait on the lock itself, so it's taken the moment it's let go.
            // The wait is a thread of its own, not the runtime's, so a wait
            // that runs out doesn't hold up the runtime's shutdown: the
            // thread stays blocked until the process ends, or gets the lock
            // and lets go of it at once, as nobody takes the file from it.
            let (taken, waiting) = tokio::sync::oneshot::channel();
            std::thread::Builder::new()
                .name("state-lock".into())
                .spawn(move || {
                    let _ = taken.send(file.lock().map(|()| file));
                })
                .map_err(lock_failed)?;
            file = match tokio::time::timeout(wait, waiting).await {
                Ok(Ok(Ok(file))) => file,
                Ok(Ok(Err(source))) => return Err(lock_failed(source)),
                Ok(Err(gone)) => return Err(lock_failed(std::io::Error::other(gone))),
                Err(_elapsed) => return Err(state_in_use(state, &path)),
            };
        }
    }
    // Only the lock's holder writes the file, so the pid written is this one's.
    file.set_len(0)
        .and_then(|()| file.write_all(format!("{}\n", std::process::id()).as_bytes()))
        .and_then(|()| file.sync_data())
        .map_err(lock_failed)?;
    Ok(file)
}

/// The refusal for a state folder another daemon holds, naming the pid
/// that daemon wrote in the lock file when it reads.
fn state_in_use(state: &Path, lock_file: &Path) -> DaemonError {
    let pid = std::fs::read_to_string(lock_file)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok());
    match pid {
        Some(pid) => DaemonError::StateInUse {
            state: state.to_owned(),
            pid,
        },
        None => DaemonError::StateInUseUnknownPid {
            state: state.to_owned(),
        },
    }
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

/// The home the agents' CLIs run in (B5): `$HOME` when it is set, since
/// they keep their conversations under it and `--root` doesn't move them;
/// `root` only when there is none.
fn agent_home(home: Option<std::ffi::OsString>, root: &Path) -> PathBuf {
    home.filter(|home| !home.is_empty())
        .map_or_else(|| root.to_owned(), PathBuf::from)
}

#[cfg(test)]
mod tests {

    /// B5: `{home}` in an agent's conversation file is the user's home,
    /// not the daemon's `--root`.
    #[test]
    fn the_agents_home_is_home_not_the_root() {
        let root = Path::new("/var/humata-root");
        assert_eq!(
            agent_home(Some("/Users/someone".into()), root),
            PathBuf::from("/Users/someone")
        );
        assert_eq!(agent_home(Some("".into()), root), root);
        assert_eq!(agent_home(None, root), root);
    }
    use std::sync::{Arc, Barrier};

    use super::*;

    /// Another holder of `state`'s lock: a description of its own, as a
    /// second daemon's would be, having written `pid`.
    fn hold_lock(state: &Path, pid: u32) -> File {
        let path = state.join(LOCK_FILE);
        std::fs::write(&path, format!("{pid}\n")).unwrap();
        let file = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        file.try_lock().unwrap();
        file
    }

    const LINUX: Platform = Platform { os: humata2_machines::Os::Linux, arch: humata2_machines::Arch::X86_64 };
    const MAC: Platform = Platform { os: humata2_machines::Os::Darwin, arch: humata2_machines::Arch::Aarch64 };

    /// A humata home whose `machine-tools` carry linux-x86_64 under release
    /// `tools_release`, and a `bin` whose bundle.json is `bin_release`.
    fn home_with_machine_tools(tools_release: &str, bin_release: &str) -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let tools = home.path().join(MACHINE_TOOLS);
        std::fs::create_dir_all(&tools).unwrap();
        let entries: Vec<String> = ["humatad2", "humata2_holder", "humata", "stack"]
            .iter()
            .map(|name| format!(r#""{name}": {{"path": "linux-x86_64/{name}", "sha256": "{}"}}"#, "0".repeat(64)))
            .collect();
        std::fs::write(
            tools.join(MANIFEST),
            format!(r#"{{"platforms": {{"linux-x86_64": {{{}}}}}}}"#, entries.join(", ")),
        )
        .unwrap();
        std::fs::write(tools.join(RELEASE_JSON), tools_release).unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(RELEASE_JSON), bin_release).unwrap();
        (home, bin)
    }

    #[test]
    fn machine_tools_of_this_release_add_their_platforms_to_the_host() {
        let (home, bin) = home_with_machine_tools("release-a", "release-a");

        let bundle = with_machine_tools(Bundle::beside(&bin, MAC), home.path(), &bin);

        assert_eq!(bundle.platforms(), [MAC, LINUX]);
    }

    #[test]
    fn machine_tools_of_another_release_are_left_out() {
        let (home, bin) = home_with_machine_tools("release-b", "release-a");

        let bundle = with_machine_tools(Bundle::beside(&bin, MAC), home.path(), &bin);

        assert_eq!(bundle.platforms(), [MAC]);
    }

    #[test]
    fn a_linux_add_after_an_app_update_asks_to_migrate_this_mac_first() {
        let (home, bin) = home_with_machine_tools("release-b", "release-a");

        let bundle = with_machine_tools(Bundle::beside(&bin, MAC), home.path(), &bin);

        assert!(matches!(
            bundle.payloads(LINUX),
            Err(humata2_machines::BuildsError::HostNeedsMigrate { platform }) if platform == LINUX
        ));
    }

    #[test]
    fn a_linux_add_with_current_machine_tools_does_not_ask_to_migrate() {
        let (home, bin) = home_with_machine_tools("release-a", "release-a");

        let bundle = with_machine_tools(Bundle::beside(&bin, MAC), home.path(), &bin);

        // The files themselves are not there in this fixture, so it is a missing
        // file, never the Migrate reason.
        assert!(!matches!(
            bundle.payloads(LINUX),
            Err(humata2_machines::BuildsError::HostNeedsMigrate { .. })
        ));
    }

    #[test]
    fn no_machine_tools_leave_the_host_alone() {
        let home = tempfile::tempdir().unwrap();

        let bundle = with_machine_tools(Bundle::beside(home.path(), MAC), home.path(), home.path());

        assert_eq!(bundle.platforms(), [MAC]);
    }

    #[test]
    fn an_unreadable_machine_tools_manifest_leaves_the_host_alone() {
        let (home, bin) = home_with_machine_tools("release-a", "release-a");
        std::fs::write(home.path().join(MACHINE_TOOLS).join(MANIFEST), "not json").unwrap();

        let bundle = with_machine_tools(Bundle::beside(&bin, MAC), home.path(), &bin);

        assert_eq!(bundle.platforms(), [MAC]);
    }

    #[tokio::test]
    async fn a_state_lock_let_go_within_the_wait_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let held = hold_lock(dir.path(), 4242);
        let letting_go = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            drop(held);
        });
        let taken = lock_state(dir.path(), std::time::Duration::from_secs(10)).await;
        assert!(taken.is_ok(), "{taken:?}");
        letting_go.await.unwrap();
        let written = std::fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap();
        assert_eq!(written.trim(), std::process::id().to_string());
    }

    #[tokio::test]
    async fn a_state_lock_held_past_the_wait_is_refused_naming_its_holder() {
        let dir = tempfile::tempdir().unwrap();
        let _held = hold_lock(dir.path(), 4242);
        let refused = lock_state(dir.path(), std::time::Duration::from_millis(100)).await;
        assert!(
            matches!(refused, Err(DaemonError::StateInUse { pid: 4242, .. })),
            "{refused:?}"
        );
    }

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
