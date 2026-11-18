//! The PER-28 suite's rig (D243): power-cut the host, a box or both at
//! every fault point an operation reaches, bring them back, and check that
//! everything PER-25 lists is back or is named in the restart report.
//!
//! One host and one box, both on the fault build with
//! `HUMATA2_POWER_CUT_ROOTS` naming each side's state and channels folders
//! (the holders' folders are under the state folder), so a stop at a point
//! is a power cut: what its processes wrote through humata2_durable and
//! didn't sync is undone first. Writes that bypass humata2_durable (T6)
//! are not undone, so a pass here proves less for the items that depend on
//! them.
//!
//! A test is one operation: it runs once with the trace on (to learn the
//! points it reaches on each side), then once per point on a fresh rig of
//! its own, with `HUMATA2_STOP_AT=<point>` armed. The tests of a binary
//! share [`AT_ONCE`] rigs, so its operations run side by side.
//! `HUMATA2_FAULT_ARM` keeps the setup's own hits from counting, so the
//! cut is at the operation's first hit.
//!
//! Use it as
//! `mod common; #[path = "common/remote_rig.rs"] mod remote_rig;
//! #[path = "common/power_cut.rs"] mod power_cut;`.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use rustix::process::{Pid, Signal};
use tokio::io::AsyncReadExt;
use tonic::transport::Channel;

use crate::common::{self, Attach, Daemon};
use crate::remote_rig::{LONG, Rig, is_live, is_offline};

const REPO: (&str, &str) = ("acme", "api");

/// The box's name.
pub const BOX: &str = "box-1";

/// How many rigs (a host, a box and their holders) the tests of one test
/// binary run at once, traces and cases of every operation together.
/// `HUMATA2_POWER_CUT_AT_ONCE` sets another number, such as 1 to run them
/// one after another.
const AT_ONCE: usize = 8;

/// How long a stop point may take to fire once the operation is asked for.
/// A point a trace saw that never fires is a failure, not a hang.
const POINT_WAIT: Duration = Duration::from_secs(90);

/// A word the comment carries, to count how often it was typed.
const MARKER: &str = "powercutmarker";

/// The host variable the setup saves, and the one the operation adds
/// (name, value): after any cut the secrets file holds the first alone or
/// both.
const HOST_VARIABLE_OLD: (&str, &str) = ("OLD_TOKEN", "old-secret-value");
const HOST_VARIABLE_NEW: (&str, &str) = ("NEW_TOKEN", "new-secret-value");

fn host_variable((name, value): (&str, &str)) -> pb::SetHostVariableRequest {
    pb::SetHostVariableRequest {
        idempotency_key: None,
        id: None,
        name: name.to_owned(),
        value: value.to_owned(),
        presence: None,
    }
}

/// What each base session's agent was told, one line at a time.
const LINES: [&str; 3] = ["out-1", "out-2", "out-3"];

/// What is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// The host's daemon and holders; the box keeps running.
    Host,
    /// The box's daemon and holders; the host keeps running.
    Machine,
    /// Both, one stopped at the point and the other at once after.
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Host,
    Box,
}

/// The multi-step operations whose points are cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// A session started on the host and one on the box.
    Start,
    /// A code-and-context fork on the host.
    LocalFork,
    /// A code-and-context fork of the host's session onto the box.
    RemoteFork,
    /// The host's session moved to the box.
    Move,
    /// The box's session archived there.
    RemoteArchive,
    /// The box's archived session, its clone freed, restored on the host.
    RemoteRestore,
    /// The host's draft review comment sent to its agent.
    CommentSend,
    /// A file copied from the host's session to the box's shared folder.
    Transfer,
    /// A host variable set on the host, so its secrets file is rewritten.
    HostVariable,
    /// A host variable set on the box through the host's relay, so the box's
    /// secrets file is rewritten.
    RemoteHostVariable,
}

impl Operation {
    /// Whether the operation does work on `side`, so a trace with no point
    /// there is a failure and not a quiet pass.
    fn runs_on(self, side: Side) -> bool {
        match self {
            Self::LocalFork | Self::CommentSend | Self::Transfer | Self::HostVariable => side == Side::Host,
            Self::RemoteHostVariable => side == Side::Box,
            _ => true,
        }
    }

    /// What names the things the operation changes: their keys in an
    /// inventory contain one of these.
    fn tokens(self, base: &Base) -> Vec<String> {
        match self {
            Self::Start | Self::LocalFork | Self::RemoteFork => Vec::new(),
            Self::Move | Self::CommentSend => {
                vec![base.h.value.clone(), "host-work".to_owned(), base.h_conversation.clone()]
            }
            Self::RemoteArchive | Self::RemoteRestore => {
                vec![base.b.value.clone(), "box-work".to_owned(), base.b_conversation.clone()]
            }
            Self::Transfer => vec!["transfer:".to_owned(), "shared".to_owned()],
            Self::HostVariable | Self::RemoteHostVariable => vec!["secret:".to_owned()],
        }
    }

    /// Whether an item that wasn't there before may be there after.
    fn creates(self, key: &str) -> bool {
        let kinds: &[&str] = match self {
            Self::Start | Self::LocalFork | Self::RemoteFork => {
                &["session:", "tabs:", "dir:", "clone:", "conv:", "agent:", "file:"]
            }
            // A finished move trashes the session folder and conversation
            // it left on the old machine.
            Self::Move => &["dir:", "clone:", "conv:", "agent:", "tabs:", "file:", "trash:"],
            Self::RemoteArchive | Self::RemoteRestore => {
                &["dir:", "clone:", "conv:", "agent:", "tabs:", "file:"]
            }
            Self::CommentSend => &["conv:"],
            Self::Transfer => &["transfer:", "dir:", "file:"],
            // The file is there from the setup; nothing else is made.
            Self::HostVariable | Self::RemoteHostVariable => &[],
        };
        kinds.iter().any(|kind| key.starts_with(*kind))
    }

    /// Asks for the operation and returns; the daemon may die under it, so
    /// no answer is looked at. Whether it finished is [`Self::finished`].
    async fn run(self, context: Context) {
        let base = &context.base;
        let mut sessions = pb::session_service_client::SessionServiceClient::new(context.channel.clone());
        let channel_id = base.channel.id.clone().unwrap_or_default();
        match self {
            Self::Start => {
                for (machine, title) in [(&base.host, "Fresh host"), (&base.boxm, "Fresh box")] {
                    let _ = sessions.start_session(common::start_request(&channel_id, machine, title)).await;
                }
            }
            Self::LocalFork | Self::RemoteFork => {
                let (machine, title) = match self {
                    Self::LocalFork => (None, "Try it"),
                    _ => (Some(base.boxm.clone()), "Try it remote"),
                };
                let _ = sessions
                    .fork_session(pb::ForkSessionRequest {
                        idempotency_key: None,
                        parent: Some(base.h.clone()),
                        kind: pb::ForkKind::CodeAndContext as i32,
                        title: title.to_owned(),
                        machine,
                        ..Default::default()
                    })
                    .await;
            }
            Self::Move => {
                let _ = sessions
                    .move_session(pb::MoveSessionRequest {
                        idempotency_key: Some(pb::IdempotencyKey { value: "power-cut-move".to_owned() }),
                        id: Some(base.h.clone()),
                        to: Some(base.boxm.clone()),
                    })
                    .await;
            }
            Self::RemoteArchive => {
                let Ok(plan) = sessions.plan_archive(pb::PlanArchiveRequest { id: Some(base.b.clone()) }).await else {
                    return;
                };
                let plan = plan.into_inner();
                let _ = sessions
                    .archive_session(pb::ArchiveSessionRequest {
                        idempotency_key: None,
                        id: Some(base.b.clone()),
                        choice: pb::ArchiveChoice::Archive as i32,
                        plan: plan.token,
                    })
                    .await;
            }
            Self::RemoteRestore => {
                let _ = sessions
                    .restore_session(pb::RestoreSessionRequest {
                        idempotency_key: None,
                        id: Some(base.b.clone()),
                        machine: Some(base.host.clone()),
                    })
                    .await;
            }
            Self::CommentSend => {
                let mut comments = pb::comment_service_client::CommentServiceClient::new(context.channel.clone());
                let _ = comments
                    .send_comments(pb::SendCommentsRequest {
                        idempotency_key: None,
                        session: Some(base.h.clone()),
                        ids: vec![base.comment.clone()],
                    })
                    .await;
            }
            Self::Transfer => {
                let mut transfers = pb::transfer_service_client::TransferServiceClient::new(context.channel.clone());
                let from = pb::FileLocation {
                    machine: None,
                    root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                        session: Some(base.h.clone()),
                        repo: Some(common::repo_ref(REPO.0, REPO.1)),
                    })),
                    path: "README.md".to_owned(),
                };
                let to = pb::FileLocation {
                    machine: Some(base.boxm.clone()),
                    root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                        channel: base.channel.id.clone(),
                    })),
                    path: "in".to_owned(),
                };
                let _ = transfers
                    .start_transfer(pb::StartTransferRequest {
                        idempotency_key: None,
                        sources: vec![from],
                        destination: Some(to),
                        mode: pb::TransferMode::Copy as i32,
                        on_existing: pb::OnExisting::Fail as i32,
                        reason: String::new(),
                        name: String::new(),
                    })
                    .await;
            }
            Self::HostVariable | Self::RemoteHostVariable => {
                let id = (self == Self::RemoteHostVariable).then(|| base.boxm.clone());
                let mut machines = pb::machine_service_client::MachineServiceClient::new(context.channel.clone());
                let _ = machines
                    .set_host_variable(pb::SetHostVariableRequest { id, ..host_variable(HOST_VARIABLE_NEW) })
                    .await;
            }
        }
    }

    /// Whether the operation has reached its end, as the host's API shows.
    async fn finished(self, channel: &Channel, base: &Base) -> bool {
        let sessions = list_sessions(channel).await;
        let by_id = |id: &pb::SessionId| sessions.iter().find(|session| session.id.as_ref() == Some(id));
        let titled = |title: &str| sessions.iter().find(|session| session.title == title);
        let live_on = |session: Option<&pb::Session>, machine: &pb::MachineId| {
            session.is_some_and(|session| session.machine.as_ref() == Some(machine) && is_live(session))
        };
        match self {
            Self::Start => live_on(titled("Fresh host"), &base.host) && live_on(titled("Fresh box"), &base.boxm),
            Self::LocalFork => live_on(titled("Try it"), &base.host),
            Self::RemoteFork => live_on(titled("Try it remote"), &base.boxm),
            Self::Move => live_on(by_id(&base.h), &base.boxm),
            Self::RemoteArchive => by_id(&base.b).is_some_and(|session| session.archive.is_some()),
            Self::RemoteRestore => {
                by_id(&base.b).is_some_and(|session| session.archive.is_none()) && live_on(by_id(&base.b), &base.host)
            }
            Self::CommentSend => comments_of(channel, &base.h)
                .await
                .iter()
                .any(|comment| comment.id.as_ref() == Some(&base.comment) && comment.status() == pb::CommentStatus::Sent),
            Self::HostVariable | Self::RemoteHostVariable => {
                let id = (self == Self::RemoteHostVariable).then(|| base.boxm.clone());
                let mut machines = pb::machine_service_client::MachineServiceClient::new(channel.clone());
                match machines.list_host_variables(pb::ListHostVariablesRequest { id }).await {
                    Ok(listed) => listed.into_inner().names.iter().any(|name| name == HOST_VARIABLE_NEW.0),
                    Err(_) => false,
                }
            }
            Self::Transfer => {
                let mut transfers = pb::transfer_service_client::TransferServiceClient::new(channel.clone());
                match transfers.list_transfers(pb::ListTransfersRequest::default()).await {
                    Ok(listed) => listed
                        .into_inner()
                        .transfers
                        .iter()
                        .any(|transfer| transfer.state() == pb::TransferState::Done),
                    Err(_) => false,
                }
            }
        }
    }
}

/// What an operation's task needs, owned so it can run on its own.
#[derive(Clone)]
struct Context {
    channel: Channel,
    base: Base,
}

/// One session's agent terminal as a client saw it before the cut.
#[derive(Clone)]
struct Output {
    session: pb::SessionId,
    tab: pb::TabId,
    screen: String,
}

/// What the setup made, on both sides.
#[derive(Clone)]
struct Base {
    channel: pb::Channel,
    host: pb::MachineId,
    boxm: pb::MachineId,
    /// A session on the host.
    h: pb::SessionId,
    /// A session on the box.
    b: pb::SessionId,
    h_conversation: String,
    b_conversation: String,
    /// A draft comment on `h`.
    comment: pb::CommentId,
    outputs: Vec<Output>,
}

// ---------------------------------------------------------------------------
// The rig.

struct Faults {
    /// The side and point to power-cut at.
    point: Option<(Side, String)>,
    trace: bool,
}

fn side_env(scratch: &Path, side: Side, faults: &Faults, operation: Operation) -> Vec<(String, String)> {
    let text = |path: PathBuf| path.display().to_string();
    let mut env = vec![
        ("HUMATA2_POWER_CUT_ROOTS".to_owned(), "{state}:{channels}".to_owned()),
        ("HUMATA2_POWER_CUT_SPARE".to_owned(), "{dir}/spare".to_owned()),
        ("HUMATA2_FAULT_ARM".to_owned(), text(scratch.join("arm"))),
    ];
    // A restore's setup archives the session and waits until the box has
    // freed its clone; the restore itself starts no undo window. Without
    // the window's 10 seconds that setup ends as soon as the free does.
    if operation == Operation::RemoteRestore {
        env.push(("HUMATA2_ARCHIVE_UNDO_MS".to_owned(), "0".to_owned()));
    }
    if faults.trace {
        let name = if side == Side::Host { "trace-host" } else { "trace-box" };
        env.push(("HUMATA2_FAULT_TRACE".to_owned(), text(scratch.join(name))));
    }
    if let Some((origin, name)) = &faults.point
        && *origin == side
    {
        env.push(("HUMATA2_STOP_AT".to_owned(), name.clone()));
    }
    env
}

async fn start_rig(faults: &Faults, scratch: &Path, operation: Operation) -> Rig {
    let (host_env, box_env) =
        (side_env(scratch, Side::Host, faults, operation), side_env(scratch, Side::Box, faults, operation));
    let host: Vec<(&str, &str)> = host_env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
    let boxes: Vec<(&str, &str)> = box_env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
    Rig::start_with_env(&[BOX], &[REPO], &host, &boxes).await
}

fn state_of(rig: &Rig, side: Side) -> PathBuf {
    match side {
        Side::Host => rig.host.state.clone(),
        Side::Box => rig.the(BOX).daemon().state.clone(),
    }
}

/// Polls `check` until it holds. The daemons tell no feed of what a start
/// settles, so this reads and reads again, `LONG` at most.
async fn until(what: &str, check: impl AsyncFn() -> bool) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + LONG;
    while !check().await {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("never saw {what} within {LONG:?}"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

/// The session clients, one for each thing the setup and the inventory read.
fn channel_client(channel: &Channel) -> pb::channel_service_client::ChannelServiceClient<Channel> {
    pb::channel_service_client::ChannelServiceClient::new(channel.clone())
}

async fn list_sessions(channel: &Channel) -> Vec<pb::Session> {
    let mut sessions = pb::session_service_client::SessionServiceClient::new(channel.clone());
    sessions
        .list_sessions(pb::ListSessionsRequest {
            include_archived: true,
            ..Default::default()
        })
        .await
        .map(|listed| listed.into_inner().sessions)
        .unwrap_or_default()
}

async fn comments_of(channel: &Channel, session: &pb::SessionId) -> Vec<pb::Comment> {
    let mut comments = pb::comment_service_client::CommentServiceClient::new(channel.clone());
    let mut all = Vec::new();
    for archived in [false, true] {
        if let Ok(listed) = comments
            .list_comments(pb::ListCommentsRequest {
                session: Some(session.clone()),
                archived,
                ..Default::default()
            })
            .await
        {
            all.extend(listed.into_inner().comments);
        }
    }
    all
}

/// Types `lines` to `id`'s agent, one by one, and returns the agent tab
/// and everything a client attached from the start saw.
async fn say(rig: &Rig, id: &pb::SessionId) -> Output {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    let mut attach = Attach::start(&rig.host, open).await;
    attach.wait_for("agent up").await;
    for line in LINES {
        attach.type_text(&format!("{line}\n")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }
    synced(rig, &tab, attach.cursor).await;
    Output {
        session: id.clone(),
        tab,
        screen: String::from_utf8_lossy(&attach.screen).into_owned(),
    }
}

/// Waits until the holder behind `tab`, on whichever side runs it, says
/// its saved output is synced through `printed`. A cut takes back what
/// wasn't synced, by design, and the base's output must outlive every cut.
/// The faults holder writes how far it synced beside its socket.
async fn synced(rig: &Rig, tab: &pb::TabId, printed: u64) {
    let socket = [Side::Host, Side::Box]
        .into_iter()
        .flat_map(|side| common::stored_tabs(&state_of(rig, side)))
        .find(|stored| stored.id == tab.value && stored.holder_socket.exists())
        .map(|stored| stored.holder_socket)
        .expect("the agent tab's holder is in a store");
    let marker = socket.with_extension("synced");
    until(&format!("tab {}'s output synced through offset {printed}", tab.value), async || {
        std::fs::read_to_string(&marker)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .is_some_and(|synced| synced >= printed)
    })
    .await
    .unwrap_or_else(|problem| panic!("{problem}"));
}

fn shared_in(rig: &Rig, channel: &pb::Channel) -> [PathBuf; 2] {
    [
        rig.host_path(channel, "shared").join("in"),
        rig.the(BOX).path(channel, "shared").join("in"),
    ]
}

/// Puts every PER-25 item that exists today on both sides: a channel on
/// both machines, a session and its agent's output on each, a draft review
/// comment, a board item, a pin, a setting, saved view state and a trashed
/// file. Inbox requests and grants need an agent's token flow and are only
/// read (see [`Inventory`]).
async fn setup(rig: &Rig, operation: Operation) -> Base {
    let host = rig.host_id().await;
    let boxm = Rig::box_id(BOX);
    let channel_handle = rig.channel().await;

    pb::settings_service_client::SettingsServiceClient::new(channel_handle.clone())
        .update_settings(pb::UpdateSettingsRequest {
            branch_prefix: Some("pc/".to_owned()),
            ..Default::default()
        })
        .await
        .expect("the branch prefix is set");

    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), boxm.clone()]).await;
    let channel_id = channel.id.clone().expect("a channel id");
    let h = rig.start_on(&channel, &host, "Host work").await;
    let b = rig.start_on(&channel, &boxm, "Box work").await;
    let live_h = rig.wait_session(&h, "live", is_live).await;
    rig.wait_session(&b, "live", is_live).await;
    // Each say waits until its holder has synced what it printed.
    let outputs = vec![say(rig, &h).await, say(rig, &b).await];
    let b_conversation = rig.the(BOX).conversation_id(&b);

    let created = pb::comment_service_client::CommentServiceClient::new(channel_handle.clone())
        .create_comment(pb::CreateCommentRequest {
            idempotency_key: None,
            session: Some(h.clone()),
            anchor: Some(pb::CommentAnchor {
                file: Some(pb::RepoPath {
                    repo: Some(common::repo_ref(REPO.0, REPO.1)),
                    path: "README.md".to_owned(),
                }),
                lines: Some(pb::LineRange { first: 1, last: 1 }),
                side: pb::DiffSide::New as i32,
                ..Default::default()
            }),
            text: format!("{MARKER} rename this"),
            destination: Some(pb::CommentDestination {
                destination: Some(pb::comment_destination::Destination::Terminal(pb::SessionTerminal {})),
            }),
        })
        .await
        .expect("the draft comment is made")
        .into_inner();

    pb::board_service_client::BoardServiceClient::new(channel_handle.clone())
        .create_item(pb::CreateItemRequest {
            board: Some(pb::BoardRef {
                board: Some(pb::board_ref::Board::Channel(channel_id.clone())),
            }),
            title: "Plan the retry".to_owned(),
            column: pb::BoardColumn::ToDo as i32,
            ..Default::default()
        })
        .await
        .expect("the board item is made");

    pb::mark_service_client::MarkServiceClient::new(channel_handle.clone())
        .set_marks(pb::SetMarksRequest {
            entity: Some(pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Session(h.clone())),
            }),
            pinned: Some(true),
            ..Default::default()
        })
        .await
        .expect("the session is pinned");

    // No view state is saved: humatad2 doesn't build ViewStateService yet.
    // The inventory still reads it, so it's covered once it's built.

    // A file in the channel's shared folder, put in the trash.
    let shared = rig.host_path(&channel, "shared");
    std::fs::create_dir_all(&shared).expect("the shared folder");
    std::fs::write(shared.join("old.txt"), "old\n").expect("a file to trash");
    pb::file_service_client::FileServiceClient::new(channel_handle.clone())
        .move_to_trash(pb::MoveToTrashRequest {
            idempotency_key: None,
            locations: vec![pb::FileLocation {
                machine: None,
                root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                    channel: Some(channel_id.clone()),
                })),
                path: "old.txt".to_owned(),
            }],
        })
        .await
        .expect("the file is trashed");

    // What the operation needs that isn't there yet.
    for folder in shared_in(rig, &channel) {
        std::fs::create_dir_all(folder).expect("a folder to copy into");
    }
    if operation == Operation::RemoteRestore {
        prepare_restore(rig, &channel, &b).await;
    }
    if matches!(operation, Operation::HostVariable | Operation::RemoteHostVariable) {
        let id = (operation == Operation::RemoteHostVariable).then(|| boxm.clone());
        pb::machine_service_client::MachineServiceClient::new(channel_handle.clone())
            .set_host_variable(pb::SetHostVariableRequest { id, ..host_variable(HOST_VARIABLE_OLD) })
            .await
            .expect("the first host variable is set");
    }
    reports_quiet(rig).await;

    Base {
        channel,
        host,
        boxm,
        h,
        b,
        h_conversation: live_h.conversation_id,
        b_conversation,
        comment: created.id.expect("the comment has an id"),
        outputs,
    }
}

/// How long the host's kept reports of the box's sessions stay the same
/// before the setup counts them all in.
const REPORTS_QUIET: Duration = Duration::from_secs(3);

/// Waits until the host has kept what the box reports of the setup's
/// sessions: the host's remote reports stay the same for `REPORTS_QUIET`.
/// The cut is armed only after that. Otherwise a report of a setup session
/// lands in the trace while the operation runs, and its write on the host
/// becomes a case although the operation never makes it: a case may never
/// see that report again, so its point never fires.
async fn reports_quiet(rig: &Rig) {
    let hosts = Store::open(rig.host.state.join("store")).expect("the host's store opens");
    let seen = std::cell::RefCell::new(None::<(Vec<humata2_store::RemoteReportRow>, std::time::Instant)>);
    let quiet = until("the host's reports of the box's setup sessions stop changing", async || {
        let Ok(mut rows) = hosts.remote_reports() else {
            return false;
        };
        rows.sort_by(|a, b| (&a.machine, &a.session).cmp(&(&b.machine, &b.session)));
        let mut seen = seen.borrow_mut();
        match seen.as_ref() {
            Some((before, since)) if *before == rows => since.elapsed() >= REPORTS_QUIET,
            _ => {
                *seen = Some((rows, std::time::Instant::now()));
                false
            }
        }
    })
    .await;
    if let Err(problem) = quiet {
        panic!("{problem}");
    }
}

/// The box's session archived, its branch pushed and its clone freed, so a
/// restore has to make a fresh one on the host.
async fn prepare_restore(rig: &Rig, channel: &pb::Channel, id: &pb::SessionId) {
    let branch = rig.session(id).await.repos[0].branch.clone();
    let clone = rig.the(BOX).path(channel, "sessions/box-work/api");
    common::git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let mut sessions = pb::session_service_client::SessionServiceClient::new(rig.channel().await);
    let plan = sessions
        .plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) })
        .await
        .expect("the archive is planned")
        .into_inner();
    let started = sessions
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
        })
        .await
        .expect("the archive starts")
        .into_inner();
    rig.ended(&started).await;
    // The box frees the clone by itself once the undo window ends; nothing
    // on the host's feeds tells of it.
    let freed = until("the box's clone freed", async || !clone.exists()).await;
    if let Err(problem) = freed {
        panic!("{problem}");
    }
    // Freeing ends after the clone goes: the box marks its archive row freed
    // and reports the session, and the host keeps that report. The cut is
    // armed only once they are in, so those writes are the setup's and not
    // the restore's, in the trace and in each case alike.
    let boxes = rig.the(BOX).store();
    let hosts = Store::open(rig.host.state.join("store")).expect("the host's store opens");
    let key = id.value.clone();
    let seen = std::cell::RefCell::new(None::<(humata2_store::RemoteReportRow, std::time::Instant)>);
    let kept = until("the host kept the box's report of the freed clone", async || {
        let Some(row) = boxes
            .archive(&key)
            .ok()
            .flatten()
            .filter(|row| row.state == humata2_store::ArchiveState::Freed)
        else {
            return false;
        };
        let Some(report) = hosts
            .remote_reports()
            .ok()
            .and_then(|rows| rows.into_iter().find(|report| report.session == key))
            .filter(|report| report.reported_unix_ms >= row.free_after_unix_ms)
        else {
            return false;
        };
        // Quiet for a second: no later report is on its way.
        let mut seen = seen.borrow_mut();
        match seen.as_ref() {
            Some((before, since)) if *before == report => since.elapsed() >= Duration::from_secs(1),
            _ => {
                *seen = Some((report, std::time::Instant::now()));
                false
            }
        }
    })
    .await;
    if let Err(problem) = kept {
        panic!("{problem}");
    }
}

// ---------------------------------------------------------------------------
// The inventory.

/// Everything PER-25 lists that a client or a file can tell, read through
/// the host's API (it relays the box's sessions) and the two sides' disks,
/// as one text per item. Volatile fields (times, revisions, offsets,
/// reachability) are left out, so an untouched item reads the same before
/// and after a cut.
pub struct Inventory {
    items: BTreeMap<String, String>,
    sessions: Vec<pb::Session>,
}

fn id_text<T: std::fmt::Debug>(id: &Option<T>) -> String {
    format!("{id:?}")
}

/// The answer's body, or a record of the error in its place.
fn answered<T>(
    result: Result<tonic::Response<T>, tonic::Status>,
    what: &str,
    items: &mut BTreeMap<String, String>,
) -> Option<T> {
    match result {
        Ok(response) => Some(response.into_inner()),
        Err(status) => {
            items.insert(format!("api:{what}"), format!("{:?}", status.code()));
            None
        }
    }
}

fn session_class(session: &pb::Session) -> String {
    if session.archive.is_some() {
        "archived".to_owned()
    } else if is_live(session) {
        "live".to_owned()
    } else if is_offline(session) {
        "offline".to_owned()
    } else {
        format!("{:?}", session.status())
    }
}

impl Inventory {
    pub async fn take(rig: &Rig) -> Self {
        let channel = rig.channel().await;
        let mut items = BTreeMap::new();
        let sessions = api_items(&channel, &mut items).await;
        file_items(rig, &mut items);
        store_items(rig, &mut items);
        Self { items, sessions }
    }
}

async fn api_items(channel: &Channel, items: &mut BTreeMap<String, String>) -> Vec<pb::Session> {
    let mut channel_ids = Vec::new();
    let mut channels = channel_client(channel);
    if let Some(listed) = answered(channels.list_channels(pb::ListChannelsRequest::default()).await, "channels", items) {
        for one in listed.channels {
            let id = one.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
            channel_ids.push(one.id.clone());
            items.insert(
                format!("channel:{id}"),
                format!("{}|{}|{:?}|{:?}", one.name, one.folder, one.repos, one.machines),
            );
            let mut settings = pb::settings_service_client::SettingsServiceClient::new(channel.clone());
            if let Some(own) = answered(
                settings
                    .get_channel_settings(pb::GetChannelSettingsRequest { channel: one.id.clone() })
                    .await,
                &format!("channel-settings-{id}"),
                items,
            ) {
                items.insert(format!("settings:{id}"), format!("{own:?}"));
            }
        }
    }

    let sessions = list_sessions(channel).await;
    let mut terminals = pb::terminal_service_client::TerminalServiceClient::new(channel.clone());
    for session in &sessions {
        let id = session.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let repos: Vec<String> = session
            .repos
            .iter()
            .map(|repo| {
                let name = repo.repo.as_ref().map(|repo| format!("{}/{}", repo.owner, repo.name)).unwrap_or_default();
                format!("{name}@{}@{}@{}", repo.branch, repo.path, repo.main_clone)
            })
            .collect();
        items.insert(
            format!("session:{id}"),
            format!(
                "{}|{}|{:?}|{:?}|{}|{}|{:?}|{}|{}|{:?}|{}",
                session_class(session),
                session.title,
                session.channel,
                session.agent,
                session.model,
                session.machine.as_ref().map(|machine| machine.value.as_str()).unwrap_or_default(),
                repos,
                session.conversation_id,
                session.pinned,
                session.fork_origin.as_ref().map(|origin| id_text(&origin.parent)),
                session.board_item.is_some(),
            ),
        );
        if let Some(tabs) = answered(
            terminals
                .list_tabs(pb::ListTabsRequest { session: session.id.clone() })
                .await,
            &format!("tabs-{id}"),
            items,
        ) {
            let mut named: Vec<String> = tabs
                .tabs
                .iter()
                .map(|tab| {
                    format!(
                        "{}/{:?}/{}",
                        tab.id.as_ref().map(|id| id.value.as_str()).unwrap_or_default(),
                        tab.kind(),
                        tab.title
                    )
                })
                .collect();
            named.sort();
            items.insert(format!("tabs:{id}"), named.join(","));
        }
        if let Some(session_id) = &session.id {
            for comment in comments_of(channel, session_id).await {
                let comment_id = comment.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
                items.insert(
                    format!("comment:{comment_id}"),
                    format!(
                        "{}|{:?}|{}|{}",
                        comment.text,
                        comment.status(),
                        comment.delivery_unconfirmed,
                        comment.archived_at.is_some()
                    ),
                );
            }
        }
    }

    let mut boards = pb::board_service_client::BoardServiceClient::new(channel.clone());
    let mut views = vec![pb::board_view::View::Board(pb::BoardRef {
        board: Some(pb::board_ref::Board::Global(pb::GlobalBoard {})),
    })];
    for id in channel_ids.iter().flatten() {
        views.push(pb::board_view::View::Board(pb::BoardRef {
            board: Some(pb::board_ref::Board::Channel(id.clone())),
        }));
    }
    for view in views {
        let listed = boards
            .list_items(pb::ListItemsRequest {
                view: Some(pb::BoardView { view: Some(view) }),
                ..Default::default()
            })
            .await;
        if let Some(listed) = answered(listed, "board", items) {
            for item in listed.items {
                items.insert(
                    format!("board:{}", id_text(&item.id)),
                    format!("{}|{:?}|{:?}|{}", item.title, item.column(), item.sessions, item.description),
                );
            }
        }
    }

    let mut transfers = pb::transfer_service_client::TransferServiceClient::new(channel.clone());
    if let Some(listed) = answered(transfers.list_transfers(pb::ListTransfersRequest::default()).await, "transfers", items) {
        for transfer in listed.transfers {
            items.insert(
                format!("transfer:{}", id_text(&transfer.id)),
                format!("{:?}|{}|{}", transfer.state(), transfer.done_files, transfer.total_files),
            );
        }
    }

    let mut trash = pb::trash_service_client::TrashServiceClient::new(channel.clone());
    if let Some(listed) = answered(trash.list_trash(pb::ListTrashRequest::default()).await, "trash", items) {
        for item in listed.items {
            items.insert(
                format!("trash:{}", id_text(&item.id)),
                format!("{}|{}|{:?}", item.description, item.bytes, item.machine),
            );
        }
    }

    let mut settings = pb::settings_service_client::SettingsServiceClient::new(channel.clone());
    if let Some(own) = answered(settings.get_settings(pb::GetSettingsRequest {}).await, "settings", items) {
        items.insert("settings:global".to_owned(), format!("{own:?}"));
    }

    let mut marks = pb::mark_service_client::MarkServiceClient::new(channel.clone());
    if let Some(listed) = answered(marks.list_pinned(pb::ListPinnedRequest {}).await, "pins", items) {
        for entry in listed.entries {
            items.insert(format!("pin:{:?}", entry.entity), format!("{}|{}", entry.title, entry.vip));
        }
    }

    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(channel.clone());
    let requests = inbox
        .list_requests(pb::ListRequestsRequest {
            include_answered: true,
            ..Default::default()
        })
        .await;
    if let Some(listed) = answered(requests, "inbox", items) {
        for request in listed.requests {
            items.insert(
                format!("inbox:{}", id_text(&request.id)),
                format!("{}|{}|{:?}|{}", request.kind, request.state, request.answer, request.action_title),
            );
        }
    }

    let mut access = pb::access_service_client::AccessServiceClient::new(channel.clone());
    if let Some(listed) = answered(access.list_grants(pb::ListGrantsRequest::default()).await, "grants", items) {
        for grant in listed.grants {
            items.insert(
                format!("grant:{}", id_text(&grant.id)),
                format!(
                    "{:?}|{:?}|{:?}|{:?}|{}",
                    grant.channel, grant.capability, grant.requester, grant.target, grant.suspended
                ),
            );
        }
    }

    let mut machines = pb::machine_service_client::MachineServiceClient::new(channel.clone());
    if let Some(listed) = answered(machines.list_machines(pb::ListMachinesRequest::default()).await, "machines", items) {
        for machine in listed.machines {
            items.insert(
                format!("machine:{}", machine.name),
                format!("{}|{}|{}|{}", machine.address, machine.root, machine.pinned, machine.vip),
            );
        }
    }

    let mut agents = pb::agent_service_client::AgentServiceClient::new(channel.clone());
    if let Some(listed) = answered(agents.list_agents(pb::ListAgentsRequest::default()).await, "agents", items) {
        for agent in listed.agents {
            items.insert(format!("agentdef:{}", agent.name), format!("{}|{}", agent.launch_command, agent.resume_arguments));
        }
    }

    let mut view = pb::view_state_service_client::ViewStateServiceClient::new(channel.clone());
    let saved = view
        .get_view_state(pb::GetViewStateRequest {
            key_prefixes: vec![String::new()],
        })
        .await;
    if let Some(saved) = answered(saved, "view-state", items) {
        for entry in saved.entries {
            items.insert(format!("view:{}", entry.key), format!("{:?}", entry.value));
        }
    }
    sessions
}

/// `git`'s answer in `path`, or `-` when it has none (not a clone, or one
/// half made).
fn git_says(path: &Path, args: &[&str]) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "-".to_owned())
}

/// The folders of a side's channels as they are: channel folders, session
/// folders and clones (each with its branch and head), the shared folders
/// and the trash. A session's clone isn't entered.
fn walk(side: &str, root: &Path, at: &Path, items: &mut BTreeMap<String, String>) {
    let Ok(read) = std::fs::read_dir(at) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let text = relative.to_string_lossy().into_owned();
        let parts: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        let second = parts.get(1).map(String::as_str);
        if !metadata.is_dir() {
            // Only the shared folders' and the trash's files: the channel
            // folders' own bookkeeping files change with every session.
            if second == Some("shared") || parts.first().map(String::as_str) == Some(".trash") {
                items.insert(format!("file:{side}:{text}"), metadata.len().to_string());
            }
            continue;
        }
        if parts.len() == 4 && second == Some("sessions") && parts[3] == ".humata" {
            // A session folder's own bookkeeping (a fork's handover and copy
            // note, D181; a contained agent's temp, D217): not a clone, and
            // it changes with the session like the channel folders' files.
            continue;
        }
        items.insert(format!("dir:{side}:{text}"), "dir".to_owned());
        if parts.len() == 4 && second == Some("sessions") {
            items.insert(
                format!("clone:{side}:{text}"),
                format!(
                    "{} {}",
                    git_says(&path, &["rev-parse", "--abbrev-ref", "HEAD"]),
                    git_says(&path, &["rev-parse", "HEAD"])
                ),
            );
        } else if second == Some("main") && parts.len() >= 3 {
            // A main clone is the channel's, not a session's.
        } else if parts.len() < 4
            || (parts.first().map(String::as_str) == Some(".trash") && parts.len() < 7)
            || (second == Some("shared") && parts.len() < 7)
        {
            walk(side, root, &path, items);
        }
    }
}

/// Each machine's secrets folder: the host variables file's content, as the
/// operations that rewrite it are judged by (check_host_variables says what
/// it may hold).
fn secret_items(rig: &Rig, items: &mut BTreeMap<String, String>) {
    for (side, state) in [("host", state_of(rig, Side::Host)), ("box", state_of(rig, Side::Box))] {
        let Ok(read) = std::fs::read_dir(state.join("secrets")) else {
            continue;
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // The lock file has no content to keep; a temporary file is
            // leftover of a cut, which check_host_variables looks at.
            if name.ends_with(".json") {
                let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                items.insert(format!("secret:{side}:{name}"), content);
            }
        }
    }
}

fn file_items(rig: &Rig, items: &mut BTreeMap<String, String>) {
    secret_items(rig, items);
    let host_root = rig.world.channels.clone();
    let box_root = rig.the(BOX).channels();
    walk("host", &host_root, &host_root, items);
    walk("box", &box_root, &box_root, items);
    for (side, dir) in [("host", rig.world.conv.clone()), ("box", rig.the(BOX).home().join("conv"))] {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
            items.insert(format!("conv:{side}:{name}"), content);
        }
    }
}

/// The agents each side's store holds, with their conversation ids.
fn store_items(rig: &Rig, items: &mut BTreeMap<String, String>) {
    for (side, state) in [("host", state_of(rig, Side::Host)), ("box", state_of(rig, Side::Box))] {
        let Ok(store) = Store::open(state.join("store")) else {
            continue;
        };
        for agent in store.agents().unwrap_or_default() {
            items.insert(format!("agent:{side}:{}", agent.session), agent.launch.conversation_id.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// What a restart said.

/// The newest restart report of a state folder: the folder with a
/// `restart.json`, last by name (the names are times).
fn newest_report(state: &Path) -> Option<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(state.join("restarts"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| folder.join("restart.json").exists())
        .collect();
    folders.sort();
    folders.pop()
}

fn report_count(state: &Path) -> usize {
    std::fs::read_dir(state.join("restarts"))
        .map(|read| {
            read.flatten()
                .filter(|entry| entry.path().join("restart.json").exists())
                .count()
        })
        .unwrap_or(0)
}

/// An operation line, as its last line in `operations.jsonl` says.
struct Recorded {
    kind: String,
    step: String,
    state: String,
}

fn operations_of(report: &Path) -> Vec<Recorded> {
    let mut latest: Vec<(String, Recorded)> = Vec::new();
    let text = std::fs::read_to_string(report.join("operations.jsonl")).unwrap_or_default();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let field = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
        let entry = Recorded {
            kind: field("kind"),
            step: field("step"),
            state: field("state"),
        };
        match latest.iter_mut().find(|(id, _)| *id == field("id")) {
            Some(known) => known.1 = entry,
            None => latest.push((field("id"), entry)),
        }
    }
    latest.into_iter().map(|(_, entry)| entry).collect()
}

/// What the newest report of each side names as not brought back.
struct Reports {
    /// Sessions a restart.json lists as recoverable or failed.
    unrestored: BTreeSet<String>,
    /// Operations a restart left stuck, as `kind@step`.
    stuck: Vec<String>,
}

impl Reports {
    fn read(rig: &Rig) -> Self {
        let mut unrestored = BTreeSet::new();
        let mut stuck = Vec::new();
        for side in [Side::Host, Side::Box] {
            let Some(report) = newest_report(&state_of(rig, side)) else {
                continue;
            };
            let json = std::fs::read_to_string(report.join("restart.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
            for entry in json.as_ref().and_then(|json| json["sessions"].as_array()).into_iter().flatten() {
                let status = entry["status"].as_str().unwrap_or_default();
                if let (Some(session), true) = (entry["session"].as_str(), ["recoverable", "failed"].contains(&status)) {
                    unrestored.insert(session.to_owned());
                }
            }
            for operation in operations_of(&report) {
                if operation.state == "stuck" {
                    stuck.push(format!("{}@{}", operation.kind, operation.step));
                }
            }
        }
        Self { unrestored, stuck }
    }

    /// Whether the reports name `key`'s thing as not brought back: one of
    /// its sessions is listed unrestored, or an operation is stuck and the
    /// thing is one the operation changes.
    fn names(&self, key: &str, touched: bool) -> bool {
        self.unrestored.iter().any(|session| key.contains(session.as_str())) || (touched && !self.stuck.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Cutting.

/// The distinct names in a trace file, first reached first.
fn traced(file: &Path) -> Vec<String> {
    let mut seen = BTreeSet::new();
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter(|name| !name.is_empty() && seen.insert((*name).to_owned()))
        .map(str::to_owned)
        .collect()
}

struct Traced {
    host: Vec<String>,
    boxes: Vec<String>,
}

/// Runs `operation` once with the trace on and returns the points each
/// side reached once the setup was over.
async fn trace(operation: Operation) -> Traced {
    let scratch = common::folder();
    let faults = Faults { point: None, trace: true };
    let rig = start_rig(&faults, scratch.path(), operation).await;
    let base = setup(&rig, operation).await;
    std::fs::write(scratch.path().join("arm"), b"").expect("the arm file is written");
    let context = Context {
        channel: rig.channel().await,
        base: base.clone(),
    };
    operation.run(context).await;
    let channel = rig.channel().await;
    let done = until("the operation finished", async || operation.finished(&channel, &base).await).await;
    if let Err(problem) = done {
        panic!("the traced {operation:?} never finished, so its points can't be told: {problem}");
    }
    // `finished` reads the host's view; a box's part can land after it (it
    // lets its copy of a restored session go once the host compared it,
    // D224), so each side's trace is read once it stops growing.
    for (side, file) in [(Side::Host, "trace-host"), (Side::Box, "trace-box")] {
        if !operation.runs_on(side) {
            continue;
        }
        let path = scratch.path().join(file);
        let length = || std::fs::metadata(&path).map_or(0, |meta| meta.len());
        // An empty trace stays empty; the check below names it.
        if until(&format!("a point traced on the {side:?} side"), async || length() > 0).await.is_err() {
            continue;
        }
        let mut last = length();
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let now = length();
            if now == last {
                break;
            }
            last = now;
        }
    }
    let found = Traced {
        host: traced(&scratch.path().join("trace-host")),
        boxes: traced(&scratch.path().join("trace-box")),
    };
    assert!(
        !found.host.is_empty() || !found.boxes.is_empty(),
        "no fault point was reached while {operation:?} ran: the trace is empty, so the suite would cut nothing"
    );
    for side in [Side::Host, Side::Box] {
        let points = if side == Side::Host { &found.host } else { &found.boxes };
        assert!(
            !operation.runs_on(side) || !points.is_empty(),
            "{operation:?} reached no fault point on the {side:?} side, where it does work"
        );
    }
    found
}

/// The pairs of side and point that a cut reaches. The host's points are
/// cut only for an operation that does work on the host: one that works on
/// the box alone (a box's host variable) leaves the host saving only the
/// reports the box sends of its setup sessions, which the trace may catch
/// and a case may never see again.
fn cases(cut: Cut, operation: Operation, found: &Traced) -> Vec<(Side, String)> {
    let mut all = Vec::new();
    if cut != Cut::Machine && operation.runs_on(Side::Host) {
        all.extend(found.host.iter().map(|name| (Side::Host, name.clone())));
    }
    if cut != Cut::Host {
        all.extend(found.boxes.iter().map(|name| (Side::Box, name.clone())));
    }
    all
}

fn signal(daemon: &Daemon, which: Signal) {
    if let Some(pid) = daemon.pid().and_then(|raw| i32::try_from(raw).ok()).and_then(Pid::from_raw) {
        let _ = rustix::process::kill_process(pid, which);
    }
}

/// Waits until `side`'s daemon is gone.
async fn stopped(rig: &mut Rig, side: Side) {
    match side {
        Side::Host => rig.host.stopped(LONG).await,
        Side::Box => rig.the_mut(BOX).stopped().await,
    }
}

/// Cuts the power of every holder of `state` still answering: each drops
/// what it hadn't synced and dies. Waits on each one's connection closing.
async fn cut_holders(state: &Path) -> Result<(), String> {
    let Ok(read) = std::fs::read_dir(state.join("holders")) else {
        return Ok(());
    };
    let mut sockets: Vec<PathBuf> = read
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sock"))
        .collect();
    sockets.sort();
    for socket in sockets {
        let Ok(mut connection) = tokio::net::UnixStream::connect(&socket).await else {
            continue;
        };
        let pid = std::fs::read_to_string(socket.with_extension("pid"))
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .and_then(Pid::from_raw);
        let Some(pid) = pid else {
            continue;
        };
        if rustix::process::kill_process(pid, Signal::USR2).is_err() {
            continue;
        }
        let gone = tokio::time::timeout(Duration::from_secs(30), async {
            let mut buffer = [0u8; 4096];
            loop {
                match connection.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await;
        if gone.is_err() {
            return Err(format!("the holder at {} never died at the power cut", socket.display()));
        }
    }
    Ok(())
}

/// The processes of the cut sides die: the daemon first (at its point, or
/// by SIGUSR2 for the other side of a double cut), then their holders.
async fn power_off(rig: &mut Rig, cut: Cut, origin: Side, name: &str) -> Result<(), String> {
    let origin_pid = match origin {
        Side::Host => rig.host.pid(),
        Side::Box => rig.the(BOX).daemon().pid(),
    };
    let fired = tokio::time::timeout(POINT_WAIT, stopped(rig, origin)).await;
    if fired.is_err() {
        // The wait gave up with the daemon still running; end it so it
        // doesn't outlive the test.
        if let Some(pid) = origin_pid.and_then(|raw| i32::try_from(raw).ok()).and_then(Pid::from_raw) {
            let _ = rustix::process::kill_process(pid, Signal::KILL);
        }
        return Err(format!(
            "the point {name} on the {origin:?} side never fired within {POINT_WAIT:?} although the trace reached it"
        ));
    }
    let mut cut_sides = vec![origin];
    if cut == Cut::Both {
        let other = if origin == Side::Host { Side::Box } else { Side::Host };
        match other {
            Side::Host => signal(&rig.host, Signal::USR2),
            Side::Box => signal(rig.the(BOX).daemon(), Signal::USR2),
        }
        stopped(rig, other).await;
        cut_sides.push(other);
    }
    for side in cut_sides {
        cut_holders(&state_of(rig, side)).await?;
    }
    Ok(())
}

/// Starts the cut sides again, the host first, and returns once the host
/// shows the box linked.
async fn restart(rig: &mut Rig, cut: Cut) {
    match cut {
        Cut::Host => rig.restart_host(&[]).await,
        Cut::Machine => rig.restart_box_env(BOX, &[]).await,
        Cut::Both => {
            // The box is off while the host starts: the host's dials wait in
            // `ssh` until the box is back, as a dial to a machine that is off
            // does, rather than reaching a machine with no daemon and waiting
            // out the link's keepalive (50 s) and its backoff. Both daemons
            // are down, so no old link is left to end.
            rig.the(BOX).hold_dials();
            rig.restart_host(&[]).await;
            rig.the_mut(BOX).relaunch(&[]).await;
            rig.link_back(BOX).await;
        }
    }
}

/// The `.json` journals left in `folders`, each with what it holds, so a
/// journal that never goes says which operation it is.
fn journals_left(folders: &[PathBuf]) -> Vec<(PathBuf, String)> {
    folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let held = std::fs::read_to_string(&path).unwrap_or_default();
            (path, held)
        })
        .collect()
}

/// How many `.json` journals `folder` holds; none when it doesn't exist.
fn journals_in(folder: &std::path::Path) -> usize {
    std::fs::read_dir(folder).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count()
    })
}

fn cut_sides(cut: Cut) -> Vec<Side> {
    match cut {
        Cut::Host => vec![Side::Host],
        Cut::Machine => vec![Side::Box],
        Cut::Both => vec![Side::Host, Side::Box],
    }
}

/// Waits until the restart is over: each restarted side wrote its report,
/// none of its operations is still pending, no session is mid-start or
/// mid-move, no transfer is still moving bytes or waiting for a machine,
/// and the draft comment isn't left queued with no word on it.
async fn settle(rig: &Rig, cut: Cut, old: &[(Side, usize)], base: &Base) -> Result<(), String> {
    until("each restarted side's restart report", async || {
        old.iter().all(|(side, count)| report_count(&state_of(rig, *side)) > *count)
    })
    .await?;
    until("the restart's operations settled", async || {
        cut_sides(cut).into_iter().all(|side| {
            newest_report(&state_of(rig, side))
                .is_none_or(|report| operations_of(&report).iter().all(|operation| operation.state != "pending"))
        })
    })
    .await?;
    let channel = rig.channel().await;
    // The host shows each of the box's sessions Offline from its link going
    // until the box's first report over the new link (the snapshot the box
    // sends once it has listed its sessions, D139): a link that is up isn't
    // yet a view that is current. The host's view is read once it heard of
    // every session the box's store holds and hasn't archived. (The box's
    // daemon runs without --admin, so its store is what the test reads.)
    let boxes = rig.the(BOX).store();
    until("the host heard the box's sessions over the new link", async || {
        let Ok(on_box) = boxes.sessions() else {
            return false;
        };
        let host = list_sessions(&channel).await;
        on_box
            .iter()
            .filter(|record| boxes.archive(&record.id).is_ok_and(|row| row.is_none()))
            .all(|record| {
                host.iter()
                    .filter(|shown| shown.id.as_ref().is_some_and(|id| id.value == record.id))
                    .filter(|shown| shown.archive.is_none())
                    .filter(|shown| shown.machine.as_ref().is_some_and(|machine| machine.value == BOX))
                    .all(|shown| !is_offline(shown))
            })
    })
    .await?;
    // A box that archived a session while the host's call to it was cut
    // frees its clones at restart (D172) and tells the host once the link
    // is back (D222): the host's view is read once it learned of it.
    until("the host learned of what the box archived", async || {
        list_sessions(&channel).await.iter().all(|session| {
            let id = session.id.as_ref().map(|id| id.value.as_str()).unwrap_or_default();
            session.archive.is_some() || !is_live(session) || !boxes.archive(id).is_ok_and(|row| row.is_some())
        })
    })
    .await?;
    // A session the box stopped reporting because it archived it shows
    // Offline, not live, until the host's follow-up asks the box (D222):
    // the follow-up's journal goes only once the host has its answer.
    let follow_ups = state_of(rig, Side::Host).join("carry").join("remote_archive");
    until("the host's remote archive follow-ups answered", async || {
        let Ok(entries) = std::fs::read_dir(&follow_ups) else {
            return true;
        };
        !entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".reconcile.json"))
    })
    .await?;
    // The host takes a fork's copy back, asks a start again and finishes or
    // undoes a move on the box's next link, outside any restart report: each
    // journal goes only once it has acted, so what a box cut left is read
    // after the move, fork and start journals are gone.
    let host_state = state_of(rig, Side::Host);
    let carry = host_state.join("carry");
    let journals = [carry.clone(), carry.join("forks"), carry.join("remote_archive").join("start")];
    until("the host's move, fork and start journals emptied", async || {
        journals.iter().all(|folder| journals_in(folder) == 0)
    })
    .await
    .map_err(|problem| format!("{problem}; left: {:?}", journals_left(&journals)))?;
    // A transfer the cut interrupted resumes from its saved offset once the
    // restart is over: what it leaves is read once it has stopped moving.
    until("the transfers settled", async || {
        let mut transfers = pb::transfer_service_client::TransferServiceClient::new(channel.clone());
        match transfers.list_transfers(pb::ListTransfersRequest::default()).await {
            Ok(listed) => listed.into_inner().transfers.iter().all(|transfer| {
                !matches!(
                    transfer.state(),
                    pb::TransferState::Running | pb::TransferState::Verifying | pb::TransferState::WaitingForMachine
                )
            }),
            Err(_) => false,
        }
    })
    .await?;
    until("the sessions settled", async || {
        let quiet = list_sessions(&channel).await.iter().all(|session| {
            !matches!(session.status(), pb::SessionStatus::Starting | pb::SessionStatus::Moving)
        });
        let comments = comments_of(&channel, &base.h).await;
        let unsettled = comments.iter().any(|comment| {
            comment.status() == pb::CommentStatus::Queued && !comment.delivery_unconfirmed
        });
        quiet && !unsettled
    })
    .await
}

// ---------------------------------------------------------------------------
// Checking.

/// How often the comment's text was typed to the agent, in the
/// conversation files of both sides.
fn typed_count(rig: &Rig, base: &Base) -> usize {
    [rig.host_conversation(&base.h_conversation), rig.the(BOX).conversation(&base.h_conversation)]
        .iter()
        .map(|file| std::fs::read_to_string(file).unwrap_or_default().matches(MARKER).count())
        .sum()
}

fn check_comment(rig: &Rig, base: &Base, after: &Inventory) -> Vec<String> {
    let mut problems = Vec::new();
    let typed = typed_count(rig, base);
    let key = format!("comment:{}", base.comment.value);
    let Some(now) = after.items.get(&key) else {
        problems.push("the draft comment is gone".to_owned());
        return problems;
    };
    if typed > 1 {
        problems.push(format!("the comment was typed {typed} times, not once"));
    }
    if now.contains("|Sent|") && typed != 1 {
        problems.push(format!("the comment shows Sent but was typed {typed} times"));
    }
    if now.contains("|Draft|") && typed != 0 {
        problems.push(format!("the comment is still a draft but was typed {typed} times"));
    }
    problems
}

/// Each base session's terminal still shows, in order and with no line
/// missing, what its client saw before the cut. A session that isn't
/// running where it was (moved, archived, named unrestored) is left out.
async fn check_terminals(rig: &Rig, base: &Base, operation: Operation, after: &Inventory, reports: &Reports) -> Vec<String> {
    let mut problems = Vec::new();
    let tokens = operation.tokens(base);
    for output in &base.outputs {
        let id = &output.session.value;
        if reports.unrestored.contains(id) || tokens.iter().any(|token| token == id) {
            continue;
        }
        let Some(session) = after
            .sessions
            .iter()
            .find(|session| session.id.as_ref() == Some(&output.session))
            .filter(|session| is_live(session))
        else {
            continue;
        };
        let _ = session;
        let open = pb::AttachOpen {
            session: Some(output.session.clone()),
            ..common::open(&output.tab, 0)
        };
        let mut attach = match Attach::try_start(&rig.host, open).await {
            Ok(attach) => attach,
            Err(status) => {
                problems.push(format!("session {id}: the terminal can't be attached: {:?}", status.code()));
                continue;
            }
        };
        let last = format!("got: {}", LINES[LINES.len() - 1]);
        // Reads until the last line shows, the stream fails, or it goes
        // quiet: what the holder kept is all there is to see.
        while !attach.shows(&last) {
            // Below Attach's own WAIT, which panics, so a quiet attach is
            // reported as a problem with the rest.
            match tokio::time::timeout(Duration::from_secs(10), attach.read()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => break,
            }
        }
        let screen = String::from_utf8_lossy(&attach.screen).into_owned();
        let mut at = 0;
        for line in LINES {
            let wanted = format!("got: {line}");
            match screen[at..].find(&wanted) {
                Some(found) => at += found + wanted.len(),
                None => {
                    problems.push(format!(
                        "session {id}: the terminal lost {wanted:?} (or shows it out of order); it shows {screen:?}"
                    ));
                    break;
                }
            }
        }
    }
    problems
}

fn check_items(
    rig: &Rig,
    operation: Operation,
    base: &Base,
    before: &Inventory,
    after: &Inventory,
    finished: bool,
    reports: &Reports,
) -> Vec<String> {
    let mut problems = Vec::new();
    let tokens = operation.tokens(base);
    let touched = |key: &str| tokens.iter().any(|token| key.contains(token.as_str()));
    for (key, was) in &before.items {
        let now = after.items.get(key);
        let same = match now {
            // A conversation only ever grows.
            Some(now) if key.starts_with("conv:") => now.starts_with(was.as_str()),
            Some(now) => now == was,
            None => false,
        };
        // A comment cut while it was sent shows its delivery unconfirmed
        // and offers a resend; check_comment guards against a second send.
        let unconfirmed = key.starts_with("comment:") && now.is_some_and(|now| now.contains("|Queued|true|"));
        if same || unconfirmed || reports.names(key, touched(key)) {
            continue;
        }
        if touched(key) {
            if !finished {
                problems.push(format!(
                    "{key} is neither as it was nor finished and no report names it: was {was:?}, now {now:?}"
                ));
            }
        } else {
            problems.push(format!("{key} changed with nothing touching it: was {was:?}, now {now:?}"));
        }
    }
    for (key, now) in &after.items {
        if before.items.contains_key(key) || reports.names(key, true) {
            continue;
        }
        if !operation.creates(key) {
            problems.push(format!("{key} appeared although the operation makes nothing like it: {now:?}"));
        } else if key.starts_with("session:") && !(now.starts_with("live|") || now.starts_with("archived|")) {
            problems.push(format!("{key} was made and left half way: {now:?}"));
        }
        if [".partial", ".rebuilding", ".humata-"].iter().any(|leftover| key.contains(leftover)) {
            problems.push(format!("{key} is left over from the operation"));
        }
    }
    // Every clone on disk belongs to a session; every running session's
    // clone is on disk.
    for key in after.items.keys().filter(|key| key.starts_with("clone:")) {
        let relative = key.splitn(3, ':').nth(2).unwrap_or_default();
        let suffix: Vec<&str> = relative.split('/').skip(1).collect();
        let suffix = format!("/{}", suffix.join("/"));
        let claimed = after
            .sessions
            .iter()
            .any(|session| session.repos.iter().any(|repo| repo.path.ends_with(&suffix)));
        if !claimed && !reports.names(key, true) {
            problems.push(format!("{key} is a clone no session claims"));
        }
    }
    for session in after.sessions.iter().filter(|session| is_live(session)) {
        let id = session.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        for repo in &session.repos {
            // The host's record of a box's session names the clone as the
            // host would lay it out; the clone is in the box's own channels
            // folder, under the same channel folder, session folder and repo name.
            let recorded = Path::new(&repo.path);
            let on_box = session.machine.as_ref().is_some_and(|machine| machine.value == BOX);
            let relative: PathBuf = recorded.components().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            let path = if on_box && recorded.is_absolute() {
                rig.the(BOX).channels().join(relative)
            } else {
                recorded.to_owned()
            };
            if path.is_absolute() && !path.exists() && !reports.names(&id, false) {
                problems.push(format!("session {id} runs but its clone {} is gone", repo.path));
            }
        }
    }
    problems
}

/// After a cut during a host variable being set, the secrets file of the
/// machine it is set on (the host, or the box for RemoteHostVariable) is the
/// old set or the new set in full (never torn, empty or missing), the file is
/// 0600 and its folder 0700, and so is any temporary file the cut left behind.
fn check_host_variables(rig: &Rig, operation: Operation) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;

    let side = match operation {
        Operation::HostVariable => Side::Host,
        Operation::RemoteHostVariable => Side::Box,
        _ => return Vec::new(),
    };
    let mut problems = Vec::new();
    let folder = state_of(rig, side).join("secrets");
    let mode = |path: &Path| std::fs::metadata(path).map(|meta| meta.permissions().mode() & 0o777).ok();
    if mode(&folder) != Some(0o700) {
        problems.push(format!("the secrets folder is {:?}, not 0o700", mode(&folder)));
    }
    let file = folder.join("host-variables.json");
    let old = BTreeMap::from([(HOST_VARIABLE_OLD.0.to_owned(), HOST_VARIABLE_OLD.1.to_owned())]);
    let mut new = old.clone();
    new.insert(HOST_VARIABLE_NEW.0.to_owned(), HOST_VARIABLE_NEW.1.to_owned());
    match std::fs::read(&file).map(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes)) {
        Ok(Ok(kept)) if kept == old || kept == new => {}
        Ok(Ok(kept)) => problems.push(format!("the host variables file holds neither set: {kept:?}")),
        Ok(Err(error)) => problems.push(format!("the host variables file is torn or empty: {error}")),
        Err(error) => problems.push(format!("the host variables file can't be read: {error}")),
    }
    if mode(&file) != Some(0o600) {
        problems.push(format!("the host variables file is {:?}, not 0o600", mode(&file)));
    }
    for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "new") && mode(&path) != Some(0o600) {
            problems.push(format!("the leftover {} is {:?}, not 0o600", path.display(), mode(&path)));
        }
    }
    problems
}

pub(crate) async fn run_case(cut: Cut, operation: Operation, origin: Side, name: &str) -> Vec<String> {
    eprintln!("power cut: {operation:?}, {cut:?}, {origin:?} at {name}");
    let scratch = common::folder();
    let faults = Faults {
        point: Some((origin, name.to_owned())),
        trace: false,
    };
    let mut phases = Phases::new(format!("{operation:?}/{cut:?}/{origin:?}@{name}"));
    let mut rig = start_rig(&faults, scratch.path(), operation).await;
    phases.done("rig");
    let base = setup(&rig, operation).await;
    let before = Inventory::take(&rig).await;
    phases.done("setup");
    let old: Vec<(Side, usize)> = cut_sides(cut)
        .into_iter()
        .map(|side| (side, report_count(&state_of(&rig, side))))
        .collect();

    std::fs::write(scratch.path().join("arm"), b"").expect("the arm file is written");
    let task = tokio::spawn(operation.run(Context {
        channel: rig.channel().await,
        base: base.clone(),
    }));
    let off = power_off(&mut rig, cut, origin, name).await;
    task.abort();
    phases.done("cut");
    let label = |problem: String| format!("{operation:?}/{cut:?}/{origin:?}@{name}: {problem}");
    if let Err(problem) = off {
        return vec![label(problem)];
    }

    restart(&mut rig, cut).await;
    phases.done("restart");
    let mut problems = Vec::new();
    if let Err(problem) = settle(&rig, cut, &old, &base).await {
        problems.push(problem);
    }
    phases.done("settle");
    let after = Inventory::take(&rig).await;
    let reports = Reports::read(&rig);
    let channel = rig.channel().await;
    let finished = operation.finished(&channel, &base).await;
    problems.extend(check_items(&rig, operation, &base, &before, &after, finished, &reports));
    problems.extend(check_comment(&rig, &base, &after));
    problems.extend(check_terminals(&rig, &base, operation, &after, &reports).await);
    problems.extend(check_host_variables(&rig, operation));
    phases.done("check");
    phases.say();
    problems.into_iter().map(label).collect()
}

/// How long each phase of a case took, written to the test's log (past
/// the harness's capture) so a slow run shows where its minutes went.
struct Phases {
    what: String,
    last: std::time::Instant,
    took: Vec<(&'static str, Duration)>,
}

impl Phases {
    fn new(what: String) -> Self {
        Self {
            what,
            last: std::time::Instant::now(),
            took: Vec::new(),
        }
    }

    fn done(&mut self, phase: &'static str) {
        let now = std::time::Instant::now();
        self.took.push((phase, now - self.last));
        self.last = now;
    }

    fn say(&self) {
        use std::io::Write as _;
        let total: Duration = self.took.iter().map(|(_, took)| *took).sum();
        let phases: Vec<String> =
            self.took.iter().map(|(phase, took)| format!("{phase} {:.1}s", took.as_secs_f64())).collect();
        let _ = writeln!(
            std::io::stderr(),
            "power cut timing: {} took {:.1}s: {}",
            self.what,
            total.as_secs_f64(),
            phases.join(", ")
        );
    }
}

/// Whether the test numbered `index` is this Bazel shard's. rules_rust
/// doesn't shard a test binary, so without this every shard runs every
/// test: the test runs in shard `index % TEST_TOTAL_SHARDS`, and the others
/// pass it at once. Outside Bazel's sharding every test runs.
fn in_this_shard(index: usize) -> bool {
    if let Some(status) = std::env::var_os("TEST_SHARD_STATUS_FILE") {
        std::fs::write(status, b"").expect("the shard status file is written");
    }
    let read = |name: &str| std::env::var(name).ok().and_then(|value| value.parse::<usize>().ok());
    match (read("TEST_TOTAL_SHARDS"), read("TEST_SHARD_INDEX")) {
        (Some(total), Some(shard)) if total > 0 => index % total == shard,
        _ => true,
    }
}

/// The rigs a test may start: [`AT_ONCE`] of them, or what
/// `HUMATA2_POWER_CUT_AT_ONCE` says, shared by every test of the binary.
/// Each test asks for one rig at a time and the oldest ask is served
/// first, so the operations take turns and a long one is not left last.
fn rigs() -> std::sync::Arc<tokio::sync::Semaphore> {
    static RIGS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    RIGS.get_or_init(|| {
        let count = std::env::var("HUMATA2_POWER_CUT_AT_ONCE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(AT_ONCE);
        std::sync::Arc::new(tokio::sync::Semaphore::new(count))
    })
    .clone()
}

/// One test: `operation` traced once, then cut at each of its points as
/// `cut` says, each case on a rig from [`rigs`], every case's problems
/// collected and reported together in the order of the trace.
pub async fn run_cases(cut: Cut, operation: Operation) {
    if !in_this_shard(operation as usize) {
        return;
    }
    let rigs = rigs();
    let mut phases = Phases::new(format!("trace of {operation:?}"));
    let found = {
        let _rig = rigs.acquire().await.expect("the rigs are never closed");
        trace(operation).await
    };
    phases.done("trace");
    phases.say();
    let cases = cases(cut, operation, &found);
    let count = cases.len();
    // Each case runs on a thread of its own, so a case's blocking waits
    // hold up no other case.
    let mut running = tokio::task::JoinSet::new();
    let mut names = BTreeMap::new();
    let handle = tokio::runtime::Handle::current();
    for (index, (origin, name)) in cases.into_iter().enumerate() {
        let rig = rigs.clone().acquire_owned().await.expect("the rigs are never closed");
        let label = format!("{operation:?}/{cut:?}/{origin:?}@{name}");
        let handle = handle.clone();
        let task = running.spawn_blocking(move || {
            let problems = handle.block_on(run_case(cut, operation, origin, &name));
            drop(rig);
            (index, problems)
        });
        names.insert(task.id(), (index, label));
    }
    let mut results: Vec<(usize, Vec<String>)> = Vec::new();
    while let Some(joined) = running.join_next_with_id().await {
        match joined {
            Ok((_, case)) => results.push(case),
            Err(error) => {
                let (index, label) = names.get(&error.id()).cloned().unwrap_or_default();
                results.push((index, vec![format!("{label}: the case itself failed: {error}")]));
            }
        }
    }
    phases.done("cases");
    phases.say();
    results.sort_by_key(|(index, _)| *index);
    let problems: Vec<String> = results.into_iter().flat_map(|(_, problems)| problems).collect();
    assert!(
        problems.is_empty(),
        "{} problem(s) after {count} power cuts during {operation:?} ({cut:?}):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
