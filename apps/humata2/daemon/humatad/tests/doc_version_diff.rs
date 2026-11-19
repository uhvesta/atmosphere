//! A Desk file's version against the file as it is now (DV-7), computed by
//! the daemon: `DiffDocVersion` answers the Changes panel's hunks and line
//! model for the Desk folder's own repository. The agent's versions here are
//! plain `git commit`s.

use humatad_test_common as common;

use std::path::{Path, PathBuf};

use common::{Build, Daemon, World, folder, git, invalid_field, not_found, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;
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

    async fn newest(&self, id: &pb::SessionId, path: &str) -> String {
        self.desk()
            .await
            .list_doc_versions(pb::ListDocVersionsRequest {
                session: Some(id.clone()),
                path: path.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
            .versions[0]
            .id
            .clone()
    }

    async fn diff(
        &self,
        id: &pb::SessionId,
        path: &str,
        version: &str,
    ) -> Result<pb::DocVersionDiff, tonic::Status> {
        self.desk()
            .await
            .diff_doc_version(pb::DiffDocVersionRequest {
                session: Some(id.clone()),
                path: path.to_owned(),
                version: version.to_owned(),
            })
            .await
            .map(tonic::Response::into_inner)
    }
}

/// What the agent does to save a version: a plain `git commit` in the folder.
fn agent_commit(folder: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(folder.join(file), text).unwrap();
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message]);
}

fn kinds(diff: &pb::DocVersionDiff) -> Vec<(pb::diff_line::Kind, String)> {
    diff.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| (line.kind(), line.text.clone()))
        .collect()
}

#[tokio::test]
async fn a_saved_version_then_an_edit_is_one_removed_and_one_added_line() {
    let fixture = Fixture::new().await;
    let (id, folder) = fixture.start("Plan").await;
    agent_commit(&folder, "plan.md", "one\ntwo\nthree\n", "First");
    let version = fixture.newest(&id, "plan.md").await;
    std::fs::write(folder.join("plan.md"), "one\nthree\nfour\n").unwrap();

    let diff = fixture.diff(&id, "plan.md", &version).await.unwrap();
    assert_eq!((diff.added, diff.removed), (1, 1));
    let changed: Vec<_> = kinds(&diff)
        .into_iter()
        .filter(|(kind, _)| *kind != pb::diff_line::Kind::Context)
        .collect();
    assert_eq!(
        changed,
        vec![
            (pb::diff_line::Kind::Removed, "two".to_owned()),
            (pb::diff_line::Kind::Added, "four".to_owned()),
        ]
    );
    let removed = diff
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .find(|line| line.kind() == pb::diff_line::Kind::Removed)
        .unwrap();
    assert_eq!((removed.old_line, removed.new_line), (2, 0));
}

#[tokio::test]
async fn a_file_that_still_reads_as_the_version_has_no_hunks() {
    let fixture = Fixture::new().await;
    let (id, folder) = fixture.start("Plan").await;
    agent_commit(&folder, "plan.md", "one\n", "First");
    let version = fixture.newest(&id, "plan.md").await;

    let diff = fixture.diff(&id, "plan.md", &version).await.unwrap();
    assert!(diff.hunks.is_empty());
    assert_eq!((diff.added, diff.removed), (0, 0));
}

#[tokio::test]
async fn an_unknown_version_or_a_file_the_version_lacks_is_not_found() {
    let fixture = Fixture::new().await;
    let (id, folder) = fixture.start("Plan").await;
    agent_commit(&folder, "plan.md", "one\n", "First");
    let version = fixture.newest(&id, "plan.md").await;

    let unknown = fixture
        .diff(&id, "plan.md", "0123456789abcdef0123456789abcdef01234567")
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
    let lacks = fixture.diff(&id, "other.md", &version).await.unwrap_err();
    assert_eq!(lacks.code(), Code::NotFound);
}

#[tokio::test]
async fn a_path_outside_the_folder_is_an_invalid_path() {
    let fixture = Fixture::new().await;
    let (id, folder) = fixture.start("Plan").await;
    agent_commit(&folder, "plan.md", "one\n", "First");
    let version = fixture.newest(&id, "plan.md").await;

    let outside = fixture.diff(&id, "../plan.md", &version).await.unwrap_err();
    assert_eq!(outside.code(), Code::InvalidArgument);
    assert_eq!(invalid_field(&outside).0, "path");
}

#[tokio::test]
async fn a_session_that_is_not_a_desk_session_is_refused() {
    let fixture = Fixture::new().await;
    let refused = fixture
        .diff(
            &pb::SessionId {
                value: "no-such-session".to_owned(),
            },
            "plan.md",
            "0123456789abcdef0123456789abcdef01234567",
        )
        .await
        .unwrap_err();
    assert_eq!(not_found(&refused), pb::EntityKind::Session);
}
