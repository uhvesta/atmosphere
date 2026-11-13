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
    /// Reading the pull request's reviews fails while set.
    reviews_fail: bool,
    /// The paths read, in order.
    read: Vec<String>,
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

/// The ETag the stand-in gives the pull request as it is.
fn pull_etag(hub: &Hub) -> String {
    format!("\"pull-{}-{}\"", hub.merged, head(hub))
}

/// Answers GitHub's API for the one repository `acme/widgets`, at the
/// address it returns. The pull request and its checks are answered
/// unchanged when asked with their ETag, and its open issues come in two
/// pages.
fn stand_in(hub: Arc<Mutex<Hub>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let base = format!("http://{address}");
    let served = base.clone();
    std::thread::spawn(move || {
        let base = served;
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
            let mut if_none_match = String::new();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    } else if name.eq_ignore_ascii_case("if-none-match") {
                        value.trim().clone_into(&mut if_none_match);
                    }
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
            let checks =
                |hub: &Hub| format!("/repos/acme/widgets/commits/{}/check-runs", head(hub));
            let mut status = "200 OK";
            let mut headers = String::new();
            let reply = {
                let mut hub = hub
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if method == "GET" {
                    hub.read.push(path.to_owned());
                }
                let etag = if path == "/repos/acme/widgets/pulls/7" {
                    pull_etag(&hub)
                } else if path == checks(&hub) {
                    format!("\"checks-{}\"", head(&hub))
                } else {
                    String::new()
                };
                if !etag.is_empty() {
                    headers = format!("ETag: {etag}\r\n");
                }
                match (method.as_str(), path) {
                    ("GET", _) if !etag.is_empty() && if_none_match == etag => {
                        status = "304 Not Modified";
                        String::new()
                    }
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
                    ("GET", "/repos/acme/widgets/pulls/7/reviews") if hub.reviews_fail => {
                        status = "500 Internal Server Error";
                        r#"{"message": "Server Error"}"#.to_owned()
                    }
                    ("GET", "/repos/acme/widgets/pulls/7/reviews") => {
                        r#"[{"user": {"login": "sam"}, "state": "APPROVED"}]"#.to_owned()
                    }
                    ("GET", "/repos/acme/widgets/issues") if query.contains("page=2") => {
                        r#"[{"number": 3, "title": "Third", "body": "Later."}]"#.to_owned()
                    }
                    ("GET", "/repos/acme/widgets/issues") => {
                        headers = format!(
                            "Link: <{base}/repos/acme/widgets/issues?state=open&per_page=100&page=2>; rel=\"next\", <{base}/repos/acme/widgets/issues?state=open&per_page=100&page=2>; rel=\"last\"\r\n"
                        );
                        r#"[{"number": 1, "title": "First", "body": "Soon."},
                            {"number": 2, "title": "A pull request", "pull_request": {}}]"#
                            .to_owned()
                    }
                    ("GET", path) if path == checks(&hub) => {
                        r#"{"check_runs": [{"status": "completed", "conclusion": "success"}]}"#
                            .to_owned()
                    }
                    ("GET", "/repos/acme/widgets/pulls/7/comments") => r#"[{"id": 901,
                        "path": "notes.txt", "line": 1, "body": "say more",
                        "user": {"login": "sam"}, "diff_hunk": "@@ -0,0 +1 @@\n+kept"}]"#
                        .to_owned(),
                    _ => {
                        status = "404 Not Found";
                        "{}".to_owned()
                    }
                }
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    base
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

/// How often the stand-in GitHub was asked for `path`.
fn reads(hub: &Mutex<Hub>, path: &str) -> usize {
    hub.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .read
        .iter()
        .filter(|read| *read == path)
        .count()
}

#[tokio::test]
async fn a_pull_request_that_cant_be_read_shows_why_and_the_list_still_comes() {
    let landed = land_a_task().await;
    let Landed {
        hub, client, task, ..
    } = &landed;
    hub.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .reviews_fail = true;
    let failing = pulls_of(client, &task.id, true).await;
    assert_eq!(failing.len(), 1);
    assert!(!failing[0].problem.is_empty());
    // The rest is as it was last read.
    assert_eq!(failing[0].state(), pb::PullRequestState::Open);
    assert_eq!(failing[0].review(), pb::ReviewStanding::None);
    // Listed without reading, it still says why.
    assert!(
        !pulls_of(client, &task.id, false).await[0]
            .problem
            .is_empty()
    );

    hub.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .reviews_fail = false;
    let read = pulls_of(client, &task.id, true).await;
    assert!(read[0].problem.is_empty());
    assert_eq!(read[0].review(), pb::ReviewStanding::Approved);
}

#[tokio::test]
async fn a_pull_request_as_it_was_isnt_read_again_but_its_checks_are() {
    let landed = land_a_task().await;
    let Landed {
        hub, client, task, ..
    } = &landed;
    let reviews = "/repos/acme/widgets/pulls/7/reviews";
    let comments = "/repos/acme/widgets/pulls/7/comments";
    let checks = "/repos/acme/widgets/commits/abc/check-runs";
    pulls_of(client, &task.id, true).await;
    assert_eq!((reads(hub, reviews), reads(hub, comments)), (1, 1));

    let unchanged = pulls_of(client, &task.id, true).await;
    assert_eq!((reads(hub, reviews), reads(hub, comments)), (1, 1));
    assert_eq!(reads(hub, checks), 2);
    assert_eq!(unchanged[0].review(), pb::ReviewStanding::Approved);
    assert_eq!(unchanged[0].checks(), pb::ChecksState::Passing);

    // Changed, it is read again.
    merge(&landed);
    let merged = pulls_of(client, &task.id, true).await;
    assert_eq!(merged[0].state(), pb::PullRequestState::Merged);
    assert_eq!(reads(hub, reviews), 2);
}

#[tokio::test]
async fn every_page_of_open_issues_is_imported_once() {
    let landed = land_a_task().await;
    let import = || {
        landed
            .client
            .call(pb::request::Kind::ImportIssues(pb::ImportIssues {
                project_id: landed.project.id.clone(),
            }))
    };
    let first = import().await;
    let Ok(pb::response::Kind::IssuesImported(first)) = first else {
        panic!("{first:?}")
    };
    let titles: Vec<&str> = first.items.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(titles, ["First", "Third"]);
    assert_eq!(first.skipped, 0);

    let again = import().await;
    let Ok(pb::response::Kind::IssuesImported(again)) = again else {
        panic!("{again:?}")
    };
    assert!(again.items.is_empty());
    assert_eq!(again.skipped, 2);
}
