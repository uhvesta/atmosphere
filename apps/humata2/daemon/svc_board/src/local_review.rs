//! Local Review Requested state machine, deterministic PR→To-do auto-attach,
//! and the single `Move to its own To-do` escape hatch (`ATM-56`, `ATM-71`, `ATM-72`).

use humata2_daemon_ports::Caller;
use humata2_rpc_kit::lock;
use humata2_todo::State;
use tonic::Status;

use crate::pb;
use crate::todo::{Entry, TodoApplied};
use crate::todo_fields::{
    self as saved, FieldsError, LocalReviewLayerRow, LocalReviewRow, LocalReviewState, StepRow,
};
use crate::BoardSvc;

/// Explicit errors for local review transitions and deterministic auto-attach.
#[derive(Debug, thiserror::Error)]
pub enum LocalReviewError {
    #[error("there is no to-do item {key}")]
    NotFound { key: String },
    #[error("to-do item {key} does not have an active local review")]
    NoLocalReview { key: String },
    #[error(
        "cannot submit local review for {key}: owner has not approved pushing yet; wait for the owner to click Looks good"
    )]
    PushNotApproved { key: String },
    #[error("cannot transition local review for {key} from {from:?} to {to:?}")]
    InvalidTransition {
        key: String,
        from: LocalReviewState,
        to: LocalReviewState,
    },
    #[error("to-do item {key} was not auto-attached, so there is nothing to detach")]
    NotAutoAttached { key: String },
    #[error("couldn't find auto-attached step or layer '{target}' on to-do item {key}")]
    LayerNotFound { key: String, target: String },
    #[error(transparent)]
    Fields(#[from] FieldsError),
    #[error("board operation failed: {0}")]
    Rpc(#[from] Status),
}

/// Buttonless review-inbox notice returned when an agent requests local review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalReviewNotice {
    pub todo_key: String,
    pub layers_count: usize,
    pub text: String,
}

impl LocalReviewNotice {
    fn new(todo_key: String, layers_count: usize) -> Self {
        let text = if layers_count > 1 {
            format!("{todo_key} · {layers_count} layers ready for your review")
        } else {
            format!("{todo_key} · ready for your review")
        };
        Self {
            todo_key,
            layers_count,
            text,
        }
    }
}

/// Input for requesting a local review on an existing To-do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalReviewRequest {
    pub layers: Vec<LocalReviewLayerRow>,
    pub summary_what: String,
    pub summary_why: String,
    pub summary_details: String,
}

/// Input when an agent requests local review for work that has no To-do yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IncomingLocalReview {
    pub channel: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub layers: Vec<LocalReviewLayerRow>,
    pub summary_what: String,
    pub summary_why: String,
    pub summary_details: String,
}

/// Result of deterministic auto-attach or standalone To-do creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachOrCreateOutcome {
    pub todo_key: String,
    pub attached: bool,
    pub attachment_reason: Option<String>,
    pub notice: LocalReviewNotice,
}

/// Produces a declarative outcome title for a newly created To-do.
///
/// Prefers `summary_what` when it is non-empty and `title` is empty or starts
/// with an imperative verb; otherwise cleans up `title`.
fn declarative_outcome_title(title: &str, summary_what: &str) -> String {
    let title = title.trim();
    let summary = summary_what.trim();
    let imperative_prefixes = [
        "add ",
        "fix ",
        "implement ",
        "update ",
        "refactor ",
        "create ",
        "remove ",
    ];
    let lower_title = title.to_ascii_lowercase();
    let title_is_imperative = imperative_prefixes
        .iter()
        .any(|prefix| lower_title.starts_with(prefix));

    if !summary.is_empty() && (title.is_empty() || title_is_imperative) {
        return summary.to_owned();
    }
    if !title.is_empty() {
        return title.to_owned();
    }
    "Local review ready".to_owned()
}

fn is_open_for_attach(entry: &Entry) -> bool {
    matches!(
        entry.state(),
        State::Open | State::InProgress | State::WatchingPr
    )
}

fn entry_matches_channel(entry: &Entry, channel: Option<&str>) -> bool {
    match channel {
        Some(wanted) if !wanted.is_empty() => entry.fields.channel.as_deref() == Some(wanted),
        _ => true,
    }
}

fn deterministic_match_reason(entry: &Entry, incoming: &IncomingLocalReview) -> Option<String> {
    // 1. Linked session: open To-do already has session_id in record.sessions or fields.owner.session.
    if let Some(sid) = incoming.session_id.as_deref().filter(|s| !s.is_empty()) {
        let in_record = entry.record.sessions.iter().any(|s| s == sid);
        let in_owner = entry
            .fields
            .owner
            .as_ref()
            .and_then(|o| o.session.as_deref())
            == Some(sid);
        if in_record || in_owner {
            return Some(format!("Linked to session {sid}"));
        }
    }

    // 2. Branch ancestry: open To-do already tracks a local-review layer in the same repo
    // whose head_branch == incoming.base_branch.
    if let Some(lr) = entry.fields.local_review.as_ref() {
        for existing in &lr.layers {
            if existing.head_branch.is_empty() {
                continue;
            }
            for inc in &incoming.layers {
                if !inc.base_branch.is_empty()
                    && existing.repo == inc.repo
                    && existing.head_branch == inc.base_branch
                {
                    let reason = match existing.pr_number {
                        Some(num) => format!("Stacked on {}#{num}", existing.repo),
                        None => format!("Stacked on {}:{}", existing.repo, existing.head_branch),
                    };
                    return Some(reason);
                }
            }
        }
    }

    None
}

impl BoardSvc {
    fn ensure_item_exists(&self, key: &str) -> Result<(), LocalReviewError> {
        let found = self
            .store
            .board_item(key)
            .map_err(|source| FieldsError::Read {
                what: "board item",
                source,
            })?;
        if found.is_none() {
            return Err(LocalReviewError::NotFound {
                key: key.to_owned(),
            });
        }
        Ok(())
    }

    /// Moves a To-do to `LocalReviewRequested` and returns a buttonless
    /// review-inbox notice. Never blocks in Agent inbox.
    pub fn request_local_review(
        &self,
        key: &str,
        request: LocalReviewRequest,
    ) -> Result<LocalReviewNotice, LocalReviewError> {
        self.ensure_item_exists(key)?;
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        let fields = map.entry(key.to_owned()).or_default();
        let prev = fields.local_review.take();
        let fixes_pushed_since_review = prev
            .as_ref()
            .is_some_and(|row| row.state == LocalReviewState::ChangesRequested);
        let (attachment_reason, auto_attached) = prev
            .as_ref()
            .map(|row| (row.attachment_reason.clone(), row.auto_attached))
            .unwrap_or((None, false));
        let layers_count = request.layers.len();
        fields.local_review = Some(LocalReviewRow {
            state: LocalReviewState::LocalReviewRequested,
            layers: request.layers,
            summary_what: request.summary_what,
            summary_why: request.summary_why,
            summary_details: request.summary_details,
            owner_approved_push: false,
            fixes_pushed_since_review,
            attachment_reason,
            auto_attached,
        });
        fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;
        Ok(LocalReviewNotice::new(key.to_owned(), layers_count))
    }

    /// Owner moves `LocalReviewRequested → ChangesRequested`.
    pub fn request_changes_local_review(&self, key: &str) -> Result<LocalReviewRow, LocalReviewError> {
        self.ensure_item_exists(key)?;
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        let fields = map.entry(key.to_owned()).or_default();
        let row = fields
            .local_review
            .as_mut()
            .ok_or_else(|| LocalReviewError::NoLocalReview {
                key: key.to_owned(),
            })?;
        if row.state != LocalReviewState::LocalReviewRequested {
            return Err(LocalReviewError::InvalidTransition {
                key: key.to_owned(),
                from: row.state,
                to: LocalReviewState::ChangesRequested,
            });
        }
        row.state = LocalReviewState::ChangesRequested;
        row.owner_approved_push = false;
        row.fixes_pushed_since_review = false;
        let out = row.clone();
        fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;
        Ok(out)
    }

    /// Owner `Looks good`: sets `owner_approved_push = true` so the agent may
    /// transition to `Submitted`.
    pub fn approve_local_review_for_push(
        &self,
        key: &str,
    ) -> Result<LocalReviewRow, LocalReviewError> {
        self.ensure_item_exists(key)?;
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        let fields = map.entry(key.to_owned()).or_default();
        let row = fields
            .local_review
            .as_mut()
            .ok_or_else(|| LocalReviewError::NoLocalReview {
                key: key.to_owned(),
            })?;
        if row.state != LocalReviewState::LocalReviewRequested {
            return Err(LocalReviewError::InvalidTransition {
                key: key.to_owned(),
                from: row.state,
                to: LocalReviewState::Submitted,
            });
        }
        row.owner_approved_push = true;
        let out = row.clone();
        fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;
        Ok(out)
    }

    /// Transitions `LocalReviewRequested → Submitted`. Refuses with
    /// `LocalReviewError::PushNotApproved` unless the owner has approved push.
    pub fn mark_local_review_submitted(
        &self,
        key: &str,
    ) -> Result<LocalReviewRow, LocalReviewError> {
        self.ensure_item_exists(key)?;
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        let fields = map.entry(key.to_owned()).or_default();
        let row = fields
            .local_review
            .as_mut()
            .ok_or_else(|| LocalReviewError::NoLocalReview {
                key: key.to_owned(),
            })?;
        if !row.owner_approved_push {
            return Err(LocalReviewError::PushNotApproved {
                key: key.to_owned(),
            });
        }
        if row.state != LocalReviewState::LocalReviewRequested {
            return Err(LocalReviewError::InvalidTransition {
                key: key.to_owned(),
                from: row.state,
                to: LocalReviewState::Submitted,
            });
        }
        row.state = LocalReviewState::Submitted;
        let out = row.clone();
        fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;
        Ok(out)
    }

    /// Transitions `Submitted → Approved`.
    pub fn mark_local_review_approved(&self, key: &str) -> Result<LocalReviewRow, LocalReviewError> {
        self.ensure_item_exists(key)?;
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        let fields = map.entry(key.to_owned()).or_default();
        let row = fields
            .local_review
            .as_mut()
            .ok_or_else(|| LocalReviewError::NoLocalReview {
                key: key.to_owned(),
            })?;
        if row.state != LocalReviewState::Submitted {
            return Err(LocalReviewError::InvalidTransition {
                key: key.to_owned(),
                from: row.state,
                to: LocalReviewState::Approved,
            });
        }
        row.state = LocalReviewState::Approved;
        let out = row.clone();
        fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;
        Ok(out)
    }

    /// Transitions `Approved → Done`.
    pub fn mark_local_review_done(&self, key: &str) -> Result<LocalReviewRow, LocalReviewError> {
        self.ensure_item_exists(key)?;
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        let fields = map.entry(key.to_owned()).or_default();
        let row = fields
            .local_review
            .as_mut()
            .ok_or_else(|| LocalReviewError::NoLocalReview {
                key: key.to_owned(),
            })?;
        if row.state != LocalReviewState::Approved {
            return Err(LocalReviewError::InvalidTransition {
                key: key.to_owned(),
                from: row.state,
                to: LocalReviewState::Done,
            });
        }
        row.state = LocalReviewState::Done;
        let out = row.clone();
        fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;
        Ok(out)
    }

    /// Deterministically attaches an incoming local review to a uniquely matching
    /// open/in-progress To-do (by linked session or branch ancestry only), or
    /// creates a new To-do with a declarative outcome title.
    pub fn attach_or_create_todo_for_local_review(
        &self,
        caller: &Caller,
        incoming: IncomingLocalReview,
        now: i64,
    ) -> Result<AttachOrCreateOutcome, LocalReviewError> {
        let entries = self.entries(caller)?;
        let mut matches: Vec<(String, String)> = Vec::new();
        for entry in &entries {
            if !is_open_for_attach(entry) {
                continue;
            }
            if !entry_matches_channel(entry, incoming.channel.as_deref()) {
                continue;
            }
            if let Some(reason) = deterministic_match_reason(entry, &incoming) {
                matches.push((entry.record.key.clone(), reason));
            }
        }

        if matches.len() == 1 {
            let (todo_key, reason) = matches.remove(0);
            let _write = lock(&self.writes);
            let mut map = saved::load(&self.store)?;
            let fields = map.entry(todo_key.clone()).or_default();
            let step_id = format!("lr-{}", fields.steps.len() + 1);
            let step_text = if !incoming.title.trim().is_empty() {
                incoming.title.trim().to_owned()
            } else {
                incoming
                    .layers
                    .first()
                    .map(|l| l.title.clone())
                    .unwrap_or_else(|| "Local review".to_owned())
            };
            fields.steps.push(StepRow {
                id: step_id,
                text: step_text,
                ..StepRow::default()
            });

            let prev = fields.local_review.take();
            let fixes_pushed_since_review = prev
                .as_ref()
                .is_some_and(|row| row.state == LocalReviewState::ChangesRequested);
            let mut layers = prev.map(|row| row.layers).unwrap_or_default();
            for layer in incoming.layers {
                if !layers.iter().any(|existing| {
                    existing.repo == layer.repo && existing.head_branch == layer.head_branch
                }) {
                    layers.push(layer);
                }
            }
            let layers_count = layers.len();
            fields.local_review = Some(LocalReviewRow {
                state: LocalReviewState::LocalReviewRequested,
                layers,
                summary_what: incoming.summary_what,
                summary_why: incoming.summary_why,
                summary_details: incoming.summary_details,
                owner_approved_push: false,
                fixes_pushed_since_review,
                attachment_reason: Some(reason.clone()),
                auto_attached: true,
            });
            fields.extra_revisions += 1;
            saved::save(&self.store, &map)?;
            let notice = LocalReviewNotice::new(todo_key.clone(), layers_count);
            return Ok(AttachOrCreateOutcome {
                todo_key,
                attached: true,
                attachment_reason: Some(reason),
                notice,
            });
        }

        // No deterministic match, or ambiguous (>1) matches: always create a new To-do.
        let title = declarative_outcome_title(&incoming.title, &incoming.summary_what);
        let description = [
            incoming.summary_what.trim(),
            incoming.summary_why.trim(),
            incoming.summary_details.trim(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        let request = pb::CreateTodoRequest {
            title,
            description,
            channel: incoming
                .channel
                .filter(|c| !c.is_empty())
                .map(|value| pb::ChannelId { value }),
            ..Default::default()
        };
        let TodoApplied::Done(done) = self.create_todo_in(caller, &request, false, now)? else {
            return Err(LocalReviewError::Rpc(Status::internal(
                "local review to-do creation was unexpectedly deferred",
            )));
        };
        let todo_key = done
            .item
            .and_then(|todo| todo.item)
            .and_then(|item| item.id)
            .map(|id| id.value)
            .ok_or_else(|| {
                LocalReviewError::Rpc(Status::internal("new to-do item came back with no id"))
            })?;
        let layers_count = incoming.layers.len();
        {
            let _write = lock(&self.writes);
            let mut map = saved::load(&self.store)?;
            let fields = map.entry(todo_key.clone()).or_default();
            fields.local_review = Some(LocalReviewRow {
                state: LocalReviewState::LocalReviewRequested,
                layers: incoming.layers,
                summary_what: incoming.summary_what,
                summary_why: incoming.summary_why,
                summary_details: incoming.summary_details,
                owner_approved_push: false,
                fixes_pushed_since_review: false,
                attachment_reason: None,
                auto_attached: false,
            });
            fields.extra_revisions += 1;
            saved::save(&self.store, &map)?;
        }
        let notice = LocalReviewNotice::new(todo_key.clone(), layers_count);
        Ok(AttachOrCreateOutcome {
            todo_key,
            attached: false,
            attachment_reason: None,
            notice,
        })
    }

    /// The single `Move to its own To-do` escape action: detaches an
    /// auto-attached step/layer from `source_key`, creates a standalone To-do
    /// with `from_keys: vec![source_key]`, and moves its `LocalReviewRow` there.
    pub fn move_local_review_to_own_todo(
        &self,
        caller: &Caller,
        source_key: &str,
        step_id_or_layer: &str,
        now: i64,
    ) -> Result<String, LocalReviewError> {
        self.ensure_item_exists(source_key)?;

        // Inspect source to validate and compute the standalone To-do's payload.
        let (title, description, channel, detached_row) = {
            let _write = lock(&self.writes);
            let map = saved::load(&self.store)?;
            let fields = map
                .get(source_key)
                .ok_or_else(|| LocalReviewError::NoLocalReview {
                    key: source_key.to_owned(),
                })?;
            let lr = fields
                .local_review
                .as_ref()
                .ok_or_else(|| LocalReviewError::NoLocalReview {
                    key: source_key.to_owned(),
                })?;
            if !lr.auto_attached {
                return Err(LocalReviewError::NotAutoAttached {
                    key: source_key.to_owned(),
                });
            }

            let target = step_id_or_layer.trim();
            let matched_step = if target.is_empty() {
                fields.steps.last()
            } else {
                fields.steps.iter().find(|s| s.id == target)
            };
            let matched_layer = if target.is_empty() {
                lr.layers.last().cloned()
            } else {
                lr.layers
                    .iter()
                    .find(|l| {
                        l.head_branch == target
                            || l.title == target
                            || l.pr_number.is_some_and(|n| format!("#{n}") == target)
                    })
                    .cloned()
                    .or_else(|| matched_step.and_then(|_| lr.layers.last().cloned()))
            };

            if !target.is_empty() && matched_step.is_none() && matched_layer.is_none() {
                return Err(LocalReviewError::LayerNotFound {
                    key: source_key.to_owned(),
                    target: target.to_owned(),
                });
            }

            let raw_title = matched_layer
                .as_ref()
                .map(|l| l.title.as_str())
                .filter(|t| !t.is_empty())
                .or_else(|| matched_step.map(|s| s.text.as_str()))
                .unwrap_or("");
            let title = declarative_outcome_title(raw_title, &lr.summary_what);
            let description = [
                lr.summary_what.trim(),
                lr.summary_why.trim(),
                lr.summary_details.trim(),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
            let layers = matched_layer.into_iter().collect();
            let detached_row = LocalReviewRow {
                state: lr.state,
                layers,
                summary_what: lr.summary_what.clone(),
                summary_why: lr.summary_why.clone(),
                summary_details: lr.summary_details.clone(),
                owner_approved_push: lr.owner_approved_push,
                fixes_pushed_since_review: lr.fixes_pushed_since_review,
                attachment_reason: None,
                auto_attached: false,
            };
            (title, description, fields.channel.clone(), detached_row)
        };

        let request = pb::CreateTodoRequest {
            title,
            description,
            channel: channel
                .filter(|c| !c.is_empty())
                .map(|value| pb::ChannelId { value }),
            ..Default::default()
        };
        let TodoApplied::Done(done) = self.create_todo_in(caller, &request, false, now)? else {
            return Err(LocalReviewError::Rpc(Status::internal(
                "moving local review to its own to-do was unexpectedly deferred",
            )));
        };
        let new_key = done
            .item
            .and_then(|todo| todo.item)
            .and_then(|item| item.id)
            .map(|id| id.value)
            .ok_or_else(|| {
                LocalReviewError::Rpc(Status::internal("new to-do item came back with no id"))
            })?;

        // Update both source and new To-do atomically under the write lock.
        let _write = lock(&self.writes);
        let mut map = saved::load(&self.store)?;
        if let Some(source) = map.get_mut(source_key) {
            let target = step_id_or_layer.trim();
            if target.is_empty() {
                source.steps.pop();
            } else {
                source.steps.retain(|s| s.id != target);
            }
            if let Some(lr) = source.local_review.as_mut() {
                if let Some(detached_layer) = detached_row.layers.first() {
                    lr.layers.retain(|l| {
                        !(l.repo == detached_layer.repo
                            && l.head_branch == detached_layer.head_branch)
                    });
                }
                lr.auto_attached = false;
                lr.attachment_reason = None;
                if lr.layers.is_empty() {
                    source.local_review = None;
                }
            }
            source.extra_revisions += 1;
        }
        let target_fields = map.entry(new_key.clone()).or_default();
        target_fields.from_keys = vec![source_key.to_owned()];
        target_fields.local_review = Some(detached_row);
        target_fields.extra_revisions += 1;
        saved::save(&self.store, &map)?;

        Ok(new_key)
    }
}

#[cfg(test)]
mod tests {
    use humata2_daemon_ports::{Caller, UserVia};
    use humata2_todo::OwnerKind;

    use super::*;
    use crate::test_support::test_svc;
    use crate::todo_fields::TodoOwnerRow;

    fn user() -> Caller {
        Caller::User(UserVia::App)
    }

    fn create_todo(svc: &BoardSvc, title: &str, channel: &str) -> String {
        let req = pb::CreateTodoRequest {
            title: title.to_owned(),
            channel: (!channel.is_empty()).then(|| pb::ChannelId {
                value: channel.to_owned(),
            }),
            ..Default::default()
        };
        let TodoApplied::Done(done) = svc.create_todo_in(&user(), &req, false, 100).unwrap() else {
            panic!("expected Done");
        };
        done.item.unwrap().item.unwrap().id.unwrap().value
    }

    fn layer(repo: &str, base: &str, head: &str, title: &str, pr: Option<u32>) -> LocalReviewLayerRow {
        LocalReviewLayerRow {
            repo: repo.to_owned(),
            base_branch: base.to_owned(),
            head_branch: head.to_owned(),
            title: title.to_owned(),
            pr_number: pr,
        }
    }

    #[test]
    fn local_review_state_machine_enforces_owner_approval_before_submit() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = create_todo(&svc, "Refund retries back off", "");

        let notice = svc
            .request_local_review(
                &key,
                LocalReviewRequest {
                    layers: vec![
                        layer("acme/api", "main", "retry-1", "Base retry", None),
                        layer("acme/api", "retry-1", "retry-2", "Jitter", None),
                        layer("acme/api", "retry-2", "retry-3", "Metrics", None),
                    ],
                    summary_what: "Refund retries use exponential backoff".to_owned(),
                    summary_why: "Avoids hammering Stripe during outages".to_owned(),
                    summary_details: "3 layers".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(notice.layers_count, 3);
        assert_eq!(
            notice.text,
            format!("{key} · 3 layers ready for your review")
        );

        // Agent cannot submit before owner clicks Looks good.
        assert!(matches!(
            svc.mark_local_review_submitted(&key),
            Err(LocalReviewError::PushNotApproved { .. })
        ));

        // Owner requests changes, then agent re-requests review -> fixes_pushed_since_review = true.
        let ch = svc.request_changes_local_review(&key).unwrap();
        assert_eq!(ch.state, LocalReviewState::ChangesRequested);

        let notice2 = svc
            .request_local_review(
                &key,
                LocalReviewRequest {
                    layers: vec![layer("acme/api", "main", "retry-1", "Base retry", None)],
                    summary_what: "Refund retries use exponential backoff".to_owned(),
                    summary_why: "Avoids hammering Stripe".to_owned(),
                    summary_details: "Addressed feedback".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(notice2.text, format!("{key} · ready for your review"));
        let map = saved::load(&svc.store).unwrap();
        assert!(map[&key].local_review.as_ref().unwrap().fixes_pushed_since_review);

        // Still refused until owner approves push.
        assert!(matches!(
            svc.mark_local_review_submitted(&key),
            Err(LocalReviewError::PushNotApproved { .. })
        ));

        // Owner clicks Looks good -> Submitted -> Approved -> Done.
        let approved_push = svc.approve_local_review_for_push(&key).unwrap();
        assert!(approved_push.owner_approved_push);
        let sub = svc.mark_local_review_submitted(&key).unwrap();
        assert_eq!(sub.state, LocalReviewState::Submitted);
        let app = svc.mark_local_review_approved(&key).unwrap();
        assert_eq!(app.state, LocalReviewState::Approved);
        let done = svc.mark_local_review_done(&key).unwrap();
        assert_eq!(done.state, LocalReviewState::Done);
    }

    #[test]
    fn auto_attaches_deterministically_by_linked_session_and_supports_escape_hatch() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let existing = create_todo(&svc, "Payments reliability", "");
        {
            let mut map = saved::load(&svc.store).unwrap();
            map.entry(existing.clone()).or_default().owner = Some(TodoOwnerRow {
                kind: OwnerKind::Agent,
                label: "Claude".to_owned(),
                session: Some("se_1".to_owned()),
            });
            saved::save(&svc.store, &map).unwrap();
        }

        let outcome = svc
            .attach_or_create_todo_for_local_review(
                &user(),
                IncomingLocalReview {
                    channel: None,
                    session_id: Some("se_1".to_owned()),
                    title: "Add retry backoff".to_owned(),
                    layers: vec![layer("acme/api", "main", "retry-backoff", "Retry backoff", None)],
                    summary_what: "Refund retries back off exponentially".to_owned(),
                    summary_why: "Prevents thundering herd".to_owned(),
                    summary_details: String::new(),
                },
                200,
            )
            .unwrap();
        assert!(outcome.attached);
        assert_eq!(outcome.todo_key, existing);
        assert_eq!(
            outcome.attachment_reason.as_deref(),
            Some("Linked to session se_1")
        );

        let map = saved::load(&svc.store).unwrap();
        let lr = map[&existing].local_review.as_ref().unwrap();
        assert!(lr.auto_attached);
        assert_eq!(map[&existing].steps.len(), 1);

        // Escape hatch: Move to its own To-do.
        let detached_key = svc
            .move_local_review_to_own_todo(&user(), &existing, "retry-backoff", 300)
            .unwrap();
        assert_ne!(detached_key, existing);
        let map = saved::load(&svc.store).unwrap();
        assert_eq!(map[&detached_key].from_keys, vec![existing.clone()]);
        let detached_lr = map[&detached_key].local_review.as_ref().unwrap();
        assert!(!detached_lr.auto_attached);
        assert_eq!(detached_lr.layers.len(), 1);
        assert!(map[&existing].local_review.is_none());
    }

    #[test]
    fn auto_attaches_deterministically_by_branch_ancestry() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let existing = create_todo(&svc, "Base ledger migration", "");
        svc.request_local_review(
            &existing,
            LocalReviewRequest {
                layers: vec![layer("acme/api", "main", "ledger-base", "Base", Some(412))],
                ..Default::default()
            },
        )
        .unwrap();

        let outcome = svc
            .attach_or_create_todo_for_local_review(
                &user(),
                IncomingLocalReview {
                    channel: None,
                    session_id: None,
                    title: "Add ledger read flag".to_owned(),
                    layers: vec![layer(
                        "acme/api",
                        "ledger-base",
                        "ledger-flag",
                        "Read flag",
                        None,
                    )],
                    summary_what: "Ledger reads gated behind a flag".to_owned(),
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        assert!(outcome.attached);
        assert_eq!(outcome.todo_key, existing);
        assert_eq!(
            outcome.attachment_reason.as_deref(),
            Some("Stacked on acme/api#412")
        );
    }

    #[test]
    fn never_fuzzy_matches_by_title_and_creates_declarative_todo_on_no_or_ambiguous_match() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        // Same title/keywords, but no deterministic signal -> must NOT attach.
        let unrelated = create_todo(&svc, "Add retry backoff", "");

        let outcome = svc
            .attach_or_create_todo_for_local_review(
                &user(),
                IncomingLocalReview {
                    channel: None,
                    session_id: None,
                    title: "Add retry backoff".to_owned(),
                    layers: vec![layer("acme/api", "main", "retry-backoff", "Retry backoff", None)],
                    summary_what: "Refund retries back off exponentially".to_owned(),
                    summary_why: "Protects upstream".to_owned(),
                    summary_details: String::new(),
                },
                200,
            )
            .unwrap();
        assert!(!outcome.attached);
        assert_ne!(outcome.todo_key, unrelated);
        assert_eq!(outcome.attachment_reason, None);

        let entries = svc.entries(&user()).unwrap();
        let created = entries
            .iter()
            .find(|e| e.record.key == outcome.todo_key)
            .unwrap();
        assert_eq!(created.record.title, "Refund retries back off exponentially");
    }
}
