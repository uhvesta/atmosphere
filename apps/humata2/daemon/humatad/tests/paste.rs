//! PasteImage: an image saved on the session's machine for the agent to
//! read, at a path to type into the tab.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Daemon, folder, invalid_field, not_found, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use pb::invalid_field::Problem;

const SESSION: &str = "se_paste";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// A daemon that knows one session, `SESSION`.
async fn daemon_with_session() -> Daemon {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "paste".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    Daemon::start_in(dir).await
}

fn request(session: &str, format: pb::ImageFormat, data: Vec<u8>) -> pb::PasteImageRequest {
    pb::PasteImageRequest {
        idempotency_key: None,
        session: Some(pb::SessionId {
            value: session.to_owned(),
        }),
        data,
        format: format as i32,
    }
}

async fn paste(
    daemon: &Daemon,
    request: pb::PasteImageRequest,
) -> Result<pb::PasteImageResponse, tonic::Status> {
    daemon
        .terminals()
        .await
        .paste_image(request)
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_image_is_saved_privately_where_the_path_says() {
    let daemon = daemon_with_session().await;

    let pasted = paste(&daemon, request(SESSION, pb::ImageFormat::Png, PNG.to_vec()))
        .await
        .unwrap();

    let path = Path::new(&pasted.path);
    assert!(path.is_absolute());
    assert!(path.starts_with(std::fs::canonicalize(&daemon.state).unwrap()));
    assert_eq!(path.extension().and_then(|extension| extension.to_str()), Some("png"));
    assert_eq!(std::fs::read(path).unwrap(), PNG);
    assert_eq!(pasted.size_bytes, PNG.len() as u64);
    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retry_with_the_same_key_saves_once() {
    let daemon = daemon_with_session().await;
    let keyed = || pb::PasteImageRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: "paste-1".to_owned(),
        }),
        ..request(SESSION, pb::ImageFormat::Png, PNG.to_vec())
    };

    let first = paste(&daemon, keyed()).await.unwrap();
    let retry = paste(&daemon, keyed()).await.unwrap();

    assert_eq!(first.path, retry.path);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_over_the_limit_is_refused() {
    let daemon = daemon_with_session().await;
    let mut huge = PNG.to_vec();
    huge.resize((32 << 20) + 1, 0);

    let refused = paste(&daemon, request(SESSION, pb::ImageFormat::Png, huge))
        .await
        .unwrap_err();

    assert_eq!(invalid_field(&refused), ("data".to_owned(), Problem::TooLong));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_format_is_refused() {
    let daemon = daemon_with_session().await;

    let unset = paste(&daemon, request(SESSION, pb::ImageFormat::Unspecified, PNG.to_vec()))
        .await
        .unwrap_err();
    let unknown = paste(
        &daemon,
        pb::PasteImageRequest {
            format: 999,
            ..request(SESSION, pb::ImageFormat::Png, PNG.to_vec())
        },
    )
    .await
    .unwrap_err();

    assert_eq!(invalid_field(&unset), ("format".to_owned(), Problem::UnknownValue));
    assert_eq!(invalid_field(&unknown), ("format".to_owned(), Problem::UnknownValue));
}

#[tokio::test(flavor = "multi_thread")]
async fn bytes_that_are_not_the_format_named_are_refused() {
    let daemon = daemon_with_session().await;

    let refused = paste(&daemon, request(SESSION, pb::ImageFormat::Jpeg, PNG.to_vec()))
        .await
        .unwrap_err();

    assert_eq!(invalid_field(&refused), ("data".to_owned(), Problem::BadFormat));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paste_into_an_unknown_session_is_not_found() {
    let daemon = daemon_with_session().await;

    let refused = paste(&daemon, request("se_missing", pb::ImageFormat::Png, PNG.to_vec()))
        .await
        .unwrap_err();

    assert_eq!(not_found(&refused), pb::EntityKind::Session);
}
