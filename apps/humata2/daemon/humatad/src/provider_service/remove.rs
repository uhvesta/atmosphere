//! PROVIDERS: taking one imported provider out of the providers file, and
//! why a removal is refused.

use humata2_grpc::humata::v1 as pb;
use tonic::{Code, Status};

use crate::status;

/// Why the providers file couldn't be rewritten without a provider.
#[derive(Debug, thiserror::Error)]
pub(super) enum WithoutError {
    #[error("the providers file isn't valid TOML: {source}")]
    Read {
        #[source]
        source: toml::de::Error,
    },
    #[error("the providers file can't be written back: {source}")]
    Write {
        #[source]
        source: toml::ser::Error,
    },
}

/// The providers file `text` without the provider `id`; the rest is kept.
pub(super) fn without(text: &str, id: &str) -> Result<String, WithoutError> {
    let mut file: toml::Table = toml::from_str(text).map_err(|source| WithoutError::Read { source })?;
    if let Some(providers) = file.get_mut("providers").and_then(toml::Value::as_table_mut) {
        providers.remove(id);
    }
    toml::to_string(&file).map_err(|source| WithoutError::Write { source })
}

fn not_here(message: String, fix: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            message,
            fix,
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "provider".to_owned(),
                problem: pb::invalid_field::Problem::NotAllowedHere as i32,
            }),
        ),
    )
}

/// A provider Humata ships stays (MP-22).
pub(super) fn built_in(label: &str) -> Status {
    not_here(
        format!("{label} is built into Humata, so it can't be removed"),
        "leave it; a provider file that defines the same id replaces it, and removing that brings the built-in one back",
    )
}

/// Machines still use the provider; `machines` is sorted and not empty.
pub(super) fn in_use(label: &str, machines: &[String]) -> Status {
    let names = match machines {
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => String::new(),
    };
    not_here(
        format!("{label} still runs {names}, so it wasn't removed"),
        "delete those machines first (Settings › Machines, or humata provider delete <machine>), then remove the provider",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_a_provider_keeps_the_others() {
        let text = "[providers.a]\nlabel = \"A\"\ncreate = 'a'\n\n[providers.b]\nlabel = \"B\"\ncreate = 'b'\n";
        let left: toml::Table = toml::from_str(&without(text, "a").unwrap()).unwrap();
        let providers = left["providers"].as_table().unwrap();
        assert!(!providers.contains_key("a"));
        assert_eq!(providers["b"]["label"].as_str(), Some("B"));
    }

    #[test]
    fn an_unreadable_file_is_an_error() {
        assert!(matches!(without("not toml [", "a"), Err(WithoutError::Read { .. })));
    }
}
