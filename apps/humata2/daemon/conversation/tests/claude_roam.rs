//! A Claude Code conversation carried whole (MV-16, CV-4): its own files
//! kept outside the per-folder record (`file-history/<id>/`, `todos/` and
//! `plans/` under the CLI's config dir, `CLAUDE_CONFIG_DIR` when it is set)
//! travel with it and land under the new machine's config dir; another
//! conversation of the same folder stays where it is; the old home, config
//! dir and folder are named nowhere on the new machine; and a folder given
//! through a link is found where the CLI recorded its real path. Every home
//! is a temp folder; the real `~/.claude` is never read.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use humata2_conversation::{Adapter, Conversation, FileRole, Format, Placement, Turn, adapter};
use serde_json::Value;

/// Three turns, with injected context, tool results and a side conversation
/// between them. `{folder}`, `{project}` and `{id}` are filled per test.
const THREE_TURNS: &str = include_str!("../testdata/claude/three_turns.jsonl");

const ID: &str = "4f6d2c1e-8a3b-4c5d-9e7f-1a2b3c4d5e6f";
/// Another conversation in the same folder.
const OTHER: &str = "9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b";
/// The name the CLI gave each conversation's plan.
const SLUG: &str = "witty-dancing-otter";
const OTHER_SLUG: &str = "other-quiet-heron";
const VERSION: &str = "2.1.325";

/// A backup of a file the agent edited, as the CLI keeps it: the person's
/// file, byte for byte. One is text, one isn't.
const LOGIN_BACKUP: &[u8] = b"pub fn login() -> Redirect {\n    Redirect::to(\"/callback\")\n}\n";
const LOGO_BACKUP: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe];

fn claude() -> &'static dyn Adapter {
    adapter(Format::ClaudeJsonl)
}

/// The old machine under `root`, its Claude config dir set.
fn old_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("old/home"),
        cli_home: Some(root.join("old/claude-config")),
        folder: root.join("old/channels/web/sessions/fix-login"),
    }
}

/// Another machine's home with its Claude config dir set, and its folder.
fn new_machine(root: &Path) -> Placement {
    Placement {
        home: root.join("new/home"),
        cli_home: Some(root.join("new/claude-config")),
        folder: root.join("new/work/web/fix-login"),
    }
}

fn dashed(folder: &Path) -> String {
    humata2_conversation::claude_project_name(folder)
}

fn config(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".claude"))
}

fn project(at: &Placement) -> PathBuf {
    config(at).join("projects").join(dashed(&at.folder))
}

fn transcript(at: &Placement, id: &str) -> PathBuf {
    project(at).join(format!("{id}.jsonl"))
}

/// [`THREE_TURNS`] as a conversation that has a plan: each record names its
/// slug, and the first prompt names a file in the home and the plan.
fn with_plan() -> String {
    THREE_TURNS.replace("\"version\":\"2.1.325\",", "\"version\":\"2.1.325\",\"slug\":\"{slug}\",").replace(
        "It loops back to /callback after sign-in.",
        "It loops back to /callback after sign-in. My notes are in {home}/notes/login.md and the plan is {config}/plans/{slug}.md.",
    )
}

/// `template` as Claude would have written it for conversation `id` with
/// plan `slug` at `at`.
fn fill(template: &str, at: &Placement, id: &str, slug: &str) -> String {
    template
        .replace("{project}", &project(at).to_string_lossy())
        .replace("{config}", &config(at).to_string_lossy())
        .replace("{home}", &at.home.to_string_lossy())
        .replace("{folder}", &at.folder.to_string_lossy())
        .replace("{slug}", slug)
        .replace("{id}", id)
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Conversation `id` with plan `slug` at `at`, and every file the CLI keeps
/// for it: a saved tool output beside the transcript, two backups, its todo
/// list and its plan. Returns the paths outside the per-folder record.
fn write_conversation(at: &Placement, id: &str, slug: &str) -> Vec<PathBuf> {
    let text = |template: &str| fill(template, at, id, slug).into_bytes();
    write(&transcript(at, id), &text(&with_plan()));
    write(
        &project(at).join(id).join("tool-results/toolu_01A.txt"),
        &text("pub fn login() at {folder}/api/src/login.rs\n"),
    );
    let side = vec![
        config(at).join("file-history").join(id).join("login.rs@v1"),
        config(at).join("file-history").join(id).join("logo.png@v1"),
        config(at).join("todos").join(format!("{id}-agent-{id}.json")),
        config(at).join("plans").join(format!("{slug}.md")),
    ];
    write(&side[0], LOGIN_BACKUP);
    write(&side[1], LOGO_BACKUP);
    write(
        &side[2],
        &text(r#"[{"content":"Fix the redirect in {folder}/api/src/login.rs","status":"in_progress","activeForm":"Fixing the redirect"}]"#),
    );
    write(&side[3], &text("# Fix the login redirect\n\n1. Change {folder}/api/src/login.rs\n2. Add {folder}/api/tests/login.rs\n"));
    side
}

/// The old machine with conversation `ID`, another conversation `OTHER` of
/// the same folder, and the folder's memory. Returns `ID`'s files outside
/// the per-folder record and `OTHER`'s files.
fn two_conversations(at: &Placement) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let own = write_conversation(at, ID, SLUG);
    let mut other = write_conversation(at, OTHER, OTHER_SLUG);
    other.push(transcript(at, OTHER));
    other.push(project(at).join(OTHER).join("tool-results/toolu_01A.txt"));
    write(&project(at).join("memory/MEMORY.md"), fill("- Login lives in {folder}/api/src/login.rs\n", at, ID, SLUG).as_bytes());
    (own, other)
}

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

/// The files under `dir` whose name or bytes hold `needle`.
fn files_naming(dir: &Path, needle: &str) -> Vec<PathBuf> {
    snapshot(dir)
        .into_iter()
        .filter(|(path, bytes)| {
            path.to_string_lossy().contains(needle) || String::from_utf8_lossy(bytes).contains(needle)
        })
        .map(|(path, _)| path)
        .collect()
}

fn roles(conversation: &Conversation) -> BTreeMap<PathBuf, FileRole> {
    conversation.files.iter().map(|file| (file.path.clone(), file.role)).collect()
}

fn three_turns() -> Vec<Turn> {
    [
        (1, "Fix the login redirect", "2026-11-16T09:01:00.000Z"),
        (2, "Now add a test for it", "2026-11-16T09:05:00.000Z"),
        (3, "Run the tests", "2026-11-16T09:10:00.000Z"),
    ]
    .into_iter()
    .map(|(index, first_line, at)| Turn { index, first_line: first_line.to_owned(), at: Some(at.to_owned()) })
    .collect()
}

#[test]
fn locate_lists_the_conversations_own_files_in_the_config_dir_and_nothing_of_another_conversation() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let at = old_machine(&root);
    let (own, other) = two_conversations(&at);

    let conversation = claude().locate(&at, ID).unwrap();

    assert_eq!(conversation.files[0].path, transcript(&at, ID));
    assert_eq!(conversation.files[0].role, FileRole::Transcript);
    let mut wanted = BTreeMap::from([
        (transcript(&at, ID), FileRole::Transcript),
        (project(&at).join(ID).join("tool-results/toolu_01A.txt"), FileRole::Companion),
        (project(&at).join("memory/MEMORY.md"), FileRole::Project),
    ]);
    wanted.extend(own.into_iter().map(|path| (path, FileRole::Companion)));
    assert_eq!(roles(&conversation), wanted);
    for path in other {
        assert!(!roles(&conversation).contains_key(&path), "{} is another conversation's", path.display());
    }
    assert!(!at.home.join(".claude").exists(), "the CLI's config dir is the one set for it");
}

#[test]
fn a_carry_takes_the_conversations_own_files_to_the_new_config_dir_and_names_no_old_path() {
    let tmp = tempfile::tempdir().unwrap();
    // Real paths, so the folder named and the folder the CLI records agree.
    let root = fs::canonicalize(tmp.path()).unwrap();
    let from = old_machine(&root);
    let (_, other) = two_conversations(&from);
    let before = snapshot(&root.join("old"));
    let to = new_machine(&root);

    let conversation = claude().locate(&from, ID).unwrap();
    let stage = root.join("stage");
    let packed = claude().pack(&conversation, &stage).unwrap();
    let placed = claude().place(&packed, &from, &to).unwrap();

    // The conversation's own files, under the config dir set on the new machine.
    let new_config = config(&to);
    assert_eq!(fs::read(new_config.join("file-history").join(ID).join("login.rs@v1")).unwrap(), LOGIN_BACKUP);
    assert_eq!(fs::read(new_config.join("file-history").join(ID).join("logo.png@v1")).unwrap(), LOGO_BACKUP);
    let todos = fs::read_to_string(new_config.join("todos").join(format!("{ID}-agent-{ID}.json"))).unwrap();
    assert!(todos.contains(&format!("{}/api/src/login.rs", to.folder.display())), "{todos}");
    let plan = fs::read_to_string(new_config.join("plans").join(format!("{SLUG}.md"))).unwrap();
    assert!(plan.contains(&format!("{}/api/tests/login.rs", to.folder.display())), "{plan}");
    assert!(!to.home.join(".claude").exists(), "nothing lands in the default config dir");

    // The new home and config dir in place of the old ones.
    let prompt = fs::read_to_string(transcript(&to, ID)).unwrap();
    assert!(prompt.contains(&format!("{}/notes/login.md", to.home.display())), "the home is the new one");
    assert!(prompt.contains(&format!("{}/plans/{SLUG}.md", new_config.display())), "the plan is the new one");
    let new_side = root.join("new");
    for old in [root.join("old").to_string_lossy().into_owned(), dashed(&from.folder)] {
        assert_eq!(files_naming(&new_side, &old), Vec::<PathBuf>::new(), "{old} is still named");
    }
    for record in records(&transcript(&to, ID)) {
        if let Some(cwd) = record.get("cwd") {
            assert_eq!(cwd, &Value::from(to.folder.to_string_lossy().into_owned()));
        }
    }

    // The other conversation of the folder stayed, and wasn't staged.
    for path in &other {
        let rel = path.strip_prefix(root.join("old")).unwrap();
        assert!(path.exists(), "{} is still on the old machine", path.display());
        assert!(!new_side.join(rel).exists(), "{} was carried", rel.display());
    }
    assert!(!new_config.join("file-history").join(OTHER).exists());
    assert!(!new_config.join("todos").join(format!("{OTHER}-agent-{OTHER}.json")).exists());
    assert!(!new_config.join("plans").join(format!("{OTHER_SLUG}.md")).exists());
    assert!(!transcript(&to, OTHER).exists());
    for needle in [OTHER, OTHER_SLUG] {
        assert_eq!(files_naming(&stage, needle), Vec::<PathBuf>::new(), "{needle} was staged");
        assert_eq!(files_naming(&new_side, needle), Vec::<PathBuf>::new(), "{needle} was placed");
    }

    // What is there is what the CLI finds, and reads.
    let found = claude().locate(&to, ID).unwrap();
    assert_eq!(roles(&found), roles(&placed));
    claude().check(&placed, Some(VERSION)).unwrap();
    assert_eq!(claude().turns(&placed).unwrap(), three_turns());
    assert_eq!(snapshot(&root.join("old")), before, "the old machine's files are as they were");
}

#[test]
fn a_carry_into_a_folder_too_long_for_one_folder_name_lands_where_the_cli_looks_for_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let from = old_machine(&root);
    two_conversations(&from);
    // Dashed whole, this folder would be longer than a file system lets one
    // folder name be (255 bytes); the CLI cuts its name and adds a hash (X2).
    let to = Placement {
        folder: root.join("new/work").join("w".repeat(250)),
        ..new_machine(&root)
    };

    let conversation = claude().locate(&from, ID).unwrap();
    let packed = claude().pack(&conversation, &root.join("stage")).unwrap();
    let placed = claude().place(&packed, &from, &to).unwrap();

    assert_eq!(placed.files[0].path, transcript(&to, ID));
    let name = project(&to).file_name().unwrap().len();
    assert!(name <= 255, "the per-folder record's name is {name} bytes");
    for record in records(&transcript(&to, ID)) {
        if let Some(cwd) = record.get("cwd") {
            assert_eq!(cwd, &Value::from(to.folder.to_string_lossy().into_owned()));
        }
    }
    let found = claude().locate(&to, ID).unwrap();
    assert_eq!(roles(&found), roles(&placed));
    assert_eq!(claude().turns(&placed).unwrap(), three_turns());
}

#[cfg(unix)]
#[test]
fn a_folder_given_through_a_link_is_found_where_the_cli_recorded_its_real_path() {
    let tmp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    // The CLI records the folder's real path (on macOS, `/private/var/...`
    // for a `/var/...` folder); Humata may name it through the link.
    let real = root.join("real/web/fix-login");
    fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
    let recorded = Placement { home: root.join("mac/home"), cli_home: None, folder: real.clone() };
    write(&transcript(&recorded, ID), fill(THREE_TURNS, &recorded, ID, SLUG).as_bytes());
    let given = Placement { folder: root.join("alias/web/fix-login"), ..recorded.clone() };

    let conversation = claude().locate(&given, ID).unwrap();
    assert_eq!(conversation.files[0].path, transcript(&recorded, ID));
    assert_eq!(claude().turns(&conversation).unwrap(), three_turns());

    let to = new_machine(&root);
    let packed = claude().pack(&conversation, &root.join("stage")).unwrap();
    let placed = claude().place(&packed, &given, &to).unwrap();

    assert_eq!(placed.files[0].path, transcript(&to, ID));
    for old in [
        real.to_string_lossy().into_owned(),
        given.folder.to_string_lossy().into_owned(),
        dashed(&real),
        dashed(&given.folder),
    ] {
        assert_eq!(files_naming(&config(&to), &old), Vec::<PathBuf>::new(), "{old} is still named");
    }
    for record in records(&placed.files[0].path) {
        if let Some(cwd) = record.get("cwd") {
            assert_eq!(cwd, &Value::from(to.folder.to_string_lossy().into_owned()));
        }
    }
    assert_eq!(claude().turns(&placed).unwrap(), three_turns());
}
