//! MP-23 provider import against the real daemon: a file the user picked is
//! previewed with its full text and every command it will run, nothing is
//! added until ImportProvider uses the plan, a plan is good once and only
//! while the providers file is as it was, and agents can do neither.
//!
//! No provider command runs here: importing only saves the file.

mod common;

use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

/// What the providers file holds before an import.
const EXISTING: &str = "[providers.kept]\nlabel = \"Kept\"\ncreate = 'make-kept {name}'\nconnect = 'reach-kept {name} -- {cmd}'\n\
     [providers.other]\nlabel = \"Other\"\ncreate = 'make-other {name}'\nconnect = 'reach-other {name} -- {cmd}'\n";

/// A file to import: one new provider, and one that replaces `kept`.
const IMPORTED: &str = "# A provider file the user picked.\n\
     [providers.fresh]\nlabel = \"Fresh\"\ncreate = 'make-fresh {name}'\nconnect = 'reach-fresh {name} -- {cmd}'\nstart = 'wake-fresh {name}'\n\
     [providers.kept]\nlabel = \"Kept again\"\ncreate = 'make-kept-again {name}'\nconnect = 'reach-kept-again {name} -- {cmd}'\n";

async fn providers(daemon: &Daemon) -> pb::provider_service_client::ProviderServiceClient<Channel> {
    pb::provider_service_client::ProviderServiceClient::new(daemon.channel().await)
}

async fn config(daemon: &Daemon) -> pb::ProvidersConfig {
    providers(daemon)
        .await
        .get_providers_config(pb::GetProvidersConfigRequest::default())
        .await
        .unwrap()
        .into_inner()
}

async fn save(daemon: &Daemon, toml: &str) {
    let revision = config(daemon).await.revision;
    let saved = providers(daemon)
        .await
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

async fn preview(daemon: &Daemon, file: &[u8]) -> Result<pb::ProviderImportPlan, tonic::Status> {
    providers(daemon)
        .await
        .preview_provider_import(pb::PreviewProviderImportRequest {
            file: file.to_vec(),
            file_name: "picked.toml".to_owned(),
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn import(daemon: &Daemon, plan: &pb::ProviderImportPlan) -> Result<pb::SetProvidersConfigResponse, tonic::Status> {
    providers(daemon)
        .await
        .import_provider(pb::ImportProviderRequest {
            idempotency_key: None,
            plan: plan.token.clone(),
            presence: None,
        })
        .await
        .map(tonic::Response::into_inner)
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

fn id(value: &str) -> Option<pb::ProviderId> {
    Some(pb::ProviderId { value: value.to_owned() })
}

fn label_of(providers: &[pb::Provider], provider: &str) -> Option<String> {
    providers
        .iter()
        .find(|listed| listed.id == id(provider))
        .map(|listed| listed.label.clone())
}

fn has_command(plan: &pb::ProviderImportPlan, provider: &str, action: &str, command: &str) -> bool {
    plan.commands
        .iter()
        .any(|shown| shown.provider == id(provider) && shown.action == action && shown.command == command)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_previewed_file_shows_its_text_and_commands_and_is_added_only_on_import() {
    let daemon = Daemon::start().await;
    save(&daemon, EXISTING).await;
    let before = config(&daemon).await;

    let plan = preview(&daemon, IMPORTED.as_bytes()).await.unwrap();
    assert_eq!(plan.text, IMPORTED);
    assert!(plan.problems.is_empty(), "{:?}", plan.problems);
    assert!(plan.token.as_ref().is_some_and(|token| !token.value.is_empty()));
    let previewed: Vec<_> = plan.providers.iter().filter_map(|provider| provider.id.clone()).collect();
    assert!(previewed.contains(&id("fresh").unwrap()) && previewed.contains(&id("kept").unwrap()));
    assert_eq!(previewed.len(), 2);
    assert!(has_command(&plan, "fresh", "create", "make-fresh {name}"));
    assert!(has_command(&plan, "fresh", "connect", "reach-fresh {name} -- {cmd}"));
    assert!(has_command(&plan, "fresh", "start", "wake-fresh {name}"));
    assert!(has_command(&plan, "kept", "create", "make-kept-again {name}"));
    assert_eq!(plan.replaces, vec![id("kept").unwrap()]);

    // The preview added nothing.
    assert_eq!(config(&daemon).await, before);
    assert_eq!(label_of(&listed(&daemon).await, "fresh"), None);

    let imported = import(&daemon, &plan).await.unwrap();
    assert!(imported.problems.is_empty(), "{:?}", imported.problems);
    let after = config(&daemon).await;
    assert_eq!(imported.revision, after.revision);
    assert_ne!(after.revision, before.revision);

    let now = listed(&daemon).await;
    assert_eq!(label_of(&now, "fresh").as_deref(), Some("Fresh"));
    assert_eq!(label_of(&now, "kept").as_deref(), Some("Kept again"));
    assert_eq!(label_of(&now, "other").as_deref(), Some("Other"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_replaces_the_built_in_lima_says_so() {
    let daemon = Daemon::start().await;
    let file = "[providers.lima]\nlabel = \"My Lima\"\ncreate = 'limactl create {name}'\nconnect = 'limactl shell {name} {cmd}'\n";

    let plan = preview(&daemon, file.as_bytes()).await.unwrap();

    assert_eq!(plan.replaces, vec![id("lima").unwrap()]);
    import(&daemon, &plan).await.unwrap();
    assert_eq!(label_of(&listed(&daemon).await, "lima").as_deref(), Some("My Lima"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_with_problems_shows_them_and_cannot_be_imported() {
    let daemon = Daemon::start().await;

    let broken = preview(&daemon, b"[providers.broken]\nstart = \"true\"\n").await.unwrap();
    assert!(broken.problems.iter().any(|problem| problem.field == "create"), "{:?}", broken.problems);
    assert!(broken.token.as_ref().is_none_or(|token| token.value.is_empty()));
    assert_eq!(config(&daemon).await.toml, "");

    let not_text = preview(&daemon, &[0xff, 0xfe, 0x00]).await.unwrap_err();
    assert_eq!(not_text.code(), Code::InvalidArgument);

    let empty = preview(&daemon, b"").await.unwrap();
    assert!(!empty.problems.is_empty() || empty.token.as_ref().is_none_or(|token| token.value.is_empty()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_is_good_once_and_only_while_the_providers_are_as_it_saw_them() {
    let daemon = Daemon::start().await;
    save(&daemon, EXISTING).await;

    // The providers changed after the preview: the plan is stale and adds nothing.
    let stale = preview(&daemon, IMPORTED.as_bytes()).await.unwrap();
    save(&daemon, "[providers.other]\nlabel = \"Other\"\ncreate = 'make-other {name}'\nconnect = 'reach-other {name} -- {cmd}'\n").await;
    let refused = import(&daemon, &stale).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::PlanStale(_))), "{refused:?}");
    assert_eq!(label_of(&listed(&daemon).await, "fresh"), None);

    // A fresh plan imports once; its token is spent after that.
    let plan = preview(&daemon, IMPORTED.as_bytes()).await.unwrap();
    import(&daemon, &plan).await.unwrap();
    let again = import(&daemon, &plan).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition);

    let unknown = import(
        &daemon,
        &pb::ProviderImportPlan {
            token: Some(pb::PlanToken { value: "pl_unknown".to_owned() }),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(unknown.code(), Code::FailedPrecondition);
}

const SESSION: &str = "se_provider_import";
const TOKEN_HEADER: &str = "humata-session-token";

/// An admin daemon with one session, and the token its shell tab got.
async fn daemon_with_agent() -> (Daemon, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_provider_import".to_owned(),
            machine: String::new(),
            title: "provider import".to_owned(),
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

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_can_neither_preview_nor_import_a_provider() {
    let (daemon, token) = daemon_with_agent().await;
    let mut client = providers(&daemon).await;

    let previewed = client
        .preview_provider_import(as_agent(
            pb::PreviewProviderImportRequest {
                file: IMPORTED.as_bytes().to_vec(),
                file_name: "picked.toml".to_owned(),
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(error_of(&previewed).kind, Some(pb::error::Kind::UserOnly(_) | pb::error::Kind::Denied(_))),
        "{previewed:?}"
    );

    // Not even with a plan the user made.
    let plan = preview(&daemon, IMPORTED.as_bytes()).await.unwrap();
    let imported = client
        .import_provider(as_agent(
            pb::ImportProviderRequest {
                idempotency_key: None,
                plan: plan.token.clone(),
                presence: None,
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(error_of(&imported).kind, Some(pb::error::Kind::UserOnly(_) | pb::error::Kind::Denied(_))),
        "{imported:?}"
    );
    assert_eq!(config(&daemon).await.toml, "");
}
