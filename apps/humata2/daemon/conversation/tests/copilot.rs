//! The Copilot adapter on session-state folders built from the real-CLI
//! layout fixture (X1, GitHub Copilot CLI 1.0.112-0) (CV-1–CV-4, CV-7,
//! FK-3, MV-13): where it finds a conversation, which records are turns,
//! what a fork before a turn holds, what a carry to another home and folder
//! places, that the CLI's index and locks are never written, and which
//! conversations and versions it refuses. Every home is a temp folder; the
//! real `~/.copilot` is never read.

use std::collections::BTreeMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use humata2_conversation::{
    Adapter, Conversation, ConversationError, FileRole, Format, GitHead, Placement, Turn, adapter,
};
use serde_json::Value;

/// Two turns as Copilot 1.0.112-0 writes them (`--prompt`, then
/// `--resume=<id> --prompt`): the start, instructions, then per turn its
/// prompt (the first with the system prompt), start, usage, reply, end and
/// the process's shutdown, the second after a resume. `{folder}` and `{id}`
/// are filled per test.
const TWO_TURNS: &str = include_str!("../testdata/copilot/1.0.112-0/two_turns.jsonl");
/// A third turn after [`TWO_TURNS`]: resumed, prompted and started, with
/// no end yet.
const THIRD_RUNNING: &str = include_str!("../testdata/copilot/1.0.112-0/third_turn_running.jsonl");
/// The session's `workspace.yaml`, `checkpoints/index.md` and
/// `rewind-file-snapshots/tracking.json` as the same run left them.
const WORKSPACE: &str = include_str!("../testdata/copilot/1.0.112-0/workspace.yaml");
const CHECKPOINTS: &str = include_str!("../testdata/copilot/1.0.112-0/index.md");
const TRACKING: &str = include_str!("../testdata/copilot/1.0.112-0/tracking.json");

/// The parent conversation's id, and ids for forks and others.
const ID: &str = "6c69568c-ac29-4516-a109-147d96a18dc9";
const FORK_A: &str = "6c69568c-f0a0-4000-8000-00000000000a";
const FORK_B: &str = "6c69568c-f0b0-4000-8000-00000000000b";
const OTHER: &str = "6c69568c-0e0e-4000-8000-0000000000ee";
/// The lines of [`TWO_TURNS`] (from 1) each fork holds. Turn 2 starts at
/// the first process's shutdown, line 9, before the resume and the prompt.
const BEFORE_2: RangeInclusive<usize> = 1..=8;
const ALL: RangeInclusive<usize> = 1..=16;
/// The version that wrote the testdata.
const VERSION: &str = "1.0.112-0";

/// The files of a session folder, under it, besides the transcript.
const COMPANIONS: [&str; 4] = [
    ".workspace-fork.lock",
    "checkpoints/index.md",
    "rewind-file-snapshots/tracking.json",
    "workspace.yaml",
];

fn copilot() -> &'static dyn Adapter {
    adapter(Format::CopilotSessionState)
}

/// A machine `name` under `root`: its home, and a session folder in it.
fn placement(root: &Path, name: &str) -> Placement {
    Placement {
        home: root.join(name).join("home"),
        cli_home: None,
        folder: root.join(name).join("channels/web/sessions/fix-login"),
    }
}

/// Another machine's home with its `COPILOT_HOME` set, and its folder.
fn new_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("new/home"),
        cli_home: Some(root.join("new/copilot-home")),
        folder: root.join("new/work/web/fix-login"),
    }
}

/// The Copilot dir of `at`.
fn copilot_dir(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".copilot"))
}

/// Where Copilot keeps conversation `id` at `at`.
fn session_dir(at: &Placement, id: &str) -> PathBuf {
    copilot_dir(at).join("session-state").join(id)
}

/// `template` as Copilot would have written it for conversation `id` at `at`.
fn fill(template: &str, at: &Placement, id: &str) -> String {
    template
        .replace("{folder}", &at.folder.to_string_lossy())
        .replace("{id}", id)
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Writes conversation `id`'s folder at `at` with `events` as its
/// transcript and `workspace` as its summary, and the files the X1 run
/// left beside them.
fn write_session(at: &Placement, id: &str, events: &str, workspace: &str) -> PathBuf {
    let dir = session_dir(at, id);
    write(&dir.join("events.jsonl"), &fill(events, at, id));
    write(&dir.join("workspace.yaml"), &fill(workspace, at, id));
    write(&dir.join("checkpoints/index.md"), CHECKPOINTS);
    write(&dir.join("rewind-file-snapshots/tracking.json"), TRACKING);
    write(&dir.join(".workspace-fork.lock"), "");
    fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

/// The CLI's own files beside the session folders, as X1 found them: the
/// shared lock of the session and its SQLite index (only its header is
/// real here, and it names the id and folder as the real one does).
fn write_cli_state(at: &Placement) -> Vec<PathBuf> {
    let dir = copilot_dir(at);
    let files = vec![
        dir.join("session-state/.session-operation-locks").join(format!("{ID}.lock")),
        dir.join("session-store.db"),
        dir.join("session-store.db-wal"),
    ];
    for file in &files {
        write(file, &format!("SQLite format 3\0{ID} {}", at.folder.display()));
    }
    files
}

/// Lines `which` (from 1) of `template` as Copilot would have written them
/// for `id` at `at`, parsed.
fn lines_of(template: &str, at: &Placement, id: &str, which: RangeInclusive<usize>) -> Vec<Value> {
    let filled = fill(template, at, id);
    let lines: Vec<&str> = filled.lines().collect();
    which.map(|line| serde_json::from_str(lines[line - 1]).unwrap()).collect()
}

/// Each complete line of the file at `path`, parsed.
fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Every plain file under `dir` with its bytes.
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

/// Whether any file under `dir` holds `needle`.
fn any_file_holds(dir: &Path, needle: &str) -> bool {
    snapshot(dir)
        .values()
        .any(|bytes| String::from_utf8_lossy(bytes).contains(needle))
}

/// The two-turn conversation `ID` at machine `name` under `root`, found.
fn parent(root: &Path, name: &str) -> (Placement, Conversation) {
    let at = placement(root, name);
    write_session(&at, ID, TWO_TURNS, WORKSPACE);
    let conversation = copilot().locate(&at, ID).unwrap();
    (at, conversation)
}

fn two_turns() -> Vec<Turn> {
    let turn = |index, first_line: &str, at: &str| Turn {
        index,
        first_line: first_line.to_owned(),
        at: Some(at.to_owned()),
    };
    vec![
        turn(1, "Fix the failing payments test", "2026-11-17T02:42:43.615Z"),
        turn(2, "Now add a test for the refund path", "2026-11-17T02:42:48.413Z"),
    ]
}

/// The session as Copilot writes it in a git folder: its start's context
/// and its summary name the root and the branch.
fn on_branch(events: &str, workspace: &str, branch: &str) -> (String, String) {
    let events = events.replacen(
        r#""context":{"cwd":"{folder}"}"#,
        &format!(r#""context":{{"cwd":"{{folder}}","gitRoot":"{{folder}}","branch":"{branch}"}}"#),
        1,
    );
    let workspace = workspace.replacen(
        "cwd: {folder}\n",
        &format!("cwd: {{folder}}\ngit_root: {{folder}}\nbranch: {branch}\n"),
        1,
    );
    (events, workspace)
}

/// The branch the session's start and summary name.
fn branches(dir: &Path) -> (Option<String>, Option<String>) {
    let start = records(&dir.join("events.jsonl"))
        .into_iter()
        .find(|record| record["type"] == "session.start")
        .and_then(|record| record.pointer("/data/context/branch").and_then(Value::as_str).map(str::to_owned));
    let summary = fs::read_to_string(dir.join("workspace.yaml"))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("branch: ").map(str::to_owned));
    (start, summary)
}

/// `template` with `line` put after line `after` (from 1).
fn with_line_after(template: &str, after: usize, line: &str) -> String {
    let mut lines: Vec<&str> = template.lines().collect();
    lines.insert(after, line);
    lines.join("\n") + "\n"
}

/// The parent's folder at `at`, rewritten as `edit` says, then checked.
fn check_with(root: &Path, edit: impl Fn(String) -> String) -> Result<(), ConversationError> {
    let at = placement(root, "old");
    write_session(&at, ID, &edit(TWO_TURNS.to_owned()), WORKSPACE);
    let found = copilot().locate(&at, ID)?;
    copilot().check(&found, None)
}

#[test]
fn locate_finds_the_session_folder_with_its_transcript_first_and_none_of_the_clis_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let dir = write_session(&at, ID, TWO_TURNS, WORKSPACE);
    write_session(&at, OTHER, TWO_TURNS, WORKSPACE);
    write_cli_state(&at);

    let found = copilot().locate(&at, ID).unwrap();

    assert_eq!(found.id, ID);
    assert_eq!(found.format, Format::CopilotSessionState);
    assert_eq!(found.placement, at);
    assert_eq!(found.files[0].path, dir.join("events.jsonl"));
    assert_eq!(found.files[0].role, FileRole::Transcript);
    let rest: Vec<(PathBuf, FileRole)> = found.files[1..].iter().map(|file| (file.path.clone(), file.role)).collect();
    let expected: Vec<(PathBuf, FileRole)> =
        COMPANIONS.iter().map(|rel| (dir.join(rel), FileRole::Companion)).collect();
    assert_eq!(rest, expected);
}

#[test]
fn locate_looks_in_the_copilot_home_when_one_is_set() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = new_machine(&root);
    write_session(&at, ID, TWO_TURNS, WORKSPACE);

    let found = copilot().locate(&at, ID).unwrap();

    assert!(found.files[0].path.starts_with(root.join("new/copilot-home/session-state")));
    assert!(!at.home.join(".copilot").exists());
}

#[test]
fn locate_refuses_an_id_that_is_not_a_lowercase_uuid_and_reports_one_that_is_not_there() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, _) = parent(&root, "old");

    for id in ["../session-state", "", "6C69568C-AC29-4516-A109-147D96A18DC9", "6c69568c"] {
        let refused = copilot().locate(&at, id).unwrap_err();
        assert!(matches!(refused, ConversationError::BadId { .. }), "{id}: {refused:?}");
    }
    let missing = copilot().locate(&at, OTHER).unwrap_err();
    assert!(matches!(missing, ConversationError::NotFound { .. }), "{missing:?}");
    fs::remove_file(session_dir(&at, ID).join("events.jsonl")).unwrap();
    let no_transcript = copilot().locate(&at, ID).unwrap_err();
    assert!(matches!(no_transcript, ConversationError::NotFound { .. }), "{no_transcript:?}");
}

#[test]
fn turns_are_the_prompts_in_order_with_their_times() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (_, found) = parent(&root, "old");

    assert_eq!(copilot().turns(&found).unwrap(), two_turns());
}

#[test]
fn a_last_line_still_being_written_is_not_read() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let cut_off = format!("{TWO_TURNS}{{\"type\":\"user.message\",\"data\":{{\"content\":\"Run");
    write_session(&at, ID, &cut_off, WORKSPACE);
    let found = copilot().locate(&at, ID).unwrap();

    assert_eq!(copilot().turns(&found).unwrap(), two_turns());
    copilot().check(&found, Some(VERSION)).unwrap();
}

#[test]
fn a_fork_before_a_turn_holds_exactly_the_lines_before_its_start_under_the_new_id() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let before = snapshot(&copilot_dir(&at));

    let fork = copilot().fork(&found, Some(2), FORK_A, &at).unwrap();

    let path = session_dir(&at, FORK_A).join("events.jsonl");
    assert_eq!(fork.id, FORK_A);
    assert_eq!(fork.files.len(), 1);
    assert_eq!(fork.files[0].path, path);
    assert_eq!(fork.files[0].role, FileRole::Transcript);
    assert_eq!(records(&path), lines_of(TWO_TURNS, &at, FORK_A, BEFORE_2));
    assert!(!fs::read_to_string(&path).unwrap().contains(ID), "the parent's id is gone");
    assert_eq!(copilot().turns(&fork).unwrap(), two_turns()[..1].to_vec());
    let mut after = snapshot(&copilot_dir(&at));
    after.remove(&path);
    assert_eq!(after, before, "only the fork's transcript was written");
}

#[test]
fn a_turns_start_is_where_a_fork_before_it_is_cut() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let bytes = fs::read(&found.files[0].path).unwrap();

    let starts = copilot().turn_starts(&found).unwrap();

    assert_eq!(starts.len(), two_turns().len());
    let kept = usize::try_from(starts[1]).unwrap();
    let before: Vec<Value> = String::from_utf8_lossy(&bytes[..kept])
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(before, lines_of(TWO_TURNS, &at, ID, BEFORE_2));
}

#[test]
fn a_fork_from_now_holds_every_line_of_a_finished_conversation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    assert_eq!(records(&fork.files[0].path), lines_of(TWO_TURNS, &at, FORK_A, ALL));
}

#[test]
fn a_fork_from_now_leaves_out_a_running_turn_from_the_shutdown_before_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let running = format!("{TWO_TURNS}{THIRD_RUNNING}");
    write_session(&at, ID, &running, WORKSPACE);
    let found = copilot().locate(&at, ID).unwrap();
    assert_eq!(copilot().turns(&found).unwrap().len(), 3);

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    assert_eq!(records(&fork.files[0].path), lines_of(&running, &at, FORK_A, 1..=15));
    assert_eq!(copilot().turns(&fork).unwrap(), two_turns());
}

#[test]
fn a_fork_into_another_folder_and_home_names_them_and_not_the_parents() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let to = new_machine(&root);

    let fork = copilot().fork(&found, Some(2), FORK_A, &to).unwrap();

    assert_eq!(fork.files[0].path, session_dir(&to, FORK_A).join("events.jsonl"));
    assert_eq!(records(&fork.files[0].path), lines_of(TWO_TURNS, &to, FORK_A, BEFORE_2));
    assert!(!any_file_holds(&copilot_dir(&to), &at.folder.to_string_lossy()));
    assert_eq!(snapshot(&copilot_dir(&to)).len(), 1, "nothing but the transcript");
}

#[test]
fn a_fork_with_nothing_before_it_or_of_a_turn_not_there_is_refused_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let before = snapshot(&copilot_dir(&at));

    let first = copilot().fork(&found, Some(1), FORK_A, &at).unwrap_err();
    assert!(matches!(first, ConversationError::NothingBefore), "{first:?}");
    for turn in [0, 3] {
        let missing = copilot().fork(&found, Some(turn), FORK_A, &at).unwrap_err();
        assert!(matches!(missing, ConversationError::NoSuchTurn { turns: 2, .. }), "{turn}: {missing:?}");
    }
    let bad = copilot().fork(&found, Some(2), "not-a-uuid", &at).unwrap_err();
    assert!(matches!(bad, ConversationError::BadId { .. }), "{bad:?}");
    assert_eq!(snapshot(&copilot_dir(&at)), before);
}

#[test]
fn a_fork_onto_a_conversation_already_there_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    write_session(&at, FORK_B, TWO_TURNS, WORKSPACE);
    let before = snapshot(&copilot_dir(&at));

    let refused = copilot().fork(&found, None, FORK_B, &at).unwrap_err();

    assert!(matches!(refused, ConversationError::Exists { .. }), "{refused:?}");
    assert_eq!(snapshot(&copilot_dir(&at)), before);
}

#[test]
fn a_pack_stages_every_file_of_the_session_folder_and_none_of_the_clis() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    write_cli_state(&at);
    let stage = root.join("stage");

    let packed = copilot().pack(&found, &stage).unwrap();

    let own = format!("session-state/{ID}");
    let mut rels: Vec<String> = packed.files.iter().map(|file| file.rel.clone()).collect();
    assert_eq!(rels.remove(0), format!("{own}/events.jsonl"));
    assert_eq!(rels, COMPANIONS.map(|rel| format!("{own}/{rel}")).to_vec());
    let staged = snapshot(&stage);
    assert_eq!(staged.len(), packed.files.len());
    for file in &packed.files {
        let original = fs::read(copilot_dir(&at).join(&file.rel)).unwrap();
        assert_eq!(staged[&stage.join(&file.rel)], original, "{} is staged as it was", file.rel);
        assert_eq!(file.size, u64::try_from(original.len()).unwrap());
    }

    let again = copilot().pack(&found, &stage).unwrap_err();
    assert!(matches!(again, ConversationError::Exists { .. }), "{again:?}");
}

#[test]
fn a_carry_to_another_home_and_folder_names_no_old_path_and_reads_there() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);

    let placed = copilot().place(&packed, &at, &to).unwrap();

    let dir = session_dir(&to, ID);
    assert_eq!(placed.files[0].path, dir.join("events.jsonl"));
    assert_eq!(placed.files.len(), packed.files.len());
    assert_eq!(records(&placed.files[0].path), lines_of(TWO_TURNS, &to, ID, ALL));
    assert_eq!(fs::read_to_string(dir.join("workspace.yaml")).unwrap(), fill(WORKSPACE, &to, ID));
    assert!(!any_file_holds(&copilot_dir(&to), &at.folder.to_string_lossy()));
    let found = copilot().locate(&to, ID).unwrap();
    assert_eq!(copilot().turns(&found).unwrap(), two_turns());
    copilot().check(&found, Some(VERSION)).unwrap();
    let written: Vec<PathBuf> = snapshot(&copilot_dir(&to)).into_keys().collect();
    assert!(written.iter().all(|path| path.starts_with(&dir)), "nothing of the CLI's: {written:?}");
}

#[test]
fn a_carry_onto_another_home_that_has_the_conversation_is_refused_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);
    write(&session_dir(&to, ID).join("events.jsonl"), "");
    let before = snapshot(&copilot_dir(&to));

    let refused = copilot().place(&packed, &at, &to).unwrap_err();

    assert!(matches!(refused, ConversationError::Exists { .. }), "{refused:?}");
    assert_eq!(snapshot(&copilot_dir(&to)), before);
}

#[test]
fn a_move_on_one_machine_rewrites_the_conversation_where_it_is_and_never_the_clis_index() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let cli = write_cli_state(&at);
    let cli_before: Vec<Vec<u8>> = cli.iter().map(|file| fs::read(file).unwrap()).collect();
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = Placement {
        folder: root.join("old/channels/payments/sessions/fix-login"),
        ..at.clone()
    };

    let placed = copilot().place(&packed, &at, &to).unwrap();

    assert_eq!(placed.files[0].path, found.files[0].path);
    assert_eq!(records(&placed.files[0].path), lines_of(TWO_TURNS, &to, ID, ALL));
    assert_eq!(fs::read_to_string(session_dir(&to, ID).join("workspace.yaml")).unwrap(), fill(WORKSPACE, &to, ID));
    let cli_after: Vec<Vec<u8>> = cli.iter().map(|file| fs::read(file).unwrap()).collect();
    assert_eq!(cli_after, cli_before, "the CLI's lock and index are as they were");
}

#[test]
fn a_carry_onto_a_head_names_its_branch_where_the_session_recorded_one() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let (events, workspace) = on_branch(TWO_TURNS, WORKSPACE, "main");
    write_session(&at, ID, &events, &workspace);
    let found = copilot().locate(&at, ID).unwrap();
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);
    let head = GitHead {
        branch: Some("humata/fix-login".to_owned()),
        sha: "3f2a6c8d0e1b4a5c9d7e8f0a1b2c3d4e5f607182".to_owned(),
    };

    copilot().place_on_head(&packed, &at, &to, Some(&head)).unwrap();

    let dir = session_dir(&to, ID);
    let named = Some("humata/fix-login".to_owned());
    assert_eq!(branches(&dir), (named.clone(), named));
    let (expected, _) = on_branch(TWO_TURNS, WORKSPACE, "humata/fix-login");
    let placed = records(&dir.join("events.jsonl"));
    assert_eq!(placed, lines_of(&expected, &to, ID, ALL), "only the branch changed");
    let start = &placed[0];
    assert_eq!(start.pointer("/data/context/gitRoot").and_then(Value::as_str), Some(to.folder.to_str().unwrap()));
}

#[test]
fn a_carry_onto_a_detached_head_names_it_and_a_session_with_no_branch_names_none() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let detached = GitHead { branch: None, sha: "3f2a6c8d0e1b4a5c9d7e8f0a1b2c3d4e5f607182".to_owned() };
    let named = GitHead { branch: Some("humata/fix-login".to_owned()), sha: detached.sha.clone() };

    let at = placement(&root, "a");
    let (events, workspace) = on_branch(TWO_TURNS, WORKSPACE, "main");
    write_session(&at, ID, &events, &workspace);
    let packed = copilot().pack(&copilot().locate(&at, ID).unwrap(), &root.join("stage-a")).unwrap();
    let to = placement(&root, "a2");
    copilot().place_on_head(&packed, &at, &to, Some(&detached)).unwrap();
    let named_detached = Some("detached@3f2a6c8".to_owned());
    assert_eq!(branches(&session_dir(&to, ID)), (named_detached.clone(), named_detached));

    let (at, found) = parent(&root, "b");
    let packed = copilot().pack(&found, &root.join("stage-b")).unwrap();
    let to = placement(&root, "b2");
    copilot().place_on_head(&packed, &at, &to, Some(&named)).unwrap();
    assert_eq!(branches(&session_dir(&to, ID)), (None, None));
    assert_eq!(records(&session_dir(&to, ID).join("events.jsonl")), lines_of(TWO_TURNS, &to, ID, ALL));
}

#[test]
fn a_place_of_a_staged_name_outside_the_conversations_folder_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (at, found) = parent(&root, "old");
    let to = new_machine(&root);

    for rel in ["../escape.jsonl", "session-state/other/events.jsonl", "session-store.db"] {
        let mut packed = copilot().pack(&found, &root.join(format!("stage-{}", rel.len()))).unwrap();
        packed.files[1].rel = rel.to_owned();

        let refused = copilot().place(&packed, &at, &to).unwrap_err();

        assert!(matches!(refused, ConversationError::BadPath { .. }), "{rel}: {refused:?}");
    }
    assert!(!copilot_dir(&to).exists());
}

#[test]
fn the_version_that_wrote_the_testdata_and_newer_ones_are_known() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (_, found) = parent(&root, "old");

    for version in [VERSION, "1.0.112", "GitHub Copilot CLI 1.0.112-0.\nRun 'copilot update' to check for updates.\n", "1.0.140", "1.2.0"] {
        copilot().check(&found, Some(version)).unwrap_or_else(|error| panic!("{version}: {error:?}"));
    }
    copilot().check(&found, None).unwrap();
}

#[test]
fn a_cli_older_than_the_first_proven_or_one_whose_version_cannot_be_read_is_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let (_, found) = parent(&root, "old");

    for version in ["1.0.111", "GitHub Copilot CLI 0.0.355.\n", "1.0", "unknown"] {
        let refused = copilot().check(&found, Some(version)).unwrap_err();
        assert!(matches!(refused, ConversationError::Unsupported { .. }), "{version}: {refused:?}");
    }
}

#[test]
fn a_session_started_by_an_older_copilot_or_another_schema_is_unsupported_for_every_operation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let edits: [(&str, &str); 2] = [
        (r#""copilotVersion":"1.0.112-0""#, r#""copilotVersion":"1.0.98-1""#),
        (r#""version":1,"#, r#""version":2,"#),
    ];
    for (n, (old, new)) in edits.iter().enumerate() {
        let at = placement(&root, &format!("m{n}"));
        write_session(&at, ID, &TWO_TURNS.replacen(old, new, 1), WORKSPACE);
        let found = copilot().locate(&at, ID).unwrap();

        let refusals = [
            copilot().check(&found, Some(VERSION)).unwrap_err(),
            copilot().turns(&found).unwrap_err(),
            copilot().fork(&found, None, FORK_A, &at).unwrap_err(),
            copilot().pack(&found, &root.join(format!("stage-{n}"))).unwrap_err(),
        ];

        for refused in refusals {
            assert!(matches!(refused, ConversationError::Unsupported { .. }), "{new}: {refused:?}");
        }
        assert!(!session_dir(&at, FORK_A).exists());
        assert!(!root.join(format!("stage-{n}")).exists());
    }
}

#[test]
fn a_record_of_a_type_the_adapter_does_not_know_makes_the_conversation_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let unknown = r#"{"type":"session.workflow_started","data":{"name":"nightly"},"id":"9a1b2c3d-0000-4000-8000-000000000001","timestamp":"2026-11-17T02:42:43.900Z","parentId":null}"#;

    let refused = check_with(&root, |events| with_line_after(&events, 7, unknown)).unwrap_err();

    assert!(matches!(refused, ConversationError::Unsupported { .. }), "{refused:?}");
}

/// A hook run and a permission prompt, as Copilot's session-event schema
/// (1.0.113-1) shapes them, inside turn 1: after its start and before its
/// reply. `{folder}` and `{id}` are filled per test.
const HOOK_AND_PERMISSION: [&str; 4] = [
    r#"{"type":"hook.start","data":{"hookInvocationId":"h-1","hookType":"preToolUse","input":{"cwd":"{folder}","toolName":"bash"}},"id":"9a1b2c3d-0000-4000-8000-0000000000a1","timestamp":"2026-11-17T02:42:43.100Z","parentId":null}"#,
    r#"{"type":"hook.end","data":{"hookInvocationId":"h-1","hookType":"preToolUse","output":{"decision":"allow"},"success":true},"id":"9a1b2c3d-0000-4000-8000-0000000000a2","timestamp":"2026-11-17T02:42:43.200Z","parentId":"9a1b2c3d-0000-4000-8000-0000000000a1"}"#,
    r#"{"type":"permission.requested","data":{"requestId":"r-1","ownerSessionId":"{id}","permissionRequest":{"kind":"write","fileName":"{folder}/notes.txt"}},"id":"9a1b2c3d-0000-4000-8000-0000000000a3","timestamp":"2026-11-17T02:42:43.300Z","parentId":null}"#,
    r#"{"type":"permission.completed","data":{"requestId":"r-1","result":{"kind":"approved"}},"id":"9a1b2c3d-0000-4000-8000-0000000000a4","timestamp":"2026-11-17T02:42:43.400Z","parentId":"9a1b2c3d-0000-4000-8000-0000000000a3"}"#,
];

/// [`TWO_TURNS`] with [`HOOK_AND_PERMISSION`] after its line `after`.
fn with_hook_and_permission(after: usize) -> String {
    HOOK_AND_PERMISSION
        .iter()
        .rev()
        .fold(TWO_TURNS.to_owned(), |events, line| with_line_after(&events, after, line))
}

#[test]
fn a_conversation_with_a_hook_and_a_permission_prompt_is_known_and_keeps_its_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    write_session(&at, ID, &with_hook_and_permission(5), WORKSPACE);
    let found = copilot().locate(&at, ID).unwrap();

    copilot().check(&found, Some(VERSION)).unwrap();
    assert_eq!(copilot().turns(&found).unwrap(), two_turns());
}

#[test]
fn a_fork_of_a_conversation_with_a_hook_and_a_permission_prompt_names_only_the_forks_id_and_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let from = placement(&root, "old");
    let to = new_machine(&root);
    write_session(&from, ID, &with_hook_and_permission(5), WORKSPACE);
    let found = copilot().locate(&from, ID).unwrap();

    let fork = copilot().fork(&found, None, FORK_A, &to).unwrap();

    let dir = session_dir(&to, FORK_A);
    let kinds: Vec<String> = records(&fork.files[0].path)
        .iter()
        .filter_map(|record| record["type"].as_str().map(str::to_owned))
        .collect();
    for kind in ["hook.start", "hook.end", "permission.requested", "permission.completed"] {
        assert!(kinds.iter().any(|seen| seen == kind), "{kind} kept: {kinds:?}");
    }
    assert!(!any_file_holds(&dir, ID));
    assert!(!any_file_holds(&dir, &from.folder.to_string_lossy()));
    assert_eq!(copilot().turns(&fork).unwrap().len(), 2);
}

#[test]
fn an_unknown_field_is_read_unless_it_holds_a_path() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let field = r#""data":{"turnId":"0","#;

    check_with(&root.join("plain"), |events| events.replacen(r#""data":{"turnId":"0"}"#, r#""data":{"turnId":"0","reason":"stop"}"#, 1))
        .unwrap();
    let refused = check_with(&root.join("path"), |events| {
        events.replacen(r#""data":{"turnId":"0"}"#, &format!(r#"{field}"log":"{{folder}}/turn.log"}}"#), 1)
    })
    .unwrap_err();
    assert!(matches!(refused, ConversationError::Unsupported { .. }), "{refused:?}");
}

#[test]
fn a_transcript_that_does_not_start_the_session_or_starts_it_twice_is_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let first = TWO_TURNS.lines().next().unwrap().to_owned();

    let headless = check_with(&root.join("a"), |events| events.lines().skip(1).map(|line| format!("{line}\n")).collect())
        .unwrap_err();
    let twice = check_with(&root.join("b"), |events| with_line_after(&events, 8, &first)).unwrap_err();

    for refused in [headless, twice] {
        assert!(matches!(refused, ConversationError::Unsupported { .. }), "{refused:?}");
    }
}

#[test]
fn a_line_that_is_not_a_record_is_malformed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();

    for (n, bad) in ["{\"type\":\"user.message\",", "[1,2]", "{\"data\":{}}"].iter().enumerate() {
        let refused = check_with(&root.join(n.to_string()), |events| with_line_after(&events, 2, bad)).unwrap_err();
        assert!(matches!(refused, ConversationError::Malformed { line: 3, .. }), "{bad}: {refused:?}");
    }
}

#[test]
fn a_summary_with_a_line_the_adapter_does_not_know_is_not_carried() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    // A block-scalar `name` is known since 1.0.113-1 (copilot_captured.rs); a key X1 never saw is not.
    let workspace = WORKSPACE.replacen("user_named: false\n", "user_named: false\nmcp_servers: {folder}/mcp.json\n", 1);
    write_session(&at, ID, TWO_TURNS, &workspace);
    let found = copilot().locate(&at, ID).unwrap();

    let refused = copilot().pack(&found, &root.join("stage")).unwrap_err();

    assert!(matches!(refused, ConversationError::Unsupported { .. }), "{refused:?}");
    assert!(!root.join("stage").exists());
    let checked = copilot().check(&found, Some(VERSION)).unwrap_err();
    assert!(matches!(checked, ConversationError::Unsupported { .. }), "{checked:?}");
}
