//! Mirroring a session's conversation file: only what is new is added,
//! scrubbed, and numbered by entry and by turn.

mod support;

use std::fs;
use std::path::Path;

use humata2_mirror::{Mirror, MirrorEntry, Role, Scrubber};
use serde_json::json;
use support::*;

fn entry(seq: u64, turn: u32, role: Role, text: &str, at: &str) -> MirrorEntry {
    MirrorEntry {
        seq,
        turn,
        role,
        text: text.to_owned(),
        at: Some(at.to_owned()),
    }
}

/// Whether any file under `dir` holds `needle`.
fn any_file_holds(dir: &Path, needle: &[u8]) -> bool {
    fs::read_dir(dir).unwrap().any(|item| {
        let path = item.unwrap().path();
        if path.is_dir() {
            any_file_holds(&path, needle)
        } else {
            fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle)
        }
    })
}

#[test]
fn catching_up_adds_what_the_file_holds_and_then_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("Why does the import fail?", &at(0)),
            says("The header row is missing.", &at(1)),
        ],
    );
    mirror
        .describe(&meta("se_1", "ch_main", "ma_mac", "claude"))
        .unwrap();

    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 2);
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);

    assert_eq!(
        entries(&mirror, "se_1"),
        vec![
            entry(0, 1, Role::User, "Why does the import fail?", &at(0)),
            entry(1, 1, Role::Agent, "The header row is missing.", &at(1)),
        ]
    );
}

#[test]
fn catching_up_again_adds_only_what_was_appended() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let file = mirror_session(
        &mirror,
        dir.path(),
        "se_1",
        &[user("first question", &at(0)), says("first answer", &at(1))],
    );

    append_lines(
        &file,
        &[
            user("second question", &at(2)),
            assistant(
                vec![
                    text("looking"),
                    tool_use("toolu_1", "Read", json!({"file_path": "/a"})),
                ],
                &at(3),
            ),
            user_blocks(vec![tool_result("toolu_1", "contents of a")], &at(4)),
        ],
    );

    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 4);
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);
    assert_eq!(
        entries(&mirror, "se_1"),
        vec![
            entry(0, 1, Role::User, "first question", &at(0)),
            entry(1, 1, Role::Agent, "first answer", &at(1)),
            entry(2, 2, Role::User, "second question", &at(2)),
            entry(3, 2, Role::Agent, "looking", &at(3)),
            entry(4, 2, Role::Tool, r#"Read {"file_path":"/a"}"#, &at(3)),
            entry(5, 2, Role::Tool, "contents of a", &at(4)),
        ]
    );
}

#[test]
fn a_line_still_being_written_is_mirrored_once_it_is_whole() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let file = mirror_session(&mirror, dir.path(), "se_1", &[user("a question", &at(0))]);
    let late = says("an answer written in two pieces", &at(1));
    let (head, tail) = late.split_at(late.len() / 2);

    append_bytes(&file, head.as_bytes());
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);
    assert_eq!(texts(&entries(&mirror, "se_1")), vec!["a question"]);

    append_bytes(&file, format!("{tail}\n").as_bytes());
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 1);
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);
    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        vec!["a question", "an answer written in two pieces"]
    );
}

/// Every file under `dir` with its bytes.
fn snapshot(dir: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for item in fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            files.extend(snapshot(&path));
        } else {
            files.push((path.clone(), fs::read(&path).unwrap()));
        }
    }
    files.sort();
    files
}

#[test]
fn a_catch_up_that_adds_nothing_writes_nothing() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mirror = Mirror::open(&state).unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("Why does the import fail?", &at(0))]);
    mirror
        .describe(&meta("se_1", "ch_main", "ma_mac", "claude"))
        .unwrap();
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 1);
    let session_dir = state.join("mirror").join("se_1");
    let before = snapshot(&session_dir);

    // A session folder nothing can be written to: a catch-up with nothing
    // new must not need to.
    fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let again = mirror.catch_up("se_1", &file, &no_vars());
    fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(again.unwrap(), 0);
    assert_eq!(snapshot(&session_dir), before);
    assert_eq!(entries(&mirror, "se_1").len(), 1);
}

#[test]
fn a_conversation_file_that_does_not_exist_yet_adds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let file = dir.path().join("not-yet.jsonl");
    mirror
        .describe(&meta("se_1", "ch_main", "ma_mac", "claude"))
        .unwrap();

    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);
    assert_eq!(entries(&mirror, "se_1"), vec![]);

    write_lines(&file, &[user("now it exists", &at(0))]);
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 1);
}

#[test]
fn turns_count_the_users_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    mirror_session(
        &mirror,
        dir.path(),
        "se_1",
        &[
            says("Resuming where we stopped.", &at(0)),
            user("first prompt", &at(1)),
            assistant(
                vec![tool_use("toolu_1", "Read", json!({"file_path": "/a"}))],
                &at(2),
            ),
            user_blocks(vec![tool_result("toolu_1", "contents of a")], &at(3)),
            says("first answer", &at(4)),
            user("second prompt", &at(5)),
            says("second answer", &at(6)),
            user_blocks(vec![text("third prompt"), text("in two blocks")], &at(7)),
        ],
    );

    let read = entries(&mirror, "se_1");
    let numbered: Vec<(u64, u32, &str)> = read
        .iter()
        .map(|entry| (entry.seq, entry.turn, entry.text.as_str()))
        .collect();
    assert_eq!(
        numbered,
        vec![
            (0, 0, "Resuming where we stopped."),
            (1, 1, "first prompt"),
            (2, 1, r#"Read {"file_path":"/a"}"#),
            (3, 1, "contents of a"),
            (4, 1, "first answer"),
            (5, 2, "second prompt"),
            (6, 2, "second answer"),
            (7, 3, "third prompt"),
            (8, 3, "in two blocks"),
        ]
    );
}

#[test]
fn numbering_goes_on_after_the_mirror_is_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let file = {
        let mirror = Mirror::open(&root).unwrap();
        mirror_session(
            &mirror,
            dir.path(),
            "se_1",
            &[user("first question", &at(0)), says("first answer", &at(1))],
        )
    };

    let mirror = Mirror::open(&root).unwrap();
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 0);
    append_lines(
        &file,
        &[
            says("an afterthought", &at(2)),
            user("second question", &at(3)),
        ],
    );
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 2);

    assert_eq!(
        entries(&mirror, "se_1"),
        vec![
            entry(0, 1, Role::User, "first question", &at(0)),
            entry(1, 1, Role::Agent, "first answer", &at(1)),
            entry(2, 1, Role::Agent, "an afterthought", &at(2)),
            entry(3, 2, Role::User, "second question", &at(3)),
        ]
    );
}

#[test]
fn sessions_are_mirrored_apart() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let one = mirror_session(&mirror, dir.path(), "se_1", &[user("question one", &at(0))]);
    let two = mirror_session(
        &mirror,
        dir.path(),
        "se_2",
        &[user("question two", &at(1)), says("answer two", &at(2))],
    );

    append_lines(&one, &[says("answer one", &at(3))]);
    assert_eq!(mirror.catch_up("se_1", &one, &no_vars()).unwrap(), 1);
    assert_eq!(mirror.catch_up("se_2", &two, &no_vars()).unwrap(), 0);

    assert_eq!(
        entries(&mirror, "se_1"),
        vec![
            entry(0, 1, Role::User, "question one", &at(0)),
            entry(1, 1, Role::Agent, "answer one", &at(3)),
        ]
    );
    assert_eq!(
        entries(&mirror, "se_2"),
        vec![
            entry(0, 1, Role::User, "question two", &at(1)),
            entry(1, 1, Role::Agent, "answer two", &at(2)),
        ]
    );
}

#[test]
fn host_variable_values_never_reach_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let mirror = Mirror::open(&root).unwrap();
    let file = dir.path().join("talk.jsonl");
    let scrubber = Scrubber::new([
        ("API_TOKEN".to_owned(), "tok-4f9a2c7e".to_owned()),
        ("DB_PASSWORD".to_owned(), "hunter2zebra".to_owned()),
    ]);
    write_lines(
        &file,
        &[
            user("deploy with tok-4f9a2c7e please", &at(0)),
            assistant(
                vec![tool_use(
                    "toolu_1",
                    "Bash",
                    json!({"command": "curl -H 'Authorization: tok-4f9a2c7e' https://example.test"}),
                )],
                &at(1),
            ),
            user_blocks(
                vec![tool_result("toolu_1", "connected as admin:hunter2zebra")],
                &at(2),
            ),
            says("Deployed; the password hunter2zebra still works.", &at(3)),
        ],
    );
    mirror
        .describe(&meta("se_1", "ch_main", "ma_mac", "claude"))
        .unwrap();

    assert_eq!(mirror.catch_up("se_1", &file, &scrubber).unwrap(), 4);

    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        vec![
            "deploy with ‹API_TOKEN› please",
            r#"Bash {"command":"curl -H 'Authorization: ‹API_TOKEN›' https://example.test"}"#,
            "connected as admin:‹DB_PASSWORD›",
            "Deployed; the password ‹DB_PASSWORD› still works.",
        ]
    );
    assert_eq!(find(&mirror, "hunter2zebra"), vec![]);
    assert_eq!(find(&mirror, "tok-4f9a2c7e"), vec![]);
    let deployed = find(&mirror, "deployed password");
    assert_eq!(places(&deployed), vec![("se_1".to_owned(), 1)]);
    assert!(!deployed[0].snippet.contains("hunter2zebra"));
    // Not in any file the mirror or its index keeps, closed or open.
    drop(mirror);
    assert!(!any_file_holds(&root, b"tok-4f9a2c7e"));
    assert!(!any_file_holds(&root, b"hunter2zebra"));
}

#[test]
fn calls_to_humatas_own_tools_are_not_mirrored() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let file = {
        let mirror = Mirror::open(&root).unwrap();
        mirror_session(
            &mirror,
            dir.path(),
            "se_1",
            &[
                user("what did we decide?", &at(0)),
                assistant(
                    vec![
                        tool_use(
                            "toolu_own_1",
                            "mcp__humata__sessions_search",
                            json!({"query": "walrus"}),
                        ),
                        tool_use(
                            "toolu_own_2",
                            "Bash",
                            json!({"command": "humata sessions search narwhal"}),
                        ),
                    ],
                    &at(1),
                ),
                user_blocks(
                    vec![tool_result("toolu_own_1", "hit: the walrus plan")],
                    &at(2),
                ),
            ],
        )
    };

    // The second answer arrives after a restart of the host.
    let mirror = Mirror::open(&root).unwrap();
    append_lines(
        &file,
        &[
            user_blocks(
                vec![tool_result("toolu_own_2", "hit: the narwhal plan")],
                &at(3),
            ),
            says("We decided on the otter plan.", &at(4)),
        ],
    );
    assert_eq!(mirror.catch_up("se_1", &file, &no_vars()).unwrap(), 1);

    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        vec!["what did we decide?", "We decided on the otter plan."]
    );
    assert_eq!(find(&mirror, "walrus"), vec![]);
    assert_eq!(find(&mirror, "narwhal"), vec![]);
    assert_eq!(
        places(&find(&mirror, "otter")),
        vec![("se_1".to_owned(), 1)]
    );
}

#[test]
fn clones_mirror_from_several_threads() {
    fn shared<T: Clone + Send + Sync + 'static>() {}
    shared::<Mirror>();

    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();

    std::thread::scope(|scope| {
        for n in 0..4 {
            let mirror = mirror.clone();
            let dir = dir.path();
            scope.spawn(move || {
                let session = format!("se_{n}");
                let lines: Vec<String> = (0..20)
                    .flat_map(|turn| {
                        [
                            user(&format!("question {turn} about the lighthouse"), &at(turn)),
                            says(&format!("answer {turn} from thread {n}"), &at(turn)),
                        ]
                    })
                    .collect();
                mirror_session(&mirror, dir, &session, &lines);
            });
        }
    });

    for n in 0..4 {
        assert_eq!(entries(&mirror, &format!("se_{n}")).len(), 40);
    }
    assert_eq!(find(&mirror, "lighthouse").len(), 80);
}
