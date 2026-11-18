//! Who the real daemon takes a caller for when it carries a script token,
//! alone or beside a session's token (agents-01, agents-18, SEC-14). The
//! real `humata whoami` runs against `humatad2`; each test checks the tier
//! and the actor the daemon names, never a message's wording.

mod common;

use common::{Attach, Daemon, binary, create_script_token, folder, shell_token, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use tonic::Request;
use tonic::metadata::MetadataValue;

const SESSION: &str = "se_actor";
const CHANNEL: &str = "ch_actor";
const SESSION_HEADER: &str = "humata-session-token";
const SCRIPT_HEADER: &str = "humata-script-token";

/// A daemon that knows one plain-shell session with a shell tab open in
/// it, that tab's session token, and a script token for its channel.
struct Fixture {
    daemon: Daemon,
    session_token: String,
    script_id: String,
    script_token: String,
    _shell: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: CHANNEL.to_owned(),
                name: "Actors".to_owned(),
                folder: "actors".to_owned(),
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
                title: "actors".to_owned(),
                agent: "zsh".to_owned(),
                folder: dir.path().to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let (session_token, shell) = shell_token(
            &daemon,
            &pb::SessionId {
                value: SESSION.to_owned(),
            },
        )
        .await;
        let (script_id, script_token) = create_script_token(
            &daemon,
            "ci",
            &[pb::Capability::ChannelRead],
            &[&pb::ChannelId {
                value: CHANNEL.to_owned(),
            }],
            0,
        )
        .await
        .unwrap();
        Self {
            daemon,
            session_token,
            script_id,
            script_token,
            _shell: shell,
        }
    }

    /// `humata --json whoami` with these variables set, and nothing else.
    async fn whoami(&self, env: &[(&str, &str)]) -> (i32, serde_json::Value) {
        let output = tokio::process::Command::new(binary("HUMATA"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.daemon.dir.path())
            .env("HUMATA_SOCKET", &self.daemon.socket)
            .envs(env.iter().copied())
            .args(["--json", "whoami"])
            .output()
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let json = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("humata printed no JSON ({error}): {text:?}"));
        (output.status.code().unwrap_or(-1), json)
    }
}

/// Agents-01 step 3 and agents-18 step 2: with `HUMATA_TOKEN` alone the
/// real CLI reports a script, and the daemon names the script by its token
/// id and name.
#[tokio::test(flavor = "multi_thread")]
async fn whoami_with_a_script_token_is_the_script_the_daemon_knows() {
    let fixture = Fixture::start().await;

    let (code, printed) = fixture
        .whoami(&[("HUMATA_TOKEN", fixture.script_token.as_str())])
        .await;

    assert_eq!(code, 0, "{printed}");
    assert_eq!(printed["result"]["tier"], "script", "{printed}");
    let script = &printed["result"]["actor"]["script"];
    assert_eq!(script["token_id"], fixture.script_id.as_str(), "{printed}");
    assert_eq!(script["name"], "ci", "{printed}");
}

/// Agents-18 step 8, the CLI: with a session's token and a script token
/// both set, the real CLI and the daemon take the caller for the agent.
#[tokio::test(flavor = "multi_thread")]
async fn whoami_with_a_session_token_and_a_script_token_is_the_agent() {
    let fixture = Fixture::start().await;

    let (code, printed) = fixture
        .whoami(&[
            ("HUMATA_SESSION_TOKEN", fixture.session_token.as_str()),
            ("HUMATA_TOKEN", fixture.script_token.as_str()),
        ])
        .await;

    assert_eq!(code, 0, "{printed}");
    assert_eq!(printed["result"]["tier"], "agent", "{printed}");
    assert!(printed["result"]["actor"]["agent"].is_object(), "{printed}");
}

/// Agents-18 step 8, the daemon: a call that carries both headers is the
/// session's agent, not the script.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_carrying_both_tokens_is_the_sessions_agent() {
    let fixture = Fixture::start().await;
    let mut request = Request::new(pb::WhoAmIRequest {});
    let metadata = request.metadata_mut();
    metadata.insert(
        SESSION_HEADER,
        MetadataValue::try_from(fixture.session_token.as_str()).unwrap(),
    );
    metadata.insert(
        SCRIPT_HEADER,
        MetadataValue::try_from(fixture.script_token.as_str()).unwrap(),
    );

    let reply = pb::daemon_service_client::DaemonServiceClient::new(fixture.daemon.channel().await)
        .who_am_i(request)
        .await
        .unwrap()
        .into_inner();

    assert!(
        matches!(
            reply.actor.and_then(|actor| actor.actor),
            Some(pb::actor::Actor::Agent(_))
        ),
        "a call with both tokens is the agent"
    );
    assert_eq!(
        reply.channel.map(|channel| channel.value).as_deref(),
        Some(CHANNEL)
    );
}
