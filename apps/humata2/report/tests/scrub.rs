//! Nothing the app seeded as secret reaches a written report: not typed
//! over several events, not in the state's changes, not in a masked field,
//! a terminal or a paste-folder file.

mod support;

use std::fs;

use humata2_report::{
    Comment, Form, Gesture, Holds, LogLine, Pasted, Rpc, ScrubError, Secrets, scrub, write,
};
use support::{Driver, add_button, assert_no_piece_of, draft_field, files, item, terminal, token_field};

const TOKEN: &str = "ghp_Zx81Qm2Lp0Vb7Rt4";

/// No six characters of a secret in a row survive.
const PIECE: usize = 6;

fn written(captured: humata2_report::Captured, form: Form, secrets: &Secrets) -> (tempfile::TempDir, std::path::PathBuf) {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let scrubbed = scrub(captured, form, secrets).expect("scrubs");
    let dir = write(reports.path(), &scrubbed).expect("writes");
    (reports, dir)
}

fn rpc(at_ms: u64, response: &str, holds: Holds) -> Rpc {
    Rpc {
        at_ms,
        machine: "studio".to_owned(),
        method: "humata.v1.SessionService/Get".to_owned(),
        status: "ok".to_owned(),
        duration_ms: 2,
        request: Some("{\"id\":\"s-1\"}".to_owned()),
        response: Some(response.to_owned()),
        holds,
    }
}

#[test]
fn a_token_typed_over_several_events_is_gone_from_every_file() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "my token is ghp_Zx8");
    driver.type_into(draft_field(), "1Qm2Lp0V");
    driver.type_into(draft_field(), "b7Rt4");
    driver.key(draft_field(), "enter");
    driver.type_into(draft_field(), "and milk");
    driver.click(add_button());
    let secrets = Secrets::new().secret("GITHUB_TOKEN", TOKEN);

    let (_reports, dir) = written(driver.stop(), Form::default(), &secrets);

    assert_no_piece_of(&dir, TOKEN, PIECE);
    let events = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    assert!(events.contains("and milk"), "text that isn't secret is kept");
}

#[test]
fn a_secret_that_was_in_the_state_only_between_two_actions_is_gone() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    let state = driver.toy.state();
    let loaded = state.replace("milk", &format!("milk {TOKEN}"));
    driver.recording.action(150, "Loaded", &loaded);
    driver.recording.action(160, "Cleared", &state);
    driver.click(add_button());
    let secrets = Secrets::new().secret("GITHUB_TOKEN", TOKEN);

    let (_reports, dir) = written(driver.stop(), Form::default(), &secrets);

    assert_no_piece_of(&dir, TOKEN, PIECE);
}

#[test]
fn a_host_variable_value_is_gone_from_calls_logs_errors_context_and_form() {
    let value = "s3cr3t-prod-database-url";
    let mut driver = Driver::start();
    driver.recording.context_mut().sessions[0].title = format!("debug {value}");
    driver.click(add_button());
    driver
        .recording
        .rpc(rpc(150, &format!("{{\"env\":{{\"DATABASE_URL\":\"{value}\"}}}}"), Holds::Plain));
    driver.recording.log(LogLine {
        at_ms: 160,
        machine: "studio".to_owned(),
        line: format!("connecting to {value}"),
    });
    driver.recording.daemon_error(humata2_report::DaemonError {
        at_ms: 170,
        machine: "studio".to_owned(),
        method: None,
        code: "internal".to_owned(),
        message: format!("couldn't reach {value}"),
    });
    let form = Form {
        happened: format!("it printed {value} in the list"),
        expected: "nothing".to_owned(),
        comments: vec![Comment {
            element: add_button(),
            text: format!("this showed {value}"),
            place: None,
        }],
        ..Form::default()
    };
    let secrets = Secrets::new().secret("DATABASE_URL", value);

    let (_reports, dir) = written(driver.stop(), form, &secrets);

    assert_no_piece_of(&dir, value, PIECE);
}

#[test]
fn what_is_typed_into_a_masked_field_is_gone_everywhere() {
    let password = "hunter2-correct-horse";
    let mut driver = Driver::start();
    driver.click(token_field());
    driver.type_into(token_field(), "hunter2-");
    driver.type_into(token_field(), "correct-horse");
    // The app echoes it somewhere it shouldn't.
    driver.recording.log(LogLine {
        at_ms: 350,
        machine: "studio".to_owned(),
        line: format!("set token {password}"),
    });

    let (_reports, dir) = written(driver.stop(), Form::default(), &Secrets::new());

    assert_no_piece_of(&dir, password, PIECE);
}

fn terminal_session() -> humata2_report::Captured {
    let mut driver = Driver::start();
    driver.click(terminal());
    driver.type_into(terminal(), "cat ~/.ssh/id_ed25519");
    driver.key(terminal(), "enter");
    let at = driver.now + 10;
    driver.recording.rpc(rpc(
        at,
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        Holds::Terminal {
            terminal: "shell".to_owned(),
        },
    ));
    driver.click(item("nothing"));
    driver.stop()
}

#[test]
fn terminal_text_is_left_out_unless_its_terminal_is_marked() {
    let (_reports, dir) = written(terminal_session(), Form::default(), &Secrets::new());

    assert_no_piece_of(&dir, "cat ~/.ssh/id_ed25519", PIECE);
    assert_no_piece_of(&dir, "-----BEGIN OPENSSH PRIVATE KEY-----", PIECE + 6);
}

#[test]
fn a_marked_terminal_keeps_its_text() {
    let secrets = Secrets::new().keep_terminal("shell");
    let (_reports, dir) = written(terminal_session(), Form::default(), &secrets);

    let events = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    assert!(events.contains("cat ~/.ssh/id_ed25519"));
    assert!(events.contains("BEGIN OPENSSH PRIVATE KEY"));
    let view = fs::read_to_string(dir.join("view.txt")).expect("view.txt");
    assert!(view.contains("cat ~/.ssh/id_ed25519"));
}

#[test]
fn a_paste_folder_file_keeps_its_path_and_loses_its_contents() {
    let path = "/home/me/.humata/paste/s-1/notes.txt";
    let contents = "the launch code is 0451-alpha-tango";
    let mut driver = Driver::start();
    driver.act(Gesture::Paste {
        target: Some(terminal()),
        content: Pasted::File {
            path: path.to_owned(),
            bytes: contents.len() as u64,
        },
    });
    driver.recording.rpc(rpc(150, "uploaded", Holds::PasteContents));
    driver.recording.rpc(Rpc {
        request: Some(contents.to_owned()),
        ..rpc(160, "stored", Holds::PasteContents)
    });
    // The same text, echoed where nothing marks it as the file's.
    driver.recording.rpc(rpc(170, &format!("{{\"preview\":\"{contents}\"}}"), Holds::Plain));
    let secrets = Secrets::new().paste_file(path, contents.as_bytes());

    let (_reports, dir) = written(driver.stop(), Form::default(), &secrets);

    assert_no_piece_of(&dir, contents, PIECE + 6);
    let events = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    assert!(events.contains(path));
}

#[test]
fn nothing_seeded_means_nothing_changes() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    let captured = driver.stop();

    let scrubbed = scrub(captured.clone(), Form::default(), &Secrets::new()).expect("scrubs");

    assert_eq!(scrubbed.captured(), &captured);
}

#[test]
fn a_change_that_doesnt_fit_its_tree_is_refused_and_nothing_is_written() {
    // Read back from a file whose state change was edited by hand.
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    let scrubbed = scrub(driver.stop(), Form::default(), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = write(reports.path(), &scrubbed).expect("writes");
    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    let edited = text.replace("\"removed\":[\"    draft: \\\"\\\",\"]", "\"removed\":[\"    draft: \\\"tea\\\",\"]");
    assert_ne!(edited, text, "the action's change removes the empty draft");
    let read = humata2_report::read_events(&edited).expect("still reads");

    let refused = scrub(read, Form::default(), &Secrets::new());

    assert!(matches!(refused, Err(ScrubError::Mismatch { tree: "state", .. })));
    assert_eq!(files(&dir).len(), 6, "the earlier report is untouched");
}
