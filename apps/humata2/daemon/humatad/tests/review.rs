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
    daemon_with_sessions(&[]).await
}

/// As [`daemon_with_clone`], with more sessions `others`, each with a
/// clone of `REPO` (from the first) in `<folder>/<session>`.
async fn daemon_with_sessions(others: &[&str]) -> (Daemon, PathBuf) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    let session = |id: &str, folder: PathBuf| SessionRecord {
        id: id.to_owned(),
        channel: "ch_test".to_owned(),
        machine: String::new(),
        title: "review".to_owned(),
        agent: "zsh".to_owned(),
        folder,
        created_unix_ms: 0,
    };
    store.put_session(&session(SESSION, dir.path().to_owned())).unwrap();
    let clone = dir.path().join(REPO);
    std::fs::create_dir(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    std::fs::write(clone.join("refund.rs"), lines(1..=10)).unwrap();
    git(&clone, &["add", "."]);
    git(&clone, &["commit", "-q", "-m", "first"]);
    for other in others {
        let folder = dir.path().join(other);
        std::fs::create_dir(&folder).unwrap();
        git(&folder, &["clone", "-q", clone.to_str().unwrap(), REPO]);
        store.put_session(&session(other, folder)).unwrap();
    }
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

#[tokio::test(flavor = "multi_thread")]
async fn a_diff_against_another_session_compares_with_the_commit_its_clone_is_on() {
    let (daemon, clone) = daemon_with_sessions(&["se_other"]).await;
    let theirs = daemon.dir.path().join("se_other").join(REPO);
    std::fs::write(theirs.join("theirs.rs"), "fn theirs() {}\n").unwrap();
    git(&theirs, &["add", "."]);
    git(&theirs, &["commit", "-q", "-m", "theirs"]);
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();

    let other = base(pb::diff_base::Base::OtherSession(pb::SessionId {
        value: "se_other".to_owned(),
    }));
    let shown = get_diff(&daemon, other).await;
    let files = &shown.repos[0].files;
    let change = |path: &str| {
        let file = files.iter().find(|file| file.path == path).unwrap();
        pb::ChangeKind::try_from(file.change).unwrap()
    };
    assert_eq!(paths(&shown), ["refund.rs", "theirs.rs"]);
    assert_eq!(change("refund.rs"), pb::ChangeKind::Modified);
    assert_eq!(change("theirs.rs"), pb::ChangeKind::Deleted, "this session doesn't have their file");
    assert_eq!(git(&theirs, &["status", "--porcelain"]), "", "their clone is left as it was");

    let unknown = diffs(&daemon)
        .await
        .get_diff(pb::GetDiffRequest {
            session: Some(session()),
            base: base(pb::diff_base::Base::OtherSession(pb::SessionId {
                value: "se_nowhere".to_owned(),
            })),
            repos: vec![repo()],
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
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

async fn comments(daemon: &Daemon) -> pb::comment_service_client::CommentServiceClient<Channel> {
    pb::comment_service_client::CommentServiceClient::new(daemon.channel().await)
}

async fn views(daemon: &Daemon) -> pb::view_service_client::ViewServiceClient<Channel> {
    pb::view_service_client::ViewServiceClient::new(daemon.channel().await)
}

fn note(path: &str, first: u32, last: u32, text: &str) -> pb::CreateCommentRequest {
    pb::CreateCommentRequest {
        idempotency_key: None,
        session: Some(session()),
        anchor: Some(pb::CommentAnchor {
            file: Some(file(path)),
            lines: Some(pb::LineRange { first, last }),
            side: pb::DiffSide::New as i32,
            ..pb::CommentAnchor::default()
        }),
        text: text.to_owned(),
        destination: Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::Note(pb::UserNote {})),
        }),
    }
}

fn note_id(comment: &pb::Comment) -> String {
    comment.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()
}

async fn context(daemon: &Daemon) -> pb::ReviewContext {
    chat(daemon)
        .await
        .get_review_context(pb::GetReviewContextRequest { session: Some(session()) })
        .await
        .unwrap()
        .into_inner()
}

fn note_texts(context: &pb::ReviewContext) -> Vec<(String, String)> {
    context
        .notes
        .iter()
        .map(|note| (note_id(note), note.text.clone()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agents_notes_are_kept_through_a_restart_and_never_reuse_an_id() {
    let (mut daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=10).replace("line 3\n", "line three\n")).unwrap();
    get_diff(&daemon, uncommitted()).await;
    let (token, _attach) = agent_token(&daemon).await;
    let mut notes = comments(&daemon).await;

    let outside = notes
        .create_comment(as_agent(note("README.md", 1, 1, "nope"), &token))
        .await
        .unwrap_err();
    assert_eq!(outside.code(), Code::FailedPrecondition, "notes go only on files the diff shows");

    let first = notes
        .create_comment(as_agent(note("refund.rs", 3, 3, "renamed for clarity"), &token))
        .await
        .unwrap()
        .into_inner();
    let second = notes
        .create_comment(as_agent(note("refund.rs", 4, 4, "unchanged"), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(!first.undo_id.is_empty());
    assert_eq!(first.anchor.as_ref().unwrap().quoted, "line three", "the note quotes its lines");
    let edited = notes
        .edit_comment(as_agent(
            pb::EditCommentRequest {
                idempotency_key: None,
                id: first.id.clone(),
                text: "renamed so it reads as a number".to_owned(),
                expected_revision: None,
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(edited.undo_id, first.undo_id, "the edit is its own change to undo");
    // The user drops one, naming it with its session.
    notes
        .drop_comment(pb::DropCommentRequest {
            idempotency_key: None,
            id: Some(pb::CommentId {
                value: format!("{SESSION}/{}", note_id(&second)),
            }),
            expected_revision: None,
        })
        .await
        .unwrap();
    let before = note_texts(&context(&daemon).await);
    assert_eq!(before, [(note_id(&first), "renamed so it reads as a number".to_owned())]);

    daemon.restart().await;

    assert_eq!(note_texts(&context(&daemon).await), before, "the notes survive the restart");
    let (token, _attach) = agent_token(&daemon).await;
    let third = comments(&daemon)
        .await
        .create_comment(as_agent(note("refund.rs", 5, 5, "after the restart"), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(
        ![note_id(&first), note_id(&second)].contains(&note_id(&third)),
        "{} was given out before the restart",
        note_id(&third)
    );
    assert!(![first.undo_id, edited.undo_id].contains(&third.undo_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn notes_are_the_agents_and_the_user_can_turn_them_off() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    get_diff(&daemon, uncommitted()).await;
    let (token, _attach) = agent_token(&daemon).await;

    let by_user = comments(&daemon)
        .await
        .create_comment(note("refund.rs", 11, 11, "mine"))
        .await
        .unwrap_err();
    assert_eq!(by_user.code(), Code::FailedPrecondition, "notes for the user come from the agent");

    chat(&daemon)
        .await
        .set_agent_switches(pb::SetAgentSwitchesRequest {
            session: Some(session()),
            switches: Some(pb::AgentSwitches {
                focus: true,
                notes: false,
                order: true,
            }),
        })
        .await
        .unwrap();
    let off = comments(&daemon)
        .await
        .create_comment(as_agent(note("refund.rs", 11, 11, "new line"), &token))
        .await
        .unwrap_err();
    assert_eq!(off.code(), Code::PermissionDenied);
    assert!(matches!(error_of(&off).kind, Some(pb::error::Kind::Denied(_))));
    assert!(context(&daemon).await.notes.is_empty());
}

async fn list_notes(daemon: &Daemon, request: pb::ListCommentsRequest) -> Vec<pb::Comment> {
    comments(daemon).await.list_comments(request).await.unwrap().into_inner().comments
}

fn all_notes() -> pb::ListCommentsRequest {
    pb::ListCommentsRequest {
        session: Some(session()),
        ..pb::ListCommentsRequest::default()
    }
}

/// Each note's id, lines and whether it is outdated.
fn note_places(notes: &[pb::Comment]) -> Vec<(String, u32, u32, bool)> {
    notes
        .iter()
        .map(|note| {
            let anchor = note.anchor.clone().unwrap_or_default();
            let lines = anchor.lines.unwrap_or_default();
            (note_id(note), lines.first, lines.last, anchor.outdated)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_later_diff_marks_notes_whose_lines_changed_outdated_and_moves_the_rest() {
    let (mut daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=10).replace("line 3\n", "line three\n")).unwrap();
    get_diff(&daemon, uncommitted()).await;
    let (token, _attach) = agent_token(&daemon).await;
    let mut notes = comments(&daemon).await;
    let changed = notes
        .create_comment(as_agent(note("refund.rs", 3, 3, "renamed"), &token))
        .await
        .unwrap()
        .into_inner();
    let kept = notes
        .create_comment(as_agent(note("refund.rs", 5, 5, "untouched"), &token))
        .await
        .unwrap()
        .into_inner();
    let (changed, kept) = (note_id(&changed), note_id(&kept));

    // The agent edits the line the first note quotes and adds one on top.
    let now = format!("line 0\n{}", lines(1..=10).replace("line 3\n", "line 3 again\n"));
    std::fs::write(clone.join("refund.rs"), now).unwrap();
    assert_eq!(
        note_places(&list_notes(&daemon, all_notes()).await),
        [(changed.clone(), 3, 3, false), (kept.clone(), 5, 5, false)],
        "nothing changes until a diff is shown"
    );
    let mut feed = chat(&daemon)
        .await
        .watch_review_changes(pb::WatchReviewChangesRequest {
            session: Some(session()),
            after_seq: 0,
        })
        .await
        .unwrap()
        .into_inner();
    let mut next = async || tokio::time::timeout(WAIT, feed.message()).await.unwrap().unwrap().unwrap();
    let made = [next().await.seq, next().await.seq];
    get_diff(&daemon, uncommitted()).await;

    let expected = [(changed, 3, 3, true), (kept, 6, 6, false)];
    assert_eq!(note_places(&list_notes(&daemon, all_notes()).await), expected);
    assert_eq!(note_places(&context(&daemon).await.notes), expected);
    let mut fed = Vec::new();
    for _ in 0..2 {
        let change = next().await;
        assert!(change.seq > made[1], "a note going outdated or moving is a new change");
        assert_eq!((change.tool.as_str(), change.undo_id.as_str()), ("diff", ""), "nobody can undo a diff");
        match change.change {
            Some(pb::review_change::Change::Note(note)) => fed.push(note),
            other => panic!("expected a note, got {other:?}"),
        }
    }
    assert_eq!(note_places(&fed), expected, "the app's feed hears of both");

    daemon.restart().await;
    assert_eq!(note_places(&list_notes(&daemon, all_notes()).await), expected, "a restart keeps it");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_comments_gives_the_review_chats_notes_by_file() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    std::fs::write(clone.join("new.rs"), "fn new() {}\n").unwrap();
    get_diff(&daemon, uncommitted()).await;
    assert!(list_notes(&daemon, all_notes()).await.is_empty());
    let (token, _attach) = agent_token(&daemon).await;
    let mut notes = comments(&daemon).await;
    let on_refund = notes
        .create_comment(as_agent(note("refund.rs", 11, 11, "new line"), &token))
        .await
        .unwrap()
        .into_inner();
    let on_new = notes
        .create_comment(as_agent(note("new.rs", 1, 1, "new file"), &token))
        .await
        .unwrap()
        .into_inner();

    let listed = list_notes(&daemon, all_notes()).await;
    assert_eq!(listed.iter().map(note_id).collect::<Vec<_>>(), [note_id(&on_refund), note_id(&on_new)]);
    assert!(listed.iter().all(|note| note.session == Some(session()) && note.author.is_some()));

    let refund_only = list_notes(
        &daemon,
        pb::ListCommentsRequest {
            file: Some(file("refund.rs")),
            ..all_notes()
        },
    )
    .await;
    assert_eq!(refund_only.iter().map(note_id).collect::<Vec<_>>(), [note_id(&on_refund)]);

    // Notes are never sent, so they have no status to filter on.
    let sent = list_notes(
        &daemon,
        pb::ListCommentsRequest {
            statuses: vec![pb::CommentStatus::Sent as i32],
            ..all_notes()
        },
    )
    .await;
    assert!(sent.is_empty());

    // The session's agent lists its own session's without naming it.
    let own = comments(&daemon)
        .await
        .list_comments(as_agent(pb::ListCommentsRequest::default(), &token))
        .await
        .unwrap()
        .into_inner()
        .comments;
    assert_eq!(own.len(), 2);
}

fn plan(order: &[&str], notes: Vec<pb::ReviewNote>) -> pb::PrepareViewRequest {
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
            order: order.iter().map(|path| file(path)).collect(),
            groups: Vec::new(),
            notes,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_in_an_open_review_chat_orders_the_review_and_leaves_its_notes() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    std::fs::write(clone.join("api.rs"), lines(1..=2)).unwrap();
    let (token, _attach) = agent_token(&daemon).await;
    let mut views = views(&daemon).await;
    let notes = vec![pb::ReviewNote {
        file: Some(file("refund.rs")),
        target: pb::review_note::Target::Lines as i32,
        lines: Some(pb::LineRange { first: 11, last: 11 }),
        side: pb::DiffSide::New as i32,
        text: "the new line".to_owned(),
        ..pb::ReviewNote::default()
    }];

    let closed = views
        .prepare_view(as_agent(plan(&["api.rs", "refund.rs"], notes.clone()), &token))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (closed.state, closed.undo_id.as_str()),
        (pb::ViewState::Draft as i32, ""),
        "outside an open review chat a plan is a draft view"
    );
    assert!(context(&daemon).await.view.unwrap().order.is_empty(), "a draft changes nothing on screen");

    chat(&daemon)
        .await
        .report_review_view(pb::ReportReviewViewRequest {
            session: Some(session()),
            view: Some(pb::ReviewViewState::default()),
        })
        .await
        .unwrap();
    let missing = views
        .prepare_view(as_agent(plan(&["gone.rs"], Vec::new()), &token))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::FailedPrecondition, "the order names only files the spec shows");

    let prepared = views
        .prepare_view(as_agent(plan(&["refund.rs", "api.rs"], notes), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(!prepared.undo_id.is_empty(), "it applied at once");
    let context = context(&daemon).await;
    let order: Vec<String> = context.view.unwrap().order.into_iter().map(|path| path.path).collect();
    assert_eq!(order, ["refund.rs", "api.rs"]);
    assert_eq!(
        context.notes.iter().map(|note| note.text.as_str()).collect::<Vec<_>>(),
        ["the new line"]
    );

    chat(&daemon)
        .await
        .undo_review_change(pb::UndoReviewChangeRequest {
            session: Some(session()),
            undo_id: prepared.undo_id,
        })
        .await
        .unwrap();
    let undone = self::context(&daemon).await;
    assert!(undone.view.unwrap().order.is_empty() && undone.notes.is_empty(), "Undo takes the plan back");
}

fn keyed<T>(mut request: T, key: &str, set: impl FnOnce(&mut T, Option<pb::IdempotencyKey>)) -> T {
    set(
        &mut request,
        Some(pb::IdempotencyKey {
            value: key.to_owned(),
        }),
    );
    request
}

async fn undo(daemon: &Daemon, undo_id: &str) -> Result<pb::ReviewChange, tonic::Status> {
    chat(daemon)
        .await
        .undo_review_change(pb::UndoReviewChangeRequest {
            session: Some(session()),
            undo_id: undo_id.to_owned(),
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_and_a_repeated_key_act_the_same_after_a_restart() {
    let (mut daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("refund.rs"), lines(1..=11)).unwrap();
    get_diff(&daemon, uncommitted()).await;
    chat(&daemon)
        .await
        .report_review_view(pb::ReportReviewViewRequest {
            session: Some(session()),
            view: Some(pb::ReviewViewState::default()),
        })
        .await
        .unwrap();
    let (token, _attach) = agent_token(&daemon).await;
    let keyed_note = || {
        keyed(
            note("refund.rs", 11, 11, "the new line"),
            "note-once",
            |request, key| {
                request.idempotency_key = key;
            },
        )
    };
    let plan_note = pb::ReviewNote {
        file: Some(file("refund.rs")),
        target: pb::review_note::Target::Lines as i32,
        lines: Some(pb::LineRange {
            first: 11,
            last: 11,
        }),
        side: pb::DiffSide::New as i32,
        text: "from the plan".to_owned(),
        ..pb::ReviewNote::default()
    };
    let keyed_plan = || {
        keyed(
            plan(&["refund.rs"], vec![plan_note.clone()]),
            "plan-once",
            |request, key| {
                request.idempotency_key = key;
            },
        )
    };

    let noted = comments(&daemon)
        .await
        .create_comment(as_agent(keyed_note(), &token))
        .await
        .unwrap()
        .into_inner();
    let prepared = views(&daemon)
        .await
        .prepare_view(as_agent(keyed_plan(), &token))
        .await
        .unwrap()
        .into_inner();
    let edited = comments(&daemon)
        .await
        .edit_comment(as_agent(
            pb::EditCommentRequest {
                idempotency_key: None,
                id: noted.id.clone(),
                text: "edited before the restart".to_owned(),
                expected_revision: None,
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    let before = note_texts(&context(&daemon).await);
    assert_eq!(before.len(), 2, "the note and the plan's note");

    daemon.restart().await;

    let (token, _attach) = agent_token(&daemon).await;
    let again = comments(&daemon)
        .await
        .create_comment(as_agent(keyed_note(), &token))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        note_id(&again),
        note_id(&noted),
        "a repeated key gets the first answer"
    );
    let replanned = views(&daemon)
        .await
        .prepare_view(as_agent(keyed_plan(), &token))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        replanned.undo_id, prepared.undo_id,
        "a repeated plan gets the first answer"
    );
    assert_eq!(
        note_texts(&context(&daemon).await),
        before,
        "neither acted again"
    );

    undo(&daemon, &edited.undo_id).await.unwrap();
    let texts: Vec<String> = note_texts(&context(&daemon).await)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    assert!(
        texts.contains(&"the new line".to_owned()),
        "Undo of an edit from before the restart: {texts:?}"
    );
    undo(&daemon, &prepared.undo_id).await.unwrap();
    let undone = context(&daemon).await;
    assert!(
        undone.view.as_ref().unwrap().order.is_empty(),
        "Undo takes back a plan from before the restart"
    );
    assert_eq!(note_texts(&undone).len(), 1, "and the plan's note");

    daemon.restart().await;
    assert!(
        undo(&daemon, &prepared.undo_id).await.is_err(),
        "an undone change stays undone"
    );
}

fn walkthrough(title: &str, path: &str) -> pb::PrepareViewRequest {
    pb::PrepareViewRequest {
        idempotency_key: None,
        title: title.to_owned(),
        summary: "Read the refund first".to_owned(),
        content: Some(pb::prepare_view_request::Content::Review(pb::ReviewWalkthrough {
            steps: vec![pb::review_walkthrough::Step {
                file: Some(file(path)),
                lines: Some(pb::LineRange { first: 1, last: 3 }),
                note: "the refund starts here".to_owned(),
            }],
        })),
    }
}

async fn list_views(daemon: &Daemon, token: Option<&str>) -> Vec<(String, String, i32)> {
    let request = pb::ListViewsRequest {
        session: Some(session()),
        ..pb::ListViewsRequest::default()
    };
    let request = match token {
        Some(token) => as_agent(request, token),
        None => Request::new(request),
    };
    views(daemon)
        .await
        .list_views(request)
        .await
        .unwrap()
        .into_inner()
        .views
        .into_iter()
        .map(|view| (view.id.unwrap().value, view.title, view.state))
        .collect()
}

fn set_state(id: &pb::ViewId, state: pb::ViewState) -> pb::SetViewStateRequest {
    pb::SetViewStateRequest {
        idempotency_key: None,
        id: Some(id.clone()),
        state: state as i32,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_view_is_the_agents_until_offered_then_the_users_and_a_restart_keeps_it() {
    let (mut daemon, _clone) = daemon_with_clone().await;
    let (token, _attach) = agent_token(&daemon).await;
    let mut views = views(&daemon).await;
    let keyed_draft = || {
        keyed(walkthrough("Refund walkthrough", "refund.rs"), "draft-once", |request, key| {
            request.idempotency_key = key;
        })
    };

    let draft = views.prepare_view(as_agent(keyed_draft(), &token)).await.unwrap().into_inner();
    let id = draft.id.clone().unwrap();
    assert_eq!(draft.state, pb::ViewState::Draft as i32);
    assert_eq!(draft.revision.unwrap().value, 1);
    let again = views.prepare_view(as_agent(keyed_draft(), &token)).await.unwrap().into_inner();
    assert_eq!(again.id, draft.id, "a repeated key makes no second draft");
    assert_eq!(list_views(&daemon, Some(&token)).await.len(), 1, "the agent sees its draft");
    assert!(list_views(&daemon, None).await.is_empty(), "a draft isn't offered (VW-14)");
    let early = views
        .set_view_state(set_state(&id, pb::ViewState::Opened))
        .await
        .unwrap_err();
    assert_eq!(early.code(), Code::InvalidArgument, "the user can't answer a draft");

    let revise = |title: &str, expected: u64| pb::ReviseViewRequest {
        id: Some(id.clone()),
        title: title.to_owned(),
        expected_revision: Some(pb::Revision { value: expected }),
        ..pb::ReviseViewRequest::default()
    };
    let revised = views.revise_view(as_agent(revise("Refund, step by step", 1), &token)).await.unwrap().into_inner();
    assert_eq!(
        (revised.title.as_str(), revised.summary.as_str(), revised.revision.unwrap().value),
        ("Refund, step by step", "Read the refund first", 2),
        "a revision replaces what it names and keeps the rest"
    );
    let stale = views.revise_view(as_agent(revise("Stale", 1), &token)).await.unwrap_err();
    assert_eq!(stale.code(), Code::Aborted, "a stale revision is a conflict");

    let ready = views
        .mark_ready(as_agent(
            pb::MarkReadyRequest {
                idempotency_key: None,
                id: Some(id.clone()),
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ready.state, pb::ViewState::Waiting as i32);
    let late = views.revise_view(as_agent(revise("Too late", 3), &token)).await.unwrap_err();
    assert_eq!(late.code(), Code::InvalidArgument, "an offered view can't be revised");
    assert!(
        views
            .set_view_state(as_agent(set_state(&id, pb::ViewState::Kept), &token))
            .await
            .is_err(),
        "only the user answers an offer"
    );
    let later = views
        .set_view_state(set_state(&id, pb::ViewState::Later))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(later.state, pb::ViewState::Later as i32);

    daemon.restart().await;

    let offered = vec![(
        id.value.clone(),
        "Refund, step by step".to_owned(),
        pb::ViewState::Later as i32,
    )];
    assert_eq!(list_views(&daemon, None).await, offered, "a restart keeps the view and its state");
    let (token, _attach) = agent_token(&daemon).await;
    let repeated = crate::views(&daemon)
        .await
        .prepare_view(as_agent(keyed_draft(), &token))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(repeated.id, draft.id, "a repeated key after a restart makes no new draft");
    assert_eq!(list_views(&daemon, Some(&token)).await, offered);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_view_is_checked_before_it_is_kept() {
    let (daemon, _clone) = daemon_with_sessions(&["se_other"]).await;
    let (token, _attach) = agent_token(&daemon).await;
    let mut views = views(&daemon).await;
    let prepare = |content| pb::PrepareViewRequest {
        title: "A view".to_owned(),
        content: Some(content),
        ..pb::PrepareViewRequest::default()
    };
    let board = |scope| {
        prepare(pb::prepare_view_request::Content::Board(pb::BoardViewOffer { scope: Some(scope) }))
    };

    let by_user = views.prepare_view(walkthrough("Mine", "refund.rs")).await.unwrap_err();
    assert_eq!(by_user.code(), Code::FailedPrecondition, "only a session's agent prepares views");
    let untitled = views.prepare_view(as_agent(walkthrough("  ", "refund.rs"), &token)).await.unwrap_err();
    assert_eq!(untitled.code(), Code::InvalidArgument, "a view needs a title");
    let empty = views
        .prepare_view(as_agent(
            prepare(pb::prepare_view_request::Content::Review(pb::ReviewWalkthrough::default())),
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(empty.code(), Code::InvalidArgument, "a walkthrough needs a step");
    let no_repos = views
        .prepare_view(as_agent(
            prepare(pb::prepare_view_request::Content::PullRequest(pb::PullRequestDraft::default())),
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(no_repos.code(), Code::InvalidArgument, "a pull request view needs a repo");
    let elsewhere = views
        .prepare_view(as_agent(
            board(pb::board_view_offer::Scope::Channel(pb::ChannelId {
                value: "ch_elsewhere".to_owned(),
            })),
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(elsewhere.code(), Code::InvalidArgument, "a board view stays in the session's channel");
    let gone = views
        .prepare_view(as_agent(plan(&["gone.rs"], Vec::new()), &token))
        .await
        .unwrap_err();
    assert_eq!(gone.code(), Code::FailedPrecondition, "a draft plan names only files its spec shows");
    assert!(list_views(&daemon, Some(&token)).await.is_empty(), "nothing refused was kept");

    let neighbour = views
        .prepare_view(as_agent(
            board(pb::board_view_offer::Scope::OneSession(pb::SessionId {
                value: "se_other".to_owned(),
            })),
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(neighbour.state, pb::ViewState::Draft as i32, "a session of the same channel is fine");
    let by_user = views
        .mark_ready(pb::MarkReadyRequest {
            idempotency_key: None,
            id: neighbour.id.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(by_user.code(), Code::FailedPrecondition, "the user doesn't offer the agent's draft");
    let unknown = views
        .mark_ready(as_agent(
            pb::MarkReadyRequest {
                idempotency_key: None,
                id: Some(pb::ViewId {
                    value: "vw_missing".to_owned(),
                }),
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
}
