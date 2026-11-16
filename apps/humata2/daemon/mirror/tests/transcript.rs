//! Reading an agent's conversation file as entries, from an offset.

mod support;

use std::fs;

use humata2_mirror::{Entry, MirrorError, OwnTools, Role, read_from};
use serde_json::json;
use support::*;

fn entry(role: Role, text: &str, at: &str, new_turn: bool) -> Entry {
    Entry {
        role,
        text: text.to_owned(),
        at: Some(at.to_owned()),
        new_turn,
    }
}

fn size(file: &std::path::Path) -> u64 {
    fs::metadata(file).unwrap().len()
}

#[test]
fn a_prompt_and_its_answer_come_back_in_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("Why does the import fail?", &at(0)),
            says("The header row is missing.", &at(1)),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(Role::User, "Why does the import fail?", &at(0), true),
            entry(Role::Agent, "The header row is missing.", &at(1), false),
        ]
    );
    assert_eq!(read.next_offset, size(&file));
}

#[test]
fn tool_calls_and_their_answers_are_tool_entries() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("Read my notes", &at(0)),
            assistant(
                vec![
                    text("Opening them."),
                    tool_use("toolu_1", "Read", json!({"file_path": "/tmp/notes.md"})),
                ],
                &at(1),
            ),
            user_blocks(vec![tool_result("toolu_1", "buy milk")], &at(2)),
            assistant(
                vec![tool_use("toolu_2", "Grep", json!({"pattern": "milk"}))],
                &at(3),
            ),
            user_blocks(
                vec![tool_result_blocks("toolu_2", "notes.md:1:buy milk")],
                &at(4),
            ),
            says("You wanted milk.", &at(5)),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(Role::User, "Read my notes", &at(0), true),
            entry(Role::Agent, "Opening them.", &at(1), false),
            entry(
                Role::Tool,
                r#"Read {"file_path":"/tmp/notes.md"}"#,
                &at(1),
                false
            ),
            entry(Role::Tool, "buy milk", &at(2), false),
            entry(Role::Tool, r#"Grep {"pattern":"milk"}"#, &at(3), false),
            entry(Role::Tool, "notes.md:1:buy milk", &at(4), false),
            entry(Role::Agent, "You wanted milk.", &at(5), false),
        ]
    );
}

#[test]
fn only_a_user_line_with_text_starts_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("first prompt", &at(0)),
            assistant(
                vec![tool_use("toolu_1", "Read", json!({"file_path": "/a"}))],
                &at(1),
            ),
            user_blocks(vec![tool_result("toolu_1", "contents of a")], &at(2)),
            says("done", &at(3)),
            user_blocks(
                vec![text("second prompt"), text("with a second paragraph")],
                &at(4),
            ),
            says("done again", &at(5)),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    let starts: Vec<(&str, bool)> = read
        .entries
        .iter()
        .map(|entry| (entry.text.as_str(), entry.new_turn))
        .collect();
    assert_eq!(
        starts,
        vec![
            ("first prompt", true),
            (r#"Read {"file_path":"/a"}"#, false),
            ("contents of a", false),
            ("done", false),
            ("second prompt", true),
            ("with a second paragraph", false),
            ("done again", false),
        ]
    );
    assert_eq!(read.entries[4].role, Role::User);
    assert_eq!(read.entries[5].role, Role::User);
}

#[test]
fn a_line_without_a_timestamp_has_no_time() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let untimed = json!({"type": "user", "message": {"role": "user", "content": "no clock"}});
    write_lines(&file, &[untimed.to_string()]);

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![Entry {
            role: Role::User,
            text: "no clock".to_owned(),
            at: None,
            new_turn: true,
        }]
    );
}

#[test]
fn reading_from_the_last_offset_returns_only_what_was_added() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let mut own = OwnTools::default();
    // Offsets count bytes, so the first lines hold text of several bytes
    // a character.
    write_lines(
        &file,
        &[user("Größe prüfen ✓", &at(0)), says("اندازه ‹ok›", &at(1))],
    );

    let first = read_from(&file, 0, &mut own).unwrap();
    assert_eq!(first.entries.len(), 2);
    assert_eq!(first.next_offset, size(&file));

    let nothing_new = read_from(&file, first.next_offset, &mut own).unwrap();
    assert_eq!(nothing_new.entries, vec![]);
    assert_eq!(nothing_new.next_offset, first.next_offset);

    append_lines(
        &file,
        &[user("and now?", &at(2)), says("still fine", &at(3))],
    );
    let second = read_from(&file, first.next_offset, &mut own).unwrap();

    assert_eq!(
        second.entries,
        vec![
            entry(Role::User, "and now?", &at(2), true),
            entry(Role::Agent, "still fine", &at(3), false),
        ]
    );
    assert_eq!(second.next_offset, size(&file));
}

#[test]
fn a_last_line_cut_short_waits_until_it_is_whole() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let mut own = OwnTools::default();
    let whole = user("a whole line", &at(0));
    let late = says("written in two pieces", &at(1));
    let (head, tail) = late.split_at(late.len() / 2);
    write_lines(&file, &[whole.clone()]);
    append_bytes(&file, head.as_bytes());

    let first = read_from(&file, 0, &mut own).unwrap();

    assert_eq!(
        first.entries,
        vec![entry(Role::User, "a whole line", &at(0), true)]
    );
    assert_eq!(first.next_offset, whole.len() as u64 + 1);

    append_bytes(&file, format!("{tail}\n").as_bytes());
    let second = read_from(&file, first.next_offset, &mut own).unwrap();

    assert_eq!(
        second.entries,
        vec![entry(Role::Agent, "written in two pieces", &at(1), false)]
    );
    assert_eq!(second.next_offset, size(&file));
}

#[test]
fn a_last_line_without_its_newline_is_not_read_early() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let mut own = OwnTools::default();
    // All of the line's JSON is there; only the newline is still to come.
    append_bytes(&file, user("almost written", &at(0)).as_bytes());

    let early = read_from(&file, 0, &mut own).unwrap();
    assert_eq!(early.entries, vec![]);
    assert_eq!(early.next_offset, 0);

    append_bytes(&file, b"\n");
    let later = read_from(&file, early.next_offset, &mut own).unwrap();

    assert_eq!(
        later.entries,
        vec![entry(Role::User, "almost written", &at(0), true)]
    );
    assert_eq!(later.next_offset, size(&file));
}

#[test]
fn lines_that_hold_no_conversation_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let meta = json!({"type": "user", "isMeta": true, "timestamp": at(3),
        "message": {"role": "user", "content": "Caveat: generated by a local command"}});
    let side_prompt = json!({"type": "user", "isSidechain": true, "timestamp": at(4),
        "message": {"role": "user", "content": "prompt for a helper agent"}});
    let side_answer = json!({"type": "assistant", "isSidechain": true, "timestamp": at(5),
        "message": {"role": "assistant", "content": [{"type": "text", "text": "helper answer"}]}});
    write_lines(
        &file,
        &[
            "this line is not JSON".to_owned(),
            json!({"type": "summary", "summary": "Fixing an import", "leafUuid": "9a1b"})
                .to_string(),
            json!({"type": "system", "subtype": "compact_boundary", "timestamp": at(1),
                "content": "Conversation compacted"})
            .to_string(),
            json!({"type": "file-history-snapshot", "messageId": "3c2d", "snapshot": {}})
                .to_string(),
            meta.to_string(),
            side_prompt.to_string(),
            side_answer.to_string(),
            json!({"type": "progress", "data": {"type": "hook_progress"}, "timestamp": at(6)})
                .to_string(),
            String::new(),
            "[1, 2, 3]".to_owned(),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"cut".to_owned(),
            user("the only real line", &at(7)),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![entry(Role::User, "the only real line", &at(7), true)]
    );
    assert_eq!(read.next_offset, size(&file));
}

#[test]
fn blocks_that_are_neither_text_nor_tools_add_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user_blocks(
                vec![
                    json!({"type": "image", "source": {"type": "base64", "media_type": "image/png",
                        "data": "aGVsbG8="}}),
                    text("what is in this picture?"),
                ],
                &at(0),
            ),
            assistant(
                vec![
                    json!({"type": "thinking", "thinking": "private reasoning", "signature": "c2ln"}),
                    text("A greeting."),
                ],
                &at(1),
            ),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(Role::User, "what is in this picture?", &at(0), true),
            entry(Role::Agent, "A greeting.", &at(1), false),
        ]
    );
}

#[test]
fn a_missing_file_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("never-written.jsonl");

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(read.entries, vec![]);
    assert_eq!(read.next_offset, 0);
}

#[test]
fn a_file_that_cannot_be_read_is_an_io_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    // A folder where the conversation file should be.
    let file = dir.path().join("talk.jsonl");
    fs::create_dir(&file).unwrap();

    let error = read_from(&file, 0, &mut OwnTools::default()).unwrap_err();

    match error {
        MirrorError::Io { path, .. } => assert_eq!(path, file),
        other => panic!("expected an Io error, got {other:?}"),
    }
}

#[test]
fn calls_to_humatas_mcp_tools_and_their_answers_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("What did we decide about the ledger?", &at(0)),
            assistant(
                vec![
                    text("Searching the history."),
                    tool_use(
                        "toolu_own_1",
                        "mcp__humata__sessions_search",
                        json!({"query": "ledger"}),
                    ),
                    tool_use("toolu_gh", "mcp__github__search", json!({"q": "ledger"})),
                ],
                &at(1),
            ),
            user_blocks(
                vec![
                    tool_result("toolu_own_1", "hit: se_9 turn 4"),
                    tool_result_blocks("toolu_gh", "two issues"),
                ],
                &at(2),
            ),
            assistant(
                vec![tool_use(
                    "toolu_own_2",
                    "mcp__humata__session_read",
                    json!({"session": "se_9", "first_turn": 4, "last_turn": 4}),
                )],
                &at(3),
            ),
            user_blocks(
                vec![tool_result_blocks("toolu_own_2", "turn 4 of se_9")],
                &at(4),
            ),
            says("We decided to keep it.", &at(5)),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(
                Role::User,
                "What did we decide about the ledger?",
                &at(0),
                true
            ),
            entry(Role::Agent, "Searching the history.", &at(1), false),
            entry(
                Role::Tool,
                r#"mcp__github__search {"q":"ledger"}"#,
                &at(1),
                false
            ),
            entry(Role::Tool, "two issues", &at(2), false),
            entry(Role::Agent, "We decided to keep it.", &at(5), false),
        ]
    );
}

#[test]
fn humata_shell_commands_and_their_answers_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    // (id, command, whether it is a call to Humata's own CLI)
    let commands = [
        ("toolu_a", "humata sessions search ledger", true),
        ("toolu_b", "  humata  ", true),
        ("toolu_c", "\thumata boards list\n", true),
        ("toolu_d", "humatactl status", false),
        ("toolu_e", "echo humata sessions", false),
        ("toolu_f", "git status", false),
    ];
    let mut lines = vec![user("look around", &at(0))];
    for (id, command, _) in commands {
        lines.push(assistant(
            vec![tool_use(id, "Bash", json!({"command": command}))],
            &at(1),
        ));
        lines.push(user_blocks(
            vec![tool_result(id, &format!("output of {id}"))],
            &at(2),
        ));
    }
    write_lines(&file, &lines);

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    let mut expected = vec![entry(Role::User, "look around", &at(0), true)];
    for (id, command, own) in commands {
        if !own {
            let call = format!("Bash {}", json!({"command": command}));
            expected.push(entry(Role::Tool, &call, &at(1), false));
            expected.push(entry(Role::Tool, &format!("output of {id}"), &at(2), false));
        }
    }
    assert_eq!(read.entries, expected);
}

#[test]
fn the_answer_to_a_left_out_call_is_left_out_of_a_later_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let mut own = OwnTools::default();
    write_lines(
        &file,
        &[
            user("search for it", &at(0)),
            assistant(
                vec![tool_use(
                    "toolu_own",
                    "mcp__humata__sessions_search",
                    json!({"query": "x"}),
                )],
                &at(1),
            ),
        ],
    );
    let first = read_from(&file, 0, &mut own).unwrap();
    assert_eq!(
        first.entries,
        vec![entry(Role::User, "search for it", &at(0), true)]
    );

    append_lines(
        &file,
        &[
            user_blocks(vec![tool_result("toolu_own", "three hits")], &at(2)),
            says("Found three.", &at(3)),
        ],
    );
    let second = read_from(&file, first.next_offset, &mut own).unwrap();

    assert_eq!(
        second.entries,
        vec![entry(Role::Agent, "Found three.", &at(3), false)]
    );
}

#[test]
fn left_out_calls_are_remembered_through_saving_and_loading() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    let mut own = OwnTools::default();
    write_lines(
        &file,
        &[assistant(
            vec![tool_use(
                "toolu_own",
                "Bash",
                json!({"command": "humata inbox list"}),
            )],
            &at(0),
        )],
    );
    let first = read_from(&file, 0, &mut own).unwrap();
    assert_eq!(first.entries, vec![]);

    let saved = serde_json::to_string(&own).unwrap();
    let mut loaded: OwnTools = serde_json::from_str(&saved).unwrap();
    append_lines(
        &file,
        &[
            user_blocks(
                vec![tool_result("toolu_own", "two requests waiting")],
                &at(1),
            ),
            says("Two requests wait for you.", &at(2)),
        ],
    );
    let second = read_from(&file, first.next_offset, &mut loaded).unwrap();

    assert_eq!(
        second.entries,
        vec![entry(
            Role::Agent,
            "Two requests wait for you.",
            &at(2),
            false
        )]
    );
}
