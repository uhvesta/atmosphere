//! `turns_of`: where each prompt of a conversation file starts (FK-3, D181).

use std::path::Path;

use humata2_mirror::turns_of;

fn write(dir: &Path, text: &str) -> std::path::PathBuf {
    let file = dir.join("conversation.jsonl");
    std::fs::write(&file, text).unwrap();
    file
}

#[test]
fn each_prompt_is_a_turn_with_its_first_line_time_and_offset() {
    let dir = tempfile::tempdir().unwrap();
    let first = r#"{"type":"user","message":{"role":"user","content":"Find the flaky test"},"timestamp":"2026-11-15T09:00:00Z"}"#;
    let answer = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"It is retry_test."}]}}"#;
    let second = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Make it deterministic\nwith a fake clock"}]},"timestamp":"2026-11-15T09:01:00Z"}"#;
    let text = format!("{first}\n{answer}\n{second}\n");
    let file = write(dir.path(), &text);

    let turns = turns_of(&file).unwrap();

    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].index, 1);
    assert_eq!(turns[0].first_line, "Find the flaky test");
    assert_eq!(turns[0].at.as_deref(), Some("2026-11-15T09:00:00Z"));
    assert_eq!(turns[0].offset, 0);
    assert_eq!(turns[1].index, 2);
    assert_eq!(turns[1].first_line, "Make it deterministic");
    assert_eq!(turns[1].at.as_deref(), Some("2026-11-15T09:01:00Z"));
    // The offset is where the line starts, so the bytes before it are the
    // conversation before the second turn.
    let offset = usize::try_from(turns[1].offset).unwrap();
    assert!(text[offset..].starts_with(second));
    assert_eq!(&text.as_bytes()[..offset], format!("{first}\n{answer}\n").as_bytes());
}

#[test]
fn tool_answers_meta_side_chains_and_other_lines_are_not_turns() {
    let dir = tempfile::tempdir().unwrap();
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Real prompt"}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"fn a() {}"}]}}"#,
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
        r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"side question"}}"#,
        r#"{"type":"user","message":{"role":"user","content":""}}"#,
        r#"{"type":"summary","summary":"a summary"}"#,
        "not json at all",
        r#"{"type":"user","message":{"role":"user","content":"Second prompt"}}"#,
    ];
    let file = write(dir.path(), &(lines.join("\n") + "\n"));

    let turns = turns_of(&file).unwrap();

    let listed: Vec<(u32, &str)> = turns
        .iter()
        .map(|turn| (turn.index, turn.first_line.as_str()))
        .collect();
    assert_eq!(listed, vec![(1, "Real prompt"), (2, "Second prompt")]);
    assert_eq!(turns[0].at, None);
}

#[test]
fn a_last_line_still_being_written_is_not_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let complete = r#"{"type":"user","message":{"role":"user","content":"Done"}}"#;
    let partial = r#"{"type":"user","message":{"role":"user","content":"Still wri"#;
    let file = write(dir.path(), &format!("{complete}\n{partial}"));

    let turns = turns_of(&file).unwrap();

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].first_line, "Done");
}

#[test]
fn a_missing_file_has_no_turns() {
    let dir = tempfile::tempdir().unwrap();
    assert!(turns_of(&dir.path().join("nothing.jsonl")).unwrap().is_empty());
}
