//! CommentService's comments for the session's agent (SM-7, SM-9-SM-21,
//! D138) through `humatad2`'s gRPC API, on real git clones in a session's
//! folder: drafting, sending into the agent's terminal, what a restart
//! keeps, and how a comment follows its lines.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_chunk::{Codec, Encoder};
use humata2_grpc::humata::v1 as pb;
use sha2::{Digest, Sha256};
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const SESSION: &str = "se_review";
const TOKEN_HEADER: &str = "humata-session-token";

/// A daemon that knows `SESSION`, whose folder holds one clone per name in
/// `repos`, each with `refund.rs` (ten lines) committed.
async fn daemon_with_repos(repos: &[&str]) -> (Daemon, PathBuf) {
    daemon_with_repos_env(repos, &[]).await
}

/// As [`daemon_with_repos`], with environment variables for the daemon's
/// first start only.
async fn daemon_with_repos_env(repos: &[&str], env: &[(&str, &str)]) -> (Daemon, PathBuf) {
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
    for name in repos {
        let clone = dir.path().join(name);
        std::fs::create_dir(&clone).unwrap();
        git(&clone, &["init", "-q", "-b", "main"]);
        std::fs::write(clone.join("refund.rs"), lines()).unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-q", "-m", "first"]);
    }
    drop(store);
    let folder = dir.path().to_owned();
    (Daemon::start_env(dir, env).await, folder)
}

async fn daemon_with_clone() -> (Daemon, PathBuf) {
    let (daemon, folder) = daemon_with_repos(&["api"]).await;
    let clone = folder.join("api");
    (daemon, clone)
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

const NAMES: [&str; 10] = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];

/// `refund.rs` as committed: "line one" .. "line ten".
fn lines() -> String {
    NAMES.iter().map(|name| format!("line {name}\n")).collect()
}

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

async fn comments(daemon: &Daemon) -> pb::comment_service_client::CommentServiceClient<Channel> {
    pb::comment_service_client::CommentServiceClient::new(daemon.channel().await)
}

fn anchor(repo: &str, path: &str, first: u32, last: u32) -> pb::CommentAnchor {
    pb::CommentAnchor {
        file: Some(pb::RepoPath {
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: repo.to_owned(),
            }),
            path: path.to_owned(),
        }),
        lines: Some(pb::LineRange { first, last }),
        side: pb::DiffSide::New as i32,
        ..pb::CommentAnchor::default()
    }
}

fn draft_in(repo: &str, path: &str, first: u32, last: u32, text: &str) -> pb::CreateCommentRequest {
    pb::CreateCommentRequest {
        idempotency_key: None,
        session: Some(session()),
        anchor: Some(anchor(repo, path, first, last)),
        text: text.to_owned(),
        destination: Some(pb::CommentDestination {
            destination: Some(pb::comment_destination::Destination::Terminal(pb::SessionTerminal {})),
        }),
    }
}

fn draft(first: u32, last: u32, text: &str) -> pb::CreateCommentRequest {
    draft_in("api", "refund.rs", first, last, text)
}

async fn create(daemon: &Daemon, request: pb::CreateCommentRequest) -> pb::Comment {
    comments(daemon).await.create_comment(request).await.unwrap().into_inner()
}

fn id_of(comment: &pb::Comment) -> pb::CommentId {
    comment.id.clone().unwrap()
}

fn local(comment: &pb::Comment) -> String {
    id_of(comment).value.rsplit('/').next().unwrap().to_owned()
}

async fn list_with(daemon: &Daemon, statuses: &[pb::CommentStatus], archived: bool) -> pb::ListCommentsResponse {
    comments(daemon)
        .await
        .list_comments(pb::ListCommentsRequest {
            session: Some(session()),
            statuses: statuses.iter().map(|status| *status as i32).collect(),
            file: None,
            page: None,
            archived,
        })
        .await
        .unwrap()
        .into_inner()
}

async fn listed(daemon: &Daemon) -> pb::ListCommentsResponse {
    list_with(daemon, &[], false).await
}

/// The comment as listed, whatever its status or archive.
async fn find(daemon: &Daemon, comment: &pb::Comment) -> pb::Comment {
    for archived in [false, true] {
        let all = [
            pb::CommentStatus::Draft,
            pb::CommentStatus::Queued,
            pb::CommentStatus::Sent,
            pb::CommentStatus::Addressed,
            pb::CommentStatus::Resolved,
        ];
        if let Some(found) = list_with(daemon, &all, archived).await.comments.into_iter().find(|c| c.id == comment.id) {
            return found;
        }
    }
    panic!("comment {:?} isn't listed", comment.id);
}

fn status_of(comment: &pb::Comment) -> pb::CommentStatus {
    pb::CommentStatus::try_from(comment.status).unwrap()
}

fn texts(response: &pb::ListCommentsResponse) -> Vec<String> {
    response.comments.iter().map(|comment| comment.text.clone()).collect()
}

async fn send(daemon: &Daemon, ids: &[&pb::Comment]) -> Result<pb::SendCommentsResponse, tonic::Status> {
    comments(daemon)
        .await
        .send_comments(pb::SendCommentsRequest {
            idempotency_key: None,
            session: Some(session()),
            ids: ids.iter().map(|comment| id_of(comment)).collect(),
            overall_note: String::new(),
        })
        .await
        .map(tonic::Response::into_inner)
}

/// A shell tab in `SESSION` (the one a Send types into), the attach that
/// keeps it open, and the session token a shell in it is given.
async fn shell_with_token(daemon: &Daemon) -> (String, pb::TabId, Attach) {
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
            return (token, tab, attach);
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

/// Reads until nothing more comes for a moment.
async fn quiet(attach: &mut Attach) {
    while let Ok(read) = tokio::time::timeout(Duration::from_millis(600), attach.read()).await {
        read.unwrap();
    }
}

/// How many times `needle` appears in everything `tab` ever showed.
async fn shown(daemon: &Daemon, tab: &pb::TabId, needle: &str) -> usize {
    let mut attach = Attach::start(daemon, open(tab, 0)).await;
    quiet(&mut attach).await;
    String::from_utf8_lossy(&attach.screen).matches(needle).count()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_drafts_edits_lists_and_drops_comments() {
    let (daemon, _clone) = daemon_with_clone().await;
    let first = create(&daemon, draft(3, 3, "rename this")).await;
    let second = create(&daemon, draft(5, 6, "why two lines")).await;
    assert_eq!(status_of(&first), pb::CommentStatus::Draft);
    assert_eq!(id_of(&first).value, format!("{SESSION}/c-1"));
    assert_eq!(first.anchor.as_ref().unwrap().quoted, "line three", "the lines are read from the clone");
    assert_eq!(second.anchor.as_ref().unwrap().quoted, "line five\nline six");

    let edited = comments(&daemon)
        .await
        .edit_comment(pb::EditCommentRequest {
            idempotency_key: None,
            id: first.id.clone(),
            text: "rename this to total".to_owned(),
            expected_revision: first.revision.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(edited.text, "rename this to total");

    let after = listed(&daemon).await;
    assert_eq!(texts(&after), ["rename this to total", "why two lines"]);
    assert_eq!(after.draft_count, 2);

    // A change made from an older read is refused, not merged.
    let stale = comments(&daemon)
        .await
        .edit_comment(pb::EditCommentRequest {
            idempotency_key: None,
            id: first.id.clone(),
            text: "lost update".to_owned(),
            expected_revision: first.revision.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Code::Aborted);

    comments(&daemon)
        .await
        .drop_comment(pb::DropCommentRequest {
            idempotency_key: None,
            id: second.id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap();
    let after = listed(&daemon).await;
    assert_eq!(texts(&after), ["rename this to total"]);
    assert_eq!(after.draft_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sessions_agent_cannot_change_the_users_comments() {
    let (daemon, _clone) = daemon_with_clone().await;
    let held = create(&daemon, draft(3, 3, "mine")).await;
    let (token, _tab, _attach) = shell_with_token(&daemon).await;
    let mut agent = comments(&daemon).await;

    let created = agent.create_comment(as_agent(draft(4, 4, "from the agent"), &token)).await.unwrap_err();
    let edited = agent
        .edit_comment(as_agent(
            pb::EditCommentRequest {
                idempotency_key: None,
                id: held.id.clone(),
                text: "changed".to_owned(),
                expected_revision: None,
            },
            &token,
        ))
        .await
        .unwrap_err();
    let sent = agent
        .send_comments(as_agent(
            pb::SendCommentsRequest {
                idempotency_key: None,
                session: Some(session()),
                ids: vec![id_of(&held)],
                overall_note: String::new(),
            },
            &token,
        ))
        .await
        .unwrap_err();
    let archived = agent
        .archive_comment(as_agent(
            pb::ArchiveCommentRequest {
                idempotency_key: None,
                id: held.id.clone(),
            },
            &token,
        ))
        .await
        .unwrap_err();
    let taken = agent
        .take_back_comments(as_agent(
            pb::TakeBackCommentsRequest {
                idempotency_key: None,
                session: Some(session()),
                ids: vec![id_of(&held)],
            },
            &token,
        ))
        .await
        .unwrap_err();
    for refused in [&created, &edited, &sent, &archived, &taken] {
        assert!(
            matches!(error_of(refused).kind, Some(pb::error::Kind::UserOnly(_))),
            "only the user changes their comments: {refused:?}"
        );
    }
    // A draft was never sent to the agent, so the agent can't resolve it either.
    agent
        .resolve_comment(as_agent(
            pb::ResolveCommentRequest {
                idempotency_key: None,
                id: held.id.clone(),
                expected_revision: None,
                reply: "done".to_owned(),
            },
            &token,
        ))
        .await
        .unwrap_err();

    let after = find(&daemon, &held).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Draft);
    assert_eq!(after.text, "mine");
    assert_eq!(listed(&daemon).await.comments.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn send_types_one_message_into_the_agents_terminal_once() {
    let (daemon, _clone) = daemon_with_clone().await;
    let (_token, tab, mut attach) = shell_with_token(&daemon).await;
    let first = create(&daemon, draft(3, 3, "rename this")).await;
    let second = create(&daemon, draft(5, 6, "why two lines")).await;
    let aside = create(&daemon, draft(8, 8, "not this one yet")).await;

    let sent = send(&daemon, &[&first, &second]).await.unwrap();
    assert!(!sent.queued);
    assert_eq!(sent.comments.len(), 2);
    for comment in &sent.comments {
        assert_eq!(status_of(comment), pb::CommentStatus::Sent);
        assert!(comment.sent_at.is_some());
    }

    attach.wait_for("[comment c-1 on refund.rs:3]").await;
    attach.wait_for("why two lines").await;
    let screen = String::from_utf8_lossy(&attach.screen).into_owned();
    let (a, b) = (screen.find("[comment c-1 on refund.rs:3]").unwrap(), screen.find("[comment c-2 on refund.rs:5-6]").unwrap());
    assert!(a < b, "the comments arrive in the order they were sent");
    assert!(!screen.contains("api/refund.rs"), "a session with one clone leaves the repo out");
    quiet(&mut attach).await;
    assert!(!attach.shows("not this one yet"), "a comment not named stays a draft");
    assert_eq!(status_of(&find(&daemon, &aside).await), pb::CommentStatus::Draft);

    // Sending them again types nothing more.
    let again = send(&daemon, &[&first]).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition);
    assert_eq!(shown(&daemon, &tab, "[comment c-1 on refund.rs:3]").await, 1);

    // With no ids, Send takes every draft there is.
    let rest = send(&daemon, &[]).await.unwrap();
    assert_eq!(rest.comments.len(), 1);
    attach.wait_for("not this one yet").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_two_clones_names_the_repo() {
    let (daemon, _folder) = daemon_with_repos(&["api", "web"]).await;
    let (_token, _tab, mut attach) = shell_with_token(&daemon).await;
    let comment = create(&daemon, draft_in("web", "refund.rs", 2, 2, "in the other repo")).await;
    send(&daemon, &[&comment]).await.unwrap();
    attach.wait_for("[comment c-1 on web/refund.rs:2]").await;
}

/// A request to comment on a file with no repository named.
fn unnamed_repo_draft(path: &str, first: u32, last: u32, text: &str) -> pb::CreateCommentRequest {
    let mut request = draft_in("api", path, first, last, text);
    if let Some(file) = request.anchor.as_mut().and_then(|anchor| anchor.file.as_mut()) {
        file.repo = None;
    }
    request
}

fn named_repo(comment: &pb::Comment) -> Option<String> {
    comment
        .anchor
        .as_ref()
        .and_then(|anchor| anchor.file.as_ref())
        .and_then(|file| file.repo.as_ref())
        .map(|repo| repo.name.clone())
        .filter(|name| !name.is_empty())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_comment_on_a_session_folder_file_needs_no_repo_even_with_two_clones() {
    let (daemon, folder) = daemon_with_repos(&["ledger", "api"]).await;
    std::fs::write(folder.join("rfc-brin.md"), lines()).unwrap();
    std::fs::create_dir(folder.join("notes")).unwrap();
    std::fs::write(folder.join("notes").join("bench-results.md"), lines()).unwrap();
    let (_token, _tab, mut attach) = shell_with_token(&daemon).await;

    let document = create(&daemon, unnamed_repo_draft("rfc-brin.md", 7, 7, "tighten this")).await;
    assert_eq!(named_repo(&document), None, "a folder file has no repository");
    assert_eq!(document.anchor.as_ref().unwrap().quoted, "line seven");
    let nested = create(&daemon, unnamed_repo_draft("notes/bench-results.md", 2, 2, "units?")).await;
    assert_eq!(named_repo(&nested), None);

    // A file that is in a clone, not named, still needs the repo.
    let refused = comments(&daemon)
        .await
        .create_comment(unnamed_repo_draft("refund.rs", 3, 3, "which one?"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    // So does a path that climbs out of the folder, or isn't a file.
    for path in ["../rfc-brin.md", "notes", "missing.md"] {
        let refused = comments(&daemon).await.create_comment(unnamed_repo_draft(path, 1, 1, "no")).await;
        assert!(refused.is_err(), "{path} is not a folder file");
    }

    send(&daemon, &[&document]).await.unwrap();
    attach.wait_for("[comment c-1 on rfc-brin.md:7]").await;
    let screen = String::from_utf8_lossy(&attach.screen).into_owned();
    assert!(!screen.contains("/rfc-brin.md") && !screen.contains(":rfc-brin.md"));

    let back = find(&daemon, &document).await;
    assert_eq!(named_repo(&back), None);
    assert_eq!(back.anchor.as_ref().unwrap().file.as_ref().unwrap().path, "rfc-brin.md");

    // Listing and resolving by that file see only it.
    let by_file = comments(&daemon)
        .await
        .list_comments(pb::ListCommentsRequest {
            session: Some(session()),
            statuses: Vec::new(),
            file: Some(pb::RepoPath {
                repo: None,
                path: "rfc-brin.md".to_owned(),
            }),
            page: None,
            archived: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(texts(&by_file), vec!["tighten this".to_owned()]);

    // Rewriting the line addresses the comment, as in a clone.
    let text: String = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| if i == 6 { "line seven rewritten\n".to_owned() } else { format!("line {name}\n") })
        .collect();
    std::fs::write(folder.join("rfc-brin.md"), text).unwrap();
    let after = find(&daemon, &document).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Addressed);
    assert_eq!(after.addressed_text, "line seven rewritten");
}

#[tokio::test(flavor = "multi_thread")]
async fn drafts_sent_and_archived_comments_survive_a_restart_and_nothing_is_typed_again() {
    let (mut daemon, _clone) = daemon_with_clone().await;
    let (_token, tab, mut attach) = shell_with_token(&daemon).await;
    let draft_one = create(&daemon, draft(1, 1, "still a draft")).await;
    let sent_one = create(&daemon, draft(2, 2, "sent before")).await;
    let archived_one = create(&daemon, draft(3, 3, "set aside")).await;
    let dropped = create(&daemon, draft(4, 4, "dropped")).await;
    send(&daemon, &[&sent_one]).await.unwrap();
    comments(&daemon).await.archive_comment(pb::ArchiveCommentRequest {
        idempotency_key: None,
        id: archived_one.id.clone(),
    })
    .await
    .unwrap();
    comments(&daemon)
        .await
        .drop_comment(pb::DropCommentRequest {
            idempotency_key: None,
            id: dropped.id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap();
    attach.wait_for("[comment c-2 on refund.rs:2]").await;
    let typed_before = shown(&daemon, &tab, "[comment c-2 on refund.rs:2]").await;

    daemon.restart().await;

    let after = listed(&daemon).await;
    assert_eq!(texts(&after), ["still a draft", "sent before"]);
    assert_eq!(after.draft_count, 1);
    let sent = find(&daemon, &sent_one).await;
    assert_eq!(status_of(&sent), pb::CommentStatus::Sent);
    assert!(sent.sent_at.is_some());
    assert_eq!(status_of(&find(&daemon, &draft_one).await), pb::CommentStatus::Draft);
    let archived = list_with(&daemon, &[], true).await;
    assert_eq!(texts(&archived), ["set aside"]);
    assert!(archived.comments[0].archived_at.is_some());

    let later = create(&daemon, draft(6, 6, "after the restart")).await;
    assert_eq!(local(&later), "c-5", "c-4 was dropped and isn't given out again");

    // Settle anything the start might have typed, then count.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(shown(&daemon, &tab, "[comment c-2 on refund.rs:2]").await, typed_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn comments_wait_when_there_is_no_terminal_and_are_typed_once_when_there_is() {
    let (daemon, _clone) = daemon_with_clone().await;
    let first = create(&daemon, draft(3, 3, "first")).await;
    let second = create(&daemon, draft(4, 4, "second")).await;

    // No tab in the session: Humata holds them.
    let held = send(&daemon, &[&first, &second]).await.unwrap();
    assert!(held.queued);
    assert!(held.comments.iter().all(|comment| status_of(comment) == pb::CommentStatus::Queued));
    assert!(held.comments.iter().all(|comment| comment.sent_at.is_none()));

    let back = comments(&daemon)
        .await
        .take_back_comments(pb::TakeBackCommentsRequest {
            idempotency_key: None,
            session: Some(session()),
            ids: vec![id_of(&first)],
        })
        .await
        .unwrap()
        .into_inner();
    assert!(back.comments.iter().any(|comment| comment.id == first.id));
    assert!(back.comments.iter().all(|comment| status_of(comment) == pb::CommentStatus::Draft));
    assert_eq!(status_of(&find(&daemon, &first).await), pb::CommentStatus::Draft);
    assert_eq!(listed(&daemon).await.draft_count, 1, "the other comment is still held, not a draft");

    // A tab exists now.
    let (_token, tab, mut attach) = shell_with_token(&daemon).await;
    let sent = send(&daemon, &[]).await.unwrap();
    assert!(!sent.queued);
    attach.wait_for("[comment c-1 on refund.rs:3]").await;
    attach.wait_for("[comment c-2 on refund.rs:4]").await;
    assert_eq!(shown(&daemon, &tab, "[comment c-1 on refund.rs:3]").await, 1);
    for comment in [&first, &second] {
        assert_eq!(status_of(&find(&daemon, comment).await), pb::CommentStatus::Sent);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_comment_is_addressed_when_its_lines_change_and_a_draft_detaches() {
    let (daemon, clone) = daemon_with_clone().await;
    let (_token, _tab, mut attach) = shell_with_token(&daemon).await;
    let changed = create(&daemon, draft(3, 3, "rename")).await;
    let spaced = create(&daemon, draft(5, 5, "spacing")).await;
    let moved = create(&daemon, draft(9, 9, "unchanged but moved")).await;
    let waiting = create(&daemon, draft(7, 7, "a draft")).await;
    send(&daemon, &[&changed, &spaced, &moved]).await.unwrap();
    attach.wait_for("[comment c-3 on refund.rs:9]").await;

    // Whitespace alone changes nothing; a new line above line nine moves it.
    let text: String = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| match i + 1 {
            3 => "line three renamed\n".to_owned(),
            5 => "   line   five  \n".to_owned(),
            7 => "something else entirely\n".to_owned(),
            9 => format!("// inserted\nline {name}\n"),
            _ => format!("line {name}\n"),
        })
        .collect();
    std::fs::write(clone.join("refund.rs"), text).unwrap();

    let after = find(&daemon, &changed).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Addressed);
    assert_eq!(after.addressed_text, "line three renamed");
    assert!(after.addressed.is_some());
    assert_eq!(after.anchor.as_ref().unwrap().quoted, "line three", "the quote stays as it was");
    assert_eq!(status_of(&find(&daemon, &spaced).await), pb::CommentStatus::Sent, "whitespace is not a change");
    let moved = find(&daemon, &moved).await;
    assert_eq!(status_of(&moved), pb::CommentStatus::Sent);
    assert_eq!(moved.anchor.as_ref().unwrap().lines, Some(pb::LineRange { first: 10, last: 10 }));

    let waiting_now = find(&daemon, &waiting).await;
    assert!(waiting_now.anchor.as_ref().unwrap().detached);
    let reattached = comments(&daemon)
        .await
        .reattach_comment(pb::ReattachCommentRequest {
            idempotency_key: None,
            id: waiting.id.clone(),
            anchor: Some(anchor("api", "refund.rs", 8, 8)),
            expected_revision: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!reattached.anchor.as_ref().unwrap().detached);
    assert!(!find(&daemon, &waiting).await.anchor.unwrap().detached);
}

/// What the user's editor does: WriteFile of `data` over `path` in the session's clone of `api`.
async fn user_writes(daemon: &Daemon, path: &str, data: &[u8]) {
    use pb::write_file_message::Message as Part;

    let chunk = Encoder::new(Codec::None).encode(data.to_vec()).unwrap();
    let sha256: String = Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect();
    let messages = vec![
        pb::WriteFileMessage {
            message: Some(Part::Header(pb::WriteFileHeader {
                idempotency_key: None,
                location: Some(pb::FileLocation {
                    machine: None,
                    root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                        session: Some(session()),
                        repo: Some(pb::RepoRef {
                            owner: "acme".to_owned(),
                            name: "api".to_owned(),
                        }),
                    })),
                    path: path.to_owned(),
                }),
                size: data.len() as u64,
                sha256,
                replace: true,
                expected_sha256: None,
            })),
        },
        pb::WriteFileMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset: 0,
                data: chunk.data,
                last: true,
                codec: chunk.header.codec.wire(),
                raw_size: chunk.header.raw_len,
                crc32: chunk.header.crc32,
                ..Default::default()
            })),
        },
    ];
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
        .write_file(tokio_stream::iter(messages))
        .await
        .unwrap_or_else(|status| panic!("the user's WriteFile of {path}: {status}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_comment_stays_sent_when_the_user_writes_its_lines_through_files() {
    let (daemon, clone) = daemon_with_clone().await;
    let (_token, _tab, mut attach) = shell_with_token(&daemon).await;
    let edited = create(&daemon, draft(3, 3, "rename")).await;
    let below = create(&daemon, draft(9, 9, "below")).await;
    send(&daemon, &[&edited, &below]).await.unwrap();
    attach.wait_for("[comment c-2 on refund.rs:9]").await;

    let text: String = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| match i + 1 {
            3 => "line three renamed\n".to_owned(),
            9 => format!("// inserted\nline {name}\n"),
            _ => format!("line {name}\n"),
        })
        .collect();
    user_writes(&daemon, "refund.rs", text.as_bytes()).await;

    let edited = find(&daemon, &edited).await;
    assert_eq!(status_of(&edited), pb::CommentStatus::Sent, "the user's own edit does not address it");
    let anchor = edited.anchor.as_ref().unwrap();
    assert_eq!(anchor.quoted, "line three renamed", "the anchor follows the new text");
    assert_eq!(anchor.lines, Some(pb::LineRange { first: 3, last: 3 }));
    let below = find(&daemon, &below).await;
    assert_eq!(status_of(&below), pb::CommentStatus::Sent);
    assert_eq!(below.anchor.as_ref().unwrap().lines, Some(pb::LineRange { first: 10, last: 10 }));

    // The agent then changes the line on disk: that does address it.
    let again = text.replace("line three renamed", "line three by the agent");
    std::fs::write(clone.join("refund.rs"), again).unwrap();
    let after = find(&daemon, &edited).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Addressed);
    assert_eq!(after.addressed_text, "line three by the agent");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sent_comment_is_addressed_when_the_file_is_changed_on_disk() {
    let (daemon, clone) = daemon_with_clone().await;
    let (_token, _tab, mut attach) = shell_with_token(&daemon).await;
    let comment = create(&daemon, draft(3, 3, "rename")).await;
    send(&daemon, &[&comment]).await.unwrap();
    attach.wait_for("[comment c-1 on refund.rs:3]").await;

    let text: String = NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| match i + 1 {
            3 => "line three renamed\n".to_owned(),
            _ => format!("line {name}\n"),
        })
        .collect();
    std::fs::write(clone.join("refund.rs"), text).unwrap();

    let after = find(&daemon, &comment).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Addressed);
    assert_eq!(after.addressed_text, "line three renamed");
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_reopen_archive_and_unarchive_move_a_comment_between_lists() {
    let (daemon, _clone) = daemon_with_clone().await;
    let (_token, _tab, mut attach) = shell_with_token(&daemon).await;
    let comment = create(&daemon, draft(3, 3, "to resolve")).await;
    send(&daemon, &[&comment]).await.unwrap();
    attach.wait_for("[comment c-1 on refund.rs:3]").await;

    let resolved = comments(&daemon)
        .await
        .resolve_comment(pb::ResolveCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
            expected_revision: None,
            reply: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(status_of(&resolved), pb::CommentStatus::Resolved);
    assert!(resolved.resolved.is_some());
    assert!(listed(&daemon).await.comments.is_empty(), "resolved comments are left out by default");
    let only_resolved = list_with(&daemon, &[pb::CommentStatus::Resolved], false).await;
    assert_eq!(only_resolved.comments.len(), 1);

    let reopened = comments(&daemon)
        .await
        .reopen_comment(pb::ReopenCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(status_of(&reopened), pb::CommentStatus::Draft);
    assert_eq!(listed(&daemon).await.draft_count, 1);

    let archived = comments(&daemon)
        .await
        .archive_comment(pb::ArchiveCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(archived.archived_at.is_some());
    assert_eq!(status_of(&archived), pb::CommentStatus::Draft, "archiving keeps the status");
    let hidden = listed(&daemon).await;
    assert!(hidden.comments.is_empty());
    assert_eq!(hidden.draft_count, 0, "an archived comment isn't counted");
    assert!(list_with(&daemon, &[pb::CommentStatus::Draft], false).await.comments.is_empty());
    assert_eq!(list_with(&daemon, &[], true).await.comments.len(), 1);

    let back = comments(&daemon)
        .await
        .unarchive_comment(pb::UnarchiveCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(back.archived_at.is_none());
    assert_eq!(status_of(&back), pb::CommentStatus::Draft);
    assert_eq!(listed(&daemon).await.comments.len(), 1);
    assert!(list_with(&daemon, &[], true).await.comments.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_tells_of_a_comment_made_changed_and_dropped() {
    let (daemon, _clone) = daemon_with_clone().await;
    let mut events = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    async fn next(events: &mut tonic::Streaming<pb::Event>) -> (i32, String, String) {
        let event = tokio::time::timeout(WAIT, events.message()).await.unwrap().unwrap().unwrap();
        match event.payload {
            Some(pb::event::Payload::Comment(comment)) => {
                (event.change, comment.id.map(|id| id.value).unwrap_or_default(), comment.text)
            }
            other => panic!("expected a comment, got {other:?}"),
        }
    }

    let made = create(&daemon, draft(3, 3, "first words")).await;
    assert_eq!(
        next(&mut events).await,
        (pb::Change::Created as i32, id_of(&made).value, "first words".to_owned())
    );
    comments(&daemon)
        .await
        .edit_comment(pb::EditCommentRequest {
            idempotency_key: None,
            id: made.id.clone(),
            text: "better words".to_owned(),
            expected_revision: None,
        })
        .await
        .unwrap();
    assert_eq!(
        next(&mut events).await,
        (pb::Change::Updated as i32, id_of(&made).value, "better words".to_owned())
    );
    comments(&daemon)
        .await
        .drop_comment(pb::DropCommentRequest {
            idempotency_key: None,
            id: made.id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap();
    assert_eq!(next(&mut events).await.0, pb::Change::Deleted as i32);
}

type Watch = tonic::Streaming<pb::CommentFeedEvent>;

async fn watch(daemon: &Daemon) -> Watch {
    comments(daemon)
        .await
        .watch_comments(pb::WatchCommentsRequest { session: Some(session()) })
        .await
        .unwrap()
        .into_inner()
}

async fn next_event(watch: &mut Watch) -> pb::comment_feed_event::Event {
    tokio::time::timeout(WAIT, watch.message())
        .await
        .expect("the feed told nothing in time")
        .unwrap()
        .expect("the feed ended")
        .event
        .expect("an event with nothing in it")
}

#[tokio::test(flavor = "multi_thread")]
async fn watching_comments_starts_with_a_resync_then_tells_each_change_in_order() {
    let (daemon, _clone) = daemon_with_clone().await;
    let mut feed = watch(&daemon).await;
    assert!(matches!(next_event(&mut feed).await, pb::comment_feed_event::Event::Resync(true)));

    let made = create(&daemon, draft(3, 3, "first words")).await;
    comments(&daemon)
        .await
        .edit_comment(pb::EditCommentRequest {
            idempotency_key: None,
            id: made.id.clone(),
            text: "better words".to_owned(),
            expected_revision: None,
        })
        .await
        .unwrap();
    comments(&daemon)
        .await
        .drop_comment(pb::DropCommentRequest {
            idempotency_key: None,
            id: made.id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap();

    match next_event(&mut feed).await {
        pb::comment_feed_event::Event::Changed(comment) => {
            assert_eq!(comment.id, made.id);
            assert_eq!(comment.text, "first words");
        }
        other => panic!("expected the new comment, got {other:?}"),
    }
    match next_event(&mut feed).await {
        pb::comment_feed_event::Event::Changed(comment) => {
            assert_eq!(comment.id, made.id);
            assert_eq!(comment.text, "better words");
        }
        other => panic!("expected the edited comment, got {other:?}"),
    }
    match next_event(&mut feed).await {
        pb::comment_feed_event::Event::Dropped(id) => assert_eq!(Some(id), made.id),
        other => panic!("expected the dropped id, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn watching_comments_tells_of_an_archive_with_the_time_it_was_archived() {
    let (daemon, _clone) = daemon_with_clone().await;
    let made = create(&daemon, draft(3, 3, "to keep")).await;
    let mut feed = watch(&daemon).await;
    assert!(matches!(next_event(&mut feed).await, pb::comment_feed_event::Event::Resync(true)));

    comments(&daemon)
        .await
        .archive_comment(pb::ArchiveCommentRequest {
            idempotency_key: None,
            id: made.id.clone(),
        })
        .await
        .unwrap();
    match next_event(&mut feed).await {
        pb::comment_feed_event::Event::Changed(comment) => {
            assert_eq!(comment.id, made.id);
            assert!(comment.archived_at.is_some());
        }
        other => panic!("expected the archived comment, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sessions_agent_cannot_watch_the_users_comments() {
    let (daemon, _clone) = daemon_with_clone().await;
    let (token, _tab, _attach) = shell_with_token(&daemon).await;
    let refused = comments(&daemon)
        .await
        .watch_comments(as_agent(pb::WatchCommentsRequest { session: Some(session()) }, &token))
        .await
        .unwrap_err();
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))),
        "only the user watches their comments: {refused:?}"
    );
}

/// How often `needle` appears in what the program in the tab was typed, as
/// that program wrote it to `typed` (see [`killed_while_sending`]). A file
/// the program writes itself, so it can be read while the daemon is dead.
fn typed_count(typed: &Path, needle: &str) -> usize {
    let read = std::fs::read(typed).unwrap_or_default();
    String::from_utf8_lossy(&read).matches(needle).count()
}

/// Waits until the comment is listed with `status`.
async fn wait_for_status(daemon: &Daemon, comment: &pb::Comment, status: pb::CommentStatus) -> pb::Comment {
    let deadline = Instant::now() + WAIT;
    loop {
        let found = find(daemon, comment).await;
        if status_of(&found) == status {
            return found;
        }
        assert!(Instant::now() < deadline, "the comment never became {status:?}: {found:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

const MARK: &str = "[comment c-1 on refund.rs:3]";

/// The daemon dies at `stop` while sending one comment; returns the tab,
/// the comment and the file the tab's program copies each line it is typed
/// to, with the daemon dead and its holder still running.
async fn killed_while_sending(stop: &str) -> (Daemon, pb::TabId, pb::Comment, PathBuf) {
    let (mut daemon, _clone) = daemon_with_repos_env(&["api"], &[("HUMATA2_STOP_AT", stop)]).await;
    let (_token, tab, mut attach) = shell_with_token(&daemon).await;
    // From here the tab runs a program that writes what it reads to a file:
    // what reached the terminal can then be told without the daemon.
    let typed = daemon.dir.path().join("typed");
    attach.type_text(&format!("cat > {}\r", typed.display())).await;
    let deadline = Instant::now() + WAIT;
    while !typed.exists() {
        assert!(Instant::now() < deadline, "the tab's shell never started the program");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let comment = create(&daemon, draft(3, 3, "rename this")).await;
    assert!(send(&daemon, &[&comment]).await.is_err(), "the daemon should have died");
    daemon.kill();
    (daemon, tab, comment, typed)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_typed_before_the_daemon_died_is_not_typed_again() {
    let (mut daemon, _tab, comment, typed) = killed_while_sending("comments.typed").await;
    // The paste reached the terminal, which the holder keeps running.
    let deadline = Instant::now() + WAIT;
    while typed_count(&typed, MARK) == 0 {
        assert!(Instant::now() < deadline, "the paste never reached the terminal");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    daemon.restart().await;

    let sent = wait_for_status(&daemon, &comment, pb::CommentStatus::Sent).await;
    assert!(!sent.delivery_unconfirmed);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(typed_count(&typed, MARK), 1);
}

/// How each comment batch the last start found ended, as that start's
/// restart report names it: `(id, state)` of the `comment_send` lines.
fn comment_sends_named(state: &Path) -> Vec<(String, String)> {
    let mut folders: Vec<_> = std::fs::read_dir(state.join("restarts"))
        .expect("the daemon wrote restart reports")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    let file = folders
        .iter()
        .rev()
        .map(|folder| folder.join("operations.jsonl"))
        .find(|file| file.exists())
        .expect("a restart report names the operations");
    let mut latest: Vec<(String, String)> = Vec::new();
    for line in std::fs::read_to_string(file).unwrap().lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value["kind"] != "comment_send" {
            continue;
        }
        let entry = (value["id"].as_str().unwrap_or_default().to_owned(), value["state"].as_str().unwrap_or_default().to_owned());
        match latest.iter_mut().find(|known| known.0 == entry.0) {
            Some(known) => *known = entry,
            None => latest.push(entry),
        }
    }
    latest
}

/// A batch whose paste reached the terminal before the daemon died is
/// named in the restart report, finished: it arrived and is not typed again.
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_typed_before_the_daemon_died_is_named_in_the_restart_report_as_finished() {
    let (mut daemon, _tab, comment, typed) = killed_while_sending("comments.typed").await;
    let deadline = Instant::now() + WAIT;
    while typed_count(&typed, MARK) == 0 {
        assert!(Instant::now() < deadline, "the paste never reached the terminal");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    daemon.restart().await;

    wait_for_status(&daemon, &comment, pb::CommentStatus::Sent).await;
    // The report is written before the start is done.
    common::last_start(&daemon).await;
    let named = comment_sends_named(&daemon.state);
    assert_eq!(named.len(), 1, "{named:?}");
    assert_eq!(named[0].1, "finished", "{named:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_begun_but_not_typed_before_the_daemon_died_is_typed_once_after() {
    let (mut daemon, _tab, comment, typed) = killed_while_sending("comments.begun").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(typed_count(&typed, MARK), 0, "nothing was typed before the death");

    daemon.restart().await;

    let sent = wait_for_status(&daemon, &comment, pb::CommentStatus::Sent).await;
    assert!(!sent.delivery_unconfirmed);
    let deadline = Instant::now() + WAIT;
    while typed_count(&typed, MARK) == 0 {
        assert!(Instant::now() < deadline, "the comment was never typed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(typed_count(&typed, MARK), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_whose_holder_was_lost_with_the_daemon_stays_unconfirmed_and_is_not_typed() {
    let (mut daemon, tab, comment, typed) = killed_while_sending("comments.begun").await;
    common::kill_holders(&common::stored_tabs(&daemon.state)).await;

    daemon.restart().await;

    // The holder that comes back is a new one: its "never arrived" proves
    // nothing, so the user decides.
    let held = wait_for_status(&daemon, &comment, pb::CommentStatus::Queued).await;
    assert!(held.delivery_unconfirmed);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(shown(&daemon, &tab, MARK).await, 0);
    assert_eq!(typed_count(&typed, MARK), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agents_reply_and_resolve_happen_once_per_idempotency_key() {
    let (daemon, _clone) = daemon_with_clone().await;
    let (token, _tab, mut attach) = shell_with_token(&daemon).await;
    let comment = create(&daemon, draft(3, 3, "rename this")).await;
    send(&daemon, &[&comment]).await.unwrap();
    attach.wait_for("[comment c-1 on refund.rs:3]").await;
    let mut agent = comments(&daemon).await;
    let key = |value: &str| {
        Some(pb::IdempotencyKey {
            value: value.to_owned(),
        })
    };

    for _ in 0..2 {
        agent
            .reply_to_comment(as_agent(
                pb::ReplyToCommentRequest {
                    idempotency_key: key("reply-1"),
                    id: comment.id.clone(),
                    text: "looking".to_owned(),
                    resolve: false,
                },
                &token,
            ))
            .await
            .unwrap();
    }
    assert_eq!(find(&daemon, &comment).await.replies.len(), 1);

    for _ in 0..2 {
        agent
            .resolve_comment(as_agent(
                pb::ResolveCommentRequest {
                    idempotency_key: key("resolve-1"),
                    id: comment.id.clone(),
                    expected_revision: None,
                    reply: "renamed".to_owned(),
                },
                &token,
            ))
            .await
            .unwrap();
    }
    let after = find(&daemon, &comment).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Resolved);
    let thread: Vec<&str> = after.replies.iter().map(|reply| reply.text.as_str()).collect();
    assert_eq!(thread, ["looking", "renamed"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_archived_comment_takes_no_answer_from_the_agent() {
    let (daemon, _clone) = daemon_with_clone().await;
    let (token, _tab, mut attach) = shell_with_token(&daemon).await;
    let comment = create(&daemon, draft(3, 3, "rename this")).await;
    send(&daemon, &[&comment]).await.unwrap();
    attach.wait_for("[comment c-1 on refund.rs:3]").await;
    comments(&daemon)
        .await
        .archive_comment(pb::ArchiveCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
        })
        .await
        .unwrap();
    let mut agent = comments(&daemon).await;

    agent
        .reply_to_comment(as_agent(
            pb::ReplyToCommentRequest {
                idempotency_key: None,
                id: comment.id.clone(),
                text: "hello".to_owned(),
                resolve: false,
            },
            &token,
        ))
        .await
        .unwrap_err();
    agent
        .resolve_comment(as_agent(
            pb::ResolveCommentRequest {
                idempotency_key: None,
                id: comment.id.clone(),
                expected_revision: None,
                reply: "done".to_owned(),
            },
            &token,
        ))
        .await
        .unwrap_err();

    comments(&daemon)
        .await
        .unarchive_comment(pb::UnarchiveCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
        })
        .await
        .unwrap();
    let after = find(&daemon, &comment).await;
    assert_eq!(status_of(&after), pb::CommentStatus::Sent);
    assert!(after.replies.is_empty());
}

async fn resolve_addressed(daemon: &Daemon, file: Option<pb::RepoPath>) -> pb::ResolveAddressedCommentsResponse {
    comments(daemon)
        .await
        .resolve_addressed_comments(pb::ResolveAddressedCommentsRequest {
            idempotency_key: None,
            session: Some(session()),
            file,
        })
        .await
        .unwrap()
        .into_inner()
}

/// `refund.rs` with each of the one-based `lines` rewritten.
fn rewritten(changed: &[usize]) -> String {
    NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if changed.contains(&(i + 1)) {
                format!("line {name} rewritten\n")
            } else {
                format!("line {name}\n")
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_addressed_resolves_every_addressed_comment_of_a_file_or_of_the_session() {
    let (daemon, folder) = daemon_with_repos(&["api", "web"]).await;
    let (token, _tab, _attach) = shell_with_token(&daemon).await;
    let api_three = create(&daemon, draft_in("api", "refund.rs", 3, 3, "rename")).await;
    let api_five = create(&daemon, draft_in("api", "refund.rs", 5, 5, "spacing")).await;
    let web_two = create(&daemon, draft_in("web", "refund.rs", 2, 2, "web")).await;
    let api_eight = create(&daemon, draft_in("api", "refund.rs", 8, 8, "untouched")).await;
    send(&daemon, &[&api_three, &api_five, &web_two, &api_eight]).await.unwrap();

    std::fs::write(folder.join("api").join("refund.rs"), rewritten(&[3, 5])).unwrap();
    std::fs::write(folder.join("web").join("refund.rs"), rewritten(&[2])).unwrap();
    for comment in [&api_three, &api_five, &web_two] {
        wait_for_status(&daemon, comment, pb::CommentStatus::Addressed).await;
    }

    // The agent may not resolve the user's comments in bulk.
    let refused = comments(&daemon)
        .await
        .resolve_addressed_comments(as_agent(
            pb::ResolveAddressedCommentsRequest {
                idempotency_key: None,
                session: Some(session()),
                file: None,
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))));
    assert_eq!(status_of(&find(&daemon, &api_three).await), pb::CommentStatus::Addressed);

    let api_file = api_three.anchor.as_ref().unwrap().file.clone();
    let resolved = resolve_addressed(&daemon, api_file.clone()).await;
    let mut ids: Vec<_> = resolved.comments.iter().map(|comment| comment.id.clone()).collect();
    ids.sort_by_key(|id| id.as_ref().map(|id| id.value.clone()));
    assert_eq!(ids, [api_three.id.clone(), api_five.id.clone()]);
    for comment in &resolved.comments {
        assert_eq!(status_of(comment), pb::CommentStatus::Resolved);
    }
    assert_eq!(status_of(&find(&daemon, &web_two).await), pb::CommentStatus::Addressed);
    assert_eq!(status_of(&find(&daemon, &api_eight).await), pb::CommentStatus::Sent);

    let rest = resolve_addressed(&daemon, None).await;
    assert_eq!(rest.comments.len(), 1);
    assert_eq!(rest.comments[0].id, web_two.id);
    assert_eq!(status_of(&find(&daemon, &web_two).await), pb::CommentStatus::Resolved);
    assert_eq!(status_of(&find(&daemon, &api_eight).await), pb::CommentStatus::Sent);

    assert!(resolve_addressed(&daemon, None).await.comments.is_empty(), "a repeat resolves nothing");
}

async fn list_page(
    daemon: &Daemon,
    statuses: &[pb::CommentStatus],
    page: Option<pb::PageRequest>,
) -> Result<pb::ListCommentsResponse, tonic::Status> {
    comments(daemon)
        .await
        .list_comments(pb::ListCommentsRequest {
            session: Some(session()),
            statuses: statuses.iter().map(|status| *status as i32).collect(),
            file: None,
            page,
            archived: false,
        })
        .await
        .map(tonic::Response::into_inner)
}

fn page_of(size: u32, token: &str) -> Option<pb::PageRequest> {
    Some(pb::PageRequest {
        page_size: size,
        page_token: token.to_owned(),
    })
}

fn token_of(response: &pb::ListCommentsResponse) -> String {
    response.page.as_ref().map(|page| page.next_page_token.clone()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn list_comments_hands_out_every_comment_once_in_pages() {
    let (daemon, _clone) = daemon_with_clone().await;
    let mut made = Vec::new();
    for (i, line) in [1, 2, 3, 4, 5].into_iter().enumerate() {
        made.push(create(&daemon, draft(line, line, &format!("note {i}"))).await);
    }
    let draft_only = [pb::CommentStatus::Draft];

    // Without a page, everything comes back at once.
    let all = list_page(&daemon, &draft_only, None).await.unwrap();
    assert_eq!(all.comments.len(), 5);
    assert!(token_of(&all).is_empty());

    let first = list_page(&daemon, &draft_only, page_of(2, "")).await.unwrap();
    assert_eq!(first.comments.len(), 2);
    assert!(!token_of(&first).is_empty());
    assert_eq!(first.draft_count, 5, "the count is of all drafts, not the page");

    // The comment the token names is dropped before the next page is asked for.
    let token = token_of(&first);
    comments(&daemon)
        .await
        .drop_comment(pb::DropCommentRequest {
            idempotency_key: None,
            id: first.comments[1].id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap();
    let second = list_page(&daemon, &draft_only, page_of(2, &token)).await.unwrap();
    assert_eq!(second.comments.iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [made[2].id.clone(), made[3].id.clone()]);
    assert!(!token_of(&second).is_empty());

    let third = list_page(&daemon, &draft_only, page_of(2, &token_of(&second))).await.unwrap();
    assert_eq!(third.comments.iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [made[4].id.clone()]);
    assert!(token_of(&third).is_empty());

    let bad = list_page(&daemon, &draft_only, page_of(2, "nonsense")).await.unwrap_err();
    assert_eq!(bad.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_comment_with_no_destination_is_refused_as_missing_one() {
    let (daemon, _clone) = daemon_with_clone().await;
    let mut unset = draft(3, 3, "rename this");
    unset.destination = None;
    let mut empty = draft(3, 3, "rename this");
    empty.destination = Some(pb::CommentDestination { destination: None });
    for request in [unset, empty] {
        let refused = comments(&daemon).await.create_comment(request).await.unwrap_err();
        let (field, problem) = common::invalid_field(&refused);
        assert_eq!(field, "destination");
        assert_eq!(problem, pb::invalid_field::Problem::Missing);
    }
    assert!(listed(&daemon).await.comments.is_empty(), "nothing is drafted");
}


/// Every state the last start's report names for a `comment_send`, in the
/// order the lines were written.
fn comment_send_states(state: &Path) -> Vec<String> {
    let mut folders: Vec<_> = std::fs::read_dir(state.join("restarts"))
        .expect("the daemon wrote restart reports")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    let file = folders
        .iter()
        .rev()
        .map(|folder| folder.join("operations.jsonl"))
        .find(|file| file.exists())
        .expect("a restart report names the operations");
    std::fs::read_to_string(file)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["kind"] == "comment_send")
        .map(|value| value["state"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// A batch found in doubt is named pending first, then how it ended: the
/// report never reads as finished while one is owed.
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_found_in_doubt_is_named_pending_before_it_is_named_finished() {
    let (mut daemon, _tab, comment, typed) = killed_while_sending("comments.typed").await;
    let deadline = Instant::now() + WAIT;
    while typed_count(&typed, MARK) == 0 {
        assert!(Instant::now() < deadline, "the paste never reached the terminal");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    daemon.restart().await;

    wait_for_status(&daemon, &comment, pb::CommentStatus::Sent).await;
    common::last_start(&daemon).await;
    assert_eq!(comment_send_states(&daemon.state), ["pending", "finished"]);
}
