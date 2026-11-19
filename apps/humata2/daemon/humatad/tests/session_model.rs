//! The model a session is started on reaches its agent's command line
//! through a real `humatad2`: on start, when the agent is brought back
//! after its holder died, and in a fork that keeps the parent's agent. A
//! session started with no model runs the agent on its own default.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Build, Daemon, World, create_channel, finished, folder, machine_id, start_request};
use humata2_grpc::humata::v1 as pb;

const MODEL: &str = "claude-haiku-5.5";

/// A world whose test agent takes its model as `--model <model>` and
/// writes the arguments of every run to `<out>/<conversation>.args`, one
/// argument per line and `--` after each run.
async fn model_daemon() -> (Daemon, World, PathBuf) {
    let dir = folder();
    let world = World::new(dir.path());
    let out = dir.path().join("args-out");
    std::fs::create_dir_all(&out).unwrap();
    let script = format!(
        r#"CONV_DIR='{}'
OUT='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
{{ printf '%s\n' "$@"; echo --; }} >> "$OUT/$id.args"
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        world.conv.display(),
        out.display()
    );
    std::fs::write(dir.path().join("agent.sh"), script).unwrap();
    let mut agents: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&world.agents).unwrap()).unwrap();
    agents[0]["model_args"] = serde_json::json!(["--model", "{model}"]);
    std::fs::write(&world.agents, serde_json::Value::from(agents).to_string()).unwrap();
    world.remote("acme", "api");
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with(Build::Admin, dir, &refs).await, world, out)
}

/// Starts a session on `model` (empty: none chosen) and waits until its agent runs.
async fn start_on(daemon: &Daemon, channel: &pb::Channel, title: &str, model: &str) -> pb::SessionId {
    let machine = machine_id(daemon).await;
    let request = pb::StartSessionRequest {
        model: model.to_owned(),
        ..start_request(channel.id.as_ref().unwrap(), &machine, title)
    };
    let response = daemon.session_service().await.start_session(request).await.unwrap().into_inner();
    let operation = finished(daemon, &response.operation.unwrap()).await;
    assert!(operation.error.is_none(), "the start failed: {:?}", operation.error);
    let id = response.session.unwrap();
    common::wait_running(daemon, &id).await;
    id
}

/// Every run of the agent the daemon made, oldest first, each as its arguments.
fn runs(out: &Path) -> Vec<Vec<String>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(out).unwrap().flatten().map(|entry| entry.path()).collect();
    files.sort();
    let mut runs = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file).unwrap();
        let mut run = Vec::new();
        for line in text.lines() {
            if line == "--" {
                runs.push(std::mem::take(&mut run));
            } else {
                run.push(line.to_owned());
            }
        }
    }
    runs
}

/// Waits until the agent has run `count` times in all, and returns the runs.
async fn wait_runs(out: &Path, count: usize) -> Vec<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let runs = runs(out);
        if runs.len() >= count {
            return runs;
        }
        assert!(Instant::now() < deadline, "the agent ran {} times, not {count}: {runs:?}", runs.len());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn model_of(run: &[String]) -> Option<&str> {
    let at = run.iter().position(|arg| arg == "--model")?;
    run.get(at + 1).map(String::as_str)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_started_on_a_model_runs_its_agent_on_it_and_one_with_none_on_the_agents_default() {
    let (daemon, _world, out) = model_daemon().await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    start_on(&daemon, &channel, "Cheap", MODEL).await;
    let first = wait_runs(&out, 1).await;
    assert_eq!(model_of(&first[0]), Some(MODEL), "{first:?}");

    start_on(&daemon, &channel, "Default", "").await;
    let both = wait_runs(&out, 2).await;
    let unmodelled: Vec<_> = both.iter().filter(|run| model_of(run).is_none()).collect();
    assert_eq!(unmodelled.len(), 1, "exactly the session with no model has no --model: {both:?}");
    assert!(!unmodelled[0].iter().any(|arg| arg == "--model"), "{both:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_brought_back_after_its_holder_died_keeps_the_sessions_model() {
    let (mut daemon, _world, out) = model_daemon().await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    start_on(&daemon, &channel, "Cheap", MODEL).await;
    wait_runs(&out, 1).await;

    common::kill_holders(&common::stored_tabs(&daemon.state)).await;
    daemon.restart().await;

    let runs = wait_runs(&out, 2).await;
    let resumed = runs.iter().find(|run| run.first().is_some_and(|mode| mode == "--resume")).expect("a resume run");
    assert_eq!(model_of(resumed), Some(MODEL), "{runs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fork_with_its_parents_agent_runs_on_the_parents_model() {
    let (daemon, _world, out) = model_daemon().await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let parent = start_on(&daemon, &channel, "Cheap", MODEL).await;
    wait_runs(&out, 1).await;

    let started = daemon
        .session_service()
        .await
        .fork_session(pb::ForkSessionRequest {
            parent: Some(parent),
            kind: pb::ForkKind::CodeAndContext as i32,
            title: "Cheap fork".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&daemon, &started).await;
    assert!(operation.error.is_none(), "the fork failed: {:?}", operation.error);

    let runs = wait_runs(&out, 2).await;
    assert!(runs.iter().all(|run| model_of(run) == Some(MODEL)), "{runs:?}");
}
