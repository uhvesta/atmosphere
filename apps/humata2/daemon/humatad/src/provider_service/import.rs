//! MP-23: what importing a provider file would do, worked out before
//! anything is saved, and the plan the user's confirmation spends.

use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use humata2_machines::Provider;
use tonic::{Code, Status};

use super::problems_of;
use crate::status;

/// How long a previewed import stays good, as removal plans do.
pub(super) const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// A previewed import waiting for the user's confirmation.
pub(super) struct Planned {
    /// The whole providers file the import writes.
    pub(super) merged: String,
    /// The providers file's revision when the preview was made.
    pub(super) base: u64,
    pub(super) made: Instant,
}

/// What the preview shows.
pub(super) struct Plan {
    /// The file's providers, in its order.
    pub(super) providers: Vec<Provider>,
    /// Ids it replaces: ones the file already has, or the built-in Lima.
    pub(super) replaces: Vec<String>,
    pub(super) problems: Vec<pb::ProviderConfigProblem>,
    /// The providers file after the import; none when there are problems.
    pub(super) merged: Option<String>,
}

/// Plans importing `text` into the providers file that holds `current`,
/// beside the `existing` providers.
pub(super) fn plan(text: &str, current: &str, existing: &[Provider]) -> Plan {
    let providers = match Provider::parse(text) {
        Ok(providers) => providers,
        Err(error) => return refused(Vec::new(), problems_of(&error)),
    };
    if providers.is_empty() {
        return refused(
            providers,
            vec![problem(
                "providers",
                "the file has no providers, so there is nothing to import".to_owned(),
                "pick a file with at least one [providers.<id>] table",
            )],
        );
    }
    let replaces: Vec<String> = providers
        .iter()
        .map(|provider| provider.id().to_owned())
        .filter(|id| existing.iter().any(|known| known.id() == id))
        .collect();
    let merged = match merge(text, current) {
        Ok(merged) => merged,
        Err(problems) => return refused(providers, problems),
    };
    match Provider::parse(&merged) {
        Ok(_) => Plan {
            providers,
            replaces,
            problems: Vec::new(),
            merged: Some(merged),
        },
        Err(error) => Plan {
            providers,
            replaces,
            problems: problems_of(&error),
            merged: None,
        },
    }
}

fn refused(providers: Vec<Provider>, problems: Vec<pb::ProviderConfigProblem>) -> Plan {
    Plan {
        providers,
        replaces: Vec::new(),
        problems,
        merged: None,
    }
}

/// The providers file with `text`'s providers added. When none of them is
/// in the file yet, the text is appended, so the file's comments and the
/// import's stay; otherwise the tables are merged and the ones the import
/// names are replaced.
fn merge(text: &str, current: &str) -> Result<String, Vec<pb::ProviderConfigProblem>> {
    let unreadable = |message: String| {
        vec![problem(
            "toml",
            message,
            "fix the providers file in Settings › Machine providers, then preview the import again",
        )]
    };
    let imported: toml::Table = toml::from_str(text).map_err(|error| unreadable(error.to_string()))?;
    let mut file: toml::Table = toml::from_str(current).map_err(|error| {
        unreadable(format!("the providers file on disk isn't valid TOML: {error}"))
    })?;
    let ids = |table: &toml::Table| -> Vec<String> {
        table
            .get("providers")
            .and_then(toml::Value::as_table)
            .map(|providers| providers.keys().cloned().collect())
            .unwrap_or_default()
    };
    let in_file = ids(&file);
    if !ids(&imported).iter().any(|id| in_file.contains(id)) {
        let mut merged = current.to_owned();
        if !merged.is_empty() && !merged.ends_with('\n') {
            merged.push('\n');
        }
        if !merged.is_empty() {
            merged.push('\n');
        }
        merged.push_str(text);
        return Ok(merged);
    }
    for (key, value) in imported {
        if let toml::Value::Table(providers) = &value
            && key == "providers"
            && let Some(table) = file.get_mut("providers").and_then(toml::Value::as_table_mut)
        {
            for (id, provider) in providers {
                table.insert(id.clone(), provider.clone());
            }
            continue;
        }
        file.insert(key, value);
    }
    toml::to_string(&file).map_err(|error| unreadable(format!("the merged providers file can't be written: {error}")))
}

fn problem(field: &str, message: String, fix: &str) -> pb::ProviderConfigProblem {
    pb::ProviderConfigProblem {
        provider: None,
        field: field.to_owned(),
        line: 0,
        error: Some(pb::Error {
            message,
            fix: fix.to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::InvalidField(pb::InvalidField {
                field: field.to_owned(),
                problem: pb::invalid_field::Problem::BadFormat as i32,
            })),
        }),
    }
}

/// A plan that can't be used: unknown, spent, too old, or made before the
/// providers file changed.
pub(super) fn stale(message: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            message.to_owned(),
            "preview the provider file again, check what it shows, then import with the new plan",
            pb::error::Kind::PlanStale(pb::PlanStale { changes: Vec::new() }),
        ),
    )
}
