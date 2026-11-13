//! Landing a session as pull requests, against a bare origin and a stand-in
//! for GitHub's API.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use humata_wire::{Client, pb};

use super::{after_turns, board, daemon, git_repository, item, project, started};
use crate::github::{GitHub, GitHubError, Repo};
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
    /// The description of the pull request open in each repository.
    open: BTreeMap<String, String>,
    /// Opening a pull request is refused this way while set.
    refusal: Option<Refusal>,
    /// Opening a pull request fails while set and another repository has
    /// one open.
    second_open_fails: bool,
}

/// How the stand-in GitHub refuses to open a pull request.
#[derive(Clone, Copy)]
struct Refusal {
    status: &'static str,
    /// Header lines, each ended.
    headers: &'static str,
    reply: &'static str,
}

fn head(hub: &Hub) -> &str {
    if hub.head.is_empty() {
        "abc"
    } else {
        &hub.head
    }
}

/// The pull request of `repo`, which are all number 7.
fn pull_json(hub: &Hub, repo: &str) -> String {
    let (state, merged_at) = if hub.merged {
        ("closed", "\"2026-11-11T00:00:00Z\"")
    } else {
        ("open", "null")
    };
    format!(
        r#"{{"number": 7, "html_url": "https://github.test/acme/widgets/pull/7",
            "state": "{state}", "merged": {}, "merged_at": {merged_at}, "body": {},
            "head": {{"sha": "{}"}}}}"#,
        hub.merged,
        serde_json::Value::from(hub.open.get(repo).cloned()),
        head(hub)
    )
}

/// The ETag the stand-in gives the pull request as it is.
fn pull_etag(hub: &Hub) -> String {
    format!("\"pull-{}-{}\"", hub.merged, head(hub))
}

/// The description a request to open or change a pull request gives it.
fn body_of(request: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(request)
        .ok()
        .and_then(|fields| fields["body"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Answers GitHub's API for the repositories of `acme`, at the address it
/// returns. They share what the pull request is but for its description. A
/// pull request and its checks are answered unchanged when asked with their
/// ETag, and the open issues come in two pages.
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
            let (repo, rest) = path
                .strip_prefix("/repos/acme/")
                .and_then(|rest| rest.split_once('/'))
                .unwrap_or_default();
            let checks = |hub: &Hub| format!("commits/{}/check-runs", head(hub));
            let mut status = "200 OK";
            let mut headers = String::new();
            let reply = {
                let mut hub = hub
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if method == "GET" {
                    hub.read.push(path.to_owned());
                }
                let etag = if rest == "pulls/7" {
                    pull_etag(&hub)
                } else if rest == checks(&hub) {
                    format!("\"checks-{}\"", head(&hub))
                } else {
                    String::new()
                };
                if !etag.is_empty() {
                    headers = format!("ETag: {etag}\r\n");
                }
                let refusal = hub.refusal.filter(|_| method == "POST" && rest == "pulls");
                match (method.as_str(), rest) {
                    _ if refusal.is_some() => {
                        let refusal = refusal.unwrap_or(Refusal {
                            status,
                            headers: "",
                            reply: "",
                        });
                        status = refusal.status;
                        refusal.headers.clone_into(&mut headers);
                        refusal.reply.to_owned()
                    }
                    ("GET", _) if !etag.is_empty() && if_none_match == etag => {
                        status = "304 Not Modified";
                        String::new()
                    }
                    ("GET", "pulls") if hub.merged || !hub.open.contains_key(repo) => {
                        "[]".to_owned()
                    }
                    ("GET", "pulls") => format!("[{}]", pull_json(&hub, repo)),
                    ("POST", "pulls")
                        if hub.second_open_fails && hub.open.keys().any(|open| open != repo) =>
                    {
                        status = "500 Internal Server Error";
                        r#"{"message": "Server Error"}"#.to_owned()
                    }
                    ("POST", "pulls") => {
                        hub.opened += 1;
                        hub.merged = false;
                        hub.open.insert(repo.to_owned(), body_of(&body));
                        pull_json(&hub, repo)
                    }
                    ("PATCH", "pulls/7") => {
                        hub.bodies.push(String::from_utf8_lossy(&body).into_owned());
                        hub.open.insert(repo.to_owned(), body_of(&body));
                        pull_json(&hub, repo)
                    }
                    ("GET", "pulls/7") => pull_json(&hub, repo),
                    ("GET", "pulls/7/reviews") if hub.reviews_fail => {
                        status = "500 Internal Server Error";
                        r#"{"message": "Server Error"}"#.to_owned()
                    }
                    ("GET", "pulls/7/reviews") => {
                        r#"[{"user": {"login": "sam"}, "state": "APPROVED"}]"#.to_owned()
                    }
                    ("GET", "issues") if query.contains("page=2") => {
                        r#"[{"number": 3, "title": "Third", "body": "Later."}]"#.to_owned()
                    }
                    ("GET", "issues") => {
                        headers = format!(
                            "Link: <{base}/repos/acme/widgets/issues?state=open&per_page=100&page=2>; rel=\"next\", <{base}/repos/acme/widgets/issues?state=open&per_page=100&page=2>; rel=\"last\"\r\n"
                        );
                        r#"[{"number": 1, "title": "First", "body": "Soon."},
                            {"number": 2, "title": "A pull request", "pull_request": {}}]"#
                            .to_owned()
                    }
                    ("GET", rest) if rest == checks(&hub) => {
                        r#"{"check_runs": [{"status": "completed", "conclusion": "success"}]}"#
                            .to_owned()
                    }
                    ("GET", "pulls/7/comments") => r#"[{"id": 901,
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

/// A repository the person is set up in, and its origin, the bare
/// `acme/<name>.git` under `origins`.
fn repository_of(origins: &Path, name: &str) -> (tempfile::TempDir, PathBuf) {
    let repository = git_repository();
    git(repository.path(), &["config", "user.name", "Pat"]);
    git(
        repository.path(),
        &["config", "user.email", "pat@example.com"],
    );
    let origin = origins.join("acme").join(format!("{name}.git"));
    std::fs::create_dir_all(&origin).unwrap_or_else(|error| panic!("{error}"));
    git(&origin, &["init", "--quiet", "--bare"]);
    git(
        repository.path(),
        &["remote", "add", "origin", &origin.display().to_string()],
    );
    (repository, origin)
}

/// This Mac with a token, and a stand-in GitHub that keeps `hub`.
fn hosts_with(hub: &Arc<Mutex<Hub>>) -> Hosts {
    Hosts {
        coordinator_env: BTreeMap::from([
            ("GH_TOKEN".to_owned(), "test-token".to_owned()),
            ("HUMATA_GITHUB_API".to_owned(), stand_in(Arc::clone(hub))),
        ]),
        ..Hosts::default()
    }
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
    let origins = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let (repository, origin) = repository_of(origins.path(), "widgets");

    let hub = Arc::new(Mutex::new(Hub::default()));
    let client = daemon(state.path(), hosts_with(&hub)).await;
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
    merge_at(landed, "keep-notes");
}

/// Tells the stand-in GitHub the pull request merged at the tip of `branch`.
fn merge_at(landed: &Landed, branch: &str) {
    let tip = git(landed.repository.path(), &["rev-parse", branch]);
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

async fn set_refresh(
    client: &Client,
    manual: bool,
    every_minutes: u32,
) -> Result<pb::response::Kind, humata_wire::ClientError> {
    client
        .call(pb::request::Kind::SetPullRefresh(pb::SetPullRefresh {
            settings: Some(pb::PullRefresh {
                manual,
                every_minutes,
            }),
        }))
        .await
}

async fn refresh_of(client: &Client) -> pb::PullRefresh {
    let read = client
        .call(pb::request::Kind::GetPullRefresh(pb::GetPullRefresh {}))
        .await;
    let Ok(pb::response::Kind::PullRefresh(refresh)) = read else {
        panic!("{read:?}")
    };
    refresh
}

#[tokio::test]
async fn how_pull_requests_are_followed_is_kept_across_a_restart() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    // Until set, they are read on their own every two minutes.
    assert_eq!(
        refresh_of(&client).await,
        pb::PullRefresh {
            manual: false,
            every_minutes: 2,
        }
    );

    let set = set_refresh(&client, true, 30).await;
    let kept = pb::PullRefresh {
        manual: true,
        every_minutes: 30,
    };
    assert!(
        matches!(&set, Ok(pb::response::Kind::PullRefresh(set)) if *set == kept),
        "{set:?}"
    );
    drop(client);

    let again = daemon(state.path(), Hosts::default()).await;
    assert_eq!(refresh_of(&again).await, kept);
    // No minutes is the default.
    let set = set_refresh(&again, false, 0).await;
    assert!(set.is_ok(), "{set:?}");
    assert_eq!(refresh_of(&again).await.every_minutes, 2);
}

#[tokio::test]
async fn minutes_past_a_day_are_refused_and_the_setting_stays() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    let set = set_refresh(&client, false, 15).await;
    assert!(set.is_ok(), "{set:?}");

    let refused = set_refresh(&client, false, 1441).await;
    assert!(
        matches!(
            refused,
            Err(humata_wire::ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(refresh_of(&client).await.every_minutes, 15);
    let a_day = set_refresh(&client, false, 1440).await;
    assert!(a_day.is_ok(), "{a_day:?}");
}

#[tokio::test]
async fn pull_requests_read_only_when_asked_are_still_read_when_asked() {
    let landed = land_a_task().await;
    let Landed { client, task, .. } = &landed;
    let set = set_refresh(client, true, 0).await;
    assert!(set.is_ok(), "{set:?}");
    merge(&landed);
    let read = pulls_of(client, &task.id, true).await;
    assert_eq!(read[0].state(), pb::PullRequestState::Merged);
}

#[tokio::test]
async fn a_task_landed_again_after_it_merged_finishes_again_when_that_merges() {
    let landed = land_a_task().await;
    let Landed {
        client,
        project,
        task,
        session,
        ..
    } = &landed;
    merge(&landed);
    pulls_of(client, &task.id, true).await;
    assert_eq!(
        board(client, &project.id).await,
        [("Keep notes".to_owned(), pb::ItemStatus::Done)]
    );
    let worktree = Path::new(&session.working_directory);
    assert!(!worktree.exists());

    // Reopened and taken out again, with more work landed as another pull
    // request.
    let reopened = client
        .call(pb::request::Kind::UpdateItem(pb::UpdateItem {
            item_id: task.id.clone(),
            status: Some(pb::ItemStatus::Todo.into()),
            ..Default::default()
        }))
        .await;
    assert!(reopened.is_ok(), "{reopened:?}");
    let unarchived = client
        .call(pb::request::Kind::UpdateSession(pb::UpdateSession {
            session_id: session.id.clone(),
            archived: Some(false),
            ..Default::default()
        }))
        .await;
    assert!(unarchived.is_ok(), "{unarchived:?}");
    let sent = client
        .call(pb::request::Kind::SendMessage(pb::SendMessage {
            session_id: session.id.clone(),
            text: "write them again".to_owned(),
            mode: pb::SendMode::Enqueue.into(),
            client_id: String::new(),
            images: Vec::new(),
        }))
        .await;
    assert!(
        matches!(sent, Ok(pb::response::Kind::MessageSent(_))),
        "{sent:?}"
    );
    after_turns(client, &session.id, 2).await;
    std::fs::write(worktree.join("more.txt"), "more\n").unwrap_or_else(|error| panic!("{error}"));
    let again = client
        .call(pb::request::Kind::LandSession(pb::LandSession {
            session_id: session.id.clone(),
            branch: "keep-more".to_owned(),
            message: "Keep more".to_owned(),
            open_pull_requests: true,
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::SessionLanded(again)) = again else {
        panic!("{again:?}")
    };
    assert_eq!(again.pull_requests.len(), 1);
    assert_eq!(again.pull_requests[0].state(), pb::PullRequestState::Open);
    assert!(
        board(client, &project.id)
            .await
            .iter()
            .all(|(_, status)| *status != pb::ItemStatus::Done)
    );

    merge_at(&landed, "keep-more");
    let merged = pulls_of(client, &task.id, true).await;
    assert_eq!(merged[0].state(), pb::PullRequestState::Merged);
    assert_eq!(
        board(client, &project.id).await,
        [("Keep notes".to_owned(), pb::ItemStatus::Done)]
    );
    assert!(!worktree.exists());
}

/// A session of two repositories whose land opened a pull request in one,
/// `first`, and failed to in the other.
struct HalfLanded {
    _state: tempfile::TempDir,
    _repositories: [tempfile::TempDir; 2],
    _origins: tempfile::TempDir,
    hub: Arc<Mutex<Hub>>,
    client: Client,
    session: pb::SessionInfo,
    first: String,
}

const DESCRIBED: &str = "Keep notes\n\nSo they last.";

async fn land_described(
    client: &Client,
    session: &str,
) -> Result<pb::response::Kind, humata_wire::ClientError> {
    client
        .call(pb::request::Kind::LandSession(pb::LandSession {
            session_id: session.to_owned(),
            branch: "keep-notes".to_owned(),
            message: DESCRIBED.to_owned(),
            open_pull_requests: true,
            ..Default::default()
        }))
        .await
}

async fn land_half() -> HalfLanded {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let origins = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let (widgets, _) = repository_of(origins.path(), "widgets");
    let (gadgets, _) = repository_of(origins.path(), "gadgets");
    let hub = Arc::new(Mutex::new(Hub {
        second_open_fails: true,
        ..Hub::default()
    }));
    let client = daemon(state.path(), hosts_with(&hub)).await;
    let repository = |name: &str, path: &Path| pb::Repository {
        url: String::new(),
        name: name.to_owned(),
        path: path.display().to_string(),
    };
    let created = client
        .call(pb::request::Kind::CreateProject(pb::CreateProject {
            name: "both".to_owned(),
            repositories: vec![
                repository("widgets", widgets.path()),
                repository("gadgets", gadgets.path()),
            ],
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::ProjectCreated(pb::ProjectCreated {
        project: Some(project),
    })) = created
    else {
        panic!("{created:?}")
    };
    // A note in each repository's folder.
    let steps: Vec<serde_json::Value> = project
        .repositories
        .iter()
        .map(|repository| {
            serde_json::json!({
                "step": "write_file",
                "path": format!("{}/notes.txt", repository.name),
                "contents": "kept\n",
            })
        })
        .collect();
    let writes = serde_json::json!({ "rules": [{ "when": "", "steps": steps }] });
    let session = started(
        &client,
        pb::request::Kind::StartSession(pb::StartSession {
            backend: pb::Backend::Scripted.into(),
            project_id: project.id.clone(),
            backend_config: writes.to_string(),
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

    let failed = land_described(&client, &session.id).await;
    assert!(failed.is_err(), "{failed:?}");
    let first = {
        let mut hub = hub
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        hub.second_open_fails = false;
        let open: Vec<String> = hub.open.keys().cloned().collect();
        assert_eq!(open.len(), 1, "{open:?}");
        open[0].clone()
    };
    HalfLanded {
        _state: state,
        _repositories: [widgets, gadgets],
        _origins: origins,
        hub,
        client,
        session,
        first,
    }
}

#[tokio::test]
async fn a_pull_request_a_failed_land_opened_is_described_in_full_when_landed_again() {
    let half = land_half().await;
    let landed = land_described(&half.client, &half.session.id).await;
    let Ok(pb::response::Kind::SessionLanded(landed)) = landed else {
        panic!("{landed:?}")
    };
    assert_eq!(landed.pull_requests.len(), 2);

    let hub = half
        .hub
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(hub.opened, 2);
    let other = hub
        .open
        .keys()
        .find(|repository| **repository != half.first)
        .unwrap_or_else(|| panic!("{:?}", hub.open));
    // What the change is, and the pull request it goes with.
    let body = &hub.open[&half.first];
    assert!(body.starts_with("So they last."), "{body}");
    assert!(body.contains(&format!("acme/{other}#7")), "{body}");
}

#[tokio::test]
async fn a_description_people_changed_since_a_failed_land_is_left_alone() {
    let half = land_half().await;
    half.hub
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .open
        .insert(half.first.clone(), "Rewritten by hand.".to_owned());
    let landed = land_described(&half.client, &half.session.id).await;
    assert!(
        matches!(landed, Ok(pb::response::Kind::SessionLanded(_))),
        "{landed:?}"
    );
    let hub = half
        .hub
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(hub.open[&half.first], "Rewritten by hand.");
}

/// What opening a pull request fails with when the stand-in GitHub refuses
/// it with `refusal`.
async fn opening_refused(refusal: Refusal) -> GitHubError {
    let hub = Arc::new(Mutex::new(Hub {
        refusal: Some(refusal),
        ..Hub::default()
    }));
    let github = GitHub::new(&stand_in(hub), "test-token");
    let repo = Repo {
        owner: "acme".to_owned(),
        name: "widgets".to_owned(),
    };
    match github
        .open_pull(&repo, "Keep notes", "keep-notes", "main", "")
        .await
    {
        Err(error) => error,
        Ok(pull) => panic!("opened {pull:?}"),
    }
}

#[tokio::test]
async fn a_refusal_that_says_when_to_retry_is_the_rate_limit_for_that_long() {
    for status in ["403 Forbidden", "429 Too Many Requests"] {
        let error = opening_refused(Refusal {
            status,
            headers: "Retry-After: 61\r\n",
            reply: r#"{"message": "You have exceeded a secondary rate limit."}"#,
        })
        .await;
        assert!(
            matches!(error, GitHubError::RateLimited { wait_minutes: 2 }),
            "{status}: {error:?}"
        );
    }
}

#[tokio::test]
async fn a_refusal_for_lack_of_access_isnt_the_rate_limit() {
    let error = opening_refused(Refusal {
        status: "403 Forbidden",
        headers: "X-RateLimit-Remaining: 4999\r\n",
        reply: r#"{"message": "Resource protected by organization SAML enforcement."}"#,
    })
    .await;
    assert!(matches!(error, GitHubError::Forbidden { .. }), "{error:?}");
}

#[tokio::test]
async fn a_pull_request_github_cant_open_as_asked_is_invalid() {
    let error = opening_refused(Refusal {
        status: "422 Unprocessable Entity",
        headers: "",
        reply: r#"{"message": "Validation Failed",
            "errors": [{"message": "No commits between main and keep-notes"}]}"#,
    })
    .await;
    assert!(matches!(error, GitHubError::Invalid { .. }), "{error:?}");
}
