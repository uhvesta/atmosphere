//! The repo tools of `humatad2 mcp` (RL-11, D479): `repo_search` and
//! `repo_library`, which an agent calls to search the repo library and
//! GitHub and to list the library. They are RepoService's SearchRepos and
//! ListRepoLibrary, so an agent reads them as a global read, may widen one
//! call, and has no way to save the scope or remove a repo. `gh` is a fake
//! script (admin builds read `HUMATA2_GH`).

mod common;

use std::path::{Path, PathBuf};
use std::process::Stdio;

use common::{Attach, Build, Daemon, WAIT, binary, folder, shell_token, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_repo_library::{Library, Place, RepoName, Use};
use humata2_store::{AgentLaunch, AgentRecord, AgentState, ChannelRecord, SessionRecord, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const HOME: &str = "ch_home";
const SESSION: &str = "se_agent";
const AGENT: &str = "claude";

/// A signed-in `gh` as `octo`. A repository search records itself in
/// `searches` and finds acme/ledger and acme/api; a `signed_out` file
/// beside it makes searching fail that way.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
case "$*" in
  "api user --jq .login") echo octo; exit 0 ;;
  "api users/"*" --jq .login") echo "${2#users/}"; exit 0 ;;
esac
case "$*" in
  *search/repositories*)
    echo "$*" >> "$here/searches"
    if [ -f "$here/signed_out" ]; then
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2
      exit 4
    fi
    echo '{"full_name":"acme/ledger","description":"The ledger","private":true}'
    echo '{"full_name":"acme/api","description":null,"private":false}'
    ;;
  *user/repos*)
    echo '{"full_name":"octo/notes","description":"Notes","private":false,"default_branch":"main","pushed_at":"2026-11-12T10:00:00Z"}'
    ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

struct Fixture {
    daemon: Daemon,
    gh: PathBuf,
    token: String,
    _attach: Attach,
}

impl Fixture {
    /// A daemon whose library holds acme/ledger, then acme/notes (the more
    /// recent), with an agent session in `HOME`.
    async fn start() -> Self {
        let dir = folder();
        let gh_dir = dir.path().join("gh");
        std::fs::create_dir_all(&gh_dir).unwrap();
        let gh = gh_dir.join("gh");
        std::fs::write(&gh, FAKE_GH).unwrap();
        std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let state = state_folder(dir.path());
        let library = Library::open(&state.join("repos.db")).unwrap();
        for (n, full) in ["acme/ledger", "acme/notes"].into_iter().enumerate() {
            library
                .record(&Use {
                    repo: RepoName::parse(full).unwrap(),
                    remote: format!("https://github.com/{full}.git"),
                    default_branch: "main".to_owned(),
                    at: 1_000 + i64::try_from(n).unwrap(),
                    place: Place::Desk,
                    canonical: false,
                })
                .unwrap();
        }
        drop(library);
        let store = Store::open(state.join("store")).unwrap();
        seed_session(&store, dir.path());
        drop(store);

        let gh_text = gh.display().to_string();
        let daemon =
            Daemon::start_with_env(Build::Admin, dir, &[], &[("HUMATA2_GH", gh_text.as_str())]).await;
        let (token, attach) = shell_token(
            &daemon,
            &pb::SessionId {
                value: SESSION.to_owned(),
            },
        )
        .await;
        Self {
            daemon,
            gh: gh_dir,
            token,
            _attach: attach,
        }
    }

    async fn mcp(&self) -> Mcp {
        Mcp::start(&self.daemon.socket, &self.token).await
    }

    fn searches(&self) -> Vec<String> {
        std::fs::read_to_string(self.gh.join("searches"))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

fn seed_session(store: &Store, dir: &Path) {
    store
        .put_channel(&ChannelRecord {
            id: HOME.to_owned(),
            name: "Payments".to_owned(),
            folder: "home".to_owned(),
            branch_prefix: None,
            default_agent: AGENT.to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: HOME.to_owned(),
            machine: String::new(),
            title: "Agent".to_owned(),
            agent: AGENT.to_owned(),
            folder: dir.to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    let file = dir.join("conv").join("agent.jsonl");
    std::fs::create_dir_all(dir.join("conv")).unwrap();
    std::fs::write(&file, "").unwrap();
    store
        .put_agent(&AgentRecord {
            session: SESSION.to_owned(),
            tab: "tb_agent".to_owned(),
            launch: AgentLaunch {
                agent: AGENT.to_owned(),
                folder: dir.to_owned(),
                conversation_id: "conv-agent".to_owned(),
                conversation_file: file,
                holder_socket: dir.join("agent.sock"),
                cols: 80,
                rows: 24,
                ..AgentLaunch::default()
            },
            state: AgentState::Stopped,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 0,
        })
        .unwrap();
}

// ---------------------------------------------------------------------------
// One `humatad2 mcp` process, spoken to as an agent's client would.

struct Mcp {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Mcp {
    async fn start(socket: &Path, token: &str) -> Self {
        let mut child = Command::new(binary("HUMATAD2"))
            .arg("mcp")
            .arg("--socket")
            .arg(socket)
            .env("HUMATA_SESSION_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self {
            _child: child,
            stdin,
            stdout,
            next_id: 0,
        };
        let initialize = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" },
        });
        let reply = mcp.exchange("initialize", initialize).await;
        assert!(reply.get("error").is_none(), "{reply}");
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        mcp
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn exchange(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let line = tokio::time::timeout(WAIT, self.stdout.next_line())
                .await
                .expect("humatad2 mcp went quiet")
                .unwrap()
                .expect("humatad2 mcp closed its output");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").is_none() && message["id"] == json!(id) {
                return message;
            }
        }
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let reply = self
            .exchange("tools/call", json!({ "name": tool, "arguments": arguments }))
            .await;
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }
}

#[track_caller]
fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["outcome"], "applied", "{result}");
    &result["structuredContent"]["result"]
}

fn names(result: &Value, group: &str, field: &str) -> Vec<String> {
    result[group]
        .as_array()
        .unwrap_or_else(|| panic!("no {group} in {result}"))
        .iter()
        .map(|repo| repo[field].as_str().unwrap_or_default().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_library_comes_back_most_recently_used_first() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let listed = mcp.call("repo_library", json!({})).await;

    assert_eq!(names(ok(&listed), "repos", "repo"), ["acme/notes", "acme/ledger"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_answers_the_library_group_then_the_github_group() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let found = mcp
        .call("repo_search", json!({ "query": "ledger", "all_of_github": true }))
        .await;
    let found = ok(&found);

    assert_eq!(names(found, "library", "repo"), ["acme/ledger"]);
    // acme/ledger is already shown under Library.
    assert_eq!(names(found, "github", "repo"), ["acme/api"]);
    assert_eq!(found["scope"]["kind"], "all_of_github");
    assert!(found["problem"].is_null(), "{found}");
}

#[tokio::test(flavor = "multi_thread")]
async fn naming_owners_widens_that_one_call_and_saves_nothing() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let found = mcp
        .call("repo_search", json!({ "query": "api", "owners": ["acme"] }))
        .await;
    let found = ok(&found);

    assert_eq!(found["scope"]["kind"], "owners");
    assert_eq!(found["scope"]["owners"], json!(["acme"]));
    assert!(
        fixture.searches().iter().any(|line| line.contains("user:acme")),
        "{:?}",
        fixture.searches()
    );
    // The next search without owners is back on the saved scope.
    let again = mcp.call("repo_search", json!({ "query": "notes" })).await;
    assert_eq!(ok(&again)["scope"]["kind"], "your_repos");
    assert!(!fixture.daemon.dir.path().join("config/repos.toml").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn owners_and_all_of_github_together_are_refused_over_the_field() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let refused = mcp
        .call(
            "repo_search",
            json!({ "query": "api", "owners": ["acme"], "all_of_github": true }),
        )
        .await;

    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(refused["structuredContent"]["error"]["kind"], "invalid_field");
    assert!(fixture.searches().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_github_failure_is_a_problem_in_the_answer_and_the_library_still_answers() {
    let fixture = Fixture::start().await;
    std::fs::write(fixture.gh.join("signed_out"), "").unwrap();
    let mut mcp = fixture.mcp().await;

    let found = mcp
        .call("repo_search", json!({ "query": "ledger", "all_of_github": true }))
        .await;
    let found = ok(&found);

    assert_eq!(found["problem"]["kind"], "gh_signed_out");
    assert_eq!(names(found, "library", "repo"), ["acme/ledger"]);
    assert!(names(found, "github", "repo").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_remove_a_repo_and_the_library_is_unchanged() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let refused = mcp.call("repo_remove", json!({ "repo": "acme/ledger" })).await;

    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(refused["structuredContent"]["error"]["kind"], "user_only");
    let listed = mcp.call("repo_library", json!({})).await;
    assert_eq!(names(ok(&listed), "repos", "repo").len(), 2);
}
