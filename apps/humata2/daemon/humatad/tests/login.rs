//! InteractiveLogin and MakeUnattended against the real daemon: a provider
//! whose connect asks for a verification code stops a create ("needs a
//! login"), the login opened for that operation shows the prompt, typing
//! the code and Done takes the shell over and the machine is listed with a
//! live peer link, Cancel leaves no machine, and an agent is refused. The
//! "machine" is this computer under a temporary home with a fake `uname`
//! and `pgrep`; nothing touches the real home, `~/.ssh` or a network.
//!
//! Enabling it: move this file up one folder to `tests/login.rs` once
//! `login_service` is wired in (see the enable lines in the report).

mod common;

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, binary, connect, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Request, Streaming};

const FILES: [&str; 3] = ["humatad2", "humata2_holder", "humata"];

/// A tools bundle for linux-x86_64, which the fake `uname` claims to be.
/// Its humatad2 runs the real one, so the peer link really comes up.
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

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The fake machine's login command and home. Without a terminal it asks
/// for a code and fails (the non-interactive try); with one it reads the
/// code, then runs the shell the daemon asked for, as the machine's user
/// with the machine's home.
fn fake_login(dir: &Path) -> PathBuf {
    let home = dir.join("machine-home");
    let fakes = home.join(".test-bin");
    std::fs::create_dir_all(&fakes).unwrap();
    executable(
        &fakes.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo Linux;; -m) echo x86_64;; *) /usr/bin/uname \"$@\";; esac\n",
    );
    executable(&fakes.join("pgrep"), "#!/bin/sh\nexit 1\n");
    let login = dir.join("login.sh");
    executable(
        &login,
        &format!(
            "#!/bin/sh\nexport HOME='{home}'\nexport PATH='{fakes}:/usr/bin:/bin:/usr/sbin:/sbin'\n\
             if [ ! -t 0 ]; then printf 'Verification code: '; exit 1; fi\n\
             printf 'Verification code: '; read code\n\
             [ \"$code\" = 123456 ] || {{ echo denied; exit 3; }}\n\
             exec sh -c \"$1\"\n",
            home = home.display(),
            fakes = fakes.display()
        ),
    );
    login
}

struct Host {
    child: Child,
    socket: PathBuf,
    dir: TempDir,
}

impl Host {
    async fn start() -> Self {
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
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self { child, socket, dir }
    }

    async fn providers(&self) -> pb::provider_service_client::ProviderServiceClient<Channel> {
        pb::provider_service_client::ProviderServiceClient::new(connect(&self.socket).await)
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(connect(&self.socket).await)
    }

    async fn operations(&self) -> pb::operation_service_client::OperationServiceClient<Channel> {
        pb::operation_service_client::OperationServiceClient::new(connect(&self.socket).await)
    }

    async fn watch(&self, operation: &pb::Operation) -> pb::Operation {
        let mut stream = self
            .operations()
            .await
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

    /// Saves the providers: `corp` connects through the fake login.
    async fn save_providers(&self, login: &Path) {
        let toml = format!(
            "[providers.corp]\nlabel = \"Corp\"\ncreate = \"true\"\nconnect = \"{} {{cmd}}\"\n",
            login.display()
        );
        let revision = self
            .providers()
            .await
            .get_providers_config(pb::GetProvidersConfigRequest::default())
            .await
            .unwrap()
            .into_inner()
            .revision;
        let saved = self
            .providers()
            .await
            .set_providers_config(pb::SetProvidersConfigRequest {
                idempotency_key: None,
                toml,
                expected_revision: revision,
                presence: None,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(saved.problems.is_empty(), "{:?}", saved.problems);
    }

    /// A create through `corp` of `name`, which stops needing a login; the
    /// failed operation.
    async fn create_needing_login(&self, name: &str) -> pb::Operation {
        let creating = self
            .providers()
            .await
            .create_machine(pb::CreateMachineRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("create-{name}"),
                }),
                provider: Some(pb::ProviderId {
                    value: "corp".to_owned(),
                }),
                name: name.to_owned(),
                params: HashMap::new(),
            })
            .await
            .unwrap()
            .into_inner();
        self.watch(&creating).await
    }

    /// Opens a login on `operation`; messages sent on the returned sender
    /// reach the daemon.
    async fn open_login(
        &self,
        operation: &pb::OperationId,
    ) -> (mpsc::Sender<pb::LoginClientMessage>, Streaming<pb::LoginServerMessage>) {
        let (sender, receiver) = mpsc::channel(16);
        sender
            .send(open_message(pb::login_open::Target::Operation(operation.clone())))
            .await
            .unwrap();
        let stream = self
            .providers()
            .await
            .interactive_login(ReceiverStream::new(receiver))
            .await
            .unwrap()
            .into_inner();
        (sender, stream)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn open_message(target: pb::login_open::Target) -> pb::LoginClientMessage {
    pb::LoginClientMessage {
        message: Some(pb::login_client_message::Message::Open(pb::LoginOpen {
            target: Some(target),
            cols: 80,
            rows: 24,
        })),
    }
}

fn client(message: pb::login_client_message::Message) -> pb::LoginClientMessage {
    pb::LoginClientMessage { message: Some(message) }
}

/// What a login stream showed, until `until` says it has seen enough or
/// the stream ends.
#[derive(Default)]
struct Seen {
    output: Vec<u8>,
    prompts: Vec<String>,
    /// The kind sent with each prompt, in order (D167).
    kinds: Vec<i32>,
    ready: Option<pb::LoginReady>,
    failed: Option<pb::Error>,
    ended: bool,
}

impl Seen {
    fn shows(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.output).contains(text)
    }
}

async fn read_until(stream: &mut Streaming<pb::LoginServerMessage>, done: impl Fn(&Seen) -> bool) -> Seen {
    use pb::login_server_message::Message;
    let mut seen = Seen::default();
    // Longer than the daemon's own waits in a takeover (a silent terminal
    // is given up after 60 s), so a stuck login ends as a failure that
    // says what the terminal printed, not as this deadline.
    let deadline = Instant::now() + Duration::from_secs(150);
    while !done(&seen) && !seen.ended {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, stream.message()).await {
            Ok(Ok(Some(message))) => match message.message {
                Some(Message::Output(bytes)) => seen.output.extend_from_slice(&bytes),
                Some(Message::PromptSeen(line)) => {
                    seen.prompts.push(line);
                    seen.kinds.push(message.prompt_kind);
                }
                Some(Message::Ready(ready)) => seen.ready = Some(ready),
                Some(Message::Failed(error)) => seen.failed = Some(error),
                None => {}
            },
            Ok(Ok(None)) | Ok(Err(_)) => seen.ended = true,
            Err(_) => panic!("the login stream went quiet; saw {:?}", String::from_utf8_lossy(&seen.output)),
        }
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_gated_provider_is_logged_in_by_hand_and_becomes_a_normal_machine() {
    let host = Host::start().await;
    host.save_providers(&fake_login(host.dir.path())).await;

    // The non-interactive try finds the prompt, so the create stops there.
    let failed = host.create_needing_login("box-1").await;
    assert_eq!(failed.state, pb::OperationState::Failed as i32);
    let Some(pb::error::Kind::LoginNeedsUser(needs)) = failed.error.as_ref().and_then(|error| error.kind.clone()) else {
        panic!("expected needs-a-login, got {:?}", failed.error);
    };
    assert_eq!(needs.prompt_kind, pb::LoginPromptKind::Code as i32);
    assert!(!needs.hung);
    assert!(!needs.prompt_line.is_empty());
    assert!(host.list().await.is_empty());

    // The login for that operation shows the terminal and the prompt.
    let (sender, mut stream) = host.open_login(failed.id.as_ref().unwrap()).await;
    let seen = read_until(&mut stream, |seen| seen.shows("Verification code") && !seen.prompts.is_empty()).await;
    assert_eq!(seen.prompts.len(), 1, "the prompt is reported once");
    assert_eq!(seen.kinds, vec![pb::LoginPromptKind::Code as i32]);

    // The code, then Done: the takeover ends in the peer link.
    sender
        .send(client(pb::login_client_message::Message::Input(b"123456\n".to_vec().into())))
        .await
        .unwrap();
    // Done is pressed once the machine's shell shows its prompt, as the
    // dialog asks the person to.
    read_until(&mut stream, |seen| seen.shows("$ ")).await;
    sender
        .send(client(pb::login_client_message::Message::Done(pb::LoginDone {})))
        .await
        .unwrap();
    let seen = read_until(&mut stream, |seen| seen.ready.is_some() || seen.failed.is_some()).await;
    assert!(seen.failed.is_none(), "{:?}", seen.failed);
    let ready = seen.ready.expect("the login ended ready");
    assert_eq!(ready.machine.as_ref().map(|machine| machine.value.as_str()), Some("box-1"));

    let listed = host.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "box-1");
    let operation = host
        .operations()
        .await
        .get_operation(pb::GetOperationRequest {
            id: failed.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32);
    assert!(host.dir.path().join("state/peer-keys/box-1").exists());

    // The machine's own daemon answers over the taken-over terminal.
    let deadline = Instant::now() + WAIT;
    loop {
        if host.list().await.iter().any(|machine| machine.name == "box-1" && !machine.daemon_version.is_empty()) {
            break;
        }
        assert!(Instant::now() < deadline, "the peer link to box-1 never came up");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_login_leaves_no_machine() {
    let host = Host::start().await;
    host.save_providers(&fake_login(host.dir.path())).await;
    let failed = host.create_needing_login("box-2").await;
    let (sender, mut stream) = host.open_login(failed.id.as_ref().unwrap()).await;
    read_until(&mut stream, |seen| seen.shows("Verification code")).await;

    sender
        .send(client(pb::login_client_message::Message::Cancel(pb::LoginCancel {})))
        .await
        .unwrap();
    let seen = read_until(&mut stream, |_| false).await;

    assert!(seen.ready.is_none());
    assert!(host.list().await.is_empty());
    assert!(!host.dir.path().join("state/peer-keys/box-2").exists());
    let operation = host
        .operations()
        .await
        .get_operation(pb::GetOperationRequest { id: failed.id })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(operation.state, pb::OperationState::Failed as i32, "the create stays failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_code_ends_with_a_failure_that_says_what_the_terminal_printed() {
    let host = Host::start().await;
    host.save_providers(&fake_login(host.dir.path())).await;
    let failed = host.create_needing_login("box-3").await;
    let (sender, mut stream) = host.open_login(failed.id.as_ref().unwrap()).await;
    read_until(&mut stream, |seen| seen.shows("Verification code")).await;

    sender
        .send(client(pb::login_client_message::Message::Input(b"000000\n".to_vec().into())))
        .await
        .unwrap();
    let seen = read_until(&mut stream, |seen| seen.failed.is_some()).await;

    let error = seen.failed.expect("the login failed");
    assert!(!error.fix.is_empty());
    assert!(host.list().await.is_empty());
}

const SESSION: &str = "se_login";
const TOKEN_HEADER: &str = "humata-session-token";

/// An admin daemon with one session, and the token its shell tab got.
async fn daemon_with_agent() -> (Daemon, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_login".to_owned(),
            machine: String::new(),
            title: "login".to_owned(),
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
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
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
async fn an_agent_can_neither_open_a_login_nor_make_a_machine_unattended() {
    let (daemon, token) = daemon_with_agent().await;
    let mut providers = pb::provider_service_client::ProviderServiceClient::new(daemon.channel().await);

    let (sender, receiver) = mpsc::channel(4);
    sender
        .send(open_message(pb::login_open::Target::Machine(pb::MachineId {
            value: "box-1".to_owned(),
        })))
        .await
        .unwrap();
    let refused = providers
        .interactive_login(as_agent(ReceiverStream::new(receiver), &token))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))), "{refused:?}");
    assert!(!error_of(&refused).fix.is_empty());

    let unattended = providers
        .make_unattended(as_agent(
            pb::MakeUnattendedRequest {
                idempotency_key: None,
                machine: Some(pb::MachineId {
                    value: "box-1".to_owned(),
                }),
                method: pb::make_unattended_request::Method::KeepCachedToken as i32,
                presence: None,
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&unattended).kind, Some(pb::error::Kind::UserOnly(_))), "{unattended:?}");
    assert!(!error_of(&unattended).fix.is_empty());
}
