//! ProviderService against the real daemon: the providers file (revision,
//! validation), creating a machine through a provider over a fake `ssh`
//! (no network, never the real ~/.ssh), a provider machine's actions and
//! delete, a failed create and Delete what was made, and agents seeing no
//! providers.

use humatad_test_common as common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, binary, connect, error_of, folder, open, state_folder};
use humata2_fake_machine::{FakeMachine, HOST_KEY};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const FILES: [&str; 4] = ["humatad2", "humata2_holder", "humata", "stack"];

/// A tools bundle for linux-x86_64, the fake machine's default platform.
fn bundle(dir: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            // A link a test starts gets its state folder named (the default, said out
            // loud), so it relays nothing and starts no daemon of its own (D146).
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" --stdio \"*) exec '{real}' \"$@\" --state \"$HOME/.humata2/state\" ;; esac\nexec '{real}' \"$@\"\n",
                real = binary("HUMATAD2_RELEASE").display()
            )
        } else {
            format!("#!/bin/sh\n# {file}\nexit 0\n")
        };
        std::fs::write(root.join("linux-x86_64").join(file), &body).unwrap();
        entries.insert(
            file.to_owned(),
            serde_json::json!({
                "path": format!("linux-x86_64/{file}"),
                "sha256": Sha256::digest(body.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            }),
        );
    }
    let manifest = serde_json::json!({ "platforms": { "linux-x86_64": entries } });
    std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
    root
}

/// The admin daemon with the fake machine's `ssh` first on its PATH.
struct Host {
    child: Child,
    socket: PathBuf,
    dir: TempDir,
}

impl Host {
    async fn start(machine: &FakeMachine) -> Self {
        let dir = folder();
        let socket = dir.path().join("d.sock");
        let tools = bundle(dir.path());
        let child = Command::new(binary("HUMATAD2"))
            .arg("serve")
            .arg("--admin")
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(dir.path().join("state"))
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--tools")
            .arg(&tools)
            .arg("--ssh-config")
            .arg(dir.path().join("ssh_config"))
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self { child, socket, dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    async fn providers(&self) -> pb::provider_service_client::ProviderServiceClient<Channel> {
        pb::provider_service_client::ProviderServiceClient::new(connect(&self.socket).await)
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(connect(&self.socket).await)
    }

    async fn watch(&self, operation: &pb::Operation) -> pb::Operation {
        let mut stream = pb::operation_service_client::OperationServiceClient::new(connect(&self.socket).await)
            .watch_operation(pb::WatchOperationRequest {
                id: operation.id.clone(),
                from_version: 0,
            })
            .await
            .unwrap()
            .into_inner();
        let mut last = None;
        while let Some(next) = tokio::time::timeout(Duration::from_secs(60), stream.message())
            .await
            .expect("the operation finished")
            .unwrap()
        {
            last = Some(next);
        }
        last.expect("the operation was watched")
    }

    async fn list(&self) -> Vec<pb::Machine> {
        self.machines()
            .await
            .list_machines(pb::ListMachinesRequest::default())
            .await
            .unwrap()
            .into_inner()
            .machines
    }

    /// box-1 once its peer link is up.
    async fn linked(&self) -> pb::Machine {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(machine) = self.list().await.into_iter().find(|machine| machine.name == "box-1")
                && !machine.daemon_version.is_empty()
            {
                return machine;
            }
            assert!(Instant::now() < deadline, "the peer link to box-1 never came up");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn config(&self) -> pb::ProvidersConfig {
        self.providers()
            .await
            .get_providers_config(pb::GetProvidersConfigRequest::default())
            .await
            .unwrap()
            .into_inner()
    }

    /// Saves `toml` over the revision the daemon has.
    async fn save(&self, toml: &str) -> pb::SetProvidersConfigResponse {
        let revision = self.config().await.revision;
        self.providers()
            .await
            .set_providers_config(pb::SetProvidersConfigRequest {
                idempotency_key: None,
                toml: toml.to_owned(),
                expected_revision: revision,
                presence: None,
            })
            .await
            .unwrap()
            .into_inner()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn provider_id(id: &str) -> Option<pb::ProviderId> {
    Some(pb::ProviderId { value: id.to_owned() })
}

fn machine_id() -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: "box-1".to_owned(),
    })
}

fn create_request(provider: &str, params: &[(&str, &str)], key: &str) -> pb::CreateMachineRequest {
    pb::CreateMachineRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        provider: provider_id(provider),
        name: "box-1".to_owned(),
        params: params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
    }
}

fn state_of(operation: &pb::Operation) -> pb::OperationState {
    pb::OperationState::try_from(operation.state).unwrap()
}

fn step_state(operation: &pb::Operation, key: &str) -> pb::StepState {
    let step = operation.steps.iter().find(|step| step.key == key).unwrap();
    pb::StepState::try_from(step.state).unwrap()
}

fn action_request(action: pb::ProviderAction, key: &str) -> pb::RunProviderActionRequest {
    pb::RunProviderActionRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        machine: machine_id(),
        action: action as i32,
        remove: None,
        presence: None,
    }
}

/// A provider that makes a marker file, says its identity from a file, and
/// defines status, start and delete but no stop; and a second one, never
/// run, that names a host variable.
fn identity_toml(host: &Host) -> String {
    let dir = host.dir.path().display();
    std::fs::write(
        host.path("identity.json"),
        format!(
            r#"{{"name":"box-1","ssh":{{"host":"box-1.internal","user":"me","port":22,"host_key":"{HOST_KEY}"}},"os":"linux","arch":"x86_64"}}"#
        ),
    )
    .unwrap();
    format!(
        "[providers.fake]\nlabel = \"Fake\"\nparams = [{{ name = \"zone\", prompt = \"Zone\" }}]\n\
         create = 'touch {dir}/created'\nidentity = 'cat {dir}/identity.json'\nstatus = 'echo stopped'\nstart = 'true'\n\
         delete = 'rm -f {dir}/created'\n\
         [providers.tokened]\nlabel = \"Tokened\"\nenv = [\"FAKE_TOKEN\"]\n\
         create = 'make-machine {{name}}'\nconnect = 'reach-machine {{name}} -- {{cmd}}'\n"
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_providers_file_is_validated_and_saved_against_its_revision() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let empty = host.config().await;
    assert_eq!(empty.toml, "");
    assert_eq!(empty.revision.unwrap().value, 0);

    // A file with problems saves nothing and names the field.
    let refused = host.save("[providers.broken]\nstart = \"true\"\n").await;
    assert!(!refused.problems.is_empty());
    assert!(refused.problems.iter().any(|problem| problem.field == "connect"));
    assert_eq!(host.config().await, empty);

    let text = identity_toml(&host);
    let saved = host.save(&text).await;
    assert!(saved.problems.is_empty(), "{:?}", saved.problems);
    let config = host.config().await;
    assert_eq!(config.toml, text);
    assert_eq!(config.revision, saved.revision);
    assert_ne!(config.revision.as_ref().unwrap().value, 0);

    // A writer that read the old revision is refused and changes nothing.
    let stale = host
        .providers()
        .await
        .set_providers_config(pb::SetProvidersConfigRequest {
            idempotency_key: None,
            toml: String::new(),
            expected_revision: empty.revision,
            presence: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error_of(&stale).kind, Some(pb::error::Kind::RevisionConflict(_))));
    assert_eq!(host.config().await.toml, text);
}

#[tokio::test(flavor = "multi_thread")]
async fn providers_are_listed_with_their_params_and_names_of_variables_and_the_lima_example() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let text = identity_toml(&host);
    host.save(&text).await;

    let listed = host
        .providers()
        .await
        .list_providers(pb::ListProvidersRequest {})
        .await
        .unwrap()
        .into_inner()
        .providers;

    let fake = listed.iter().find(|provider| provider.id == provider_id("fake")).unwrap();
    assert_eq!(fake.label, "Fake");
    assert_eq!(fake.params[0].name, "zone");
    // A provider's host variables are listed by name only; the daemon has
    // no host variables to give one yet, so only `tokened` (never run
    // here) names one.
    let tokened = listed.iter().find(|provider| provider.id == provider_id("tokened")).unwrap();
    assert_eq!(tokened.env, ["FAKE_TOKEN"]);
    assert!(fake.env.is_empty());
    assert!(fake.has_create);
    assert_eq!(fake.reach(), pb::provider::Reach::Identity);
    assert!(!fake.built_in);
    let actions: Vec<_> = fake.actions().collect();
    assert!(actions.contains(&pb::ProviderAction::Status));
    assert!(!actions.contains(&pb::ProviderAction::Stop));
    let lima = listed.iter().find(|provider| provider.id == provider_id("lima")).unwrap();
    assert!(lima.built_in);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_is_created_through_its_provider_then_run_and_deleted_by_the_provider() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let text = identity_toml(&host);
    host.save(&text).await;

    // Params must be the declared ones, and are checked before anything runs.
    let undeclared = host
        .providers()
        .await
        .create_machine(create_request("fake", &[("zone", "a"), ("colour", "red")], "k0"))
        .await
        .unwrap_err();
    assert_eq!(undeclared.code(), Code::InvalidArgument);
    let missing = host
        .providers()
        .await
        .create_machine(create_request("fake", &[], "k0"))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::InvalidArgument);
    assert!(!host.path("created").exists());

    let creating = host
        .providers()
        .await
        .create_machine(create_request("fake", &[("zone", "a")], "k1"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        pb::OperationKind::try_from(creating.kind).unwrap(),
        pb::OperationKind::ProviderCreate
    );
    let last = host.watch(&creating).await;
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    for key in ["create", "identity", "connect", "check", "install", "start"] {
        assert_eq!(step_state(&last, key), pb::StepState::Done, "{key}");
    }
    assert_eq!(last.result, Some(pb::operation::Result::Machine(machine_id().unwrap())));
    assert!(host.path("created").exists());
    for file in FILES {
        assert!(machine.installed(file).is_some(), "{file} wasn't installed");
    }
    let listed = host.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].provider, provider_id("fake"));
    let detail = host
        .machines()
        .await
        .get_machine(pb::GetMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    let actions: Vec<_> = detail.actions().collect();
    assert!(actions.contains(&pb::ProviderAction::Status));
    assert!(actions.contains(&pb::ProviderAction::Delete));
    assert!(!actions.contains(&pb::ProviderAction::Stop));

    // Status says stopped, so the machine shows as stopped.
    host.linked().await;
    let status = host
        .providers()
        .await
        .run_provider_action(action_request(pb::ProviderAction::Status, "s1"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(state_of(&host.watch(&status).await), pb::OperationState::Succeeded);
    assert_eq!(host.list().await[0].reachability(), pb::Reachability::Stopped);

    // An action the provider doesn't define is refused.
    let refused = host
        .providers()
        .await
        .run_provider_action(action_request(pb::ProviderAction::Stop, "s2"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);

    // Delete removes the machine by its plan, then runs the provider's delete.
    let plan = host
        .machines()
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    let mut delete = action_request(pb::ProviderAction::Delete, "d1");
    delete.remove = Some(pb::RemoveMachineRequest {
        id: machine_id(),
        plan: plan.token,
        ..Default::default()
    });
    let deleting = host
        .providers()
        .await
        .run_provider_action(delete)
        .await
        .unwrap()
        .into_inner();
    let last = host.watch(&deleting).await;
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    assert!(host.list().await.is_empty());
    assert!(!host.path("created").exists(), "the provider's delete ran");
    assert!(!machine.unit_installed());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_create_that_fails_lists_nothing_and_what_it_made_is_deleted_on_request() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let dir = host.dir.path().display();
    host.save(&format!(
        "[providers.flaky]\ncreate = 'touch {dir}/created; echo boom >&2; exit 3'\nconnect = 'ssh {{cmd}}'\ndelete = 'rm -f {dir}/created'\n\
         [providers.nodelete]\ncreate = 'exit 3'\nconnect = 'ssh {{cmd}}'\n"
    ))
    .await;

    let creating = host
        .providers()
        .await
        .create_machine(create_request("flaky", &[], "k1"))
        .await
        .unwrap()
        .into_inner();
    let last = host.watch(&creating).await;
    assert_eq!(state_of(&last), pb::OperationState::Failed);
    assert_eq!(step_state(&last, "create"), pb::StepState::Failed);
    assert!(matches!(
        last.error.as_ref().and_then(|error| error.kind.clone()),
        Some(pb::error::Kind::ProviderStepFailed(_))
    ));
    assert!(host.list().await.is_empty());
    assert!(host.path("created").exists());

    let cleaning = host
        .providers()
        .await
        .delete_what_was_made(pb::DeleteWhatWasMadeRequest {
            idempotency_key: None,
            operation: creating.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let last = host.watch(&cleaning).await;
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    assert!(!host.path("created").exists());

    // Without a delete command there is nothing to run, and it says so.
    let stuck = host
        .providers()
        .await
        .create_machine(create_request("nodelete", &[], "k2"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(state_of(&host.watch(&stuck).await), pb::OperationState::Failed);
    let refused = host
        .providers()
        .await
        .delete_what_was_made(pb::DeleteWhatWasMadeRequest {
            idempotency_key: None,
            operation: stuck.id,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(host.list().await.is_empty());
}

const SESSION: &str = "se_providers";
const TOKEN_HEADER: &str = "humata-session-token";

/// An admin daemon with one session, and the token its shell tab got.
async fn daemon_with_agent() -> (Daemon, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_providers".to_owned(),
            machine: String::new(),
            title: "providers".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let tools = bundle(dir.path());
    let ssh_config = dir.path().join("ssh_config");
    let daemon = Daemon::start_with(
        Build::Admin,
        dir,
        &[
            "--tools",
            tools.to_str().unwrap(),
            "--ssh-config",
            ssh_config.to_str().unwrap(),
        ],
    )
    .await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
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
async fn an_agent_lists_providers_but_can_neither_change_the_file_nor_create_a_machine() {
    let (daemon, token) = daemon_with_agent().await;
    let mut providers = pb::provider_service_client::ProviderServiceClient::new(daemon.channel().await);

    let listed = providers
        .list_providers(as_agent(pb::ListProvidersRequest {}, &token))
        .await
        .unwrap()
        .into_inner();
    // Listing is read-only, so an agent sees the built-ins.
    assert!(listed.providers.iter().any(|provider| provider.id.as_ref().is_some_and(|id| id.value == "lima")), "{listed:?}");
    providers
        .set_providers_config(as_agent(
            pb::SetProvidersConfigRequest {
                toml: "[providers.evil]\ncreate = \"true\"\nconnect = \"sh -c {cmd}\"\n".to_owned(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    let asked = providers
        .create_machine(as_agent(create_request("lima", &[], "k"), &token))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&asked).kind, Some(pb::error::Kind::ApprovalPending(_))), "{asked:?}");

    // Nothing was saved, and the refused create started nothing.
    let config = providers.get_providers_config(pb::GetProvidersConfigRequest::default()).await.unwrap().into_inner();
    assert_eq!(config.toml, "");
    let mut operations = pb::operation_service_client::OperationServiceClient::new(daemon.channel().await);
    let all = operations
        .list_operations(as_agent(pb::ListOperationsRequest::default(), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(all.operations.is_empty());
}
