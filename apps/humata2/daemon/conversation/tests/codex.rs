//! The Codex adapter on rollouts built from the real-CLI layout fixture (X1,
//! codex-cli 0.169.0) (CV-1–CV-4, CV-7, FK-3, MV-13): where it finds a
//! conversation, which lines are turns, what a fork before a turn holds,
//! what a carry to another home and folder places, that the CLI's SQLite
//! files are never written, and which lines it refuses to read. Every home
//! is a temp folder; the real `~/.codex` is never read.

use std::collections::BTreeMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use humata2_conversation::{
    Adapter, Conversation, ConversationError, ConversationFile, FileRole, Format, PackedFile, Placement, Turn, adapter,
};
use serde_json::Value;

/// Three turns as codex-cli 0.169.0 writes them (`exec`, then two
/// `exec resume`): the session line, injected instructions and context, and
/// per turn its start, settings, prompt, reply, usage and end. `{codex}`,
/// `{folder}` and `{id}` are filled per test.
const THREE_TURNS: &str = include_str!("../testdata/codex/three_turns.jsonl");
/// Four lines, the third cut off mid-record.
const MALFORMED: &str = include_str!("../testdata/codex/malformed.jsonl");
/// A fourth turn after [`THREE_TURNS`], started and prompted but not
/// complete.
const FOURTH_RUNNING: &str = include_str!("../testdata/codex/fourth_turn_running.jsonl");

/// The parent conversation's id (X1's).
const ID: &str = "01a1e655-aca1-7d40-9236-87fee361b5eb";
/// Ids for forks and other conversations.
const FORK_A: &str = "01a1e655-f0a0-7000-8000-00000000000a";
const FORK_B: &str = "01a1e655-f0b0-7000-8000-00000000000b";
const FORK_C: &str = "01a1e655-f0c0-7000-8000-00000000000c";
const OTHER: &str = "01a1e655-0e0e-7000-8000-0000000000ee";
/// The parent's date folder and the local time in its name.
const DAY: &str = "sessions/2026/11/16";
const STAMP: &str = "2026-11-16T13-48-56";
/// The lines of [`THREE_TURNS`] (from 1) each fork holds. Turn 2 starts at
/// line 16 (after the resumed process's settings, line 15), turn 3 at 25.
const BEFORE_2: RangeInclusive<usize> = 1..=15;
const BEFORE_3: RangeInclusive<usize> = 1..=24;
const ALL: RangeInclusive<usize> = 1..=33;
/// The version that wrote the testdata.
const VERSION: &str = "0.169.0";

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

/// Another machine's home with its `CODEX_HOME` set, and its folder.
fn new_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("new/home"),
        cli_home: Some(root.join("new/codex-home")),
        folder: root.join("new/work/web/fix-login"),
    }
}

/// The Codex dir of `at`.
fn codex_dir(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".codex"))
}

/// Where Codex keeps conversation `id` of the parent's day at `at`.
fn rollout(at: &Placement, id: &str) -> PathBuf {
    codex_dir(at).join(DAY).join(format!("rollout-{STAMP}-{id}.jsonl"))
}

/// `template` as Codex would have written it for conversation `id` at `at`.
fn fill(template: &str, at: &Placement, id: &str) -> String {
    template
        .replace("{codex}", &codex_dir(at).to_string_lossy())
        .replace("{folder}", &at.folder.to_string_lossy())
        .replace("{id}", id)
}

/// Writes `template` as conversation `id`'s rollout at `at`.
fn write_conversation(at: &Placement, id: &str, template: &str) -> PathBuf {
    let path = rollout(at, id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, fill(template, at, id)).unwrap();
    path
}

/// Lines `which` (from 1) of `template` as Codex would have written them for
/// `id` at `at`, parsed. A fork or carry that renamed and moved a
/// conversation right holds exactly these.
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

/// The CLI's own state beside the rollouts, as X1 found it: SQLite files
/// (only their header is real here) and an empty shell snapshot folder.
fn write_cli_state(at: &Placement) -> Vec<PathBuf> {
    let dir = codex_dir(at);
    fs::create_dir_all(dir.join("shell_snapshots")).unwrap();
    let files: Vec<PathBuf> = [
        "state_5.sqlite",
        "state_5.sqlite-wal",
        "thread_history_1.sqlite",
        "thread_history_1.sqlite-wal",
        "goals_1.sqlite",
        "logs_2.sqlite",
    ]
    .iter()
    .map(|name| dir.join(name))
    .collect();
    for file in &files {
        let mut bytes = b"SQLite format 3\0".to_vec();
        bytes.extend_from_slice(format!("{ID} {}", at.folder.display()).as_bytes());
        fs::write(file, bytes).unwrap();
    }
    files
}

/// The SQLite files under `dir`.
fn sqlite_files(dir: &Path) -> Vec<PathBuf> {
    snapshot(dir)
        .into_keys()
        .filter(|path| path.to_string_lossy().contains(".sqlite"))
        .collect()
}

/// The three-turn conversation `ID` at machine `name` under `root`, found.
fn parent(root: &Path, name: &str) -> (Placement, Conversation) {
    let at = placement(root, name);
    write_conversation(&at, ID, THREE_TURNS);
    let conversation = codex().locate(&at, ID).unwrap();
    (at, conversation)
}

fn turn(index: u32, first_line: &str, at: &str) -> Turn {
    Turn {
        index,
        first_line: first_line.to_owned(),
        at: Some(at.to_owned()),
    }
}

fn three_turns() -> Vec<Turn> {
    vec![
        turn(1, "Fix the login redirect", "2026-11-16T21:48:57.415Z"),
        turn(2, "Now add a test for it", "2026-11-16T21:48:57.824Z"),
        turn(3, "Run the tests", "2026-11-16T21:52:10.116Z"),
    ]
}

/// The lines of `template`, with `line` put after line `after` (from 1).
fn with_line_after(template: &str, after: usize, line: &str) -> String {
    let mut lines: Vec<&str> = template.lines().collect();
    lines.insert(after, line);
    lines.join("\n") + "\n"
}

/// The lines of `template`, with line `which` (from 1) changed by `change`.
fn with_line_changed(template: &str, which: usize, change: impl Fn(&str) -> String) -> String {
    let lines: Vec<String> = template
        .lines()
        .enumerate()
        .map(|(index, line)| if index + 1 == which { change(line) } else { line.to_owned() })
        .collect();
    lines.join("\n") + "\n"
}

// Locate.

#[test]
fn locate_finds_the_rollout_under_its_date_folder_in_the_home() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    assert_eq!(conversation.id, ID);
    assert_eq!(conversation.format, Format::CodexRollout);
    assert_eq!(conversation.placement, at);
    assert_eq!(
        conversation.files,
        vec![ConversationFile {
            path: at.home.join(".codex").join(DAY).join(format!("rollout-{STAMP}-{ID}.jsonl")),
            role: FileRole::Transcript,
        }]
    );
}

#[test]
fn locate_reads_the_cli_home_as_codex_home() {
    let root = tempfile::tempdir().unwrap();
    let in_home = placement(root.path(), "mac");
    write_conversation(&in_home, ID, THREE_TURNS);
    let configured = Placement {
        cli_home: Some(root.path().join("mac/codex-home")),
        ..in_home.clone()
    };

    // The copy under ~/.codex is not where a CLI with CODEX_HOME set looks.
    assert!(matches!(
        codex().locate(&configured, ID),
        Err(ConversationError::NotFound { .. })
    ));

    write_conversation(&configured, ID, THREE_TURNS);
    let found = codex().locate(&configured, ID).unwrap();
    let wanted = root
        .path()
        .join("mac/codex-home")
        .join(DAY)
        .join(format!("rollout-{STAMP}-{ID}.jsonl"));
    assert_eq!(found.files[0].path, wanted);
    assert_eq!(found.files[0].role, FileRole::Transcript);
}

#[test]
fn locate_finds_a_rollout_in_any_date_folder() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let path = codex_dir(&at).join(format!("sessions/2025/01/02/rollout-2025-01-02T03-04-05-{ID}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, fill(THREE_TURNS, &at, ID)).unwrap();

    let found = codex().locate(&at, ID).unwrap();

    assert_eq!(
        found.files,
        vec![ConversationFile {
            path,
            role: FileRole::Transcript,
        }]
    );
    assert_eq!(codex().turns(&found).unwrap(), three_turns());
}

#[test]
fn locate_lists_only_the_rollout_not_the_clis_state_or_other_conversations() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, THREE_TURNS);
    write_conversation(&at, OTHER, THREE_TURNS);
    write_cli_state(&at);

    let conversation = codex().locate(&at, ID).unwrap();

    assert_eq!(
        conversation.files,
        vec![ConversationFile {
            path: rollout(&at, ID),
            role: FileRole::Transcript,
        }]
    );
}

#[test]
fn locate_refuses_a_missing_conversation_and_an_id_that_is_not_a_full_uuid() {
    let root = tempfile::tempdir().unwrap();
    let (at, _) = parent(root.path(), "mac");

    assert!(matches!(
        codex().locate(&at, OTHER),
        Err(ConversationError::NotFound { .. })
    ));
    for bad in [
        "",
        ".",
        "..",
        "../x",
        "a/b",
        "/abs",
        "fork-1",
        "01a1e655",
        "01a1e655-aca1-7d40-9236",
        "01A1E655-ACA1-7D40-9236-87FEE361B5EB",
        "01a1e655aca17d40923687fee361b5eb",
        "01a1e655-aca1-7d40-9236-87fee361b5eb.jsonl",
        "*",
    ] {
        assert!(
            matches!(codex().locate(&at, bad), Err(ConversationError::BadId { .. })),
            "{bad:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_link_as_the_rollout_is_refused_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let outside = root.path().join("outside.jsonl");
    fs::write(&outside, fill(THREE_TURNS, &at, ID)).unwrap();
    let path = rollout(&at, ID);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(outside, path).unwrap();

    assert!(matches!(
        codex().locate(&at, ID),
        Err(ConversationError::NotAFile { .. })
    ));
}

// Turns.

#[test]
fn turns_are_the_users_prompts_in_order_without_the_injected_context() {
    let root = tempfile::tempdir().unwrap();
    let (_, conversation) = parent(root.path(), "mac");

    // Lines 3 and 4 (developer instructions, AGENTS.md and environment
    // context as a user message) are not turns.
    assert_eq!(codex().turns(&conversation).unwrap(), three_turns());
}

#[test]
fn a_prompt_added_to_a_running_turn_is_not_a_turn_of_its_own() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let lines: Vec<&str> = THREE_TURNS.lines().collect();
    // Turn 3's prompt again, as a second UserMessage of the same turn.
    let again = lines[27]
        .replace("01a1e655-b3d2-7b31-83d5-7f9a1b2c4d66", "01a1e655-b3d9-7b31-83d5-7f9a1b2c4dff")
        .replace("Run the tests", "Only the login ones");
    write_conversation(&at, ID, &with_line_after(THREE_TURNS, 28, &again));
    let conversation = codex().locate(&at, ID).unwrap();

    assert_eq!(codex().turns(&conversation).unwrap(), three_turns());
}

#[test]
fn a_line_still_being_written_is_not_a_turn_and_is_not_forked() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let path = write_conversation(&at, ID, THREE_TURNS);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(fill(FOURTH_RUNNING, &at, ID).lines().next().unwrap().as_bytes());
    bytes.truncate(bytes.len() - 20);
    fs::write(path, bytes).unwrap();
    let conversation = codex().locate(&at, ID).unwrap();

    assert_eq!(codex().turns(&conversation).unwrap(), three_turns());
    let fork = codex().fork(&conversation, None, FORK_A, &at).unwrap();
    assert_eq!(records(&fork.files[0].path), lines_of(THREE_TURNS, &at, FORK_A, ALL));
    assert!(fs::read(&fork.files[0].path).unwrap().ends_with(b"\n"));
}

#[test]
fn a_fork_from_now_while_a_turn_runs_ends_after_the_last_finished_turn() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, &format!("{THREE_TURNS}{FOURTH_RUNNING}"));
    let conversation = codex().locate(&at, ID).unwrap();

    // The running turn is listed, but a fork from now ends before its start.
    let turns = codex().turns(&conversation).unwrap();
    assert_eq!(turns.len(), 4);
    assert_eq!(turns[3].first_line, "Deploy it");
    let fork = codex().fork(&conversation, None, FORK_A, &at).unwrap();
    assert_eq!(codex().turns(&fork).unwrap(), three_turns());
    assert_eq!(records(&fork.files[0].path), lines_of(THREE_TURNS, &at, FORK_A, ALL));

    // While the only turn runs (prompted, no task_complete), there is
    // nothing finished to fork.
    let first_only: String = THREE_TURNS.lines().take(9).map(|line| format!("{line}\n")).collect();
    write_conversation(&at, OTHER, &first_only);
    let first_running = codex().locate(&at, OTHER).unwrap();
    assert_eq!(codex().turns(&first_running).unwrap().len(), 1);
    assert!(matches!(
        codex().fork(&first_running, None, FORK_B, &at),
        Err(ConversationError::NothingBefore)
    ));
    assert!(!rollout(&at, FORK_B).exists());
}

// Fork.

#[test]
fn a_fork_before_a_turn_holds_exactly_the_lines_before_its_start() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    for (before, lines, new_id) in [(2_u32, BEFORE_2, FORK_A), (3, BEFORE_3, FORK_B)] {
        let fork = codex().fork(&conversation, Some(before), new_id, &at).unwrap();

        let kept = usize::try_from(before).unwrap() - 1;
        assert_eq!(codex().turns(&fork).unwrap(), three_turns()[..kept].to_vec());
        // The same lines, with the new id where the parent's was.
        assert_eq!(records(&fork.files[0].path), lines_of(THREE_TURNS, &at, new_id, lines), "before {before}");
    }
}

#[test]
fn a_turns_start_is_where_a_fork_before_it_is_cut() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let bytes = fs::read(&conversation.files[0].path).unwrap();

    let starts = codex().turn_starts(&conversation).unwrap();

    assert_eq!(starts.len(), three_turns().len());
    for (turn, lines) in [(2_usize, BEFORE_2), (3, BEFORE_3)] {
        let kept = usize::try_from(starts[turn - 1]).unwrap();
        let before: Vec<Value> = String::from_utf8_lossy(&bytes[..kept])
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(before, lines_of(THREE_TURNS, &at, ID, lines), "turn {turn}");
    }
}

#[test]
fn a_fork_from_now_holds_the_whole_rollout() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    let fork = codex().fork(&conversation, None, FORK_A, &at).unwrap();

    assert_eq!(codex().turns(&fork).unwrap(), three_turns());
    assert_eq!(records(&fork.files[0].path), lines_of(THREE_TURNS, &at, FORK_A, ALL));
}

#[test]
fn a_fork_is_named_by_its_new_id_in_its_file_name_and_its_lines() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    let fork = codex().fork(&conversation, Some(3), FORK_A, &at).unwrap();

    assert_eq!(fork.id, FORK_A);
    assert_eq!(fork.format, Format::CodexRollout);
    assert_eq!(fork.placement, at);
    assert_eq!(
        fork.files,
        vec![ConversationFile {
            path: rollout(&at, FORK_A),
            role: FileRole::Transcript,
        }]
    );
    let text = fs::read_to_string(&fork.files[0].path).unwrap();
    assert!(!text.contains(ID));
    let meta = &records(&fork.files[0].path)[0];
    assert_eq!(meta["payload"]["id"], FORK_A);
    assert_eq!(meta["payload"]["session_id"], FORK_A);
    assert_eq!(codex().locate(&at, FORK_A).unwrap(), fork);
}

#[test]
fn forking_leaves_the_parent_and_the_clis_state_as_they_were() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    write_cli_state(&at);
    let before = snapshot(&at.home);
    let elsewhere = placement(root.path(), "elsewhere");

    codex().fork(&conversation, Some(2), FORK_A, &at).unwrap();
    codex().fork(&conversation, None, FORK_B, &at).unwrap();
    codex().fork(&conversation, Some(3), FORK_C, &elsewhere).unwrap();

    let mut after = snapshot(&at.home);
    for fork in [FORK_A, FORK_B] {
        assert!(after.remove(&rollout(&at, fork)).is_some(), "{fork}");
    }
    assert_eq!(after, before);
    // Nor does a fork elsewhere make any state there.
    assert_eq!(snapshot(&elsewhere.home).into_keys().collect::<Vec<_>>(), vec![rollout(&elsewhere, FORK_C)]);
}

#[test]
fn a_fork_before_the_first_turn_or_past_the_last_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let before = snapshot(&at.home);

    assert!(matches!(
        codex().fork(&conversation, Some(1), FORK_A, &at),
        Err(ConversationError::NothingBefore)
    ));
    assert!(matches!(
        codex().fork(&conversation, Some(4), FORK_A, &at),
        Err(ConversationError::NoSuchTurn { turn: 4, turns: 3 })
    ));
    assert!(matches!(
        codex().fork(&conversation, Some(0), FORK_A, &at),
        Err(ConversationError::NoSuchTurn { .. })
    ));
    assert_eq!(snapshot(&at.home), before);
}

#[test]
fn a_fork_needs_a_full_uuid_for_its_new_id() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let before = snapshot(&at.home);

    for bad in ["fork-1", "../x", "01A1E655-F0A0-7000-8000-00000000000A"] {
        assert!(
            matches!(
                codex().fork(&conversation, Some(2), bad, &at),
                Err(ConversationError::BadId { .. })
            ),
            "{bad:?}"
        );
    }
    assert_eq!(snapshot(&at.home), before);
}

#[test]
fn a_fork_never_overwrites_a_conversation() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let parent_bytes = fs::read(rollout(&at, ID)).unwrap();

    assert!(matches!(
        codex().fork(&conversation, Some(2), ID, &at),
        Err(ConversationError::Exists { .. })
    ));
    assert_eq!(fs::read(rollout(&at, ID)).unwrap(), parent_bytes);

    codex().fork(&conversation, Some(2), FORK_A, &at).unwrap();
    let first = fs::read(rollout(&at, FORK_A)).unwrap();
    assert!(matches!(
        codex().fork(&conversation, None, FORK_A, &at),
        Err(ConversationError::Exists { .. })
    ));
    assert_eq!(fs::read(rollout(&at, FORK_A)).unwrap(), first);
}

#[test]
fn a_fork_into_another_folder_and_codex_home_names_them() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let to = Placement {
        cli_home: Some(root.path().join("mac/codex-home")),
        folder: root.path().join("mac/channels/web/sessions/try-cookies"),
        ..at.clone()
    };

    let fork = codex().fork(&conversation, Some(3), FORK_A, &to).unwrap();

    assert_eq!(fork.files[0].path, rollout(&to, FORK_A));
    assert_eq!(fork.placement, to);
    // As Codex would have written it in that folder and home.
    assert_eq!(records(&fork.files[0].path), lines_of(THREE_TURNS, &to, FORK_A, BEFORE_3));
    let text = fs::read_to_string(&fork.files[0].path).unwrap();
    assert!(!text.contains(&*at.folder.to_string_lossy()));
    assert!(!text.contains(&*codex_dir(&at).to_string_lossy()));
    assert_eq!(codex().turns(&fork).unwrap(), three_turns()[..2].to_vec());
}

// Carry.

#[test]
fn a_carried_conversation_is_found_in_the_new_home_and_folder_with_its_paths_rewritten() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = parent(root.path(), "old");
    let parent_files = snapshot(&from.home);
    let to = new_machine(root.path());

    let stage = root.path().join("stage");
    let packed = codex().pack(&conversation, &stage).unwrap();
    assert_eq!(packed.id, ID);
    assert_eq!(packed.format, Format::CodexRollout);
    let rel = format!("{DAY}/rollout-{STAMP}-{ID}.jsonl");
    let size = fs::metadata(stage.join(&rel)).unwrap().len();
    assert_eq!(
        packed.files,
        vec![PackedFile {
            rel,
            role: FileRole::Transcript,
            size,
        }]
    );
    let placed = codex().place(&packed, &from, &to).unwrap();

    let found = codex().locate(&to, ID).unwrap();
    assert_eq!(found, placed);
    assert_eq!(found.files[0].path, rollout(&to, ID));
    assert_eq!(codex().turns(&found).unwrap(), three_turns());
    // As Codex would have written it there, the id kept.
    assert_eq!(records(&found.files[0].path), lines_of(THREE_TURNS, &to, ID, ALL));
    for old in [
        from.folder.to_string_lossy().into_owned(),
        codex_dir(&from).to_string_lossy().into_owned(),
    ] {
        assert!(!any_file_holds(&codex_dir(&to), &old), "{old} is still named");
    }

    assert_eq!(snapshot(&from.home), parent_files);
}

#[test]
fn a_carry_never_writes_the_clis_state() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = parent(root.path(), "old");
    write_cli_state(&from);
    let to = new_machine(root.path());
    let to_state = write_cli_state(&to);
    let to_before: BTreeMap<PathBuf, Vec<u8>> =
        to_state.iter().map(|path| (path.clone(), fs::read(path).unwrap())).collect();

    let stage = root.path().join("stage");
    let packed = codex().pack(&conversation, &stage).unwrap();
    assert!(sqlite_files(&stage).is_empty());
    codex().place(&packed, &from, &to).unwrap();

    // The same SQLite files, unchanged, and no new ones.
    assert_eq!(sqlite_files(&codex_dir(&to)), to_before.keys().cloned().collect::<Vec<_>>());
    for (path, bytes) in &to_before {
        assert_eq!(&fs::read(path).unwrap(), bytes, "{}", path.display());
    }
    assert!(sqlite_files(&to.home).is_empty());
}

#[test]
fn a_carry_keeps_a_running_turn_for_the_cli() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "old");
    let running = format!("{THREE_TURNS}{FOURTH_RUNNING}");
    write_conversation(&from, ID, &running);
    let conversation = codex().locate(&from, ID).unwrap();
    let to = new_machine(root.path());

    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = codex().place(&packed, &from, &to).unwrap();

    assert_eq!(records(&placed.files[0].path), lines_of(&running, &to, ID, 1..=37));
    assert_eq!(codex().turns(&placed).unwrap().len(), 4);
}

#[test]
fn placing_over_a_conversation_already_there_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = parent(root.path(), "old");
    let to = new_machine(root.path());
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    codex().place(&packed, &from, &to).unwrap();
    let first = snapshot(&codex_dir(&to));

    assert!(matches!(
        codex().place(&packed, &from, &to),
        Err(ConversationError::Exists { .. })
    ));
    assert_eq!(snapshot(&codex_dir(&to)), first);
}

#[test]
fn packing_into_a_stage_that_holds_files_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (_, conversation) = parent(root.path(), "old");
    let stage = root.path().join("stage");
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("left-over"), "x").unwrap();

    assert!(matches!(
        codex().pack(&conversation, &stage),
        Err(ConversationError::Exists { .. })
    ));
}

#[test]
fn a_staged_name_that_leaves_its_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = parent(root.path(), "old");
    let to = new_machine(root.path());
    let packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();

    for bad in ["../escape.jsonl", "/abs.jsonl", "sessions/../../x.jsonl"] {
        let mut tampered = packed.clone();
        tampered.files.push(PackedFile {
            rel: bad.to_owned(),
            role: FileRole::Companion,
            size: 1,
        });
        assert!(
            matches!(codex().place(&tampered, &from, &to), Err(ConversationError::BadPath { .. })),
            "{bad}"
        );
    }
    assert!(snapshot(&root.path().join("new")).is_empty());
}

#[test]
fn a_staged_set_without_its_rollout_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = parent(root.path(), "old");
    let to = new_machine(root.path());
    let mut packed = codex().pack(&conversation, &root.path().join("stage")).unwrap();
    packed.files.clear();

    assert!(matches!(
        codex().place(&packed, &from, &to),
        Err(ConversationError::NotFound { .. })
    ));
    assert!(snapshot(&root.path().join("new")).is_empty());
}

#[test]
fn a_rollout_that_is_not_text_is_refused_not_copied() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "old");
    let path = write_conversation(&at, ID, THREE_TURNS);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(&[0x00, 0xff, b'\n']);
    fs::write(path, bytes).unwrap();
    let conversation = codex().locate(&at, ID).unwrap();
    let stage = root.path().join("stage");

    assert!(matches!(
        codex().pack(&conversation, &stage),
        Err(ConversationError::NotText { .. })
    ));
    assert!(snapshot(&stage).is_empty());
}

// Malformed lines and versions.

#[test]
fn a_malformed_line_is_named_not_panicked_on() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, MALFORMED);
    let conversation = codex().locate(&at, ID).unwrap();
    let before = snapshot(&at.home);

    assert!(matches!(
        codex().turns(&conversation),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert!(matches!(
        codex().check(&conversation, Some(VERSION)),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert!(matches!(
        codex().fork(&conversation, None, FORK_A, &at),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert!(matches!(
        codex().pack(&conversation, &root.path().join("stage")),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert_eq!(snapshot(&at.home), before);
}

#[test]
fn a_line_of_json_that_is_not_a_record_is_malformed() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    for (n, not_a_record) in ["[\"a\",\"list\"]", "{\"no\":\"type\"}", "{\"type\":7}"].into_iter().enumerate() {
        let id = format!("0b0c0d0e-0000-7000-8000-00000000000{n}");
        write_conversation(&at, &id, &with_line_changed(THREE_TURNS, 2, |_| not_a_record.to_owned()));
        let conversation = codex().locate(&at, &id).unwrap();

        assert!(
            matches!(
                codex().turns(&conversation),
                Err(ConversationError::Malformed { line: 2, .. })
            ),
            "{not_a_record}"
        );
    }
}

#[test]
fn a_newer_cli_version_is_known_when_every_line_parses() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    codex().check(&conversation, Some(VERSION)).unwrap();
    codex().check(&conversation, None).unwrap();
    // The launch version doesn't decide by itself.
    codex().check(&conversation, Some("9.0.0")).unwrap();

    // A newer CLI wrote it, adding fields that hold no path.
    let newer = THREE_TURNS
        .replace(&format!("\"cli_version\":\"{VERSION}\""), "\"cli_version\":\"9.0.0\"")
        .replace("\"realtime_active\":false", "\"realtime_active\":false,\"future_meta\":{\"count\":2,\"note\":\"no paths here\"}")
        .replace("\"ordinal\":", "\"future_tone\":\"teal\",\"ordinal\":");
    write_conversation(&at, OTHER, &newer);
    let newer = codex().locate(&at, OTHER).unwrap();
    codex().check(&newer, Some("9.0.0")).unwrap();
    assert_eq!(codex().turns(&newer).unwrap(), three_turns());
}

#[test]
fn a_line_of_a_kind_the_adapter_does_not_know_makes_the_conversation_unknown() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let turn = "01a1e655-add5-79e0-91ea-3d39f626a868";
    // After turn 1's prompt (line 9): records X1 never showed.
    let unknown = [
        r#"{"timestamp":"2026-11-16T21:48:57.416Z","ordinal":9,"type":"compacted","payload":{"message":"summary"}}"#.to_owned(),
        format!(r#"{{"timestamp":"2026-11-16T21:48:57.416Z","ordinal":9,"type":"event_msg","payload":{{"type":"turn_aborted","turn_id":"{turn}","reason":"interrupted"}}}}"#),
        r#"{"timestamp":"2026-11-16T21:48:57.416Z","ordinal":9,"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}"#.to_owned(),
        r#"{"timestamp":"2026-11-16T21:48:57.416Z","ordinal":9,"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}","call_id":"call_1"}}"#.to_owned(),
        r#"{"timestamp":"2026-11-16T21:48:57.416Z","ordinal":9,"type":"response_item","payload":{"type":"reasoning","summary":[]}}"#.to_owned(),
        format!(r#"{{"timestamp":"2026-11-16T21:48:57.416Z","ordinal":9,"type":"event_msg","payload":{{"type":"item_completed","thread_id":"{{id}}","turn_id":"{turn}","item":{{"type":"CommandExecution","id":"cmd_1"}}}}}}"#),
    ];
    let stage = root.path().join("stage");
    for (n, line) in unknown.iter().enumerate() {
        let id = format!("0b0c0d0e-0000-7000-8000-0000000000a{n}");
        write_conversation(&at, &id, &with_line_after(THREE_TURNS, 9, line));
        let conversation = codex().locate(&at, &id).unwrap();
        let before = snapshot(&at.home);

        // The refusal names the version: the launch's when given, else the
        // one that started the rollout.
        match codex().check(&conversation, None) {
            Err(ConversationError::Unsupported {
                format: Format::CodexRollout,
                version,
            }) => assert_eq!(version.as_deref(), Some(VERSION), "{line}"),
            other => panic!("wanted Unsupported for {line}, got {other:?}"),
        }
        match codex().check(&conversation, Some("0.170.0")) {
            Err(ConversationError::Unsupported { version, .. }) => {
                assert_eq!(version.as_deref(), Some("0.170.0"), "{line}");
            }
            other => panic!("wanted Unsupported for {line}, got {other:?}"),
        }
        let unsupported =
            |result: Result<(), ConversationError>| matches!(result, Err(ConversationError::Unsupported { .. }));
        assert!(unsupported(codex().turns(&conversation).map(drop)), "{line}");
        assert!(unsupported(codex().fork(&conversation, None, FORK_A, &at).map(drop)), "{line}");
        assert!(unsupported(codex().fork(&conversation, Some(2), FORK_B, &at).map(drop)), "{line}");
        assert!(unsupported(codex().pack(&conversation, &stage).map(drop)), "{line}");
        assert_eq!(snapshot(&at.home), before, "{line}");
        assert!(snapshot(&stage).is_empty(), "{line}");
    }
}

#[test]
fn a_field_the_adapter_does_not_know_that_holds_a_path_makes_the_conversation_unknown() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    // Line 6 is turn 1's turn_context.
    let changes: [(&str, fn(&str) -> String); 5] = [
        ("top-level folder", |line: &str| line.replacen("\"ordinal\":", "\"future_root\":\"{folder}\",\"ordinal\":", 1)),
        ("payload home path", |line: &str| line.replacen("\"cwd\":", "\"future_dirs\":[\"~/notes\"],\"cwd\":", 1)),
        ("nested absolute path", |line: &str| line.replacen("\"cwd\":", "\"future_meta\":{\"where\":{\"dir\":\"/opt/shared\"}},\"cwd\":", 1)),
        ("folder inside text", |line: &str| line.replacen("\"cwd\":", "\"future_note\":\"see {folder}/web\",\"cwd\":", 1)),
        ("unknown top-level path", |line: &str| line.replacen("\"ordinal\":", "\"future_log\":\"/var/log/codex\",\"ordinal\":", 1)),
    ];
    for (n, (what, change)) in changes.iter().enumerate() {
        let id = format!("0b0c0d0e-0000-7000-8000-0000000000b{n}");
        write_conversation(&at, &id, &with_line_changed(THREE_TURNS, 6, *change));
        let conversation = codex().locate(&at, &id).unwrap();

        assert!(
            matches!(
                codex().check(&conversation, Some(VERSION)),
                Err(ConversationError::Unsupported { .. })
            ),
            "{what}"
        );
        assert!(
            matches!(codex().turns(&conversation), Err(ConversationError::Unsupported { .. })),
            "{what}"
        );
    }
}
