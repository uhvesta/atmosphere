//! A moved session's records arrive whole and name the new machine's paths
//! (MV-13, MV-14, MV-16, D195): every table that belongs to the session is
//! carried in the side file, and every path the store keeps for it is moved
//! under the new folder or conversation folder, so no row on the new machine
//! names the old ones.

use std::path::{Path, PathBuf};

use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, ArchiveRow, ArchiveState, ChannelDeskRow, ChannelRecord, ChannelRepo,
    CommentChange, CommentDeliveryRecord, CommentDeliveryState, CommentNumbers, CommentRecord, DeskFolderKind, DeskRow,
    ForkOrigin, FreshSeed, MainClone, PreviousConversation, SessionComments, SessionDetail, SessionRecord, SessionRepo,
    StartStep, Store, Take,
};

const OLD_FOLDER: &str = "/old/payments/sessions/fix";
const NEW_FOLDER: &str = "/new/payments/sessions/fix";
/// Where the agent kept the old folder's conversations, and keeps the new one's.
const OLD_PROJECT: &str = "/old/home/.claude/projects/-old-payments-sessions-fix";
const NEW_PROJECT: &str = "/new/home/.claude/projects/-new-payments-sessions-fix";

/// The tables whose rows belong to a session.
const SESSION_TABLES: &[&str] = &[
    "sessions",
    "session_details",
    "session_repos",
    "agents",
    "interruptions",
    "reviews",
    "views",
    "desk_sessions",
    "channel_desk_sessions",
    "previous_conversations",
    "fresh_seeds",
    "fork_origins",
    "archives",
    "comments",
    "comment_deliveries",
    "comment_numbers",
];

fn channel() -> ChannelRecord {
    ChannelRecord {
        id: "ch_1".to_owned(),
        name: "Payments".to_owned(),
        folder: "payments".to_owned(),
        branch_prefix: None,
        default_agent: "claude".to_owned(),
        repos: vec![ChannelRepo { owner: "acme".to_owned(), name: "api".to_owned(), main: MainClone::Ready }],
        machines: vec!["ma_1".to_owned()],
        revision: 1,
        created_unix_ms: 1,
    }
}

fn desk_row() -> DeskRow {
    DeskRow {
        session_id: "se_1".to_owned(),
        kind: DeskFolderKind::Scratch { kept: true },
        boards: true,
        channels_and_machines: false,
        last_used_unix_ms: 20,
        closed_unix_ms: Some(21),
        deleted_unix_ms: None,
        held_work: "api has 2 changed or untracked file(s)".to_owned(),
    }
}

fn channel_desk_row() -> ChannelDeskRow {
    ChannelDeskRow {
        session_id: "se_1".to_owned(),
        channel: "ch_1".to_owned(),
        target_session: Some("se_0".to_owned()),
        read_files: true,
        send_notes: true,
        type_into_terminal: false,
        start_and_fork: true,
        stop_and_archive: false,
        change_board_items: true,
        last_used_unix_ms: 22,
        closed_unix_ms: None,
        target_gone: false,
        trash_item: None,
    }
}

fn seed() -> FreshSeed {
    FreshSeed {
        session_id: "se_1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        seed: "Carry on with the login fix".to_owned(),
        created_unix_ms: 23,
    }
}

fn origin() -> ForkOrigin {
    ForkOrigin { session_id: "se_1".to_owned(), parent: "se_0".to_owned(), kind: 1, before_turn: 3, created_unix_ms: 24 }
}

fn archive() -> ArchiveRow {
    ArchiveRow {
        session_id: "se_1".to_owned(),
        archived_unix_ms: 25,
        state: ArchiveState::Archived,
        summary: b"\x0a\x07summary".to_vec(),
        free_after_unix_ms: 26,
    }
}

fn comments() -> SessionComments {
    SessionComments {
        comments: vec![CommentRecord {
            id: "c-1".to_owned(),
            archived: false,
            delivery_state: CommentDeliveryState::Unconfirmed,
            comment: b"\x0a\x05hello".to_vec(),
            updated_unix_ms: 27,
        }],
        deliveries: vec![CommentDeliveryRecord {
            key: "d-1".to_owned(),
            state: CommentDeliveryState::Unconfirmed,
            delivery: b"\x0a\x03c-1".to_vec(),
        }],
        numbers: CommentNumbers { last_comment: 1, last_delivery: 1 },
    }
}

/// A store at `root` holding session `se_1` in `OLD_FOLDER` with a row in
/// every table that belongs to a session.
fn filled(root: &Path) -> Store {
    let store = Store::open(root).unwrap();
    store.put_channel(&channel()).unwrap();
    store
        .put_session(&SessionRecord {
            id: "se_1".to_owned(),
            channel: "ch_1".to_owned(),
            machine: String::new(),
            title: "Fix".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::from(OLD_FOLDER),
            created_unix_ms: 1,
        })
        .unwrap();
    store
        .put_session_detail(&SessionDetail {
            session: "se_1".to_owned(),
            repos: vec![SessionRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                branch: "fix".to_owned(),
                path: Path::new(OLD_FOLDER).join("api"),
            }],
            model: None,
            first_message: None,
            start: StartStep::Done,
            operation: None,
            revision: 1,
        })
        .unwrap();
    store
        .put_agent(&AgentRecord {
            session: "se_1".to_owned(),
            tab: "tb_1".to_owned(),
            launch: AgentLaunch {
                agent: "claude".to_owned(),
                folder: PathBuf::from(OLD_FOLDER),
                conversation_id: "conv-1".to_owned(),
                conversation_file: Path::new(OLD_PROJECT).join("conv-1.jsonl"),
                holder_socket: PathBuf::from("/run/h/tb_1.sock"),
                ..AgentLaunch::default()
            },
            state: AgentState::Stopped,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 1,
        })
        .unwrap();
    store.put_desk(&desk_row()).unwrap();
    store.put_channel_desk(&channel_desk_row()).unwrap();
    store
        .push_previous_conversation("se_1", "conv-0", &Path::new(OLD_PROJECT).join("conv-0.jsonl"), 19)
        .unwrap();
    store.put_fresh_seed(&seed()).unwrap();
    store.put_fork_origin(&origin()).unwrap();
    store.put_archive(&archive()).unwrap();
    let saved = comments();
    store
        .apply_comments(&CommentChange {
            session: "se_1".to_owned(),
            put: saved.comments,
            removed: Vec::new(),
            put_deliveries: saved.deliveries,
            removed_deliveries: Vec::new(),
            numbers: saved.numbers,
        })
        .unwrap();
    store
}

/// The moves a carry of `se_1` to the new machine asks for: its folder, and
/// the folder its agent keeps the conversations of that folder in.
fn moves() -> Vec<(PathBuf, PathBuf)> {
    vec![
        (PathBuf::from(OLD_PROJECT), PathBuf::from(NEW_PROJECT)),
        (PathBuf::from(OLD_FOLDER), PathBuf::from(NEW_FOLDER)),
    ]
}

/// The tables of `SESSION_TABLES` (in the state at `root`) with a text or
/// blob value that holds `needle`.
fn tables_holding(root: &Path, needle: &str) -> Vec<String> {
    let connection = rusqlite::Connection::open(root.join("state.sqlite")).unwrap();
    let mut found = Vec::new();
    for table in SESSION_TABLES {
        let mut statement = connection.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for column in 0..columns {
                let bytes = match row.get_ref(column).unwrap() {
                    rusqlite::types::ValueRef::Text(bytes) | rusqlite::types::ValueRef::Blob(bytes) => bytes.to_vec(),
                    _ => continue,
                };
                if bytes.windows(needle.len()).any(|window| window == needle.as_bytes()) {
                    found.push((*table).to_owned());
                }
            }
        }
    }
    found
}

#[test]
fn a_moved_session_brings_every_row_of_its_own_into_a_store_that_never_had_it() {
    let dir = tempfile::tempdir().unwrap();
    let old = filled(&dir.path().join("old"));
    let side = dir.path().join("records.sqlite");
    old.export_session("se_1", &side).unwrap();

    let new_root = dir.path().join("new");
    let new = Store::open(&new_root).unwrap();
    new.put_channel(&channel()).unwrap();
    new.import_moved_session("se_1", &side, Take::Everything, &moves()).unwrap();

    assert_eq!(new.desk("se_1").unwrap(), Some(desk_row()));
    assert_eq!(new.channel_desk("se_1").unwrap(), Some(channel_desk_row()));
    assert_eq!(new.fresh_seed("se_1").unwrap(), Some(seed()));
    assert_eq!(new.fork_origin("se_1").unwrap(), Some(origin()));
    assert_eq!(new.archive("se_1").unwrap(), Some(archive()));
    assert_eq!(new.comments("se_1").unwrap(), comments());
    assert_eq!(
        new.previous_conversations("se_1").unwrap(),
        vec![PreviousConversation {
            conversation_id: "conv-0".to_owned(),
            path: Path::new(NEW_PROJECT).join("conv-0.jsonl"),
            ended_unix_ms: 19,
        }]
    );
    assert_eq!(old.desk("se_1").unwrap(), Some(desk_row()), "the old store is as it was");
}

#[test]
fn no_row_of_a_moved_session_names_its_old_folder() {
    let dir = tempfile::tempdir().unwrap();
    let old = filled(&dir.path().join("old"));
    let side = dir.path().join("records.sqlite");
    old.export_session("se_1", &side).unwrap();

    let new_root = dir.path().join("new");
    let new = Store::open(&new_root).unwrap();
    new.put_channel(&channel()).unwrap();
    new.import_moved_session("se_1", &side, Take::Everything, &moves()).unwrap();

    assert_eq!(new.session("se_1").unwrap().unwrap().folder, PathBuf::from(NEW_FOLDER));
    assert_eq!(new.session_detail("se_1").unwrap().unwrap().repos[0].path, Path::new(NEW_FOLDER).join("api"));
    let launch = new.agent("se_1").unwrap().unwrap().launch;
    assert_eq!(launch.folder, PathBuf::from(NEW_FOLDER));
    assert_eq!(launch.conversation_file, Path::new(NEW_PROJECT).join("conv-1.jsonl"));
    assert!(new.previous_conversations("se_1").unwrap().iter().all(|previous| previous.path.starts_with(NEW_PROJECT)));
    for old_path in [OLD_FOLDER, OLD_PROJECT] {
        assert_eq!(tables_holding(&new_root, old_path), Vec::<String>::new(), "{old_path} is still named");
    }
}

#[test]
fn a_path_outside_every_move_is_kept_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let old = filled(&dir.path().join("old"));
    let side = dir.path().join("records.sqlite");
    old.export_session("se_1", &side).unwrap();

    let new = Store::open(dir.path().join("new")).unwrap();
    new.put_channel(&channel()).unwrap();
    // Only the folder moves: the agent's conversations stay where they are,
    // and a path that merely starts with the folder's text isn't in it.
    let folder_only = [(PathBuf::from("/old/payments/sessions/fi"), PathBuf::from(NEW_FOLDER))];
    new.import_moved_session("se_1", &side, Take::Everything, &folder_only).unwrap();

    assert_eq!(new.session("se_1").unwrap().unwrap().folder, PathBuf::from(OLD_FOLDER));
    assert_eq!(
        new.agent("se_1").unwrap().unwrap().launch.conversation_file,
        Path::new(OLD_PROJECT).join("conv-1.jsonl")
    );
    assert_eq!(new.desk("se_1").unwrap(), Some(desk_row()));
}

/// The old machine's agent runs under a holder the new machine doesn't
/// have: the moved row comes in stopped, with no pid of the old machine's.
#[test]
fn a_moved_sessions_running_agent_comes_in_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let old = filled(&dir.path().join("old"));
    let mut agent = old.agent("se_1").unwrap().unwrap();
    agent.state = AgentState::Running { holder_pid: 4242 };
    old.put_agent(&agent).unwrap();
    let side = dir.path().join("records.sqlite");
    old.export_session("se_1", &side).unwrap();

    let new = Store::open(dir.path().join("new")).unwrap();
    new.put_channel(&channel()).unwrap();
    new.import_moved_session("se_1", &side, Take::Everything, &moves()).unwrap();

    assert_eq!(new.agent("se_1").unwrap().unwrap().state, AgentState::Stopped);
    assert_eq!(
        old.agent("se_1").unwrap().unwrap().state,
        AgentState::Running { holder_pid: 4242 },
        "the old store is as it was"
    );
}
