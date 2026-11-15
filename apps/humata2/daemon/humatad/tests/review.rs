//! DiffService and ReviewChatService through `humatad2`'s gRPC API, on a
//! real git clone in a session's folder (D66): what a diff shows, what
//! Mark reviewed snapshots, and the review chat's view, switches and Undo,
//! with what a session's agent may and may not do.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const SESSION: &str = "se_review";
const REPO: &str = "api";
const TOKEN_HEADER: &str = "humata-session-token";

/// A daemon that knows `SESSION`, whose folder holds a clone of `REPO`
/// with one commit.
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
    drop(store);
    let clone = dir.path().join(REPO);
    std::fs::create_dir(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    std::fs::write(clone.join("refund.rs"), lines(1..=10)).unwrap();
    git(&clone, &["add", "."]);
    git(&clone, &["commit", "-q", "-m", "first"]);
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

fn base(base: pb::diff_base::Base) -> Option<pb::DiffBase> {
    Some(pb::DiffBase { base: Some(base) })
}

fn uncommitted() -> Option<pb::DiffBase> {
    base(pb::diff_base::Base::UncommittedOnly(pb::UncommittedOnly {}))
}

async fn diffs(daemon: &Daemon) -> pb::diff_service_client::DiffServiceClient<Channel> {
    pb::diff_service_client::DiffServiceClient::new(daemon.channel().await)
}

async fn chat(daemon: &Daemon) -> pb::review_chat_service_client::ReviewChatServiceClient<Channel> {
    pb::review_chat_service_client::ReviewChatServiceClient::new(daemon.channel().await)
}

async fn get_diff(daemon: &Daemon, base: Option<pb::DiffBase>) -> pb::GetDiffResponse {
    diffs(daemon)
        .await
        .get_diff(pb::GetDiffRequest {
            session: Some(session()),
            base,
            repos: vec![repo()],
        })
        .await
        .unwrap()
        .into_inner()
}

fn paths(response: &pb::GetDiffResponse) -> Vec<String> {
    response
        .repos
        .iter()
        .flat_map(|repo| repo.files.iter().map(|file| file.path.clone()))
        .collect()
}

fn focus(path: &str, first: u32, last: u32) -> pb::FocusReviewRequest {
    pb::FocusReviewRequest {
        idempotency_key: None,
        session: Some(session()),
        file: Some(file(path)),
        lines: Some(pb::LineRange { first, last }),
        side: pb::DiffSide::New as i32,
        highlights: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_diff_lists_the_changed_files_and_gives_each_ones_hunks() {
    let (daemon, clone) = daemon_with_clone().await;
    assert!(!get_diff(&daemon, uncommitted()).await.any_changes, "a clean clone has no changes");

    std::fs::write(clone.join("refund.rs"), lines(1..=10).replace("line 5\n", "line five\n")).unwrap();
    std::fs::write(clone.join("new.rs"), "fn new() {}\n").unwrap();
    let diff = get_diff(&daemon, uncommitted()).await;
    assert!(diff.any_changes);
    let mut changed = paths(&diff);
    changed.sort();
    assert_eq!(changed, ["new.rs", "refund.rs"], "edits and untracked files both show");

    let hunks = diffs(&daemon)
        .await
        .get_file_diff(pb::GetFileDiffRequest {
            session: Some(session()),
            base: uncommitted(),
            file: Some(file("refund.rs")),
            trees: diff.repos[0].trees.clone(),
            ..pb::GetFileDiffRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    let texts: Vec<(i32, String)> = hunks
        .hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != pb::diff_line::Kind::Context as i32)
        .map(|line| (line.kind, line.text.clone()))
        .collect();
    assert_eq!(
        texts,
        [
            (pb::diff_line::Kind::Removed as i32, "line 5".to_owned()),
            (pb::diff_line::Kind::Added as i32, "line five".to_owned()),
        ]
    );

    // RV-29: viewed at this content, it stays folded.
    let viewed = diffs(&daemon)
        .await
        .get_file_diff(pb::GetFileDiffRequest {
            session: Some(session()),
            base: uncommitted(),
            file: Some(file("refund.rs")),
            viewed_blob_sha: hunks.new_blob_sha.clone(),
            ..pb::GetFileDiffRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(viewed.viewed, pb::ViewedResult::Unchanged as i32);
    assert!(viewed.hunks.is_empty());

    let unchanged = diffs(&daemon)
        .await
        .get_file_diff(pb::GetFileDiffRequest {
            session: Some(session()),
            base: uncommitted(),
            file: Some(file("README.md")),
            ..pb::GetFileDiffRequest::default()
        })
        .await
        .unwrap_err();
    assert_eq!(unchanged.code(), Code::NotFound, "a file without changes has no hunks");
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_reviewed_snapshots_what_was_shown_not_what_changed_since() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    let shown = get_diff(&daemon, uncommitted()).await;
    // Edited after the user saw the diff and before they marked it.
    std::fs::write(clone.join("late.rs"), "fn late() {}\n").unwrap();

    let marked = diffs(&daemon)
        .await
        .mark_reviewed(pb::MarkReviewedRequest {
            session: Some(session()),
            ..pb::MarkReviewedRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(marked.snapshots.len(), 1);
    let shown_tree = shown.repos[0].trees.as_ref().unwrap().new_tree.clone();
    assert_eq!(marked.snapshots[0].tree, shown_tree, "the snapshot is the diff as shown");

    let since = get_diff(&daemon, base(pb::diff_base::Base::SinceLastReviewed(pb::SinceLastReviewed {}))).await;
    assert_eq!(paths(&since), ["late.rs"], "only what came after the review is new");

    let kept = diffs(&daemon)
        .await
        .list_reviewed_snapshots(pb::ListReviewedSnapshotsRequest {
            session: Some(session()),
            repos: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(kept.snapshots.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shown_tree_from_the_client_wins_over_the_last_diff() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    let first = get_diff(&daemon, uncommitted()).await;
    std::fs::write(clone.join("refund.rs"), lines(1..=12)).unwrap();
    let second = get_diff(&daemon, uncommitted()).await;
    let tree = |diff: &pb::GetDiffResponse| diff.repos[0].trees.as_ref().unwrap().new_tree.clone();
    assert_ne!(tree(&first), tree(&second));

    // Another client showed the first diff, and says so.
    let marked = diffs(&daemon)
        .await
        .mark_reviewed(pb::MarkReviewedRequest {
            session: Some(session()),
            shown_tree: vec![pb::ShownTree {
                repo: Some(repo()),
                tree: tree(&first),
            }],
            ..pb::MarkReviewedRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(marked.snapshots[0].tree, tree(&first));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_review_chat_moves_the_view_and_the_user_can_undo_or_turn_it_off() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=10).replace("line 3\n", "line three\n")).unwrap();
    get_diff(&daemon, uncommitted()).await;
    let mut chat = chat(&daemon).await;

    let outside = chat.focus_review(focus("README.md", 1, 1)).await.unwrap_err();
    assert_eq!(outside.code(), Code::FailedPrecondition, "only files in the diff can be shown");

    let mut feed = chat
        .watch_review_changes(pb::WatchReviewChangesRequest {
            session: Some(session()),
            after_seq: 0,
        })
        .await
        .unwrap()
        .into_inner();
    let moved = chat.focus_review(focus("refund.rs", 3, 4)).await.unwrap().into_inner();
    let fed = tokio::time::timeout(WAIT, feed.message()).await.unwrap().unwrap().unwrap();
    assert_eq!(fed.undo_id, moved.undo_id, "the app's feed gets the change");

    let context = chat
        .get_review_context(pb::GetReviewContextRequest { session: Some(session()) })
        .await
        .unwrap()
        .into_inner();
    let open = context.view.unwrap().open.unwrap();
    assert_eq!(open.file.unwrap().path, "refund.rs");
    assert_eq!(context.selected_text, ["line three", "line 4"], "the selection comes quoted");

    let undone = chat
        .undo_review_change(pb::UndoReviewChangeRequest {
            session: Some(session()),
            undo_id: moved.undo_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(undone.undid, moved.undo_id);
    let context = chat
        .get_review_context(pb::GetReviewContextRequest { session: Some(session()) })
        .await
        .unwrap()
        .into_inner();
    assert!(context.view.unwrap().open.is_none(), "Undo puts the view back");

    chat.set_agent_switches(pb::SetAgentSwitchesRequest {
        session: Some(session()),
        switches: Some(pb::AgentSwitches {
            focus: false,
            notes: true,
            order: true,
        }),
    })
    .await
    .unwrap();
    let off = chat.focus_review(focus("refund.rs", 3, 4)).await.unwrap_err();
    assert_eq!(off.code(), Code::PermissionDenied);
    match error_of(&off).kind {
        Some(pb::error::Kind::Denied(denied)) => {
            assert_eq!(denied.source, pb::denied::Source::ReviewSwitchOff as i32);
        }
        other => panic!("a switched-off tool is denied, got {other:?}"),
    }
}

/// The session token a shell tab in `SESSION` is given, and the attach
/// that keeps the tab open.
async fn agent_token(daemon: &Daemon) -> (String, Attach) {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session()),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    let path = daemon.dir.path().join("token");
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

#[tokio::test(flavor = "multi_thread")]
async fn the_sessions_agent_may_read_and_focus_but_not_change_the_users_review_state() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    let (token, _attach) = agent_token(&daemon).await;

    let diff = diffs(&daemon)
        .await
        .get_diff(as_agent(
            pb::GetDiffRequest {
                session: Some(session()),
                base: uncommitted(),
                repos: vec![repo()],
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(paths(&diff), ["refund.rs"], "the agent reads its own session's diff");

    let mut chat = chat(&daemon).await;
    chat.focus_review(as_agent(focus("refund.rs", 11, 11), &token))
        .await
        .unwrap();
    chat.get_review_context(as_agent(pb::GetReviewContextRequest { session: Some(session()) }, &token))
        .await
        .unwrap();

    let marked = diffs(&daemon)
        .await
        .mark_reviewed(as_agent(
            pb::MarkReviewedRequest {
                session: Some(session()),
                ..pb::MarkReviewedRequest::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(error_of(&marked).kind, Some(pb::error::Kind::UserOnly(_))),
        "only the user decides what they've reviewed: {marked:?}"
    );
    let switched = chat
        .set_agent_switches(as_agent(
            pb::SetAgentSwitchesRequest {
                session: Some(session()),
                switches: None,
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&switched).kind, Some(pb::error::Kind::UserOnly(_))));
}
