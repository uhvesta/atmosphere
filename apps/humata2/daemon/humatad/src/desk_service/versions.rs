//! ListDocVersions, ReadDocVersion, SaveDocVersion and RestoreDocVersion
//! (DV-2, DV-5, DV-6, DV-8): the versions of a file in a Desk session's
//! folder, which is a git repository of its own (DV-1). All four are the
//! user's; the agent saves with `git commit` (DV-9). A channel's Desk
//! session works the same, its folder being a repository too.

use std::path::PathBuf;

use humata2_grpc::humata::v1 as pb;
use humata2_access::{Call, Entry, Place, RepoPath, Root};
use humata2_sessions::doc_repo::{self, DocGit, DocRepoError, SavedBy, Version};
use humata2_store::{SessionRecord, StoreError};
use tonic::{Code, Status};

use super::{DeskSvc, failed_precondition, not_desk, now_ms, session_id, user_only};
use crate::channel_service::{blocking, error_of, timestamp_ms};
use crate::host_relay::assigned;
use crate::guard::Checked;
use crate::identity::{self, Caller};
use crate::remote_work::call_failed;
use crate::status;

/// The note a save made with no message of its own carries, as the
/// versions repository words it.
const DEFAULT_NOTE: &str = "Saved by you";

impl DeskSvc {
    /// The versions of one file, newest first, and whether the folder has
    /// changes no version holds yet.
    pub(super) async fn list_versions(
        &self,
        caller: &Caller,
        request: pb::ListDocVersionsRequest,
    ) -> Result<pb::ListDocVersionsResponse, Status> {
        let id = session_id(request.session.clone())?;
        if let Some(machine) = self.versions_machine(caller, &id).await? {
            return self.list_versions_there(&machine, request).await;
        }
        let (session, folder) = self.readable_folder(caller, &id, &request.path).await?;
        let (path, label) = (request.path, self.starter.agents.person_name(&session.agent));
        let recorded_path = path.clone();
        let (versions, unsaved_changes) = blocking(move || -> Result<_, DocRepoError> {
            let git = DocGit::inherit();
            Ok((
                doc_repo::versions(&git, &folder, &path)?,
                doc_repo::has_changes(&git, &folder)?,
            ))
        })
        .await?
        .map_err(|error| repo_status(&error))?;
        let recorded = self.recorded_commits(&id, recorded_path).await?;
        Ok(pb::ListDocVersionsResponse {
            versions: versions
                .iter()
                .map(|version| {
                    let mut shown = version_pb(version, &label);
                    shown.has_comments = recorded.contains(&version.id);
                    shown
                })
                .collect(),
            unsaved_changes,
        })
    }

    /// One version's text and the comments recorded with it (DV-6, DV-12);
    /// a version with no record answers `comments_recorded: false`.
    pub(super) async fn read_version(
        &self,
        caller: &Caller,
        request: pb::ReadDocVersionRequest,
    ) -> Result<pb::DocVersionText, Status> {
        let recorded_path = request.path.clone();
        let id = session_id(request.session.clone())?;
        if let Some(machine) = self.versions_machine(caller, &id).await? {
            return self.read_version_there(&machine, request).await;
        }
        let (session, folder) = self.readable_folder(caller, &id, &request.path).await?;
        let label = self.starter.agents.person_name(&session.agent);
        let (path, wanted) = (request.path, request.version);
        let (text, version) = blocking(move || -> Result<_, DocRepoError> {
            let git = DocGit::inherit();
            let text = doc_repo::read(&git, &folder, &path, &wanted)?;
            let version = doc_repo::versions(&git, &folder, &path)?
                .into_iter()
                .find(|version| version.id == wanted)
                .ok_or(DocRepoError::NoSuchVersion { path, id: wanted })?;
            Ok((text, version))
        })
        .await?
        .map_err(|error| repo_status(&error))?;
        let recorded = self
            .recorded_comments(&id, version.id.clone(), recorded_path)
            .await?;
        Ok(pb::DocVersionText {
            version: Some(version_pb(&version, &label)),
            text,
            comments_recorded: recorded.is_some(),
            comments: recorded.unwrap_or_default(),
        })
    }

    /// One version of a file against the file as it is on disk now (DV-7):
    /// what the Changes panel shows for Compare with current. Read-only, so
    /// it is judged as a read of the version (`readable_folder`).
    pub(super) async fn diff_version(
        &self,
        caller: &Caller,
        request: pb::DiffDocVersionRequest,
    ) -> Result<pb::DocVersionDiff, Status> {
        let id = session_id(request.session.clone())?;
        if let Some(machine) = self.versions_machine(caller, &id).await? {
            return self.diff_version_there(&machine, request).await;
        }
        let (_, folder) = self.readable_folder(caller, &id, &request.path).await?;
        let (path, wanted) = (request.path, request.version);
        let shown = blocking(move || doc_repo::diff(&DocGit::inherit(), &folder, &path, &wanted))
            .await?
            .map_err(|error| repo_status(&error))?;
        Ok(diff_pb(&shown))
    }

    /// The user's Save version: every change in the folder becomes one
    /// version by the user.
    pub(super) async fn save_version(
        &self,
        caller: &Caller,
        request: pb::SaveDocVersionRequest,
    ) -> Result<pb::DocVersion, Status> {
        let id = session_id(request.session.clone())?;
        let _held = self.locks.hold(&id).await;
        if let Some(machine) = self.versions_machine(caller, &id).await? {
            return self.save_version_there(caller, &machine, &id, request).await;
        }
        let (_, folder) = self.versions_folder(&id).await?;
        let watched = folder.clone();
        let message = Some(request.message.trim().to_owned()).filter(|text| !text.is_empty());
        let note = message.clone().unwrap_or_else(|| DEFAULT_NOTE.to_owned());
        let saved = blocking(move || doc_repo::save(&DocGit::inherit(), &folder, message.as_deref()))
            .await?
            .map_err(|error| repo_status(&error))?;
        self.record_commits(&id, watched, vec![saved.clone()]).await;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(own_version_pb(saved, note))
    }

    /// The user's Restore as new version: unsaved changes are saved first,
    /// every old version stays.
    pub(super) async fn restore_version(
        &self,
        caller: &Caller,
        request: pb::RestoreDocVersionRequest,
    ) -> Result<pb::RestoreDocVersionResponse, Status> {
        let id = session_id(request.session.clone())?;
        let _held = self.locks.hold(&id).await;
        if let Some(machine) = self.versions_machine(caller, &id).await? {
            return self.restore_version_there(caller, &machine, &id, request).await;
        }
        let (session, folder) = self.versions_folder(&id).await?;
        let watched = folder.clone();
        let label = self.starter.agents.person_name(&session.agent);
        let (path, wanted) = (request.path, request.version);
        let (restored, number, listed) = blocking(move || -> Result<_, DocRepoError> {
            let git = DocGit::inherit();
            let number = doc_repo::versions(&git, &folder, &path)?
                .iter()
                .find(|version| version.id == wanted)
                .map(|version| version.number);
            let restored = doc_repo::restore(&git, &folder, &path, &wanted)?;
            let listed = doc_repo::versions(&git, &folder, &path)?;
            Ok((restored, number, listed))
        })
        .await?
        .map_err(|error| repo_status(&error))?;
        let made: Vec<String> = restored
            .saved_first
            .iter()
            .chain(std::iter::once(&restored.restored))
            .cloned()
            .collect();
        self.record_commits(&id, watched, made).await;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        let saved_first = restored.saved_first.map(|saved| {
            let note = number.map_or_else(String::new, |n| format!("Saved before restoring v{n}"));
            own_version_pb(saved, note)
        });
        let new = restored.restored;
        let restored = match listed.iter().find(|version| version.id == new) {
            Some(version) => version_pb(version, &label),
            None => own_version_pb(new, number.map_or_else(String::new, |n| format!("Restored v{n}"))),
        };
        Ok(pb::RestoreDocVersionResponse {
            saved_first,
            restored: Some(restored),
        })
    }

    /// The machine a Desk session's versions are on, when it isn't this one
    /// and the caller is the user: that machine answers the user's own call
    /// (D139). An agent or script is never relayed as the user; for them
    /// the session's folder is refused as it always was.
    async fn versions_machine(&self, caller: &Caller, id: &str) -> Result<Option<String>, Status> {
        if !matches!(caller, Caller::User(_)) {
            return Ok(None);
        }
        let session = self.versions_session(id).await?;
        Ok(self.elsewhere(&session))
    }

    async fn list_versions_there(
        &self,
        machine: &str,
        request: pb::ListDocVersionsRequest,
    ) -> Result<pb::ListDocVersionsResponse, Status> {
        let mut desk = self.desk_on(machine)?;
        desk.list_doc_versions(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "list the file's versions", source)))
    }

    async fn read_version_there(
        &self,
        machine: &str,
        request: pb::ReadDocVersionRequest,
    ) -> Result<pb::DocVersionText, Status> {
        let mut desk = self.desk_on(machine)?;
        desk.read_doc_version(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "read the file's version", source)))
    }

    async fn diff_version_there(
        &self,
        machine: &str,
        request: pb::DiffDocVersionRequest,
    ) -> Result<pb::DocVersionDiff, Status> {
        let mut desk = self.desk_on(machine)?;
        desk.diff_doc_version(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "compare the file's version", source)))
    }

    async fn save_version_there(
        &self,
        caller: &Caller,
        machine: &str,
        id: &str,
        request: pb::SaveDocVersionRequest,
    ) -> Result<pb::DocVersion, Status> {
        let mut desk = self.desk_on(machine)?;
        let saved = desk
            .save_doc_version(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "save a version", source)))?;
        self.tell
            .session(id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(saved)
    }

    async fn restore_version_there(
        &self,
        caller: &Caller,
        machine: &str,
        id: &str,
        request: pb::RestoreDocVersionRequest,
    ) -> Result<pb::RestoreDocVersionResponse, Status> {
        let mut desk = self.desk_on(machine)?;
        let restored = desk
            .restore_doc_version(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "restore the version", source)))?;
        self.tell
            .session(id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(restored)
    }

    /// The session's record and folder, for a Desk session or a channel's
    /// Desk session on this machine. One on another machine keeps its
    /// versions there, and the user's calls go to it (`versions_machine`).
    async fn versions_folder(&self, id: &str) -> Result<(SessionRecord, PathBuf), Status> {
        let session = self.versions_session(id).await?;
        self.here_folder(session)
    }

    /// What a read of versions may see (DV-9): the user, anything; an
    /// agent, its own session's folder freely and another's as the policy
    /// judges a read of that session's files (FILES_READ_OTHER_SESSION,
    /// FILES_READ_OTHER_MACHINE, as `file_read`). The rule is checked
    /// before the session's machine, so a refusal names the rule.
    async fn readable_folder(
        &self,
        caller: &Caller,
        id: &str,
        path: &str,
    ) -> Result<(SessionRecord, PathBuf), Status> {
        let session = self.versions_session(id).await?;
        match caller {
            Caller::User(_) => {}
            Caller::Agent(_) => {
                let place = Place {
                    machine: identity::session_ref(&session, &self.machine).machine,
                    root: Root::Clone(identity::session_ref(&session, &self.machine)),
                    path: RepoPath::new(path).map_err(|_| {
                        repo_status(&DocRepoError::BadPath {
                            path: path.to_owned(),
                        })
                    })?,
                    entry: Entry::File,
                };
                // D768: an asked read waits in the Inbox, and is repeated
                // once approved.
                match self.guard.check(caller, &Call::ReadFiles(place))? {
                    Checked::Allowed => {}
                    Checked::Ask(asked) => {
                        self.inbox.ask_read(caller, asked, "versions", None).await?;
                    }
                }
            }
            Caller::Script(_) | Caller::Peer => user_only(caller)?,
        }
        self.here_folder(session)
    }

    /// The session's record, for a Desk session or a channel's Desk
    /// session.
    async fn versions_session(&self, id: &str) -> Result<SessionRecord, Status> {
        let (store, key) = (self.starter.store.clone(), id.to_owned());
        let found = blocking(move || -> Result<_, StoreError> {
            Ok((store.session(&key)?, store.desk(&key)?, store.channel_desk(&key)?))
        })
        .await?;
        let session = match found {
            Ok((Some(session), Some(row), _)) if row.deleted_unix_ms.is_none() => session,
            Ok((Some(session), None, Some(_))) => session,
            Ok(_) | Err(StoreError::InvalidId { .. }) => return Err(not_desk(id)),
            Err(error) => {
                return Err(status::store(error, &self.machine, "look up the Desk session"));
            }
        };
        Ok(session)
    }

    /// The session's folder on this machine. One on another machine keeps
    /// its versions there.
    fn here_folder(&self, session: SessionRecord) -> Result<(SessionRecord, PathBuf), Status> {
        let id = session.id.clone();
        if let Some(machine) = self.elsewhere(&session) {
            return Err(failed_precondition(
                format!("Desk session {id} works on {machine}, and its file versions are kept there, not on this machine"),
                format!("read and save versions from {machine}'s own Humata"),
            ));
        }
        let folder = session.folder.clone();
        Ok((session, folder))
    }
}

fn version_pb(version: &Version, agent: &str) -> pb::DocVersion {
    let (by, who) = match version.by {
        SavedBy::You => (pb::DocVersionBy::You, "You".to_owned()),
        SavedBy::Agent => (pb::DocVersionBy::Agent, agent.to_owned()),
    };
    pb::DocVersion {
        id: version.id.clone(),
        number: version.number,
        at: Some(timestamp_ms(version.unix_ms)),
        by: by as i32,
        who,
        message: version.message.clone(),
        added: version.added,
        removed: version.removed,
        has_comments: false,
    }
}

/// The wire form of a version diff: the review panel's hunks and lines.
fn diff_pb(diff: &doc_repo::VersionDiff) -> pb::DocVersionDiff {
    let kind = |change: doc_repo::Change| match change {
        doc_repo::Change::Context => pb::diff_line::Kind::Context,
        doc_repo::Change::Added => pb::diff_line::Kind::Added,
        doc_repo::Change::Removed => pb::diff_line::Kind::Removed,
    };
    pb::DocVersionDiff {
        hunks: diff
            .hunks
            .iter()
            .enumerate()
            .map(|(index, hunk)| pb::Hunk {
                header: hunk.header.clone(),
                lines: hunk
                    .lines
                    .iter()
                    .map(|line| pb::DiffLine {
                        kind: kind(line.change) as i32,
                        old_line: line.old_line,
                        new_line: line.new_line,
                        text: line.text.clone(),
                    })
                    .collect(),
                index: u32::try_from(index).unwrap_or(u32::MAX),
                old_start: hunk.old_start,
                new_start: hunk.new_start,
                section: hunk.section.clone(),
                continued: false,
            })
            .collect(),
        added: diff.added,
        removed: diff.removed,
    }
}

/// A version the user just made, whose per-file number and line counts the
/// call doesn't know (number 0).
fn own_version_pb(id: String, message: String) -> pb::DocVersion {
    pb::DocVersion {
        id,
        number: 0,
        at: Some(timestamp_ms(now_ms())),
        by: pb::DocVersionBy::You as i32,
        who: "You".to_owned(),
        message,
        added: 0,
        removed: 0,
        has_comments: false,
    }
}

/// The status for a failed versions call, with the error the app shows.
fn repo_status(error: &DocRepoError) -> Status {
    let (code, fix, kind) = match error {
        DocRepoError::BadPath { .. } => (
            Code::InvalidArgument,
            "give the file's path relative to the session's folder, without `..`",
            Some(pb::error::Kind::InvalidField(pb::InvalidField {
                field: "path".to_owned(),
                problem: pb::invalid_field::Problem::BadFormat as i32,
            })),
        ),
        DocRepoError::NoSuchVersion { .. } => (
            Code::NotFound,
            "list the file's versions again and pick one of them",
            None,
        ),
        DocRepoError::NoChanges { .. } => (
            Code::FailedPrecondition,
            "change a file in the session's folder, then save a version",
            None,
        ),
        DocRepoError::AlreadyCurrent { .. } => (
            Code::FailedPrecondition,
            "pick an older version, or keep the file as it is",
            None,
        ),
        DocRepoError::NotARepo { .. } => (
            Code::FailedPrecondition,
            "a folder you picked has no versions; a scratch folder made before versions existed gets them when you migrate it from the Inbox's \"Migrate Desk folders\" item",
            None,
        ),
        DocRepoError::Git { .. } | DocRepoError::Io { .. } => (
            Code::Internal,
            "check that git is installed and the session's folder is there and writable, then try again; humatad2's log has the details",
            None,
        ),
    };
    status::failure(code, error_of(error, fix, kind))
}
