//! A Desk file's versions through DeskService (DV-2, DV-5, DV-6, DV-8,
//! D450, D455): listed newest first with who made each, saved by the user,
//! read back as they were, and restored as a new version that keeps every
//! other one. The folder's own repository is a plain git repository, so the
//! agent's versions here are plain `git commit`s.

use humatad_test_common as common;

use std::path::{Path, PathBuf};

use common::{Build, Daemon, World, folder, git, wait_status};
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

    async fn restore(
        &self,
        id: &pb::SessionId,
        path: &str,
        version: &str,
    ) -> Result<pb::RestoreDocVersionResponse, tonic::Status> {
        self.desk()
            .await
            .restore_doc_version(pb::RestoreDocVersionRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                path: path.to_owned(),
                version: version.to_owned(),
            })
            .await
            .map(tonic::Response::into_inner)
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
}

/// What the agent does to save a version: a plain `git commit` in the folder.
fn agent_commit(folder: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(folder.join(file), text).unwrap();
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message]);
}

#[tokio::test]
async fn a_file_lists_its_versions_newest_first_with_who_made_each() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Index rollout").await;
    assert!(fixture.list(&id, "rfc.md").await.versions.is_empty(), "no version yet");
    agent_commit(&path, "rfc.md", "# Draft\n", "First draft");
    std::fs::write(path.join("rfc.md"), "# Draft\n\nRisks\n").unwrap();
    std::fs::write(path.join("notes.md"), "elsewhere\n").unwrap();
    fixture.save(&id, "Before review").await.unwrap();

    let listed = fixture.list(&id, "rfc.md").await;
    assert!(!listed.unsaved_changes);
    let numbers: Vec<u32> = listed.versions.iter().map(|v| v.number).collect();
    assert_eq!(numbers, [2, 1], "newest first, numbered from the oldest");
    assert_eq!(listed.versions[0].by(), pb::DocVersionBy::You);
    assert_eq!(listed.versions[0].message, "Before review");
    assert_eq!((listed.versions[0].added, listed.versions[0].removed), (2, 0));
    assert_eq!(listed.versions[1].by(), pb::DocVersionBy::Agent);
    assert!(!listed.versions[1].who.is_empty());
    assert_ne!(listed.versions[0].who, listed.versions[1].who);
    assert_eq!(fixture.list(&id, "notes.md").await.versions.len(), 1, "versions are per file");
}

#[tokio::test]
async fn saving_with_nothing_changed_is_refused_and_a_change_shows_as_unsaved() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    agent_commit(&path, "plan.md", "one\n", "First");
    let refused = fixture.save(&id, "").await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    std::fs::write(path.join("plan.md"), "two\n").unwrap();
    assert!(fixture.list(&id, "plan.md").await.unsaved_changes);
    let saved = fixture.save(&id, "").await.unwrap();
    assert!(!saved.message.is_empty(), "an empty message saves with a default one");
    assert_eq!(saved.by(), pb::DocVersionBy::You);
}

#[tokio::test]
async fn an_old_version_reads_as_it_was() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    agent_commit(&path, "plan.md", "one\n", "First");
    agent_commit(&path, "plan.md", "two\n", "Second");
    let listed = fixture.list(&id, "plan.md").await;
    let oldest = &listed.versions[1];
    let read = fixture.read(&id, "plan.md", &oldest.id).await;
    assert_eq!(read.text, "one\n");
    assert_eq!(read.version.unwrap().number, 1);
    assert_eq!(std::fs::read_to_string(path.join("plan.md")).unwrap(), "two\n", "reading changes nothing");
}

#[tokio::test]
async fn restoring_saves_unsaved_changes_first_and_keeps_every_version() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    agent_commit(&path, "plan.md", "one\n", "First");
    agent_commit(&path, "plan.md", "two\n", "Second");
    std::fs::write(path.join("plan.md"), "typed\n").unwrap();
    std::fs::write(path.join("other.md"), "untouched\n").unwrap();
    let v1 = fixture.list(&id, "plan.md").await.versions[1].id.clone();

    let restored = fixture.restore(&id, "plan.md", &v1).await.unwrap();
    assert!(restored.saved_first.is_some(), "the typed text was saved before restoring");
    assert_eq!(std::fs::read_to_string(path.join("plan.md")).unwrap(), "one\n");
    assert_eq!(std::fs::read_to_string(path.join("other.md")).unwrap(), "untouched\n");
    let listed = fixture.list(&id, "plan.md").await;
    assert_eq!(listed.versions.len(), 4, "two old, the saved typing, the restore");
    assert!(!listed.unsaved_changes);
    let typed = &listed.versions[1];
    assert_eq!(fixture.read(&id, "plan.md", &typed.id).await.text, "typed\n");
}

#[tokio::test]
async fn restoring_the_text_the_file_already_has_is_refused() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    agent_commit(&path, "plan.md", "one\n", "First");
    let newest = fixture.list(&id, "plan.md").await.versions[0].id.clone();
    let refused = fixture.restore(&id, "plan.md", &newest).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn a_path_outside_the_folder_or_an_unknown_version_is_refused() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    agent_commit(&path, "plan.md", "one\n", "First");
    let mut desk = fixture.desk().await;
    let outside = desk
        .list_doc_versions(pb::ListDocVersionsRequest {
            session: Some(id.clone()),
            path: "../elsewhere.md".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(outside.code(), Code::InvalidArgument);
    let unknown = desk
        .read_doc_version(pb::ReadDocVersionRequest {
            session: Some(id.clone()),
            path: "plan.md".to_owned(),
            version: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
}

#[tokio::test]
async fn the_desk_session_names_its_newest_version_once_there_is_one() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    let before = fixture.desk_session(&id).await;
    assert!(before.has_versions);
    assert!(before.doc_head.is_empty());
    agent_commit(&path, "plan.md", "one\n", "First");
    let newest = fixture.list(&id, "plan.md").await.versions[0].id.clone();
    assert_eq!(fixture.desk_session(&id).await.doc_head, newest);
}

#[tokio::test]
async fn the_desk_session_counts_its_versions_as_they_are_saved() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    assert_eq!(fixture.desk_session(&id).await.version_count, 0);
    agent_commit(&path, "plan.md", "one\n", "First");
    assert_eq!(fixture.desk_session(&id).await.version_count, 1);
    std::fs::write(path.join("plan.md"), "two\n").unwrap();
    fixture.save(&id, "").await.unwrap();
    assert_eq!(fixture.desk_session(&id).await.version_count, 2);
}

#[tokio::test]
async fn the_desk_session_holds_files_once_a_file_is_outside_its_clones() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan").await;
    assert!(!fixture.desk_session(&id).await.holds_files);
    let clone = path.join("api");
    std::fs::create_dir_all(&clone).unwrap();
    git(&["init", "-q", &clone.display().to_string()]);
    std::fs::write(clone.join("main.rs"), "fn main() {}\n").unwrap();
    assert!(!fixture.desk_session(&id).await.holds_files);
    std::fs::write(path.join("notes.txt"), "hello\n").unwrap();
    assert!(fixture.desk_session(&id).await.holds_files);
}
