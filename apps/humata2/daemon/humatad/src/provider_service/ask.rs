//! MP-23, D792: an agent asking the user to import a provider file. The
//! agent hands over the file's text; a file with problems is refused at
//! once, and otherwise the ask waits in the Inbox with the whole file,
//! every command its providers run and the ids it replaces on the card.
//! Approving writes the merged providers file while it is as it was when
//! the agent asked.

use humata2_grpc::humata::v1 as pb;
use humata2_machines::Provider;
use tonic::{Code, Status};

use super::import::Plan;
use crate::status;

/// The most text an agent may hand over: a provider file is a few tables.
pub(super) const MOST_BYTES: usize = 64 * 1024;

/// Why an agent's ask to import a provider file was refused before it
/// reached the Inbox.
#[derive(Debug, thiserror::Error)]
pub(super) enum AskError {
    #[error(
        "the provider file is {bytes} bytes, more than the {most} an agent may hand over, so nothing was asked. Hand over only the provider tables, or ask the user to import the file in Settings › Machines › Providers",
        most = MOST_BYTES
    )]
    TooLarge { bytes: usize },
    #[error("the ask has no reason, so nothing was asked. Say why the task needs the provider, and ask again")]
    NoReason,
    #[error("the provider file has problems, so nothing was asked: {}. Fix the file and ask again", listed(.problems))]
    Problems { problems: Vec<pb::ProviderConfigProblem> },
}

impl AskError {
    pub(super) fn into_status(self) -> Status {
        let (field, problem) = match &self {
            Self::TooLarge { .. } => ("text", pb::invalid_field::Problem::TooLong),
            Self::NoReason => ("reason", pb::invalid_field::Problem::Missing),
            Self::Problems { .. } => ("text", pb::invalid_field::Problem::BadFormat),
        };
        status::failure(
            Code::InvalidArgument,
            status::error(
                self.to_string(),
                "",
                pb::error::Kind::InvalidField(pb::InvalidField {
                    field: field.to_owned(),
                    problem: problem as i32,
                }),
            ),
        )
    }
}

fn listed(problems: &[pb::ProviderConfigProblem]) -> String {
    problems
        .iter()
        .map(|problem| {
            let message = problem.error.as_ref().map_or("", |error| error.message.as_str());
            match &problem.provider {
                Some(provider) => format!("{} {}: {message}", provider.value, problem.field),
                None => format!("{}: {message}", problem.field),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Checks the ask before anything is put in the Inbox.
pub(super) fn check(request: &pb::RequestProviderImportRequest) -> Result<(), AskError> {
    if request.text.len() > MOST_BYTES {
        return Err(AskError::TooLarge {
            bytes: request.text.len(),
        });
    }
    if request.reason.trim().is_empty() {
        return Err(AskError::NoReason);
    }
    Ok(())
}

/// The merged providers file the ask would write, or its problems.
pub(super) fn merged(plan: &Plan) -> Result<String, AskError> {
    match (&plan.merged, plan.problems.is_empty()) {
        (Some(merged), true) => Ok(merged.clone()),
        _ => Err(AskError::Problems {
            problems: plan.problems.clone(),
        }),
    }
}

/// The Inbox card's preview: the file whole and what it would do.
pub(super) fn preview(request: &pb::RequestProviderImportRequest, plan: &Plan) -> pb::request_preview::Preview {
    let id = |value: &str| pb::ProviderId { value: value.to_owned() };
    pb::request_preview::Preview::ProviderImport(pb::ProviderImportPreview {
        text: request.text.clone(),
        file_name: request.file_name.clone(),
        providers: plan
            .providers
            .iter()
            .map(|provider: &Provider| pb::provider_import_preview::Provider {
                id: Some(id(provider.id())),
                label: provider.label().to_owned(),
            })
            .collect(),
        commands: plan
            .providers
            .iter()
            .flat_map(|provider| {
                provider
                    .commands()
                    .into_iter()
                    .map(|(action, command)| pb::provider_import_preview::Command {
                        provider: Some(id(provider.id())),
                        action: action.to_owned(),
                        command: command.to_owned(),
                    })
            })
            .collect(),
        replaces: plan.replaces.iter().map(|value| id(value)).collect(),
    })
}

/// An approved ask whose providers file changed since the agent asked.
pub(super) fn stale() -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            "the providers changed after the agent asked to import its file, so nothing was added".to_owned(),
            "ask again so the Inbox shows the file against the providers as they are now, and approve that",
            pb::error::Kind::PlanStale(pb::PlanStale { changes: Vec::new() }),
        ),
    )
}

/// The user's own calls import with PreviewProviderImport, not this.
pub(super) fn not_for_the_user() -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            "RequestProviderImport is an agent's ask; the user imports a provider file directly".to_owned(),
            "preview the file with PreviewProviderImport (humata provider import <file>) and import the plan it gives",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "caller".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}
