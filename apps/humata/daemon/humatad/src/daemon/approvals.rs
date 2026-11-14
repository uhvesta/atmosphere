//! Admin actions from clients other than the app: the person's own board
//! actions, which `humata` or `humata mcp` may ask for but which run only
//! once the person approves them in the app, or under a grant.

use std::sync::Arc;
use std::time::Duration;

use humata_wire::{Reply, Service, ServiceFuture, describe, error, pb};

use super::Daemon;
use crate::board::{RUNNING_UNDER, Via};
use crate::error::DaemonError;
use crate::peer::Peer;
use crate::store::now_unix_ms;

/// The longest a client waits on one [`pb::WaitForApproval`].
const LONGEST_WAIT: Duration = Duration::from_secs(300);

/// The longest a grant lasts.
const LONGEST_GRANT: Duration = Duration::from_secs(24 * 60 * 60);

/// The scope of an admin action; None for any other request, which runs
/// for any client.
pub(super) fn scope_of(request: &pb::request::Kind) -> Option<pb::ApprovalScope> {
    use pb::request::Kind as Request;
    match request {
        Request::ApplyProposal(_) | Request::DeclineProposal(_) => {
            Some(pb::ApprovalScope::Proposals)
        }
        Request::CreateItem(_) | Request::UpdateItem(_) | Request::ImportIssues(_) => {
            Some(pb::ApprovalScope::Board)
        }
        Request::DeleteItem(_) | Request::RevertBoardChange(_) => {
            Some(pb::ApprovalScope::Destructive)
        }
        _ => None,
    }
}

/// Requests only the app may send: deciding approvals, revoking grants and
/// what's kept on disk are the person's, so no client may ask for them.
fn app_only(request: &pb::request::Kind) -> bool {
    use pb::request::Kind as Request;
    matches!(
        request,
        Request::DecideApprovals(_)
            | Request::RevokeGrant(_)
            | Request::SetStorageSettings(_)
            | Request::CleanUpStorage(_)
            | Request::SetPullRefresh(_)
    )
}

/// Records `surface`, the client the change really came from, in a
/// request that names one, whatever the client said.
fn set_surface(request: &mut pb::request::Kind, surface: pb::ChangeSurface) {
    use pb::request::Kind as Request;
    match request {
        Request::CreateItem(create) => create.set_surface(surface),
        Request::UpdateItem(update) => update.set_surface(surface),
        Request::DeleteItem(delete) => delete.set_surface(surface),
        Request::RevertBoardChange(revert) => revert.set_surface(surface),
        _ => {}
    }
}

/// One client's connection to the daemon: its admin actions wait for the
/// person unless it is the app.
pub struct Connection {
    daemon: Arc<Daemon>,
    peer: Peer,
}

impl Connection {
    pub fn new(daemon: Arc<Daemon>, peer: Peer) -> Arc<Self> {
        Arc::new(Self { daemon, peer })
    }

    async fn answer(&self, request: pb::request::Kind) -> Result<Reply, DaemonError> {
        let Peer::Client {
            program,
            parent,
            surface,
        } = &self.peer
        else {
            return self.daemon.answer(request).await;
        };
        if app_only(&request) {
            return Err(DaemonError::AppOnly);
        }
        if self.daemon.decides_agent_sessions(&request)? {
            return Err(crate::error::AgentSessionsError::AppOnly.into());
        }
        let Some(scope) = scope_of(&request) else {
            return self.daemon.answer(request).await;
        };
        let mut request = request;
        set_surface(&mut request, *surface);
        self.daemon
            .ask_approval(program, parent, scope, request)
            .await
            .map(|approval| {
                if approval.state() == pb::ApprovalState::Pending {
                    Reply::Response(pb::response::Kind::NeedsApproval(Box::new(
                        pb::NeedsApproval {
                            approval: Some(Box::new(approval)),
                        },
                    )))
                } else {
                    let result = approval.result.and_then(|result| result.kind);
                    Reply::Response(result.unwrap_or_else(|| {
                        error(pb::ErrorCode::StorageFailed, "the action left no answer")
                    }))
                }
            })
    }
}

impl Service for Connection {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            match self.answer(request).await {
                Ok(reply) => reply,
                Err(failure) => Reply::Response(error(failure.code(), describe(&failure))),
            }
        })
    }
}

impl Daemon {
    /// Runs admin action `request` from `program` if a grant covers it,
    /// returning it approved with its result; otherwise keeps it for the
    /// person and returns it pending.
    async fn ask_approval(
        &self,
        program: &str,
        parent: &str,
        scope: pb::ApprovalScope,
        request: pb::request::Kind,
    ) -> Result<pb::Approval, DaemonError> {
        let (summary, before, after, project_id) = self.describe_action(&request).await;
        let mut approval = pb::Approval {
            id: uuid::Uuid::new_v4().to_string(),
            client: program.to_owned(),
            parent: parent.to_owned(),
            scope: scope.into(),
            summary,
            before,
            after,
            state: pb::ApprovalState::Pending.into(),
            asked_unix_ms: now_unix_ms(),
            project_id,
            ..Default::default()
        };
        let request = pb::Request {
            kind: Some(request),
        };
        let grant = self.grant_for(scope)?;
        if let Some(grant) = grant {
            approval.grant_id = grant.id.clone();
            approval.state = pb::ApprovalState::Approved.into();
            approval.decided_unix_ms = now_unix_ms();
            // Kept before it runs, so a crash while it does is known, not
            // lost, and isn't taken for never having been asked.
            self.store.add_approval(&approval, &request)?;
            let result = self.run_approved(&approval, request.kind.clone()).await;
            approval.result = Some(Box::new(result));
            self.store.record_result(&approval)?;
            self.approvals_changed.notify_waiters();
            return Ok(approval);
        }
        self.store.add_approval(&approval, &request)?;
        self.approvals_changed.notify_waiters();
        Ok(approval)
    }

    /// A grant not expired or revoked that covers `scope`.
    fn grant_for(&self, scope: pb::ApprovalScope) -> Result<Option<pb::Grant>, DaemonError> {
        let now = now_unix_ms();
        Ok(self.store.grants()?.into_iter().find(|grant| {
            !grant.revoked && grant.expires_unix_ms > now && grant.scopes().any(|s| s == scope)
        }))
    }

    /// Runs `request` under `approval`, as the app would, so the board's
    /// journal records the approval or grant behind each change.
    ///
    /// Boxed and declared Send: deciding approvals runs requests through
    /// `answer`, which decides approvals, so the compiler can't work it out.
    fn run_approved<'a>(
        &'a self,
        approval: &pb::Approval,
        request: Option<pb::request::Kind>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = pb::Response> + Send + 'a>> {
        let via = Via {
            approval_id: approval.id.clone(),
            grant_id: approval.grant_id.clone(),
        };
        Box::pin(async move {
            let Some(request) = request else {
                return pb::Response {
                    kind: Some(error(
                        pb::ErrorCode::InvalidRequest,
                        "the approved request was empty",
                    )),
                };
            };
            // On a task of its own: a stack of its own, rather than on top
            // of the request that decided it, and it finishes even if
            // whoever asked goes away.
            let Some(daemon) = self.me.upgrade() else {
                return pb::Response {
                    kind: Some(error(
                        pb::ErrorCode::HostUnavailable,
                        "humatad is shutting down, so the approved action didn't run; ask for it again once it's back",
                    )),
                };
            };
            let running =
                tokio::spawn(RUNNING_UNDER.scope(via, async move { daemon.answer(request).await }));
            let kind = match running.await {
                Ok(Ok(Reply::Response(response))) => response,
                Ok(Ok(Reply::Stream { response, .. })) => response,
                Ok(Err(failure)) => error(failure.code(), describe(&failure)),
                Err(_) => error(
                    pb::ErrorCode::StorageFailed,
                    "the approved action stopped before it finished; check the board and ask for it again if it's missing",
                ),
            };
            pb::Response { kind: Some(kind) }
        })
    }

    /// What `request` will do, in a line, with the text it changes from and
    /// to, and its project.
    async fn describe_action(
        &self,
        request: &pb::request::Kind,
    ) -> (String, String, String, String) {
        use pb::request::Kind as Request;
        let item = |id: &str| self.store.item(id).ok();
        let quoted = |title: &str| format!("\u{201c}{title}\u{201d}");
        match request {
            Request::ApplyProposal(apply) => {
                let proposal = self
                    .pending(&apply.session_id, &apply.proposal_id)
                    .await
                    .ok();
                let title = apply
                    .title
                    .clone()
                    .or_else(|| proposal.as_ref().map(|proposal| proposal.title.clone()))
                    .unwrap_or_default();
                let spec = apply
                    .spec
                    .clone()
                    .or_else(|| proposal.as_ref().map(|proposal| proposal.spec.clone()))
                    .unwrap_or_default();
                let target = proposal
                    .as_ref()
                    .filter(|proposal| !proposal.item_id.is_empty())
                    .and_then(|proposal| item(&proposal.item_id));
                let project = proposal
                    .as_ref()
                    .map(|proposal| proposal.project_id.clone())
                    .unwrap_or_default();
                match target {
                    Some(target) => (
                        format!(
                            "Apply the proposed edit to {}",
                            quoted(&target.fields.title)
                        ),
                        target.fields.spec.clone(),
                        spec,
                        target.project_id,
                    ),
                    None => (
                        format!("Apply the proposed task {}", quoted(&title)),
                        String::new(),
                        spec,
                        project,
                    ),
                }
            }
            Request::DeclineProposal(decline) => {
                let proposal = self
                    .pending(&decline.session_id, &decline.proposal_id)
                    .await
                    .ok();
                let title = proposal
                    .as_ref()
                    .map(|proposal| proposal.title.clone())
                    .unwrap_or_default();
                (
                    format!("Decline the proposal {}", quoted(&title)),
                    String::new(),
                    String::new(),
                    proposal
                        .map(|proposal| proposal.project_id)
                        .unwrap_or_default(),
                )
            }
            Request::CreateItem(create) => {
                let under = item(&create.parent_id)
                    .map(|parent| format!(" under {}", quoted(&parent.fields.title)))
                    .unwrap_or_default();
                (
                    format!("Add the task {}{under}", quoted(&create.title)),
                    String::new(),
                    create.spec.clone(),
                    create.project_id.clone(),
                )
            }
            Request::UpdateItem(update) => {
                let Some(current) = item(&update.item_id) else {
                    return (
                        format!("Change item {}", update.item_id),
                        String::new(),
                        String::new(),
                        String::new(),
                    );
                };
                let mut changes = Vec::new();
                if let Some(title) = &update.title {
                    changes.push(format!("title to {}", quoted(title)));
                }
                if update.spec.is_some() {
                    changes.push("its spec".to_owned());
                }
                if update.status.is_some() {
                    changes.push(format!("status to {:?}", update.status()));
                }
                if update.priority.is_some() {
                    changes.push(format!("priority to {:?}", update.priority()));
                }
                let (before, after) = match &update.spec {
                    Some(spec) => (current.fields.spec.clone(), spec.clone()),
                    None => (String::new(), String::new()),
                };
                (
                    format!(
                        "Change {}: {}",
                        quoted(&current.fields.title),
                        changes.join(", ")
                    ),
                    before,
                    after,
                    current.project_id,
                )
            }
            Request::DeleteItem(delete) => {
                let current = item(&delete.item_id);
                (
                    format!(
                        "Delete {}",
                        quoted(
                            &current
                                .as_ref()
                                .map(|item| item.fields.title.clone())
                                .unwrap_or_else(|| delete.item_id.clone())
                        )
                    ),
                    String::new(),
                    String::new(),
                    current.map(|item| item.project_id).unwrap_or_default(),
                )
            }
            Request::RevertBoardChange(revert) => {
                let current = self.store.changed_item(revert.change_id).ok().flatten();
                (
                    format!(
                        "Revert board change {} to {}",
                        revert.change_id,
                        quoted(
                            &current
                                .as_ref()
                                .map(|item| item.fields.title.clone())
                                .unwrap_or_default()
                        )
                    ),
                    String::new(),
                    String::new(),
                    current.map(|item| item.project_id).unwrap_or_default(),
                )
            }
            Request::ImportIssues(import) => {
                let name = self
                    .store
                    .project(&import.project_id)
                    .map(|project| project.name)
                    .unwrap_or_default();
                (
                    format!("Import the open GitHub issues of {}", quoted(&name)),
                    String::new(),
                    String::new(),
                    import.project_id.clone(),
                )
            }
            _ => Default::default(),
        }
    }

    /// Approval `id`, waiting up to `wait` for the person while it's
    /// pending.
    pub(super) async fn wait_for_approval(
        &self,
        id: &str,
        wait: Duration,
    ) -> Result<pb::Approval, DaemonError> {
        let until = tokio::time::Instant::now() + wait.min(LONGEST_WAIT);
        loop {
            let changed = self.approvals_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let (approval, _) = self
                .store
                .approval(id)?
                .ok_or_else(|| DaemonError::UnknownApproval { id: id.to_owned() })?;
            // Approved but not yet answered is still running.
            let settled = match approval.state() {
                pb::ApprovalState::Pending => false,
                pb::ApprovalState::Approved => approval.result.is_some(),
                _ => true,
            };
            if settled || tokio::time::timeout_at(until, changed).await.is_err() {
                return Ok(approval);
            }
        }
    }

    /// The person's decision on the pending approvals `ids`, and the grant
    /// made with it, if asked for.
    pub(super) async fn decide_approvals(
        &self,
        decide: pb::DecideApprovals,
    ) -> Result<pb::ApprovalsDecided, DaemonError> {
        let grant = if decide.approve && decide.grant_seconds > 0 && !decide.grant_scopes.is_empty()
        {
            let lasts = Duration::from_secs(decide.grant_seconds.into()).min(LONGEST_GRANT);
            let now = now_unix_ms();
            let grant = pb::Grant {
                id: uuid::Uuid::new_v4().to_string(),
                scopes: decide.grant_scopes.clone(),
                granted_unix_ms: now,
                expires_unix_ms: now
                    .saturating_add(i64::try_from(lasts.as_millis()).unwrap_or(i64::MAX)),
                revoked: false,
                used: 0,
            };
            self.store.add_grant(&grant)?;
            Some(grant)
        } else {
            None
        };
        let mut decided = Vec::new();
        for id in &decide.ids {
            let Some((mut approval, request)) = self.store.approval(id)? else {
                continue;
            };
            if approval.state() != pb::ApprovalState::Pending {
                continue;
            }
            approval.decided_unix_ms = now_unix_ms();
            if decide.approve {
                approval.state = pb::ApprovalState::Approved.into();
                // Kept as decided first, so a second decision can't run it again.
                if !self.store.decide_approval(&approval)? {
                    continue;
                }
                // The person approved the change from the text they were
                // shown; text edited since would be overwritten unseen.
                let now = match &request.kind {
                    Some(kind) => self.describe_action(kind).await.1,
                    None => String::new(),
                };
                approval.result = Some(Box::new(if now == approval.before {
                    self.run_approved(&approval, request.kind).await
                } else {
                    pb::Response {
                        kind: Some(error(
                            pb::ErrorCode::Changed,
                            "the text this changes was edited after it was asked for, so it didn't run, to keep that edit; read it again and ask for the change anew if it's still wanted",
                        )),
                    }
                }));
                approval.decided_unix_ms = now_unix_ms();
                self.store.record_result(&approval)?;
            } else {
                approval.state = pb::ApprovalState::Denied.into();
                if !self.store.decide_approval(&approval)? {
                    continue;
                }
            }
            decided.push(approval);
        }
        self.approvals_changed.notify_waiters();
        Ok(pb::ApprovalsDecided {
            approvals: decided,
            grant,
        })
    }

    /// Marks the approved actions that were running when the daemon
    /// stopped, which never answered, as cut short: whoever waits on one is
    /// told to look before asking again, which could run it twice.
    pub(super) fn interrupt_unfinished_approvals(&self) -> Result<(), DaemonError> {
        for mut approval in self.store.approvals(false)? {
            if approval.state() != pb::ApprovalState::Approved || approval.result.is_some() {
                continue;
            }
            approval.result = Some(Box::new(pb::Response {
                kind: Some(error(
                    pb::ErrorCode::BackendFailed,
                    "Humata stopped while this ran, so it may or may not have taken effect; check the board and its history before asking for it again",
                )),
            }));
            self.store.record_result(&approval)?;
        }
        Ok(())
    }

    /// The approvals kept, newest first, and the grants still in force.
    pub(super) fn list_approvals(&self) -> Result<pb::Approvals, DaemonError> {
        let now = now_unix_ms();
        Ok(pb::Approvals {
            approvals: self.store.approvals(false)?,
            grants: self
                .store
                .grants()?
                .into_iter()
                .filter(|grant| !grant.revoked && grant.expires_unix_ms > now)
                .collect(),
        })
    }

    /// The approvals waiting for the person, oldest first.
    pub(super) fn pending_approvals(&self) -> Result<Vec<pb::Approval>, DaemonError> {
        Ok(self.store.approvals(true)?)
    }

    pub(super) fn revoke_grant(&self, id: &str) -> Result<(), DaemonError> {
        if self.store.revoke_grant(id)? {
            Ok(())
        } else {
            Err(DaemonError::UnknownGrant { id: id.to_owned() })
        }
    }
}
