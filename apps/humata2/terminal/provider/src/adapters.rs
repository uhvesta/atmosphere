use crate::{Adapter, Error, NormalizedEvent, ProviderSignal, decode, required};
use serde::{Deserialize, de::IgnoredAny};

pub(super) fn builtins() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(Hooks(Style::Claude)),
        Box::new(Hooks(Style::Codex)),
        Box::new(Hooks(Style::CopilotCamel)),
        Box::new(Hooks(Style::CopilotVscode)),
        Box::new(OpenCode),
    ]
}

#[derive(Clone, Copy)]
enum Style {
    Claude,
    Codex,
    CopilotCamel,
    CopilotVscode,
}
struct Hooks(Style);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Started,
    TurnStarted,
    TurnFinished,
    Interrupted,
    Ended,
    Notice,
}

#[derive(Deserialize)]
struct Header {
    hook_event_name: Option<String>,
    agent_id: Option<IgnoredAny>,
    #[serde(rename = "agentId")]
    camel_agent_id: Option<IgnoredAny>,
}
#[derive(Deserialize)]
struct SnakeSession {
    session_id: Option<String>,
    cwd: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CamelSession {
    session_id: Option<String>,
    cwd: Option<String>,
}
#[derive(Deserialize)]
struct Start {
    source: Option<String>,
}
#[derive(Deserialize)]
struct Notice {
    notification_type: Option<String>,
    title: Option<String>,
    message: Option<String>,
}

impl Hooks {
    fn kind(&self, event: &str) -> Option<Kind> {
        use Kind::*;
        match (self.0, event) {
            (Style::CopilotCamel, "sessionStart") => Some(Started),
            (Style::CopilotCamel, "userPromptSubmitted") => Some(TurnStarted),
            (Style::CopilotCamel, "agentStop") => Some(TurnFinished),
            (Style::CopilotCamel, "sessionEnd") => Some(Ended),
            // Copilot documents hook_event_name:"Notification" even in the
            // camelCase notification payload; its configured key is lowercase.
            (Style::CopilotCamel, "notification" | "Notification") => Some(Notice),
            (Style::CopilotCamel, _) => None,
            (_, "SessionStart") => Some(Started),
            (_, "UserPromptSubmit") => Some(TurnStarted),
            (_, "Stop") => Some(TurnFinished),
            (_, "SessionEnd") => Some(Ended),
            (Style::Codex, "Interrupt") => Some(Interrupted),
            (Style::Claude | Style::CopilotVscode, "Notification") => Some(Notice),
            _ => None,
        }
    }
}

impl Adapter for Hooks {
    fn schema(&self) -> &str {
        match self.0 {
            Style::Claude => crate::CLAUDE_HOOKS_V1,
            Style::Codex => crate::CODEX_HOOKS_V1,
            Style::CopilotCamel => crate::COPILOT_CAMEL_HOOKS_V1,
            Style::CopilotVscode => crate::COPILOT_VSCODE_HOOKS_V1,
        }
    }
    fn normalize(
        &self,
        payload: &[u8],
        configured: Option<&str>,
    ) -> Result<Option<NormalizedEvent>, Error> {
        let header: Header = decode(payload, self.schema())?;
        // Builtins deliberately observe root sessions only. Dedicated child
        // bindings can use a separately registered adapter; no parent-ID guess.
        if header.agent_id.is_some() || header.camel_agent_id.is_some() {
            return Ok(None);
        }
        let event = if matches!(self.0, Style::CopilotCamel) {
            configured
                .or(header.hook_event_name.as_deref())
                .ok_or(Error::MissingField {
                    field: "hook_event",
                })?
        } else {
            header
                .hook_event_name
                .as_deref()
                .ok_or(Error::MissingField {
                    field: "hook_event_name",
                })?
        };
        if let (Some(configured), Some(supplied)) = (configured, header.hook_event_name.as_deref())
        {
            let matches = configured == supplied
                || self
                    .kind(configured)
                    .is_some_and(|kind| self.kind(supplied) == Some(kind));
            if !matches {
                return Err(Error::EventMismatch);
            }
        }
        let Some(kind) = self.kind(event) else {
            return Ok(None);
        };
        let (session_id, cwd) = if matches!(self.0, Style::CopilotCamel) {
            let session: CamelSession = decode(payload, self.schema())?;
            (session.session_id, session.cwd)
        } else {
            let session: SnakeSession = decode(payload, self.schema())?;
            (session.session_id, session.cwd)
        };
        let signal = match kind {
            Kind::Started => {
                let start: Start = decode(payload, self.schema())?;
                ProviderSignal::SessionStarted {
                    source: Some(required(start.source, "source")?),
                }
            }
            Kind::TurnStarted => ProviderSignal::TurnStarted,
            Kind::TurnFinished => ProviderSignal::TurnFinished,
            Kind::Interrupted => ProviderSignal::Interrupted,
            Kind::Ended => ProviderSignal::SessionEnded,
            Kind::Notice => {
                let notice: Notice = decode(payload, self.schema())?;
                ProviderSignal::Notification {
                    kind: required(notice.notification_type, "notification_type")?,
                    title: notice.title,
                    message: required(notice.message, "message")?,
                }
            }
        };
        Ok(Some(NormalizedEvent {
            session_id: required(session_id, "session_id")?,
            cwd: Some(required(cwd, "cwd")?),
            signal,
        }))
    }
}

struct OpenCode;
#[derive(Deserialize)]
struct OpenHeader {
    #[serde(rename = "type")]
    kind: Option<String>,
}
#[derive(Deserialize)]
struct OpenPayload {
    data: Option<OpenSession>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenSession {
    #[serde(rename = "sessionID")]
    session_id: Option<String>,
    #[serde(rename = "parentID")]
    parent_id: Option<IgnoredAny>,
}

impl Adapter for OpenCode {
    fn schema(&self) -> &str {
        crate::OPENCODE_EVENTS_V2
    }
    fn normalize(
        &self,
        payload: &[u8],
        configured: Option<&str>,
    ) -> Result<Option<NormalizedEvent>, Error> {
        let header: OpenHeader = decode(payload, self.schema())?;
        let kind = required(header.kind, "type")?;
        if configured.is_some_and(|event| event != kind) {
            return Err(Error::EventMismatch);
        }
        let signal = match kind.as_str() {
            "session.created" => ProviderSignal::SessionStarted {
                source: Some("created".into()),
            },
            "session.execution.started" => ProviderSignal::TurnStarted,
            "session.idle" | "session.execution.succeeded" | "session.execution.failed" => {
                ProviderSignal::TurnFinished
            }
            "session.execution.interrupted" => ProviderSignal::Interrupted,
            "session.deleted" => ProviderSignal::SessionEnded,
            _ => return Ok(None),
        };
        let payload: OpenPayload = decode(payload, self.schema())?;
        let data = payload.data.ok_or(Error::MissingField { field: "data" })?;
        if data.parent_id.is_some() {
            return Ok(None);
        }
        Ok(Some(NormalizedEvent {
            session_id: required(data.session_id, "data.sessionID")?,
            cwd: None,
            signal,
        }))
    }
}
