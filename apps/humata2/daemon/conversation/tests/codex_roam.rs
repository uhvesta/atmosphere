//! A Codex conversation moved to another folder of the same machine (MV-16,
//! CV-4): the rollout is rewritten where it is, not refused as already
//! there; the CLI's thread index row for it names the new folder, and no
//! other row changes; its rows in the CLI's turn history are dropped for
//! the CLI to rebuild, and no other conversation's; and a carry onto a new
//! clone names that clone's git branch and commit. Every home is a temp
//! folder; the real `~/.codex` is
//! never read.

use std::collections::BTreeMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use humata2_conversation::{Adapter, Format, GitHead, Placement, Turn, adapter};
use serde_json::Value;

/// Three turns as codex-cli 0.169.0 writes them. `{codex}`, `{folder}` and
/// `{id}` are filled per test.
const THREE_TURNS: &str = include_str!("../testdata/codex/three_turns.jsonl");

/// The conversation's id (X1's).
const ID: &str = "01a1e655-aca1-7d40-9236-87fee361b5eb";
/// Another conversation of the same Codex home.
const OTHER: &str = "01a1e655-0e0e-7000-8000-0000000000ee";
const DAY: &str = "sessions/2026/11/16";
const STAMP: &str = "2026-11-16T13-48-56";
const ALL: RangeInclusive<usize> = 1..=33;
const VERSION: &str = "0.169.0";
/// The commit and branch the testdata's session line names.
const OLD_SHA: &str = "3f2a6c8d0e1b4a5c9d7e8f0a1b2c3d4e5f607182";
const NEW_SHA: &str = "9b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f708192";

fn codex() -> &'static dyn Adapter {
    adapter(Format::CodexRollout)
}

/// A machine `name` under `root`: its home, and a session folder in it.
fn placement(root: &Path, name: &str) -> Placement {
    Placement {
        home: root.join(name).join("home"),
        cli_home: None,
        folder: root.join(name).join("channels/web/sessions/fix-login"),
    }
}

/// The same machine as `at`, the session moved to another channel's folder.
fn moved(at: &Placement) -> Placement {
    let machine = at.home.parent().unwrap();
    Placement { folder: machine.join("channels/billing/sessions/fix-login"), ..at.clone() }
}

/// Another machine's home with its `CODEX_HOME` set, and its folder.
fn new_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("new/home"),
        cli_home: Some(root.join("new/codex-home")),
        folder: root.join("new/work/web/fix-login"),
    }
}

fn codex_dir(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".codex"))
}

fn rollout(at: &Placement, id: &str) -> PathBuf {
    codex_dir(at).join(DAY).join(format!("rollout-{STAMP}-{id}.jsonl"))
}

fn fill(template: &str, at: &Placement, id: &str) -> String {
    template
        .replace("{codex}", &codex_dir(at).to_string_lossy())
        .replace("{folder}", &at.folder.to_string_lossy())
        .replace("{id}", id)
}

fn write_conversation(at: &Placement, id: &str) -> PathBuf {
    let path = rollout(at, id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, fill(THREE_TURNS, at, id)).unwrap();
    path
}

fn lines_of(at: &Placement, id: &str, which: RangeInclusive<usize>) -> Vec<Value> {
    let filled = fill(THREE_TURNS, at, id);
    let lines: Vec<&str> = filled.lines().collect();
    which.map(|line| serde_json::from_str(lines[line - 1]).unwrap()).collect()
}

fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                folders.push(path);
            } else if meta.is_file() {
                let bytes = fs::read(&path).unwrap();
                found.insert(path, bytes);
            }
        }
    }
    found
}

/// The rollouts under `dir` (not the CLI's SQLite files) that hold `needle`.
fn rollouts_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    snapshot(dir)
        .into_iter()
        .filter(|(path, bytes)| path.extension().is_some_and(|ext| ext == "jsonl") && String::from_utf8_lossy(bytes).contains(needle))
        .map(|(path, _)| path)
        .collect()
}

fn three_turns() -> Vec<Turn> {
    [
        (1, "Fix the login redirect", "2026-11-16T21:48:57.415Z"),
        (2, "Now add a test for it", "2026-11-16T21:48:57.824Z"),
        (3, "Run the tests", "2026-11-16T21:52:10.116Z"),
    ]
    .into_iter()
    .map(|(index, first_line, at)| Turn { index, first_line: first_line.to_owned(), at: Some(at.to_owned()) })
    .collect()
}

/// One row of the CLI's thread index: the columns a move changes.
#[derive(Debug, PartialEq, Eq)]
struct Thread {
    rollout_path: String,
    cwd: String,
    git_sha: Option<String>,
    git_branch: Option<String>,
    title: String,
}

/// The CLI's thread index at `at` (`state_5.sqlite`, table `threads`, with
/// the columns codex-cli 0.169.0 has) holding conversations `ID` and
/// `OTHER`, both in `at`'s folder.
fn write_threads(at: &Placement) -> PathBuf {
    let path = codex_dir(at).join("state_5.sqlite");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                 id TEXT PRIMARY KEY,
                 rollout_path TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 source TEXT NOT NULL,
                 model_provider TEXT NOT NULL,
                 cwd TEXT NOT NULL,
                 title TEXT NOT NULL,
                 sandbox_policy TEXT NOT NULL,
                 approval_mode TEXT NOT NULL,
                 git_sha TEXT,
                 git_branch TEXT,
                 git_origin_url TEXT,
                 cli_version TEXT NOT NULL DEFAULT '',
                 first_user_message TEXT NOT NULL DEFAULT ''
             );",
        )
        .unwrap();
    for id in [ID, OTHER] {
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider, cwd, title,
                     sandbox_policy, approval_mode, git_sha, git_branch, git_origin_url, cli_version, first_user_message)
                 VALUES (?1, ?2, 1763329736, 1763329930, 'exec', 'fixture', ?3, 'Fix the login redirect',
                     'workspace-write', 'never', ?4, 'main', 'https://git.example.com/team/web.git', ?5,
                     'Fix the login redirect')",
                rusqlite::params![
                    id,
                    rollout(at, id).to_string_lossy(),
                    at.folder.to_string_lossy(),
                    OLD_SHA,
                    VERSION
                ],
            )
            .unwrap();
    }
    path
}

fn thread(db: &Path, id: &str) -> Thread {
    let connection = rusqlite::Connection::open(db).unwrap();
    connection
        .query_row(
            "SELECT rollout_path, cwd, git_sha, git_branch, title FROM threads WHERE id = ?1",
            [id],
            |row| {
                Ok(Thread {
                    rollout_path: row.get(0)?,
                    cwd: row.get(1)?,
                    git_sha: row.get(2)?,
                    git_branch: row.get(3)?,
                    title: row.get(4)?,
                })
            },
        )
        .unwrap()
}

/// The CLI's turn history at `at` (`thread_history_1.sqlite`, with the
/// tables and the cleanup trigger codex-cli 0.169.0 has) holding a
/// projection of `ID` and of `OTHER`: two turns with their byte offsets,
/// their items, a realtime item and where the projection stopped.
fn write_history(at: &Placement) -> PathBuf {
    let path = codex_dir(at).join("thread_history_1.sqlite");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE thread_turns (
                 thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, rollout_ordinal INTEGER NOT NULL,
                 status TEXT NOT NULL, error_json TEXT, started_at INTEGER, completed_at INTEGER,
                 duration_ms INTEGER, first_user_item_id TEXT, final_agent_item_id TEXT,
                 rollout_byte_offset INTEGER, rollout_end_ordinal INTEGER, rollout_end_byte_offset INTEGER,
                 root_turn_id TEXT, PRIMARY KEY (thread_id, turn_id));
             CREATE TABLE thread_items (
                 thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, item_id TEXT NOT NULL,
                 rollout_ordinal INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, item_json TEXT NOT NULL,
                 item_type TEXT NOT NULL DEFAULT '', updated_at_ordinal INTEGER NOT NULL DEFAULT 0,
                 started_at_ms INTEGER, completed_at_ms INTEGER, PRIMARY KEY (thread_id, turn_id, item_id));
             CREATE TABLE thread_history_projection_state (
                 thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER NOT NULL,
                 next_rollout_ordinal INTEGER NOT NULL);
             CREATE TABLE thread_realtime_items (
                 thread_id TEXT NOT NULL, item_id TEXT NOT NULL, rollout_ordinal INTEGER NOT NULL,
                 created_at_ms INTEGER NOT NULL, item_type TEXT NOT NULL, item_json TEXT NOT NULL,
                 PRIMARY KEY (thread_id, item_id));
             CREATE TRIGGER thread_realtime_items_projection_cleanup
                 AFTER DELETE ON thread_history_projection_state
             BEGIN
                 DELETE FROM thread_realtime_items WHERE thread_id = OLD.thread_id;
             END;",
        )
        .unwrap();
    for id in [ID, OTHER] {
        for ordinal in [1_i64, 2] {
            let turn = format!("turn-{ordinal}");
            connection
                .execute(
                    "INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, status, rollout_byte_offset,
                         rollout_end_ordinal, rollout_end_byte_offset)
                     VALUES (?1, ?2, ?3, 'completed', ?4, ?3, ?5)",
                    rusqlite::params![id, turn, ordinal, ordinal * 1000, ordinal * 1000 + 900],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO thread_items (thread_id, turn_id, item_id, rollout_ordinal, created_at_ms, item_json, item_type)
                     VALUES (?1, ?2, ?3, ?4, 1763329736000, '{}', 'userMessage')",
                    rusqlite::params![id, turn, format!("item-{ordinal}"), ordinal],
                )
                .unwrap();
        }
        connection
            .execute("INSERT INTO thread_history_projection_state VALUES (?1, 2900, 3)", [id])
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_realtime_items VALUES (?1, 'live-1', 3, 1763329736000, 'agentMessage', '{}')",
                [id],
            )
            .unwrap();
    }
    path
}

/// The rows of thread `id` in each table of the turn history `db`.
fn history_rows(db: &Path, id: &str) -> BTreeMap<&'static str, Vec<String>> {
    let connection = rusqlite::Connection::open(db).unwrap();
    ["thread_turns", "thread_items", "thread_history_projection_state", "thread_realtime_items"]
        .into_iter()
        .map(|table| {
            let mut statement =
                connection.prepare(&format!("SELECT * FROM {table} WHERE thread_id = ?1 ORDER BY 2")).unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([id], |row| {
                    let cell = |column: usize| row.get::<_, rusqlite::types::Value>(column);
                    let cells = (0..columns)
                        .map(|column| cell(column).map(|value| format!("{value:?}")))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(cells.join("|"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

fn new_head() -> GitHead {
    GitHead { branch: Some("humata/billing".to_owned()), sha: NEW_SHA.to_owned() }
}

#[test]
fn a_move_to_another_folder_of_the_same_machine_rewrites_the_rollout_where_it_is() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "mac");
    write_conversation(&from, ID);
    let to = moved(&from);

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = codex().place(&packed, &from, &to).unwrap();

    assert_eq!(placed.files[0].path, rollout(&to, ID), "the rollout keeps its place in the Codex dir");
    assert_eq!(records(&rollout(&to, ID)), lines_of(&to, ID, ALL));
    assert_eq!(rollouts_holding(&codex_dir(&to), &from.folder.to_string_lossy()), Vec::<PathBuf>::new());
    let found = codex().locate(&to, ID).unwrap();
    assert_eq!(found.files, placed.files);
    codex().check(&found, Some(VERSION)).unwrap();
    assert_eq!(codex().turns(&found).unwrap(), three_turns());
}

#[test]
fn a_move_on_the_same_machine_points_the_clis_thread_row_at_the_new_folder_and_leaves_other_rows() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "mac");
    write_conversation(&from, ID);
    write_conversation(&from, OTHER);
    let db = write_threads(&from);
    let other_before = thread(&db, OTHER);
    let to = moved(&from);

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    codex().place(&packed, &from, &to).unwrap();

    assert_eq!(
        thread(&db, ID),
        Thread {
            rollout_path: rollout(&to, ID).to_string_lossy().into_owned(),
            cwd: to.folder.to_string_lossy().into_owned(),
            git_sha: Some(OLD_SHA.to_owned()),
            git_branch: Some("main".to_owned()),
            title: "Fix the login redirect".to_owned(),
        }
    );
    assert_eq!(thread(&db, OTHER), other_before, "another conversation's row is as it was");
    assert_eq!(records(&rollout(&from, OTHER)), lines_of(&from, OTHER, ALL), "another rollout is as it was");
}

#[test]
fn a_carry_onto_a_new_clone_names_its_branch_and_commit_in_the_session_line() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "old");
    write_conversation(&from, ID);
    let to = new_machine(root.path());

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = codex().place_on_head(&packed, &from, &to, Some(&new_head())).unwrap();

    let got = records(&placed.files[0].path);
    let mut wanted = lines_of(&to, ID, ALL);
    wanted[0]["payload"]["git"]["branch"] = Value::from("humata/billing");
    wanted[0]["payload"]["git"]["commit_hash"] = Value::from(NEW_SHA);
    assert_eq!(got, wanted, "only the session line's branch and commit change; the remote is kept");
    codex().check(&placed, Some(VERSION)).unwrap();
}

#[test]
fn a_carry_with_no_new_clone_keeps_the_branch_and_commit_it_had() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "old");
    write_conversation(&from, ID);
    let to = new_machine(root.path());

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = codex().place_on_head(&packed, &from, &to, None).unwrap();

    assert_eq!(records(&placed.files[0].path), lines_of(&to, ID, ALL));
}

#[test]
fn a_move_on_the_same_machine_onto_a_new_clone_names_it_in_the_rollout_and_the_thread_row() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "mac");
    write_conversation(&from, ID);
    let db = write_threads(&from);
    let to = moved(&from);

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    codex().place_on_head(&packed, &from, &to, Some(&new_head())).unwrap();

    let session = &records(&rollout(&to, ID))[0];
    assert_eq!(session["payload"]["git"]["branch"], Value::from("humata/billing"));
    assert_eq!(session["payload"]["git"]["commit_hash"], Value::from(NEW_SHA));
    let row = thread(&db, ID);
    assert_eq!(row.cwd, to.folder.to_string_lossy());
    assert_eq!(row.git_branch.as_deref(), Some("humata/billing"));
    assert_eq!(row.git_sha.as_deref(), Some(NEW_SHA));
}

#[test]
fn a_move_on_the_same_machine_drops_the_conversations_turn_history_for_the_cli_to_rebuild_and_leaves_others() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "mac");
    write_conversation(&from, ID);
    write_conversation(&from, OTHER);
    let history = write_history(&from);
    let other_before = history_rows(&history, OTHER);
    let to = moved(&from);

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    codex().place(&packed, &from, &to).unwrap();

    for (table, rows) in history_rows(&history, ID) {
        assert_eq!(rows, Vec::<String>::new(), "{table} still points into the old rollout");
    }
    assert_eq!(history_rows(&history, OTHER), other_before, "another conversation's history is as it was");
    let found = codex().locate(&to, ID).unwrap();
    assert_eq!(codex().turns(&found).unwrap(), three_turns());
}

#[test]
fn a_carry_to_another_machine_leaves_the_old_machines_turn_history_as_it_was() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "old");
    write_conversation(&from, ID);
    let history = write_history(&from);
    let before = history_rows(&history, ID);
    let to = new_machine(root.path());

    let conversation = codex().locate(&from, ID).unwrap();
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    codex().place(&packed, &from, &to).unwrap();

    assert_eq!(history_rows(&history, ID), before);
    assert!(!codex_dir(&to).join("thread_history_1.sqlite").exists(), "the new machine's CLI builds its own");
}
