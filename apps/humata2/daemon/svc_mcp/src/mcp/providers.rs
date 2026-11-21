//! Offline read-only machine provider authoring tools (PROVIDERCLI, D906):
//! `provider_guide`, `provider_schema`, `provider_example`, `provider_check`,
//! `provider_render`, and `provider_parse`.
//!
//! Every tool delegates directly to [`humata2_machines::spec`], the same engine
//! behind `humata provider {guide,schema,example,check,render,parse}`. Nothing
//! is executed, written, or imported.

use std::collections::BTreeMap;

use humata2_machines::spec::{
    self, ParseKind, RenderAction, RenderError,
};
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, Value, json};

use super::{GrpcDaemon, applied, text};

fn invalid_field(field: &str, message: impl Into<String>, fix: impl Into<String>) -> ToolError {
    let mut details = Map::new();
    details.insert("field".to_owned(), json!(field));
    details.insert("problem".to_owned(), json!("bad_format"));
    ToolError {
        kind: ErrorKind::InvalidField,
        message: message.into(),
        fix: fix.into(),
        retry: None,
        details,
    }
}

impl GrpcDaemon {
    /// `provider_guide`: plain-text guide to authoring a machine provider.
    pub(super) fn provider_guide(&self) -> Result<Reply, ToolError> {
        Ok(applied(json!({ "guide": spec::provider_guide() }), None))
    }

    /// `provider_schema`: machine provider TOML & command output schema.
    pub(super) fn provider_schema(&self, call: Call) -> Result<Reply, ToolError> {
        let json_only = call
            .arguments
            .get("json")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let schema = spec::provider_schema();
        let value = serde_json::to_value(&schema).map_err(|error| ToolError {
            kind: ErrorKind::Internal,
            message: format!("couldn't serialize provider schema: {error}"),
            fix: String::new(),
            retry: None,
            details: Map::new(),
        })?;
        if json_only {
            Ok(applied(json!({ "schema": value }), None))
        } else {
            Ok(applied(
                json!({
                    "schema": value,
                    "text": schema.render_text(),
                }),
                None,
            ))
        }
    }

    /// `provider_example`: list available examples or return one complete TOML.
    pub(super) fn provider_example(&self, call: Call) -> Result<Reply, ToolError> {
        let kind = call
            .arguments
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match kind {
            None => {
                let examples: Vec<Value> = spec::list_examples()
                    .iter()
                    .map(|example| {
                        json!({
                            "kind": example.kind,
                            "summary": example.summary,
                        })
                    })
                    .collect();
                Ok(applied(json!({ "examples": examples }), None))
            }
            Some(kind) => {
                let example = spec::provider_example(kind).map_err(|error| {
                    let available: Vec<&str> =
                        spec::list_examples().iter().map(|e| e.kind).collect();
                    invalid_field(
                        "kind",
                        error.to_string(),
                        format!("pick one of: {}", available.join(", ")),
                    )
                })?;
                Ok(applied(
                    json!({
                        "kind": example.kind,
                        "summary": example.summary,
                        "toml": example.toml,
                    }),
                    None,
                ))
            }
        }
    }

    /// `provider_check`: validate a provider TOML string offline.
    pub(super) fn provider_check(&self, call: Call) -> Result<Reply, ToolError> {
        let toml = text(&call.arguments, "toml");
        let report = spec::check_providers(&toml);
        let mut value = serde_json::to_value(&report).map_err(|error| ToolError {
            kind: ErrorKind::Internal,
            message: format!("couldn't serialize check report: {error}"),
            fix: String::new(),
            retry: None,
            details: Map::new(),
        })?;
        if let Value::Object(map) = &mut value {
            map.insert("text".to_owned(), json!(report.render_text()));
        }
        Ok(applied(value, None))
    }

    /// `provider_render`: render exact command invocation without running it.
    pub(super) fn provider_render(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let toml = text(arguments, "toml");
        let action_str = text(arguments, "action");
        let action = RenderAction::parse(&action_str).ok_or_else(|| {
            invalid_field(
                "action",
                format!("unknown provider action `{action_str}`"),
                "use one of: create, connect, identity, list, discover, start, stop, wake, refresh, ssh, delete, status",
            )
        })?;
        let machine = text(arguments, "machine");
        let provider_id = arguments
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let cmd = arguments
            .get("cmd")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let mut params = BTreeMap::new();
        if let Some(raw_params) = arguments.get("params") {
            let Some(map) = raw_params.as_object() else {
                return Err(invalid_field(
                    "params",
                    "params must be a JSON object mapping param names to string values",
                    "pass params as {\"key\": \"value\"}",
                ));
            };
            for (k, v) in map {
                let Some(s) = v.as_str() else {
                    return Err(invalid_field(
                        "params",
                        format!("param `{k}` must be a string"),
                        "pass every param value as a string",
                    ));
                };
                params.insert(k.clone(), s.to_owned());
            }
        }
        let rendered = spec::render_provider_action(
            &toml,
            provider_id,
            action,
            &machine,
            &params,
            cmd,
        )
        .map_err(|error| {
            let (field, fix) = match &error {
                RenderError::InvalidConfig { .. } => (
                    "toml",
                    "fix the provider TOML problems reported above (or run provider_check)",
                ),
                RenderError::UnknownProvider { .. } | RenderError::AmbiguousProvider { .. } => (
                    "provider",
                    "pass `provider` with one of the provider ids defined in the TOML",
                ),
                RenderError::BadMachineName(_) => (
                    "machine",
                    "use a lowercase machine name matching [a-z0-9-] (max 63 chars)",
                ),
                RenderError::BadValues(_) => (
                    "params",
                    "supply valid values for every declared param",
                ),
                RenderError::ActionUndefined { .. } => (
                    "action",
                    "pick an action defined by this provider or add it to the TOML",
                ),
                RenderError::Quote(_) => (
                    "cmd",
                    "pass a command string without NUL bytes",
                ),
            };
            invalid_field(field, error.to_string(), fix)
        })?;
        let mut value = serde_json::to_value(&rendered).map_err(|error| ToolError {
            kind: ErrorKind::Internal,
            message: format!("couldn't serialize rendered action: {error}"),
            fix: String::new(),
            retry: None,
            details: Map::new(),
        })?;
        if let Value::Object(map) = &mut value {
            map.insert("text".to_owned(), json!(rendered.render_text()));
        }
        Ok(applied(value, None))
    }

    /// `provider_parse`: validate stdout from `identity`, `status`, or `discover`.
    pub(super) fn provider_parse(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let kind_str = text(arguments, "kind");
        let kind = ParseKind::parse(&kind_str).ok_or_else(|| {
            invalid_field(
                "kind",
                format!("unknown output kind `{kind_str}`"),
                "use one of: identity, status, discover",
            )
        })?;
        let output = text(arguments, "output");
        let machine = arguments
            .get("machine")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let parsed = spec::parse_provider_output(kind, output.as_bytes(), machine).map_err(
            |error| {
                invalid_field(
                    error.field.as_deref().unwrap_or("output"),
                    error.message,
                    error.fix,
                )
            },
        )?;
        let mut value = serde_json::to_value(&parsed).map_err(|error| ToolError {
            kind: ErrorKind::Internal,
            message: format!("couldn't serialize parsed output: {error}"),
            fix: String::new(),
            retry: None,
            details: Map::new(),
        })?;
        if let Value::Object(map) = &mut value {
            map.insert("text".to_owned(), json!(parsed.render_text()));
        }
        Ok(applied(value, None))
    }
}
