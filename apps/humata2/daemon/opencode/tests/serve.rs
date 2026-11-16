//! The wrapper's view of `opencode` (D201): the URL serve prints, which
//! versions have the server API, the session it names, and where replies go.
use humata2_opencode::{VersionError, listening_url, reply_path, session_id, supported};

#[test]
fn the_url_comes_from_serves_listening_line() {
    let printed = include_str!("../testdata/serve-port0-stdout.txt");
    let url = printed.lines().find_map(listening_url).expect("serve printed its URL");
    assert!(url.starts_with("http://127.0.0.1:"));
    assert_eq!(listening_url("loading config"), None);
    // Anything but loopback is refused.
    assert_eq!(listening_url("server listening on http://0.0.0.0:4096"), None);
}

#[test]
fn versions_from_2_3_have_the_server_api() {
    assert!(supported("opencode v2.3.0").is_ok());
    assert!(supported("opencode v2.10.1\n").is_ok());
    assert!(supported("3.0.0").is_ok());
    assert!(matches!(supported("opencode v2.2.7"), Err(VersionError::TooOld { .. })));
    assert!(matches!(supported("opencode v1.19.5"), Err(VersionError::TooOld { .. })));
    assert!(matches!(supported("command not found"), Err(VersionError::Unreadable { .. })));
}

#[test]
fn a_conversation_names_one_opencode_session() {
    let conversation = "0f8a1c2e-7b3d-4e5f-9a6b-1c2d3e4f5a6b";
    let id = session_id(conversation);
    // OpenCode keeps an id it is given only when it starts `ses`.
    assert!(id.starts_with("ses_"));
    assert!(id["ses_".len()..].chars().all(|c| c.is_ascii_alphanumeric()));
    assert_eq!(id, session_id(conversation));
    assert_ne!(id, session_id("9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b"));
}

#[test]
fn a_reply_goes_to_its_asks_own_session() {
    assert_eq!(
        reply_path("ses_1", "per_2"),
        "/api/session/ses_1/permission/per_2/reply"
    );
}
