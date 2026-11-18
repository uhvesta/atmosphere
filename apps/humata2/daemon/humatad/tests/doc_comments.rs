//! The comments a Desk file had when a version was made (DV-12, DV-6) and
//! the strip updating by itself (DV-5): an agent's plain `git commit` of a
//! file records that file's comments as they were, later changes to the
//! comments leave the record alone, and the session is told it changed
//! without anyone asking for the list.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{Build, Daemon, WAIT, World, folder, git, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::transport::Channel;

struct Fixture {
    daemon: Daemon,
    _world: World,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        Self { daemon, _world: world }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    async fn start(&self, title: &str) -> (pb::SessionId, PathBuf) {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: title.to_owned(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        wait_status(&self.daemon, &id, pb::SessionStatus::Working).await;
        let path = PathBuf::from(started.desk.unwrap().folder.unwrap().path);
        (id, path)
    }

    async fn list(&self, id: &pb::SessionId, path: &str) -> pb::ListDocVersionsResponse {
        self.desk()
            .await
            .list_doc_versions(pb::ListDocVersionsRequest {
                session: Some(id.clone()),
                path: path.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn save(&self, id: &pb::SessionId, message: &str) -> Result<pb::DocVersion, tonic::Status> {
        self.desk()
            .await
            .save_doc_version(pb::SaveDocVersionRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                message: message.to_owned(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn read(&self, id: &pb::SessionId, path: &str, version: &str) -> pb::DocVersionText {
        self.desk()
            .await
            .read_doc_version(pb::ReadDocVersionRequest {
                session: Some(id.clone()),
                path: path.to_owned(),
                version: version.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn desk_session(&self, id: &pb::SessionId) -> pb::DeskSession {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .sessions
            .into_iter()
            .find(|desk| desk.session.as_ref() == Some(id))
            .unwrap()
    }

    async fn comments(&self) -> pb::comment_service_client::CommentServiceClient<Channel> {
        pb::comment_service_client::CommentServiceClient::new(self.daemon.channel().await)
    }

    /// A draft on the first line of the Desk file `path`, as the user types it.
    async fn comment(&self, id: &pb::SessionId, path: &str, text: &str) -> pb::Comment {
        self.comments()
            .await
            .create_comment(pb::CreateCommentRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                anchor: Some(pb::CommentAnchor {
                    file: Some(pb::RepoPath {
                        repo: None,
                        path: path.to_owned(),
                    }),
                    lines: Some(pb::LineRange { first: 1, last: 1 }),
                    side: pb::DiffSide::New as i32,
                    ..pb::CommentAnchor::default()
                }),
                text: text.to_owned(),
                destination: Some(pb::CommentDestination {
                    destination: Some(pb::comment_destination::Destination::Terminal(pb::SessionTerminal {})),
                }),
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// The version once Humata has recorded the comments it had, which it
    /// does a moment after the commit.
    async fn recorded(&self, id: &pb::SessionId, path: &str, version: &str) -> pb::DocVersionText {
        tokio::time::timeout(WAIT, async {
            loop {
                let read = self.read(id, path, version).await;
                if read.comments_recorded {
                    return read;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the version's comments were never recorded")
    }

    async fn newest(&self, id: &pb::SessionId, path: &str) -> pb::DocVersion {
        self.list(id, path).await.versions.remove(0)
    }
}

/// What the agent does to save a version: a plain `git commit` in the folder.
fn agent_commit(folder: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(folder.join(file), text).unwrap();
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message]);
}

fn texts(read: &pb::DocVersionText) -> Vec<&str> {
    read.comments.iter().map(|comment| comment.text.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_commit_records_the_files_comments_and_later_changes_leave_them() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Index rollout").await;
    std::fs::write(path.join("rfc.md"), "# Draft\n\nRisks\n").unwrap();
    let comment = fixture.comment(&id, "rfc.md", "tighten this").await;

    agent_commit(&path, "rfc.md", "# Draft\n\nRisks\n", "First draft");
    let first = fixture.newest(&id, "rfc.md").await;
    let read = fixture.recorded(&id, "rfc.md", &first.id).await;
    assert_eq!(texts(&read), ["tighten this"]);
    assert!(fixture.newest(&id, "rfc.md").await.has_comments);

    // The comment changes, then the next version records it as it is.
    fixture
        .comments()
        .await
        .edit_comment(pb::EditCommentRequest {
            idempotency_key: None,
            id: comment.id.clone(),
            text: "tighten the risks".to_owned(),
            expected_revision: None,
        })
        .await
        .unwrap();
    agent_commit(&path, "rfc.md", "# Draft\n\nRisks, tightened\n", "Second draft");
    let second = fixture.newest(&id, "rfc.md").await;
    assert_ne!(second.id, first.id);
    assert_eq!(texts(&fixture.recorded(&id, "rfc.md", &second.id).await), ["tighten the risks"]);
    assert_eq!(
        texts(&fixture.read(&id, "rfc.md", &first.id).await),
        ["tighten this"],
        "the first version's record is as it was"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_version_made_before_a_comment_existed_records_none() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    agent_commit(&path, "plan.md", "one\n", "First");
    let first = fixture.newest(&id, "plan.md").await;
    let read = fixture.recorded(&id, "plan.md", &first.id).await;
    assert!(read.comments.is_empty(), "recorded, and nothing was said yet");

    fixture.comment(&id, "plan.md", "why one?").await;
    agent_commit(&path, "plan.md", "two\n", "Second");
    let second = fixture.newest(&id, "plan.md").await;
    assert_eq!(texts(&fixture.recorded(&id, "plan.md", &second.id).await), ["why one?"]);
    let again = fixture.read(&id, "plan.md", &first.id).await;
    assert!(again.comments_recorded && again.comments.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_version_the_user_saves_records_the_comments_too() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    std::fs::write(path.join("plan.md"), "one\n").unwrap();
    fixture.comment(&id, "plan.md", "why one?").await;
    let saved = fixture.save(&id, "Before review").await.unwrap();
    let read = fixture.read(&id, "plan.md", &saved.id).await;
    assert!(read.comments_recorded, "recorded by the save itself, with no wait");
    assert_eq!(texts(&read), ["why one?"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_commit_tells_the_session_without_anyone_listing() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    let mut events = pb::event_service_client::EventServiceClient::new(fixture.daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    // Let what starting the session said settle, so what comes next is the commit's.
    while let Ok(event) = tokio::time::timeout(Duration::from_millis(300), events.message()).await {
        event.unwrap().expect("the feed ended");
    }

    agent_commit(&path, "plan.md", "one\n", "First");
    let told = tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if event.change == pb::Change::Updated as i32
                && matches!(&event.payload, Some(pb::event::Payload::Session(session)) if session.id.as_ref() == Some(&id))
            {
                return event;
            }
        }
    })
    .await;
    assert!(told.is_ok(), "the session was never told its folder gained a version");
    let newest = fixture.newest(&id, "plan.md").await;
    assert_eq!(fixture.desk_session(&id).await.doc_head, newest.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_version_made_while_the_daemon_was_stopped_is_recorded_when_it_starts() {
    let mut fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    fixture.comment(&id, "plan.md", "why one?").await;

    fixture.daemon.kill();
    agent_commit(&path, "plan.md", "one\n", "While away");
    fixture.daemon.restart().await;

    let newest = fixture.newest(&id, "plan.md").await;
    let read = fixture.recorded(&id, "plan.md", &newest.id).await;
    assert!(read.comments_recorded);
    assert_eq!(texts(&read), ["why one?"], "the comments as they are now");
}
