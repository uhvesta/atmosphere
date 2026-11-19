//! A stack review (D800, D803): `review_prepare` with `from_stack` reads the
//! repo's stack metadata through the `stack` tool, the review context tells
//! each layer's resolved commits and drift, and review_focus moves between
//! layers. A fake `stack` on the daemon's PATH prints the metadata a test
//! writes; the clone is real git.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tempfile::TempDir;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const SESSION: &str = "se_stack_review";
const REPO: &str = "api";
const TOKEN_HEADER: &str = "humata-session-token";

struct Rig {
    daemon: Daemon,
    clone: PathBuf,
    /// Where the fake `stack` reads its answer.
    status: PathBuf,
    token: String,
    _attach: Attach,
    _bin: TempDir,
    /// The commits of topics `a` and `b`, and the base under them.
    base: String,
    a: String,
    b: String,
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit(clone: &Path, file: &str, text: &str, subject: &str) -> String {
    std::fs::write(clone.join(file), text).unwrap();
    git(clone, &["add", "."]);
    git(clone, &["commit", "-q", "-m", subject]);
    git(clone, &["rev-parse", "HEAD"])
}

fn status_json(topics: &[(&str, Option<&str>, Option<&str>, &str)]) -> String {
    let items: Vec<String> = topics
        .iter()
        .map(|(topic, parent, head, base)| {
            let parent = parent.map_or("null".to_owned(), |parent| format!("\"{parent}\""));
            let head = head.map_or(String::new(), |head| format!(",\"head\":\"{head}\""));
            let base = if head.is_empty() { String::new() } else { format!(",\"base_commit\":\"{base}\"") };
            format!("{{\"topic\":\"{topic}\",\"parent\":{parent}{head}{base}}}")
        })
        .collect();
    format!("{{\"restack\":null,\"topics\":[{}]}}", items.join(","))
}

async fn rig() -> Rig {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "review".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    let clone = dir.path().join(REPO);
    std::fs::create_dir(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    let base = commit(&clone, "base.txt", "base\n", "base");
    let a = commit(&clone, "alpha.txt", "alpha\n", "add alpha");
    let b = commit(&clone, "beta.txt", "beta\n", "add beta");
    drop(store);

    let bin = TempDir::new().unwrap();
    let status = bin.path().join("status.json");
    std::fs::write(&status, status_json(&[("a", None, Some(&a), &base), ("b", Some("a"), Some(&b), &a)])).unwrap();
    let script = bin.path().join("stack");
    std::fs::write(&script, format!("#!/bin/sh\ncat '{}'\n", status.display())).unwrap();
    assert!(Command::new("chmod").arg("+x").arg(&script).status().unwrap().success());
    let path = format!("{}:{}", bin.path().display(), std::env::var("PATH").unwrap_or_default());
    let daemon = Daemon::start_env(dir, &[("PATH", &path)]).await;
    let (token, attach) = agent_token(&daemon).await;
    Rig {
        daemon,
        clone,
        status,
        token,
        _attach: attach,
        _bin: bin,
        base,
        a,
        b,
    }
}

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

fn repo() -> pb::RepoRef {
    pb::RepoRef {
        owner: "acme".to_owned(),
        name: REPO.to_owned(),
    }
}

async fn agent_token(daemon: &Daemon) -> (String, Attach) {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    let path = daemon.dir.path().join(format!("token-{}", tab.value));
    attach
        .type_text(&format!("printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r", path.display()))
        .await;
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(token) = std::fs::read_to_string(&path)
            && !token.is_empty()
        {
            return (token, attach);
        }
        assert!(Instant::now() < deadline, "the shell never wrote its token");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

async fn views(daemon: &Daemon) -> pb::view_service_client::ViewServiceClient<Channel> {
    pb::view_service_client::ViewServiceClient::new(daemon.channel().await)
}

async fn chat(daemon: &Daemon) -> pb::review_chat_service_client::ReviewChatServiceClient<Channel> {
    pb::review_chat_service_client::ReviewChatServiceClient::new(daemon.channel().await)
}

/// The user has the review open, so a plan applies at once.
async fn open_review(rig: &Rig) {
    chat(&rig.daemon)
        .await
        .report_review_view(pb::ReportReviewViewRequest {
            session: Some(session()),
            view: Some(pb::ReviewViewState {
                spec: Some(pb::DiffSpec::default()),
                ..pb::ReviewViewState::default()
            }),
        })
        .await
        .unwrap();
}

fn plan(from_stack: Option<pb::ReviewFromStack>, layers: Vec<pb::ReviewLayer>) -> pb::PrepareViewRequest {
    pb::PrepareViewRequest {
        idempotency_key: None,
        title: "Stack".to_owned(),
        summary: String::new(),
        content: Some(pb::prepare_view_request::Content::ReviewPlan(pb::ReviewPlan {
            from_stack,
            stack: (!layers.is_empty()).then_some(pb::ReviewStack { layers }),
            ..pb::ReviewPlan::default()
        })),
    }
}

fn from_repo() -> Option<pb::ReviewFromStack> {
    Some(pb::ReviewFromStack {
        repo: Some(repo()),
        ..pb::ReviewFromStack::default()
    })
}

fn worktree_layer(title: &str, path: &str) -> pb::ReviewLayer {
    pb::ReviewLayer {
        title: title.to_owned(),
        repos: vec![pb::ReviewLayerRepo {
            repo: Some(repo()),
            part: Some(pb::review_layer_repo::Part::Worktree(pb::ReviewLayerWorktree {
                include_paths: vec![path.to_owned()],
            })),
            ..pb::ReviewLayerRepo::default()
        }],
        ..pb::ReviewLayer::default()
    }
}

async fn context(rig: &Rig) -> pb::ReviewContext {
    chat(&rig.daemon)
        .await
        .get_review_context(as_agent(
            pb::GetReviewContextRequest {
                session: Some(session()),
            },
            &rig.token,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn layers(context: &pb::ReviewContext) -> Vec<pb::ReviewLayer> {
    context.view.as_ref().and_then(|view| view.stack.clone()).unwrap_or_default().layers
}

fn drift_of(layer: &pb::ReviewLayer) -> i32 {
    layer.repos[0].resolved.as_ref().map_or(-1, |resolved| resolved.drift)
}

async fn prepare(rig: &Rig, request: pb::PrepareViewRequest) -> Result<pb::PreparedView, tonic::Status> {
    views(&rig.daemon)
        .await
        .prepare_view(as_agent(request, &rig.token))
        .await
        .map(tonic::Response::into_inner)
}

async fn focus(rig: &Rig, layer: &str, whole_stack: bool) -> Result<pb::ReviewChange, tonic::Status> {
    chat(&rig.daemon)
        .await
        .focus_review(as_agent(
            pb::FocusReviewRequest {
                session: Some(session()),
                layer: layer.to_owned(),
                whole_stack,
                ..pb::FocusReviewRequest::default()
            },
            &rig.token,
        ))
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stack_read_from_the_repo_has_a_layer_per_topic_with_its_commits() {
    let rig = rig().await;
    open_review(&rig).await;
    prepare(&rig, plan(from_repo(), Vec::new())).await.unwrap();

    let found = layers(&context(&rig).await);

    let titles: Vec<&str> = found.iter().map(|layer| layer.title.as_str()).collect();
    assert_eq!(titles, ["add alpha", "add beta"], "named by each topic's first commit, bottom first");
    assert_eq!(found[0].topic, "a");
    let resolved = found[1].repos[0].resolved.clone().expect("the layer's commits are told");
    assert_eq!((resolved.base_commit, resolved.head_commit), (rig.a.clone(), rig.b.clone()));
    assert!(found.iter().all(|layer| drift_of(layer) == pb::review_layer_resolved::Drift::None as i32));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stack_tool_too_old_to_report_heads_is_refused() {
    let rig = rig().await;
    open_review(&rig).await;
    std::fs::write(&rig.status, status_json(&[("a", None, None, "")])).unwrap();

    let refused = prepare(&rig, plan(from_repo(), Vec::new())).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(layers(&context(&rig).await).is_empty(), "nothing was prepared");
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_layers_follow_the_layers_read_from_the_stack() {
    let rig = rig().await;
    open_review(&rig).await;
    std::fs::write(rig.clone.join("notes.txt"), "scratch\n").unwrap();

    prepare(&rig, plan(from_repo(), vec![worktree_layer("scratch", "notes.txt")])).await.unwrap();

    let titles: Vec<String> = layers(&context(&rig).await).into_iter().map(|layer| layer.title).collect();
    assert_eq!(titles, ["add alpha", "add beta", "scratch"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_range_layer_in_the_stack_repo_is_refused_beside_from_stack() {
    let rig = rig().await;
    open_review(&rig).await;
    let range = pb::ReviewLayer {
        title: "mine".to_owned(),
        repos: vec![pb::ReviewLayerRepo {
            repo: Some(repo()),
            part: Some(pb::review_layer_repo::Part::Range(pb::ReviewLayerRange {
                base: rig.base.clone(),
                head: rig.a.clone(),
            })),
            ..pb::ReviewLayerRepo::default()
        }],
        ..pb::ReviewLayer::default()
    };

    let refused = prepare(&rig, plan(from_repo(), vec![range])).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_moves_to_a_layer_and_back_to_the_whole_stack() {
    let rig = rig().await;
    open_review(&rig).await;
    prepare(&rig, plan(from_repo(), Vec::new())).await.unwrap();

    focus(&rig, "ADD ALPHA", false).await.unwrap();
    assert_eq!(context(&rig).await.view.unwrap().layer, "add alpha", "matched ignoring case, kept as titled");

    focus(&rig, "", true).await.unwrap();
    assert_eq!(context(&rig).await.view.unwrap().layer, "", "the whole stack");

    let refused = focus(&rig, "no such layer", false).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_topic_that_moved_since_the_prepare_shows_as_drifted() {
    let rig = rig().await;
    open_review(&rig).await;
    prepare(&rig, plan(from_repo(), Vec::new())).await.unwrap();
    // Rebuild topic b: same parent, a new head.
    git(&rig.clone, &["commit", "-q", "--amend", "-m", "add beta again"]);
    let moved = git(&rig.clone, &["rev-parse", "HEAD"]);
    std::fs::write(
        &rig.status,
        status_json(&[("a", None, Some(&rig.a), &rig.base), ("b", Some("a"), Some(&moved), &rig.a)]),
    )
    .unwrap();

    let found = layers(&context(&rig).await);

    assert_eq!(drift_of(&found[0]), pb::review_layer_resolved::Drift::None as i32);
    assert_eq!(drift_of(&found[1]), pb::review_layer_resolved::Drift::Moved as i32);
}
