//! Desk folders made before DOCV get a versions repository through a VIP
//! "Migrate the Desk folders" Inbox item (D458, DV-1, DV-11), one for this
//! machine and one for each machine, which the host asks over its link
//! (CheckDeskFolders, MigrateDeskFolders). An item is posted at start
//! when a Humata-made folder (a plain Desk scratch folder or a channel's Desk
//! folder, open, closed or kept) isn't a repository yet, and is finished when
//! the user clicks Migrate: `git init` in each, with its clones left out and
//! no commit. A picked folder is never touched, and nothing is deleted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::doc_repo::{self, DocGit, DocRepoError};
use humata2_store::{DeskFolderKind, SessionRecord, StoreError};
use tonic::{Code, Status};

use super::DeskSvc;
use crate::channel_service::blocking;
use crate::host_relay::{LinkState, assigned};
use crate::remote_work::call_failed;
use crate::status;
use crate::upgrade::items::FolderMigrator;

/// Why one Desk folder couldn't be made a versions repository.
#[derive(Debug, thiserror::Error)]
enum FolderError {
    #[error(
        "couldn't set up versions in {folder}: {source}; check that git is installed and the folder can be written to, then click Migrate again"
    )]
    Init {
        folder: PathBuf,
        #[source]
        source: DocRepoError,
    },

    #[error(
        "couldn't look through {folder} for repositories to leave out of its versions: {source}; check the folder can be read, then click Migrate again"
    )]
    Read {
        folder: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't leave {name} out of the versions in {folder}: {source}; check the folder can be written to, then click Migrate again"
    )]
    Exclude {
        folder: PathBuf,
        name: String,
        #[source]
        source: DocRepoError,
    },
}

/// A Desk folder that isn't a versions repository yet, and the agent of the
/// session it belongs to (the name its versions are saved under).
struct Folder {
    path: PathBuf,
    agent: String,
}

/// Starts the check and lets a click on the item reach this service.
pub(super) fn wire(svc: &DeskSvc) {
    svc.migrations.set_desk_folders(std::sync::Arc::new(svc.clone()));
    let svc = svc.clone();
    tokio::spawn(async move {
        svc.check_folders().await;
        svc.check_machines().await;
    });
}

impl DeskSvc {
    /// Posts the item when a Desk folder lacks its repository, and finishes
    /// it when none does. A host only: it asks each machine about its own
    /// folders (`check_machines`).
    async fn check_folders(&self) {
        if self.remote.is_none() {
            return;
        }
        let machine = pb::MachineId {
            value: self.machine.clone(),
        };
        let folders = match self.stale_folders().await {
            Ok(folders) => folders,
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: couldn't check whether the Desk folders have versions, so no Migrate item was posted: {}",
                    error.message()
                );
                return;
            }
        };
        self.show(&machine, !folders.is_empty()).await;
    }

    /// Follows the machines' links: each machine that is up now, and each
    /// that comes up later, is asked about its own Desk folders. A machine
    /// that is away keeps the item it had; its link coming up asks again.
    /// Never blocks the start.
    async fn check_machines(&self) {
        let Some(remote) = &self.remote else {
            return;
        };
        // Followed before the machines up now are read, so no link comes up unseen.
        let mut links = remote.watch_links();
        for machine in remote.machines_up() {
            self.check_later(machine);
        }
        while let Some((machine, state)) = links.next().await {
            if matches!(state, LinkState::Up { .. }) {
                self.check_later(machine);
            }
        }
    }

    /// Checks `machine` on its own task, so a machine that answers slowly
    /// holds up no other.
    fn check_later(&self, machine: String) {
        let svc = self.clone();
        tokio::spawn(async move {
            svc.check_machine(&machine).await;
        });
    }

    /// Posts `machine`'s item when it has Desk folders without a repository,
    /// and finishes it when it has none. A machine that can't answer leaves
    /// its item as it was.
    async fn check_machine(&self, machine: &str) {
        let stale = match self.count_there(machine).await {
            Ok(stale) => stale,
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: couldn't check whether the Desk folders on {machine} have versions, so its Migrate item stays as it was: {}",
                    error.message()
                );
                return;
            }
        };
        self.show(&pb::MachineId { value: machine.to_owned() }, stale > 0).await;
    }

    async fn count_there(&self, machine: &str) -> Result<u32, Status> {
        let mut desk = self.desk_on(machine)?;
        let request = assigned(pb::CheckDeskFoldersRequest::default(), None).map_err(Status::from)?;
        desk.check_desk_folders(request)
            .await
            .map(|answer| answer.into_inner().without_versions)
            .map_err(|source| Status::from(call_failed(machine, "check its Desk folders", source)))
    }

    /// `machine`'s item is posted while it has folders to migrate and
    /// withdrawn when it has none.
    async fn show(&self, machine: &pb::MachineId, stale: bool) {
        let thing = pb::MigrateThing::DeskFolders;
        let done = if stale {
            self.migrations
                .observe(thing, machine, pb::BuildInfo::default(), pb::BuildInfo::default())
                .await
                .map(|_| ())
        } else {
            self.migrations.resolved(thing, machine).await
        };
        if let Err(error) = done {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
    }

    /// The Desk folders on this machine that Humata made and that aren't a
    /// versions repository.
    async fn stale_folders(&self) -> Result<Vec<Folder>, Status> {
        let mut found: Vec<Folder> = Vec::new();
        for (session, row) in self.desk_rows().await? {
            if matches!(row.kind, DeskFolderKind::Scratch { .. }) && self.elsewhere(&session).is_none() {
                found.push(Folder {
                    path: session.folder,
                    agent: self.starter.agents.person_name(&session.agent),
                });
            }
        }
        let store = self.starter.store.clone();
        let (rows, sessions) = blocking(move || -> Result<_, StoreError> {
            Ok((store.channel_desks()?, store.sessions()?))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "list the channel Desk sessions"))?;
        let mut records: HashMap<String, SessionRecord> = sessions
            .into_iter()
            .map(|session| (session.id.clone(), session))
            .collect();
        for row in rows {
            if let Some(session) = records.remove(&row.session_id)
                && self.elsewhere(&session).is_none()
            {
                found.push(Folder {
                    path: session.folder,
                    agent: self.starter.agents.person_name(&session.agent),
                });
            }
        }
        blocking(move || {
            found
                .into_iter()
                .filter(|folder| folder.path.is_dir() && !doc_repo::is_doc_repo(&folder.path))
                .collect()
        })
        .await
    }
}

/// Makes `folder` a versions repository with no commit, leaving each
/// repository directly in it out of the folder's versions.
fn make_repo(folder: &Path, agent: &str) -> Result<(), FolderError> {
    let git = DocGit::inherit();
    doc_repo::init(&git, folder, agent).map_err(|source| FolderError::Init {
        folder: folder.to_owned(),
        source,
    })?;
    let read = |source| FolderError::Read {
        folder: folder.to_owned(),
        source,
    };
    for entry in std::fs::read_dir(folder).map_err(read)? {
        let path = entry.map_err(read)?.path();
        if !path.join(".git").exists() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        doc_repo::exclude_clone(&git, folder, name).map_err(|source| FolderError::Exclude {
            folder: folder.to_owned(),
            name: name.to_owned(),
            source,
        })?;
    }
    Ok(())
}

impl DeskSvc {
    /// Makes each folder here that lacks a repository one. Answers how many
    /// were made; a folder that fails is named in the error, and the others
    /// are done anyway.
    async fn migrate_here(&self) -> Result<u32, pb::Error> {
        let folders = self.stale_folders().await.map_err(|error| pb::Error {
            message: format!("couldn't list the Desk folders to migrate: {}", error.message()),
            fix: "restart humatad2 on the machine the folders are on and click Migrate again".to_owned(),
            retry: None,
            kind: None,
        })?;
        let mut migrated = 0u32;
        let mut failed = Vec::new();
        for folder in folders {
            let (path, agent) = (folder.path.clone(), folder.agent);
            match blocking(move || make_repo(&path, &agent)).await {
                Ok(Ok(())) => migrated += 1,
                Ok(Err(error)) => failed.push(status::chain(&error)),
                Err(error) => failed.push(format!("{}: {}", folder.path.display(), error.message())),
            }
        }
        if failed.is_empty() {
            return Ok(migrated);
        }
        Err(pb::Error {
            message: failed.join("; "),
            fix: "fix what is named above; the other folders were migrated and nothing was deleted or saved, so click Migrate again to retry the rest".to_owned(),
            retry: None,
            kind: None,
        })
    }

    /// Asks `machine` to make its own folders repositories, and answers what
    /// it said as it said it: its error says what to fix there.
    async fn migrate_there(&self, machine: &str) -> Result<(), pb::Error> {
        let mut desk = self.desk_on(machine).map_err(|status| problem_of(&status))?;
        let request = assigned(pb::MigrateDeskFoldersRequest::default(), None)
            .map_err(|error| problem_of(&Status::from(error)))?;
        desk.migrate_desk_folders(request)
            .await
            .map(|_| ())
            .map_err(|source| problem_of(&Status::from(call_failed(machine, "migrate its Desk folders", source))))
    }

    /// CheckDeskFolders: how many of this machine's Desk folders have no
    /// repository yet.
    pub(super) async fn count_stale(&self) -> Result<pb::CheckDeskFoldersResponse, Status> {
        let folders = self.stale_folders().await?;
        Ok(pb::CheckDeskFoldersResponse {
            without_versions: u32::try_from(folders.len()).unwrap_or(u32::MAX),
        })
    }

    /// MigrateDeskFolders: the same work as the Migrate item's click, here.
    /// A host's own item is settled afterwards.
    pub(super) async fn migrate_asked(&self) -> Result<pb::MigrateDeskFoldersResponse, Status> {
        let migrated = self
            .migrate_here()
            .await
            .map_err(|error| status::failure(Code::FailedPrecondition, error))?;
        self.check_folders().await;
        Ok(pb::MigrateDeskFoldersResponse { migrated })
    }
}

/// The error a status carries, or one saying what the status said.
fn problem_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: "check that the machine is reachable, then click Migrate again".to_owned(),
        retry: None,
        kind: None,
    })
}

#[tonic::async_trait]
impl FolderMigrator for DeskSvc {
    async fn migrate_folders(&self, machine: &pb::MachineId) -> Result<(), pb::Error> {
        if machine.value.is_empty() || machine.value == self.machine {
            return self.migrate_here().await.map(|_| ());
        }
        self.migrate_there(&machine.value).await
    }
}
