//! Who may call the daemon (SEC-9): the release daemon refuses a call with
//! no credential except WhoAmI, the admin test daemon run with `--admin`
//! takes one for the user, and a session token works on either.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::{Code, Request, Status};

const SESSION: &str = "se_auth";
const TOKEN_HEADER: &str = "humata-session-token";

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

async fn who_am_i(daemon: &Daemon, request: Request<pb::WhoAmIRequest>) -> pb::WhoAmIResponse {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .who_am_i(request)
        .await
        .unwrap()
        .into_inner()
}

/// Refused because the call carried no credential.
fn assert_unauthenticated(refused: &Status) {
    assert_eq!(refused.code(), Code::Unauthenticated, "{refused:?}");
    let error = error_of(refused);
    assert!(
        matches!(error.kind, Some(pb::error::Kind::PresenceRequired(_))),
        "{error:?}"
    );
    assert!(!error.fix.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_release_daemon_refuses_a_call_with_no_credential() {
    let daemon = Daemon::start_build(Build::Release, folder()).await;

    let list = daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest::default())
        .await
        .unwrap_err();
    let shell = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap_err();
    let info = pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap_err();
    // Refused before the call learns the service isn't built.
    let not_built = pb::channel_service_client::ChannelServiceClient::new(daemon.channel().await)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap_err();

    for refused in [&list, &shell, &info, &not_built] {
        assert_unauthenticated(refused);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn whoami_with_no_credential_answers_with_no_actor() {
    let daemon = Daemon::start_build(Build::Release, folder()).await;

    let me = who_am_i(&daemon, Request::new(pb::WhoAmIRequest {})).await;

    assert_eq!(me.actor, None);
    assert_eq!(me.channel, None);
    assert!(!me.presence_possible);
}

#[tokio::test(flavor = "multi_thread")]
async fn on_an_admin_daemon_a_call_with_no_credential_is_the_user() {
    let daemon = Daemon::start().await;

    let me = who_am_i(&daemon, Request::new(pb::WhoAmIRequest {})).await;
    let tabs = daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest::default())
        .await;

    let Some(pb::actor::Actor::User(user)) = me.actor.and_then(|actor| actor.actor) else {
        panic!("expected the user");
    };
    assert_eq!(user.via(), pb::user_actor::Via::CliOnHost);
    assert!(tabs.is_ok(), "{tabs:?}");
}

/// The release daemon has no way to open a tab without the user, so an
/// admin daemon opens one and hands its state, token included, to a
/// release daemon.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_token_works_on_the_release_daemon() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_auth".to_owned(),
            machine: String::new(),
            title: "auth".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let mut daemon = Daemon::start_build(Build::Admin, dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session()),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join("token");
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    let token = read_when_written(&file).await;
    drop(attach);

    daemon.restart_as(Build::Release).await;
    let me = who_am_i(&daemon, as_agent(pb::WhoAmIRequest {}, &token)).await;
    let tabs = daemon
        .terminals()
        .await
        .list_tabs(as_agent(
            pb::ListTabsRequest {
                session: Some(session()),
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner()
        .tabs;

    let Some(pb::actor::Actor::Agent(agent)) = me.actor.and_then(|actor| actor.actor) else {
        panic!("expected the session's agent");
    };
    assert_eq!(agent.session, Some(session()));
    assert!(tabs.iter().any(|listed| listed.id.as_ref() == Some(&tab)));
}

/// `path`'s contents once something non-empty is written there.
async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(Instant::now() < deadline, "{} was never written", path.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
