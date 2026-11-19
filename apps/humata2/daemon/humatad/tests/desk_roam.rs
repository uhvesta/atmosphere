//! MV-17, D316, CD-1, flow 01 step 6: a Desk session, plain or a channel's
//! own, starts on any linked machine and moves to any linked machine like
//! any other session, whatever machines its channel lists. It works in
//! `<root>/desk/<name>` (a channel's in `<root>/<channel>/desk/<name>`) on
//! the machine it runs on, keeps its kind, channel and target, and the
//! machine it left keeps no folder, conversation or agent of it. A host
//! killed at any point of a Desk session's move finishes or undoes the move
//! at restart: one agent is live, its files and conversation are whole, and
//! neither machine keeps a half folder.
//!
//! Every wait is on the host's feeds (the rig's `wait_session`, `ended`);
//! `LONG` is only a safety net.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use common::{git, holder_answers, stored_tabs};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, TabKind, TabRecord};
use remote_rig::{LONG, Rig, is_live};
use tonic::Status;

const REPO: (&str, &str) = ("acme", "api");

/// The undo window an archive gets: none, so an archived target is gone at once.
const NO_UNDO: (&str, &str) = ("HUMATA2_ARCHIVE_UNDO_MS", "0");

/// A file the session's own folder holds before it moves.
const NOTES: &str = "# notes\n";

/// One of the rig's machines.
#[derive(Clone, Copy, Debug)]
enum At {
    Host,
    Box(&'static str),
}

impl At {
    async fn id(self, rig: &Rig) -> pb::MachineId {
        match self {
            At::Host => rig.host_id().await,
            At::Box(name) => Rig::box_id(name),
        }
    }

    /// Where the machine keeps its channels and its Desk.
    fn root(self, rig: &Rig) -> PathBuf {
        match self {
            At::Host => rig.world.channels.clone(),
            At::Box(name) => rig.the(name).channels(),
        }
    }

    fn store(self, rig: &Rig) -> Store {
        match self {
            At::Host => Store::open(rig.host.state.join("store")).unwrap(),
            At::Box(name) => rig.the(name).store(),
        }
    }

    fn tabs(self, rig: &Rig) -> Vec<TabRecord> {
        match self {
            At::Host => stored_tabs(&rig.host.state),
            At::Box(name) => rig.the(name).tabs(),
        }
    }
}

const MACHINES: [At; 3] = [At::Host, At::Box("box-1"), At::Box("box-2")];

/// Which Desk a session is in: the plain one, or a channel's own.
#[derive(Clone, Debug)]
enum Desk {
    Plain,
    Channel(pb::Channel),
}

impl Desk {
    /// Where this Desk's sessions work on `at`.
    fn root(&self, rig: &Rig, at: At) -> PathBuf {
        match self {
            Desk::Plain => at.root(rig).join("desk"),
            Desk::Channel(channel) => at.root(rig).join(&channel.folder).join("desk"),
        }
    }

    /// Lists this Desk's open sessions through the host.
    fn listing(&self) -> pb::ListDeskSessionsRequest {
        match self {
            Desk::Plain => pb::ListDeskSessionsRequest::default(),
            Desk::Channel(channel) => pb::ListDeskSessionsRequest {
                channel: channel.id.clone(),
                target: None,
            },
        }
    }
}

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<tonic::transport::Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey { value: value.to_owned() })
}

/// A plain Desk session titled `title`, on `on` (unset: this Mac).
fn plain(title: &str, on: Option<&pb::MachineId>) -> pb::StartDeskSessionRequest {
    pb::StartDeskSessionRequest {
        idempotency_key: key(&format!("desk-{title}")),
        title: title.to_owned(),
        machine: on.cloned(),
        access: Some(pb::DeskAccess {
            boards: true,
            channels_and_machines: false,
        }),
        ..Default::default()
    }
}

/// A Desk session of `channel` looking after `target`, on `on` (unset:
/// this Mac).
fn of_channel(
    channel: &pb::Channel,
    target: &pb::SessionId,
    title: &str,
    on: Option<&pb::MachineId>,
) -> pb::StartDeskSessionRequest {
    pb::StartDeskSessionRequest {
        idempotency_key: key(&format!("channel-desk-{title}")),
        title: title.to_owned(),
        machine: on.cloned(),
        channel: channel.id.clone(),
        target: Some(target.clone()),
        ..Default::default()
    }
}

/// Starts `request` through the host, waits for its start to succeed and
/// its agent to be up; returns the session and the folder it works in.
async fn start(rig: &Rig, request: pb::StartDeskSessionRequest) -> (pb::SessionId, PathBuf) {
    let started = desks(rig)
        .await
        .start_desk_session(request)
        .await
        .expect("the Desk session starts")
        .into_inner();
    if let Some(operation) = &started.operation {
        let ended = rig.ended(operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the Desk start failed: {:?}", ended.error);
    }
    let id = started.session.expect("a start names its session");
    rig.wait_session(&id, "live", is_live).await;
    let folder = started
        .desk
        .and_then(|desk| desk.folder)
        .expect("a start says where the session works")
        .path;
    (id, PathBuf::from(folder))
}

/// The refusal a start ended in, whether the call failed or its operation
/// did.
async fn refusal_of(rig: &Rig, started: Result<pb::StartDeskSessionResponse, Status>) -> pb::Error {
    match started {
        Err(status) => common::error_of(&status),
        Ok(response) => {
            let operation = response.operation.expect("a start that wasn't refused has an operation");
            let ended = rig.ended(&operation).await;
            assert_eq!(ended.state, pb::OperationState::Failed as i32, "the start went through: {ended:?}");
            ended.error.expect("a failed operation says why")
        }
    }
}

/// The open sessions `desk` lists on the host.
async fn listed(rig: &Rig, desk: &Desk) -> Vec<pb::DeskSession> {
    desks(rig)
        .await
        .list_desk_sessions(desk.listing())
        .await
        .expect("the host lists Desk sessions")
        .into_inner()
        .sessions
}

async fn find(rig: &Rig, desk: &Desk, id: &pb::SessionId) -> Option<pb::DeskSession> {
    listed(rig, desk).await.into_iter().find(|shown| shown.session.as_ref() == Some(id))
}

/// The names in `dir`, sorted; none when it isn't there.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|found| {
            found
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn name_of(folder: &Path) -> String {
    folder.file_name().expect("a Desk folder has a name").to_string_lossy().into_owned()
}

/// What a move left staged on `at`, wherever a Desk session's move would
/// stage it: beside the machine's channels, in the Desk itself, and in the
/// channel's folder.
fn staged(rig: &Rig, at: At, desk: &Desk) -> Vec<PathBuf> {
    let mut places = vec![at.root(rig).join(".carry"), desk.root(rig, at).join(".carry")];
    if let Desk::Channel(channel) = desk {
        places.push(at.root(rig).join(&channel.folder).join(".carry"));
    }
    places
        .iter()
        .flat_map(|place| entries(place).into_iter().map(|name| place.join(name)))
        .collect()
}

/// How many agents of `id` are live on all the rig's machines together: a
/// holder answering behind an agent tab of the session, counted once per
/// holder.
fn live_agents(rig: &Rig, id: &pb::SessionId) -> usize {
    let mut holders = HashSet::new();
    for at in MACHINES {
        for tab in at.tabs(rig) {
            if tab.session.as_deref() == Some(id.value.as_str()) && tab.kind == TabKind::Agent && holder_answers(&tab) {
                holders.insert(tab.holder_socket);
            }
        }
    }
    holders.len()
}

/// The pid of the holder of `id`'s agent on `at`.
fn agent_holder(rig: &Rig, at: At, id: &pb::SessionId) -> u32 {
    at.tabs(rig)
        .into_iter()
        .find(|tab| tab.session.as_deref() == Some(id.value.as_str()) && tab.kind == TabKind::Agent)
        .unwrap_or_else(|| panic!("{at:?} has no agent tab for {}", id.value))
        .holder_pid
}

/// What `id`'s conversation on `at` holds.
fn talk_on(rig: &Rig, at: At, id: &pb::SessionId) -> Vec<u8> {
    let agent = at
        .store(rig)
        .agent(&id.value)
        .unwrap()
        .unwrap_or_else(|| panic!("{at:?} has no agent for {}", id.value));
    std::fs::read(&agent.launch.conversation_file).unwrap()
}

/// Says `line` to the session's agent and waits for its answer, so the
/// conversation file holds it.
async fn say(rig: &Rig, id: &pb::SessionId, line: &str) {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    let mut attach = common::Attach::start(&rig.host, open).await;
    attach.wait_for("agent up").await;
    attach.type_text(&format!("{line}\n")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

fn request(id: &pb::SessionId, to: &pb::MachineId, key_text: &str) -> pb::MoveSessionRequest {
    pb::MoveSessionRequest {
        idempotency_key: key(key_text),
        id: Some(id.clone()),
        to: Some(to.clone()),
        ..Default::default()
    }
}

/// Moves `id` to `to` and waits for the move to succeed and the session to
/// run there.
async fn move_to(rig: &Rig, id: &pb::SessionId, to: &pb::MachineId) {
    let started = sessions(rig)
        .await
        .move_session(request(id, to, &format!("move-{}-{}", id.value, to.value)))
        .await
        .expect("the move starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
    rig.wait_session(id, "live where it moved", |s| s.machine.as_ref() == Some(to) && is_live(s) && not_moving(s))
        .await;
}

fn not_moving(session: &pb::Session) -> bool {
    session.status != pb::SessionStatus::Moving as i32
}

/// Starts a move of `id` to `to` on a host that stops itself at `at`, then
/// starts the host again.
async fn move_killed_at(rig: &mut Rig, id: &pb::SessionId, to: &pb::MachineId, at: &str) {
    rig.restart_host(&[("HUMATA2_STOP_AT", at)]).await;
    let _ = sessions(rig).await.move_session(request(id, to, &format!("move-killed-{at}"))).await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
}

/// A Desk session as it was before it moved.
struct Before {
    id: pb::SessionId,
    desk: Desk,
    /// Its folder's name, the same on every machine.
    name: String,
    /// What its conversation held.
    talk: Vec<u8>,
}

/// The session is on `home` alone: its folder there holds its files and
/// nothing else is in that Desk, its agent works in that folder with the
/// whole conversation, one agent is live, and `away` keeps no folder (whole
/// or half), agent, record or staged files.
fn settled(rig: &Rig, before: &Before, home: At, away: At, at: &str) {
    let Before { id, desk, name, talk } = before;
    let folder = desk.root(rig, home).join(name);
    assert_eq!(entries(&desk.root(rig, home)), vec![name.clone()], "{at}: {home:?}'s Desk holds the session alone");
    assert_eq!(std::fs::read_to_string(folder.join("NOTES.md")).unwrap(), NOTES, "{at}: the folder's files are whole");
    let agent = home
        .store(rig)
        .agent(&id.value)
        .unwrap()
        .unwrap_or_else(|| panic!("{at}: {home:?} has no agent for the session"));
    assert_eq!(agent.launch.folder, folder, "{at}: the agent works in the session's folder on {home:?}");
    assert!(std::fs::read(&agent.launch.conversation_file).unwrap().starts_with(talk), "{at}: the conversation is whole");
    assert_eq!(live_agents(rig, id), 1, "{at}: exactly one agent is live");

    assert!(entries(&desk.root(rig, away)).is_empty(), "{at}: {away:?} keeps a folder of the session, whole or half");
    let left = away.store(rig);
    assert!(left.agent(&id.value).unwrap().is_none(), "{at}: {away:?} keeps an agent of the session");
    if let At::Box(_) = away {
        assert!(left.session(&id.value).unwrap().is_none(), "{at}: {away:?} keeps a record of the session");
        assert!(left.channel_desk(&id.value).unwrap().is_none(), "{at}: {away:?} keeps the session's channel Desk row");
    }
    for place in [home, away] {
        assert!(staged(rig, place, desk).is_empty(), "{at}: {place:?} keeps files staged: {:?}", staged(rig, place, desk));
    }
}

/// MV-17, flow 01 step 6: a plain Desk session started on box-2 runs there,
/// in box-2's `~/Humata/desk/<name>`; this Mac and box-1 make nothing, and
/// the Desk lists it.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_started_on_box_2_works_in_box_2_s_desk_and_makes_nothing_here() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");

    let (id, folder) = start(&rig, plain("Scratch notes", Some(&two))).await;

    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    let name = name_of(&folder);
    assert_eq!(folder, Desk::Plain.root(&rig, At::Box("box-2")).join(&name), "it works in box-2's Desk");
    assert!(folder.is_dir(), "its folder is on box-2");
    let record = At::Box("box-2").store(&rig).session(&id.value).unwrap().expect("box-2 keeps the session");
    assert_eq!(record.folder, folder);
    say(&rig, &id, "jot this down").await;
    assert!(
        String::from_utf8_lossy(&talk_on(&rig, At::Box("box-2"), &id)).contains("jot this down"),
        "the agent on box-2 keeps the conversation"
    );
    assert_eq!(live_agents(&rig, &id), 1, "one agent runs it");
    for at in [At::Host, At::Box("box-1")] {
        assert!(entries(&Desk::Plain.root(&rig, at)).is_empty(), "{at:?} made a Desk folder");
        assert!(at.store(&rig).agent(&id.value).unwrap().is_none(), "{at:?} keeps an agent of it");
    }
    assert!(At::Box("box-1").store(&rig).session(&id.value).unwrap().is_none(), "box-1 keeps a record of it");
    let shown = find(&rig, &Desk::Plain, &id).await.expect("the Desk lists the session on box-2");
    assert_eq!(shown.folder.map(|folder| PathBuf::from(folder.path)), Some(folder));
}

/// MV-17, D316: a channel's Desk session can start on any linked machine,
/// one the channel doesn't list included. It runs in that machine's
/// `~/Humata/<channel>/desk/<name>`, looking after its target, and nothing
/// is made on this Mac.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_desk_session_started_on_a_machine_outside_its_channel_works_in_the_channel_s_desk_there() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());

    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch the ledger", Some(&two))).await;

    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    assert_eq!(folder, desk.root(&rig, At::Box("box-2")).join(name_of(&folder)), "it works in Payments' Desk on box-2");
    assert!(folder.is_dir(), "its folder is on box-2");
    assert!(entries(&desk.root(&rig, At::Host)).is_empty(), "this Mac made a folder for it");
    say(&rig, &id, "watch the ledger").await;
    assert_eq!(live_agents(&rig, &id), 1, "one agent runs it");
    let shown = find(&rig, &desk, &id).await.expect("the channel lists its Desk session on box-2");
    let of = shown.channel_desk.expect("it is a channel's Desk session");
    assert_eq!(of.channel, channel.id);
    assert_eq!(of.target, Some(target));
}

/// MV-17: a Desk session for a machine the host doesn't know, or one whose
/// link is down, is refused with a typed error, plain or a channel's, and
/// nothing is made on any machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_on_an_unknown_or_offline_machine_is_refused_and_makes_nothing() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let unknown = pb::MachineId { value: "nowhere".to_owned() };

    for request in [plain("Lost notes", Some(&unknown)), of_channel(&channel, &target, "Lost watch", Some(&unknown))] {
        let started = desks(&rig).await.start_desk_session(request).await.map(tonic::Response::into_inner);
        let refused = refusal_of(&rig, started).await;
        // D331: a machine this host has never linked is an unknown value of
        // `machine`, as it is for a fork.
        match refused.kind {
            Some(pb::error::Kind::InvalidField(field)) => {
                assert_eq!(field.field, "machine");
                assert_eq!(field.problem(), pb::invalid_field::Problem::UnknownValue);
            }
            other => panic!("an unknown machine should be an unknown value of machine: {other:?}"),
        }
    }

    rig.link_lost("box-2").await;
    for request in [plain("Offline notes", Some(&two)), of_channel(&channel, &target, "Offline watch", Some(&two))] {
        let started = desks(&rig).await.start_desk_session(request).await.map(tonic::Response::into_inner);
        let refused = refusal_of(&rig, started).await;
        assert!(matches!(refused.kind, Some(pb::error::Kind::MachineOffline(_))), "{refused:?}");
    }

    for at in MACHINES {
        for desk in [Desk::Plain, Desk::Channel(channel.clone())] {
            assert!(entries(&desk.root(&rig, at)).is_empty(), "{at:?} made a folder in {desk:?}");
        }
    }
    assert!(listed(&rig, &Desk::Plain).await.is_empty(), "the Desk lists a refused session");
    assert!(listed(&rig, &Desk::Channel(channel)).await.is_empty(), "the channel lists a refused session");
}

/// MV-17: a plain Desk session moved from this Mac to box-2 lands in box-2's
/// `~/Humata/desk/<name>` with its files and its conversation, and is still
/// a Desk session with a scratch folder. This Mac keeps no folder,
/// conversation or agent of it; box-2's records name only the new place.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_moved_to_box_2_lands_in_box_2_s_desk_and_stays_a_desk_session() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");
    let (id, folder) = start(&rig, plain("Scratch notes", None)).await;
    let name = name_of(&folder);
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    say(&rig, &id, "remember the notes").await;
    let old_talk = At::Host.store(&rig).agent(&id.value).unwrap().expect("this Mac runs its agent").launch.conversation_file;
    let talk = std::fs::read(&old_talk).unwrap();

    move_to(&rig, &id, &two).await;

    let moved = Desk::Plain.root(&rig, At::Box("box-2")).join(&name);
    assert_eq!(std::fs::read_to_string(moved.join("NOTES.md")).unwrap(), NOTES);
    let store = At::Box("box-2").store(&rig);
    assert_eq!(store.session(&id.value).unwrap().expect("box-2 keeps the session").folder, moved);
    let agent = store.agent(&id.value).unwrap().expect("box-2 runs its agent");
    assert_eq!(agent.launch.folder, moved);
    assert!(std::fs::read(&agent.launch.conversation_file).unwrap().starts_with(&talk), "the conversation came along");
    let shown = find(&rig, &Desk::Plain, &id).await.expect("the Desk still lists it");
    assert!(shown.channel_desk.is_none(), "it is still a plain Desk session");
    let place = shown.folder.expect("the Desk says where it works");
    assert_eq!(PathBuf::from(&place.path), moved);
    assert!(matches!(place.kind, Some(pb::desk_folder::Kind::Scratch(_))), "its folder is still a scratch folder: {place:?}");

    assert!(!folder.exists(), "this Mac keeps the old folder");
    assert!(!old_talk.exists(), "this Mac keeps the old conversation");
    assert!(At::Host.store(&rig).agent(&id.value).unwrap().is_none(), "this Mac keeps an agent of it");
    assert_eq!(live_agents(&rig, &id), 1);
    say(&rig, &id, "still here").await;
}

/// MV-17, D316: a channel's Desk session moves to any linked machine, ones
/// its channel doesn't list included. Each time it lands in that machine's
/// `~/Humata/<channel>/desk/<name>`, with the same channel and target, and
/// the machine it left keeps no folder, record or agent of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_desk_session_moves_to_machines_outside_its_channel_and_keeps_its_channel_and_target() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, one, two) = (rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch the ledger", None)).await;
    let name = name_of(&folder);
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    say(&rig, &id, "watch the ledger").await;
    let before = Before { id: id.clone(), desk, name, talk: talk_on(&rig, At::Host, &id) };

    move_to(&rig, &id, &two).await;
    settled(&rig, &before, At::Box("box-2"), At::Host, "this Mac to box-2");

    move_to(&rig, &id, &one).await;
    settled(&rig, &before, At::Box("box-1"), At::Box("box-2"), "box-2 to box-1");

    let by_target = desks(&rig)
        .await
        .list_desk_sessions(pb::ListDeskSessionsRequest {
            channel: None,
            target: Some(target.clone()),
        })
        .await
        .expect("the host lists Desk sessions by target")
        .into_inner()
        .sessions;
    let shown = by_target
        .into_iter()
        .find(|shown| shown.session.as_ref() == Some(&id))
        .expect("its target still lists it");
    let of = shown.channel_desk.expect("it is still a channel's Desk session");
    assert_eq!(of.channel, channel.id);
    assert_eq!(of.target, Some(target));
    say(&rig, &id, "still watching").await;
}

/// MV-14, MV-17: a host killed before a Desk session's move switches undoes
/// the move at restart, at each such point in turn: the session runs on
/// this Mac with the agent that never stopped, its files and conversation
/// whole, and box-2 keeps nothing of it, not even a half-copied folder.
/// Moving it afterwards works.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_s_move_cut_short_before_the_switch_is_undone_at_restart() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let (id, folder) = start(&rig, plain("Scratch notes", None)).await;
    let name = name_of(&folder);
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    say(&rig, &id, "remember the notes").await;
    let before = Before { id: id.clone(), desk: Desk::Plain, name, talk: talk_on(&rig, At::Host, &id) };
    let holder = agent_holder(&rig, At::Host, &id);

    for at in ["move-started", "move-packed", "move-copying", "move-copied", "move-unpacked"] {
        move_killed_at(&mut rig, &id, &two, at).await;

        rig.wait_session(&id, "back on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s) && not_moving(s))
            .await;
        settled(&rig, &before, At::Host, At::Box("box-2"), at);
        assert_eq!(agent_holder(&rig, At::Host, &id), holder, "{at}: the same agent runs on this Mac");
    }

    move_to(&rig, &id, &two).await;
    settled(&rig, &before, At::Box("box-2"), At::Host, "the move after");
}

/// MV-14, MV-17: a host killed once a channel Desk session's agent runs on
/// the new machine finishes the move at restart, at each such point in
/// turn, to box-2 and back: the session runs where it moved with one agent,
/// its files and conversation whole, and the machine it left keeps nothing
/// of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_desk_session_s_move_cut_short_after_the_agent_moved_is_finished_at_restart() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch the ledger", None)).await;
    let name = name_of(&folder);
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    say(&rig, &id, "watch the ledger").await;
    let before = Before { id: id.clone(), desk, name, talk: talk_on(&rig, At::Host, &id) };

    let (mut here, mut there) = (At::Host, At::Box("box-2"));
    for at in ["move-resumed", "move-switched", "move-released"] {
        let to = there.id(&rig).await;
        move_killed_at(&mut rig, &id, &to, at).await;

        rig.wait_session(&id, "where it moved", |s| s.machine.as_ref() == Some(&to) && is_live(s) && not_moving(s)).await;
        settled(&rig, &before, there, here, at);
        std::mem::swap(&mut here, &mut there);
    }
}

/// What the Bin on `at` holds, by item.
fn trashed(rig: &Rig, at: At) -> Vec<String> {
    entries(&at.root(rig).join(".trash"))
}

/// What this Mac holds of the Desk: its folders and its Bin.
fn host_holds(rig: &Rig, desk: &Desk) -> (Vec<String>, Vec<String>) {
    (entries(&desk.root(rig, At::Host)), trashed(rig, At::Host))
}

fn close_request(id: &pb::SessionId, tag: &str) -> pb::CloseDeskSessionRequest {
    pb::CloseDeskSessionRequest {
        idempotency_key: key(&format!("close-{tag}")),
        session: Some(id.clone()),
        ..Default::default()
    }
}

/// MV-17: a Desk session that works on box-2 is closed and cleaned up on
/// box-2 through the host. Closing stops its agent there and keeps its
/// folder; Clean up sends the folder to box-2's Bin and the host's row goes
/// with it. This Mac's folders and Bin are not touched.
#[tokio::test(flavor = "multi_thread")]
async fn closing_and_cleaning_up_a_desk_session_on_box_2_acts_on_box_2_and_leaves_this_mac_alone() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");
    let (id, folder) = start(&rig, plain("Remote scratch", Some(&two))).await;
    assert!(folder.starts_with(Desk::Plain.root(&rig, At::Box("box-2"))), "it works in box-2's Desk");
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    let host_before = host_holds(&rig, &Desk::Plain);
    let bin_before = trashed(&rig, At::Box("box-2"));

    let closed = desks(&rig)
        .await
        .close_desk_session(close_request(&id, "remote"))
        .await
        .expect("the host closes it on box-2")
        .into_inner();
    assert!(closed.closed_at.is_some(), "the closed session says so");
    assert_eq!(live_agents(&rig, &id), 0, "box-2's agent stopped");
    assert_eq!(std::fs::read_to_string(folder.join("NOTES.md")).unwrap(), NOTES, "closing keeps the folder");
    let row = At::Box("box-2").store(&rig).desk(&id.value).unwrap().expect("box-2 keeps its row");
    assert!(row.closed_unix_ms.is_some(), "box-2 holds it closed");

    let plan = desks(&rig)
        .await
        .plan_desk_clean_up(pb::PlanDeskCleanUpRequest::default())
        .await
        .expect("the host plans the clean up")
        .into_inner();
    assert!(plan.rows.iter().any(|row| row.session.as_ref() == Some(&id)), "the plan lists it");
    let cleaned = desks(&rig)
        .await
        .clean_up_desk(pb::CleanUpDeskRequest {
            idempotency_key: key("clean-remote"),
            plan: plan.token,
            delete: vec![id.clone()],
        })
        .await
        .expect("the host cleans it up on box-2")
        .into_inner();
    let Some(pb::clean_up_desk_response::Result::Deleted(done)) = cleaned.result else {
        panic!("the clean up should delete it: {cleaned:?}");
    };
    assert_eq!(done.sessions, 1);

    assert!(!folder.exists(), "box-2 keeps the folder");
    assert_ne!(trashed(&rig, At::Box("box-2")), bin_before, "box-2's Bin took the folder");
    assert!(find(&rig, &Desk::Plain, &id).await.is_none(), "the Desk no longer lists it");
    assert_eq!(host_holds(&rig, &Desk::Plain), host_before, "this Mac's folders and Bin are as they were");
}

/// MV-17, CD-4: closing a channel's Desk session that works on box-2 sends
/// its folder to box-2's Bin, there, and nothing on this Mac moves.
#[tokio::test(flavor = "multi_thread")]
async fn closing_a_channel_desk_session_on_box_2_sends_its_folder_to_box_2_s_bin() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch remotely", Some(&two))).await;
    assert!(folder.starts_with(desk.root(&rig, At::Box("box-2"))), "it works in box-2's channel Desk");
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    let host_before = host_holds(&rig, &desk);
    let bin_before = trashed(&rig, At::Box("box-2"));

    desks(&rig)
        .await
        .close_desk_session(close_request(&id, "remote-channel"))
        .await
        .expect("the host closes it on box-2");

    assert!(!folder.exists(), "box-2 keeps the folder");
    assert_ne!(trashed(&rig, At::Box("box-2")), bin_before, "box-2's Bin took the folder");
    assert_eq!(live_agents(&rig, &id), 0, "box-2's agent stopped");
    assert_eq!(host_holds(&rig, &desk), host_before, "this Mac's folders and Bin are as they were");
}

/// MV-17: closing a Desk session on a machine whose link is down is refused
/// as machine-offline at once and queues nothing: the session stays open on
/// both machines, and closes when asked again once the machine is back.
#[tokio::test(flavor = "multi_thread")]
async fn closing_a_desk_session_on_an_offline_machine_is_refused_and_changes_nothing() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");
    let (id, folder) = start(&rig, plain("Offline close", Some(&two))).await;

    rig.link_lost("box-2").await;
    let refused = desks(&rig).await.close_desk_session(close_request(&id, "offline")).await.unwrap_err();
    assert!(
        matches!(common::error_of(&refused).kind, Some(pb::error::Kind::MachineOffline(_))),
        "{refused:?}"
    );
    let shown = find(&rig, &Desk::Plain, &id).await.expect("the Desk still lists it");
    assert!(shown.closed_at.is_none(), "the host shows it open");
    assert!(folder.exists(), "box-2 keeps its folder");

    rig.link_back("box-2").await;
    let closed = desks(&rig).await.close_desk_session(close_request(&id, "back")).await.expect("it closes once box-2 is back").into_inner();
    assert!(closed.closed_at.is_some());
}

/// MV-17: the Markdown files a remote Desk session shows come from the
/// machine it works on, for a plain Desk session and a channel's own. A
/// machine that is away shows none, and the list still answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_remote_desk_session_lists_the_documents_its_machine_holds_and_none_while_it_is_away() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (plain_id, plain_folder) = start(&rig, plain("Remote notes", Some(&two))).await;
    let (own_id, own_folder) = start(&rig, of_channel(&channel, &target, "Remote watch", Some(&two))).await;
    std::fs::write(plain_folder.join("PLAIN.md"), NOTES).unwrap();
    std::fs::write(own_folder.join("OWN.md"), NOTES).unwrap();

    let shown = find(&rig, &Desk::Plain, &plain_id).await.expect("the Desk lists the session on box-2");
    assert!(shown.documents.contains(&"PLAIN.md".to_owned()), "{:?}", shown.documents);
    let shown = find(&rig, &desk, &own_id).await.expect("the channel lists its Desk session on box-2");
    assert!(shown.documents.contains(&"OWN.md".to_owned()), "{:?}", shown.documents);

    rig.link_lost("box-2").await;
    let shown = find(&rig, &Desk::Plain, &plain_id).await.expect("the list still shows the session while box-2 is away");
    assert!(shown.documents.is_empty(), "nothing is shown that the host can't see: {:?}", shown.documents);
}

/// CH-24, MV-17: deleting a channel closes its Desk sessions on the machines
/// they work on: box-2 stops the agent and sends the folder to its own Bin,
/// and this Mac's disk isn't touched.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_channel_closes_its_desk_session_on_box_2_into_box_2_s_bin() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Remote watch", Some(&two))).await;
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    let bin_before = trashed(&rig, At::Box("box-2"));
    let mut channels = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await);

    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: channel.id.clone() })
        .await
        .expect("the host plans the delete")
        .into_inner();
    let deleted = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
            end_live: Some(true),
        })
        .await
        .expect("the host deletes the channel")
        .into_inner();
    let ended = rig.ended(deleted.operation.as_ref().expect("a delete has an operation")).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert!(!folder.exists(), "box-2 keeps no folder of it");
    assert_ne!(trashed(&rig, At::Box("box-2")), bin_before, "box-2's Bin took the folder");
    assert_eq!(live_agents(&rig, &id), 0, "box-2's agent stopped");
    assert!(entries(&desk.root(&rig, At::Host)).is_empty(), "this Mac made no folder for it");
}

/// CD-4, MV-17: restoring a closed channel Desk session from box-2's items in
/// the host's Bin restores it on box-2: the folder comes back there, its
/// agent resumes there, and the host lists it open again.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_channel_desk_session_from_the_host_s_bin_view_restores_it_on_box_2() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), two.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Remote watch", Some(&two))).await;
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    desks(&rig).await.close_desk_session(close_request(&id, "restorable")).await.expect("the host closes it on box-2");
    assert!(!folder.exists(), "box-2's Bin holds the folder");
    assert!(find(&rig, &desk, &id).await.is_none(), "a closed session is in the Bin, not listed");
    let mut bin = pb::trash_service_client::TrashServiceClient::new(rig.channel().await);
    let items = bin
        .list_trash(pb::ListTrashRequest { machine: Some(two.clone()), page: None })
        .await
        .expect("the host lists box-2's Bin")
        .into_inner()
        .items;
    assert_eq!(items.len(), 1, "box-2's Bin holds the one folder: {items:?}");

    bin.restore_from_trash(pb::RestoreFromTrashRequest {
        idempotency_key: key("restore-remote"),
        id: items[0].id.clone(),
    })
    .await
    .expect("the host restores it on box-2");

    assert!(folder.is_dir(), "box-2 has its folder back");
    // The resume runs after the answer: Starting is box-2 still bringing it up.
    let live = rig
        .wait_session(&id, "running on box-2 again", |s| {
            s.machine.as_ref() == Some(&two) && is_live(s) && s.status != pb::SessionStatus::Starting as i32
        })
        .await;
    assert_eq!(live_agents(&rig, &id), 1, "box-2 resumed its agent; the host shows {live:?}");
    let shown = find(&rig, &desk, &id).await.expect("the host lists it open again");
    assert!(shown.closed_at.is_none());
    assert!(entries(&Desk::Channel(channel.clone()).root(&rig, At::Host)).is_empty(), "this Mac restored nothing");
}

/// The host's Sessions feed, opened now: [`next_change`] waits for what it
/// tells of after this.
async fn sessions_feed(rig: &Rig) -> tonic::Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Sessions feed opens")
        .into_inner()
}

/// Waits for the next change the host's Sessions feed tells of; the start's
/// Resync (D341) is not a change. Fails once `LONG` passes without one, or
/// when the feed ends.
async fn next_change(feed: &mut tonic::Streaming<pb::Event>, what: &str) {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(event))) if matches!(event.payload, Some(pb::event::Payload::Resync(_))) => {}
            Ok(Ok(Some(_))) => return,
            Ok(ended) => panic!("the host's Sessions feed ended ({ended:?}) while waiting for {what}"),
            Err(_) => panic!("{what} never happened within {LONG:?}"),
        }
    }
}

/// Archives `target` on the host with no undo window, and waits for the
/// archive to finish.
async fn archive_target(rig: &Rig, target: &pb::SessionId) {
    let plan = sessions(rig)
        .await
        .plan_archive(pb::PlanArchiveRequest { id: Some(target.clone()) })
        .await
        .expect("the archive is planned")
        .into_inner();
    let choice = if plan.choices.contains(&(pb::ArchiveChoice::Archive as i32)) {
        pb::ArchiveChoice::Archive
    } else {
        pb::ArchiveChoice::ArchiveAndDeleteWork
    };
    let started = sessions(rig)
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(target.clone()),
            choice: choice as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await
        .expect("the archive starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the archive failed: {:?}", ended.error);
}

/// The items in `on`'s Bin, as the host lists them.
async fn bin_of(rig: &Rig, on: &pb::MachineId) -> Vec<pb::TrashItem> {
    pb::trash_service_client::TrashServiceClient::new(rig.channel().await)
        .list_trash(pb::ListTrashRequest {
            machine: Some(on.clone()),
            page: None,
        })
        .await
        .expect("the host lists the Bin")
        .into_inner()
        .items
}

/// Whether the host's row of channel Desk session `id` is closed.
fn closed_on_host(rig: &Rig, id: &pb::SessionId) -> bool {
    At::Host
        .store(rig)
        .channel_desk(&id.value)
        .unwrap()
        .is_some_and(|row| row.closed_unix_ms.is_some())
}

/// CD-5, MV-17: a channel Desk session on box-2 whose target is archived
/// past its undo window closes on box-2. Its folder holds no Markdown, so
/// the watch closes it rather than stopping it: box-2's agent stops, its
/// folder goes to box-2's Bin, and the host shows the session closed and no
/// longer open.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_2_channel_desk_session_closes_on_box_2_once_its_target_is_archived_past_its_undo_window() {
    let rig = Rig::start_with_env(&["box-1", "box-2"], &[REPO], &[NO_UNDO], &[]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch the ledger", Some(&two))).await;
    std::fs::write(folder.join("scratch.txt"), "not markdown\n").unwrap();
    let disk_before = trashed(&rig, At::Box("box-2")).len();
    let items_before = bin_of(&rig, &two).await.len();
    let mut feed = sessions_feed(&rig).await;

    archive_target(&rig, &target).await;

    while !(closed_on_host(&rig, &id) && bin_of(&rig, &two).await.len() == items_before + 1) {
        next_change(&mut feed, "box-2 closing the Desk session").await;
    }
    assert!(!folder.exists(), "box-2 keeps no folder of the session");
    assert_eq!(trashed(&rig, At::Box("box-2")).len(), disk_before + 1, "box-2's Bin took the folder");
    assert_eq!(live_agents(&rig, &id), 0, "box-2's agent stopped");
    assert!(find(&rig, &desk, &id).await.is_none(), "the channel no longer lists it open");
    assert!(closed_on_host(&rig, &id), "the host shows it closed");
}

/// CH-24, CD-5, MV-17: deleting the channel of a Desk session that works on
/// box-2 closes it on box-2: its agent stops there and its folder goes to
/// box-2's Bin.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_2_channel_desk_session_closes_on_box_2_when_its_channel_is_deleted() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Remote watch", Some(&two))).await;
    let disk_before = trashed(&rig, At::Box("box-2")).len();
    let items_before = bin_of(&rig, &two).await.len();
    let mut feed = sessions_feed(&rig).await;
    let mut channels = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await);

    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest {
            channel: channel.id.clone(),
        })
        .await
        .expect("the host plans the delete")
        .into_inner();
    let deleted = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
            end_live: Some(true),
        })
        .await
        .expect("the host deletes the channel")
        .into_inner();
    let ended = rig.ended(deleted.operation.as_ref().expect("a delete has an operation")).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    while bin_of(&rig, &two).await.len() != items_before + 1 {
        next_change(&mut feed, "box-2's Bin to take the Desk session's folder").await;
    }
    assert!(!folder.exists(), "box-2 keeps no folder of the session");
    assert_eq!(trashed(&rig, At::Box("box-2")).len(), disk_before + 1, "box-2's Bin took the folder");
    assert_eq!(live_agents(&rig, &id), 0, "box-2's agent stopped");
}

/// CD-5, MV-17: a channel Desk session on box-2 whose folder holds Markdown
/// shows its target gone, and does not close, once its target is archived
/// past its undo window. box-2's agent keeps running (D801 I: the watch
/// never ends a live holder); its folder and Markdown file stay on box-2;
/// box-2's Bin doesn't grow; the host lists the session stopped, with its
/// target gone, and not closed.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_2_channel_desk_session_with_markdown_stops_on_box_2_once_its_target_is_archived_past_its_undo_window() {
    let rig = Rig::start_with_env(&["box-1", "box-2"], &[REPO], &[NO_UNDO], &[]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch with notes", Some(&two))).await;
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    let items_before = bin_of(&rig, &two).await.len();
    let mut feed = sessions_feed(&rig).await;

    archive_target(&rig, &target).await;

    while !find(&rig, &desk, &id)
        .await
        .and_then(|shown| shown.channel_desk)
        .is_some_and(|channel_desk| channel_desk.target_gone)
    {
        next_change(&mut feed, "box-2 stopping the Desk session").await;
    }
    assert_eq!(live_agents(&rig, &id), 1, "box-2's agent keeps running");
    assert_eq!(std::fs::read_to_string(folder.join("NOTES.md")).unwrap(), NOTES, "box-2 keeps the Markdown file");
    assert!(folder.is_dir(), "box-2 keeps the session's folder");
    assert_eq!(bin_of(&rig, &two).await.len(), items_before, "box-2's Bin didn't grow");
    assert!(find(&rig, &desk, &id).await.is_some(), "the host still lists the stopped session");
    assert!(!closed_on_host(&rig, &id), "the host shows it stopped, not closed");
}

/// CD-5, MV-17: a new target ends a box-2 channel Desk session's target
/// gone; its agent, which the watch left running (D801 I), runs on on
/// box-2, in its own folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_target_runs_a_stopped_box_2_channel_desk_session_again_on_box_2() {
    let rig = Rig::start_with_env(&["box-1", "box-2"], &[REPO], &[NO_UNDO], &[]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let other = rig.start_on(&channel, &host, "Docs pass").await;
    let desk = Desk::Channel(channel.clone());
    let (id, folder) = start(&rig, of_channel(&channel, &target, "Watch with notes", Some(&two))).await;
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    let mut feed = sessions_feed(&rig).await;
    archive_target(&rig, &target).await;
    while !find(&rig, &desk, &id)
        .await
        .and_then(|shown| shown.channel_desk)
        .is_some_and(|channel_desk| channel_desk.target_gone)
    {
        next_change(&mut feed, "box-2 stopping the Desk session").await;
    }
    assert_eq!(live_agents(&rig, &id), 1, "box-2's agent keeps running");

    let answered = desks(&rig)
        .await
        .set_desk_target(pb::SetDeskTargetRequest {
            session: Some(id.clone()),
            target: Some(other.clone()),
        })
        .await
        .expect("the host retargets the stopped session")
        .into_inner();
    assert!(!answered.channel_desk.is_some_and(|channel_desk| channel_desk.target_gone));

    // The resume runs after the answer: Starting is box-2 still bringing it up.
    let live = rig
        .wait_session(&id, "running on box-2", |s| {
            s.machine.as_ref() == Some(&two) && is_live(s) && s.status != pb::SessionStatus::Starting as i32
        })
        .await;
    assert_eq!(live_agents(&rig, &id), 1, "box-2's agent runs again; the host shows {live:?}");
    assert_eq!(std::fs::read_to_string(folder.join("NOTES.md")).unwrap(), NOTES, "its folder is as it was");
    assert!(find(&rig, &desk, &id).await.is_some());
}

/// DV-10: a Desk session's versions (the folder's own repository, `.git`
/// included) and the comments recorded for them move with it to another
/// machine: the same commits, the same head, the same records.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_moved_to_another_machine_keeps_its_versions_and_their_comments() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");
    let (id, folder) = start(&rig, plain("Versioned notes", None)).await;
    let name = name_of(&folder);
    let there = folder.display().to_string();
    std::fs::write(folder.join("NOTES.md"), NOTES).unwrap();
    git(&["-C", &there, "add", "NOTES.md"]);
    git(&["-C", &there, "commit", "-q", "-m", "First notes"]);
    std::fs::write(folder.join("NOTES.md"), "# notes\nmore\n").unwrap();
    git(&["-C", &there, "commit", "-q", "-a", "-m", "More notes"]);
    let versions = git(&["-C", &there, "log", "--format=%H", "--", "NOTES.md"]);
    let head = git(&["-C", &there, "rev-parse", "HEAD"]);
    assert_eq!(versions.lines().count(), 2);
    // A record under a commit the watcher never sees, so it is ours alone.
    let recorded: &[u8] = b"a comment as it was";
    assert!(
        At::Host
            .store(&rig)
            .put_doc_comments(&id.value, "feedface00", "NOTES.md", recorded, 1)
            .unwrap()
    );

    move_to(&rig, &id, &two).await;

    let moved = Desk::Plain.root(&rig, At::Box("box-2")).join(&name);
    let here = moved.display().to_string();
    assert_eq!(git(&["-C", &here, "rev-parse", "HEAD"]), head);
    assert_eq!(git(&["-C", &here, "log", "--format=%H", "--", "NOTES.md"]), versions);
    let store = At::Box("box-2").store(&rig);
    assert_eq!(
        store.doc_comments(&id.value, "feedface00", "NOTES.md").unwrap().as_deref(),
        Some(recorded)
    );
}
