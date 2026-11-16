#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Provider event needs {bytes} bytes, above the {limit}-byte limit. Send only a bounded hook event, not transcript contents."
    )]
    PayloadLimit { bytes: usize, limit: usize },
    #[error(
        "Provider field {field} needs {bytes} bytes/items, above its {limit} limit. Reduce the supplied metadata before retrying."
    )]
    FieldLimit {
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    #[error(
        "Provider event is missing {field}. Select the adapter matching the documented hook format and supply its explicit session metadata."
    )]
    MissingField { field: &'static str },
    #[error(
        "Provider field {field} is invalid: {reason}. Correct the explicit hook metadata before retrying."
    )]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error(
        "Provider JSON does not match schema {schema}. Check the selected hook schema and preserve provider files; no configuration was changed."
    )]
    Json {
        schema: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Provider schema {schema} is not registered. Register a compatible adapter or select a supported version explicitly."
    )]
    UnknownSchema { schema: String },
    #[error(
        "Provider schema {schema} is already registered. Choose a different versioned schema key instead of replacing it."
    )]
    DuplicateSchema { schema: String },
    #[error(
        "Provider identity or schema differs from the trusted terminal binding. Reconnect through the correct registered binding; recovery metadata was not changed."
    )]
    BindingIdentity,
    #[error(
        "The supplied hook event differs from its configured event selector. Use the matching hook binding before retrying."
    )]
    EventMismatch,
}
