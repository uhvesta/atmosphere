//! Who may call the daemon (SEC-9): the release daemon refuses a call with
//! no credential except WhoAmI, the admin test daemon run with `--admin`
//! takes one for the user, and a session token works on either.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, error_of, folder, humata_in_tab, open, shell_token, state_folder,
};
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
    // Under --admin the token-less caller is the app's authority (D41), so
    // whoami reads the same via the recordings use.
    assert_eq!(user.via(), pb::user_actor::Via::App);
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
    read_when_written(&file).await;
    // The release daemon takes no attach without a credential, so the shell
    // runs a script that waits to call humata until the daemon has
    // restarted. The script says it is running before the attach goes, so
    // the typed line can't be lost with the attach.
    let ready = daemon.dir.path().join("ready");
    let go = daemon.dir.path().join("go");
    let out = daemon.dir.path().join("whoami.json");
    let part = daemon.dir.path().join("whoami.part");
    let script = daemon.dir.path().join("whoami.sh");
    std::fs::write(
        &script,
        format!(
            "touch '{ready}'\nwhile [ ! -e '{go}' ]; do sleep 0.1; done\nHUMATA_SOCKET='{socket}' '{humata}' --json whoami > '{part}' 2>/dev/null\nmv '{part}' '{out}'\n",
            ready = ready.display(),
            go = go.display(),
            socket = daemon.socket.display(),
            humata = common::binary("HUMATA").display(),
            part = part.display(),
            out = out.display(),
        ),
    )
    .unwrap();
    attach
        .type_text(&format!("/bin/sh '{}'\r", script.display()))
        .await;
    common::wait_for_path(&ready).await;
    drop(attach);

    daemon.restart_as(Build::Release).await;
    // The token is good in its own tab's tree, on the release daemon too.
    std::fs::write(&go, "").unwrap();
    common::wait_for_path(&out).await;
    let text = std::fs::read_to_string(&out).unwrap();
    let me: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

    assert_eq!(me["result"]["tier"], "agent", "{me}");
}

fn seed_session(dir: &Path) {
    let store = Store::open(state_folder(dir).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_auth".to_owned(),
            machine: String::new(),
            title: "auth".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
}

/// SEC-15: a session's token from outside its tab's process tree is
/// refused, and the same token inside the tab works.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_token_used_outside_its_tab_is_refused_and_works_inside_it() {
    let dir = folder();
    seed_session(dir.path());
    let daemon = Daemon::start_build(Build::AdminForThisProcess, dir).await;
    let (token, mut attach) = shell_token(&daemon, &session()).await;

    let inside = humata_in_tab(&daemon, &mut attach, "whoami").await;
    let refused = daemon
        .terminals()
        .await
        .list_tabs(as_agent(
            pb::ListTabsRequest {
                session: Some(session()),
            },
            &token,
        ))
        .await
        .unwrap_err();
    let after = humata_in_tab(&daemon, &mut attach, "whoami").await;

    let error = error_of(&refused);
    assert!(
        matches!(error.kind, Some(pb::error::Kind::IdentityMismatch(_))),
        "{refused:?}"
    );
    assert!(!error.fix.is_empty());
    assert_eq!(inside["result"]["tier"], "agent", "{inside}");
    // Misuse revoked the token, so even the session's own shell is refused
    // until the session resumes and gets a new one.
    assert_ne!(after["result"]["tier"], "agent", "{after}");
}

/// `path`'s contents once something non-empty is written there.
async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "{} was never written",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
