//! The Copilot adapter on conversations the real CLI wrote with features the
//! first run did not use (ROAM round 3, GitHub Copilot CLI 1.0.113-1, from
//! `fixtures/copilot_roam2.py`): tool calls and a subagent, `/compact`, an
//! interactive process with `/rewind`, a multi-line name, and a branch or a
//! detached head; and from a signed-in run of the same CLI, the `model.*` and
//! `instruction.update_delivery` records, a background shell's
//! `system.notification`, and the summary's `mc_*` keys. Each file under `testdata/copilot/1.0.113-1/` is the
//! CLI's own, with the temp folder, Copilot dir and id replaced by
//! `{folder}`, `{copilot}` and `{id}`. Every home is a temp folder; the real
//! `~/.copilot` is never read.

use std::fs;
use std::path::{Path, PathBuf};

use humata2_conversation::{
    Adapter, Conversation, ConversationError, Format, GitHead, Placement, Turn, adapter,
};
use serde_json::Value;

/// A conversation with a `view`, a `bash` and a `task` (a subagent) turn.
const TOOLS: &str = include_str!("../testdata/copilot/1.0.113-1/tools.jsonl");
const TOOLS_SUMMARY: &str = include_str!("../testdata/copilot/1.0.113-1/tools.yaml");
/// A conversation whose history was compacted between its second and third turn.
const COMPACT: &str = include_str!("../testdata/copilot/1.0.113-1/compact.jsonl");
const COMPACT_SUMMARY: &str = include_str!("../testdata/copilot/1.0.113-1/compact.yaml");
const COMPACT_CHECKPOINT: &str = include_str!("../testdata/copilot/1.0.113-1/compact_checkpoint.md");
const COMPACT_INDEX: &str = include_str!("../testdata/copilot/1.0.113-1/compact_index.md");
/// An interactive process: three prompts without a restart, the last one rewound, then a resume.
const INTERACTIVE: &str = include_str!("../testdata/copilot/1.0.113-1/interactive.jsonl");
/// A name of two lines, as a block scalar.
const BLOCK_NAME: &str = include_str!("../testdata/copilot/1.0.113-1/block_name.yaml");
/// A session started on a detached head.
const DETACHED: &str = include_str!("../testdata/copilot/1.0.113-1/detached.jsonl");
const DETACHED_SUMMARY: &str = include_str!("../testdata/copilot/1.0.113-1/detached.yaml");
/// Two prompts from a signed-in 1.0.113-1: an `instruction.update_delivery` in the first turn, the
/// `model.*` records of a side model call in the second, and a summary with the `mc_*` keys and the
/// `mc_pending_repository_warning` mapping. Long texts scrubbed, shapes kept.
const MODEL_RECORDS: &str = include_str!("../testdata/copilot/1.0.113-1/model_records.jsonl");
const MODEL_RECORDS_SUMMARY: &str = include_str!("../testdata/copilot/1.0.113-1/model_records.yaml");
/// Two prompts from a signed-in 1.0.113-1 run whose first turn started a background shell: its
/// `system.notification` (`shell_completed`) and the reply it set off end that turn. Some tool rounds
/// and the run's first prompt left out, long texts scrubbed, shapes kept.
const NOTIFICATION: &str = include_str!("../testdata/copilot/1.0.113-1/notification.jsonl");
const NOTIFICATION_SUMMARY: &str = include_str!("../testdata/copilot/1.0.113-1/notification.yaml");

const ID: &str = "6c69568c-ac29-4516-a109-147d96a18dc9";
const FORK_A: &str = "6c69568c-f0a0-4000-8000-00000000000a";
/// The checkpoint file's name in the compacted conversation.
const CHECKPOINT: &str = "checkpoints/001-session-6c69568c.md";

fn copilot() -> &'static dyn Adapter {
    adapter(Format::CopilotSessionState)
}

fn placement(root: &Path, name: &str) -> Placement {
    Placement {
        home: root.join(name).join("home"),
        cli_home: None,
        folder: root.join(name).join("work dir/proj.one"),
    }
}

fn new_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("new/home"),
        cli_home: Some(root.join("new/copilot-home")),
        folder: root.join("new/work/web"),
    }
}

fn copilot_dir(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".copilot"))
}

fn session_dir(at: &Placement, id: &str) -> PathBuf {
    copilot_dir(at).join("session-state").join(id)
}

fn fill(template: &str, at: &Placement, id: &str) -> String {
    template
        .replace("{copilot}", &copilot_dir(at).to_string_lossy())
        .replace("{folder}", &at.folder.to_string_lossy())
        .replace("{id}", id)
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Conversation `ID` at `at`: `events` and `summary` as the CLI wrote them, and `extra` files beside them.
fn session(at: &Placement, events: &str, summary: &str, extra: &[(&str, &[u8])]) -> Conversation {
    let dir = session_dir(at, ID);
    write(&dir.join("events.jsonl"), fill(events, at, ID));
    write(&dir.join("workspace.yaml"), fill(summary, at, ID));
    for (rel, bytes) in extra {
        write(&dir.join(rel), bytes);
    }
    copilot().locate(at, ID).unwrap()
}

fn records_of(text: &str) -> Vec<Value> {
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn records(path: &Path) -> Vec<Value> {
    records_of(&fs::read_to_string(path).unwrap())
}

fn kinds(records: &[Value]) -> Vec<&str> {
    records.iter().filter_map(|record| record["type"].as_str()).collect()
}

/// The prompts the person typed, in order.
fn typed(records: &[Value]) -> Vec<&str> {
    records
        .iter()
        .filter(|record| record["type"] == "user.message" && record.pointer("/data/source").is_none())
        .filter_map(|record| record.pointer("/data/content").and_then(Value::as_str))
        .collect()
}

fn first_lines(turns: &[Turn]) -> Vec<&str> {
    turns.iter().map(|turn| turn.first_line.as_str()).collect()
}

/// The text of the first `lines` lines of `text`, each with its newline.
fn head(text: &str, lines: usize) -> String {
    text.split_inclusive('\n').take(lines).collect()
}

/// The number of the line (from 1) of the first record after line `after` that `is` accepts.
fn line_where(text: &str, after: usize, is: impl Fn(&Value) -> bool) -> usize {
    records_of(text)
        .iter()
        .enumerate()
        .skip(after)
        .find(|(_, record)| is(record))
        .map(|(index, _)| index + 1)
        .unwrap()
}

fn is_typed_prompt(record: &Value) -> bool {
    record["type"] == "user.message" && record.pointer("/data/source").is_none()
}

fn holds_in_any_file(dir: &Path, needle: &str) -> bool {
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if String::from_utf8_lossy(&fs::read(&path).unwrap()).contains(needle) {
                return true;
            }
        }
    }
    false
}

#[test]
fn a_subagents_prompt_is_not_a_turn_and_the_tool_turns_are_listed_by_their_prompts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, TOOLS, TOOLS_SUMMARY, &[]);

    let turns = copilot().turns(&found).unwrap();

    assert_eq!(turns.len(), 3, "{:?}", first_lines(&turns));
    for (turn, word) in turns.iter().zip(["SCENARIO_VIEW", "SCENARIO_BASH", "SCENARIO_SUB"]) {
        assert!(turn.first_line.contains(word), "{turn:?}");
    }
    assert!(turns.iter().all(|turn| !turn.first_line.contains("SUBAGENT_WORK_MARK")));
    copilot().check(&found, Some("1.0.112-0")).unwrap();
}

#[test]
fn a_fork_before_a_tool_turn_keeps_the_tool_records_of_the_turns_before_it_under_the_new_id() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, TOOLS, TOOLS_SUMMARY, &[]);

    let fork = copilot().fork(&found, Some(3), FORK_A, &at).unwrap();

    let kept = records(&fork.files[0].path);
    let kinds = kinds(&kept);
    assert!(kinds.contains(&"tool.execution_start") && kinds.contains(&"tool.execution_complete"));
    assert!(!kinds.iter().any(|kind| kind.starts_with("subagent.")), "the subagent turn is after the cut");
    assert_eq!(typed(&kept).len(), 2);
    let text = fs::read_to_string(&fork.files[0].path).unwrap();
    assert!(!text.contains(ID), "the parent's id is gone");
    assert!(text.contains(FORK_A));
    assert_eq!(copilot().turns(&fork).unwrap().len(), 2);
    copilot().check(&fork, Some("1.0.112-0")).unwrap();
}

#[test]
fn a_fork_from_now_holds_every_line_of_a_conversation_whose_last_turn_ended_in_an_answer() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, TOOLS, TOOLS_SUMMARY, &[]);

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    let all = fill(TOOLS, &at, FORK_A);
    assert_eq!(records(&fork.files[0].path), records_of(&all));
}

#[test]
fn a_turn_stopped_after_its_tool_ran_is_still_running_so_a_fork_from_now_leaves_it_out() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let text = fill(TOOLS, &at, ID);
    // Turn 2 (the bash prompt), cut right after the end that follows its tool: the model's
    // answer that uses the tool's output is not there yet.
    let second = line_where(&text, 0, |record| is_typed_prompt(record) && record["data"]["content"].as_str().is_some_and(|c| c.contains("SCENARIO_BASH")));
    let tool_end = line_where(&text, second, |record| record["type"] == "assistant.turn_end" && record.pointer("/data/parentToolCallId").is_none());
    let stopped = head(&text, tool_end);
    let found = session(&at, &stopped, TOOLS_SUMMARY, &[]);
    assert_eq!(copilot().turns(&found).unwrap().len(), 2);

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    let kept = records(&fork.files[0].path);
    assert_eq!(typed(&kept).len(), 1, "only the finished first turn");
    assert!(kept.len() < second, "cut before the second prompt");
}

#[test]
fn a_turn_stopped_after_its_subagent_finished_is_still_running() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let text = fill(TOOLS, &at, ID);
    let sub_end = line_where(&text, 0, |record| record["type"] == "assistant.turn_end" && record.pointer("/data/parentToolCallId").is_some());
    let found = session(&at, &head(&text, sub_end), TOOLS_SUMMARY, &[]);
    assert_eq!(copilot().turns(&found).unwrap().len(), 3);

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    assert_eq!(typed(&records(&fork.files[0].path)).len(), 2, "turns 1 and 2 only");
}

#[test]
fn a_carry_of_a_tool_conversation_names_the_new_folder_and_copilot_dir_and_not_the_old() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, TOOLS, TOOLS_SUMMARY, &[]);
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);

    let placed = copilot().place(&packed, &at, &to).unwrap();

    assert_eq!(records(&placed.files[0].path), records_of(&fill(TOOLS, &to, ID)));
    for old in [at.folder.to_string_lossy(), copilot_dir(&at).to_string_lossy()] {
        assert!(!holds_in_any_file(&copilot_dir(&to), &old), "{old}");
    }
    let again = copilot().locate(&to, ID).unwrap();
    assert_eq!(copilot().turns(&again).unwrap().len(), 3);
}

#[test]
fn a_compaction_adds_no_turn_and_a_fork_after_it_keeps_its_records_and_one_before_it_has_none() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, COMPACT, COMPACT_SUMMARY, &[(CHECKPOINT, COMPACT_CHECKPOINT.as_bytes())]);
    assert_eq!(copilot().turns(&found).unwrap().len(), 3);
    copilot().check(&found, Some("1.0.112-0")).unwrap();

    let after = copilot().fork(&found, Some(3), FORK_A, &at).unwrap();
    let before = copilot()
        .fork(&found, Some(2), "6c69568c-f0b0-4000-8000-00000000000b", &at)
        .unwrap();

    let kept = records(&after.files[0].path);
    assert!(kinds(&kept).contains(&"session.compaction_complete"));
    assert_eq!(typed(&kept).len(), 2);
    let earlier = records(&before.files[0].path);
    assert!(!kinds(&earlier).iter().any(|kind| kind.contains("compaction")));
    assert_eq!(typed(&earlier).len(), 1);
}

#[test]
fn a_carry_of_a_compacted_conversation_takes_its_checkpoint_and_its_sqlite_file_as_they_are() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let mut db = b"SQLite format 3\0".to_vec();
    db.extend((0..=255_u8).cycle().take(5000));
    let found = session(
        &at,
        COMPACT,
        COMPACT_SUMMARY,
        &[
            (CHECKPOINT, COMPACT_CHECKPOINT.as_bytes()),
            ("checkpoints/index.md", COMPACT_INDEX.as_bytes()),
            ("session.db", db.as_slice()),
        ],
    );
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);

    let placed = copilot().place(&packed, &at, &to).unwrap();

    let dir = session_dir(&to, ID);
    assert_eq!(fs::read(dir.join("session.db")).unwrap(), db);
    assert_eq!(fs::read(dir.join(CHECKPOINT)).unwrap(), COMPACT_CHECKPOINT.as_bytes());
    assert_eq!(placed.files.len(), packed.files.len());
    assert!(!holds_in_any_file(&dir, &at.folder.to_string_lossy()));
    let again = copilot().locate(&to, ID).unwrap();
    assert_eq!(copilot().turns(&again).unwrap().len(), 3);
}

#[test]
fn a_binary_file_that_is_not_sqlite_or_that_names_the_folder_stops_the_carry_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let mut names = b"SQLite format 3\0".to_vec();
    names.extend(at.folder.to_string_lossy().as_bytes());
    for (n, bytes) in [("other", vec![0_u8, 159, 146, 150]), ("naming", names)] {
        let found = session(&at, COMPACT, COMPACT_SUMMARY, &[("session.db", bytes.as_slice())]);

        let refused = copilot().pack(&found, &root.join(format!("stage-{n}"))).unwrap_err();

        assert!(matches!(refused, ConversationError::NotText { .. }), "{n}: {refused:?}");
        assert!(!root.join(format!("stage-{n}")).exists() || fs::read_dir(root.join(format!("stage-{n}"))).unwrap().next().is_none());
        fs::remove_dir_all(session_dir(&at, ID)).unwrap();
    }
}

#[test]
fn an_interactive_process_with_a_rewind_lists_the_turns_that_are_left_and_forks_between_prompts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, INTERACTIVE, TOOLS_SUMMARY, &[]);
    let all = records_of(&fill(INTERACTIVE, &at, ID));

    let turns = copilot().turns(&found).unwrap();

    assert_eq!(turns.len(), typed(&all).len());
    assert!(!turns.is_empty());
    assert!(kinds(&all).contains(&"session.info"));
    copilot().check(&found, Some("1.0.112-0")).unwrap();
    // The second prompt came from the same process as the first: no shutdown before it.
    let fork = copilot().fork(&found, Some(2), FORK_A, &at).unwrap();
    let kept = records(&fork.files[0].path);
    assert_eq!(typed(&kept).len(), 1);
    assert_eq!(kinds(&kept).last(), Some(&"assistant.turn_end"));
}

#[test]
fn a_multi_line_name_in_the_summary_is_known_and_a_stray_line_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, TOOLS, BLOCK_NAME, &[]);
    copilot().check(&found, Some("1.0.112-0")).unwrap();

    let strays = [
        format!("{BLOCK_NAME}not a key\n"),
        BLOCK_NAME.replacen("name: |-", "name: single", 1),
        BLOCK_NAME.replacen("name: |-\n", "name: |-\nnot indented\n", 1),
    ];
    for (n, summary) in strays.iter().enumerate() {
        fs::remove_dir_all(session_dir(&at, ID)).unwrap();
        let found = session(&at, TOOLS, summary, &[]);

        let refused = copilot().check(&found, Some("1.0.112-0")).unwrap_err();

        assert!(matches!(refused, ConversationError::Unsupported { .. }), "{n}: {refused:?}");
    }
}

fn branch_of(dir: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let start = records(&dir.join("events.jsonl"))
        .into_iter()
        .find(|record| record["type"] == "session.start")
        .unwrap();
    let in_start = |key: &str| start.pointer(&format!("/data/context/{key}")).and_then(Value::as_str).map(str::to_owned);
    let summary = fs::read_to_string(dir.join("workspace.yaml"))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("branch: ").map(str::to_owned));
    (in_start("branch"), summary, in_start("headCommit"))
}

fn carry_onto(root: &Path, head: &GitHead) -> (Option<String>, Option<String>, Option<String>) {
    let at = placement(root, "old");
    let found = session(&at, DETACHED, DETACHED_SUMMARY, &[]);
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(root);
    copilot().place_on_head(&packed, &at, &to, Some(head)).unwrap();
    branch_of(&session_dir(&to, ID))
}

#[test]
fn a_carry_onto_a_detached_head_names_it_as_the_cli_does_and_its_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let head = GitHead { branch: None, sha: sha.to_owned() };

    let (in_start, in_summary, commit) = carry_onto(&root, &head);

    assert_eq!(in_start.as_deref(), Some("detached@0123456"));
    assert_eq!(in_summary.as_deref(), Some("detached@0123456"));
    assert_eq!(commit.as_deref(), Some(sha));
}

#[test]
fn a_carry_onto_a_branch_writes_it_plain_or_single_quoted_so_that_yaml_reads_it_back_the_same() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let cases = [
        ("feat/x", "feat/x"),
        ("humata/fix-login", "humata/fix-login"),
        ("true", "'true'"),
        ("null", "'null'"),
        ("123", "'123'"),
        ("1e3", "'1e3'"),
        ("@at#x", "'@at#x'"),
        ("{x}", "'{x}'"),
        ("x'y", "x'y"),
        ("a#b", "a#b"),
        ("'q", "'''q'"),
    ];
    for (n, (branch, written)) in cases.iter().enumerate() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let head = GitHead { branch: Some((*branch).to_owned()), sha: sha.to_owned() };

        let (in_start, in_summary, commit) = carry_onto(&root, &head);

        assert_eq!(in_start.as_deref(), Some(*branch), "{n}");
        assert_eq!(in_summary.as_deref(), Some(*written), "{n}");
        assert_eq!(commit.as_deref(), Some(sha), "{n}");
    }
}

#[test]
fn a_conversation_with_model_and_instruction_delivery_records_lists_its_prompts_and_is_known() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, MODEL_RECORDS, MODEL_RECORDS_SUMMARY, &[]);

    copilot().check(&found, Some("1.0.113-1")).unwrap();
    let turns = copilot().turns(&found).unwrap();

    assert_eq!(turns.len(), 2, "{:?}", first_lines(&turns));
    assert!(turns[0].first_line.contains("APPLE"), "{:?}", turns[0]);
    assert!(turns[1].first_line.contains("PEAR"), "{:?}", turns[1]);
}

#[test]
fn a_fork_before_the_second_turn_keeps_the_first_turns_records_and_none_of_the_model_records_after_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, MODEL_RECORDS, MODEL_RECORDS_SUMMARY, &[]);

    let fork = copilot().fork(&found, Some(2), FORK_A, &at).unwrap();

    let kept = records(&fork.files[0].path);
    assert_eq!(typed(&kept), ["Remember the word APPLE. Reply only with OK."]);
    assert!(kinds(&kept).contains(&"instruction.update_delivery"));
    assert!(!kinds(&kept).iter().any(|kind| kind.starts_with("model.")), "the model records are in turn 2");
    let text = fs::read_to_string(&fork.files[0].path).unwrap();
    assert!(!text.contains(ID), "the parent's id is gone");
    assert!(text.contains(FORK_A));
    assert_eq!(copilot().turns(&fork).unwrap().len(), 1);
}

#[test]
fn a_fork_from_now_of_a_conversation_with_model_records_holds_every_line_under_the_new_id() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, MODEL_RECORDS, MODEL_RECORDS_SUMMARY, &[]);

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    assert_eq!(records(&fork.files[0].path), records_of(&fill(MODEL_RECORDS, &at, FORK_A)));
    assert_eq!(copilot().turns(&fork).unwrap().len(), 2);
}

#[test]
fn a_carry_of_a_conversation_with_model_records_and_mc_keys_names_the_new_folder_and_not_the_old() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, MODEL_RECORDS, MODEL_RECORDS_SUMMARY, &[]);
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);

    let placed = copilot().place(&packed, &at, &to).unwrap();

    assert_eq!(records(&placed.files[0].path), records_of(&fill(MODEL_RECORDS, &to, ID)));
    for old in [at.folder.to_string_lossy(), copilot_dir(&at).to_string_lossy()] {
        assert!(!holds_in_any_file(&copilot_dir(&to), &old), "{old}");
    }
    let again = copilot().locate(&to, ID).unwrap();
    copilot().check(&again, Some("1.0.113-1")).unwrap();
    assert_eq!(copilot().turns(&again).unwrap().len(), 2);
}

#[test]
fn a_summary_mapping_with_a_key_the_cli_did_not_write_or_a_deeper_line_is_not_known() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let warning = "mc_pending_repository_warning:\n";
    let strays = [
        MODEL_RECORDS_SUMMARY.replacen(warning, &format!("{warning}  stray: x\n"), 1),
        MODEL_RECORDS_SUMMARY.replacen(warning, &format!("{warning}  message:\n    deeper: x\n"), 1),
        MODEL_RECORDS_SUMMARY.replacen("name: Remember APPLE Word\n", "name: Remember APPLE Word\n  indented: x\n", 1),
    ];
    for (n, summary) in strays.iter().enumerate() {
        assert_ne!(summary, MODEL_RECORDS_SUMMARY, "{n}: the stray line went in");
        let found = session(&at, MODEL_RECORDS, summary, &[]);

        let refused = copilot().check(&found, Some("1.0.113-1")).unwrap_err();

        assert!(matches!(refused, ConversationError::Unsupported { .. }), "{n}: {refused:?}");
        fs::remove_dir_all(session_dir(&at, ID)).unwrap();
    }
}

#[test]
fn a_tool_call_that_names_the_files_it_wrote_is_known_and_a_carry_moves_those_paths_to_the_new_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    // As a signed-in 1.0.113-1 writes it after a tool created a file in the folder.
    let edited: String = TOOLS
        .lines()
        .map(|line| {
            let mut record: Value = serde_json::from_str(line).unwrap();
            if record["type"] == "tool.execution_complete" {
                record["data"]["fileEdits"] = serde_json::json!([{ "path": "{folder}/note.txt", "kind": "create" }]);
            }
            format!("{record}\n")
        })
        .collect();
    let found = session(&at, &edited, TOOLS_SUMMARY, &[]);
    copilot().check(&found, Some("1.0.113-1")).unwrap();
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);

    let placed = copilot().place(&packed, &at, &to).unwrap();

    let new_path = to.folder.join("note.txt");
    let edits: Vec<Value> = records(&placed.files[0].path)
        .into_iter()
        .filter(|record| record["type"] == "tool.execution_complete")
        .map(|record| record["data"]["fileEdits"][0]["path"].clone())
        .collect();
    assert!(!edits.is_empty());
    assert!(edits.iter().all(|path| path.as_str() == Some(new_path.to_string_lossy().as_ref())), "{edits:?}");
}

/// `NOTIFICATION` with each `system.notification` changed by `change`.
fn with_notification(change: impl Fn(&mut Value)) -> String {
    NOTIFICATION
        .lines()
        .map(|line| {
            let mut record: Value = serde_json::from_str(line).unwrap();
            if record["type"] == "system.notification" {
                change(&mut record);
            }
            format!("{record}\n")
        })
        .collect()
}

/// The line number of the second prompt the person typed in `NOTIFICATION`.
fn second_prompt() -> usize {
    line_where(NOTIFICATION, line_where(NOTIFICATION, 0, is_typed_prompt), is_typed_prompt)
}

#[test]
fn a_background_shells_notification_is_no_turn_and_its_conversation_lists_its_two_prompts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, NOTIFICATION, NOTIFICATION_SUMMARY, &[]);

    copilot().check(&found, Some("1.0.113-1")).unwrap();
    let turns = copilot().turns(&found).unwrap();

    assert_eq!(turns.len(), 2, "{:?}", first_lines(&turns));
    assert!(turns[0].first_line.contains("example-org/example-two "), "{:?}", turns[0]);
    assert!(turns[1].first_line.contains("example-org/example-three "), "{:?}", turns[1]);
}

#[test]
fn a_fork_before_the_second_turn_keeps_the_shells_notification_and_the_reply_it_set_off() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let found = session(&at, NOTIFICATION, NOTIFICATION_SUMMARY, &[]);

    let fork = copilot().fork(&found, Some(2), FORK_A, &at).unwrap();

    let kept = records(&fork.files[0].path);
    assert_eq!(kept, records_of(&fill(&head(NOTIFICATION, second_prompt() - 1), &at, FORK_A)));
    let kinds = kinds(&kept);
    let notification = kinds.iter().position(|kind| *kind == "system.notification").unwrap();
    assert_eq!(kinds.last(), Some(&"hook.end"));
    assert!(kinds[notification..].contains(&"assistant.turn_end"), "its reply ends before the cut");
    assert!(!fs::read_to_string(&fork.files[0].path).unwrap().contains(ID));
    assert_eq!(copilot().turns(&fork).unwrap().len(), 1);
}

#[test]
fn a_carry_of_a_notification_that_quotes_the_folder_leaves_no_old_path_and_the_parent_as_it_was() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let quoted = with_notification(|record| {
        record["data"]["kind"]["description"] = Value::from("List {folder}/src");
        record["data"]["content"] = Value::from(
            "<system_notification>\nShell command \"List {folder}/src\" (shellId: 6) has completed successfully.\n</system_notification>",
        );
    });
    let found = session(&at, &quoted, NOTIFICATION_SUMMARY, &[]);
    let parent = fs::read(&found.files[0].path).unwrap();
    let packed = copilot().pack(&found, &root.join("stage")).unwrap();
    let to = new_machine(&root);

    let placed = copilot().place(&packed, &at, &to).unwrap();

    assert_eq!(records(&placed.files[0].path), records_of(&fill(&quoted, &to, ID)));
    for old in [at.folder.to_string_lossy(), copilot_dir(&at).to_string_lossy()] {
        assert!(!holds_in_any_file(&copilot_dir(&to), &old), "{old}");
    }
    let again = copilot().locate(&to, ID).unwrap();
    copilot().check(&again, Some("1.0.113-1")).unwrap();
    assert_eq!(copilot().turns(&again).unwrap().len(), 2);
    assert_eq!(fs::read(&found.files[0].path).unwrap(), parent, "the parent is unchanged");
}

#[test]
fn a_notification_with_a_path_field_humata_does_not_know_or_of_another_kind_is_not_known() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    let strays = [
        with_notification(|record| record["data"]["logPath"] = Value::from("{folder}/shell-6.log")),
        with_notification(|record| record["data"]["kind"]["sourcePath"] = Value::from("{folder}/AGENTS.md")),
        with_notification(|record| record["data"]["kind"]["type"] = Value::from("instruction_discovered")),
    ];
    for (n, events) in strays.iter().enumerate() {
        assert_ne!(events, NOTIFICATION, "{n}: the change went in");
        let found = session(&at, events, NOTIFICATION_SUMMARY, &[]);

        let refused = copilot().check(&found, Some("1.0.113-1")).unwrap_err();

        assert!(matches!(refused, ConversationError::Unsupported { .. }), "{n}: {refused:?}");
        fs::remove_dir_all(session_dir(&at, ID)).unwrap();
    }
}

#[test]
fn a_fork_from_now_while_a_notifications_reply_is_running_leaves_out_the_notification_and_the_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = placement(&root, "old");
    // The second turn's first tool round, then the first turn's notification and the start of its
    // reply (a reply asking for `read_bash`, and the call) again, as a second shell's would come.
    let first_end = line_where(NOTIFICATION, second_prompt(), |record| record["type"] == "assistant.turn_end");
    let notification = line_where(NOTIFICATION, 0, |record| record["type"] == "system.notification");
    let mut events = head(NOTIFICATION, first_end);
    for (n, line) in NOTIFICATION.lines().enumerate().skip(notification - 1).take(3) {
        let mut record: Value = serde_json::from_str(line).unwrap();
        record["id"] = Value::from(format!("0b5e0000-0000-4000-8000-{n:012}"));
        events.push_str(&format!("{record}\n"));
    }
    let found = session(&at, &events, NOTIFICATION_SUMMARY, &[]);
    assert_eq!(copilot().turns(&found).unwrap().len(), 2);

    let fork = copilot().fork(&found, None, FORK_A, &at).unwrap();

    let kept = records(&fork.files[0].path);
    assert_eq!(kept, records_of(&fill(&head(NOTIFICATION, second_prompt() - 1), &at, FORK_A)));
    assert_eq!(kinds(&kept).iter().filter(|kind| **kind == "system.notification").count(), 1);
    assert_eq!(copilot().turns(&fork).unwrap().len(), 1);
}
