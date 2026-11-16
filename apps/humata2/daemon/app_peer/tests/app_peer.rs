use humata2_app_peer::{incoming, Requirement};
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::StreamExt;
use tonic::transport::server::Connected;

#[test]
fn requirement_needs_a_well_formed_team_and_bundle_id() {
    assert!(Requirement::app("ABCDE12345", "com.example.humata2").is_some());
    assert!(Requirement::app("", "com.example.humata2").is_none());
    assert!(Requirement::app("ABCDE1234", "com.example.humata2").is_none());
    assert!(Requirement::app("abcde12345", "com.example.humata2").is_none());
    assert!(Requirement::app("ABCDE12345", "").is_none());
    assert!(Requirement::app("ABCDE12345", "com.example\" or true").is_none());
    assert!(Requirement::app("ABCDE12345", "com example").is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn peer_token_names_the_peer_process() {
    use humata2_app_peer::AuditToken;
    use std::os::fd::AsFd;
    let (a, _b) = std::os::unix::net::UnixStream::pair().expect("socketpair");
    let token = AuditToken::of_peer(a.as_fd()).expect("peer token");
    assert_eq!(token.pid(), std::process::id() as i32);
}

#[cfg(target_os = "macos")]
#[test]
fn this_test_binary_is_not_another_team_s_app() {
    use humata2_app_peer::{check, AuditToken};
    use std::os::fd::AsFd;
    let (a, _b) = std::os::unix::net::UnixStream::pair().expect("socketpair");
    let token = AuditToken::of_peer(a.as_fd()).expect("peer token");
    let requirement = Requirement::app("ABCDE12345", "com.example.humata2").expect("requirement");
    assert!(check(&token, &requirement).is_err());
}

async fn accept_one(requirement: Option<Requirement>) -> humata2_app_peer::PeerInfo {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let mut stream = Box::pin(incoming(listener, requirement));
    let _client = UnixStream::connect(&path).await.expect("connect");
    let accepted = stream.next().await.expect("a connection").expect("accepted");
    accepted.connect_info()
}

#[tokio::test]
async fn nothing_is_the_app_without_a_requirement() {
    let info = accept_one(None).await;
    assert!(!info.app);
    let cred = info.uds.peer_cred.expect("peer credentials");
    assert_eq!(cred.pid(), Some(std::process::id() as i32));
}

#[tokio::test]
async fn a_peer_that_fails_the_check_is_still_served_but_not_as_the_app() {
    let info = accept_one(Requirement::app("ABCDE12345", "com.example.humata2")).await;
    assert!(!info.app);
    let cred = info.uds.peer_cred.expect("peer credentials");
    assert_eq!(cred.pid(), Some(std::process::id() as i32));
}
