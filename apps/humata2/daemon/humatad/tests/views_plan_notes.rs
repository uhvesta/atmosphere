//! Notes of a review plan prepared as a view stay attached while the files
//! change (RV-34, VIEWS): ListViews reads each plan's notes against the
//! clones as they are now. A note whose lines only moved is shown at its new
//! lines; a note whose lines changed is kept and marked outdated. The store
//! keeps the plan as it was prepared. Real git clones, a real daemon.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::Request;

const SESSION: &str = "se_plan_notes";
const REPO: &str = "api";
const TOKEN_HEADER: &str = "humata-session-token";

/// A daemon that knows `SESSION`, whose folder holds a clone of `REPO` with
/// one commit of `refund.rs` (lines 1 to 10).
async fn daemon_with_clone() -> (Daemon, PathBuf) {
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
    std::fs::write(clone.join("refund.rs"), lines(1..=10)).unwrap();
    git(&clone, &["add", "."]);
    git(&clone, &["commit", "-q", "-m", "first"]);
    drop(store);
    (Daemon::start_in(dir).await, clone)
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

fn lines(range: std::ops::RangeInclusive<u32>) -> String {
    range.map(|n| format!("line {n}\n")).collect()
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

fn file(path: &str) -> pb::RepoPath {
    pb::RepoPath {
        repo: Some(repo()),
        path: path.to_owned(),
    }
}

/// The session token a shell tab in `SESSION` is given, and the attach that
/// keeps the tab open.
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

/// A plan of `refund.rs` with a note on line 11 (new side), which the clone's
/// uncommitted line 11 is.
fn plan() -> pb::PrepareViewRequest {
    pb::PrepareViewRequest {
        idempotency_key: None,
        title: "Refund review".to_owned(),
        summary: String::new(),
        content: Some(pb::prepare_view_request::Content::ReviewPlan(pb::ReviewPlan {
            spec: Some(pb::DiffSpec {
                repos: vec![pb::RepoDiffSpec {
                    repo: Some(repo()),
                    base: Some(pb::DiffSpecBase {
                        base: Some(pb::diff_spec_base::Base::Rev("HEAD".to_owned())),
                    }),
                    ..pb::RepoDiffSpec::default()
                }],
            }),
            order: vec![file("refund.rs")],
            groups: Vec::new(),
            notes: vec![
                pb::ReviewNote {
                    file: Some(file("refund.rs")),
                    target: pb::review_note::Target::Lines as i32,
                    lines: Some(pb::LineRange { first: 11, last: 11 }),
                    side: pb::DiffSide::New as i32,
                    text: "the new line".to_owned(),
                    ..pb::ReviewNote::default()
                },
                pb::ReviewNote {
                    file: Some(file("refund.rs")),
                    target: pb::review_note::Target::File as i32,
                    text: "about the file".to_owned(),
                    ..pb::ReviewNote::default()
                },
            ],
            ..pb::ReviewPlan::default()
        })),
    }
}

/// An offered plan, prepared with line 11 added to `refund.rs`; then the
/// user's view of its notes after `change` is made to the clone.
async fn notes_after(change: impl FnOnce(&Path)) -> Vec<pb::ReviewNote> {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    let (token, _attach) = agent_token(&daemon).await;
    let mut views = views(&daemon).await;
    let draft = views.prepare_view(as_agent(plan(), &token)).await.unwrap().into_inner();
    views
        .mark_ready(as_agent(
            pb::MarkReadyRequest {
                idempotency_key: None,
                id: draft.id.clone(),
            },
            &token,
        ))
        .await
        .unwrap();

    change(&clone);

    let listed = views
        .list_views(pb::ListViewsRequest {
            session: Some(session()),
            ..pb::ListViewsRequest::default()
        })
        .await
        .unwrap()
        .into_inner()
        .views;
    let Some(pb::prepared_view::Content::ReviewPlan(plan)) = listed.into_iter().next().and_then(|view| view.content)
    else {
        panic!("the session lists its offered plan");
    };
    plan.notes
}

fn line_note(notes: &[pb::ReviewNote]) -> &pb::ReviewNote {
    notes
        .iter()
        .find(|note| note.target == pb::review_note::Target::Lines as i32)
        .expect("the plan keeps its line note")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_on_a_line_that_changed_is_listed_outdated() {
    let notes = notes_after(|clone| {
        std::fs::write(clone.join("refund.rs"), format!("{}line eleven\n", lines(1..=10))).unwrap();
    })
    .await;

    let note = line_note(&notes);
    assert!(note.outdated, "the line the note was on changed");
    assert_eq!(note.text, "the new line", "an outdated note is kept");
    assert_eq!(note.lines, Some(pb::LineRange { first: 11, last: 11 }), "shown where it was");
    let file_note = notes.iter().find(|note| note.target == pb::review_note::Target::File as i32).unwrap();
    assert!(!file_note.outdated, "a note on the file isn't made outdated by edits");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_whose_lines_only_moved_is_listed_at_its_new_lines() {
    let notes = notes_after(|clone| {
        std::fs::write(clone.join("refund.rs"), format!("inserted\n{}", lines(1..=11))).unwrap();
    })
    .await;

    let note = line_note(&notes);
    assert!(!note.outdated, "the lines are as they were");
    assert_eq!(note.lines, Some(pb::LineRange { first: 12, last: 12 }), "one line was added above");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_of_untouched_lines_is_listed_as_prepared() {
    let notes = notes_after(|_| {}).await;

    let note = line_note(&notes);
    assert!(!note.outdated);
    assert_eq!(note.lines, Some(pb::LineRange { first: 11, last: 11 }));
}
