//! AddDeskRepo and RemoveDeskRepo (DK-3, MV-9, D182.2): a repository cloned
//! into a Desk session's folder, and one moved out of it. Both run under the
//! session's lock, and neither touches a clone it didn't make or a folder it
//! doesn't own.

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{clone_repo, held_work, repo_clone};
use humata2_transfers::NewItem;
use tonic::Status;

use super::{DeskSvc, desk_status, failed_precondition, now_ms, session_id, view};
use crate::channel_service::blocking;
use crate::identity::{self, Caller};
use crate::status;
use crate::trash_service::caller_data;

impl DeskSvc {
    /// Clones the repository's default branch into the session's folder as
    /// `<name>`. Refused while the session is closed, and when the folder
    /// already has that name, which is then left as it is.
    pub(super) async fn add_repo(
        &self,
        caller: &Caller,
        request: pb::AddDeskRepoRequest,
    ) -> Result<pb::DeskSession, Status> {
        let id = session_id(request.session)?;
        let repo = request.repo.ok_or_else(|| status::missing("repo"))?;
        let _held = self.locks.hold(&id).await;
        let (session, row) = self.load(&id).await?;
        if row.closed_unix_ms.is_some() {
            return Err(failed_precondition(
                format!("Desk session {id} is closed, so it takes no repository"),
                "reopen the session, then add the repository",
            ));
        }
        let (template, folder) = (self.starter.remote_template.clone(), session.folder.clone());
        let (owner, name) = (repo.owner, repo.name);
        blocking(move || clone_repo(&template, &owner, &name, &folder))
            .await?
            .map_err(|error| desk_status(&error))?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(view::desk_pb(&session, &row))
    }

    /// Moves a repository's clone to this machine's Bin (DK-3, MV-9). Refused,
    /// naming the work, while the clone holds uncommitted or unpushed work.
    pub(super) async fn remove_repo(
        &self,
        caller: &Caller,
        request: pb::RemoveDeskRepoRequest,
    ) -> Result<pb::DeskSession, Status> {
        let id = session_id(request.session)?;
        let repo = request.repo.ok_or_else(|| status::missing("repo"))?;
        let _held = self.locks.hold(&id).await;
        let (session, row) = self.load(&id).await?;
        let (folder, name) = (session.folder.clone(), repo.name.clone());
        let clone = blocking(move || repo_clone(&folder, &name))
            .await?
            .map_err(|error| desk_status(&error))?;
        let checked = clone.clone();
        let work = blocking(move || held_work(&checked)).await?;
        if !work.is_empty() {
            return Err(failed_precondition(
                format!(
                    "repository {}/{} has work that would be lost ({work}), so it was kept",
                    repo.owner, repo.name
                ),
                "commit and push it, or copy what you need, then remove the repo again",
            ));
        }
        let was = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let item = NewItem {
            description: format!(
                "Repository {}/{} from Desk session {}",
                repo.owner, repo.name, session.title
            ),
            caller_data: caller_data(&identity::actor(caller), &was),
            now_unix_ms: now_ms(),
        };
        let bin = self.bin.clone();
        blocking(move || -> Result<(), Status> {
            let trash = bin.get()?;
            trash.put(&clone, item).map(drop).map_err(|error| {
                status::internal(
                    format!(
                        "couldn't move the repository to the Bin: {}; check that the Bin's folder can be written to, then remove the repo again",
                        status::chain(&error)
                    ),
                    "",
                )
            })
        })
        .await??;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(view::desk_pb(&session, &row))
    }
}
