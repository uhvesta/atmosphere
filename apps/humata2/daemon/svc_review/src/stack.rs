//! A review that is a stack of layers (D800, D803): `review_prepare` with
//! `stack` and/or `from_stack` is expanded and checked here, review_context
//! reports each layer's resolved commits and drift, and review_focus moves
//! between layers.

use std::collections::BTreeSet;

use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::{
    CheckedStack, Clones, DiffSpec, DriftKind, FromStack, FromStackError, Note, ReviewPlan, Stack,
    StackError, StackStatus, wire,
};
use humata2_store::SessionRecord;
use thiserror::Error;
use tonic::{Code, Status};

use super::{
    Review, SessionReview, anchor_failed, blocking, clones, context_of, diff_failed, lock,
    plan_failed, repo_keys,
};
use crate::ports::{error, failure, invalid_field};

/// What a prepared stack review keeps beside the chat.
#[derive(Default)]
pub(super) struct Stacked {
    pub(super) checked: Option<CheckedStack>,
    pub(super) from: Option<FromStack>,
}

/// Why a stack couldn't be prepared.
#[derive(Debug, Error)]
enum PrepareError {
    #[error(transparent)]
    From(#[from] FromStackError),
    #[error(transparent)]
    Stack(#[from] StackError),
}

/// "3 layers over 2 repos": what an offered stack review says about itself
/// when the agent gave no summary.
pub(super) fn offer_text(plan: &pb::ReviewPlan) -> Option<String> {
    let stack = plan.stack.as_ref();
    if stack.is_none() && plan.from_stack.is_none() {
        return None;
    }
    let layers = stack.map_or(0, |stack| stack.layers.len());
    let mut repos = BTreeSet::new();
    if let Some(stack) = stack {
        for layer in &stack.layers {
            for part in &layer.repos {
                if let Some(repo) = &part.repo {
                    repos.insert(wire::repo_key(repo));
                }
            }
        }
    }
    if layers == 0 {
        return None;
    }
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    Some(format!(
        "{} over {}",
        plural(layers, "layer", "layers"),
        plural(repos.len(), "repo", "repos")
    ))
}

fn invalid(field: &str, message: String) -> Status {
    failure(
        Code::FailedPrecondition,
        error(
            message,
            "",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

fn prepare_failed(err: &PrepareError) -> Status {
    match err {
        PrepareError::From(from) => invalid("from_stack", from.to_string()),
        PrepareError::Stack(stack) => invalid("stack", stack.to_string()),
    }
}

fn prepare_blocking(
    clones: &Clones,
    runner: &dyn StackStatus,
    from: Option<&FromStack>,
    explicit: Option<Stack>,
) -> Result<CheckedStack, PrepareError> {
    let (whitespace, renames, explicit_layers) = match explicit {
        Some(stack) => (Some(stack.whitespace), Some(stack.renames), stack.layers),
        None => (None, None, Vec::new()),
    };
    let layers = match from {
        Some(from) => {
            let checkout = clones
                .get(&from.repo)
                .map_err(|source| FromStackError::Git { source })?;
            let read = from.read_blocking(checkout, runner)?;
            FromStack::merge(read, explicit_layers, &from.repo)?
        }
        None => explicit_layers,
    };
    let mut stack = Stack::new(layers);
    if let Some(whitespace) = whitespace {
        stack.whitespace = whitespace;
    }
    if let Some(renames) = renames {
        stack.renames = renames;
    }
    Ok(stack.check_blocking(clones)?)
}

fn stack_repos(plan: &ReviewPlan) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut add = |key: &str| {
        if !keys.iter().any(|held| held == key) {
            keys.push(key.to_owned());
        }
    };
    if let Some(from) = &plan.from_stack {
        add(&from.repo);
    }
    if let Some(stack) = &plan.stack {
        for layer in &stack.layers {
            for part in &layer.repos {
                add(&part.repo);
            }
        }
    }
    keys
}

impl Review {
    pub(super) async fn checked_stack_plan(
        &self,
        caller: &Caller,
        record: &SessionRecord,
        session: &humata2_access::SessionRef,
        mut plan: ReviewPlan,
    ) -> Result<(ReviewPlan, Vec<humata2_review_diff::RepoDiff>, CheckedStack), Status> {
        let asked: Vec<pb::RepoRef> = stack_repos(&plan)
            .iter()
            .map(|key| wire::repo_ref(key))
            .collect();
        let keys = repo_keys(&self.inner, record, &asked).await?;
        self.require_read(caller, session, &keys)?;
        let clones = clones(record, &keys).await?;
        let runner = self.inner.stack_status.clone();
        let from = plan.from_stack.clone();
        let explicit = plan.stack.take();
        let (clones, checked) = blocking(move || {
            let checked = prepare_blocking(&clones, runner.as_ref(), from.as_ref(), explicit);
            (clones, checked)
        })
        .await?;
        let checked = checked.map_err(|err| prepare_failed(&err))?;
        plan.stack = Some(checked.stack().clone());
        plan.spec = checked.whole();
        for (index, layer) in checked.stack().layers.iter().enumerate() {
            humata2_review_diff::check_importance(&layer.importance).map_err(|err| {
                super::importance_failed(
                    &format!("review_plan.stack.layers[{index}].importance"),
                    &err,
                )
            })?;
        }
        let joined = plan.order.is_empty() && plan.groups.is_empty() && plan.importance.is_empty();
        let context = context_of(record);
        let diffs = clones
            .diff(&plan.spec, &context)
            .await
            .map_err(|err| diff_failed(&err))?;
        if joined {
            let shown = |file: &humata2_review_diff::FileRef| {
                diffs.iter().any(|diff| {
                    diff.repo == file.repo && diff.files.iter().any(|each| each.path == file.path)
                })
            };
            let whole = checked.whole_plan();
            plan.order = whole.order.into_iter().filter(|file| shown(file)).collect();
            plan.importance = whole
                .importance
                .into_iter()
                .filter(|entry| shown(&entry.file))
                .collect();
        }
        plan.check(&diffs).map_err(|err| plan_failed(&err))?;
        let notes = std::mem::take(&mut plan.notes);
        plan.notes = anchored_notes(&clones, &checked, &plan, &context, &diffs, notes).await?;
        Ok((plan, diffs, checked))
    }

    pub(super) async fn context_stack(
        &self,
        record: &SessionRecord,
        review: &SessionReview,
        stack: &Stack,
    ) -> Option<pb::ReviewStack> {
        let mut keys: Vec<String> = Vec::new();
        for layer in &stack.layers {
            for part in &layer.repos {
                if !keys.contains(&part.repo) {
                    keys.push(part.repo.clone());
                }
            }
        }
        let clones = clones(record, &keys).await.ok()?;
        let (kept, from) = {
            let held = lock(&review.stacked);
            (held.checked.clone(), held.from.clone())
        };
        let kept = kept.filter(|kept| kept.stack() == stack);
        let runner = self.inner.stack_status.clone();
        let sent = stack.clone();
        let (checked, drift) = blocking(move || {
            let checked = match kept {
                Some(kept) => kept,
                None => sent.check_blocking(&clones).ok()?,
            };
            let mut drift = checked.drift_blocking(&clones);
            if let Some(from) = from
                && let Ok(checkout) = clones.get(&from.repo)
                && let Ok(now) = from.read_blocking(checkout, runner.as_ref())
            {
                drift.extend(FromStack::drift(&checked.stack().layers, &now));
            }
            Some((checked, drift))
        })
        .await
        .ok()??;
        lock(&review.stacked).checked = Some(checked.clone());
        let mut told = wire::stack_to_pb(stack, Some(&checked));
        for found in drift {
            let kind = match found.kind {
                DriftKind::Moved => pb::review_layer_resolved::Drift::Moved,
                DriftKind::Missing => pb::review_layer_resolved::Drift::Missing,
                DriftKind::HeadMoved => pb::review_layer_resolved::Drift::HeadMoved,
            };
            let resolved = told
                .layers
                .iter_mut()
                .filter(|layer| layer.title == found.layer)
                .flat_map(|layer| layer.repos.iter_mut())
                .filter(|part| {
                    part.repo.as_ref().map(wire::repo_key).as_deref() == Some(found.repo.as_str())
                })
                .filter_map(|part| part.resolved.as_mut());
            for resolved in resolved {
                if resolved.drift == pb::review_layer_resolved::Drift::None as i32
                    || resolved.drift == pb::review_layer_resolved::Drift::Unspecified as i32
                {
                    resolved.drift = kind as i32;
                }
            }
        }
        Some(told)
    }

    pub(super) fn keep_stack(
        review: &SessionReview,
        checked: CheckedStack,
        from: Option<FromStack>,
    ) {
        let mut held = lock(&review.stacked);
        held.checked = Some(checked);
        held.from = from;
    }
}

async fn anchored_notes(
    clones: &Clones,
    checked: &CheckedStack,
    plan: &ReviewPlan,
    context: &humata2_review_diff::BaseContext,
    whole: &[humata2_review_diff::RepoDiff],
    notes: Vec<Note>,
) -> Result<Vec<Note>, Status> {
    let titles: Vec<Option<String>> = notes.iter().map(|note| note.layer.clone()).collect();
    let mut done: Vec<Option<Note>> = vec![None; notes.len()];
    let mut groups: Vec<Option<String>> = Vec::new();
    for title in &titles {
        if !groups.contains(title) {
            groups.push(title.clone());
        }
    }
    for group in groups {
        let (spec, diffs): (DiffSpec, Vec<humata2_review_diff::RepoDiff>) = match &group {
            None => (plan.spec.clone(), whole.to_vec()),
            Some(title) => {
                let spec = checked
                    .layer_spec(title)
                    .map_err(|err| prepare_failed(&PrepareError::Stack(err)))?;
                let diffs = clones
                    .diff(&spec, context)
                    .await
                    .map_err(|err| diff_failed(&err))?;
                (spec, diffs)
            }
        };
        let mut indexes = Vec::new();
        let mut mine = Vec::new();
        for (index, note) in notes.iter().enumerate() {
            if titles.get(index) == Some(&group) {
                indexes.push(index);
                mine.push(note.clone());
            }
        }
        if group.is_some() {
            let mut alone = plan.clone();
            alone.order.clear();
            alone.groups.clear();
            alone.notes.clone_from(&mine);
            alone.check(&diffs).map_err(|err| plan_failed(&err))?;
        }
        for (diff, repo) in diffs.iter().zip(&spec.repos) {
            let checkout = clones
                .get(&diff.repo)
                .map_err(|err| diff_failed(&err))?
                .clone();
            mine = checkout
                .anchor_notes(diff.clone(), humata2_review_diff::Matching::from(repo), mine)
                .await
                .map_err(|err| anchor_failed(&err))?;
        }
        for (index, note) in indexes.into_iter().zip(mine) {
            if let Some(slot) = done.get_mut(index) {
                *slot = Some(note);
            }
        }
    }
    Ok(done.into_iter().flatten().collect())
}

pub(super) fn layer_asked(
    request: &pb::FocusReviewRequest,
) -> Result<Option<Option<String>>, Status> {
    match (request.layer.trim().is_empty(), request.whole_stack) {
        (false, true) => Err(invalid_field(
            "layer",
            pb::invalid_field::Problem::NotAllowedHere,
            "send `layer` or `whole_stack`, not both".to_owned(),
            "send one of them",
        )),
        (false, false) => Ok(Some(Some(request.layer.clone()))),
        (true, true) => Ok(Some(None)),
        (true, false) => Ok(None),
    }
}
