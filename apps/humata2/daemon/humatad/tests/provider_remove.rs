//! PROVIDERS against the real daemon: ListProviders shows each provider's
//! commands as written, the host variables they read (set or not, never a
//! value) and the machines it made; RemoveProvider takes an imported
//! provider out, and refuses a built-in one, one machines still use, and an
//! agent.
//!
//! No provider command runs here.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

/// Two imported providers; `cloud` reads a host variable.
const FILE: &str = "[providers.cloud]\nlabel = \"Cloud\"\ncreate = 'make-cloud {name}'\nconnect = 'reach-cloud {name} -- {cmd}'\nenv = [\"CLOUD_TOKEN\"]\n\
     [providers.spare]\nlabel = \"Spare\"\ncreate = 'make-spare {name}'\nconnect = 'reach-spare {name} -- {cmd}'\n";

async fn providers(daemon: &Daemon) -> pb::provider_service_client::ProviderServiceClient<Channel> {
    pb::provider_service_client::ProviderServiceClient::new(daemon.channel().await)
}

async fn save(daemon: &Daemon, toml: &str) {
    let mut client = providers(daemon).await;
    let revision = client
        .get_providers_config(pb::GetProvidersConfigRequest::default())
        .await
        .unwrap()
        .into_inner()
        .revision;
    let saved = client
        .set_providers_config(pb::SetProvidersConfigRequest {
            idempotency_key: None,
            toml: toml.to_owned(),
            expected_revision: revision,
            presence: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(saved.problems.is_empty(), "{:?}", saved.problems);
}

async fn listed(daemon: &Daemon) -> Vec<pb::Provider> {
    providers(daemon)
        .await
        .list_providers(pb::ListProvidersRequest {})
        .await
        .unwrap()
        .into_inner()
        .providers
}

fn find<'a>(providers: &'a [pb::Provider], id: &str) -> Option<&'a pb::Provider> {
    providers
        .iter()
        .find(|provider| provider.id.as_ref().is_some_and(|listed| listed.value == id))
}

fn remove_request(id: &str) -> pb::RemoveProviderRequest {
    pb::RemoveProviderRequest {
        idempotency_key: None,
        provider: Some(pb::ProviderId { value: id.to_owned() }),
        presence: None,
    }
}

async fn remove(daemon: &Daemon, id: &str) -> Result<pb::SetProvidersConfigResponse, tonic::Status> {
    providers(daemon)
        .await
        .remove_provider(remove_request(id))
        .await
        .map(tonic::Response::into_inner)
}

/// An admin daemon whose `hosts.json` already has machines `cloud` made.
async fn daemon_with_machines(names: &[&str]) -> Daemon {
    let dir = folder();
    let state = state_folder(dir.path());
    std::fs::write(state.join("providers.toml"), FILE).unwrap();
    let machines: Vec<String> = names
        .iter()
        .map(|name| {
            format!(
                r#"{{"name":"{name}","ssh_host":"{name}","root":"~/Humata","state":"ready","provider":{{"id":"cloud","params":{{}},"link":null}}}}"#
            )
        })
        .collect();
    std::fs::write(
        state.join("hosts.json"),
        format!(r#"{{"version":1,"machines":[{}]}}"#, machines.join(",")),
    )
    .unwrap();
    for file in ["providers.toml", "hosts.json"] {
        std::fs::set_permissions(state.join(file), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    Daemon::start_with(Build::Admin, dir, &[]).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_shows_commands_variables_and_machines_of_every_provider() {
    let daemon = daemon_with_machines(&["box-b", "box-a"]).await;
    let before = listed(&daemon).await;

    // A built-in provider brings its commands too.
    let lima = find(&before, "lima").expect("lima is built in");
    assert!(lima.built_in);
    assert!(!lima.commands.is_empty());

    let cloud = find(&before, "cloud").expect("cloud is listed");
    assert!(cloud
        .commands
        .iter()
        .any(|command| command.action == "create" && command.command == "make-cloud {name}"));
    assert_eq!(
        cloud.variables,
        [pb::ProviderVariable { name: "CLOUD_TOKEN".to_owned(), set: false }]
    );
    let mut machines: Vec<_> = cloud.machines.iter().map(|machine| machine.value.clone()).collect();
    machines.sort();
    assert_eq!(machines, ["box-a", "box-b"]);
    assert!(find(&before, "spare").is_some_and(|spare| spare.machines.is_empty() && spare.variables.is_empty()));

    pb::machine_service_client::MachineServiceClient::new(daemon.channel().await)
        .set_host_variable(pb::SetHostVariableRequest {
            idempotency_key: None,
            id: None,
            name: "CLOUD_TOKEN".to_owned(),
            value: "tok-not-listed".to_owned(),
            presence: None,
        })
        .await
        .unwrap();
    let after = listed(&daemon).await;
    let cloud = find(&after, "cloud").unwrap();
    assert_eq!(
        cloud.variables,
        [pb::ProviderVariable { name: "CLOUD_TOKEN".to_owned(), set: true }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_imported_provider_keeps_the_others() {
    let daemon = Daemon::start().await;
    save(&daemon, FILE).await;

    let removed = remove(&daemon, "spare").await.unwrap();
    assert!(removed.problems.is_empty(), "{:?}", removed.problems);
    let now = listed(&daemon).await;
    assert!(find(&now, "spare").is_none());
    assert!(find(&now, "cloud").is_some());
    assert!(find(&now, "lima").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_built_in_or_unknown_provider_is_not_removed() {
    let daemon = Daemon::start().await;

    let built_in = remove(&daemon, "lima").await.unwrap_err();
    assert_eq!(built_in.code(), Code::FailedPrecondition, "{built_in:?}");
    assert!(matches!(error_of(&built_in).kind, Some(pb::error::Kind::InvalidField(_))));
    assert!(find(&listed(&daemon).await, "lima").is_some());

    let unknown = remove(&daemon, "nowhere").await.unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound, "{unknown:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_machines_still_use_is_not_removed_and_the_error_names_them() {
    let daemon = daemon_with_machines(&["box-a"]).await;

    let refused = remove(&daemon, "cloud").await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(refused.message().contains("box-a"), "{refused:?}");
    assert!(find(&listed(&daemon).await, "cloud").is_some());
}

const CHANNEL: &str = "ch-provider-remove";
const SESSION: &str = "s-provider-remove";
const TOKEN_HEADER: &str = "humata-session-token";

/// An admin daemon with one session, and the token its shell tab got.
async fn daemon_with_agent() -> (Daemon, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "Provider remove".to_owned(),
            folder: "provider-remove".to_owned(),
            branch_prefix: None,
            default_agent: "zsh".to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: CHANNEL.to_owned(),
            machine: String::new(),
            title: "provider remove".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_with(Build::Admin, dir, &[]).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId { value: SESSION.to_owned() }),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join("token");
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!("printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r", file.display()))
        .await;
    let deadline = Instant::now() + WAIT;
    let token = loop {
        match std::fs::read_to_string(&file) {
            Ok(text) if !text.is_empty() => break text,
            _ => {}
        }
        assert!(Instant::now() < deadline, "the token was never written");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    (daemon, token)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_can_not_remove_a_provider() {
    let (daemon, token) = daemon_with_agent().await;
    save(&daemon, FILE).await;

    let mut request = Request::new(remove_request("spare"));
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token.as_str()).unwrap());
    let refused = providers(&daemon).await.remove_provider(request).await.unwrap_err();
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_) | pb::error::Kind::Denied(_))),
        "{refused:?}"
    );
    assert!(find(&listed(&daemon).await, "spare").is_some());
}
