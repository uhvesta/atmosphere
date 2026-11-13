//! Landing a session as pull requests, against a bare origin and a stand-in
//! for GitHub's API.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use humata_wire::{Client, pb};

use super::{after_turns, board, daemon, git_repository, item, project, started};
use crate::hosts::Hosts;

const WRITES: &str = r#"{"rules": [
    {"when": "", "steps": [{"step": "write_file", "path": "notes.txt", "contents": "kept\n"}]}
]}"#;

/// What the stand-in GitHub was told and will say.
#[derive(Default)]
struct Hub {
    merged: bool,
    /// The commit the pull request's branch is at; `abc` until set.
    head: String,
    opened: u32,
    bodies: Vec<String>,
}

fn head(hub: &Hub) -> &str {
    if hub.head.is_empty() {
        "abc"
    } else {
        &hub.head
    }
}

fn pull_json(hub: &Hub) -> String {
    let (state, merged_at) = if hub.merged {
        ("closed", "\"2026-11-11T00:00:00Z\"")
    } else {
        ("open", "null")
    };
    format!(
        r#"{{"number": 7, "html_url": "https://github.test/acme/widgets/pull/7",
            "state": "{state}", "merged": {}, "merged_at": {merged_at},
            "head": {{"sha": "{}"}}}}"#,
        hub.merged,
        head(hub)
    )
}

/// Answers GitHub's API for the one repository `acme/widgets`, at the
/// address it returns.
fn stand_in(hub: Arc<Mutex<Hub>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_owned();
            let target = parts.next().unwrap_or_default().to_owned();
            let mut length = 0;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let path = target.split('?').next().unwrap_or_default();
            let reply = {
                let mut hub = hub
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match (method.as_str(), path) {
                    ("GET", "/repos/acme/widgets/pulls") => "[]".to_owned(),
                    ("POST", "/repos/acme/widgets/pulls") => {
                        hub.opened += 1;
                        pull_json(&hub)
                    }
                    ("PATCH", "/repos/acme/widgets/pulls/7") => {
                        hub.bodies.push(String::from_utf8_lossy(&body).into_owned());
                        pull_json(&hub)
                    }
                    ("GET", "/repos/acme/widgets/pulls/7") => pull_json(&hub),
                    ("GET", "/repos/acme/widgets/pulls/7/reviews") => {
                        r#"[{"user": {"login": "sam"}, "state": "APPROVED"}]"#.to_owned()
                    }
                    ("GET", path)
                        if path
                            == format!("/repos/acme/widgets/commits/{}/check-runs", head(&hub)) =>
                    {
                        r#"{"check_runs": [{"status": "completed", "conclusion": "success"}]}"#
                            .to_owned()
                    }
                    ("GET", "/repos/acme/widgets/pulls/7/comments") => r#"[{"id": 901,
                        "path": "notes.txt", "line": 1, "body": "say more",
                        "user": {"login": "sam"}, "diff_hunk": "@@ -0,0 +1 @@\n+kept"}]"#
                        .to_owned(),
                    _ => String::new(),
                }
            };
            let status = if reply.is_empty() {
                "404 Not Found"
            } else {
                "200 OK"
            };
            let reply = if reply.is_empty() {
                "{}".to_owned()
            } else {
                reply
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    format!("http://{address}")
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(output.status.success(), "git {arguments:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

async fn pulls_of(client: &Client, item: &str, refresh: bool) -> Vec<pb::PullRequest> {
    let listed = client
        .call(pb::request::Kind::ListPullRequests(pb::ListPullRequests {
            item_id: item.to_owned(),
            refresh,
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::PullRequests(pulls)) = listed else {
        panic!("{listed:?}")
    };
    pulls.pull_requests
}

/// A task's session that wrote a note and landed it as pull request 7 of
/// `acme/widgets`, on the branch `keep-notes`.
struct Landed {
    _state: tempfile::TempDir,
    repository: tempfile::TempDir,
    _origins: tempfile::TempDir,
    hub: Arc<Mutex<Hub>>,
    client: Client,
    project: pb::Project,
    task: pb::Item,
    session: pb::SessionInfo,
}

async fn land_a_task() -> Landed {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    git(repository.path(), &["config", "user.name", "Pat"]);
    git(
        repository.path(),
        &["config", "user.email", "pat@example.com"],
    );
    let origins = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let origin = origins.path().join("acme").join("widgets.git");
    std::fs::create_dir_all(&origin).unwrap_or_else(|error| panic!("{error}"));
    git(&origin, &["init", "--quiet", "--bare"]);
    git(
        repository.path(),
        &["remote", "add", "origin", &origin.display().to_string()],
    );

    let hub = Arc::new(Mutex::new(Hub::default()));
    let api = stand_in(Arc::clone(&hub));
    let hosts = Hosts {
        coordinator_env: BTreeMap::from([
            ("GH_TOKEN".to_owned(), "test-token".to_owned()),
            ("HUMATA_GITHUB_API".to_owned(), api),
        ]),
        ..Hosts::default()
    };
    let client = daemon(state.path(), hosts).await;
    let project = project(&client, repository.path()).await;
    let task = item(&client, &project.id, "", "Keep notes").await;
    let session = started(
        &client,
        pb::request::Kind::StartSession(pb::StartSession {
            backend: pb::Backend::Scripted.into(),
            project_id: project.id.clone(),
            item_id: task.id.clone(),
            backend_config: WRITES.to_owned(),
            approval_mode: pb::ApprovalMode::AllowEverything.into(),
            ..Default::default()
        }),
    )
    .await;
    let sent = client
        .call(pb::request::Kind::SendMessage(pb::SendMessage {
            session_id: session.id.clone(),
            text: "write the notes".to_owned(),
            mode: pb::SendMode::Enqueue.into(),
            client_id: String::new(),
            images: Vec::new(),
        }))
        .await;
    assert!(
        matches!(sent, Ok(pb::response::Kind::MessageSent(_))),
        "{sent:?}"
    );
    after_turns(&client, &session.id, 1).await;

    let landed = client
        .call(pb::request::Kind::LandSession(pb::LandSession {
            session_id: session.id.clone(),
            branch: "keep-notes".to_owned(),
            message: "Keep notes\n\nSo they last.".to_owned(),
            open_pull_requests: true,
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::SessionLanded(landed)) = landed else {
        panic!("{landed:?}")
    };
    assert!(landed.repositories.iter().all(|landed| landed.pushed));
    assert_eq!(landed.pull_requests.len(), 1);
    assert_eq!(landed.pull_requests[0].number, 7);
    assert_eq!(
        git(&origin, &["show", "keep-notes:notes.txt"]),
        "kept",
        "the branch wasn't pushed"
    );

    Landed {
        _state: state,
        repository,
        _origins: origins,
        hub,
        client,
        project,
        task,
        session,
    }
}

/// Tells the stand-in GitHub the pull request merged at the branch's tip.
fn merge(landed: &Landed) {
    let tip = git(landed.repository.path(), &["rev-parse", "keep-notes"]);
    let mut hub = landed
        .hub
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    hub.head = tip;
    hub.merged = true;
}

#[tokio::test]
async fn a_landed_task_opens_a_pull_request_and_finishes_when_it_merges() {
    let landed = land_a_task().await;
    let Landed {
        repository,
        hub,
        client,
        project,
        task,
        session,
        ..
    } = &landed;
    // Landing again finds the pull request it opened.
    let again = client
        .call(pb::request::Kind::LandSession(pb::LandSession {
            session_id: session.id.clone(),
            branch: "keep-notes".to_owned(),
            message: "Keep notes".to_owned(),
            open_pull_requests: true,
            ..Default::default()
        }))
        .await;
    assert!(
        matches!(again, Ok(pb::response::Kind::SessionLanded(_))),
        "{again:?}"
    );
    assert_eq!(
        hub.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .opened,
        1
    );
    // ...and leaves its description as people may have edited it.
    assert_eq!(
        hub.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .bodies
            .len(),
        1
    );

    // Read again: approved, checks passing, and the reviewer's comment in a
    // thread.
    let open = pulls_of(&client, &task.id, true).await;
    assert_eq!(open[0].state(), pb::PullRequestState::Open);
    assert_eq!(open[0].review(), pb::ReviewStanding::Approved);
    assert_eq!(open[0].checks(), pb::ChecksState::Passing);
    let threads = client
        .call(pb::request::Kind::ListThreads(pb::ListThreads {
            session_id: session.id.clone(),
        }))
        .await;
    let Ok(pb::response::Kind::Threads(threads)) = threads else {
        panic!("{threads:?}")
    };
    assert_eq!(threads.threads.len(), 1);
    assert_eq!(threads.threads[0].code, ["kept"]);
    assert_eq!(
        threads.threads[0].comments[0].author(),
        pb::CommentAuthor::Github
    );
    // Brought in once, however often it is read.
    pulls_of(&client, &task.id, true).await;
    let threads = client
        .call(pb::request::Kind::ListThreads(pb::ListThreads {
            session_id: session.id.clone(),
        }))
        .await;
    let Ok(pb::response::Kind::Threads(threads)) = threads else {
        panic!("{threads:?}")
    };
    assert_eq!(threads.threads[0].comments.len(), 1);

    // Merged: the task is done, the worktree gone and the branch deleted.
    merge(&landed);
    let merged = pulls_of(&client, &task.id, true).await;
    assert_eq!(merged[0].state(), pb::PullRequestState::Merged);
    assert_eq!(
        board(&client, &project.id).await,
        [("Keep notes".to_owned(), pb::ItemStatus::Done)]
    );
    assert!(!Path::new(&session.working_directory).exists());
    assert_eq!(
        git(repository.path(), &["branch", "--list", "keep-notes"]),
        ""
    );

    // A task reopened after it was finished stays open.
    let reopened = client
        .call(pb::request::Kind::UpdateItem(pb::UpdateItem {
            item_id: task.id.clone(),
            status: Some(pb::ItemStatus::Todo.into()),
            ..Default::default()
        }))
        .await;
    assert!(reopened.is_ok(), "{reopened:?}");
    pulls_of(&client, &task.id, true).await;
    assert_eq!(
        board(&client, &project.id).await,
        [("Keep notes".to_owned(), pb::ItemStatus::Todo)]
    );
}

#[tokio::test]
async fn work_since_the_merge_keeps_the_session_as_it_is() {
    let landed = land_a_task().await;
    // Changes made after the pull request was opened, not committed.
    let worktree = Path::new(&landed.session.working_directory);
    std::fs::write(worktree.join("later.txt"), "later\n").unwrap_or_else(|error| panic!("{error}"));
    merge(&landed);
    pulls_of(&landed.client, &landed.task.id, true).await;
    assert_eq!(
        board(&landed.client, &landed.project.id).await,
        [("Keep notes".to_owned(), pb::ItemStatus::Done)]
    );
    assert!(worktree.join("later.txt").exists());
    assert!(
        git(
            landed.repository.path(),
            &["branch", "--list", "keep-notes"]
        )
        .ends_with("keep-notes"),
        "the branch was deleted"
    );
    // Read again, it is still left for the person.
    pulls_of(&landed.client, &landed.task.id, true).await;
    assert!(worktree.join("later.txt").exists());
}

#[tokio::test]
async fn landing_as_pull_requests_without_a_token_lands_nothing() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let session = started(
        &client,
        pb::request::Kind::StartSession(pb::StartSession {
            backend: pb::Backend::Scripted.into(),
            project_id: project.id.clone(),
            backend_config: WRITES.to_owned(),
            ..Default::default()
        }),
    )
    .await;
    let landed = client
        .call(pb::request::Kind::LandSession(pb::LandSession {
            session_id: session.id.clone(),
            branch: "keep-notes".to_owned(),
            message: "Keep notes".to_owned(),
            open_pull_requests: true,
            ..Default::default()
        }))
        .await;
    assert!(landed.is_err(), "{landed:?}");
    assert_eq!(
        git(repository.path(), &["branch", "--list", "keep-notes"]),
        ""
    );
}
