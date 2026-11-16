//! The state file the wrapper leaves for the daemon (D201): it round-trips,
//! only its owner can read it, and a missing or broken one says so.
use humata2_opencode::{StateError, StateFile};
use std::os::unix::fs::PermissionsExt;

fn sample() -> StateFile {
    StateFile {
        url: "http://127.0.0.1:55444".to_owned(),
        password: "secret".to_owned(),
        session: "ses_e1c537029ffejTdBfVrnGEG0jm".to_owned(),
        serve_pid: 4242,
    }
}

#[test]
fn it_round_trips_and_only_its_owner_can_read_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("opencode.json");
    sample().write(&path).expect("write the state file");
    assert_eq!(StateFile::read(&path).expect("read it back"), sample());
    let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // Writing again replaces it whole.
    let mut next = sample();
    next.serve_pid = 7;
    next.write(&path).expect("write again");
    assert_eq!(StateFile::read(&path).expect("read it back").serve_pid, 7);
}

#[test]
fn a_missing_or_broken_file_says_which() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("opencode.json");
    assert!(matches!(StateFile::read(&path), Err(StateError::Missing { .. })));
    std::fs::write(&path, "{").expect("write");
    assert!(matches!(StateFile::read(&path), Err(StateError::Broken { .. })));
    // A file from a newer wrapper isn't guessed at.
    std::fs::write(&path, r#"{"version":99,"url":"u","password":"p","session":"s","serve_pid":1}"#)
        .expect("write");
    assert!(matches!(StateFile::read(&path), Err(StateError::Broken { .. })));
}
