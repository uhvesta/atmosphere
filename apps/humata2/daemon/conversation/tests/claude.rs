//! The Claude Code adapter on hand-written JSONL (CV-1–CV-4, CV-7, FK-3,
//! MV-13): where it finds a conversation, which records are turns of its
//! current path, what a fork before a turn holds, what a carry to another
//! home and folder places, and which records it refuses to read. Every home
//! is a temp folder; the real `~/.claude` is never read.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use humata2_conversation::{
    Adapter, Conversation, ConversationError, ConversationFile, FileRole, Format, PackedFile, Placement, Turn,
    adapter,
};
use serde_json::Value;

/// Three turns, with injected context, tool results and a side conversation
/// between them. `{folder}`, `{project}` and `{id}` are filled per test.
const THREE_TURNS: &str = include_str!("../testdata/claude/three_turns.jsonl");
/// Four lines, the third cut off mid-record.
const MALFORMED: &str = include_str!("../testdata/claude/malformed.jsonl");
/// Two turns on the current path, and between them a prompt and its reply
/// abandoned by a rewind (lines 4 to 7: its snapshot, prompt, reply and a
/// summary of that branch).
const REWOUND: &str = include_str!("../testdata/claude/rewound.jsonl");
/// Two turns with a compaction between them: a boundary record (line 3)
/// pointing back at the first turn's reply, and the compact summary
/// (line 4).
const COMPACTED: &str = include_str!("../testdata/claude/compacted.jsonl");

/// Two turns as Claude Code 2.1.325 writes them (scrubbed): queue entries,
/// attachments between a prompt and its reply (user, attachment, assistant),
/// and the records with no `uuid` (queue, latch, API request log, cost, mode,
/// last prompt). The folder is also inside the attachments and the logged
/// request.
const REAL_SHAPE: &str = include_str!("../testdata/claude/real_shape.jsonl");
/// The lines of [`REAL_SHAPE`] (from 1) a fork before turn 2 holds: every
/// record before the second prompt, but not the queue entries (15, 16) of
/// that prompt.
const REAL_BEFORE_2: &[usize] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14];

/// The parent conversation's id.
const ID: &str = "4f6d2c1e-8a3b-4c5d-9e7f-1a2b3c4d5e6f";
/// The lines of [`THREE_TURNS`] (from 1) a fork before turn 2 holds: the
/// current path before the second prompt and the first prompt's snapshot,
/// without the side conversation written inline (lines 7 and 8).
const BEFORE_2: &[usize] = &[1, 2, 3, 4, 5, 6, 9, 10, 11];
/// The lines of [`THREE_TURNS`] a fork before turn 3 holds.
const BEFORE_3: &[usize] = &[1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];
/// The lines of [`THREE_TURNS`] a fork from now holds: all but the side
/// conversation, the summary of the path's last record included.
const FROM_NOW: &[usize] = &[1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21];
/// The lines of [`REWOUND`] a fork from now holds: the abandoned branch's
/// snapshot, prompt, reply and summary are left out.
const REWOUND_NOW: &[usize] = &[1, 2, 3, 8, 9, 10];
/// The `uuid`s of [`REWOUND`]'s abandoned prompt and its reply.
const ABANDONED: [&str; 2] = [
    "5b1f3e20-0003-4c7a-8d2e-6a9b0c1d0003",
    "5b1f3e20-0004-4c7a-8d2e-6a9b0c1d0004",
];
/// The version every record of the testdata was written by.
const VERSION: &str = "2.1.325";

fn claude() -> &'static dyn Adapter {
    adapter(Format::ClaudeJsonl)
}

/// A machine `name` under `root`: its home, and a session folder in it.
fn placement(root: &Path, name: &str) -> Placement {
    Placement {
        home: root.join(name).join("home"),
        cli_home: None,
        folder: root.join(name).join("channels/web/sessions/fix-login"),
    }
}

/// What Claude Code names the folder's record: every character that isn't an
/// ASCII letter or digit becomes `-`.
fn dashed(folder: &Path) -> String {
    folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Where Claude keeps the conversations of `at`'s folder.
fn project(at: &Placement) -> PathBuf {
    let config = at.cli_home.clone().unwrap_or_else(|| at.home.join(".claude"));
    config.join("projects").join(dashed(&at.folder))
}

fn transcript(at: &Placement, id: &str) -> PathBuf {
    project(at).join(format!("{id}.jsonl"))
}

/// `template` as Claude would have written it for conversation `id` at `at`.
fn fill(template: &str, at: &Placement, id: &str) -> String {
    template
        .replace("{project}", &project(at).to_string_lossy())
        .replace("{folder}", &at.folder.to_string_lossy())
        .replace("{id}", id)
}

/// Writes `template` as conversation `id`'s transcript at `at`.
fn write_conversation(at: &Placement, id: &str, template: &str) -> PathBuf {
    let path = transcript(at, id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, fill(template, at, id)).unwrap();
    path
}

/// Lines `which` (from 1) of `template` as written for `id` at `at`, parsed
/// and without their `sessionId`.
fn lines_of(template: &str, at: &Placement, id: &str, which: &[usize]) -> Vec<Value> {
    let filled = fill(template, at, id);
    let lines: Vec<&str> = filled.lines().collect();
    which
        .iter()
        .map(|line| without_session(&serde_json::from_str(lines[line - 1]).unwrap()))
        .collect()
}

/// The records of the file at `path`, without their `sessionId`.
fn records_without_session(path: &Path) -> Vec<Value> {
    records(path).iter().map(without_session).collect()
}

/// Each complete line of the file at `path`, parsed.
fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// `record` without its `sessionId`.
fn without_session(record: &Value) -> Value {
    let mut record = record.clone();
    if let Some(object) = record.as_object_mut() {
        object.remove("sessionId");
    }
    record
}

/// Every plain file under `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                folders.push(path);
            } else if meta.is_file() {
                found.insert(path.clone(), fs::read(&path).unwrap());
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

/// The three-turn conversation `ID` at machine `name` under `root`, found.
fn parent(root: &Path, name: &str) -> (Placement, Conversation) {
    let at = placement(root, name);
    write_conversation(&at, ID, THREE_TURNS);
    let conversation = claude().locate(&at, ID).unwrap();
    (at, conversation)
}

fn turn(index: u32, first_line: &str, at: &str) -> Turn {
    Turn {
        index,
        first_line: first_line.to_owned(),
        at: Some(at.to_owned()),
    }
}

fn rewound_turns() -> Vec<Turn> {
    vec![
        turn(1, "Fix the login redirect", "2026-11-16T09:01:00.000Z"),
        turn(2, "Use a session token instead", "2026-11-16T09:08:00.000Z"),
    ]
}

fn three_turns() -> Vec<Turn> {
    vec![
        turn(1, "Fix the login redirect", "2026-11-16T09:01:00.000Z"),
        turn(2, "Now add a test for it", "2026-11-16T09:05:00.000Z"),
        turn(3, "Run the tests", "2026-11-16T09:10:00.000Z"),
    ]
}

// What the real CLI writes.

/// A placement whose folder holds a space, an underscore and a dot.
fn awkward(root: &Path) -> Placement {
    Placement {
        home: root.join("mac/home"),
        cli_home: None,
        folder: root.join("mac/my_work dir/fix.login"),
    }
}

/// [`REAL_SHAPE`] written for `ID` at machine `name`, found.
fn real_parent(root: &Path, name: &str) -> (Placement, Conversation) {
    let at = placement(root, name);
    write_conversation(&at, ID, REAL_SHAPE);
    let conversation = claude().locate(&at, ID).unwrap();
    (at, conversation)
}

fn real_turns() -> Vec<Turn> {
    vec![
        turn(1, "Fix the login redirect", "2026-11-16T21:48:56.688Z"),
        turn(2, "Add a test for it", "2026-11-16T21:48:57.314Z"),
    ]
}

#[test]
fn a_conversation_whose_folder_has_a_space_and_an_underscore_is_found() {
    let root = tempfile::tempdir().unwrap();
    let at = awkward(root.path());
    let path = write_conversation(&at, ID, THREE_TURNS);

    let found = claude().locate(&at, ID).unwrap();

    assert_eq!(found.files[0].path, path);
    let name = path.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{name}");
    assert_eq!(claude().turns(&found).unwrap(), three_turns());
}

#[test]
fn a_carry_from_a_folder_with_a_space_and_an_underscore_renames_its_dashed_form() {
    let root = tempfile::tempdir().unwrap();
    let from = awkward(root.path());
    write_conversation(&from, ID, THREE_TURNS);
    let conversation = claude().locate(&from, ID).unwrap();
    let to = new_machine(root.path());

    let packed = claude().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = claude().place(&packed, &from, &to).unwrap();

    assert_eq!(claude().turns(&placed).unwrap(), three_turns());
    assert_eq!(placed.files[0].path, transcript(&to, ID));
    let new_project = project(&to);
    for old in [from.folder.to_string_lossy().into_owned(), dashed(&from.folder)] {
        assert!(!any_file_holds(&new_project, &old), "{old} is still named");
    }
}

#[test]
fn the_real_cli_shape_is_accepted_and_has_its_two_turns() {
    let root = tempfile::tempdir().unwrap();
    let (_, conversation) = real_parent(root.path(), "mac");

    claude().check(&conversation, Some(VERSION)).unwrap();
    assert_eq!(claude().turns(&conversation).unwrap(), real_turns());
}

#[test]
fn a_claude_conversation_has_no_turn_starts_since_its_fork_keeps_only_the_current_path() {
    let root = tempfile::tempdir().unwrap();
    let (_, conversation) = parent(root.path(), "mac");

    let refused = claude().turn_starts(&conversation);

    assert!(matches!(refused, Err(ConversationError::Unsupported { .. })), "{refused:?}");
}

#[test]
fn a_fork_before_turn_2_of_the_real_shape_keeps_the_records_before_it_with_and_without_a_uuid() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = real_parent(root.path(), "mac");

    let fork = claude().fork(&conversation, Some(2), "fork-1", &at).unwrap();

    assert_eq!(
        records_without_session(&fork.files[0].path),
        lines_of(REAL_SHAPE, &at, ID, REAL_BEFORE_2)
    );
    assert_eq!(claude().turns(&fork).unwrap(), real_turns()[..1].to_vec());
    claude().check(&fork, Some(VERSION)).unwrap();
    // The second prompt is neither queued nor logged in the fork.
    let text = fs::read_to_string(&fork.files[0].path).unwrap();
    assert!(!text.contains("Add a test for it"));
}

#[test]
fn a_fork_from_now_of_the_real_shape_keeps_every_record() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = real_parent(root.path(), "mac");

    let fork = claude().fork(&conversation, None, "fork-now", &at).unwrap();

    let all: Vec<usize> = (1..=REAL_SHAPE.lines().count()).collect();
    assert_eq!(records_without_session(&fork.files[0].path), lines_of(REAL_SHAPE, &at, ID, &all));
}

#[test]
fn a_carry_of_the_real_shape_rewrites_the_folder_inside_its_attachments() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = real_parent(root.path(), "old");
    let to = new_machine(root.path());
    let (old, new) = (from.folder.to_string_lossy().into_owned(), to.folder.to_string_lossy().into_owned());

    let packed = claude().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = claude().place(&packed, &from, &to).unwrap();

    claude().check(&placed, Some(VERSION)).unwrap();
    assert_eq!(claude().turns(&placed).unwrap(), real_turns());
    let found = records(&placed.files[0].path);
    assert_eq!(found.len(), REAL_SHAPE.lines().count());
    let environment = found
        .iter()
        .find(|record| record.pointer("/attachment/type") == Some(&Value::from("environment")))
        .unwrap();
    assert_eq!(environment.pointer("/attachment/snapshot/workingDirectory"), Some(&Value::from(new.as_str())));
    let rendered = environment.pointer("/rendered/0/content").and_then(Value::as_str).unwrap();
    assert!(rendered.contains(&new) && !rendered.contains(&old));
    let logged = found
        .iter()
        .find(|record| record.get("type") == Some(&Value::from("api-request-blob")))
        .unwrap();
    let text = logged.pointer("/message/content/0/text").and_then(Value::as_str).unwrap();
    assert!(text.contains(&new) && !text.contains(&old));
    assert!(!any_file_holds(&project(&to), &old));
    assert!(!any_file_holds(&project(&to), &dashed(&from.folder)));
}

// Locate.

#[test]
fn locate_finds_the_transcript_under_the_dashed_folder_in_the_home() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    assert_eq!(conversation.id, ID);
    assert_eq!(conversation.format, Format::ClaudeJsonl);
    assert_eq!(conversation.placement, at);
    assert_eq!(
        conversation.files,
        vec![ConversationFile {
            path: at.home.join(".claude/projects").join(dashed(&at.folder)).join(format!("{ID}.jsonl")),
            role: FileRole::Transcript,
        }]
    );
}

#[test]
fn locate_reads_the_cli_home_as_the_claude_config_dir() {
    let root = tempfile::tempdir().unwrap();
    let in_home = placement(root.path(), "mac");
    write_conversation(&in_home, ID, THREE_TURNS);
    let configured = Placement {
        cli_home: Some(root.path().join("mac/claude-config")),
        ..in_home.clone()
    };

    // The copy under ~/.claude is not where a CLI with its config dir set looks.
    assert!(matches!(
        claude().locate(&configured, ID),
        Err(ConversationError::NotFound { .. })
    ));

    write_conversation(&configured, ID, THREE_TURNS);
    let found = claude().locate(&configured, ID).unwrap();
    let wanted = root
        .path()
        .join("mac/claude-config/projects")
        .join(dashed(&configured.folder))
        .join(format!("{ID}.jsonl"));
    assert_eq!(found.files[0].path, wanted);
    assert_eq!(found.files[0].role, FileRole::Transcript);
}

#[test]
fn locate_lists_the_conversations_companions_and_the_folders_own_files() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, THREE_TURNS);
    let project = project(&at);
    let side = project.join(ID).join("subagents/agent-1.jsonl");
    let memory = project.join("memory/MEMORY.md");
    let other = project.join("0a1b2c3d-other.jsonl");
    for file in [&side, &memory, &other] {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "{\"type\":\"summary\",\"summary\":\"x\"}\n").unwrap();
    }

    let conversation = claude().locate(&at, ID).unwrap();

    assert_eq!(conversation.files[0].role, FileRole::Transcript);
    assert_eq!(conversation.files[0].path, transcript(&at, ID));
    let rest: BTreeMap<PathBuf, FileRole> = conversation.files[1..]
        .iter()
        .map(|file| (file.path.clone(), file.role))
        .collect();
    assert_eq!(
        rest,
        BTreeMap::from([
            (side, FileRole::Companion),
            (memory, FileRole::Project),
        ])
    );
    // Another conversation of the folder is its own, not this one's (MV-16).
    assert!(!conversation.files.iter().any(|file| file.path == other));
}

#[test]
fn locate_refuses_a_missing_conversation_and_an_id_that_is_not_a_name() {
    let root = tempfile::tempdir().unwrap();
    let (at, _) = parent(root.path(), "mac");

    assert!(matches!(
        claude().locate(&at, "11111111-2222-3333-4444-555555555555"),
        Err(ConversationError::NotFound { .. })
    ));
    for bad in ["", ".", "..", "../x", "a/b", "/abs"] {
        assert!(
            matches!(claude().locate(&at, bad), Err(ConversationError::BadId { .. })),
            "{bad:?}"
        );
    }
}

// Turns.

#[test]
fn turns_are_the_users_prompts_in_order() {
    let root = tempfile::tempdir().unwrap();
    let (_, conversation) = parent(root.path(), "mac");

    assert_eq!(claude().turns(&conversation).unwrap(), three_turns());
}

#[test]
fn a_prompt_abandoned_by_a_rewind_is_not_a_turn() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, REWOUND);
    let conversation = claude().locate(&at, ID).unwrap();

    assert_eq!(claude().turns(&conversation).unwrap(), rewound_turns());
}

#[test]
fn turns_before_a_compaction_are_still_turns_and_its_summary_is_not_one() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, COMPACTED);
    let conversation = claude().locate(&at, ID).unwrap();

    assert_eq!(
        claude().turns(&conversation).unwrap(),
        vec![
            turn(1, "Fix the login redirect", "2026-11-16T09:01:00.000Z"),
            turn(2, "Now add a test for it", "2026-11-16T09:05:00.000Z"),
        ]
    );
    let fork = claude().fork(&conversation, Some(2), "fork-1", &at).unwrap();
    assert_eq!(records_without_session(&fork.files[0].path), lines_of(COMPACTED, &at, ID, &[1, 2, 3, 4]));
}

#[test]
fn a_line_still_being_written_is_not_a_turn_and_is_not_forked() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let path = write_conversation(&at, ID, THREE_TURNS);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(br#"{"type":"user","message":{"role":"user","content":"Deploy it"#);
    fs::write(&path, bytes).unwrap();
    let conversation = claude().locate(&at, ID).unwrap();

    assert_eq!(claude().turns(&conversation).unwrap(), three_turns());
    let fork = claude().fork(&conversation, None, "fork-now", &at).unwrap();
    assert_eq!(records_without_session(&fork.files[0].path), lines_of(THREE_TURNS, &at, ID, FROM_NOW));
    assert!(fs::read(&fork.files[0].path).unwrap().ends_with(b"\n"));
}

#[test]
fn a_fork_from_now_while_a_turn_runs_ends_after_the_last_finished_turn() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    // A fourth prompt whose reply is waiting on a tool.
    let running = format!(
        "{}{}\n{}\n",
        THREE_TURNS,
        r#"{"parentUuid":"7c0e5a10-0024-4a8e-9b1d-3f2a6c8d0024","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","type":"user","message":{"role":"user","content":"Deploy it"},"uuid":"7c0e5a10-0004-4a8e-9b1d-3f2a6c8d0004","timestamp":"2026-11-16T09:20:00.000Z"}"#,
        r#"{"parentUuid":"7c0e5a10-0004-4a8e-9b1d-3f2a6c8d0004","isSidechain":false,"userType":"external","cwd":"{folder}","sessionId":"{id}","version":"2.1.325","message":{"id":"msg_04A","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"tool_use","id":"toolu_04A","name":"Bash","input":{"command":"make deploy"}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":812,"output_tokens":64}},"type":"assistant","uuid":"7c0e5a10-0025-4a8e-9b1d-3f2a6c8d0025","timestamp":"2026-11-16T09:20:05.000Z"}"#,
    );
    write_conversation(&at, ID, &running);
    let conversation = claude().locate(&at, ID).unwrap();

    // The running turn is listed, but a fork from now ends before it.
    assert_eq!(claude().turns(&conversation).unwrap().len(), 4);
    let fork = claude().fork(&conversation, None, "fork-now", &at).unwrap();
    assert_eq!(claude().turns(&fork).unwrap(), three_turns());
    assert_eq!(records_without_session(&fork.files[0].path), lines_of(THREE_TURNS, &at, ID, FROM_NOW));

    // While the only turn runs, there is nothing finished to fork.
    let first_only: String = THREE_TURNS.lines().take(4).map(|line| format!("{line}\n")).collect();
    write_conversation(&at, "first-running", &first_only);
    let first_running = claude().locate(&at, "first-running").unwrap();
    assert!(matches!(
        claude().fork(&first_running, None, "fork-none", &at),
        Err(ConversationError::NothingBefore)
    ));
    assert!(!transcript(&at, "fork-none").exists());
}

// Fork.

#[test]
fn a_fork_before_a_turn_holds_exactly_the_turns_before_it() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    for (before, lines) in [(2_u32, BEFORE_2), (3, BEFORE_3)] {
        let new_id = format!("fork-before-{before}");
        let fork = claude().fork(&conversation, Some(before), &new_id, &at).unwrap();

        let kept = usize::try_from(before).unwrap() - 1;
        assert_eq!(claude().turns(&fork).unwrap(), three_turns()[..kept].to_vec());
        assert_eq!(
            records_without_session(&fork.files[0].path),
            lines_of(THREE_TURNS, &at, ID, lines),
            "before {before}"
        );
    }
}

#[test]
fn a_fork_leaves_out_a_branch_abandoned_by_a_rewind() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, REWOUND);
    let conversation = claude().locate(&at, ID).unwrap();

    let now = claude().fork(&conversation, None, "fork-now", &at).unwrap();
    assert_eq!(claude().turns(&now).unwrap(), rewound_turns());
    assert_eq!(records_without_session(&now.files[0].path), lines_of(REWOUND, &at, ID, REWOUND_NOW));

    let before = claude().fork(&conversation, Some(2), "fork-before-2", &at).unwrap();
    assert_eq!(records_without_session(&before.files[0].path), lines_of(REWOUND, &at, ID, &[1, 2, 3]));

    for fork in [&now, &before] {
        let text = fs::read_to_string(&fork.files[0].path).unwrap();
        assert!(!text.contains("Use cookies instead"));
        for uuid in ABANDONED {
            assert!(!text.contains(uuid), "{uuid}");
        }
    }
}

#[test]
fn a_fork_is_named_by_its_new_id_in_its_file_and_its_records() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    let fork = claude().fork(&conversation, Some(3), "fork-1", &at).unwrap();

    assert_eq!(fork.id, "fork-1");
    assert_eq!(fork.format, Format::ClaudeJsonl);
    assert_eq!(
        fork.files,
        vec![ConversationFile {
            path: transcript(&at, "fork-1"),
            role: FileRole::Transcript,
        }]
    );
    // In the testdata every record with a uuid names its conversation, and
    // the snapshots name none.
    for forked in records(&fork.files[0].path) {
        match forked.get("uuid") {
            Some(_) => assert_eq!(forked["sessionId"], "fork-1"),
            None => assert!(forked.get("sessionId").is_none()),
        }
    }
    assert_eq!(claude().locate(&at, "fork-1").unwrap().files[0].path, fork.files[0].path);
}

#[test]
fn forking_leaves_the_parents_files_as_they_were() {
    let root = tempfile::tempdir().unwrap();
    let (at, _) = parent(root.path(), "mac");
    let memory = project(&at).join("memory/MEMORY.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, fill("- Login lives in {folder}/api/src/login.rs\n", &at, ID)).unwrap();
    let conversation = claude().locate(&at, ID).unwrap();
    let before = snapshot(&at.home);

    claude().fork(&conversation, Some(2), "fork-a", &at).unwrap();
    claude().fork(&conversation, None, "fork-b", &at).unwrap();
    claude()
        .fork(&conversation, Some(3), "fork-c", &placement(root.path(), "elsewhere"))
        .unwrap();

    let mut after = snapshot(&at.home);
    for fork in ["fork-a", "fork-b"] {
        assert!(after.remove(&transcript(&at, fork)).is_some(), "{fork}");
    }
    assert_eq!(after, before);
}

#[test]
fn a_fork_before_the_first_turn_or_past_the_last_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let before = snapshot(&at.home);

    assert!(matches!(
        claude().fork(&conversation, Some(1), "fork-1", &at),
        Err(ConversationError::NothingBefore)
    ));
    assert!(matches!(
        claude().fork(&conversation, Some(4), "fork-4", &at),
        Err(ConversationError::NoSuchTurn { turn: 4, turns: 3 })
    ));
    assert!(matches!(
        claude().fork(&conversation, Some(0), "fork-0", &at),
        Err(ConversationError::NoSuchTurn { .. })
    ));
    assert_eq!(snapshot(&at.home), before);
}

#[test]
fn a_fork_from_now_holds_the_whole_current_path() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    let fork = claude().fork(&conversation, None, "fork-now", &at).unwrap();

    assert_eq!(claude().turns(&fork).unwrap(), three_turns());
    assert_eq!(records_without_session(&fork.files[0].path), lines_of(THREE_TURNS, &at, ID, FROM_NOW));
}

#[test]
fn a_fork_never_overwrites_a_conversation() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let parent_bytes = fs::read(transcript(&at, ID)).unwrap();

    assert!(matches!(
        claude().fork(&conversation, Some(2), ID, &at),
        Err(ConversationError::Exists { .. })
    ));
    assert_eq!(fs::read(transcript(&at, ID)).unwrap(), parent_bytes);

    claude().fork(&conversation, Some(2), "fork-1", &at).unwrap();
    let first = fs::read(transcript(&at, "fork-1")).unwrap();
    assert!(matches!(
        claude().fork(&conversation, None, "fork-1", &at),
        Err(ConversationError::Exists { .. })
    ));
    assert_eq!(fs::read(transcript(&at, "fork-1")).unwrap(), first);
}

#[test]
fn a_fork_into_another_folder_names_that_folder() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");
    let to = Placement {
        folder: root.path().join("mac/channels/web/sessions/try-cookies"),
        ..at.clone()
    };

    let fork = claude().fork(&conversation, Some(3), "fork-1", &to).unwrap();

    assert_eq!(fork.files[0].path, transcript(&to, "fork-1"));
    assert_eq!(fork.placement, to);
    for record in records(&fork.files[0].path) {
        if let Some(cwd) = record.get("cwd") {
            assert_eq!(cwd, &Value::from(to.folder.to_string_lossy().into_owned()));
        }
    }
    let text = fs::read_to_string(&fork.files[0].path).unwrap();
    assert!(!text.contains(&*at.folder.to_string_lossy()));
    assert_eq!(claude().turns(&fork).unwrap(), three_turns()[..2].to_vec());
}

// Carry.

/// The parent at machine "old", with a companion file and a memory file
/// that both name its folder.
fn carried_parent(root: &Path) -> (Placement, Conversation) {
    let at = placement(root, "old");
    write_conversation(&at, ID, THREE_TURNS);
    let saved = project(&at).join(ID).join("tool-results/toolu_01A.txt");
    let memory = project(&at).join("memory/MEMORY.md");
    for (file, text) in [
        (&saved, "pub fn login() at {folder}/api/src/login.rs\n"),
        (&memory, "- Login lives in {folder}/api/src/login.rs\n- Notes: {project}/memory/login.md\n"),
    ] {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, fill(text, &at, ID)).unwrap();
    }
    let conversation = claude().locate(&at, ID).unwrap();
    (at, conversation)
}

/// Another machine's home with its Claude config dir set, and its folder.
fn new_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("new/home"),
        cli_home: Some(root.join("new/claude-config")),
        folder: root.join("new/work/web/fix-login"),
    }
}

#[test]
fn a_carried_conversation_is_found_in_the_new_home_and_folder_with_its_paths_rewritten() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = carried_parent(root.path());
    let parent_files = snapshot(&from.home);
    let to = new_machine(root.path());

    let stage = root.path().join("stage");
    let packed = claude().pack(&conversation, &stage).unwrap();
    assert_eq!(packed.id, ID);
    let rel = format!("{ID}.jsonl");
    assert!(packed.files.contains(&PackedFile {
        rel: rel.clone(),
        role: FileRole::Transcript,
        size: fs::metadata(stage.join(&rel)).unwrap().len(),
    }));
    let placed = claude().place(&packed, &from, &to).unwrap();

    let found = claude().locate(&to, ID).unwrap();
    assert_eq!(found, placed);
    assert_eq!(found.files[0].path, transcript(&to, ID));
    let roles: Vec<FileRole> = found.files.iter().map(|file| file.role).collect();
    assert_eq!(roles.iter().filter(|role| **role == FileRole::Companion).count(), 1);
    assert_eq!(roles.iter().filter(|role| **role == FileRole::Project).count(), 1);
    assert_eq!(claude().turns(&found).unwrap(), three_turns());

    for record in records(&found.files[0].path) {
        if let Some(cwd) = record.get("cwd") {
            assert_eq!(cwd, &Value::from(to.folder.to_string_lossy().into_owned()));
        }
        if let Some(session) = record.get("sessionId") {
            assert_eq!(session, ID);
        }
    }
    let new_project = project(&to);
    for old in [
        project(&from).to_string_lossy().into_owned(),
        from.folder.to_string_lossy().into_owned(),
        dashed(&from.folder),
    ] {
        assert!(!any_file_holds(&new_project, &old), "{old} is still named");
    }
    let memory = fs::read_to_string(new_project.join("memory/MEMORY.md")).unwrap();
    assert!(memory.contains(&format!("{}/api/src/login.rs", to.folder.display())));
    assert!(memory.contains(&format!("{}/memory/login.md", new_project.display())));
    let saved = fs::read_to_string(new_project.join(ID).join("tool-results/toolu_01A.txt")).unwrap();
    assert!(saved.contains(&*to.folder.to_string_lossy()));

    assert_eq!(snapshot(&from.home), parent_files);
}

#[test]
fn a_carry_leaves_a_branch_abandoned_by_a_rewind_behind() {
    let root = tempfile::tempdir().unwrap();
    let from = placement(root.path(), "old");
    write_conversation(&from, ID, REWOUND);
    let conversation = claude().locate(&from, ID).unwrap();
    let parent_files = snapshot(&from.home);
    let to = new_machine(root.path());

    let packed = claude().pack(&conversation, &root.path().join("stage")).unwrap();
    let placed = claude().place(&packed, &from, &to).unwrap();

    assert_eq!(claude().turns(&placed).unwrap(), rewound_turns());
    let text = fs::read_to_string(transcript(&to, ID)).unwrap();
    for uuid in ABANDONED {
        assert!(!text.contains(uuid), "{uuid}");
    }
    assert_eq!(records(&transcript(&to, ID)).len(), REWOUND_NOW.len());
    assert_eq!(snapshot(&from.home), parent_files);
}

#[test]
fn placing_over_a_conversation_already_there_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = carried_parent(root.path());
    let to = new_machine(root.path());
    let packed = claude().pack(&conversation, &root.path().join("stage")).unwrap();
    claude().place(&packed, &from, &to).unwrap();
    let first = snapshot(&project(&to));

    assert!(matches!(
        claude().place(&packed, &from, &to),
        Err(ConversationError::Exists { .. })
    ));
    assert_eq!(snapshot(&project(&to)), first);
}

#[test]
fn packing_into_a_stage_that_holds_files_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (_, conversation) = carried_parent(root.path());
    let stage = root.path().join("stage");
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("left-over"), "x").unwrap();

    assert!(matches!(
        claude().pack(&conversation, &stage),
        Err(ConversationError::Exists { .. })
    ));
}

#[test]
fn a_staged_name_that_leaves_its_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (from, conversation) = carried_parent(root.path());
    let to = new_machine(root.path());
    let packed = claude().pack(&conversation, &root.path().join("stage")).unwrap();

    for bad in ["../escape.jsonl", "/abs.jsonl", "memory/../../x.md"] {
        let mut tampered = packed.clone();
        tampered.files.push(PackedFile {
            rel: bad.to_owned(),
            role: FileRole::Project,
            size: 1,
        });
        assert!(
            matches!(
                claude().place(&tampered, &from, &to),
                Err(ConversationError::BadPath { .. })
            ),
            "{bad}"
        );
    }
    assert!(snapshot(&root.path().join("new")).is_empty());
}

#[test]
fn a_file_that_is_not_text_is_refused_not_copied() {
    let root = tempfile::tempdir().unwrap();
    let (at, _) = carried_parent(root.path());
    let blob = project(&at).join(ID).join("tool-results/screenshot.png");
    fs::write(&blob, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff]).unwrap();
    let conversation = claude().locate(&at, ID).unwrap();
    let stage = root.path().join("stage");

    assert!(matches!(
        claude().pack(&conversation, &stage),
        Err(ConversationError::NotText { .. })
    ));
    assert!(snapshot(&stage).is_empty());
}

#[cfg(unix)]
#[test]
fn a_link_among_the_files_is_refused_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let (at, _) = carried_parent(root.path());
    let outside = root.path().join("outside.txt");
    fs::write(&outside, "not the agent's").unwrap();
    std::os::unix::fs::symlink(&outside, project(&at).join("memory/linked.md")).unwrap();
    let conversation = claude().locate(&at, ID).unwrap();
    let stage = root.path().join("stage");

    assert!(matches!(
        claude().pack(&conversation, &stage),
        Err(ConversationError::NotAFile { .. })
    ));
    assert!(snapshot(&stage).is_empty());
}

// Malformed lines and versions.

#[test]
fn a_malformed_line_is_named_not_panicked_on() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    write_conversation(&at, ID, MALFORMED);
    let conversation = claude().locate(&at, ID).unwrap();
    let before = snapshot(&at.home);

    assert!(matches!(
        claude().turns(&conversation),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert!(matches!(
        claude().check(&conversation, Some(VERSION)),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert!(matches!(
        claude().fork(&conversation, None, "fork-1", &at),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert!(matches!(
        claude().pack(&conversation, &root.path().join("stage")),
        Err(ConversationError::Malformed { line: 3, .. })
    ));
    assert_eq!(snapshot(&at.home), before);
}

#[test]
fn a_line_of_json_that_is_not_a_record_is_malformed() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let mut lines: Vec<&str> = THREE_TURNS.lines().collect();
    for not_a_record in ["[\"a\",\"list\"]", "{\"no\":\"type\"}", "{\"type\":7}"] {
        lines[1] = not_a_record;
        let id = format!("c-{}", not_a_record.len());
        write_conversation(&at, &id, &(lines.join("\n") + "\n"));
        let conversation = claude().locate(&at, &id).unwrap();

        assert!(
            matches!(
                claude().turns(&conversation),
                Err(ConversationError::Malformed { line: 2, .. })
            ),
            "{not_a_record}"
        );
    }
}

#[test]
fn a_newer_cli_version_is_known_when_every_record_parses() {
    let root = tempfile::tempdir().unwrap();
    let (at, conversation) = parent(root.path(), "mac");

    claude().check(&conversation, Some(VERSION)).unwrap();
    claude().check(&conversation, None).unwrap();
    // The launch version doesn't decide by itself.
    claude().check(&conversation, Some("9.0.0")).unwrap();

    // A newer CLI wrote it, adding a field that holds no path.
    let newer = THREE_TURNS
        .replace(&format!("\"version\":\"{VERSION}\""), "\"version\":\"9.0.0\"")
        .replace("\"userType\":\"external\"", "\"userType\":\"external\",\"futureTone\":\"teal\",\"futureMeta\":{\"count\":2,\"note\":\"no paths here\"}");
    write_conversation(&at, "newer", &newer);
    let newer = claude().locate(&at, "newer").unwrap();
    claude().check(&newer, Some("9.0.0")).unwrap();
    assert_eq!(claude().turns(&newer).unwrap(), three_turns());

    // Nothing says which CLI wrote it.
    let unversioned = THREE_TURNS.replace(&format!(",\"version\":\"{VERSION}\""), "");
    write_conversation(&at, "unversioned", &unversioned);
    let unversioned = claude().locate(&at, "unversioned").unwrap();
    claude().check(&unversioned, None).unwrap();
}

/// `THREE_TURNS` with `record` added after the first prompt (line 3).
fn with_record_after_the_first_prompt(record: &str) -> String {
    let mut lines: Vec<&str> = THREE_TURNS.lines().collect();
    lines.insert(3, record);
    lines.join("\n") + "\n"
}

#[test]
fn a_record_of_a_kind_the_adapter_does_not_know_makes_the_conversation_unknown() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    let unknown = with_record_after_the_first_prompt(
        r#"{"type":"checkpoint","uuid":"7c0e5a10-0099-4a8e-9b1d-3f2a6c8d0099","parentUuid":"7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001","sessionId":"{id}","version":"9.1.0","timestamp":"2026-11-16T09:01:01.000Z"}"#,
    );
    write_conversation(&at, ID, &unknown);
    let conversation = claude().locate(&at, ID).unwrap();
    let before = snapshot(&at.home);
    let stage = root.path().join("stage");

    match claude().check(&conversation, Some(VERSION)) {
        Err(ConversationError::Unsupported {
            format: Format::ClaudeJsonl,
            version,
        }) => assert_eq!(version.as_deref(), Some("9.1.0")),
        other => panic!("wanted Unsupported, got {other:?}"),
    }
    let unsupported = |result: Result<(), ConversationError>| matches!(result, Err(ConversationError::Unsupported { .. }));
    assert!(unsupported(claude().turns(&conversation).map(drop)));
    assert!(unsupported(claude().fork(&conversation, None, "fork-1", &at).map(drop)));
    assert!(unsupported(claude().fork(&conversation, Some(2), "fork-2", &at).map(drop)));
    assert!(unsupported(claude().pack(&conversation, &stage).map(drop)));
    assert_eq!(snapshot(&at.home), before);
    assert!(snapshot(&stage).is_empty());
}

#[test]
fn a_field_the_adapter_does_not_know_that_holds_a_path_makes_the_conversation_unknown() {
    let root = tempfile::tempdir().unwrap();
    let at = placement(root.path(), "mac");
    for (n, field) in [
        r#""futureRoot":"{folder}""#,
        r#""futureDirs":["~/notes"]"#,
        r#""futureMeta":{"where":{"dir":"/opt/shared"}}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let record = format!(
            r#"{{"parentUuid":"7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001","isSidechain":false,"cwd":"{{folder}}","sessionId":"{{id}}","version":"2.1.325",{field},"type":"system","subtype":"informational","content":"Note","uuid":"7c0e5a10-0098-4a8e-9b1d-3f2a6c8d0098","timestamp":"2026-11-16T09:01:01.000Z"}}"#
        );
        let id = format!("c-{n}");
        write_conversation(&at, &id, &with_record_after_the_first_prompt(&record));
        let conversation = claude().locate(&at, &id).unwrap();

        assert!(
            matches!(
                claude().check(&conversation, Some(VERSION)),
                Err(ConversationError::Unsupported { .. })
            ),
            "{field}"
        );
        assert!(
            matches!(claude().turns(&conversation), Err(ConversationError::Unsupported { .. })),
            "{field}"
        );
    }
}
