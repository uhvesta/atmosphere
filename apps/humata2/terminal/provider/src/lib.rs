//! Pure, bounded normalization of explicitly supplied provider hook events.
//! No files, processes, provider configuration or notification services are read.
mod adapters;
mod error;
#[cfg(test)]
mod tests;

pub use error::Error;
use std::collections::BTreeMap;

pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
pub const CLAUDE_HOOKS_V1: &str = "claude-hooks-v1";
pub const CODEX_HOOKS_V1: &str = "codex-hooks-v1";
pub const COPILOT_CAMEL_HOOKS_V1: &str = "copilot-camel-hooks-v1";
pub const COPILOT_VSCODE_HOOKS_V1: &str = "copilot-vscode-hooks-v1";
pub const OPENCODE_EVENTS_V2: &str = "opencode-events-v2";

/// Registered by the launcher/host, never extracted from supplied hook JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedContext {
    pub provider: String,
    pub schema: String,
    /// Required for Copilot camelCase hooks whose payload omits an event name.
    pub hook_event: Option<String>,
    /// Explicit launcher-owned recovery paths. Never delivered in UI events.
    pub state_paths: Vec<String>,
}

/// Provider lifecycle is independent of the terminal process's lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderSignal {
    SessionStarted {
        source: Option<String>,
    },
    TurnStarted,
    TurnFinished,
    Interrupted,
    SessionEnded,
    /// Ephemeral text supplied by a documented notification event only.
    Notification {
        kind: String,
        title: Option<String>,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedEvent {
    pub session_id: String,
    pub cwd: Option<String>,
    pub signal: ProviderSignal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEvent {
    pub schema: String,
    pub provider: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub signal: ProviderSignal,
}

/// An explicit extension point. Registry enforces JSON size, identity and
/// normalized field bounds around every adapter, including custom adapters.
pub trait Adapter: Send + Sync {
    fn schema(&self) -> &str;
    fn normalize(
        &self,
        payload: &[u8],
        hook_event: Option<&str>,
    ) -> Result<Option<NormalizedEvent>, Error>;
}

pub struct Registry {
    adapters: BTreeMap<String, Box<dyn Adapter>>,
}

impl Default for Registry {
    fn default() -> Self {
        let adapters = adapters::builtins()
            .into_iter()
            .map(|adapter| (adapter.schema().to_owned(), adapter))
            .collect();
        Self { adapters }
    }
}

impl Registry {
    /// Existing registrations are never silently replaced.
    pub fn register(&mut self, adapter: Box<dyn Adapter>) -> Result<(), Error> {
        text(adapter.schema(), "schema", 128, false)?;
        let schema = adapter.schema().to_owned();
        if self.adapters.contains_key(&schema) {
            return Err(Error::DuplicateSchema { schema });
        }
        self.adapters.insert(schema, adapter);
        Ok(())
    }

    pub fn normalize(
        &self,
        context: &TrustedContext,
        payload: &[u8],
    ) -> Result<Option<ProviderEvent>, Error> {
        validate_context(context)?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::PayloadLimit {
                bytes: payload.len(),
                limit: MAX_PAYLOAD_BYTES,
            });
        }
        let adapter = self
            .adapters
            .get(&context.schema)
            .ok_or_else(|| Error::UnknownSchema {
                schema: context.schema.clone(),
            })?;
        // Validate a complete JSON value, skipping rather than retaining its
        // contents. Builtins then deserialize only documented metadata fields.
        let _: serde::de::IgnoredAny = decode(payload, &context.schema)?;
        let Some(normalized) = adapter.normalize(payload, context.hook_event.as_deref())? else {
            return Ok(None);
        };
        let event = ProviderEvent {
            schema: context.schema.clone(),
            provider: context.provider.clone(),
            session_id: normalized.session_id,
            cwd: normalized.cwd,
            signal: normalized.signal,
        };
        validate_event(context, &event)?;
        Ok(Some(event))
    }
}

/// Also validates native caller-supplied events at a trusted host binding.
/// Does not authorize replacing a previous session ID: the host decides that
/// transition, and only SessionStarted should update persistent recovery.
pub fn validate_event(context: &TrustedContext, event: &ProviderEvent) -> Result<(), Error> {
    validate_context(context)?;
    if event.provider != context.provider || event.schema != context.schema {
        return Err(Error::BindingIdentity);
    }
    text(&event.session_id, "session_id", 512, false)?;
    if let Some(cwd) = &event.cwd {
        text(cwd, "cwd", 4096, false)?;
    }
    match &event.signal {
        ProviderSignal::SessionStarted {
            source: Some(source),
        } => text(source, "source", 128, false)?,
        ProviderSignal::Notification {
            kind,
            title,
            message,
        } => {
            text(kind, "notification_type", 128, false)?;
            if let Some(title) = title {
                text(title, "title", 512, false)?;
            }
            text(message, "message", 8192, true)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_context(context: &TrustedContext) -> Result<(), Error> {
    text(&context.provider, "provider", 64, false)?;
    text(&context.schema, "schema", 128, false)?;
    if let Some(event) = &context.hook_event {
        text(event, "hook_event", 128, false)?;
    }
    if context.state_paths.len() > 16 {
        return Err(Error::FieldLimit {
            field: "state_paths",
            bytes: context.state_paths.len(),
            limit: 16,
        });
    }
    for path in &context.state_paths {
        text(path, "state_path", 4096, false)?;
    }
    Ok(())
}

fn text(value: &str, field: &'static str, limit: usize, multiline: bool) -> Result<(), Error> {
    if value.len() > limit {
        return Err(Error::FieldLimit {
            field,
            bytes: value.len(),
            limit,
        });
    }
    if value.trim().is_empty()
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
    {
        return Err(Error::InvalidField {
            field,
            reason: "must be nonempty text without unsupported control characters",
        });
    }
    Ok(())
}

fn required(value: Option<String>, field: &'static str) -> Result<String, Error> {
    value.ok_or(Error::MissingField { field })
}

fn decode<'a, T: serde::Deserialize<'a>>(bytes: &'a [u8], schema: &str) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|source| Error::Json {
        schema: schema.to_owned(),
        source,
    })
}
