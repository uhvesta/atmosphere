//! "Rename the branch too" on a session rename (FC-17, D196): the branch
//! is renamed from the new title while it was never pushed, and the
//! session's folder keeps its name.

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{BranchRename, DESK_CHANNEL};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use super::{Loaded, SessionSvc, blocking, start_error};
use crate::status;

/// The tick can't be honoured: nothing is changed, the title included.
fn refused(message: String, fix: &str, problem: Problem) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            message,
            fix,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "rename_branch".to_owned(),
                problem: problem as i32,
            }),
        ),
    )
}

impl SessionSvc {
    /// The branch rename `ticked` asks for, checked: `None` when the tick
    /// is off. Nothing is changed here.
    pub(super) async fn plan_branch_rename(
        &self,
        loaded: &Loaded,
        title: &str,
        ticked: bool,
    ) -> Result<Option<BranchRename>, Status> {
        if !ticked {
            return Ok(None);
        }
        let id = loaded.record.id.clone();
        if loaded.record.channel == DESK_CHANNEL {
            return Err(refused(
                "a Desk session has no branch to rename".to_owned(),
                "rename the session without \"Rename the branch too\"",
                Problem::NotAllowedHere,
            ));
        }
        if self.lives_elsewhere(&loaded.record).is_some() {
            // The machine plans and renames it itself; see `rename_branch_on_machine`.
            return Ok(None);
        }
        let Some(detail) = loaded.detail.clone() else {
            return Err(refused(
                "this session has no branch to rename".to_owned(),
                "rename the session without \"Rename the branch too\"",
                Problem::NotAllowedHere,
            ));
        };
        let (store, starter, title) = (self.starter.store.clone(), self.starter.clone(), title.to_owned());
        let planned = blocking(move || {
            let review = store.review(&id)?.is_some();
            let shared = store.session_details()?.into_iter().any(|other| {
                other.session != id
                    && other.repos.iter().any(|r| detail.repos.iter().any(|mine| mine.path == r.path))
            });
            Ok::<_, humata2_store::StoreError>((review, shared))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "check the session's branch"))?;
        let (review, shared) = planned;
        if review || shared {
            return Err(refused(
                if review {
                    "a review session has no branch of its own to rename".to_owned()
                } else {
                    "this session works in another session's clones, so it has no branch of its own to rename".to_owned()
                },
                "rename the session without \"Rename the branch too\"",
                Problem::NotAllowedHere,
            ));
        }
        let id = loaded.record.id.clone();
        let plan = blocking(move || {
            let plan = starter.plan_branch_rename(&id, &title)?;
            let pushed = if plan.changes() { plan.pushed() } else { Vec::new() };
            Ok::<_, humata2_sessions::StartError>((plan, pushed))
        })
        .await?
        .map_err(|error| {
            let (code, failure) = start_error(&error);
            status::failure(code, failure)
        })?;
        let (plan, pushed) = plan;
        if !pushed.is_empty() {
            return Err(refused(
                format!(
                    "the branch was already pushed in {}, so it keeps its name",
                    pushed.join(", ")
                ),
                "rename the session without \"Rename the branch too\"; a pushed branch is renamed on the remote by hand",
                Problem::NotAllowedHere,
            ));
        }
        Ok(Some(plan))
    }

    /// "Rename the branch too" for a session on another machine: the machine
    /// renames its clone's branch by the same rules (refused once pushed) and
    /// answers with the names it now has, which `loaded` takes on. True when
    /// a branch changed. Nothing is changed here when the machine refuses or
    /// is offline.
    pub(super) async fn rename_branch_on_machine(
        &self,
        loaded: &mut Loaded,
        title: &str,
        ticked: bool,
    ) -> Result<bool, Status> {
        if !ticked {
            return Ok(false);
        }
        let Some((machine, remote)) = self.lives_elsewhere(&loaded.record) else {
            return Ok(false);
        };
        let renamed = remote
            .rename_session_on(&machine, &loaded.record.id, title, true)
            .await
            .map_err(Status::from)?;
        let mut changed = false;
        if let Some(detail) = loaded.detail.as_mut() {
            for repo in &mut detail.repos {
                let now = renamed.repos.iter().find(|shown| {
                    shown
                        .repo
                        .as_ref()
                        .is_some_and(|named| named.owner == repo.owner && named.name == repo.name)
                });
                if let Some(now) = now.filter(|now| now.branch != repo.branch) {
                    repo.branch.clone_from(&now.branch);
                    changed = true;
                }
            }
        }
        Ok(changed)
    }

    /// Renames the branches and shows the new names in `loaded`. The caller
    /// writes the record, and calls [`BranchRename::undo`] if that fails.
    pub(super) async fn apply_branch_rename(
        &self,
        loaded: &mut Loaded,
        plan: Option<&BranchRename>,
    ) -> Result<(), Status> {
        let Some(plan) = plan.filter(|plan| plan.changes()).cloned() else {
            return Ok(());
        };
        let name = plan.new_name().to_owned();
        blocking(move || plan.apply())
            .await?
            .map_err(|error| {
                let (code, failure) = start_error(&error);
                status::failure(code, failure)
            })?;
        if let Some(detail) = loaded.detail.as_mut() {
            for repo in &mut detail.repos {
                repo.branch.clone_from(&name);
            }
        }
        Ok(())
    }
}
